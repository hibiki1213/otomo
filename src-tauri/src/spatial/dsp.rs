//! DSP building blocks for the spatial stage: an FFT wrapper, a uniformly
//! partitioned overlap-save convolver, a windowed-sinc resampler and a small
//! deterministic RNG.

use std::f64::consts::PI;
use std::sync::Arc;

use realfft::num_complex::Complex32;
use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};

/// Real FFT of size `2 * block` (overlap-save frames), with owned scratch so
/// nothing allocates on the audio thread.
pub struct Fft {
    block: usize,
    fwd: Arc<dyn RealToComplex<f32>>,
    inv: Arc<dyn ComplexToReal<f32>>,
    time: Vec<f32>,
    spec: Vec<Complex32>,
    scratch_fwd: Vec<Complex32>,
    scratch_inv: Vec<Complex32>,
}

impl Fft {
    pub fn new(block: usize) -> Self {
        let mut planner = RealFftPlanner::<f32>::new();
        let fwd = planner.plan_fft_forward(block * 2);
        let inv = planner.plan_fft_inverse(block * 2);
        Self {
            block,
            time: fwd.make_input_vec(),
            spec: fwd.make_output_vec(),
            scratch_fwd: fwd.make_scratch_vec(),
            scratch_inv: inv.make_scratch_vec(),
            fwd,
            inv,
        }
    }

    pub fn block(&self) -> usize {
        self.block
    }

    pub fn bins(&self) -> usize {
        self.block + 1
    }

    /// Spectrum of `[prev, cur]` (each `block` long).
    pub fn forward_frame(&mut self, prev: &[f32], cur: &[f32], out: &mut [Complex32]) {
        let n = self.block;
        self.time[..n].copy_from_slice(prev);
        self.time[n..].copy_from_slice(cur);
        let _ = self.fwd.process_with_scratch(&mut self.time, out, &mut self.scratch_fwd);
    }

    /// Spectrum of `data` zero-padded to `2 * block`, pre-scaled by the
    /// inverse-FFT normalisation so products need no further scaling.
    pub fn filter_spectrum(&mut self, data: &[f32]) -> Vec<Complex32> {
        let len = data.len().min(self.block * 2);
        self.time.fill(0.0);
        self.time[..len].copy_from_slice(&data[..len]);
        let mut out = self.fwd.make_output_vec();
        let _ = self.fwd.process_with_scratch(&mut self.time, &mut out, &mut self.scratch_fwd);
        let scale = 1.0 / (self.block * 2) as f32;
        for c in &mut out {
            *c *= scale;
        }
        out
    }

    /// Inverse of an overlap-save product; adds the valid (second) half,
    /// multiplied by the per-sample `gain`, into `out`.
    pub fn inverse_add(&mut self, spec: &[Complex32], out: &mut [f32], gain: impl Fn(usize) -> f32) {
        let n = self.block;
        self.spec.copy_from_slice(spec);
        self.spec[0].im = 0.0;
        self.spec[n].im = 0.0;
        let _ = self.inv.process_with_scratch(&mut self.spec, &mut self.time, &mut self.scratch_inv);
        for (i, o) in out.iter_mut().enumerate().take(n) {
            *o += self.time[n + i] * gain(i);
        }
    }
}

/// `acc += a * b` over complex vectors.
#[inline]
pub fn complex_mac(acc: &mut [Complex32], a: &[Complex32], b: &[Complex32]) {
    for ((o, x), h) in acc.iter_mut().zip(a).zip(b) {
        o.re += x.re * h.re - x.im * h.im;
        o.im += x.re * h.im + x.im * h.re;
    }
}

/// A spectrum stored as separate real / imaginary arrays, so the
/// multiply-accumulate below vectorises (SIMD) well.
#[derive(Clone)]
struct SplitSpectrum {
    re: Vec<f32>,
    im: Vec<f32>,
}

impl SplitSpectrum {
    fn zeros(bins: usize) -> Self {
        Self { re: vec![0.0; bins], im: vec![0.0; bins] }
    }

    fn from_complex(c: &[Complex32]) -> Self {
        Self { re: c.iter().map(|v| v.re).collect(), im: c.iter().map(|v| v.im).collect() }
    }

