/**
 * ConnectAiStep.tsx
 *
 * Onboarding Step 2 — "Connect your AI" with 2×2 grid of mini provider cards.
 * Each card has inline connect forms (no modal). No model selector in onboarding —
 * default models applied per provider.
 *
 * CRITICAL D-14/D-15 INVARIANT:
 *   The Skip handler MUST NOT call useAiBannerStore.dismiss() or touch the banner
 *   store in any way. Banner appears post-onboarding-completion automatically
 *   because hasActiveProvider stays false. Calling dismiss() here would suppress
 *   the post-onboarding banner for users who skipped, breaking D-14 and D-15.
 *
 * Color tokens: ALWAYS use semantic tokens — never hardcode hex values.
 */

import { useState } from "react";
import { Check, Copy, ArrowRight, SkipForward, Loader2, ExternalLink, Zap } from "lucide-react";
import { toast } from "sonner";
import { cn } from "@/lib/utils";
import { useProviders, useSaveSetupToken, useConnectProvider } from "@/hooks/useTauri";

// ---------------------------------------------------------------------------
// Static data
// ---------------------------------------------------------------------------

const ANTHROPIC_DEFAULT_MODEL = "claude-haiku-4-5-20251001";
// Direct OpenAI API key connect (NOT ChatGPT/Codex OAuth — see ai/service.rs
// for the separate openai-codex model restriction). gpt-4o-mini is broadly
// available on api.openai.com with a real API key.
const OPENAI_DEFAULT_MODEL = "gpt-4o-mini";
const GEMINI_DEFAULT_MODEL = "gemini-2.5-flash";
const OLLAMA_DEFAULT_URL = "http://localhost:11434";
const OLLAMA_DEFAULT_MODEL = "llama3";

type ProviderId = "anthropic" | "openai" | "gemini" | "ollama";

const PROVIDER_LABELS: Record<ProviderId, string> = {
  anthropic: "Anthropic",
  openai: "OpenAI",
  gemini: "Gemini",
  ollama: "Ollama",
};

// ---------------------------------------------------------------------------
// Props
// ---------------------------------------------------------------------------

export interface ConnectAiStepProps {
  onContinue: () => void;
  onSkip: () => void;
}

// ---------------------------------------------------------------------------
// Anthropic mini connect form (inline, condensed)
// ---------------------------------------------------------------------------

function AnthropicMiniForm({ onConnected }: { onConnected: () => void }) {
  const [token, setToken] = useState("");
  const [isPending, setIsPending] = useState(false);
  const saveSetupToken = useSaveSetupToken();

  const tokenInvalid = token.length > 0 && !token.startsWith("sk-ant-oat01-");

  const handleConnect = async () => {
    if (!token.trim()) return;
    setIsPending(true);
    try {
      await saveSetupToken.mutateAsync(token.trim());
      toast.success("Anthropic connected");
      onConnected();
    } catch (err) {
      toast.error((err as Error).message);
    } finally {
      setIsPending(false);
    }
  };

  return (
    <div className="space-y-3 mt-3 pt-3 border-t border-border-primary">
      {/* Setup token command */}
      <div className="relative rounded-md bg-bg-tertiary border border-border-secondary px-3 py-2">
        <code className="font-mono text-sm text-text-primary">claude setup-token</code>
        <button
          type="button"
          aria-label="Copy command"
          onClick={() => {
            navigator.clipboard.writeText("claude setup-token");
            toast.success("Copied");
          }}
          className="absolute top-1 right-1 min-w-[32px] min-h-[32px] flex items-center justify-center text-text-tertiary hover:text-text-primary transition-colors"
        >
          <Copy size={12} />
        </button>
      </div>

      {/* Token input */}
      <input
        type="password"
        placeholder="sk-ant-oat01-…"
        className="w-full rounded-md border border-border-primary bg-bg-primary px-3 py-2 text-sm text-text-primary placeholder:text-text-tertiary focus:border-accent-primary focus:outline-none font-mono"
        autoComplete="off"
        value={token}
        onChange={(e) => setToken(e.target.value)}
      />
      {tokenInvalid && (
        <p className="text-xs text-error">Setup tokens start with sk-ant-oat01-</p>
      )}

      {/* Help link */}
      <p className="text-xs text-text-secondary">
        No Claude CLI?{" "}
        <a
          href="https://claude.ai/download"
          target="_blank"
          rel="noreferrer"
          className="text-accent-primary hover:underline inline-flex items-center gap-0.5"
        >
          Install <ExternalLink size={10} />
        </a>
      </p>

      {/* Connect button */}
      <button
        type="button"
        onClick={handleConnect}
        disabled={isPending || !token.trim()}
        className="w-full flex items-center justify-center gap-2 rounded-md bg-accent-primary px-3 py-2 text-sm font-semibold text-white transition-colors hover:bg-accent-hover disabled:opacity-50 disabled:cursor-not-allowed min-h-[36px]"
      >
        {isPending && <Loader2 size={12} className="animate-spin" />}
        {isPending ? "Validating…" : "Connect"}
      </button>
    </div>
  );
}

