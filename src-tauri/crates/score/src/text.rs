//! How a passage is written down for (and later, by) the model.
//!
//! One line per bar, notes as `pitch@beat:length`, where the beat is 1-based
//! within the bar and the length is in beats:
//!
//! ```text
//! b5   C4@1:1 E4@2:1 [C4 E4 G4]@3:2
//! b6
//! ```
//!
//! Notes struck together are grouped as a chord, which is both shorter and
//! easier to read as harmony. The syntax is meant to be symmetric: what an
//! edit tool accepts is what a read returns.
//!
//! Most MIDI in the wild is played, not typed, so positions are snapped to a
//! grid before being written down — otherwise every note reads as
//! `C4@1.688:0.188` and the music disappears behind the timing. Crucially,
//! what counts as a chord is decided from the performance's own timing, not
//! from the grid: two fast notes that land in the same slot are still two
//! notes, and printing them as a chord would be a lie.

use super::{gm, BarMap, Score};

const NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
const FLAT_NAMES: [&str; 12] =
    ["C", "Db", "D", "Eb", "E", "F", "Gb", "G", "Ab", "A", "Bb", "B"];

/// "C4", "F#3" — middle C is C4, MIDI key 60.
pub fn note_name(key: u8) -> String {
    format!("{}{}", NAMES[usize::from(key) % 12], i32::from(key) / 12 - 1)
}

/// Note names as the key signature would write them: a piece in F major says
/// Bb, not A#. The parser takes either, so this is only about reading well.
fn note_name_in(key: u8, flats: bool) -> String {
    let table = if flats { FLAT_NAMES } else { NAMES };
    format!("{}{}", table[usize::from(key) % 12], i32::from(key) / 12 - 1)
}

/// What to call a note on this kind of track. On a drum kit a pitch is not a
/// pitch but an instrument, so it is written as one.
fn sound_name(key: u8, percussion: bool, flats: bool) -> String {
    match percussion.then(|| gm::drum_name(key)).flatten() {
        Some(drum) => drum.to_string(),
        None => note_name_in(key, flats),
    }
}

/// A rendered bar, kept structured so callers can add their own framing.
pub struct BarLine {
    pub bar: u32,
    /// Empty when the bar has no notes.
    pub notes: String,
}

/// The note value positions are snapped to. `Grid::Raw` keeps the file's own
/// timing, rounded only enough to print.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grid {
    /// Denominator of the note value: 16 is a sixteenth, 12 a triplet eighth.
    Divisions(u32),
    Raw,
}

impl Grid {
    /// "1/16", "1/8", "off" — what the tools accept.
    pub fn parse(text: &str) -> Result<Self, String> {
        let text = text.trim();
        if text.eq_ignore_ascii_case("off") || text.eq_ignore_ascii_case("raw") {
            return Ok(Grid::Raw);
        }
        let divisions = text
            .strip_prefix("1/")
            .and_then(|n| n.parse::<u32>().ok())
            .filter(|n| (1..=64).contains(n))
            .ok_or_else(|| {
                format!("quantize は \"1/4\"〜\"1/64\" か \"off\" で指定してください（{text} は不明）")
            })?;
        Ok(Grid::Divisions(divisions))
    }

    pub fn label(&self) -> String {
        match self {
            Grid::Divisions(n) => format!("1/{n}"),
            Grid::Raw => "off".into(),
        }
    }

    fn ticks(&self, ppq: u32) -> Option<u32> {
        match self {
            Grid::Divisions(n) => Some((ppq * 4 / n).max(1)),
            Grid::Raw => None,
        }
    }
}

/// Round to a musically meaningful precision: anything finer than a 64th
/// triplet is a performance artefact, not a written value.
fn beats(ticks: u32, beat_ticks: u32) -> f64 {
    (f64::from(ticks) / f64::from(beat_ticks) * 48.0).round() / 48.0
}

fn snap(ticks: u32, grid: u32) -> u32 {
    (ticks + grid / 2) / grid * grid
}