    fn set(&mut self, c: &[Complex32]) {
        for ((r, i), v) in self.re.iter_mut().zip(self.im.iter_mut()).zip(c) {
            *r = v.re;
            *i = v.im;
        }
    }

    fn clear(&mut self) {
        self.re.fill(0.0);
        self.im.fill(0.0);
    }

    /// `self += a * b`
    #[inline]
    fn mac(&mut self, a: &SplitSpectrum, b: &SplitSpectrum) {
        let n = self.re.len();
        let (ore, oim) = (&mut self.re[..n], &mut self.im[..n]);
        let (ar, ai, br, bi) = (&a.re[..n], &a.im[..n], &b.re[..n], &b.im[..n]);
        for k in 0..n {
            ore[k] += ar[k] * br[k] - ai[k] * bi[k];
            oim[k] += ar[k] * bi[k] + ai[k] * br[k];
        }
    }
}

/// Uniformly partitioned overlap-save convolution of `I` inputs into `O`
/// outputs through an `I x O` matrix of (long) impulse responses.
pub struct MatrixConvolver {
    parts: usize,
    inputs: usize,
    /// `[input][output][partition]` filter spectra.
    filters: Vec<Vec<Vec<SplitSpectrum>>>,
    /// Per-input frequency-domain delay line (ring of `parts` spectra).
    fdl: Vec<Vec<SplitSpectrum>>,
    prev: Vec<Vec<f32>>,
    ring: usize,
    frame: Vec<Complex32>,
    acc: Vec<SplitSpectrum>,
    joined: Vec<Complex32>,
    silent_blocks: usize,
}

impl MatrixConvolver {
    /// `irs[input][output]`; all responses are split into `block`-long
    /// partitions. Heavy: build off the audio thread.
    pub fn new(fft: &mut Fft, irs: &[Vec<Vec<f32>>]) -> Self {
        let block = fft.block();
        let inputs = irs.len();
        let outputs = irs.first().map_or(0, |r| r.len());
        let longest = irs.iter().flatten().map(|h| h.len()).max().unwrap_or(0);
        let parts = longest.div_ceil(block).max(1);
        let filters = irs
            .iter()
            .map(|row| {
                row.iter()
                    .map(|h| {
                        (0..parts)
                            .map(|p| {
                                let from = (p * block).min(h.len());
                                let to = ((p + 1) * block).min(h.len());
                                SplitSpectrum::from_complex(&fft.filter_spectrum(&h[from..to]))
                            })
                            .collect()
                    })
                    .collect()
            })
            .collect();
        let bins = fft.bins();
        Self {
            parts,
            inputs,
            filters,
            fdl: vec![vec![SplitSpectrum::zeros(bins); parts]; inputs],
            prev: vec![vec![0.0; block]; inputs],
            ring: 0,
            frame: vec![Complex32::default(); bins],
            acc: vec![SplitSpectrum::zeros(bins); outputs],
            joined: vec![Complex32::default(); bins],
            silent_blocks: usize::MAX,
        }
    }

    /// Convolve one block; results are added into `outputs`.
    pub fn process(&mut self, fft: &mut Fft, inputs: &[&[f32]], outputs: &mut [&mut [f32]]) {
        let silent = inputs.iter().all(|x| x.iter().all(|v| v.abs() < 1e-9));
        if silent {
            // Once the delay line has fully drained the output is zero.
            if self.silent_blocks >= self.parts + 1 {
                return;
            }
            self.silent_blocks = self.silent_blocks.saturating_add(1);
        } else {
            self.silent_blocks = 0;
        }

        for (i, x) in inputs.iter().enumerate().take(self.inputs) {
            fft.forward_frame(&self.prev[i], x, &mut self.frame);
            self.fdl[i][self.ring].set(&self.frame);
            self.prev[i].copy_from_slice(x);
        }
        for acc in &mut self.acc {
            acc.clear();
        }
        for i in 0..self.inputs {
            for p in 0..self.parts {
                let x = &self.fdl[i][(self.ring + self.parts - p) % self.parts];
                for (o, acc) in self.acc.iter_mut().enumerate() {
                    acc.mac(x, &self.filters[i][o][p]);
                }
            }
        }
        for (acc, out) in self.acc.iter().zip(outputs.iter_mut()) {
            for ((c, r), im) in self.joined.iter_mut().zip(&acc.re).zip(&acc.im) {
                *c = Complex32::new(*r, *im);
            }
            fft.inverse_add(&self.joined, out, |_| 1.0);
        }
        self.ring = (self.ring + 1) % self.parts;
    }

