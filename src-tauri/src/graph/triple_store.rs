//! In-memory HashMap-backed store for (subject, predicate, object) relation
//! triples extracted by Pass 3 (or added manually via `add_manual_triple`).
//!
//! Persists to `{app_data_dir}/triples.json` — the same JSON-sidecar pattern
//! used by `saved_searches/store.rs` for `saved_searches.json` and
//! `spaces/label_cache.rs` for `space_labels.json`.
//!
//! # Schema (D-09, D-10, D-11, D-12 from 11.5-CONTEXT.md)
//! Only the `triples` map is persisted. The `forward_index`, `reverse_index`,
//! and `entity_touches` fields are `#[serde(skip)]` and reconstructed via
//! `rebuild_indices()` after every `load()`.
//! ```json
//! {
//!   "triples": {
//!     "t-uuid-1": {
//!       "id": "t-uuid-1", "subjectId": "ent-alex", "predicate": "owns",
//!       "objectId": "ent-raga2004", "docIds": ["d1"], "userAdded": false,
//!       "createdAt": "2026-07-08T10:00:00Z"
//!     }
//!   }
//! }
//! ```
//!
//! # Two-index design (D-10)
//! - `forward_index`: `(subject_id, predicate) -> HashSet<object_id>` — fast
//!   "X owns ?" queries via `get_objects_for`.
//! - `reverse_index`: `(predicate, object_id) -> HashSet<subject_id>` — fast
//!   "? owns Y" queries via `get_subjects_for`.
//! - `entity_touches`: `entity_id -> HashSet<triple_id>` — fast "all triples
//!   touching this entity (subject OR object)" via `get_by_entity`, used by
//!   the entity detail page Relations panel (D-13, D-17).
//!
//! # Auto-inverse + symmetric writes (D-03)
//! Writing `(A, owns, B)` automatically writes the inverse `(B, owned_by, A)`
//! per `AUTO_INVERSE_PAIRS`. Writing a symmetric predicate `(A, married_to, B)`
//! automatically writes `(B, married_to, A)` per `SYMMETRIC_PREDICATES`.
//!
//! # User override preservation (D-12)
//! `upsert_from_doc` never clobbers a triple with `user_added == true` —
//! instead it merges the incoming `doc_id` into the existing user-added
//! triple's `doc_ids`. This preserves manual corrections across LLM re-runs.
//!
//! # Error resilience (T-11.5-04, mirrors T-11-04)
//! `load()` never panics. Any I/O or JSON parse error silently returns the
//! `Default` (empty store). The worst-case outcome is an empty triple store,
//! not a crash.
//!
//! # Thread-safety
//! `load` / `save` are synchronous. Callers (Plan 04) wrap this in
//! `Arc<tokio::sync::Mutex<TripleStore>>` inside `AppState`, mirroring
//! `SavedSearchStore`.

use crate::types::{is_valid_predicate, Triple, AUTO_INVERSE_PAIRS, SYMMETRIC_PREDICATES};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use uuid::Uuid;

/// In-memory mirror of `{app_data_dir}/triples.json`, plus three rebuilt
/// indices for fast lookup. Only `triples` is serialized; the indices are
/// `#[serde(skip)]` and reconstructed by `rebuild_indices()`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TripleStore {
    /// Canonical map keyed by `triple.id`. Source of truth — indices are
    /// derived from this map and never diverge from it after a `rebuild_indices()`.
    pub triples: HashMap<String, Triple>,

    /// (subject_id, predicate) -> set of object_ids. Fast "X owns ?" queries.
    #[serde(skip)]
    pub forward_index: HashMap<(String, String), HashSet<String>>,

    /// (predicate, object_id) -> set of subject_ids. Fast "? owns Y" queries.
    #[serde(skip)]
    pub reverse_index: HashMap<(String, String), HashSet<String>>,

    /// entity_id -> set of triple_ids where entity is subject OR object.
    /// Enables fast `get_by_entity` for the Relations panel (D-13, D-17).
    #[serde(skip)]
    pub entity_touches: HashMap<String, HashSet<String>>,
}

/// Scans `AUTO_INVERSE_PAIRS` and returns the paired inverse predicate for
/// `predicate`, if one exists.
fn find_inverse_predicate(predicate: &str) -> Option<&'static str> {
    AUTO_INVERSE_PAIRS
        .iter()
        .find(|(a, _)| *a == predicate)
        .map(|(_, b)| *b)
}

impl TripleStore {
    /// Load the store from `{app_data_dir}/triples.json`.
    ///
    /// Returns `Default::default()` (empty store) on any I/O or JSON parse
    /// error — never panics (T-11.5-04 mitigation). On success, rebuilds all
    /// three indices before returning.
    pub fn load(app_data_dir: &Path) -> Self {
        let path = app_data_dir.join("triples.json");
        let mut store: Self = std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        store.rebuild_indices();
        store
    }

