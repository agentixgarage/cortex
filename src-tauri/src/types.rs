use serde::{Deserialize, Serialize};

// === Document types ===

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Document {
    pub id: String,
    pub name: String,
    pub path: String,
    pub doc_type: String,  // "pdf", "docx", "txt", etc.
    pub size: u64,
    pub created_at: String,  // ISO 8601
    pub modified_at: String,
    pub excerpt: Option<String>,
    pub space_ids: Vec<String>,
    pub tags: Vec<String>,  // User/space tags (Cortex-level — NOT LLM extraction tags)
    pub is_favorite: bool,
    pub extracted_entities: Vec<ExtractedEntity>,
    pub thumbnail_color: Option<String>,

    // === Phase 8 Plan 08-08: LLM-extracted doc-level semantic fields ===
    // Both fields use #[serde(default)] so existing Phase 6 metadata (which lacks
    // these keys) still deserializes without error (backward compat).
    // topic: single free-form doc-level topic from Pass 2 LLM (snake_case normalized, D-35).
    // llm_tags: 2-5 free-form keyword tags per doc (snake_case normalized, D-38).
    // Named llm_tags (not tags) to avoid collision with space-level `tags` above.
    #[serde(default)]
    pub topic: Option<String>,
    #[serde(default)]
    pub llm_tags: Vec<String>,
}

// ===== Phase 8 constants: entities_version sentinels =====

/// Pass 1 only (deterministic pattern extraction complete, LLM not yet run).
/// Docs at this version need Pass 2 when a provider is connected.
/// D-02, D-23: backfill picks up docs with entities_version < TWO_PASS_TARGET_VERSION.
pub const PASS1_ONLY_VERSION: f32 = 2.5;

/// Full two-pass extraction complete (Pass 1 + Pass 2 LLM refinement).
/// D-23: target version used by the backfill engine as the completion gate.
pub const TWO_PASS_TARGET_VERSION: f32 = 3.0;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractedEntity {
    pub label: String,
    pub value: String,
    pub entity_type: String,  // "date", "amount", "person", "organization", "location", "email"
    #[serde(default)]
    pub canonical_id: Option<String>,

    // ===== Phase 8 extension fields (D-08, D-09) =====
    // All new fields use #[serde(default)] so existing Phase 6 RuVector metadata
    // (which lacks these keys) still deserializes without error (T-08-02 mitigation).

    /// One of the 8 locked entity classes: Person, Organization, Location, Date,
    /// Amount, Email, Phone, Identifier. None means this entity was extracted by
    /// Pass 1 but class assignment is pending (D-04 weak-format IDs) or the record
    /// is legacy BERT output (entities_version <= 2.0). D-09.
    #[serde(default)]
    pub class: Option<String>,

    /// Free-form subclass string within the class, e.g. "aadhaar", "iban", "pan".
    /// No whitelist — Pass 2 LLM assigns based on context. D-09.
    #[serde(default)]
    pub subclass: Option<String>,

    /// Phase 11.6 (D-09): rule-based canonical short form (e.g. "AlphaCorp
    /// Sattva Complex-Unit-204" -> "Unit 204"). None when the normalizer
    /// produced no rewrite (falls back to `value`).
    #[serde(default)]
    pub canonical_short_name: Option<String>,

    /// Extraction confidence in [0.0, 1.0]. Values below 0.7 indicate potential
    /// OCR corruption and are shown under "Also found" expander in the UI. D-15.
    #[serde(default)]
    pub confidence: Option<f32>,
}

/// Document-level entity extraction result wrapping the per-entity Vec.
/// Carries topic (single emergent label) and tags (free-form multi-label) produced
/// by Pass 2 LLM refinement. Both fields are normalized to snake_case at write
/// time via `normalize_tag()` (D-35).
///
/// `entities_version` semantics:
///   2.0  — legacy BERT (NerService, absent after Phase 8)
///   2.5  — PASS1_ONLY_VERSION: deterministic patterns only, Pass 2 not yet run
///   3.0  — TWO_PASS_TARGET_VERSION: Pass 1 + Pass 2 complete
///
/// D-21: stored as JSON in RuVector VectorEntry metadata alongside the embedding.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractedEntities {
    /// Individual entity instances extracted from this document.
    pub entities: Vec<ExtractedEntity>,

    /// Single emergent topic label (normalized snake_case). Examples: "finance",
    /// "identity", "vehicle". None when Pass 2 has not yet run (PASS1_ONLY_VERSION).
    /// D-36: LLM chooses from a 19-topic seed but may invent new topics.
    #[serde(default)]
    pub topic: Option<String>,

    /// 2-5 free-form tags per doc (normalized snake_case). Empty until Pass 2 runs.
    /// D-38: no tag whitelist; UI renders tag cloud in /tags sorted by count.
    #[serde(default)]
    pub tags: Vec<String>,

    /// Version sentinel. Defaults to 2.0 (legacy BERT assumption) when the field is
    /// missing from stored JSON — allows forward-reading old metadata without panicking.
    /// D-02, D-23.
    #[serde(default = "default_entities_version")]
    pub entities_version: f32,

    /// Optional BCP-47 language code detected by Pass 2 (e.g. "en", "hi"). Soft
    /// signal for Phase 9 space labeling — not used for filtering in v1.1.
    #[serde(default)]
    pub language: Option<String>,
}

/// Serde default for `entities_version`: missing field → 2.0 (legacy BERT assumption).
fn default_entities_version() -> f32 {
    2.0
}

impl ExtractedEntities {
    /// Constructor for Pass-1-only results. Sets entities_version = PASS1_ONLY_VERSION (2.5),
    /// topic = None, tags = empty vec. Pass 2 upgrades the container to version 3.0
    /// when an active provider is available. D-02, D-26.
    pub fn pass1_only(entities: Vec<ExtractedEntity>) -> Self {
        Self {
            entities,
            topic: None,
            tags: Vec::new(),
            entities_version: PASS1_ONLY_VERSION,
            language: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentMeta {
    pub id: String,
    pub name: String,
    pub path: String,
    pub doc_type: String,
    pub size: u64,
    pub created_at: String,  // ISO 8601
    pub modified_at: String,
}

/// Entity class+value filter pair. Produced by splitting URL param `?entity={class}:{value}`
/// on the first ':' (e.g. "Person:Alex Doe" → class="Person", value="Alex Doe").
/// URL-decoding happens in the frontend before the value reaches Rust (11-RESEARCH.md pitfall #3).
/// Consumed by Plan 03 `apply_entity_class_filters` in search/filters.rs.
/// D-01, D-03 from 11-CONTEXT.md.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntityClassFilter {
    pub class: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchFilters {
    pub doc_type: Option<String>,
    pub space_id: Option<String>,
    pub date_from: Option<String>,
    pub date_to: Option<String>,
    pub tags: Option<Vec<String>>,
    // pitfall #2 (11-RESEARCH.md): MUST be Option<Vec<...>> not bare Vec<...>.
    // Pre-Phase-11 frontend callers omit this field entirely — a bare Vec would
    // fail deserialization (missing field). Option + #[serde(default)] yields None.
    #[serde(default)]
    pub entity_filters: Option<Vec<EntityClassFilter>>,
}

/// Persisted filters for a SavedSearch. All fields are optional/defaulted so
/// a saved search with only entity filters (no topic/date) still deserializes.
/// entities: "{class}:{value}" strings per D-06 shape; matches URL param format.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedSearchFilters {
    #[serde(default)]
    pub entities: Vec<String>,     // e.g. ["Person:Alex Doe", "Location:AlphaComplex"]
    #[serde(default)]
    pub topic: Option<String>,
    #[serde(default)]
    pub doc_type: Option<String>,
    #[serde(default)]
    pub space_id: Option<String>,
    #[serde(default)]
    pub date_from: Option<String>,
    #[serde(default)]
    pub date_to: Option<String>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
}

/// A saved virtual search Space persisted to `app_data_dir/saved_searches.json`.
/// id format: "ss-{uuid}" (enforced by Plan 04 `save_search`).
/// doc_count_cache is a render hint; live count re-evaluates on Sidebar mount (D-08, ENEX-04).
/// D-05, D-06 from 11-CONTEXT.md.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedSearch {
    pub id: String,              // "ss-{uuid}"
    pub name: String,
    pub query: String,
    pub filters: SavedSearchFilters,
    pub created_at: String,      // ISO 8601
    pub doc_count_cache: u32,    // hint for immediate render; stale after new docs indexed
}

/// A document similar to a given document, scored by composite relevance.
/// score = 0.6 × cosine + 0.4 × entity_overlap_jaccard (D-10, D-11).
/// snippet: text excerpt around entity-overlap region — UI §5 uses this;
/// when absent the panel renders title + badge only.
/// Pattern 3 from 11-RESEARCH.md.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelatedDocScored {
    pub document: Document,
    pub score: f64,              // composite (0.6*cosine + 0.4*jaccard)
    pub cosine_score: f64,
    pub jaccard_score: f64,
    pub snippet: Option<String>, // text excerpt around entity-overlap region
}

