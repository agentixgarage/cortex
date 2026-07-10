/**
 * React Query hooks for all Tauri IPC commands.
 *
 * Each query hook falls back to mock data in browser mode (isTauri() === false).
 * Each mutation hook uses useMutation with appropriate query invalidation.
 *
 * Usage:
 *   const { data: spaces, isLoading } = useSpaces();
 *   const { data: stats } = useStats();
 *   const { mutate: updateSettings } = useUpdateSettings();
 */

import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { tauriInvoke, isTauri } from "../lib/tauri";
import {
  mockSpaces,
  mockDocuments,
  mockStats,
  mockWatchedFolders,
  mockTags,
  mockSearchResults,
  mockSpaceGraph,
  mockSearchAnalytics,
  mockActivityItems,
  defaultSettings,
  mockEntities,
  mockRelatedEntities,
  mockProviders,
  mockExtractionSettings,
  mockTopics,
} from "../lib/mock-data";
import type {
  Document,
  Space,
  Tag,
  WatchedFolder,
  Stats,
  SearchFilters,
  SearchResult,
  SpaceGraph,
  SearchAnalytics,
  ScanProgress,
  ActivityItem,
  Settings,
  DocumentMeta,
  EntitySummary,
  CanonicalEntity,
  RelatedEntity,
  ProviderAuthStatus,
  ConnectProviderRequest,
  OAuthStartResult,
  ExtractionSettings,
  TopicCount,
  SpaceLabelEntry,
  // Phase 11 types (Plan 01) — consumed by hooks in Plans 07/08/09
  SavedSearch,
  SavedSearchFilters,
  EntityPageData,
  RelatedDocScored,
  // Phase 11.5 types (Plan 01) — consumed by hooks in this plan (11.5-06)
  Triple,
  RelationsPageData,
  OwnershipPageData,
  PredicateSubjectPair,
  PredicateObjectPair,
  // Phase 11.6 types (Plan 01) — kept for Plan 09's benefit (hooks not added here)
  OntologyStoreSchema,
  PendingConsolidation,
  ProminentEntity,
  // v1.2 #2: User profile (onboarding "About You")
  UserProfile,
  // v1.2 #3: Daily quiz
  QuizQuestion,
  QuizAnswer,
} from "../lib/types";

// --- Query Keys ---------------------------------------------------------------

export const queryKeys = {
  spaces: ["spaces"] as const,
  spaceDocuments: (spaceId: string) => ["spaces", spaceId, "documents"] as const,
  spaceGraph: ["spaces", "graph"] as const,
  document: (id: string) => ["documents", id] as const,
  relatedDocuments: (id: string) => ["documents", id, "related"] as const,
  recentDocuments: ["documents", "recent"] as const,
  favoriteDocuments: ["documents", "favorites"] as const,
  documentText: (id: string) => ["documents", id, "text"] as const,
  search: (query: string, filters: SearchFilters) => ["search", query, filters] as const,
  searchAnalytics: ["search", "analytics"] as const,
  stats: ["stats"] as const,
  watchedFolders: ["watched-folders"] as const,
  tags: ["tags"] as const,
  activityFeed: ["activity-feed"] as const,
  settings: ["settings"] as const,
  entities: ["entities"] as const,
  entitiesByType: (type: string) => ["entities", "byType", type] as const,
  entity: (id: string) => ["entities", id] as const,
  entityDocuments: (id: string) => ["entities", id, "documents"] as const,
  relatedEntities: (id: string) => ["entities", id, "related"] as const,
  providers: ["providers"] as const,
  activeProvider: ["providers", "active"] as const,
  extractionSettings: ["extraction-settings"] as const,
  topics: ["topics"] as const,
  spaceLabels: ["space-labels"] as const,
  // --- Phase 11: Entity-Driven Exploration query keys ---
  // savedSearchCounts: sorted-joined id list prevents cache collision when two Sidebar
  // mounts have different ID sets (11-RESEARCH.md pitfall #6).
  savedSearches: ["saved-searches"] as const,
  savedSearchCounts: (ids: string[]) => ["saved-searches", "counts", [...ids].sort().join(",")] as const,
  entityPage: (cls: string, value: string, page: number) => ["entity-page", cls, value, page] as const,
  relatedDocsScored: (docId: string) => ["documents", docId, "related-scored"] as const,
  // --- Phase 11.5: Ontology / Relation Extraction query keys ---
  // entityRelations / entityOwnership are separate keys since they hit different
  // IPC endpoints and have different cache semantics (ownership is grouped by
  // asset type; related-to is a flat list per D-15). The two triples-by-* keys
  // support the exploratory helper queries per D-14 / D-15.
  entityRelations: (entityId: string) => ["entity-relations", entityId] as const,
  entityOwnership: (personId: string) => ["entity-ownership", personId] as const,
  entityRelatedTo: (entityId: string) => ["entity-related-to", entityId] as const,
  triplesByPredicateObject: (predicate: string, objectId: string) =>
    ["triples-by-predicate-object", predicate, objectId] as const,
  triplesBySubjectPredicate: (subjectId: string, predicate: string) =>
    ["triples-by-subject-predicate", subjectId, predicate] as const,
  // --- Phase 11.6: Adaptive Ontology query keys ---
  ontology: ["ontology"] as const,
  pendingConsolidation: ["ontology", "pending-consolidation"] as const,
  prominentEntities: ["entities", "prominent"] as const,
  // --- v1.2 #2: User profile ---
  userProfile: ["user-profile"] as const,
  // --- v1.2 #3: Daily quiz ---
  dailyQuiz: (limit: number) => ["daily-quiz", limit] as const,
};

