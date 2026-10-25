import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import ChatMarkdown from "@/components/ChatMarkdown";
import PanelToggle from "@/components/PanelToggle";
import {
  EFFORTS,
  cancelLogin,
  chatStatus,
  deleteConversation,
  isLoginError,
  listConversations,
  listMessages,
  loginClaude,
  runningConversations,
  sendChat,
  setConversationEffort,
  setConversationModel,
  stopChat,
  type ChatStatus,
  type Conversation,
  type Effort,
  type Message,
  type Part,
  type ToolEvent,
} from "@/lib/chatApi";
import { useI18n } from "@/lib/i18n";
import { selectionContext, selectionLabel, type RollSelection } from "@/lib/rollSelection";
import { libraryAvailable } from "@/lib/songsApi";
import type { SongLibrary } from "@/lib/useSongLibrary";
import type { Messages } from "@/locales/en";

/**
 * Aliases, not versions: Claude Code resolves each to the newest model of
 * that family, so a new release is picked up without touching this list.
 */
const MODELS: { value: string | null; label: string | null }[] = [
  /** null: the localized "Auto". */
  { value: null, label: null },
  { value: "fable", label: "Fable" },
  { value: "opus", label: "Opus" },
  { value: "sonnet", label: "Sonnet" },
  { value: "haiku", label: "Haiku" },
];

const EFFORT_LABEL: Record<Effort, string> = {
  low: "Low",
  medium: "Medium",
  high: "High",
  xhigh: "XHigh",
  max: "Max",
};

/** Haiku has no effort control; the backend leaves the flag off for it. */
const takesEffort = (model: string | null) => !model?.includes("haiku");

const LAST_CONV_KEY = "otomo.chat.conv";
/** The last model and effort picked, as the start for a new conversation. */
const MODEL_KEY = "otomo.chat.model";
const EFFORT_KEY = "otomo.chat.effort";

