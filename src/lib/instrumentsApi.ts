import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/** What a track plays through (mirrors `InstrumentChoice` in Rust). */
export type InstrumentChoice =
  | { kind: "sf2" }
  | { kind: "sfz"; path: string; gainDb?: number }
  | {
      kind: "sfzSplit";
      parts: { path: string; lo: number; hi: number }[];
      gainDb?: number;
    }
  | { kind: "audioUnit"; id: string }
  /** A preset of the built-in synthesizer. */
  | { kind: "synth"; preset: string };

export interface InstrumentEntry {
  id: string;
  name: string;
  library: string;
  libraryName: string;
  /** Pass to setTrackInstrument as-is. */
  choice: InstrumentChoice;
}

/** Stable key for a choice (for <select> values and comparisons). */
export function choiceKey(choice: InstrumentChoice): string {
  switch (choice.kind) {
    case "sf2":
      return "sf2";
    case "sfz":
      return `sfz:${choice.path}`;
    case "sfzSplit":
      return `split:${choice.parts.map((p) => p.path).join("|")}`;
    case "audioUnit":
      return `au:${choice.id}`;
    case "synth":
      return `synth:${choice.preset}`;
  }
}

export interface LibraryStatus {
  id: string;
  name: string;
  description: string;
  license: string;
  attribution: string;
  homepage: string;
  downloadSize: number;
  installed: boolean;
  installing: boolean;
  instruments: InstrumentEntry[];
}

export interface PluginInfo {
  id: string;
  name: string;
  manufacturer: string;
  format: string;
}

export interface LibraryProgress {
  id: string;
  phase: "download" | "extract" | "done" | "error";
  received: number;
  total: number;
  message: string | null;
}

/** Resolves once the instrument is loaded and playing on the track. */
export function setTrackInstrument(
  track: number,
  choice: InstrumentChoice
): Promise<void> {
  return invoke("engine_set_track_instrument", { track, choice });
}

export function listPlugins(): Promise<PluginInfo[]> {
  return invoke("plugin_list");
}

export function openPluginEditor(track: number, title: string): Promise<void> {
  return invoke("plugin_open_editor", { track, title });
}

/** The built-in synthesizer's presets, always available. */
export function listSynthPresets(): Promise<InstrumentEntry[]> {
  return invoke("synth_presets");
}

export function listLibraries(): Promise<LibraryStatus[]> {
  return invoke("library_list");
}

export function installLibrary(id: string): Promise<void> {
  return invoke("library_install", { id });
}

export function removeLibrary(id: string): Promise<void> {
  return invoke("library_remove", { id });
}

/** SFZ files the user added from disk ("マイ音源"), still in place. */
export function listUserInstruments(): Promise<InstrumentEntry[]> {
  return invoke("library_user_instruments");
}

/** Remember an .sfz picked from disk, so it is offered from now on. */
export function addUserInstrument(path: string): Promise<InstrumentEntry> {
  return invoke("library_add_user_instrument", { path });
}

/** A track's instrument as remembered for a song. */
export interface SavedInstrument {
  track: number;
  choice: InstrumentChoice;
  label: string;
}

export function loadSongInstruments(songId: number): Promise<SavedInstrument[]> {
  return invoke("songs_instruments", { id: songId });
}

export function saveSongInstrument(
  songId: number,
  track: number,
  choice: InstrumentChoice,
  label: string
): Promise<void> {
  return invoke("songs_set_instrument", { id: songId, track, choice, label });
}

export function clearSongInstruments(songId: number): Promise<void> {
  return invoke("songs_clear_instruments", { id: songId });
}

/** Claude changed a track's instrument (through the MCP server). */
export function onInstrumentAssigned(
  cb: (a: SavedInstrument) => void
): Promise<UnlistenFn> {
  return listen<SavedInstrument>("instrument-assigned", (e) => cb(e.payload));
}

/** Best installed SFZ instrument per track, or null to keep the SoundFont. */
export function suggestInstruments(
  tracks: { program: number; percussion: boolean }[]
): Promise<(InstrumentEntry | null)[]> {
  return invoke("library_suggest", { tracks });
}

export function onLibraryProgress(
  cb: (p: LibraryProgress) => void
): Promise<UnlistenFn> {
  return listen<LibraryProgress>("library-progress", (e) => cb(e.payload));
}

/** Let the user pick any .sfz file on disk. */
export async function pickSfzFile(): Promise<string | null> {
  const { open } = await import("@tauri-apps/plugin-dialog");
  const picked = await open({
    multiple: false,
    directory: false,
    filters: [{ name: "SFZ", extensions: ["sfz"] }],
  });
  return typeof picked === "string" ? picked : null;
}

export function formatBytes(n: number): string {
  if (n >= 1e9) return `${(n / 1e9).toFixed(1)} GB`;
  if (n >= 1e6) return `${Math.round(n / 1e6)} MB`;
  return `${Math.max(1, Math.round(n / 1e3))} KB`;
}