// --- Space Hooks --------------------------------------------------------------

/**
 * Fetches all Smart Spaces (auto-organized virtual folders).
 */
export function useSpaces() {
  return useQuery({
    queryKey: queryKeys.spaces,
    queryFn: () => tauriInvoke<Space[]>("get_spaces", {}, () => mockSpaces),
  });
}

/**
 * Fetches documents belonging to a specific space.
 */
export function useSpaceDocuments(spaceId: string) {
  return useQuery({
    queryKey: queryKeys.spaceDocuments(spaceId),
    queryFn: () =>
      tauriInvoke<Document[]>(
        "get_space_documents",
        { spaceId },
        () => mockDocuments.filter((d) => d.spaceIds.includes(spaceId)),
      ),
    enabled: Boolean(spaceId),
  });
}

/**
 * Fetches the space relationship graph for visualization.
 */
export function useSpaceGraph() {
  return useQuery({
    queryKey: queryKeys.spaceGraph,
    queryFn: () => tauriInvoke<SpaceGraph>("get_space_graph", {}, () => mockSpaceGraph),
  });
}

/**
 * Triggers a re-clustering of spaces based on current document embeddings.
 */
export function useReclusterSpaces() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: () => tauriInvoke<Space[]>("recluster_spaces", {}, () => mockSpaces),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.spaces });
      queryClient.invalidateQueries({ queryKey: queryKeys.spaceGraph });
    },
  });
}

// === Phase 9 Plan 05: Space Labeling Hooks ===
// These hooks expose the four IPC commands added by Plan 03 (Rust backend).
// All mutations invalidate queryKeys.spaces + queryKeys.spaceLabels on success so
// SpacesPage and SpaceDetailPage re-render with current label data.
// IPC arg names are camelCase — Tauri v2 serde converts to snake_case Rust params.
// No onError handlers here — UI callers (Plan 07) wire toast.error(err.message).

/**
 * Fetches the full label-cache map (fingerprint → SpaceLabelEntry) for all spaces.
 * Returns an empty object in mock/browser mode.
 * Used by SpaceCard (description tooltip) and SpaceDetailPage (rename UI).
 */
export function useSpaceLabels() {
  return useQuery({
    queryKey: queryKeys.spaceLabels,
    queryFn: () =>
      tauriInvoke<Record<string, SpaceLabelEntry>>("get_space_labels", {}, () => ({})),
  });
}

/**
 * Renames a space label and locks it against future LLM re-labeling.
 * Sets Space.user_locked = true in the backend cache (D-15).
 * Invalidates spaces + spaceLabels on success.
 */
export function useRenameSpace() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({
      spaceId,
      label,
      description,
    }: {
      spaceId: string;
      label: string;
      description?: string;
    }) =>
      tauriInvoke<SpaceLabelEntry>(
        "rename_space_label",
        { spaceId, label, description },
        () => ({
          fingerprint: "mock",
          label,
          description: description ?? "",
          canonicalEntityHint: undefined,
          generatedAt: new Date().toISOString(),
          userLocked: true,
        }),
      ),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.spaces });
      queryClient.invalidateQueries({ queryKey: queryKeys.spaceLabels });
    },
  });
}

/**
 * Clears a user-supplied space label override, reverting to LLM-generated label.
 * Sets Space.user_locked = false in the backend cache (D-15 reset via "Clear override").
 * Invalidates spaces + spaceLabels on success.
 */
export function useClearSpaceOverride() {
  const queryClient = useQueryClient();
  return useMutation({
    // Rust clear_space_override returns Result<(), AppError> which serialises as null/void.
    mutationFn: (spaceId: string) =>
      tauriInvoke<void>("clear_space_override", { spaceId }, () => undefined),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.spaces });
      queryClient.invalidateQueries({ queryKey: queryKeys.spaceLabels });
    },
  });
}

/**
 * Triggers a single-space LLM re-label (force re-generate even if fingerprint unchanged).
 * Progress arrives via "space-labeling-progress" Tauri event (useSpaceLabelingProgress).
 * Throws in mock/browser mode — plan 09-06 wraps with try/catch + toast.
 * Invalidates spaces + spaceLabels on success.
 */
export function useRelabelSpace() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (spaceId: string) =>
      tauriInvoke<Space>("trigger_relabel", { spaceId }, () => {
        throw new Error("Cannot relabel in mock runtime");
      }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.spaces });
      queryClient.invalidateQueries({ queryKey: queryKeys.spaceLabels });
    },
  });
}

/**
 * Moves a document to a different space.
 */
