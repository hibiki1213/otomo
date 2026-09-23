import { useCallback, useEffect, useRef, useState } from "react";
import type { SongInfo } from "./midiEngine";
import type { SongSource } from "@/components/Player";
import {
  importSong,
  libraryAvailable,
  listSongs,
  pickMidiFiles,
  removeSong,
  renameSong,
  setSongFavorite,
  songData,
  songPlayed,
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
  // Ignore bytes that arrive after another song was picked.
  const wanted = useRef<number | null>(null);

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

  /** Load a library entry into the player. */
  const play = useCallback(async (song: SongRecord) => {
    wanted.current = song.id;
    setError(null);
    setCurrent(song);
    try {
      const buffer = await songData(song.id);
      if (wanted.current !== song.id) return;
      setSource({ id: song.id, name: song.fileName, title: song.title, buffer });
    } catch (e) {
      console.error(e);
      setError(`「${song.title}」を開けませんでした。`);
    }
  }, []);

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
