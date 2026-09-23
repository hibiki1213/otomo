import { useEffect, useState } from "react";
import Player from "@/components/Player";
import Sidebar from "@/components/Sidebar";
import type { PlayerState } from "@/lib/midiEngine";
import { libraryAvailable } from "@/lib/songsApi";
import { useSongLibrary } from "@/lib/useSongLibrary";

export default function App() {
  const library = useSongLibrary();
  const [state, setState] = useState<PlayerState>("idle");
  const [dropping, setDropping] = useState(false);
  const { importPaths, openFiles } = library;

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

  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "o") {
        e.preventDefault();
        void openFiles();
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
        <Player
          source={library.source}
          onLoaded={library.onLoaded}
          onStateChange={setState}
          onOpenFiles={openFiles}
        />
      </main>
      {dropping && (
        <div className="drop-overlay">
          <p>MIDI ファイルを追加</p>
        </div>
      )}
    </div>
  );
}
