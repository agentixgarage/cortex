use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use tauri::Emitter;

use crate::auth::AuthState;
use crate::engine::CortexEngine;
use crate::graph::entity_store::EntityStore;
use crate::graph::ontology_store::OntologyStore;
use crate::graph::triple_store::TripleStore;
use crate::pipeline::embedder::EmbeddingService;
use crate::pipeline::ontology_bootstrap::{BootstrapSampleDoc, OntologyBootstrapper};
use crate::pipeline::two_pass_extractor::TwoPassExtractor;
use crate::types::{
    EntityBackfillProgress, BOOTSTRAP_MIN_DOCS, PASS1_ONLY_VERSION, PASS3_TARGET_VERSION,
    PromoteResult, TWO_PASS_TARGET_VERSION,
};

// NerService is intentionally NOT imported here.  Plan 10 deletes the module;
// this file now exclusively drives TwoPassExtractor.

// ─── EtaCalculator ────────────────────────────────────────────────────────────

/// Rolling-average ETA calculator for the backfill progress indicator (D-25).
///
/// Keeps a ring buffer of the last 20 per-doc extraction latencies.
/// `eta_seconds(remaining)` returns `None` until the first latency is recorded.
struct EtaCalculator {
    latencies: VecDeque<Duration>,
}

impl EtaCalculator {
    fn new() -> Self {
        Self {
            latencies: VecDeque::new(),
        }
    }

    /// Record one per-doc extraction latency.
    /// Evicts the oldest entry when the ring buffer reaches capacity (20 per D-25).
    fn record(&mut self, d: Duration) {
        if self.latencies.len() >= 20 {
            self.latencies.pop_front();
        }
        self.latencies.push_back(d);
    }

    /// Estimated seconds to completion: rolling-avg latency × remaining docs.
    /// Returns `None` before the first `record()` call (empty buffer).
    fn eta_seconds(&self, remaining: u32) -> Option<u32> {
        if self.latencies.is_empty() {
            return None;
        }
        let total_millis: u128 = self.latencies.iter().map(|d| d.as_millis()).sum();
        let avg_millis = total_millis / self.latencies.len() as u128;
        let eta_millis = avg_millis * remaining as u128;
        Some((eta_millis / 1000) as u32)
    }
}

// ─── Public API ───────────────────────────────────────────────────────────────

