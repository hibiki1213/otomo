//! Reading MusicXML — and its zipped form, `.mxl` — into a [`Score`].
//!
//! Scores on the web are more often MusicXML than MIDI, and a MusicXML file
//! says exactly what is written: pitches, lengths, parts, key and metre.
//! Taking it as data is what keeps a piece from being copied out by eye, with
//! the mistakes that brings.
//!
//! What playback needs is carried over: notes (tied notes joined, grace notes
//! left out), parts and their instruments (transposing ones at sounding
//! pitch), key, time, tempo and dynamics, and repeats with first and second
//! endings, played out in order. Jumps (D.C., D.S., coda) are not followed;
//! the import says so when a score has them.

use std::collections::HashMap;
use std::io::{Cursor, Read};
use std::str::FromStr;

use roxmltree::{Document, Node, ParsingOptions};

use crate::gm::program_number;
use crate::{KeyChange, Note, Score, TempoChange, TimeSigChange, Track};

const PPQ: u32 = 480;
const DEFAULT_BPM: f64 = 120.0;
/// mf, for a score that marks no dynamics.
const DEFAULT_VELOCITY: u8 = 80;
/// A score unzips to a few megabytes at most; this only stops a zip bomb.
const MAX_UNZIPPED: u64 = 64 * 1024 * 1024;
/// Repeats that play out longer than this come from a broken file.
const MAX_PLAYED_MEASURES: usize = 20_000;

pub struct MusicXml {
    pub score: Score,
    pub title: Option<String>,
    /// What the import did or could not do, in words, for whoever asked.
    pub remarks: Vec<String>,
}

/// Whether these bytes are MusicXML, zipped or not. Decided by content:
/// servers and file names both label it every which way.
pub fn is_musicxml(bytes: &[u8]) -> bool {
    if bytes.starts_with(b"PK\x03\x04") {
        return zip_root(bytes).is_ok_and(|xml| names_a_score(&xml));
    }
    names_a_score(bytes)
}

fn names_a_score(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(8192)];
    let text = match head {
        [0xFF, 0xFE, ..] | [0xFE, 0xFF, ..] => utf16(head),
        _ => String::from_utf8_lossy(head).into_owned(),
    };
    text.contains("<score-partwise") || text.contains("<score-timewise")
}

pub fn read_musicxml(bytes: &[u8]) -> Result<MusicXml, String> {
    let xml = if bytes.starts_with(b"PK\x03\x04") { zip_root(bytes)? } else { bytes.to_vec() };
    let text = decode(&xml)?;
    let options = ParsingOptions { allow_dtd: true, ..ParsingOptions::default() };
    let doc = Document::parse_with_options(&text, options)
        .map_err(|e| format!("MusicXML を読めませんでした: {e}"))?;
    let root = doc.root_element();
    match root.tag_name().name() {
        "score-partwise" => {}
        "score-timewise" => {
            return Err("timewise 形式の MusicXML には未対応です（partwise で書き出したものを使ってください）".into())
        }
        other => return Err(format!("MusicXML ではありません（<{other}>）")),
    }

    let infos = part_list(root);
    let mut graces = 0usize;
    let parts: Vec<Part> = root
        .children()
        .filter(|n| n.has_tag_name("part"))
        .map(|node| {
            let info = node.attribute("id").and_then(|id| infos.get(id));
            read_part(node, info, &mut graces)
        })
        .collect();
    if parts.iter().all(|p| p.measures.iter().all(|m| m.notes.is_empty())) {
        return Err("音符が1つもない楽譜です。".into());
    }

    let mut remarks = Vec::new();
    let score = assemble(&parts, &mut remarks);
    if graces > 0 {
        remarks.push(format!("装飾音符（{graces}個）は省きました。"));
    }
    Ok(MusicXml { score, title: title(root), remarks })
}

/* ---------- reading ---------- */

#[derive(Clone, Copy)]
struct RawNote {
    /// Ticks from the start of the measure.
    start: u32,
    len: u32,
    key: u8,
    velocity: u8,
    tie_start: bool,
    tie_stop: bool,
}

#[derive(Default)]
struct Measure {
    notes: Vec<RawNote>,
    /// How far the music in it reaches, in ticks.
    filled: u32,
    /// Marked as not a full bar (a pickup, or half of a bar split by a repeat).
    implicit: bool,
    time: Option<(u8, u16)>,
    key: Option<(i8, bool)>,
    /// (ticks into the measure, quarter notes per minute)
    tempos: Vec<(u32, f64)>,
    forward_repeat: bool,
    /// How many times the passage is played in all.
    backward_repeat: Option<u32>,
    /// The passes this measure is played on, when it is under a volta.
    ending: Option<Vec<u32>>,
    /// D.C., D.S., segno, coda or fine: jumps this import does not follow.
    jump: bool,
}

