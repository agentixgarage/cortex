/**
 * ProviderCard.tsx
 *
 * Single provider card with collapsed/expanded states for all 4 providers.
 * Handles: Anthropic (setup-token), OpenAI (API key), Gemini (API key), Ollama (URL + dynamic model).
 *
 * Color tokens: ALWAYS use semantic tokens (text-text-primary, bg-bg-secondary, etc.)
 * NEVER hardcode hex values — verified by grep gate.
 */

import { useState, useEffect } from "react";
import { ChevronDown, Loader2, Copy, ExternalLink, Sparkles } from "lucide-react";
import { RadioGroupItem } from "@/components/ui/radio-group";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { toast } from "sonner";
import { cn } from "@/lib/utils";
import type { ProviderAuthStatus } from "@/lib/types";
import {
  useConnectProvider,
  useDisconnectProvider,
  useSaveSetupToken,
  useStartOpenAiOAuth,
  useSetProviderModel,
} from "@/hooks/useTauri";

// ---------------------------------------------------------------------------
// Static data
// ---------------------------------------------------------------------------

const PROVIDER_LABELS: Record<string, string> = {
  anthropic: "Anthropic",
  openai: "OpenAI",
  gemini: "Gemini",
  ollama: "Ollama",
};

const ANTHROPIC_MODELS = [
  { label: "Claude Haiku 4.5 (fast)", value: "claude-haiku-4-5-20251001" },
  { label: "Claude Sonnet 4.6 (balanced)", value: "claude-sonnet-4-6" },
  { label: "Claude Opus 4.7 (powerful)", value: "claude-opus-4" },
] as const;

// Current as of July 2026 — confirmed via learn.chatgpt.com/docs/models.
// OpenAI deprecates model slugs frequently (gpt-5, gpt-4o, gpt-4o-mini,
// gpt-4.1, o1 family have ALL been retired/rejected already) — this list
// WILL go stale again. A dynamic model-list fetch is the real fix
// (ROADMAP.md) rather than another hardcoded guess.
const OPENAI_MODELS = [
  { label: "GPT-5.6 Luna (fast)", value: "gpt-5.6-luna" },
  { label: "GPT-5.6 Terra (balanced)", value: "gpt-5.6-terra" },
  { label: "GPT-5.6 Sol (powerful)", value: "gpt-5.6-sol" },
  { label: "GPT-5.5 (previous-gen)", value: "gpt-5.5" },
  { label: "GPT-5.4 mini (fast, legacy)", value: "gpt-5.4-mini" },
  { label: "GPT-5.4 (legacy)", value: "gpt-5.4" },
] as const;

// Confirmed supported via ChatGPT sign-in (learn.chatgpt.com/docs/models):
// the full gpt-5.6 + gpt-5.5/5.4 family. Same list as direct API-key for now.
const OPENAI_CODEX_MODELS = OPENAI_MODELS;

const GEMINI_MODELS = [
  { label: "Gemini 2.5 Flash (fast)", value: "gemini-2.5-flash" },
  { label: "Gemini 2.5 Pro (powerful)", value: "gemini-2.5-pro" },
] as const;

const ANTHROPIC_DEFAULT_MODEL = "claude-haiku-4-5-20251001";
const OPENAI_DEFAULT_MODEL = "gpt-5.6-terra";
const GEMINI_DEFAULT_MODEL = "gemini-2.5-flash";
const OLLAMA_DEFAULT_URL = "http://localhost:11434";

// ---------------------------------------------------------------------------
// Button lifecycle type
// ---------------------------------------------------------------------------

type ButtonPhase = "idle" | "validating" | "saved" | "error";

// ---------------------------------------------------------------------------
// Anthropic connect form
// ---------------------------------------------------------------------------

