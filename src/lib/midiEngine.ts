import type {
  Smplr,
  SampleLoader as SampleLoaderT,
  SmplrPreset,
  SmplrRegion,
} from "smplr";
import { DrumSynth } from "./drumSynth";
import { diskCacheStorage, fetchCachedArrayBuffer } from "./assetCache";
import {
  prettyInstrumentName,
  instrumentFamily,
  isPianoProgram,
} from "./gm";

/**
 * GeneralUser GS — a CORS-served, GM-compliant SoundFont2 file (~31 MB).
 * Provides much higher quality samples than the MusyngKite kit for non-piano
 * instruments. Fetched once and parsed; subsequent track instances reuse the
 * parsed SoundFont2 via the module-level cache below.
 */
export const SF2_URL =
  "https://raw.githubusercontent.com/ad-si/GeneralUser/master/GeneralUser.sf2";

// SF2 generator IDs we care about (from the SF2.04 spec). Listed here rather
// than imported because the soundfont2 library's enum is types-only.
const GEN_INITIAL_ATTENUATION = 48; // centibels of attenuation (positive = quieter)
const GEN_COARSE_TUNE = 51; // semitones
const GEN_FINE_TUNE = 52; // cents
const GEN_SAMPLE_MODES = 54; // 0 = no loop, 1 = continuous loop, 3 = loop until release
const GEN_OVERRIDING_ROOT_KEY = 58;

interface ZoneItemsLike {
  keyRange?: { lo: number; hi: number };
  generators?: Record<number, { value?: number } | undefined>;
}
interface SampleLike {
  data: Int16Array;
  header: {
    name: string;
    sampleRate: number;
    originalPitch: number;
    pitchCorrection: number;
    startLoop: number;
    endLoop: number;
  };
}
type PresetZoneLike = ZoneItemsLike & {
  instrument?: {
    globalZone?: ZoneItemsLike;
    zones: InstrumentZoneLike[];
  };
};
type InstrumentZoneLike = ZoneItemsLike & {
  sample?: SampleLike;
};
type Sf2Lib = typeof import("soundfont2");
type ParsedSf2 = InstanceType<Sf2Lib["SoundFont2"]>;

function genValue(zone: ZoneItemsLike | undefined, id: number): number | undefined {
  return zone?.generators?.[id]?.value;
}

let cachedSf2: ParsedSf2 | null = null;
let cachedSf2Promise: Promise<ParsedSf2> | null = null;

/** Fetch + parse GeneralUser GS once, sharing the result across all instruments. */
async function ensureSf2(sfLib: Sf2Lib): Promise<ParsedSf2> {
  if (cachedSf2) return cachedSf2;
  if (cachedSf2Promise) return cachedSf2Promise;
  cachedSf2Promise = (async () => {
    const buf = await fetchCachedArrayBuffer(SF2_URL);
    const sf2 = new sfLib.SoundFont2(new Uint8Array(buf));
    cachedSf2 = sf2;
    if (typeof window !== "undefined") {
      (window as unknown as { __sf2: ParsedSf2 }).__sf2 = sf2;
    }
    return sf2;
  })();
  return cachedSf2Promise;
}

interface PresetBuild {
  json: SmplrPreset;
  buffers: Map<string, AudioBuffer>;
}

/** Per-AudioContext cache of AudioBuffers, keyed by `sampleName#variantKey`. */
const audioBufferCacheByCtx = new WeakMap<
  BaseAudioContext,
  Map<string, AudioBuffer>
>();
/** Per-AudioContext cache of fully-built presets keyed by `bank:program`. */
const presetCacheByCtx = new WeakMap<BaseAudioContext, Map<string, PresetBuild>>();

const presetKey = (bank: number, program: number) => `${bank}:${program}`;

function int16ToAudioBuffer(
  ctx: BaseAudioContext,
  data: Int16Array,
  sampleRate: number
): AudioBuffer {
  const buf = ctx.createBuffer(1, data.length, sampleRate);
  const out = buf.getChannelData(0);
  for (let i = 0; i < data.length; i++) out[i] = data[i] / 32768;
  return buf;
}

/**
 * Build a smplr preset for a single GM program from the parsed SF2.
 *
 * SF2 generators we honour:
 *  - OverridingRootKey (58) + sample.originalPitch → region.pitch
 *  - CoarseTune (51) + FineTune (52) + sample.pitchCorrection → folded into
 *    a fractional region.pitch (smplr does `(midi - pitch) * 100` for detune,
 *    so subtracting cents from the root pitch is exact)
 *  - InitialAttenuation (48) → region.volume in dB. SF2 stores centibels
 *    of positive attenuation; dB volume = -cB / 10.
 *  - SampleModes (54) → loop on/off
 *
 * Both preset-level and instrument-level zones contribute additively per
 * SF2 spec §9.5.
 */
