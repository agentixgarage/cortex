/**
 * Cortex shared data types used across frontend hooks and components.
 * These mirror the Rust backend types returned by IPC commands.
 *
 * Field names match the exact camelCase JSON produced by Rust serde
 * with #[serde(rename_all = "camelCase")].
 */

// -------------------------------------------------------------------------
// Phase 8: LLM Entity Extraction types
// See .planning/phases/08-llm-entity-extraction/08-CONTEXT.md (D-08, D-09)
// Mirrors Rust ExtractedEntity struct in src-tauri/src/types.rs (Plan 01).
// -------------------------------------------------------------------------

/**
 * A single entity extracted from a document.
 * Phase 6 shape: label + value + entityType + canonicalId (backward-compatible).
 * Phase 8 additions: class (8-class taxonomy), subclass (free-form), confidence (0.0–1.0).
 */
export interface ExtractedEntity {
  label: string;
  value: string;
  entityType: string; // "date", "amount", "person", "organization", "location", "email", "phone", "identifier"
  canonicalId?: string; // Links occurrence to CanonicalEntity (Phase 6 KG)
  // Phase 8 additions — optional for Phase 6 backward compatibility (D-08, D-09)
  class?: string; // 8-class fixed taxonomy: Person | Organization | Location | Date | Amount | Email | Phone | Identifier
  subclass?: string; // free-form e.g. "aadhaar", "iban", "pan", "gstin"
  // Phase 11.6 (D-09): rule-based canonical short form (e.g. "Alpha Beta Corp
  // AlphaComplex-Unit-204" -> "Unit 204"). Absent when the normalizer produced no
  // rewrite (falls back to `value`).
  canonicalShortName?: string;
  confidence?: number; // 0.0–1.0; UI shows < 0.7 under "Also found" expander (D-15)
}

/**
 * Document-level entity extraction result container.
 * Mirrors Rust ExtractedEntities struct (Plan 01 SUMMARY).
 * Wraps Phase 8 doc-level metadata: topic, tags, version, language.
 */
export interface ExtractedEntities {
  entities: ExtractedEntity[];
  topic: string | null; // single free-form topic, snake_case normalized; null when not extracted
  tags: string[]; // 2-5 free-form hashtag-style tags, snake_case normalized
  entitiesVersion: number; // 2=BERT (legacy), 2.5=Pass1 only, 3=Pass1+Pass2 complete
  language: string | null; // ISO 639-1 language code e.g. "en"; null if not detected
}

/**
 * Standalone extraction settings type consumed by useExtractionSettings hook
 * and the ExtractionSettings component in Plan 07.
 * Mirrors Rust ExtractionSettings struct from commands/settings.rs.
 */
export interface ExtractionSettings {
  extractionModel: string; // IPC: "claude-haiku-4-5-20251001", "gpt-5-mini", "gemini-2.5-flash", ""
  useLlmExtraction: boolean; // When false, Pass 1 only (dates/amounts/IDs); Pass 2 skipped (D-33)
}

export interface Document {
  id: string;
  name: string;
  path: string;
  docType: string; // "pdf", "docx", "txt", "png", "jpg", "xlsx", "csv", "md", "other"
  size: number;
  createdAt: string; // ISO date string
  modifiedAt: string; // ISO date string
  excerpt?: string;
  spaceIds: string[];
  tags: string[];
  isFavorite: boolean;
  extractedEntities: ExtractedEntity[]; // Phase 8: uses ExtractedEntity (was inline type)
  thumbnailColor?: string;
  // Phase 8 Plan 09: doc-level topic label from Pass 2 LLM extraction.
  // Optional: undefined/null when topic not yet extracted (Pass 1 only) or "other".
  // snake_case normalized per D-35. Used by TopicFilterBar client-side filter.
  topic?: string;
  // Phase 8 Plan 08-08: LLM-extracted free-form keyword tags (2-5 per doc, snake_case).
  // Named `llmTags` (not `tags`) to avoid collision with existing `tags: string[]`
  // (which stores user/space tags at the Cortex level). Mirrors Rust `llm_tags` field
  // on the Document struct (serialized as `llmTags` via serde camelCase). Default: absent.
  llmTags?: string[];
}