    /// Persist the store to `{app_data_dir}/triples.json`.
    ///
    /// Creates the parent directory if needed. Overwrites atomically per
    /// POSIX `write()` semantics (mirror of `SavedSearchStore::save`).
    pub fn save(&self, app_data_dir: &Path) -> std::io::Result<()> {
        if !app_data_dir.exists() {
            std::fs::create_dir_all(app_data_dir)?;
        }
        let path = app_data_dir.join("triples.json");
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        std::fs::write(path, json)
    }

    /// Clear and repopulate `forward_index`, `reverse_index`, and
    /// `entity_touches` from `self.triples`. Called by `load()` and by tests
    /// after mutating `triples` directly.
    pub fn rebuild_indices(&mut self) {
        self.forward_index.clear();
        self.reverse_index.clear();
        self.entity_touches.clear();

        for triple in self.triples.values() {
            self.forward_index
                .entry((triple.subject_id.clone(), triple.predicate.clone()))
                .or_default()
                .insert(triple.object_id.clone());
            self.reverse_index
                .entry((triple.predicate.clone(), triple.object_id.clone()))
                .or_default()
                .insert(triple.subject_id.clone());
            self.entity_touches
                .entry(triple.subject_id.clone())
                .or_default()
                .insert(triple.id.clone());
            self.entity_touches
                .entry(triple.object_id.clone())
                .or_default()
                .insert(triple.id.clone());
        }
    }

    /// Direct HashMap lookup by triple id.
    pub fn get(&self, triple_id: &str) -> Option<&Triple> {
        self.triples.get(triple_id)
    }

    /// Iterate all triples in the store.
    pub fn all(&self) -> impl Iterator<Item = &Triple> {
        self.triples.values()
    }

    /// Internal helper: validate (via a caller-supplied predicate-acceptance
    /// check), assign id/created_at defaults, and upsert a single triple
    /// into `triples` + all three indices.
    ///
    /// `insert_one` (fixed static seed vocabulary) and `insert_one_with_vocab`
    /// (seed + adaptive/corpus-seeded vocabulary) are both thin wrappers
    /// around this shared implementation.
    ///
    /// Upsert semantics (D-12): if a triple with the same
    /// (subject_id, predicate, object_id) already exists AND the stored
    /// triple has `user_added == true`, the stored triple's `user_added`
    /// flag is preserved — only `doc_ids` are merged in. Otherwise the
    /// incoming triple replaces/creates the entry.
    fn insert_one_checked(
        &mut self,
        mut triple: Triple,
        predicate_ok: impl Fn(&str) -> bool,
    ) -> Result<String, String> {
        if !predicate_ok(&triple.predicate) {
            return Err(format!("invalid predicate: {}", triple.predicate));
        }
        if triple.subject_id == triple.object_id {
            return Err("subject_id must differ from object_id".to_string());
        }

        if triple.id.is_empty() {
            triple.id = format!("t-{}", Uuid::new_v4());
        }
        if triple.created_at.is_empty() {
            triple.created_at = chrono::Utc::now().to_rfc3339();
        }

        // Look for an existing triple with the same (subject, predicate, object).
        let existing_id = self.triples.values().find_map(|t| {
            if t.subject_id == triple.subject_id
                && t.predicate == triple.predicate
                && t.object_id == triple.object_id
            {
                Some(t.id.clone())
            } else {
                None
            }
        });

        if let Some(existing_id) = existing_id {
            let stored_user_added = self
                .triples
                .get(&existing_id)
                .map(|t| t.user_added)
                .unwrap_or(false);

            if stored_user_added {
                // Preserve user override; merge doc_ids only.
                if let Some(stored) = self.triples.get_mut(&existing_id) {
                    for doc_id in triple.doc_ids {
                        if !stored.doc_ids.contains(&doc_id) {
                            stored.doc_ids.push(doc_id);
                        }
                    }
                }
                return Ok(existing_id);
            }

            // Not user-added: replace in place (reuse existing id, merge doc_ids).
            triple.id = existing_id.clone();
            self.remove_from_indices(&existing_id);
            self.triples.insert(existing_id.clone(), triple.clone());
            self.add_to_indices(&triple);
            return Ok(existing_id);
        }

        // Brand new triple.
        let id = triple.id.clone();
        self.triples.insert(id.clone(), triple.clone());
        self.add_to_indices(&triple);
        Ok(id)
    }

    /// Validate + upsert a single triple against the fixed static
    /// `PREDICATE_VOCABULARY` (21 seed predicates). Used for manual triples
    /// and internal auto-inverse/symmetric writes, where predicates are
    /// always seed predicates by construction.
    pub fn insert_one(&mut self, triple: Triple) -> Result<String, String> {
        self.insert_one_checked(triple, is_valid_predicate)
    }

