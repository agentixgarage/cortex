//! Daily quiz (v1.2 #3, ROADMAP.md) — one lightweight confirm/deny card per
//! day on the Dashboard, plus a "Take more quizzes" section for users who
//! want to actively help train the model.
//!
//! v1 question kind: `alias_confirm` — "Is entity A the same as entity B?"
//! Candidates are pairs of canonical entities (same type) that share at
//! least one name token but were NOT already auto-merged by
//! `EntityStore::run_full_alias_merge` (which handles token-subset and
//! high-cosine cases automatically — see entity_store.rs D-10/D-11). These
//! are exactly the *ambiguous* pairs a human can resolve in one click but
//! the deterministic merge logic correctly declines to guess on.
//!
//! Candidate generation is a PURE function over `&HashMap<String,
//! CanonicalEntity>` — no embeddings, no I/O, no LLM call — so it is fast,
//! deterministic, and fully unit-testable without a running engine.
//!
//! ## Scope note (documented deviation, not a silent gap)
//! Acting on a "Yes, same" answer — actually merging the two canonicals in
//! `EntityStore` (rewriting `alias_index`, `doc_index`, and every indexed
//! document's `extractedEntities` metadata) — is NOT implemented in this
//! pass. `EntityStore` has no pairwise `merge_canonicals(id_a, id_b)`
//! method today; only `run_full_alias_merge` (corpus-wide, embedding-driven)
//! and `split_alias` (the inverse operation) exist. Building a safe pairwise
//! merge is real surgery (doc-index rewrites, entity_store persistence,
//! concurrent-scan interaction) that deserves its own plan rather than being
//! bolted on here under time pressure.
//!
//! What DOES happen today: every answer is logged to
//! `quiz_feedback.json` with a timestamp — the "how well are we doing"
//! measurement the user asked for. A fast-follow can either (a) add
//! `EntityStore::merge_canonicals` and apply confirmed answers immediately,
//! or (b) batch-apply logged "yes" answers on next full realias pass.

pub mod store;

use std::collections::HashMap;

use crate::types::{CanonicalEntity, QuizQuestion};

/// Token-Jaccard similarity between two whitespace-tokenized, lowercased
/// name strings. Returns 0.0 for either-empty input.
fn token_jaccard(a: &str, b: &str) -> f32 {
    let a_tokens: std::collections::HashSet<String> =
        a.split_whitespace().map(|s| s.to_lowercase()).collect();
    let b_tokens: std::collections::HashSet<String> =
        b.split_whitespace().map(|s| s.to_lowercase()).collect();

    if a_tokens.is_empty() || b_tokens.is_empty() {
        return 0.0;
    }

    let intersection = a_tokens.intersection(&b_tokens).count();
    let union = a_tokens.union(&b_tokens).count();
    if union == 0 {
        0.0
    } else {
        intersection as f32 / union as f32
    }
}

/// Whitespace-tokenized subset check (mirrors `entity_store::is_token_subset`
/// — duplicated here rather than imported since that fn is private to
/// entity_store and this candidate generator must stay decoupled from it).
fn is_token_subset(a: &str, b: &str) -> bool {
    let a_tokens: std::collections::HashSet<String> =
        a.split_whitespace().map(|s| s.to_lowercase()).collect();
    let b_tokens: std::collections::HashSet<String> =
        b.split_whitespace().map(|s| s.to_lowercase()).collect();
    if a_tokens.is_empty() || b_tokens.is_empty() {
        return false;
    }
    a_tokens.is_subset(&b_tokens) || b_tokens.is_subset(&a_tokens)
}