function recall(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

function remember(key: string, value: string | null) {
  try {
    if (value == null) localStorage.removeItem(key);
    else localStorage.setItem(key, value);
  } catch {
    /* private window: only the convenience is lost */
  }
}

function recallEffort(): Effort | null {
  const saved = recall(EFFORT_KEY);
  return EFFORTS.find((e) => e === saved) ?? null;
}

/** "bars 5–8 · track 2" — the part of a call's input worth a glance. */
function summarize(input: Record<string, unknown>, t: Messages["chat"]): string {
  const v = (k: string) => input[k] as string | number | undefined;
  const out: string[] = [];
  if (v("query")) out.push(t.quoted(String(v("query"))));
  if (typeof input.url === "string") out.push(shortUrl(input.url));
  if (v("pages")) out.push(t.pages(String(v("pages"))));
  if (v("title")) out.push(t.quoted(String(v("title"))));
  if (v("from_track") != null && v("to_track") != null)
    out.push(t.trackToTrack(String(v("from_track")), String(v("to_track"))));
  else if (v("track") != null) out.push(t.track(String(v("track"))));
  if (Array.isArray(input.tracks)) out.push(t.onlyTracks(input.tracks.join(",")));
  if (v("from_bar") != null)
    out.push(
      v("to_bar") != null && v("to_bar") !== v("from_bar")
        ? t.bars(String(v("from_bar")), String(v("to_bar")))
        : t.bar(String(v("from_bar")))
    );
  else if (v("at_bar") != null) out.push(t.bar(String(v("at_bar"))));
  const bpm = v("to_bpm") ?? v("tempo");
  if (bpm != null) out.push(`${bpm} BPM`);
  if (v("key")) out.push(String(v("key")));
  const sig = v("time_signature") ?? v("signature");
  if (sig) out.push(String(sig));
  if (v("instrument")) out.push(String(v("instrument")));
  if (Array.isArray(input.instruments)) out.push(input.instruments.join(t.instrumentSeparator));
  if (input.loop) out.push(t.loop);
  return out.join(" · ");
}

/** "commons.wikimedia.org/…/Il_Canto.mid": the host and the file, not the query. */
function shortUrl(url: string): string {
  try {
    const u = new URL(url);
    const file = decodeURIComponent(u.pathname.split("/").filter(Boolean).pop() ?? "");
    return file ? `${u.host}/…/${file}` : u.host;
  } catch {
    return url;
  }
}

function emptyAssistant(): Message {
  return {
    id: -1,
    role: "assistant",
    parts: [],
    createdAt: Date.now() / 1000,
    model: null,
    effort: null,
    costUsd: null,
    durationMs: null,
    error: null,
    stopped: false,
  };
}

function withTool(m: Message, e: ToolEvent): Message {
  const parts = [...m.parts];
  const i = parts.findIndex((p) => p.type === "tool" && p.id === e.id);
  if (i < 0) {
    parts.push({
      type: "tool",
      id: e.id,
      name: e.name ?? "",
      input: e.input ?? {},
      status: e.status ?? "running",
      result: e.result,
    });
  } else {
    const p = parts[i] as Extract<Part, { type: "tool" }>;
    parts[i] = {
      ...p,
      name: e.name ?? p.name,
      input: e.input ?? p.input,
      status: e.status ?? p.status,
      result: e.result ?? p.result,
    };
  }
  return { ...m, parts };
}

function withText(m: Message, text: string): Message {
  const parts = [...m.parts];
  const last = parts[parts.length - 1];
  if (last?.type === "text") parts[parts.length - 1] = { type: "text", text: last.text + text };
  else parts.push({ type: "text", text });
  return { ...m, parts };
}

/**
 * Claude, beside the song: ask for music, hear it, keep it or not.
 *
 * Stays mounted while collapsed so an answer keeps streaming in. Edits land
 * as the same pending edits Claude Desktop makes, so approving one is the
 * banner's Apply — or the copy of it at the bottom of this panel.
 */
export default function ChatPanel({
  library,
  open,
  onToggle,
  width,
  onResize,
  attachments,
  onAttachmentsChange,
  home,
  homeSlot,
  onHomeSend,
  onRunningChange,
}: {
  library: SongLibrary;
  /**
   * The launch screen is up: the composer moves to its middle (`homeSlot`)
   * and the panel steps aside until the first message is sent.
   */
  home: boolean;
  homeSlot: HTMLElement | null;
  /** A message went from the launch screen; the app goes to its usual layout. */
  onHomeSend: () => void;
  /** Whether Claude is working on anything, for the launch screen. */
  onRunningChange: (running: boolean) => void;
  open: boolean;
  onToggle: () => void;
  width: number;
  onResize: (width: number) => void;
  /** Piano-roll ranges waiting to go with the next message. */
  attachments: RollSelection[];
  onAttachmentsChange: (next: RollSelection[]) => void;
}) {
  const { t } = useI18n();
  const [status, setStatus] = useState<ChatStatus | null>(null);
  const [convs, setConvs] = useState<Conversation[]>([]);
  const [activeId, setActiveId] = useState<number | null>(() => {
    try {
      const saved = localStorage.getItem(LAST_CONV_KEY);
      return saved ? Number(saved) : null;
    } catch {
      return null;
    }
  });
  const [messages, setMessages] = useState<Message[]>([]);
  const [live, setLive] = useState<Record<number, Message>>({});
  const [running, setRunning] = useState<Set<number>>(new Set());
  const [draft, setDraft] = useState("");
  const [newModel, setNewModel] = useState<string | null>(() => recall(MODEL_KEY));
  const [newEffort, setNewEffort] = useState<Effort | null>(recallEffort);
  const [historyOpen, setHistoryOpen] = useState(false);
  const [confirmDelete, setConfirmDelete] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);

  const activeRef = useRef(activeId);
  activeRef.current = activeId;
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const stick = useRef(true);
  const inputRef = useRef<HTMLTextAreaElement | null>(null);

  const { refreshSongs, refreshPending } = library;

  const reloadConvs = useCallback(async () => {
    try {
      setConvs(await listConversations());
    } catch (e) {
      console.error("chat list", e);
    }
  }, []);

  const reloadMessages = useCallback(async (convId: number | null) => {
    if (convId == null) {
      setMessages([]);
      return;
    }
    try {
      const list = await listMessages(convId);
      if (activeRef.current === convId) setMessages(list);
    } catch (e) {
      console.error("chat messages", e);
    }
  }, []);

  const recheck = useCallback(() => void chatStatus().then(setStatus), []);

  useEffect(() => {
    if (!libraryAvailable()) return;
    recheck();
    void reloadConvs();
    void runningConversations().then((ids) => {
      setRunning(new Set(ids));
      setLive(Object.fromEntries(ids.map((id) => [id, emptyAssistant()])));
    });
  }, [reloadConvs, recheck]);

  // Claude Code installed while the panel said it was missing: notice it when
  // the person comes back from Terminal, without a restart.
  const blocked = !!status?.blocked;
  useEffect(() => {
    if (!blocked) return;
    window.addEventListener("focus", recheck);
    return () => window.removeEventListener("focus", recheck);
  }, [blocked, recheck]);

  useEffect(() => {
    try {
      if (activeId == null) localStorage.removeItem(LAST_CONV_KEY);
      else localStorage.setItem(LAST_CONV_KEY, String(activeId));
    } catch {
      /* private window: only the convenience is lost */
    }
    stick.current = true;
    void reloadMessages(activeId);
  }, [activeId, reloadMessages]);

  // A remembered conversation that has since been deleted.
  useEffect(() => {
    if (activeId != null && convs.length > 0 && !convs.some((c) => c.id === activeId)) {
      setActiveId(null);
    }
  }, [convs, activeId]);

  useEffect(() => {
    if (!libraryAvailable()) return;
    const offs: (() => void)[] = [];
    let cancelled = false;
    void (async () => {
      const { listen } = await import("@tauri-apps/api/event");
      const subs = await Promise.all([
        listen<{ convId: number; text: string }>("chat://delta", (e) => {
          setLive((prev) => ({
            ...prev,
            [e.payload.convId]: withText(prev[e.payload.convId] ?? emptyAssistant(), e.payload.text),
          }));
        }),
        listen<ToolEvent>("chat://tool", (e) => {
          setLive((prev) => ({
            ...prev,
            [e.payload.convId]: withTool(prev[e.payload.convId] ?? emptyAssistant(), e.payload),
          }));
          // A song may have been created, or an edit staged.
          if (e.payload.status === "ok") {
            void refreshSongs();
            void refreshPending();
          }
        }),
        listen<{ convId: number; message: Message }>("chat://done", (e) => {
          const id = e.payload.convId;
          setLive((prev) => {
            const next = { ...prev };
            delete next[id];
            return next;
          });
          setRunning((prev) => {
            const next = new Set(prev);
            next.delete(id);
            return next;
          });
          if (activeRef.current === id) void reloadMessages(id);
          void reloadConvs();
          void refreshSongs();
          void refreshPending();
        }),
      ]);
      if (cancelled) subs.forEach((un) => un());
      else offs.push(...subs);
    })();
    return () => {
      cancelled = true;
      offs.forEach((un) => un());
    };
  }, [reloadConvs, reloadMessages, refreshSongs, refreshPending]);

  // Follow the answer as it grows, unless the reader has scrolled up.
  useLayoutEffect(() => {
    const el = scrollRef.current;
    if (el && stick.current) el.scrollTop = el.scrollHeight;
  }, [messages, live, activeId, open]);

  useEffect(() => {
    if (open) inputRef.current?.focus();
  }, [open, activeId]);

  // A range just handed over: the next thing to do is say what to do with it.
  const attachedCount = useRef(attachments.length);
  useEffect(() => {
    if (attachments.length > attachedCount.current) inputRef.current?.focus();
    attachedCount.current = attachments.length;
  }, [attachments.length]);

  const active = convs.find((c) => c.id === activeId) ?? null;
  const model = active ? active.model : newModel;
  const effort = active ? active.effort : newEffort;
  const effortOn = takesEffort(model);
  const busy = activeId != null && running.has(activeId);
  const song = library.current;

  useEffect(() => onRunningChange(running.size > 0), [running, onRunningChange]);

  // A message from the launch screen always starts a new conversation.
  const send = async (text = draft, fresh = home) => {
    const body = text.trim();
    if ((!body && attachments.length === 0) || (busy && !fresh)) return;
    if (fresh) onHomeSend();
    setError(null);
    setDraft("");
    const attached = attachments;
    const selections = attached.map((a, i) => ({
      label: selectionLabel(a),
      context: selectionContext(a, i + 1),
    }));
    onAttachmentsChange([]);
    stick.current = true;
    const convId = fresh ? null : activeId;
    const optimistic: Message = {
      ...emptyAssistant(),
      role: "user",
      parts: [
        ...selections.map((sel) => ({ type: "selection" as const, ...sel })),
        ...(body ? [{ type: "text" as const, text: body }] : []),
      ],
    };
    setMessages((prev) => (fresh ? [optimistic] : [...prev, optimistic]));
    try {
      const conv = await sendChat({
        convId,
        text: body,
        song: song ? { id: song.id, title: song.title } : null,
        selections,
        model: convId == null ? newModel : null,
        effort: convId == null ? newEffort : null,
      });
      setRunning((prev) => new Set(prev).add(conv.id));
      setLive((prev) => ({ ...prev, [conv.id]: prev[conv.id] ?? emptyAssistant() }));
      // Listed before it is selected, or the "was it deleted?" check below
      // would drop it while the list is still being re-read.
      setConvs((prev) => [conv, ...prev.filter((c) => c.id !== conv.id)]);
      if (convId == null) {
        activeRef.current = conv.id;
        setActiveId(conv.id);
      } else {
        void reloadMessages(conv.id);
      }
      void reloadConvs();
    } catch (e) {
      setError(String(e));
      setDraft(body);
      onAttachmentsChange(attached);
      setMessages((prev) => prev.filter((m) => m !== optimistic));
    }
  };

  // A pick also becomes where the next new conversation starts.
  const chooseModel = async (value: string | null) => {
    setNewModel(value);
    remember(MODEL_KEY, value);
    if (!active) return;
    try {
      await setConversationModel(active.id, value);
      setConvs((prev) => prev.map((c) => (c.id === active.id ? { ...c, model: value } : c)));
    } catch (e) {
      setError(String(e));
    }
  };

  const chooseEffort = async (value: Effort | null) => {
    setNewEffort(value);
    remember(EFFORT_KEY, value);
    if (!active) return;
    try {
      await setConversationEffort(active.id, value);
      setConvs((prev) => prev.map((c) => (c.id === active.id ? { ...c, effort: value } : c)));
    } catch (e) {
      setError(String(e));
    }
  };

  const startResize = (e: React.PointerEvent) => {
    e.preventDefault();
    const move = (ev: PointerEvent) =>
      onResize(Math.min(640, Math.max(300, window.innerWidth - ev.clientX)));
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      document.body.classList.remove("resizing");
    };
    document.body.classList.add("resizing");
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  };

  // One toggle for both states, fixed in the title bar's top-right corner.
  const toggle = (
    <PanelToggle
      side="right"
      open={open}
      onToggle={onToggle}
      title={open ? t.chat.closeTitle : t.chat.open}
      label={t.chat.openLabel}
      busy={running.size > 0}
    />
  );

  // The message box: at the foot of the panel, or in the launch screen.
  const composer = (where: "panel" | "home") => (
    <div className={`chat-compose${where === "home" ? " is-home" : ""}`}>
      {error && (
        <p className="chat-error" onClick={() => setError(null)}>
          {error}
        </p>
      )}
      {attachments.length > 0 && (
        <div className="chat-attachments">
          {attachments.map((a, i) => (
            <AttachmentChip
              key={a.id}
              selection={a}
              number={i + 1}
              otherSong={a.songId !== (song?.id ?? null)}
              onRemove={() => onAttachmentsChange(attachments.filter((x) => x.id !== a.id))}
              onToggleTrack={(track) =>
                onAttachmentsChange(
                  attachments.map((x) =>
                    x.id !== a.id
                      ? x
                      : {
                          ...x,
                          excluded: x.excluded.includes(track)
                            ? x.excluded.filter((t) => t !== track)
                            : [...x.excluded, track],
                        }
                  )
                )
              }
            />
          ))}
        </div>
      )}
      <textarea
        ref={inputRef}
        className="chat-input"
        rows={where === "home" ? 3 : 2}
        autoFocus={where === "home"}
        value={draft}
        disabled={!!status?.blocked}
        placeholder={
          where === "home"
            ? t.home.placeholder
            : attachments.length > 0
            ? t.chat.placeholderSelection
            : song
              ? t.chat.placeholderSong(song.title)
              : t.chat.placeholderNew
        }
        onChange={(e) => setDraft(e.target.value)}
        onInput={(e) => {
          const el = e.currentTarget;
          el.style.height = "auto";
          el.style.height = `${Math.min(el.scrollHeight, 200)}px`;
        }}
        onKeyDown={(e) => {
          // Enter while converting kana is the IME's, not ours.
          if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing && e.keyCode !== 229) {
            e.preventDefault();
            void send();
          }
        }}
      />
      <div className="chat-compose-foot">
        <span className="chat-hint">{t.chat.sendHint}</span>
        <label className="chat-model" title={t.chat.modelTitle}>
          <span className="chat-model-badge">
            {model == null
              ? t.chat.auto
              : (MODELS.find((m) => m.value === model)?.label ?? modelName(model))}
          </span>
          <select
            value={model ?? ""}
            onChange={(e) => void chooseModel(e.target.value || null)}
            aria-label={t.chat.model}
          >
            {MODELS.map((m) => (
              <option key={m.value ?? ""} value={m.value ?? ""}>
                {m.label ?? t.chat.auto}
              </option>
            ))}
          </select>
        </label>
        <label
          className={`chat-model${effortOn ? "" : " is-off"}`}
          title={
            effortOn ? t.chat.effortTitle : t.chat.effortUnavailable
          }
        >
          <span className="chat-model-badge">
            <span className="chat-model-key">effort</span>
            {effortOn ? (effort ? EFFORT_LABEL[effort] : t.chat.auto) : "—"}
          </span>
          <select
            value={effort ?? ""}
            disabled={!effortOn}
            onChange={(e) => void chooseEffort(EFFORTS.find((l) => l === e.target.value) ?? null)}
            aria-label="effort"
          >
            <option value="">{t.chat.auto}</option>
            {EFFORTS.map((l) => (
              <option key={l} value={l}>
                {EFFORT_LABEL[l]}
              </option>
            ))}
          </select>
        </label>
        {busy ? (
          <button
            className="chat-btn"
            onClick={() => activeId != null && void stopChat(activeId)}
          >
            {t.chat.stop}
          </button>
        ) : (
          <button
            className="chat-btn is-primary"
            disabled={(!draft.trim() && attachments.length === 0) || !!status?.blocked}
            onClick={() => void send()}
          >
            {t.chat.send}
          </button>
        )}
      </div>
    </div>
  );

  if (home) {
    if (!homeSlot) return null;
    return createPortal(
      <div className="home-compose">
        {status?.blocked ? (
          <div className="home-blocked">
            <p className="home-blocked-title">{t.chat.unavailable}</p>
            <p className="home-blocked-desc">{status.blocked}</p>
            <button className="chat-btn" onClick={recheck}>
              {t.chat.checkAgain}
            </button>
          </div>
        ) : (
          <>
            {composer("home")}
            <div className="home-starters">
              {t.home.starters.map((text) => (
                <button key={text} className="home-starter" onClick={() => void send(text, true)}>
                  {text}
                </button>
              ))}
              <button className="home-starter" onClick={() => void library.openFiles()}>
                {t.home.openMidi}
              </button>
            </div>
          </>
        )}
      </div>,
      homeSlot
    );
  }

  if (!open) return toggle;

  const shown = activeId != null && live[activeId] ? [...messages, live[activeId]] : messages;
  // The last answer failed on Claude Code's login: offer to sign in, then to
  // ask the same thing again.
  const lastShown = shown[shown.length - 1];
  const loginFailed = lastShown?.role === "assistant" && !!lastShown.error && isLoginError(lastShown.error);
  const lastQuestion = [...shown]
    .reverse()
    .find((m) => m.role === "user")
    ?.parts.map((p) => (p.type === "text" ? p.text : ""))
    .join("");

  return (
    <>
      {toggle}
      <aside className="chat" style={{ width }}>
        <div className="chat-resize" onPointerDown={startResize} />

        <header className="chat-head" data-tauri-drag-region>
          <div className="chat-head-text" data-tauri-drag-region>
            <span className="chat-head-title" data-tauri-drag-region>
              {historyOpen ? t.chat.history : "Claude"}
            </span>
            {!historyOpen && active && (
              <span className="chat-head-sub" data-tauri-drag-region>
                {active.title}
              </span>
            )}
          </div>
          <button
            className={`chat-icon${historyOpen ? " on" : ""}`}
            title={t.chat.history}
            aria-label={t.chat.history}
            aria-pressed={historyOpen}
            onClick={() => {
              setHistoryOpen((v) => !v);
              setConfirmDelete(null);
            }}
          >
            <HistoryIcon />
          </button>
          <button
            className="chat-icon"
            title={t.chat.newChat}
            aria-label={t.chat.newChat}
            onClick={() => {
              setActiveId(null);
              setHistoryOpen(false);
              setError(null);
              inputRef.current?.focus();
            }}
          >
            <PlusIcon />
          </button>
        </header>

        {historyOpen ? (
          <div className="chat-history">
            {convs.length === 0 && <p className="chat-hint chat-history-empty">{t.chat.noHistory}</p>}
            {convs.map((c) => (
              <div
                key={c.id}
                className={`chat-history-item${c.id === activeId ? " is-active" : ""}`}
                onClick={() => {
                  setActiveId(c.id);
                  setHistoryOpen(false);
                }}
              >
                <div className="chat-history-main">
                  <div className="chat-history-title">{c.title}</div>
                  <div className="chat-history-meta">
                    {running.has(c.id) ? t.chat.replying : relTime(c.updatedAt, t)}
                  </div>
                </div>
                {confirmDelete === c.id ? (
                  <div className="chat-history-confirm" onClick={(e) => e.stopPropagation()}>
                    <button
                      className="chat-link is-danger"
                      onClick={async () => {
                        await deleteConversation(c.id);
                        if (c.id === activeId) setActiveId(null);
                        setConfirmDelete(null);
                        void reloadConvs();
                      }}
                    >
                      {t.chat.delete}
                    </button>
                    <button className="chat-link" onClick={() => setConfirmDelete(null)}>
                      {t.chat.cancel}
                    </button>
                  </div>
                ) : (
                  <button
                    className="chat-history-x"
                    title={t.chat.deleteConversation}
                    aria-label={t.chat.deleteConversation}
                    onClick={(e) => {
                      e.stopPropagation();
                      setConfirmDelete(c.id);
                    }}
                  >
                    <TrashIcon />
                  </button>
                )}
              </div>
            ))}
          </div>
        ) : (
          <div
            className="chat-body"
            ref={scrollRef}
            onScroll={(e) => {
              const el = e.currentTarget;
              stick.current = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
            }}
          >
            {status?.blocked ? (
              <div className="chat-empty">
                <p className="chat-empty-title">{t.chat.unavailable}</p>
                <p className="chat-empty-desc">{status.blocked}</p>
                <button className="chat-btn" onClick={recheck}>
                  {t.chat.checkAgain}
                </button>
              </div>
            ) : shown.length === 0 ? (
              <div className="chat-empty">
                <p className="chat-empty-title">{t.chat.emptyTitle}</p>
                <p className="chat-empty-desc">{t.chat.emptyDesc}</p>
                <div className="chat-starters">
                  {t.chat.suggestions.map((s) => (
                    <button
                      key={s.text}
                      className="chat-starter"
                      onClick={() => void send(s.text)}
                      disabled={!song && s.needsSong}
                    >
                      {s.text}
                    </button>
                  ))}
                </div>
              </div>
            ) : (
              <div className="chat-log">
                {shown.map((m, i) => (
                  <MessageView
                    key={m.id > 0 ? m.id : `live-${i}`}
                    message={m}
                    streaming={m.id === -1 && m.role === "assistant"}
                  />
                ))}
                {loginFailed && (
                  <LoginPrompt
                    key={lastShown.id}
                    onRetry={lastQuestion && !busy ? () => void send(lastQuestion, false) : null}
                  />
                )}
              </div>
            )}
          </div>
        )}

        {library.pending.length > 0 && !historyOpen && (
          <div className="chat-pending">
            {library.pending.map((edit) => {
              const previewing =
                library.source?.staged === true && library.source.id === edit.songId;
              return (
                <div key={edit.songId} className="chat-pending-row">
                  <div className="chat-pending-text">
                    <span className="chat-pending-title">{t.chat.pending(edit.changes.length)}</span>
                    <span className="chat-pending-song">{edit.title}</span>
                  </div>
                  <button
                    className="chat-link"
                    disabled={previewing}
                    onClick={() => void library.previewPending(edit.songId)}
                  >
                    {previewing ? t.common.previewing : t.common.preview}
                  </button>
                  <button className="chat-link" onClick={() => void library.discardEdit(edit.songId)}>
                    {t.common.discard}
                  </button>
                  <button
                    className="chat-btn is-primary"
                    onClick={() => void library.applyEdit(edit.songId)}
                  >
                    {t.common.apply}
                  </button>
                </div>
              );
            })}
          </div>
        )}

        {composer("panel")}
      </aside>
    </>
  );
}

