import { messages } from "./i18n";

/**
 * A range the user drew on the piano roll (time × pitch), attached to a chat
 * message so "here" and "this register" mean something to Claude.
 */

/** A position in the song: 1-based bar, 1-based beat (quarter notes). */
export interface BarBeat {
  bar: number;
  beat: number;
}

export interface SelectedNote {
  track: number;
  midi: number;
  at: BarBeat;
  /** Length in beats. */
  beats: number;
}

export interface RollSelection {
  id: string;
  songId: number | null;
  songTitle: string;
  /** First beat of the range. */
  start: BarBeat;
  /** Last beat of the range (included to its end). */
  end: BarBeat;
  /** Lowest and highest MIDI key, both included. */
  lo: number;
  hi: number;
  /** Tracks with notes in the range, in track order. */
  tracks: { index: number; name: string }[];
  /** Tracks the user took out of this attachment. */
  excluded: number[];
  notes: SelectedNote[];
}

const NAMES = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

/** Same spelling as the MCP server's read_bars (C4 = 60). */
export function noteName(midi: number): string {
  return `${NAMES[midi % 12]}${Math.floor(midi / 12) - 1}`;
}

function num(n: number): string {
  return String(Math.round(n * 100) / 100);
}

/** "bar 5 beat 2" — how Claude is told a position. */
function at(p: BarBeat): string {
  return `bar ${p.bar} beat ${num(p.beat)}`;
}

function included(sel: RollSelection) {
  const out = new Set(sel.excluded);
  return {
    tracks: sel.tracks.filter((t) => !out.has(t.index)),
    notes: sel.notes.filter((n) => !out.has(n.track)),
  };
}

/** What the chip in the chat box says, in the current language. */
export function selectionLabel(sel: RollSelection): string {
  const t = messages().selection;
  const time =
    sel.start.bar === sel.end.bar
      ? t.withinBar(sel.start.bar, num(sel.start.beat), num(sel.end.beat))
      : t.acrossBars(sel.start.bar, num(sel.start.beat), sel.end.bar, num(sel.end.beat));
  const range = sel.lo === sel.hi ? noteName(sel.lo) : `${noteName(sel.lo)}–${noteName(sel.hi)}`;
  const count = included(sel).notes.length;
  return `${time} · ${range} · ${count ? t.notes(count) : t.empty}`;
}

/** How many notes are spelled out before handing over to read_bars. */
const NOTE_LIMIT = 160;

/**
 * What Claude is told about the range (number `n` in this message). English
 * whatever the display language, like everything else Claude reads.
 */
export function selectionContext(sel: RollSelection, n: number): string {
  const { tracks, notes } = included(sel);
  const lines = [
    `[Selection ${n}] Song "${sel.songTitle}"${sel.songId != null ? ` (song_id: ${sel.songId})` : ""}`,
    `Time: from ${at(sel.start)} to the end of ${at(sel.end)}`,
    `Pitch: ${noteName(sel.lo)} (MIDI ${sel.lo}) to ${noteName(sel.hi)} (MIDI ${sel.hi})`,
  ];
  if (tracks.length === 0) {
    lines.push("Tracks: none (no notes in this range)");
    return lines.join("\n");
  }
  lines.push(`Tracks: ${tracks.map((t) => `${t.index} "${t.name}"`).join(", ")}`);
  lines.push(`Notes in range: ${notes.length} (bar:beat pitch length[beats])`);
  let shown = 0;
  for (const t of tracks) {
    const own = notes.filter((x) => x.track === t.index);
    const room = NOTE_LIMIT - shown;
    if (room <= 0) break;
    const listed = own.slice(0, room);
    shown += listed.length;
    lines.push(
      `  Track ${t.index}: ` +
        listed.map((x) => `${x.at.bar}:${num(x.at.beat)} ${noteName(x.midi)} ${num(x.beats)}`).join(", ")
    );
  }
  if (shown < notes.length) {
    lines.push(`  ${notes.length - shown} more not listed (use read_bars)`);
  }
  return lines.join("\n");
}