function buildPresetForProgram(
  ctx: BaseAudioContext,
  sf2: ParsedSf2,
  bank: number,
  program: number
): PresetBuild | null {
  let presetCache = presetCacheByCtx.get(ctx);
  if (!presetCache) {
    presetCache = new Map();
    presetCacheByCtx.set(ctx, presetCache);
  }
  const cacheKey = presetKey(bank, program);
  const hit = presetCache.get(cacheKey);
  if (hit) return hit;

  const preset = sf2.presets.find(
    (p) => p.header.bank === bank && p.header.preset === program
  );
  if (!preset) return null;

  let bufCache = audioBufferCacheByCtx.get(ctx);
  if (!bufCache) {
    bufCache = new Map();
    audioBufferCacheByCtx.set(ctx, bufCache);
  }

  const pg = (preset as unknown as { globalZone?: ZoneItemsLike }).globalZone;
  const regions: SmplrRegion[] = [];
  const buffers = new Map<string, AudioBuffer>();
  let zoneCounter = 0;

  for (const pz of preset.zones as unknown as PresetZoneLike[]) {
    if (!pz.instrument) continue;
    const inst = pz.instrument;
    const ig = inst.globalZone;
    const pRange = pz.keyRange;

    const presetCoarse =
      (genValue(pg, GEN_COARSE_TUNE) ?? 0) +
      (genValue(pz, GEN_COARSE_TUNE) ?? 0);
    const presetFine =
      (genValue(pg, GEN_FINE_TUNE) ?? 0) +
      (genValue(pz, GEN_FINE_TUNE) ?? 0);
    const presetAttenCb =
      (genValue(pg, GEN_INITIAL_ATTENUATION) ?? 0) +
      (genValue(pz, GEN_INITIAL_ATTENUATION) ?? 0);

    for (const iz of inst.zones) {
      if (!iz.sample) continue;
      const iRange = iz.keyRange;
      let keyRange: { lo: number; hi: number } | undefined;
      if (pRange && iRange) {
        const lo = Math.max(pRange.lo, iRange.lo);
        const hi = Math.min(pRange.hi, iRange.hi);
        if (lo > hi) continue;
        keyRange = { lo, hi };
      } else {
        keyRange = iRange ?? pRange;
      }

      const coarseTune =
        presetCoarse +
        (genValue(ig, GEN_COARSE_TUNE) ?? 0) +
        (genValue(iz, GEN_COARSE_TUNE) ?? 0);
      const fineTune =
        presetFine +
        (genValue(ig, GEN_FINE_TUNE) ?? 0) +
        (genValue(iz, GEN_FINE_TUNE) ?? 0);
      const attenCb =
        presetAttenCb +
        (genValue(ig, GEN_INITIAL_ATTENUATION) ?? 0) +
        (genValue(iz, GEN_INITIAL_ATTENUATION) ?? 0);
      const overrideRoot =
        genValue(iz, GEN_OVERRIDING_ROOT_KEY) ??
        genValue(ig, GEN_OVERRIDING_ROOT_KEY) ??
        -1;
      const sampleModes =
        genValue(iz, GEN_SAMPLE_MODES) ??
        genValue(ig, GEN_SAMPLE_MODES) ??
        0;

      const hdr = iz.sample.header;
      const effectiveRoot =
        overrideRoot >= 0 && overrideRoot <= 127
          ? overrideRoot
          : hdr.originalPitch;
      const pitch =
        effectiveRoot - coarseTune - (fineTune + hdr.pitchCorrection) / 100;
      const volumeDb = -attenCb / 10;

      // Cache AudioBuffers per unique underlying sample so multiple regions /
      // multiple presets sharing the same sample don't duplicate decoding.
      const bufKey = `${hdr.name}@${hdr.sampleRate}`;
      let abuf = bufCache.get(bufKey);
      if (!abuf) {
        abuf = int16ToAudioBuffer(ctx, iz.sample.data, hdr.sampleRate);
        bufCache.set(bufKey, abuf);
      }
      // Region-level sample reference: smplr looks up the region.sample string
      // in the buffers Map passed to loadInstrument. Use a per-region key so
      // loop info (which is region-level, not buffer-level) doesn't collide.
      const regionSampleName = `${bufKey}#${zoneCounter++}`;
      buffers.set(regionSampleName, abuf);

      const loopOn = sampleModes === 1 || sampleModes === 3;
      const region: SmplrRegion = {
        sample: regionSampleName,
        pitch,
        volume: volumeDb,
      };
      if (keyRange) region.keyRange = [keyRange.lo, keyRange.hi];
      if (loopOn && hdr.endLoop > hdr.startLoop) {
        region.loop = true;
        region.loopStart = hdr.startLoop / hdr.sampleRate;
        region.loopEnd = hdr.endLoop / hdr.sampleRate;
      }
      regions.push(region);
    }
  }

  if (regions.length === 0) return null;

  const built: PresetBuild = {
    json: {
      samples: { baseUrl: "", formats: [] },
      groups: [{ regions }],
    },
    buffers,
  };
  presetCache.set(cacheKey, built);
  return built;
}

export type PlayerState =
  | "idle"
  | "loading"
  | "ready"
  | "playing"
  | "paused";

export interface NoteEvent {
  trackIndex: number;
  midi: number;
  time: number;
  /** Original note duration in seconds (display / piano roll). */
  duration: number;
  /** Audible duration after applying sustain pedal extension. */
  effectiveDuration: number;
  velocity: number;
  /** Program in effect at note start (used to pick the right soundfont). */
  program: number;
}

