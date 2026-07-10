import { useEffect, useRef, useState } from "react";
import { Link } from "react-router-dom";
import { Send, Loader2, MessageSquarePlus, Trash2, FileText } from "lucide-react";
import { Button } from "@/components/ui/button";
import { ScrollArea } from "@/components/ui/scroll-area";
import { tauriInvoke, isTauri } from "@/lib/tauri";
import { cn } from "@/lib/utils";

type ChatRole = "user" | "assistant";

interface Citation {
  index: number;
  docId: string;
  docTitle: string;
  chunkStart: number;
  chunkEnd: number;
}

interface ChatMessage {
  id: string;
  role: ChatRole;
  content: string;
  citations?: Citation[] | null;
  createdAt: string;
}

interface ChatSession {
  id: string;
  title: string;
  createdAt: string;
  updatedAt: string;
  messages: ChatMessage[];
}

interface StartChatResult {
  sessionId: string;
  userMessageId: string;
  assistantMessageId: string;
}

interface TokenPayload {
  sessionId: string;
  messageId: string;
  token: string;
  cumulativeIndex: number;
}

interface CompletePayload {
  sessionId: string;
  messageId: string;
  citations: Citation[];
  inputTokens: number | null;
  outputTokens: number | null;
}

interface ErrorPayload {
  sessionId: string;
  messageId: string;
  error: string;
}

interface SuggestionsPayload {
  sessionId: string;
  messageId: string;
  suggestions: string[];
}

