//! General MIDI instruments and drums, by name.
//!
//! A program number is the wrong thing to ask anyone to remember, and a drum
//! part written as `D2@1:0.25 F#2@1:0.25` is unreadable. Both directions are
//! here so that the tools can speak in names — `Flute`, `Kick`, `HatClosed` —
//! and still write the numbers the file format needs.

/// The 128 General MIDI programs, in the order the standard defines them.
pub const PROGRAMS: [&str; 128] = [
    "Acoustic Grand Piano", "Bright Acoustic Piano", "Electric Grand Piano", "Honky-tonk Piano",
    "Electric Piano 1", "Electric Piano 2", "Harpsichord", "Clavi",
    "Celesta", "Glockenspiel", "Music Box", "Vibraphone",
    "Marimba", "Xylophone", "Tubular Bells", "Dulcimer",
    "Drawbar Organ", "Percussive Organ", "Rock Organ", "Church Organ",
    "Reed Organ", "Accordion", "Harmonica", "Tango Accordion",
    "Acoustic Guitar (nylon)", "Acoustic Guitar (steel)", "Electric Guitar (jazz)",
    "Electric Guitar (clean)", "Electric Guitar (muted)", "Overdriven Guitar",
    "Distortion Guitar", "Guitar Harmonics",
    "Acoustic Bass", "Electric Bass (finger)", "Electric Bass (pick)", "Fretless Bass",
    "Slap Bass 1", "Slap Bass 2", "Synth Bass 1", "Synth Bass 2",
    "Violin", "Viola", "Cello", "Contrabass",
    "Tremolo Strings", "Pizzicato Strings", "Orchestral Harp", "Timpani",
    "String Ensemble 1", "String Ensemble 2", "Synth Strings 1", "Synth Strings 2",
    "Choir Aahs", "Voice Oohs", "Synth Voice", "Orchestra Hit",
    "Trumpet", "Trombone", "Tuba", "Muted Trumpet",
    "French Horn", "Brass Section", "Synth Brass 1", "Synth Brass 2",
    "Soprano Sax", "Alto Sax", "Tenor Sax", "Baritone Sax",
    "Oboe", "English Horn", "Bassoon", "Clarinet",
    "Piccolo", "Flute", "Recorder", "Pan Flute",
    "Blown Bottle", "Shakuhachi", "Whistle", "Ocarina",
    "Lead 1 (square)", "Lead 2 (sawtooth)", "Lead 3 (calliope)", "Lead 4 (chiff)",
    "Lead 5 (charang)", "Lead 6 (voice)", "Lead 7 (fifths)", "Lead 8 (bass + lead)",
    "Pad 1 (new age)", "Pad 2 (warm)", "Pad 3 (polysynth)", "Pad 4 (choir)",
    "Pad 5 (bowed)", "Pad 6 (metallic)", "Pad 7 (halo)", "Pad 8 (sweep)",
    "FX 1 (rain)", "FX 2 (soundtrack)", "FX 3 (crystal)", "FX 4 (atmosphere)",
    "FX 5 (brightness)", "FX 6 (goblins)", "FX 7 (echoes)", "FX 8 (sci-fi)",
    "Sitar", "Banjo", "Shamisen", "Koto",
    "Kalimba", "Bag pipe", "Fiddle", "Shanai",
    "Tinkle Bell", "Agogo", "Steel Drums", "Woodblock",
    "Taiko Drum", "Melodic Tom", "Synth Drum", "Reverse Cymbal",
    "Guitar Fret Noise", "Breath Noise", "Seashore", "Bird Tweet",
    "Telephone Ring", "Helicopter", "Applause", "Gunshot",
];

pub fn program_name(program: u8) -> &'static str {
    PROGRAMS[usize::from(program) & 0x7f]
}