struct Part {
    name: String,
    program: u8,
    percussion: bool,
    /// Semitones from written to sounding pitch (−2 for a B♭ trumpet).
    transpose: i32,
    measures: Vec<Measure>,
}

#[derive(Default)]
struct PartInfo {
    name: String,
    program: Option<u8>,
    percussion: bool,
    /// Drum sounds by instrument id, as MIDI keys.
    unpitched: HashMap<String, u8>,
}

fn part_list(root: Node) -> HashMap<String, PartInfo> {
    let mut out = HashMap::new();
    let Some(list) = child(root, "part-list") else { return out };
    for part in list.children().filter(|n| n.has_tag_name("score-part")) {
        let Some(id) = part.attribute("id") else { continue };
        let mut info = PartInfo { name: text_of(part, "part-name").unwrap_or_default(), ..Default::default() };
        // MIDI numbers in MusicXML count from 1.
        for midi in part.children().filter(|n| n.has_tag_name("midi-instrument")) {
            if num_of::<u8>(midi, "midi-channel") == Some(10) {
                info.percussion = true;
            }
            if info.program.is_none() {
                info.program = num_of::<u8>(midi, "midi-program").map(|p| p.clamp(1, 128) - 1);
            }
            if let (Some(id), Some(key)) = (midi.attribute("id"), num_of::<u8>(midi, "midi-unpitched")) {
                info.unpitched.insert(id.to_string(), key.clamp(1, 128) - 1);
            }
        }
        out.insert(id.to_string(), info);
    }
    out
}

fn read_part(node: Node, info: Option<&PartInfo>, graces: &mut usize) -> Part {
    let blank = PartInfo::default();
    let info = info.unwrap_or(&blank);
    let mut divisions = 1u32;
    let mut transpose = 0i32;
    let mut velocity = DEFAULT_VELOCITY;
    let mut ending: Option<Vec<u32>> = None;
    let mut saw_unpitched = false;
    let mut measures = Vec::new();

    for measure in node.children().filter(|n| n.has_tag_name("measure")) {
        let mut m = Measure { implicit: measure.attribute("implicit") == Some("yes"), ..Default::default() };
        let mut cursor = 0i64;
        let mut last_start = 0i64;
        let mut reach = 0i64;
        let mut close_ending = false;

        for el in measure.children().filter(Node::is_element) {
            match el.tag_name().name() {
                "attributes" => {
                    if let Some(d) = num_of::<u32>(el, "divisions") {
                        divisions = d.max(1);
                    }
                    if let Some(key) = child(el, "key") {
                        if let Some(fifths) = num_of::<i8>(key, "fifths") {
                            let minor = text_of(key, "mode").as_deref() == Some("minor");
                            m.key.get_or_insert((fifths.clamp(-7, 7), minor));
                        }
                    }
                    if let Some(time) = child(el, "time").and_then(signature) {
                        m.time = Some(time);
                    }
                    // Written pitch plus this is what sounds.
                    if let Some(t) = child(el, "transpose") {
                        transpose = num_of::<i32>(t, "chromatic").unwrap_or(0)
                            + 12 * num_of::<i32>(t, "octave-change").unwrap_or(0);
                    }
                }
                "note" => {
                    if child(el, "grace").is_some() {
                        *graces += 1;
                        continue;
                    }
                    let chord = child(el, "chord").is_some();
                    let len = ticks(num_of::<f64>(el, "duration").unwrap_or(0.0), divisions);
                    let start = if chord { last_start } else { cursor };
                    if !chord {
                        last_start = cursor;
                        cursor += len;
                        reach = reach.max(cursor);
                    }
                    // Cue notes take up time but belong to another part.
                    if len == 0 || child(el, "rest").is_some() || child(el, "cue").is_some() {
                        continue;
                    }
                    let key = if let Some(pitch) = child(el, "pitch") {
                        written_key(pitch, "step", "alter", "octave").map(|k| k + transpose)
                    } else if let Some(unpitched) = child(el, "unpitched") {
                        saw_unpitched = true;
                        child(el, "instrument")
                            .and_then(|i| i.attribute("id"))
                            .and_then(|id| info.unpitched.get(id))
                            .map(|k| i32::from(*k))
                            .or_else(|| written_key(unpitched, "display-step", "", "display-octave"))
                    } else {
                        None
                    };
                    let Some(key) = key.filter(|k| (0..=127).contains(k)) else { continue };
                    let (tie_start, tie_stop) = ties(el);
                    let velocity = el
                        .attribute("dynamics")
                        .and_then(|d| d.parse::<f64>().ok())
                        .map(dynamics_velocity)
                        .unwrap_or(velocity);
                    m.notes.push(RawNote {
                        start: start.max(0) as u32,
                        len: len as u32,
                        key: key as u8,
                        velocity,
                        tie_start,
                        tie_stop,
                    });
                }
                "backup" => cursor = (cursor - ticks(num_of(el, "duration").unwrap_or(0.0), divisions)).max(0),
                "forward" => {
                    cursor += ticks(num_of(el, "duration").unwrap_or(0.0), divisions);
                    reach = reach.max(cursor);
                }
                "direction" => {
                    let mut tempo_given = false;
                    for sound in el.children().filter(|n| n.has_tag_name("sound")) {
                        tempo_given |= read_sound(sound, cursor, &mut m, &mut velocity);
                    }
                    for kind in el.children().filter(|n| n.has_tag_name("direction-type")) {
                        if let Some(metronome) = child(kind, "metronome").filter(|_| !tempo_given) {
                            if let Some(bpm) = metronome_bpm(metronome) {
                                m.tempos.push((cursor.max(0) as u32, bpm));
                            }
                        }
                        if let Some(mark) = child(kind, "dynamics").and_then(|d| d.children().find(Node::is_element)) {
                            if let Some(v) = mark_velocity(mark.tag_name().name()) {
                                velocity = v;
                            }
                        }
                        if child(kind, "segno").is_some() || child(kind, "coda").is_some() {
                            m.jump = true;
                        }
                        for words in kind.children().filter(|n| n.has_tag_name("words")) {
                            m.jump |= is_jump(words.text().unwrap_or(""));
                        }
                    }
                }
                "sound" => {
                    read_sound(el, cursor, &mut m, &mut velocity);
                }
                "barline" => {
                    if let Some(repeat) = child(el, "repeat") {
                        match repeat.attribute("direction") {
                            Some("forward") => m.forward_repeat = true,
                            Some("backward") => {
                                let times = repeat.attribute("times").and_then(|t| t.parse().ok());
                                m.backward_repeat = Some(times.unwrap_or(2).clamp(2, 16));
                            }
                            _ => {}
                        }
                    }
                    if let Some(e) = child(el, "ending") {
                        match e.attribute("type") {
                            Some("start") => ending = Some(ending_numbers(e.attribute("number").unwrap_or("1"))),
                            Some("stop") | Some("discontinue") => close_ending = true,
                            _ => {}
                        }
                    }
                    if child(el, "segno").is_some() || child(el, "coda").is_some() {
                        m.jump = true;
                    }
                }
                _ => {}
            }
        }
        m.filled = reach.max(0) as u32;
        m.ending = ending.clone();
        if close_ending {
            ending = None;
        }
        measures.push(m);
    }

    let name = if info.name.trim().is_empty() { "Part".to_string() } else { info.name.trim().to_string() };
    // Without a MIDI program, the part's name is the best guess there is.
    let program = info.program.or_else(|| program_number(&name).ok()).unwrap_or(0);
    Part { name, program, percussion: info.percussion || saw_unpitched, transpose, measures }
}