// -------------------------------------------------------------------------
// Phase 8 Plan 09: Topic filter types
// -------------------------------------------------------------------------

/**
 * Aggregate count of documents assigned a given topic label.
 * Mirrors Rust TopicCount struct from src-tauri/src/types.rs.
 * Returned by get_topics IPC; consumed by TopicFilterBar.
 */
export interface TopicCount {
  topic: string; // snake_case (normalize_tag output, e.g. "term_insurance")
  count: number; // number of indexed documents with this topic
}

export interface DocumentMeta {
  id: string;
  name: string;
  path: string;
  docType: string;
  size: number;
  createdAt: string; // ISO date string
  modifiedAt: string; // ISO date string
}

// -------------------------------------------------------------------------
// Phase 11 Plan 01: Entity-Driven Exploration type surface
// Mirrors Rust structs added in src-tauri/src/types.rs (Plan 01).
// All fields are camelCase matching #[serde(rename_all = "camelCase")] on Rust side.
// -------------------------------------------------------------------------

/**
 * Entity class+value filter pair. Produced by splitting URL param `?entity={class}:{value}`
 * on the first ':' (e.g. "Person:Alex Doe" → class="Person", value="Alex Doe").
 * Mirrors Rust EntityClassFilter (11-CONTEXT.md D-01, D-03).
 */
export interface EntityClassFilter {
  class: string;
  value: string;
}

/**
 * Persisted filter state for a SavedSearch.
 * entities: "{class}:{value}" strings per D-06 shape (e.g. ["Person:Alex Doe"]).
 * Mirrors Rust SavedSearchFilters.
 */
export interface SavedSearchFilters {
  entities?: string[];        // "{class}:{value}" strings
  topic?: string | null;
  docType?: string;
  spaceId?: string;
  dateFrom?: string;
  dateTo?: string;
  tags?: string[];
}

/**
 * A saved virtual search Space persisted to app_data_dir/saved_searches.json.
 * id format: "ss-{uuid}" (enforced by Plan 04 save_search).
 * docCountCache is a render hint; live count re-evaluates on Sidebar mount (D-08, ENEX-04).
 * Mirrors Rust SavedSearch (11-CONTEXT.md D-05, D-06).
 */
export interface SavedSearch {
  id: string;           // "ss-{uuid}"
  name: string;
  query: string;
  filters: SavedSearchFilters;
  createdAt: string;    // ISO 8601
  docCountCache: number; // hint for immediate render; may be stale after new docs indexed
}

/**
 * A document similar to a given document, scored by composite relevance.
 * score = 0.6 × cosine + 0.4 × entity_overlap_jaccard (D-10, D-11).
 * snippet: text excerpt around entity-overlap region; null when absent (panel shows title + badge only).
 * Mirrors Rust RelatedDocScored (11-RESEARCH.md Pattern 3).
 */
export interface RelatedDocScored {
  document: Document;
  score: number;         // composite (0.6*cosine + 0.4*jaccard)
  cosineScore: number;
  jaccardScore: number;
  snippet?: string | null; // text excerpt around entity-overlap region
}

/**
 * Class+value keyed reference to a co-occurring entity, used in EntityPageData.
 * Distinct from RelatedEntity (which keys by canonical id).
 * coDocCount: documents mentioning both this entity and the source entity.
 * Mirrors Rust RelatedEntityRef (11-CONTEXT.md D-15, D-17).
 */
export interface RelatedEntityRef {
  class: string;
  value: string;
  coDocCount: number;
}

/**
 * Full data payload for the /entity/:class/:value detail page.
 * canonical: CanonicalEntity with aliases from Phase 6 entity_store alias index.
 * documents: paginated documents mentioning this entity (20/page per D-16).
 * coOccurringEntities: top-10 class+value pairs from Phase 6 co-occurrence.
 * Mirrors Rust EntityPageData (11-RESEARCH.md Pattern 4; 11-CONTEXT.md D-15, D-16).
 */
export interface EntityPageData {
  canonical: CanonicalEntity;
  documents: Document[];
  totalDocumentCount: number;
  coOccurringEntities: RelatedEntityRef[];
  page: number;
  pageSize: number;
}

