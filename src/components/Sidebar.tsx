import { useEffect, useRef, useState } from "react";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import ClaudeConnection from "@/components/ClaudeConnection";
import LanguageSelect from "@/components/LanguageSelect";
import { useI18n } from "@/lib/i18n";
import type { Messages } from "@/locales/en";
import type { SongRecord } from "@/lib/songsApi";
import type { SongLibrary } from "@/lib/useSongLibrary";

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
  onExport,
}: {
  library: SongLibrary;
  /** Whether the current song is sounding, for the row indicator. */
  playing: boolean;
  onExport: (song: SongRecord) => void;
}) {
  const { t } = useI18n();
  const [menu, setMenu] = useState<MenuState | null>(null);
  const [renaming, setRenaming] = useState<number | null>(null);

  useEffect(() => {
    if (!menu) return;
    const close = () => setMenu(null);
    window.addEventListener("click", close);
    window.addEventListener("blur", close);
    const esc = (e: KeyboardEvent) => e.key === "Escape" && close();
    window.addEventListener("keydown", esc);
    return () => {
      window.removeEventListener("click", close);
      window.removeEventListener("blur", close);
      window.removeEventListener("keydown", esc);
    };
  }, [menu]);

  const { songs, query, setQuery, current } = library;
  const favorites = songs.filter((s) => s.favorite);
  const rest = songs.filter((s) => !s.favorite);

  const row = (song: SongRecord) => (
    <li key={song.id}>
      <button
        className={`song-row${current?.id === song.id ? " current" : ""}`}
        onClick={() => void library.play(song)}
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
    </li>
  );

  return (
    <aside className="sidebar">
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
          onClick={() => void library.openFiles()}
        >
          +
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
      <LanguageSelect />

      {menu && (
        <div
          className="context-menu"
          style={{ left: menu.x, top: menu.y }}
          onClick={(e) => e.stopPropagation()}
        >
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
                void library.undoEdit(menu.song.id);
                setMenu(null);
              }}
            >
              {t.sidebar.undoEdit}
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
