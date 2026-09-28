//! A drum machine in the 909/808 tradition, laid out on the General MIDI
//! percussion map so any drum track plays on it as written.

use std::f32::consts::TAU;

use crate::dsp::{soft_clip, FilterMode, Rng, Svf};

#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    Kick,
    Snare,
    Clap,
    Rim,
    ClosedHat,
    PedalHat,
    OpenHat,
    /// Base frequency.
    Tom(f32),
    Crash,
    Ride,
    RideBell,
    Shaker,
    Cowbell,
}

/// Six detuned square waves: the 808/909 way of making a cymbal's metal.
const METAL: [f32; 6] = [205.3, 304.4, 369.6, 522.7, 540.0, 800.0];

fn kind_for(key: u8) -> Option<Kind> {
    Some(match key {
        35 | 36 => Kind::Kick,
        37 => Kind::Rim,
        38 | 40 => Kind::Snare,
        39 => Kind::Clap,
        42 => Kind::ClosedHat,
        44 => Kind::PedalHat,
        46 => Kind::OpenHat,
        41 => Kind::Tom(78.0),
        43 => Kind::Tom(92.0),
        45 => Kind::Tom(108.0),
        47 => Kind::Tom(126.0),
        48 => Kind::Tom(146.0),
        50 => Kind::Tom(170.0),
        60..=64 => Kind::Tom(220.0 + f32::from(key - 60) * 45.0),
        49 | 52 | 55 | 57 => Kind::Crash,
        51 | 59 => Kind::Ride,
        53 => Kind::RideBell,
        54 | 69 | 70 | 82 => Kind::Shaker,
        56 => Kind::Cowbell,
        _ => return None,
    })
}

/// Where each sound sits, -1 (left) … 1 (right), as seen from the audience.
fn position(kind: Kind) -> f32 {
    match kind {
        Kind::ClosedHat | Kind::PedalHat | Kind::OpenHat => 0.3,
        Kind::Tom(f) => (0.5 - (f - 78.0) / 92.0).clamp(-0.6, 0.6),
        Kind::Crash => -0.4,
        Kind::Ride | Kind::RideBell => 0.45,
        Kind::Shaker => 0.25,
        Kind::Cowbell => -0.2,
        _ => 0.0,
    }
}

/// How long a hit can ring before it is dropped, in seconds.
fn length(kind: Kind) -> f32 {
    match kind {
        Kind::Kick => 1.0,
        Kind::Snare | Kind::Clap => 0.7,
        Kind::Rim | Kind::ClosedHat | Kind::PedalHat | Kind::Shaker => 0.3,
        Kind::OpenHat => 1.4,
        Kind::Tom(_) => 1.1,
        Kind::Cowbell => 0.6,
        Kind::Crash => 3.5,
        Kind::Ride | Kind::RideBell => 2.5,
    }
}

pub struct Hit {
    kind: Kind,
    age: usize,
    velocity: f32,
    left: f32,
    right: f32,
    phase: f32,
    phase2: f32,
    metal: [f32; 6],
    rng: Rng,
    f1: Svf,
    f2: Svf,
    /// Set when a closed hat cuts an open one off.
    choke: f32,
    end: usize,
}