export interface TrackInfo {
  index: number;
  channel: number;
  name: string;
  instrument: string;
  family: string;
  isPercussion: boolean;
  noteCount: number;
  color: string;
  programs: number[];
  /** ユーザーが上書きした楽器プログラム（未指定なら元のMIDIの値）。 */
  program: number;
}

export interface TempoEvent {
  time: number;
  bpm: number;
}

export interface TimeSignatureEvent {
  time: number;
  numerator: number;
  denominator: number;
}

export interface KeySignatureEvent {
  time: number;
  key: string;
  scale: "major" | "minor";
}

export interface MarkerEvent {
  time: number;
  text: string;
  kind: "marker" | "cuePoint" | "lyrics" | "text";
}

export interface SongInfo {
  name: string;
  duration: number;
  bpm: number;
  bpmMin: number;
  bpmMax: number;
  minMidi: number;
  maxMidi: number;
  /** Longest note duration in seconds — used to size lookback windows. */
  maxNoteDuration: number;
  tracks: TrackInfo[];
  notes: NoteEvent[];
  tempos: TempoEvent[];
  timeSignatures: TimeSignatureEvent[];
  keySignatures: KeySignatureEvent[];
  markers: MarkerEvent[];
  textMeta: string[];
}

interface CCPoint {
  time: number;
  value: number;
}

interface BendPoint {
  time: number;
  /** Normalized -1..1; cents = value * bendRangeSemitones * 100 */
  value: number;
}

interface ProgramChange {
  time: number;
  program: number;
}

export interface InternalTrackData {
  channel: number;
  programs: number[];
  programChanges: ProgramChange[];
  volume: CCPoint[];
  expression: CCPoint[];
  pan: CCPoint[];
  pedal: CCPoint[];
  reverbSend: CCPoint[];
  modulation: CCPoint[];
  channelAftertouch: CCPoint[];
  pitchBends: BendPoint[];
  bendRangeSemitones: number;
}

// Apple system color palette — readable on both light and dark surfaces.
const TRACK_COLORS = [
  "#0A84FF",
  "#30D158",
  "#FF9F0A",
  "#FF375F",
  "#BF5AF2",
  "#64D2FF",
  "#5E5CE6",
  "#FFD60A",
  "#FF453A",
  "#AC8E68",
  "#66D4CF",
  "#FF6482",
];

const KEY_NAMES = [
  "Cb",
  "Gb",
  "Db",
  "Ab",
  "Eb",
  "Bb",
  "F",
  "C",
  "G",
  "D",
  "A",
  "E",
  "B",
  "F#",
  "C#",
];

const clamp = (lo: number, v: number, hi: number) =>
  Math.max(lo, Math.min(hi, v));

const LOOKAHEAD = 0.3;

/** Everything extracted from a MIDI file, independent of any audio backend. */
export interface ParsedSong {
  info: SongInfo;
  tracksData: InternalTrackData[];
  /** Channel voice messages (controllers, pitch bend) in time order. */
  channelEvents: RawChannelEvent[];
}

export interface RawChannelEvent {
  time: number;
  channel: number;
  status: number;
  data1: number;
  data2: number;
}