    /// Same as `insert_one` but additionally accepts any predicate present
    /// in `extra_vocab` — used for Pass 3 upserts, where the OntologyStore's
    /// runtime-effective vocabulary (seed + corpus-seeded + adaptive +
    /// manual, Phase 11.6) may include predicates beyond the static
    /// `PREDICATE_VOCABULARY`. Fixes a real bug: Pass 3's own
    /// `validate_and_normalize` already accepts these predicates (it's
    /// passed the same effective vocabulary), so a corpus-seeded/adaptive
    /// predicate would pass extraction validation only to be rejected here
    /// at the final storage step — `{"detail":"invalid predicate:
    /// gst_registered_as"}` — permanently stalling that doc's Pass 3.
    pub fn insert_one_with_vocab(
        &mut self,
        triple: Triple,
        extra_vocab: &[String],
    ) -> Result<String, String> {
        self.insert_one_checked(triple, |p| {
            is_valid_predicate(p) || extra_vocab.iter().any(|v| v == p)
        })
    }

    /// Add a single triple's id to all three indices.
    fn add_to_indices(&mut self, triple: &Triple) {
        self.forward_index
            .entry((triple.subject_id.clone(), triple.predicate.clone()))
            .or_default()
            .insert(triple.object_id.clone());
        self.reverse_index
            .entry((triple.predicate.clone(), triple.object_id.clone()))
            .or_default()
            .insert(triple.subject_id.clone());
        self.entity_touches
            .entry(triple.subject_id.clone())
            .or_default()
            .insert(triple.id.clone());
        self.entity_touches
            .entry(triple.object_id.clone())
            .or_default()
            .insert(triple.id.clone());
    }

    /// Remove a triple's id from all three indices (used before replacing or
    /// deleting an existing triple). Does not touch `self.triples`.
    fn remove_from_indices(&mut self, triple_id: &str) {
        let Some(triple) = self.triples.get(triple_id).cloned() else {
            return;
        };
        if let Some(set) = self
            .forward_index
            .get_mut(&(triple.subject_id.clone(), triple.predicate.clone()))
        {
            set.remove(&triple.object_id);
            if set.is_empty() {
                self.forward_index
                    .remove(&(triple.subject_id.clone(), triple.predicate.clone()));
            }
        }
        if let Some(set) = self
            .reverse_index
            .get_mut(&(triple.predicate.clone(), triple.object_id.clone()))
        {
            set.remove(&triple.subject_id);
            if set.is_empty() {
                self.reverse_index
                    .remove(&(triple.predicate.clone(), triple.object_id.clone()));
            }
        }
        if let Some(set) = self.entity_touches.get_mut(&triple.subject_id) {
            set.remove(&triple.id);
            if set.is_empty() {
                self.entity_touches.remove(&triple.subject_id);
            }
        }
        if let Some(set) = self.entity_touches.get_mut(&triple.object_id) {
            set.remove(&triple.id);
            if set.is_empty() {
                self.entity_touches.remove(&triple.object_id);
            }
        }
    }

    /// Apply auto-inverse + symmetric writes for a just-inserted primary triple.
    /// `user_added` on the generated partner mirrors the primary triple's flag
    /// (D-12: manual overrides propagate to their partner too).
    fn apply_auto_writes(&mut self, primary: &Triple) {
        if let Some(inverse_pred) = find_inverse_predicate(&primary.predicate) {
            let inverse = Triple {
                id: String::new(),
                subject_id: primary.object_id.clone(),
                predicate: inverse_pred.to_string(),
                object_id: primary.subject_id.clone(),
                doc_ids: primary.doc_ids.clone(),
                user_added: primary.user_added,
                created_at: String::new(),
            };
            let _ = self.insert_one(inverse);
        }

        if SYMMETRIC_PREDICATES.contains(&primary.predicate.as_str())
            && primary.subject_id != primary.object_id
        {
            let symmetric = Triple {
                id: String::new(),
                subject_id: primary.object_id.clone(),
                predicate: primary.predicate.clone(),
                object_id: primary.subject_id.clone(),
                doc_ids: primary.doc_ids.clone(),
                user_added: primary.user_added,
                created_at: String::new(),
            };
            let _ = self.insert_one(symmetric);
        }
    }

    /// Upsert triples extracted by Pass 3 for a single document. For each
    /// incoming triple: scope `doc_ids` to `[doc_id]`, set `user_added = false`,
    /// and upsert. If a matching (subject, predicate, object) triple already
    /// exists with `user_added == true`, the doc_id is merged into that
    /// triple's provenance without overwriting the override (D-12). After the
    /// primary triple lands, auto-inverse / symmetric partners are written.
    ///
    /// Returns the ids of the primary triples (auto-generated partner ids are
    /// not returned — they are book-keeping only).
    pub fn upsert_from_doc(
        &mut self,
        doc_id: &str,
        incoming: Vec<Triple>,
    ) -> Result<Vec<String>, String> {
        let mut ids = Vec::with_capacity(incoming.len());
        for mut t in incoming {
            t.doc_ids = vec![doc_id.to_string()];
            t.user_added = false;

            let inserted = self.insert_one(t.clone())?;
            ids.push(inserted.clone());

            // Re-fetch the stored triple (it may carry merged doc_ids or a
            // preserved user_added flag) before propagating auto-writes.
            if let Some(stored) = self.triples.get(&inserted).cloned() {
                self.apply_auto_writes(&stored);
            }
        }
        Ok(ids)
    }