/**
 * One attached range in the chat box. Click to choose which of its tracks go
 * with it; × takes it off.
 */
function AttachmentChip({
  selection,
  number,
  otherSong,
  onRemove,
  onToggleTrack,
}: {
  selection: RollSelection;
  number: number;
  otherSong: boolean;
  onRemove: () => void;
  onToggleTrack: (track: number) => void;
}) {
  const { t } = useI18n();
  const [open, setOpen] = useState(false);
  const choosable = selection.tracks.length > 1;
  return (
    <div className="chat-chip" onMouseLeave={() => setOpen(false)}>
      <button
        className="chat-chip-main"
        onClick={() => choosable && setOpen((v) => !v)}
        title={`${otherSong ? `${t.chat.rangeIn(selection.songTitle)}\n` : ""}${selectionContext(selection, number)}`}
      >
        <span className="chat-chip-n">{number}</span>
        {otherSong && <span className="chat-chip-song">{selection.songTitle}</span>}
        {selectionLabel(selection)}
        {choosable && <span className="chat-chip-caret">▾</span>}
      </button>
      <button className="chat-chip-x" onClick={onRemove} aria-label={t.chat.removeRange}>
        ×
      </button>
      {open && (
        <div className="chat-chip-menu">
          {selection.tracks.map((t) => (
            <label key={t.index}>
              <input
                type="checkbox"
                checked={!selection.excluded.includes(t.index)}
                onChange={() => onToggleTrack(t.index)}
              />
              {t.index}: {t.name}
            </label>
          ))}
        </div>
      )}
    </div>
  );
}

