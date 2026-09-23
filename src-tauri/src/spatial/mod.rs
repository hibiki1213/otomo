//! Spatial stage between the synthesizer and the audio device: venue reverb
//! and binaural (headphone) rendering.
//!
//! Signal flow per block, by room × output:
//!
//! | room      | speakers                    | headphones                                   |
//! |-----------|-----------------------------|----------------------------------------------|
//! | original  | synth mix (its own reverb)  | synth mix on virtual speakers at ±30° (HRTF) |
//! | dry/venue | dry mix + venue reverb (mic)| each track placed on stage by its pan        |
//! |           |                             | (HRTF) + venue reverb (binaural)             |

pub mod dsp;
pub mod hrtf;
pub mod room;

use std::sync::Arc;

use realfft::num_complex::Complex32;
use serde::{Deserialize, Serialize};

use dsp::{complex_mac, Fft, MatrixConvolver};
use hrtf::Hrtf;
use room::Geometry;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputMode {
    Speakers,
    Headphones,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct SpatialConfig {
    /// `"original"` (the MIDI's own reverb/chorus), `"dry"`, or a venue id.
    pub room: String,
    /// 0 = front, 1 = middle, 2 = back.
    pub seat: usize,
    pub output: OutputMode,
}

impl Default for SpatialConfig {
    fn default() -> Self {
        Self { room: "original".into(), seat: 1, output: OutputMode::Speakers }
    }
}

impl SpatialConfig {
    pub fn is_original(&self) -> bool {
        self.room == "original"
    }
}

/// What the instruments produced for one engine block.
pub struct TrackBuffers {
    /// Full stereo mix with MIDI pan applied (and, in "original", the
    /// synth's own reverb).
    pub mix: [Vec<f32>; 2],
    /// Per-track dry output, not yet panned.
    pub tracks: Vec<[Vec<f32>; 2]>,
    pub active: Vec<bool>,
    /// Per-track pan, -1 … 1.
    pub pan: Vec<f32>,
    /// Number of tracks in use.
    pub count: usize,
    /// SoundFont chorus (and reverb, in "original") return.
    pub fx: [Vec<f32>; 2],
}

impl TrackBuffers {
    pub fn new(block: usize, max_tracks: usize) -> Self {
        Self {
            mix: [vec![0.0; block], vec![0.0; block]],
            tracks: vec![[vec![0.0; block], vec![0.0; block]]; max_tracks],
            active: vec![false; max_tracks],
            pan: vec![0.0; max_tracks],
            count: 0,
            fx: [vec![0.0; block], vec![0.0; block]],
        }
    }

    pub fn block(&self) -> usize {
        self.mix[0].len()
    }

    pub fn clear(&mut self) {
        for b in self.mix.iter_mut().chain(self.fx.iter_mut()) {
            b.fill(0.0);
        }
        for (tr, active) in self.tracks.iter_mut().zip(self.active.iter_mut()) {
            if *active {
                tr[0].fill(0.0);
                tr[1].fill(0.0);
                *active = false;
            }
        }
    }
}

/// Everything that depends on the chosen room; built off the audio thread.
pub struct Scene {
    pub config: SpatialConfig,
    reverb: Option<MatrixConvolver>,
    geometry: Geometry,
    /// Compensates the energy the room adds, so switching rooms doesn't
    /// change loudness (far seats end up with a quieter direct sound, as in
    /// a real hall).
    gain: f32,
}

impl Scene {
    /// Generates the room response when needed (tens of milliseconds).
    pub fn build(config: SpatialConfig, sample_rate: u32, block: usize, hrtf: &Hrtf) -> Result<Self, String> {
        let venue = match config.room.as_str() {
            "original" | "dry" => None,
            id => Some(room::venue(id).ok_or_else(|| format!("unknown room {id}"))?),
        };
        // "dry": close and wide, like sitting in front of the players.
        let geometry = venue.map_or(Geometry { distance: 3.0, stage_width: 7.0 }, |v| v.geometry(config.seat));
        let mut gain = 1.0;
        let reverb = venue.map(|v| {
            let irs = room::generate(v, config.seat, config.output, sample_rate, hrtf);
            // Energy one source adds (both ears) relative to the unit-energy
            // direct sound, weighted like music rather than white noise: big
            // rooms ring longest in the bass, where music has most energy.
            let wet = irs.iter().flatten().map(|ir| music_weighted_energy(ir, sample_rate)).sum::<f64>() / 2.0;
            gain = (1.0 / (1.0 + wet)).sqrt() as f32;
            let matrix: Vec<Vec<Vec<f32>>> = irs.into_iter().map(|ears| ears.into_iter().collect()).collect();
            MatrixConvolver::new(&mut Fft::new(block), &matrix)
        });
        Ok(Self { config, reverb, geometry, gain })
    }
}

/// Energy of `ir` as heard on typical music: its power spectrum averaged
/// with a pink (1/f, 50 Hz–16 kHz) weighting. A unit impulse gives 1.
fn music_weighted_energy(ir: &[f32], sample_rate: u32) -> f64 {
    if ir.is_empty() {
        return 0.0;
    }
    let n = ir.len().next_power_of_two() * 2;
    let fft = realfft::RealFftPlanner::<f64>::new().plan_fft_forward(n);
    let mut input = fft.make_input_vec();
    for (d, s) in input.iter_mut().zip(ir) {
        *d = f64::from(*s);
    }
    let mut spectrum = fft.make_output_vec();
    let _ = fft.process(&mut input, &mut spectrum);
    let (mut num, mut den) = (0.0, 0.0);
    for (k, c) in spectrum.iter().enumerate().skip(1) {
        let f = k as f64 * f64::from(sample_rate) / n as f64;
        if !(50.0..=16_000.0).contains(&f) {
            continue;
        }
        let w = 1.0 / f;
        num += c.norm_sqr() * w;
        den += w;
    }
    if den > 0.0 { num / den } else { 0.0 }
}

/// A point source rendered through the HRTF, with overlap-save state.
struct Source {
    prev: Vec<f32>,
    prev_nonzero: bool,
    dir: Option<usize>,
}

pub struct SpatialProcessor {
    block: usize,
    fft: Fft,
    hrtf: Arc<Hrtf>,
    scene: Box<Scene>,
    /// Per track two images (left / right of its stereo output), then the
    /// effects return pair, then the virtual speakers for the original mix.
    sources: Vec<Source>,
    max_tracks: usize,
    spectrum: Vec<Complex32>,
    product: Vec<Complex32>,
    acc: [Vec<Complex32>; 2],
    zeros: Vec<f32>,
}

/// Coherent mono content feeds both images of a channel; keep its loudness
/// equal to the speaker rendering.
const IMAGE_GAIN: f32 = std::f32::consts::FRAC_1_SQRT_2;

impl SpatialProcessor {
    pub fn new(block: usize, max_tracks: usize, hrtf: Arc<Hrtf>, scene: Box<Scene>) -> Self {
        let fft = Fft::new(block);
        let bins = fft.bins();
        Self {
            block,
            hrtf,
            scene,
            sources: (0..max_tracks * 2 + 4)
                .map(|_| Source { prev: vec![0.0; block], prev_nonzero: false, dir: None })
                .collect(),
            max_tracks,
            spectrum: vec![Complex32::default(); bins],
            product: vec![Complex32::default(); bins],
            acc: [vec![Complex32::default(); bins], vec![Complex32::default(); bins]],
            zeros: vec![0.0; block],
            fft,
        }
    }

    #[cfg(test)]
    pub fn hrtf(&self) -> Arc<Hrtf> {
        self.hrtf.clone()
    }

    pub fn config(&self) -> &SpatialConfig {
        &self.scene.config
    }

    /// Swap in a new room; returns the old one so the caller can drop it off
    /// the audio thread.
    pub fn set_scene(&mut self, scene: Box<Scene>) -> Box<Scene> {
        self.reset();
        std::mem::replace(&mut self.scene, scene)
    }

    /// Clear reverb tails and filter history (after a seek).
    pub fn reset(&mut self) {
        if let Some(r) = self.scene.reverb.as_mut() {
            r.reset();
        }
        for s in &mut self.sources {
            s.prev.fill(0.0);
            s.prev_nonzero = false;
        }
    }

    pub fn process(&mut self, input: &TrackBuffers, out: &mut [Vec<f32>; 2]) {
        let fx_source = self.max_tracks * 2;
        let speaker_source = fx_source + 2;
        for o in out.iter_mut() {
            o.fill(0.0);
        }
        let headphones = self.scene.config.output == OutputMode::Headphones;

        if self.scene.config.is_original() {
            if headphones {
                self.begin();
                self.source(speaker_source, Some(&input.mix[0]), 30.0, 0.0, 1.0, out);
                self.source(speaker_source + 1, Some(&input.mix[1]), -30.0, 0.0, 1.0, out);
                self.finish(out);
            } else {
                out[0].copy_from_slice(&input.mix[0]);
                out[1].copy_from_slice(&input.mix[1]);
            }
            return;
        }

        if headphones {
            let Geometry { distance, stage_width } = self.scene.geometry;
            let spread = (stage_width / 12.0).atan2(distance).to_degrees() as f32;
            self.begin();
            for t in 0..input.count.min(self.max_tracks) {
                let signal = input.active[t].then_some(&input.tracks[t]);
                // Pan -1..1 → position across the stage, seen from the seat.
                let x = f64::from(input.pan[t]) * stage_width / 2.0;
                let az = (-x).atan2(distance).to_degrees() as f32;
                self.source(t * 2, signal.map(|s| &s[0][..]), az + spread, 0.0, IMAGE_GAIN, out);
                self.source(t * 2 + 1, signal.map(|s| &s[1][..]), az - spread, 0.0, IMAGE_GAIN, out);
            }
            self.source(fx_source, Some(&input.fx[0]), 35.0, 0.0, 1.0, out);
            self.source(fx_source + 1, Some(&input.fx[1]), -35.0, 0.0, 1.0, out);
            self.finish(out);
        } else {
            // The synth's mix already has the MIDI pan and chorus, minus its
            // own reverb (disabled while a venue is active).
            out[0].copy_from_slice(&input.mix[0]);
            out[1].copy_from_slice(&input.mix[1]);
        }

        if let Some(reverb) = self.scene.reverb.as_mut() {
            let [l, r] = out;
            reverb.process(&mut self.fft, &[&input.mix[0], &input.mix[1]], &mut [&mut l[..], &mut r[..]]);
        }
        if self.scene.gain != 1.0 {
            for v in out.iter_mut().flatten() {
                *v *= self.scene.gain;
            }
        }
    }

    fn begin(&mut self) {
        for a in &mut self.acc {
            a.fill(Complex32::default());
        }
    }

    fn finish(&mut self, out: &mut [Vec<f32>; 2]) {
        for (ear, o) in out.iter_mut().enumerate() {
            self.fft.inverse_add(&self.acc[ear], o, |_| 1.0);
        }
    }

    /// Accumulate one HRTF-filtered source. A change of direction is
    /// cross-faded over the block to avoid clicks.
    fn source(&mut self, index: usize, signal: Option<&[f32]>, az: f32, el: f32, gain: f32, out: &mut [Vec<f32>; 2]) {
        let nonzero = signal.is_some_and(|s| s.iter().any(|v| v.abs() > 1e-9));
        let dir = self.hrtf.nearest(az, el);
        let src = &mut self.sources[index];
        if !nonzero && !src.prev_nonzero {
            src.dir = Some(dir);
            return;
        }
        let cur = if nonzero { signal.unwrap() } else { &self.zeros[..] };
        self.fft.forward_frame(&src.prev, cur, &mut self.spectrum);
        src.prev.copy_from_slice(cur);
        src.prev_nonzero = nonzero;
        if gain != 1.0 {
            for c in &mut self.spectrum {
                *c *= gain;
            }
        }

        let old = src.dir.replace(dir);
        match old {
            Some(old) if old != dir => {
                let n = self.block as f32;
                for ear in 0..2 {
                    for (d, fade) in [(old, false), (dir, true)] {
                        self.product.fill(Complex32::default());
                        complex_mac(&mut self.product, &self.spectrum, &self.hrtf.spectrum(d)[ear]);
                        self.fft.inverse_add(&self.product, &mut out[ear], |i| {
                            let t = i as f32 / n;
                            if fade { t } else { 1.0 - t }
                        });
                    }
                }
            }
            _ => {
                for ear in 0..2 {
                    complex_mac(&mut self.acc[ear], &self.spectrum, &self.hrtf.spectrum(dir)[ear]);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weighted_energy_of_impulse_is_one() {
        let mut ir = vec![0f32; 4800];
        ir[0] = 1.0;
        assert!((music_weighted_energy(&ir, 48_000) - 1.0).abs() < 1e-6);
        // A bass-heavy response counts more than its plain energy.
        let low: Vec<f32> = (0..48_000).map(|i| (i as f32 * 100.0 * std::f32::consts::TAU / 48_000.0).sin() * 0.01).collect();
        let plain: f64 = low.iter().map(|v| f64::from(*v).powi(2)).sum();
        assert!(music_weighted_energy(&low, 48_000) > 3.0 * plain);
    }
}

#[cfg(test)]
mod rate_tests {
    use super::*;

    #[test]
    fn scenes_are_finite_at_common_rates() {
        for sr in [44_100u32, 48_000, 88_200, 96_000] {
            let hrtf = Hrtf::load(sr, &mut Fft::new(1024));
            for venue in room::VENUES {
                for output in [OutputMode::Speakers, OutputMode::Headphones] {
                    let config = SpatialConfig { room: venue.id.into(), seat: 1, output };
                    let scene = Scene::build(config, sr, 1024, &hrtf).unwrap();
                    eprintln!("{sr} {} {output:?}: gain {}", venue.id, scene.gain);
                    assert!(scene.gain.is_finite() && scene.gain > 0.05, "{sr} {} gain {}", venue.id, scene.gain);
                }
            }
        }
    }
}
