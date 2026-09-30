//! SFZ instruments through the vendored sfizz library (BSD-2-Clause).
//!
//! Threading follows sfizz's contract: an instance is created and its file
//! loaded on a worker thread, then moved to the audio thread, which is the only
//! one that touches it afterwards (events + rendering). It is freed on the
//! engine's janitor thread. Samples stream from disk on sfizz's own threads.
//!
//! Instances are created and destroyed one at a time (see [`setup_lock`]);
//! everything after that is instance-local and runs unlocked.

use std::ffi::{c_char, c_int, CString};
use std::path::Path;
use std::sync::{Mutex, MutexGuard};

use crate::engine::player::Instrument;

/// Creating, loading and freeing a sfizz instance all touch process-wide state
/// that sfizz itself does not lock: the shared sample-reading thread pool
/// (`FilePool.cpp`, read outside its own mutex) and a global map of envelope
/// shapes that every parsed file inserts into and prunes (`FlexEGDescription.
/// cpp`). Automatic assignment loads a whole song's instruments at once, and
/// those concurrent loads corrupted the heap — so they take turns here.
///
/// Playing does not: rendering, notes and controllers stay instance-local and
/// never wait on this lock, so the audio thread is unaffected.
static SFIZZ_SETUP: Mutex<()> = Mutex::new(());

fn setup_lock() -> MutexGuard<'static, ()> {
    // A panic inside sfizz would poison the lock; there is no state to
    // protect beyond "one at a time", so carry on.
    SFIZZ_SETUP.lock().unwrap_or_else(|e| e.into_inner())
}

#[allow(non_camel_case_types)]
#[repr(C)]
struct sfizz_synth_t {
    _private: [u8; 0],
}

extern "C" {
    fn sfizz_create_synth() -> *mut sfizz_synth_t;
    fn sfizz_free(s: *mut sfizz_synth_t);
    fn sfizz_load_file(s: *mut sfizz_synth_t, path: *const c_char) -> bool;
    fn sfizz_set_sample_rate(s: *mut sfizz_synth_t, sample_rate: f32);
    fn sfizz_set_samples_per_block(s: *mut sfizz_synth_t, samples: c_int);
    fn sfizz_set_num_voices(s: *mut sfizz_synth_t, voices: c_int);
    fn sfizz_get_num_regions(s: *mut sfizz_synth_t) -> c_int;
    fn sfizz_render_block(s: *mut sfizz_synth_t, channels: *mut *mut f32, num_channels: c_int, frames: c_int);
    fn sfizz_send_note_on(s: *mut sfizz_synth_t, delay: c_int, note: c_int, velocity: c_int);
    fn sfizz_send_note_off(s: *mut sfizz_synth_t, delay: c_int, note: c_int, velocity: c_int);
    fn sfizz_send_cc(s: *mut sfizz_synth_t, delay: c_int, cc: c_int, value: c_int);
    fn sfizz_send_pitch_wheel(s: *mut sfizz_synth_t, delay: c_int, pitch: c_int);
    fn sfizz_all_sound_off(s: *mut sfizz_synth_t);
    fn sfizz_set_volume(s: *mut sfizz_synth_t, db: f32);
}

/// Largest block the engine renders in one call.
const MAX_FRAMES: usize = 1024;
const VOICES: c_int = 128;

/// Controllers passed through. Many SFZ libraries bind their own controls to
/// other CCs (e.g. 98-101 for mic mixes), which GM files would clobber when
/// setting the pitch-bend range via RPN.
const PASSED_CCS: [u8; 8] = [1, 7, 11, 64, 66, 67, 121, 123];

pub struct SfzInstrument {
    synth: *mut sfizz_synth_t,
    /// Keys currently held, to release them without a sound-off.
    held: [bool; 128],
    /// RPN tracking: the MIDI file's pitch-bend range (semitones). SFZ
    /// regions bend ±2 semitones by default, so wheel values are rescaled.
    rpn: (u8, u8),
    bend_range: f32,
}

// The pointer is only ever used from one thread at a time (see module docs).
unsafe impl Send for SfzInstrument {}

