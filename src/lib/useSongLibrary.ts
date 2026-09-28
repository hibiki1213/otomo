import { useCallback, useEffect, useRef, useState } from "react";
import type { SongInfo } from "./midiEngine";
import type { SongSource } from "@/components/Player";
import {
  applyPending,
  discardPending,
  getSong,
  importSong,
  listPendingEdits,
  pendingData,
  undoApply,
  libraryAvailable,
  listSongs,
  pickMidiFiles,
  removeSong,
  renameSong,
  setSongFavorite,
  songData,
  songPlayed,
  type PendingEdit,
  type SongRecord,
} from "./songsApi";

/**
 * The song library: what has been opened before, what is playing now, and the
 * ways in (the open panel, drag and drop, clicking an entry).
 */
export function useSongLibrary() {
  const [songs, setSongs] = useState<SongRecord[]>([]);
  const [query, setQuery] = useState("");
  const [current, setCurrent] = useState<SongRecord | null>(null);
  const [source, setSource] = useState<SongSource | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState<PendingEdit[]>([]);
  // Ignore bytes that arrive after another song was picked.
  const wanted = useRef<number | null>(null);
  // Bumped per request, so asking for the same song again still reloads it
  // while editing the entry (a rename) never does.
  const token = useRef(0);

  const refresh = useCallback(async (q = ""): Promise<SongRecord[]> => {
    if (!libraryAvailable()) return [];
    try {
      const list = await listSongs(q);
      setSongs(list);
      return list;
    } catch (e) {
      console.error("song library", e);
      return [];
    }
  }, []);

  useEffect(() => {
    const t = setTimeout(() => void refresh(query), query ? 120 : 0);
    return () => clearTimeout(t);
  }, [query, refresh]);

  const refreshPending = useCallback(async () => {
    if (!libraryAvailable()) return;
    try {
      setPending(await listPendingEdits());
    } catch (e) {
      console.error("pending edits", e);
    }
  }, []);

  // Edits can also be staged while the app is closed, so look on startup.
  useEffect(() => {
    void refreshPending();
  }, [refreshPending]);

  /**
   * Load a library entry into the player, optionally starting at `startAt`
   * seconds and playing straight away (which is what MCP's `play` asks for).
   */
  const play = useCallback(
    async (
      song: SongRecord,
      startAt?: number,
      staged = false,
      range?: { stopAt?: number; loop?: boolean; solo?: number[] }
    ) => {
      wanted.current = song.id;
      const mine = ++token.current;
      setError(null);
      setCurrent(song);
      try {
        const buffer = staged
          ? await pendingData(song.id)
          : await songData(song.id);
        if (token.current !== mine) return;
        setSource({
          id: song.id,
          token: mine,
          name: song.fileName,
          title: song.title,
          buffer,
          startAt,
          autoplay: startAt !== undefined,
          staged,
          ...range,
        });
      } catch (e) {
        console.error(e);
        setError(`「${song.title}」を開けませんでした。`);
      }
    },
    []
  );

  /** Import files from disk (open panel or drop) and play the first one. */
  const importPaths = useCallback(
    async (paths: string[]) => {
      if (paths.length === 0) return;
      setError(null);
      const added: SongRecord[] = [];
      for (const path of paths) {
        try {
          added.push(await importSong(path));
        } catch (e) {
          console.error(e);
          setError(String(e));
        }
      }
      await refresh(query);
      if (added[0]) await play(added[0]);
    },
    [play, query, refresh]
  );

  const openFiles = useCallback(async () => {
    if (!libraryAvailable()) return;
    await importPaths(await pickMidiFiles());
  }, [importPaths]);

  /** Save what opening the song revealed, and count the play. */
  const onLoaded = useCallback(
    (id: number, info: SongInfo) => {
      void songPlayed(id, info.duration, info.tracks.length)
        .then(() => refresh(query))
        .catch((e) => console.error("songs_played", e));
    },
    [query, refresh]
  );

  const mutate = useCallback(
    async (id: number, run: () => Promise<void>) => {
      try {
        await run();
      } catch (e) {
        console.error(e);
        setError(String(e));
      }
      const list = await refresh(query);
      // Keep what the player shows in step with the edit (e.g. a rename).
      const updated = list.find((s) => s.id === id);
      if (updated && current?.id === id) {
        setCurrent(updated);
        setSource((s) => (s && s.id === id ? { ...s, title: updated.title } : s));
      }
    },
    [current?.id, query, refresh]
  );

  return {
    songs,
    query,
    setQuery,
    current,
    source,
    error,
    dismissError: useCallback(() => setError(null), []),
    play,
    openFiles,
    importPaths,
    onLoaded,
    pending,
    refreshPending,
    /** Re-read the list, for songs added from outside (Claude's create_song). */
    refreshSongs: useCallback(() => refresh(query), [query, refresh]),
    /** Play a song by id — the route the MCP server's `play` tool takes. */
    playById: useCallback(
      async (
        id: number,
        seconds: number,
        staged = false,
        range?: { stopAt?: number; loop?: boolean; solo?: number[] }
      ) => {
        try {
          await play(await getSong(id), seconds, staged, range);
        } catch (e) {
          console.error(e);
          setError(String(e));
        }
      },
      [play]
    ),
    /** Listen to a staged edit before deciding on it. */
    previewPending: useCallback(
      async (songId: number) => {
        try {
          await play(await getSong(songId), 0, true);
        } catch (e) {
          console.error(e);
          setError(String(e));
        }
      },
      [play]
    ),
    applyEdit: useCallback(
      async (songId: number) => {
        try {
          const record = await applyPending(songId);
          await refreshPending();
          await refresh(query);
          await play(record);
        } catch (e) {
          console.error(e);
          setError(String(e));
        }
      },
      [play, query, refresh, refreshPending]
    ),
    discardEdit: useCallback(
      async (songId: number) => {
        try {
          await discardPending(songId);
        } catch (e) {
          console.error(e);
          setError(String(e));
        }
        await refreshPending();
        // Whatever was being previewed is gone; go back to the saved song.
        if (wanted.current === songId) {
          try {
            await play(await getSong(songId));
          } catch {
            /* the song may have gone too */
          }
        }
      },
      [play, refreshPending]
    ),
    undoEdit: useCallback(
      async (songId: number) => {
        try {
          await play(await undoApply(songId));
          await refresh(query);
        } catch (e) {
          console.error(e);
          setError(String(e));
        }
      },
      [play, query, refresh]
    ),
    rename: useCallback(
      (id: number, title: string) => mutate(id, () => renameSong(id, title)),
      [mutate]
    ),
    toggleFavorite: useCallback(
      (s: SongRecord) => mutate(s.id, () => setSongFavorite(s.id, !s.favorite)),
      [mutate]
    ),
    remove: useCallback(
      (id: number) => {
        if (current?.id === id) {
          setCurrent(null);
          setSource(null);
          wanted.current = null;
        }
        return mutate(id, () => removeSong(id));
      },
      [current?.id, mutate]
    ),
  };
}

export type SongLibrary = ReturnType<typeof useSongLibrary>;