function AnthropicConnectForm({ onConnected }: { onConnected: () => void }) {
  const [token, setToken] = useState("");
  const [phase, setPhase] = useState<ButtonPhase>("idle");
  const saveSetupToken = useSaveSetupToken();

  const tokenInvalid = token.length > 0 && !token.startsWith("sk-ant-oat01-");

  const handleConnect = async () => {
    if (!token.trim()) return;
    setPhase("validating");
    try {
      await saveSetupToken.mutateAsync(token.trim());
      setPhase("saved");
      setTimeout(() => {
        setPhase("idle");
        onConnected();
      }, 1200);
    } catch (err) {
      setPhase("error");
      toast.error((err as Error).message);
      setTimeout(() => setPhase("idle"), 200);
    }
  };

  return (
    <div className="px-4 pb-4 pt-3 space-y-4 border-t border-border-primary">
      {/* Step 1: Get setup token */}
      <div className="space-y-2">
        <p className="section-title text-text-primary text-sm font-semibold">Get your setup token</p>
        <div className="relative rounded-md bg-bg-tertiary border border-border-secondary px-4 py-4">
          <code className="font-mono text-sm text-text-primary">claude setup-token</code>
          <button
            type="button"
            aria-label="Copy command"
            onClick={() => {
              navigator.clipboard.writeText("claude setup-token");
              toast.success("Copied");
            }}
            className="absolute top-2 right-2 min-w-[44px] min-h-[44px] flex items-center justify-center text-text-tertiary hover:text-text-primary transition-colors"
          >
            <Copy size={14} />
          </button>
        </div>
      </div>

      {/* Step 2: Paste token */}
      <div className="space-y-2">
        <p className="text-sm font-semibold text-text-primary">Setup token</p>
        <input
          type="password"
          placeholder="sk-ant-oat01-…"
          className="input-base w-full font-mono text-sm"
          autoComplete="off"
          value={token}
          onChange={(e) => setToken(e.target.value)}
          aria-describedby={tokenInvalid ? "token-hint" : undefined}
        />
        {tokenInvalid && (
          <p id="token-hint" className="text-error text-xs">
            Setup tokens start with sk-ant-oat01-
          </p>
        )}
      </div>

      {/* Help link */}
      <p className="text-sm text-text-secondary">
        Don&apos;t have Claude CLI?{" "}
        <a
          href="https://claude.ai/download"
          target="_blank"
          rel="noreferrer"
          className="text-accent-primary hover:underline inline-flex items-center gap-0.5"
        >
          Install Claude CLI
          <ExternalLink size={12} />
        </a>
      </p>

      {/* Connect button */}
      <button
        type="button"
        onClick={handleConnect}
        disabled={phase === "validating" || !token.trim()}
        aria-busy={phase === "validating"}
        className={cn(
          "w-full btn-primary flex items-center justify-center gap-2 min-h-[44px]",
          phase === "saved" && "text-success",
        )}
      >
        {phase === "validating" && <Loader2 size={14} className="animate-spin" />}
        {phase === "idle" || phase === "error" ? "Connect" : null}
        {phase === "validating" ? "Validating…" : null}
        {phase === "saved" ? "Connected" : null}
      </button>
    </div>
  );
}

// ---------------------------------------------------------------------------
// OpenAI connect form — two-mode (OAuth primary, API key secondary)
// ---------------------------------------------------------------------------

/**
 * Shared API-key input form used by both OpenAIConnectForm (api-key mode)
 * and GeminiApiKeyConnectForm. Extracted to avoid duplication.
 */