/// Reads tempo, dynamics and jumps off a `<sound>`. True if it set a tempo.
fn read_sound(sound: Node, cursor: i64, m: &mut Measure, velocity: &mut u8) -> bool {
    if ["dacapo", "dalsegno", "tocoda", "fine", "segno", "coda"].iter().any(|a| sound.attribute(*a).is_some()) {
        m.jump = true;
    }
    if let Some(d) = sound.attribute("dynamics").and_then(|d| d.parse::<f64>().ok()) {
        *velocity = dynamics_velocity(d);
    }
    match sound.attribute("tempo").and_then(|t| t.parse::<f64>().ok()).filter(|t| *t > 0.0) {
        Some(bpm) => {
            m.tempos.push((cursor.max(0) as u32, bpm.clamp(10.0, 400.0)));
            true
        }
        None => false,
    }
}

/// "♩. = 88" is 132 quarter notes a minute; the score keeps quarters.
fn metronome_bpm(metronome: Node) -> Option<f64> {
    let per_minute: f64 = text_of(metronome, "per-minute")?
        .chars()
        .filter(|c| c.is_ascii_digit() || *c == '.')
        .collect::<String>()
        .parse()
        .ok()?;
    let unit = match text_of(metronome, "beat-unit")?.as_str() {
        "whole" => 4.0,
        "half" => 2.0,
        "quarter" => 1.0,
        "eighth" => 0.5,
        "16th" => 0.25,
        _ => return None,
    };
    let dotted = if child(metronome, "beat-unit-dot").is_some() { 1.5 } else { 1.0 };
    Some((per_minute * unit * dotted).clamp(10.0, 400.0))
}