/// Class+value keyed reference to a co-occurring entity, used in EntityPageData.
/// Distinct from RelatedEntity (which keys by canonical_id) — RelatedEntityRef
/// uses the URL-format class:value pair (D-15, D-17 from 11-CONTEXT.md).
/// Do NOT confuse with RelatedEntity (Plan 06, canonical_id keyed).
/// Pattern 4 from 11-RESEARCH.md.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelatedEntityRef {
    pub class: String,
    pub value: String,
    pub co_doc_count: u32,  // number of documents mentioning both entities
}

/// Full data payload for the `/entity/:class/:value` detail page.
/// canonical: CanonicalEntity with aliases from Phase 6 entity_store alias index.
/// documents: paginated documents mentioning this entity (20/page per D-16).
/// co_occurring_entities: top-10 class+value pairs from Phase 6 co-occurrence.
/// Pattern 4 from 11-RESEARCH.md; D-15, D-16 from 11-CONTEXT.md.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntityPageData {
    pub canonical: CanonicalEntity,
    pub documents: Vec<Document>,
    pub total_document_count: u32,
    pub co_occurring_entities: Vec<RelatedEntityRef>,
    pub page: u32,
    pub page_size: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub document: Document,
    pub score: f64,
    pub matched_excerpt: Option<String>,
}

// === Space types ===

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Space {
    pub id: String,
    pub name: String,
    pub icon: String,
    pub color: String,
    pub document_count: u32,
    pub last_updated: String,
    pub sub_spaces: Vec<Space>,
    pub parent_id: Option<String>,
    pub sample_files: Vec<String>,

    // === Phase 10 Plan 01: Hierarchical Space fields (D-07) ===
    // Both fields use #[serde(default)] so pre-Phase-10 serialized Space values
    // (e.g. from SpaceManager state or IPC responses lacking these keys) still
    // deserialize cleanly.  T-10-01 mitigation — pitfall #4 in 10-RESEARCH.md.

    /// Hierarchy depth: 0 = top-level, 1 = sub-space (D-07, D-03).
    /// Max depth is 2 (Parent → Sub), gated in SpaceManager::recluster (Plan 10-05).
    #[serde(default)]
    pub depth: u8,

    /// IDs of direct sub-spaces, populated by SpaceManager::recluster (Plan 10-05)
    /// for top-level spaces with > 50 docs (D-07, D-01).  Empty for sub-spaces
    /// (depth=1) and top-level spaces below the threshold.
    /// Serializes to `subSpaceIds` per the struct's `serde(rename_all = "camelCase")`.
    #[serde(default)]
    pub sub_space_ids: Vec<String>,

    // === Phase 9 Plan 01: LLM Space labeling fields ===
    // All use #[serde(default)] so pre-Phase-9 serialized Space values (e.g. from
    // cached SpaceManager state lacking these keys) still deserialize cleanly.
    // T-09-01 mitigation — pitfall #3 in 09-RESEARCH.md.

    /// Human-readable description of the space, LLM-generated (D-02).
    /// Surfaces as tooltip on SpaceCard hover and full text on /spaces/:id (D-16).
    #[serde(default)]
    pub description: Option<String>,

    /// When true, the user has manually renamed this space — LLM re-labels will skip it (D-15).
    /// Reset to false via "Clear override" button on the space detail page.
    #[serde(default)]
    pub user_locked: bool,

    /// Highest-count entity across the space's documents, formatted as "{Class}: {value}"
    /// e.g. "Person: Alex Doe", "Property: AlphaComplex". None when no entity dominates
    /// (top count < 20% of doc count). Feeds Phase 11 entity navigation (D-17, D-18).
    #[serde(default)]
    pub canonical_entity_hint: Option<String>,

    /// Label generation status. Values: "ready" | "generating". Stored as String today
    /// so a future migration can widen to a proper enum without breaking serde (D-14).
    /// None = not yet set (treat as "ready" on the frontend — 09-UI-SPEC §"Data Type Extensions").
    #[serde(default)]
    pub label_status: Option<String>,
}

// === Folder types ===

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WatchedFolder {
    pub id: String,
    pub path: String,
    pub document_count: u32,
    pub last_scan: String,
    pub status: String,  // "watching", "paused", "error"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanProgress {
    pub folder_id: String,
    pub total_files: u32,
    pub processed_files: u32,
    pub status: String,  // "scanning", "complete", "error"
}

// === Analytics types ===

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub total_documents: u32,
    pub smart_spaces: u32,
    pub last_scan: String,
    pub index_size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpaceGraphNode {
    pub id: String,
    pub name: String,
    pub color: String,
    pub document_count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpaceGraphEdge {
    pub source: String,
    pub target: String,
    pub weight: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SpaceGraph {
    pub nodes: Vec<SpaceGraphNode>,
    pub edges: Vec<SpaceGraphEdge>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TopQuery {
    pub query: String,
    pub count: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchAnalytics {
    pub total_searches: u32,
    pub top_queries: Vec<TopQuery>,
    pub avg_results_per_query: f64,
    pub queries_this_week: u32,
}

// === Settings types ===

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub theme: String,              // "dark", "light", "system"
    pub sidebar_collapsed: bool,
    pub embedding_model: String,    // "local", "openai"
    pub watched_folders: Vec<String>,
    pub excluded_patterns: Vec<String>,
    pub index_on_startup: bool,
    pub index_size: u64,            // Bytes -- visible in Settings > Storage
    pub storage_path: String,       // Path to RuVector data dir -- visible in Settings > Storage

    // ===== Phase 8 extension fields (D-33) =====
    // Both fields use #[serde(default)] so existing settings.json (Phase 5 vintage)
    // without these keys still loads cleanly (T-08-01 mitigation).

    /// LLM model identifier used for Pass 2 entity extraction.
    /// Empty string ("") means "use the provider default at request time" per D-11:
    ///   Anthropic → claude-haiku-4-5-20251001
    ///   OpenAI-Codex → gpt-5-mini
    ///   Gemini → gemini-2.5-flash
    ///   Ollama → user's connected model
    /// D-33, D-11.
    #[serde(default)]
    pub extraction_model: String,

    /// When true, Pass 2 LLM refinement runs on every indexed doc (if an active
    /// provider is connected). When false, only Pass 1 (deterministic patterns) runs.
    /// Privacy-strict users set this to false to ensure zero doc content reaches an LLM.
    /// Default is false here (safe default) — default_settings() in commands/settings.rs
    /// sets it to true when appropriate (D-33). #[serde(default)] gives false for missing
    /// keys (matching Rust's bool default).
    #[serde(default)]
    pub use_llm_extraction: bool,
}

// === normalize_tag helper (D-35) ===

/// Normalize a free-form topic or tag string to snake_case per D-35.
///
/// Algorithm (applied in order):
/// 1. Trim leading/trailing whitespace
/// 2. Lowercase
/// 3. Replace runs of whitespace or dashes with a single '_'
/// 4. Strip every character that is NOT alphanumeric (ASCII) or '_'
/// 5. Collapse consecutive '_' to a single '_'
/// 6. Strip leading and trailing '_'
///
/// Examples:
///   "Term Insurance"     → "term_insurance"
///   "term-insurance"     → "term_insurance"   (WR-08: dashes → underscore)
///   "self-employed"      → "self_employed"
///   "Investments 2024!"  → "investments_2024"
///   "  khush   school  " → "khush_school"
///   "already_snake"      → "already_snake"
///   "café"               → "caf"
///   "-"                  → ""
pub fn normalize_tag(input: &str) -> String {
    // Step 1 + 2: trim whitespace, then lowercase
    let lowered = input.trim().to_lowercase();

    // Step 3: replace each run of ASCII whitespace OR dashes with a single '_'.
    // WR-08 fix: dashes were previously dropped silently, so "term-insurance" became
    // "terminsurance" rather than "term_insurance". LLM-generated tags commonly use
    // hyphen separators ("self-employed", "co-owner"). Treating '-' like whitespace
    // (→ '_') keeps these consistent with space-separated forms.
    let mut result = String::with_capacity(lowered.len());
    let mut in_underscore = false;
    for ch in lowered.chars() {
        if ch.is_ascii_whitespace() || ch == '-' {
            if !in_underscore {
                result.push('_');
                in_underscore = true;
            }
        } else {
            in_underscore = false;
            // Step 4: only keep ASCII alphanumeric or '_'
            if ch.is_ascii_alphanumeric() || ch == '_' {
                result.push(ch);
            }
            // non-ASCII and non-alphanumeric chars (e.g. accents, '!') are dropped
        }
    }

    // Step 5: collapse consecutive '_' (from whitespace-to-underscore + adjacent underscores)
    let mut collapsed = String::with_capacity(result.len());
    let mut prev_underscore = false;
    for ch in result.chars() {
        if ch == '_' {
            if !prev_underscore {
                collapsed.push('_');
                prev_underscore = true;
            }
        } else {
            prev_underscore = false;
            collapsed.push(ch);
        }
    }

    // Step 6: strip leading and trailing '_'
    collapsed.trim_matches('_').to_string()
}

// === Tag types ===

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tag {
    pub id: String,
    pub name: String,
    pub color: String,
    pub document_count: u32,
    pub tag_type: String,  // "auto", "user"
}

// === Entity graph types (Plan 06-02) ===

/// A canonical entity node in the entity graph.
/// Canonical entities are created by NER + alias merging (D-05..D-07).
/// Plan 03 EntityStore populates canonical_id on ExtractedEntity instances.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CanonicalEntity {
    pub id: String,
    pub canonical_name: String,
    pub entity_type: String,
    pub aliases: Vec<String>,
    pub document_count: u32,
    /// Phase 11.6 (D-11): most-frequent canonical_short_name across aliases;
    /// falls back to canonical_name when None.
    #[serde(default)]
    pub canonical_short_name: Option<String>,
}

/// Lightweight summary of a canonical entity for list/grid views.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntitySummary {
    pub id: String,
    pub canonical_name: String,
    pub entity_type: String,
    pub document_count: u32,
}

/// An entity related to a queried entity by co-occurrence (D-11).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelatedEntity {
    pub entity: EntitySummary,
    pub co_occurrence_count: u32,
}

