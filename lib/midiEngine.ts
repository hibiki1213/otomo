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

interface InternalTrackData {
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

interface ScheduledVoice {
  trackIndex: number;
  note: NoteEvent;
  audioNode: AudioNode | null;
  source: AudioBufferSourceNode | null;
  stopAt: number;
}

export class MidiEngine {
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
  // Shared Players by program number — one Player per unique program across
  // the whole file. Notes are re-routed per-track via Web Audio node connects.
  private programPlayers = new Map<number, SoundfontPlayer | null>();
  private programDummyDest: GainNode | null = null;
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

  private setState(s: PlayerState) {
    this.state = s;
    this.onState?.(s);
  }

  async load(buffer: ArrayBuffer, fileName: string): Promise<SongInfo> {
    this.disposeAudio();
    this.setState("loading");

    const { Midi } = await import("@tonejs/midi");
    const midiFile = await import("midi-file");
    const sfModule = await import("soundfont-player");
    const Soundfont = sfModule.default ?? sfModule;

    const midi = new Midi(buffer);
    this.duration = midi.duration;

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
            const arr = lowChannel[ch].cc.get(ct) ?? [];
            arr.push({ time, value: v / 127 });
            lowChannel[ch].cc.set(ct, arr);
            // RPN parsing for bend range
            if (ct === 101) rpnState[ch].msb = v;
            else if (ct === 100) rpnState[ch].lsb = v;
            else if (ct === 6) {
              rpnState[ch].dataMsb = v;
              // RPN 0 = pitch bend range, MSB = semitones
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

    /* ----- per-track signal chain ----- */
    const sourceTracks = midi.tracks.filter((t) => t.notes.length > 0);

    this.notes = [];
    this.trackInputs = [];
    this.muteGains = [];
    this.volumeGains = [];
    this.expressionGains = [];
    this.panNodes = [];
    this.reverbSends = [];
    this.isPercussionTrack = [];
    this.programPlayers = new Map();
    this.tracksData = [];
    this.muted = [];

    const tracks: TrackInfo[] = [];
    let minMidi = 127;
    let maxMidi = 0;
    const loadJobs: Promise<void>[] = [];

    sourceTracks.forEach((track, index) => {
      const color = TRACK_COLORS[index % TRACK_COLORS.length];
      const channel = track.channel;
      const isPercussion = track.instrument.percussion;
      const program = track.instrument.number ?? 0;

      // Build per-track CC arrays from low-level pass (channel-scoped).
      // Note on programs: we deliberately do NOT use the channel-scoped
      // programChanges here, because orchestral Format-1 files often have
      // multiple "tracks" sharing one channel (one with notes + several
      // metadata-only label tracks), and channel-scoped programs would
      // bleed across them — giving wrong instruments. @tonejs/midi's
      // `track.instrument.number` is already derived from the FIRST
      // programChange in this raw track's own events, so it's reliable.
      const lc = lowChannel[channel] ?? {
        programChanges: [],
        channelAftertouch: [],
        cc: new Map(),
      };
      const getCC = (n: number) => (lc.cc.get(n) ?? []).slice();
      const programChanges: ProgramChange[] = [{ time: 0, program }];
      const programs = [program];

      // Build pitch bend timeline. @tonejs/midi exposes the raw 14-bit
      // value (-8192..8191) from midi-file. Normalize to -1..1.
      const pitchBends: BendPoint[] = track.pitchBends.map((p) => ({
        time: p.time,
        value: clamp(-1, p.value / 8192, 1),
      }));

      // Audio chain
      const trackInput = ctx.createGain();
      trackInput.gain.value = 1;
      const muteGain = ctx.createGain();
      muteGain.gain.value = 1;
      const volumeGain = ctx.createGain();
      volumeGain.gain.value = 1;
      const expressionGain = ctx.createGain();
      expressionGain.gain.value = 1;
      const panNode = ctx.createStereoPanner();
      panNode.pan.value = 0;
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

      // Collect CC timelines (audio scheduling happens in scheduleAutomation)
      const volumeCC = getCC(7);
      const expressionCC = getCC(11);
      const panCC = getCC(10);
      const pedalCC = getCC(64);
      const reverbCC = getCC(91);
      const modulationCC = getCC(1);
      const aftertouchCC = lc.channelAftertouch.slice();

      // Instrument
      const name =
        track.name?.trim() ||
        (isPercussion ? "ドラム" : `トラック ${index + 1}`);
      const instrument = isPercussion
        ? "ドラム / パーカッション"
        : prettyInstrumentName(program);

      const data: InternalTrackData = {
        channel,
        programs,
        programChanges,
        volume: volumeCC,
        expression: expressionCC,
        pan: panCC,
        pedal: pedalCC,
        reverbSend: reverbCC,
        modulation: modulationCC,
        channelAftertouch: aftertouchCC,
        pitchBends,
        bendRangeSemitones: bendRangeSemitones[channel] ?? 2,
      };
      this.tracksData.push(data);

      // Notes
      const trackNotes: NoteEvent[] = track.notes.map((n) => ({
        trackIndex: index,
        midi: n.midi,
        time: n.time,
        duration: Math.max(0.05, n.duration),
        effectiveDuration: Math.max(0.05, n.duration),
        velocity: clamp(0.05, n.velocity, 1),
        program: programAtTime(programChanges, n.time),
      }));
      // Apply sustain pedal extension
      applySustainPedal(trackNotes, pedalCC);

      for (const note of trackNotes) {
        this.notes.push(note);
        if (note.midi < minMidi) minMidi = note.midi;
        if (note.midi > maxMidi) maxMidi = note.midi;
      }

      this.isPercussionTrack.push(isPercussion);
      // Programs are loaded ONCE per unique program after this loop, not
      // per track — see the dedup pass below.

      tracks.push({
        index,
        channel,
        name,
        instrument,
        family: isPercussion ? "ドラム" : instrumentFamily(program),
        isPercussion,
        noteCount: track.notes.length,
        color,
        programs,
      });
    });

    this.notes.sort((a, b) => a.time - b.time);

    /* Deduplicate programs across all tracks and load each soundfont ONCE.
     * For a large orchestral file with 50+ tracks we go from 50+ soundfont
     * loads (each independently decoding the same MP3 bank) down to ~10
     * unique programs — the difference between hanging and loading. */
    const dummy = ctx.createGain();
    dummy.gain.value = 0;
    // Don't connect dummy to anything: it acts as a sink that goes nowhere
    // until we re-route per-note nodes to the per-track signal chain.
    this.programDummyDest = dummy;

    const uniquePrograms = new Set<number>();
    for (let i = 0; i < this.tracksData.length; i++) {
      if (this.isPercussionTrack[i]) continue;
      for (const p of this.tracksData[i].programs) {
        uniquePrograms.add(p);
      }
    }

    for (const prog of uniquePrograms) {
      this.programPlayers.set(prog, null);
      loadJobs.push(
        Soundfont.instrument(ctx, soundfontNameForProgram(prog), {
          soundfont: "MusyngKite",
          format: "mp3",
          destination: dummy,
        })
          .then((player) => {
            this.programPlayers.set(prog, player);
          })
          .catch(() =>
            Soundfont.instrument(ctx, "acoustic_grand_piano", {
              destination: dummy,
            })
              .then((player) => {
                this.programPlayers.set(prog, player);
              })
              .catch(() => undefined)
          )
      );
    }

    await Promise.all(loadJobs);

    if (minMidi > maxMidi) {
      minMidi = 48;
      maxMidi = 72;
    }

    let maxNoteDuration = 0;
    for (const n of this.notes) {
      if (n.duration > maxNoteDuration) maxNoteDuration = n.duration;
    }

    this.pausedAt = 0;
    this.setState("ready");

    return {
      name: fileName,
      duration: this.duration,
      bpm,
      bpmMin,
      bpmMax,
      minMidi,
      maxMidi,
      maxNoteDuration,
      tracks,
      notes: this.notes,
      tempos,
      timeSignatures,
      keySignatures,
      markers,
      textMeta,
    };
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
      // Find current value at fromTime
      let v = defaultValue;
      for (const e of events) {
        if (e.time <= fromTime + epsilon) v = transform(e.value);
        else break;
      }
      param.setValueAtTime(v, Math.max(now, baseAudio + fromTime));
      // Schedule remaining events after fromTime
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
      // Percussion: trigger the drum synth through trackInput
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

    const player = this.programPlayers.get(note.program) ?? null;
    if (!player) return;

    const duration = note.effectiveDuration;
    const node = player.play(midiToNoteName(note.midi), when, {
      duration,
      gain: note.velocity,
    }) as unknown as
      | (AudioNode & {
          source?: AudioBufferSourceNode;
          stop?: (when: number) => void;
        })
      | undefined;

    if (!node?.source) return;

    // Re-route the voice from the shared Player's output to this track's
    // input. Each note's gain node is created fresh by soundfont-player,
    // so disconnecting it doesn't affect other simultaneously-playing notes.
    try {
      node.disconnect();
    } catch {
      /* ignore */
    }
    node.connect(this.trackInputs[note.trackIndex]);

    // Apply pitch bend automation across the note's lifetime
    if (tdata.pitchBends.length > 0) {
      applyBendToVoice(
        node.source.detune,
        tdata.pitchBends,
        note.time,
        note.time + duration,
        this.audioStartTime,
        tdata.bendRangeSemitones
      );
    }

    // Modulation (CC1) → simple LFO vibrato attached to this voice
    const modulationActive = anyCCActive(
      tdata.modulation,
      note.time,
      note.time + duration
    );
    if (modulationActive) {
      attachVibrato(
        ctx,
        node.source,
        tdata.modulation,
        note.time,
        note.time + duration,
        this.audioStartTime,
        when
      );
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
    for (const player of this.programPlayers.values()) {
      try {
        player?.stop();
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
    this.programPlayers = new Map();
    this.programDummyDest = null;
    this.notes = [];
    this.tracksData = [];
    this.muted = [];
  }
}

/* ---------- helpers ---------- */

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
 * held past the note's natural noteOff. The note is released at the first
 * pedal-down→up transition that occurs at or after the note's natural end.
 */
function applySustainPedal(notes: NoteEvent[], pedal: CCPoint[]) {
  if (pedal.length === 0) return;
  // Convert to on/off transitions
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
    // Was the pedal held down at the natural noteOff time?
    let pedalDown = false;
    let nextRelease = -1;
    for (const tr of transitions) {
      if (tr.time <= naturalEnd + 0.001) {
        pedalDown = tr.on;
      } else {
        // Look for first pedal-off transition after naturalEnd
        if (pedalDown && !tr.on) {
          nextRelease = tr.time;
          break;
        } else if (!pedalDown) {
          // pedal currently up — nothing more to do
          break;
        }
      }
    }
    if (pedalDown && nextRelease > naturalEnd) {
      note.effectiveDuration = nextRelease - note.time;
    }
  }
}

function anyCCActive(events: CCPoint[], start: number, end: number): boolean {
  if (events.length === 0) return false;
  // Find value at start
  let v = events[0].value;
  for (const e of events) {
    if (e.time <= start) v = e.value;
    else break;
  }
  if (v > 0.001) return true;
  // Any nonzero within window?
  for (const e of events) {
    if (e.time > start && e.time < end && e.value > 0.001) return true;
  }
  return false;
}

function applyBendToVoice(
  detuneParam: AudioParam,
  bends: BendPoint[],
  noteStart: number,
  noteEnd: number,
  audioStartTime: number,
  rangeSemitones: number
) {
  // Find bend value at noteStart
  let initial = 0;
  let firstIdx = bends.length;
  for (let i = 0; i < bends.length; i++) {
    if (bends[i].time <= noteStart + 0.0001) {
      initial = bends[i].value;
      firstIdx = i + 1;
    } else {
      break;
    }
  }
  const cents0 = initial * rangeSemitones * 100;
  detuneParam.setValueAtTime(cents0, audioStartTime + noteStart);
  for (let i = firstIdx; i < bends.length; i++) {
    const b = bends[i];
    if (b.time >= noteEnd) break;
    detuneParam.linearRampToValueAtTime(
      b.value * rangeSemitones * 100,
      audioStartTime + b.time
    );
  }
}

function attachVibrato(
  ctx: AudioContext,
  source: AudioBufferSourceNode,
  modulation: CCPoint[],
  noteStart: number,
  noteEnd: number,
  audioStartTime: number,
  when: number
) {
  const lfo = ctx.createOscillator();
  lfo.frequency.value = 5.5;
  const depth = ctx.createGain();
  depth.gain.value = 0;
  lfo.connect(depth);
  depth.connect(source.detune);
  lfo.start(when);
  lfo.stop(audioStartTime + noteEnd + 0.05);

  // Automate depth (cents) from CC1
  let initial = 0;
  let firstIdx = modulation.length;
  for (let i = 0; i < modulation.length; i++) {
    if (modulation[i].time <= noteStart + 0.0001) {
      initial = modulation[i].value;
      firstIdx = i + 1;
    } else {
      break;
    }
  }
  // Max vibrato depth: ~50 cents at full modulation
  const MAX_CENTS = 50;
  depth.gain.setValueAtTime(initial * MAX_CENTS, audioStartTime + noteStart);
  for (let i = firstIdx; i < modulation.length; i++) {
    const m = modulation[i];
    if (m.time >= noteEnd) break;
    depth.gain.linearRampToValueAtTime(
      m.value * MAX_CENTS,
      audioStartTime + m.time
    );
  }
}

function midiToNoteName(midi: number): string {
  const names = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
  const octave = Math.floor(midi / 12) - 1;
  return `${names[midi % 12]}${octave}`;
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

// Re-export key-name helper for UI
export function keySignatureLabel(k: KeySignatureEvent): string {
  // @tonejs/midi already returns key as a sharp/flat name like "F" or "C#"
  const name = k.key || "C";
  return `${name} ${k.scale === "major" ? "Major" : "Minor"}`;
}

export { KEY_NAMES };
