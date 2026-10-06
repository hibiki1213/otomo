//! The song as a page of music, written out as MusicXML.
//!
//! A MIDI file stores a performance; a score needs the intent behind it. So
//! positions are snapped to the subdivision each beat actually uses
//! (sixteenths here, triplets there), notes that overlap become a second
//! voice — or, on an instrument that plays one note at a time, a legato —
//! and lengths are written as the note values a copyist would choose, tied
//! where a bar line or the beat says they must be. Pitches are spelled for
//! the key, and transposing instruments are written as their players read.
//!
//! The result is plain MusicXML, so any notation program can take it further;
//! the app engraves the same file into PDF pages.

mod instrument;
mod pitch;
mod rhythm;
#[cfg(test)]
mod tests;
mod xml;

use std::collections::{HashMap, HashSet};

use crate::text::{chord_window, same_strike};
use crate::{gm, BarMap, Score, Track};
use instrument::{Clef, DrumMark, Family};
use pitch::{BarAccidentals, Interval, Spelled};
use rhythm::{BarContext, Beam, Meter, Value, TRIPLET_SPAN};

/// Divisions per quarter note: a 64th is 3, a triplet eighth 16.
pub(crate) const DIV: u32 = 48;

pub struct ExportOptions {
    pub title: String,
    /// Tracks to write, in the order they should appear.
    pub tracks: Vec<usize>,
    /// Transposing instruments as their players read them (a B♭ clarinet a
    /// tone up). Off, everything is at concert pitch except the octave
    /// transpositions every score keeps (piccolo, guitar, double bass).
    pub transposed: bool,
    /// Runs of empty bars as one multi-bar rest, as in a player's part.
    pub multi_rests: bool,
    /// Only the first bars (a quick preview of a long piece).
    pub max_bars: Option<u32>,
}

/// The tracks as a MusicXML 4.0 score.
pub fn to_musicxml(score: &Score, options: &ExportOptions) -> Result<String, String> {
    Ok(xml::write(&Sheet::build(score, options)?))
}

/* ---------- the page, as the writer sees it ---------- */

pub(crate) struct Sheet {
    pub title: String,
    pub bars: Vec<Bar>,
    pub parts: Vec<PartSheet>,
}

pub(crate) struct Bar {
    /// As printed: a pickup is bar 0.
    pub number: u32,
    pub implicit: bool,
    /// Divisions from the start of the piece.
    pub start: u32,
    pub len: u32,
    /// The time signature printed (for a pickup, the one that follows it).
    pub meter: Meter,
    pub show_meter: bool,
    /// Concert key: sharps (negative: flats), minor.
    pub key: (i8, bool),
    pub show_key: bool,
    /// (offset, quarter notes per minute).
    pub tempos: Vec<(u32, f64)>,
    pub rehearsals: Vec<(u32, String)>,
    tick: u32,
    ticks: u32,
}

impl Bar {
    /// How far into a full bar this one begins (a pickup ends on the bar line).
    pub fn shift(&self) -> u32 {
        self.meter.len().saturating_sub(self.len)
    }
}

pub(crate) struct PartSheet {
    pub name: String,
    pub abbreviation: String,
    pub program: u8,
    pub channel: u8,
    pub percussion: bool,
    pub family: Family,
    /// Written = sounding + this.
    pub transposition: Interval,
    pub clefs: Vec<Clef>,
    /// The drums this part plays, by key.
    pub drums: Vec<u8>,
    pub bars: Vec<PartBar>,
}

