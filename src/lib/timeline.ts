/**
 * Seconds ↔ beats ↔ bars for a loaded song: the tempo map and the bar lines
 * its time signatures make. Shared by the piano roll, its lanes and the
 * event list, so a bar number means the same everywhere.
 */
import type { SongInfo, TempoEvent, TimeSignatureEvent } from "./midiEngine";
import type { BarBeat } from "./rollSelection";

export interface TempoSegment {
  startTime: number;
  endTime: number;
  startBeat: number;
  endBeat: number;
  bpm: number;
}

export interface TempoMap {
  segments: TempoSegment[];
  totalBeats: number;
}

export function buildTempoMap(tempos: TempoEvent[], duration: number): TempoMap {
  const segments: TempoSegment[] = [];
  let beat = 0;
  for (let i = 0; i < tempos.length; i++) {
    const startTime = tempos[i].time;
    const endTime = i + 1 < tempos.length ? tempos[i + 1].time : duration;
    const span = Math.max(0, endTime - startTime);
    const beats = (span * tempos[i].bpm) / 60;
    segments.push({
      startTime,
      endTime,
      startBeat: beat,
      endBeat: beat + beats,
      bpm: tempos[i].bpm,
    });
    beat += beats;
  }
  return { segments, totalBeats: beat };
}

function findTempoSegmentByTime(map: TempoMap, t: number): TempoSegment {
  const segs = map.segments;
  let lo = 0;
  let hi = segs.length - 1;
  while (lo < hi) {
    const mid = (lo + hi + 1) >>> 1;
    if (segs[mid].startTime <= t) lo = mid;
    else hi = mid - 1;
  }
  return segs[lo];
}

function findTempoSegmentByBeat(map: TempoMap, b: number): TempoSegment {
  const segs = map.segments;
  let lo = 0;
  let hi = segs.length - 1;
  while (lo < hi) {
    const mid = (lo + hi + 1) >>> 1;
    if (segs[mid].startBeat <= b) lo = mid;
    else hi = mid - 1;
  }
  return segs[lo];
}

export function timeToBeat(map: TempoMap, t: number): number {
  const seg = findTempoSegmentByTime(map, t);
  return seg.startBeat + ((t - seg.startTime) * seg.bpm) / 60;
}

export function beatToTime(map: TempoMap, b: number): number {
  const seg = findTempoSegmentByBeat(map, b);
  return seg.startTime + ((b - seg.startBeat) * 60) / seg.bpm;
}

export function bpmAtTime(map: TempoMap, t: number): number {
  return findTempoSegmentByTime(map, t).bpm;
}

export interface BarBoundary {
  beat: number;
  bar: number;
}

/**
 * The (beat, bar-number) boundaries the time signatures make, within
 * [0, totalBeats].
 *
 * Same rule as the Rust side (`BarMap` in src-tauri/crates/score), so that a
 * bar number means the same here as in what Claude reads and writes: a
 * signature change takes effect at the next bar line of the signature in
 * force, and of two changes inside one bar the later wins. Files that put a
 * change mid-bar exist (a 1/4 pickup with 4/4 starting half-way through it);
 * starting the new bars right there would put every later bar off by the rest
 * of that bar.
 */
export function buildBarMap(
  timeSignatures: TimeSignatureEvent[],
  map: TempoMap
): { bars: BarBoundary[]; beatsPerBarAt: (beat: number) => number } {
  const sigs = timeSignatures
    .map((s) => ({ ...s, startBeat: timeToBeat(map, s.time) }))
    .sort((a, b) => a.startBeat - b.startBeat);
  // Spans of bars under one signature: first bar (0-based), start beat, bar length.
  const spans: { first: number; start: number; perBar: number }[] = [];
  for (const s of sigs) {
    const perBar = s.numerator * (4 / s.denominator);
    const prev = spans[spans.length - 1];
    let first = 0;
    let start = 0;
    if (prev) {
      const bars = Math.ceil(Math.max(0, s.startBeat - prev.start) / prev.perBar - 1e-6);
      first = prev.first + bars;
      start = prev.start + bars * prev.perBar;
    }
    if (prev && prev.first === first) spans.pop();
    spans.push({ first, start, perBar });
  }
  const total = map.totalBeats;
  const bars: BarBoundary[] = [];
  for (let i = 0; i < spans.length; i++) {
    const { first, start, perBar } = spans[i];
    const end = i + 1 < spans.length ? spans[i + 1].start : total;
    for (let k = 0, b = start; b < end - 1e-6; k++, b = start + k * perBar) {
      bars.push({ beat: b, bar: first + k });
    }
  }
  if (bars.length === 0) bars.push({ beat: 0, bar: 0 });
  const beatsPerBarAt = (beat: number): number => {
    let span = spans[0];
    for (const s of spans) {
      if (s.start <= beat + 1e-6) span = s;
      else break;
    }
    return span?.perBar ?? 4;
  };
  return { bars, beatsPerBarAt };
}

/** Where 1-based `bar` starts, in quarter notes; past the end, bars of the last length. */
export function beatOfBar(song: SongInfo, bar: number): number {
  const { bars, beatsPerBarAt } = buildBarMap(song.timeSignatures, buildTempoMap(song.tempos, song.duration));
  if (bar <= bars.length) return bars[Math.max(0, bar - 1)].beat;
  const last = bars[bars.length - 1];
  return last.beat + (bar - bars.length) * beatsPerBarAt(last.beat);
}

/** The bar the playhead is in. */
export function playheadBar(song: SongInfo, seconds: number): number {
  const tempo = buildTempoMap(song.tempos, song.duration);
  return beatPosition(buildBarMap(song.timeSignatures, tempo).bars, timeToBeat(tempo, seconds)).bar;
}

/** A beat (quarter notes from the start) as 1-based bar and beat-in-bar. */
export function beatPosition(bars: BarBoundary[], beat: number): BarBeat {
  let lo = 0;
  let hi = bars.length - 1;
  while (lo < hi) {
    const mid = (lo + hi + 1) >> 1;
    if (bars[mid].beat <= beat + 1e-6) lo = mid;
    else hi = mid - 1;
  }
  const b = bars[lo] ?? { beat: 0, bar: 0 };
  return { bar: b.bar + 1, beat: beat - b.beat + 1 };
}

