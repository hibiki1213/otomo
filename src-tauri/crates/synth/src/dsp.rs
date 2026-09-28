//! Building blocks: band-limited oscillators, a zero-delay-feedback filter,
//! envelopes, and the small delay line the effects share.

use std::f32::consts::{PI, TAU};

/// Keeps a value from sinking into denormals, which are slow on the CPU.
const TINY: f32 = 1.0e-18;

/// xorshift: cheap noise and random start phases, same sequence every run.
#[derive(Clone)]
pub struct Rng(u32);

impl Rng {
    pub fn new(seed: u32) -> Self {
        Self(seed.max(1))
    }

    pub fn next_u32(&mut self) -> u32 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.0 = x;
        x
    }

    /// 0 … 1
    pub fn unit(&mut self) -> f32 {
        (self.next_u32() >> 8) as f32 / (1u32 << 24) as f32
    }

    /// -1 … 1
    pub fn bipolar(&mut self) -> f32 {
        self.unit() * 2.0 - 1.0
    }
}

/// Soft saturation that stays close to linear for small signals.
#[inline]
pub fn soft_clip(x: f32) -> f32 {
    let x = x.clamp(-3.0, 3.0);
    x * (27.0 + x * x) / (27.0 + 9.0 * x * x)
}

/// Transparent below 0.8, then bends smoothly toward ±1: a safety net for
/// the output, not a sound.
#[inline]
pub fn limit(x: f32) -> f32 {
    let a = x.abs();
    if a <= 0.8 {
        x
    } else {
        let over = (a - 0.8) / 0.2;
        (0.8 + 0.2 * (over / (1.0 + over))).copysign(x)
    }
}

#[inline]
pub fn semitones_to_ratio(st: f32) -> f32 {
    (st * (std::f32::consts::LN_2 / 12.0)).exp()
}

#[inline]
pub fn midi_to_hz(key: f32) -> f32 {
    440.0 * semitones_to_ratio(key - 69.0)
}