export interface SearchFilters {
  docType?: string;
  spaceId?: string;
  dateFrom?: string; // ISO date string
  dateTo?: string; // ISO date string
  tags?: string[];
  // Optional: matches Rust entity_filters: Option<Vec<EntityClassFilter>> + #[serde(default)].
  // Pre-Phase-11 callers omitting this field continue to work (backward compatible).
  entityFilters?: EntityClassFilter[];
}

export interface SearchResult {
  document: Document;
  score: number;
  matchedExcerpt?: string;
}

export interface Space {
  id: string;
  name: string;
  icon: string; // Lucide icon name (e.g., 'Home', 'Briefcase')
  color: string; // Hex color for accent
  documentCount: number;
  lastUpdated: string; // ISO date string
  subSpaces: Space[];
  parentId?: string;
  sampleFiles: string[];
  // -------------------------------------------------------------------------
  // Phase 9 Plan 01: LLM Space labeling fields
  // Mirrors Rust Space struct additions (all #[serde(default)] on Rust side).
  // Frontend must treat absent / undefined values as follows:
  //   description        → undefined = no description to show
  //   userLocked         → undefined/false = space is not locked
  //   canonicalEntityHint → undefined = no entity hint chip to render
  //   labelStatus        → undefined | 'ready' = ready (no shimmer)
  //                        'generating' = show shimmer skeleton (D-14)
  // -------------------------------------------------------------------------
  /** LLM-generated description of the space. Shown as tooltip on SpaceCard (D-16). */
  description?: string;
  /** True when the user has manually renamed the space — LLM re-labels skip it (D-15). */
  userLocked?: boolean;
  /** Highest-count entity across space docs, format "{Class}: {value}" (D-17, D-18). */
  canonicalEntityHint?: string;
  /** 'ready' | 'generating'. Treat undefined as 'ready'. Drives shimmer state in SpaceCard (D-14). */
  labelStatus?: 'ready' | 'generating';
  // -------------------------------------------------------------------------
  // Phase 10 Plan 02: Hierarchical Spaces fields
  // Mirrors Rust Space struct additions (all #[serde(default)] on Rust side).
  // Frontend must treat absent / undefined values as follows:
  //   depth       → undefined | 0 = top-level space
  //   subSpaceIds → undefined | [] = no sub-spaces
  // -------------------------------------------------------------------------
  /** 0 for top-level spaces, 1 for sub-spaces. Max depth = 2 (gated by backend). D-03/D-07. */
  depth?: number;
  /** IDs of direct child sub-spaces. Empty array for sub-spaces and spaces < 50 docs. D-07. */
  subSpaceIds?: string[];
}

export interface WatchedFolder {
  id: string;
  path: string;
  documentCount: number;
  lastScan: string; // ISO date string
  status: string; // "watching", "paused", "error"
}

export interface ScanProgress {
  folderId: string;
  totalFiles: number;
  processedFiles: number;
  status: string; // "scanning", "complete", "error"
}

export interface Stats {
  totalDocuments: number;
  smartSpaces: number;
  lastScan: string; // ISO date string
  indexSize: number; // bytes
}

export interface SpaceGraph {
  nodes: Array<{
    id: string;
    name: string;
    color: string;
    documentCount: number;
  }>;
  edges: Array<{
    source: string;
    target: string;
    weight: number;
  }>;
}

export interface TopQuery {
  query: string;
  count: number;
}

export interface SearchAnalytics {
  totalSearches: number;
  topQueries: TopQuery[];
  avgResultsPerQuery: number;
  queriesThisWeek: number;
}

export interface Settings {
  theme: string; // "dark", "light", "system"
  sidebarCollapsed: boolean;
  embeddingModel: string; // "local", "openai"
  watchedFolders: string[];
  excludedPatterns: string[];
  indexOnStartup: boolean;
  indexSize: number; // bytes
  storagePath: string;
  // Phase 8: LLM entity extraction settings (D-11, D-33)
  // Defaults guaranteed by backend default_settings(); always present in JSON response.
  extractionModel: string; // model ID for Pass 2; defaults to provider fast-tier (D-11)
  useLlmExtraction: boolean; // controls Pass 2; default true when provider connected (D-33)
}

export interface Tag {
  id: string;
  name: string;
  color: string;
  documentCount: number;
  tagType: string; // "auto", "user"
}