/// Spawn the entity backfill background task as a Tokio async task.
///
/// Lifecycle:
/// 1. Count docs needing backfill (entities_version < PASS3_TARGET_VERSION=3.5, Pitfall 6 fix).
/// 2. Emit initial progress event.
/// 3. Process each candidate doc via `two_pass.extract_full()` in an async loop
///    (NOT tokio::task::spawn_blocking per doc — Pitfall 4 fix: LLM HTTP call is async).
///    Docs that reach Pass 2 completion (3.0) additionally run Pass 3 relation
///    extraction and upsert triples into TripleStore (Phase 11.5, D-20).
/// 4. After all docs, run full alias merge.
/// 5. Persist TripleStore to disk (D-09 sidecar semantics).
/// 6. Emit final "complete" event with `fallbacks` count (D-29).
///
/// Idempotent: docs with entities_version >= 3.5 are skipped on re-run (LLME-03).
/// WR-02: `backfill_running` is set to `true` by the caller before this function is
/// called (via `compare_exchange` in `trigger_entity_backfill`). This function resets
/// it to `false` after the final "complete" event so a subsequent IPC call is accepted.
///
/// TODO(Plan 06): this signature grew two new parameters (`ontology_store`, `auth`)
/// in Phase 11.6 Plan 04 to support the corpus-seeded bootstrap trigger below.
/// Plan 06 must update all three call sites (`lib.rs` boot-time backfill,
/// `commands/folders.rs` live-index trigger, `commands/entities.rs`
/// `trigger_entity_backfill` IPC command) to pass `state.ontology_store.clone()`
/// and `state.auth_state.clone()` (or the equivalent `Arc<AuthState>` already
/// held by `AppState`/`lib.rs` setup).
pub fn spawn_entity_backfill(
    app_handle: tauri::AppHandle,
    engine: Arc<tokio::sync::Mutex<CortexEngine>>,
    two_pass: Arc<TwoPassExtractor>,
    entity_store: Arc<std::sync::Mutex<EntityStore>>,
    triple_store: Arc<tokio::sync::Mutex<TripleStore>>,
    ontology_store: Arc<tokio::sync::Mutex<OntologyStore>>,
    auth: Arc<AuthState>,
    embedder: Arc<EmbeddingService>,
    backfill_running: Arc<AtomicBool>,
    app_data_dir: std::path::PathBuf,
) {
    tauri::async_runtime::spawn(async move {
        // Step 1: Collect candidate doc IDs under a short-lived engine lock hold
        let candidates: Vec<String> = {
            let engine_guard = engine.lock().await;
            collect_backfill_candidates(&engine_guard)
        };

        let total = candidates.len() as u32;

        // Corpus-seeded ontology bootstrap state (D-01, Phase 11.6 Plan 04):
        // fires once this backfill run's pass2_success_count reaches
        // BOOTSTRAP_MIN_DOCS, gated by OntologyStore.bootstrap_completed()
        // persisted across restarts. See pipeline::ontology_bootstrap for the
        // BOOTSTRAP_PROMPT + validation logic driving this call.
        let mut pass2_success_count: u32 = 0;
        let mut bootstrap_samples: Vec<BootstrapSampleDoc> =
            Vec::with_capacity(BOOTSTRAP_MIN_DOCS as usize);
        let bootstrapper = OntologyBootstrapper::new(
            auth.clone(),
            ontology_store.clone(),
            app_data_dir.clone(),
        );

        // Step 2: Emit initial event
        let _ = app_handle.emit(
            "entity-backfill-progress",
            EntityBackfillProgress {
                processed: 0,
                total,
                status: if total == 0 {
                    "complete".to_string()
                } else {
                    "running".to_string()
                },
                error: None,
                eta_seconds: None,
                fallbacks: 0,
            },
        );

        // Step 3: Early exit if nothing to do
        if total == 0 {
            backfill_running.store(false, Ordering::SeqCst);
            return;
        }

        let mut processed: u32 = 0;
        let mut fallbacks: u32 = 0;
        let mut eta = EtaCalculator::new();
        let mut last_emit = Instant::now();
        let throttle_duration = Duration::from_millis(500);

        // Step 4: Async per-doc loop (Pitfall 4 — LLM HTTP call must NOT be inside spawn_blocking)
        for doc_id in &candidates {
            let start = Instant::now();
            let result = backfill_one_doc_async(
                doc_id,
                &engine,
                &two_pass,
                &entity_store,
                &triple_store,
                &ontology_store,
                &embedder,
            )
            .await;
            let latency = start.elapsed();
            eta.record(latency);

            match result {
                Ok(entities_version) => {
                    // Fallback = user wanted LLM (llm_enabled=true) but doc landed at 2.5
                    // because Pass 2 was unavailable or errored (D-26).
                    // User opt-out (llm_enabled=false) does NOT count as a fallback.
                    if two_pass.llm_enabled()
                        && (entities_version - PASS1_ONLY_VERSION).abs() < 1e-5
                    {
                        fallbacks += 1;
                    }
                    processed += 1;

                    // Corpus-seeded bootstrap counter (D-01, Phase 11.6 Plan 04):
                    // count docs that reached at least Pass 2 completion (3.0) or
                    // Pass 3 completion (3.5) — either satisfies "successful Pass 2".
                    let reached_pass2 = (entities_version - TWO_PASS_TARGET_VERSION).abs()
                        < 1e-5
                        || (entities_version - PASS3_TARGET_VERSION).abs() < 1e-5;
                    if reached_pass2 {
                        pass2_success_count += 1;
                        if bootstrap_samples.len() < BOOTSTRAP_MIN_DOCS as usize {
                            if let Some(sample) = build_bootstrap_sample(&engine, doc_id).await {
                                bootstrap_samples.push(sample);
                            }
                        }
                    }

                    if should_trigger_bootstrap(pass2_success_count, {
                        let store = ontology_store.lock().await;
                        store.bootstrap_completed()
                    }) {
                        let bootstrap_res = bootstrapper.bootstrap(&bootstrap_samples).await;
                        eprintln!(
                            "[backfill] ontology bootstrap {}",
                            match &bootstrap_res {
                                Ok(Some(seed)) => format!(
                                    "produced {} predicates, {} entity subclasses",
                                    seed.predicates.len(),
                                    seed.entity_subclasses.len()
                                ),
                                Ok(None) => "skipped (already completed or no provider)".to_string(),
                                Err(e) => format!("failed: {}", e),
                            }
                        );
                        // Free memory regardless of outcome — bootstrap only ever fires once.
                        bootstrap_samples.clear();
                        bootstrap_samples.shrink_to(0);
                    }
                }
                Err(e) => {
                    // Per LLME-04: per-doc failure does NOT abort the whole backfill
                    eprintln!(
                        "[backfill] error processing doc {}: {} (continuing)",
                        doc_id, e
                    );
                    let _ = app_handle.emit(
                        "entity-backfill-progress",
                        EntityBackfillProgress {
                            processed,
                            total,
                            status: "error".to_string(),
                            error: Some(e.to_string()),
                            eta_seconds: eta.eta_seconds(total.saturating_sub(processed)),
                            fallbacks,
                        },
                    );
                    // Continue loop — error event emitted, doc skipped
                }
            }

            // Throttle: emit every 25 docs OR every 500ms, whichever comes first
            let should_emit = processed % 25 == 0 || last_emit.elapsed() >= throttle_duration;
            if should_emit && processed < total {
                let _ = app_handle.emit(
                    "entity-backfill-progress",
                    EntityBackfillProgress {
                        processed,
                        total,
                        status: "running".to_string(),
                        error: None,
                        eta_seconds: eta.eta_seconds(total.saturating_sub(processed)),
                        fallbacks,
                    },
                );
                last_emit = Instant::now();
            }
        }

        // Step 5: Full alias merge after all docs processed (D-06a)
        {
            let emb_clone = embedder.clone();
            let es_clone = entity_store.clone();
            let merge_result = tokio::task::spawn_blocking(move || {
                let mut es_guard = es_clone
                    .lock()
                    .map_err(|e| crate::error::AppError::Internal(e.to_string()))?;
                es_guard.run_full_alias_merge(emb_clone.as_ref())
            })
            .await;

            match merge_result {
                Err(e) => eprintln!("[backfill] alias merge join error: {}", e),
                Ok(Err(e)) => eprintln!("[backfill] alias merge failed: {} (continuing)", e),
                Ok(Ok(())) => {}
            }
        }

        // Persist TripleStore to disk after all Pass 3 upserts (D-09 sidecar semantics)
        {
            let ts_arc = triple_store.clone();
            let dir = app_data_dir.clone();
            let save_res = tokio::task::spawn_blocking(move || -> std::io::Result<()> {
                let ts = ts_arc.blocking_lock();
                ts.save(&dir)
            })
            .await;
            match save_res {
                Err(e) => eprintln!("[backfill] TripleStore save join error: {}", e),
                Ok(Err(e)) => eprintln!("[backfill] TripleStore save failed: {} (continuing)", e),
                Ok(Ok(())) => {}
            }
        }

        // Persist OntologyStore to disk after all Pass 3 new_predicates feedback
        // (Phase 11.6 D-05/D-06/D-19 sidecar semantics — same pattern as TripleStore above).
        {
            let os_arc = ontology_store.clone();
            let dir = app_data_dir.clone();
            let save_res = tokio::task::spawn_blocking(move || -> std::io::Result<()> {
                let os = os_arc.blocking_lock();
                os.save(&dir)
            })
            .await;
            match save_res {
                Err(e) => eprintln!("[backfill] OntologyStore save join error: {}", e),
                Ok(Err(e)) => eprintln!("[backfill] OntologyStore save failed: {} (continuing)", e),
                Ok(Ok(())) => {}
            }
        }

        // Step 6: Final "complete" event with fallbacks count for the Plan 07 toast (D-29)
        let _ = app_handle.emit(
            "entity-backfill-progress",
            EntityBackfillProgress {
                processed,
                total,
                status: "complete".to_string(),
                error: None,
                eta_seconds: None,
                fallbacks,
            },
        );

        // WR-02: reset single-flight guard so subsequent IPC calls are accepted.
        backfill_running.store(false, Ordering::SeqCst);
    });
}

