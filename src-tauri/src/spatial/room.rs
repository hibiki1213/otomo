//! Venue presets and room impulse-response synthesis.
//!
//! No recorded IRs are shipped: each venue is a shoebox room with measured-
//! style reverberation times, rendered as
//! * early reflections — image-source method, every reflection placed by its
//!   actual direction (through the HRTF for headphones, a virtual ORTF
//!   microphone pair for speakers), duller with each wall bounce;
//! * late reverberation — noise shaped in the STFT domain with a per-
//!   frequency decay (RT60 by octave band) and a realistic inter-ear /
//!   inter-mic coherence, level-matched to classical room acoustics
//!   (Sabine critical distance).

use std::f64::consts::PI;

use realfft::num_complex::Complex32;
use realfft::RealFftPlanner;
use serde::Serialize;

use super::dsp::Rng;
use super::hrtf::Hrtf;
use super::OutputMode;

const SPEED_OF_SOUND: f64 = 343.0;
/// Octave band centres for `rt60`.
const BANDS: [f64; 7] = [125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0];
/// Recordings sit closer to the dry sound than a real seat; tame the
/// physically-predicted reverberant level a little.
const MUSICAL_WET: f64 = 0.6;
/// Upper bound on reverberant / direct energy per seat (front, middle,
/// back) before `liveness`: roughly -5 dB, 0 dB and +3 dB. Physics alone
/// puts back rows of big rooms at +10 dB, which buries the music.
const SEAT_WET_CAP: [f64; 3] = [0.3, 1.0, 2.0];

pub struct VenueSpec {
    pub id: &'static str,
    pub name: &'static str,
    /// Room length (stage → back wall), width, height in metres.
    pub size: [f64; 3],
    pub rt60: [f64; 7],
    /// Distance of the stage sources from the front wall.
    pub stage_depth: f64,
    pub stage_width: f64,
    /// Listener distance from the stage for front / middle / back seats.
    pub seats: [f64; 3],
    /// Floor reflection relative to the walls (audience seating absorbs).
    pub floor_factor: f64,
    /// Scales `SEAT_WET_CAP`: how reverberant the room feels.
    pub liveness: f64,
}

pub const VENUES: &[VenueSpec] = &[
    VenueSpec {
        id: "studio",
        name: "レコーディングスタジオ",
        size: [12.0, 9.0, 4.5],
        rt60: [0.45, 0.42, 0.40, 0.38, 0.36, 0.33, 0.28],
        stage_depth: 3.0,
        stage_width: 5.0,
        seats: [2.0, 3.5, 6.0],
        floor_factor: 1.0,
        liveness: 0.5,
    },
    VenueSpec {
        id: "club",
        name: "ライブハウス",
        size: [20.0, 12.0, 4.5],
        rt60: [0.9, 0.8, 0.7, 0.65, 0.6, 0.55, 0.45],
        stage_depth: 2.5,
        stage_width: 7.0,
        seats: [3.0, 7.0, 13.0],
        floor_factor: 0.85,
        liveness: 0.7,
    },
    VenueSpec {
        id: "chamber",
        name: "室内楽ホール",
        size: [30.0, 17.0, 11.0],
        rt60: [1.8, 1.7, 1.6, 1.5, 1.4, 1.25, 1.0],
        stage_depth: 4.0,
        stage_width: 9.0,
        seats: [5.0, 11.0, 20.0],
        floor_factor: 0.7,
        liveness: 1.0,
    },
    VenueSpec {
        id: "concert",
        name: "コンサートホール（大ホール）",
        size: [50.0, 27.0, 19.0],
        rt60: [2.4, 2.25, 2.1, 2.0, 1.85, 1.6, 1.25],
        stage_depth: 6.0,
        stage_width: 16.0,
        seats: [9.0, 20.0, 35.0],
        floor_factor: 0.6,
        liveness: 1.1,
    },
    VenueSpec {
        id: "cathedral",
        name: "大聖堂",
        size: [90.0, 32.0, 32.0],
        rt60: [6.5, 6.0, 5.4, 4.8, 4.0, 3.1, 2.1],
        stage_depth: 8.0,
        stage_width: 12.0,
        seats: [10.0, 25.0, 50.0],
        floor_factor: 0.9,
        liveness: 1.4,
    },
];

