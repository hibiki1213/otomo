import type { SoundfontPlayer } from "soundfont-player";
import { DrumSynth } from "./drumSynth";
import {
  soundfontNameForProgram,
  prettyInstrumentName,
  instrumentFamily,
} from "./gm";

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
  duration: number;
  velocity: number;
}

export interface TrackInfo {
  index: number;
  name: string;
  instrument: string;
  family: string;
  isPercussion: boolean;
  noteCount: number;
  color: string;
}

export interface SongInfo {
  name: string;
  duration: number;
  bpm: number;
  minMidi: number;
  maxMidi: number;
  tracks: TrackInfo[];
  notes: NoteEvent[];
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

const clamp = (lo: number, v: number, hi: number) =>
  Math.max(lo, Math.min(hi, v));

// Seconds of audio scheduled ahead of the playhead by the just-in-time
// scheduler. Large enough to absorb timer jitter, small enough to avoid
// committing thousands of audio nodes to the clock at once.
const LOOKAHEAD = 0.3;

export class MidiEngine {
  private ctx: AudioContext | null = null;
  private master: GainNode | null = null;
  private trackGains: GainNode[] = [];
  private instruments: (SoundfontPlayer | null)[] = [];
  private drum: DrumSynth | null = null;

  private notes: NoteEvent[] = [];
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

  private setState(s: PlayerState) {
    this.state = s;
    this.onState?.(s);
  }

  /** Parse a MIDI file and load every required instrument sound. */
  async load(buffer: ArrayBuffer, fileName: string): Promise<SongInfo> {
    this.disposeAudio();
    this.setState("loading");

    const { Midi } = await import("@tonejs/midi");
    const sfModule = await import("soundfont-player");
    const Soundfont = sfModule.default ?? sfModule;

    const midi = new Midi(buffer);
    this.duration = midi.duration;
    const bpm = Math.round(midi.header.tempos[0]?.bpm ?? 120);

    const ctx = new (window.AudioContext ||
      (window as unknown as { webkitAudioContext: typeof AudioContext })
        .webkitAudioContext)();
    this.ctx = ctx;
    const master = ctx.createGain();
    master.gain.value = 0.9;
    // A gentle limiter keeps dense, many-voice passages from clipping
    // harshly when lots of notes sound at once.
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

    const sourceTracks = midi.tracks.filter((t) => t.notes.length > 0);

    this.notes = [];
    this.trackGains = [];
    this.instruments = [];
    this.muted = [];
    const tracks: TrackInfo[] = [];
    let minMidi = 127;
    let maxMidi = 0;

    const loadJobs: Promise<void>[] = [];

    sourceTracks.forEach((track, index) => {
      const color = TRACK_COLORS[index % TRACK_COLORS.length];
      const isPercussion = track.instrument.percussion;
      const program = track.instrument.number ?? 0;

      const gain = ctx.createGain();
      gain.gain.value = 1;
      gain.connect(master);
      this.trackGains.push(gain);
      this.muted.push(false);

      const instrument = isPercussion
        ? "ドラム / パーカッション"
        : prettyInstrumentName(program);
      const name =
        track.name?.trim() ||
        (isPercussion ? "ドラム" : `トラック ${index + 1}`);

      for (const note of track.notes) {
        this.notes.push({
          trackIndex: index,
          midi: note.midi,
          time: note.time,
          duration: Math.max(0.05, note.duration),
          velocity: clamp(0.05, note.velocity, 1),
        });
        if (note.midi < minMidi) minMidi = note.midi;
        if (note.midi > maxMidi) maxMidi = note.midi;
      }

      tracks.push({
        index,
        name,
        instrument,
        family: isPercussion ? "ドラム" : instrumentFamily(program),
        isPercussion,
        noteCount: track.notes.length,
        color,
      });

      if (isPercussion) {
        this.instruments.push(null);
      } else {
        const slot = this.instruments.length;
        this.instruments.push(null);
        loadJobs.push(
          Soundfont.instrument(ctx, soundfontNameForProgram(program), {
            soundfont: "MusyngKite",
            format: "mp3",
            destination: gain,
          })
            .then((player) => {
              this.instruments[slot] = player;
            })
            .catch(() => {
              // Fall back silently to a piano if a sound fails to load.
              return Soundfont.instrument(ctx, "acoustic_grand_piano", {
                destination: gain,
              })
                .then((player) => {
                  this.instruments[slot] = player;
                })
                .catch(() => undefined);
            })
        );
      }
    });

    this.notes.sort((a, b) => a.time - b.time);
    await Promise.all(loadJobs);

    if (minMidi > maxMidi) {
      minMidi = 48;
      maxMidi = 72;
    }

    this.pausedAt = 0;
    this.setState("ready");

    return {
      name: fileName,
      duration: this.duration,
      bpm,
      minMidi,
      maxMidi,
      tracks,
      notes: this.notes,
    };
  }

  async play() {
    if (!this.ctx) return;
    if (this.state === "paused") {
      await this.ctx.resume();
      this.audioStartTime = this.ctx.currentTime - this.pausedAt;
      this.setState("playing");
      this.startLoop();
      return;
    }
    if (this.state !== "ready") return;

    await this.ctx.resume();
    this.audioStartTime = this.ctx.currentTime - this.pausedAt;
    this.resetCursor(this.pausedAt);
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
    }
    this.onTick?.(target);
  }

  setMasterVolume(value: number) {
    if (this.master) this.master.gain.value = clamp(0, value, 1);
  }

  setTrackMuted(index: number, mutedState: boolean) {
    this.muted[index] = mutedState;
    const gain = this.trackGains[index];
    if (gain) gain.gain.value = mutedState ? 0 : 1;
  }

  position(): number {
    if (!this.ctx) return 0;
    if (this.state === "playing") {
      return clamp(0, this.ctx.currentTime - this.audioStartTime, this.duration);
    }
    return this.pausedAt;
  }

  dispose() {
    this.stopLoop();
    this.disposeAudio();
  }

  // Move the scheduling cursor to the first note at or after `fromTime`.
  // Notes are kept sorted by start time, so this is a simple scan.
  private resetCursor(fromTime: number) {
    let i = 0;
    while (i < this.notes.length && this.notes[i].time < fromTime - 0.002) {
      i++;
    }
    this.scheduleCursor = i;
  }

  private playNote(note: NoteEvent, when: number) {
    const instrument = this.instruments[note.trackIndex];
    if (instrument) {
      instrument.play(note.midi, when, {
        duration: note.duration,
        gain: note.velocity,
      });
    } else if (this.drum) {
      this.drum.trigger(
        note.midi,
        when,
        note.velocity,
        this.trackGains[note.trackIndex]
      );
    }
  }

  // Just-in-time scheduling: only commit notes that fall inside the
  // look-ahead window to the Web Audio clock. This prevents a large MIDI
  // file from creating tens of thousands of audio nodes synchronously
  // (which froze the page) while keeping note timing sample-accurate.
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
    for (const instrument of this.instruments) {
      try {
        instrument?.stop();
      } catch {
        /* ignore */
      }
    }
    this.drum?.cancelAll();
  }

  // A setInterval-based clock is used instead of requestAnimationFrame so that
  // playback position and end-of-song detection keep working in background
  // tabs, where rAF is throttled or paused entirely.
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
      // Throttle UI updates to ~30 fps; the scheduler itself runs faster.
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
    if (this.ctx) {
      this.ctx.close().catch(() => undefined);
    }
    this.ctx = null;
    this.master = null;
    this.drum = null;
    this.trackGains = [];
    this.instruments = [];
    this.notes = [];
    this.muted = [];
  }
}