    /// Forget all history (after a seek).
    pub fn reset(&mut self) {
        for line in &mut self.fdl {
            for s in line {
                s.clear();
            }
        }
        for p in &mut self.prev {
            p.fill(0.0);
        }
        self.silent_blocks = usize::MAX;
    }
}

/// Band-limited resampling with a Blackman-windowed sinc (offline use).
pub fn resample(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to {
        return input.to_vec();
    }
    let ratio = f64::from(to) / f64::from(from);
    let out_len = (input.len() as f64 * ratio).ceil() as usize;
    let cutoff = ratio.min(1.0); // relative to the input Nyquist
    let half = 32.0 / cutoff;
    (0..out_len)
        .map(|j| {
            let t = j as f64 / ratio;
            let lo = (t - half).floor().max(0.0) as usize;
            let hi = ((t + half).ceil() as usize).min(input.len().saturating_sub(1));
            let mut acc = 0.0;
            for (k, &x) in input.iter().enumerate().take(hi + 1).skip(lo) {
                let d = k as f64 - t;
                let sinc = if d.abs() < 1e-9 {
                    1.0
                } else {
                    (PI * cutoff * d).sin() / (PI * cutoff * d)
                };
                let w = 0.42 + 0.5 * (PI * d / half).cos() + 0.08 * (2.0 * PI * d / half).cos();
                if d.abs() <= half {
                    acc += f64::from(x) * cutoff * sinc * w;
                }
            }
            acc as f32
        })
        .collect()
}

/// xorshift64* — deterministic so generated rooms are reproducible.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniform in (0, 1].
    pub fn uniform(&mut self) -> f64 {
        ((self.next_u64() >> 11) as f64 + 1.0) / (1u64 << 53) as f64
    }

    /// Standard normal (Box–Muller).
    pub fn gaussian(&mut self) -> f64 {
        let (u1, u2) = (self.uniform(), self.uniform());
        (-2.0 * u1.ln()).sqrt() * (2.0 * PI * u2).cos()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn direct_convolve(x: &[f32], h: &[f32]) -> Vec<f32> {
        let mut y = vec![0.0; x.len() + h.len()];
        for (i, &xv) in x.iter().enumerate() {
            for (j, &hv) in h.iter().enumerate() {
                y[i + j] += xv * hv;
            }
        }
        y
    }

    #[test]
    fn partitioned_convolution_matches_direct() {
        let block = 64;
        let mut fft = Fft::new(block);
        let mut rng = Rng::new(7);
        let h: Vec<f32> = (0..300).map(|_| rng.gaussian() as f32 * 0.1).collect();
        let x: Vec<f32> = (0..block * 8).map(|_| rng.gaussian() as f32).collect();
        let mut conv = MatrixConvolver::new(&mut fft, &[vec![h.clone()]]);
        let mut y = vec![0.0; x.len()];
        for (xb, yb) in x.chunks(block).zip(y.chunks_mut(block)) {
            conv.process(&mut fft, &[xb], &mut [yb]);
        }
        let reference = direct_convolve(&x, &h);
        let err = y.iter().zip(&reference).map(|(a, b)| (a - b).abs()).fold(0f32, f32::max);
        assert!(err < 1e-4, "max error {err}");
    }

    #[test]
    fn resample_preserves_a_low_tone() {
        let from = 48_000;
        let to = 44_100;
        let tone: Vec<f32> = (0..4800).map(|i| (2.0 * PI * 1000.0 * i as f64 / 48_000.0).sin() as f32).collect();
        let out = resample(&tone, from, to);
        assert_eq!(out.len(), 4410);
        for (j, &v) in out.iter().enumerate().skip(100).take(4000) {
            let expected = (2.0 * PI * 1000.0 * j as f64 / 44_100.0).sin() as f32;
            assert!((v - expected).abs() < 1e-3, "sample {j}: {v} vs {expected}");
        }
    }
}