/// Text preview of a document (used by in-app preview, D-16).
/// If the file exceeds the size limit (D-15), text is None and truncated is true.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentTextPreview {
    pub text: Option<String>,
    pub truncated: bool,
    pub size: u64,
}

/// Progress event for the entity backfill background task (D-03, D-25, D-29).
/// Emitted as Tauri event "entity-backfill-progress" every 25 docs or 500ms.
///
/// `eta_seconds` and `fallbacks` use `#[serde(default)]` so existing frontend
/// consumers (BackfillIndicator, Plan 04) that lack these fields still deserialize
/// without error — TypeScript optional fields tolerate absent JSON keys (D-29).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntityBackfillProgress {
    pub processed: u32,
    pub total: u32,
    pub status: String,          // "running" | "complete" | "error"
    pub error: Option<String>,
    /// Estimated seconds to completion — rolling average of last 20 per-doc latencies
    /// multiplied by remaining docs (D-25). None before the first doc completes or at
    /// "complete" status.
    #[serde(default)]
    pub eta_seconds: Option<u32>,
    /// Count of docs that fell back to Pass-1-only (entities_version=2.5) because
    /// Pass 2 LLM was expected but either the provider was absent or Pass 2 errored.
    /// Docs where the user has explicitly disabled LLM are NOT counted as fallbacks.
    /// Used by the Plan 07 sonner toast (D-29).
    #[serde(default)]
    pub fallbacks: u32,
}

// === Extraction settings (Phase 8, Plan 05) ===

/// Runtime view of LLM extraction settings returned / accepted by the three
/// extraction IPC commands.  Mirrors `Settings.extraction_model` and
/// `Settings.use_llm_extraction` but is a separate lightweight struct so the
/// commands can read/write just these two fields without touching the full
/// Settings blob.
///
/// JSON keys are camelCase to match the existing Settings surface on the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtractionSettings {
    /// LLM model identifier for Pass 2 extraction.
    /// Empty string = "use provider default".  See D-11.
    pub extraction_model: String,
    /// When true, Pass 2 LLM refinement is active (requires connected provider).
    pub use_llm_extraction: bool,
}

// === Topic types (Phase 8 Plan 09) ===

/// Aggregate count of documents assigned a given topic label.
/// Returned by `get_topics` IPC; used by TopicFilterBar for top-N display.
/// topic is snake_case (normalize_tag output); count is document occurrences.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TopicCount {
    pub topic: String,
    pub count: u32,
}

// === Activity types ===

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityItem {
    pub id: String,
    pub action: String,    // "indexed", "moved", "tagged", "searched"
    pub subject: String,
    pub timestamp: String,
    #[serde(rename = "type")]
    pub activity_type: String,  // "info", "success", "warning", "error"
    pub document_id: Option<String>,
}

// === Phase 11.5: Ontology / Relation Extraction types ===

/// Pass 1 + Pass 2 + Pass 3 relation extraction complete. Docs at
/// `TWO_PASS_TARGET_VERSION` (3.0) are eligible for Pass 3 upgrade (D-21, D-22).
pub const PASS3_TARGET_VERSION: f32 = 3.5;

/// Closed predicate vocabulary (v1) enforced by Pass3RelationExtractor. The LLM
/// cannot invent new predicates (D-02) — `mentioned_with` is the weak default
/// used when no other predicate fits (D-01).
pub const PREDICATE_VOCABULARY: &[&str] = &[
    "owns",
    "owned_by",
    "purchased_from",
    "sold_to",
    "located_in",
    "part_of",
    "address_of",
    "married_to",
    "parent_of",
    "child_of",
    "member_of",
    "employer_of",
    "employee_of",
    "partner_of",
    "issued_by",
    "dated",
    "signed_by",
    "uses_pan",
    "uses_aadhaar",
    "has_voter_id",
    "mentioned_with",
];

/// Returns true when `p` is a member of the closed `PREDICATE_VOCABULARY` set.
pub fn is_valid_predicate(p: &str) -> bool {
    PREDICATE_VOCABULARY.contains(&p)
}

/// Directional predicate pairs that MUST be auto-inverted at TripleStore write
/// time (D-03): storing `(A, pair.0, B)` also stores `(B, pair.1, A)`.
/// TripleStore consults this on `upsert_from_doc` to insert the inverse triple
/// automatically. Symmetric pairs (`married_to`, `partner_of`) are self-inverse
/// and handled at store level via `SYMMETRIC_PREDICATES`, not via this table.
pub const AUTO_INVERSE_PAIRS: &[(&str, &str)] = &[
    ("owns", "owned_by"),
    ("owned_by", "owns"),
    ("purchased_from", "sold_to"),
    ("sold_to", "purchased_from"),
    ("employer_of", "employee_of"),
    ("employee_of", "employer_of"),
    ("parent_of", "child_of"),
    ("child_of", "parent_of"),
];

/// Self-inverse predicates: storing `(A, p, B)` also stores `(B, p, A)`.
pub const SYMMETRIC_PREDICATES: &[&str] = &["married_to", "partner_of", "mentioned_with"];

/// A single (subject, predicate, object) relation extracted by Pass 3 (or added
/// manually via `add_manual_triple`). `id` is a stable UUID assigned by
/// TripleStore on insert ("t-{uuid}"). `doc_ids` is append-only provenance
/// (D-11). `user_added` distinguishes manual overrides from LLM-extracted
/// triples and is preserved across LLM re-runs (D-12).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Triple {
    pub id: String,
    pub subject_id: String,
    pub predicate: String,
    pub object_id: String,
    #[serde(default)]
    pub doc_ids: Vec<String>,
    pub user_added: bool,
    pub created_at: String,
}

/// Expanded view of a Triple with its subject and object CanonicalEntity
/// records resolved, for direct frontend consumption without a second lookup.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TripleWithEntities {
    pub triple: Triple,
    pub subject: CanonicalEntity,
    pub object: CanonicalEntity,
}

/// Full data payload for the entity detail page "Relations" panel.
/// `outgoing`: triples where `entity` is the subject. `incoming`: triples
/// where `entity` is the object.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RelationsPageData {
    pub entity: CanonicalEntity,
    #[serde(default)]
    pub outgoing: Vec<TripleWithEntities>,
    #[serde(default)]
    pub incoming: Vec<TripleWithEntities>,
}

/// Asset category derived from the object entity's class + topic (D-14, D-18).
/// Serializes as PascalCase strings ("Property", "Vehicle", ...) to match the
/// TS `AssetType` string union.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "PascalCase")]
pub enum AssetType {
    Property,
    Vehicle,
    Investment,
    Business,
    Financial,
    Other,
}

/// Full data payload for the `/ownership/:person_id` page. Assets are grouped
/// by `AssetType` per D-14 so the UI can render Property / Vehicle /
/// Investment / Business / Financial sections.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OwnershipPageData {
    pub person: CanonicalEntity,
    #[serde(default)]
    pub assets_by_type: std::collections::HashMap<AssetType, Vec<TripleWithEntities>>,
    pub total_assets: u32,
}

/// Payload entry for `get_subjects_by_predicate_object` — "who --predicate--> object?".
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PredicateSubjectPair {
    pub subject: CanonicalEntity,
    pub triple: Triple,
}

/// Payload entry for `get_objects_by_subject_predicate` — "subject --predicate--> who?".
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PredicateObjectPair {
    pub object: CanonicalEntity,
    pub triple: Triple,
}

// === Phase 11.6: Adaptive Ontology types ===