/// General MIDI percussion, keyed by the note that plays it. One word each,
/// so a drum part reads as `Kick@1:0.25 HatClosed@1.5:0.25`.
const DRUMS: [(u8, &str); 47] = [
    (35, "KickSoft"), (36, "Kick"), (37, "Rimshot"), (38, "Snare"),
    (39, "Clap"), (40, "SnareElectric"), (41, "TomFloorLow"), (42, "HatClosed"),
    (43, "TomFloorHigh"), (44, "HatPedal"), (45, "TomLow"), (46, "HatOpen"),
    (47, "TomLowMid"), (48, "TomHiMid"), (49, "Crash"), (50, "TomHigh"),
    (51, "Ride"), (52, "China"), (53, "RideBell"), (54, "Tambourine"),
    (55, "Splash"), (56, "Cowbell"), (57, "Crash2"), (58, "Vibraslap"),
    (59, "Ride2"), (60, "BongoHigh"), (61, "BongoLow"), (62, "CongaMute"),
    (63, "CongaHigh"), (64, "CongaLow"), (65, "TimbaleHigh"), (66, "TimbaleLow"),
    (67, "AgogoHigh"), (68, "AgogoLow"), (69, "Cabasa"), (70, "Maracas"),
    (71, "WhistleShort"), (72, "WhistleLong"), (73, "GuiroShort"), (74, "GuiroLong"),
    (75, "Claves"), (76, "WoodblockHigh"), (77, "WoodblockLow"), (78, "CuicaMute"),
    (79, "CuicaOpen"), (80, "TriangleMute"), (81, "TriangleOpen"),
];

/// The drum this note plays, if the standard names one.
pub fn drum_name(key: u8) -> Option<&'static str> {
    DRUMS.iter().find(|(k, _)| *k == key).map(|(_, n)| *n)
}

/// The note a named drum plays.
pub fn drum_key(name: &str) -> Option<u8> {
    let wanted = simplify(name);
    DRUMS.iter().find(|(_, n)| simplify(n) == wanted).map(|(k, _)| *k)
}

/// A program by name or by number: `"Flute"`, `"flute"`, `"73"` all work.
///
/// Names are matched loosely — case, spaces and punctuation are ignored, and
/// a name that is contained in exactly one instrument is accepted — because a
/// model asking for "french horn" should not have to know it is program 60.
pub fn program_number(text: &str) -> Result<u8, String> {
    let text = text.trim();
    if let Ok(n) = text.parse::<u8>() {
        if n <= 127 {
            return Ok(n);
        }
        return Err(format!("{text}: General MIDI program numbers are 0–127"));
    }
    let wanted = simplify(text);
    if wanted.is_empty() {
        return Err("Give an instrument name".into());
    }
    if let Some(i) = PROGRAMS.iter().position(|p| simplify(p) == wanted) {
        return Ok(i as u8);
    }
    let near: Vec<&str> = PROGRAMS
        .iter()
        .filter(|p| simplify(p).contains(&wanted))
        .copied()
        .take(6)
        .collect();
    match near.len() {
        1 => Ok(PROGRAMS.iter().position(|p| *p == near[0]).expect("just found") as u8),
        0 => Err(format!(
            "{text}: there is no such General MIDI instrument. Use names like \"Flute\", \"Trumpet\" or \"String Ensemble 1\"."
        )),
        _ => Err(format!("{text}: which one do you mean? {}", near.join(" / "))),
    }
}

/// Lowercase, letters and digits only, so "French Horn" == "frenchhorn".
fn simplify(text: &str) -> String {
    text.chars().filter(|c| c.is_alphanumeric()).flat_map(char::to_lowercase).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instruments_answer_to_their_names() {
        assert_eq!(program_number("Flute").unwrap(), 73);
        assert_eq!(program_number("flute").unwrap(), 73);
        assert_eq!(program_number("French Horn").unwrap(), 60);
        assert_eq!(program_number("frenchhorn").unwrap(), 60);
        assert_eq!(program_number("timpani").unwrap(), 47);
        assert_eq!(program_number("0").unwrap(), 0);
        assert_eq!(program_number("73").unwrap(), 73);
        // Uniquely contained, even though it is not the whole name.
        assert_eq!(program_number("Harpsichord").unwrap(), 6);
        assert_eq!(program_name(60), "French Horn");
    }

    #[test]
    fn a_wrong_instrument_name_suggests_rather_than_guesses() {
        let err = program_number("Trombone Section").unwrap_err();
        assert!(err.contains("no such"), "{err}");
        let err = program_number("piano").unwrap_err();
        assert!(err.contains("which one") && err.contains("Piano"), "{err}");
        assert!(program_number("200").unwrap_err().contains("0–127"));
    }

    #[test]
    fn drums_answer_to_their_names() {
        assert_eq!(drum_key("Kick").unwrap(), 36);
        assert_eq!(drum_key("hatclosed").unwrap(), 42);
        assert_eq!(drum_name(38), Some("Snare"));
        assert_eq!(drum_name(20), None, "outside the standard's range");
        for (key, name) in DRUMS {
            assert_eq!(drum_key(name), Some(key), "{name}");
        }
    }
}