pub(crate) struct PartBar {
    /// Staff 1's first voice comes first; it always fills the bar.
    pub voices: Vec<VoiceBar>,
    pub dynamics: Vec<(u32, &'static str, u8)>,
    /// Starts a run of this many empty bars, shown as one rest.
    pub multi_rest: Option<u32>,
}

pub(crate) struct VoiceBar {
    pub staff: u8,
    pub voice: u8,
    pub stem: Option<&'static str>,
    pub items: Vec<Item>,
}

pub(crate) struct Item {
    pub offset: u32,
    pub len: u32,
    /// None: a whole-bar rest.
    pub value: Option<Value>,
    /// Empty: a rest.
    pub heads: Vec<Head>,
    pub tuplet_start: bool,
    pub tuplet_stop: bool,
    pub beams: Vec<Beam>,
    pub staccato: bool,
}

pub(crate) struct Head {
    pub pitch: HeadPitch,
    pub velocity: u8,
    pub tie_stop: bool,
    pub tie_start: bool,
    pub accidental: Option<&'static str>,
}

pub(crate) enum HeadPitch {
    /// Before the bar's key is known: the sounding MIDI key.
    Sounding(u8),
    Pitched(Spelled),
    Unpitched { key: u8, mark: DrumMark },
}

impl Sheet {
    fn build(score: &Score, options: &ExportOptions) -> Result<Self, String> {
        if options.tracks.is_empty() {
            return Err("Choose at least one track to export.".into());
        }
        if let Some(t) = options.tracks.iter().find(|t| **t >= score.tracks.len()) {
            return Err(format!("There is no track {t}."));
        }
        let mut bars = layout_bars(score, &score.bars());
        if let Some(max) = options.max_bars {
            bars.truncate(max.max(1) as usize);
        }
        let mut parts: Vec<PartSheet> =
            options.tracks.iter().map(|&t| build_part(score, &score.tracks[t], &bars, options)).collect();
        if options.multi_rests {
            for part in &mut parts {
                mark_multi_rests(part, &bars);
            }
        }
        Ok(Sheet { title: options.title.clone(), bars, parts })
    }
}

/* ---------- bars ---------- */

fn meter_of(map: &BarMap, bar: u32) -> Meter {
    let (num, den) = map.time_signature(bar);
    Meter { num, den }
}

fn layout_bars(score: &Score, map: &BarMap) -> Vec<Bar> {
    // A first bar shorter than the second's signature is a pickup: printed
    // under the real signature, not as a bar of 1/4.
    let pickup = map.count >= 2 && meter_of(map, 1).len() < meter_of(map, 2).len();
    let mut out: Vec<Bar> = Vec::with_capacity(map.count as usize);
    let mut start = 0u32;
    for b in 1..=map.count {
        let len = meter_of(map, b).len();
        let meter = if pickup && b == 1 { meter_of(map, 2) } else { meter_of(map, b) };
        let tick = map.bar_start(b);
        let key = score
            .key_signatures
            .iter()
            .rev()
            .find(|k| k.tick <= tick)
            .map_or((0, false), |k| (k.sharps, k.minor));
        let (show_meter, show_key) = match out.last() {
            None => (true, true),
            Some(prev) => (prev.meter != meter, prev.key != key),
        };
        out.push(Bar {
            number: if pickup { b - 1 } else { b },
            implicit: pickup && b == 1,
            start,
            len,
            meter,
            show_meter,
            key,
            show_key,
            tempos: Vec::new(),
            rehearsals: Vec::new(),
            tick,
            ticks: map.bar_ticks(b),
        });
        start += len;
    }

    // Tempo marks, but not every step of a ritardando: a change is shown if
    // it holds for a bar, and the last one always is.
    let mut shown: Option<f64> = None;
    for (i, t) in score.tempos.iter().enumerate() {
        let bpm = t.bpm();
        let holds = score.tempos.get(i + 1).is_none_or(|next| next.tick - t.tick >= map.bar_ticks(map.position(t.tick).0));
        if !(i == 0 || holds) || shown.is_some_and(|s| (s - bpm).abs() < 0.5) {
            continue;
        }
        shown = Some(bpm);
        if let Some((bar, offset)) = locate(&out, t.tick) {
            out[bar].tempos.push((offset, bpm));
        }
    }
    for (tick, text) in &score.markers {
        let text = text.trim();
        if text.is_empty() {
            continue;
        }
        if let Some((bar, offset)) = locate(&out, *tick) {
            out[bar].rehearsals.push((offset, text.to_string()));
        }
    }
    out
}

/// The bar a tick falls in, and the offset (snapped to a sixteenth) in it.
fn locate(bars: &[Bar], tick: u32) -> Option<(usize, u32)> {
    let i = bars.iter().rposition(|b| b.tick <= tick)?;
    let b = &bars[i];
    let raw = f64::from(tick - b.tick) / f64::from(b.ticks.max(1)) * f64::from(b.len);
    let offset = ((raw / f64::from(DIV / 4)).round() as u32 * (DIV / 4)).min(b.len.saturating_sub(1));
    Some((i, offset))
}

/// A tick as divisions from the start, not yet snapped.
fn divisions(bars: &[Bar], tick: u32) -> f64 {
    let i = bars.iter().rposition(|b| b.tick <= tick).unwrap_or(0);
    let b = &bars[i];
    f64::from(b.start) + f64::from(tick.saturating_sub(b.tick)) / f64::from(b.ticks.max(1)) * f64::from(b.len)
}

/* ---------- snapping to the grid each beat uses ---------- */

/// Which grid applies where: (bar index, metric start of the region).
type Grids = HashMap<(usize, u32), u32>;

struct Grid<'a> {
    bars: &'a [Bar],
    grids: Grids,
}