export interface ActivityItem {
  id: string;
  action: string; // "indexed", "moved", "tagged", "searched"
  subject: string;
  timestamp: string; // ISO date string
  type: string; // "info", "success", "warning", "error"
  documentId?: string;
}

// -------------------------------------------------------------------------
// Knowledge Graph types — mirrors of Rust types from Plan 02 (Phase 6)
// All fields are camelCase matching #[serde(rename_all = "camelCase")] on Rust side.
// -------------------------------------------------------------------------

export interface CanonicalEntity {
  id: string;
  canonicalName: string;
  entityType: string; // "person", "organization", "location", "date", "amount", "email"
  aliases: string[];
  documentCount: number;
  // Phase 11.6 (D-11): most-frequent canonicalShortName across aliases; falls
  // back to canonicalName when absent.
  canonicalShortName?: string;
}

export interface EntitySummary {
  id: string;
  canonicalName: string;
  entityType: string;
  documentCount: number;
}

export interface RelatedEntity {
  entity: EntitySummary;
  coOccurrenceCount: number;
}

export interface DocumentTextPreview {
  text: string | null;
  truncated: boolean;
  size: number;
}

export interface EntityBackfillProgress {
  processed: number;
  total: number;
  status: "running" | "complete" | "error";
  error?: string;
}

// -------------------------------------------------------------------------
// Phase 9 Plan 05: LLM Space Labeling progress + label cache entry types
// See .planning/phases/09-llm-space-labeling/09-CONTEXT.md D-14, D-15, D-17
// Field names match Plan 03's SpaceLabelingProgress + Plan 02's SpaceLabelEntry
// after serde camelCase conversion (snake_case → camelCase).
// -------------------------------------------------------------------------

/**
 * Tauri event payload emitted per-space during LLM batch labeling.
 * Event name: "space-labeling-progress".
 * Mirrors Rust SpaceLabelingProgress (Plan 03, serde camelCase).
 */
export interface SpaceLabelingProgress {
  spaceId: string;
  status: "labeling" | "complete" | "error";
  processed: number;
  total: number;
  label?: string;
  error?: string;
}

/**
 * A single cached space label entry returned by get_space_labels IPC.
 * Mirrors Rust SpaceLabelEntry (Plan 02, serde camelCase).
 * userLocked = true when the user has manually renamed the space (D-15).
 * canonicalEntityHint = highest-count entity across space docs (D-17, D-18).
 */
export interface SpaceLabelEntry {
  fingerprint: string;
  label: string;
  description: string;
  canonicalEntityHint?: string;
  generatedAt: string;
  userLocked: boolean;
}

// === Phase 7: AI Provider Foundation ===
// These types mirror Rust IPC structs from src-tauri/src/auth/commands.rs and src-tauri/src/ai/service.rs.
// Field names match the exact camelCase JSON produced by Rust serde with #[serde(rename_all = "camelCase")].

/**
 * Mirrors Rust `ProviderAuthStatus` struct (serde camelCase).
 * Returned by `list_providers` and `connect_provider` IPC commands.
 */
export interface ProviderAuthStatus {
  provider: string; // "anthropic" | "openai" | "gemini" | "ollama"
  authenticated: boolean;
  method: string; // "oauth" | "api-key" | "none"  (kebab-case — matches Rust AuthMethod serde rename_all="kebab-case")
  displayName: string | null;
  model: string | null;
  isActive: boolean;
}

/**
 * Request shape for `connect_provider` IPC command.
 * Mirrors Rust `LoginRequest` struct.
 * IMPORTANT: `method` must be kebab-case ("api-key" not "apikey") — matches Rust wire format.
 */
export interface ConnectProviderRequest {
  provider: string; // "anthropic" | "openai" | "gemini" | "ollama"
  method: string; // "api-key" | "ollama"  — kebab-case REQUIRED (Rust LoginRequest wire format)
  credential?: string; // API key value
  model?: string;
  baseUrl?: string; // Ollama base URL
}

/**
 * Response from `save_setup_token` IPC command.
 * Mirrors Rust `OAuthStartResult` struct.
 */
export interface OAuthStartResult {
  started: boolean;
  provider: string;
}

/**
 * Single message in an AI chat conversation.
 * Mirrors Rust `ServiceMessage` struct from src-tauri/src/ai/service.rs.
 */