/// The notes an instrument can actually play, where that is a real constraint.
///
/// Written for the model's benefit: it cannot hear that the flute part has
/// gone below the instrument's lowest note, and neither can a synthesiser —
/// a sampler will happily pitch a flute sample down two octaves and the
/// result will be unusable. Ranges are the practical ones an arranger would
/// write for, not the record-breaking extremes.
const RANGES: [(u8, u8, u8); 40] = [
    // (program, lowest, highest)
    (40, 55, 96),  // Violin            G3–C7
    (41, 48, 88),  // Viola             C3–E6
    (42, 36, 81),  // Cello             C2–A5
    (43, 28, 67),  // Contrabass        E1–G4
    (44, 36, 96),  // Tremolo Strings
    (45, 28, 96),  // Pizzicato Strings
    (46, 23, 104), // Orchestral Harp   B0–G7
    (47, 36, 57),  // Timpani           C2–A3
    (48, 28, 96),  // String Ensemble 1
    (49, 28, 96),  // String Ensemble 2
    (56, 54, 84),  // Trumpet           F#3–C6
    (57, 40, 72),  // Trombone          E2–C5
    (58, 28, 58),  // Tuba              E1–A#3
    (59, 54, 84),  // Muted Trumpet
    (60, 34, 77),  // French Horn       A#1–F5
    (61, 34, 84),  // Brass Section
    (64, 56, 87),  // Soprano Sax
    (65, 49, 81),  // Alto Sax          C#3–A5
    (66, 44, 76),  // Tenor Sax
    (67, 37, 69),  // Baritone Sax
    (68, 58, 91),  // Oboe              A#3–G6
    (69, 52, 81),  // English Horn
    (70, 34, 75),  // Bassoon           A#1–D#5
    (71, 50, 91),  // Clarinet          D3–G6
    (72, 74, 103), // Piccolo           D5–G7
    (73, 60, 96),  // Flute             C4–C7
    (74, 60, 91),  // Recorder
    (75, 60, 91),  // Pan Flute
    (0, 21, 108),  // Acoustic Grand Piano  A0–C8
    (1, 21, 108),
    (2, 21, 108),
    (3, 21, 108),
    (6, 29, 89),   // Harpsichord
    (9, 72, 108),  // Glockenspiel
    (11, 53, 89),  // Vibraphone
    (12, 45, 96),  // Marimba
    (13, 65, 108), // Xylophone
    (32, 28, 67),  // Acoustic Bass
    (33, 28, 67),  // Electric Bass (finger)
    (34, 28, 67),  // Electric Bass (pick)
];

/// The playable range of a program, when the standard instrument has one.
pub fn range(program: u8) -> Option<(u8, u8)> {
    RANGES.iter().find(|(p, ..)| *p == program).map(|(_, lo, hi)| (*lo, *hi))
}

#[cfg(test)]
mod range_tests {
    use super::*;

    #[test]
    fn ranges_are_sane_and_cover_the_orchestra() {
        for (program, lo, hi) in RANGES {
            assert!(lo < hi, "{}", program_name(program));
            assert!(hi - lo >= 12, "{} spans less than an octave", program_name(program));
        }
        assert_eq!(range(73), Some((60, 96)), "Flute starts at middle C");
        assert_eq!(range(43), Some((28, 67)), "Contrabass");
        assert_eq!(range(80), None, "a synth lead has no natural limit");
    }
}