impl Grid<'_> {
    /// (bar index, metric offset in the bar, region start) for a position.
    fn place(&self, pos: f64) -> (usize, f64, u32) {
        let i = self.bars.iter().rposition(|b| f64::from(b.start) <= pos + 1e-6).unwrap_or(0);
        let b = &self.bars[i];
        let metric = (pos - f64::from(b.start)).max(0.0) + f64::from(b.shift());
        let region = b.meter.region();
        (i, metric, (metric / f64::from(region)).floor() as u32 * region)
    }

    fn step_at(&self, pos: f64) -> u32 {
        let (i, _, r) = self.place(pos);
        self.grids.get(&(i, r)).copied().unwrap_or(DIV / 4)
    }

    fn snap(&self, pos: f64) -> u32 {
        let (i, metric, r) = self.place(pos);
        let b = &self.bars[i];
        let step = self.grids.get(&(i, r)).copied().unwrap_or(DIV / 4);
        let snapped = f64::from(r) + ((metric - f64::from(r)) / f64::from(step)).round() * f64::from(step);
        let offset = (snapped.max(0.0) as u32).saturating_sub(b.shift()).min(b.len);
        b.start + offset
    }
}

/// Pick a grid per region from the onsets (and, when they matter, the ends)
/// that fall in it: the plain sixteenth unless triplets fit clearly better,
/// and something finer only when the coarse grids would merge notes.
fn choose_grids(bars: &[Bar], onsets: &[f64], ends: &[f64]) -> Grids {
    let probe = Grid { bars, grids: HashMap::new() };
    let mut regions: HashMap<(usize, u32), (Vec<f64>, Vec<f64>)> = HashMap::new();
    for &p in onsets {
        let (i, metric, r) = probe.place(p);
        regions.entry((i, r)).or_default().0.push(metric - f64::from(r));
    }
    for &p in ends {
        let (i, metric, r) = probe.place(p);
        regions.entry((i, r)).or_default().1.push(metric - f64::from(r));
    }
    regions
        .into_iter()
        .map(|((i, r), (mut on, off))| {
            on.sort_by(f64::total_cmp);
            on.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
            let size = bars[i].meter.region();
            ((i, r), pick_grid(size, &on, &off))
        })
        .collect()
}

fn pick_grid(region: u32, onsets: &[f64], ends: &[f64]) -> u32 {
    let tiers: &[&[u32]] = if region == TRIPLET_SPAN {
        &[&[12, 16], &[6, 8]]
    } else if region % 12 == 0 {
        &[&[12], &[6]]
    } else {
        &[&[6], &[3]]
    };
    let error = |g: u32| -> f64 {
        let g = f64::from(g);
        let all = onsets.iter().chain(ends);
        let n = onsets.len() + ends.len();
        if n == 0 {
            return 0.0;
        }
        all.map(|x| (x - (x / g).round() * g).abs() / g).sum::<f64>() / n as f64
    };
    let merges = |g: u32| -> bool {
        let slots: Vec<i64> = onsets.iter().map(|x| (x / f64::from(g)).round() as i64).collect();
        slots.windows(2).any(|w| w[0] == w[1])
    };
    for tier in tiers {
        let fits: Vec<(u32, f64)> = tier.iter().filter(|g| !merges(**g)).map(|&g| (g, error(g))).collect();
        if let Some(&(first, first_err)) = fits.first() {
            // The plainer grid wins unless the other is clearly closer.
            return fits
                .iter()
                .skip(1)
                .find(|(_, e)| *e + 0.05 < first_err)
                .map_or(first, |(g, _)| *g);
        }
    }
    tiers.last().and_then(|t| t.first()).copied().unwrap_or(DIV / 8)
}

/* ---------- voices ---------- */

