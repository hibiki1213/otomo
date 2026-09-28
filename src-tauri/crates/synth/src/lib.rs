//! midiss's own synthesizer: the EDM sounds a General MIDI SoundFont does
//! badly — supersaws, plucks, reese and wobble basses, 808s, acid lines — and
//! a drum machine, all computed rather than sampled, so nothing is
//! downloaded and every preset is available the moment the app starts.
//!
//! The host drives it like any MIDI instrument: notes and controllers with a
//! frame offset into the next [`Synth::render`] call.

mod drums;
pub mod dsp;
mod fx;
pub mod presets;
mod voice;

use drums::Hit;
use dsp::db_to_gain;
use fx::Effects;
pub use presets::{for_program, Preset, PRESETS};
use presets::Kind;
use voice::{Controls, Voice};

/// Voices sounding at once (per instrument). Supersaw stacks are expensive,
/// and eight-note chords with release tails fit comfortably.
const POLYPHONY: usize = 16;
const DRUM_POLYPHONY: usize = 32;

#[derive(Clone, Copy, Debug)]
enum Event {
    On(u8, u8),
    Off(u8),
    Cc(u8, u8),
    /// Pitch bend, -1 … 1.
    Bend(f32),
    AllOff(bool),
}

pub struct Synth {
    preset: &'static Preset,
    sample_rate: f32,
    voices: Vec<Voice>,
    hits: Vec<Hit>,
    events: Vec<(usize, Event)>,
    serial: u64,
    /// Keys held, newest last (mono presets fall back along it).
    held: Vec<(u8, u8)>,
    sustain: bool,
    bend: f32,
    bend_range: f32,
    rpn: (u8, u8),
    wheel: f32,
    brightness: f32,
    volume: f32,
    expression: f32,
    /// Channel level actually applied, smoothed toward volume × expression.
    level: f32,
    output_gain: f32,
    fx: Effects,
    /// Samples since anything was audible, to report silence honestly while
    /// an echo is still dying away.
    quiet: usize,
}

impl Synth {
    /// `None` for an unknown preset id.
    pub fn new(preset_id: &str, sample_rate: u32) -> Option<Self> {
        let preset = presets::find(preset_id)?;
        let sr = sample_rate as f32;
        let fx = match preset.kind {
            Kind::Tonal(p) => Effects::new(&p.fx, sr),
            Kind::Drums => Effects::new(&presets::Fx::DRY, sr),
        };
        Some(Self {
            preset,
            sample_rate: sr,
            voices: Vec::with_capacity(POLYPHONY),
            hits: Vec::with_capacity(DRUM_POLYPHONY),
            events: Vec::with_capacity(256),
            serial: 0,
            held: Vec::with_capacity(16),
            sustain: false,
            bend: 0.0,
            bend_range: 2.0,
            rpn: (127, 127),
            wheel: 0.0,
            brightness: 0.0,
            volume: 100.0 / 127.0,
            expression: 1.0,
            level: gm_level(100.0 / 127.0, 1.0) * db_to_gain(preset.gain_db),
            output_gain: db_to_gain(preset.gain_db),
            fx,
            quiet: 0,
        })
    }

