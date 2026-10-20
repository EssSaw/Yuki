import { useEffect, useRef, useState } from "react";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import "./ChatPanel.css";
import ModelPicker from "./ModelPicker";

export type MessageStatus = "complete" | "cancelled" | "failed";

export type ChatMessage = {
  id: string;
  role: "user" | "assistant";
  content: string;
  status: MessageStatus;
};

export type ConversationSummary = {
  id: string;
  title: string;
  updatedAt: string;
};

export type ConversationDetail = {
  id: string;
  title: string;
  updatedAt: string;
  messages: ChatMessage[];
};

export type ToolActivity = {
  id: string;
  name: string;
  status: "running" | "succeeded" | "failed";
  summary?: string;
};

type DisplayToolActivity = Omit<ToolActivity, "status"> & {
  status: ToolActivity["status"] | "stopped";
};

export type ChatStreamPayload = {
  requestId: string;
  conversationId?: string;
  delta?: string;
  done?: boolean;
  error?: string;
  status?: MessageStatus;
  toolActivity?: ToolActivity;
};

export type ChatSettings = {
  baseUrl: string;
  model: string;
  hasApiKey: boolean;
};

interface ChatPanelProps {
  onNavigate?: (section: "Projects" | "Notes & canvas" | "Schedule" | "Settings") => void;
}