#[derive(Debug, Clone)]
struct Event {
    start: u32,
    end: u32,
    /// (key, velocity), lowest first.
    keys: Vec<(u8, u8)>,
    /// Written longer than it sounds, with a dot.
    staccato: bool,
}

struct QNote {
    start: u32,
    end: u32,
    key: u8,
    velocity: u8,
}

/// The track's notes on the grid. Notes struck together (by the performance's
/// own timing) share one onset, so a rolled chord stays a chord.
fn quantize(score: &Score, track: &Track, bars: &[Bar]) -> Vec<QNote> {
    let window = chord_window(score.ppq);
    let mut onset_of = vec![0f64; track.notes.len()];
    let mut i = 0;
    while i < track.notes.len() {
        let head = &track.notes[i];
        let mut j = i + 1;
        while j < track.notes.len() && same_strike((head.start, head.len), (track.notes[j].start, track.notes[j].len), window)
        {
            j += 1;
        }
        let at = divisions(bars, head.start);
        onset_of[i..j].iter_mut().for_each(|o| *o = at);
        i = j;
    }
    let ends: Vec<f64> = if track.is_percussion() {
        Vec::new() // a drum's length is how long until the next hit
    } else {
        track.notes.iter().map(|n| divisions(bars, n.end())).collect()
    };
    let grid = Grid { bars, grids: choose_grids(bars, &onset_of, &ends) };
    track
        .notes
        .iter()
        .enumerate()
        .map(|(k, n)| {
            let start = grid.snap(onset_of[k]);
            let end = if track.is_percussion() { start } else { grid.snap(divisions(bars, n.end())) };
            let end = if end > start { end } else { start + grid.step_at(onset_of[k]) };
            QNote { start, end, key: n.key, velocity: n.velocity }
        })
        .collect()
}

/// Split a staff's notes into at most `max` voices. Chords (same start and
/// end) stay together; a note that overlaps the next by a release's worth is
/// a legato, cut to where the next one starts, not a second voice.
fn voices(notes: &[&QNote], max: usize) -> Vec<Vec<Event>> {
    let mut chords: Vec<Event> = Vec::new();
    for n in notes {
        match chords.iter_mut().find(|c| c.start == n.start && c.end == n.end) {
            Some(c) => c.keys.push((n.key, n.velocity)),
            None => chords.push(Event { start: n.start, end: n.end, keys: vec![(n.key, n.velocity)], staccato: false }),
        }
    }
    for c in &mut chords {
        c.keys.sort_unstable();
        c.keys.dedup_by_key(|k| k.0);
    }
    let top = |c: &Event| c.keys.last().map_or(0, |k| k.0);
    chords.sort_by(|a, b| a.start.cmp(&b.start).then(top(b).cmp(&top(a))));

    let mut out: Vec<Vec<Event>> = Vec::new();
    for c in chords {
        if let Some(v) = out.iter_mut().find(|v| v.last().is_none_or(|l| l.end <= c.start)) {
            v.push(c);
            continue;
        }
        let legato = |l: &Event| l.start < c.start && l.end - c.start <= (DIV / 4).min((l.end - l.start) / 4);
        if let Some(v) = out.iter_mut().find(|v| v.last().is_some_and(legato)) {
            v.last_mut().expect("checked").end = c.start;
            v.push(c);
            continue;
        }
        if out.len() < max {
            out.push(vec![c]);
            continue;
        }
        // No room: cut short whichever voice would have finished first.
        let v = out.iter_mut().min_by_key(|v| v.last().map_or(0, |l| l.end)).expect("max is at least 1");
        let last = v.last_mut().expect("voices are never empty");
        if last.start < c.start {
            last.end = c.start;
            v.push(c);
        } else {
            last.keys.extend(c.keys);
            last.keys.sort_unstable();
            last.keys.dedup_by_key(|k| k.0);
            last.end = last.end.max(c.end);
        }
    }
    out
}

/// Short, detached notes — a synth stab, a played staccato — written as the
/// value up to the next note (within the beat) with a dot, rather than as a
/// sixteenth and a sixteenth rest.
fn staccato(voice: &mut [Event], bars: &[Bar]) {
    for k in 0..voice.len() {
        let next = voice.get(k + 1).map_or(u32::MAX, |n| n.start);
        let e = &mut voice[k];
        let room = next.min(beat_end(bars, e.start)).saturating_sub(e.start);
        let len = e.end - e.start;
        if room <= DIV && len * 2 <= room && len < room {
            e.end = e.start + room;
            e.staccato = true;
        }
    }
}