export function useMoveDocumentToSpace() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ docId, spaceId }: { docId: string; spaceId: string }) =>
      tauriInvoke<void>("move_document_to_space", { docId, spaceId }, () => undefined),
    onSuccess: (_data, { spaceId }) => {
      queryClient.invalidateQueries({ queryKey: queryKeys.spaces });
      queryClient.invalidateQueries({ queryKey: queryKeys.spaceDocuments(spaceId) });
      queryClient.invalidateQueries({ queryKey: queryKeys.recentDocuments });
    },
  });
}

// --- Document Hooks -----------------------------------------------------------

/**
 * Fetches a single document by ID.
 */
export function useDocument(id: string) {
  return useQuery({
    queryKey: queryKeys.document(id),
    queryFn: () =>
      tauriInvoke<Document>(
        "get_document",
        { id },
        () => mockDocuments.find((d) => d.id === id) ?? mockDocuments[0],
      ),
    enabled: Boolean(id),
  });
}

/**
 * Fetches documents related to a given document (by embedding similarity).
 */
export function useRelatedDocuments(id: string, limit = 5) {
  return useQuery({
    queryKey: queryKeys.relatedDocuments(id),
    queryFn: () =>
      tauriInvoke<Document[]>(
        "get_related_documents",
        { id, limit },
        () => mockDocuments.filter((d) => d.id !== id).slice(0, limit),
      ),
    enabled: Boolean(id),
  });
}

/**
 * Fetches recent documents, sorted by modification date (newest first).
 */
export function useRecentDocuments(limit = 50) {
  return useQuery({
    queryKey: [...queryKeys.recentDocuments, limit],
    queryFn: () =>
      tauriInvoke<Document[]>(
        "get_recent_documents",
        { limit },
        () =>
          [...mockDocuments]
            .sort((a, b) => new Date(b.modifiedAt).getTime() - new Date(a.modifiedAt).getTime())
            .slice(0, limit),
      ),
  });
}

/**
 * Fetches documents marked as favorite.
 */
export function useFavoriteDocuments() {
  return useQuery({
    queryKey: queryKeys.favoriteDocuments,
    queryFn: () =>
      tauriInvoke<Document[]>(
        "get_favorite_documents",
        {},
        () => mockDocuments.filter((d) => d.isFavorite),
      ),
  });
}

/**
 * Indexes a new document from a file path.
 */
export function useIndexDocument() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (path: string) =>
      tauriInvoke<DocumentMeta>("index_document", { path }, () => ({
        id: `doc-${Date.now()}`,
        name: path.split("/").pop() ?? "unknown",
        path,
        docType: "other",
        size: 0,
        createdAt: new Date().toISOString(),
        modifiedAt: new Date().toISOString(),
      })),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.stats });
      queryClient.invalidateQueries({ queryKey: queryKeys.recentDocuments });
      queryClient.invalidateQueries({ queryKey: queryKeys.spaces });
    },
  });
}

/**
 * Toggles the favorite status of a document.
 */
export function useToggleFavorite() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (docId: string) =>
      tauriInvoke<void>("toggle_favorite", { docId }, () => undefined),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.favoriteDocuments });
      queryClient.invalidateQueries({ queryKey: queryKeys.recentDocuments });
    },
  });
}

// --- Search Hooks -------------------------------------------------------------

/**
 * Searches documents using semantic similarity.
 * Only fires when query has content.
 */
export function useDocumentSearch(query: string, filters: SearchFilters = {}) {
  return useQuery({
    queryKey: queryKeys.search(query, filters),
    queryFn: () =>
      tauriInvoke<SearchResult[]>(
        "search_documents",
        { query, filters },
        () =>
          mockSearchResults.filter((r) =>
            r.document.name.toLowerCase().includes(query.toLowerCase()),
          ),
      ),
    enabled: query.length > 0,
  });
}

/**
 * Records a search result click for analytics and self-learning.
 */
export function useRecordSearchClick() {
  return useMutation({
    mutationFn: ({ query, documentId }: { query: string; documentId: string }) =>
      tauriInvoke<void>(
        "record_search_click",
        { query, documentId },
        () => undefined,
      ),
  });
}

/**
 * Fetches search analytics (top queries, usage stats).
 */
export function useSearchAnalytics() {
  return useQuery({
    queryKey: queryKeys.searchAnalytics,
    queryFn: () =>
      tauriInvoke<SearchAnalytics>("get_search_analytics", {}, () => mockSearchAnalytics),
  });
}

// --- Stats & Activity Hooks ---------------------------------------------------

/**
 * Fetches high-level document and space statistics.
 */
export function useStats() {
  return useQuery({
    queryKey: queryKeys.stats,
    queryFn: () => tauriInvoke<Stats>("get_stats", {}, () => mockStats),
    refetchInterval: 30_000, // poll every 30s to reflect indexing progress
  });
}

/**
 * Fetches recent activity events (indexing, clustering, user actions).
 */
export function useActivityFeed() {
  return useQuery({
    queryKey: queryKeys.activityFeed,
    queryFn: () =>
      tauriInvoke<ActivityItem[]>("get_activity_feed", {}, () => mockActivityItems),
    refetchInterval: 15_000, // poll every 15s
  });
}

