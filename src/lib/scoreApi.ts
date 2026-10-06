import { invoke } from "@tauri-apps/api/core";
import type { Engraved } from "./engrave";

/** A track as the export dialog lists it (see `src-tauri/src/export.rs`). */
export interface ScoreTrack {
  index: number;
  name: string;
  /** The General MIDI instrument, or "Drums". */
  instrument: string;
  percussion: boolean;
  notes: number;
}

export const scoreTracks = (id: number, staged: boolean) =>
  invoke<ScoreTrack[]>("score_tracks", { id, staged });

export interface ScoreRequest {
  id: number;
  /** Claude's staged version rather than the saved one. */
  staged: boolean;
  /** Track numbers, in the order to print. */
  tracks: number[];
  /** Transposing instruments at the pitch their players read. */
  transposed: boolean;
  /** Runs of empty bars as one rest (a player's part). */
  multiRests: boolean;
  /** Only the first bars: a quick preview of a long piece. */
  maxBars?: number;
}

export const scoreMusicXml = (request: ScoreRequest) => invoke<string>("score_musicxml", { request });

export const writeTextFile = (path: string, contents: string) =>
  invoke<void>("export_write_text", { path, contents });

export const writePdf = (path: string, engraved: Engraved) =>
  invoke<void>("pdf_write", {
    path,
    pages: engraved.pages,
    widthMm: engraved.widthMm,
    heightMm: engraved.heightMm,
  });

/** A file name from a title: no path separators or characters Finder refuses. */
export function fileStem(text: string): string {
  return text.replace(/[/\\:*?"<>|]+/g, "-").replace(/\s+/g, " ").trim() || "Score";
}

/** A chord symbol on the timeline (see `score_chords` in export.rs). */
export interface ChordAt {
  /** Seconds from the start. */
  time: number;
  /** "F", "Bbmaj7", "G7/B"; "—" for silence, "?" when nothing fits. */
  name: string;
}

/** The song's harmony beat by beat, as Claude's `harmony` tool reads it. */
export const scoreChords = (id: number, staged: boolean) =>
  invoke<ChordAt[]>("score_chords", { id, staged });