/// A drum kit's two voices, hands above and feet below. A hit lasts until
/// the next one in its voice, but no further than its beat.
fn drum_voices(notes: &[QNote], bars: &[Bar]) -> [Vec<Event>; 2] {
    let mut hands: Vec<Event> = Vec::new();
    let mut feet: Vec<Event> = Vec::new();
    for n in notes {
        let voice = if instrument::drum_mark(n.key).foot { &mut feet } else { &mut hands };
        match voice.iter_mut().find(|e| e.start == n.start) {
            Some(e) => e.keys.push((n.key, n.velocity)),
            None => voice.push(Event { start: n.start, end: n.start, keys: vec![(n.key, n.velocity)], staccato: false }),
        }
    }
    for voice in [&mut hands, &mut feet] {
        voice.sort_by_key(|e| e.start);
        let starts: Vec<u32> = voice.iter().map(|e| e.start).collect();
        for (k, e) in voice.iter_mut().enumerate() {
            e.keys.sort_unstable();
            e.keys.dedup_by_key(|k| k.0);
            let next = starts.get(k + 1).copied().unwrap_or(u32::MAX);
            e.end = next.min(beat_end(bars, e.start));
        }
    }
    [hands, feet]
}

/// The end of the beat a position is in.
fn beat_end(bars: &[Bar], pos: u32) -> u32 {
    let i = bars.iter().rposition(|b| b.start <= pos).unwrap_or(0);
    let b = &bars[i];
    let beat = b.meter.beat();
    let metric = pos - b.start + b.shift();
    (b.start + ((metric / beat + 1) * beat).saturating_sub(b.shift())).min(b.start + b.len)
}

/* ---------- parts ---------- */

struct VoiceLine {
    staff: u8,
    voice: u8,
    events: Vec<Event>,
    /// Always written, as rests if need be.
    primary: bool,
    /// Drums: stems fixed by the hand or foot that plays.
    fixed_stem: Option<&'static str>,
}

fn build_part(score: &Score, track: &Track, bars: &[Bar], options: &ExportOptions) -> PartSheet {
    let percussion = track.is_percussion();
    let program = track.program;
    let full = instrument::transposition(program, percussion);
    let transposition = if options.transposed { full } else { full.octave_part() };
    let notes = quantize(score, track, bars);

    let mut name = if !track.name.is_empty() {
        track.name.clone()
    } else if percussion {
        "Drum Set".to_string()
    } else {
        gm::program_name(program).to_string()
    };
    if let Some(key) = transposition.instrument_key() {
        if !name.contains(" in ") {
            name = format!("{name} in {key}");
        }
    }
    let abbreviation = instrument::abbreviation(program, percussion, &track.name);

    let (lo, hi) = notes.iter().fold((u8::MAX, 0u8), |(lo, hi), n| (lo.min(n.key), hi.max(n.key)));
    let mono = instrument::monophonic(program);
    let grand = !percussion
        && (instrument::grand_staff(program)
            || (!mono && instrument::fixed_clef(program).is_none() && !notes.is_empty() && lo < 50 && hi > 71));

    let mut lines: Vec<VoiceLine> = Vec::new();
    let clefs: Vec<Clef>;
    if percussion {
        clefs = vec![Clef::Percussion];
        let [hands, feet] = drum_voices(&notes, bars);
        lines.push(VoiceLine { staff: 1, voice: 1, events: hands, primary: true, fixed_stem: Some("up") });
        if !feet.is_empty() {
            lines.push(VoiceLine { staff: 1, voice: 2, events: feet, primary: false, fixed_stem: Some("down") });
        }
    } else {
        let staves: Vec<Vec<&QNote>> = if grand {
            clefs = vec![Clef::Treble, Clef::Bass];
            vec![notes.iter().filter(|n| n.key >= 60).collect(), notes.iter().filter(|n| n.key < 60).collect()]
        } else {
            let mut written: Vec<i32> = notes.iter().map(|n| i32::from(n.key) + transposition.chromatic).collect();
            written.sort_unstable();
            let median = written.get(written.len() / 2).copied().unwrap_or(67);
            // The instrument's own clef, unless the part lives far from it (a
            // bass clarinet on the clarinet program).
            let by_range = if median < 57 { Clef::Bass } else { Clef::Treble };
            let clef = match instrument::fixed_clef(program) {
                Some(Clef::Treble) if median < 53 => Clef::Bass,
                Some(Clef::Bass | Clef::Alto) if median > 71 => Clef::Treble,
                Some(fixed) => fixed,
                None => by_range,
            };
            clefs = vec![clef];
            vec![notes.iter().collect()]
        };
        for (s, staff_notes) in staves.iter().enumerate() {
            let staff = s as u8 + 1;
            let mut vs = voices(staff_notes, if mono { 1 } else { 2 });
            for v in &mut vs {
                staccato(v, bars);
            }
            if vs.is_empty() {
                vs.push(Vec::new());
            }
            for (k, events) in vs.into_iter().enumerate() {
                // Staff 2's voices are 5 and 6, as notation programs number them.
                let voice = (s as u8) * 4 + k as u8 + 1;
                lines.push(VoiceLine { staff, voice, events, primary: k == 0, fixed_stem: None });
            }
        }
    }

    let mut part_bars: Vec<PartBar> = bars.iter().map(|_| PartBar { voices: Vec::new(), dynamics: Vec::new(), multi_rest: None }).collect();
    for line in &lines {
        write_line(line, bars, &mut part_bars);
    }
    for (b, pb) in part_bars.iter_mut().enumerate() {
        set_stems(pb, &lines);
        if !percussion {
            let (fifths, minor) = bars[b].key;
            spell_bar(pb, transposition.key(fifths), minor, transposition.chromatic, clefs.len());
        }
    }
    dynamics(&lines, bars, &mut part_bars);

    let mut drums: Vec<u8> = if percussion { notes.iter().map(|n| n.key).collect() } else { Vec::new() };
    drums.sort_unstable();
    drums.dedup();

    PartSheet {
        name,
        abbreviation,
        program,
        channel: if percussion { 9 } else { track.channel },
        percussion,
        family: instrument::family(program, percussion),
        transposition,
        clefs,
        drums,
        bars: part_bars,
    }
}

