/**
 * Mock data for frontend development (browser mode, no Tauri runtime).
 * Used by useTauri.ts hooks as fallback when isTauri() returns false.
 */

import type {
  Document,
  Space,
  Tag,
  WatchedFolder,
  Stats,
  SearchResult,
  SpaceGraph,
  SearchAnalytics,
  ActivityItem,
  Settings,
  EntitySummary,
  RelatedEntity,
  ProviderAuthStatus,
  ExtractionSettings,
  ExtractedEntities,
  TopicCount,
} from "./types";

// === Phase 7: AI Provider mock data ===
export const mockProviders: ProviderAuthStatus[] = [
  {
    provider: "anthropic",
    authenticated: true,
    method: "oauth",
    displayName: "Claude (Subscription)",
    model: "claude-haiku-4-5-20251001",
    isActive: true,
  },
  {
    provider: "openai",
    authenticated: false,
    method: "none",
    displayName: null,
    model: null,
    isActive: false,
  },
  {
    provider: "gemini",
    authenticated: false,
    method: "none",
    displayName: null,
    model: null,
    isActive: false,
  },
  {
    provider: "ollama",
    authenticated: true,
    method: "none",
    displayName: null,
    model: "llama3:latest",
    isActive: false,
  },
  // openai-codex: OAuth-connected state (used as mock fallback for useStartOpenAiOAuth)
  {
    provider: "openai-codex",
    authenticated: true,
    method: "oauth",
    displayName: "ChatGPT (Codex)",
    model: "gpt-4o",
    isActive: false,
  },
];

export const mockStats: Stats = {
  totalDocuments: 3942,
  smartSpaces: 24,
  lastScan: new Date(Date.now() - 2 * 60 * 1000).toISOString(), // 2 min ago
  indexSize: 1_288_490_188, // ~1.2 GB
};

export const mockSpaces: Space[] = [
  {
    id: "space-property",
    name: "Property",
    icon: "Home",
    color: "#8B5CF6",
    documentCount: 124,
    lastUpdated: new Date(Date.now() - 3 * 24 * 60 * 60 * 1000).toISOString(),
    sampleFiles: ["Property_Tax_2025.pdf", "Home_Insurance.pdf"],
    // Phase 9: description (surfaces as tooltip on SpaceCard, full text on /spaces/:id — D-16)
    description:
      "Documents related to property ownership, tax assessments, insurance policies, and registration records.",
    // Phase 9: canonicalEntityHint (drives entity hint chip in Plan 06 — D-17)
    canonicalEntityHint: "Person: Alex Doe",
    // Phase 9: labelStatus defaults to 'ready' (explicitly set for clarity)
    labelStatus: "ready",
    subSpaces: [
      {
        id: "space-property-tax",
        name: "Tax",
        icon: "Receipt",
        color: "#7C3AED",
        documentCount: 34,
        lastUpdated: new Date(Date.now() - 5 * 24 * 60 * 60 * 1000).toISOString(),
        parentId: "space-property",
        subSpaces: [],
        sampleFiles: [],
      },
      {
        id: "space-property-insurance",
        name: "Insurance",
        icon: "Shield",
        color: "#7C3AED",
        documentCount: 18,
        lastUpdated: new Date(Date.now() - 10 * 24 * 60 * 60 * 1000).toISOString(),
        parentId: "space-property",
        subSpaces: [],
        sampleFiles: [],
      },
    ],
  },
  {
    id: "space-kids",
    name: "Kids",
    icon: "Users",
    color: "#10B981",
    documentCount: 341,
    lastUpdated: new Date(Date.now() - 1 * 24 * 60 * 60 * 1000).toISOString(),
    sampleFiles: ["School_Report.pdf", "Medical_Record.pdf"],
    // Phase 9: description (tooltip + detail page)
    description: "School reports, progress cards, medical records, and activity documents for children.",
    // Phase 9: userLocked (user manually renamed this space — LLM re-labels skip it — D-15)
    userLocked: true,
    subSpaces: [],
  },
  {
    id: "space-work",
    name: "Work",
    icon: "Briefcase",
    color: "#3B82F6",
    documentCount: 1560,
    lastUpdated: new Date(Date.now() - 30 * 60 * 1000).toISOString(),
    sampleFiles: ["Q4_Report.xlsx", "Project_Plan.docx"],
    // Phase 9: labelStatus 'generating' drives shimmer skeleton in SpaceCard (D-14)
    labelStatus: "generating",
    subSpaces: [],
  },
  {
    id: "space-invoices",
    name: "Invoices",
    icon: "Receipt",
    color: "#F59E0B",
    documentCount: 213,
    lastUpdated: new Date(Date.now() - 6 * 60 * 60 * 1000).toISOString(),
    sampleFiles: ["Invoice_Feb2026.pdf"],
    // Phase 9: no new fields — exercises the "all Phase 9 fields absent" path
    // (backwards compat: frontend treats undefined labelStatus as 'ready')
    subSpaces: [],
  },
  {
    id: "space-medical",
    name: "Medical",
    icon: "Heart",
    color: "#EF4444",
    documentCount: 87,
    lastUpdated: new Date(Date.now() - 14 * 24 * 60 * 60 * 1000).toISOString(),
    sampleFiles: ["Lab_Results_2025.pdf"],
    // Phase 9: no new fields — also exercises backward-compat path
    subSpaces: [],
  },
];

