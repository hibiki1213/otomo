//! What each General MIDI instrument looks like on the page: its clef or
//! staves, how it transposes, what it is called in the margin, and — for a
//! drum kit — where each drum sits on the staff.

use super::pitch::Interval;
use crate::gm;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Clef {
    Treble,
    /// Treble with an 8 below: guitars, read an octave above where they sound.
    Treble8vb,
    Bass,
    Alto,
    Percussion,
}

impl Clef {
    /// (sign, line, octave change) as MusicXML writes it.
    pub(crate) fn xml(self) -> (&'static str, u8, i8) {
        match self {
            Clef::Treble => ("G", 2, 0),
            Clef::Treble8vb => ("G", 2, -1),
            Clef::Bass => ("F", 4, 0),
            Clef::Alto => ("C", 3, 0),
            Clef::Percussion => ("percussion", 3, 0),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Family {
    Woodwind,
    Brass,
    Percussion,
    Keyboard,
    Strings,
    Other,
}

pub(crate) fn family(program: u8, percussion: bool) -> Family {
    if percussion {
        return Family::Percussion;
    }
    match program {
        0..=7 | 16..=23 => Family::Keyboard,
        8..=15 | 47 => Family::Percussion,
        40..=45 | 48..=51 => Family::Strings,
        56..=63 => Family::Brass,
        64..=79 => Family::Woodwind,
        _ => Family::Other,
    }
}

/// Sounding to written, for instruments that do not read at pitch.
pub(crate) fn transposition(program: u8, percussion: bool) -> Interval {
    if percussion {
        return Interval::NONE;
    }
    let (diatonic, chromatic) = match program {
        8 | 13 => (-7, -12),         // celesta, xylophone: sound an octave up
        9 => (-14, -24),             // glockenspiel: two octaves up
        24..=31 => (7, 12),          // guitars: an octave down
        32..=39 | 43 => (7, 12),     // basses: an octave down
        56 | 59 | 64 | 71 => (1, 2), // B♭ trumpet, soprano sax, clarinet
        60 | 69 => (4, 7),           // F horn, English horn
        65 => (5, 9),                // E♭ alto sax
        66 => (8, 14),               // B♭ tenor sax: a ninth
        67 => (12, 21),              // E♭ baritone sax: an octave and a sixth
        72 => (-7, -12),             // piccolo
        _ => (0, 0),
    };
    Interval { diatonic, chromatic }
}

/// Instruments written on a grand staff.
pub(crate) fn grand_staff(program: u8) -> bool {
    matches!(program, 0..=8 | 16..=21 | 23 | 46)
}

/// Instruments that sound one note at a time: overlaps are legato, not chords.
pub(crate) fn monophonic(program: u8) -> bool {
    matches!(program, 56..=60 | 64..=79)
}

/// The clef an instrument is always written in, if it has one.
pub(crate) fn fixed_clef(program: u8) -> Option<Clef> {
    Some(match program {
        24..=31 => Clef::Treble8vb,
        32..=39 | 42 | 43 | 47 | 57 | 58 | 70 => Clef::Bass,
        41 => Clef::Alto,
        9..=15 | 40 | 56 | 59 | 60 | 64..=69 | 71..=79 => Clef::Treble,
        _ => return None,
    })
}

/// Instruments recognised inside a track's own name ("A5-Clarinet", "Flute 2",
/// "第1ヴァイオリン"), longest first where one contains another.
const NAMED: [(&str, &str); 45] = [
    ("bass clarinet", "B. Cl."),
    ("english horn", "E. Hn."),
    ("double bass", "Cb."),
    ("contrabass", "Cb."),
    ("piccolo", "Picc."),
    ("flute", "Fl."),
    ("oboe", "Ob."),
    ("clarinet", "Cl."),
    ("bassoon", "Bsn."),
    ("saxophone", "Sax."),
    ("sax", "Sax."),
    ("horn", "Hn."),
    ("trumpet", "Tpt."),
    ("trombone", "Tbn."),
    ("tuba", "Tba."),
    ("violin", "Vln."),
    ("viola", "Vla."),
    ("cello", "Vc."),
    ("harp", "Hp."),
    ("timpani", "Timp."),
    ("piano", "Pno."),
    ("guitar", "Gtr."),
    ("organ", "Org."),
    ("drum", "Dr."),
    ("percussion", "Perc."),
    ("strings", "Str."),
    ("choir", "Ch."),
    ("bass", "Bass"),
    ("コントラバス", "Cb."),
    ("ピッコロ", "Picc."),
    ("フルート", "Fl."),
    ("オーボエ", "Ob."),
    ("クラリネット", "Cl."),
    ("ファゴット", "Bsn."),
    ("サックス", "Sax."),
    ("ホルン", "Hn."),
    ("トランペット", "Tpt."),
    ("トロンボーン", "Tbn."),
    ("チューバ", "Tba."),
    ("ヴァイオリン", "Vln."),
    ("バイオリン", "Vln."),
    ("ヴィオラ", "Vla."),
    ("ビオラ", "Vla."),
    ("チェロ", "Vc."),
    ("ピアノ", "Pno."),
];

/// The short name printed in front of later systems.
pub(crate) fn abbreviation(program: u8, percussion: bool, name: &str) -> String {
    if percussion {
        return "Dr.".into();
    }
    let lower = name.to_lowercase();
    if let Some((_, short)) = NAMED.iter().find(|(word, _)| lower.contains(word)) {
        return (*short).into();
    }
    let custom = !name.is_empty() && name != gm::program_name(program);
    if !custom {
        let short = match program {
            0..=7 => "Pno.",
            8 => "Cel.",
            9 => "Glk.",
            11 => "Vib.",
            12 => "Mar.",
            13 => "Xyl.",
            16..=20 => "Org.",
            21 | 23 => "Acc.",
            24..=31 => "Gtr.",
            32..=39 => "Bass",
            40 => "Vln.",
            41 => "Vla.",
            42 => "Vc.",
            43 => "Cb.",
            44 | 45 | 48..=51 => "Str.",
            46 => "Hp.",
            47 => "Timp.",
            52..=54 => "Ch.",
            56 | 59 => "Tpt.",
            57 => "Tbn.",
            58 => "Tba.",
            60 => "Hn.",
            61..=63 => "Br.",
            64 => "S. Sax",
            65 => "A. Sax",
            66 => "T. Sax",
            67 => "B. Sax",
            68 => "Ob.",
            69 => "E. Hn.",
            70 => "Bsn.",
            71 => "Cl.",
            72 => "Picc.",
            73 => "Fl.",
            74 => "Rec.",
            _ => "",
        };
        if !short.is_empty() {
            return short.into();
        }
    }
    let chars: Vec<char> = name.chars().collect();
    if chars.len() <= 6 {
        name.to_string()
    } else if chars.iter().all(char::is_ascii) {
        format!("{}.", chars[..4].iter().collect::<String>())
    } else {
        chars[..3].iter().collect()
    }
}

/// Where a drum is written on a five-line percussion staff.
#[derive(Debug, Clone, Copy)]
pub(crate) struct DrumMark {
    pub step: char,
    pub octave: u8,
    pub notehead: Option<&'static str>,
    /// Played with the feet: written stems down, in the lower voice.
    pub foot: bool,
}

/// The common drum-kit legend (kick in the bottom space, snare in the third,
/// cymbals above the staff with x heads). Anything else from the GM drum map
/// goes above the staff with a triangle head, named in the instrument list.
pub(crate) fn drum_mark(key: u8) -> DrumMark {
    let (step, octave, notehead, foot) = match key {
        35 | 36 => ('F', 4, None, true),
        44 => ('D', 4, Some("x"), true),
        37 => ('C', 5, Some("x"), false),
        38 | 40 => ('C', 5, None, false),
        39 => ('C', 5, Some("x"), false),
        41 => ('G', 4, None, false),
        43 => ('A', 4, None, false),
        45 | 47 => ('B', 4, None, false),
        48 => ('D', 5, None, false),
        50 => ('E', 5, None, false),
        42 | 46 => ('G', 5, Some("x"), false),
        51 | 59 => ('F', 5, Some("x"), false),
        53 => ('F', 5, Some("diamond"), false),
        49 | 57 | 55 => ('A', 5, Some("x"), false),
        52 => ('B', 5, Some("x"), false),
        _ => ('B', 5, Some("triangle"), false),
    };
    DrumMark { step, octave, notehead, foot }
}

#[cfg(test)]
mod tests {
    use super::abbreviation;

    #[test]
    fn abbreviations_come_from_the_name_when_it_names_an_instrument() {
        assert_eq!(abbreviation(71, false, "A5-Clarinet"), "Cl.");
        assert_eq!(abbreviation(70, false, "A7-Bassoon"), "Bsn.");
        assert_eq!(abbreviation(60, false, "A8-French Horns"), "Hn.");
        assert_eq!(abbreviation(69, false, "English Horn"), "E. Hn.");
        assert_eq!(abbreviation(40, false, "第1ヴァイオリン"), "Vln.");
        assert_eq!(abbreviation(73, false, ""), "Fl.", "the program's own");
        assert_eq!(abbreviation(81, false, "chords"), "chords", "short enough as it is");
        assert_eq!(abbreviation(81, false, "Supersaw Lead"), "Supe.");
    }
}