impl SfzInstrument {
    /// Create and load an instrument. Blocking (reads sample preloads).
    pub fn load(path: &Path, sample_rate: u32) -> Result<Self, String> {
        let c_path = CString::new(path.to_string_lossy().as_bytes()).map_err(|e| e.to_string())?;
        let _setup = setup_lock();
        // SAFETY: fresh instance, used on this thread only until returned.
        unsafe {
            let synth = sfizz_create_synth();
            if synth.is_null() {
                return Err("sfizz: could not create synth".into());
            }
            sfizz_set_sample_rate(synth, sample_rate as f32);
            sfizz_set_samples_per_block(synth, MAX_FRAMES as c_int);
            sfizz_set_num_voices(synth, VOICES);
            if !sfizz_load_file(synth, c_path.as_ptr()) || sfizz_get_num_regions(synth) == 0 {
                sfizz_free(synth);
                return Err(format!("sfizz: could not load {}", path.display()));
            }
            Ok(Self { synth, held: [false; 128], rpn: (127, 127), bend_range: 2.0 })
        }
    }

    /// Overall level trim in dB (call before handing to the audio thread).
    pub fn set_gain_db(&mut self, db: f32) {
        unsafe { sfizz_set_volume(self.synth, db.clamp(-40.0, 24.0)) }
    }
}

impl Drop for SfzInstrument {
    fn drop(&mut self) {
        let _setup = setup_lock();
        // SAFETY: we own the instance and nothing else references it.
        unsafe { sfizz_free(self.synth) }
    }
}

impl Instrument for SfzInstrument {
    fn note_on(&mut self, delay: usize, key: u8, velocity: u8) {
        self.held[usize::from(key & 0x7f)] = true;
        unsafe { sfizz_send_note_on(self.synth, delay as c_int, c_int::from(key), c_int::from(velocity)) }
    }

    fn note_off(&mut self, delay: usize, key: u8) {
        self.held[usize::from(key & 0x7f)] = false;
        unsafe { sfizz_send_note_off(self.synth, delay as c_int, c_int::from(key), 64) }
    }

    fn channel_message(&mut self, delay: usize, status: u8, data1: u8, data2: u8) {
        unsafe {
            match (status, data1) {
                (0xB0, 101) => self.rpn.0 = data2,
                (0xB0, 100) => self.rpn.1 = data2,
                (0xB0, 6) if self.rpn == (0, 0) => self.bend_range = f32::from(data2.max(1)),
                (0xB0, cc) if PASSED_CCS.contains(&cc) => {
                    sfizz_send_cc(self.synth, delay as c_int, c_int::from(cc), c_int::from(data2))
                }
                (0xE0, _) => {
                    let raw = (i32::from(data2) << 7) | i32::from(data1);
                    let scaled = (raw - 8192) as f32 * self.bend_range / 2.0;
                    sfizz_send_pitch_wheel(self.synth, delay as c_int, (scaled as c_int).clamp(-8191, 8191));
                }
                _ => {}
            }
        }
    }

    fn all_notes_off(&mut self, immediate: bool) {
        if immediate {
            self.held = [false; 128];
            unsafe { sfizz_all_sound_off(self.synth) }
            return;
        }
        for key in 0..128u8 {
            if self.held[usize::from(key)] {
                self.note_off(0, key);
            }
        }
        // Let the release phase play out but lift the pedal too.
        unsafe { sfizz_send_cc(self.synth, 0, 64, 0) }
    }

    fn reset(&mut self) {
        self.held = [false; 128];
        self.rpn = (127, 127);
        self.bend_range = 2.0;
        unsafe {
            sfizz_all_sound_off(self.synth);
            // GM "reset all controllers" plus defaults sfizz does not restore.
            sfizz_send_cc(self.synth, 0, 121, 0);
            sfizz_send_cc(self.synth, 0, 7, 100);
            sfizz_send_cc(self.synth, 0, 11, 127);
            sfizz_send_cc(self.synth, 0, 64, 0);
            sfizz_send_pitch_wheel(self.synth, 0, 0);
        }
    }

    fn render(&mut self, left: &mut [f32], right: &mut [f32]) -> bool {
        let frames = left.len().min(right.len()).min(MAX_FRAMES);
        let mut channels = [left.as_mut_ptr(), right.as_mut_ptr()];
        // SAFETY: both buffers hold at least `frames` samples.
        unsafe { sfizz_render_block(self.synth, channels.as_mut_ptr(), 2, frames as c_int) };
        left[..frames].iter().chain(&right[..frames]).any(|v| v.abs() > 1e-9)
    }
}