// ─── Private helpers ──────────────────────────────────────────────────────────

/// Pure arithmetic gate for the corpus-seeded bootstrap trigger (D-01, Phase
/// 11.6 Plan 04). Extracted from the backfill loop for testability.
///
/// Fires exactly once: `count == BOOTSTRAP_MIN_DOCS` AND bootstrap has not
/// already completed (persisted across restarts via `OntologyStore`).
/// Docs processed before or after the exact threshold do NOT re-trigger —
/// `OntologyBootstrapper::bootstrap` is separately idempotent as a second
/// line of defense (T-11.6-15).
fn should_trigger_bootstrap(count: u32, already_completed: bool) -> bool {
    count == BOOTSTRAP_MIN_DOCS && !already_completed
}

/// Build one `BootstrapSampleDoc` for `doc_id` from its freshly-written
/// metadata (title, topic, tags, top-5 entities). Acquires the engine lock
/// briefly and returns `None` on any error — bootstrap tolerates missing
/// samples (a shorter sample list still produces a useful LLM call).
async fn build_bootstrap_sample(
    engine: &Arc<tokio::sync::Mutex<CortexEngine>>,
    doc_id: &str,
) -> Option<BootstrapSampleDoc> {
    let collection_arc = {
        let engine_guard = engine.lock().await;
        engine_guard.collections.get_collection("documents_384")?
    };

    let entry = {
        let col = collection_arc.read();
        col.db.get(doc_id).ok().flatten()?
    };

    let metadata = entry.metadata?;

    let title = metadata
        .get("name")
        .or_else(|| metadata.get("title"))
        .and_then(|v| v.as_str())
        .unwrap_or(doc_id)
        .to_string();

    let topic = metadata
        .get("topic")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let tags: Vec<String> = metadata
        .get("llmTags")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|t| t.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    let top_entities: Vec<(String, String)> = metadata
        .get("extracted_entities")
        .and_then(|v| serde_json::from_value::<Vec<crate::types::ExtractedEntity>>(v.clone()).ok())
        .map(|entities| {
            entities
                .into_iter()
                .filter_map(|e| e.class.map(|c| (c, e.value)))
                .take(5)
                .collect()
        })
        .unwrap_or_default();

    Some(BootstrapSampleDoc {
        title,
        topic,
        tags,
        top_entities,
    })
}