/**
 * Under an answer that failed because Claude Code's login ran out: sign in
 * again from here (the browser opens), then ask the question once more.
 */
function LoginPrompt({ onRetry }: { onRetry: (() => void) | null }) {
  const { t } = useI18n();
  const [state, setState] = useState<"idle" | "waiting" | "done">("idle");
  const [url, setUrl] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (state !== "waiting") return;
    let off: (() => void) | null = null;
    let cancelled = false;
    void import("@tauri-apps/api/event").then(({ listen }) =>
      listen<string>("chat://login-url", (e) => setUrl(e.payload)).then((un) => {
        if (cancelled) un();
        else off = un;
      })
    );
    return () => {
      cancelled = true;
      off?.();
    };
  }, [state]);

  const login = async () => {
    setError(null);
    setUrl(null);
    setState("waiting");
    try {
      setState((await loginClaude()) ? "done" : "idle");
    } catch (e) {
      setError(String(e));
      setState("idle");
    }
  };

  const openLink = async (link: string) => {
    const { openUrl } = await import("@tauri-apps/plugin-opener");
    await openUrl(link);
  };

  return (
    <div className="chat-login">
      {state === "done" ? (
        <>
          <p className="chat-login-text">{t.chat.loginDone}</p>
          {onRetry && (
            <button className="chat-btn is-primary" onClick={onRetry}>
              {t.chat.retry}
            </button>
          )}
        </>
      ) : state === "waiting" ? (
        <>
          <p className="chat-login-text">{t.chat.loginWaiting}</p>
          <div className="chat-login-actions">
            {url && (
              <button className="chat-link" onClick={() => void openLink(url)}>
                {t.chat.loginOpenAgain}
              </button>
            )}
            <button className="chat-link" onClick={() => void cancelLogin()}>
              {t.chat.cancel}
            </button>
          </div>
        </>
      ) : (
        <>
          <p className="chat-login-text">{t.chat.loginNeeded}</p>
          <button className="chat-btn is-primary" onClick={() => void login()}>
            {t.chat.login}
          </button>
          {error && <p className="chat-login-error">{error}</p>}
        </>
      )}
    </div>
  );
}

