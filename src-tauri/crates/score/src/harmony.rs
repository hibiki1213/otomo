//! Reading a score the way an arranger does: as chords.
//!
//! Thirty staves of notes is the wrong way to ask what a passage *is*. The
//! answer is usually four symbols a bar, and that is a hundred times less to
//! read — which is the difference between a piece being workable and not.
//!
//! The method is deliberately plain: for each beat, weigh every pitch class by
//! how long it sounds, score that against the usual chord shapes at all twelve
//! roots, and take the best. Adjacent beats that agree are merged, so a bar of
//! one harmony reports once.

use super::{BarMap, Score};

/// The chord qualities worth naming, as intervals from the root.
const SHAPES: [(&str, &[u8]); 11] = [
    ("", &[0, 4, 7]),
    ("m", &[0, 3, 7]),
    ("7", &[0, 4, 7, 10]),
    ("maj7", &[0, 4, 7, 11]),
    ("m7", &[0, 3, 7, 10]),
    ("m7b5", &[0, 3, 6, 10]),
    ("dim", &[0, 3, 6]),
    ("dim7", &[0, 3, 6, 9]),
    ("aug", &[0, 4, 8]),
    ("sus4", &[0, 5, 7]),
    ("6", &[0, 4, 7, 9]),
];

const SHARP: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
const FLAT: [&str; 12] = ["C", "Db", "D", "Eb", "E", "F", "Gb", "G", "Ab", "A", "Bb", "B"];

/// One stretch of one harmony.
pub struct Chord {
    pub bar: u32,
    /// 1-based beat within the bar.
    pub beat: f64,
    /// "F", "Bbmaj7", "G7/B", or "—" where nothing is sounding.
    pub name: String,
}

/// Name the harmony beat by beat over `from..=to`, merging repeats.
pub fn analyse(score: &Score, bars: &BarMap, tracks: &[usize], from: u32, to: u32) -> Vec<Chord> {
    let flats = score.key_signatures.first().is_some_and(|k| k.sharps < 0);
    let (from, to) = (from.max(1), to.min(bars.count));
    let mut out: Vec<Chord> = Vec::new();

    for bar in from..=to {
        let bar_start = bars.bar_start(bar);
        let beats = u32::from(bars.time_signature(bar).0.max(1));
        let beat_ticks = (bars.bar_ticks(bar) / beats).max(1);
        for beat in 0..beats {
            let start = bar_start + beat * beat_ticks;
            let name = name_at(score, tracks, start, start + beat_ticks, flats);
            // A held harmony is one chord, not four identical ones.
            if out.last().is_some_and(|c| c.name == name) {
                continue;
            }
            out.push(Chord { bar, beat: f64::from(beat) + 1.0, name });
        }
    }
    out
}

/// How much each pitch class sounds in a window, and what the lowest note is.
fn sounding(score: &Score, tracks: &[usize], start: u32, end: u32) -> ([f64; 12], Option<u8>) {
    let mut weight = [0.0f64; 12];
    let mut bass: Option<u8> = None;
    for &index in tracks {
        let Some(track) = score.tracks.get(index) else { continue };
        if track.is_percussion() {
            continue;
        }
        for note in &track.notes {
            let overlap = note.end().min(end).saturating_sub(note.start.max(start));
            if overlap == 0 {
                continue;
            }
            // Long notes and loud notes carry more of the harmony; a passing
            // sixteenth should not rename the chord.
            weight[usize::from(note.key) % 12] +=
                f64::from(overlap) * (0.4 + f64::from(note.velocity) / 127.0);
            if bass.is_none_or(|b| note.key < b) {
                bass = Some(note.key);
            }
        }
    }
    (weight, bass)
}