export async function parseSong(
  buffer: ArrayBuffer,
  fileName: string
): Promise<ParsedSong> {
  const { Midi } = await import("@tonejs/midi");
  const midiFile = await import("midi-file");

  const midi = new Midi(buffer);

  /* ----- tempos ----- */
  const rawTempos = midi.header.tempos
    .map((t) => ({ time: t.time ?? 0, bpm: t.bpm }))
    .sort((a, b) => a.time - b.time);
  const tempos: TempoEvent[] =
    rawTempos.length === 0
      ? [{ time: 0, bpm: 120 }]
      : rawTempos[0].time > 0.0001
      ? [{ time: 0, bpm: rawTempos[0].bpm }, ...rawTempos]
      : rawTempos;
  const bpm = Math.round(tempos[0].bpm);
  const bpmMin = Math.round(Math.min(...tempos.map((t) => t.bpm)));
  const bpmMax = Math.round(Math.max(...tempos.map((t) => t.bpm)));

  /* ----- time signatures ----- */
  const rawTS = midi.header.timeSignatures
    .map((t) => ({
      time: midi.header.ticksToSeconds(t.ticks),
      numerator: t.timeSignature[0],
      denominator: t.timeSignature[1],
    }))
    .sort((a, b) => a.time - b.time);
  const timeSignatures: TimeSignatureEvent[] =
    rawTS.length === 0
      ? [{ time: 0, numerator: 4, denominator: 4 }]
      : rawTS[0].time > 0.0001
      ? [{ time: 0, numerator: 4, denominator: 4 }, ...rawTS]
      : rawTS;

  /* ----- key signatures ----- */
  const keySignatures: KeySignatureEvent[] = midi.header.keySignatures
    .map((k) => ({
      time: midi.header.ticksToSeconds(k.ticks),
      key: k.key as string,
      scale: (k.scale as "major" | "minor") || "major",
    }))
    .sort((a, b) => a.time - b.time);

  /* ----- low-level pass for events @tonejs/midi does not expose ----- */
  const raw = midiFile.parseMidi(new Uint8Array(buffer));
  const markers: MarkerEvent[] = [];
  const channelEvents: RawChannelEvent[] = [];
  const textMeta: string[] = [];
  // Per-channel low-level events:
  const lowChannel: {
    programChanges: ProgramChange[];
    channelAftertouch: CCPoint[];
    // CC subset by controllerType:
    cc: Map<number, CCPoint[]>;
  }[] = Array.from({ length: 16 }, () => ({
    programChanges: [],
    channelAftertouch: [],
    cc: new Map(),
  }));

  // RPN state per channel for pitch bend range parsing
  const rpnState: {
    msb: number;
    lsb: number;
    dataMsb?: number;
  }[] = Array.from({ length: 16 }, () => ({ msb: 0x7f, lsb: 0x7f }));
  const bendRangeSemitones: number[] = Array.from(
    { length: 16 },
    () => 2
  );

  for (const trackEvents of raw.tracks) {
    let ticks = 0;
    for (const ev of trackEvents) {
      ticks += ev.deltaTime;
      const time = midi.header.ticksToSeconds(ticks);
      switch (ev.type) {
        case "marker":
          markers.push({ time, text: ev.text, kind: "marker" });
          break;
        case "cuePoint":
          markers.push({ time, text: ev.text, kind: "cuePoint" });
          break;
        case "lyrics":
          markers.push({ time, text: ev.text, kind: "lyrics" });
          break;
        case "text":
        case "copyrightNotice":
          if (ev.text && !textMeta.includes(ev.text)) {
            textMeta.push(ev.text);
          }
          break;
        case "programChange": {
          const ch = ev.channel ?? 0;
          lowChannel[ch].programChanges.push({
            time,
            program: ev.programNumber,
          });
          break;
        }
        case "pitchBend": {
          // midi-file reports -8192..8191; the wire format is 14-bit unsigned.
          const raw = clamp(0, ev.value + 8192, 16383);
          channelEvents.push({
            time,
            channel: ev.channel ?? 0,
            status: 0xe0,
            data1: raw & 0x7f,
            data2: raw >> 7,
          });
          break;
        }
        case "channelAftertouch": {
          const ch = ev.channel ?? 0;
          lowChannel[ch].channelAftertouch.push({
            time,
            value: ev.amount / 127,
          });
          break;
        }
        case "controller": {
          const ch = ev.channel ?? 0;
          const ct = ev.controllerType;
          const v = ev.value;
          channelEvents.push({ time, channel: ch, status: 0xb0, data1: ct, data2: v });
          const arr = lowChannel[ch].cc.get(ct) ?? [];
          arr.push({ time, value: v / 127 });
          lowChannel[ch].cc.set(ct, arr);
          // RPN parsing for bend range
          if (ct === 101) rpnState[ch].msb = v;
          else if (ct === 100) rpnState[ch].lsb = v;
          else if (ct === 6) {
            rpnState[ch].dataMsb = v;
            if (rpnState[ch].msb === 0 && rpnState[ch].lsb === 0) {
              bendRangeSemitones[ch] = v;
            }
          }
          break;
        }
      }
    }
  }
  markers.sort((a, b) => a.time - b.time);
  channelEvents.sort((a, b) => a.time - b.time);

  const sourceTracks = midi.tracks.filter((t) => t.notes.length > 0);
  const notes: NoteEvent[] = [];
  const tracksData: InternalTrackData[] = [];
  const tracks: TrackInfo[] = [];
  let minMidi = 127;
  let maxMidi = 0;

  sourceTracks.forEach((track, index) => {
    const color = TRACK_COLORS[index % TRACK_COLORS.length];
    const channel = track.channel;
    const isPercussion = track.instrument.percussion;
    const program = track.instrument.number ?? 0;

    // Note on programs: we deliberately do NOT use the channel-scoped
    // programChanges here, because orchestral Format-1 files often have
    // multiple "tracks" sharing one channel.
    const lc = lowChannel[channel] ?? {
      programChanges: [],
      channelAftertouch: [],
      cc: new Map(),
    };
    const getCC = (n: number) => (lc.cc.get(n) ?? []).slice();
    const programChanges: ProgramChange[] = [{ time: 0, program }];
    const programs = [program];

    const pitchBends: BendPoint[] = track.pitchBends.map((p) => ({
      time: p.time,
      value: clamp(-1, p.value / 8192, 1),
    }));

    const pedalCC = getCC(64);
    tracksData.push({
      channel,
      programs,
      programChanges,
      volume: getCC(7),
      expression: getCC(11),
      pan: getCC(10),
      pedal: pedalCC,
      reverbSend: getCC(91),
      modulation: getCC(1),
      channelAftertouch: lc.channelAftertouch.slice(),
      pitchBends,
      bendRangeSemitones: bendRangeSemitones[channel] ?? 2,
    });

    const trackNotes: NoteEvent[] = track.notes.map((n) => ({
      trackIndex: index,
      midi: n.midi,
      time: n.time,
      duration: Math.max(0.05, n.duration),
      effectiveDuration: Math.max(0.05, n.duration),
      velocity: clamp(0.05, n.velocity, 1),
      program: programAtTime(programChanges, n.time),
    }));
    applySustainPedal(trackNotes, pedalCC);

    for (const note of trackNotes) {
      notes.push(note);
      if (note.midi < minMidi) minMidi = note.midi;
      if (note.midi > maxMidi) maxMidi = note.midi;
    }

    tracks.push({
      index,
      channel,
      name:
        track.name?.trim() ||
        (isPercussion ? "ドラム" : `トラック ${index + 1}`),
      instrument: isPercussion
        ? "ドラム / パーカッション"
        : prettyInstrumentName(program),
      family: isPercussion ? "ドラム" : instrumentFamily(program),
      isPercussion,
      noteCount: track.notes.length,
      color,
      programs,
      program,
    });
  });

  notes.sort((a, b) => a.time - b.time);

  if (minMidi > maxMidi) {
    minMidi = 48;
    maxMidi = 72;
  }

  let maxNoteDuration = 0;
  for (const n of notes) {
    if (n.duration > maxNoteDuration) maxNoteDuration = n.duration;
  }

  return {
    info: {
      name: fileName,
      duration: midi.duration,
      bpm,
      bpmMin,
      bpmMax,
      minMidi,
      maxMidi,
      maxNoteDuration,
      tracks,
      notes,
      tempos,
      timeSignatures,
      keySignatures,
      markers,
      textMeta,
    },
    tracksData,
    channelEvents,
  };
}