export default function ChatPanel({ onNavigate }: ChatPanelProps) {
  const [conversations, setConversations] = useState<ConversationSummary[]>([]);
  const [activeConversationId, setActiveConversationId] = useState<string | null>(null);
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [input, setInput] = useState("");
  const [isStreaming, setIsStreaming] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [cancelError, setCancelError] = useState<string | null>(null);
  const [settings, setSettings] = useState<ChatSettings | null>(null);
  const [models, setModels] = useState<string[]>([]);
  const [modelBusy, setModelBusy] = useState(false);
  const [historyOpen, setHistoryOpen] = useState(false);
  const [deleteTarget, setDeleteTarget] = useState<ConversationSummary | null>(null);
  const [deleting, setDeleting] = useState(false);
  const [toolActivities, setToolActivities] = useState<DisplayToolActivity[]>([]);

  function stopToolActivities() {
    setToolActivities((items) => items.map((item) =>
      item.status === "running" ? { ...item, status: "stopped" } : item
    ));
  }

  async function deleteConversation() {
    if (!deleteTarget || isStreaming || deleting) return;
    setDeleting(true); setError(null);
    try {
      await invoke("delete_conversation", { id: deleteTarget.id });
      setConversations((items) => items.filter((item) => item.id !== deleteTarget.id));
      setToolActivities([]);
      if (activeConversationId === deleteTarget.id) { setActiveConversationId(null); setMessages([]); }
      setDeleteTarget(null);
    } catch (reason) { setError(String(reason)); }
    finally { setDeleting(false); }
  }

  async function loadModels() {
    if (!desktop || isStreaming || modelBusy) return;
    setModelBusy(true); setError(null);
    try { setModels(await invoke<string[]>("list_chat_models")); }
    catch (reason) { setError(String(reason)); }
    finally { setModelBusy(false); }
  }

  async function selectModel(model: string) {
    if (!settings || !model || isStreaming || modelBusy) return;
    setModelBusy(true); setError(null);
    try {
      const saved = await invoke<ChatSettings>("save_chat_settings", { baseUrl: settings.baseUrl, model });
      setSettings(saved);
    } catch (reason) { setError(String(reason)); }
    finally { setModelBusy(false); }
  }

  const activeRequestIdRef = useRef<string | null>(null);
  const unlistenRef = useRef<UnlistenFn | null>(null);
  const desktop = isTauri();

  useEffect(() => {
    let active = true;
    if (!desktop) return;

    invoke<ChatSettings>("get_chat_settings")
      .then((s) => {
        if (active) setSettings(s);
      })
      .catch(() => {});

    invoke<ConversationSummary[]>("list_conversations")
      .then((list) => {
        if (active) setConversations(list || []);
      })
      .catch((err: unknown) => {
        if (active) setError(String(err));
      });

    return () => {
      active = false;
      if (activeRequestIdRef.current) {
        invoke("cancel_chat", { requestId: activeRequestIdRef.current }).catch(() => {});
        activeRequestIdRef.current = null;
      }
      if (unlistenRef.current) {
        unlistenRef.current();
        unlistenRef.current = null;
      }
    };
  }, [desktop]);

  async function refreshConversations() {
    if (!desktop) return;
    try {
      const list = await invoke<ConversationSummary[]>("list_conversations");
      setConversations(list || []);
    } catch {
      // ignore
    }
  }

  async function reopenConversation(id: string) {
    if (id === activeConversationId) return;
    setToolActivities([]);

    if (activeRequestIdRef.current) {
      try {
        await invoke("cancel_chat", { requestId: activeRequestIdRef.current });
      } catch {
        // ignore
      }
      activeRequestIdRef.current = null;
      setIsStreaming(false);
      if (unlistenRef.current) {
        unlistenRef.current();
        unlistenRef.current = null;
      }
    }

    setError(null);
    setCancelError(null);

    try {
      const detail = await invoke<ConversationDetail>("load_conversation", { id });
      setActiveConversationId(detail.id);
      setMessages(detail.messages || []);
    } catch (err) {
      setError(`Failed to load conversation: ${String(err)}`);
    }
  }

  async function handleNewChat() {
    setToolActivities([]);
    if (activeRequestIdRef.current) {
      try {
        await invoke("cancel_chat", { requestId: activeRequestIdRef.current });
      } catch {
        // ignore
      }
      activeRequestIdRef.current = null;
      setIsStreaming(false);
      if (unlistenRef.current) {
        unlistenRef.current();
        unlistenRef.current = null;
      }
    }

    setActiveConversationId(null);
    setMessages([]);
    setError(null);
    setCancelError(null);
  }

  async function handleSend(overrideContent?: string) {
    const textToSend = (overrideContent ?? input).trim();
    if (!textToSend || isStreaming || deleting || modelBusy || !settings?.model || !desktop) return;

    const requestId = crypto.randomUUID();
    activeRequestIdRef.current = requestId;
    setToolActivities([]);
    setIsStreaming(true);
    setError(null);
    setCancelError(null);

    const userMsgId = crypto.randomUUID();
    const assistantMsgId = crypto.randomUUID();

    const userMsg: ChatMessage = {
      id: userMsgId,
      role: "user",
      content: textToSend,
      status: "complete",
    };

    const assistantMsg: ChatMessage = {
      id: assistantMsgId,
      role: "assistant",
      content: "",
      status: "complete",
    };

    setMessages((prev) => [...prev, userMsg, assistantMsg]);
    if (!overrideContent) {
      setInput("");
    }

    let unlisten: UnlistenFn | null = null;

    try {
      // 1. Subscribe to 'chat-stream' BEFORE starting chat
      unlisten = await listen<ChatStreamPayload>("chat-stream", (event) => {
        const payload = event.payload;
        // Accept only events matching current requestId
        if (!payload || payload.requestId !== requestId || activeRequestIdRef.current !== requestId) {
          return;
        }

        if (payload.conversationId) {
          setActiveConversationId(payload.conversationId);
        }

        if (payload.toolActivity) {
          const activity = payload.toolActivity;
          setToolActivities((items) => {
            const existing = items.find((item) => item.id === activity.id);
            // A late running event must not revive a completed operation.
            if (existing && existing.status !== "running" && activity.status === "running") return items;
            return [...items.filter((item) => item.id !== activity.id), activity].slice(-16);
          });
        }

        if (payload.done || payload.error || payload.status === "cancelled" || payload.status === "failed") {
          stopToolActivities();
        }

        if (payload.delta) {
          setMessages((prev) =>
            prev.map((m) =>
              m.id === assistantMsgId
                ? { ...m, content: m.content + payload.delta }
                : m
            )
          );
        }

        if (payload.error) {
          setError(payload.error);
          setMessages((prev) =>
            prev.map((m) =>
              m.id === assistantMsgId ? { ...m, status: "failed" } : m
            )
          );
        }

        if (payload.status) {
          setMessages((prev) =>
            prev.map((m) =>
              m.id === assistantMsgId ? { ...m, status: payload.status! } : m
            )
          );
        }

        if (payload.done) {
          setIsStreaming(false);
          activeRequestIdRef.current = null;
          if (unlistenRef.current) {
            unlistenRef.current();
            unlistenRef.current = null;
          }
          void refreshConversations();
        }
      });

      if (activeRequestIdRef.current !== requestId) { unlisten(); return; }
      unlistenRef.current = unlisten;

      // 2. Invoke start_chat
      const res = await invoke<{ conversationId: string }>("start_chat", {
        conversationId: activeConversationId,
        content: textToSend,
        requestId,
      });

      if (res?.conversationId && activeRequestIdRef.current === requestId) {
        setActiveConversationId(res.conversationId);
      }
    } catch (err) {
      if (activeRequestIdRef.current !== requestId) return;
      stopToolActivities();
      const errMsg = String(err);
      setError(errMsg);
      setMessages((prev) =>
        prev.map((m) =>
          m.id === assistantMsgId ? { ...m, status: "failed" } : m
        )
      );
      setIsStreaming(false);
      activeRequestIdRef.current = null;
      if (unlistenRef.current) {
        unlistenRef.current();
        unlistenRef.current = null;
      }
    }
  }

  async function handleCancel() {
    if (!activeRequestIdRef.current) return;
    const reqId = activeRequestIdRef.current;
    setCancelError(null);

    try {
      await invoke("cancel_chat", { requestId: reqId });
      // Acknowledgement is not terminal: keep listening until persisted done arrives.
    } catch (err) {
      // Don't show success on rejected cancel
      setCancelError(`Failed to cancel: ${String(err)}`);
    }
  }

  function handleRetry(failedMsgId: string) {
    if (isStreaming || !desktop) return;
    const index = messages.findIndex((m) => m.id === failedMsgId);
    if (index > 0 && messages[index - 1]?.role === "user") {
      const prompt = messages[index - 1].content;
      // Remove the failed assistant message and preceding user message
      setMessages((prev) => prev.filter((_, i) => i !== index && i !== index - 1));
      void handleSend(prompt);
    }
  }

  return (
    <section
      className="chat-view"
      aria-labelledby={messages.length === 0 ? "welcome-title" : undefined}
      aria-label={messages.length > 0 ? "Chat conversation" : undefined}
    >
      <div className="chat-history-bar">
        <div className="chat-heading">
          <span className="chat-heading-mark" aria-hidden="true">✦</span>
          <div>
            <strong>Yuki</strong>
            <small>A little space to think together</small>
          </div>
        </div>
        <div className="chat-header-actions">
          <button type="button" onClick={() => setHistoryOpen(!historyOpen)} aria-expanded={historyOpen}>
            Conversations <span>{conversations.length}</span>
          </button>
          <button type="button" className="new-chat-btn" onClick={handleNewChat} disabled={!desktop || isStreaming || deleting}>
            + New chat
          </button>
        </div>
      </div>
      {historyOpen && (
        <section className="conversation-drawer" aria-label="Recent conversations">
          <div className="conversation-drawer-heading">
            <strong>Your conversations</strong>
            <button onClick={() => setHistoryOpen(false)} aria-label="Close conversations">Close</button>
          </div>
          {!conversations.length && <p>No saved conversations yet.</p>}
          {conversations.map((conversation) => (
            <div className="conversation-row" key={conversation.id}>
              <button
                type="button"
                aria-pressed={conversation.id === activeConversationId}
                disabled={isStreaming || deleting}
                onClick={() => { void reopenConversation(conversation.id); setHistoryOpen(false); }}
              >
                {conversation.title || "Untitled conversation"}
              </button>
              <button
                type="button"
                className="conversation-delete"
                aria-label={`Delete conversation: ${conversation.title || "Untitled conversation"}`}
                disabled={isStreaming || deleting}
                onClick={() => setDeleteTarget(conversation)}
              >
                Delete
              </button>
            </div>
          ))}
        </section>
      )}
      {deleteTarget && (
        <section className="delete-confirmation" role="alertdialog" aria-labelledby="delete-title" aria-describedby="delete-detail">
          <strong id="delete-title">Delete this conversation?</strong>
          <p id="delete-detail">"{deleteTarget.title || "Untitled conversation"}" and its saved history will be permanently removed. Project files stay untouched.</p>
          <div>
            <button disabled={deleting} onClick={() => setDeleteTarget(null)}>Keep conversation</button>
            <button className="confirm-delete" disabled={deleting || isStreaming} onClick={() => void deleteConversation()}>
              {deleting ? "Deleting…" : "Delete permanently"}
            </button>
          </div>
        </section>
      )}

      {!desktop && (
        <p className="browser-warning" role="status">
          Open the desktop app to chat with Yuki. Browser preview has no native or provider access.
        </p>
      )}

      {error && (
        <p className="chat-error" role="alert">
          {error}
        </p>
      )}

      {cancelError && (
        <p className="chat-error" role="alert">
          {cancelError}
        </p>
      )}

      {messages.length === 0 ? (
        <div className="welcome">
          <div className="portrait-placeholder" aria-label="Avatar placeholder">
            <span className="portrait-mark">Y</span>
            <span className="portrait-spark" aria-hidden="true">✦</span>
          </div>
          <p className="eyebrow">AT YOUR PACE, ON YOUR TERMS</p>
          <h1 id="welcome-title">
            A little space to <br />
            think together.
          </h1>
          <p className="intro">
            Code, collect an idea, or untangle a plan.<br />
            This is the beginning of your space with Yuki.
          </p>
          <div className="workspace-links">
            <button type="button" onClick={() => onNavigate?.("Projects")}>
              Explore a project <span aria-hidden="true">&gt;</span>
            </button>
            <button type="button" onClick={() => onNavigate?.("Notes & canvas")}>
              Make room for an idea <span aria-hidden="true">&gt;</span>
            </button>
          </div>
        </div>
      ) : (
        <div className="chat-messages" role="log" aria-live="polite" aria-label="Chat messages">
          {messages.map((msg) => (
            <article
              key={msg.id}
              className={`chat-message chat-message-${msg.role}`}
              data-status={msg.status}
            >
              <div className="chat-message-header">
                <span className="chat-message-role">
                  {msg.role === "user" ? "You" : "Yuki"}
                </span>
                {msg.status !== "complete" && (
                  <span
                    className={`chat-message-status chat-message-status-${msg.status}`}
                  >
                    {msg.status === "cancelled" ? "Cancelled" : "Failed"}
                  </span>
                )}
              </div>
              <div className="chat-message-content">{msg.content}</div>
              {msg.status === "failed" && (
                <button
                  type="button"
                  className="chat-retry-btn"
                  aria-label="Retry message"
                  onClick={() => handleRetry(msg.id)}
                  disabled={!desktop || isStreaming}
                >
                  Retry
                </button>
              )}
            </article>
          ))}
        </div>
      )}

      {toolActivities.length > 0 && (
        <section className="tool-activity" aria-label="Tool activity">
          <div className="tool-activity-heading">
            <strong>Tool activity</strong>
            <span>Current turn only</span>
          </div>
          <ol className="tool-activity-list">
            {toolActivities.map((activity) => (
              <li key={activity.id} className="tool-activity-row" data-status={activity.status}>
                <span className="tool-activity-indicator" aria-hidden="true" />
                <span className="tool-activity-summary">{activity.summary || "Tool operation"}</span>
                <span className="tool-activity-status">
                  {activity.status === "running" ? "Running" : activity.status === "succeeded" ? "Succeeded" : activity.status === "failed" ? "Failed" : "Stopped - completion not confirmed"}
                </span>
              </li>
            ))}
          </ol>
        </section>
      )}

      <div className="composer">
        <label className="composer-label" htmlFor="message">Message Yuki</label>
        <textarea
          id="message"
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              void handleSend();
            }
          }}
          placeholder={
            !desktop
              ? "Chat is disabled in browser preview."
              : isStreaming
              ? "Yuki is responding..."
              : !settings?.model
              ? "Chat becomes available when your model is connected."
              : "Message Yuki..."
          }
          disabled={!desktop || isStreaming}
          rows={2}
        />
        <div className="composer-footer">
          <ModelPicker model={settings?.model ?? ""} models={models} busy={modelBusy}
            disabled={!desktop || isStreaming || deleting || !settings} onLoad={() => void loadModels()} onSelect={selectModel} />
          <div className="composer-actions">
            {isStreaming && (
              <button
                type="button"
                onClick={handleCancel}
                aria-label="Cancel response"
                className="cancel-btn"
              >
                Cancel
              </button>
            )}
            <button
              type="button"
              onClick={() => void handleSend()}
              aria-label="Send message"
              className="send-button"
              disabled={!desktop || deleting || isStreaming || modelBusy || !input.trim() || !settings?.model}
            >
              <span className="send-icon" aria-hidden="true">
                <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round">
                  <line x1="22" y1="2" x2="11" y2="13" />
                  <polygon points="22 2 15 22 11 13 2 9 22 2" />
                </svg>
              </span>
            </button>
          </div>
        </div>
      </div>
    </section>
  );
}