/// Fixed baseline vocabulary preserved from Phase 11.5. Adaptive vocab merges
/// SEED_PREDICATES + corpus_seed + adaptive_predicates on read (D-03). This is
/// a compile-time alias, not a copy — `PREDICATE_VOCABULARY` itself is never
/// mutated; the baked-in 21-predicate list survives a user "reset to seed".
pub const SEED_PREDICATES: &[&str] = PREDICATE_VOCABULARY;

/// Vocabulary hard cap (D-08): when the total adaptive + pending + manual
/// predicate count hits this, consolidation must run before more can be added.
pub const VOCABULARY_HARD_CAP: usize = 200;

/// Minimum distinct-document support before a pending predicate is promoted
/// to `adaptive_predicates` (D-06).
pub const PENDING_PROMOTION_MIN_SUPPORT: u32 = 2;

/// Corpus-seeded bootstrap fires once the first-backfill batch reaches this
/// many docs (D-01, D-04).
pub const BOOTSTRAP_MIN_DOCS: u32 = 30;

/// Consolidation loop trigger: fires after this many new triples upserted
/// since the last consolidation run (D-15).
pub const CONSOLIDATION_TRIGGER_TRIPLES: u32 = 500;

/// Consolidation loop trigger: fires after this many hours since the last
/// consolidation run, whichever comes first (D-15).
pub const CONSOLIDATION_TRIGGER_HOURS: u64 = 24;

/// Provenance of a predicate or entity subclass entry in the ontology store.
/// `Seed` = baked-in Phase 11.5 vocabulary, `Corpus` = corpus-seeded bootstrap
/// output (D-01/D-02), `Adaptive` = promoted from Pass 3 `new_predicates`
/// (D-06/D-07), `Manual` = user-added via Settings > Ontology (D-21).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PromotionSource {
    Seed,
    Corpus,
    Adaptive,
    Manual,
}

/// A single predicate entry in the adaptive ontology store (D-18). Distinct
/// from the plain `&str` entries in `PREDICATE_VOCABULARY` — this is the
/// record shape persisted to `ontology.json` with provenance + support count.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Predicate {
    pub name: String,
    pub description: String,
    pub source: PromotionSource,
    #[serde(default)]
    pub count: u32,
    #[serde(default)]
    pub first_seen_doc_id: Option<String>,
    /// RFC3339 timestamp.
    #[serde(default)]
    pub first_seen_at: Option<String>,
    #[serde(default)]
    pub promoted_at: Option<String>,
    #[serde(default)]
    pub subject_class: Option<String>,
    #[serde(default)]
    pub object_class: Option<String>,
}

/// A single entity subclass entry in the adaptive ontology store (D-18), e.g.
/// class="Location", subclass="apartment".
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EntitySubclass {
    pub class: String,
    pub subclass: String,
    pub source: PromotionSource,
    #[serde(default)]
    pub count: u32,
    #[serde(default)]
    pub first_seen_doc_id: Option<String>,
    #[serde(default)]
    pub example_value: Option<String>,
}

/// Output shape of the corpus-seeded bootstrap LLM call (D-01, D-02): a batch
/// of proposed predicates + entity subclasses derived from ~30-50 sample docs.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BootstrapSeed {
    #[serde(default)]
    pub predicates: Vec<Predicate>,
    #[serde(default)]
    pub entity_subclasses: Vec<EntitySubclass>,
    pub generated_at: String,
    #[serde(default)]
    pub sample_doc_count: u32,
    #[serde(default)]
    pub model_used: String,
}

/// A single proposed ontology change from the consolidation loop (D-16).
/// Tagged union so the frontend can discriminate on `kind` without a
/// separate variant-detection field.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum ConsolidationKind {
    Merge { from: Vec<String>, into: String },
    Rename { from: String, to: String },
    Split { from: String, into: Vec<String> },
}

/// A single consolidation suggestion awaiting user approval (D-16, D-17).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ConsolidationSuggestion {
    /// Stable UUID, distinct per suggestion.
    pub id: String,
    pub kind: ConsolidationKind,
    pub rationale: String,
    pub confidence: f32,
}

/// Batch of consolidation suggestions generated by a single consolidation
/// loop run (D-15..D-17). Persisted as `pending_consolidation` in
/// `OntologyStoreSchema`; cleared on user accept/reject (D-20).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingConsolidation {
    #[serde(default)]
    pub suggestions: Vec<ConsolidationSuggestion>,
    pub generated_at: String,
    #[serde(default)]
    pub model_used: String,
    #[serde(default)]
    pub triple_count_at_generation: u32,
}

/// Default `version` for `OntologyStoreSchema` when the field is absent from
/// legacy/partial JSON (T-11.6-01 mitigation, mirrors Phase 8 Pitfall 5).
fn default_ontology_version() -> u32 {
    1
}

/// On-disk JSON schema for `app_data_dir/ontology.json` (D-18, D-19). Mirrors
/// the `SpaceLabelCache` / `SavedSearchStore` / `TripleStore` JSON-sidecar
/// pattern — loaded once at boot into AppState. All fields use
/// `#[serde(default)]` so a corrupt or partial file still deserializes into
/// safe defaults instead of panicking (T-11.6-01).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OntologyStoreSchema {
    #[serde(default = "default_ontology_version")]
    pub version: u32,
    #[serde(default)]
    pub corpus_seed: Option<BootstrapSeed>,
    #[serde(default)]
    pub adaptive_predicates: Vec<Predicate>,
    #[serde(default)]
    pub pending_predicates: Vec<Predicate>,
    #[serde(default)]
    pub manual_predicates: Vec<Predicate>,
    #[serde(default)]
    pub entity_subclasses: Vec<EntitySubclass>,
    #[serde(default)]
    pub pending_consolidation: Option<PendingConsolidation>,
    /// Opt-in per D-21; defaults to false for privacy-strict users.
    #[serde(default)]
    pub automatic_growth_enabled: bool,
    #[serde(default)]
    pub bootstrap_completed_at: Option<String>,
    #[serde(default)]
    pub last_consolidation_at: Option<String>,
    #[serde(default)]
    pub triples_since_last_consolidation: u32,
}

impl Default for OntologyStoreSchema {
    fn default() -> Self {
        Self {
            version: default_ontology_version(),
            corpus_seed: None,
            adaptive_predicates: Vec::new(),
            pending_predicates: Vec::new(),
            manual_predicates: Vec::new(),
            entity_subclasses: Vec::new(),
            pending_consolidation: None,
            automatic_growth_enabled: false,
            bootstrap_completed_at: None,
            last_consolidation_at: None,
            triples_since_last_consolidation: 0,
        }
    }
}

/// Result of attempting to promote a pending predicate (D-06, D-08).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum PromoteResult {
    Promoted,
    StillPending { count: u32 },
    CapExceeded,
    AlreadyPresent,
}

// === Phase 11.7: Chat with Your Docs (RAG) types ===
//
// NOTE: These are the Plan 01 (11.7-01-PLAN.md) type contracts, added here as a
// Rule 3 (auto-fix blocking issue) deviation by Plan 02's executor: Plan 02
// depends on these types compiling (ChatSessionStore uses ChatSession /
// ChatMessage) but Plan 01 had not yet been executed. Only the Rust-side
// types needed for Plan 01 Task 1 are added; TS interfaces (Plan 01 Task 2)
// and queryKeys (Plan 01 Task 3) are out of scope here and remain for
// Plan 01's own execution pass (idempotent — Plan 01 should find these
// already present and skip Task 1).

/// Role of a chat message sender. Matches TS `ChatRole = "user" | "assistant"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChatRole {
    User,
    Assistant,
}

/// Citation attached to an assistant ChatMessage — points back to the source
/// document chunk that grounded part of the response (D-17 highlight target).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Citation {
    pub index: u32, // 1-based, matches "[1]" markers in streamed text
    pub doc_id: String,
    pub doc_title: String,
    pub chunk_start: u32,
    pub chunk_end: u32,
}

/// A single message within a ChatSession. `citations` is `None` for user
/// messages, `Some` (possibly empty) for assistant messages.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    pub id: String, // "mid-{uuid}"
    pub role: ChatRole,
    pub content: String,
    pub citations: Option<Vec<Citation>>,
    pub created_at: String, // RFC3339
}

/// A persisted chat session — one entry in `app_data_dir/chat_sessions.json`
/// (ChatSessionStore, Plan 02). Title is auto-generated from the first user
/// message per D-13; user can rename via `rename_session`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatSession {
    pub id: String, // "cs-{uuid}"
    pub title: String,
    pub created_at: String,
    pub updated_at: String,
    pub messages: Vec<ChatMessage>,
}

/// Event payload for the `chat-stream-token` Tauri event (D-10).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatStreamTokenPayload {
    pub session_id: String,
    pub message_id: String,
    pub token: String,
    pub cumulative_index: u32,
}