/** Common surface of the Web Audio engine and the native (Rust) engine. */
export interface PlaybackEngine {
  state: PlayerState;
  duration: number;
  onState?: (s: PlayerState) => void;
  onTick?: (position: number) => void;
  onEnd?: () => void;
  onTrackProgramChanged?: (trackIndex: number, program: number) => void;
  load(buffer: ArrayBuffer, fileName: string): Promise<SongInfo>;
  play(): Promise<void>;
  pause(): Promise<void>;
  stop(): Promise<void>;
  seek(time: number): Promise<void>;
  setMasterVolume(value: number): void;
  setTrackMuted(index: number, muted: boolean): void;
  setTrackProgram(trackIndex: number, program: number): Promise<void>;
  position(): number;
  dispose(): void;
}

export class MidiEngine implements PlaybackEngine {
  private ctx: AudioContext | null = null;
  private master: GainNode | null = null;

  // Signal chain per track:
  // voice -> trackInput -> muteGain -> volumeGain -> expressionGain -> panNode
  //         -> dryToMaster
  //         -> reverbSend -> globalReverbInput
  private trackInputs: GainNode[] = [];
  private muteGains: GainNode[] = [];
  private volumeGains: GainNode[] = [];
  private expressionGains: GainNode[] = [];
  private panNodes: StereoPannerNode[] = [];
  private reverbSends: GainNode[] = [];

  // Per-track flag: true if percussion (use DrumSynth instead of soundfont).
  private isPercussionTrack: boolean[] = [];
  // Per-track smplr instance (null for percussion or while (re)loading).
  // Each track owns its instrument so that the audio output can be routed to
  // the track's signal chain; the underlying sample buffers are shared via
  // the SampleLoader so a swapped piano doesn't re-fetch the bank.
  private trackInstruments: (Smplr | null)[] = [];
  // Track override program (mutated by setTrackProgram). For percussion this
  // is left at the original value but unused.
  private trackPrograms: number[] = [];
  private sampleLoader: SampleLoaderT | null = null;
  private drum: DrumSynth | null = null;
  private reverbReturn: GainNode | null = null;

  private notes: NoteEvent[] = [];
  private tracksData: InternalTrackData[] = [];
  private muted: boolean[] = [];

  duration = 0;
  state: PlayerState = "idle";

  private audioStartTime = 0;
  private pausedAt = 0;
  private scheduleCursor = 0;
  private lastEmit = 0;
  private tickTimer: ReturnType<typeof setInterval> | null = null;

  onState?: (s: PlayerState) => void;
  onTick?: (position: number) => void;
  onEnd?: () => void;
  /** トラックの楽器変更が完了したときに呼ばれる。UI 更新用。 */
  onTrackProgramChanged?: (trackIndex: number, program: number) => void;

  private setState(s: PlayerState) {
    this.state = s;
    this.onState?.(s);
  }

