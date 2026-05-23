// General MIDI program number (0-127) -> soundfont-player instrument name.
export const GM_SOUNDFONT_NAMES: string[] = [
  "acoustic_grand_piano",
  "bright_acoustic_piano",
  "electric_grand_piano",
  "honkytonk_piano",
  "electric_piano_1",
  "electric_piano_2",
  "harpsichord",
  "clavinet",
  "celesta",
  "glockenspiel",
  "music_box",
  "vibraphone",
  "marimba",
  "xylophone",
  "tubular_bells",
  "dulcimer",
  "drawbar_organ",
  "percussive_organ",
  "rock_organ",
  "church_organ",
  "reed_organ",
  "accordion",
  "harmonica",
  "tango_accordion",
  "acoustic_guitar_nylon",
  "acoustic_guitar_steel",
  "electric_guitar_jazz",
  "electric_guitar_clean",
  "electric_guitar_muted",
  "overdriven_guitar",
  "distortion_guitar",
  "guitar_harmonics",
  "acoustic_bass",
  "electric_bass_finger",
  "electric_bass_pick",
  "fretless_bass",
  "slap_bass_1",
  "slap_bass_2",
  "synth_bass_1",
  "synth_bass_2",
  "violin",
  "viola",
  "cello",
  "contrabass",
  "tremolo_strings",
  "pizzicato_strings",
  "orchestral_harp",
  "timpani",
  "string_ensemble_1",
  "string_ensemble_2",
  "synth_strings_1",
  "synth_strings_2",
  "choir_aahs",
  "voice_oohs",
  "synth_choir",
  "orchestra_hit",
  "trumpet",
  "trombone",
  "tuba",
  "muted_trumpet",
  "french_horn",
  "brass_section",
  "synth_brass_1",
  "synth_brass_2",
  "soprano_sax",
  "alto_sax",
  "tenor_sax",
  "baritone_sax",
  "oboe",
  "english_horn",
  "bassoon",
  "clarinet",
  "piccolo",
  "flute",
  "recorder",
  "pan_flute",
  "blown_bottle",
  "shakuhachi",
  "whistle",
  "ocarina",
  "lead_1_square",
  "lead_2_sawtooth",
  "lead_3_calliope",
  "lead_4_chiff",
  "lead_5_charang",
  "lead_6_voice",
  "lead_7_fifths",
  "lead_8_bass__lead",
  "pad_1_new_age",
  "pad_2_warm",
  "pad_3_polysynth",
  "pad_4_choir",
  "pad_5_bowed",
  "pad_6_metallic",
  "pad_7_halo",
  "pad_8_sweep",
  "fx_1_rain",
  "fx_2_soundtrack",
  "fx_3_crystal",
  "fx_4_atmosphere",
  "fx_5_brightness",
  "fx_6_goblins",
  "fx_7_echoes",
  "fx_8_scifi",
  "sitar",
  "banjo",
  "shamisen",
  "koto",
  "kalimba",
  "bagpipe",
  "fiddle",
  "shanai",
  "tinkle_bell",
  "agogo",
  "steel_drums",
  "woodblock",
  "taiko_drum",
  "melodic_tom",
  "synth_drum",
  "reverse_cymbal",
  "guitar_fret_noise",
  "breath_noise",
  "seashore",
  "bird_tweet",
  "telephone_ring",
  "helicopter",
  "applause",
  "gunshot",
];

// Japanese display labels for the 16 GM instrument families.
const FAMILY_LABELS = [
  "ピアノ",
  "クロマチック・パーカッション",
  "オルガン",
  "ギター",
  "ベース",
  "ストリングス",
  "アンサンブル",
  "ブラス",
  "リード",
  "パッド",
  "シンセ・エフェクト",
  "エスニック",
  "パーカッシブ",
  "サウンドエフェクト",
];

export function soundfontNameForProgram(program: number): string {
  const p = Math.max(0, Math.min(127, program | 0));
  return GM_SOUNDFONT_NAMES[p];
}

export function prettyInstrumentName(program: number): string {
  const name = soundfontNameForProgram(program);
  return name
    .split("_")
    .filter(Boolean)
    .map((w) => w.charAt(0).toUpperCase() + w.slice(1))
    .join(" ");
}

export function instrumentFamily(program: number): string {
  const idx = Math.min(FAMILY_LABELS.length - 1, Math.floor(program / 8));
  return FAMILY_LABELS[idx] ?? "楽器";
}

export interface InstrumentChoice {
  program: number;
  name: string;
}

export interface InstrumentFamilyGroup {
  family: string;
  instruments: InstrumentChoice[];
}

/** 16 GM ファミリー × 各 8 楽器のグループ一覧（楽器ピッカー用）。 */
export function gmInstrumentGroups(): InstrumentFamilyGroup[] {
  const groups: InstrumentFamilyGroup[] = [];
  for (let f = 0; f < FAMILY_LABELS.length; f++) {
    const instruments: InstrumentChoice[] = [];
    for (let i = 0; i < 8; i++) {
      const program = f * 8 + i;
      if (program > 127) break;
      instruments.push({ program, name: prettyInstrumentName(program) });
    }
    groups.push({ family: FAMILY_LABELS[f], instruments });
  }
  return groups;
}

/** ピアノ系プログラム（0=Acoustic Grand 〜 7=Clavinet）かどうか。 */
export function isPianoProgram(program: number): boolean {
  return program >= 0 && program <= 7;
}