fn number(value: f64) -> String {
    let s = format!("{value:.3}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s.is_empty() { "0".into() } else { s.to_string() }
}

/// How far apart two onsets can be and still be one gesture. Generous,
/// because the overlap test below is what actually decides.
pub(crate) fn chord_window(ppq: u32) -> u32 {
    (ppq / 4).max(1)
}

/// Whether a note belongs to a strike already begun by `head`.
///
/// Onset proximity alone cannot tell a rolled chord from a fast run — at 120
/// BPM, consecutive thirty-seconds are further apart than a strummed chord is
/// wide. What separates them is that a chord's notes go on sounding together:
/// they overlap. A run's notes hand over, so they do not.
pub(crate) fn same_strike(head: (u32, u32), note: (u32, u32), window: u32) -> bool {
    let (head_start, head_len) = head;
    let (start, len) = note;
    if start - head_start >= window {
        return false;
    }
    let overlap = (head_start + head_len).min(start + len).saturating_sub(start);
    overlap * 2 > head_len.min(len)
}

/// Notes sharing one onset, as they will be printed.
struct Strike {
    offset: u32,
    notes: Vec<(u32, u8, u8)>, // (length, key, velocity)
}

/// Group a bar's notes by the onset they were played on, snapping each onset
/// (and each length) to `step` if there is one.
fn strikes(raw: &[(u32, u32, u8, u8)], step: Option<u32>, window: u32, last_slot: u32) -> Vec<Strike> {
    let mut out: Vec<Strike> = Vec::new();
    let mut i = 0;
    while i < raw.len() {
        let (onset, head_len) = (raw[i].0, raw[i].1);
        let mut j = i + 1;
        while j < raw.len() && same_strike((onset, head_len), (raw[j].0, raw[j].1), window) {
            j += 1;
        }
        let offset = match step {
            Some(step) => snap(onset, step).min(last_slot),
            None => onset,
        };
        let notes = raw[i..j]
            .iter()
            .map(|(_, len, key, vel)| {
                let len = match step {
                    Some(step) => snap(*len, step).max(step),
                    None => *len,
                };
                (len, *key, *vel)
            })
            .collect();
        out.push(Strike { offset, notes });
        i = j;
    }
    out
}

/// Notes in this range, per bar, as `(offset in bar, length, key, velocity)`.
fn bar_notes(score: &Score, bars: &BarMap, track: usize, bar: u32) -> Vec<(u32, u32, u8, u8)> {
    let Some(track) = score.tracks.get(track) else {
        return Vec::new();
    };
    let start = bars.bar_start(bar);
    let end = start + bars.bar_ticks(bar);
    track
        .notes
        .iter()
        .filter(|n| n.start >= start && n.start < end)
        .map(|n| (n.start - start, n.len, n.key, n.velocity))
        .collect()
}

/// Render bars `from..=to` of one track. Bars outside the score are skipped.
pub fn render_bars(
    score: &Score,
    bars: &BarMap,
    track: usize,
    from: u32,
    to: u32,
    velocity: bool,
    grid: Grid,
) -> Vec<BarLine> {
    let Some(part) = score.tracks.get(track) else {
        return Vec::new();
    };
    let percussion = part.is_percussion();
    let step = grid.ticks(score.ppq);
    let window = chord_window(score.ppq);
    let (from, to) = (from.max(1), to.min(bars.count));
    let mut out = Vec::new();

    for bar in from..=to {
        let flats = spells_with_flats(score, bars.bar_start(bar));
        let bar_ticks = bars.bar_ticks(bar);
        let beat_ticks = bar_ticks / u32::from(bars.time_signature(bar).0.max(1));
        // Snapping must never push a note past its own bar line, or it would
        // print as a beat the time signature does not have.
        let last_slot = bar_ticks.saturating_sub(step.unwrap_or(1));

        let raw = bar_notes(score, bars, track, bar);
        let mut tokens: Vec<String> = Vec::new();
        for strike in strikes(&raw, step, window, last_slot) {
            let at = number(beats(strike.offset, beat_ticks) + 1.0);
            // One token per distinct length: notes struck together but held
            // for different times are different voices, not one chord. The
            // key is the printed length, so notes that would read alike stay
            // together instead of splitting over a tick of release.
            let mut by_length: Vec<(String, Vec<(u8, u8)>)> = Vec::new();
            for (len, key, vel) in strike.notes {
                let label = number(beats(len, beat_ticks));
                match by_length.iter_mut().find(|(l, _)| *l == label) {
                    Some((_, keys)) => keys.push((key, vel)),
                    None => by_length.push((label, vec![(key, vel)])),
                }
            }
            for (len, mut keys) in by_length {
                keys.sort_unstable();
                let pitches = if keys.len() == 1 {
                    sound_name(keys[0].0, percussion, flats)
                } else {
                    format!(
                        "[{}]",
                        keys.iter()
                            .map(|(k, _)| sound_name(*k, percussion, flats))
                            .collect::<Vec<_>>()
                            .join(" ")
                    )
                };
                let mut token = format!("{pitches}@{at}:{len}");
                if velocity {
                    let avg =
                        keys.iter().map(|(_, v)| u32::from(*v)).sum::<u32>() / keys.len() as u32;
                    token.push_str(&format!("v{avg}"));
                }
                tokens.push(token);
            }
        }
        out.push(BarLine { bar, notes: tokens.join(" ") });
    }
    out
}

/// Whether the key in effect here is written with flats.
fn spells_with_flats(score: &Score, tick: u32) -> bool {
    score
        .key_signatures
        .iter()
        .rev()
        .find(|k| k.tick <= tick)
        .is_some_and(|k| k.sharps < 0)
}

/// The coarsest grid that still keeps separate onsets apart, over the bars and
/// tracks being read. A waltz reads as `1/8`; a flourish of 32nds needs
/// `1/32`, and saying so beats silently merging the notes.
pub fn suggest_grid(score: &Score, bars: &BarMap, tracks: &[usize], from: u32, to: u32) -> Grid {
    let window = chord_window(score.ppq);
    let (from, to) = (from.max(1), to.min(bars.count));

    // Every distinct onset in the range, bar by bar, and how long the notes
    // are: a grid coarser than the notes themselves would print a drum
    // pattern of eighths as one of quarters.
    let mut per_bar: Vec<(u32, Vec<u32>)> = Vec::new();
    let mut per_track: Vec<Vec<u32>> = vec![Vec::new(); tracks.len()];
    for bar in from..=to {
        let mut onsets: Vec<u32> = Vec::new();
        for (i, &track) in tracks.iter().enumerate() {
            let raw = bar_notes(score, bars, track, bar);
            per_track[i].extend(raw.iter().map(|(_, len, ..)| *len));
            onsets.extend(strikes(&raw, None, window, u32::MAX).into_iter().map(|s| s.offset));
        }
        onsets.sort_unstable();
        onsets.dedup();
        if !onsets.is_empty() {
            per_bar.push((bars.bar_ticks(bar), onsets));
        }
    }
    let total: usize = per_bar.iter().map(|(_, o)| o.len()).sum();
    if total == 0 {
        return Grid::Divisions(16);
    }

    // The typical note of the part that moves fastest: every part on the page
    // has to be representable, and a drum pattern of eighths must not print as
    // quarters just because the strings underneath hold whole notes. The
    // median keeps one short flourish from dragging the page to thirty-seconds.
    let typical = per_track
        .iter_mut()
        .filter(|l| !l.is_empty())
        .map(|lengths| {
            lengths.sort_unstable();
            lengths[lengths.len() / 2]
        })
        .min()
        .unwrap_or(score.ppq);

    for divisions in [4u32, 8, 12, 16, 24, 32] {
        let step = (score.ppq * 4 / divisions).max(1);
        if step > typical {
            continue;
        }
        let mut merged = 0usize;
        for (bar_ticks, onsets) in &per_bar {
            let last_slot = bar_ticks.saturating_sub(step);
            let mut slots: Vec<u32> = onsets.iter().map(|o| snap(*o, step).min(last_slot)).collect();
            let before = slots.len();
            slots.dedup();
            merged += before - slots.len();
        }
        // A little merging is the point of a grid; a lot of it is a wrong one.
        if merged * 20 <= total {
            return Grid::Divisions(divisions);
        }
    }
    Grid::Divisions(32)
}