pub fn venue(id: &str) -> Option<&'static VenueSpec> {
    VENUES.iter().find(|v| v.id == id)
}

#[derive(Serialize)]
pub struct VenueInfo {
    id: &'static str,
    name: &'static str,
    rt60: f64,
}

pub fn venue_list() -> Vec<VenueInfo> {
    VENUES
        .iter()
        .map(|v| VenueInfo { id: v.id, name: v.name, rt60: v.rt60[3] })
        .collect()
}

/// Where the listener sits and how wide the stage appears from there.
pub struct Geometry {
    pub distance: f64,
    pub stage_width: f64,
}

impl VenueSpec {
    fn volume(&self) -> f64 {
        self.size.iter().product()
    }

    fn surface(&self) -> f64 {
        let [l, w, h] = self.size;
        2.0 * (l * w + l * h + w * h)
    }

    pub fn geometry(&self, seat: usize) -> Geometry {
        let distance = self.seats[seat.min(2)].min(self.size[0] - self.stage_depth - 1.5);
        Geometry { distance, stage_width: self.stage_width }
    }

    fn rt_at(&self, f: f64) -> f64 {
        if f <= BANDS[0] {
            return self.rt60[0];
        }
        for i in 1..BANDS.len() {
            if f <= BANDS[i] {
                let t = (f / BANDS[i - 1]).log2();
                return self.rt60[i - 1] + t * (self.rt60[i] - self.rt60[i - 1]);
            }
        }
        self.rt60[BANDS.len() - 1]
    }
}

/// Four responses: `[source][ear]`, sources being the stage-left and
/// stage-right halves fed by the left / right channel of the dry mix.
pub type RoomIrs = [[Vec<f32>; 2]; 2];

/// Direction of arrival as seen by a listener facing the stage.
fn direction(v: [f64; 3]) -> (f32, f32) {
    // Room axes: x = across, y = away from the stage, z = up. The listener
    // faces -y and we take smaller x as their left (a mirror image of a
    // right-handed frame, which a symmetric shoebox doesn't care about), so
    // the stage-left source sits at smaller x.
    let front = -v[1];
    let left = -v[0];
    let az = left.atan2(front).to_degrees() as f32;
    let horiz = (v[0] * v[0] + v[1] * v[1]).sqrt();
    let el = v[2].atan2(horiz).to_degrees() as f32;
    (az, el)
}

/// Cardioid pair at ±55° (ORTF-like) for the speaker rendering.
fn mic_gains(az_deg: f32, el_deg: f32) -> [f32; 2] {
    let (az, el) = (f64::from(az_deg).to_radians(), f64::from(el_deg).to_radians());
    let u = [el.cos() * az.cos(), el.cos() * az.sin(), el.sin()];
    let axis = |a: f64| [a.to_radians().cos(), a.to_radians().sin(), 0.0];
    let card = |ax: [f64; 3]| (0.5 + 0.5 * (u[0] * ax[0] + u[1] * ax[1] + u[2] * ax[2])) as f32;
    [card(axis(55.0)), card(axis(-55.0))]
}

/// Forward-backward one-pole low-pass (zero phase).
fn lowpass_zero_phase(x: &mut [f32], cutoff: f64, sample_rate: f64) {
    let a = (-2.0 * PI * cutoff / sample_rate).exp() as f32;
    let mut y = 0.0;
    for v in x.iter_mut() {
        y = (1.0 - a) * *v + a * y;
        *v = y;
    }
    y = 0.0;
    for v in x.iter_mut().rev() {
        y = (1.0 - a) * *v + a * y;
        *v = y;
    }
}

