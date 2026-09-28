//! One sounding note of a tonal preset.

use std::f32::consts::{FRAC_PI_4, TAU};

use crate::dsp::{midi_to_hz, semitones_to_ratio, soft_clip, wave_sample, Adsr, Rng, Svf, Wave};
use crate::presets::{LfoTarget, Patch};

pub const MAX_UNISON: usize = 7;
/// Filters and pitch are recomputed every this many samples; envelopes and
/// oscillators run every sample.
pub const CONTROL: usize = 16;

/// What the whole instrument knows that every voice needs.
pub struct Controls {
    pub sample_rate: f32,
    /// Pitch bend in semitones.
    pub bend: f32,
    /// Mod wheel, 0 … 1.
    pub wheel: f32,
    /// CC74, in octaves of extra cutoff.
    pub brightness: f32,
}

#[derive(Clone, Copy, Default)]
struct Unison {
    phase: f32,
    dt: f32,
    left: f32,
    right: f32,
    /// Detune of this copy, in semitones.
    offset: f32,
}

/// Equal-power pan position `p` (-1 … 1) → (left, right), unity in the middle.
fn pan(p: f32) -> (f32, f32) {
    let a = (p.clamp(-1.0, 1.0) + 1.0) * FRAC_PI_4;
    (a.cos() * std::f32::consts::SQRT_2, a.sin() * std::f32::consts::SQRT_2)
}

fn spread_unison(voices: usize, detune_cents: f32, spread: f32, rng: &mut Rng) -> ([Unison; MAX_UNISON], usize) {
    let n = voices.clamp(1, MAX_UNISON);
    let mut out = [Unison::default(); MAX_UNISON];
    for (i, u) in out.iter_mut().take(n).enumerate() {
        // -1 … 1 across the stack; a lone voice sits in the middle.
        let pos = if n == 1 { 0.0 } else { i as f32 / (n - 1) as f32 * 2.0 - 1.0 };
        u.offset = pos * detune_cents / 100.0;
        let (l, r) = pan(pos * spread);
        u.left = l;
        u.right = r;
        // Free-running start phases: what makes a supersaw sound wide
        // instead of flanging on every attack.
        u.phase = if n == 1 { 0.0 } else { rng.unit() };
    }
    (out, n)
}

pub struct Voice {
    pub key: u8,
    /// Order of arrival, for stealing the oldest note.
    pub serial: u64,
    /// The key is held (or the sustain pedal is keeping it).
    pub held: bool,
    pub sustained: bool,
    velocity: f32,
    /// Current pitch in MIDI keys (glides toward `key`).
    pitch: f32,
    glide_coef: f32,
    osc1: [Unison; MAX_UNISON],
    n1: usize,
    osc2: [Unison; MAX_UNISON],
    n2: usize,
    sub_phase: f32,
    fm_phase: f32,
    amp: Adsr,
    fenv: Adsr,
    filters: [[Svf; 2]; 2],
    age: usize,
    lfo_phase: f32,
    rng: Rng,
    /// Gains for this note, recomputed at control rate.
    level: f32,
    fm_index: f32,
    lfo_amp: f32,
}

impl Voice {
    pub fn new(patch: &Patch, key: u8, velocity: u8, serial: u64, sample_rate: f32) -> Self {
        let mut rng = Rng::new(0x9E37_79B9 ^ (serial as u32).wrapping_mul(2_654_435_761) ^ u32::from(key));
        let (osc1, n1) = spread_unison(patch.osc1.unison, patch.osc1.detune, patch.osc1.spread, &mut rng);
        let (osc2, n2) = spread_unison(patch.osc2.unison, patch.osc2.detune, patch.osc2.spread, &mut rng);
        let mut voice = Self {
            key,
            serial,
            held: true,
            sustained: false,
            velocity: f32::from(velocity) / 127.0,
            pitch: f32::from(key),
            glide_coef: if patch.glide > 0.0 {
                1.0 - (-4.6 / (patch.glide * sample_rate / CONTROL as f32)).exp()
            } else {
                1.0
            },
            osc1,
            n1,
            osc2,
            n2,
            sub_phase: 0.0,
            fm_phase: 0.0,
            amp: Adsr::new(patch.amp, sample_rate),
            fenv: Adsr::new(patch.filter.env, sample_rate),
            filters: Default::default(),
            age: 0,
            lfo_phase: 0.0,
            rng,
            level: 0.0,
            fm_index: 0.0,
            lfo_amp: 1.0,
        };
        voice.amp.trigger();
        voice.fenv.trigger();
        voice
    }