// --- Watched Folder Hooks -----------------------------------------------------

/**
 * Fetches the list of watched folders.
 */
export function useWatchedFolders() {
  return useQuery({
    queryKey: queryKeys.watchedFolders,
    queryFn: () =>
      tauriInvoke<WatchedFolder[]>("get_watched_folders", {}, () => mockWatchedFolders),
  });
}

/**
 * Adds a new folder to the watch list.
 */
export function useAddWatchedFolder() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (path: string) =>
      tauriInvoke<WatchedFolder>(
        "add_watched_folder",
        { path },
        () => ({
          id: `folder-${Date.now()}`,
          path,
          documentCount: 0,
          lastScan: new Date().toISOString(),
          status: "watching",
        }),
      ),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.watchedFolders });
    },
  });
}

/**
 * Removes a folder from the watch list.
 */
export function useRemoveWatchedFolder() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: string) =>
      tauriInvoke<void>("remove_watched_folder", { id }, () => undefined),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.watchedFolders });
    },
  });
}

/**
 * Triggers an immediate scan of a watched folder.
 */
export function useTriggerScan() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (folderId: string) =>
      tauriInvoke<ScanProgress>(
        "trigger_scan",
        { folderId },
        () => ({
          folderId,
          totalFiles: 100,
          processedFiles: 100,
          status: "complete",
        }),
      ),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.watchedFolders });
      queryClient.invalidateQueries({ queryKey: queryKeys.stats });
      queryClient.invalidateQueries({ queryKey: queryKeys.recentDocuments });
    },
  });
}

// --- Tag Hooks ----------------------------------------------------------------

/**
 * Fetches all document tags (auto-generated + user-created).
 */
export function useTags() {
  return useQuery({
    queryKey: queryKeys.tags,
    queryFn: () => tauriInvoke<Tag[]>("get_tags", {}, () => mockTags),
  });
}

// --- Settings Hooks -----------------------------------------------------------

/**
 * Fetches the current application settings.
 */
export function useSettings() {
  return useQuery({
    queryKey: queryKeys.settings,
    queryFn: () => tauriInvoke<Settings>("get_settings", {}, () => defaultSettings),
  });
}

/**
 * Persists updated application settings.
 */
export function useUpdateSettings() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (settings: Settings) =>
      tauriInvoke<void>("update_settings", { settings }, () => undefined),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.settings });
    },
  });
}

// --- Entity Mutation Hooks (Plan 06-07) ----------------------------------------

/**
 * Renames a canonical entity name.
 * Invalidates the entity by ID and the full entities list on success.
 */
export function useRenameEntityCanonical() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ id, newName }: { id: string; newName: string }) =>
      tauriInvoke<CanonicalEntity>(
        "rename_entity_canonical",
        { id, newName },
        () => undefined as never,
      ),
    onSuccess: (_, { id }) => {
      queryClient.invalidateQueries({ queryKey: queryKeys.entity(id) });
      queryClient.invalidateQueries({ queryKey: queryKeys.entities });
    },
  });
}

/**
 * Splits an alias off into a new canonical entity.
 * Invalidates entity, its documents, and the full entities list on success.
 */
export function useSplitEntityAlias() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ canonicalId, alias }: { canonicalId: string; alias: string }) =>
      tauriInvoke<CanonicalEntity>(
        "split_entity_alias",
        { canonicalId, alias },
        () => undefined as never,
      ),
    onSuccess: (_, { canonicalId }) => {
      queryClient.invalidateQueries({ queryKey: queryKeys.entity(canonicalId) });
      queryClient.invalidateQueries({ queryKey: queryKeys.entityDocuments(canonicalId) });
      queryClient.invalidateQueries({ queryKey: queryKeys.entities });
    },
  });
}

// --- Entity Hooks (Plan 06-06) -----------------------------------------------

/**
 * Fetches all canonical entities (all types).
 */
export function useEntities() {
  return useQuery({
    queryKey: queryKeys.entities,
    queryFn: () =>
      tauriInvoke<EntitySummary[]>("get_entities_by_type", { entityType: undefined }, () => mockEntities),
  });
}

/**
 * Fetches canonical entities filtered by type.
 * Only fires when type is a non-empty string.
 */
export function useEntitiesByType(type: string) {
  return useQuery({
    queryKey: queryKeys.entitiesByType(type),
    queryFn: () =>
      tauriInvoke<EntitySummary[]>(
        "get_entities_by_type",
        { entityType: type },
        () => mockEntities.filter((e) => e.entityType === type),
      ),
    enabled: Boolean(type),
  });
}

/**
 * Fetches a single canonical entity by ID.
 * Only fires when id is a non-empty string.
 */
export function useEntity(id: string) {
  return useQuery({
    queryKey: queryKeys.entity(id),
    queryFn: () =>
      tauriInvoke<CanonicalEntity>(
        "get_entity",
        { id },
        () => ({
          ...(mockEntities[0]),
          aliases: [mockEntities[0].canonicalName],
        }),
      ),
    enabled: Boolean(id),
  });
}