impl Hit {
    pub fn new(key: u8, velocity: u8, serial: u64, sample_rate: f32) -> Option<Self> {
        let kind = kind_for(key)?;
        let p = position(kind);
        let a = (p + 1.0) * std::f32::consts::FRAC_PI_4;
        let mut hit = Self {
            kind,
            age: 0,
            velocity: {
                let v = f32::from(velocity) / 127.0;
                0.25 + 0.75 * v * v
            },
            left: a.cos() * std::f32::consts::SQRT_2,
            right: a.sin() * std::f32::consts::SQRT_2,
            phase: 0.0,
            phase2: 0.0,
            metal: [0.0; 6],
            rng: Rng::new(0x1234_5678 ^ (serial as u32).wrapping_mul(2_246_822_519)),
            f1: Svf::default(),
            f2: Svf::default(),
            choke: 1.0,
            end: (length(kind) * sample_rate) as usize,
        };
        let sr = sample_rate;
        match kind {
            Kind::Snare => {
                hit.f1.set(1_800.0, 0.1, sr);
                hit.f2.set(9_000.0, 0.0, sr);
            }
            Kind::Clap => hit.f1.set(1_150.0, 0.45, sr),
            Kind::Rim => hit.f1.set(2_500.0, 0.2, sr),
            Kind::ClosedHat | Kind::PedalHat | Kind::OpenHat => {
                hit.f1.set(7_500.0, 0.2, sr);
                hit.f2.set(10_500.0, 0.3, sr);
            }
            Kind::Crash => {
                hit.f1.set(4_500.0, 0.1, sr);
                hit.f2.set(8_000.0, 0.1, sr);
            }
            Kind::Ride | Kind::RideBell => {
                hit.f1.set(5_000.0, 0.25, sr);
                hit.f2.set(9_000.0, 0.1, sr);
            }
            Kind::Shaker => hit.f1.set(6_500.0, 0.1, sr),
            Kind::Cowbell => hit.f1.set(820.0, 0.55, sr),
            _ => {}
        }
        for (m, r) in hit.metal.iter_mut().zip([0.13, 0.71, 0.29, 0.53, 0.97, 0.41]) {
            *m = r;
        }
        Some(hit)
    }

    pub fn is_open_hat(&self) -> bool {
        self.kind == Kind::OpenHat
    }

    pub fn is_hat(&self) -> bool {
        matches!(self.kind, Kind::ClosedHat | Kind::PedalHat)
    }

    /// Cut this hit off quickly (an open hat closed by the pedal).
    pub fn choke(&mut self) {
        if self.choke >= 1.0 {
            self.choke = 0.999;
        }
    }

    pub fn done(&self) -> bool {
        self.age >= self.end || self.choke < 1.0e-4
    }

    fn metal(&mut self, sr: f32, scale: f32) -> f32 {
        let mut sum = 0.0;
        for (phase, f) in self.metal.iter_mut().zip(METAL) {
            *phase = (*phase + f * scale / sr).fract();
            sum += if *phase < 0.5 { 1.0 } else { -1.0 };
        }
        sum / 6.0
    }