    /// Same as `upsert_from_doc` but validates predicates against
    /// `extra_vocab` in addition to the static seed vocabulary (see
    /// `insert_one_with_vocab`), and NEVER aborts the whole batch on one bad
    /// triple — each triple is validated independently; a failing one is
    /// logged and skipped so the rest of the doc's valid triples still land.
    ///
    /// Real bug this fixes: `upsert_from_doc`'s `?` propagation meant a
    /// single invalid triple permanently stalled a doc's Pass 3 completion
    /// (backfill leaves `entities_version` at 3.0 on any upsert error,
    /// retrying — and re-failing on the exact same triple — every
    /// subsequent backfill run). One observed cause: a self-referential
    /// triple that Pass 3's own pre-filter (`validate_and_normalize`)
    /// cannot catch, because it operates on raw `e_N` entity INDICES before
    /// canonical-id resolution — two distinct indices can resolve to the
    /// SAME canonical entity after alias merging (e.g. "Alex" and "Alex
    /// Doe" both resolving to one Person), producing a subject==object
    /// triple only visible after this function's own resolved-id check.
    pub fn upsert_from_doc_with_vocab(
        &mut self,
        doc_id: &str,
        incoming: Vec<Triple>,
        extra_vocab: &[String],
    ) -> Vec<String> {
        let mut ids = Vec::with_capacity(incoming.len());
        for mut t in incoming {
            t.doc_ids = vec![doc_id.to_string()];
            t.user_added = false;

            match self.insert_one_with_vocab(t.clone(), extra_vocab) {
                Ok(inserted) => {
                    ids.push(inserted.clone());
                    // Re-fetch the stored triple (it may carry merged doc_ids
                    // or a preserved user_added flag) before propagating
                    // auto-writes.
                    if let Some(stored) = self.triples.get(&inserted).cloned() {
                        self.apply_auto_writes(&stored);
                    }
                }
                Err(e) => {
                    eprintln!(
                        "[triple_store] skipping invalid triple for doc {}: {} --{}--> {} ({})",
                        doc_id, t.subject_id, t.predicate, t.object_id, e
                    );
                }
            }
        }
        ids
    }

    /// Add a manual (user-authored) triple, called by the `add_manual_triple`
    /// IPC command. Builds a Triple with `user_added = true` and propagates
    /// auto-inverse / symmetric writes with `user_added = true` on the
    /// partner as well (D-12).
    pub fn add_manual(
        &mut self,
        subject_id: String,
        predicate: String,
        object_id: String,
        doc_id: Option<String>,
    ) -> Result<String, String> {
        let triple = Triple {
            id: String::new(),
            subject_id,
            predicate,
            object_id,
            doc_ids: doc_id.map(|d| vec![d]).unwrap_or_default(),
            user_added: true,
            created_at: String::new(),
        };

        let inserted = self.insert_one(triple)?;
        if let Some(stored) = self.triples.get(&inserted).cloned() {
            self.apply_auto_writes(&stored);
        }
        Ok(inserted)
    }

    /// Remove `triple_id` from `triples` and all three indices. Also removes
    /// its auto-inverse / symmetric partner, if one exists (matched by
    /// subject/predicate/object per AUTO_INVERSE_PAIRS + SYMMETRIC_PREDICATES).
    /// Returns true when at least one triple was removed.
    pub fn delete(&mut self, triple_id: &str) -> bool {
        let Some(primary) = self.triples.get(triple_id).cloned() else {
            return false;
        };

        // Find the partner triple, if any, before removing the primary.
        let partner_id = self.find_partner_id(&primary);

        self.remove_from_indices(triple_id);
        self.triples.remove(triple_id);

        if let Some(partner_id) = partner_id {
            self.remove_from_indices(&partner_id);
            self.triples.remove(&partner_id);
        }

        true
    }

    /// Locate the auto-inverse or symmetric partner triple id for `primary`,
    /// if one exists in the store.
    fn find_partner_id(&self, primary: &Triple) -> Option<String> {
        let partner_predicate = find_inverse_predicate(&primary.predicate)
            .map(|s| s.to_string())
            .or_else(|| {
                if SYMMETRIC_PREDICATES.contains(&primary.predicate.as_str()) {
                    Some(primary.predicate.clone())
                } else {
                    None
                }
            })?;

        self.triples.values().find_map(|t| {
            if t.id != primary.id
                && t.subject_id == primary.object_id
                && t.object_id == primary.subject_id
                && t.predicate == partner_predicate
            {
                Some(t.id.clone())
            } else {
                None
            }
        })
    }

    /// Return all triples where `entity_id` is subject OR object, via the
    /// `entity_touches` index. Sorted by (predicate, object_id) for stable
    /// UI rendering order.
    pub fn get_by_entity(&self, entity_id: &str) -> Vec<&Triple> {
        let mut results: Vec<&Triple> = self
            .entity_touches
            .get(entity_id)
            .map(|ids| ids.iter().filter_map(|id| self.triples.get(id)).collect())
            .unwrap_or_default();
        results.sort_by(|a, b| {
            a.predicate
                .cmp(&b.predicate)
                .then_with(|| a.object_id.cmp(&b.object_id))
        });
        results
    }