// ---------------------------------------------------------------------------
// OpenAI mini connect form
// ---------------------------------------------------------------------------

function OpenAIMiniForm({ onConnected }: { onConnected: () => void }) {
  const [apiKey, setApiKey] = useState("");
  const [isPending, setIsPending] = useState(false);
  const connectProvider = useConnectProvider();

  const handleConnect = async () => {
    if (!apiKey.trim()) return;
    setIsPending(true);
    try {
      await connectProvider.mutateAsync({
        provider: "openai",
        method: "api-key",
        credential: apiKey.trim(),
        model: OPENAI_DEFAULT_MODEL,
      });
      toast.success("OpenAI connected");
      onConnected();
    } catch (err) {
      toast.error((err as Error).message);
    } finally {
      setIsPending(false);
    }
  };

  return (
    <div className="space-y-3 mt-3 pt-3 border-t border-border-primary">
      <input
        type="password"
        placeholder="sk-…"
        className="w-full rounded-md border border-border-primary bg-bg-primary px-3 py-2 text-sm text-text-primary placeholder:text-text-tertiary focus:border-accent-primary focus:outline-none"
        autoComplete="off"
        value={apiKey}
        onChange={(e) => setApiKey(e.target.value)}
      />
      <button
        type="button"
        onClick={handleConnect}
        disabled={isPending || !apiKey.trim()}
        className="w-full flex items-center justify-center gap-2 rounded-md bg-accent-primary px-3 py-2 text-sm font-semibold text-white transition-colors hover:bg-accent-hover disabled:opacity-50 disabled:cursor-not-allowed min-h-[36px]"
      >
        {isPending && <Loader2 size={12} className="animate-spin" />}
        {isPending ? "Validating…" : "Connect"}
      </button>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Gemini mini connect form
// ---------------------------------------------------------------------------

function GeminiMiniForm({ onConnected }: { onConnected: () => void }) {
  const [apiKey, setApiKey] = useState("");
  const [isPending, setIsPending] = useState(false);
  const connectProvider = useConnectProvider();

  const handleConnect = async () => {
    if (!apiKey.trim()) return;
    setIsPending(true);
    try {
      await connectProvider.mutateAsync({
        provider: "gemini",
        method: "api-key",
        credential: apiKey.trim(),
        model: GEMINI_DEFAULT_MODEL,
      });
      toast.success("Gemini connected");
      onConnected();
    } catch (err) {
      toast.error((err as Error).message);
    } finally {
      setIsPending(false);
    }
  };

  return (
    <div className="space-y-3 mt-3 pt-3 border-t border-border-primary">
      <input
        type="password"
        placeholder="AIza…"
        className="w-full rounded-md border border-border-primary bg-bg-primary px-3 py-2 text-sm text-text-primary placeholder:text-text-tertiary focus:border-accent-primary focus:outline-none"
        autoComplete="off"
        value={apiKey}
        onChange={(e) => setApiKey(e.target.value)}
      />
      <button
        type="button"
        onClick={handleConnect}
        disabled={isPending || !apiKey.trim()}
        className="w-full flex items-center justify-center gap-2 rounded-md bg-accent-primary px-3 py-2 text-sm font-semibold text-white transition-colors hover:bg-accent-hover disabled:opacity-50 disabled:cursor-not-allowed min-h-[36px]"
      >
        {isPending && <Loader2 size={12} className="animate-spin" />}
        {isPending ? "Validating…" : "Connect"}
      </button>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Ollama mini connect form (fixed default model — no dynamic /api/tags fetch
// in onboarding; Settings page does dynamic fetch)
// ---------------------------------------------------------------------------

function OllamaMiniForm({ onConnected }: { onConnected: () => void }) {
  const [baseUrl, setBaseUrl] = useState(OLLAMA_DEFAULT_URL);
  const [isPending, setIsPending] = useState(false);
  const connectProvider = useConnectProvider();

  const handleConnect = async () => {
    setIsPending(true);
    try {
      await connectProvider.mutateAsync({
        provider: "ollama",
        method: "ollama",
        baseUrl,
        model: OLLAMA_DEFAULT_MODEL,
      });
      toast.success("Ollama connected");
      onConnected();
    } catch (err) {
      toast.error((err as Error).message);
    } finally {
      setIsPending(false);
    }
  };

  return (
    <div className="space-y-3 mt-3 pt-3 border-t border-border-primary">
      <input
        type="text"
        className="w-full rounded-md border border-border-primary bg-bg-primary px-3 py-2 text-sm text-text-primary placeholder:text-text-tertiary focus:border-accent-primary focus:outline-none font-mono"
        value={baseUrl}
        onChange={(e) => setBaseUrl(e.target.value)}
        placeholder={OLLAMA_DEFAULT_URL}
      />
      <button
        type="button"
        onClick={handleConnect}
        disabled={isPending}
        className="w-full flex items-center justify-center gap-2 rounded-md bg-accent-primary px-3 py-2 text-sm font-semibold text-white transition-colors hover:bg-accent-hover disabled:opacity-50 disabled:cursor-not-allowed min-h-[36px]"
      >
        {isPending && <Loader2 size={12} className="animate-spin" />}
        {isPending ? "Connecting…" : "Connect"}
      </button>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Mini provider card
// ---------------------------------------------------------------------------

interface MiniProviderCardProps {
  provider: ProviderId;
  isAuthenticated: boolean;
}

function MiniProviderCard({ provider, isAuthenticated }: MiniProviderCardProps) {
  const [isExpanded, setIsExpanded] = useState(false);

  const label = PROVIDER_LABELS[provider];

  if (isAuthenticated) {
    return (
      <div className="rounded-lg border border-success/50 bg-success/5 p-4 space-y-3">
        <div className="flex items-center gap-2">
          <Check size={16} className="text-success flex-shrink-0" />
          <span className="text-sm font-semibold text-text-primary">{label}</span>
        </div>
        <span className="text-xs text-success">Connected</span>
      </div>
    );
  }

  return (
    <div
      className={cn(
        "rounded-lg border border-border-primary bg-bg-secondary p-4 space-y-3 transition-colors",
        isExpanded ? "border-border-secondary" : "hover:border-border-secondary cursor-pointer",
      )}
    >
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2">
          <Zap size={14} className="text-text-tertiary flex-shrink-0" />
          <span className="text-sm font-semibold text-text-primary">{label}</span>
        </div>
        {!isExpanded && (
          <button
            type="button"
            onClick={() => setIsExpanded(true)}
            className="text-sm text-accent-primary hover:underline"
          >
            Connect
          </button>
        )}
      </div>

      {isExpanded && provider === "anthropic" && (
        <AnthropicMiniForm onConnected={() => setIsExpanded(false)} />
      )}
      {isExpanded && provider === "openai" && (
        <OpenAIMiniForm onConnected={() => setIsExpanded(false)} />
      )}
      {isExpanded && provider === "gemini" && (
        <GeminiMiniForm onConnected={() => setIsExpanded(false)} />
      )}
      {isExpanded && provider === "ollama" && (
        <OllamaMiniForm onConnected={() => setIsExpanded(false)} />
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// ConnectAiStep — exported named component (CLAUDE.md: named exports)
// ---------------------------------------------------------------------------

export function ConnectAiStep({ onContinue, onSkip }: ConnectAiStepProps) {
  const { data: providers } = useProviders();

  // Continue enabled once any provider is authenticated
  const hasConnectedProvider = providers?.some((p) => p.authenticated) ?? false;

  const isProviderAuthenticated = (provider: ProviderId) =>
    providers?.find((p) => p.provider === provider)?.authenticated ?? false;

  return (
    <div className="space-y-6 animate-in fade-in duration-500">
      {/* Heading */}
      <div className="text-center space-y-2">
        <h2 className="page-title text-text-primary">Connect your AI</h2>
        <p className="text-sm text-text-secondary">
          Cortex uses AI to name your Smart Spaces and extract entities. Connect
          any provider to get started.
        </p>
      </div>

      {/* 2×2 provider grid */}
      <div className="grid grid-cols-2 gap-4">
        {(["anthropic", "openai", "gemini", "ollama"] as const).map((provider) => (
          <MiniProviderCard
            key={provider}
            provider={provider}
            isAuthenticated={isProviderAuthenticated(provider)}
          />
        ))}
      </div>

      {/* Continue button — enabled only after any provider connects */}
      <button
        type="button"
        onClick={onContinue}
        disabled={!hasConnectedProvider}
        className={cn(
          "w-full inline-flex items-center justify-center gap-2 rounded-lg px-6 py-3 text-sm font-semibold transition-colors min-h-[44px]",
          hasConnectedProvider
            ? "bg-accent-primary text-white hover:bg-accent-hover"
            : "bg-bg-tertiary text-text-tertiary cursor-not-allowed",
        )}
      >
        Continue
        <ArrowRight size={16} />
      </button>

      {/* Skip link — D-14/D-15: MUST NOT call useAiBannerStore.dismiss().
          Skip only advances the step. Banner appears post-onboarding when
          hasActiveProvider stays false — that is the intended nudge mechanism. */}
      <div className="flex justify-center">
        <button
          type="button"
          onClick={onSkip}
          className="inline-flex items-center gap-1 text-sm text-text-tertiary hover:text-text-secondary transition-colors"
        >
          <SkipForward size={14} />
          Skip for now
        </button>
      </div>
    </div>
  );
}