    #[inline]
    fn next(&mut self, sr: f32) -> f32 {
        let t = self.age as f32 / sr;
        self.age += 1;
        let v = match self.kind {
            Kind::Kick => {
                // Pitch dives from ~160 Hz to ~48 Hz: the "thump".
                let f = 48.0 + 115.0 * (-t / 0.03).exp();
                self.phase = (self.phase + f / sr).fract();
                let body = (self.phase * TAU).sin() * (-t / 0.3).exp();
                let click = self.rng.bipolar() * (-t / 0.0015).exp() * 0.35;
                soft_clip((body + click) * 1.8) * 0.8
            }
            Kind::Snare => {
                let f = 185.0 * (1.0 + 0.35 * (-t / 0.008).exp());
                self.phase = (self.phase + f / sr).fract();
                self.phase2 = (self.phase2 + 330.0 / sr).fract();
                let tone = ((self.phase * TAU).sin() * 0.65 + (self.phase2 * TAU).sin() * 0.35) * (-t / 0.045).exp();
                let n = self.rng.bipolar();
                let noise = self.f2.process(self.f1.process(n, FilterMode::HighPass), FilterMode::LowPass) * (-t / 0.13).exp();
                tone * 0.75 + noise * 1.1
            }
            Kind::Clap => {
                // Three quick slaps, then the room.
                let mut env = 0.0f32;
                for k in 0..3 {
                    let s = t - k as f32 * 0.011;
                    if s >= 0.0 {
                        env = env.max((-s / 0.005).exp());
                    }
                }
                if t > 0.033 {
                    env = env.max(0.7 * (-(t - 0.033) / 0.15).exp());
                }
                self.f1.process(self.rng.bipolar(), FilterMode::BandPass) * env * 2.6
            }
            Kind::Rim => {
                self.phase = (self.phase + 1_700.0 / sr).fract();
                let tone = (self.phase * TAU).sin() * (-t / 0.012).exp();
                let click = self.f1.process(self.rng.bipolar(), FilterMode::HighPass) * (-t / 0.004).exp();
                tone * 0.6 + click * 0.6
            }
            Kind::ClosedHat | Kind::PedalHat | Kind::OpenHat => {
                let decay = match self.kind {
                    Kind::ClosedHat => 0.04,
                    Kind::PedalHat => 0.028,
                    _ => 0.32,
                };
                let m = self.metal(sr, 1.0) * 0.6 + self.rng.bipolar() * 0.4;
                let y = self.f2.process(self.f1.process(m, FilterMode::HighPass), FilterMode::BandPass);
                y * (-t / decay).exp() * 1.6
            }
            Kind::Tom(base) => {
                let f = base * (1.0 + 0.45 * (-t / 0.03).exp());
                self.phase = (self.phase + f / sr).fract();
                let decay = if base > 200.0 { 0.12 } else { 0.22 };
                let body = (self.phase * TAU).sin() * (-t / decay).exp();
                let skin = self.rng.bipolar() * (-t / 0.01).exp() * 0.15;
                soft_clip((body + skin) * 1.3) * 0.75
            }
            Kind::Crash => {
                let m = self.metal(sr, 1.45) * 0.45 + self.rng.bipolar() * 0.55;
                let y = self.f2.process(self.f1.process(m, FilterMode::HighPass), FilterMode::LowPass);
                y * ((-t / 0.8).exp() * 0.85 + (-t / 0.05).exp() * 0.3) * 1.2
            }
            Kind::Ride | Kind::RideBell => {
                let m = self.metal(sr, 1.2) * 0.7 + self.rng.bipolar() * 0.2;
                let mut y = self.f1.process(m, FilterMode::BandPass) * (-t / 0.7).exp();
                if self.kind == Kind::RideBell {
                    self.phase = (self.phase + 2_450.0 / sr).fract();
                    y += (self.phase * TAU).sin() * (-t / 0.45).exp() * 0.35;
                }
                y * 1.1
            }
            Kind::Shaker => self.f1.process(self.rng.bipolar(), FilterMode::HighPass) * (-t / 0.045).exp() * 0.9,
            Kind::Cowbell => {
                self.phase = (self.phase + 540.0 / sr).fract();
                self.phase2 = (self.phase2 + 800.0 / sr).fract();
                let sq = |p: f32| if p < 0.5 { 1.0 } else { -1.0 };
                let raw = (sq(self.phase) + sq(self.phase2)) * 0.5;
                self.f1.process(raw, FilterMode::BandPass) * ((-t / 0.02).exp() * 0.6 + (-t / 0.15).exp() * 0.4) * 1.3
            }
        };
        if self.choke < 1.0 {
            self.choke *= 0.995;
        }
        v * self.velocity * self.choke
    }

    pub fn render(&mut self, sample_rate: f32, left: &mut [f32], right: &mut [f32]) {
        for (l, r) in left.iter_mut().zip(right.iter_mut()) {
            if self.done() {
                return;
            }
            let v = self.next(sample_rate);
            *l += v * self.left;
            *r += v * self.right;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_mapped_key_sounds_and_ends() {
        let sr = 48_000.0;
        for key in 0..128u8 {
            let Some(mut hit) = Hit::new(key, 110, 1, sr) else { continue };
            let (mut l, mut r) = (vec![0.0; 48_000 * 4], vec![0.0; 48_000 * 4]);
            hit.render(sr, &mut l, &mut r);
            let peak = l.iter().chain(&r).fold(0.0f32, |a, v| a.max(v.abs()));
            assert!(peak > 0.05 && peak < 2.5, "key {key} peak {peak}");
            assert!(hit.done(), "key {key} rings forever");
        }
    }
}
