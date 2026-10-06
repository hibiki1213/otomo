/**
 * Text inside MIDI files: what it says (the bytes carry no encoding), chord
 * symbols written as text events, and lyrics split into lines.
 */

const utf8 = new TextDecoder("utf-8", { fatal: true });
let sjis: TextDecoder | null | undefined;

/**
 * The MIDI parser reads text one byte per character. Japanese files are
 * usually Shift_JIS and newer ones UTF-8, so take whichever decodes cleanly;
 * plain ASCII and Latin-1 come through as they are.
 */
export function decodeText(text: string): string {
  if (!/[\x80-\xff]/.test(text) || /[^\x00-\xff]/.test(text)) return text;
  const bytes = Uint8Array.from(text, (c) => c.charCodeAt(0));
  try {
    return utf8.decode(bytes);
  } catch {
    /* not UTF-8 */
  }
  if (sjis === undefined) {
    try {
      sjis = new TextDecoder("shift_jis", { fatal: true });
    } catch {
      sjis = null;
    }
  }
  if (sjis) {
    try {
      return sjis.decode(bytes);
    } catch {
      /* not Shift_JIS either */
    }
  }
  return text;
}

/* ---------- chord symbols ---------- */

export interface ChordEvent {
  time: number;
  name: string;
}

// Root, then the quality, then an optional bass: "C# 6", "B  Maj_7th",
// "F  /A", "Bbm7(9)", "Gsus4".
const CHORD = /^([A-G])([#b♯♭]?)\s*([^/]*?)\s*(?:\/\s*([A-G][#b♯♭]?))?$/;
// What a quality may be made of; anything else ("Over", "Bass") is not a chord.
const QUALITY = /^(?:maj|min|mi|ma|m|dim|aug|sus|add|alt|omit|no|th|st|nd|rd|\d|[#b♯♭+°ø()\-,.]|o)*$/i;

/** A chord symbol written out tidily, or null when the text is not one. */
export function chordSymbol(text: string): string | null {
  const m = CHORD.exec(text.trim());
  if (!m) return null;
  const [, root, accidental, rawQuality, bass] = m;
  let quality = rawQuality.replace(/[\s_]+/g, "");
  if (!QUALITY.test(quality)) return null;
  quality = quality
    .replace(/(\d+)(?:st|nd|rd|th)/g, "$1")
    .replace(/^maj(?=$)/i, "")
    .replace(/^ma(?:j)?(?=\d)/i, "maj")
    .replace(/^mi(?:n)?/, "m")
    .replace(/^Min/, "m");
  const flat = (s: string) => s.replace("♭", "b").replace("♯", "#");
  return `${root}${flat(accidental)}${quality}${bass ? `/${flat(bass)}` : ""}`;
}

/**
 * Text events that are a chord chart rather than notes to the reader: most
 * of them read as chord symbols. Returns the chords, or none.
 */
export function chordsFromText(texts: { time: number; text: string }[]): ChordEvent[] {
  const chords: ChordEvent[] = [];
  let other = 0;
  for (const { time, text } of texts) {
    if (!text.trim()) continue;
    const name = chordSymbol(text);
    if (name) chords.push({ time, name });
    else other++;
  }
  if (chords.length < 4 || chords.length < other) return [];
  return chords.sort((a, b) => a.time - b.time);
}

/* ---------- lyrics ---------- */

export interface LyricLine {
  time: number;
  syllables: { time: number; text: string }[];
}

/**
 * Lyric events as lines. Karaoke files mark a new line with "\r", "\n" or
 * "/" and a new verse with "\\", alone or at either end of a syllable.
 */
export function lyricLines(lyrics: { time: number; text: string }[]): LyricLine[] {
  const lines: LyricLine[] = [];
  let current: LyricLine | null = null;
  let breakNext = true;
  for (const { time, text } of lyrics) {
    const startsLine = /^[\r\n/\\]/.test(text);
    const endsLine = /[\r\n]$/.test(text);
    const clean = text.replace(/[\r\n/\\]/g, "");
    if (startsLine) breakNext = true;
    if (clean.trim()) {
      if (breakNext || !current) {
        current = { time, syllables: [] };
        lines.push(current);
        breakNext = false;
      }
      current.syllables.push({ time, text: clean });
    }
    if (endsLine) breakNext = true;
  }
  return lines;
}