/// MusicXML gives dynamics as a percentage of forte, which MIDI plays at 90.
fn dynamics_velocity(percent: f64) -> u8 {
    (percent * 0.9).round().clamp(1.0, 127.0) as u8
}

fn mark_velocity(mark: &str) -> Option<u8> {
    Some(match mark {
        "ppp" => 30,
        "pp" => 42,
        "p" => 56,
        "mp" => 70,
        "mf" => 80,
        "f" => 96,
        "ff" => 110,
        "fff" => 122,
        "sf" | "sfz" | "fz" => 105,
        _ => return None,
    })
}

fn is_jump(words: &str) -> bool {
    let w = words.to_ascii_lowercase();
    ["d.c.", "d.s.", "da capo", "dal segno", "coda", "fine"].iter().any(|j| w.contains(j))
}

fn signature(time: Node) -> Option<(u8, u16)> {
    // "3+2" over 8 is five eighths.
    let beats: u32 = text_of(time, "beats")?.split('+').filter_map(|b| b.trim().parse::<u32>().ok()).sum();
    let beat_type = num_of::<u16>(time, "beat-type")?;
    let ok = (1..=255).contains(&beats) && beat_type.is_power_of_two() && beat_type <= 64;
    ok.then_some((beats as u8, beat_type))
}

/// "1, 2" → [1, 2]; "1-3" → [1, 2, 3].
fn ending_numbers(text: &str) -> Vec<u32> {
    let mut out = Vec::new();
    for piece in text.split([',', ' ']).filter(|p| !p.is_empty()) {
        match piece.split_once('-') {
            Some((a, b)) => {
                if let (Ok(a), Ok(b)) = (a.trim().parse::<u32>(), b.trim().parse::<u32>()) {
                    out.extend(a..=b.min(a + 16));
                }
            }
            None => out.extend(piece.trim().parse::<u32>().ok()),
        }
    }
    if out.is_empty() {
        out.push(1);
    }
    out
}

/// `tie` is what sounds; `tied` (under notations) is only the drawing, but
/// some writers give only that.
fn ties(note: Node) -> (bool, bool) {
    let mut kinds: Vec<&str> =
        note.children().filter(|n| n.has_tag_name("tie")).filter_map(|t| t.attribute("type")).collect();
    if kinds.is_empty() {
        if let Some(notations) = child(note, "notations") {
            kinds = notations
                .children()
                .filter(|n| n.has_tag_name("tied"))
                .filter_map(|t| t.attribute("type"))
                .collect();
        }
    }
    (kinds.contains(&"start"), kinds.contains(&"stop"))
}

fn written_key(node: Node, step: &str, alter: &str, octave: &str) -> Option<i32> {
    let semitone = match text_of(node, step)?.as_str() {
        "C" => 0,
        "D" => 2,
        "E" => 4,
        "F" => 5,
        "G" => 7,
        "A" => 9,
        "B" => 11,
        _ => return None,
    };
    // Microtones round to the nearest key.
    let alter = if alter.is_empty() { 0 } else { num_of::<f64>(node, alter).unwrap_or(0.0).round() as i32 };
    let octave = num_of::<i32>(node, octave)?;
    Some((octave + 1) * 12 + semitone + alter)
}

fn ticks(duration: f64, divisions: u32) -> i64 {
    (duration * f64::from(PPQ) / f64::from(divisions.max(1))).round().max(0.0) as i64
}

fn title(root: Node) -> Option<String> {
    child(root, "work")
        .and_then(|w| text_of(w, "work-title"))
        .or_else(|| text_of(root, "movement-title"))
        .or_else(|| {
            root.children()
                .filter(|n| n.has_tag_name("credit"))
                .find(|c| text_of(*c, "credit-type").as_deref() == Some("title"))
                .and_then(|c| text_of(c, "credit-words"))
        })
        .map(|t| t.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|t| !t.is_empty())
}

/* ---------- putting it together ---------- */

