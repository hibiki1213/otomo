//! The built-in synthesizer (`otomo-synth`) as a track instrument.

use otomo_synth::Synth;

use crate::engine::player::Instrument;
use crate::i18n::tr;

pub struct SynthInstrument(Synth);

impl SynthInstrument {
    pub fn load(preset: &str, sample_rate: u32) -> Result<Self, String> {
        Synth::new(preset, sample_rate)
            .map(Self)
            .ok_or_else(|| tr!("{preset}: 内蔵シンセにそのプリセットはありません", "{preset}: the built-in synth has no such preset"))
    }
}

impl Instrument for SynthInstrument {
    fn note_on(&mut self, delay: usize, key: u8, velocity: u8) {
        self.0.note_on(delay, key, velocity);
    }

    fn note_off(&mut self, delay: usize, key: u8) {
        self.0.note_off(delay, key);
    }

    fn channel_message(&mut self, delay: usize, status: u8, data1: u8, data2: u8) {
        match status {
            0xB0 => self.0.control_change(delay, data1, data2),
            0xE0 => self.0.pitch_bend(delay, (u16::from(data2) << 7) | u16::from(data1)),
            _ => {}
        }
    }

    fn all_notes_off(&mut self, immediate: bool) {
        self.0.all_notes_off(immediate);
    }

    fn reset(&mut self) {
        self.0.reset();
    }

    fn render(&mut self, left: &mut [f32], right: &mut [f32]) -> bool {
        self.0.render(left, right)
    }
}
