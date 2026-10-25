import { useCallback, useEffect, useRef, useState } from "react";
import ChatPanel from "@/components/ChatPanel";
import ExportDialog, { type ExportTarget } from "@/components/ExportDialog";
import Home from "@/components/Home";
import PanelToggle from "@/components/PanelToggle";
import PendingBar from "@/components/PendingBar";
import Player from "@/components/Player";
import SettingsDialog from "@/components/SettingsDialog";
import Sidebar from "@/components/Sidebar";
import UpdateDialog from "@/components/UpdateDialog";
import type { PlayerState } from "@/lib/midiEngine";
import type { RollSelection } from "@/lib/rollSelection";
import { useI18n } from "@/lib/i18n";
import { libraryAvailable, type PasteRequest, type RollClip } from "@/lib/songsApi";
import { appVersion, checkForUpdates } from "@/lib/updater";
import { useSongLibrary } from "@/lib/useSongLibrary";

export default function App() {
  const { t } = useI18n();
  const library = useSongLibrary();
  const [state, setState] = useState<PlayerState>("idle");
  const [dropping, setDropping] = useState(false);
  // Bumped when MCP asks for silence; the player watches it.
  const [stopRequest, setStopRequest] = useState(0);
  const { importPaths, openFiles } = library;
  const [chatOpen, setChatOpen] = useState(() => readPref("otomo.chat.open", "1") === "1");
  const [chatWidth, setChatWidth] = useState(() => Number(readPref("otomo.chat.width", "360")) || 360);

  useEffect(() => writePref("otomo.chat.open", chatOpen ? "1" : "0"), [chatOpen]);
  const [sidebarOpen, setSidebarOpen] = useState(() => readPref("otomo.sidebar.open", "1") === "1");
  const [sidebarWidth, setSidebarWidth] = useState(() => Number(readPref("otomo.sidebar.width", "248")) || 248);
  useEffect(() => writePref("otomo.sidebar.open", sidebarOpen ? "1" : "0"), [sidebarOpen]);
  useEffect(() => writePref("otomo.sidebar.width", String(sidebarWidth)), [sidebarWidth]);
  // The traffic lights hide in full screen (and don't exist in a browser),
  // so the title bar stops leaving room for them.
  const [trafficLights, setTrafficLights] = useState(libraryAvailable);
  useEffect(() => {
    if (!libraryAvailable()) return;
    let stop: (() => void) | undefined;
    let cancelled = false;
    void (async () => {
      const { getCurrentWindow } = await import("@tauri-apps/api/window");
      const win = getCurrentWindow();
      const check = async () => setTrafficLights(!(await win.isFullscreen()));
      await check();
      // Entering and leaving full screen both resize the window.
      const un = await win.onResized(() => void check());
      if (cancelled) un();
      else stop = un;
    })();
    return () => {
      cancelled = true;
      stop?.();
    };
  }, []);
  const [exporting, setExporting] = useState<ExportTarget | null>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);
  // What ⌘V pastes: a piano-roll range, from this song or any other.
  const [clip, setClip] = useState<RollClip | null>(null);
  // A short note at the bottom, with an action when there is one to take.
  const [toast, setToast] = useState<{ text: string; action?: { label: string; run: () => void } } | null>(null);
  useEffect(() => {
    if (!toast) return;
    const t = window.setTimeout(() => setToast(null), toast.action ? 8000 : 4000);
    return () => window.clearTimeout(t);
  }, [toast]);
  const copy = useCallback(
    (next: RollClip) => {
      setClip(next);
      setToast({ text: t.paste.copied });
    },
    [t]
  );
  const { paste, stepHistory } = library;
  // Undo / redo, from ⌘Z, the sidebar or a toast; the toast says what was
  // stepped over and offers the way back.
  const history = useCallback(
    async (songId: number, back: boolean, seconds?: number) => {
      const done = await stepHistory(songId, back, seconds);
      if (typeof done === "string") {
        setToast({ text: done });
        return;
      }
      const { kind, count } = done.step;
      const what =
        kind === "paste" ? t.history.paste(count) : kind === "claude" ? t.history.claude(count) : t.history.edit;
      setToast({
        text: back ? t.history.undone(what) : t.history.redone(what),
        action: {
          label: back ? t.history.redo : t.history.undo,
          run: () => void history(songId, !back, seconds),
        },
      });
    },
    [stepHistory, t]
  );
  const pasteClip = useCallback(
    async (request: PasteRequest) => {
      const result = await paste(request);
      if (!result) return false;
      // A saved paste can be taken back; one that joined Claude's edits goes with them.
      setToast({
        text: t.paste.pasted(result.notes),
        action: result.staged
          ? undefined
          : { label: t.paste.undo, run: () => void history(request.toId, true, result.atSeconds) },
      });
      return true;
    },
    [paste, history, t]
  );
  const { source } = library;
  // What is on screen is what gets exported, Claude's staged edits included.
  const exportSong = useCallback(
    (song: { id: number; title: string }) =>
      setExporting({ id: song.id, title: song.title, staged: source?.id === song.id && source.staged === true }),
    [source]
  );
  // Piano-roll ranges waiting in the chat box.
  const [attachments, setAttachments] = useState<RollSelection[]>([]);
  const attach = useCallback((sel: Omit<RollSelection, "id" | "excluded">) => {
    const id = `${Date.now()}-${Math.random().toString(36).slice(2)}`;
    setAttachments((prev) => [...prev, { ...sel, id, excluded: [] }]);
    setChatOpen(true);
  }, []);
  useEffect(() => writePref("otomo.chat.width", String(chatWidth)), [chatWidth]);

  // The launch screen: until the first message, with no song open, the chat
  // sits in the middle of the window. Sending moves it to its panel.
  const [asked, setAsked] = useState(false);
  const [homeSlot, setHomeSlot] = useState<HTMLDivElement | null>(null);
  const [claudeBusy, setClaudeBusy] = useState(false);
  const home = libraryAvailable() && !library.source && !asked;
  // Songs that existed when the launch screen sent its message; one that
  // appears after it is what Claude made, and opens when Claude is done.
  const knownSongs = useRef<Set<number> | null>(null);
  const sawBusy = useRef(false);
  const onHomeSend = useCallback(() => {
    knownSongs.current = new Set(library.songs.map((s) => s.id));
    sawBusy.current = false;
    setAsked(true);
    setChatOpen(true);
  }, [library.songs]);
  const { songs, pending, previewPending, play } = library;
  useEffect(() => {
    const known = knownSongs.current;
    if (!known) return;
    if (library.source) {
      knownSongs.current = null;
      return;
    }
    if (claudeBusy) {
      sawBusy.current = true;
      return;
    }
    if (!sawBusy.current) return;
    knownSongs.current = null;
    // An edit waiting on a new song is the music itself; else the new song.
    const staged = pending.find((p) => !known.has(p.songId));
    if (staged) {
      void previewPending(staged.songId);
      return;
    }
    const made = songs.filter((s) => !known.has(s.id)).sort((a, b) => b.addedAt - a.addedAt)[0];
    if (made) void play(made);
  }, [claudeBusy, songs, pending, library.source, previewPending, play]);

  // Files dropped on the window arrive from the OS with real paths, so they
  // can be imported the same way the open panel's are.
  useEffect(() => {
    if (!libraryAvailable()) return;
    let stop: (() => void) | undefined;
    let cancelled = false;
    void (async () => {
      const { getCurrentWebview } = await import("@tauri-apps/api/webview");
      const un = await getCurrentWebview().onDragDropEvent((e) => {
        if (e.payload.type === "over") setDropping(true);
        else if (e.payload.type === "leave") setDropping(false);
        else if (e.payload.type === "drop") {
          setDropping(false);
          void importPaths(e.payload.paths);
        }
      });
      if (cancelled) un();
      else stop = un;
    })();
    return () => {
      cancelled = true;
      stop?.();
    };
  }, [importPaths]);

  // The MCP server asks the running app to play, so the user can hear what
  // Claude is talking about (see src-tauri/src/control.rs).
  const { playById, refreshPending } = library;
  useEffect(() => {
    if (!libraryAvailable()) return;
    let stop: (() => void) | undefined;
    let cancelled = false;
    void (async () => {
      const { listen } = await import("@tauri-apps/api/event");
      const un = await listen<{
        cmd: string;
        songId?: number;
        seconds?: number;
        until?: number | null;
        looping?: boolean;
        solo?: number[] | null;
        pending?: boolean;
      }>(
        "otomo://control",
        (e) => {
          if (e.payload.cmd === "play" && e.payload.songId != null) {
            void playById(e.payload.songId, e.payload.seconds ?? 0, e.payload.pending, {
              stopAt: e.payload.until ?? undefined,
              loop: e.payload.looping,
              solo: e.payload.solo ?? undefined,
            });
          } else if (e.payload.cmd === "stop") {
            setStopRequest((n) => n + 1);
          } else if (e.payload.cmd === "staged") {
            void refreshPending();
          }
        }
      );
      if (cancelled) un();
      else stop = un;
    })();
    return () => {
      cancelled = true;
      stop?.();
    };
  }, [playById, refreshPending]);

  // Updates: a quiet check shortly after launch, which says something only
  // when there is a new version; and the person's own check, from the
  // sidebar or the app menu, which always answers.
  const checkUpdatesNow = useCallback(async () => {
    if (import.meta.env.DEV) {
      setToast({ text: t.updates.devBuild });
      return;
    }
    const result = await checkForUpdates();
    if (result === "latest") setToast({ text: t.updates.latest(await appVersion()) });
    else if (result === "failed") setToast({ text: t.updates.checkFailed });
    // The update dialog takes over from Settings rather than stacking on it.
    else setSettingsOpen(false);
  }, [t]);
  useEffect(() => {
    if (!libraryAvailable() || import.meta.env.DEV) return;
    const timer = window.setTimeout(() => void checkForUpdates(), 10_000);
    return () => window.clearTimeout(timer);
  }, []);
  useEffect(() => {
    if (!libraryAvailable()) return;
    let stop: (() => void) | undefined;
    let cancelled = false;
    void (async () => {
      const { listen } = await import("@tauri-apps/api/event");
      const unCheck = await listen("otomo://check-updates", () => void checkUpdatesNow());
      const unSettings = await listen("otomo://open-settings", () => setSettingsOpen(true));
      const un = () => {
        unCheck();
        unSettings();
      };
      if (cancelled) un();
      else stop = un;
    })();
    return () => {
      cancelled = true;
      stop?.();
    };
  }, [checkUpdatesNow]);

  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "o") {
        e.preventDefault();
        void openFiles();
      }
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "e" && libraryAvailable() && source) {
        e.preventDefault();
        exportSong(source);
      }
      // ⌘L, as in Cursor: show or hide Claude.
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "l") {
        e.preventDefault();
        setChatOpen((v) => !v);
      }
      // ⌘, opens Settings (in the app the menu item takes it first).
      if (e.metaKey && e.key === ",") {
        e.preventDefault();
        setSettingsOpen(true);
      }
      // ⌃⌘S, the Mac's own "Show Sidebar".
      if (e.metaKey && e.ctrlKey && e.code === "KeyS") {
        e.preventDefault();
        setSidebarOpen((v) => !v);
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [openFiles, exportSong, source]);

  return (
    <div
      className={`shell${dropping ? " dropping" : ""}${sidebarOpen ? "" : " sidebar-closed"}${
        chatOpen && libraryAvailable() && !home ? "" : " chat-closed"
      }${trafficLights ? "" : " no-traffic-lights"}`}
    >
      {sidebarOpen && (
        <Sidebar
          library={library}
          playing={state === "playing"}
          width={sidebarWidth}
          onResize={setSidebarWidth}
          onExport={exportSong}
          onHistory={(id, back) => void history(id, back)}
          onOpenSettings={() => setSettingsOpen(true)}
        />
      )}
      <PanelToggle
        side="left"
        open={sidebarOpen}
        onToggle={() => setSidebarOpen((v) => !v)}
        title={sidebarOpen ? t.sidebar.hide : t.sidebar.show}
        label={t.sidebar.toggleLabel}
      />
      <main className="content">
        {/* With a song open the toolbar is the drag region; without one,
            this strip keeps the window movable from the content side. */}
        {!library.source && <div className="content-drag" data-tauri-drag-region />}
        <Player
          source={library.source}
          empty={
            libraryAvailable() ? (
              <Home
                mode={home ? "ask" : claudeBusy ? "working" : "idle"}
                slotRef={setHomeSlot}
                songs={library.songs}
                onOpenSong={(song) => void library.play(song)}
                onOpenFiles={() => void openFiles()}
              />
            ) : undefined
          }
          notices={
            <>
              {library.error && (
                <div className="error" onClick={library.dismissError}>
                  {library.error}
                </div>
              )}
              {library.pending.map((edit) => (
                <PendingBar
                  key={edit.songId}
                  edit={edit}
                  library={library}
                  previewing={library.source?.staged === true && library.source.id === edit.songId}
                />
              ))}
            </>
          }
          onLoaded={library.onLoaded}
          onStateChange={setState}
          stopRequest={stopRequest}
          onOpenFiles={openFiles}
          onAttachSelection={libraryAvailable() ? attach : undefined}
          onExport={libraryAvailable() && source ? () => exportSong(source) : undefined}
          clip={clip}
          onCopy={libraryAvailable() ? copy : undefined}
          onPaste={libraryAvailable() ? pasteClip : undefined}
          onHistory={libraryAvailable() ? (id, back, seconds) => void history(id, back, seconds) : undefined}
          pasteStaged={library.pending.some((p) => p.songId === source?.id)}
        />
      </main>
      {libraryAvailable() && (
        <ChatPanel
          library={library}
          open={chatOpen}
          onToggle={() => setChatOpen((v) => !v)}
          width={chatWidth}
          onResize={setChatWidth}
          attachments={attachments}
          onAttachmentsChange={setAttachments}
          home={home}
          homeSlot={homeSlot}
          onHomeSend={onHomeSend}
          onRunningChange={setClaudeBusy}
        />
      )}
      {exporting && <ExportDialog target={exporting} onClose={() => setExporting(null)} />}
      {settingsOpen && (
        <SettingsDialog
          onClose={() => setSettingsOpen(false)}
          onCheckUpdates={libraryAvailable() ? () => void checkUpdatesNow() : undefined}
        />
      )}
      <UpdateDialog busy={state === "playing" || claudeBusy || library.pending.length > 0} />
      {toast && (
        <div className="toast" role="status">
          {toast.text}
          {toast.action && (
            <button
              className="link-btn"
              onClick={() => {
                const run = toast.action!.run;
                setToast(null);
                run();
              }}
            >
              {toast.action.label}
            </button>
          )}
        </div>
      )}
      {dropping && (
        <div className="drop-overlay">
          <p>{t.app.dropOverlay}</p>
        </div>
      )}
    </div>
  );
}

/** Layout preferences; a blocked storage only costs remembering them. */
function readPref(key: string, fallback: string): string {
  try {
    return localStorage.getItem(key) ?? fallback;
  } catch {
    return fallback;
  }
}

function writePref(key: string, value: string) {
  try {
    localStorage.setItem(key, value);
  } catch {
    /* ignore */
  }
}