/**
 * Fetches all documents mentioning a specific canonical entity.
 * Only fires when id is a non-empty string.
 */
export function useEntityDocuments(id: string) {
  return useQuery({
    queryKey: queryKeys.entityDocuments(id),
    queryFn: () =>
      tauriInvoke<Document[]>("get_documents_for_entity", { id }, () => []),
    enabled: Boolean(id),
  });
}

/**
 * Fetches related entities for a canonical entity, ranked by co-occurrence count.
 * Only fires when id is a non-empty string.
 */
export function useRelatedEntities(id: string, min?: number, limit?: number) {
  return useQuery({
    queryKey: queryKeys.relatedEntities(id),
    queryFn: () =>
      tauriInvoke<RelatedEntity[]>(
        "get_related_entities",
        { id, minCoOccurrence: min, limit },
        () => mockRelatedEntities[id] ?? [],
      ),
    enabled: Boolean(id),
  });
}

// === Phase 7: AI Provider Hooks ===
// All hooks go through tauriInvoke which gates on window.__TAURI__ (mock fallback for browser dev).
// Mutations invalidate queryKeys.providers so the Settings page and AppShell banner re-render on state change.
// No onError handlers in hooks — callers (Plans 05/06) handle errors with toast.error(err.message).

/**
 * Fetches the list of all AI provider authentication statuses.
 * Returns ProviderAuthStatus[] — one entry per supported provider.
 */
export function useProviders() {
  return useQuery({
    queryKey: queryKeys.providers,
    queryFn: () => tauriInvoke<ProviderAuthStatus[]>("list_providers", {}, () => mockProviders),
    staleTime: 30_000, // credential status doesn't change without explicit user action
  });
}

/**
 * Connects an AI provider using an API key or Ollama endpoint.
 * Validates credentials before storing (D-08 enforce-before-store contract).
 */
export function useConnectProvider() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (request: ConnectProviderRequest) =>
      tauriInvoke<ProviderAuthStatus>("connect_provider", { request }, () => mockProviders[0]),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.providers });
      queryClient.invalidateQueries({ queryKey: queryKeys.activeProvider });
    },
  });
}

/**
 * Disconnects an AI provider and removes its stored credentials.
 */
export function useDisconnectProvider() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (provider: string) =>
      tauriInvoke<void>("disconnect_provider", { provider }, () => undefined as void),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.providers });
      queryClient.invalidateQueries({ queryKey: queryKeys.activeProvider });
    },
  });
}

/**
 * Sets the active AI provider (the one used for chat and Smart Space naming).
 */
export function useSetActiveProvider() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (provider: string) =>
      tauriInvoke<void>("set_active_provider", { provider }, () => undefined as void),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.providers });
      queryClient.invalidateQueries({ queryKey: queryKeys.activeProvider });
    },
  });
}

/**
 * Saves an Anthropic OAuth setup token (sk-ant-oat01-...) as the Anthropic credential.
 * Validates the token against the Anthropic API before storing.
 */
export function useSaveSetupToken() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (token: string) =>
      tauriInvoke<OAuthStartResult>(
        "save_setup_token",
        { token },
        () => ({ started: true, provider: "anthropic" }),
      ),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.providers });
    },
  });
}

/**
 * Tests the connection to the currently active AI provider.
 * Sends a minimal 1-token request to verify the provider is reachable and the credential is valid.
 */
export function useTestConnection() {
  return useMutation({
    mutationFn: (provider: string) =>
      tauriInvoke<void>("test_connection", { provider }, () => undefined as void),
  });
}

/**
 * Starts the OpenAI Codex OAuth PKCE flow.
 * Opens the system browser to auth.openai.com, captures the loopback callback,
 * exchanges the code for tokens, and stores them under provider slug "openai-codex".
 *
 * Returns ProviderAuthStatus on success (authenticated=true, method="oauth", provider="openai-codex").
 * Rejects with an Error if the flow is cancelled, times out, or encounters a network error.
 *
 * onSuccess invalidates both `providers` and `activeProvider` query keys so Settings page
 * and AppShell banner re-render immediately after OAuth completes.
 */
export function useStartOpenAiOAuth() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: () =>
      tauriInvoke<ProviderAuthStatus>(
        "start_openai_oauth",
        {},
        () =>
          mockProviders.find((p) => p.provider === "openai-codex") ?? {
            provider: "openai-codex",
            authenticated: true,
            method: "oauth",
            displayName: "ChatGPT (Codex)",
            model: "gpt-4o",
            isActive: false,
          },
      ),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.providers });
      queryClient.invalidateQueries({ queryKey: queryKeys.activeProvider });
    },
  });
}

// === Phase 8: LLM Entity Extraction Hooks ===
// All hooks go through tauriInvoke which gates on isTauri() (mock fallback for browser dev).
// IPC commands delivered by Plan 05: get_extraction_settings, set_extraction_settings, trigger_entity_backfill.
// No onError handlers in hooks — callers (Plan 07 ExtractionSettings component) handle errors with toast.error().

/**
 * Fetches the extraction settings (extractionModel + useLlmExtraction).
 * Browser fallback returns mockExtractionSettings (extractionModel="", useLlmExtraction=true).
 * Tauri path invokes get_extraction_settings returning ExtractionSettings.
 */