/** "Just now", "5 min ago", "Yesterday" — for the history list. */
function relTime(unix: number, t: Messages): string {
  const mins = Math.floor((Date.now() / 1000 - unix) / 60);
  if (mins < 1) return t.common.justNow;
  if (mins < 60) return t.common.minutesAgo(mins);
  if (mins < 60 * 24) return t.common.hoursAgo(Math.floor(mins / 60));
  if (mins < 60 * 48) return t.common.yesterday;
  return t.common.date(new Date(unix * 1000));
}

type ToolPart = Extract<Part, { type: "tool" }>;

/** Consecutive tool calls read as one log, not as a pile of chips. */
function group(parts: Part[]): (Part | ToolPart[])[] {
  const out: (Part | ToolPart[])[] = [];
  for (const p of parts) {
    const last = out[out.length - 1];
    if (p.type === "tool" && Array.isArray(last)) last.push(p);
    else out.push(p.type === "tool" ? [p] : p);
  }
  return out;
}

function MessageView({ message, streaming }: { message: Message; streaming: boolean }) {
  const { t } = useI18n();
  if (message.role === "user") {
    const text = message.parts.map((p) => (p.type === "text" ? p.text : "")).join("");
    const ranges = message.parts.filter(
      (p): p is Extract<Part, { type: "selection" }> => p.type === "selection"
    );
    return (
      <div className="chat-q">
        {ranges.length > 0 && (
          <div className="chat-q-ranges">
            {ranges.map((r, i) => (
              <span key={i} className="chat-chip is-static" title={r.context}>
                <span className="chat-chip-n">{i + 1}</span>
                {r.label}
              </span>
            ))}
          </div>
        )}
        {text}
      </div>
    );
  }
  const running = [...message.parts]
    .reverse()
    .find((p): p is ToolPart => p.type === "tool" && p.status === "running");
  const last = message.parts[message.parts.length - 1];
  const working = running
    ? `${t.chat.tools[running.name] ?? running.name}…`
    : last?.type === "text"
      ? t.chat.writing
      : t.chat.thinking;
  const meta = [
    message.model ? modelName(message.model) : null,
    message.effort ? `effort ${EFFORT_LABEL[message.effort]}` : null,
    message.durationMs != null ? t.chat.seconds(Math.round(message.durationMs / 1000)) : null,
  ]
    .filter(Boolean)
    .join(" · ");
  return (
    <div className="chat-turn">
      {group(message.parts).map((g, i) =>
        Array.isArray(g) ? (
          <ToolLog key={i} tools={g} />
        ) : g.type === "text" ? (
          <div key={i} className="chat-a">
            <ChatMarkdown text={g.text} />
          </div>
        ) : null
      )}
      {streaming && (
        <div className="chat-working">
          <span className="chat-dots" aria-hidden>
            <i />
            <i />
            <i />
          </span>
          {working}
        </div>
      )}
      {message.error && <div className="chat-a is-error">{message.error}</div>}
      {message.stopped && <div className="chat-meta">{t.chat.stopped}</div>}
      {!streaming && meta && <div className="chat-meta">{meta}</div>}
    </div>
  );
}