/// One voice's events, bar by bar, as written notes and rests.
fn write_line(line: &VoiceLine, bars: &[Bar], out: &mut [PartBar]) {
    let mut first = 0usize;
    for (b, bar) in bars.iter().enumerate() {
        let (b0, b1) = (bar.start, bar.start + bar.len);
        while first < line.events.len() && line.events[first].end <= b0 {
            first += 1;
        }
        // (offset, end, event, tied in, tied out)
        let mut pieces: Vec<(u32, u32, Option<&Event>, bool, bool)> = Vec::new();
        let mut cursor = 0u32;
        for ev in line.events[first..].iter().take_while(|e| e.start < b1) {
            let (o, e) = (ev.start.max(b0) - b0, ev.end.min(b1) - b0);
            if o > cursor {
                pieces.push((cursor, o, None, false, false));
            }
            pieces.push((o, e, Some(ev), ev.start < b0, ev.end > b1 && b + 1 < bars.len()));
            cursor = e;
        }
        let has_notes = pieces.iter().any(|p| p.2.is_some());
        if !has_notes && !line.primary {
            continue;
        }
        if !has_notes {
            out[b].voices.push(VoiceBar {
                staff: line.staff,
                voice: line.voice,
                stem: line.fixed_stem,
                items: vec![Item {
                    offset: 0,
                    len: bar.len,
                    value: None,
                    heads: Vec::new(),
                    tuplet_start: false,
                    tuplet_stop: false,
                    beams: Vec::new(),
                    staccato: false,
                }],
            });
            continue;
        }
        if cursor < bar.len {
            pieces.push((cursor, bar.len, None, false, false));
        }

        // Triplet groups: quarter-note regions this voice divides in three.
        let shift = bar.shift();
        let triplets: HashSet<u32> = pieces
            .iter()
            .flat_map(|p| [p.0, p.1])
            .map(|x| x + shift)
            .filter(|m| (m % TRIPLET_SPAN) % (DIV / 8) != 0)
            .map(|m| m / TRIPLET_SPAN * TRIPLET_SPAN)
            .collect();
        let is_triplet = |r: u32| triplets.contains(&r);
        let ctx = BarContext { meter: bar.meter, shift, triplets: &is_triplet };

        let mut items: Vec<Item> = Vec::new();
        for (o, e, ev, tied_in, tied_out) in pieces {
            let written = ctx.spell(o, e, ev.is_none());
            let n = written.len();
            for (k, piece) in written.into_iter().enumerate() {
                let heads = ev.map_or_else(Vec::new, |ev| {
                    ev.keys
                        .iter()
                        .map(|&(key, velocity)| Head {
                            pitch: if line.fixed_stem.is_some() {
                                HeadPitch::Unpitched { key, mark: instrument::drum_mark(key) }
                            } else {
                                HeadPitch::Sounding(key)
                            },
                            velocity,
                            tie_stop: if k == 0 { tied_in } else { true },
                            tie_start: if k + 1 == n { tied_out } else { true },
                            accidental: None,
                        })
                        .collect()
                });
                items.push(Item {
                    offset: piece.offset,
                    len: piece.len,
                    value: Some(piece.value),
                    heads,
                    tuplet_start: false,
                    tuplet_stop: false,
                    beams: Vec::new(),
                    staccato: k == 0 && ev.is_some_and(|ev| ev.staccato),
                });
            }
        }
        mark_tuplets(&mut items, shift);
        beam(&mut items, bar.meter, shift);
        out[b].voices.push(VoiceBar { staff: line.staff, voice: line.voice, stem: line.fixed_stem, items });
    }
}