export const mockDocuments: Document[] = [
  {
    id: "doc-1",
    name: "Property_Tax_2025.pdf",
    path: "/Users/demo/Documents/Property/Property_Tax_2025.pdf",
    docType: "pdf",
    size: 2_048_576,
    createdAt: "2025-02-15T00:00:00Z",
    modifiedAt: "2025-02-15T00:00:00Z",
    excerpt: "Notice of Property Tax Assessment for fiscal year 2025...",
    spaceIds: ["space-property", "space-property-tax"],
    tags: ["tax", "property", "2025"],
    isFavorite: false,
    topic: "finance", // Phase 8 Plan 09: doc-level topic for TopicFilterBar client-side filter
    llmTags: ["property_tax", "assessment", "annual"], // Phase 8 Plan 08-08: LLM-extracted tags
    extractedEntities: [
      // Phase 6 shape — backward compat (no class/confidence)
      { label: "Year", value: "2025", entityType: "date", class: "Date", confidence: 0.98 },
      // Phase 8: Amount with class + confidence
      { label: "Amount", value: "$4,200.00", entityType: "amount", class: "Amount", confidence: 0.97 },
      // Phase 8: Person entity (Pass 2 — LLM found)
      { label: "Owner", value: "Alex Doe", entityType: "person", class: "Person", confidence: 0.91 },
      // Phase 8: Identifier with subclass (Aadhaar — Pass 1 + Pass 2 refinement)
      {
        label: "Aadhaar",
        value: "1234 5678 9012",
        entityType: "identifier",
        class: "Identifier",
        subclass: "aadhaar",
        confidence: 0.92,
      },
      // Phase 8: low-confidence OCR entity (< 0.7) — shown in "Also found" expander (D-15)
      {
        label: "PAN",
        value: "ABCPS123?Z",
        entityType: "identifier",
        class: "Identifier",
        subclass: "pan",
        confidence: 0.65, // OCR noise — below 0.7 threshold for "Also found"
      },
    ],
    thumbnailColor: "#8B5CF6",
  },
  {
    id: "doc-2",
    name: "Home_Insurance.pdf",
    path: "/Users/demo/Documents/Property/Home_Insurance.pdf",
    docType: "pdf",
    size: 1_572_864,
    createdAt: "2025-01-03T00:00:00Z",
    modifiedAt: "2025-01-03T00:00:00Z",
    excerpt: "Homeowners Insurance Policy -- Coverage Summary...",
    spaceIds: ["space-property", "space-property-insurance"],
    tags: ["insurance", "property"],
    isFavorite: true,
    topic: "property",
    extractedEntities: [],
    thumbnailColor: "#7C3AED",
  },
  {
    id: "doc-3",
    name: "School_Report.pdf",
    path: "/Users/demo/Documents/Kids/School_Report.pdf",
    docType: "pdf",
    size: 524_288,
    createdAt: "2026-02-10T00:00:00Z",
    modifiedAt: "2026-02-10T00:00:00Z",
    excerpt: "Semester progress report for Spring 2026...",
    spaceIds: ["space-kids"],
    tags: ["school", "kids"],
    isFavorite: false,
    topic: "kids",
    extractedEntities: [],
    thumbnailColor: "#10B981",
  },
  {
    id: "doc-4",
    name: "Invoice_Feb2026.pdf",
    path: "/Users/demo/Documents/Invoices/Invoice_Feb2026.pdf",
    docType: "pdf",
    size: 348_160,
    createdAt: "2026-02-20T00:00:00Z",
    modifiedAt: "2026-02-20T00:00:00Z",
    excerpt: "Invoice #INV-2026-0214 for professional services...",
    spaceIds: ["space-invoices"],
    tags: ["invoice", "2026"],
    isFavorite: false,
    topic: "finance",
    extractedEntities: [
      { label: "Amount", value: "$1,500.00", entityType: "amount" },
      { label: "Date", value: "Feb 20, 2026", entityType: "date" },
    ],
    thumbnailColor: "#F59E0B",
  },
];

