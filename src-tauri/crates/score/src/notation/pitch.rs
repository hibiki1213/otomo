//! Spelling: which letter a MIDI key is written on, and which accidentals
//! the bar needs.
//!
//! A key number says nothing about whether it is G♯ or A♭. The key does: in
//! E major it is G♯, in C minor A♭. Every spelling is a position on the line
//! of fifths (… B♭ F C G D A E B F♯ …), and the right one is the position
//! nearest the key's own — with major keys leaning to the flat side for
//! their chromatic notes (B♭, E♭, A♭ in C) and minor keys to the sharp side,
//! where their leading tones are.

use std::collections::HashMap;

/// A written note: letter (0 = C … 6 = B), alteration, octave (C4 = middle C).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Spelled {
    pub step: u8,
    pub alter: i8,
    pub octave: i8,
}

const LETTERS: [char; 7] = ['C', 'D', 'E', 'F', 'G', 'A', 'B'];
/// Semitones above C for each letter.
const NATURAL: [i32; 7] = [0, 2, 4, 5, 7, 9, 11];
/// The letters in fifths order, starting from F (position −1).
const BY_FIFTH: [u8; 7] = [3, 0, 4, 1, 5, 2, 6];

impl Spelled {
    pub(crate) fn letter(self) -> char {
        LETTERS[usize::from(self.step)]
    }
}

/// Spell `key` in a key signature of `fifths` sharps (negative: flats).
pub(crate) fn spell(key: u8, fifths: i8, minor: bool) -> Spelled {
    let pc = i32::from(key) % 12;
    // Positions p on the line of fifths with 7p ≡ pc (mod 12); 7 is its own
    // inverse mod 12.
    let base = (7 * pc) % 12;
    let centre = f64::from(fifths) + if minor { 2.5 } else { 1.5 };
    let p = [base - 24, base - 12, base, base + 12]
        .into_iter()
        .min_by(|a, b| {
            (f64::from(*a) - centre).abs().total_cmp(&(f64::from(*b) - centre).abs())
        })
        .expect("four candidates");
    let step = BY_FIFTH[(p + 1).rem_euclid(7) as usize];
    let alter = (p + 1).div_euclid(7) as i8;
    let octave = ((i32::from(key) - i32::from(alter) - NATURAL[usize::from(step)]) / 12 - 1) as i8;
    Spelled { step, alter, octave }
}

/// The alteration the key signature gives a letter.
fn key_alter(step: u8, fifths: i8) -> i8 {
    const SHARPS: [u8; 7] = [3, 0, 4, 1, 5, 2, 6]; // F C G D A E B
    const FLATS: [u8; 7] = [6, 2, 5, 1, 4, 0, 3]; // B E A D G C F
    let order = |list: &[u8; 7]| list.iter().position(|s| *s == step).unwrap_or(7) as i8;
    if fifths > 0 && order(&SHARPS) < fifths {
        1
    } else if fifths < 0 && order(&FLATS) < -fifths {
        -1
    } else {
        0
    }
}

/// What has been written so far in a bar, on one staff: an accidental holds
/// for its own line (letter and octave) to the bar line.
pub(crate) struct BarAccidentals {
    fifths: i8,
    seen: HashMap<(u8, i8), i8>,
}

impl BarAccidentals {
    pub(crate) fn new(fifths: i8) -> Self {
        Self { fifths, seen: HashMap::new() }
    }

    /// The accidental to print for this note, if any. A tied continuation
    /// prints none and leaves the bar's state alone.
    pub(crate) fn mark(&mut self, note: Spelled, tied_from_before: bool) -> Option<&'static str> {
        if tied_from_before {
            return None;
        }
        let line = (note.step, note.octave);
        let current = self.seen.get(&line).copied().unwrap_or_else(|| key_alter(note.step, self.fifths));
        if current == note.alter {
            return None;
        }
        self.seen.insert(line, note.alter);
        Some(match note.alter {
            2 => "double-sharp",
            1 => "sharp",
            0 => "natural",
            -1 => "flat",
            _ => "flat-flat",
        })
    }
}

/// From sounding to written: up `diatonic` letters and `chromatic`
/// semitones. A B♭ clarinet is (1, 2): it reads D for a sounding C.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Interval {
    pub diatonic: i32,
    pub chromatic: i32,
}

impl Interval {
    pub(crate) const NONE: Interval = Interval { diatonic: 0, chromatic: 0 };