export function useExtractionSettings() {
  return useQuery({
    queryKey: queryKeys.extractionSettings,
    queryFn: () =>
      tauriInvoke<ExtractionSettings>(
        "get_extraction_settings",
        {},
        () => mockExtractionSettings,
      ),
  });
}

/**
 * Persists updated extraction settings (model + toggle).
 * On success, invalidates extractionSettings query so Settings page re-renders
 * with the saved values.
 *
 * No onError handler — Plan 07 ExtractionSettings component wires toast.error().
 */
export function useUpdateExtractionSettings() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (settings: ExtractionSettings) =>
      tauriInvoke<void>("set_extraction_settings", { settings }, () => undefined),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.extractionSettings });
    },
  });
}

/**
 * Triggers entity backfill across all docs with entities_version < 3.
 * Fire-and-forget: progress arrives via existing entity-backfill-progress Tauri event
 * (consumed by BackfillIndicator). Does NOT invalidate any queries.
 *
 * Browser mode (isTauri()=false): no-op, returns immediately.
 * No onError handler — Plan 07 Re-extract button wires toast.error() on mutate failure.
 */
export function useTriggerEntityBackfill() {
  return useMutation({
    mutationFn: async () => {
      if (!isTauri()) return;
      await tauriInvoke<void>("trigger_entity_backfill");
    },
  });
}

// === Phase 8 Plan 09: Topic filter hooks ===

/**
 * Fetches topic counts aggregated across all indexed documents.
 * Returns TopicCount[] sorted by count DESC then topic ASC.
 *
 * Browser mode fallback: mockTopics (5 entries covering finance, identity,
 * vehicle, kids, property — finance + property + kids have matching mockDocuments
 * so the TopicFilterBar client-side filter produces results in browser dev mode).
 *
 * Tauri mode: invokes get_topics IPC (aggregate_topics scan over documents_384).
 */
export function useTopics() {
  return useQuery({
    queryKey: queryKeys.topics,
    queryFn: () => tauriInvoke<TopicCount[]>("get_topics", {}, () => mockTopics),
  });
}

// === Phase 11 Plan 07: Entity-Driven Exploration Hooks ===
// Six hooks bridging SearchPage URL filters, saved searches, related docs, and entity detail page.
// Query keys are all pre-defined in Plan 01 (queryKeys.savedSearches, savedSearchCounts, etc.).
// Mock fallbacks return minimal valid shapes so browser-mode dev works without Rust backend.
// No onError handlers — callers (SaveSearchDialog, etc.) wire toast.error() on mutate failure.

/**
 * Fetches all saved searches from the sidecar store.
 * Browser fallback: empty array (no pre-seeded saved searches in dev mode).
 * staleTime 30s per D-08 (count refresh cadence).
 */
export function useSavedSearches() {
  return useQuery({
    queryKey: queryKeys.savedSearches,
    queryFn: () => tauriInvoke<SavedSearch[]>("get_saved_searches", {}, () => []),
    staleTime: 30_000,
  });
}

/**
 * Saves the current search (name + query + filters) to the sidecar store.
 * On success: invalidates savedSearches list AND all saved-search count entries
 * so the Sidebar re-renders with the new entry and correct doc counts (pitfall #4, 11-RESEARCH.md).
 * Mock: returns a synthetic SavedSearch with a timestamped id.
 */
export function useSaveSearch() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({
      name,
      query,
      filters,
    }: {
      name: string;
      query: string;
      filters: SavedSearchFilters;
    }) =>
      tauriInvoke<SavedSearch>(
        "save_search",
        { name, query, filters },
        () => ({
          id: `ss-mock-${Date.now()}`,
          name,
          query,
          filters,
          createdAt: new Date().toISOString(),
          docCountCache: 0,
        }),
      ),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.savedSearches });
      // WR-04 fix: use resetQueries (not invalidateQueries) so any stale count entry
      // is synchronously removed from cache. With invalidateQueries, the new saved
      // search (ss-N) appears in the Sidebar list before its count query key exists,
      // showing count 0 from doc_count_cache until the 30s staleTime expires.
      // resetQueries forces an immediate re-fetch with the new id set included.
      queryClient.resetQueries({ queryKey: ["saved-searches", "counts"] });
    },
  });
}

/**
 * Deletes a saved search by id from the sidecar store.
 * On success: invalidates savedSearches list AND all count cache entries.
 * Mock: no-op (returns void).
 */
export function useDeleteSavedSearch() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (id: string) =>
      tauriInvoke<void>("delete_saved_search", { id }, () => undefined),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.savedSearches });
      queryClient.invalidateQueries({ queryKey: ["saved-searches", "counts"] });
    },
  });
}

/**
 * Fetches live document counts for a batch of saved-search IDs.
 * Only fires when ids array is non-empty (skipped on initial Sidebar mount before searches load).
 * staleTime 30s per D-08 (count TTL — updates on next Sidebar mount after new docs indexed).
 * Mock: returns an empty map (all counts show as 0 in browser mode).
 */