    /// Forward-index lookup: object_ids where (subject_id, predicate) matches.
    /// Empty when no match.
    pub fn get_objects_for(&self, subject_id: &str, predicate: &str) -> Vec<&str> {
        self.forward_index
            .get(&(subject_id.to_string(), predicate.to_string()))
            .map(|set| set.iter().map(|s| s.as_str()).collect())
            .unwrap_or_default()
    }

    /// Reverse-index lookup: subject_ids where (predicate, object_id) matches.
    pub fn get_subjects_for(&self, predicate: &str, object_id: &str) -> Vec<&str> {
        self.reverse_index
            .get(&(predicate.to_string(), object_id.to_string()))
            .map(|set| set.iter().map(|s| s.as_str()).collect())
            .unwrap_or_default()
    }

    /// Called when a doc is re-indexed (D-25 discretion). Removes `doc_id`
    /// from every triple's `doc_ids`; if a triple ends up with empty
    /// `doc_ids` AND `user_added == false`, the triple is deleted entirely.
    /// User-added triples are preserved even when their last supporting doc
    /// disappears.
    pub fn cleanup_doc(&mut self, doc_id: &str) {
        let mut to_delete: Vec<String> = Vec::new();

        for triple in self.triples.values_mut() {
            triple.doc_ids.retain(|d| d != doc_id);
            if triple.doc_ids.is_empty() && !triple.user_added {
                to_delete.push(triple.id.clone());
            }
        }

        for id in to_delete {
            self.remove_from_indices(&id);
            self.triples.remove(&id);
        }
    }

    /// Rewrite every triple whose predicate == `old` to use `new` instead.
    /// Rebuilds `forward_index`/`reverse_index`/`entity_touches` atomically
    /// (via `rebuild_indices`). Returns the count of triples affected.
    /// Called by `commands::ontology::apply_consolidation`/`rename_predicate`
    /// after the corresponding `OntologyStore` rename (Phase 11.6, Plan 06).
    pub fn rename_predicate_across_all_triples(&mut self, old: &str, new: &str) -> u32 {
        let mut count = 0u32;
        for t in self.triples.values_mut() {
            if t.predicate == old {
                t.predicate = new.to_string();
                count += 1;
            }
        }
        self.rebuild_indices();
        count
    }

    /// Rewrite every triple whose predicate is in `from` to use `into`
    /// instead. Returns the count of triples affected. Called after
    /// `OntologyStore::merge_predicates`/`apply_consolidation` (Plan 06).
    pub fn merge_predicate_across_all_triples(&mut self, from: &[String], into: &str) -> u32 {
        let mut count = 0u32;
        for t in self.triples.values_mut() {
            if from.iter().any(|f| f == &t.predicate) && t.predicate != into {
                t.predicate = into.to_string();
                count += 1;
            }
        }
        self.rebuild_indices();
        count
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn triple(subject: &str, predicate: &str, object: &str) -> Triple {
        Triple {
            id: String::new(),
            subject_id: subject.to_string(),
            predicate: predicate.to_string(),
            object_id: object.to_string(),
            doc_ids: vec![],
            user_added: false,
            created_at: String::new(),
        }
    }

    /// Test 1: `TripleStore::default()` has 0 triples.
    #[test]
    fn test_default_empty() {
        let store = TripleStore::default();
        assert!(store.triples.is_empty(), "Default store must be empty");
    }

    /// Test 2: `load()` on a nonexistent path returns empty default without panicking.
    #[test]
    fn test_load_missing_returns_default() {
        let store = TripleStore::load(Path::new("/nonexistent/path/cortex-test-11.5-02"));
        assert!(
            store.triples.is_empty(),
            "Load from missing path must return empty default, not panic"
        );
    }

    /// Test 3: insert 2 triples, save, load, assert both present with fields intact
    /// and indices rebuilt.
    #[test]
    fn test_roundtrip_preserves_triples() {
        let dir = TempDir::new().unwrap();
        let mut store = TripleStore::default();

        let id1 = store
            .insert_one(triple("ent-a", "owns", "ent-b"))
            .expect("insert 1");
        let id2 = store
            .insert_one(triple("ent-c", "located_in", "ent-d"))
            .expect("insert 2");

        store.save(dir.path()).unwrap();
        let loaded = TripleStore::load(dir.path());

        let t1 = loaded.get(&id1).expect("triple 1 must survive round-trip");
        assert_eq!(t1.subject_id, "ent-a");
        assert_eq!(t1.predicate, "owns");
        assert_eq!(t1.object_id, "ent-b");

        let t2 = loaded.get(&id2).expect("triple 2 must survive round-trip");
        assert_eq!(t2.subject_id, "ent-c");
        assert_eq!(t2.predicate, "located_in");

        // Indices rebuilt after load.
        assert_eq!(
            loaded.get_objects_for("ent-a", "owns"),
            vec!["ent-b"],
            "forward_index must be rebuilt after load"
        );
        assert_eq!(
            loaded.get_subjects_for("located_in", "ent-d"),
            vec!["ent-c"],
            "reverse_index must be rebuilt after load"
        );
    }

    /// Test 4: malformed JSON on disk -> load() returns Default without panicking.
    #[test]
    fn test_malformed_json_returns_default() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("triples.json");
        std::fs::write(&path, b"{{{not valid JSON!!!}}}").unwrap();

        let store = TripleStore::load(dir.path());
        assert!(
            store.triples.is_empty(),
            "Malformed JSON must yield empty default, not panic"
        );
    }