/** "claude-opus-5-5" → "Opus 5.5". */
function modelName(id: string): string {
  const m = /claude-([a-z]+)-(\d+)(?:-(\d+))?/.exec(id);
  if (!m) return id;
  const family = m[1][0].toUpperCase() + m[1].slice(1);
  return m[3] && m[3].length < 3 ? `${family} ${m[2]}.${m[3]}` : `${family} ${m[2]}`;
}

function ToolLog({ tools }: { tools: ToolPart[] }) {
  return (
    <div className="chat-tools">
      {tools.map((t) => (
        <ToolRow key={t.id} part={t} />
      ))}
    </div>
  );
}

function ToolRow({ part }: { part: ToolPart }) {
  const { t } = useI18n();
  const [open, setOpen] = useState(false);
  const summary = summarize(part.input, t.chat);
  return (
    <div className={`chat-tool is-${part.status}${open ? " is-open" : ""}`}>
      <button className="chat-tool-row" onClick={() => setOpen((v) => !v)} disabled={!part.result}>
        <span className="chat-tool-mark" aria-label={t.chat.status[part.status]}>
          {part.status === "ok" ? <CheckIcon /> : part.status === "error" ? "!" : null}
        </span>
        <span className="chat-tool-name">{t.chat.tools[part.name] ?? part.name}</span>
        {summary && <span className="chat-tool-sum">{summary}</span>}
      </button>
      {open && part.result && <pre className="chat-tool-result">{part.result}</pre>}
    </div>
  );
}

