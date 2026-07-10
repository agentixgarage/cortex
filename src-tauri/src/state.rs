use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tokio::sync::Mutex;
use tokio::sync::mpsc;
use crate::auth::AuthState;
use crate::engine::CortexEngine;
use crate::pipeline::embedder::EmbeddingService;
use crate::pipeline::indexer::DocumentIndexer;
use crate::pipeline::two_pass_extractor::TwoPassExtractor;
use crate::graph::edges::DocumentGraph;
use crate::graph::entity_store::EntityStore;
use crate::graph::ontology_store::OntologyStore;
use crate::graph::triple_store::TripleStore;
use crate::intelligence::analytics::{ActivityLog, SearchTracker};
use crate::intelligence::sona_bridge::SearchLearner;
use crate::spaces::hyp_index::{HypIdMapState, HypIndexState};
use crate::chat::session_store::ChatSessionStore;
use crate::types::UserProfile;
use crate::saved_searches::store::SavedSearchStore;
use crate::spaces::label_cache::SpaceLabelCache;
use crate::spaces::manager::SpaceManager;
use crate::watcher::registry::WatcherRegistry;

/// Commands sent to the file watcher background task.
pub enum WatcherCommand {
    /// Start watching a new folder.
    AddFolder { path: String, folder_id: String },
    /// Stop watching a folder and remove it from active watchers.
    RemoveFolder { folder_id: String, path: String },
    /// Pause watching all folders (unwatch without removing config).
    Pause,
    /// Resume watching all non-paused folders.
    Resume,
    /// Shut down the watcher task cleanly.
    Shutdown,
}

/// Events emitted by the indexing pipeline.
pub enum IndexEvent {
    DocumentIndexed { path: String },
    ScanComplete { folder_id: String },
    Error(String),
}

pub struct AppState {
    pub engine: Arc<Mutex<CortexEngine>>,
    /// Send commands to the file watcher.
    pub watcher_tx: mpsc::Sender<WatcherCommand>,
    /// Receive indexing events from background pipeline.
    pub index_rx: Arc<Mutex<mpsc::Receiver<IndexEvent>>>,
    /// Local embedding service (fastembed all-MiniLM-L6-v2, 384-dim).
    pub embedding_service: Arc<EmbeddingService>,
    /// Document indexer orchestrating parse → hash → embed → store.
    pub indexer: Arc<DocumentIndexer>,
    /// Two-pass entity extractor (Pass 1 patterns + Pass 2 LLM refinement, Phase 8).
    /// Registered here so IPC commands (get/set_extraction_settings, trigger_entity_backfill)
    /// can access it via State<'_, AppState>.  Plan 10 replaces ner_service references
    /// throughout the codebase with this field.
    pub two_pass_extractor: Arc<TwoPassExtractor>,
    /// Watched folder registry (persists to JSON).
    pub registry: Arc<std::sync::Mutex<WatcherRegistry>>,
    /// Path to watcher-registry.json on disk.
    pub registry_path: PathBuf,
    /// Smart Spaces manager: clustering, naming, manual moves.
    /// Phase 9: switched to tokio::sync::Mutex — recluster() is now async
    /// (calls LlmSpaceLabeler via ai_request), so a std::sync::MutexGuard
    /// cannot be held across the await.
    pub space_manager: Arc<Mutex<SpaceManager>>,
    /// Document relationship graph for related docs and space network viz.
    pub doc_graph: Arc<std::sync::Mutex<DocumentGraph>>,
    /// SONA self-learning engine for search quality improvement.
    pub search_learner: Arc<std::sync::Mutex<SearchLearner>>,
    /// Search analytics tracker for query history and click-through data.
    pub search_tracker: Arc<std::sync::Mutex<SearchTracker>>,
    /// Activity log for the activity feed (indexed, moved, searched events).
    pub activity_log: Arc<std::sync::Mutex<ActivityLog>>,
    /// Entity knowledge-graph store: canonical entities, alias index, reverse doc index.
    pub entity_store: Arc<std::sync::Mutex<EntityStore>>,
    /// WR-02 single-flight guard: true while a backfill task is running.
    /// compare_exchange(false, true) in trigger_entity_backfill rejects duplicate calls.
    /// The backfill worker stores false on completion.
    pub backfill_running: Arc<AtomicBool>,
    /// Phase 9: Space label cache (JSON sidecar).
    pub space_label_cache: Arc<Mutex<SpaceLabelCache>>,
    /// Phase 9: App data directory (used to persist space_labels.json).
    pub app_data_dir: PathBuf,
    /// Phase 11 Plan 04: Saved-search sidecar store (persists to app_data_dir/saved_searches.json).
    /// Wrapped in tokio::sync::Mutex because save_search + get_saved_search_counts use
    /// spawn_blocking joins; the lock does not need to survive an await, but the tokio Mutex
    /// keeps this consistent with space_label_cache. Loaded in lib.rs setup (ENEX-02, ENEX-04).
    pub saved_search_store: Arc<Mutex<SavedSearchStore>>,
    /// Phase 10 (Plan 06): Secondary hyperbolic HNSW index over top-level Space centroids.
    /// None until first successful rebuild_hyp_index() call (D-11 silent fallback active).
    /// Populated after recluster() returns; consumed by search path when parent_space_id filter present.
    pub hyp_index: HypIndexState,
    /// Phase 10 (Plan 06): Maps HNSW internal usize ids → space_id strings.
    /// Position corresponds to insertion order in rebuild_hyp_index().
    pub hyp_id_to_space: HypIdMapState,
    /// Phase 11.5 (Plan 04): Relation triple store (Pass 3 output), persisted to
    /// app_data_dir/triples.json. Loaded in lib.rs setup; consumed by the backfill
    /// worker (Pass 3 upsert) and the relation IPC commands (Plan 06).
    pub triple_store: Arc<Mutex<TripleStore>>,
    /// Phase 11.7: RAG chat session store.
    pub chat_session_store: Arc<Mutex<ChatSessionStore>>,
    /// Phase 11.6: Adaptive ontology store.
    pub ontology_store: Arc<Mutex<OntologyStore>>,
    /// Phase 11.6: Shared AI credential store for backfill bootstrap.
    pub auth_state: Arc<AuthState>,
    /// v1.2 #2: Optional user profile (name/aliases/family/countries/
    /// currencies) collected in onboarding's "About You" step. Persisted to
    /// app_data_dir/user_profile.json. Consumed by ChatEngine to seed the
    /// RAG system prompt with context documents alone cannot supply.
    pub user_profile: Arc<Mutex<UserProfile>>,
    /// v1.2 #3: Daily quiz feedback log (alias-confirm answers), persisted
    /// to app_data_dir/quiz_feedback.json. Used both to record the "how
    /// well are we doing" measurement and to avoid re-asking the same
    /// question across daily-card refreshes.
    pub quiz_feedback: Arc<Mutex<crate::quiz::store::QuizFeedbackStore>>,
}