/// Several SFZ instruments sharing the keyboard by range, e.g. contrabass /
/// cello / viola / violin sections standing in for one GM "strings" patch.
pub struct SfzSplit {
    parts: Vec<(u8, u8, SfzInstrument)>,
    scratch: [Vec<f32>; 2],
}

impl SfzSplit {
    /// `parts`: (lowest key, highest key, .sfz path); first match wins.
    pub fn load(parts: &[(u8, u8, std::path::PathBuf)], sample_rate: u32) -> Result<Self, String> {
        let parts = parts
            .iter()
            .map(|(lo, hi, path)| Ok((*lo, *hi, SfzInstrument::load(path, sample_rate)?)))
            .collect::<Result<Vec<_>, String>>()?;
        if parts.is_empty() {
            return Err("empty split".into());
        }
        Ok(Self { parts, scratch: [vec![0.0; MAX_FRAMES], vec![0.0; MAX_FRAMES]] })
    }

    pub fn set_gain_db(&mut self, db: f32) {
        for (_, _, p) in &mut self.parts {
            p.set_gain_db(db);
        }
    }

    fn part(&mut self, key: u8) -> Option<&mut SfzInstrument> {
        self.parts.iter_mut().find(|(lo, hi, _)| (*lo..=*hi).contains(&key)).map(|(_, _, p)| p)
    }
}

impl Instrument for SfzSplit {
    fn note_on(&mut self, delay: usize, key: u8, velocity: u8) {
        if let Some(p) = self.part(key) {
            p.note_on(delay, key, velocity);
        }
    }
    fn note_off(&mut self, delay: usize, key: u8) {
        if let Some(p) = self.part(key) {
            p.note_off(delay, key);
        }
    }
    fn channel_message(&mut self, delay: usize, status: u8, data1: u8, data2: u8) {
        for (_, _, p) in &mut self.parts {
            p.channel_message(delay, status, data1, data2);
        }
    }
    fn all_notes_off(&mut self, immediate: bool) {
        for (_, _, p) in &mut self.parts {
            p.all_notes_off(immediate);
        }
    }
    fn reset(&mut self) {
        for (_, _, p) in &mut self.parts {
            p.reset();
        }
    }
    fn render(&mut self, left: &mut [f32], right: &mut [f32]) -> bool {
        let n = left.len().min(MAX_FRAMES);
        left[..n].fill(0.0);
        right[..n].fill(0.0);
        let mut any = false;
        let [sl, sr] = &mut self.scratch;
        for (_, _, p) in &mut self.parts {
            if p.render(&mut sl[..n], &mut sr[..n]) {
                any = true;
                for i in 0..n {
                    left[i] += sl[i];
                    right[i] += sr[i];
                }
            }
        }
        any
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A one-region instrument built from a generated sine WAV.
    fn sine_instrument(dir: &Path) -> std::path::PathBuf {
        let sr = 48_000u32;
        let samples: Vec<i16> = (0..sr)
            .map(|i| ((i as f32 * 440.0 * std::f32::consts::TAU / sr as f32).sin() * 12000.0) as i16)
            .collect();
        let data: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&sr.to_le_bytes());
        wav.extend_from_slice(&(sr * 2).to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&(data.len() as u32).to_le_bytes());
        wav.extend_from_slice(&data);
        std::fs::write(dir.join("sine.wav"), wav).unwrap();
        let sfz = dir.join("sine.sfz");
        std::fs::write(&sfz, "<region> sample=sine.wav pitch_keycenter=69 loop_mode=loop_continuous ampeg_release=0.05\n")
            .unwrap();
        sfz
    }