/// Generate up to `limit` alias-confirm quiz candidates from the current
/// canonical entity set. Ranked by similarity descending (most-ambiguous /
/// most-likely-useful pairs first). Deterministic given the same input map
/// (HashMap iteration order is NOT guaranteed by Rust, so results are
/// sorted by similarity then by id-pair for stable output).
///
/// Excludes:
/// - Pairs across different `entity_type`
/// - Pairs already token-subset related (those auto-merge; asking is noise)
/// - Identical names (trivially the same, not a real question)
/// - Pairs with zero token overlap (nothing suggests they're related)
pub fn generate_alias_candidates(
    canonicals: &HashMap<String, CanonicalEntity>,
    limit: usize,
) -> Vec<QuizQuestion> {
    let entities: Vec<&CanonicalEntity> = canonicals.values().collect();
    let mut candidates: Vec<QuizQuestion> = Vec::new();

    for i in 0..entities.len() {
        for j in (i + 1)..entities.len() {
            let a = entities[i];
            let b = entities[j];

            if a.entity_type != b.entity_type {
                continue;
            }
            if a.canonical_name.eq_ignore_ascii_case(&b.canonical_name) {
                continue;
            }
            if is_token_subset(&a.canonical_name, &b.canonical_name) {
                continue;
            }

            let sim = token_jaccard(&a.canonical_name, &b.canonical_name);
            if sim <= 0.0 {
                continue;
            }

            // Stable id-pair ordering so re-runs produce identical question ids.
            let (id_a, name_a, id_b, name_b) = if a.id <= b.id {
                (a.id.clone(), a.canonical_name.clone(), b.id.clone(), b.canonical_name.clone())
            } else {
                (b.id.clone(), b.canonical_name.clone(), a.id.clone(), a.canonical_name.clone())
            };

            candidates.push(QuizQuestion {
                id: format!("quiz-alias-{}-{}", id_a, id_b),
                kind: "alias_confirm".to_string(),
                entity_type: a.entity_type.clone(),
                entity_id_a: id_a,
                name_a,
                entity_id_b: id_b,
                name_b,
                similarity: sim,
            });
        }
    }

    candidates.sort_by(|x, y| {
        y.similarity
            .partial_cmp(&x.similarity)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| x.id.cmp(&y.id))
    });
    candidates.truncate(limit);
    candidates
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::CanonicalEntity;

    fn make_entity(id: &str, name: &str, entity_type: &str) -> CanonicalEntity {
        CanonicalEntity {
            id: id.to_string(),
            canonical_name: name.to_string(),
            entity_type: entity_type.to_string(),
            aliases: vec![],
            document_count: 1,
            canonical_short_name: None,
        }
    }

    #[test]
    fn test_token_jaccard_no_overlap() {
        assert_eq!(token_jaccard("Alex Doe", "Jane Roe"), 0.0);
    }

    #[test]
    fn test_token_jaccard_partial_overlap() {
        // {alex, doe} ∩ {alex, roe} = {alex}; union = {alex, doe, roe} → 1/3
        let sim = token_jaccard("Alex Doe", "Alex Roe");
        assert!((sim - 1.0 / 3.0).abs() < 0.001);
    }

    #[test]
    fn test_candidates_excludes_different_types() {
        let mut m = HashMap::new();
        m.insert("1".to_string(), make_entity("1", "Alex Corp", "organization"));
        m.insert("2".to_string(), make_entity("2", "Alex Corp", "person"));
        let out = generate_alias_candidates(&m, 10);
        assert!(out.is_empty());
    }

    #[test]
    fn test_candidates_excludes_identical_names() {
        let mut m = HashMap::new();
        m.insert("1".to_string(), make_entity("1", "Alex Doe", "person"));
        m.insert("2".to_string(), make_entity("2", "alex doe", "person"));
        let out = generate_alias_candidates(&m, 10);
        assert!(out.is_empty());
    }

    #[test]
    fn test_candidates_excludes_token_subset() {
        // "Alex" ⊂ "Alex Doe" — this is exactly what run_full_alias_merge
        // auto-merges; the quiz must not ask about it.
        let mut m = HashMap::new();
        m.insert("1".to_string(), make_entity("1", "Alex", "person"));
        m.insert("2".to_string(), make_entity("2", "Alex Doe", "person"));
        let out = generate_alias_candidates(&m, 10);
        assert!(out.is_empty());
    }

    #[test]
    fn test_candidates_excludes_zero_overlap() {
        let mut m = HashMap::new();
        m.insert("1".to_string(), make_entity("1", "Alex Doe", "person"));
        m.insert("2".to_string(), make_entity("2", "Jane Roe", "person"));
        let out = generate_alias_candidates(&m, 10);
        assert!(out.is_empty());
    }

    #[test]
    fn test_candidates_finds_ambiguous_pair() {
        let mut m = HashMap::new();
        m.insert("1".to_string(), make_entity("1", "Alex Doe", "person"));
        m.insert("2".to_string(), make_entity("2", "Alex Roe", "person"));
        let out = generate_alias_candidates(&m, 10);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].kind, "alias_confirm");
        assert_eq!(out[0].entity_type, "person");
    }

    #[test]
    fn test_candidates_respects_limit() {
        let mut m = HashMap::new();
        for i in 0..6 {
            m.insert(i.to_string(), make_entity(&i.to_string(), &format!("Alex Doe{}", i), "person"));
        }
        let out = generate_alias_candidates(&m, 3);
        assert_eq!(out.len(), 3);
    }

    #[test]
    fn test_candidates_sorted_by_similarity_desc() {
        let mut m = HashMap::new();
        m.insert("1".to_string(), make_entity("1", "Alex Doe Roe", "person"));
        m.insert("2".to_string(), make_entity("2", "Alex Doe Zed", "person")); // higher overlap
        m.insert("3".to_string(), make_entity("3", "Alex Kim", "person")); // lower overlap
        let out = generate_alias_candidates(&m, 10);
        assert!(out.len() >= 2);
        for w in out.windows(2) {
            assert!(w[0].similarity >= w[1].similarity);
        }
    }

    #[test]
    fn test_candidates_stable_id_ordering() {
        let mut m = HashMap::new();
        m.insert("z-id".to_string(), make_entity("z-id", "Alex Roe", "person"));
        m.insert("a-id".to_string(), make_entity("a-id", "Alex Doe", "person"));
        let out = generate_alias_candidates(&m, 10);
        assert_eq!(out[0].entity_id_a, "a-id");
        assert_eq!(out[0].entity_id_b, "z-id");
    }

    #[test]
    fn test_empty_store_returns_empty() {
        let m: HashMap<String, CanonicalEntity> = HashMap::new();
        assert!(generate_alias_candidates(&m, 10).is_empty());
    }
}
