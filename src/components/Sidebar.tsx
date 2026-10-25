import { useEffect, useRef, useState } from "react";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import ClaudeConnection from "@/components/ClaudeConnection";
import { useI18n } from "@/lib/i18n";
import type { Messages } from "@/locales/en";
import type { SongRecord } from "@/lib/songsApi";
import type { SongLibrary } from "@/lib/useSongLibrary";
import { useUpdater } from "@/lib/updater";

/** "3 min ago", "Yesterday", "Mar 4" — short enough for a sidebar row. */
function whenLabel(unix: number | null, t: Messages): string {
  if (!unix) return t.sidebar.neverPlayed;
  const d = new Date(unix * 1000);
  const mins = Math.floor((Date.now() - d.getTime()) / 60000);
  if (mins < 1) return t.common.justNow;
  if (mins < 60) return t.common.minutesAgo(mins);
  if (mins < 60 * 24) return t.common.hoursAgo(Math.floor(mins / 60));
  if (mins < 60 * 48) return t.common.yesterday;
  if (mins < 60 * 24 * 7) return t.common.daysAgo(Math.floor(mins / 1440));
  return t.common.date(d);
}

function lengthLabel(seconds: number): string {
  if (!seconds) return "";
  const m = Math.floor(seconds / 60);
  const s = Math.round(seconds % 60);
  return `${m}:${s.toString().padStart(2, "0")}`;
}

interface MenuState {
  song: SongRecord;
  x: number;
  y: number;
}