    /// Whole octaves, which even a concert-pitch score keeps (a piccolo is
    /// still written an octave down).
    pub(crate) fn octaves(self) -> i32 {
        self.chromatic / 12
    }

    pub(crate) fn octave_part(self) -> Interval {
        let o = self.octaves();
        Interval { diatonic: 7 * o, chromatic: 12 * o }
    }

    pub(crate) fn is_none(self) -> bool {
        self == Interval::NONE
    }

    /// How far it moves a key signature around the circle of fifths.
    fn fifths(self) -> i32 {
        7 * self.chromatic - 12 * self.diatonic
    }

    /// The written key signature for a concert one. Past six sharps or flats
    /// the enharmonic key is the one anybody would read.
    pub(crate) fn key(self, fifths: i8) -> i8 {
        let mut f = i32::from(fifths) + self.fifths();
        while f > 6 {
            f -= 12;
        }
        while f < -6 {
            f += 12;
        }
        if i32::from(fifths) == 7 || i32::from(fifths) == -7 {
            // The concert key itself was written at the extreme; stay there.
            f = (i32::from(fifths) + self.fifths()).clamp(-7, 7);
        }
        f as i8
    }

    /// The key a player of this instrument reads ("B♭", "E♭", "F"), for the
    /// part name; None for octave transpositions.
    pub(crate) fn instrument_key(self) -> Option<&'static str> {
        match self.chromatic.rem_euclid(12) {
            0 => None,
            2 => Some("B♭"),
            3 => Some("A"),
            7 => Some("F"),
            9 => Some("E♭"),
            10 => Some("D"),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(s: Spelled) -> String {
        let acc = match s.alter {
            1 => "#",
            -1 => "b",
            2 => "##",
            -2 => "bb",
            _ => "",
        };
        format!("{}{acc}{}", s.letter(), s.octave)
    }

    #[test]
    fn chromatic_notes_follow_the_key() {
        // C major: C# Eb F# Ab Bb.
        let c: Vec<String> = (60..72).map(|k| name(spell(k, 0, false))).collect();
        assert_eq!(c, ["C4", "C#4", "D4", "Eb4", "E4", "F4", "F#4", "G4", "Ab4", "A4", "Bb4", "B4"]);
        // A minor's leading tone is G#, not Ab.
        assert_eq!(name(spell(68, 0, true)), "G#4");
        // Gb major has a Cb, and it is in octave 4 although it sounds as B3.
        assert_eq!(name(spell(59, -6, false)), "Cb4");
        // F# major has an E#.
        assert_eq!(name(spell(65, 6, false)), "E#4");
        // B# sits in the octave below the C it sounds as.
        assert_eq!(name(spell(60, 7, false)), "B#3");
    }

    #[test]
    fn accidentals_last_to_the_bar_line_on_their_own_line() {
        let mut bar = BarAccidentals::new(0);
        let fs4 = spell(66, 0, false);
        assert_eq!(bar.mark(fs4, false), Some("sharp"));
        assert_eq!(bar.mark(fs4, false), None, "already sharp in this bar");
        assert_eq!(bar.mark(spell(65, 0, false), false), Some("natural"));
        assert_eq!(bar.mark(spell(78, 0, false), false), Some("sharp"), "another octave");
        // In G major F# is in the signature, and F natural needs a sign.
        let mut g = BarAccidentals::new(1);
        assert_eq!(g.mark(spell(66, 1, false), false), None);
        assert_eq!(g.mark(spell(65, 1, false), false), Some("natural"));
    }

    #[test]
    fn transposing_moves_the_key_signature() {
        let clarinet = Interval { diatonic: 1, chromatic: 2 };
        assert_eq!(clarinet.key(0), 2, "C major concert is D major for a B♭ clarinet");
        assert_eq!(clarinet.key(-2), 0);
        let alto = Interval { diatonic: 5, chromatic: 9 };
        assert_eq!(alto.key(0), 3);
        assert_eq!(alto.key(4), -5, "E major + 3 sharps reads as D♭, not C♯");
        let horn = Interval { diatonic: 4, chromatic: 7 };
        assert_eq!(horn.key(-1), 0);
        assert_eq!(horn.instrument_key(), Some("F"));
        let tenor = Interval { diatonic: 8, chromatic: 14 };
        assert_eq!(tenor.octaves(), 1);
        assert_eq!(tenor.instrument_key(), Some("B♭"));
    }
}