    /// Mono legato: move to a new key without restarting the envelopes.
    pub fn glide_to(&mut self, key: u8, velocity: u8) {
        self.key = key;
        self.velocity = self.velocity.max(f32::from(velocity) / 127.0 * 0.9);
        self.held = true;
        self.sustained = false;
    }

    /// Mono retrigger: a new note after the previous one was let go.
    /// Glide only happens between overlapping notes (legato), as on a 303.
    pub fn retrigger(&mut self, key: u8, velocity: u8, serial: u64) {
        self.key = key;
        self.pitch = f32::from(key);
        self.velocity = f32::from(velocity) / 127.0;
        self.serial = serial;
        self.held = true;
        self.sustained = false;
        self.age = 0;
        self.amp.trigger();
        self.fenv.trigger();
    }

    pub fn release(&mut self) {
        self.held = false;
        self.sustained = false;
        self.amp.release();
        self.fenv.release();
    }

    pub fn is_releasing(&self) -> bool {
        self.amp.is_released()
    }

    pub fn done(&self) -> bool {
        self.amp.done()
    }

    fn update(&mut self, patch: &Patch, c: &Controls) {
        let sr = c.sample_rate;
        let t = self.age as f32 / sr;
        self.pitch += (f32::from(self.key) - self.pitch) * self.glide_coef;

        // LFO, faded in after its delay.
        let lfo = &patch.lfo;
        let mut lfo_value = 0.0;
        if lfo.target != LfoTarget::None {
            let fade = if lfo.delay > 0.0 { ((t - lfo.delay) / 0.3).clamp(0.0, 1.0) } else { 1.0 };
            let depth = lfo.depth * fade + lfo.wheel * c.wheel;
            let shape = match lfo.wave {
                Wave::Pulse(_) => {
                    if self.lfo_phase < 0.5 {
                        1.0
                    } else {
                        -1.0
                    }
                }
                Wave::Triangle => 1.0 - 4.0 * (self.lfo_phase - 0.5).abs(),
                _ => (self.lfo_phase * TAU).sin(),
            };
            lfo_value = shape * depth;
            self.lfo_phase = (self.lfo_phase + lfo.rate * CONTROL as f32 / sr).fract();
        }

        let (sweep, settle) = patch.pitch_env;
        let pitch_env = if sweep != 0.0 { sweep * (-t * 4.6 / settle.max(0.001)).exp() } else { 0.0 };
        let vibrato = if lfo.target == LfoTarget::Pitch { lfo_value } else { 0.0 };
        let base = self.pitch + c.bend + pitch_env + vibrato;

        let set = |units: &mut [Unison], n: usize, semis: f32, cents: f32| {
            for u in units.iter_mut().take(n) {
                u.dt = (midi_to_hz(base + semis + cents / 100.0 + u.offset) / sr).min(0.49);
            }
        };
        set(&mut self.osc1, self.n1, patch.osc1.semitones, patch.osc1.cents);
        set(&mut self.osc2, self.n2, patch.osc2.semitones, patch.osc2.cents);

        let f = &patch.filter;
        let cutoff_lfo = if lfo.target == LfoTarget::Cutoff { lfo_value } else { 0.0 };
        let env_depth = f.env_octaves * (1.0 - f.velocity + f.velocity * self.velocity * 1.4);
        let octaves = self.fenv.value() * env_depth
            + cutoff_lfo
            + f.key_track * (self.pitch - 60.0) / 12.0
            + c.brightness;
        let cutoff = f.cutoff * semitones_to_ratio(octaves * 12.0);
        for side in &mut self.filters {
            for stage in side.iter_mut() {
                stage.set(cutoff, f.resonance, sr);
            }
        }

        self.fm_index = match patch.fm {
            Some((_, index, fall)) => index * (-t * 4.6 / fall.max(0.01)).exp() * (0.4 + 0.6 * self.velocity),
            None => 0.0,
        };
        self.lfo_amp = if lfo.target == LfoTarget::Amp { 1.0 - (lfo_value * 0.5 + 0.5).clamp(0.0, 1.0) * lfo.depth.min(1.0) } else { 1.0 };
        let vel = 1.0 - patch.velocity + patch.velocity * self.velocity * self.velocity;
        // Stacks are normalised so adding unison widens rather than just
        // getting louder.
        self.level = vel;
    }