/// Event payload for the `chat-stream-complete` Tauri event (D-10).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatStreamCompletePayload {
    pub session_id: String,
    pub message_id: String,
    pub citations: Vec<Citation>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}

/// Event payload for the `chat-stream-error` Tauri event (D-10).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatStreamErrorPayload {
    pub session_id: String,
    pub message_id: String,
    pub error: String,
}

/// Event payload for the `chat-stream-suggestions` Tauri event (v1.2 #1:
/// chat follow-up suggestions). Emitted after `chat-stream-complete`, as a
/// separate lightweight follow-up so the main answer is never blocked on
/// suggestion generation. `suggestions` is 0-3 short follow-up questions
/// derived deterministically from the just-completed turn (no extra LLM
/// call — see `ChatEngine::generate_suggestions`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatStreamSuggestionsPayload {
    pub session_id: String,
    pub message_id: String,
    pub suggestions: Vec<String>,
}

/// IPC request shape for the `start_chat` command (Plan 05).
/// `session_id: None` starts a new session (D-15); `filters` carries D-04
/// filter passthrough from the current search view, reusing `SearchFilters`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartChatArgs {
    pub query: String,
    pub session_id: Option<String>,
    pub filters: Option<SearchFilters>,
}

// -------------------------------------------------------------------------
// v1.2 #2: User profile (onboarding "About You" step)
// Optional, user-entered context that seeds retrieval + RAG chat with
// knowledge Cortex cannot otherwise infer from documents alone (the user's
// own name/aliases, family relationships, countries of residence/asset
// ownership, preferred currencies). Entirely local, never leaves the
// device except as part of prompts sent to whichever AI provider the user
// has configured (same trust boundary as document content already sent
// during Chat/Pass 2/Pass 3).
// -------------------------------------------------------------------------

/// One family member entry — free-form relation label (e.g. "spouse",
/// "child", "parent", "sibling") so this works across cultures/family
/// structures without a closed enum.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FamilyMember {
    pub name: String,
    pub relation: String,
}

/// Persisted user profile — sidecar JSON at `app_data_dir/user_profile.json`.
/// Entirely optional; every field defaults to empty. Collected via the
/// onboarding "About You" step (skippable) and editable later in Settings.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct UserProfile {
    /// The user's own preferred display name.
    pub display_name: String,
    /// Alternate names/spellings the user is known by across documents
    /// (e.g. legal name vs. commonly-used first name).
    pub aliases: Vec<String>,
    pub family_members: Vec<FamilyMember>,
    /// ISO-ish free-form country names/codes the user has documents/assets in.
    pub countries: Vec<String>,
    /// Currency symbols or ISO codes the user's documents commonly use.
    pub currencies: Vec<String>,
}

