//! Head-related impulse responses from the SADIE II database (Neumann KU100
//! dummy head, University of York, Apache-2.0 — see assets/NOTICE-SADIE.txt).
//! Converted by scripts/convert_hrir.py; 649 directions, 5° azimuth steps.

use realfft::num_complex::Complex32;

use super::dsp::{resample, Fft};

const DATA: &[u8] = include_bytes!("../../assets/sadie_ku100_48k.hrir");

pub struct Hrtf {
    /// Unit vectors (x = front, y = left, z = up) per direction.
    dirs: Vec<[f32; 3]>,
    /// Time-domain responses at the output rate, `[left, right]`.
    irs: Vec<[Vec<f32>; 2]>,
    /// Overlap-save filter spectra for the engine's block size.
    spectra: Vec<[Vec<Complex32>; 2]>,
}

fn unit(az_deg: f32, el_deg: f32) -> [f32; 3] {
    let (az, el) = (az_deg.to_radians(), el_deg.to_radians());
    [el.cos() * az.cos(), el.cos() * az.sin(), el.sin()]
}

impl Hrtf {
    /// Parse the embedded set, resample to `sample_rate` and normalise so a
    /// frontal source has unit total (left + right) energy — the same
    /// loudness as a centre-panned source on speakers.
    pub fn load(sample_rate: u32, fft: &mut Fft) -> Self {
        let rd_u32 = |o: usize| u32::from_le_bytes(DATA[o..o + 4].try_into().unwrap());
        let rd_f32 = |o: usize| f32::from_le_bytes(DATA[o..o + 4].try_into().unwrap());
        assert_eq!(&DATA[..4], b"HRIR", "bad HRIR asset");
        let (rate, taps, count) = (rd_u32(4), rd_u32(8) as usize, rd_u32(12) as usize);

        let mut dirs = Vec::with_capacity(count);
        let mut irs = Vec::with_capacity(count);
        let mut o = 16;
        for _ in 0..count {
            let (az, el) = (rd_f32(o), rd_f32(o + 4));
            o += 8;
            let mut ears: [Vec<f32>; 2] = Default::default();
            for ear in &mut ears {
                let raw: Vec<f32> = (0..taps).map(|i| rd_f32(o + i * 4)).collect();
                o += taps * 4;
                *ear = resample(&raw, rate, sample_rate);
            }
            dirs.push(unit(az, el));
            irs.push(ears);
        }

        let mut hrtf = Self { dirs, irs, spectra: Vec::new() };
        let front = hrtf.nearest(0.0, 0.0);
        let energy: f32 = hrtf.irs[front].iter().flatten().map(|v| v * v).sum();
        let gain = 1.0 / energy.sqrt();
        for ears in &mut hrtf.irs {
            for ear in ears.iter_mut() {
                for v in ear.iter_mut() {
                    *v *= gain;
                }
            }
        }
        hrtf.spectra = hrtf
            .irs
            .iter()
            .map(|[l, r]| [fft.filter_spectrum(l), fft.filter_spectrum(r)])
            .collect();
        hrtf
    }

    /// Index of the measured direction closest to (azimuth, elevation) in
    /// degrees; azimuth 0 = front, +90 = left.
    pub fn nearest(&self, az_deg: f32, el_deg: f32) -> usize {
        let u = unit(az_deg, el_deg);
        let mut best = (0, f32::MIN);
        for (i, d) in self.dirs.iter().enumerate() {
            let dot = d[0] * u[0] + d[1] * u[1] + d[2] * u[2];
            if dot > best.1 {
                best = (i, dot);
            }
        }
        best.0
    }

    pub fn ir(&self, index: usize) -> &[Vec<f32>; 2] {
        &self.irs[index]
    }

    pub fn spectrum(&self, index: usize) -> &[Vec<Complex32>; 2] {
        &self.spectra[index]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn energy(x: &[f32]) -> f32 {
        x.iter().map(|v| v * v).sum()
    }

    fn onset(x: &[f32]) -> usize {
        let peak = x.iter().fold(0f32, |m, v| m.max(v.abs()));
        x.iter().position(|v| v.abs() > 0.1 * peak).unwrap()
    }

    #[test]
    fn left_source_reaches_left_ear_first_and_louder() {
        let mut fft = Fft::new(1024);
        let h = Hrtf::load(48_000, &mut fft);
        let [l, r] = h.ir(h.nearest(90.0, 0.0));
        assert!(energy(l) > 10.0 * energy(r));
        assert!(onset(l) + 10 < onset(r));
        let [l, r] = h.ir(h.nearest(-90.0, 0.0));
        assert!(energy(r) > 10.0 * energy(l));
    }

    #[test]
    fn frontal_source_is_normalised_and_balanced() {
        let mut fft = Fft::new(1024);
        let h = Hrtf::load(44_100, &mut fft);
        let [l, r] = h.ir(h.nearest(0.0, 0.0));
        assert!(((energy(l) + energy(r)) - 1.0).abs() < 1e-3);
        assert!((energy(l) / energy(r)).log10().abs() < 0.1, "front should be balanced");
    }
}