    /// Test 5: insert_one with an invalid predicate returns Err.
    #[test]
    fn test_invalid_predicate_rejected() {
        let mut store = TripleStore::default();
        let result = store.insert_one(triple("ent-a", "resides_at", "ent-b"));
        assert!(result.is_err(), "invalid predicate must be rejected");
    }

    /// Test 6: subject_id == object_id returns Err.
    #[test]
    fn test_self_referential_rejected() {
        let mut store = TripleStore::default();
        let result = store.insert_one(triple("ent-a", "owns", "ent-a"));
        assert!(result.is_err(), "self-referential triple must be rejected");
    }

    // === Adaptive-vocab upsert: real bug fix (corpus-seeded/adaptive
    // predicates rejected at storage even though Pass 3 already validated
    // them against the same effective vocabulary) ===

    #[test]
    fn test_insert_one_rejects_adaptive_predicate_without_vocab() {
        let mut store = TripleStore::default();
        let result = store.insert_one(triple("ent-a", "gst_registered_as", "ent-b"));
        assert!(result.is_err(), "plain insert_one must still reject non-seed predicates");
    }

    #[test]
    fn test_insert_one_with_vocab_accepts_extra_predicate() {
        let mut store = TripleStore::default();
        let extra_vocab = vec!["gst_registered_as".to_string()];
        let result = store.insert_one_with_vocab(
            triple("ent-a", "gst_registered_as", "ent-b"),
            &extra_vocab,
        );
        assert!(result.is_ok(), "adaptive predicate present in extra_vocab must be accepted");
    }

    #[test]
    fn test_insert_one_with_vocab_still_rejects_unknown_predicate() {
        let mut store = TripleStore::default();
        let extra_vocab = vec!["gst_registered_as".to_string()];
        let result = store.insert_one_with_vocab(
            triple("ent-a", "totally_made_up", "ent-b"),
            &extra_vocab,
        );
        assert!(result.is_err(), "predicate absent from both seed AND extra_vocab must still be rejected");
    }

    #[test]
    fn test_insert_one_with_vocab_still_accepts_seed_predicate() {
        let mut store = TripleStore::default();
        let extra_vocab: Vec<String> = vec![];
        let result = store.insert_one_with_vocab(triple("ent-a", "owns", "ent-b"), &extra_vocab);
        assert!(result.is_ok(), "seed predicates must still work with an empty extra_vocab");
    }

    #[test]
    fn test_upsert_from_doc_with_vocab_skips_bad_triple_keeps_good_ones() {
        // Real bug: upsert_from_doc's `?` propagation meant ONE invalid
        // triple aborted the WHOLE batch, silently dropping otherwise-valid
        // triples too. upsert_from_doc_with_vocab must not do this.
        let mut store = TripleStore::default();
        let incoming = vec![
            triple("ent-a", "owns", "ent-b"),      // valid
            triple("ent-c", "owns", "ent-c"),      // self-referential — must be skipped
            triple("ent-d", "owns", "ent-e"),      // valid — must still land
        ];
        let ids = store.upsert_from_doc_with_vocab("doc-1", incoming, &[]);

        assert_eq!(ids.len(), 2, "2 valid triples must be inserted despite 1 bad triple in the batch");
        assert!(
            store.get_by_entity("ent-a").iter().any(|t| t.predicate == "owns" && t.object_id == "ent-b"),
            "first valid triple must have landed"
        );
        assert!(
            store.get_by_entity("ent-d").iter().any(|t| t.predicate == "owns" && t.object_id == "ent-e"),
            "second valid triple (after the bad one) must have landed"
        );
        assert!(
            !store.get_by_entity("ent-c").iter().any(|t| t.subject_id == "ent-c" && t.object_id == "ent-c"),
            "self-referential triple must never be stored"
        );
    }

    #[test]
    fn test_upsert_from_doc_with_vocab_uses_extra_vocab() {
        let mut store = TripleStore::default();
        let extra_vocab = vec!["gst_registered_as".to_string()];
        let ids = store.upsert_from_doc_with_vocab(
            "doc-1",
            vec![triple("ent-a", "gst_registered_as", "ent-b")],
            &extra_vocab,
        );
        assert_eq!(ids.len(), 1, "adaptive predicate must be accepted via upsert_from_doc_with_vocab");
    }