pub fn generate(spec: &VenueSpec, seat: usize, output: OutputMode, sample_rate: u32, hrtf: &Hrtf) -> RoomIrs {
    let sr = f64::from(sample_rate);
    let [length, width, height] = spec.size;
    let geo = spec.geometry(seat);

    let alpha = (0.161 * spec.volume() / (spec.surface() * spec.rt60[3])).clamp(0.02, 0.9);
    let r_wall = (1.0 - alpha).sqrt();
    let r_floor = (r_wall * spec.floor_factor).min(0.98);

    // Slightly off-centre so left and right reflections don't arrive
    // perfectly symmetrically (which sounds phasey).
    let listener = [width / 2.0 + 0.35, spec.stage_depth + geo.distance, 1.2];
    let sources = [
        [width / 2.0 - spec.stage_width / 4.0, spec.stage_depth, 1.6],
        [width / 2.0 + spec.stage_width / 4.0, spec.stage_depth, 1.6],
    ];

    let t60_max = spec.rt60.iter().cloned().fold(0.0, f64::max);
    let ir_len = ((t60_max * 1.1).min(7.0) * sr) as usize;
    let mixing_time = (spec.volume().sqrt() / 1000.0).clamp(0.04, 0.2);
    let critical = 0.057 * (spec.volume() / spec.rt60[3]).sqrt();

    let mut out: RoomIrs = Default::default();
    for (s, src) in sources.iter().enumerate() {
        let d0 = (0..3).map(|k| (src[k] - listener[k]).powi(2)).sum::<f64>().sqrt();

        /* ----- early reflections, bucketed by reflection order ----- */
        const MAX_ORDER: usize = 12;
        let er_len = ((mixing_time + 0.05) * sr) as usize + 1024;
        let mut buckets = vec![[vec![0f32; er_len], vec![0f32; er_len]]; MAX_ORDER + 1];
        let mut er_energy = 0.0;
        let mut first_reflection = mixing_time;
        let reach = SPEED_OF_SOUND * (mixing_time + 0.02) + d0;
        let n_axis = |dim: f64| (reach / (2.0 * dim)).ceil() as i64 + 1;

        let dims = [width, length, height];
        let (nx, ny, nz) = (n_axis(width), n_axis(length), n_axis(height));
        for ix in -nx..=nx {
            for px in 0..2i64 {
                for iy in -ny..=ny {
                    for py in 0..2i64 {
                        for iz in -nz..=nz {
                            for pz in 0..2i64 {
                                let n = [ix, iy, iz];
                                let p = [px, py, pz];
                                if n == [0, 0, 0] && p == [0, 0, 0] {
                                    continue; // direct sound is rendered separately
                                }
                                let mut img = [0.0; 3];
                                let mut hits = [0i64; 6];
                                for k in 0..3 {
                                    img[k] = (1 - 2 * p[k]) as f64 * src[k] + 2.0 * n[k] as f64 * dims[k];
                                    hits[2 * k] = (n[k] - p[k]).abs();
                                    hits[2 * k + 1] = n[k].abs();
                                }
                                let v = [img[0] - listener[0], img[1] - listener[1], img[2] - listener[2]];
                                let d = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
                                let delay = (d - d0) / SPEED_OF_SOUND;
                                if delay > mixing_time || delay <= 0.0 {
                                    continue;
                                }
                                // Floor is hits[4] (z = 0); everything else is wall/ceiling.
                                let walls = hits.iter().sum::<i64>() - hits[4];
                                let amp = r_wall.powi(walls as i32) * r_floor.powi(hits[4] as i32) * d0 / d;
                                let order = (walls + hits[4]) as usize;
                                if amp < 1e-4 {
                                    continue;
                                }
                                er_energy += amp * amp;
                                first_reflection = first_reflection.min(delay);

                                let at = (delay * sr).round() as usize;
                                let (az, el) = direction(v);
                                let bucket = &mut buckets[order.min(MAX_ORDER)];
                                match output {
                                    OutputMode::Headphones => {
                                        let [hl, hr] = hrtf.ir(hrtf.nearest(az, el));
                                        for (ear, h) in [hl, hr].into_iter().enumerate() {
                                            for (k, &hv) in h.iter().enumerate() {
                                                if let Some(slot) = bucket[ear].get_mut(at + k) {
                                                    *slot += amp as f32 * hv;
                                                }
                                            }
                                        }
                                    }
                                    OutputMode::Speakers => {
                                        let g = mic_gains(az, el);
                                        for ear in 0..2 {
                                            if let Some(slot) = bucket[ear].get_mut(at) {
                                                *slot += amp as f32 * g[ear];
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        let mut ir = [vec![0f32; ir_len], vec![0f32; ir_len]];
        for (order, bucket) in buckets.iter_mut().enumerate().skip(1) {
            // Each bounce absorbs more high end.
            let cutoff = 14_000.0 * 0.78f64.powi(order as i32);
            for (ear, sig) in bucket.iter_mut().enumerate() {
                lowpass_zero_phase(sig, cutoff.min(sr * 0.45), sr);
                for (o, v) in ir[ear].iter_mut().zip(sig.iter()) {
                    *o += v;
                }
            }
        }

        /* ----- late reverberation ----- */
        let cap = SEAT_WET_CAP[seat.min(2)] * spec.liveness;
        let target = (MUSICAL_WET * (d0 / critical).powi(2)).min(cap);
        // Keep early reflections inside the budget too.
        if er_energy > 0.7 * target {
            let g = (0.7 * target / er_energy).sqrt() as f32;
            for v in ir.iter_mut().flatten() {
                *v *= g;
            }
            er_energy = 0.7 * target;
        }
        let late_energy = (target - er_energy).max(0.3 * target);
        let spacing = match output {
            OutputMode::Headphones => 0.2, // ear distance
            OutputMode::Speakers => 0.8,   // spaced room mics
        };
        let tail = diffuse_tail(
            spec,
            ir_len,
            sr,
            first_reflection,
            mixing_time,
            spacing,
            0x5eed_0000 + s as u64 * 7919 + seat as u64 * 104_729,
        );
        let tail_energy: f64 = tail.iter().flatten().map(|v| f64::from(*v).powi(2)).sum();
        // Energy is split across both ears / mics.
        let gain = if tail_energy > 0.0 { (late_energy / tail_energy).sqrt() as f32 } else { 0.0 };
        for ear in 0..2 {
            for (o, v) in ir[ear].iter_mut().zip(&tail[ear]) {
                *o += v * gain;
            }
        }

        // Drop the inaudible end.
        let peak = ir.iter().flatten().fold(0f32, |m, v| m.max(v.abs()));
        let floor = peak * 1e-4;
        let end = ir
            .iter()
            .map(|e| e.iter().rposition(|v| v.abs() > floor).unwrap_or(0) + 1)
            .max()
            .unwrap_or(0);
        for e in &mut ir {
            e.truncate(end);
        }
        out[s] = ir;
    }
    out
}

/// Two-channel diffuse tail with frequency-dependent decay and coherence.
fn diffuse_tail(
    spec: &VenueSpec,
    len: usize,
    sr: f64,
    start: f64,
    mixing_time: f64,
    spacing: f64,
    seed: u64,
) -> [Vec<f32>; 2] {
    const N: usize = 1024;
    const HOP: usize = N / 4;
    let mut planner = RealFftPlanner::<f32>::new();
    let inv = planner.plan_fft_inverse(N);
    let mut spec_l = inv.make_input_vec();
    let mut spec_r = inv.make_input_vec();
    let mut frame = inv.make_output_vec();
    let window: Vec<f32> = (0..N).map(|i| (0.5 - 0.5 * (2.0 * PI * i as f64 / N as f64).cos()) as f32).collect();

    let bins = N / 2 + 1;
    let freq = |k: usize| k as f64 * sr / N as f64;
    let decay: Vec<f64> = (0..bins).map(|k| 6.91 / spec.rt_at(freq(k).max(20.0))).collect();
    let coherence: Vec<f64> = (0..bins)
        .map(|k| {
            let x = 2.0 * PI * freq(k) * spacing / SPEED_OF_SOUND;
            if x < 1e-6 { 1.0 } else { (x.sin() / x).clamp(-1.0, 1.0) }
        })
        .collect();

    let mut rng = Rng::new(seed);
    let mut out = [vec![0f32; len + N], vec![0f32; len + N]];
    let mut pos = 0;
    while pos < len {
        let t = (pos + N / 2) as f64 / sr;
        for k in 0..bins {
            let env = (-decay[k] * t).exp();
            let (a, b) = (
                Complex32::new(rng.gaussian() as f32, rng.gaussian() as f32),
                Complex32::new(rng.gaussian() as f32, rng.gaussian() as f32),
            );
            let c = coherence[k] as f32;
            let side = (1.0 - c * c).max(0.0).sqrt();
            spec_l[k] = a * env as f32;
            spec_r[k] = (a * c + b * side) * env as f32;
        }
        for spec in [&mut spec_l, &mut spec_r] {
            spec[0].im = 0.0;
            spec[bins - 1].im = 0.0;
        }
        for (ch, spec) in [&mut spec_l, &mut spec_r].into_iter().enumerate() {
            let _ = inv.process(spec, &mut frame);
            for i in 0..N {
                out[ch][pos + i] += frame[i] * window[i];
            }
        }
        pos += HOP;
    }

    // Fade in between the first reflection and the mixing time, where the
    // discrete early reflections hand over to the diffuse field.
    for ch in &mut out {
        ch.truncate(len);
        for (i, v) in ch.iter_mut().enumerate() {
            let t = i as f64 / sr;
            let ramp = if t <= start {
                0.0
            } else if t >= mixing_time {
                1.0
            } else {
                let x = (t - start) / (mixing_time - start).max(1e-3);
                (x * PI / 2.0).sin().powi(2)
            };
            *v *= ramp as f32;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::dsp::Fft;
    use super::*;

    /// Schroeder backward integration → T30-based RT60 estimate.
    fn rt60_estimate(ir: &[f32], sr: f64) -> f64 {
        let mut edc: Vec<f64> = ir.iter().map(|v| f64::from(*v).powi(2)).collect();
        for i in (0..edc.len() - 1).rev() {
            edc[i] += edc[i + 1];
        }
        let total = edc[0];
        let db = |i: usize| 10.0 * (edc[i] / total).log10();
        let t5 = (0..edc.len()).find(|&i| db(i) < -5.0).unwrap();
        let t35 = (0..edc.len()).find(|&i| db(i) < -35.0).unwrap();
        2.0 * (t35 - t5) as f64 / sr
    }

    fn broadband_rt(spec: &VenueSpec) -> f64 {
        // Mid-band (500 Hz–2 kHz) average, what a broadband T30 tracks.
        (spec.rt60[2] + spec.rt60[3] + spec.rt60[4]) / 3.0
    }

    #[test]
    fn generated_decay_matches_venue_rt60() {
        let sr = 48_000;
        let mut fft = Fft::new(1024);
        let hrtf = Hrtf::load(sr, &mut fft);
        for spec in VENUES {
            for output in [OutputMode::Speakers, OutputMode::Headphones] {
                let irs = generate(spec, 1, output, sr, &hrtf);
                let rt = rt60_estimate(&irs[0][0], f64::from(sr));
                let want = broadband_rt(spec);
                eprintln!("{:>10} {:?}: RT60 {rt:.2}s (spec {want:.2}s)", spec.id, output);
                assert!((rt / want - 1.0).abs() < 0.25, "{} RT60 {rt} vs {want}", spec.id);
            }
        }
    }

    #[test]
    fn back_seats_are_wetter_than_front() {
        let sr = 48_000;
        let mut fft = Fft::new(1024);
        let hrtf = Hrtf::load(sr, &mut fft);
        let spec = venue("concert").unwrap();
        let energy = |seat| -> f64 {
            generate(spec, seat, OutputMode::Speakers, sr, &hrtf)
                .iter()
                .flatten()
                .flatten()
                .map(|v| f64::from(*v).powi(2))
                .sum()
        };
        let (front, back) = (energy(0), energy(2));
        eprintln!("reverb energy front {front:.2}, back {back:.2}");
        assert!(back > 2.0 * front);
    }

    #[test]
    fn stage_left_reflections_favour_the_left() {
        let sr = 48_000;
        let mut fft = Fft::new(1024);
        let hrtf = Hrtf::load(sr, &mut fft);
        let spec = venue("chamber").unwrap();
        let irs = generate(spec, 0, OutputMode::Headphones, sr, &hrtf);
        let early = |x: &[f32]| x[..(0.05 * f64::from(sr)) as usize].iter().map(|v| v * v).sum::<f32>();
        let [l, r] = &irs[0];
        assert!(early(l) > early(r), "stage-left early energy should lean left");
    }
}