  async load(buffer: ArrayBuffer, fileName: string): Promise<SongInfo> {
    this.disposeAudio();
    this.setState("loading");

    const smplr = await import("smplr");
    const sfLib = await import("soundfont2");
    const { info, tracksData } = await parseSong(buffer, fileName);
    this.duration = info.duration;

    /* ----- audio context & buses ----- */
    const ctx = new (window.AudioContext ||
      (window as unknown as { webkitAudioContext: typeof AudioContext })
        .webkitAudioContext)();
    this.ctx = ctx;

    const master = ctx.createGain();
    master.gain.value = 0.9;
    const limiter = ctx.createDynamicsCompressor();
    limiter.threshold.value = -8;
    limiter.knee.value = 8;
    limiter.ratio.value = 12;
    limiter.attack.value = 0.003;
    limiter.release.value = 0.25;
    master.connect(limiter);
    limiter.connect(ctx.destination);
    this.master = master;
    this.drum = new DrumSynth(ctx);

    // Global reverb bus
    const reverb = ctx.createConvolver();
    reverb.buffer = makeImpulseResponse(ctx, 2.2, 2.5);
    const reverbReturn = ctx.createGain();
    reverbReturn.gain.value = 0.85;
    reverb.connect(reverbReturn);
    reverbReturn.connect(master);
    this.reverbReturn = reverbReturn;

    // Shared sample loader — buffers fetched by one track's instrument are
    // reused if another track loads the same instrument (e.g. swapping a
    // second track to the same piano doesn't re-download samples).
    this.sampleLoader = smplr.SampleLoader(ctx, { storage: diskCacheStorage });

    /* ----- per-track signal chain ----- */
    this.notes = info.notes;
    this.tracksData = tracksData;
    this.trackInputs = [];
    this.muteGains = [];
    this.volumeGains = [];
    this.expressionGains = [];
    this.panNodes = [];
    this.reverbSends = [];
    this.isPercussionTrack = [];
    this.trackInstruments = [];
    this.trackPrograms = [];
    this.muted = [];
    const loadJobs: Promise<unknown>[] = [];

    for (const track of info.tracks) {
      const trackInput = ctx.createGain();
      const muteGain = ctx.createGain();
      const volumeGain = ctx.createGain();
      const expressionGain = ctx.createGain();
      const panNode = ctx.createStereoPanner();
      const reverbSend = ctx.createGain();
      reverbSend.gain.value = 0;

      trackInput.connect(muteGain);
      muteGain.connect(volumeGain);
      volumeGain.connect(expressionGain);
      expressionGain.connect(panNode);
      panNode.connect(master);
      panNode.connect(reverbSend);
      reverbSend.connect(reverb);

      this.trackInputs.push(trackInput);
      this.muteGains.push(muteGain);
      this.volumeGains.push(volumeGain);
      this.expressionGains.push(expressionGain);
      this.panNodes.push(panNode);
      this.reverbSends.push(reverbSend);
      this.muted.push(false);
      this.isPercussionTrack.push(track.isPercussion);
      this.trackPrograms.push(track.program);

      let inst: Smplr | null = null;
      if (!track.isPercussion) {
        const built = buildInstrumentFor(
          smplr,
          sfLib,
          ctx,
          track.program,
          trackInput,
          this.sampleLoader!
        );
        inst = built.inst;
        loadJobs.push(built.ready);
      }
      this.trackInstruments.push(inst);
    }

    await Promise.all(loadJobs);

    this.pausedAt = 0;
    this.setState("ready");
    return info;
  }

  async play() {
    if (!this.ctx) return;
    if (this.state === "paused") {
      await this.ctx.resume();
      this.audioStartTime = this.ctx.currentTime - this.pausedAt;
      this.scheduleAutomation(this.pausedAt);
      this.setState("playing");
      this.startLoop();
      return;
    }
    if (this.state !== "ready") return;

    await this.ctx.resume();
    this.audioStartTime = this.ctx.currentTime - this.pausedAt;
    this.resetCursor(this.pausedAt);
    this.scheduleAutomation(this.pausedAt);
    this.setState("playing");
    this.startLoop();
  }

  async pause() {
    if (!this.ctx || this.state !== "playing") return;
    this.pausedAt = this.position();
    this.stopLoop();
    await this.ctx.suspend();
    this.setState("paused");
  }

  async stop() {
    if (!this.ctx) return;
    this.stopLoop();
    this.cancelAll();
    this.pausedAt = 0;
    this.scheduleCursor = 0;
    if (this.ctx.state === "running") await this.ctx.suspend();
    this.setState("ready");
    this.onTick?.(0);
  }

  async seek(time: number) {
    if (!this.ctx) return;
    const target = clamp(0, time, this.duration);
    const wasPlaying = this.state === "playing";
    this.cancelAll();
    this.pausedAt = target;
    this.resetCursor(target);
    if (wasPlaying) {
      this.audioStartTime = this.ctx.currentTime - target;
      this.scheduleAutomation(target);
    }
    this.onTick?.(target);
  }

  setMasterVolume(value: number) {
    if (this.master) this.master.gain.value = clamp(0, value, 1);
  }

  setTrackMuted(index: number, mutedState: boolean) {
    this.muted[index] = mutedState;
    const gain = this.muteGains[index];
    if (gain) gain.gain.value = mutedState ? 0 : 1;
  }

  /**
   * Swap the smplr instrument for a track at runtime. Percussion tracks
   * cannot be changed (use DrumSynth and have no notion of "program").
   * Notes already scheduled within the lookahead window keep their current
   * voices; subsequent notes play through the new instrument.
   */
  async setTrackProgram(trackIndex: number, program: number) {
    const ctx = this.ctx;
    if (!ctx) return;
    if (
      trackIndex < 0 ||
      trackIndex >= this.trackInstruments.length ||
      this.isPercussionTrack[trackIndex]
    ) {
      return;
    }
    const old = this.trackInstruments[trackIndex];
    this.trackInstruments[trackIndex] = null;
    try {
      old?.stop();
    } catch {
      /* ignore */
    }
    try {
      old?.dispose();
    } catch {
      /* ignore */
    }

    const smplr = await import("smplr");
    const sfLib = await import("soundfont2");
    const trackInput = this.trackInputs[trackIndex];
    const loader = this.sampleLoader;
    if (!trackInput || !loader) return;
    const built = buildInstrumentFor(
      smplr,
      sfLib,
      ctx,
      program,
      trackInput,
      loader
    );
    this.trackInstruments[trackIndex] = built.inst;
    this.trackPrograms[trackIndex] = program;
    // Update every note belonging to this track so the piano-roll labelling
    // and scheduleAhead's playNote use the new program.
    for (const n of this.notes) {
      if (n.trackIndex === trackIndex) n.program = program;
    }
    this.tracksData[trackIndex].programs = [program];

    await built.ready;
    this.onTrackProgramChanged?.(trackIndex, program);
  }

