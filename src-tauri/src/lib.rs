mod auth;
mod ai;
mod commands;
mod error;
mod state;
mod engine;
mod types;
pub mod pipeline;
pub mod watcher;
pub mod search;
pub mod spaces;
pub mod saved_searches;
pub mod graph;
pub mod intelligence;
pub mod chat;
pub mod profile;
pub mod quiz;

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tokio::sync::Mutex;
use tokio::sync::mpsc;
use tauri::Manager;
use state::AppState;
use engine::CortexEngine;
use pipeline::embedder::EmbeddingService;
use pipeline::indexer::DocumentIndexer;
use pipeline::two_pass_extractor::TwoPassExtractor;
use graph::edges::DocumentGraph;
use graph::entity_store::EntityStore;
use graph::ontology_store::OntologyStore;
use intelligence::analytics::{ActivityLog, SearchTracker};
use intelligence::sona_bridge::SearchLearner;
use spaces::manager::SpaceManager;
use watcher::registry::WatcherRegistry;

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_fs::init())
        .setup(|app| {
            let app_data = app.path().app_data_dir()
                .expect("could not resolve app data dir");
            std::fs::create_dir_all(&app_data)?;

            let data_dir = app_data.join("vectors");
            let registry_path = app_data.join("watcher-registry.json");

            let engine = CortexEngine::new_with_path(data_dir)
                .expect("RuVector initialization failed");

            // Initialize embedding service (downloads ~90MB model on first run)
            let embedding_service = Arc::new(
                EmbeddingService::new_local()
                    .expect("Embedding model init failed — check ~/.cache/fastembed/"),
            );

            // Load persistent watcher registry
            let registry = Arc::new(std::sync::Mutex::new(
                WatcherRegistry::load(&registry_path),
            ));

            // Create document indexer
            let indexer = Arc::new(DocumentIndexer::new());

            // Channels for watcher communication
            let (watcher_tx, watcher_rx) = mpsc::channel(32);
            let (_index_tx, index_rx) = mpsc::channel(32);

            let engine_arc = Arc::new(Mutex::new(engine));

            // Rebuild path index from persisted vectors so already-indexed files are skipped
            {
                let engine_guard = engine_arc.blocking_lock();
                if let Err(e) = indexer.rebuild_path_index(&engine_guard) {
                    eprintln!("Warning: failed to rebuild path index: {}", e);
                }
            }

            // Create EntityStore and rebuild from existing collection metadata
            let entity_store = Arc::new(std::sync::Mutex::new(EntityStore::new()));
            {
                let engine_guard = engine_arc.blocking_lock();
                let mut store = entity_store.lock().expect("entity_store lock");
                if let Err(e) = store.rebuild_from_engine(&engine_guard, &embedding_service) {
                    eprintln!("Warning: failed to rebuild entity store: {}", e);
                }
            }

            // Create SpaceManager for Smart Spaces
            // Phase 9: tokio::sync::Mutex since recluster() became async.
            let space_manager = Arc::new(Mutex::new(SpaceManager::new()));

            // Create DocumentGraph for related documents
            let doc_graph = Arc::new(std::sync::Mutex::new(DocumentGraph::new()));

            // Create SearchLearner (SONA self-learning, 384-dim)
            let search_learner = Arc::new(std::sync::Mutex::new(SearchLearner::new(384)));

            // Create SearchTracker for analytics
            let search_tracker = Arc::new(std::sync::Mutex::new(SearchTracker::new()));

            // Create ActivityLog for activity feed
            let activity_log = Arc::new(std::sync::Mutex::new(ActivityLog::new()));

            // Phase 7: AI provider credential store
            // MUST be registered before AppState to avoid Tauri "State was not properly initialized" panic
            let auth_state = crate::auth::AuthState::new(&app_data);
            let oauth_flow_state = crate::auth::oauth::OAuthFlowState::new();

            // Phase 8 Plan 05: TwoPassExtractor
            // AuthState is Clone (derives Clone; inner Arc<Mutex<CredentialStore>> is shared by ref).
            // We clone before passing ownership to app.manage() so both auth_state and auth_arc
            // see the same underlying CredentialStore.
            // Created before spawn_watcher_task so the Arc can be shared with the watcher.
            let auth_arc = Arc::new(auth_state.clone());
            let two_pass = Arc::new(
                TwoPassExtractor::new(auth_arc.clone())
                    .expect("TwoPassExtractor init failed — Pass 1 regex compile error"),
            );

            // Load persisted extraction settings and apply to runtime extractor.
            {
                let settings_path = app_data.join("settings.json");
                if let Ok(contents) = std::fs::read_to_string(&settings_path) {
                    if let Ok(s) = serde_json::from_str::<crate::types::Settings>(&contents) {
                        // Apply model synchronously via block_on (setup() runs in the Tokio context).
                        tauri::async_runtime::block_on(async {
                            two_pass.set_model(s.extraction_model.clone()).await;
                        });
                        two_pass.set_llm_enabled(s.use_llm_extraction);
                    }
                }
                // If settings.json is missing, defaults are fine: model="" (provider default), llm_enabled=true.
            }

            // Spawn persistent watcher background task
            let app_handle = app.handle().clone();
            watcher::worker::spawn_watcher_task(
                app_handle,
                engine_arc.clone(),
                embedding_service.clone(),
                two_pass.clone(),
                indexer.clone(),
                registry.clone(),
                registry_path.clone(),
                watcher_rx,
                activity_log.clone(),
                entity_store.clone(),
            );

            // Phase 11.5 Plan 04: Relation triple store (loaded from triples.json sidecar, empty if absent)
            let triple_store = Arc::new(Mutex::new(
                crate::graph::triple_store::TripleStore::load(&app_data),
            ));

            // Phase 11.6: Adaptive ontology store.
            let ontology_store = Arc::new(Mutex::new(
                crate::graph::ontology_store::OntologyStore::load(&app_data),
            ));

            // Boot-time backfill: silently upgrade legacy docs (entities_version < 3.5)
            // to full three-pass (v3.5) when a provider is connected.
            // Decision (Plan 06): kept as boot-time so BERT-era (v2.0) and Pass-1-only
            // (v2.5) docs are migrated transparently without user action.
            // The explicit "Re-extract" button in Settings (Plan 07) is for forcing
            // re-extraction after switching providers or models.
            // WR-02: single-flight guard shared between boot-time backfill and
            // trigger_entity_backfill IPC command. Constructed once here so both
            // paths see the same AtomicBool.
            let backfill_running = Arc::new(AtomicBool::new(false));
            {
                let app_handle_bf = app.handle().clone();
                backfill_running.store(true, std::sync::atomic::Ordering::SeqCst);
                pipeline::backfill::spawn_entity_backfill(
                    app_handle_bf,
                    engine_arc.clone(),
                    two_pass.clone(),
                    entity_store.clone(),
                    triple_store.clone(),
                    ontology_store.clone(),
                    auth_arc.clone(),
                    embedding_service.clone(),
                    backfill_running.clone(),
                    app_data.clone(),
                );
            }

            // auth_state and oauth_flow_state must be registered before AppState
            // (commands that use State<'_, AuthState> would panic otherwise).
            app.manage(auth_state);
            app.manage(oauth_flow_state);

            // Phase 9: Space label cache (loaded from space_labels.json sidecar, empty if absent)
            let space_label_cache = Arc::new(Mutex::new(
                crate::spaces::label_cache::SpaceLabelCache::load(&app_data),
            ));

            // Phase 11 Plan 04: Saved-search store (loaded from saved_searches.json sidecar, empty if absent)
            let saved_search_store = Arc::new(Mutex::new(
                crate::saved_searches::store::SavedSearchStore::load(&app_data),
            ));

            // Phase 11.7 Plan 05: RAG chat session store
            let chat_session_store = Arc::new(Mutex::new(
                crate::chat::session_store::ChatSessionStore::load(&app_data),
            ));

            // v1.2 #2: User profile (onboarding "About You" step)
            let user_profile = Arc::new(Mutex::new(
                crate::profile::store::load(&app_data),
            ));

            // v1.2 #3: Daily quiz feedback log
            let quiz_feedback = Arc::new(Mutex::new(
                crate::quiz::store::QuizFeedbackStore::load(&app_data),
            ));

            // Phase 10 (Plan 06): Initialize hyperbolic HNSW secondary index fields.
            // Both start empty — None for the index (D-11 fallback active until first recluster),
            // and an empty Vec for the id map. Populated by rebuild_hyp_index() after recluster.
            let hyp_index = Arc::new(Mutex::new(None));
            let hyp_id_to_space = Arc::new(Mutex::new(Vec::new()));

            app.manage(AppState {
                engine: engine_arc,
                watcher_tx,
                index_rx: Arc::new(Mutex::new(index_rx)),
                embedding_service,
                indexer,
                registry,
                registry_path,
                space_manager,
                doc_graph,
                search_learner,
                search_tracker,
                activity_log,
                entity_store,
                two_pass_extractor: two_pass,
                backfill_running,
                space_label_cache,
                app_data_dir: app_data,
                hyp_index,
                hyp_id_to_space,
                saved_search_store,
                triple_store,
                chat_session_store,
                ontology_store,
                auth_state: auth_arc,
                user_profile,
                quiz_feedback,
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // documents (5)
            commands::documents::index_document,
            commands::documents::search_documents,
            commands::documents::get_document,
            commands::documents::get_related_documents,
            commands::documents::toggle_favorite,
            commands::documents::record_search_click,
            commands::documents::get_recent_documents,
            commands::documents::get_favorite_documents,
            // spaces (8) — Phase 9 adds 4 label-related commands
            commands::spaces::get_spaces,
            commands::spaces::get_space_documents,
            commands::spaces::move_document_to_space,
            commands::spaces::recluster_spaces,
            commands::spaces::get_space_labels,
            commands::spaces::rename_space_label,
            commands::spaces::clear_space_override,
            commands::spaces::trigger_relabel,
            // folders (4)
            commands::folders::add_watched_folder,
            commands::folders::remove_watched_folder,
            commands::folders::trigger_scan,
            commands::folders::get_watched_folders,
            // analytics (6)
            commands::analytics::get_stats,
            commands::analytics::get_space_graph,
            commands::analytics::get_search_analytics,
            commands::analytics::get_tags,
            commands::analytics::get_activity_feed,
            commands::analytics::get_topics,
            // settings (2)
            commands::settings::get_settings,
            commands::settings::update_settings,
            // entities (6) — KG-01 through KG-04
            commands::entities::get_entities_by_type,
            commands::entities::get_entity,
            commands::entities::get_documents_for_entity,
            commands::entities::get_related_entities,
            commands::entities::rename_entity_canonical,
            commands::entities::split_entity_alias,
            // document text preview (PAGE-13)
            commands::documents::read_document_text,
            // AI provider commands (Phase 7)
            commands::ai::list_providers,
            commands::ai::connect_provider,
            commands::ai::disconnect_provider,
            commands::ai::set_active_provider,
            commands::ai::get_active_provider,
            commands::ai::save_setup_token,
            commands::ai::test_connection,
            commands::ai::chat,
            // OAuth provider commands (Plan 07-09)
            commands::ai::start_openai_oauth,
            // Extraction settings + backfill commands (Phase 8, Plan 05)
            commands::entities::get_extraction_settings,
            commands::entities::set_extraction_settings,
            commands::entities::trigger_entity_backfill,
            // Phase 11 — Saved searches (ENEX-02, ENEX-04)
            crate::saved_searches::commands::get_saved_searches,
            crate::saved_searches::commands::save_search,
            crate::saved_searches::commands::delete_saved_search,
            crate::saved_searches::commands::get_saved_search_counts,
            // Phase 11 — Related docs + entity page (ENEX-01, ENEX-03)
            commands::documents::get_related_docs_scored,
            commands::entities::get_entity_page_data,
            // Phase 11.5 — Ontology / Relation Extraction (ONTO-01..05)
            crate::commands::relations::get_entity_relations,
            crate::commands::relations::get_all_owned_by,
            crate::commands::relations::get_all_related_to,
            crate::commands::relations::get_subjects_by_predicate_object,
            crate::commands::relations::get_objects_by_subject_predicate,
            crate::commands::relations::add_manual_triple,
            crate::commands::relations::delete_triple,
            // Phase 11.7 — RAG Chat (Plan 05)
            crate::chat::commands::start_chat,
            crate::chat::commands::list_chat_sessions,
            crate::chat::commands::delete_chat_session,
            crate::chat::commands::rename_chat_session,
            // Phase 11.6 — Adaptive ontology (Plan 06)
            crate::commands::ontology::get_ontology,
            crate::commands::ontology::apply_consolidation,
            crate::commands::ontology::add_manual_predicate,
            crate::commands::ontology::rename_predicate,
            crate::commands::ontology::merge_predicates,
            crate::commands::ontology::reset_ontology_to_seed,
            crate::commands::ontology::regenerate_corpus_seed,
            crate::commands::ontology::set_automatic_ontology_growth,
            // Phase 11.8 Plan 06 — local-ruvllm model download (D-04)
            commands::ai::download_ruvllm_model,
            // v1.2 #2 — User profile (onboarding "About You")
            commands::profile::get_user_profile,
            commands::profile::save_user_profile,
            // v1.2 #3 — Daily quiz feedback loop
            commands::quiz::get_daily_quiz,
            commands::quiz::submit_quiz_answer,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