fn assemble(parts: &[Part], remarks: &mut Vec<String>) -> Score {
    let count = parts.iter().map(|p| p.measures.len()).max().unwrap_or(0);
    // Repeats and endings are the same in every part; the first has them.
    let order = play_order(&parts[0].measures);
    if order.len() != count {
        remarks.push(format!(
            "繰り返しを展開しました（楽譜の{count}小節 → 演奏順で{}小節）。",
            order.len()
        ));
    }
    if parts.iter().flat_map(|p| &p.measures).any(|m| m.jump) {
        remarks.push(
            "D.C.・D.S.・コーダ などの指示は展開していません。必要なら copy_bars で補ってください。".into(),
        );
    }

    // Each written measure's length and the signature that gives it.
    let mut nominal = (4u8, 4u16);
    let mut lengths = Vec::with_capacity(count);
    let mut signatures = Vec::with_capacity(count);
    for k in 0..count {
        let here = || parts.iter().filter_map(move |p| p.measures.get(k));
        if let Some(time) = here().find_map(|m| m.time) {
            nominal = time;
        }
        let full = bar_ticks(nominal);
        let filled = here().map(|m| m.filled).max().unwrap_or(0);
        // A short measure is only taken at its word where short measures
        // belong; elsewhere it is a file that left out its trailing rests.
        let short_allowed = k == 0 || k + 1 == count || here().any(|m| m.implicit);
        let len = if filled > full || (filled > 0 && filled < full && short_allowed) { filled } else { full };
        lengths.push(len);
        signatures.push(if len == full { nominal } else { fit_signature(len, nominal.1).unwrap_or(nominal) });
    }

    // Key from a part that sounds as written if there is one, else from a
    // transposing part taken to concert pitch. A drum part's key means nothing.
    let key_part = parts
        .iter()
        .filter(|p| !p.percussion && p.measures.iter().any(|m| m.key.is_some()))
        .min_by_key(|p| p.transpose != 0);

    let mut score = Score {
        ppq: PPQ,
        tempos: Vec::new(),
        time_signatures: Vec::new(),
        key_signatures: Vec::new(),
        tracks: Vec::new(),
        end_tick: 0,
        text: Vec::new(),
        conductor_extra: Vec::new(),
        markers: Vec::new(),
    };
    let mut starts = Vec::with_capacity(order.len());
    let mut tick = 0u32;
    for &k in &order {
        starts.push(tick);
        let sig = signatures[k];
        if score.time_signatures.last().is_none_or(|t| (t.numerator, t.denominator) != sig) {
            score.time_signatures.push(TimeSigChange { tick, numerator: sig.0, denominator: sig.1 });
        }
        if let Some((part, (written, minor))) =
            key_part.and_then(|p| Some((p, p.measures.get(k)?.key?)))
        {
            let sharps = concert_fifths(written, part.transpose);
            if score.key_signatures.last().is_none_or(|c| (c.sharps, c.minor) != (sharps, minor)) {
                score.key_signatures.push(KeyChange { tick, sharps, minor });
            }
        }
        let mut marks: Vec<(u32, f64)> =
            parts.iter().filter_map(|p| p.measures.get(k)).flat_map(|m| m.tempos.iter().copied()).collect();
        marks.sort_by_key(|(at, _)| *at);
        marks.dedup_by_key(|(at, _)| *at);
        for (at, bpm) in marks {
            let us_per_beat = (60_000_000.0 / bpm).round() as u32;
            if score.tempos.last().is_none_or(|t| t.us_per_beat != us_per_beat) {
                score.tempos.push(TempoChange { tick: tick + at.min(lengths[k]), us_per_beat });
            }
        }
        tick += lengths[k];
    }
    if score.tempos.is_empty() {
        remarks.push(format!("テンポの指定がなかったので ♩={DEFAULT_BPM} にしました。"));
    }
    if score.tempos.first().is_none_or(|t| t.tick > 0) {
        let us_per_beat = score.tempos.first().map_or((60_000_000.0 / DEFAULT_BPM) as u32, |t| t.us_per_beat);
        score.tempos.insert(0, TempoChange { tick: 0, us_per_beat });
    }

    let mut next_channel = 0u8;
    for part in parts {
        let notes = lay_out(part, &order, &starts);
        if notes.is_empty() {
            continue;
        }
        let channel = if part.percussion {
            9
        } else {
            let c = next_channel;
            // Channel 10 is the drum kit; past 16 parts, channels are shared.
            next_channel = match c + 1 {
                9 => 10,
                16 => 0,
                n => n,
            };
            c
        };
        score.tracks.push(Track {
            name: part.name.clone(),
            channel,
            program: if part.percussion { 0 } else { part.program },
            notes,
            passthrough: Vec::new(),
        });
    }
    score.end_tick = score.tracks.iter().flat_map(|t| t.notes.iter().map(Note::end)).max().unwrap_or(0);
    score
}