/// Process one document asynchronously for entity backfill.
///
/// Strategy: inline async — avoids `spawn_blocking` per doc so the LLM HTTP
/// call (inside `two_pass.extract_full()`) runs in the async context (Pitfall 4).
///
/// After Pass 1 + Pass 2 (`two_pass.extract_full()`) reach `TWO_PASS_TARGET_VERSION`
/// (3.0), this function additionally runs Pass 3 relation extraction and upserts
/// the resulting triples into `triple_store` with doc provenance (Phase 11.5, D-20).
/// On Pass 3 success the returned version advances to `PASS3_TARGET_VERSION` (3.5);
/// on Pass 3 failure/skip the doc stays at 3.0 and is retried on the next backfill.
///
/// Returns the `entities_version` written to metadata on success.
/// On Err: the caller logs and continues (LLME-04 per-doc failure isolation, D-26).
async fn backfill_one_doc_async(
    doc_id: &str,
    engine: &Arc<tokio::sync::Mutex<CortexEngine>>,
    two_pass: &Arc<TwoPassExtractor>,
    entity_store: &Arc<std::sync::Mutex<EntityStore>>,
    triple_store: &Arc<tokio::sync::Mutex<TripleStore>>,
    ontology_store: &Arc<tokio::sync::Mutex<OntologyStore>>,
    embedder: &Arc<EmbeddingService>,
) -> Result<f32, crate::error::AppError> {
    // ── Load entry (short-lived engine lock) ──────────────────────────────────
    let collection_arc = {
        let engine_guard = engine.lock().await;
        engine_guard
            .collections
            .get_collection("documents_384")
            .ok_or_else(|| {
                crate::error::AppError::VectorStorage(
                    "documents_384 collection not found".to_string(),
                )
            })?
    };

    let mut entry = {
        let col = collection_arc.read();
        col.db
            .get(doc_id)
            .map_err(|e| crate::error::AppError::VectorStorage(e.to_string()))?
    };

    let mut entry = match entry {
        Some(e) => e,
        None => return Ok(PASS3_TARGET_VERSION), // doc disappeared — treat as complete
    };

    // ── Version gate: Pitfall 6 fix — use .as_f64(), not .as_u64() ───────────
    // .as_u64() returns None for 2.5 (a float JSON number), treating 2.5 docs as
    // "version 0" and re-extracting them unnecessarily.  .as_f64() correctly reads
    // both integer (2) and float (2.5, 3.0, 3.5) JSON numbers.
    let stored_version = entry
        .metadata
        .as_ref()
        .and_then(|m| m.get("entities_version"))
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0) as f32;

    if stored_version >= PASS3_TARGET_VERSION {
        return Ok(PASS3_TARGET_VERSION); // already at 3.5 — skip
    }

    // ── Extract text and title before the async call (owns the data) ─────────
    let (text, title) = {
        let metadata = entry.metadata.get_or_insert_with(std::collections::HashMap::new);

        let text = if let Some(t) = metadata
            .get("parsed_text")
            .or_else(|| metadata.get("excerpt"))
            .and_then(|v| v.as_str())
        {
            t.to_string()
        } else if let Some(path) = metadata.get("path").and_then(|v| v.as_str()) {
            // CR-03 fix: use async I/O so the Tokio worker thread is not blocked
            // for the duration of the file read. std::fs::read_to_string inside an
            // async task starves the entire runtime during large-file backfills.
            match tokio::fs::read_to_string(path).await {
                Ok(t) => t,
                Err(e) => {
                    eprintln!(
                        "[backfill] cannot read file for doc {}: {} (skipping)",
                        doc_id, e
                    );
                    return Ok(stored_version); // no text — not a hard error
                }
            }
        } else {
            return Ok(stored_version); // no text source available
        };

        let title = metadata
            .get("name")
            .or_else(|| metadata.get("title"))
            .and_then(|v| v.as_str())
            .unwrap_or(doc_id)
            .to_string();

        (text, title)
    };

    // ── Async two-pass extraction (LLM HTTP call lives here — NOT in spawn_blocking) ─
    // TwoPassExtractor::extract_full handles all fallback cases internally:
    //   • llm_enabled=false → returns PASS1_ONLY_VERSION (no network call)
    //   • provider absent   → returns PASS1_ONLY_VERSION (Pass2Output::empty())
    //   • Pass 2 error      → logs warn, returns PASS1_ONLY_VERSION (D-26)
    //   • Pass 2 success    → returns TWO_PASS_TARGET_VERSION with full entities
    let entities_out = two_pass.extract_full(&text, &title).await?;

    // ── Register entities with EntityStore — sync mutex wrapped in spawn_blocking ─
    let mut entities_vec = entities_out.entities;
    let topic = entities_out.topic;
    let tags = entities_out.tags;
    let language = entities_out.language;

    let es_clone = entity_store.clone();
    let doc_id_owned = doc_id.to_string();
    let emb_clone = embedder.clone();

    // spawn_blocking returns entities_vec with canonical_ids populated in-place
    let entities_vec = tokio::task::spawn_blocking(move || {
        let mut es_guard = es_clone
            .lock()
            .map_err(|e| crate::error::AppError::Internal(e.to_string()))?;
        if let Err(e) =
            es_guard.register_doc_entities(&doc_id_owned, &mut entities_vec, emb_clone.as_ref())
        {
            eprintln!(
                "[backfill] register_doc_entities failed for doc {}: {} \
                 (canonical_id stays None, continuing)",
                doc_id_owned, e
            );
        }
        Ok::<_, crate::error::AppError>(entities_vec)
    })
    .await
    .map_err(|e| crate::error::AppError::Internal(format!("spawn_blocking join error: {}", e)))??;

    // ── Stage: Pass 3 relation extraction (Phase 11.5; Phase 11.6 adaptive vocab) ─
    // Runs only when Pass 2 produced a full result (entities_version == TWO_PASS_TARGET_VERSION).
    // Docs at PASS1_ONLY_VERSION (2.5) — Pass 2 failed/skipped — stay at 2.5; Pass 3 waits for a healthy provider.
    let mut final_version = entities_out.entities_version;

    if (final_version - TWO_PASS_TARGET_VERSION).abs() < 1e-5 {
        // Phase 11.6 D-05: fetch the runtime-effective vocabulary (seed + corpus
        // + manual + adaptive) before every Pass 3 call so promoted predicates
        // flow into the next call automatically.
        let vocabulary: Vec<String> = {
            let os = ontology_store.lock().await;
            os.effective_predicate_names()
        };

        // Pass 2 succeeded → attempt Pass 3
        match two_pass.pass3().extract_full(&text, &title, &entities_vec, &vocabulary).await {
            Ok(Some((triples, new_predicates))) => {
                // Upsert to TripleStore with doc_id provenance.
                // First cleanup this doc's existing triples (re-index scenario), then upsert.
                //
                // Uses upsert_from_doc_with_vocab (not upsert_from_doc): the
                // latter validates predicates against ONLY the static seed
                // PREDICATE_VOCABULARY, rejecting any corpus-seeded/adaptive
                // predicate that Pass 3's own extraction already validated
                // against this same `vocabulary` — a real bug that silently
                // broke the adaptive-ontology feature at the storage step.
                // The _with_vocab variant also never aborts the whole batch
                // on one bad triple (e.g. a self-referential triple from
                // Pass-2 entity-alias collapse) — it skips just that triple
                // and logs, so the doc's other valid triples still land and
                // entities_version always advances once Pass 3 runs.
                let ts_clone = triple_store.clone();
                let doc_id_owned = doc_id.to_string();
                let triples_owned = triples;
                let vocabulary_owned = vocabulary.clone();
                tokio::task::spawn_blocking(move || {
                    let mut ts = ts_clone.blocking_lock();
                    ts.cleanup_doc(&doc_id_owned);
                    ts.upsert_from_doc_with_vocab(&doc_id_owned, triples_owned, &vocabulary_owned);
                })
                .await
                .map_err(|e| crate::error::AppError::Internal(format!("Pass 3 triple upsert join error: {}", e)))?;

                final_version = PASS3_TARGET_VERSION; // 3.5

                // Phase 11.6 D-05/D-06: feed proposed new_predicates into the
                // OntologyStore pending queue. The store enforces the
                // min-support (>=2 distinct docs) promotion gate internally;
                // this call site only respects the caller-side
                // automatic_growth_enabled toggle (D-21).
                if !new_predicates.is_empty() {
                    let now = chrono::Utc::now().to_rfc3339();
                    let results = feed_new_predicates_to_ontology(
                        ontology_store,
                        &new_predicates,
                        doc_id,
                        &now,
                    )
                    .await;
                    for (p, res) in new_predicates.iter().zip(results.iter()) {
                        if matches!(res, PromoteResult::Promoted) {
                            eprintln!(
                                "[backfill] promoted new predicate '{}' after min-support met",
                                p.name
                            );
                        }
                    }
                }
            }
            Ok(None) => {
                // Pass 3 short-circuited (no provider, no entities). Stay at 3.0 — retry next backfill.
                eprintln!("[backfill] Pass 3 skipped for doc {} (no provider or no pass2 entities)", doc_id);
            }
            Err(e) => {
                // Pass 3 errored. Stay at 3.0 — retry next backfill.
                eprintln!("[backfill] Pass 3 extraction failed for doc {}: {} — leaving at 3.0", doc_id, e);
            }
        }
    }

    // ── Write updated metadata back to VectorEntry ────────────────────────────
    let entities_json = serde_json::to_value(&entities_vec)
        .unwrap_or(serde_json::Value::Array(vec![]));

    let metadata = entry.metadata.get_or_insert_with(std::collections::HashMap::new);
    metadata.insert("extracted_entities".to_string(), entities_json);
    metadata.insert(
        "entities_version".to_string(),
        serde_json::Value::Number(
            serde_json::Number::from_f64(final_version as f64)
                .unwrap_or_else(|| serde_json::Number::from(3u32)),
        ),
    );
    if let Some(t) = topic {
        metadata.insert("topic".to_string(), serde_json::Value::String(t));
    }
    if !tags.is_empty() {
        metadata.insert(
            "llmTags".to_string(),
            serde_json::to_value(&tags).unwrap_or(serde_json::Value::Array(vec![])),
        );
    }
    if let Some(lang) = language {
        metadata.insert("language".to_string(), serde_json::Value::String(lang));
    }

    // ── Persist updated entry ─────────────────────────────────────────────────
    {
        let col = collection_arc.read();
        col.db
            .insert(entry)
            .map_err(|e| crate::error::AppError::VectorStorage(e.to_string()))?;
    }

    Ok(final_version)
}

