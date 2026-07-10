/**
 * ExtractionSettings.tsx
 *
 * Settings → AI & Models tab: Entity Extraction section.
 * Contains three controls:
 *   1. Extraction model dropdown (per active provider, D-11 defaults)
 *   2. "Use LLM for entity extraction" toggle (D-33)
 *   3. "Re-extract entities" button (D-22) — primary focal point per UI-SPEC §Visual Hierarchy
 *
 * Cost estimate tooltip on the button uses in-lined pricing constants (D-22).
 * Model-switch toast fires when model changes and indexed doc count > 0.
 * Note: provider-switch toast lives in AiProvidersSection (D-30), NOT here.
 *
 * Plan 08-07
 */

import { useState, useEffect, useCallback, useRef } from "react";
import { Loader2 } from "lucide-react";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { Label } from "@/components/ui/label";
import { Button } from "@/components/ui/button";
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@/components/ui/tooltip";
import { toast } from "sonner";
import {
  useExtractionSettings,
  useUpdateExtractionSettings,
  useTriggerEntityBackfill,
  useProviders,
  useStats,
} from "@/hooks/useTauri";
import { useBackfillStore } from "@/lib/stores";

// ---------------------------------------------------------------------------
// Constants & Helpers
// ---------------------------------------------------------------------------

/** Average input tokens per document for cost estimate (stated assumption — user-adjustable in v1.2). */
const AVG_INPUT_TOKENS_PER_DOC = 2000;

/** Model pricing in USD per 1M input tokens. */
const MODEL_PRICING: Record<string, number> = {
  "claude-haiku-4-5-20251001": 0.80,
  "claude-sonnet-4-5": 3.00,
  "gpt-4o-mini": 0.15,
  "gpt-4o": 2.50,
  "gpt-4.1-mini": 0.40,
  "gpt-4.1": 3.00,
  "o1-mini": 3.00,
  "o1": 15.00,
  "gemini-2.5-flash": 0.075,
  "gemini-2.5-pro": 1.25,
};

/** Human-readable display names for model IDs. */
const MODEL_DISPLAY_NAMES: Record<string, string> = {
  "claude-haiku-4-5-20251001": "Claude Haiku 4.5",
  "claude-sonnet-4-5": "Claude Sonnet 4.5",
  "gpt-4o-mini": "GPT-4o mini",
  "gpt-4o": "GPT-4o",
  "gpt-4.1-mini": "GPT-4.1 mini",
  "gpt-4.1": "GPT-4.1",
  "o1-mini": "o1-mini",
  "o1": "o1",
  "gemini-2.5-flash": "Gemini 2.5 Flash",
  "gemini-2.5-pro": "Gemini 2.5 Pro",
};

/** Model options per provider slug.
 *  Note: openai-codex (ChatGPT subscription OAuth) has restricted model access —
 *  gpt-5 is Codex-only endpoint, NOT available via ChatGPT accounts.
 *  ChatGPT subscribers get gpt-4o, gpt-4o-mini, o1 family, gpt-4.1 family. */
const PROVIDER_MODELS: Record<string, { value: string; label: string }[]> = {
  anthropic: [
    { value: "claude-haiku-4-5-20251001", label: "Claude Haiku 4.5" },
    { value: "claude-sonnet-4-5", label: "Claude Sonnet 4.5" },
  ],
  openai: [
    { value: "gpt-4o-mini", label: "GPT-4o mini" },
    { value: "gpt-4o", label: "GPT-4o" },
    { value: "gpt-4.1-mini", label: "GPT-4.1 mini" },
    { value: "gpt-4.1", label: "GPT-4.1" },
    { value: "o1-mini", label: "o1-mini" },
    { value: "o1", label: "o1" },
  ],
  "openai-codex": [
    { value: "gpt-4o-mini", label: "GPT-4o mini" },
    { value: "gpt-4o", label: "GPT-4o" },
    { value: "gpt-4.1-mini", label: "GPT-4.1 mini" },
    { value: "gpt-4.1", label: "GPT-4.1" },
    { value: "o1-mini", label: "o1-mini" },
  ],
  gemini: [
    { value: "gemini-2.5-flash", label: "Gemini 2.5 Flash" },
    { value: "gemini-2.5-pro", label: "Gemini 2.5 Pro" },
  ],
};