function ApiKeyFormBody({
  provider,
  onConnected,
  headerSlot,
}: {
  provider: "openai" | "gemini";
  onConnected: () => void;
  /** Optional content rendered above the API key label */
  headerSlot?: React.ReactNode;
}) {
  const models = provider === "openai" ? OPENAI_MODELS : GEMINI_MODELS;
  const defaultModel = provider === "openai" ? OPENAI_DEFAULT_MODEL : GEMINI_DEFAULT_MODEL;
  const placeholder = provider === "openai" ? "sk-…" : "AIza…";

  const [apiKey, setApiKey] = useState("");
  const [selectedModel, setSelectedModel] = useState(defaultModel);
  const [phase, setPhase] = useState<ButtonPhase>("idle");
  const connectProvider = useConnectProvider();

  const handleConnect = async () => {
    if (!apiKey.trim()) return;
    setPhase("validating");
    try {
      await connectProvider.mutateAsync({
        provider,
        method: "api-key",
        credential: apiKey.trim(),
        model: selectedModel,
      });
      setPhase("saved");
      setTimeout(() => {
        setPhase("idle");
        onConnected();
      }, 1200);
    } catch (err) {
      setPhase("error");
      toast.error((err as Error).message);
      setTimeout(() => setPhase("idle"), 200);
    }
  };

  return (
    <>
      {headerSlot}
      <div className="space-y-2">
        <label htmlFor={`${provider}-api-key`} className="text-sm font-semibold text-text-primary block">
          API Key
        </label>
        <input
          id={`${provider}-api-key`}
          type="password"
          placeholder={placeholder}
          className="input-base w-full"
          autoComplete="off"
          value={apiKey}
          onChange={(e) => setApiKey(e.target.value)}
        />
      </div>

      <div className="space-y-2">
        <p className="text-sm font-semibold text-text-primary">Model</p>
        <Select value={selectedModel} onValueChange={setSelectedModel}>
          <SelectTrigger className="w-full">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {models.map((m) => (
              <SelectItem key={m.value} value={m.value}>
                {m.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>

      <button
        type="button"
        onClick={handleConnect}
        disabled={phase === "validating" || !apiKey.trim()}
        aria-busy={phase === "validating"}
        className={cn(
          "w-full btn-primary flex items-center justify-center gap-2 min-h-[44px]",
          phase === "saved" && "text-success",
        )}
      >
        {phase === "validating" && <Loader2 size={14} className="animate-spin" />}
        {phase === "idle" || phase === "error" ? "Connect" : null}
        {phase === "validating" ? "Validating…" : null}
        {phase === "saved" ? "Connected" : null}
      </button>
    </>
  );
}

/**
 * OpenAI connect form — two-mode per D-25:
 *   mode "oauth" (default): primary "Sign in with ChatGPT" CTA + "Use API key instead" toggle
 *   mode "api-key":         existing API key paste form + "Sign in with ChatGPT instead" back-link
 *
 * Gemini card is NOT two-mode (Option C confirmed — API-key-only, no start_gemini_oauth).
 */
function OpenAIConnectForm({ onConnected }: { onConnected: () => void }) {
  const [mode, setMode] = useState<"oauth" | "api-key">("oauth");
  const oauth = useStartOpenAiOAuth();

  if (mode === "oauth") {
    return (
      <div className="px-4 pb-4 pt-3 space-y-3 border-t border-border-primary">
        {/* Primary CTA: Sign in with ChatGPT */}
        <button
          type="button"
          onClick={async () => {
            try {
              await oauth.mutateAsync();
              toast.success("Signed in via ChatGPT");
              onConnected();
            } catch (err) {
              toast.error(
                (err as Error).message ||
                  "Sign-in cancelled or timed out. Try again or use an API key instead.",
              );
            }
          }}
          disabled={oauth.isPending}
          aria-busy={oauth.isPending}
          className="w-full btn-primary flex items-center justify-center gap-2 min-h-[44px]"
        >
          {oauth.isPending ? (
            <>
              <Loader2 size={14} className="animate-spin" />
              Opening browser…
            </>
          ) : (
            <>
              <Sparkles size={14} />
              Sign in with ChatGPT
            </>
          )}
        </button>

        {/* Body copy */}
        <p className="text-sm text-text-secondary">
          Use your existing ChatGPT Plus / Team subscription. No API key required.
        </p>

        {/* Secondary toggle → api-key mode */}
        <button
          type="button"
          onClick={() => setMode("api-key")}
          className="text-sm text-text-secondary hover:text-text-primary underline"
        >
          Use API key instead
        </button>
      </div>
    );
  }

  // mode === "api-key": existing Plan 05 form with back-to-oauth toggle above
  return (
    <div className="px-4 pb-4 pt-3 space-y-4 border-t border-border-primary">
      <ApiKeyFormBody
        provider="openai"
        onConnected={onConnected}
        headerSlot={
          <button
            type="button"
            onClick={() => setMode("oauth")}
            className="text-sm text-text-secondary hover:text-text-primary underline"
          >
            Sign in with ChatGPT instead
          </button>
        }
      />
    </div>
  );
}

// ---------------------------------------------------------------------------
// Gemini connect form — API-key-only (Option C: no Sign in with Google CTA)
// ---------------------------------------------------------------------------

function GeminiApiKeyConnectForm({ onConnected }: { onConnected: () => void }) {
  return (
    <div className="px-4 pb-4 pt-3 space-y-4 border-t border-border-primary">
      <ApiKeyFormBody provider="gemini" onConnected={onConnected} />
    </div>
  );
}

// ---------------------------------------------------------------------------
// Ollama connect form
// ---------------------------------------------------------------------------

function OllamaConnectForm({ onConnected }: { onConnected: () => void }) {
  const [baseUrl, setBaseUrl] = useState(OLLAMA_DEFAULT_URL);
  const [models, setModels] = useState<string[]>([]);
  const [selectedModel, setSelectedModel] = useState("");
  const [modelLoadState, setModelLoadState] = useState<"loading" | "loaded" | "error">("loading");
  const [phase, setPhase] = useState<ButtonPhase>("idle");
  const connectProvider = useConnectProvider();

  // Fetch Ollama models dynamically with 500ms debounce (RESEARCH Pitfall 5)
  useEffect(() => {
    setModelLoadState("loading");
    setModels([]);
    setSelectedModel("");

    const timer = setTimeout(async () => {
      try {
        const res = await fetch(`${baseUrl}/api/tags`);
        if (!res.ok) throw new Error(`HTTP ${res.status}`);
        // Response shape: { models: [{ name: string }] }  (Assumption A3)
        const data = (await res.json()) as { models: { name: string }[] };
        const names = data.models?.map((m) => m.name) ?? [];
        setModels(names);
        setSelectedModel(names[0] ?? "");
        setModelLoadState("loaded");
      } catch {
        setModelLoadState("error");
      }
    }, 500);

    return () => clearTimeout(timer);
  }, [baseUrl]);

  const handleConnect = async () => {
    if (!selectedModel) return;
    setPhase("validating");
    try {
      await connectProvider.mutateAsync({
        provider: "ollama",
        method: "ollama",
        baseUrl,
        model: selectedModel,
      });
      setPhase("saved");
      setTimeout(() => {
        setPhase("idle");
        onConnected();
      }, 1200);
    } catch (err) {
      setPhase("error");
      toast.error((err as Error).message);
      setTimeout(() => setPhase("idle"), 200);
    }
  };

  return (
    <div className="px-4 pb-4 pt-3 space-y-4 border-t border-border-primary">
      <div className="space-y-2">
        <p className="text-sm font-semibold text-text-primary">Base URL</p>
        <input
          type="text"
          className="input-base w-full font-mono text-sm"
          value={baseUrl}
          onChange={(e) => setBaseUrl(e.target.value)}
          placeholder={OLLAMA_DEFAULT_URL}
        />
      </div>

      <div className="space-y-2">
        <p className="text-sm font-semibold text-text-primary">Model</p>
        {modelLoadState === "loading" ? (
          <Select disabled>
            <SelectTrigger className="w-full">
              <SelectValue placeholder="Loading models…" />
            </SelectTrigger>
            <SelectContent />
          </Select>
        ) : modelLoadState === "error" || models.length === 0 ? (
          <>
            <Select disabled>
              <SelectTrigger className="w-full">
                <SelectValue placeholder="Start Ollama to see models" />
              </SelectTrigger>
              <SelectContent />
            </Select>
            <p className="text-error text-xs">Could not connect to Ollama at {baseUrl}</p>
          </>
        ) : (
          <Select value={selectedModel} onValueChange={setSelectedModel}>
            <SelectTrigger className="w-full">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {models.map((name) => (
                <SelectItem key={name} value={name}>
                  {name}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        )}
      </div>

      <button
        type="button"
        onClick={handleConnect}
        disabled={phase === "validating" || !selectedModel || modelLoadState !== "loaded"}
        aria-busy={phase === "validating"}
        className={cn(
          "w-full btn-primary flex items-center justify-center gap-2 min-h-[44px]",
          phase === "saved" && "text-success",
        )}
      >
        {phase === "validating" && <Loader2 size={14} className="animate-spin" />}
        {phase === "idle" || phase === "error" ? "Connect" : null}
        {phase === "validating" ? "Validating…" : null}
        {phase === "saved" ? "Connected" : null}
      </button>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Management form (connected state, expanded)
// ---------------------------------------------------------------------------

/**
 * Editable model dropdown for an already-connected provider. Calls
 * set_provider_model immediately on change (no separate Save button —
 * matches the rest of Settings' auto-save pattern). Uses status.provider
 * (the BACKEND slug, e.g. "openai-codex") as the credential key so the
 * update hits the correct stored credential, not just the UI slot.
 */
function ModelDropdown({
  backendSlug,
  models,
  currentModel,
  defaultModel,
}: {
  backendSlug: string;
  models: readonly { label: string; value: string }[];
  currentModel: string | null | undefined;
  defaultModel: string;
}) {
  const setModel = useSetProviderModel();
  const value = currentModel ?? defaultModel;

  return (
    <div className="space-y-2">
      <p className="text-sm font-semibold text-text-primary">Model</p>
      <Select
        value={value}
        onValueChange={async (next) => {
          try {
            await setModel.mutateAsync({ provider: backendSlug, model: next });
            toast.success(`Model changed to ${models.find((m) => m.value === next)?.label ?? next}`);
          } catch (err) {
            toast.error((err as Error).message);
          }
        }}
        disabled={setModel.isPending}
      >
        <SelectTrigger className="w-full">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          {models.map((m) => (
            <SelectItem key={m.value} value={m.value}>
              {m.label}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    </div>
  );
}

function ManagementForm({
  provider,
  status,
}: {
  provider: "anthropic" | "openai" | "gemini" | "ollama";
  status: ProviderAuthStatus;
}) {
  const disconnectProvider = useDisconnectProvider();

  const handleDisconnect = async () => {
    // No confirmation dialog per UI-SPEC Destructive actions contract.
    // Use status.provider (backend slug — may be "openai-codex" when the OpenAI card
    // is actually representing an OAuth-connected Codex credential) so the disconnect
    // hits the correct credential entry. Falls back to UI slug for non-OpenAI cards.
    const backendSlug = status.provider ?? provider;
    try {
      await disconnectProvider.mutateAsync(backendSlug);
      toast.success("Disconnected");
    } catch (err) {
      toast.error((err as Error).message);
    }
  };

  // status.provider is the BACKEND slug ("openai" or "openai-codex" for the
  // OpenAI card, matching provider for anthropic/gemini/ollama).
  const backendSlug = status.provider ?? provider;
  const isCodex = backendSlug === "openai-codex";

  return (
    <div className="px-4 pb-4 pt-3 space-y-4 border-t border-border-primary">
      {provider === "anthropic" ? (
        <ModelDropdown
          backendSlug={backendSlug}
          models={ANTHROPIC_MODELS}
          currentModel={status.model}
          defaultModel={ANTHROPIC_DEFAULT_MODEL}
        />
      ) : provider === "openai" ? (
        <ModelDropdown
          backendSlug={backendSlug}
          models={isCodex ? OPENAI_CODEX_MODELS : OPENAI_MODELS}
          currentModel={status.model}
          defaultModel={OPENAI_DEFAULT_MODEL}
        />
      ) : provider === "gemini" ? (
        <ModelDropdown
          backendSlug={backendSlug}
          models={GEMINI_MODELS}
          currentModel={status.model}
          defaultModel={GEMINI_DEFAULT_MODEL}
        />
      ) : (
        <div className="space-y-1">
          <p className="text-sm font-semibold text-text-primary">Model</p>
          <p className="text-sm text-text-secondary font-mono">{status.model ?? "(no model)"}</p>
          <p className="text-xs text-text-tertiary">Ollama model is set via the connection URL above</p>
        </div>
      )}

      {/* Disconnect button — text-only, error color, NO confirmation dialog */}
      <button
        type="button"
        onClick={handleDisconnect}
        className="text-error hover:underline text-sm min-h-[44px] px-0"
      >
        Disconnect
      </button>
    </div>
  );
}

// ---------------------------------------------------------------------------
// ProviderCard — exported named component (CLAUDE.md: named exports)
// ---------------------------------------------------------------------------

export interface ProviderCardProps {
  provider: "anthropic" | "openai" | "gemini" | "ollama";
  /** Status from useProviders(); undefined while loading */
  status: ProviderAuthStatus | undefined;
}

export function ProviderCard({ provider, status }: ProviderCardProps) {
  const [isExpanded, setIsExpanded] = useState(false);

  const isAuthenticated = status?.authenticated ?? false;
  const isActive = status?.isActive ?? false;

  const toggleExpanded = () => setIsExpanded((prev) => !prev);

  const handleConnected = () => {
    setIsExpanded(false);
  };

  return (
    <div className="rounded-lg border border-border-primary bg-bg-secondary overflow-hidden">
      {/* Collapsed row — always visible */}
      <div
        role="button"
        tabIndex={0}
        aria-expanded={isExpanded}
        aria-controls={`provider-form-${provider}`}
        onClick={toggleExpanded}
        onKeyDown={(e) => {
          if (e.key === "Enter" || e.key === " ") {
            e.preventDefault();
            toggleExpanded();
          }
        }}
        className="flex items-center gap-3 px-4 py-4 min-h-[44px] cursor-pointer hover:bg-bg-tertiary transition-colors"
      >
        {/* Active-provider radio selector (D-17)
            RadioGroupItem is in the row so the RadioGroup context from AiProvidersSection
            can select it. Disabled when not authenticated (AIPV-05). */}
        <div
          onClick={(e) => e.stopPropagation()}
          className={cn(!isAuthenticated && "opacity-50")}
        >
          <RadioGroupItem
            value={provider}
            disabled={!isAuthenticated}
            id={`provider-radio-${provider}`}
            aria-disabled={!isAuthenticated}
          />
        </div>

        {/* Provider name */}
        <span className="text-sm font-semibold text-text-primary flex-1">
          {PROVIDER_LABELS[provider]}
        </span>

        {/* Status badge */}
        {isAuthenticated ? (
          <span
            role="status"
            className="bg-success/10 text-success text-sm font-semibold px-2 py-0.5 rounded"
          >
            {/* D-25: OpenAI OAuth connection shows distinct badge label */}
            {provider === "openai" && status?.method === "oauth"
              ? "Connected via ChatGPT (Codex)"
              : "Connected"}
          </span>
        ) : (
          <span
            role="status"
            className="bg-error/10 text-error text-sm font-semibold px-2 py-0.5 rounded"
          >
            Not connected
          </span>
        )}

        {/* Model name (mono, only when connected) */}
        {isAuthenticated && status?.model && (
          <span className="text-xs text-text-tertiary font-mono">{status.model}</span>
        )}

        {/* Active label */}
        {isActive && (
          <span className="text-xs text-accent-primary font-semibold">Active</span>
        )}

        {/* Chevron */}
        <button
          type="button"
          aria-label={isExpanded ? "Collapse" : "Expand"}
          onClick={(e) => {
            e.stopPropagation();
            toggleExpanded();
          }}
          className="min-w-[44px] min-h-[44px] flex items-center justify-center text-text-tertiary hover:text-text-primary transition-colors"
        >
          <ChevronDown
            size={14}
            className={cn("transition-transform duration-200", isExpanded && "rotate-180")}
          />
        </button>
      </div>

      {/* Expandable content */}
      {isExpanded && (
        <div
          id={`provider-form-${provider}`}
          className="animate-in slide-in-from-top-2 duration-200"
        >
          {isAuthenticated ? (
            <ManagementForm provider={provider} status={status!} />
          ) : provider === "anthropic" ? (
            <AnthropicConnectForm onConnected={handleConnected} />
          ) : provider === "ollama" ? (
            <OllamaConnectForm onConnected={handleConnected} />
          ) : provider === "openai" ? (
            <OpenAIConnectForm onConnected={handleConnected} />
          ) : (
            // gemini — API-key-only (Option C confirmed: no Sign in with Google CTA)
            <GeminiApiKeyConnectForm onConnected={handleConnected} />
          )}
        </div>
      )}
    </div>
  );
}
