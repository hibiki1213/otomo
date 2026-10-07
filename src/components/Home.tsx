import { useI18n } from "@/lib/i18n";
import type { SongRecord } from "@/lib/songsApi";

/**
 * What the middle of the window shows with no song open. At launch it is the
 * way in: a question and the chat's message box (which the chat panel draws
 * into `slotRef`). Once something has been asked the chat is back on the
 * right, and this is just where the song will appear.
 */
export default function Home({
  mode,
  slotRef,
  songs,
  onOpenSong,
  onOpenFiles,
}: {
  /** "ask": the launch screen; "working": Claude is on it; "idle": nothing open. */
  mode: "ask" | "working" | "idle";
  slotRef: (el: HTMLDivElement | null) => void;
  songs: SongRecord[];
  onOpenSong: (song: SongRecord) => void;
  onOpenFiles: () => void;
}) {
  const { t } = useI18n();
  const recent = [...songs]
    .sort((a, b) => (b.lastPlayed ?? b.addedAt) - (a.lastPlayed ?? a.addedAt))
    .slice(0, 5);
  return (
    <div className={`home is-${mode}`}>
      <div className="home-main">
        {mode === "ask" ? (
          <>
            <h1 className="home-title">{t.home.title}</h1>
            <div ref={slotRef} className="home-slot" />
          </>
        ) : (
          <div className="home-status">
            {mode === "working" && <div className="spinner" />}
            <p className="home-status-title">{mode === "working" ? t.home.working : t.home.idle}</p>
            <p className="home-status-sub">{mode === "working" ? t.home.workingSub : t.home.idleSub}</p>
            {mode === "idle" && (
              <button className="home-starter" onClick={onOpenFiles}>
                {t.home.openMidi}
              </button>
            )}
          </div>
        )}

        {recent.length > 0 && (
          <section className="home-recent">
            <h2>{t.home.recent}</h2>
            <ul>
              {recent.map((song) => (
                <li key={song.id}>
                  <button onClick={() => onOpenSong(song)} title={song.title}>
                    {song.title}
                  </button>
                </li>
              ))}
            </ul>
          </section>
        )}
      </div>
      <p className="home-drop">{t.home.dropHint}</p>
    </div>
  );
}
