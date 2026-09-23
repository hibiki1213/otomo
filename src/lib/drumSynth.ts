// Lightweight General MIDI percussion synthesizer built on the Web Audio API.
// Soundfont libraries do not cleanly cover the GM drum map, so percussion
// channels are synthesized here from oscillators and filtered noise.

type DrumKind =
  | "kick"
  | "snare"
  | "rim"
  | "clap"
  | "hatClosed"
  | "hatOpen"
  | "crash"
  | "ride"
  | "tom"
  | "cowbell"
  | "click"
  | "perc";

function classify(note: number): DrumKind {
  if (note === 35 || note === 36) return "kick";
  if (note === 38 || note === 40) return "snare";
  if (note === 37) return "rim";
  if (note === 39) return "clap";
  if (note === 42 || note === 44) return "hatClosed";
  if (note === 46) return "hatOpen";
  if (note === 49 || note === 52 || note === 55 || note === 57) return "crash";
  if (note === 51 || note === 53 || note === 59) return "ride";
  if (note >= 41 && note <= 50) return "tom";
  if (note === 56) return "cowbell";
  if (note === 75 || note === 76 || note === 77) return "click";
  return "perc";
}

export class DrumSynth {
  private ctx: AudioContext;
  private noiseBuffer: AudioBuffer;
  private active = new Set<AudioScheduledSourceNode>();

  constructor(ctx: AudioContext) {
    this.ctx = ctx;
    const length = Math.floor(ctx.sampleRate * 1.2);
    const buffer = ctx.createBuffer(1, length, ctx.sampleRate);
    const data = buffer.getChannelData(0);
    for (let i = 0; i < length; i++) data[i] = Math.random() * 2 - 1;
    this.noiseBuffer = buffer;
  }

  cancelAll() {
    for (const node of this.active) {
      try {
        node.stop();
      } catch {
        /* already stopped */
      }
    }
    this.active.clear();
  }

  trigger(note: number, when: number, velocity: number, destination: AudioNode) {
    const v = Math.max(0.1, Math.min(1, velocity));
    const kind = classify(note);
    switch (kind) {
      case "kick":
        return this.kick(when, v, destination);
      case "snare":
        return this.snare(when, v, destination);
      case "rim":
        return this.rim(when, v, destination);
      case "clap":
        return this.clap(when, v, destination);
      case "hatClosed":
        return this.hat(when, v, destination, 0.045);
      case "hatOpen":
        return this.hat(when, v, destination, 0.34);
      case "crash":
        return this.cymbal(when, v, destination, 1.4, 4500);
      case "ride":
        return this.cymbal(when, v, destination, 0.5, 6500);
      case "tom":
        return this.tom(note, when, v, destination);
      case "cowbell":
        return this.cowbell(when, v, destination);
      case "click":
        return this.hat(when, v, destination, 0.03);
      default:
        return this.perc(when, v, destination);
    }
  }

  private track(node: AudioScheduledSourceNode, when: number, stopAt: number) {
    node.start(when);
    node.stop(stopAt);
    this.active.add(node);
    node.onended = () => this.active.delete(node);
  }

  private envelope(when: number, duration: number, peak: number): GainNode {
    const g = this.ctx.createGain();
    g.gain.setValueAtTime(0.0001, when);
    g.gain.linearRampToValueAtTime(peak, when + 0.003);
    g.gain.exponentialRampToValueAtTime(0.0001, when + duration);
    return g;
  }

  private noise(): AudioBufferSourceNode {
    const src = this.ctx.createBufferSource();
    src.buffer = this.noiseBuffer;
    src.loop = true;
    return src;
  }

  private kick(when: number, v: number, dest: AudioNode) {
    const osc = this.ctx.createOscillator();
    osc.type = "sine";
    osc.frequency.setValueAtTime(155, when);
    osc.frequency.exponentialRampToValueAtTime(47, when + 0.12);
    const g = this.envelope(when, 0.36, 0.95 * v);
    osc.connect(g).connect(dest);
    this.track(osc, when, when + 0.4);
  }