    #[test]
    fn plays_and_releases_a_note() {
        let dir = std::env::temp_dir().join(format!("otomo-sfz-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut inst = SfzInstrument::load(&sine_instrument(&dir), 48_000).expect("load");
        let (mut l, mut r) = (vec![0f32; 1024], vec![0f32; 1024]);
        assert!(!inst.render(&mut l, &mut r), "silent before any note");
        inst.note_on(0, 69, 100);
        let mut energy = 0.0;
        for _ in 0..10 {
            inst.render(&mut l, &mut r);
            energy += l.iter().map(|v| v * v).sum::<f32>();
        }
        assert!(energy > 1.0, "note audible: {energy}");
        inst.note_off(0, 69);
        for _ in 0..20 {
            inst.render(&mut l, &mut r);
        }
        assert!(!inst.render(&mut l, &mut r), "released");
        assert!(SfzInstrument::load(&dir.join("missing.sfz"), 48_000).is_err());

        // Split: keys above 70 go to a second (identical) part only.
        let sfz = dir.join("sine.sfz");
        let mut split = SfzSplit::load(&[(0, 70, sfz.clone()), (71, 127, sfz)], 48_000).unwrap();
        split.note_on(0, 80, 100);
        split.render(&mut l, &mut r);
        assert!(split.parts[1].2.held[80] && !split.parts[0].2.held[80]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Auto-assignment loads every track's instrument at once. sfizz's
    /// process-wide state is not built for that, so [`setup_lock`] serialises
    /// it; without the lock this corrupts the heap and aborts the process.
    /// Every file uses different envelope shapes, which is what makes the
    /// threads collide in sfizz's global shape map.
    #[test]
    fn concurrent_loads_do_not_corrupt_the_heap() {
        let dir = std::env::temp_dir().join(format!("otomo-sfz-par-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        sine_instrument(&dir);
        let files: Vec<_> = (0..8)
            .map(|t| {
                let regions: String = (0..16)
                    .map(|i| {
                        let shape = (t * 16 + i) as f32 * 0.37 - 20.0;
                        format!(
                            "<region> sample=sine.wav pitch_keycenter={} ampeg_release=0.05 \
                             eg1_time1=0.1 eg1_level1=1 eg1_shape1={shape} eg1_ampeg=1\n",
                            40 + i
                        )
                    })
                    .collect();
                let path = dir.join(format!("shapes{t}.sfz"));
                std::fs::write(&path, regions).unwrap();
                path
            })
            .collect();
        let threads: Vec<_> = files
            .into_iter()
            .map(|sfz| {
                std::thread::spawn(move || {
                    for _ in 0..24 {
                        // Load, play a note, and free while others do the same.
                        let mut inst = SfzInstrument::load(&sfz, 48_000).expect("load");
                        inst.note_on(0, 45, 100);
                        let (mut l, mut r) = (vec![0f32; 256], vec![0f32; 256]);
                        inst.render(&mut l, &mut r);
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().expect("no thread crashed");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod bench {
    //! Diagnostics against the libraries installed by the app.
    //! `cargo test --release -- --ignored sfz_bench --nocapture`

    use super::*;
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    fn lib(rel: &str) -> PathBuf {
        PathBuf::from(std::env::var("HOME").unwrap())
            .join("Library/Application Support/io.github.hibiki1213.otomo/libraries")
            .join(rel)
    }

    fn energy_of(inst: &mut dyn Instrument, keys: &[u8], vel: u8) -> f32 {
        for &k in keys {
            inst.note_on(0, k, vel);
        }
        let (mut l, mut r) = (vec![0f32; 1024], vec![0f32; 1024]);
        let mut e = 0.0;
        let mut peak = 0f32;
        for _ in 0..40 {
            inst.render(&mut l, &mut r);
            e += l.iter().chain(&r).map(|v| v * v).sum::<f32>();
            peak = l.iter().chain(&r).fold(peak, |m, v| m.max(v.abs()));
        }
        eprintln!("    peak {peak:.3}");
        e
    }

    /// What automatic assignment does on song load: every track's instrument
    /// at once, from real libraries. Without [`setup_lock`] this aborts the
    /// process within a few hundred rounds ("memory corruption of free
    /// block"); with it, 3600 loads pass. Takes ~20 s.
    /// `cargo test --release -- --ignored sfz_parallel_load --nocapture`
    #[test]
    #[ignore]
    fn sfz_parallel_load() {
        let vsco = lib("vsco2-ce");
        if !vsco.exists() {
            return;
        }
        let mut jobs: Vec<PathBuf> = std::fs::read_dir(&vsco)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "sfz"))
            .collect();
        jobs.sort();
        jobs.truncate(12);
        assert!(jobs.len() > 3, "need a few VSCO instruments installed");
        for round in 0..300 {
            let t = Instant::now();
            let threads: Vec<_> = jobs
                .iter()
                .cloned()
                .map(|p| {
                    std::thread::spawn(move || {
                        let t = Instant::now();
                        let inst = SfzInstrument::load(&p, 48_000);
                        (p, inst.is_ok(), t.elapsed())
                    })
                })
                .collect();
            let mut slowest = Duration::ZERO;
            for th in threads {
                let (p, ok, took) = th.join().expect("a loader thread crashed");
                assert!(ok, "load {}", p.display());
                slowest = slowest.max(took);
            }
            if round % 50 == 0 {
                eprintln!("round {round}: {} instruments in {:?} (slowest {slowest:?})", jobs.len(), t.elapsed());
            }
        }
    }

    #[test]
    #[ignore]
    fn sfz_bench() {
        let vsco = lib("vsco2-ce");
        if !vsco.exists() {
            return;
        }
        let strings = |n: &str| vsco.join(n);
        let split_parts = vec![
            (0u8, 38u8, strings("ContrabassSusVB.sfz")),
            (39, 54, strings("CelloEnsSusVib.sfz")),
            (55, 61, strings("ViolaEnsSusVib.sfz")),
            (62, 127, strings("ViolinEnsSusVib.sfz")),
        ];

        // 1) Cost of idle instances: 20 string splits (80 sfizz) rendering
        //    64-frame sub-blocks, as the engine does.
        let t = Instant::now();
        let mut splits: Vec<SfzSplit> = (0..20).map(|_| SfzSplit::load(&split_parts, 48_000).unwrap()).collect();
        eprintln!("load 20 splits (80 sfizz): {:?}", t.elapsed());
        let (mut l, mut r) = (vec![0f32; 64], vec![0f32; 64]);
        let blocks = 200; // engine blocks of 1024 frames
        let t = Instant::now();
        for _ in 0..blocks {
            for s in &mut splits {
                for _ in 0..16 {
                    s.render(&mut l, &mut r);
                }
            }
        }
        let per_block = t.elapsed().as_secs_f64() / blocks as f64;
        eprintln!("idle, 64-frame calls: {:.2} ms per 21.3 ms block ({:.0}% of realtime)", per_block * 1e3, per_block / 0.02133 * 100.0);

        let (mut l2, mut r2) = (vec![0f32; 1024], vec![0f32; 1024]);
        let t = Instant::now();
        for _ in 0..blocks {
            for s in &mut splits {
                s.render(&mut l2, &mut r2);
            }
        }
        let per_block = t.elapsed().as_secs_f64() / blocks as f64;
        eprintln!("idle, 1024-frame calls: {:.2} ms per block ({:.0}%)", per_block * 1e3, per_block / 0.02133 * 100.0);

        // Busy: 4-note chord on each split.
        for s in &mut splits {
            for k in [48u8, 60, 67, 76] {
                s.note_on(0, k, 100);
            }
        }
        let t = Instant::now();
        for _ in 0..blocks {
            for s in &mut splits {
                for _ in 0..16 {
                    s.render(&mut l, &mut r);
                }
            }
        }
        let per_block = t.elapsed().as_secs_f64() / blocks as f64;
        eprintln!("busy (80 voices), 64-frame calls: {:.2} ms per block ({:.0}%)", per_block * 1e3, per_block / 0.02133 * 100.0);
        drop(splits);

        // 2) Loudness vs the SoundFont reference measured the same way
        //    (see engine::tests::print_soundfont_note_energy, 40 blocks).
        eprintln!("strings split, chord C3 C4 G4 E5 v100:");
        let mut s = SfzSplit::load(&split_parts, 48_000).unwrap();
        let e = energy_of(&mut s, &[48, 60, 67, 76], 100);
        eprintln!("    energy {e:.1}");
        eprintln!("salamander, C4 v110:");
        let mut p = SfzInstrument::load(
            &lib("salamander-piano/SalamanderGrandPiano-V3+20200602.sfz"),
            48_000,
        )
        .unwrap();
        let e = energy_of(&mut p, &[60], 110);
        eprintln!("    energy {e:.1}");
        for (name, key) in [("TrumpetSusVib.sfz", 67u8), ("FluteSusVib.sfz", 72), ("FHornSus.sfz", 60), ("Timpani.sfz", 48), ("Harp.sfz", 60)] {
            let mut i = SfzInstrument::load(&vsco.join(name), 48_000).unwrap();
            let e = energy_of(&mut i, &[key], 110);
            eprintln!("{name} key {key} v110: energy {e:.1}");
        }
    }
}