/** Default model to use when settings.extractionModel is empty. */
const PROVIDER_DEFAULT_MODEL: Record<string, string> = {
  anthropic: "claude-haiku-4-5-20251001",
  openai: "gpt-4o-mini",
  "openai-codex": "gpt-4o-mini",
  gemini: "gemini-2.5-flash",
};

/**
 * Computes the tooltip text for the Re-extract button.
 * Exported for direct unit testing.
 *
 * @param model       - Model ID (e.g. "claude-haiku-4-5-20251001")
 * @param docCount    - Total indexed document count
 * @param providerName - Active provider slug (e.g. "anthropic", "ollama")
 */
export function estimateCost(
  model: string,
  docCount: number,
  providerName: string | null,
): string {
  if (docCount === 0) return "No documents to re-extract";
  if (!providerName || providerName === "ollama") {
    return `Est: free (local model) across ${docCount} docs`;
  }
  const pricePerMillion = MODEL_PRICING[model];
  if (pricePerMillion === undefined) {
    // Unknown model (e.g. user-entered Ollama model that mapped to a different provider)
    return `Est: free (local model) across ${docCount} docs`;
  }
  const totalTokens = docCount * AVG_INPUT_TOKENS_PER_DOC;
  const cost = (totalTokens / 1_000_000) * pricePerMillion;
  const displayName = MODEL_DISPLAY_NAMES[model] ?? model;
  return `Est: $${cost.toFixed(2)} across ${docCount} docs on ${displayName}`;
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export function ExtractionSettings() {
  const { data: settings } = useExtractionSettings();
  const { mutate: updateSettings } = useUpdateExtractionSettings();
  const { mutate: triggerBackfill, isPending: isBackfillPending } =
    useTriggerEntityBackfill();
  const { data: providers } = useProviders();
  const { data: stats } = useStats();
  const { status: backfillStatus } = useBackfillStore();

  // Derive active provider entry
  const activeProviderEntry = providers?.find((p) => p.isActive) ?? null;
  const activeProviderName = activeProviderEntry?.provider ?? null;

  // Local pending state (mirrors settings, synced on load)
  const [pendingModel, setPendingModel] = useState<string>("");
  const [pendingToggle, setPendingToggle] = useState<boolean>(true);

  // Sync from remote settings when they load
  useEffect(() => {
    if (!settings) return;
    const defaultModel =
      activeProviderName ? (PROVIDER_DEFAULT_MODEL[activeProviderName] ?? "") : "";
    setPendingModel(settings.extractionModel || defaultModel);
    setPendingToggle(settings.useLlmExtraction);
  }, [settings?.extractionModel, settings?.useLlmExtraction, activeProviderName]);

  // Track previous model to detect changes (for model-switch toast)
  const prevModelRef = useRef<string>("");
  useEffect(() => {
    if (pendingModel) prevModelRef.current = pendingModel;
  }, [pendingModel]);

  // Doc count for tooltip estimate
  const docCount = stats?.totalDocuments ?? 0;

  // Model options for the active provider
  const modelOptions = activeProviderName
    ? (PROVIDER_MODELS[activeProviderName] ??
      // Ollama: user-entered model shown verbatim; no hardcoded list
      (activeProviderEntry?.model
        ? [{ value: activeProviderEntry.model, label: activeProviderEntry.model }]
        : []))
    : [];

  // Disabled states
  const isBackfillRunning = backfillStatus === "running";
  const buttonDisabled =
    !activeProviderName ||
    !pendingToggle ||
    isBackfillRunning ||
    isBackfillPending;

  // Tooltip content
  const tooltipText = estimateCost(pendingModel, docCount, activeProviderName);

  // Model change handler
  const handleModelChange = useCallback(
    (newModel: string) => {
      const prevModel = pendingModel;
      setPendingModel(newModel);
      updateSettings(
        { extractionModel: newModel, useLlmExtraction: pendingToggle },
        {
          onError: () => toast.error("Failed to save extraction settings."),
        },
      );
      // Model-switch toast when doc count > 0 and the model actually changed
      if (docCount > 0 && newModel !== prevModel) {
        const displayName = MODEL_DISPLAY_NAMES[newModel] ?? newModel;
        toast.info(
          `Extraction model set to ${displayName}. Run 'Re-extract entities' to relabel existing documents.`,
          { duration: 5000 },
        );
      }
    },
    [pendingModel, pendingToggle, docCount, updateSettings],
  );

  // Toggle change handler
  const handleToggleChange = useCallback(
    (checked: boolean) => {
      setPendingToggle(checked);
      updateSettings(
        { extractionModel: pendingModel, useLlmExtraction: checked },
        {
          onError: () => toast.error("Failed to save extraction settings."),
        },
      );
    },
    [pendingModel, updateSettings],
  );

  // Re-extract click handler
  const handleReextract = useCallback(() => {
    triggerBackfill(undefined, {
      onSuccess: () =>
        toast.success("Backfill started. Progress appears in the top bar."),
      onError: (err) =>
        toast.error(`Failed to start backfill. ${(err as Error).message}`),
    });
  }, [triggerBackfill]);

  // Show empty-state caption when no provider connected
  const noProvider = !activeProviderName;

  return (
    <div className="border-t border-border-primary pt-6 mt-6">
      {/* Section header */}
      <h3 className="text-lg font-semibold text-text-primary mb-1">
        Entity Extraction
      </h3>
      <p className="text-xs text-text-tertiary mb-6">
        Choose which model Cortex uses to extract people, organizations, topics,
        and tags from your documents. Faster models reduce cost; use a capable
        model for best accuracy.
      </p>

      {/* Row 1: Extraction model dropdown */}
      <div className="flex items-center justify-between py-4">
        <Label className="text-sm font-semibold text-text-primary">
          Extraction model
        </Label>
        <Select
          value={pendingModel}
          onValueChange={handleModelChange}
          disabled={!activeProviderName}
        >
          <SelectTrigger className="w-[240px]" disabled={!activeProviderName}>
            <SelectValue
              placeholder={
                activeProviderName
                  ? "Select a model"
                  : "Connect a provider first"
              }
            />
          </SelectTrigger>
          <SelectContent>
            {modelOptions.map((opt) => (
              <SelectItem key={opt.value} value={opt.value}>
                {opt.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>

      {/* Row 2: Use LLM for entity extraction toggle */}
      <div className="py-4">
        <div
          className={`flex items-center justify-between ${noProvider ? "opacity-50 cursor-not-allowed" : ""}`}
        >
          <Label className="text-sm font-semibold text-text-primary">
            Use LLM for entity extraction
          </Label>
          <Switch
            checked={pendingToggle}
            onCheckedChange={handleToggleChange}
            disabled={noProvider}
            aria-label="Use LLM for entity extraction"
          />
        </div>
        <p className="text-xs text-text-tertiary mt-1">
          When off, only dates, amounts, emails, phone numbers, and IDs are
          extracted. People, organizations, and topic tags require AI.
        </p>
      </div>

      {/* Row 3: Re-extract entities button (primary focal point) */}
      <div className="flex items-center justify-between py-4">
        <p className="text-xs text-text-tertiary">
          Re-extracts entities across all indexed documents using the selected
          model.
        </p>
        <Tooltip>
          <TooltipTrigger asChild>
            {/* Wrapper needed: disabled button doesn't fire mouse events for tooltip */}
            <span>
              <Button
                variant="default"
                disabled={buttonDisabled}
                onClick={handleReextract}
                className="btn-primary px-4 py-2 text-sm"
              >
                {isBackfillPending ? (
                  <>
                    <Loader2 size={16} className="animate-spin mr-2" />
                    Re-extracting...
                  </>
                ) : (
                  "Re-extract entities"
                )}
              </Button>
            </span>
          </TooltipTrigger>
          <TooltipContent>
            <p className="text-xs">{tooltipText}</p>
          </TooltipContent>
        </Tooltip>
      </div>

      {/* Empty-state caption when no provider */}
      {noProvider && (
        <p className="text-xs text-text-tertiary mt-2">
          Connect a provider in the AI Providers section above to enable LLM
          entity extraction.
        </p>
      )}
    </div>
  );
}