// ---------------------------------------------------------------------------
// Phase 8 Plan 09: mockTopics — browser-dev fallback for useTopics() hook
// ---------------------------------------------------------------------------

/**
 * Top topics by document count — used by TopicFilterBar in browser dev mode.
 * Entries cover topics that appear in mockDocuments so client-side filter matches:
 *   finance  → doc-1 (Property_Tax_2025) + doc-4 (Invoice_Feb2026)
 *   property → doc-2 (Home_Insurance)
 *   kids     → doc-3 (School_Report)
 * identity and vehicle appear in a full corpus but have no matching mock docs;
 * selecting them shows 0 results in browser mode (expected — filter still works).
 * Sort: count DESC, topic ASC (matches aggregate_topics output contract).
 */
export const mockTopics: TopicCount[] = [
  { topic: "finance", count: 12 },
  { topic: "identity", count: 8 },
  { topic: "vehicle", count: 5 },
  { topic: "kids", count: 4 },
  { topic: "property", count: 3 },
];

export const mockTags: Tag[] = [
  { id: "tag-tax", name: "tax", color: "#8B5CF6", documentCount: 45, tagType: "auto" },
  { id: "tag-property", name: "property", color: "#7C3AED", documentCount: 124, tagType: "auto" },
  { id: "tag-2025", name: "2025", color: "#6D28D9", documentCount: 234, tagType: "auto" },
  { id: "tag-invoice", name: "invoice", color: "#F59E0B", documentCount: 213, tagType: "auto" },
  { id: "tag-insurance", name: "insurance", color: "#3B82F6", documentCount: 18, tagType: "auto" },
  { id: "tag-school", name: "school", color: "#10B981", documentCount: 34, tagType: "user" },
  { id: "tag-kids", name: "kids", color: "#14B8A6", documentCount: 87, tagType: "user" },
  { id: "tag-medical", name: "medical", color: "#EF4444", documentCount: 87, tagType: "auto" },
];

export const mockWatchedFolders: WatchedFolder[] = [
  {
    id: "folder-1",
    path: "/Users/demo/Documents",
    documentCount: 2_340,
    lastScan: new Date(Date.now() - 2 * 60 * 1000).toISOString(),
    status: "watching",
  },
  {
    id: "folder-2",
    path: "/Users/demo/Desktop",
    documentCount: 45,
    lastScan: new Date(Date.now() - 5 * 60 * 1000).toISOString(),
    status: "watching",
  },
  {
    id: "folder-3",
    path: "/Users/demo/Downloads",
    documentCount: 128,
    lastScan: new Date(Date.now() - 15 * 60 * 1000).toISOString(),
    status: "paused",
  },
];

export const mockSearchResults: SearchResult[] = mockDocuments.map((doc, i) => ({
  document: doc,
  score: 0.95 - i * 0.1,
  matchedExcerpt: doc.excerpt ?? "",
}));

export const mockSpaceGraph: SpaceGraph = {
  nodes: mockSpaces.map((s) => ({
    id: s.id,
    name: s.name,
    documentCount: s.documentCount,
    color: s.color,
  })),
  edges: [
    { source: "space-property", target: "space-invoices", weight: 0.6 },
    { source: "space-work", target: "space-invoices", weight: 0.8 },
    { source: "space-kids", target: "space-medical", weight: 0.4 },
  ],
};

export const mockSearchAnalytics: SearchAnalytics = {
  totalSearches: 142,
  topQueries: [
    { query: "property tax 2025", count: 12 },
    { query: "invoice February", count: 8 },
    { query: "school report spring", count: 5 },
    { query: "medical records", count: 4 },
  ],
  avgResultsPerQuery: 8.5,
  queriesThisWeek: 34,
};

export const mockActivityItems: ActivityItem[] = [
  {
    id: "activity-1",
    action: "indexed",
    subject: "3 new documents added today",
    type: "info",
    timestamp: new Date(Date.now() - 30 * 60 * 1000).toISOString(),
  },
  {
    id: "activity-2",
    action: "moved",
    subject: '"Tax 2025" space updated',
    type: "info",
    timestamp: new Date(Date.now() - 2 * 60 * 60 * 1000).toISOString(),
  },
  {
    id: "activity-3",
    action: "moved",
    subject: "12 documents re-categorized",
    type: "success",
    timestamp: new Date(Date.now() - 4 * 60 * 60 * 1000).toISOString(),
  },
  {
    id: "activity-4",
    action: "indexed",
    subject: "Scan completed: ~/Documents",
    type: "success",
    timestamp: new Date(Date.now() - 6 * 60 * 60 * 1000).toISOString(),
  },
];

