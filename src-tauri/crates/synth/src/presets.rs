//! The sounds. Each preset is a small patch for the one voice architecture in
//! `voice.rs` — two oscillators with unison, a sub, a filter with its own
//! envelope, an LFO — plus the effects in `fx.rs`.

use crate::dsp::{AdsrSpec, FilterMode, Wave};

#[derive(Clone, Copy, Debug)]
pub struct Osc {
    pub wave: Wave,
    pub level: f32,
    /// Transposition in semitones.
    pub semitones: f32,
    /// Fine tune in cents.
    pub cents: f32,
    /// Stacked copies (1 … 7), spread in pitch by `detune` cents and across
    /// the stereo field by `spread` (0 … 1). Seven detuned saws = supersaw.
    pub unison: usize,
    pub detune: f32,
    pub spread: f32,
}

impl Osc {
    pub const OFF: Osc = Osc::new(Wave::Sine, 0.0);

    pub const fn new(wave: Wave, level: f32) -> Self {
        Self { wave, level, semitones: 0.0, cents: 0.0, unison: 1, detune: 0.0, spread: 0.0 }
    }

    pub const fn pitch(mut self, semitones: f32, cents: f32) -> Self {
        self.semitones = semitones;
        self.cents = cents;
        self
    }

    pub const fn unison(mut self, voices: usize, detune: f32, spread: f32) -> Self {
        self.unison = voices;
        self.detune = detune;
        self.spread = spread;
        self
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Filter {
    pub mode: FilterMode,
    pub cutoff: f32,
    pub resonance: f32,
    /// Filter envelope depth, in octaves above `cutoff`.
    pub env_octaves: f32,
    pub env: AdsrSpec,
    /// 0 = cutoff fixed, 1 = cutoff follows the key one-for-one.
    pub key_track: f32,
    /// How much velocity opens the filter (0 … 1), for accents.
    pub velocity: f32,
    /// Two filters in series: 24 dB/oct instead of 12.
    pub steep: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LfoTarget {
    None,
    /// Depth in octaves.
    Cutoff,
    /// Depth in semitones (vibrato).
    Pitch,
    /// Depth 0 … 1 (tremolo; with a square wave, a gate).
    Amp,
}

#[derive(Clone, Copy, Debug)]
pub struct Lfo {
    pub target: LfoTarget,
    pub wave: Wave,
    pub rate: f32,
    pub depth: f32,
    /// Seconds before it fades in (delayed vibrato).
    pub delay: f32,
    /// Extra depth at full mod wheel (CC1), in the target's unit.
    pub wheel: f32,
}

impl Lfo {
    pub const NONE: Lfo = Lfo { target: LfoTarget::None, wave: Wave::Sine, rate: 1.0, depth: 0.0, delay: 0.0, wheel: 0.0 };
}

#[derive(Clone, Copy, Debug)]
pub struct Fx {
    /// Pre-filter saturation: 1 = clean, 2 – 4 = gritty.
    pub drive: f32,
    pub chorus: f32,
    /// Ping-pong echo: seconds, feedback, wet level.
    pub delay_time: f32,
    pub delay_feedback: f32,
    pub delay_mix: f32,
}

impl Fx {
    pub const DRY: Fx = Fx { drive: 1.0, chorus: 0.0, delay_time: 0.3, delay_feedback: 0.0, delay_mix: 0.0 };
}

#[derive(Clone, Copy, Debug)]
pub struct Patch {
    pub osc1: Osc,
    pub osc2: Osc,
    /// Sine one octave below the note.
    pub sub: f32,
    pub noise: f32,
    /// Two-operator FM on osc1 (a sine): modulator ratio, index at the attack,
    /// seconds for the index to fall away. `None` for plain subtractive.
    pub fm: Option<(f32, f32, f32)>,
    pub filter: Filter,
    pub amp: AdsrSpec,
    /// Pitch sweep at the attack: semitones, seconds to settle (808s, zaps).
    pub pitch_env: (f32, f32),
    pub lfo: Lfo,
    /// One note at a time, gliding between held notes (acid, 808s).
    pub mono: bool,
    pub glide: f32,
    /// 0 = every note equally loud, 1 = fully velocity-sensitive.
    pub velocity: f32,
    pub fx: Fx,
}

#[derive(Clone, Copy, Debug)]
pub enum Kind {
    Tonal(Patch),
    Drums,
}

pub struct Preset {
    pub id: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    /// General MIDI programs this preset is the better stand-in for.
    pub programs: &'static [u8],
    pub kind: Kind,
    /// Output trim in dB, so every preset sits at the SoundFont's loudness
    /// (see `tests::print_levels`).
    pub gain_db: f32,
}

const OPEN: Filter = Filter {
    mode: FilterMode::LowPass,
    cutoff: 18_000.0,
    resonance: 0.0,
    env_octaves: 0.0,
    env: AdsrSpec::new(0.001, 0.1, 1.0, 0.1),
    key_track: 0.0,
    velocity: 0.0,
    steep: false,
};

const fn lowpass(cutoff: f32, resonance: f32, env_octaves: f32, env: AdsrSpec) -> Filter {
    Filter {
        mode: FilterMode::LowPass,
        cutoff,
        resonance,
        env_octaves,
        env,
        key_track: 0.5,
        velocity: 0.3,
        steep: false,
    }
}

const BASE: Patch = Patch {
    osc1: Osc::new(Wave::Saw, 1.0),
    osc2: Osc::OFF,
    sub: 0.0,
    noise: 0.0,
    fm: None,
    filter: OPEN,
    amp: AdsrSpec::new(0.003, 0.2, 1.0, 0.15),
    pitch_env: (0.0, 0.05),
    lfo: Lfo::NONE,
    mono: false,
    glide: 0.0,
    velocity: 0.6,
    fx: Fx::DRY,
};

const VIBRATO: Lfo = Lfo { target: LfoTarget::Pitch, wave: Wave::Sine, rate: 5.5, depth: 0.12, delay: 0.35, wheel: 0.5 };

pub const PRESETS: &[Preset] = &[
    Preset {
        id: "supersaw-lead",
        name: "スーパーソウ・リード",
        description: "7本重ねのデチューン・ソウ。トランスやビッグルームの主旋律に。",
        programs: &[81, 84],
        kind: Kind::Tonal(Patch {
            osc1: Osc::new(Wave::Saw, 0.8).unison(7, 24.0, 0.85),
            osc2: Osc::new(Wave::Saw, 0.35).pitch(12.0, 0.0).unison(3, 12.0, 0.6),
            filter: lowpass(5_500.0, 0.12, 1.2, AdsrSpec::new(0.005, 0.5, 0.45, 0.3)),
            amp: AdsrSpec::new(0.004, 0.3, 0.85, 0.28),
            lfo: VIBRATO,
            fx: Fx { drive: 1.0, chorus: 0.0, delay_time: 0.36, delay_feedback: 0.32, delay_mix: 0.18 },
            ..BASE
        }),
        gain_db: -16.4,
    },
    Preset {
        id: "supersaw-chord",
        name: "スーパーソウ・コード",
        description: "分厚く広がる和音用。フューチャーベースやEDMのコード、ドロップの壁に。",
        programs: &[90],
        kind: Kind::Tonal(Patch {
            osc1: Osc::new(Wave::Saw, 0.8).unison(7, 32.0, 1.0),
            osc2: Osc::new(Wave::Saw, 0.4).pitch(12.0, 0.0).unison(5, 26.0, 1.0),
            filter: lowpass(3_800.0, 0.1, 1.4, AdsrSpec::new(0.004, 0.35, 0.6, 0.3)),
            amp: AdsrSpec::new(0.006, 0.4, 0.75, 0.35),
            fx: Fx { drive: 1.0, chorus: 0.25, delay_time: 0.3, delay_feedback: 0.2, delay_mix: 0.1 },
            ..BASE
        }),
        gain_db: -14.8,
    },
    Preset {
        id: "pluck",
        name: "プラック",
        description: "弾いてすぐ消える、明るいシンセ・プラック。アルペジオやトロピカルハウスに。",
        programs: &[],
        kind: Kind::Tonal(Patch {
            osc1: Osc::new(Wave::Saw, 0.8).unison(3, 12.0, 0.7),
            osc2: Osc::new(Wave::Pulse(0.5), 0.35).pitch(-12.0, 0.0),
            filter: Filter {
                steep: true,
                ..lowpass(350.0, 0.25, 4.6, AdsrSpec::new(0.001, 0.2, 0.0, 0.15))
            },
            amp: AdsrSpec::new(0.001, 0.45, 0.0, 0.25),
            fx: Fx { drive: 1.0, chorus: 0.1, delay_time: 0.27, delay_feedback: 0.38, delay_mix: 0.24 },
            ..BASE
        }),
        gain_db: -8.3,
    },
    Preset {
        id: "warm-pad",
        name: "ウォーム・パッド",
        description: "ゆっくり立ち上がって、ゆっくり揺れる柔らかいパッド。背景の和音に。",
        programs: &[88, 89, 92, 94, 95],
        kind: Kind::Tonal(Patch {
            osc1: Osc::new(Wave::Saw, 0.7).unison(5, 16.0, 0.9),
            osc2: Osc::new(Wave::Triangle, 0.45).pitch(12.0, 3.0).unison(2, 8.0, 0.8),
            filter: lowpass(1_600.0, 0.12, 1.0, AdsrSpec::new(0.9, 1.6, 0.6, 1.6)),
            amp: AdsrSpec::new(0.8, 1.2, 0.85, 1.8),
            lfo: Lfo { target: LfoTarget::Cutoff, wave: Wave::Sine, rate: 0.22, depth: 0.35, delay: 0.0, wheel: 0.6 },
            velocity: 0.3,
            fx: Fx { drive: 1.0, chorus: 0.45, delay_time: 0.42, delay_feedback: 0.4, delay_mix: 0.14 },
            ..BASE
        }),
        gain_db: -6.9,
    },
    Preset {
        id: "synth-brass",
        name: "シンセ・ブラス",
        description: "フィルターが開いていくブラス系。80年代風やシンセウェーブのスタブに。",
        programs: &[62, 63],
        kind: Kind::Tonal(Patch {
            osc1: Osc::new(Wave::Saw, 0.7).unison(3, 10.0, 0.5),
            osc2: Osc::new(Wave::Saw, 0.5).pitch(0.0, 7.0),
            filter: lowpass(800.0, 0.15, 2.3, AdsrSpec::new(0.06, 0.5, 0.45, 0.3)),
            amp: AdsrSpec::new(0.025, 0.3, 0.9, 0.22),
            lfo: VIBRATO,
            fx: Fx { drive: 1.2, chorus: 0.2, delay_time: 0.3, delay_feedback: 0.0, delay_mix: 0.0 },
            ..BASE
        }),
        gain_db: -16.1,
    },
    Preset {
        id: "square-lead",
        name: "スクエア・リード",
        description: "芯のある矩形波リード。ディレイ付きで、メロディがよく通ります。",
        programs: &[80],
        kind: Kind::Tonal(Patch {
            osc1: Osc::new(Wave::Pulse(0.5), 0.7).unison(2, 6.0, 0.4),
            osc2: Osc::new(Wave::Pulse(0.25), 0.3).pitch(-12.0, 0.0),
            filter: lowpass(4_000.0, 0.15, 0.8, AdsrSpec::new(0.003, 0.3, 0.6, 0.2)),
            amp: AdsrSpec::new(0.004, 0.2, 0.9, 0.12),
            lfo: VIBRATO,
            mono: true,
            glide: 0.035,
            fx: Fx { drive: 1.1, chorus: 0.0, delay_time: 0.33, delay_feedback: 0.3, delay_mix: 0.2 },
            ..BASE
        }),
        gain_db: -20.1,
    },
    Preset {
        id: "chiptune",
        name: "チップチューン",
        description: "ファミコン風の細い矩形波。8ビット系やゲーム音楽に。",
        programs: &[],
        kind: Kind::Tonal(Patch {
            osc1: Osc::new(Wave::Pulse(0.25), 0.8),
            amp: AdsrSpec::new(0.001, 0.08, 0.8, 0.03),
            lfo: Lfo { delay: 0.25, depth: 0.2, rate: 6.0, ..VIBRATO },
            velocity: 0.3,
            ..BASE
        }),
        gain_db: -20.5,
    },
    Preset {
        id: "fm-bell",
        name: "FM ベル",
        description: "金属的にきらめくFMベル。ローファイのキーやイントロの飾りに。",
        programs: &[98],
        kind: Kind::Tonal(Patch {
            osc1: Osc::new(Wave::Sine, 0.9),
            osc2: Osc::new(Wave::Sine, 0.18).pitch(12.0, 4.0),
            fm: Some((3.5, 4.0, 0.9)),
            amp: AdsrSpec::new(0.001, 1.6, 0.0, 0.7),
            velocity: 0.7,
            fx: Fx { drive: 1.0, chorus: 0.2, delay_time: 0.38, delay_feedback: 0.35, delay_mix: 0.18 },
            ..BASE
        }),
        gain_db: -14.0,
    },
    Preset {
        id: "sub-bass",
        name: "サブベース",
        description: "床を揺らす純粋な低音。キックと一緒に鳴らす土台に。",
        programs: &[],
        kind: Kind::Tonal(Patch {
            osc1: Osc::new(Wave::Sine, 1.0),
            osc2: Osc::new(Wave::Triangle, 0.15),
            amp: AdsrSpec::new(0.004, 0.2, 1.0, 0.08),
            mono: true,
            glide: 0.02,
            velocity: 0.25,
            fx: Fx { drive: 1.4, ..Fx::DRY },
            ..BASE
        }),
        gain_db: -19.7,
    },
    Preset {
        id: "reese-bass",
        name: "リース・ベース",
        description: "うねる2本のデチューン・ソウ。ドラムンベースやダブステップの低音に。",
        programs: &[38, 87],
        kind: Kind::Tonal(Patch {
            osc1: Osc::new(Wave::Saw, 0.7).pitch(0.0, -14.0).unison(2, 8.0, 0.5),
            osc2: Osc::new(Wave::Saw, 0.7).pitch(0.0, 14.0).unison(2, 8.0, 0.5),
            sub: 0.5,
            filter: Filter { steep: true, key_track: 0.3, ..lowpass(650.0, 0.2, 0.6, AdsrSpec::new(0.01, 0.4, 0.5, 0.2)) },
            amp: AdsrSpec::new(0.004, 0.2, 1.0, 0.1),
            mono: true,
            glide: 0.03,
            velocity: 0.3,
            fx: Fx { drive: 2.2, ..Fx::DRY },
            ..BASE
        }),
        gain_db: -18.7,
    },
    Preset {
        id: "wobble-bass",
        name: "ワブル・ベース",
        description: "フィルターが周期的に開閉する、ダブステップの定番ベース。",
        programs: &[],
        kind: Kind::Tonal(Patch {
            osc1: Osc::new(Wave::Saw, 0.8).unison(2, 10.0, 0.3),
            osc2: Osc::new(Wave::Pulse(0.5), 0.5).pitch(-12.0, 0.0),
            sub: 0.4,
            filter: Filter { steep: true, key_track: 0.2, ..lowpass(260.0, 0.55, 0.0, AdsrSpec::new(0.001, 0.1, 1.0, 0.1)) },
            amp: AdsrSpec::new(0.004, 0.2, 1.0, 0.1),
            lfo: Lfo { target: LfoTarget::Cutoff, wave: Wave::Sine, rate: 3.0, depth: 3.6, delay: 0.0, wheel: 1.0 },
            mono: true,
            glide: 0.02,
            velocity: 0.3,
            fx: Fx { drive: 2.6, ..Fx::DRY },
            ..BASE
        }),
        gain_db: -19.9,
    },
    Preset {
        id: "acid-bass",
        name: "アシッド・ベース（303風）",
        description: "レゾナンスの効いたうねるベースライン。強く弾いた音ほどフィルターが開きます。",
        programs: &[39],
        kind: Kind::Tonal(Patch {
            osc1: Osc::new(Wave::Saw, 1.0),
            filter: Filter {
                mode: FilterMode::LowPass,
                cutoff: 220.0,
                resonance: 0.82,
                env_octaves: 3.8,
                env: AdsrSpec::new(0.001, 0.22, 0.0, 0.1),
                key_track: 0.4,
                velocity: 0.8,
                steep: false,
            },
            amp: AdsrSpec::new(0.002, 0.3, 0.8, 0.06),
            mono: true,
            glide: 0.06,
            velocity: 0.35,
            fx: Fx { drive: 1.8, chorus: 0.0, delay_time: 0.28, delay_feedback: 0.25, delay_mix: 0.1 },
            ..BASE
        }),
        gain_db: -18.9,
    },
    Preset {
        id: "808-bass",
        name: "808 ベース",
        description: "ピッチが落ちながら長く伸びる、トラップ／ヒップホップの 808。",
        programs: &[],
        kind: Kind::Tonal(Patch {
            osc1: Osc::new(Wave::Sine, 1.0),
            amp: AdsrSpec::new(0.001, 2.2, 0.0, 0.25),
            pitch_env: (7.0, 0.06),
            mono: true,
            glide: 0.05,
            velocity: 0.3,
            fx: Fx { drive: 2.4, ..Fx::DRY },
            ..BASE
        }),
        gain_db: -16.7,
    },
    Preset {
        id: "edm-drums",
        name: "EDM ドラム（909/808風）",
        description: "合成のキック・スネア・クラップ・ハイハット。GM のドラム配列どおりに鳴ります。",
        programs: &[],
        kind: Kind::Drums,
        gain_db: -14.7,
    },
];

/// The preset that should play a General MIDI program, when one fits better
/// than the SoundFont's own patch.
pub fn for_program(program: u8) -> Option<&'static Preset> {
    PRESETS.iter().find(|p| p.programs.contains(&program))
}

pub fn find(id: &str) -> Option<&'static Preset> {
    PRESETS.iter().find(|p| p.id == id)
}
