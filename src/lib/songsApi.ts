import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";

/** A song in the library (see `src-tauri/src/songs.rs`). */
export interface SongRecord {
  id: number;
  title: string;
  fileName: string;
  bytes: number;
  /** Where it was imported from, if it came from a file on disk. */
  sourcePath: string | null;
  /** Seconds; 0 until the song has been opened once. */
  duration: number;
  trackCount: number;
  /** Unix seconds. */
  addedAt: number;
  lastPlayed: number | null;
  playCount: number;
  favorite: boolean;
  /** An applied edit replaced this song, so it can be put back once. */
  canUndo: boolean;
}

/** An edit the MCP server staged, waiting for approval. */
export interface PendingEdit {
  songId: number;
  title: string;
  /** What the tools did, newest last. */
  changes: string[];
  updated: number;
}

export const libraryAvailable = () => "__TAURI_INTERNALS__" in window;

export const getSong = (id: number) => invoke<SongRecord>("songs_get", { id });

export const listSongs = (query?: string) =>
  invoke<SongRecord[]>("songs_list", { query: query || null });

/** Copy a MIDI file on disk into the library (idempotent per file content). */
export const importSong = (path: string) =>
  invoke<SongRecord>("songs_import", { path });

export async function songData(id: number): Promise<ArrayBuffer> {
  return invoke<ArrayBuffer>("songs_data", { id });
}

export const songPlayed = (id: number, duration: number, trackCount: number) =>
  invoke<void>("songs_played", { id, duration, trackCount });

export const renameSong = (id: number, title: string) =>
  invoke<void>("songs_rename", { id, title });

export const setSongFavorite = (id: number, favorite: boolean) =>
  invoke<void>("songs_set_favorite", { id, favorite });

export const removeSong = (id: number) =>
  invoke<void>("songs_remove", { id });

export const listPendingEdits = () => invoke<PendingEdit[]>("songs_pending");

export async function pendingData(id: number): Promise<ArrayBuffer> {
  return invoke<ArrayBuffer>("songs_pending_data", { id });
}

export const applyPending = (id: number) =>
  invoke<SongRecord>("songs_apply_pending", { id });

export const discardPending = (id: number) =>
  invoke<void>("songs_discard_pending", { id });

export const undoApply = (id: number) =>
  invoke<SongRecord>("songs_undo_apply", { id });

/** The system open panel, filtered to MIDI and MusicXML. Returns the chosen paths. */
export async function pickMidiFiles(): Promise<string[]> {
  const picked = await open({
    multiple: true,
    filters: [{ name: "MIDI・MusicXML", extensions: ["mid", "midi", "musicxml", "xml", "mxl"] }],
  });
  if (!picked) return [];
  return Array.isArray(picked) ? picked : [picked];
}