// ---------------------------------------------------------------------------
// Entity mock data — used as browser-dev fallback in entity hooks
// ---------------------------------------------------------------------------

export const mockEntities: EntitySummary[] = [
  // person (2)
  { id: "entity-person-1", canonicalName: "John Smith", entityType: "person", documentCount: 12 },
  { id: "entity-person-2", canonicalName: "Jane Doe", entityType: "person", documentCount: 7 },
  // organization (2)
  { id: "entity-org-1", canonicalName: "Acme Corp", entityType: "organization", documentCount: 8 },
  { id: "entity-org-2", canonicalName: "Initcron Inc", entityType: "organization", documentCount: 3 },
  // location (2)
  { id: "entity-loc-1", canonicalName: "Brooklyn", entityType: "location", documentCount: 5 },
  { id: "entity-loc-2", canonicalName: "San Francisco", entityType: "location", documentCount: 9 },
  // date (2)
  { id: "entity-date-1", canonicalName: "2024-03-15", entityType: "date", documentCount: 4 },
  { id: "entity-date-2", canonicalName: "2025-01-01", entityType: "date", documentCount: 6 },
  // amount (2)
  { id: "entity-amount-1", canonicalName: "$1,200", entityType: "amount", documentCount: 2 },
  { id: "entity-amount-2", canonicalName: "$4,200.00", entityType: "amount", documentCount: 3 },
  // email (2)
  { id: "entity-email-1", canonicalName: "support@example.com", entityType: "email", documentCount: 1 },
  { id: "entity-email-2", canonicalName: "john.smith@acme.com", entityType: "email", documentCount: 2 },
];

export const mockRelatedEntities: Record<string, RelatedEntity[]> = {
  "entity-person-1": [
    {
      entity: { id: "entity-org-1", canonicalName: "Acme Corp", entityType: "organization", documentCount: 8 },
      coOccurrenceCount: 6,
    },
    {
      entity: { id: "entity-loc-1", canonicalName: "Brooklyn", entityType: "location", documentCount: 5 },
      coOccurrenceCount: 3,
    },
  ],
  "entity-org-1": [
    {
      entity: { id: "entity-person-1", canonicalName: "John Smith", entityType: "person", documentCount: 12 },
      coOccurrenceCount: 6,
    },
    {
      entity: { id: "entity-email-1", canonicalName: "support@example.com", entityType: "email", documentCount: 1 },
      coOccurrenceCount: 2,
    },
  ],
};

export const defaultSettings: Settings = {
  theme: "dark",
  sidebarCollapsed: false,
  embeddingModel: "local",
  watchedFolders: ["/Users/demo/Documents", "/Users/demo/Desktop", "/Users/demo/Downloads"],
  excludedPatterns: [".git", "node_modules", ".DS_Store"],
  indexOnStartup: true,
  indexSize: 0,
  storagePath: "~/Library/Application Support/com.cortex.app/vectors",
  // Phase 8: extraction settings defaults — match Rust default_settings() (D-11, D-33)
  extractionModel: "",
  useLlmExtraction: true,
};

// ---------------------------------------------------------------------------
// Phase 8: Extraction settings mock data
// ---------------------------------------------------------------------------

/**
 * Default extraction settings returned by get_extraction_settings IPC in browser dev mode.
 * extractionModel="" means "use provider default" (backend resolves to fast-tier model).
 * useLlmExtraction=true matches Rust default_settings() — on when provider connected (D-33).
 */
export const mockExtractionSettings: ExtractionSettings = {
  extractionModel: "",
  useLlmExtraction: true,
};

/**
 * ExtractedEntities container for mockDocuments[0] (Property Tax 2025).
 * Used by Plan 08 Document detail sidebar to render topic + tags + entity list.
 * entitiesVersion=3 means Pass 1 + Pass 2 complete (D-23).
 */
export const mockDocumentEntitiesContainer: ExtractedEntities = {
  entities: mockDocuments[0].extractedEntities,
  topic: "identity",
  tags: ["aadhaar", "personal_id", "property_tax"],
  entitiesVersion: 3,
  language: "en",
};