fn mark_tuplets(items: &mut [Item], shift: u32) {
    let region = |it: &Item| it.value.filter(|v| v.triplet).map(|_| (it.offset + shift) / TRIPLET_SPAN);
    let regions: Vec<Option<u32>> = items.iter().map(region).collect();
    for (k, item) in items.iter_mut().enumerate() {
        let Some(r) = regions[k] else { continue };
        item.tuplet_start = k == 0 || regions[k - 1] != Some(r);
        item.tuplet_stop = k + 1 == regions.len() || regions[k + 1] != Some(r);
    }
}

fn beam(items: &mut [Item], meter: Meter, shift: u32) {
    let group = meter.beam_group();
    let beamable = |it: &Item| -> Option<(u32, u8)> {
        let v = it.value?;
        (!it.heads.is_empty() && v.kind.beams() > 0).then(|| ((it.offset + shift) / group, v.kind.beams()))
    };
    let mut k = 0;
    while k < items.len() {
        let Some((g, _)) = beamable(&items[k]) else {
            k += 1;
            continue;
        };
        let mut end = k + 1;
        while end < items.len() && beamable(&items[end]).is_some_and(|(h, _)| h == g) {
            end += 1;
        }
        if end - k >= 2 {
            let levels: Vec<u8> = items[k..end].iter().map(|it| beamable(it).expect("in run").1).collect();
            for (it, beams) in items[k..end].iter_mut().zip(rhythm::beam_run(&levels)) {
                it.beams = beams;
            }
        }
        k = end;
    }
}

/// Two voices on a staff: the higher one stems up.
fn set_stems(bar: &mut PartBar, lines: &[VoiceLine]) {
    let staves: HashSet<u8> = bar.voices.iter().map(|v| v.staff).collect();
    for staff in staves {
        let sounding: Vec<usize> = (0..bar.voices.len())
            .filter(|&i| bar.voices[i].staff == staff && bar.voices[i].items.iter().any(|it| !it.heads.is_empty()))
            .collect();
        let fixed = lines.iter().any(|l| l.staff == staff && l.fixed_stem.is_some());
        if fixed || sounding.len() < 2 {
            continue;
        }
        let mean = |i: usize| -> f64 {
            let keys: Vec<f64> = bar.voices[i]
                .items
                .iter()
                .flat_map(|it| &it.heads)
                .filter_map(|h| match h.pitch {
                    HeadPitch::Sounding(key) => Some(f64::from(key)),
                    _ => None,
                })
                .collect();
            keys.iter().sum::<f64>() / keys.len().max(1) as f64
        };
        let (a, b) = (sounding[0], sounding[1]);
        let (up, down) = if mean(a) >= mean(b) { (a, b) } else { (b, a) };
        bar.voices[up].stem = Some("up");
        bar.voices[down].stem = Some("down");
    }
}

