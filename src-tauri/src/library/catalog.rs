//! Built-in catalog of free SFZ libraries and their General MIDI mapping.
//!
//! Every URL was checked to resolve; sizes are from Content-Length (VSCO's
//! GitHub source zip sends none, so its figure is the repository size).

#[derive(Clone, Copy, Debug)]
pub enum Archive {
    Zip,
    TarGz,
    TarXz,
}

/// One key range of a split instrument.
pub struct SplitPart {
    pub sfz: &'static str,
    pub lo: u8,
    pub hi: u8,
}

pub struct CatalogInstrument {
    pub id: &'static str,
    pub name: &'static str,
    pub name_ja: &'static str,
    /// Path of the .sfz file relative to the library root (ignored when
    /// `split` is non-empty).
    pub sfz: &'static str,
    /// Several .sfz files sharing the keyboard by range.
    pub split: &'static [SplitPart],
    /// GM programs (0-127) this instrument stands in for.
    pub programs: &'static [u8],
    /// A drum kit for GM percussion (MIDI channel 10).
    pub percussion: bool,
    /// Level trim so the library sits at the SoundFont's loudness (measured:
    /// energy of a C4 at velocity 110 vs. the GM patch it replaces).
    pub gain_db: f32,
}

impl CatalogInstrument {
    const fn gain(mut self, db: f32) -> Self {
        self.gain_db = db;
        self
    }
}

pub struct CatalogLibrary {
    pub id: &'static str,
    pub name: &'static str,
    pub name_ja: &'static str,
    pub description: &'static str,
    pub description_ja: &'static str,
    pub license: &'static str,
    pub license_ja: &'static str,
    /// Credit to show in the app, as the license requires or asks.
    pub attribution: &'static str,
    pub homepage: &'static str,
    pub url: &'static str,
    /// Approximate archive size in bytes (for display before downloading).
    pub download_size: u64,
    pub archive: Archive,
    /// Folder inside the archive to install; "" = its single top-level folder.
    pub root: &'static str,
    pub instruments: &'static [CatalogInstrument],
}

/// Names are given in English, then Japanese.
const fn inst(
    id: &'static str,
    name: &'static str,
    name_ja: &'static str,
    sfz: &'static str,
    programs: &'static [u8],
) -> CatalogInstrument {
    CatalogInstrument { id, name, name_ja, sfz, split: &[], programs, percussion: false, gain_db: 0.0 }
}

const fn split(
    id: &'static str,
    name: &'static str,
    name_ja: &'static str,
    split: &'static [SplitPart],
    programs: &'static [u8],
) -> CatalogInstrument {
    CatalogInstrument { id, name, name_ja, sfz: "", split, programs, percussion: false, gain_db: 0.0 }
}

/// VSCO string sections, each playing the part of the keyboard it covers:
/// contrabass up to D2, cello to F#3, viola to C#4, violins above.
macro_rules! vsco_strings {
    ($bass:literal, $cello:literal, $viola:literal, $violin:literal) => {
        &[
            SplitPart { sfz: $bass, lo: 0, hi: 38 },
            SplitPart { sfz: $cello, lo: 39, hi: 54 },
            SplitPart { sfz: $viola, lo: 55, hi: 61 },
            SplitPart { sfz: $violin, lo: 62, hi: 127 },
        ]
    };
}