export default function Sidebar({
  library,
  playing,
  width,
  onResize,
  onExport,
  onHistory,
  onOpenSettings,
}: {
  library: SongLibrary;
  /** Whether the current song is sounding, for the row indicator. */
  playing: boolean;
  width: number;
  onResize: (width: number) => void;
  onExport: (song: SongRecord) => void;
  /** Undo (back) or redo a song's last change. */
  onHistory: (songId: number, back: boolean) => void;
  onOpenSettings: () => void;
}) {
  const { t } = useI18n();
  // Installed and waiting for a restart: worth a mark on the way to Settings.
  const updateReady = useUpdater().status.phase === "ready";
  const [menu, setMenu] = useState<MenuState | null>(null);
  const [renaming, setRenaming] = useState<number | null>(null);

  const menuRef = useRef<HTMLDivElement | null>(null);
  useEffect(() => {
    if (!menu) return;
    const close = () => setMenu(null);
    // A press outside closes it — not a click: on a Mac the release of the
    // secondary click that opened the menu can arrive as a click and would
    // close it the moment it appears.
    const outside = (e: PointerEvent) => {
      if (!menuRef.current?.contains(e.target as Node)) close();
    };
    const esc = (e: KeyboardEvent) => e.key === "Escape" && close();
    window.addEventListener("pointerdown", outside, true);
    window.addEventListener("keydown", esc);
    window.addEventListener("blur", close);
    window.addEventListener("resize", close);
    return () => {
      window.removeEventListener("pointerdown", outside, true);
      window.removeEventListener("keydown", esc);
      window.removeEventListener("blur", close);
      window.removeEventListener("resize", close);
    };
  }, [menu]);

  const { songs, query, setQuery, current } = library;
  const favorites = songs.filter((s) => s.favorite);
  const rest = songs.filter((s) => !s.favorite);

  const startResize = (e: React.PointerEvent) => {
    e.preventDefault();
    const move = (ev: PointerEvent) => onResize(Math.min(400, Math.max(200, ev.clientX)));
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      document.body.classList.remove("resizing");
    };
    document.body.classList.add("resizing");
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  };

  const row = (song: SongRecord) => (
    <li key={song.id} className="song-item">
      <button
        className={`song-row${current?.id === song.id ? " current" : ""}`}
        onClick={() => void library.play(song)}
        onDoubleClick={() => setRenaming(song.id)}
        onContextMenu={(e) => {
          e.preventDefault();
          setMenu({ song, x: e.clientX, y: e.clientY });
        }}
      >
        <span className={`song-state${playing && current?.id === song.id ? " playing" : ""}`}>
          <i />
          <i />
          <i />
        </span>
        <span className="song-text">
          {renaming === song.id ? (
            <RenameField
              value={song.title}
              onDone={(title) => {
                setRenaming(null);
                if (title && title !== song.title) void library.rename(song.id, title);
              }}
            />
          ) : (
            <span className="song-title" title={song.sourcePath ?? song.fileName}>
              {song.title}
            </span>
          )}
          <span className="song-sub">
            {[lengthLabel(song.duration), whenLabel(song.lastPlayed, t)]
              .filter(Boolean)
              .join(" · ")}
          </span>
        </span>
        {song.favorite && <span className="song-star">★</span>}
      </button>
      {/* The context menu, for those who don't think to right-click. */}
      <button
        className={`song-more${current?.id === song.id ? " current" : ""}${menu?.song.id === song.id ? " open" : ""}`}
        title={t.sidebar.more}
        aria-label={t.sidebar.more}
        onClick={(e) => {
          const r = e.currentTarget.getBoundingClientRect();
          setMenu({ song, x: r.left, y: r.bottom + 4 });
        }}
      >
        <svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor" aria-hidden>
          <circle cx="6" cy="12" r="1.6" />
          <circle cx="12" cy="12" r="1.6" />
          <circle cx="18" cy="12" r="1.6" />
        </svg>
      </button>
    </li>
  );

  return (
    <aside className="sidebar" style={{ width }}>
      <div className="sidebar-resize" onPointerDown={startResize} />
      {/* The title bar: traffic lights, the toggle over it, and room to drag. */}
      <div className="sidebar-drag" data-tauri-drag-region />

      <div className="sidebar-search">
        <input
          type="search"
          placeholder={t.sidebar.search}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
        <button
          className="icon-btn"
          title={t.sidebar.openFiles}
          aria-label={t.sidebar.openFiles}
          onClick={() => void library.openFiles()}
        >
          <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={1.7} strokeLinecap="round" aria-hidden>
            <path d="M12 5.5v13M5.5 12h13" />
          </svg>
        </button>
      </div>

      <nav className="sidebar-list">
        {songs.length === 0 && (
          <p className="sidebar-empty">
            {query ? t.sidebar.noMatches : t.sidebar.empty}
          </p>
        )}
        {favorites.length > 0 && (
          <>
            <h2 className="sidebar-head">{t.sidebar.favorites}</h2>
            <ul>{favorites.map(row)}</ul>
          </>
        )}
        {rest.length > 0 && (
          <>
            {favorites.length > 0 && <h2 className="sidebar-head">{t.sidebar.library}</h2>}
            <ul>{rest.map(row)}</ul>
          </>
        )}
      </nav>

      <ClaudeConnection />
      <div className="sidebar-foot">
        <button className="sidebar-settings" onClick={onOpenSettings} title={t.settings.open}>
          <svg
            width="14"
            height="14"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth={1.6}
            strokeLinecap="round"
            strokeLinejoin="round"
            aria-hidden
          >
            <circle cx="12" cy="12" r="3" />
            <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
          </svg>
          <span>{t.settings.title}</span>
          {updateReady && <span className="sidebar-settings-dot" role="img" aria-label={t.settings.updateWaiting} />}
        </button>
      </div>

      {menu && (
        <div ref={menuRef} className="context-menu" style={{ left: menu.x, top: menu.y }}>
          <button
            onClick={() => {
              setRenaming(menu.song.id);
              setMenu(null);
            }}
          >
            {t.sidebar.rename}
          </button>
          <button
            onClick={() => {
              void library.toggleFavorite(menu.song);
              setMenu(null);
            }}
          >
            {menu.song.favorite ? t.sidebar.unfavorite : t.sidebar.favorite}
          </button>
          {menu.song.canUndo && (
            <button
              onClick={() => {
                onHistory(menu.song.id, true);
                setMenu(null);
              }}
            >
              {t.sidebar.undoEdit}
            </button>
          )}
          {menu.song.canRedo && (
            <button
              onClick={() => {
                onHistory(menu.song.id, false);
                setMenu(null);
              }}
            >
              {t.sidebar.redoEdit}
            </button>
          )}
          <button
            onClick={() => {
              onExport(menu.song);
              setMenu(null);
            }}
          >
            {t.sidebar.exportScore}
          </button>
          {menu.song.sourcePath && (
            <button
              onClick={() => {
                void revealItemInDir(menu.song.sourcePath!).catch(() => undefined);
                setMenu(null);
              }}
            >
              {t.sidebar.showInFinder}
            </button>
          )}
          <div className="context-sep" />
          <button
            className="danger"
            onClick={() => {
              void library.remove(menu.song.id);
              setMenu(null);
            }}
          >
            {t.sidebar.remove}
          </button>
        </div>
      )}
    </aside>
  );
}

function RenameField({
  value,
  onDone,
}: {
  value: string;
  onDone: (title: string | null) => void;
}) {
  const ref = useRef<HTMLInputElement | null>(null);
  useEffect(() => {
    ref.current?.select();
  }, []);
  return (
    <input
      ref={ref}
      className="song-rename"
      defaultValue={value}
      autoFocus
      onClick={(e) => e.stopPropagation()}
      onBlur={(e) => onDone(e.currentTarget.value.trim())}
      onKeyDown={(e) => {
        e.stopPropagation();
        if (e.key === "Enter") onDone(e.currentTarget.value.trim());
        if (e.key === "Escape") onDone(null);
      }}
    />
  );
}