    #[test]
    fn test_upsert_from_doc_with_vocab_never_panics_on_all_invalid() {
        let mut store = TripleStore::default();
        let incoming = vec![
            triple("ent-a", "not_a_real_predicate", "ent-b"),
            triple("ent-c", "owns", "ent-c"),
        ];
        let ids = store.upsert_from_doc_with_vocab("doc-1", incoming, &[]);
        assert!(ids.is_empty(), "all-invalid batch returns empty ids, not a panic or Err");
    }

    /// Test 7: insert (A, owns, B) auto-writes (B, owned_by, A) via upsert_from_doc.
    #[test]
    fn test_auto_inverse_writes_owned_by() {
        let mut store = TripleStore::default();
        store
            .upsert_from_doc("doc-1", vec![triple("ent-a", "owns", "ent-b")])
            .expect("upsert_from_doc");

        assert_eq!(
            store.get_objects_for("ent-a", "owns"),
            vec!["ent-b"],
            "forward_index must have (A, owns) -> {{B}}"
        );
        assert_eq!(
            store.get_objects_for("ent-b", "owned_by"),
            vec!["ent-a"],
            "auto-inverse (B, owned_by) -> {{A}} must exist"
        );

        let has_primary = store
            .all()
            .any(|t| t.subject_id == "ent-a" && t.predicate == "owns" && t.object_id == "ent-b");
        let has_inverse = store.all().any(|t| {
            t.subject_id == "ent-b" && t.predicate == "owned_by" && t.object_id == "ent-a"
        });
        assert!(has_primary, "primary triple must exist in triples map");
        assert!(has_inverse, "inverse triple must exist in triples map");
    }

    /// Test 8: insert (A, married_to, B) writes both directions.
    #[test]
    fn test_symmetric_predicate_writes_both_directions() {
        let mut store = TripleStore::default();
        store
            .upsert_from_doc("doc-1", vec![triple("ent-a", "married_to", "ent-b")])
            .expect("upsert_from_doc");

        let forward = store.all().any(|t| {
            t.subject_id == "ent-a" && t.predicate == "married_to" && t.object_id == "ent-b"
        });
        let reverse = store.all().any(|t| {
            t.subject_id == "ent-b" && t.predicate == "married_to" && t.object_id == "ent-a"
        });
        assert!(forward, "(A, married_to, B) must exist");
        assert!(reverse, "(B, married_to, A) must exist");
    }

    /// Test 9: upsert_from_doc merges doc_ids into an existing user_added triple
    /// without clobbering user_added.
    #[test]
    fn test_upsert_from_doc_merges_doc_ids_on_user_added() {
        let mut store = TripleStore::default();
        let manual_id = store
            .add_manual(
                "ent-a".to_string(),
                "owns".to_string(),
                "ent-b".to_string(),
                Some("d1".to_string()),
            )
            .expect("add_manual");

        store
            .upsert_from_doc("d2", vec![triple("ent-a", "owns", "ent-b")])
            .expect("upsert_from_doc");

        let stored = store.get(&manual_id).expect("manual triple must still exist");
        assert!(stored.user_added, "user_added must remain true after upsert_from_doc");
        assert!(stored.doc_ids.contains(&"d1".to_string()), "doc_ids must retain d1");
        assert!(stored.doc_ids.contains(&"d2".to_string()), "doc_ids must gain d2");
    }

    /// Test 10: delete removes both the primary triple and its auto-inverse partner.
    #[test]
    fn test_delete_removes_inverse_partner() {
        let mut store = TripleStore::default();
        let ids = store
            .upsert_from_doc("doc-1", vec![triple("ent-a", "owns", "ent-b")])
            .expect("upsert_from_doc");
        let primary_id = ids[0].clone();

        assert!(store.delete(&primary_id), "delete must return true");

        assert!(store.get(&primary_id).is_none(), "primary triple must be gone");
        let inverse_still_present = store.all().any(|t| {
            t.subject_id == "ent-b" && t.predicate == "owned_by" && t.object_id == "ent-a"
        });
        assert!(!inverse_still_present, "auto-inverse partner must also be deleted");
    }

    /// Test 11: cleanup_doc preserves a user_added triple even when its last
    /// supporting doc is removed.
    #[test]
    fn test_cleanup_doc_preserves_user_added() {
        let mut store = TripleStore::default();
        let id = store
            .add_manual(
                "ent-a".to_string(),
                "owns".to_string(),
                "ent-b".to_string(),
                Some("d1".to_string()),
            )
            .expect("add_manual");

        store.cleanup_doc("d1");

        let stored = store.get(&id).expect("user-added triple must survive cleanup_doc");
        assert!(stored.doc_ids.is_empty(), "doc_ids must be empty after cleanup");
        assert!(stored.user_added, "user_added must remain true");
    }

    /// Test 12: cleanup_doc removes an LLM-only (user_added=false) triple whose
    /// last doc_id disappears.
    #[test]
    fn test_cleanup_doc_removes_llm_only() {
        let mut store = TripleStore::default();
        let ids = store
            .upsert_from_doc("d1", vec![triple("ent-a", "dated", "ent-b")])
            .expect("upsert_from_doc");
        let id = ids[0].clone();

        store.cleanup_doc("d1");

        assert!(
            store.get(&id).is_none(),
            "LLM-only triple with no remaining doc_ids must be removed"
        );
    }