export interface AiServiceMessage {
  role: string; // "user" | "assistant" | "system"
  content: string;
}

/**
 * Request shape for `chat` IPC command.
 * Mirrors Rust `AIServiceRequest` struct.
 */
export interface AiChatRequest {
  systemPrompt: string;
  messages: AiServiceMessage[];
  maxTokens?: number;
  temperature?: number;
}

/**
 * Response from `chat` IPC command.
 * Mirrors Rust `AIServiceResponse` struct.
 */
export interface AiChatResponse {
  content: string;
  model: string;
  inputTokens: number | null;
  outputTokens: number | null;
}

// -------------------------------------------------------------------------
// Phase 11.5: Ontology / Relation Extraction types
// Mirrors src-tauri/src/types.rs section "Phase 11.5" — keep field names in
// exact camelCase parity with the Rust structs (#[serde(rename_all = "camelCase")]).
// See .planning/phases/11.5-ontology-relations/11.5-CONTEXT.md D-01, D-11, D-12, D-14
// for provenance and design rationale.
// -------------------------------------------------------------------------

/**
 * Closed predicate vocabulary (v1) — 21 tokens, same order as Rust
 * PREDICATE_VOCABULARY. The LLM cannot invent new predicates (D-02);
 * `mentioned_with` is the weak default.
 */
export const PREDICATE_VOCABULARY = [
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
] as const;

/** Union of all 21 predicate string literals. */
export type Predicate = (typeof PREDICATE_VOCABULARY)[number];

/**
 * Asset category derived from the object entity's class + topic (D-14, D-18).
 * Matches Rust `AssetType` enum with `#[serde(rename_all = "PascalCase")]`.
 */
export type AssetType = "Property" | "Vehicle" | "Investment" | "Business" | "Financial" | "Other";

/**
 * A single (subject, predicate, object) relation extracted by Pass 3 (or added
 * manually via `add_manual_triple`). `docIds` is append-only provenance (D-11).
 * `userAdded` distinguishes manual overrides from LLM-extracted triples and is
 * preserved across LLM re-runs (D-12). Mirrors Rust `Triple`.
 */
export interface Triple {
  id: string;
  subjectId: string;
  predicate: Predicate;
  objectId: string;
  docIds: string[];
  userAdded: boolean;
  createdAt: string; // ISO 8601
}

/**
 * Expanded view of a Triple with its subject and object CanonicalEntity
 * records resolved, for direct frontend consumption without a second lookup.
 * Mirrors Rust `TripleWithEntities`.
 */
export interface TripleWithEntities {
  triple: Triple;
  subject: CanonicalEntity;
  object: CanonicalEntity;
}

/**
 * Full data payload for the entity detail page "Relations" panel.
 * `outgoing`: triples where `entity` is the subject. `incoming`: triples
 * where `entity` is the object. Mirrors Rust `RelationsPageData`.
 */
export interface RelationsPageData {
  entity: CanonicalEntity;
  outgoing: TripleWithEntities[];
  incoming: TripleWithEntities[];
}

/**
 * Full data payload for the `/ownership/:person_id` page. Assets are grouped
 * by AssetType per D-14; keys may be absent when that category is empty.
 * Mirrors Rust `OwnershipPageData`.
 */
export interface OwnershipPageData {
  person: CanonicalEntity;
  assetsByType: Partial<Record<AssetType, TripleWithEntities[]>>;
  totalAssets: number;
}

/** Payload entry for `get_subjects_by_predicate_object` — "who --predicate--> object?". */
export interface PredicateSubjectPair {
  subject: CanonicalEntity;
  triple: Triple;
}

/** Payload entry for `get_objects_by_subject_predicate` — "subject --predicate--> who?". */
export interface PredicateObjectPair {
  object: CanonicalEntity;
  triple: Triple;
}

// -------------------------------------------------------------------------
// Phase 11.6: Adaptive Ontology types
// Mirrors src-tauri/src/types.rs section "Phase 11.6" — keep field names in
// exact camelCase parity with the Rust structs (#[serde(rename_all = "camelCase")]).
// See .planning/phases/11.6-adaptive-ontology/11.6-CONTEXT.md D-01..D-23.
// -------------------------------------------------------------------------