pub const CATALOG: &[CatalogLibrary] = &[
    CatalogLibrary {
        id: "salamander-piano",
        name: "Salamander Grand Piano",
        name_ja: "Salamander Grand Piano",
        description: "A Yamaha C5 grand piano: the go-to free piano, with 16 velocity layers and even pedal and string resonance recorded.",
        description_ja: "ヤマハ C5 グランドピアノ。16段階の強弱、ペダルや弦の共鳴音まで収録した定番のピアノ音源です。",
        license: "CC BY 3.0",
        license_ja: "CC BY 3.0",
        attribution: "Salamander Grand Piano V3 by Alexander Holm (CC BY 3.0); SFZ+FLAC packaging by FreePats",
        homepage: "https://freepats.zenvoid.org/Piano/acoustic-grand-piano.html",
        url: "https://freepats.zenvoid.org/Piano/SalamanderGrandPiano/SalamanderGrandPiano-SFZ+FLAC-V3+20200602.tar.gz",
        download_size: 741_757_374,
        archive: Archive::TarGz,
        root: "",
        instruments: &[inst("grand", "Grand Piano (Salamander)", "グランドピアノ（Salamander）", "SalamanderGrandPiano-V3+20200602.sfz", &[0, 1, 2, 3]).gain(6.9)],
    },
    CatalogLibrary {
        id: "vsco2-ce",
        name: "VS Chamber Orchestra 2 CE",
        name_ja: "VS Chamber Orchestra 2 CE",
        description: "A whole chamber orchestra: string sections, woodwinds, brass, harp, percussion, organ and more. It is large, so the download takes a while.",
        description_ja: "室内オーケストラ一式。弦セクション、木管、金管、ハープ、打楽器、オルガンなど。容量が大きいのでダウンロードに時間がかかります。",
        license: "CC0",
        license_ja: "CC0",
        attribution: "VS Chamber Orchestra: Community Edition — Versilian Studios (Sam Gossner) & Ivy Audio (Simon Dalzell), CC0",
        homepage: "https://versilian-studios.com/vsco-community/",
        url: "https://github.com/sgossner/VSCO-2-CE/archive/refs/tags/1.1.0.zip",
        download_size: 3_240_000_000,
        archive: Archive::Zip,
        root: "",
        instruments: &[
            split(
                "strings",
                "String Ensemble",
                "弦楽合奏",
                vsco_strings!("ContrabassSusVB.sfz", "CelloEnsSusVib.sfz", "ViolaEnsSusVib.sfz", "ViolinEnsSusVib.sfz"),
                &[48, 50],
            ).gain(-1.9),
            split(
                "strings-soft",
                "String Ensemble (soft)",
                "弦楽合奏（弱奏）",
                vsco_strings!(
                    "ContrabassSusVB-Quiet.sfz",
                    "CelloEnsSusVib-Quiet.sfz",
                    "ViolaEnsSusVib-Quiet.sfz",
                    "ViolinEnsSusVib-Quiet.sfz"
                ),
                &[49, 51],
            ).gain(3.3),
            split(
                "strings-trem",
                "Strings Tremolo",
                "弦楽トレモロ",
                vsco_strings!("ContrabassTrem.sfz", "CelloEnsTrem.sfz", "ViolaEnsTrem.sfz", "ViolinEnsTrem.sfz"),
                &[44],
            ).gain(4.0),
            split(
                "strings-pizz",
                "Strings Pizzicato",
                "弦楽ピチカート",
                vsco_strings!("ContrabassPizz.sfz", "CelloEnsPizz.sfz", "ViolaEnsPizz.sfz", "ViolinEnsPizz.sfz"),
                &[45],
            ).gain(0.0),
            inst("violin", "Solo Violin", "ソロ・バイオリン", "SViolinVib.sfz", &[40, 110]).gain(7.8),
            inst("viola", "Viola", "ビオラ", "ViolaEnsSusVib.sfz", &[41]).gain(-6.9),
            inst("cello", "Cello", "チェロ", "CelloEnsSusVib.sfz", &[42]).gain(1.1),
            inst("contrabass", "Contrabass", "コントラバス", "ContrabassSusVB.sfz", &[43]).gain(2.8),
            inst("harp", "Harp", "ハープ", "Harp.sfz", &[46]).gain(11.3),
            inst("timpani", "Timpani", "ティンパニ", "Timpani.sfz", &[47]).gain(8.0),
            inst("trumpet", "Trumpet", "トランペット", "TrumpetSusVib.sfz", &[56]).gain(12.0),
            inst("trombone", "Trombone", "トロンボーン", "TromboneSus.sfz", &[57]).gain(12.0),
            inst("tuba", "Tuba", "チューバ", "TubaSus.sfz", &[58]).gain(4.8),
            inst("trumpet-mute", "Trumpet (muted)", "トランペット（ミュート）", "TrumpetStraightMuteSus.sfz", &[59]).gain(2.3),
            inst("horn", "French Horn", "ホルン", "FHornSus.sfz", &[60, 61]).gain(1.4),
            inst("oboe", "Oboe", "オーボエ", "OboeSusVib.sfz", &[68, 69]).gain(11.7),
            inst("bassoon", "Bassoon", "ファゴット", "BassoonSus.sfz", &[70]).gain(3.5),
            inst("clarinet", "Clarinet", "クラリネット", "ClarinetSus.sfz", &[71]).gain(10.7),
            inst("piccolo", "Piccolo", "ピッコロ", "PiccoloSus.sfz", &[72]).gain(2.8),
            inst("flute", "Flute", "フルート", "FluteSusVib.sfz", &[73]).gain(4.2),
            inst("glockenspiel", "Glockenspiel", "グロッケンシュピール", "Glockenspiel.sfz", &[9]).gain(12.0),
            inst("marimba", "Marimba", "マリンバ", "Marimba.sfz", &[12]).gain(11.4),
            inst("xylophone", "Xylophone", "シロフォン", "Xylophone.sfz", &[13]).gain(-5.6),
            inst("tubular-bells", "Tubular Bells", "チューブラーベル", "TubularBells.sfz", &[14]).gain(3.1),
            inst("organ", "Pipe Organ", "パイプオルガン", "OrganLoud.sfz", &[19]).gain(4.2),
            inst("organ-soft", "Pipe Organ (soft)", "パイプオルガン（弱）", "OrganQuiet.sfz", &[20]).gain(6.8),
        ],
    },
    CatalogLibrary {
        id: "freepats-gm-drums",
        name: "FreePats GM Drums",
        name_ja: "FreePats GM ドラム",
        description: "A drum and percussion kit in the General MIDI layout. Small, and plays drum tracks as they are.",
        description_ja: "GM 配列どおりのドラム・パーカッションセット。軽量で、ドラムトラックにそのまま使えます。",
        license: "GPL-3.0 (with an exception for music you make with it)",
        license_ja: "GPL-3.0（音楽制作での利用は例外として自由）",
        attribution: "FreePats GM Percussion Set — FreePats project (freepats.zenvoid.org)",
        homepage: "https://freepats.zenvoid.org/Percussion/",
        url: "https://freepats.zenvoid.org/SoundSets/GM-PercussionSet/FreePatsGM-Percussion-SFZ-20200822.tar.xz",
        download_size: 20_534_324,
        archive: Archive::TarXz,
        root: "",
        instruments: &[CatalogInstrument {
            id: "kit",
            name: "GM Drum Kit (FreePats)",
            name_ja: "GM ドラムセット（FreePats）",
            sfz: "FreePatsGM-Percussion-20200822.sfz",
            split: &[],
            programs: &[],
            percussion: true,
            gain_db: 2.4,
        }],
    },
    CatalogLibrary {
        id: "freepats-drawbar-organ",
        name: "FreePats Drawbar Organ",
        name_ja: "FreePats ドローバーオルガン",
        description: "An emulation of a Hammond-style drawbar organ.",
        description_ja: "ハモンド系ドローバーオルガンのエミュレーション。",
        license: "CC0",
        license_ja: "CC0",
        attribution: "Drawbar Organ Emulation — FreePats project, CC0",
        homepage: "https://freepats.zenvoid.org/Organ/",
        url: "https://freepats.zenvoid.org/Organ/DrawbarOrganEmulation/DrawbarOrganEmulation-SFZ-20190712.tar.xz",
        download_size: 6_042_972,
        archive: Archive::TarXz,
        root: "",
        instruments: &[inst("drawbar", "Drawbar Organ", "ドローバーオルガン", "DrawbarOrganEmulation-20190712.sfz", &[16, 17, 18]).gain(-12.0)],
    },
    CatalogLibrary {
        id: "freepats-church-organ",
        name: "FreePats Church Organ",
        name_ja: "FreePats チャーチオルガン",
        description: "A church pipe organ sound.",
        description_ja: "教会のパイプオルガン風の音色。",
        license: "CC0",
        license_ja: "CC0",
        attribution: "Church Organ Emulation — FreePats project, CC0",
        homepage: "https://freepats.zenvoid.org/Organ/",
        url: "https://freepats.zenvoid.org/Organ/ChurchOrganEmulation/ChurchOrganEmulation-SFZ-20190924.tar.xz",
        download_size: 13_033_856,
        archive: Archive::TarXz,
        root: "",
        instruments: &[inst("church", "Church Organ", "チャーチオルガン", "ChurchOrganEmulation-20190924.sfz", &[19, 20]).gain(-3.4)],
    },
    CatalogLibrary {
        id: "freepats-steel-guitar",
        name: "FreePats Steel-String Guitar",
        name_ja: "FreePats スチール弦ギター",
        description: "Acoustic steel-string guitar (11 velocity layers).",
        description_ja: "アコースティック・スチール弦ギター（11段階の強弱）。",
        license: "GPL-3.0 (with an exception for music you make with it)",
        license_ja: "GPL-3.0（音楽制作での利用は例外として自由）",
        attribution: "FSS Steel String Guitar — FreePats project",
        homepage: "https://freepats.zenvoid.org/Guitar/",
        url: "https://freepats.zenvoid.org/Guitar/FSS-SteelStringGuitar/FSS-SteelStringGuitar-SFZ-20200521.tar.xz",
        download_size: 13_130_512,
        archive: Archive::TarXz,
        root: "",
        instruments: &[inst("steel", "Steel-String Guitar", "スチール弦ギター", "FSS-SteelStringGuitar-20200521.sfz", &[24, 25]).gain(-5.0)],
    },
    CatalogLibrary {
        id: "freepats-tenor-sax",
        name: "FreePats Tenor Sax",
        name_ja: "FreePats テナーサックス",
        description: "Tenor saxophone, used for saxophone tracks.",
        description_ja: "テナーサックス。サックス系のトラックに使われます。",
        license: "CC0",
        license_ja: "CC0",
        attribution: "Tenor Saxophone — FreePats project, CC0",
        homepage: "https://freepats.zenvoid.org/Reed/",
        url: "https://freepats.zenvoid.org/Reed/TenorSaxophone/TenorSaxophone-SFZ+FLAC-20200717.tar.gz",
        download_size: 32_204_648,
        archive: Archive::TarGz,
        root: "",
        instruments: &[inst("tenor-sax", "Tenor Sax", "テナーサックス", "TenorSaxophone-20200717.sfz", &[64, 65, 66, 67]).gain(-7.9)],
    },
    CatalogLibrary {
        id: "karoryfer-growlybass",
        name: "Karoryfer Growlybass",
        name_ja: "Karoryfer Growlybass",
        description: "Electric bass (fingerstyle, clean).",
        description_ja: "エレキベース（指弾き・クリーン）。",
        license: "CC0",
        license_ja: "CC0",
        attribution: "Growlybass — Karoryfer Samples, CC0",
        homepage: "https://github.com/sfzinstruments/karoryfer.growlybass",
        url: "https://github.com/sfzinstruments/karoryfer.growlybass/releases/download/v1.002/Karoryfer.Growlybass.v1.002.zip",
        download_size: 167_454_256,
        archive: Archive::Zip,
        root: "",
        instruments: &[inst("bass", "Electric Bass (Growlybass)", "エレキベース（Growlybass）", "growlybass_clean.sfz", &[33, 34, 35]).gain(8.0)],
    },
];
