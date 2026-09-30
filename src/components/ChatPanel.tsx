import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import ChatMarkdown from "@/components/ChatMarkdown";
import {
  EFFORTS,
  chatStatus,
  deleteConversation,
  listConversations,
  listMessages,
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
import { libraryAvailable } from "@/lib/songsApi";
import type { SongLibrary } from "@/lib/useSongLibrary";

/**
 * Aliases, not versions: Claude Code resolves each to the newest model of
 * that family, so a new release is picked up without touching this list.
 */
const MODELS: { value: string | null; label: string }[] = [
  { value: null, label: "自動" },
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

const SUGGESTIONS = [
  "8小節のローファイなピアノ曲を作って",
  "この曲に合うベースラインを足して",
  "この曲のコード進行を教えて",
];

/** What each Otomo tool is doing, in words. */
const TOOL_LABEL: Record<string, string> = {
  WebSearch: "ウェブで調べる",
  WebFetch: "ページを読む",
  download: "ダウンロード",
  list_songs: "曲の一覧を見る",
  get_song: "曲を調べる",
  read_bars: "小節を読む",
  harmony: "和声を読む",
  play: "再生",
  stop: "停止",
  list_playback_instruments: "音源を調べる",
  set_playback_instrument: "音源を切り替え",
  create_song: "曲を作る",
  write_bars: "小節を書く",
  write_score: "譜面を書く",
  add_track: "トラックを追加",
  add_tracks: "トラックを追加",
  set_track: "トラックを変更",
  remove_track: "トラックを削除",
  copy_bars: "小節をコピー",
  insert_bars: "小節を挿入",
  delete_bars: "小節を削除",
  set_tempo: "テンポを変更",
  set_time_signature: "拍子を変更",
  set_key: "調を変更",
  set_dynamics: "強弱を変更",
  set_pickup: "弱起を設定",
  discard_changes: "編集を破棄",
};

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

/** "小節 5–8 · トラック 2" — the part of a call's input worth a glance. */
function summarize(input: Record<string, unknown>): string {
  const v = (k: string) => input[k] as string | number | undefined;
  const out: string[] = [];
  if (v("query")) out.push(`「${v("query")}」`);
  if (typeof input.url === "string") out.push(shortUrl(input.url));
  if (v("pages")) out.push(`${v("pages")}ページ`);
  if (v("title")) out.push(`「${v("title")}」`);
  if (v("from_track") != null && v("to_track") != null)
    out.push(`トラック ${v("from_track")}→${v("to_track")}`);
  else if (v("track") != null) out.push(`トラック ${v("track")}`);
  if (Array.isArray(input.tracks)) out.push(`トラック ${input.tracks.join(",")} のみ`);
  if (v("from_bar") != null)
    out.push(
      v("to_bar") != null && v("to_bar") !== v("from_bar")
        ? `小節 ${v("from_bar")}–${v("to_bar")}`
        : `小節 ${v("from_bar")}`
    );
  else if (v("at_bar") != null) out.push(`小節 ${v("at_bar")}`);
  const bpm = v("to_bpm") ?? v("tempo");
  if (bpm != null) out.push(`${bpm} BPM`);
  if (v("key")) out.push(String(v("key")));
  const sig = v("time_signature") ?? v("signature");
  if (sig) out.push(String(sig));
  if (v("instrument")) out.push(String(v("instrument")));
  if (Array.isArray(input.instruments)) out.push(input.instruments.join("・"));
  if (input.loop) out.push("繰り返し");
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
 * banner's 適用 — or the copy of it at the bottom of this panel.
 */
export default function ChatPanel({
  library,
  open,
  onToggle,
  width,
  onResize,
}: {
  library: SongLibrary;
  open: boolean;
  onToggle: () => void;
  width: number;
  onResize: (width: number) => void;
}) {
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

  useEffect(() => {
    if (!libraryAvailable()) return;
    void chatStatus().then(setStatus);
    void reloadConvs();
    void runningConversations().then((ids) => {
      setRunning(new Set(ids));
      setLive(Object.fromEntries(ids.map((id) => [id, emptyAssistant()])));
    });
  }, [reloadConvs]);

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

  const active = convs.find((c) => c.id === activeId) ?? null;
  const model = active ? active.model : newModel;
  const effort = active ? active.effort : newEffort;
  const effortOn = takesEffort(model);
  const busy = activeId != null && running.has(activeId);
  const song = library.current;

  const send = async (text = draft) => {
    const body = text.trim();
    if (!body || busy) return;
    setError(null);
    setDraft("");
    stick.current = true;
    const convId = activeId;
    const optimistic: Message = {
      ...emptyAssistant(),
      role: "user",
      parts: [{ type: "text", text: body }],
    };
    setMessages((prev) => [...prev, optimistic]);
    try {
      const conv = await sendChat({
        convId,
        text: body,
        song: song ? { id: song.id, title: song.title } : null,
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

  if (!open) {
    return (
      <aside className="chat-rail">
        <div className="chat-rail-drag" data-tauri-drag-region />
        <button
          className={`chat-rail-btn${running.size > 0 ? " busy" : ""}`}
          onClick={onToggle}
          title="Claude を開く（⌘L）"
          aria-label="Claude を開く"
        >
          <BubbleIcon />
          {running.size > 0 && <span className="chat-rail-dot" />}
        </button>
      </aside>
    );
  }

  const shown = activeId != null && live[activeId] ? [...messages, live[activeId]] : messages;

  return (
    <aside className="chat" style={{ width }}>
      <div className="chat-resize" onPointerDown={startResize} />

      <header className="chat-head" data-tauri-drag-region>
        <div className="chat-head-text" data-tauri-drag-region>
          <span className="chat-head-title" data-tauri-drag-region>
            {historyOpen ? "履歴" : "Claude"}
          </span>
          {!historyOpen && active && (
            <span className="chat-head-sub" data-tauri-drag-region>
              {active.title}
            </span>
          )}
        </div>
        <button
          className={`chat-icon${historyOpen ? " on" : ""}`}
          title="履歴"
          aria-label="履歴"
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
          title="新しい会話"
          aria-label="新しい会話"
          onClick={() => {
            setActiveId(null);
            setHistoryOpen(false);
            setError(null);
            inputRef.current?.focus();
          }}
        >
          <PlusIcon />
        </button>
        <button className="chat-icon" title="閉じる（⌘L）" aria-label="閉じる" onClick={onToggle}>
          <PanelIcon />
        </button>
      </header>

      {historyOpen ? (
        <div className="chat-history">
          {convs.length === 0 && <p className="chat-hint chat-history-empty">まだ会話はありません</p>}
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
                  {running.has(c.id) ? "返事を書いています" : relTime(c.updatedAt)}
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
                    消す
                  </button>
                  <button className="chat-link" onClick={() => setConfirmDelete(null)}>
                    やめる
                  </button>
                </div>
              ) : (
                <button
                  className="chat-history-x"
                  title="この会話を消す"
                  aria-label="この会話を消す"
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
              <p className="chat-empty-title">使えません</p>
              <p className="chat-empty-desc">{status.blocked}</p>
            </div>
          ) : shown.length === 0 ? (
            <div className="chat-empty">
              <p className="chat-empty-title">曲について話す</p>
              <p className="chat-empty-desc">
                作りたい曲や、直したいところを書いてください。編集は「適用」を押すまで保存されません。
              </p>
              <div className="chat-starters">
                {SUGGESTIONS.map((s) => (
                  <button
                    key={s}
                    className="chat-starter"
                    onClick={() => void send(s)}
                    disabled={!song && s.includes("この曲")}
                  >
                    {s}
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
                  <span className="chat-pending-title">未適用の編集 · {edit.changes.length}</span>
                  <span className="chat-pending-song">{edit.title}</span>
                </div>
                <button
                  className="chat-link"
                  disabled={previewing}
                  onClick={() => void library.previewPending(edit.songId)}
                >
                  {previewing ? "試聴中" : "試聴"}
                </button>
                <button className="chat-link" onClick={() => void library.discardEdit(edit.songId)}>
                  破棄
                </button>
                <button
                  className="chat-btn is-primary"
                  onClick={() => void library.applyEdit(edit.songId)}
                >
                  適用
                </button>
              </div>
            );
          })}
        </div>
      )}

      <div className="chat-compose">
        {error && (
          <p className="chat-error" onClick={() => setError(null)}>
            {error}
          </p>
        )}
        <textarea
          ref={inputRef}
          className="chat-input"
          rows={2}
          value={draft}
          disabled={!!status?.blocked}
          placeholder={song ? `「${song.title}」について頼む` : "作りたい曲を書く"}
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
          <span className="chat-hint">↵ 送信　⇧↵ 改行</span>
          <label className="chat-model" title="この会話で使うモデル（常にその系統の最新版）">
            <span className="chat-model-badge">
              {MODELS.find((m) => m.value === model)?.label ?? (model ? modelName(model) : "")}
            </span>
            <select
              value={model ?? ""}
              onChange={(e) => void chooseModel(e.target.value || null)}
              aria-label="モデル"
            >
              {MODELS.map((m) => (
                <option key={m.label} value={m.value ?? ""}>
                  {m.label}
                </option>
              ))}
            </select>
          </label>
          <label
            className={`chat-model${effortOn ? "" : " is-off"}`}
            title={
              effortOn
                ? "考える深さ（effort）。上げるほど丁寧になり、時間と使用量が増える"
                : "Haiku では effort を選べません"
            }
          >
            <span className="chat-model-badge">
              <span className="chat-model-key">effort</span>
              {effortOn ? (effort ? EFFORT_LABEL[effort] : "自動") : "—"}
            </span>
            <select
              value={effort ?? ""}
              disabled={!effortOn}
              onChange={(e) => void chooseEffort(EFFORTS.find((l) => l === e.target.value) ?? null)}
              aria-label="effort"
            >
              <option value="">自動</option>
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
              止める
            </button>
          ) : (
            <button
              className="chat-btn is-primary"
              disabled={!draft.trim() || !!status?.blocked}
              onClick={() => void send()}
            >
              送信
            </button>
          )}
        </div>
      </div>
    </aside>
  );
}

/** "たった今", "5分前", "昨日" — for the history list. */
function relTime(unix: number): string {
  const mins = Math.floor((Date.now() / 1000 - unix) / 60);
  if (mins < 1) return "たった今";
  if (mins < 60) return `${mins}分前`;
  if (mins < 60 * 24) return `${Math.floor(mins / 60)}時間前`;
  if (mins < 60 * 48) return "昨日";
  const d = new Date(unix * 1000);
  return `${d.getMonth() + 1}月${d.getDate()}日`;
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
  if (message.role === "user") {
    const text = message.parts.map((p) => (p.type === "text" ? p.text : "")).join("");
    return <div className="chat-q">{text}</div>;
  }
  const running = [...message.parts]
    .reverse()
    .find((p): p is ToolPart => p.type === "tool" && p.status === "running");
  const last = message.parts[message.parts.length - 1];
  const working = running
    ? `${TOOL_LABEL[running.name] ?? running.name}…`
    : last?.type === "text"
      ? "書いています"
      : "考えています";
  const meta = [
    message.model ? modelName(message.model) : null,
    message.effort ? `effort ${EFFORT_LABEL[message.effort]}` : null,
    message.durationMs != null ? `${Math.round(message.durationMs / 1000)}秒` : null,
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
      {message.stopped && <div className="chat-meta">止めました</div>}
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
  const [open, setOpen] = useState(false);
  const summary = summarize(part.input);
  return (
    <div className={`chat-tool is-${part.status}${open ? " is-open" : ""}`}>
      <button className="chat-tool-row" onClick={() => setOpen((v) => !v)} disabled={!part.result}>
        <span className="chat-tool-mark" aria-label={STATUS_LABEL[part.status]}>
          {part.status === "ok" ? <CheckIcon /> : part.status === "error" ? "!" : null}
        </span>
        <span className="chat-tool-name">{TOOL_LABEL[part.name] ?? part.name}</span>
        {summary && <span className="chat-tool-sum">{summary}</span>}
      </button>
      {open && part.result && <pre className="chat-tool-result">{part.result}</pre>}
    </div>
  );
}

const STATUS_LABEL = { running: "実行中", ok: "完了", error: "失敗" } as const;

/* ---------- icons (1.6px strokes, like the rest of the app) ---------- */

const stroke = {
  fill: "none",
  stroke: "currentColor",
  strokeWidth: 1.6,
  strokeLinecap: "round",
  strokeLinejoin: "round",
} as const;

function BubbleIcon() {
  return (
    <svg width="16" height="16" viewBox="0 0 24 24" {...stroke} aria-hidden>
      <path d="M5 5.5h14a1.5 1.5 0 0 1 1.5 1.5v8.5A1.5 1.5 0 0 1 19 17h-7l-4.5 3.5V17H5a1.5 1.5 0 0 1-1.5-1.5V7A1.5 1.5 0 0 1 5 5.5z" />
    </svg>
  );
}

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

function PanelIcon() {
  return (
    <svg width="15" height="15" viewBox="0 0 24 24" {...stroke} aria-hidden>
      <rect x="3.5" y="4.5" width="17" height="15" rx="2" />
      <path d="M14.5 4.5v15" />
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