    /// Add this voice's next `left.len()` samples into the buffers.
    pub fn render(&mut self, patch: &Patch, c: &Controls, left: &mut [f32], right: &mut [f32]) {
        let g1 = patch.osc1.level / (self.n1 as f32).sqrt();
        let g2 = patch.osc2.level / (self.n2 as f32).sqrt();
        let drive = patch.fx.drive.max(1.0);
        let drive_norm = 1.0 / soft_clip(drive).max(0.1);
        let fm_ratio = patch.fm.map_or(0.0, |(r, _, _)| r);
        let steep = patch.filter.steep;
        let mode = patch.filter.mode;

        for i in 0..left.len() {
            if self.age % CONTROL == 0 {
                self.update(patch, c);
            }
            self.age += 1;
            self.fenv.next();
            let amp = self.amp.next();

            let (mut l, mut r) = (0.0, 0.0);
            if patch.fm.is_some() {
                // Carrier: osc1's first voice, phase-modulated by a sine.
                let u = &mut self.osc1[0];
                let m = (self.fm_phase * TAU).sin() * self.fm_index;
                let v = (u.phase * TAU + m).sin() * g1;
                l += v;
                r += v;
                self.fm_phase = (self.fm_phase + u.dt * fm_ratio).fract();
                u.phase = (u.phase + u.dt).fract();
            } else if patch.osc1.level > 0.0 {
                for u in self.osc1.iter_mut().take(self.n1) {
                    let v = wave_sample(patch.osc1.wave, u.phase, u.dt, &mut self.rng) * g1;
                    l += v * u.left;
                    r += v * u.right;
                    u.phase += u.dt;
                    if u.phase >= 1.0 {
                        u.phase -= 1.0;
                    }
                }
            }
            if patch.osc2.level > 0.0 {
                for u in self.osc2.iter_mut().take(self.n2) {
                    let v = wave_sample(patch.osc2.wave, u.phase, u.dt, &mut self.rng) * g2;
                    l += v * u.left;
                    r += v * u.right;
                    u.phase += u.dt;
                    if u.phase >= 1.0 {
                        u.phase -= 1.0;
                    }
                }
            }
            if patch.sub > 0.0 {
                let v = (self.sub_phase * TAU).sin() * patch.sub;
                l += v;
                r += v;
                self.sub_phase = (self.sub_phase + self.osc1[0].dt * 0.5).fract();
            }
            if patch.noise > 0.0 {
                let v = self.rng.bipolar() * patch.noise;
                l += v;
                r += v;
            }

            if drive > 1.0 {
                l = soft_clip(l * drive) * drive_norm;
                r = soft_clip(r * drive) * drive_norm;
            }
            let [fl, fr] = &mut self.filters;
            l = fl[0].process(l, mode);
            r = fr[0].process(r, mode);
            if steep {
                l = fl[1].process(l, mode);
                r = fr[1].process(r, mode);
            }

            let g = amp * self.level * self.lfo_amp;
            left[i] += l * g;
            right[i] += r * g;
        }
    }
}