/// Replace the placeholder keys with spellings in the written key, and work
/// out which accidentals each staff's bar needs, in time order.
fn spell_bar(bar: &mut PartBar, fifths: i8, minor: bool, transpose: i32, staves: usize) {
    for staff in 1..=staves as u8 {
        let mut heads: Vec<(u32, usize, usize, usize)> = Vec::new();
        for (v, voice) in bar.voices.iter().enumerate().filter(|(_, v)| v.staff == staff) {
            for (i, item) in voice.items.iter().enumerate() {
                for h in 0..item.heads.len() {
                    heads.push((item.offset, v, i, h));
                }
            }
        }
        heads.sort_unstable();
        let mut state = BarAccidentals::new(fifths);
        for (_, v, i, h) in heads {
            let head = &mut bar.voices[v].items[i].heads[h];
            if let HeadPitch::Sounding(key) = head.pitch {
                let written = (i32::from(key) + transpose).clamp(0, 127) as u8;
                let spelled = pitch::spell(written, fifths, minor);
                head.pitch = HeadPitch::Pitched(spelled);
                head.accidental = state.mark(spelled, head.tie_stop);
            }
        }
    }
}

const LEVELS: [(u8, &str); 8] =
    [(16, "ppp"), (32, "pp"), (48, "p"), (64, "mp"), (80, "mf"), (96, "f"), (112, "ff"), (127, "fff")];

fn level(velocity: f64) -> (u8, &'static str) {
    *LEVELS
        .iter()
        .min_by(|a, b| (f64::from(a.0) - velocity).abs().total_cmp(&(f64::from(b.0) - velocity).abs()))
        .expect("eight levels")
}

/// Dynamics from velocities, if the part has any to speak of: a mark where
/// the level of a bar changes and the next bar that plays agrees.
fn dynamics(lines: &[VoiceLine], bars: &[Bar], out: &mut [PartBar]) {
    let all: Vec<u8> = lines.iter().flat_map(|l| &l.events).flat_map(|e| e.keys.iter().map(|k| k.1)).collect();
    if all.windows(2).all(|w| w[0] == w[1]) {
        return; // one velocity throughout: nobody marked dynamics
    }
    let per_bar: Vec<Option<(u32, &'static str, u8)>> = bars
        .iter()
        .map(|bar| {
            let (b0, b1) = (bar.start, bar.start + bar.len);
            let hits: Vec<&Event> =
                lines.iter().flat_map(|l| &l.events).filter(|e| e.start >= b0 && e.start < b1).collect();
            let first = hits.iter().map(|e| e.start).min()?;
            let vs: Vec<f64> = hits.iter().flat_map(|e| e.keys.iter().map(|k| f64::from(k.1))).collect();
            let (v, mark) = level(vs.iter().sum::<f64>() / vs.len() as f64);
            Some((first - b0, mark, v))
        })
        .collect();
    let playing: Vec<usize> = (0..bars.len()).filter(|&b| per_bar[b].is_some()).collect();
    let mut shown: Option<&str> = None;
    for (n, &b) in playing.iter().enumerate() {
        let (offset, mark, v) = per_bar[b].expect("playing");
        let lasts = playing.get(n + 1).is_none_or(|&next| per_bar[next].is_some_and(|p| p.1 == mark));
        if shown != Some(mark) && (shown.is_none() || lasts) {
            out[b].dynamics.push((offset, mark, v));
            shown = Some(mark);
        }
    }
}

/// Mark runs of bars where the part rests throughout, for a player's part.
fn mark_multi_rests(part: &mut PartSheet, bars: &[Bar]) {
    let silent: Vec<bool> =
        part.bars.iter().map(|pb| pb.voices.iter().all(|v| v.items.iter().all(|it| it.heads.is_empty()))).collect();
    let empty = |b: usize| silent[b];
    let quiet = |b: usize| {
        let bar = &bars[b];
        !bar.show_meter && !bar.show_key && bar.tempos.is_empty() && bar.rehearsals.is_empty()
    };
    // Bar 1 keeps its own rest: it carries the clef, key, time and tempo,
    // and a multi-bar rest starting there swallows them.
    let mut b = 1;
    while b < bars.len() {
        if !empty(b) || !quiet(b) {
            b += 1;
            continue;
        }
        let mut end = b + 1;
        while end < bars.len() && empty(end) && quiet(end) {
            end += 1;
        }
        if end - b >= 2 {
            part.bars[b].multi_rest = Some((end - b) as u32);
        }
        b = end;
    }
}