/// Provider slug for the local ruvllm (Metal-accelerated on-device) inference backend (D-03).
///
/// NOTE: This constant was originally scoped to Plan 11.8-04, which had not yet executed
/// when Plan 11.8-06 ran. Added here directly (Rule 3 — blocking-issue auto-fix) since it
/// is a minimal, single-line, non-architectural contract addition that 11.8-06 depends on.
/// `normalize_provider_name` falls through unknown slugs unchanged (see ai/service.rs),
/// so this constant requires no change to normalization logic.
pub const LOCAL_RUVLLM_PROVIDER: &str = "local-ruvllm";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_canonical_entity_serde_roundtrip() {
        let entity = CanonicalEntity {
            id: "ent-1".to_string(),
            canonical_name: "Acme Corp".to_string(),
            entity_type: "organization".to_string(),
            aliases: vec!["Acme".to_string(), "ACME Corporation".to_string()],
            document_count: 5,
            canonical_short_name: None,
        };
        let json = serde_json::to_string(&entity).expect("serialize CanonicalEntity");
        let decoded: CanonicalEntity = serde_json::from_str(&json).expect("deserialize CanonicalEntity");
        assert_eq!(decoded.id, entity.id);
        assert_eq!(decoded.canonical_name, entity.canonical_name);
        assert_eq!(decoded.aliases.len(), 2);
        // Verify camelCase serialization
        assert!(json.contains("canonicalName"), "expected camelCase 'canonicalName' in: {}", json);
    }

    #[test]
    fn test_entity_summary_serde_roundtrip() {
        let summary = EntitySummary {
            id: "ent-2".to_string(),
            canonical_name: "John Smith".to_string(),
            entity_type: "person".to_string(),
            document_count: 3,
        };
        let json = serde_json::to_string(&summary).expect("serialize EntitySummary");
        let decoded: EntitySummary = serde_json::from_str(&json).expect("deserialize EntitySummary");
        assert_eq!(decoded.id, summary.id);
        assert_eq!(decoded.document_count, 3);
    }

    #[test]
    fn test_related_entity_serde_roundtrip() {
        let related = RelatedEntity {
            entity: EntitySummary {
                id: "ent-3".to_string(),
                canonical_name: "Brooklyn".to_string(),
                entity_type: "location".to_string(),
                document_count: 2,
            },
            co_occurrence_count: 7,
        };
        let json = serde_json::to_string(&related).expect("serialize RelatedEntity");
        let decoded: RelatedEntity = serde_json::from_str(&json).expect("deserialize RelatedEntity");
        assert_eq!(decoded.co_occurrence_count, 7);
        assert_eq!(decoded.entity.canonical_name, "Brooklyn");
        assert!(json.contains("coOccurrenceCount"), "expected camelCase 'coOccurrenceCount' in: {}", json);
    }

    #[test]
    fn test_document_text_preview_serde_roundtrip() {
        let preview = DocumentTextPreview {
            text: Some("hello world".to_string()),
            truncated: false,
            size: 1024,
        };
        let json = serde_json::to_string(&preview).expect("serialize DocumentTextPreview");
        let decoded: DocumentTextPreview = serde_json::from_str(&json).expect("deserialize DocumentTextPreview");
        assert_eq!(decoded.text, Some("hello world".to_string()));
        assert!(!decoded.truncated);
    }

    #[test]
    fn test_entity_backfill_progress_serde_roundtrip() {
        let progress = EntityBackfillProgress {
            processed: 42,
            total: 100,
            status: "running".to_string(),
            error: None,
            eta_seconds: Some(15),
            fallbacks: 3,
        };
        let json = serde_json::to_string(&progress).expect("serialize EntityBackfillProgress");
        let decoded: EntityBackfillProgress = serde_json::from_str(&json).expect("deserialize EntityBackfillProgress");
        assert_eq!(decoded.processed, 42);
        assert_eq!(decoded.status, "running");
        assert!(decoded.error.is_none());
        assert_eq!(decoded.eta_seconds, Some(15), "etaSeconds must survive serde roundtrip");
        assert_eq!(decoded.fallbacks, 3, "fallbacks must survive serde roundtrip");
        // Verify camelCase keys
        assert!(json.contains("etaSeconds"), "expected camelCase 'etaSeconds' in: {}", json);
        assert!(json.contains("fallbacks"),  "expected 'fallbacks' in: {}", json);
    }

    // ===== Phase 10 Plan 01 tests: Space struct extension (D-07) =====

    /// Test 1: Space with depth=1, parent_id=Some(...), sub_space_ids=[...] round-trips
    /// through serde_json::to_string → from_str preserving all three fields exactly.
    /// D-07: depth u8 + sub_space_ids Vec<String> mandatory on Space.
    #[test]
    fn space_phase10_fields_roundtrip() {
        let space = Space {
            id: "sub-space-tax".to_string(),
            name: "Tax Records".to_string(),
            icon: "FileText".to_string(),
            color: "#6D28D9".to_string(),
            document_count: 12,
            last_updated: "2026-07-08T00:00:00Z".to_string(),
            sub_spaces: vec![],
            parent_id: Some("parent-uuid-property".to_string()),
            sample_files: vec![],
            description: None,
            user_locked: false,
            canonical_entity_hint: None,
            label_status: None,
            depth: 1,
            sub_space_ids: vec!["a".to_string(), "b".to_string()],
        };
        let json = serde_json::to_string(&space).expect("serialize Phase-10 Space");
        let decoded: Space = serde_json::from_str(&json).expect("deserialize Phase-10 Space");
        assert_eq!(decoded.depth, 1, "depth must survive roundtrip");
        assert_eq!(decoded.parent_id, Some("parent-uuid-property".to_string()),
            "parent_id must survive roundtrip");
        assert_eq!(decoded.sub_space_ids, vec!["a".to_string(), "b".to_string()],
            "sub_space_ids must survive roundtrip");
        // Verify camelCase serialization
        assert!(json.contains("subSpaceIds"), "expected camelCase 'subSpaceIds' in: {}", json);
        assert!(json.contains("\"depth\":1"), "expected depth=1 in: {}", json);
    }

    /// Test 2: Deserializing JSON that lacks `depth` and `subSpaceIds` must succeed,
    /// yielding depth=0 and sub_space_ids=vec![] (the serde defaults, D-07).
    /// Prevents pitfall #4 (wiping state on app upgrade to Phase 10).
    #[test]
    fn space_phase10_backward_compat_no_fields() {
        // Phase 9 JSON shape — no depth or subSpaceIds keys.
        let json = r##"{
            "id": "space-phase9",
            "name": "Property",
            "icon": "Home",
            "color": "#8B5CF6",
            "documentCount": 55,
            "lastUpdated": "2026-07-04T00:00:00Z",
            "subSpaces": [],
            "sampleFiles": [],
            "userLocked": false
        }"##;
        let decoded: Space = serde_json::from_str(json)
            .expect("Phase-9 Space JSON must deserialize without depth/subSpaceIds (backward compat)");
        assert_eq!(decoded.depth, 0,
            "missing depth must default to 0 (top-level)");
        assert!(decoded.sub_space_ids.is_empty(),
            "missing subSpaceIds must default to empty vec");
    }

    /// Test 3: A fresh top-level Space (depth=0, parent_id=None, sub_space_ids=[]) serializes
    /// with `"depth":0` and `"subSpaceIds":[]` in the camelCase JSON output.
    #[test]
    fn space_top_level_defaults() {
        let space = Space {
            id: "top-level-space".to_string(),
            name: "Work".to_string(),
            icon: "Briefcase".to_string(),
            color: "#10B981".to_string(),
            document_count: 0,
            last_updated: "2026-07-08T00:00:00Z".to_string(),
            sub_spaces: vec![],
            parent_id: None,
            sample_files: vec![],
            description: None,
            user_locked: false,
            canonical_entity_hint: None,
            label_status: None,
            depth: 0,
            sub_space_ids: vec![],
        };
        let json = serde_json::to_string(&space).expect("serialize top-level Space");
        assert!(json.contains("\"depth\":0"), "expected depth=0 in: {}", json);
        assert!(json.contains("\"subSpaceIds\":[]"), "expected subSpaceIds=[] in: {}", json);
    }

    // ===== Phase 9 Plan 01 tests: Space struct extension (T-09-01) =====

    /// Verify that a pre-Phase-9 Space JSON payload (missing description, user_locked,
    /// canonical_entity_hint, label_status) still deserializes without error and that
    /// all four new fields receive their correct Rust defaults:
    ///   description         → None
    ///   user_locked         → false   (bool default)
    ///   canonical_entity_hint → None
    ///   label_status        → None
    /// This is the backwards-compatibility test required by T-09-01 / pitfall #3.
    #[test]
    fn space_deserialize_backwards_compat() {
        // JSON that a pre-Phase-9 SpaceManager would have emitted — four new keys absent.
        // Note: r##"..."## used instead of r#"..."# because hex colors contain `"#` which
        // would prematurely close a single-hash raw string delimiter.
        let json = r##"{
            "id": "space-legacy",
            "name": "Old Space",
            "icon": "Home",
            "color": "#8B5CF6",
            "documentCount": 42,
            "lastUpdated": "2025-01-01T00:00:00Z",
            "subSpaces": [],
            "sampleFiles": []
        }"##;
        let decoded: Space = serde_json::from_str(json)
            .expect("pre-Phase-9 Space JSON must deserialize without the four new fields (T-09-01)");
        assert_eq!(decoded.id, "space-legacy");
        assert_eq!(decoded.name, "Old Space");
        assert_eq!(decoded.description, None,
            "missing description must default to None");
        assert!(!decoded.user_locked,
            "missing user_locked must default to false");
        assert_eq!(decoded.canonical_entity_hint, None,
            "missing canonical_entity_hint must default to None");
        assert_eq!(decoded.label_status, None,
            "missing label_status must default to None");
    }

    /// Verify that a Phase-9 Space JSON payload (all four new fields present) round-trips
    /// correctly and camelCase renames are applied (serde rename_all = "camelCase").
    #[test]
    fn space_phase9_fields_roundtrip() {
        let space = Space {
            id: "space-property".to_string(),
            name: "Property".to_string(),
            icon: "Home".to_string(),
            color: "#8B5CF6".to_string(),
            document_count: 124,
            last_updated: "2026-07-04T00:00:00Z".to_string(),
            sub_spaces: vec![],
            parent_id: None,
            sample_files: vec!["Property_Tax_2025.pdf".to_string()],
            description: Some("Documents related to property ownership and tax assessments.".to_string()),
            user_locked: true,
            canonical_entity_hint: Some("Person: Alex Doe".to_string()),
            label_status: Some("ready".to_string()),
            depth: 0,
            sub_space_ids: vec![],
        };
        let json = serde_json::to_string(&space).expect("serialize Phase-9 Space");
        let decoded: Space = serde_json::from_str(&json).expect("deserialize Phase-9 Space");
        assert_eq!(decoded.description, Some("Documents related to property ownership and tax assessments.".to_string()));
        assert!(decoded.user_locked, "user_locked must survive roundtrip");
        assert_eq!(decoded.canonical_entity_hint, Some("Person: Alex Doe".to_string()));
        assert_eq!(decoded.label_status, Some("ready".to_string()));
        // Verify camelCase keys in serialized JSON
        assert!(json.contains("userLocked"),           "expected camelCase 'userLocked' in: {}", json);
        assert!(json.contains("canonicalEntityHint"),  "expected camelCase 'canonicalEntityHint' in: {}", json);
        assert!(json.contains("labelStatus"),          "expected camelCase 'labelStatus' in: {}", json);
    }

    /// Test: old consumers without etaSeconds/fallbacks in JSON must still deserialize (serde(default)).
    #[test]
    fn test_entity_backfill_progress_backward_compat() {
        let json = r#"{"processed":10,"total":50,"status":"running","error":null}"#;
        let decoded: EntityBackfillProgress = serde_json::from_str(json)
            .expect("old progress JSON without eta/fallbacks must deserialize");
        assert_eq!(decoded.eta_seconds, None,  "missing etaSeconds must default to None");
        assert_eq!(decoded.fallbacks,   0,     "missing fallbacks must default to 0");
    }

    #[test]
    fn test_extracted_entity_without_canonical_id_roundtrip() {
        // Verify that JSON without canonicalId deserializes with canonical_id = None
        let json = r#"{"label":"Date","value":"2024-01-01","entityType":"date"}"#;
        let decoded: ExtractedEntity = serde_json::from_str(json).expect("deserialize ExtractedEntity without canonicalId");
        assert_eq!(decoded.canonical_id, None, "missing canonicalId field should default to None");
    }

    // ===== Phase 8 Task 1 tests: ExtractedEntity extension + ExtractedEntities container =====

    /// Test A (backward compat): v2 metadata without class/subclass/confidence MUST succeed
    /// with class=None, subclass=None, confidence=None — no deserialization failure.
    #[test]
    fn test_extracted_entity_backward_compat_v2() {
        let json = r#"{"label":"John","value":"John","entityType":"person","canonicalId":null}"#;
        let decoded: ExtractedEntity = serde_json::from_str(json)
            .expect("v2 metadata must deserialize without class/subclass/confidence");
        assert_eq!(decoded.label, "John");
        assert_eq!(decoded.entity_type, "person");
        assert_eq!(decoded.class, None, "class must default to None for v2 blobs");
        assert_eq!(decoded.subclass, None, "subclass must default to None for v2 blobs");
        assert_eq!(decoded.confidence, None, "confidence must default to None for v2 blobs");
    }

    /// Test B (full v3): v3 blob with class/subclass/confidence populates all new fields.
    #[test]
    fn test_extracted_entity_full_v3() {
        let json = r#"{"label":"Aadhaar","value":"1234567890123","entityType":"identifier","canonicalId":null,"class":"Identifier","subclass":"aadhaar","confidence":0.92}"#;
        let decoded: ExtractedEntity = serde_json::from_str(json)
            .expect("v3 metadata must deserialize with all new fields");
        assert_eq!(decoded.class, Some("Identifier".to_string()));
        assert_eq!(decoded.subclass, Some("aadhaar".to_string()));
        assert!((decoded.confidence.unwrap() - 0.92_f32).abs() < 1e-5,
            "confidence must be approximately 0.92");
    }

    /// Test C (ExtractedEntities round-trip): serialize+deserialize round-trips exactly.
    #[test]
    fn test_extracted_entities_roundtrip() {
        let entity = ExtractedEntity {
            label: "Test".to_string(),
            value: "test_value".to_string(),
            entity_type: "identifier".to_string(),
            canonical_id: None,
            class: Some("Identifier".to_string()),
            subclass: Some("pan".to_string()),
            canonical_short_name: None,
            confidence: Some(0.85),
        };
        let container = ExtractedEntities {
            entities: vec![entity],
            topic: Some("finance".to_string()),
            tags: vec!["bank_statement".to_string()],
            entities_version: TWO_PASS_TARGET_VERSION,
            language: Some("en".to_string()),
        };
        let json = serde_json::to_string(&container).expect("serialize ExtractedEntities");
        let decoded: ExtractedEntities = serde_json::from_str(&json)
            .expect("deserialize ExtractedEntities");
        assert_eq!(decoded.topic, Some("finance".to_string()));
        assert_eq!(decoded.tags, vec!["bank_statement".to_string()]);
        assert!((decoded.entities_version - TWO_PASS_TARGET_VERSION).abs() < 1e-5);
        assert_eq!(decoded.language, Some("en".to_string()));
        assert_eq!(decoded.entities.len(), 1);
        // Verify camelCase in JSON
        assert!(json.contains("entitiesVersion"), "expected camelCase 'entitiesVersion'");
    }

    /// Test D: ExtractedEntities::pass1_only returns topic=None, tags=empty, entities_version=PASS1_ONLY_VERSION.
    #[test]
    fn test_extracted_entities_pass1_only_constructor() {
        let entities = vec![ExtractedEntity {
            label: "2024-01-01".to_string(),
            value: "2024-01-01".to_string(),
            entity_type: "date".to_string(),
            canonical_id: None,
            class: Some("Date".to_string()),
            subclass: None,
            canonical_short_name: None,
            confidence: Some(1.0),
        }];
        let container = ExtractedEntities::pass1_only(entities);
        assert_eq!(container.topic, None, "pass1_only topic must be None");
        assert!(container.tags.is_empty(), "pass1_only tags must be empty");
        assert!((container.entities_version - PASS1_ONLY_VERSION).abs() < 1e-5,
            "pass1_only entities_version must be PASS1_ONLY_VERSION (2.5)");
        assert_eq!(container.language, None);
    }

    /// Test E: v3 blob missing entitiesVersion must NOT panic — defaults to 2.0 (legacy assumption).
    #[test]
    fn test_extracted_entities_missing_version_defaults_to_legacy() {
        let json = r#"{"entities":[],"topic":null,"tags":[],"language":null}"#;
        let decoded: ExtractedEntities = serde_json::from_str(json)
            .expect("missing entitiesVersion must not panic — must use serde default");
        assert!((decoded.entities_version - 2.0_f32).abs() < 1e-5,
            "missing entitiesVersion must default to 2.0 (legacy BERT assumption), got: {}",
            decoded.entities_version);
    }

    // ===== Phase 8 Task 2 tests: Settings extension + normalize_tag =====

    /// Test A (Settings backward compat): Phase 5 settings.json without new fields
    /// MUST deserialize with extraction_model="" and use_llm_extraction=false (defaults).
    #[test]
    fn test_settings_backward_compat_phase5() {
        let json = r#"{
            "theme": "dark",
            "sidebarCollapsed": false,
            "embeddingModel": "local",
            "watchedFolders": [],
            "excludedPatterns": [],
            "indexOnStartup": true,
            "indexSize": 0,
            "storagePath": "~/Library"
        }"#;
        let decoded: Settings = serde_json::from_str(json)
            .expect("Phase 5 settings.json without new fields must deserialize without error (T-08-01)");
        assert_eq!(decoded.extraction_model, String::new(),
            "missing extractionModel must default to empty string");
        assert!(!decoded.use_llm_extraction,
            "missing useLlmExtraction must default to false");
    }

    /// Test B (Settings round-trip with new fields): new fields survive serde round-trip.
    #[test]
    fn test_settings_roundtrip_with_new_fields() {
        let settings = Settings {
            theme: "dark".to_string(),
            sidebar_collapsed: false,
            embedding_model: "local".to_string(),
            watched_folders: vec![],
            excluded_patterns: vec![],
            index_on_startup: true,
            index_size: 0,
            storage_path: "~/Library".to_string(),
            extraction_model: "claude-haiku-4-5-20251001".to_string(),
            use_llm_extraction: true,
        };
        let json = serde_json::to_string(&settings).expect("serialize Settings");
        let decoded: Settings = serde_json::from_str(&json).expect("deserialize Settings");
        assert_eq!(decoded.extraction_model, "claude-haiku-4-5-20251001");
        assert!(decoded.use_llm_extraction);
        // Verify camelCase keys are in JSON
        assert!(json.contains("extractionModel"), "expected camelCase 'extractionModel' in: {}", json);
        assert!(json.contains("useLlmExtraction"), "expected camelCase 'useLlmExtraction' in: {}", json);
    }

    // normalize_tag tests (D-35)

    /// Test D: "Term Insurance" → "term_insurance"
    #[test]
    fn test_normalize_tag_basic_phrase() {
        assert_eq!(normalize_tag("Term Insurance"), "term_insurance");
    }

    /// Test E: "Investments 2024!" → "investments_2024" (trailing special char stripped, no trailing '_')
    #[test]
    fn test_normalize_tag_special_chars_stripped() {
        assert_eq!(normalize_tag("Investments 2024!"), "investments_2024");
    }

    /// Test F: "  khush   school  " → "khush_school" (collapses runs of whitespace)
    #[test]
    fn test_normalize_tag_collapses_whitespace() {
        assert_eq!(normalize_tag("  khush   school  "), "khush_school");
    }

    /// Test G: "already_snake" → "already_snake" (idempotent)
    #[test]
    fn test_normalize_tag_idempotent() {
        assert_eq!(normalize_tag("already_snake"), "already_snake");
    }

    /// Test H: "café" → "caf" (non-ASCII stripped per D-35, only alphanumeric-or-underscore survives)
    #[test]
    fn test_normalize_tag_non_ascii_stripped() {
        assert_eq!(normalize_tag("café"), "caf");
    }

    /// WR-08: dashes are now converted to '_' (same as whitespace) so hyphenated LLM
    /// tags match their space-separated equivalents.
    #[test]
    fn test_normalize_tag_dashes_and_mixed() {
        assert_eq!(normalize_tag("term-insurance"), "term_insurance");   // WR-08 fix
        assert_eq!(normalize_tag("self-employed"),  "self_employed");    // common LLM tag
        assert_eq!(normalize_tag("  "), "");
        assert_eq!(normalize_tag("__foo__"), "foo");
    }

    // ===== Phase 11.5 Plan 01 tests: Ontology / Relation Extraction types =====

    #[test]
    fn test_predicate_vocabulary_has_21_entries() {
        assert_eq!(
            PREDICATE_VOCABULARY.len(),
            21,
            "PREDICATE_VOCABULARY must have exactly 21 tokens per D-01, got {}",
            PREDICATE_VOCABULARY.len()
        );
    }

    #[test]
    fn test_symmetric_predicates_subset_of_vocabulary() {
        for p in SYMMETRIC_PREDICATES {
            assert!(
                PREDICATE_VOCABULARY.contains(p),
                "SYMMETRIC_PREDICATES entry '{}' must be in PREDICATE_VOCABULARY",
                p
            );
        }
    }

    #[test]
    fn test_auto_inverse_pairs_all_valid_predicates() {
        for (a, b) in AUTO_INVERSE_PAIRS {
            assert!(
                PREDICATE_VOCABULARY.contains(a),
                "AUTO_INVERSE_PAIRS entry '{}' must be in PREDICATE_VOCABULARY",
                a
            );
            assert!(
                PREDICATE_VOCABULARY.contains(b),
                "AUTO_INVERSE_PAIRS entry '{}' must be in PREDICATE_VOCABULARY",
                b
            );
        }
    }

    #[test]
    fn test_is_valid_predicate() {
        assert!(is_valid_predicate("owns"), "'owns' must be a valid predicate");
        assert!(!is_valid_predicate("owns_a_thing"), "'owns_a_thing' must not be a valid predicate");
    }

    #[test]
    fn test_pass3_target_version_greater_than_two_pass() {
        assert!(
            PASS3_TARGET_VERSION > TWO_PASS_TARGET_VERSION,
            "PASS3_TARGET_VERSION ({}) must be greater than TWO_PASS_TARGET_VERSION ({})",
            PASS3_TARGET_VERSION,
            TWO_PASS_TARGET_VERSION
        );
    }

    fn sample_canonical_entity(id: &str, name: &str) -> CanonicalEntity {
        CanonicalEntity {
            id: id.to_string(),
            canonical_name: name.to_string(),
            entity_type: "person".to_string(),
            aliases: vec![],
            document_count: 1,
            canonical_short_name: None,
        }
    }

    fn sample_triple() -> Triple {
        Triple {
            id: "t-1234".to_string(),
            subject_id: "ent-alex".to_string(),
            predicate: "owns".to_string(),
            object_id: "ent-raga2004".to_string(),
            doc_ids: vec!["doc-1".to_string(), "doc-2".to_string()],
            user_added: false,
            created_at: "2026-07-08T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn test_triple_serde_roundtrip() {
        let triple = sample_triple();
        let json = serde_json::to_string(&triple).expect("serialize Triple");
        let decoded: Triple = serde_json::from_str(&json).expect("deserialize Triple");
        assert_eq!(decoded, triple);
        assert!(json.contains("subjectId"), "expected camelCase 'subjectId' in: {}", json);
        assert!(json.contains("objectId"), "expected camelCase 'objectId' in: {}", json);
        assert!(json.contains("docIds"), "expected camelCase 'docIds' in: {}", json);
        assert!(json.contains("userAdded"), "expected camelCase 'userAdded' in: {}", json);
        assert!(json.contains("createdAt"), "expected camelCase 'createdAt' in: {}", json);
    }

    #[test]
    fn test_triple_with_entities_serde_roundtrip() {
        let twe = TripleWithEntities {
            triple: sample_triple(),
            subject: sample_canonical_entity("ent-alex", "Alex Doe"),
            object: sample_canonical_entity("ent-raga2004", "Unit 204"),
        };
        let json = serde_json::to_string(&twe).expect("serialize TripleWithEntities");
        let decoded: TripleWithEntities =
            serde_json::from_str(&json).expect("deserialize TripleWithEntities");
        assert_eq!(decoded.triple, twe.triple);
        assert_eq!(decoded.subject.canonical_name, "Alex Doe");
        assert_eq!(decoded.object.canonical_name, "Unit 204");
    }

    #[test]
    fn test_relations_page_data_serde_roundtrip() {
        let data = RelationsPageData {
            entity: sample_canonical_entity("ent-alex", "Alex Doe"),
            outgoing: vec![TripleWithEntities {
                triple: sample_triple(),
                subject: sample_canonical_entity("ent-alex", "Alex Doe"),
                object: sample_canonical_entity("ent-raga2004", "Unit 204"),
            }],
            incoming: vec![],
        };
        let json = serde_json::to_string(&data).expect("serialize RelationsPageData");
        let decoded: RelationsPageData =
            serde_json::from_str(&json).expect("deserialize RelationsPageData");
        assert_eq!(decoded.entity.id, "ent-alex");
        assert_eq!(decoded.outgoing.len(), 1);
        assert!(decoded.incoming.is_empty());
    }

    #[test]
    fn test_ownership_page_data_serde_roundtrip() {
        let mut assets_by_type = std::collections::HashMap::new();
        assets_by_type.insert(
            AssetType::Property,
            vec![TripleWithEntities {
                triple: sample_triple(),
                subject: sample_canonical_entity("ent-alex", "Alex Doe"),
                object: sample_canonical_entity("ent-raga2004", "Unit 204"),
            }],
        );
        let data = OwnershipPageData {
            person: sample_canonical_entity("ent-alex", "Alex Doe"),
            assets_by_type,
            total_assets: 1,
        };
        let json = serde_json::to_string(&data).expect("serialize OwnershipPageData");
        assert!(json.contains("\"Property\""), "expected PascalCase 'Property' key in: {}", json);
        let decoded: OwnershipPageData =
            serde_json::from_str(&json).expect("deserialize OwnershipPageData");
        assert_eq!(decoded.total_assets, 1);
        assert_eq!(
            decoded.assets_by_type.get(&AssetType::Property).map(|v| v.len()),
            Some(1)
        );
    }

    #[test]
    fn test_predicate_subject_object_pair_serde_roundtrip() {
        let subject_pair = PredicateSubjectPair {
            subject: sample_canonical_entity("ent-alex", "Alex Doe"),
            triple: sample_triple(),
        };
        let json = serde_json::to_string(&subject_pair).expect("serialize PredicateSubjectPair");
        let decoded: PredicateSubjectPair =
            serde_json::from_str(&json).expect("deserialize PredicateSubjectPair");
        assert_eq!(decoded.subject.id, "ent-alex");

        let object_pair = PredicateObjectPair {
            object: sample_canonical_entity("ent-raga2004", "Unit 204"),
            triple: sample_triple(),
        };
        let json = serde_json::to_string(&object_pair).expect("serialize PredicateObjectPair");
        let decoded: PredicateObjectPair =
            serde_json::from_str(&json).expect("deserialize PredicateObjectPair");
        assert_eq!(decoded.object.id, "ent-raga2004");
    }

    // ===== Phase 11.6 Plan 01 tests: Adaptive Ontology types =====

    #[test]
    fn test_seed_predicates_aliases_baseline_vocabulary() {
        assert_eq!(
            SEED_PREDICATES, PREDICATE_VOCABULARY,
            "SEED_PREDICATES must be a compile-time alias of PREDICATE_VOCABULARY (D-03)"
        );
        assert_eq!(
            SEED_PREDICATES.len(),
            21,
            "SEED_PREDICATES must preserve the 21-token Phase 11.5 baseline, got {}",
            SEED_PREDICATES.len()
        );
    }

    #[test]
    fn test_predicate_roundtrip_camelcase() {
        let predicate = Predicate {
            name: "registered_to".to_string(),
            description: "Vehicle registered to a person".to_string(),
            source: PromotionSource::Adaptive,
            count: 3,
            first_seen_doc_id: Some("doc-1".to_string()),
            first_seen_at: Some("2026-07-01T00:00:00Z".to_string()),
            promoted_at: Some("2026-07-05T00:00:00Z".to_string()),
            subject_class: Some("Vehicle".to_string()),
            object_class: Some("Person".to_string()),
        };
        let json = serde_json::to_string(&predicate).expect("serialize Predicate");
        assert!(
            json.contains("\"firstSeenDocId\""),
            "expected camelCase 'firstSeenDocId' in: {}",
            json
        );
        let decoded: Predicate = serde_json::from_str(&json).expect("deserialize Predicate");
        assert_eq!(decoded, predicate);
    }

    #[test]
    fn test_bootstrap_seed_partial_json() {
        let json = r#"{"generatedAt":"2026-07-09T00:00:00Z"}"#;
        let decoded: BootstrapSeed = serde_json::from_str(json)
            .expect("partial BootstrapSeed JSON must deserialize (Phase 8 Pitfall 5 policy)");
        assert!(decoded.predicates.is_empty());
        assert!(decoded.entity_subclasses.is_empty());
        assert_eq!(decoded.generated_at, "2026-07-09T00:00:00Z");
        assert_eq!(decoded.sample_doc_count, 0);
        assert_eq!(decoded.model_used, "");
    }

    #[test]
    fn test_consolidation_kind_tagged_serde() {
        let kind = ConsolidationKind::Merge {
            from: vec!["a".to_string(), "b".to_string()],
            into: "c".to_string(),
        };
        let json = serde_json::to_string(&kind).expect("serialize ConsolidationKind");
        assert_eq!(json, r#"{"kind":"merge","from":["a","b"],"into":"c"}"#);
        let decoded: ConsolidationKind =
            serde_json::from_str(&json).expect("deserialize ConsolidationKind");
        assert_eq!(decoded, kind);
    }

    #[test]
    fn test_ontology_store_schema_empty_default() {
        let schema = OntologyStoreSchema::default();
        assert_eq!(schema.version, 1, "default version must be 1 per D-18");
        assert!(schema.corpus_seed.is_none());
        assert!(schema.adaptive_predicates.is_empty());
        assert!(schema.pending_predicates.is_empty());
        assert!(schema.manual_predicates.is_empty());
        assert!(schema.entity_subclasses.is_empty());
        assert!(schema.pending_consolidation.is_none());
        assert!(!schema.automatic_growth_enabled, "opt-in default must be false (D-21)");
        assert!(schema.bootstrap_completed_at.is_none());
        assert!(schema.last_consolidation_at.is_none());
        assert_eq!(schema.triples_since_last_consolidation, 0);

        // Also verify a minimal/partial JSON payload deserializes with version defaulting to 1.
        let decoded: OntologyStoreSchema =
            serde_json::from_str("{}").expect("empty JSON must deserialize to safe defaults");
        assert_eq!(decoded.version, 1);
    }

    #[test]
    fn test_extracted_entity_backward_compat_no_canonical_short_name() {
        let json = r#"{"label":"Aadhaar","value":"1234567890123","entityType":"identifier","canonicalId":null,"class":"Identifier","subclass":"aadhaar","confidence":0.92}"#;
        let decoded: ExtractedEntity = serde_json::from_str(json)
            .expect("legacy JSON without canonicalShortName must still deserialize");
        assert_eq!(decoded.canonical_short_name, None);
    }

    #[test]
    fn test_canonical_entity_backward_compat_no_canonical_short_name() {
        let json = r#"{"id":"ent-1","canonicalName":"Acme Corp","entityType":"organization","aliases":["Acme"],"documentCount":1}"#;
        let decoded: CanonicalEntity = serde_json::from_str(json)
            .expect("legacy JSON without canonicalShortName must still deserialize");
        assert_eq!(decoded.canonical_short_name, None);
    }

    #[test]
    fn test_promotion_source_lowercase_serde() {
        let json = serde_json::to_string(&PromotionSource::Adaptive)
            .expect("serialize PromotionSource");
        assert_eq!(json, "\"adaptive\"");
    }
}
