import { invoke } from "@tauri-apps/api/core";
import { isTauri } from "./assetCache";
import {
  parseSong,
  type NoteEvent,
  type PlaybackEngine,
  type PlayerState,
  type RawChannelEvent,
  type SongInfo,
} from "./midiEngine";

interface EngineStatus {
  position: number;
  playing: boolean;
  ended: boolean;
}

const clamp = (lo: number, v: number, hi: number) =>
  Math.max(lo, Math.min(hi, v));

/** Status poll interval; the playhead is extrapolated in between. */
const POLL_MS = 30;

/**
 * Pack a song for `engine_load_song` (layout documented at `decode_song` in
 * engine.rs). Binary because dense files carry hundreds of thousands of
 * controller events.
 */
function encodeSong(info: SongInfo, events: RawChannelEvent[]): Uint8Array {
  const { tracks, notes } = info;
  const size = 20 + tracks.length * 2 + notes.length * 20 + events.length * 12;
  const buf = new ArrayBuffer(size);
  const v = new DataView(buf);
  let o = 0;
  v.setFloat64(o, info.duration, true);
  v.setUint32(o + 8, tracks.length, true);
  v.setUint32(o + 12, notes.length, true);
  v.setUint32(o + 16, events.length, true);
  o += 20;
  for (const t of tracks) {
    v.setUint8(o, t.channel);
    v.setUint8(o + 1, t.program);
    o += 2;
  }
  for (const n of notes) {
    v.setFloat64(o, n.time, true);
    // Original length: the synth applies the sustain pedal itself.
    v.setFloat64(o + 8, n.time + n.duration, true);
    v.setUint16(o + 16, n.trackIndex, true);
    v.setUint8(o + 18, n.midi);
    v.setUint8(o + 19, Math.round(clamp(1, n.velocity * 127, 127)));
    o += 20;
  }
  for (const e of events) {
    v.setFloat64(o, e.time, true);
    v.setUint8(o + 8, e.channel);
    v.setUint8(o + 9, e.status);
    v.setUint8(o + 10, e.data1);
    v.setUint8(o + 11, e.data2);
    o += 12;
  }
  return new Uint8Array(buf);
}

let soundfontReady: Promise<void> | null = null;

/**
 * Load the bundled SoundFont into the Rust synth, once. A failure is
 * forgotten, so the next song tries again.
 */
function ensureSoundfont(): Promise<void> {
  if (!soundfontReady) {
    soundfontReady = invoke<void>("engine_load_soundfont").catch((e) => {
      soundfontReady = null;
      throw e;
    });
  }
  return soundfontReady;
}

/**
 * Frontend handle for the native engine (src-tauri/src/engine.rs). Parsing
 * stays in TypeScript so track indices match the UI; synthesis, sequencing
 * and audio output all happen in Rust.
 */
export class RustEngine implements PlaybackEngine {
  static available(): boolean {
    return isTauri();
  }

  state: PlayerState = "idle";
  duration = 0;

  onState?: (s: PlayerState) => void;
  onTick?: (position: number) => void;
  onEnd?: () => void;
  onTrackProgramChanged?: (trackIndex: number, program: number) => void;
  onSoundsMissing?: () => void;

  private notes: NoteEvent[] = [];
  private lastPos = 0;
  private lastAt = 0;
  private timer: ReturnType<typeof setInterval> | null = null;
  private polling = false;

  private setState(s: PlayerState) {
    this.state = s;
    this.onState?.(s);
  }

  async load(buffer: ArrayBuffer, fileName: string): Promise<SongInfo> {
    this.stopPolling();
    this.setState("loading");
    await invoke("engine_stop").catch(() => undefined);

    const { info, channelEvents } = await parseSong(buffer, fileName);
    // Without the SoundFont the song still opens: it can be read, edited and
    // played through any sample-library instruments, and the rest is silent.
    const sounds = await ensureSoundfont().then(
      () => true,
      (e) => {
        console.warn("default SoundFont unavailable", e);
        return false;
      }
    );
    await invoke("engine_load_song", encodeSong(info, channelEvents));

    this.duration = info.duration;
    this.notes = info.notes;
    this.lastPos = 0;
    this.setState("ready");
    if (!sounds) this.onSoundsMissing?.();
    return info;
  }

  async play() {
    if (this.state !== "ready" && this.state !== "paused") return;
    await invoke("engine_play");
    this.lastAt = performance.now();
    this.setState("playing");
    this.startPolling();
  }

  async pause() {
    if (this.state !== "playing") return;
    this.stopPolling();
    await invoke("engine_pause");
    await this.poll();
    this.setState("paused");
  }

  async stop() {
    this.stopPolling();
    await invoke("engine_stop");
    this.lastPos = 0;
    this.setState("ready");
    this.onTick?.(0);
  }

  async seek(time: number) {
    const target = clamp(0, time, this.duration);
    await invoke("engine_seek", { seconds: target });
    this.lastPos = target;
    this.lastAt = performance.now();
    this.onTick?.(target);
  }

  setMasterVolume(value: number) {
    void invoke("engine_set_master_volume", { value: clamp(0, value, 1) });
  }

  setTrackMuted(index: number, muted: boolean) {
    void invoke("engine_set_track_muted", { track: index, muted });
  }

  setTrackGain(index: number, gain: number) {
    void invoke("engine_set_track_gain", { track: index, gain });
  }

  async setTrackProgram(trackIndex: number, program: number) {
    await invoke("engine_set_track_program", { track: trackIndex, program });
    for (const n of this.notes) {
      if (n.trackIndex === trackIndex) n.program = program;
    }
    this.onTrackProgramChanged?.(trackIndex, program);
  }

  position(): number {
    if (this.state !== "playing") return this.lastPos;
    const elapsed = (performance.now() - this.lastAt) / 1000;
    return clamp(0, this.lastPos + elapsed, this.duration);
  }

  dispose() {
    this.stopPolling();
    void invoke("engine_stop").catch(() => undefined);
  }

  private async poll() {
    const st = await invoke<EngineStatus>("engine_status");
    this.lastPos = st.position;
    this.lastAt = performance.now();
    return st;
  }

  private startPolling() {
    this.stopPolling();
    this.timer = setInterval(async () => {
      if (this.polling || this.state !== "playing") return;
      this.polling = true;
      try {
        const st = await this.poll();
        if (this.state !== "playing") return;
        if (st.ended) {
          this.stopPolling();
          this.onTick?.(this.duration);
          await invoke("engine_seek", { seconds: 0 });
          this.lastPos = 0;
          this.setState("ready");
          this.onEnd?.();
          return;
        }
        this.onTick?.(st.position);
      } finally {
        this.polling = false;
      }
    }, POLL_MS);
  }

  private stopPolling() {
    if (this.timer !== null) {
      clearInterval(this.timer);
      this.timer = null;
    }
  }
}

/* ---------- spatial (venue reverb / binaural) ---------- */

export type OutputMode = "speakers" | "headphones";

export interface SpatialConfig {
  /** "original" (the MIDI's own reverb), "dry", or a venue id. */
  room: string;
  /** 0 = front, 1 = middle, 2 = back. */
  seat: number;
  output: OutputMode;
}

export interface VenueInfo {
  id: string;
  name: string;
  /** Mid-frequency reverberation time in seconds. */
  rt60: number;
}

export function spatialOptions(): Promise<{
  venues: VenueInfo[];
  current: SpatialConfig;
}> {
  return invoke("engine_spatial_options");
}

/** Resolves once the room response is built and handed to the audio thread. */
export function setSpatial(config: SpatialConfig): Promise<void> {
  return invoke("engine_set_spatial", { config });
}