/// One part's notes in playing order, with tied notes joined into one.
fn lay_out(part: &Part, order: &[usize], starts: &[u32]) -> Vec<Note> {
    let mut notes: Vec<Note> = Vec::new();
    // Key → the note a tie from it is still waiting to extend.
    let mut tied: HashMap<u8, usize> = HashMap::new();
    for (i, &k) in order.iter().enumerate() {
        let Some(measure) = part.measures.get(k) else { continue };
        let mut raw = measure.notes.clone();
        raw.sort_by_key(|n| (n.start, n.key));
        for n in raw {
            let start = starts[i] + n.start;
            if n.tie_stop {
                if let Some(&j) = tied.get(&n.key) {
                    let held = &mut notes[j];
                    // Rounding may leave a tick between the two halves.
                    if held.end().abs_diff(start) <= 2 {
                        held.len = start + n.len - held.start;
                        if !n.tie_start {
                            tied.remove(&n.key);
                        }
                        continue;
                    }
                }
            }
            if n.tie_start {
                tied.insert(n.key, notes.len());
            } else {
                tied.remove(&n.key);
            }
            notes.push(Note { start, len: n.len.max(1), key: n.key, velocity: n.velocity });
        }
    }
    notes.sort_by_key(|n| (n.start, n.key));
    // Two voices on one pitch at once are one sound.
    notes.dedup_by(|later, kept| {
        let same = later.start == kept.start && later.key == kept.key;
        if same {
            kept.len = kept.len.max(later.len);
        }
        same
    });
    notes
}

/// The order the measures are played in, repeats and endings followed.
fn play_order(measures: &[Measure]) -> Vec<usize> {
    let mut order = Vec::new();
    let mut taken: HashMap<usize, u32> = HashMap::new();
    let mut section = 0usize;
    let mut pass = 1u32;
    let mut returning = false;
    let mut after_ending = false;
    let mut i = 0usize;
    while i < measures.len() && order.len() < MAX_PLAYED_MEASURES {
        let m = &measures[i];
        // A new passage starts at a start-repeat, or right after the last
        // ending — unless this is the jump back into the passage itself.
        if !returning && (m.forward_repeat || (m.ending.is_none() && after_ending)) {
            section = i;
            pass = 1;
        }
        returning = false;
        if let Some(passes) = &m.ending {
            if !passes.contains(&pass) {
                i += 1;
                continue;
            }
        }
        order.push(i);
        after_ending = m.ending.is_some();
        if let Some(times) = m.backward_repeat {
            let done = taken.entry(i).or_insert(1);
            if *done < times {
                *done += 1;
                pass += 1;
                i = section;
                returning = true;
                continue;
            }
            // A later end-repeat with no start of its own returns to here.
            section = i + 1;
            pass = 1;
        }
        i += 1;
    }
    order
}

/// A written key signature at sounding pitch: every semitone of
/// transposition is seven fifths round the circle.
fn concert_fifths(written: i8, transpose: i32) -> i8 {
    let mut fifths = i32::from(written) + (transpose * 7).rem_euclid(12);
    while fifths > 7 {
        fifths -= 12;
    }
    while fifths < -7 {
        fifths += 12;
    }
    fifths as i8
}

fn bar_ticks((numerator, denominator): (u8, u16)) -> u32 {
    u32::from(numerator) * PPQ * 4 / u32::from(denominator.max(1))
}

/// A signature whose bar is exactly `len` ticks: how a pickup or a bar split
/// by a repeat keeps the bar lines after it where they belong.
fn fit_signature(len: u32, preferred: u16) -> Option<(u8, u16)> {
    [preferred, 4, 8, 16, 32, 64].into_iter().find_map(|den| {
        let unit = PPQ * 4 / u32::from(den);
        (len % unit == 0).then(|| len / unit).filter(|n| (1..=255).contains(n)).map(|n| (n as u8, den))
    })
}

/* ---------- files ---------- */

/// The score inside an .mxl: the file `META-INF/container.xml` names, or else
/// the first MusicXML file outside META-INF.
fn zip_root(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let mut zip =
        zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| format!("ZIP を開けませんでした: {e}"))?;
    let named = read_entry(&mut zip, "META-INF/container.xml").ok().and_then(|c| rootfile(&c));
    let path = match named {
        Some(path) => path,
        None => zip
            .file_names()
            .find(|n| !n.starts_with("META-INF/") && (n.ends_with(".xml") || n.ends_with(".musicxml")))
            .map(str::to_owned)
            .ok_or("ZIP の中に MusicXML がありません。")?,
    };
    read_entry(&mut zip, &path)
}

fn read_entry(zip: &mut zip::ZipArchive<Cursor<&[u8]>>, name: &str) -> Result<Vec<u8>, String> {
    let file = zip.by_name(name).map_err(|e| format!("{name}: {e}"))?;
    let mut out = Vec::new();
    file.take(MAX_UNZIPPED + 1).read_to_end(&mut out).map_err(|e| format!("{name}: {e}"))?;
    if out.len() as u64 > MAX_UNZIPPED {
        return Err("展開すると大きすぎる ZIP です。".into());
    }
    Ok(out)
}