  private snare(when: number, v: number, dest: AudioNode) {
    const tone = this.ctx.createOscillator();
    tone.type = "triangle";
    tone.frequency.setValueAtTime(186, when);
    const g1 = this.envelope(when, 0.16, 0.45 * v);
    tone.connect(g1).connect(dest);
    this.track(tone, when, when + 0.2);

    const n = this.noise();
    const hp = this.ctx.createBiquadFilter();
    hp.type = "highpass";
    hp.frequency.value = 1400;
    const g2 = this.envelope(when, 0.2, 0.7 * v);
    n.connect(hp).connect(g2).connect(dest);
    this.track(n, when, when + 0.24);
  }

  private rim(when: number, v: number, dest: AudioNode) {
    const n = this.noise();
    const bp = this.ctx.createBiquadFilter();
    bp.type = "bandpass";
    bp.frequency.value = 1700;
    bp.Q.value = 6;
    const g = this.envelope(when, 0.06, 0.6 * v);
    n.connect(bp).connect(g).connect(dest);
    this.track(n, when, when + 0.09);
  }

  private clap(when: number, v: number, dest: AudioNode) {
    const bp = this.ctx.createBiquadFilter();
    bp.type = "bandpass";
    bp.frequency.value = 1500;
    bp.Q.value = 2.5;
    bp.connect(dest);
    for (let i = 0; i < 3; i++) {
      const t = when + i * 0.012;
      const n = this.noise();
      const g = this.envelope(t, 0.09, 0.5 * v);
      n.connect(g).connect(bp);
      this.track(n, t, t + 0.12);
    }
  }

  private hat(when: number, v: number, dest: AudioNode, duration: number) {
    const n = this.noise();
    const hp = this.ctx.createBiquadFilter();
    hp.type = "highpass";
    hp.frequency.value = 7200;
    const g = this.envelope(when, duration, 0.4 * v);
    n.connect(hp).connect(g).connect(dest);
    this.track(n, when, when + duration + 0.03);
  }

  private cymbal(
    when: number,
    v: number,
    dest: AudioNode,
    duration: number,
    cutoff: number
  ) {
    const n = this.noise();
    const hp = this.ctx.createBiquadFilter();
    hp.type = "highpass";
    hp.frequency.value = cutoff;
    const g = this.envelope(when, duration, 0.42 * v);
    n.connect(hp).connect(g).connect(dest);
    this.track(n, when, when + duration + 0.05);
  }

  private tom(note: number, when: number, v: number, dest: AudioNode) {
    const base = 92 * Math.pow(2, (50 - note) / 14);
    const osc = this.ctx.createOscillator();
    osc.type = "sine";
    osc.frequency.setValueAtTime(base * 1.5, when);
    osc.frequency.exponentialRampToValueAtTime(base, when + 0.18);
    const g = this.envelope(when, 0.42, 0.75 * v);
    osc.connect(g).connect(dest);
    this.track(osc, when, when + 0.46);
  }

  private cowbell(when: number, v: number, dest: AudioNode) {
    const g = this.envelope(when, 0.3, 0.35 * v);
    g.connect(dest);
    for (const freq of [540, 845]) {
      const osc = this.ctx.createOscillator();
      osc.type = "square";
      osc.frequency.value = freq;
      osc.connect(g);
      this.track(osc, when, when + 0.34);
    }
  }

  private perc(when: number, v: number, dest: AudioNode) {
    const n = this.noise();
    const bp = this.ctx.createBiquadFilter();
    bp.type = "bandpass";
    bp.frequency.value = 2200;
    bp.Q.value = 1.5;
    const g = this.envelope(when, 0.16, 0.45 * v);
    n.connect(bp).connect(g).connect(dest);
    this.track(n, when, when + 0.2);
  }
}