  /** Read-only snapshot of the current program for each track. */
  getTrackPrograms(): number[] {
    return this.trackPrograms.slice();
  }

  position(): number {
    if (!this.ctx) return 0;
    if (this.state === "playing") {
      return clamp(
        0,
        this.ctx.currentTime - this.audioStartTime,
        this.duration
      );
    }
    return this.pausedAt;
  }

  dispose() {
    this.stopLoop();
    this.disposeAudio();
  }

  /**
   * Re-anchor all per-track CC automation to the current audio clock.
   * Called on play() and on seek-while-playing so that absolute scheduled
   * times stay aligned with `audioStartTime`.
   */
  private scheduleAutomation(fromTime: number) {
    const ctx = this.ctx;
    if (!ctx) return;
    const baseAudio = this.audioStartTime;
    const epsilon = 0.0001;
    const now = ctx.currentTime;

    const sched = (
      param: AudioParam,
      events: CCPoint[],
      defaultValue: number,
      transform: (v: number) => number
    ) => {
      param.cancelScheduledValues(now);
      let v = defaultValue;
      for (const e of events) {
        if (e.time <= fromTime + epsilon) v = transform(e.value);
        else break;
      }
      param.setValueAtTime(v, Math.max(now, baseAudio + fromTime));
      for (const e of events) {
        if (e.time <= fromTime + epsilon) continue;
        const at = baseAudio + e.time;
        if (at <= now) continue;
        param.setValueAtTime(transform(e.value), at);
      }
    };

    for (let i = 0; i < this.tracksData.length; i++) {
      const t = this.tracksData[i];
      sched(this.volumeGains[i].gain, t.volume, 1, (v) => v);
      sched(this.expressionGains[i].gain, t.expression, 1, (v) => v);
      sched(this.panNodes[i].pan, t.pan, 0, (v) => v * 2 - 1);
      sched(this.reverbSends[i].gain, t.reverbSend, 0, (v) => v * 0.6);
      if (t.channelAftertouch.length > 0) {
        sched(this.trackInputs[i].gain, t.channelAftertouch, 1, (v) =>
          1 + v * 0.25
        );
      }
    }
  }

  private resetCursor(fromTime: number) {
    let i = 0;
    while (i < this.notes.length && this.notes[i].time < fromTime - 0.002) {
      i++;
    }
    this.scheduleCursor = i;
  }

  private playNote(note: NoteEvent, when: number) {
    const ctx = this.ctx;
    if (!ctx) return;
    if (this.muted[note.trackIndex]) return;
    const tdata = this.tracksData[note.trackIndex];

    if (this.isPercussionTrack[note.trackIndex]) {
      if (this.drum) {
        this.drum.trigger(
          note.midi,
          when,
          note.velocity,
          this.trackInputs[note.trackIndex]
        );
      }
      return;
    }

    const inst = this.trackInstruments[note.trackIndex];
    if (!inst) return;

    // Pitch bend value at note start, converted to cents. smplr does not
    // expose the voice's detune AudioParam, so we can't animate bend across
    // the note — but capturing the start-of-note value covers the common
    // case (a bend set before the note plays).
    let detuneCents = 0;
    if (tdata.pitchBends.length > 0) {
      let v = 0;
      for (const b of tdata.pitchBends) {
        if (b.time <= note.time + 0.0001) v = b.value;
        else break;
      }
      detuneCents = v * tdata.bendRangeSemitones * 100;
    }

    try {
      inst.start({
        note: note.midi,
        time: when,
        duration: note.effectiveDuration,
        velocity: Math.round(clamp(1, note.velocity * 127, 127)),
        detune: detuneCents,
      });
    } catch {
      /* ignore — instrument may have been disposed mid-schedule */
    }
  }

  private scheduleAhead(position: number) {
    if (!this.ctx) return;
    const horizon = position + LOOKAHEAD;
    const now = this.ctx.currentTime;
    while (this.scheduleCursor < this.notes.length) {
      const note = this.notes[this.scheduleCursor];
      if (note.time > horizon) break;
      this.playNote(note, Math.max(now, this.audioStartTime + note.time));
      this.scheduleCursor++;
    }
  }

  private cancelAll() {
    for (const inst of this.trackInstruments) {
      try {
        inst?.stop();
      } catch {
        /* ignore */
      }
    }
    this.drum?.cancelAll();
  }

  private startLoop() {
    this.stopLoop();
    this.lastEmit = 0;
    this.tickTimer = setInterval(() => {
      if (this.state !== "playing") return;
      const pos = this.position();
      this.scheduleAhead(pos);
      if (
        pos >= this.duration - 0.01 &&
        this.scheduleCursor >= this.notes.length
      ) {
        this.onTick?.(this.duration);
        this.cancelAll();
        this.pausedAt = 0;
        this.scheduleCursor = 0;
        this.ctx?.suspend();
        this.setState("ready");
        this.onEnd?.();
        return;
      }
      const now = performance.now();
      if (now - this.lastEmit >= 33) {
        this.lastEmit = now;
        this.onTick?.(pos);
      }
    }, 25);
  }