fn rootfile(container: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(container).ok()?;
    let doc = Document::parse(text).ok()?;
    let found = doc.descendants().filter(|n| n.has_tag_name("rootfile")).find(|n| {
        // A container may also list a PDF of the same score.
        n.attribute("media-type").is_none_or(|t| t.contains("musicxml"))
    })?;
    found.attribute("full-path").map(str::to_owned)
}

/// MusicXML is UTF-8, or UTF-16 with a byte-order mark (some Finale exports).
fn decode(bytes: &[u8]) -> Result<String, String> {
    match bytes {
        [0xEF, 0xBB, 0xBF, rest @ ..] => String::from_utf8(rest.to_vec()).map_err(|_| unreadable()),
        [0xFF, 0xFE, ..] | [0xFE, 0xFF, ..] => Ok(utf16(bytes)),
        _ => String::from_utf8(bytes.to_vec()).map_err(|_| unreadable()),
    }
}

fn unreadable() -> String {
    "文字コードを読めません（UTF-8 か UTF-16 の MusicXML にしてください）。".into()
}

/// From a byte-order mark on. Lossy, so a head cut short for sniffing still
/// reads.
fn utf16(bytes: &[u8]) -> String {
    let big_endian = bytes.starts_with(&[0xFE, 0xFF]);
    let units: Vec<u16> = bytes[2..]
        .chunks_exact(2)
        .map(|c| if big_endian { u16::from_be_bytes([c[0], c[1]]) } else { u16::from_le_bytes([c[0], c[1]]) })
        .collect();
    String::from_utf16_lossy(&units)
}

/* ---------- tree helpers ---------- */

fn child<'a, 'i>(node: Node<'a, 'i>, name: &str) -> Option<Node<'a, 'i>> {
    node.children().find(|c| c.has_tag_name(name))
}

fn text_of(node: Node, name: &str) -> Option<String> {
    child(node, name).and_then(|c| c.text()).map(|t| t.trim().to_string()).filter(|t| !t.is_empty())
}