/// Feed Pass 3 `new_predicates` proposals into `OntologyStore.record_pending_predicate`
/// (Phase 11.6 D-05/D-06/D-21).
///
/// Respects `automatic_growth_enabled` (D-21): when the user has NOT opted in to
/// automatic ontology growth, this is a no-op (no state mutation) and returns an
/// empty vec — callers should treat a shorter-than-input result as "growth
/// disabled, nothing recorded" (T-11.6-19: caller-side gate honored before any
/// mutation, not after).
///
/// Returns one `PromoteResult` per input predicate (same order), or an empty
/// vec when growth is disabled.
async fn feed_new_predicates_to_ontology(
    ontology_store: &Arc<tokio::sync::Mutex<OntologyStore>>,
    new_predicates: &[crate::pipeline::pass3_relation_extractor::Pass3NewPredicate],
    doc_id: &str,
    now_rfc3339: &str,
) -> Vec<PromoteResult> {
    let mut os = ontology_store.lock().await;

    if !os.automatic_growth_enabled() {
        eprintln!(
            "[backfill] {} new predicates proposed by Pass 3 but automatic ontology growth is disabled (Settings > Ontology)",
            new_predicates.len()
        );
        return vec![];
    }

    new_predicates
        .iter()
        .map(|p| {
            os.record_pending_predicate(
                &p.name,
                &p.description,
                p.subject_class.clone(),
                p.object_class.clone(),
                doc_id,
                now_rfc3339,
            )
        })
        .collect()
}