    pub fn preset(&self) -> &'static Preset {
        self.preset
    }

    pub fn note_on(&mut self, delay: usize, key: u8, velocity: u8) {
        if velocity == 0 {
            self.events.push((delay, Event::Off(key)));
        } else {
            self.events.push((delay, Event::On(key & 0x7f, velocity.min(127))));
        }
    }

    pub fn note_off(&mut self, delay: usize, key: u8) {
        self.events.push((delay, Event::Off(key & 0x7f)));
    }

    pub fn control_change(&mut self, delay: usize, cc: u8, value: u8) {
        self.events.push((delay, Event::Cc(cc, value)));
    }

    /// 14-bit pitch bend as sent on the wire (0 … 16383, centre 8192).
    pub fn pitch_bend(&mut self, delay: usize, value: u16) {
        let v = (f32::from(value.min(16383)) - 8192.0) / 8192.0;
        self.events.push((delay, Event::Bend(v)));
    }

    /// Release every note; `immediate` also cuts the tails (and the echo).
    pub fn all_notes_off(&mut self, immediate: bool) {
        self.events.clear();
        self.apply(Event::AllOff(immediate));
    }

    /// Controllers back to their power-on state (before a seek replays them).
    pub fn reset(&mut self) {
        self.all_notes_off(true);
        self.sustain = false;
        self.bend = 0.0;
        self.bend_range = 2.0;
        self.rpn = (127, 127);
        self.wheel = 0.0;
        self.brightness = 0.0;
        self.volume = 100.0 / 127.0;
        self.expression = 1.0;
    }

    fn apply(&mut self, event: Event) {
        match event {
            Event::On(key, vel) => self.start(key, vel),
            Event::Off(key) => self.stop(key),
            Event::Bend(v) => self.bend = v * self.bend_range,
            Event::Cc(cc, value) => self.controller(cc, value),
            Event::AllOff(immediate) => {
                self.held.clear();
                if immediate {
                    self.voices.clear();
                    self.hits.clear();
                    self.fx.clear();
                } else {
                    for v in &mut self.voices {
                        v.release();
                    }
                }
            }
        }
    }

    fn controller(&mut self, cc: u8, value: u8) {
        let v = f32::from(value) / 127.0;
        match cc {
            1 => self.wheel = v,
            7 => self.volume = v,
            11 => self.expression = v,
            // Brightness: ±2 octaves of cutoff around the centre.
            74 => self.brightness = (v - 0.5) * 4.0,
            64 => {
                self.sustain = value >= 64;
                if !self.sustain {
                    for voice in &mut self.voices {
                        if voice.sustained {
                            voice.release();
                        }
                    }
                }
            }
            101 => self.rpn.0 = value,
            100 => self.rpn.1 = value,
            6 if self.rpn == (0, 0) => self.bend_range = f32::from(value.max(1)),
            120 => self.apply(Event::AllOff(true)),
            121 => {
                self.wheel = 0.0;
                self.expression = 1.0;
                self.bend = 0.0;
                self.brightness = 0.0;
            }
            123 => self.apply(Event::AllOff(false)),
            _ => {}
        }
    }

    fn start(&mut self, key: u8, vel: u8) {
        self.serial += 1;
        let serial = self.serial;
        let patch = match self.preset.kind {
            Kind::Drums => {
                if let Some(hit) = Hit::new(key, vel, serial, self.sample_rate) {
                    // A closed or pedal hat shuts an open one.
                    if hit.is_hat() {
                        for h in self.hits.iter_mut().filter(|h| h.is_open_hat()) {
                            h.choke();
                        }
                    }
                    if self.hits.len() >= DRUM_POLYPHONY {
                        self.hits.remove(0);
                    }
                    self.hits.push(hit);
                }
                return;
            }
            Kind::Tonal(p) => p,
        };
        self.held.retain(|(k, _)| *k != key);
        self.held.push((key, vel));

        if patch.mono {
            if let Some(voice) = self.voices.iter_mut().find(|v| !v.done()) {
                if voice.held || voice.sustained {
                    voice.glide_to(key, vel);
                } else {
                    voice.retrigger(key, vel, serial);
                }
                return;
            }
            self.voices.clear();
            self.voices.push(Voice::new(&patch, key, vel, serial, self.sample_rate));
            return;
        }

        // Retriggering a ringing key replaces it rather than doubling it.
        for v in self.voices.iter_mut().filter(|v| v.key == key && !v.is_releasing()) {
            v.release();
        }
        if self.voices.len() >= POLYPHONY {
            // Steal the oldest released note, else the oldest note.
            let victim = self
                .voices
                .iter()
                .enumerate()
                .filter(|(_, v)| v.is_releasing())
                .min_by_key(|(_, v)| v.serial)
                .or_else(|| self.voices.iter().enumerate().min_by_key(|(_, v)| v.serial))
                .map(|(i, _)| i);
            if let Some(i) = victim {
                self.voices.swap_remove(i);
            }
        }
        self.voices.push(Voice::new(&patch, key, vel, serial, self.sample_rate));
    }

    fn stop(&mut self, key: u8) {
        let Kind::Tonal(patch) = self.preset.kind else { return };
        self.held.retain(|(k, _)| *k != key);
        if patch.mono {
            // Fall back to the previous held key, as a mono synth does.
            if let Some(voice) = self.voices.iter_mut().find(|v| v.key == key && v.held) {
                match self.held.last() {
                    Some(&(prev, vel)) => voice.glide_to(prev, vel),
                    None if self.sustain => voice.sustained = true,
                    None => voice.release(),
                }
            }
            return;
        }
        for v in self.voices.iter_mut().filter(|v| v.key == key && v.held) {
            if self.sustain {
                v.held = false;
                v.sustained = true;
            } else {
                v.release();
            }
        }
    }

    /// Render `left.len()` frames, overwriting both buffers. Returns false
    /// when the output is silent (no notes and no echo left).
    pub fn render(&mut self, left: &mut [f32], right: &mut [f32]) -> bool {
        let n = left.len().min(right.len());
        left[..n].fill(0.0);
        right[..n].fill(0.0);

        let mut events = std::mem::take(&mut self.events);
        events.sort_by_key(|(at, _)| *at);
        let mut pos = 0;
        let mut next = 0;
        while pos < n {
            while next < events.len() && events[next].0 <= pos {
                self.apply(events[next].1);
                next += 1;
            }
            let end = events.get(next).map_or(n, |(at, _)| (*at).clamp(pos + 1, n));
            self.render_voices(&mut left[pos..end], &mut right[pos..end]);
            pos = end;
        }
        // Events addressed past the end of this block wait for the next.
        let late: Vec<_> = events.drain(next..).map(|(at, e)| (at.saturating_sub(n), e)).collect();
        events.clear();
        self.events = events;
        self.events.extend(late);

        let target = gm_level(self.volume, self.expression) * self.output_gain;
        let step = (target - self.level) / n.max(1) as f32;
        let mut peak = 0.0f32;
        for i in 0..n {
            self.level += step;
            left[i] *= self.level;
            right[i] *= self.level;
        }
        self.level = target;
        self.fx.process(&mut left[..n], &mut right[..n]);
        for i in 0..n {
            left[i] = dsp::limit(left[i]);
            right[i] = dsp::limit(right[i]);
            peak = peak.max(left[i].abs()).max(right[i].abs());
        }

        let sounding = !self.voices.is_empty() || !self.hits.is_empty();
        if sounding || peak > 1.0e-5 {
            self.quiet = 0;
            true
        } else {
            self.quiet += n;
            // One more silent block, so the host sees the tail reach zero.
            self.quiet <= n
        }
    }

    fn render_voices(&mut self, left: &mut [f32], right: &mut [f32]) {
        match self.preset.kind {
            Kind::Tonal(patch) => {
                let controls = Controls {
                    sample_rate: self.sample_rate,
                    bend: self.bend,
                    wheel: self.wheel,
                    brightness: self.brightness,
                };
                for v in &mut self.voices {
                    v.render(&patch, &controls, left, right);
                }
                self.voices.retain(|v| !v.done());
            }
            Kind::Drums => {
                for h in &mut self.hits {
                    h.render(self.sample_rate, left, right);
                }
                self.hits.retain(|h| !h.done());
            }
        }
    }
}