/** Provenance of a predicate or entity subclass entry in the ontology store. */
export type PromotionSource = "seed" | "corpus" | "adaptive" | "manual";

/**
 * Phase 11.6 record for an entry in the ontology store. Distinct from
 * `Predicate` (above), which is the 11.5 literal-union type of the baked-in
 * 21-string vocabulary. `PredicateEntry` is the persisted record shape with
 * provenance + support count, mirroring the Rust `Predicate` struct.
 */
export interface PredicateEntry {
  name: string;
  description: string;
  source: PromotionSource;
  count: number;
  firstSeenDocId?: string;
  firstSeenAt?: string; // RFC3339
  promotedAt?: string;
  subjectClass?: string;
  objectClass?: string;
}

/** A single entity subclass entry in the adaptive ontology store (D-18). */
export interface EntitySubclass {
  class: string;
  subclass: string;
  source: PromotionSource;
  count: number;
  firstSeenDocId?: string;
  exampleValue?: string;
}

/**
 * Output shape of the corpus-seeded bootstrap LLM call (D-01, D-02): a batch
 * of proposed predicates + entity subclasses derived from ~30-50 sample docs.
 */
export interface BootstrapSeed {
  predicates: PredicateEntry[];
  entitySubclasses: EntitySubclass[];
  generatedAt: string;
  sampleDocCount: number;
  modelUsed: string;
}

/**
 * A single proposed ontology change from the consolidation loop (D-16).
 * Discriminated union mirroring Rust `ConsolidationKind`
 * (`#[serde(tag = "kind", rename_all = "lowercase")]`).
 */
export type ConsolidationKind =
  | { kind: "merge"; from: string[]; into: string }
  | { kind: "rename"; from: string; to: string }
  | { kind: "split"; from: string; into: string[] };

/** A single consolidation suggestion awaiting user approval (D-16, D-17). */
export interface ConsolidationSuggestion {
  id: string; // stable UUID, distinct per suggestion
  kind: ConsolidationKind;
  rationale: string;
  confidence: number;
}

/**
 * Batch of consolidation suggestions generated by a single consolidation
 * loop run (D-15..D-17). Cleared on user accept/reject (D-20).
 */
export interface PendingConsolidation {
  suggestions: ConsolidationSuggestion[];
  generatedAt: string;
  modelUsed: string;
  tripleCountAtGeneration: number;
}

/**
 * On-disk JSON schema for `app_data_dir/ontology.json` (D-18, D-19). Mirrors
 * Rust `OntologyStoreSchema` — loaded once at boot into AppState.
 */
export interface OntologyStoreSchema {
  version: number;
  corpusSeed: BootstrapSeed | null;
  adaptivePredicates: PredicateEntry[];
  pendingPredicates: PredicateEntry[];
  manualPredicates: PredicateEntry[];
  entitySubclasses: EntitySubclass[];
  pendingConsolidation: PendingConsolidation | null;
  automaticGrowthEnabled: boolean;
  bootstrapCompletedAt: string | null;
  lastConsolidationAt: string | null;
  triplesSinceLastConsolidation: number;
}

/**
 * Result of attempting to promote a pending predicate (D-06, D-08).
 * Mirrors Rust `PromoteResult` (`#[serde(tag = "kind", rename_all = "lowercase")]`).
 */
export type PromoteResult =
  | { kind: "promoted" }
  | { kind: "stillpending"; count: number }
  | { kind: "capexceeded" }
  | { kind: "alreadypresent" };

/**
 * Frequency-weighted entity ranking payload (D-12..D-14). Used by Sidebar
 * top-5 prominent entities and entity detail pages.
 */
export interface ProminentEntity {
  entity: CanonicalEntity;
  docCount: number;
  isProminent: boolean;
}

// -------------------------------------------------------------------------
// v1.2 #2: User profile (onboarding "About You" step)
// Mirrors Rust `UserProfile` / `FamilyMember` in types.rs.
// -------------------------------------------------------------------------

export interface FamilyMember {
  name: string;
  relation: string;
}

export interface UserProfile {
  displayName: string;
  aliases: string[];
  familyMembers: FamilyMember[];
  countries: string[];
  currencies: string[];
}