export default function ChatPage() {
  const [sessions, setSessions] = useState<ChatSession[]>([]);
  const [activeSessionId, setActiveSessionId] = useState<string | null>(null);
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [input, setInput] = useState("");
  const [streaming, setStreaming] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [suggestions, setSuggestions] = useState<string[]>([]);
  const scrollRef = useRef<HTMLDivElement>(null);
  const unlistenersRef = useRef<Array<() => void>>([]);

  useEffect(() => {
    void refreshSessions();
    return () => {
      unlistenersRef.current.forEach((u) => u());
      unlistenersRef.current = [];
    };
  }, []);

  useEffect(() => {
    if (scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [messages]);

  async function refreshSessions() {
    try {
      const list = await tauriInvoke<ChatSession[]>(
        "list_chat_sessions",
        undefined,
        () => [],
      );
      setSessions(list);
    } catch (e) {
      console.error("list_chat_sessions failed", e);
    }
  }

  function loadSession(session: ChatSession) {
    setActiveSessionId(session.id);
    setMessages(session.messages);
    setError(null);
    setSuggestions([]);
  }

  function newChat() {
    setActiveSessionId(null);
    setMessages([]);
    setError(null);
    setSuggestions([]);
  }

  async function deleteSession(id: string, e: React.MouseEvent) {
    e.stopPropagation();
    if (!confirm("Delete this chat?")) return;
    try {
      await tauriInvoke("delete_chat_session", { sessionId: id });
      if (activeSessionId === id) newChat();
      await refreshSessions();
    } catch (e) {
      console.error("delete_chat_session failed", e);
    }
  }

  async function send(overrideQuery?: string) {
    const query = (overrideQuery ?? input).trim();
    if (!query || streaming) return;
    setInput("");
    setError(null);
    setSuggestions([]);

    const userTemp: ChatMessage = {
      id: `tmp-user-${Date.now()}`,
      role: "user",
      content: query,
      createdAt: new Date().toISOString(),
    };
    setMessages((prev) => [...prev, userTemp]);
    setStreaming(true);

    if (!isTauri()) {
      setTimeout(() => {
        setMessages((prev) => [
          ...prev,
          {
            id: `tmp-a-${Date.now()}`,
            role: "assistant",
            content:
              "Chat requires the Tauri desktop shell. Run `cargo tauri dev` to test end-to-end.",
            createdAt: new Date().toISOString(),
          },
        ]);
        setStreaming(false);
      }, 400);
      return;
    }

    try {
      const { listen } = await import("@tauri-apps/api/event");

      const result = await tauriInvoke<StartChatResult>("start_chat", {
        args: {
          query,
          sessionId: activeSessionId,
          filters: null,
        },
      });

      setActiveSessionId(result.sessionId);
      const assistantId = result.assistantMessageId;

      setMessages((prev) => {
        const next = prev.filter((m) => m.id !== userTemp.id);
        return [
          ...next,
          {
            id: result.userMessageId,
            role: "user",
            content: query,
            createdAt: new Date().toISOString(),
          },
          {
            id: assistantId,
            role: "assistant",
            content: "",
            createdAt: new Date().toISOString(),
            citations: [],
          },
        ];
      });

      const unlistenToken = await listen<TokenPayload>(
        "chat-stream-token",
        (evt) => {
          if (evt.payload.messageId !== assistantId) return;
          setMessages((prev) =>
            prev.map((m) =>
              m.id === assistantId
                ? { ...m, content: m.content + evt.payload.token }
                : m,
            ),
          );
        },
      );

      const unlistenComplete = await listen<CompletePayload>(
        "chat-stream-complete",
        (evt) => {
          if (evt.payload.messageId !== assistantId) return;
          setMessages((prev) =>
            prev.map((m) =>
              m.id === assistantId
                ? { ...m, citations: evt.payload.citations }
                : m,
            ),
          );
          setStreaming(false);
          cleanupListeners();
          void refreshSessions();
        },
      );

      const unlistenError = await listen<ErrorPayload>(
        "chat-stream-error",
        (evt) => {
          if (evt.payload.messageId !== assistantId) return;
          setError(evt.payload.error);
          setStreaming(false);
          cleanupListeners();
        },
      );

      // Suggestions arrive AFTER chat-stream-complete (backend emits them
      // once persist finishes) — kept out of cleanupListeners() so it isn't
      // torn down by the completion handler before it fires.
      const unlistenSuggestions = await listen<SuggestionsPayload>(
        "chat-stream-suggestions",
        (evt) => {
          if (evt.payload.messageId !== assistantId) return;
          setSuggestions(evt.payload.suggestions);
          unlistenSuggestions();
          unlistenersRef.current = unlistenersRef.current.filter(
            (u) => u !== unlistenSuggestions,
          );
        },
      );

      unlistenersRef.current.push(
        unlistenToken,
        unlistenComplete,
        unlistenError,
        unlistenSuggestions,
      );

      function cleanupListeners() {
        unlistenToken();
        unlistenComplete();
        unlistenError();
        unlistenersRef.current = unlistenersRef.current.filter(
          (u) => u !== unlistenToken && u !== unlistenComplete && u !== unlistenError,
        );
      }
    } catch (e) {
      console.error("start_chat failed", e);
      setError(String(e));
      setStreaming(false);
    }
  }

  function onKeyDown(e: React.KeyboardEvent<HTMLTextAreaElement>) {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      void send();
    }
  }

  return (
    <div className="flex h-[calc(100vh-52px)] w-full">
      <aside className="flex w-64 flex-col border-r border-border bg-background/50">
        <div className="border-b border-border p-3">
          <Button
            variant="outline"
            className="w-full justify-start gap-2"
            onClick={newChat}
          >
            <MessageSquarePlus className="h-4 w-4" />
            New chat
          </Button>
        </div>
        <ScrollArea className="flex-1">
          <div className="space-y-1 p-2">
            {sessions.length === 0 && (
              <p className="p-3 text-sm text-muted-foreground">
                No conversations yet.
              </p>
            )}
            {sessions.map((s) => (
              <button
                key={s.id}
                onClick={() => loadSession(s)}
                className={cn(
                  "group flex w-full items-center gap-2 rounded-md px-3 py-2 text-left text-sm transition-colors hover:bg-accent",
                  activeSessionId === s.id && "bg-accent",
                )}
              >
                <span className="flex-1 truncate">{s.title}</span>
                <Trash2
                  className="h-3.5 w-3.5 shrink-0 opacity-0 transition-opacity hover:text-destructive group-hover:opacity-100"
                  onClick={(e) => deleteSession(s.id, e)}
                />
              </button>
            ))}
          </div>
        </ScrollArea>
      </aside>

      <main className="flex flex-1 flex-col">
        <div className="border-b border-border px-6 py-3">
          <h1 className="text-lg font-semibold">Chat with your docs</h1>
          <p className="text-xs text-muted-foreground">
            Ask a question in plain English — answers are grounded in your indexed
            documents with citations.
          </p>
        </div>

        <ScrollArea className="flex-1" ref={scrollRef as any}>
          <div className="mx-auto max-w-3xl space-y-4 p-6">
            {messages.length === 0 && (
              <div className="mt-20 text-center text-muted-foreground">
                <p className="mb-2 text-lg">Start a conversation</p>
                <p className="text-sm">
                  Try: <em>"Which docs mention my property tax?"</em>
                </p>
              </div>
            )}
            {messages.map((m) => (
              <MessageBubble key={m.id} message={m} />
            ))}
            {error && (
              <div className="rounded-md border border-destructive/50 bg-destructive/10 p-3 text-sm text-destructive">
                {error}
              </div>
            )}
          </div>
        </ScrollArea>

        {!streaming && suggestions.length > 0 && (
          <div className="border-t border-border/50 px-6 py-3">
            <div className="mx-auto flex max-w-3xl flex-wrap gap-2">
              {suggestions.map((s, i) => (
                <button
                  key={i}
                  onClick={() => void send(s)}
                  className="rounded-full border border-border bg-background px-3 py-1.5 text-xs text-muted-foreground transition-colors hover:border-primary hover:text-foreground"
                >
                  {s}
                </button>
              ))}
            </div>
          </div>
        )}

        <div className="border-t border-border p-4">
          <div className="mx-auto flex max-w-3xl items-end gap-2">
            <textarea
              value={input}
              onChange={(e) => setInput(e.target.value)}
              onKeyDown={onKeyDown}
              placeholder="Ask about your documents…"
              rows={2}
              disabled={streaming}
              className="flex-1 resize-none rounded-md border border-input bg-background px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-ring disabled:opacity-50"
            />
            <Button onClick={() => void send()} disabled={streaming || !input.trim()}>
              {streaming ? (
                <Loader2 className="h-4 w-4 animate-spin" />
              ) : (
                <Send className="h-4 w-4" />
              )}
            </Button>
          </div>
        </div>
      </main>
    </div>
  );
}

function MessageBubble({ message }: { message: ChatMessage }) {
  const isUser = message.role === "user";
  return (
    <div className={cn("flex gap-3", isUser ? "justify-end" : "justify-start")}>
      <div
        className={cn(
          "max-w-[80%] rounded-lg px-4 py-3 text-sm",
          isUser
            ? "bg-primary text-primary-foreground"
            : "bg-muted text-foreground",
        )}
      >
        <div className="whitespace-pre-wrap">{message.content || (
          !isUser && <span className="text-muted-foreground">…</span>
        )}</div>
        {message.citations && message.citations.length > 0 && (
          <div className="mt-3 space-y-1 border-t border-border/50 pt-2">
            <p className="text-xs font-semibold text-muted-foreground">Sources:</p>
            {message.citations.map((c) => (
              <Link
                key={`${c.docId}-${c.index}`}
                to={`/document/${c.docId}?highlight=${c.chunkStart}-${c.chunkEnd}`}
                className="flex items-center gap-1.5 text-xs text-primary hover:underline"
              >
                <FileText className="h-3 w-3" />
                <span>[{c.index}] {c.docTitle}</span>
              </Link>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