export function useSavedSearchCounts(ids: string[]) {
  return useQuery({
    queryKey: queryKeys.savedSearchCounts(ids),
    queryFn: () =>
      tauriInvoke<Record<string, number>>("get_saved_search_counts", { ids }, () => ({})),
    staleTime: 30_000,
    enabled: ids.length > 0,
  });
}

/**
 * Fetches top-N related documents for a given document, scored by composite relevance.
 * score = 0.6 × cosine + 0.4 × entity_overlap_jaccard (D-10, D-11).
 * staleTime 5 min per D-13 (compute on demand; HNSW fast enough < 20ms on 10k corpus).
 * Only fires when docId is non-empty.
 * Mock: empty array (Related panel not shown in browser mode without real embeddings).
 */
export function useRelatedDocsScored(docId: string, topN = 5) {
  return useQuery({
    queryKey: queryKeys.relatedDocsScored(docId),
    queryFn: () =>
      tauriInvoke<RelatedDocScored[]>(
        "get_related_docs_scored",
        { docId, topN },
        () => [],
      ),
    staleTime: 5 * 60_000,
    enabled: Boolean(docId),
  });
}

/**
 * Fetches the full data payload for the /entity/:class/:value detail page.
 * Returns canonical entity metadata, paginated documents, and co-occurring entities.
 * Only fires when both cls and value are non-empty strings.
 * Note: Rust param is named `class`; passing `{ class: cls }` in JS object literal is legal
 * (`class` is not reserved as an object property key). Tauri v2 auto-serializes to snake_case.
 *
 * Mock: returns a minimal EntityPageData with the provided class/value so the EntityDetailPage
 * renders its empty-state path (totalDocumentCount: 0) without throwing in browser dev mode.
 */
export function useEntityPageData(cls: string, value: string, page = 0) {
  return useQuery({
    queryKey: queryKeys.entityPage(cls, value, page),
    queryFn: () =>
      tauriInvoke<EntityPageData>(
        "get_entity_page_data",
        { class: cls, value, page },
        () => ({
          canonical: {
            id: "mock",
            canonicalName: value,
            entityType: cls,
            aliases: [value],
            documentCount: 0,
          },
          documents: [],
          totalDocumentCount: 0,
          coOccurringEntities: [],
          page,
          pageSize: 20,
        }),
      ),
    enabled: Boolean(cls) && Boolean(value),
  });
}

// === Phase 11.5: Relation Extraction hooks ===
// Six read hooks + two mutation hooks — see Plan 11.5-06.
// Read hooks are backed by the 7 relation IPC commands from Plan 11.5-05.
// Mutation hooks invalidate entityRelations / entityOwnership queryKeys per D-13..D-16
// so the Relations panel + Ownership page re-render immediately after user edits.

/**
 * Fetches all relations touching a canonical entity (outgoing + incoming).
 * Backed by get_entity_relations IPC (Plan 11.5-05 command 1).
 * ONTO-03.
 */
export function useEntityRelations(entityId: string) {
  return useQuery({
    queryKey: queryKeys.entityRelations(entityId),
    queryFn: () =>
      tauriInvoke<RelationsPageData>(
        "get_entity_relations",
        { entityId },
        () => ({
          entity: {
            id: entityId,
            canonicalName: "",
            entityType: "",
            aliases: [],
            documentCount: 0,
          } as CanonicalEntity,
          outgoing: [],
          incoming: [],
        }),
      ),
    enabled: Boolean(entityId),
  });
}

/**
 * Fetches ownership assets grouped by AssetType for a person entity.
 * Backed by get_all_owned_by IPC (Plan 11.5-05 command 2).
 * ONTO-04.
 */
export function useAllOwnedBy(personId: string) {
  return useQuery({
    queryKey: queryKeys.entityOwnership(personId),
    queryFn: () =>
      tauriInvoke<OwnershipPageData>(
        "get_all_owned_by",
        { personId },
        () => ({
          person: {
            id: personId,
            canonicalName: "",
            entityType: "person",
            aliases: [],
            documentCount: 0,
          } as CanonicalEntity,
          assetsByType: {},
          totalAssets: 0,
        }),
      ),
    enabled: Boolean(personId),
  });
}

/**
 * Fetches a flat list of (predicate, other-entity) pairs for every triple
 * touching the entity (both directions). Distinct from useEntityRelations,
 * which returns structured outgoing/incoming buckets.
 * Backed by get_all_related_to IPC (Plan 11.5-05 command 3).
 * ONTO-03.
 */
export function useAllRelatedTo(entityId: string) {
  return useQuery({
    queryKey: queryKeys.entityRelatedTo(entityId),
    queryFn: () =>
      tauriInvoke<Array<[string, CanonicalEntity]>>(
        "get_all_related_to",
        { entityId },
        () => [],
      ),
    enabled: Boolean(entityId),
  });
}

/**
 * "What does {subject} {predicate}?" e.g. "What does Alex own?"
 * Backed by get_objects_by_subject_predicate IPC (Plan 11.5-05 command 5).
 */
