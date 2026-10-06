import { useCallback, useEffect, useState } from "react";
import ChatPanel from "@/components/ChatPanel";
import PendingBar from "@/components/PendingBar";
import Player from "@/components/Player";
import Sidebar from "@/components/Sidebar";
import type { PlayerState } from "@/lib/midiEngine";
import type { RollSelection } from "@/lib/rollSelection";
import { libraryAvailable } from "@/lib/songsApi";
import { useSongLibrary } from "@/lib/useSongLibrary";

export default function App() {
  const library = useSongLibrary();
  const [state, setState] = useState<PlayerState>("idle");
  const [dropping, setDropping] = useState(false);
  // Bumped when MCP asks for silence; the player watches it.
  const [stopRequest, setStopRequest] = useState(0);
  const { importPaths, openFiles } = library;
  const [chatOpen, setChatOpen] = useState(() => readPref("otomo.chat.open", "1") === "1");
  const [chatWidth, setChatWidth] = useState(() => Number(readPref("otomo.chat.width", "360")) || 360);

  useEffect(() => writePref("otomo.chat.open", chatOpen ? "1" : "0"), [chatOpen]);
  // Piano-roll ranges waiting in the chat box.
  const [attachments, setAttachments] = useState<RollSelection[]>([]);
  const attach = useCallback((sel: Omit<RollSelection, "id" | "excluded">) => {
    const id = `${Date.now()}-${Math.random().toString(36).slice(2)}`;
    setAttachments((prev) => [...prev, { ...sel, id, excluded: [] }]);
    setChatOpen(true);
  }, []);
  useEffect(() => writePref("otomo.chat.width", String(chatWidth)), [chatWidth]);

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

  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "o") {
        e.preventDefault();
        void openFiles();
      }
      // ⌘L, as in Cursor: show or hide Claude.
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "l") {
        e.preventDefault();
        setChatOpen((v) => !v);
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [openFiles]);

  return (
    <div className={`shell${dropping ? " dropping" : ""}`}>
      <Sidebar library={library} playing={state === "playing"} />
      <main className="content">
        {/* With a song open the toolbar is the drag region; without one,
            this strip keeps the window movable from the content side. */}
        {!library.source && <div className="content-drag" data-tauri-drag-region />}
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
            previewing={
              library.source?.staged === true && library.source.id === edit.songId
            }
          />
        ))}
        <Player
          source={library.source}
          onLoaded={library.onLoaded}
          onStateChange={setState}
          stopRequest={stopRequest}
          onOpenFiles={openFiles}
          onAttachSelection={libraryAvailable() ? attach : undefined}
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
        />
      )}
      {dropping && (
        <div className="drop-overlay">
          <p>MIDI ファイルを追加</p>
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
