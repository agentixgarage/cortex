/**
 * AiProvidersSection.tsx
 *
 * Container wrapping 4 stacked ProviderCards inside a single RadioGroup
 * for active-provider selection (D-17).
 *
 * The id="ai-providers" anchor target is required for the D-20 embedding
 * unification link in SettingsPage.tsx ("Connect OpenAI below →").
 */

import { useRef, useEffect } from "react";
import { RadioGroup } from "@/components/ui/radio-group";
import { useProviders, useSetActiveProvider, useExtractionSettings } from "@/hooks/useTauri";
import { ProviderCard } from "./ProviderCard";
import { toast } from "sonner";

const PROVIDERS = ["anthropic", "openai", "gemini", "ollama"] as const;
type Provider = (typeof PROVIDERS)[number];

// Human-readable default model display names per provider (D-11 defaults).
// Used for the D-30 provider-switch toast copy.
const PROVIDER_DEFAULT_MODEL_DISPLAY: Record<string, string> = {
  anthropic: "Claude Haiku 4.5",
  "openai-codex": "GPT-4o mini",
  openai: "GPT-4o mini",
  gemini: "Gemini 2.5 Flash",
};

/**
 * Resolve the status entry for a UI provider slot.
 *
 * OpenAI is special: the backend maintains TWO provider entries — plain "openai" (API key)
 * and "openai-codex" (ChatGPT/Codex OAuth subscription per D-22). The Settings UI shows
 * ONE OpenAI card that reflects whichever variant is authenticated (openai-codex takes
 * precedence when authenticated, since it's the primary sign-in path per D-25).
 */
function resolveCardStatus(uiSlug: Provider, providers: ReturnType<typeof useProviders>["data"]) {
  if (!providers) return undefined;
  if (uiSlug === "openai") {
    const codex = providers.find((p) => p.provider === "openai-codex");
    if (codex?.authenticated) return codex;
    return providers.find((p) => p.provider === "openai");
  }
  return providers.find((p) => p.provider === uiSlug);
}

export function AiProvidersSection() {
  const { data: providers } = useProviders();
  const { data: extractionSettings } = useExtractionSettings();
  const setActive = useSetActiveProvider();

  // Map backend active-provider slug to UI slot (openai-codex maps to "openai" slot).
  const rawActive = providers?.find((p) => p.isActive)?.provider ?? "";
  const activeProvider = rawActive === "openai-codex" ? "openai" : rawActive;

  // D-30 provider-switch toast: fires when active provider changes, NOT on initial mount.
  // Using useRef to hold previous value avoids re-renders and initial-mount false-positive.
  const prevActiveRef = useRef<string | undefined>(undefined);
  useEffect(() => {
    const prev = prevActiveRef.current;
    prevActiveRef.current = rawActive;

    // Don't fire on first render (prev is undefined) or when provider is deselected (rawActive="")
    if (prev === undefined || prev === null || !rawActive) return;
    if (prev === rawActive) return;

    // Determine model display name: prefer the user's current extraction settings model,
    // else fall back to the D-11 default for this provider.
    const currentModel = extractionSettings?.extractionModel;
    const modelDisplayName =
      (currentModel && currentModel !== "")
        ? currentModel
        : (PROVIDER_DEFAULT_MODEL_DISPLAY[rawActive] ?? rawActive);

    toast.info(
      `Provider switched. New extractions use ${modelDisplayName}. Run 'Re-extract entities' for consistent labels across all docs.`,
      { duration: 6000 },
    );
  }, [rawActive]); // eslint-disable-line react-hooks/exhaustive-deps — intentionally omits extractionSettings to avoid re-firing

  const handleActiveChange = async (uiSlug: string) => {
    // Map UI slot back to whichever OpenAI variant is currently authenticated.
    let backendSlug = uiSlug;
    if (uiSlug === "openai" && providers) {
      const codex = providers.find((p) => p.provider === "openai-codex");
      if (codex?.authenticated) {
        backendSlug = "openai-codex";
      }
    }
    try {
      await setActive.mutateAsync(backendSlug);
    } catch (err) {
      toast.error((err as Error).message);
    }
  };

  return (
    <section id="ai-providers" className="space-y-6">
      <h3 className="section-header text-text-primary">AI Providers</h3>
      <RadioGroup
        value={activeProvider}
        onValueChange={handleActiveChange}
        className="space-y-6"
      >
        {PROVIDERS.map((id: Provider) => (
          <ProviderCard
            key={id}
            provider={id}
            status={resolveCardStatus(id, providers)}
          />
        ))}
      </RadioGroup>
    </section>
  );
}