#[inline]
pub fn db_to_gain(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// Polynomial correction around a discontinuity; removes most aliasing from
/// saw and pulse waves for the price of a couple of multiplies.
#[inline]
fn poly_blep(t: f32, dt: f32) -> f32 {
    if t < dt {
        let t = t / dt;
        t + t - t * t - 1.0
    } else if t > 1.0 - dt {
        let t = (t - 1.0) / dt;
        t * t + t + t + 1.0
    } else {
        0.0
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Wave {
    Sine,
    Triangle,
    Saw,
    /// Pulse with this duty cycle (0.5 = square).
    Pulse(f32),
    Noise,
}

/// One sample of `wave` at `phase` (0…1) advancing by `dt` per sample.
#[inline]
pub fn wave_sample(wave: Wave, phase: f32, dt: f32, rng: &mut Rng) -> f32 {
    match wave {
        Wave::Sine => (phase * TAU).sin(),
        Wave::Triangle => {
            let t = phase * 4.0;
            if t < 1.0 {
                t
            } else if t < 3.0 {
                2.0 - t
            } else {
                t - 4.0
            }
        }
        Wave::Saw => 2.0 * phase - 1.0 - poly_blep(phase, dt),
        Wave::Pulse(width) => {
            let w = width.clamp(0.05, 0.95);
            let naive = if phase < w { 1.0 } else { -1.0 };
            let mut falling = phase - w;
            if falling < 0.0 {
                falling += 1.0;
            }
            naive + poly_blep(phase, dt) - poly_blep(falling, dt)
        }
        Wave::Noise => rng.bipolar(),
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FilterMode {
    LowPass,
    BandPass,
    HighPass,
}

/// State-variable filter in the topology-preserving (zero-delay feedback)
/// form: stable under fast cutoff sweeps and at high resonance, which is
/// what acid lines and wobbles do all the time.
#[derive(Clone, Default)]
pub struct Svf {
    s1: f32,
    s2: f32,
    g: f32,
    k: f32,
    a1: f32,
}

impl Svf {
    /// `resonance` 0 … 1 (self-oscillation just short of 1).
    pub fn set(&mut self, cutoff: f32, resonance: f32, sample_rate: f32) {
        let fc = cutoff.clamp(20.0, sample_rate * 0.45);
        self.g = (PI * fc / sample_rate).tan();
        self.k = 2.0 - 1.96 * resonance.clamp(0.0, 1.0);
        self.a1 = 1.0 / (1.0 + self.g * (self.g + self.k));
    }

    #[inline]
    pub fn process(&mut self, x: f32, mode: FilterMode) -> f32 {
        let hp = (x - (self.k + self.g) * self.s1 - self.s2) * self.a1;
        let bp = self.g * hp + self.s1;
        self.s1 = self.g * hp + bp + TINY;
        let lp = self.g * bp + self.s2;
        self.s2 = self.g * bp + lp + TINY;
        match mode {
            FilterMode::LowPass => lp,
            FilterMode::BandPass => bp,
            FilterMode::HighPass => hp,
        }
    }

    pub fn reset(&mut self) {
        self.s1 = 0.0;
        self.s2 = 0.0;
    }
}

/// Attack, decay, sustain, release. Times in seconds; the attack is linear
/// (it reads as a fade-in), decay and release are exponential (they read as
/// natural). `Adsr::done` says when a released note has faded out.
#[derive(Clone, Copy, Debug)]
pub struct AdsrSpec {
    pub attack: f32,
    pub decay: f32,
    pub sustain: f32,
    pub release: f32,
}

impl AdsrSpec {
    pub const fn new(attack: f32, decay: f32, sustain: f32, release: f32) -> Self {
        Self { attack, decay, sustain, release }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Stage {
    Attack,
    Decay,
    Release,
    Idle,
}

#[derive(Clone)]
pub struct Adsr {
    stage: Stage,
    value: f32,
    attack_step: f32,
    decay_coef: f32,
    release_coef: f32,
    sustain: f32,
}

/// One-pole coefficient that covers ~99% of the way in `seconds`.
fn coef(seconds: f32, sample_rate: f32) -> f32 {
    1.0 - (-4.6 / (seconds.max(0.0005) * sample_rate)).exp()
}

impl Adsr {
    pub fn new(spec: AdsrSpec, sample_rate: f32) -> Self {
        Self {
            stage: Stage::Idle,
            value: 0.0,
            attack_step: 1.0 / (spec.attack.max(0.0005) * sample_rate),
            decay_coef: coef(spec.decay, sample_rate),
            release_coef: coef(spec.release, sample_rate),
            sustain: spec.sustain,
        }
    }

    /// Start (or restart from where it is, so a retrigger never clicks).
    pub fn trigger(&mut self) {
        self.stage = Stage::Attack;
    }

    pub fn release(&mut self) {
        if self.stage != Stage::Idle {
            self.stage = Stage::Release;
        }
    }

    pub fn is_released(&self) -> bool {
        matches!(self.stage, Stage::Release | Stage::Idle)
    }

    pub fn done(&self) -> bool {
        self.stage == Stage::Idle
    }

    pub fn value(&self) -> f32 {
        self.value
    }

    #[inline]
    pub fn next(&mut self) -> f32 {
        match self.stage {
            Stage::Attack => {
                self.value += self.attack_step;
                if self.value >= 1.0 {
                    self.value = 1.0;
                    self.stage = Stage::Decay;
                }
            }
            Stage::Decay => {
                self.value += (self.sustain - self.value) * self.decay_coef;
                // A zero-sustain envelope (plucks, drums) ends by itself.
                if self.sustain <= 0.0 && self.value < 1.0e-4 {
                    self.value = 0.0;
                    self.stage = Stage::Idle;
                }
            }
            Stage::Release => {
                self.value -= self.value * self.release_coef;
                if self.value < 1.0e-4 {
                    self.value = 0.0;
                    self.stage = Stage::Idle;
                }
            }
            Stage::Idle => {}
        }
        self.value
    }
}

/// A fractional delay line, for chorus and echo.
pub struct DelayLine {
    buf: Vec<f32>,
    write: usize,
}

impl DelayLine {
    pub fn new(max_samples: usize) -> Self {
        Self { buf: vec![0.0; max_samples.max(4)], write: 0 }
    }

    #[inline]
    pub fn push(&mut self, x: f32) {
        self.buf[self.write] = x;
        self.write = (self.write + 1) % self.buf.len();
    }

    /// The sample written `delay` samples ago (linear interpolation).
    #[inline]
    pub fn read(&self, delay: f32) -> f32 {
        let len = self.buf.len();
        let d = delay.clamp(1.0, (len - 2) as f32);
        let pos = self.write as f32 - d + len as f32;
        let i = pos as usize;
        let frac = pos - i as f32;
        let a = self.buf[i % len];
        let b = self.buf[(i + 1) % len];
        a + (b - a) * frac
    }

    pub fn clear(&mut self) {
        self.buf.fill(0.0);
    }
}

/// A one-pole low-pass, for darkening echoes and smoothing controls.
#[derive(Clone, Default)]
pub struct OnePole {
    z: f32,
    a: f32,
}

impl OnePole {
    pub fn new(cutoff: f32, sample_rate: f32) -> Self {
        Self { z: 0.0, a: 1.0 - (-TAU * cutoff / sample_rate).exp() }
    }

    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        self.z += (x - self.z) * self.a + TINY;
        self.z
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saw_is_bounded_and_centered() {
        let mut rng = Rng::new(1);
        let (mut phase, dt) = (0.0f32, 440.0 / 48_000.0);
        let (mut sum, mut peak) = (0.0f32, 0.0f32);
        for _ in 0..48_000 {
            let v = wave_sample(Wave::Saw, phase, dt, &mut rng);
            sum += v;
            peak = peak.max(v.abs());
            phase = (phase + dt).fract();
        }
        assert!(peak <= 1.05, "peak {peak}");
        assert!((sum / 48_000.0).abs() < 0.01);
    }

    #[test]
    fn svf_low_pass_passes_lows_and_cuts_highs() {
        let sr = 48_000.0;
        let run = |hz: f32| {
            let mut f = Svf::default();
            f.set(1_000.0, 0.0, sr);
            let mut peak = 0.0f32;
            for n in 0..48_000 {
                let y = f.process((n as f32 * TAU * hz / sr).sin(), FilterMode::LowPass);
                if n > 24_000 {
                    peak = peak.max(y.abs());
                }
            }
            peak
        };
        assert!(run(100.0) > 0.95);
        assert!(run(10_000.0) < 0.02);
    }

    #[test]
    fn envelope_rises_holds_and_fades() {
        let sr = 48_000.0;
        let mut e = Adsr::new(AdsrSpec::new(0.01, 0.1, 0.5, 0.1), sr);
        e.trigger();
        for _ in 0..(sr as usize / 2) {
            e.next();
        }
        assert!((e.value() - 0.5).abs() < 0.01);
        e.release();
        for _ in 0..(sr as usize / 2) {
            e.next();
        }
        assert!(e.done());
    }
}