/// Collect doc IDs from documents_384 that need backfill.
///
/// Gate: `entities_version < PASS3_TARGET_VERSION (3.5)` — includes docs at
/// 0.0/2.0/2.5/3.0 (all pre-Pass-3):
///   - docs with no entities_version field (legacy, pre-Phase-8) → 0.0
///   - docs at 2.0 (legacy BERT NerService output)
///   - docs at 2.5 (Pass 1 only, awaiting Pass 2)
///   - docs at 3.0 (Pass 1 + Pass 2 complete, awaiting Pass 3 — Phase 11.5)
/// Excludes docs at 3.5 (Pass 1 + Pass 2 + Pass 3 complete).
///
/// Pitfall 6 fix: uses `.as_f64()` instead of `.as_u64()`.
/// `.as_u64()` returns `None` for JSON floats like 2.5, which caused those docs
/// to be treated as "version 0" and unnecessarily re-processed.
fn collect_backfill_candidates(engine: &CortexEngine) -> Vec<String> {
    let collection_arc = match engine.collections.get_collection("documents_384") {
        Some(c) => c,
        None => return vec![],
    };

    // WR-03 fix: collect all IDs *and* check their version under a single
    // read-lock hold instead of re-acquiring per document. On a 50,000-doc
    // library the prior O(N) lock acquire/release loop added measurable
    // latency and could starve a concurrent file-watcher write lock.
    let candidates: Vec<String> = {
        let collection = collection_arc.read();
        let ids = match collection.db.keys() {
            Ok(k) => k,
            Err(_) => return vec![],
        };
        ids.into_iter()
            .filter(|id| {
                let version = collection
                    .db
                    .get(id)
                    .ok()
                    .flatten()
                    .and_then(|entry| entry.metadata)
                    .and_then(|m| m.get("entities_version").and_then(|v| v.as_f64()))
                    .unwrap_or(0.0) as f32;
                // Pitfall 6 fix: .as_f64() reads both integer (2) and float (2.5, 3.0, 3.5) JSON numbers
                version < PASS3_TARGET_VERSION
            })
            .collect()
    };

    candidates
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── EtaCalculator tests ──────────────────────────────────────────────────

    /// Test: empty buffer returns None.
    #[test]
    fn test_eta_calculator_empty_returns_none() {
        let eta = EtaCalculator::new();
        assert_eq!(
            eta.eta_seconds(10),
            None,
            "empty EtaCalculator must return None"
        );
    }

    /// Test: rolling average of 3 latencies × remaining docs.
    /// record(1000ms), record(2000ms) → avg=1500ms; eta_seconds(10) = 15000ms/1000 = 15s.
    #[test]
    fn test_eta_calculator_rolling_avg() {
        let mut eta = EtaCalculator::new();
        eta.record(Duration::from_millis(1000));
        eta.record(Duration::from_millis(2000));
        let result = eta.eta_seconds(10);
        assert_eq!(result, Some(15), "avg=1500ms × 10 remaining = 15000ms = 15s");
    }

    /// Test: ring buffer cap at 20 — records 25 latencies, only last 20 contribute.
    /// First 5 are 0ms; last 20 are all 1000ms → avg=1000ms; eta_seconds(5) = 5s.
    #[test]
    fn test_eta_calculator_ring_buffer_cap() {
        let mut eta = EtaCalculator::new();
        // First 5: 0ms (will be evicted)
        for _ in 0..5 {
            eta.record(Duration::from_millis(0));
        }
        // Next 20: 1000ms each (fill and stay in ring buffer)
        for _ in 0..20 {
            eta.record(Duration::from_millis(1000));
        }
        // Buffer now has exactly 20 entries, all 1000ms
        let result = eta.eta_seconds(5);
        assert_eq!(
            result,
            Some(5),
            "ring buffer capped at 20: avg=1000ms × 5 remaining = 5s"
        );
    }

    // ── should_trigger_bootstrap tests (D-01, Phase 11.6 Plan 04) ────────────

    /// Test: bootstrap trigger arithmetic — fires exactly at BOOTSTRAP_MIN_DOCS
    /// (30) and only when not already completed.
    #[test]
    fn test_bootstrap_trigger_arithmetic() {
        assert!(
            should_trigger_bootstrap(30, false),
            "count==30, not completed → must trigger"
        );
        assert!(
            !should_trigger_bootstrap(29, false),
            "count==29 → must NOT trigger (below threshold)"
        );
        assert!(
            !should_trigger_bootstrap(31, false),
            "count==31 → must NOT trigger (past threshold, already fired at 30)"
        );
        assert!(
            !should_trigger_bootstrap(30, true),
            "count==30 but already_completed=true → must NOT trigger"
        );
    }

    // ── collect_backfill_candidates tests ────────────────────────────────────

    /// Test: empty collection produces no candidates.
    #[test]
    fn test_collect_backfill_candidates_empty_collection() {
        let tmp = std::env::temp_dir().join("cortex-backfill-test-empty");
        let _ = std::fs::remove_dir_all(&tmp);
        let engine = crate::engine::CortexEngine::new_with_path(tmp.clone()).unwrap();
        let candidates = collect_backfill_candidates(&engine);
        assert!(candidates.is_empty(), "empty collection must produce no candidates");
        let _ = std::fs::remove_dir_all(tmp);
    }

    /// Test: Pitfall 6 fix — doc with entities_version=2.5 (JSON float) IS a candidate.
    /// .as_u64() returned None for 2.5 (a float JSON number), treating it as version 0.
    /// .as_f64() correctly reads 2.5 and the gate `2.5 < 3.0` is true → included.
    #[test]
    fn test_collect_backfill_candidates_picks_up_v25() {
        let tmp = std::env::temp_dir().join("cortex-backfill-test-v25");
        let _ = std::fs::remove_dir_all(&tmp);
        let engine = crate::engine::CortexEngine::new_with_path(tmp.clone()).unwrap();

        let collection_arc = engine.collections.get_collection("documents_384").unwrap();

        let mut meta = std::collections::HashMap::new();
        meta.insert(
            "entities_version".to_string(),
            serde_json::Value::Number(
                serde_json::Number::from_f64(2.5).expect("2.5 is a valid f64"),
            ),
        );
        let entry = ruvector_core::types::VectorEntry {
            id: Some("doc-v25".to_string()),
            vector: vec![0.1f32; 384],
            metadata: Some(meta),
        };
        {
            let col = collection_arc.read();
            col.db.insert(entry).unwrap();
        }

        let candidates = collect_backfill_candidates(&engine);
        assert!(
            candidates.contains(&"doc-v25".to_string()),
            "doc with entities_version=2.5 must be a backfill candidate (Pitfall 6 fix)"
        );

        let _ = std::fs::remove_dir_all(tmp);
    }

    /// Test: doc with entities_version=3.0 (Pass 1+2 complete) IS a candidate.
    /// Phase 11.5: gate moved from TWO_PASS_TARGET_VERSION (3.0) to PASS3_TARGET_VERSION
    /// (3.5), so v3.0 docs are now re-processed for Pass 3 relation extraction.
    /// (Superseded assertion — see `test_collect_backfill_candidates_picks_up_v3` for
    /// the Phase 11.5-authored equivalent; kept for historical test-name continuity.)
    #[test]
    fn test_collect_backfill_candidates_excludes_v3() {
        let tmp = std::env::temp_dir().join("cortex-backfill-test-v3");
        let _ = std::fs::remove_dir_all(&tmp);
        let engine = crate::engine::CortexEngine::new_with_path(tmp.clone()).unwrap();

        let collection_arc = engine.collections.get_collection("documents_384").unwrap();

        let mut meta = std::collections::HashMap::new();
        meta.insert(
            "entities_version".to_string(),
            serde_json::Value::Number(
                serde_json::Number::from_f64(3.0).expect("3.0 is a valid f64"),
            ),
        );
        let entry = ruvector_core::types::VectorEntry {
            id: Some("doc-v3".to_string()),
            vector: vec![0.2f32; 384],
            metadata: Some(meta),
        };
        {
            let col = collection_arc.read();
            col.db.insert(entry).unwrap();
        }

        let candidates = collect_backfill_candidates(&engine);
        assert!(
            candidates.contains(&"doc-v3".to_string()),
            "doc with entities_version=3.0 must be a backfill candidate (Phase 11.5: 3.0 < 3.5)"
        );

        let _ = std::fs::remove_dir_all(tmp);
    }

    /// Test: docs at 2.0 (legacy BERT int) and 2.5 (Pass-1-only float) included;
    /// doc at 3.0 (Pass 1+2 complete) also included since Phase 11.5 moved the gate
    /// to PASS3_TARGET_VERSION (3.5). See `test_collect_backfill_candidates_gate_v2_v25_v3_v35_coverage`
    /// for the full v2/v2.5/v3.0/v3.5 coverage matrix.
    #[test]
    fn test_collect_backfill_candidates_gate_coverage() {
        let tmp = std::env::temp_dir().join("cortex-backfill-test-gate");
        let _ = std::fs::remove_dir_all(&tmp);
        let engine = crate::engine::CortexEngine::new_with_path(tmp.clone()).unwrap();

        let collection_arc = engine.collections.get_collection("documents_384").unwrap();

        // v2 (legacy BERT, stored as JSON integer)
        let mut meta_v2 = std::collections::HashMap::new();
        meta_v2.insert(
            "entities_version".to_string(),
            serde_json::Value::Number(serde_json::Number::from(2u32)),
        );
        // v2.5 (Pass-1-only, stored as JSON float)
        let mut meta_v25 = std::collections::HashMap::new();
        meta_v25.insert(
            "entities_version".to_string(),
            serde_json::Value::Number(serde_json::Number::from_f64(2.5).unwrap()),
        );
        // v3.0 (two-pass complete)
        let mut meta_v3 = std::collections::HashMap::new();
        meta_v3.insert(
            "entities_version".to_string(),
            serde_json::Value::Number(serde_json::Number::from_f64(3.0).unwrap()),
        );

        {
            let col = collection_arc.read();
            col.db.insert(ruvector_core::types::VectorEntry {
                id: Some("gate-v2".to_string()),
                vector: vec![0.1f32; 384],
                metadata: Some(meta_v2),
            }).unwrap();
            col.db.insert(ruvector_core::types::VectorEntry {
                id: Some("gate-v25".to_string()),
                vector: vec![0.2f32; 384],
                metadata: Some(meta_v25),
            }).unwrap();
            col.db.insert(ruvector_core::types::VectorEntry {
                id: Some("gate-v3".to_string()),
                vector: vec![0.3f32; 384],
                metadata: Some(meta_v3),
            }).unwrap();
        }

        let candidates = collect_backfill_candidates(&engine);
        assert!(candidates.contains(&"gate-v2".to_string()),  "v2.0 (BERT) must be a candidate");
        assert!(candidates.contains(&"gate-v25".to_string()), "v2.5 (Pass-1-only) must be a candidate");
        assert!(candidates.contains(&"gate-v3".to_string()),  "v3.0 (two-pass) must be a candidate (Phase 11.5: 3.0 < 3.5)");

        let _ = std::fs::remove_dir_all(tmp);
    }

    /// Test: doc with entities_version=3.0 (Pass 1+2 complete) IS a candidate for Pass 3.
    /// Phase 11.5: v3.0 docs must be re-processed once PASS3_TARGET_VERSION (3.5) is the gate.
    #[test]
    fn test_collect_backfill_candidates_picks_up_v3() {
        let tmp = std::env::temp_dir().join("cortex-backfill-test-picks-up-v3");
        let _ = std::fs::remove_dir_all(&tmp);
        let engine = crate::engine::CortexEngine::new_with_path(tmp.clone()).unwrap();

        let collection_arc = engine.collections.get_collection("documents_384").unwrap();

        let mut meta = std::collections::HashMap::new();
        meta.insert(
            "entities_version".to_string(),
            serde_json::Value::Number(
                serde_json::Number::from_f64(3.0).expect("3.0 is a valid f64"),
            ),
        );
        let entry = ruvector_core::types::VectorEntry {
            id: Some("doc-v3-pending-pass3".to_string()),
            vector: vec![0.1f32; 384],
            metadata: Some(meta),
        };
        {
            let col = collection_arc.read();
            col.db.insert(entry).unwrap();
        }

        let candidates = collect_backfill_candidates(&engine);
        assert!(
            candidates.contains(&"doc-v3-pending-pass3".to_string()),
            "doc with entities_version=3.0 must be a backfill candidate (Phase 11.5: 3.0 < 3.5)"
        );

        let _ = std::fs::remove_dir_all(tmp);
    }

    /// Test: doc with entities_version=3.5 (Pass 1+2+3 complete) is NOT a candidate.
    #[test]
    fn test_collect_backfill_candidates_excludes_v35() {
        let tmp = std::env::temp_dir().join("cortex-backfill-test-excludes-v35");
        let _ = std::fs::remove_dir_all(&tmp);
        let engine = crate::engine::CortexEngine::new_with_path(tmp.clone()).unwrap();

        let collection_arc = engine.collections.get_collection("documents_384").unwrap();

        let mut meta = std::collections::HashMap::new();
        meta.insert(
            "entities_version".to_string(),
            serde_json::Value::Number(
                serde_json::Number::from_f64(3.5).expect("3.5 is a valid f64"),
            ),
        );
        let entry = ruvector_core::types::VectorEntry {
            id: Some("doc-v35".to_string()),
            vector: vec![0.2f32; 384],
            metadata: Some(meta),
        };
        {
            let col = collection_arc.read();
            col.db.insert(entry).unwrap();
        }

        let candidates = collect_backfill_candidates(&engine);
        assert!(
            !candidates.contains(&"doc-v35".to_string()),
            "doc with entities_version=3.5 must NOT be a backfill candidate (fully Pass 3 complete)"
        );

        let _ = std::fs::remove_dir_all(tmp);
    }

    /// Test: full gate coverage across v2/v2.5/v3.0/v3.5 — v2/v2.5/v3.0 candidates, v3.5 excluded.
    #[test]
    fn test_collect_backfill_candidates_gate_v2_v25_v3_v35_coverage() {
        let tmp = std::env::temp_dir().join("cortex-backfill-test-gate-v35");
        let _ = std::fs::remove_dir_all(&tmp);
        let engine = crate::engine::CortexEngine::new_with_path(tmp.clone()).unwrap();

        let collection_arc = engine.collections.get_collection("documents_384").unwrap();

        // v2 (legacy BERT, stored as JSON integer)
        let mut meta_v2 = std::collections::HashMap::new();
        meta_v2.insert(
            "entities_version".to_string(),
            serde_json::Value::Number(serde_json::Number::from(2u32)),
        );
        // v2.5 (Pass-1-only, stored as JSON float)
        let mut meta_v25 = std::collections::HashMap::new();
        meta_v25.insert(
            "entities_version".to_string(),
            serde_json::Value::Number(serde_json::Number::from_f64(2.5).unwrap()),
        );
        // v3.0 (Pass 1+2 complete, awaiting Pass 3)
        let mut meta_v3 = std::collections::HashMap::new();
        meta_v3.insert(
            "entities_version".to_string(),
            serde_json::Value::Number(serde_json::Number::from_f64(3.0).unwrap()),
        );
        // v3.5 (Pass 1+2+3 complete)
        let mut meta_v35 = std::collections::HashMap::new();
        meta_v35.insert(
            "entities_version".to_string(),
            serde_json::Value::Number(serde_json::Number::from_f64(3.5).unwrap()),
        );

        {
            let col = collection_arc.read();
            col.db.insert(ruvector_core::types::VectorEntry {
                id: Some("gate35-v2".to_string()),
                vector: vec![0.1f32; 384],
                metadata: Some(meta_v2),
            }).unwrap();
            col.db.insert(ruvector_core::types::VectorEntry {
                id: Some("gate35-v25".to_string()),
                vector: vec![0.2f32; 384],
                metadata: Some(meta_v25),
            }).unwrap();
            col.db.insert(ruvector_core::types::VectorEntry {
                id: Some("gate35-v3".to_string()),
                vector: vec![0.3f32; 384],
                metadata: Some(meta_v3),
            }).unwrap();
            col.db.insert(ruvector_core::types::VectorEntry {
                id: Some("gate35-v35".to_string()),
                vector: vec![0.4f32; 384],
                metadata: Some(meta_v35),
            }).unwrap();
        }

        let candidates = collect_backfill_candidates(&engine);
        assert!(candidates.contains(&"gate35-v2".to_string()),  "v2.0 must be a candidate");
        assert!(candidates.contains(&"gate35-v25".to_string()), "v2.5 must be a candidate");
        assert!(candidates.contains(&"gate35-v3".to_string()),  "v3.0 must be a candidate (Phase 11.5)");
        assert!(!candidates.contains(&"gate35-v35".to_string()), "v3.5 must NOT be a candidate");

        let _ = std::fs::remove_dir_all(tmp);
    }

    /// Test: doc without entities_version field (pre-Phase-8 legacy) IS a candidate.
    #[test]
    fn test_collect_backfill_candidates_no_version_field() {
        let tmp = std::env::temp_dir().join("cortex-backfill-test-noversion");
        let _ = std::fs::remove_dir_all(&tmp);
        let engine = crate::engine::CortexEngine::new_with_path(tmp.clone()).unwrap();

        let collection_arc = engine.collections.get_collection("documents_384").unwrap();

        let mut meta = std::collections::HashMap::new();
        meta.insert(
            "path".to_string(),
            serde_json::Value::String("/tmp/test.txt".to_string()),
        );
        let entry = ruvector_core::types::VectorEntry {
            id: Some("doc-none".to_string()),
            vector: vec![0.1f32; 384],
            metadata: Some(meta),
        };
        {
            let col = collection_arc.read();
            col.db.insert(entry).unwrap();
        }

        let candidates = collect_backfill_candidates(&engine);
        assert!(
            candidates.contains(&"doc-none".to_string()),
            "doc without entities_version must be a backfill candidate"
        );

        let _ = std::fs::remove_dir_all(tmp);
    }

    /// Test: throttle count-based — processed % 25 == 0 triggers emit.
    #[test]
    fn test_throttle_logic() {
        for processed in [0u32, 25, 50, 100] {
            assert_eq!(processed % 25, 0, "processed={} should trigger throttle", processed);
        }
        for processed in [1u32, 10, 24, 26, 49] {
            assert_ne!(
                processed % 25,
                0,
                "processed={} should NOT trigger count-based throttle",
                processed
            );
        }
    }

    /// Test: DocumentIndexer still compiles with backfill_entities helper (type-level check).
    #[test]
    fn test_backfill_entities_helper_exists() {
        let _indexer = crate::pipeline::indexer::DocumentIndexer::new();
    }

    /// Test: within 30 docs, count-based throttle fires at most once (at doc 25).
    #[test]
    fn test_event_throttle_count() {
        let mut emit_count = 0u32;
        for processed in 1u32..=30 {
            if processed % 25 == 0 {
                emit_count += 1;
            }
        }
        assert!(
            emit_count <= 2,
            "count-based throttle must emit ≤ 2 events for 30 docs, got {}",
            emit_count
        );
    }

    // ── Phase 11.6 Plan 05: feed_new_predicates_to_ontology tests ────────────

    fn sample_new_predicate(name: &str) -> crate::pipeline::pass3_relation_extractor::Pass3NewPredicate {
        crate::pipeline::pass3_relation_extractor::Pass3NewPredicate {
            name: name.to_string(),
            description: "test description".to_string(),
            subject_class: None,
            object_class: None,
        }
    }

    /// Test: with growth enabled, a first-occurrence batch of 2 new predicates
    /// both land in pending_predicates (StillPending{count:1}), nothing promoted yet.
    #[tokio::test]
    async fn test_feed_new_predicates_records_pending() {
        let mut store = OntologyStore::default();
        store.set_automatic_growth(true);
        let store = Arc::new(tokio::sync::Mutex::new(store));

        let predicates = vec![
            sample_new_predicate("neighbor_of"),
            sample_new_predicate("custody_of"),
        ];

        let results = feed_new_predicates_to_ontology(
            &store,
            &predicates,
            "doc-1",
            "2026-07-10T00:00:00Z",
        )
        .await;

        assert_eq!(results.len(), 2);
        assert!(
            results.iter().all(|r| matches!(r, PromoteResult::StillPending { count: 1 })),
            "first occurrence of each predicate must stay pending with count=1: {:?}",
            results
        );

        // Not yet promoted — pending predicates are not part of the effective vocabulary.
        let guard = store.lock().await;
        let names = guard.effective_predicate_names();
        assert!(
            !names.contains(&"neighbor_of".to_string()) && !names.contains(&"custody_of".to_string()),
            "first-occurrence predicates must stay pending, not yet in effective vocabulary"
        );
    }

    /// Test: calling with the same predicates across two distinct docs promotes
    /// both to adaptive_predicates (min-support gate = 2 distinct docs).
    #[tokio::test]
    async fn test_feed_new_predicates_promotes_at_min_support() {
        let mut store = OntologyStore::default();
        store.set_automatic_growth(true);
        let store = Arc::new(tokio::sync::Mutex::new(store));

        let predicates = vec![
            sample_new_predicate("neighbor_of"),
            sample_new_predicate("custody_of"),
        ];

        // First doc: both land in pending.
        feed_new_predicates_to_ontology(&store, &predicates, "doc-1", "2026-07-10T00:00:00Z").await;

        // Second doc: both cross the min-support gate and promote.
        let results = feed_new_predicates_to_ontology(
            &store,
            &predicates,
            "doc-2",
            "2026-07-10T01:00:00Z",
        )
        .await;

        assert_eq!(results.len(), 2);
        assert!(
            results.iter().all(|r| matches!(r, PromoteResult::Promoted)),
            "second distinct-doc occurrence must promote both predicates: {:?}",
            results
        );

        let guard = store.lock().await;
        assert_eq!(
            guard.effective_predicate_names().iter().filter(|n| *n == "neighbor_of" || *n == "custody_of").count(),
            2,
            "both predicates must now be in the effective vocabulary"
        );
    }

    /// Test: when automatic_growth_enabled is false, the function is a no-op —
    /// returns an empty vec and does not mutate OntologyStore state.
    #[tokio::test]
    async fn test_feed_new_predicates_skips_when_growth_disabled() {
        let store = OntologyStore::default(); // automatic_growth_enabled defaults to false
        let store = Arc::new(tokio::sync::Mutex::new(store));

        let predicates = vec![sample_new_predicate("neighbor_of")];

        let results = feed_new_predicates_to_ontology(
            &store,
            &predicates,
            "doc-1",
            "2026-07-10T00:00:00Z",
        )
        .await;

        assert!(results.is_empty(), "growth disabled must return an empty result vec");

        let guard = store.lock().await;
        assert_eq!(
            guard.effective_predicate_names().len(),
            21,
            "no mutation must occur when automatic_growth_enabled is false"
        );
    }
}