/// Volume × expression the way General MIDI asks: squared, so CC7 at 64 is
/// about -12 dB rather than -6.
fn gm_level(volume: f32, expression: f32) -> f32 {
    let v = volume * expression;
    v * v
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: u32 = 48_000;

    fn play(id: &str, keys: &[u8], seconds: f32) -> (Vec<f32>, Vec<f32>) {
        let mut s = Synth::new(id, SR).unwrap();
        for &k in keys {
            s.note_on(0, k, 110);
        }
        let blocks = (seconds * SR as f32 / 1024.0) as usize;
        let (mut l, mut r) = (Vec::new(), Vec::new());
        let (mut bl, mut br) = (vec![0.0; 1024], vec![0.0; 1024]);
        for b in 0..blocks {
            if b == blocks / 2 {
                for &k in keys {
                    s.note_off(0, k);
                }
            }
            s.render(&mut bl, &mut br);
            l.extend_from_slice(&bl);
            r.extend_from_slice(&br);
        }
        (l, r)
    }

    /// Same measure the app uses to line SFZ libraries up with the
    /// SoundFont: energy of 20 blocks of 1024 after one note at velocity 110.
    fn energy(id: &str, key: u8) -> f32 {
        let mut s = Synth::new(id, SR).unwrap();
        s.note_on(0, key, 110);
        let (mut l, mut r) = (vec![0.0; 1024], vec![0.0; 1024]);
        let mut e = 0.0;
        for _ in 0..20 {
            s.render(&mut l, &mut r);
            e += l.iter().chain(&r).map(|v| v * v).sum::<f32>();
        }
        e
    }

    fn reference_key(p: &Preset) -> u8 {
        if matches!(p.kind, Kind::Drums) {
            36
        } else if p.id.contains("bass") {
            36
        } else {
            60
        }
    }

    #[test]
    fn every_preset_sounds_stays_finite_and_ends() {
        for p in PRESETS {
            let key = reference_key(p);
            let (l, r) = play(p.id, &[key, key + 7], 6.0);
            assert!(l.iter().chain(&r).all(|v| v.is_finite()), "{} produced NaN", p.id);
            let peak = l.iter().chain(&r).fold(0.0f32, |a, v| a.max(v.abs()));
            assert!(peak > 0.02, "{} is silent (peak {peak})", p.id);
            assert!(peak <= 1.0, "{} clips (peak {peak})", p.id);
            let tail = &l[l.len() - 4096..];
            assert!(tail.iter().all(|v| v.abs() < 1e-3), "{} never fades out", p.id);
        }
    }

    #[test]
    fn presets_sit_near_the_soundfont_level() {
        // The GM SoundFont's piano C4 measures ~63 and its drums ~49 here.
        for p in PRESETS {
            let e = energy(p.id, reference_key(p));
            assert!((12.0..=240.0).contains(&e), "{}: energy {e:.1} is far from the SoundFont's", p.id);
        }
    }

    #[test]
    #[ignore]
    fn print_levels() {
        for p in PRESETS {
            let e = energy(p.id, reference_key(p));
            let trim = 10.0 * (60.0f32 / e).log10();
            eprintln!("{:16} energy {:7.1}  trim to 60: {:+.1} dB (now {:+.1})", p.id, e, trim + p.gain_db, p.gain_db);
        }
    }

    #[test]
    fn mono_preset_glides_instead_of_stacking() {
        let mut s = Synth::new("acid-bass", SR).unwrap();
        s.note_on(0, 36, 100);
        s.note_on(100, 43, 100);
        let (mut l, mut r) = (vec![0.0; 1024], vec![0.0; 1024]);
        s.render(&mut l, &mut r);
        assert_eq!(s.voices.len(), 1);
        s.note_off(0, 43);
        s.render(&mut l, &mut r);
        assert_eq!(s.voices[0].key, 36, "falls back to the held key");
    }

    #[test]
    fn sustain_pedal_holds_released_keys() {
        let mut s = Synth::new("warm-pad", SR).unwrap();
        s.control_change(0, 64, 127);
        s.note_on(0, 60, 100);
        s.note_off(10, 60);
        let (mut l, mut r) = (vec![0.0; 1024], vec![0.0; 1024]);
        s.render(&mut l, &mut r);
        assert!(!s.voices[0].is_releasing());
        s.control_change(0, 64, 0);
        s.render(&mut l, &mut r);
        assert!(s.voices[0].is_releasing());
    }

    #[test]
    fn realtime_budget() {
        // Eight supersaw notes for one second of audio must render far faster
        // than real time, or dense arrangements will drop out.
        let mut s = Synth::new("supersaw-chord", SR).unwrap();
        for k in [48, 52, 55, 60, 64, 67, 72, 76] {
            s.note_on(0, k, 100);
        }
        let (mut l, mut r) = (vec![0.0; 1024], vec![0.0; 1024]);
        let start = std::time::Instant::now();
        for _ in 0..47 {
            s.render(&mut l, &mut r);
        }
        let took = start.elapsed().as_secs_f32();
        eprintln!("8-note supersaw chord: {:.1}x real time", 1.0 / took);
        if !cfg!(debug_assertions) {
            assert!(took < 0.2, "{took}s for 1s of audio");
        }
    }
}