    /// Test 13: get_by_entity returns both the outgoing triple and its auto-inverse.
    #[test]
    fn test_get_by_entity_returns_both_directions() {
        let mut store = TripleStore::default();
        store
            .upsert_from_doc("doc-1", vec![triple("ent-a", "owns", "ent-b")])
            .expect("upsert_from_doc");

        let a_triples = store.get_by_entity("ent-a");
        assert!(
            a_triples
                .iter()
                .any(|t| t.subject_id == "ent-a" && t.predicate == "owns" && t.object_id == "ent-b"),
            "get_by_entity(A) must contain the outgoing owns triple"
        );

        let b_triples = store.get_by_entity("ent-b");
        assert!(
            b_triples.iter().any(|t| t.subject_id == "ent-b"
                && t.predicate == "owned_by"
                && t.object_id == "ent-a"),
            "get_by_entity(B) must contain the auto-inverse owned_by triple"
        );
    }

    // === Phase 11.6 Plan 06: predicate-rewrite helper tests ===
    //
    // These helpers must operate on adaptive/manual predicate names that are
    // NOT in the fixed PREDICATE_VOCABULARY (is_valid_predicate would reject
    // them), so tests insert directly into `store.triples` + call
    // `rebuild_indices()` rather than going through `insert_one`/`upsert_from_doc`.

    fn insert_raw(store: &mut TripleStore, id: &str, subject: &str, predicate: &str, object: &str) {
        store.triples.insert(
            id.to_string(),
            Triple {
                id: id.to_string(),
                subject_id: subject.to_string(),
                predicate: predicate.to_string(),
                object_id: object.to_string(),
                doc_ids: vec!["doc-1".to_string()],
                user_added: false,
                created_at: "2026-07-10T00:00:00Z".to_string(),
            },
        );
    }

    /// Renaming a predicate present on 5 triples rewrites all 5 occurrences
    /// and rebuilds the forward_index under the new predicate key.
    #[test]
    fn test_rename_predicate_across_all_triples_updates_all_occurrences() {
        let mut store = TripleStore::default();
        for i in 0..5 {
            insert_raw(
                &mut store,
                &format!("t-{i}"),
                &format!("ent-subject-{i}"),
                "custody_of",
                &format!("ent-object-{i}"),
            );
        }
        store.rebuild_indices();

        let rewritten = store.rename_predicate_across_all_triples("custody_of", "guardian_of");

        assert_eq!(rewritten, 5, "all 5 custody_of triples must be rewritten");
        assert!(
            store.triples.values().all(|t| t.predicate == "guardian_of"),
            "no triple should retain the old predicate name"
        );
        assert!(
            store
                .forward_index
                .keys()
                .all(|(_, pred)| pred != "custody_of"),
            "forward_index must not retain any custody_of keys after rebuild"
        );
        assert!(
            store
                .forward_index
                .keys()
                .any(|(_, pred)| pred == "guardian_of"),
            "forward_index must contain rebuilt guardian_of keys"
        );
    }

    /// Merging `from=["custody_of"]` into `"guardian_of"` collapses only the
    /// `from` predicate; pre-existing `guardian_of` triples are untouched and
    /// only the migrated triples count toward the returned rewrite count.
    #[test]
    fn test_merge_predicate_across_all_triples_collapses() {
        let mut store = TripleStore::default();
        for i in 0..3 {
            insert_raw(
                &mut store,
                &format!("custody-{i}"),
                &format!("ent-subject-{i}"),
                "custody_of",
                &format!("ent-object-{i}"),
            );
        }
        for i in 0..2 {
            insert_raw(
                &mut store,
                &format!("guardian-{i}"),
                &format!("ent-g-subject-{i}"),
                "guardian_of",
                &format!("ent-g-object-{i}"),
            );
        }
        store.rebuild_indices();

        let rewritten = store
            .merge_predicate_across_all_triples(&["custody_of".to_string()], "guardian_of");

        assert_eq!(rewritten, 3, "only the 3 custody_of triples should be rewritten");
        assert_eq!(store.triples.len(), 5, "no triples should be deleted, only re-predicated");
        assert!(
            store.triples.values().all(|t| t.predicate == "guardian_of"),
            "only guardian_of should survive as a predicate after merge"
        );
    }

    /// Renaming a predicate that doesn't exist in the store is a no-op:
    /// returns 0 and leaves all triples/state unchanged.
    #[test]
    fn test_rename_predicate_no_op_when_absent() {
        let mut store = TripleStore::default();
        insert_raw(&mut store, "t-1", "ent-a", "owns", "ent-b");
        store.rebuild_indices();

        let rewritten = store.rename_predicate_across_all_triples("no_such_predicate", "renamed");

        assert_eq!(rewritten, 0);
        assert_eq!(store.triples.len(), 1);
        assert_eq!(store.triples.get("t-1").unwrap().predicate, "owns");
    }
}