/* ---------- icons (1.6px strokes, like the rest of the app) ---------- */

const stroke = {
  fill: "none",
  stroke: "currentColor",
  strokeWidth: 1.6,
  strokeLinecap: "round",
  strokeLinejoin: "round",
} as const;

function HistoryIcon() {
  return (
    <svg width="15" height="15" viewBox="0 0 24 24" {...stroke} aria-hidden>
      <path d="M3.5 12a8.5 8.5 0 1 0 2.6-6.1M3.5 4.5v4h4" />
      <path d="M12 8v4.2l2.8 1.8" />
    </svg>
  );
}

function PlusIcon() {
  return (
    <svg width="15" height="15" viewBox="0 0 24 24" {...stroke} aria-hidden>
      <path d="M12 5.5v13M5.5 12h13" />
    </svg>
  );
}

function TrashIcon() {
  return (
    <svg width="13" height="13" viewBox="0 0 24 24" {...stroke} aria-hidden>
      <path d="M4.5 7h15M9.5 7V4.5h5V7M6.5 7l1 12.5h9l1-12.5" />
    </svg>
  );
}

function CheckIcon() {
  return (
    <svg width="11" height="11" viewBox="0 0 24 24" {...stroke} strokeWidth={2.4} aria-hidden>
      <path d="M5 12.5l4.5 4.5L19 7.5" />
    </svg>
  );
}