export function useTriplesBySubjectPredicate(subjectId: string, predicate: string) {
  return useQuery({
    queryKey: queryKeys.triplesBySubjectPredicate(subjectId, predicate),
    queryFn: () =>
      tauriInvoke<PredicateObjectPair[]>(
        "get_objects_by_subject_predicate",
        { subjectId, predicate },
        () => [],
      ),
    enabled: Boolean(subjectId && predicate),
  });
}

/**
 * "Who {predicate} {object}?" e.g. "Who owns AlphaComplex?"
 * Backed by get_subjects_by_predicate_object IPC (Plan 11.5-05 command 4).
 */
export function useTriplesByPredicateObject(predicate: string, objectId: string) {
  return useQuery({
    queryKey: queryKeys.triplesByPredicateObject(predicate, objectId),
    queryFn: () =>
      tauriInvoke<PredicateSubjectPair[]>(
        "get_subjects_by_predicate_object",
        { predicate, objectId },
        () => [],
      ),
    enabled: Boolean(predicate && objectId),
  });
}

export interface AddManualTripleArgs {
  subjectId: string;
  predicate: string;
  objectId: string;
  docId?: string;
}

/**
 * Insert a user-created triple. Persists to triples.json immediately.
 * Invalidates entityRelations for both endpoints so the Relations panel refreshes.
 * ONTO-05.
 */
export function useAddManualTriple() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (args: AddManualTripleArgs) =>
      tauriInvoke<Triple>(
        "add_manual_triple",
        args as unknown as Record<string, unknown>,
        () => ({
          id: "t-mock",
          subjectId: args.subjectId,
          predicate: args.predicate as Triple["predicate"],
          objectId: args.objectId,
          docIds: args.docId ? [args.docId] : [],
          userAdded: true,
          createdAt: new Date().toISOString(),
        }),
      ),
    onSuccess: (_data, variables) => {
      // Invalidate the Relations panel query for both endpoints.
      queryClient.invalidateQueries({ queryKey: queryKeys.entityRelations(variables.subjectId) });
      queryClient.invalidateQueries({ queryKey: queryKeys.entityRelations(variables.objectId) });
      // Ownership counts change when 'owns' triples are added.
      if (variables.predicate === "owns") {
        queryClient.invalidateQueries({ queryKey: queryKeys.entityOwnership(variables.subjectId) });
      }
      if (variables.predicate === "owned_by") {
        queryClient.invalidateQueries({ queryKey: queryKeys.entityOwnership(variables.objectId) });
      }
    },
  });
}

export interface DeleteTripleArgs {
  tripleId: string;
  /** Optional entity ids whose Relations panels should be invalidated after delete. */
  affectedEntityIds?: string[];
}

/**
 * Delete a triple by id (and its auto-inverse partner, if any).
 * Invalidates entityRelations + entityOwnership for affected entities.
 * ONTO-05.
 */
export function useDeleteTriple() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (args: DeleteTripleArgs) =>
      tauriInvoke<null>(
        "delete_triple",
        { tripleId: args.tripleId },
        () => null,
      ),
    onSuccess: (_data, variables) => {
      (variables.affectedEntityIds ?? []).forEach((eid) => {
        queryClient.invalidateQueries({ queryKey: queryKeys.entityRelations(eid) });
        queryClient.invalidateQueries({ queryKey: queryKeys.entityOwnership(eid) });
      });
    },
  });
}

/**
 * Return the id of the top-document-count Person entity, or undefined when no
 * Person entities are indexed yet.
 * Backed by existing get_entities_by_type IPC (Phase 6). Used by Sidebar
 * "Owned by me" quick link (Plan 11.5-08).
 * ONTO-04.
 */
export function useTopPersonId(): string | undefined {
  const { data: persons } = useEntitiesByType("person");
  if (!persons || persons.length === 0) return undefined;
  // get_entities_by_type already sorts by document_count desc, but be defensive
  const top = [...persons].sort((a, b) => b.documentCount - a.documentCount)[0];
  return top?.id;
}

// --- v1.2 #2: User profile (onboarding "About You") ---------------------

const emptyUserProfile: UserProfile = {
  displayName: "",
  aliases: [],
  familyMembers: [],
  countries: [],
  currencies: [],
};

export function useUserProfile() {
  return useQuery({
    queryKey: queryKeys.userProfile,
    queryFn: () =>
      tauriInvoke<UserProfile>("get_user_profile", undefined, () => emptyUserProfile),
  });
}

export function useSaveUserProfile() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (profile: UserProfile) =>
      tauriInvoke<null>("save_user_profile", { profile }, () => null),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.userProfile });
    },
  });
}

// --- v1.2 #3: Daily quiz (occasional confirm/deny feedback loop) --------

export function useDailyQuiz(limit: number = 1) {
  return useQuery({
    queryKey: queryKeys.dailyQuiz(limit),
    queryFn: () =>
      tauriInvoke<QuizQuestion[]>("get_daily_quiz", { limit }, () => []),
  });
}

export function useSubmitQuizAnswer() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (answer: QuizAnswer) =>
      tauriInvoke<null>("submit_quiz_answer", { answer }, () => null),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["daily-quiz"] });
    },
  });
}