fn name_at(score: &Score, tracks: &[usize], start: u32, end: u32, flats: bool) -> String {
    let (weight, bass) = sounding(score, tracks, start, end);
    let total: f64 = weight.iter().sum();
    if total <= 0.0 {
        return "—".into();
    }

    let names = if flats { FLAT } else { SHARP };
    let mut best = (f64::NEG_INFINITY, 0usize, "");
    for root in 0..12usize {
        for (label, intervals) in SHAPES {
            let inside: f64 =
                intervals.iter().map(|i| weight[(root + usize::from(*i)) % 12]).sum();
            let outside = total - inside;
            // Reward what fits, punish what does not, and lean towards the
            // simpler shape when two explain the notes equally well.
            let score_of = inside - outside * 1.4 - intervals.len() as f64 * total * 0.02;
            // The root wants to be heard, especially in the bass.
            let root_bonus = weight[root] * 0.25
                + bass.map_or(0.0, |b| {
                    if usize::from(b) % 12 == root { total * 0.15 } else { 0.0 }
                });
            let value = score_of + root_bonus;
            if value > best.0 {
                best = (value, root, label);
            }
        }
    }
    // Nothing explains these notes: say so rather than inventing a chord.
    if best.0 < total * 0.1 {
        return "?".into();
    }

    let (_, root, label) = best;
    let mut name = format!("{}{label}", names[root]);
    if let Some(bass) = bass {
        if usize::from(bass) % 12 != root {
            name.push('/');
            name.push_str(names[usize::from(bass) % 12]);
        }
    }
    name
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Score;

    fn piece(notes: &str, key: i8) -> Score {
        let mut score = Score::blank(120.0, 4, 4);
        score.add_track(0, "Piano", false);
        score.set_key(key, false).unwrap();
        score.write_bars(0, 1, 4, notes).unwrap();
        score
    }

    fn read(score: &Score, to: u32) -> Vec<String> {
        let bars = score.bars();
        analyse(score, &bars, &[0], 1, to)
            .into_iter()
            .map(|c| format!("b{}:{} {}", c.bar, c.beat, c.name))
            .collect()
    }

    #[test]
    fn names_a_plain_progression() {
        let score = piece(
            "b1  [C3 E4 G4]@1:4\nb2  [F3 A4 C5]@1:4\nb3  [G3 B4 D5 F5]@1:4\nb4  [C3 E4 G4]@1:4",
            0,
        );
        assert_eq!(read(&score, 4), vec!["b1:1 C", "b2:1 F", "b3:1 G7", "b4:1 C"]);
    }

    #[test]
    fn hears_a_change_inside_a_bar() {
        let score = piece("b1  [C3 E4 G4]@1:2 [F3 A4 C5]@3:2", 0);
        assert_eq!(read(&score, 1), vec!["b1:1 C", "b1:3 F"]);
    }

    #[test]
    fn spells_the_chords_the_way_the_key_does() {
        let flat = piece("b1  [Bb2 D4 F4]@1:4", -2);
        assert_eq!(read(&flat, 1), vec!["b1:1 Bb"]);
        let sharp = piece("b1  [F#2 A#3 C#4]@1:4", 3);
        assert_eq!(read(&sharp, 1), vec!["b1:1 F#"]);
    }

    #[test]
    fn says_when_the_bass_is_not_the_root() {
        let score = piece("b1  [E2 C4 E4 G4]@1:4", 0);
        assert_eq!(read(&score, 1), vec!["b1:1 C/E"]);
    }

    #[test]
    fn minor_and_seventh_shapes() {
        let score = piece(
            "b1  [A2 C4 E4]@1:4\nb2  [D3 F4 A4 C5]@1:4\nb3  [E3 G#3 B3 D4]@1:4\nb4  [A2 C4 E4]@1:4",
            0,
        );
        assert_eq!(read(&score, 4), vec!["b1:1 Am", "b2:1 Dm7", "b3:1 E7", "b4:1 Am"]);
    }

    #[test]
    fn silence_and_nonsense_are_reported_as_such() {
        let mut score = Score::blank(120.0, 4, 4);
        score.add_track(0, "Piano", false);
        score.write_bars(0, 1, 3, "b1  C4@1:4\nb2  -\nb3  C4@1:4").unwrap();
        let bars = score.bars();
        let read = analyse(&score, &bars, &[0], 1, 3);
        assert_eq!(read[0].name, "C", "a single note still implies its triad's root");
        assert!(
            read.iter().any(|c| c.bar == 2 && c.name == "—"),
            "a bar of rest says nothing: {:?}",
            read.iter().map(|c| format!("b{} {}", c.bar, c.name)).collect::<Vec<_>>()
        );
    }
}
