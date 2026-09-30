//! Instruments other than the built-in SoundFont synthesizer.

#[cfg(target_os = "macos")]
pub mod audio_unit;
pub mod sfz;
pub mod synth;

use serde::{Deserialize, Serialize};

/// What a track plays through, as chosen in the UI.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum InstrumentChoice {
    /// The built-in General MIDI SoundFont.
    Sf2,
    /// An .sfz file (from a downloaded library or picked by the user).
    Sfz {
        path: String,
        /// Level trim in dB.
        #[serde(default, rename = "gainDb")]
        gain_db: f32,
    },
    /// Several .sfz files splitting the keyboard (first matching range wins).
    SfzSplit {
        parts: Vec<SfzPart>,
        #[serde(default, rename = "gainDb")]
        gain_db: f32,
    },
    /// An AudioUnit instrument, by `audio_unit::PluginInfo::id`.
    AudioUnit { id: String },
    /// A preset of the built-in synthesizer (`otomo_synth::PRESETS`).
    Synth { preset: String },
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct SfzPart {
    pub path: String,
    pub lo: u8,
    pub hi: u8,
}