  private stopLoop() {
    if (this.tickTimer !== null) {
      clearInterval(this.tickTimer);
      this.tickTimer = null;
    }
  }

  private disposeAudio() {
    this.cancelAll();
    for (const inst of this.trackInstruments) {
      try {
        inst?.dispose();
      } catch {
        /* ignore */
      }
    }
    if (this.ctx) {
      this.ctx.close().catch(() => undefined);
    }
    this.ctx = null;
    this.master = null;
    this.drum = null;
    this.reverbReturn = null;
    this.trackInputs = [];
    this.muteGains = [];
    this.volumeGains = [];
    this.expressionGains = [];
    this.panNodes = [];
    this.reverbSends = [];
    this.isPercussionTrack = [];
    this.trackInstruments = [];
    this.trackPrograms = [];
    this.sampleLoader = null;
    this.notes = [];
    this.tracksData = [];
    this.muted = [];
  }
}

/* ---------- helpers ---------- */

interface BuiltInstrument {
  inst: Smplr;
  ready: Promise<void>;
}

type Sf2InstrumentOptions = {
  sfLib: Sf2Lib;
  bank: number;
  program: number;
  destination?: AudioNode;
};

// Module-level cache of the factory itself. We can only build it once smplr
// is dynamically imported, but the factory is reused for every track.
let sf2InstrumentFactory:
  | ((ctx: AudioContext, options: Sf2InstrumentOptions) => Smplr & { ready: Promise<void> })
  | null = null;

function getSf2InstrumentFactory(smplr: typeof import("smplr")) {
  if (sf2InstrumentFactory) return sf2InstrumentFactory;
  sf2InstrumentFactory = smplr.Instrument<Sf2InstrumentOptions>(
    (ctx, options, smplrApi) => {
      const ready = (async () => {
        const sf2 = await ensureSf2(options.sfLib);
        const built = buildPresetForProgram(
          ctx,
          sf2,
          options.bank,
          options.program
        );
        if (!built) return;
        await smplrApi.loadInstrument(built.json, built.buffers);
      })();
      return { ready };
    }
  ) as unknown as typeof sf2InstrumentFactory;
  return sf2InstrumentFactory!;
}

function buildInstrumentFor(
  smplr: typeof import("smplr"),
  sfLib: typeof import("soundfont2"),
  ctx: AudioContext,
  program: number,
  destination: AudioNode,
  loader: SampleLoaderT
): BuiltInstrument {
  if (isPianoProgram(program)) {
    const inst = smplr.SplendidGrandPiano(ctx, {
      destination,
      loader,
      storage: diskCacheStorage,
    });
    return { inst, ready: inst.ready.catch(() => undefined) };
  }
  const factory = getSf2InstrumentFactory(smplr);
  const inst = factory(ctx, {
    sfLib,
    bank: 0,
    program,
    destination,
  });
  return { inst, ready: inst.ready.catch(() => undefined) };
}

function programAtTime(
  changes: ProgramChange[],
  time: number
): number {
  let p = changes[0]?.program ?? 0;
  for (const c of changes) {
    if (c.time <= time + 0.0001) p = c.program;
    else break;
  }
  return p;
}

/**
 * Extends a note's `effectiveDuration` whenever the sustain pedal (CC64) is
 * held past the note's natural noteOff.
 */
function applySustainPedal(notes: NoteEvent[], pedal: CCPoint[]) {
  if (pedal.length === 0) return;
  const transitions: { time: number; on: boolean }[] = [];
  let prevOn = false;
  for (const p of pedal) {
    const on = p.value >= 0.5;
    if (on !== prevOn) {
      transitions.push({ time: p.time, on });
      prevOn = on;
    }
  }
  for (const note of notes) {
    const naturalEnd = note.time + note.duration;
    let pedalDown = false;
    let nextRelease = -1;
    for (const tr of transitions) {
      if (tr.time <= naturalEnd + 0.001) {
        pedalDown = tr.on;
      } else {
        if (pedalDown && !tr.on) {
          nextRelease = tr.time;
          break;
        } else if (!pedalDown) {
          break;
        }
      }
    }
    if (pedalDown && nextRelease > naturalEnd) {
      note.effectiveDuration = nextRelease - note.time;
    }
  }
}

function makeImpulseResponse(
  ctx: AudioContext,
  durationSec: number,
  decay: number
): AudioBuffer {
  const rate = ctx.sampleRate;
  const length = Math.floor(rate * durationSec);
  const buf = ctx.createBuffer(2, length, rate);
  for (let ch = 0; ch < 2; ch++) {
    const data = buf.getChannelData(ch);
    for (let i = 0; i < length; i++) {
      const t = i / length;
      data[i] = (Math.random() * 2 - 1) * Math.pow(1 - t, decay);
    }
  }
  return buf;
}

export function keySignatureLabel(k: KeySignatureEvent): string {
  const name = k.key || "C";
  return `${name} ${k.scale === "major" ? "Major" : "Minor"}`;
}

export { KEY_NAMES };