fn num_of<T: FromStr>(node: Node, name: &str) -> Option<T> {
    text_of(node, name)?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// A pickup, a repeat with first and second endings, a tie across the
    /// barline into the first ending, a B♭ trumpet, a drum and a grace note.
    const SCORE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE score-partwise PUBLIC "-//Recordare//DTD MusicXML 4.0 Partwise//EN" "http://www.musicxml.org/dtds/partwise.dtd">
<score-partwise version="4.0">
  <work><work-title>Test  Anthem</work-title></work>
  <part-list>
    <score-part id="P1"><part-name>Trumpet in Bb</part-name>
      <midi-instrument id="P1-I1"><midi-channel>1</midi-channel><midi-program>57</midi-program></midi-instrument>
    </score-part>
    <score-part id="P2"><part-name>Drums</part-name>
      <midi-instrument id="P2-I39"><midi-channel>10</midi-channel><midi-unpitched>39</midi-unpitched></midi-instrument>
    </score-part>
  </part-list>
  <part id="P1">
    <measure number="0" implicit="yes">
      <attributes><divisions>2</divisions><key><fifths>0</fifths></key>
        <time><beats>4</beats><beat-type>4</beat-type></time>
        <transpose><diatonic>-1</diatonic><chromatic>-2</chromatic></transpose></attributes>
      <direction><direction-type><metronome><beat-unit>quarter</beat-unit><per-minute>104</per-minute></metronome></direction-type><sound tempo="104"/></direction>
      <note><pitch><step>D</step><octave>5</octave></pitch><duration>2</duration><voice>1</voice><type>quarter</type></note>
    </measure>
    <measure number="1">
      <barline location="left"><repeat direction="forward"/></barline>
      <note><pitch><step>D</step><octave>5</octave></pitch><duration>2</duration></note>
      <note><pitch><step>D</step><octave>5</octave></pitch><duration>2</duration></note>
      <note><pitch><step>E</step><octave>5</octave></pitch><duration>4</duration><tie type="start"/></note>
    </measure>
    <measure number="2">
      <barline location="left"><ending number="1" type="start"/></barline>
      <note><pitch><step>E</step><octave>5</octave></pitch><duration>4</duration><tie type="stop"/></note>
      <note><rest/><duration>4</duration></note>
      <barline location="right"><ending number="1" type="stop"/><repeat direction="backward"/></barline>
    </measure>
    <measure number="3">
      <barline location="left"><ending number="2" type="start"/></barline>
      <note><grace/><pitch><step>G</step><octave>5</octave></pitch></note>
      <note><pitch><step>F</step><octave>5</octave></pitch><duration>8</duration></note>
      <note><chord/><pitch><step>A</step><octave>5</octave></pitch><duration>8</duration></note>
      <barline location="right"><ending number="2" type="discontinue"/></barline>
    </measure>
  </part>
  <part id="P2">
    <measure number="0" implicit="yes">
      <attributes><divisions>2</divisions></attributes>
      <note><unpitched><display-step>C</display-step><display-octave>5</display-octave></unpitched><duration>2</duration><instrument id="P2-I39"/></note>
    </measure>
    <measure number="1"><note><rest measure="yes"/><duration>8</duration></note></measure>
    <measure number="2"><note><rest measure="yes"/><duration>8</duration></note></measure>
    <measure number="3"><note><rest measure="yes"/><duration>8</duration></note></measure>
  </part>
</score-partwise>"#;

    fn keys_at(track: &Track) -> Vec<(u32, u32, u8)> {
        track.notes.iter().map(|n| (n.start, n.len, n.key)).collect()
    }

    #[test]
    fn a_score_plays_as_written() {
        assert!(is_musicxml(SCORE.as_bytes()));
        let read = read_musicxml(SCORE.as_bytes()).unwrap();
        assert_eq!(read.title.as_deref(), Some("Test Anthem"));
        let s = &read.score;

        // A one-beat pickup, then 4/4; B♭ major at 104.
        let sigs: Vec<_> = s.time_signatures.iter().map(|t| (t.tick, t.numerator, t.denominator)).collect();
        assert_eq!(sigs, [(0, 1, 4), (480, 4, 4)]);
        assert_eq!((s.key_signatures[0].sharps, s.key_signatures[0].minor), (-2, false));
        assert_eq!(s.tempos[0].bpm().round(), 104.0);

        // Pickup, bars 1–2, bar 1 again, then the second ending — at
        // sounding pitch, a tone below what the trumpet reads.
        let trumpet = &s.tracks[0];
        assert_eq!((trumpet.program, trumpet.channel), (56, 0));
        let second = 480 + 1920 * 2;
        assert_eq!(
            keys_at(trumpet),
            [
                (0, 480, 72),
                (480, 480, 72),
                (960, 480, 72),
                (1440, 1920, 74), // tied into the first ending
                (second, 480, 72),
                (second + 480, 480, 72),
                (second + 960, 960, 74), // the tie's other half is skipped on the way round
                (second + 1920, 1920, 75),
                (second + 1920, 1920, 79),
            ]
        );

        let drums = &s.tracks[1];
        assert_eq!((drums.channel, keys_at(drums)), (9, vec![(0, 480, 38)]));

        assert!(read.remarks.iter().any(|r| r.contains("4小節 → 演奏順で5小節")), "{:?}", read.remarks);
        assert!(read.remarks.iter().any(|r| r.contains("装飾音符（1個）")));

        // What goes into the library is an ordinary MIDI file.
        let back = Score::parse(&s.to_smf().unwrap()).unwrap();
        assert_eq!(back.tracks.len(), 2);
        assert_eq!(back.bars().count, 5);
    }

    #[test]
    fn a_zipped_score_reads_the_same() {
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        zip.start_file("META-INF/container.xml", stored).unwrap();
        zip.write_all(
            br#"<?xml version="1.0"?><container><rootfiles>
                <rootfile full-path="score.pdf" media-type="application/pdf"/>
                <rootfile full-path="score.xml" media-type="application/vnd.recordare.musicxml+xml"/>
            </rootfiles></container>"#,
        )
        .unwrap();
        zip.start_file("score.xml", stored).unwrap();
        zip.write_all(SCORE.as_bytes()).unwrap();
        let bytes = zip.finish().unwrap().into_inner();

        assert!(is_musicxml(&bytes));
        let read = read_musicxml(&bytes).unwrap();
        assert_eq!(read.score.tracks[0].notes.len(), 9);
    }

    #[test]
    fn other_files_are_not_taken_for_scores() {
        assert!(!is_musicxml(b"MThd\0\0\0\x06"));
        assert!(!is_musicxml(b"<html><body>score-partwise</body></html>"));
        assert!(read_musicxml(b"<svg xmlns='http://www.w3.org/2000/svg'/>").is_err());
    }

    #[test]
    fn short_measures_become_signatures() {
        assert_eq!(fit_signature(480, 4), Some((1, 4)));
        assert_eq!(fit_signature(720, 4), Some((3, 8)));
        assert_eq!(fit_signature(160, 4), None); // a triplet eighth cannot be a bar
        assert_eq!(ending_numbers("1, 2"), [1, 2]);
        assert_eq!(ending_numbers("1-3"), [1, 2, 3]);
        // B♭ instruments read a tone up, E♭ a sixth up, horn in F a fifth up.
        assert_eq!(concert_fifths(0, -2), -2);
        assert_eq!(concert_fifths(3, -9), 0);
        assert_eq!(concert_fifths(1, -7), 0);
    }
}
