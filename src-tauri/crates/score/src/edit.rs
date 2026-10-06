//! Changing a score.
//!
//! Every operation is scoped — a range of bars, one track, one setting — so
//! that it can be described, retried and undone as a unit. Nothing here
//! touches the file on disk; callers serialise with [`Score::to_smf`].

use super::parse_text::parse_passage;
use super::{KeyChange, Note, Passthrough, Score, TempoChange, TimeSigChange, Track};

/// What an edit did, in words, for the user to read before applying it.
pub type Summary = String;

impl Score {
    /// An empty score to build on.
    pub fn blank(bpm: f64, numerator: u8, denominator: u16) -> Self {
        Score {
            ppq: 480,
            tempos: vec![TempoChange { tick: 0, us_per_beat: bpm_to_us(bpm) }],
            time_signatures: vec![TimeSigChange { tick: 0, numerator, denominator }],
            key_signatures: Vec::new(),
            tracks: Vec::new(),
            end_tick: 0,
            text: Vec::new(),
            conductor_extra: Vec::new(),
            markers: Vec::new(),
        }
    }

    pub(crate) fn recount(&mut self) {
        self.end_tick = self
            .tracks
            .iter()
            .flat_map(|t| t.notes.iter().map(Note::end))
            .max()
            .unwrap_or(0);
    }

    fn check_track(&self, track: usize) -> Result<(), String> {
        if track < self.tracks.len() {
            return Ok(());
        }
        Err(match self.tracks.len() {
            0 => "This song has no tracks yet. Add one with add_track.".into(),
            n => format!("There is no track {track} (tracks are 0–{}).", n - 1),
        })
    }

    /// Replace everything this track plays in `from..=to` with `notes`.
    ///
    /// The range is what makes this safe to retry: whatever was there before,
    /// afterwards the range holds exactly what was written. Bars inside the
    /// range that the text does not mention end up empty.
    ///
    /// Only notes are replaced. The pedal, expression and other controllers
    /// stay as they were — partly because they usually still fit, and partly
    /// because removing half of a pedal gesture would leave the pedal down
    /// for the rest of the piece.
    pub fn write_bars(
        &mut self,
        track: usize,
        from: u32,
        to: u32,
        notes: &str,
    ) -> Result<Summary, String> {
        self.check_track(track)?;
        if to < from {
            return Err("to_bar must be at least from_bar.".into());
        }
        let bars = self.bars();
        let written = parse_passage(notes, from, self.tracks[track].is_percussion())?;
        if let Some(bar) = written.keys().find(|b| **b < from || **b > to) {
            return Err(format!(
                "Bar {bar} is outside the given range ({from}–{to}). Widen the range, or write that bar in a separate call."
            ));
        }

        let mut added: Vec<Note> = Vec::new();
        for (bar, wanted) in &written {
            let start = bars.bar_start(*bar);
            let bar_ticks = bars.bar_ticks(*bar);
            let beat_ticks = bar_ticks / u32::from(bars.time_signature(*bar).0.max(1));
            for note in wanted {
                let offset = (note.beat * f64::from(beat_ticks)).round();
                if offset < 0.0 || offset >= f64::from(bar_ticks) {
                    let (num, den) = bars.time_signature(*bar);
                    return Err(format!(
                        "Bar {bar} is in {num}/{den}, so beats must be from 1 to {}.",
                        f64::from(num) + 0.999
                    ));
                }
                let len = (note.beats_long * f64::from(beat_ticks)).round().max(1.0);
                added.push(Note {
                    start: start + offset as u32,
                    len: len as u32,
                    key: note.key,
                    velocity: note.velocity,
                });
            }
        }

        // The range is cleared first, so writing is idempotent: running the
        // same call twice leaves the same music, not two copies of it.
        let clear_from = bars.bar_start(from);
        let clear_to = bars.bar_start(to) + bars.bar_ticks(to);
        let part = &mut self.tracks[track];
        let before = part.notes.len();
        part.notes.retain(|n| n.start < clear_from || n.start >= clear_to);
        let removed = before - part.notes.len();
        let count = added.len();
        part.notes.extend(added);
        part.notes.sort_by_key(|n| (n.start, n.key));
        self.recount();

        Ok(format!(
            "Rewrote {} of track {track} \"{}\" ({} → {}){}",
            bar_span(from, to),
            self.tracks[track].display_name(),
            note_count(removed),
            note_count(count),
            self.out_of_range(track, from, to)
        ))
    }

    /// Add an empty track. Channel 9 is General MIDI percussion.
    ///
    /// The name is never left blank: a part with no notes is only recognised
    /// as a part on reload because it is named, so an unnamed empty track
    /// would disappear the moment the song was saved.
    pub fn add_track(&mut self, program: u8, name: &str, percussion: bool) -> Summary {
        let channel = if percussion { 9 } else { self.free_channel() };
        let name = match name.trim() {
            "" if percussion => "Percussion".to_string(),
            "" => format!("Track {}", self.tracks.len() + 1),
            given => given.to_string(),
        };
        self.tracks.push(Track {
            name,
            channel,
            program: program & 0x7f,
            notes: Vec::new(),
            passthrough: Vec::new(),
        });
        let added = self.tracks.last().expect("just pushed");
        format!(
            "Added track {} \"{}\" ({})",
            self.tracks.len() - 1,
            added.display_name(),
            if percussion { "Percussion" } else { crate::program_name(added.program) }
        )
    }

    /// The lowest channel no track is using, skipping percussion.
    fn free_channel(&self) -> u8 {
        (0u8..16)
            .find(|c| *c != 9 && !self.tracks.iter().any(|t| t.channel == *c))
            .unwrap_or(0)
    }

    pub fn set_track(
        &mut self,
        track: usize,
        program: Option<u8>,
        name: Option<&str>,
    ) -> Result<Summary, String> {
        self.check_track(track)?;
        let part = &mut self.tracks[track];
        if let Some(program) = program {
            part.program = program & 0x7f;
        }
        if let Some(name) = name {
            part.name = name.trim().to_string();
        }
        let part = &self.tracks[track];
        Ok(format!(
            "Set track {track} to \"{}\" ({})",
            part.display_name(),
            if part.is_percussion() { "Percussion" } else { crate::program_name(part.program) }
        ))
    }

    pub fn remove_track(&mut self, track: usize) -> Result<Summary, String> {
        self.check_track(track)?;
        let gone = self.tracks.remove(track);
        self.recount();
        Ok(format!("Removed track {track} \"{}\"", gone.display_name()))
    }

    /// Set the tempo from `from_bar` on, replacing any change already there.
    pub fn set_tempo(&mut self, bpm: f64, from_bar: u32) -> Result<Summary, String> {
        if !(10.0..=400.0).contains(&bpm) {
            return Err("The tempo must be between 10 and 400 BPM.".into());
        }
        let tick = self.bars().bar_start(from_bar.max(1));
        self.tempos.retain(|t| t.tick != tick);
        self.tempos.push(TempoChange { tick, us_per_beat: bpm_to_us(bpm) });
        self.tempos.sort_by_key(|t| t.tick);
        Ok(if from_bar <= 1 {
            format!("Set the tempo to {bpm:.0} BPM")
        } else {
            format!("Set the tempo to {bpm:.0} BPM from bar {from_bar}")
        })
    }

    /// Set the time signature from `from_bar` on. Bars after it are renumbered,
    /// and the notes stay where they are in time.
    pub fn set_time_signature(
        &mut self,
        numerator: u8,
        denominator: u16,
        from_bar: u32,
    ) -> Result<Summary, String> {
        if !(1..=32).contains(&numerator) || !matches!(denominator, 1 | 2 | 4 | 8 | 16 | 32 | 64) {
            return Err("Give the time signature like 3/4 or 6/8, with a denominator that is a power of two from 1 to 64.".into());
        }
        let tick = self.bars().bar_start(from_bar.max(1));
        self.time_signatures.retain(|t| t.tick != tick);
        self.time_signatures.push(TimeSigChange { tick, numerator, denominator });
        self.time_signatures.sort_by_key(|t| t.tick);
        Ok(if from_bar <= 1 {
            format!("Set the time signature to {numerator}/{denominator}")
        } else {
            format!("Set the time signature to {numerator}/{denominator} from bar {from_bar}")
        })
    }

    pub fn set_key(&mut self, sharps: i8, minor: bool) -> Result<Summary, String> {
        if !(-7..=7).contains(&sharps) {
            return Err("A key signature is a number of sharps/flats from -7 to 7.".into());
        }
        self.key_signatures.retain(|k| k.tick != 0);
        self.key_signatures.insert(0, KeyChange { tick: 0, sharps, minor });
        Ok(format!("Set the key to {}", self.key_signatures[0].name()))
    }

    /// Open up `count` empty bars before `at_bar`, pushing later music back.
    pub fn insert_bars(&mut self, at_bar: u32, count: u32) -> Result<Summary, String> {
        if count == 0 {
            return Err("The number of bars to insert must be at least 1.".into());
        }
        let bars = self.bars();
        let at = bars.bar_start(at_bar.max(1));
        let span: u32 = (0..count).map(|i| bars.bar_ticks(at_bar.max(1) + i)).sum();
        self.shift(at, span as i64);
        let empty = if count == 1 { "1 empty bar".to_string() } else { format!("{count} empty bars") };
        Ok(format!("Inserted {empty} before bar {at_bar}"))
    }

    /// Cut `from..=to` out; later music moves up to fill the gap.
    pub fn delete_bars(&mut self, from: u32, to: u32) -> Result<Summary, String> {
        if to < from {
            return Err("to_bar must be at least from_bar.".into());
        }
        let bars = self.bars();
        if from > bars.count {
            return Err(format!("This song ends at bar {}.", bars.count));
        }
        let start = bars.bar_start(from);
        let end = bars.bar_start(to) + bars.bar_ticks(to);
        let span = end - start;
        for track in &mut self.tracks {
            track.notes.retain(|n| n.start < start || n.start >= end);
            collapse(&mut track.passthrough, start, end);
        }
        collapse(&mut self.conductor_extra, start, end);
        self.shift(start, -(span as i64));
        Ok(format!("Deleted {}", bar_span(from, to)))
    }

    /// Move everything at or after `at` by `delta` ticks.
    fn shift(&mut self, at: u32, delta: i64) {
        let moved = |tick: u32| -> u32 {
            if tick < at {
                tick
            } else {
                (i64::from(tick) + delta).max(i64::from(at)) as u32
            }
        };
        for track in &mut self.tracks {
            for note in &mut track.notes {
                note.start = moved(note.start);
            }
            track.notes.sort_by_key(|n| (n.start, n.key));
            // Pedal and expression belong to the music, so they travel with it.
            for (tick, _) in &mut track.passthrough {
                *tick = moved(*tick);
            }
        }
        for (tick, _) in &mut self.conductor_extra {
            *tick = moved(*tick);
        }
        for (tick, _) in &mut self.markers {
            *tick = moved(*tick);
        }
        for t in &mut self.tempos {
            t.tick = moved(t.tick);
        }
        for t in &mut self.time_signatures {
            t.tick = moved(t.tick);
        }
        for k in &mut self.key_signatures {
            k.tick = moved(k.tick);
        }
        // Changes from a deleted span land on its start; of those, the last
        // is the one in effect from there on.
        keep_last(&mut self.tempos, |t| t.tick);
        keep_last(&mut self.time_signatures, |t| t.tick);
        keep_last(&mut self.key_signatures, |k| k.tick);
        self.recount();
    }
}

impl Score {
    /// The start of a new song made from this one: `tracks` (all of them if
    /// None), over bars `from..=to`, with the tempo, meter and key in effect.
    pub fn excerpt(&self, tracks: Option<&[usize]>, from: u32, to: u32) -> Result<Score, String> {
        if to < from {
            return Err("to_bar must be at least from_bar.".into());
        }
        let count = self.bars().count;
        if from > count {
            return Err(format!("This song ends at bar {count}."));
        }
        let mut out = self.clone();
        if let Some(list) = tracks {
            if list.is_empty() {
                return Err("Choose at least one track.".into());
            }
            for &t in list {
                self.check_track(t)?;
            }
            out.tracks = list.iter().map(|&t| self.tracks[t].clone()).collect();
        }
        if to < count {
            out.delete_bars(to + 1, count)?;
        }
        if from > 1 {
            out.delete_bars(1, from - 1)?;
        }
        out.recount();
        Ok(out)
    }
}

impl Track {
    pub fn display_name(&self) -> String {
        if self.name.is_empty() {
            format!("ch{}", self.channel + 1)
        } else {
            self.name.clone()
        }
    }
}

fn bpm_to_us(bpm: f64) -> u32 {
    (60_000_000.0 / bpm.clamp(10.0, 400.0)).round() as u32
}

/// "bar 3" or "bars 3–5", for the summaries.
pub(crate) fn bar_span(from: u32, to: u32) -> String {
    if from == to {
        format!("bar {from}")
    } else {
        format!("bars {from}–{to}")
    }
}

/// "1 note" or "3 notes".
pub(crate) fn note_count(n: usize) -> String {
    if n == 1 {
        "1 note".into()
    } else {
        format!("{n} notes")
    }
}

impl Score {
    /// A warning when a part has been written outside its instrument's range,
    /// appended to whatever the edit reports.
    ///
    /// Not an error: a "String Ensemble" patch may stand in for anything, and
    /// refusing the write would be worse than saying so. But a model cannot
    /// hear that the flute has been sent below middle C, so it has to be told.
    pub(crate) fn out_of_range(&self, track: usize, from: u32, to: u32) -> String {
        let Some(part) = self.tracks.get(track) else {
            return String::new();
        };
        if part.is_percussion() {
            return String::new();
        }
        let Some((lo, hi)) = crate::program_range(part.program) else {
            return String::new();
        };
        let bars = self.bars();
        let (start, end) = (bars.bar_start(from), bars.bar_start(to) + bars.bar_ticks(to));
        let stray: Vec<u8> = part
            .notes
            .iter()
            .filter(|n| n.start >= start && n.start < end)
            .map(|n| n.key)
            .filter(|k| *k < lo || *k > hi)
            .collect();
        if stray.is_empty() {
            return String::new();
        }
        let worst_low = stray.iter().copied().min().unwrap_or(lo);
        let worst_high = stray.iter().copied().max().unwrap_or(hi);
        format!(
            "\n⚠ {} outside the range of {} ({}–{}). {} covers {}–{}.",
            note_count(stray.len()),
            crate::program_name(part.program),
            crate::note_name(worst_low),
            crate::note_name(worst_high),
            crate::program_name(part.program),
            crate::note_name(lo),
            crate::note_name(hi),
        )
    }
}

/// Drop the events inside a range being cut, except the last value of each
/// thing that holds a state.
///
/// Controllers are latched: the pedal stays down until something lifts it, a
/// volume stays where it was set. Simply shifting the events out of a deleted
/// range would stack a whole passage's worth of them on the seam; simply
/// dropping them could leave the pedal down for the rest of the piece. So the
/// last value of each controller survives, at the cut, and the rest goes.
/// Of changes that share a tick, keep the last (the list is in time order).
fn keep_last<T>(list: &mut Vec<T>, tick: impl Fn(&T) -> u32) {
    list.reverse();
    list.dedup_by_key(|x| tick(x));
    list.reverse();
}

fn collapse(events: &mut Passthrough, start: u32, end: u32) {
    use midly::{MidiMessage, TrackEventKind};

    // What holds a state: a controller number, the bend, the program.
    let latch = |kind: &TrackEventKind<'_>| -> Option<(u8, u16)> {
        match kind {
            TrackEventKind::Midi { channel, message } => match message {
                MidiMessage::Controller { controller, .. } => {
                    Some((channel.as_int(), u16::from(controller.as_int())))
                }
                MidiMessage::PitchBend { .. } => Some((channel.as_int(), 1000)),
                MidiMessage::ProgramChange { .. } => Some((channel.as_int(), 1001)),
                MidiMessage::ChannelAftertouch { .. } => Some((channel.as_int(), 1002)),
                _ => None,
            },
            _ => None,
        }
    };

    let mut keep: Vec<(u32, TrackEventKind<'static>)> = Vec::new();
    let mut last: Vec<((u8, u16), TrackEventKind<'static>)> = Vec::new();
    for (tick, kind) in events.drain(..) {
        if tick < start || tick >= end {
            keep.push((tick, kind));
            continue;
        }
        if let Some(id) = latch(&kind) {
            match last.iter_mut().find(|(seen, _)| *seen == id) {
                Some((_, slot)) => *slot = kind,
                None => last.push((id, kind)),
            }
        }
    }
    keep.extend(last.into_iter().map(|(_, kind)| (start, kind)));
    keep.sort_by_key(|(tick, _)| *tick);
    *events = keep;
}

/// Which notes of a chord to take when copying a part somewhere else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Voice {
    All,
    /// The top line of the chords — the melody, usually.
    Top,
    /// The bottom line — the bass.
    Bottom,
}

impl Voice {
    pub fn parse(text: &str) -> Result<Self, String> {
        match text.trim().to_lowercase().as_str() {
            "all" | "" => Ok(Voice::All),
            "top" | "melody" | "highest" => Ok(Voice::Top),
            "bottom" | "bass" | "lowest" => Ok(Voice::Bottom),
            other => Err(format!("voice must be \"all\", \"top\" or \"bottom\" (unknown: {other})")),
        }
    }
}

impl Score {
    /// Copy a passage from one part to another — the move orchestration is
    /// mostly made of. Optionally transposed, optionally taking only the top
    /// or bottom line of the chords, optionally landing on a different bar.
    #[allow(clippy::too_many_arguments)]
    pub fn copy_bars(
        &mut self,
        from_track: usize,
        to_track: usize,
        from: u32,
        to: u32,
        at: u32,
        transpose: i32,
        voice: Voice,
        merge: bool,
    ) -> Result<Summary, String> {
        self.check_track(from_track)?;
        self.check_track(to_track)?;
        if to < from {
            return Err("to_bar must be at least from_bar.".into());
        }
        let bars = self.bars();
        let start = bars.bar_start(from);
        let end = bars.bar_start(to) + bars.bar_ticks(to);
        let shift = i64::from(bars.bar_start(at.max(1))) - i64::from(start);

        let taken: Vec<Note> = self.tracks[from_track]
            .notes
            .iter()
            .filter(|n| n.start >= start && n.start < end)
            .copied()
            .collect();
        if taken.is_empty() {
            return Err(format!("Track {from_track} has no notes in {}.", bar_span(from, to)));
        }
        let chosen = pick_voice(&taken, voice, self.ppq);

        let mut moved = Vec::with_capacity(chosen.len());
        for note in chosen {
            let key = i32::from(note.key) + transpose;
            if !(0..=127).contains(&key) {
                return Err(format!(
                    "Transposing by {transpose} semitones goes out of range ({} would fall outside MIDI's range).",
                    crate::note_name(note.key)
                ));
            }
            moved.push(Note {
                start: (i64::from(note.start) + shift).max(0) as u32,
                len: note.len,
                key: key as u8,
                velocity: note.velocity,
            });
        }

        let target_start = bars.bar_start(at.max(1));
        let target_end = target_start + (end - start);
        let part = &mut self.tracks[to_track];
        if !merge {
            part.notes.retain(|n| n.start < target_start || n.start >= target_end);
        }
        let count = moved.len();
        part.notes.extend(moved);
        part.notes.sort_by_key(|n| (n.start, n.key));
        self.recount();

        let span = to - from + 1;
        Ok(format!(
            "{} {} from {} of track {from_track} \"{}\" to track {to_track} \"{}\"{}{} ({}){}",
            if merge { "Layered" } else { "Copied" },
            note_count(count),
            bar_span(from, to),
            self.tracks[from_track].display_name(),
            self.tracks[to_track].display_name(),
            if at == from { String::new() } else { format!(", starting at bar {at}") },
            match transpose {
                0 => String::new(),
                n => format!(", transposed {n:+} semitones"),
            },
            if span == 1 { "1 bar".to_string() } else { format!("{span} bars") },
            self.out_of_range(to_track, at.max(1), at.max(1) + (to - from)),
        ))
    }

    /// Write several parts at once, in the form `read_bars` returns: a
    /// `[3] …` line switches to that track, and the bars follow.
    ///
    /// Orchestration writes ten parts over the same bars; doing that one tool
    /// call at a time is what makes it impractical.
    pub fn write_score(&mut self, from: u32, to: u32, text: &str) -> Result<Summary, String> {
        let mut sections: Vec<(usize, String)> = Vec::new();
        for line in text.lines() {
            let trimmed = line.trim();
            if let Some(rest) = trimmed.strip_prefix('[') {
                let (number, _) = rest
                    .split_once(']')
                    .ok_or_else(|| format!("{trimmed}: the track header is missing its closing ]"))?;
                let track: usize = number.trim().parse().map_err(|_| {
                    format!("{trimmed}: a track header looks like `[0] Name`")
                })?;
                self.check_track(track)?;
                sections.push((track, String::new()));
            } else if let Some((_, body)) = sections.last_mut().map(|s| (s.0, &mut s.1)) {
                body.push_str(line);
                body.push('\n');
            } else if !trimmed.is_empty() {
                return Err(
                    "Start with a header such as `[0] Track name` (the same format read_bars prints)"
                        .into(),
                );
            }
        }
        if sections.is_empty() {
            return Err("There are no tracks to write.".into());
        }

        let mut done = Vec::new();
        for (track, body) in sections {
            done.push(self.write_bars(track, from, to, &body)?);
        }
        Ok(done.join("\n"))
    }
}

/// Keep only the top or bottom note of each chord. Notes played together are
/// found the same way they are when a passage is written down, so a rolled
/// chord counts as one chord.
pub(crate) fn pick_voice(notes: &[Note], voice: Voice, ppq: u32) -> Vec<Note> {
    if voice == Voice::All {
        return notes.to_vec();
    }
    let window = crate::text::chord_window(ppq);
    let mut out = Vec::new();
    let mut i = 0;
    while i < notes.len() {
        let head = notes[i];
        let mut j = i + 1;
        while j < notes.len()
            && crate::text::same_strike(
                (head.start, head.len),
                (notes[j].start, notes[j].len),
                window,
            )
        {
            j += 1;
        }
        let group = &notes[i..j];
        let pick = match voice {
            Voice::Top => group.iter().max_by_key(|n| n.key),
            Voice::Bottom => group.iter().min_by_key(|n| n.key),
            Voice::All => unreachable!("handled above"),
        };
        if let Some(note) = pick {
            out.push(*note);
        }
        i = j;
    }
    out
}

/// A dynamic level, as a score marks it.
fn dynamic_level(text: &str) -> Result<u8, String> {
    if let Ok(n) = text.trim().parse::<u8>() {
        return Ok(n.clamp(1, 127));
    }
    Ok(match text.trim().to_lowercase().as_str() {
        "ppp" => 16,
        "pp" => 32,
        "p" => 48,
        "mp" => 64,
        "mf" => 80,
        "f" => 96,
        "ff" => 112,
        "fff" => 127,
        other => {
            return Err(format!(
                "{other}: a dynamic is pp / p / mp / mf / f / ff or a number 1–127"
            ))
        }
    })
}

impl Score {
    /// Shape the loudness of a passage, from `from_level` to `to_level`.
    ///
    /// Note velocities are set, which is what shapes attacks — and, when the
    /// level changes, an expression curve (CC11) is drawn too, because a note
    /// already sounding cannot get louder from its velocity. A crescendo
    /// under held strings needs the curve; a drum pattern needs the
    /// velocities. Writing both is what makes the marking mean what it says.
    pub fn set_dynamics(
        &mut self,
        track: usize,
        from: u32,
        to: u32,
        from_level: &str,
        to_level: Option<&str>,
    ) -> Result<Summary, String> {
        self.check_track(track)?;
        if to < from {
            return Err("to_bar must be at least from_bar.".into());
        }
        let start_level = dynamic_level(from_level)?;
        let end_level = match to_level {
            Some(text) => dynamic_level(text)?,
            None => start_level,
        };

        let bars = self.bars();
        let (start, end) = (bars.bar_start(from), bars.bar_start(to) + bars.bar_ticks(to));
        let span = (end - start).max(1);
        let at = |tick: u32| -> f64 { f64::from(tick.saturating_sub(start)) / f64::from(span) };
        let level = |p: f64| -> u8 {
            (f64::from(start_level) + (f64::from(end_level) - f64::from(start_level)) * p)
                .round()
                .clamp(1.0, 127.0) as u8
        };

        let part = &mut self.tracks[track];
        let mut touched = 0usize;
        for note in part.notes.iter_mut().filter(|n| n.start >= start && n.start < end) {
            note.velocity = level(at(note.start));
            touched += 1;
        }
        if touched == 0 {
            return Err(format!("Track {track} has no notes in {}.", bar_span(from, to)));
        }

        let changing = start_level != end_level;
        // Our own curve replaces whatever expression was written here; two
        // overlapping shapes would fight each other.
        part.passthrough.retain(|(tick, kind)| {
            !(*tick >= start && *tick < end && is_expression(kind))
        });
        if changing {
            let channel = part.channel;
            let steps = ((span / (self.ppq / 4).max(1)) as usize).clamp(8, 64);
            for i in 0..=steps {
                let p = i as f64 / steps as f64;
                let tick = start + (f64::from(span) * p) as u32;
                part.passthrough.push((tick.min(end - 1), expression_event(channel, level(p))));
            }
            part.passthrough.sort_by_key(|(tick, _)| *tick);
        }

        let name = self.tracks[track].display_name();
        Ok(if changing {
            format!(
                "Shaped {} of track {track} \"{name}\" from {} to {} ({}, {})",
                bar_span(from, to),
                from_level.trim(),
                to_level.unwrap_or("").trim(),
                note_count(touched),
                if end_level > start_level { "crescendo" } else { "diminuendo" }
            )
        } else {
            format!(
                "Set {} of track {track} \"{name}\" to {} ({})",
                bar_span(from, to),
                from_level.trim(),
                note_count(touched)
            )
        })
    }

    /// Bend the tempo across a passage — a ritardando, or a gathering of pace.
    pub fn tempo_curve(&mut self, from: u32, to: u32, from_bpm: f64, to_bpm: f64) -> Result<Summary, String> {
        if to < from {
            return Err("to_bar must be at least from_bar.".into());
        }
        for bpm in [from_bpm, to_bpm] {
            if !(10.0..=400.0).contains(&bpm) {
                return Err("The tempo must be between 10 and 400 BPM.".into());
            }
        }
        let bars = self.bars();
        let (start, end) = (bars.bar_start(from), bars.bar_start(to) + bars.bar_ticks(to));
        let span = (end - start).max(1);
        // One change per beat is smooth enough to hear as a curve and coarse
        // enough to stay readable in another program.
        let steps = ((span / self.ppq.max(1)) as usize).clamp(4, 64);

        self.tempos.retain(|t| t.tick < start || t.tick > end);
        for i in 0..=steps {
            let p = i as f64 / steps as f64;
            let bpm = from_bpm + (to_bpm - from_bpm) * p;
            let tick = start + (f64::from(span) * p) as u32;
            self.tempos.push(TempoChange { tick, us_per_beat: bpm_to_us(bpm) });
        }
        self.tempos.sort_by_key(|t| t.tick);
        self.tempos.dedup_by_key(|t| t.tick);
        Ok(format!(
            "{} the tempo over {} from {from_bpm:.0} to {to_bpm:.0} BPM ({})",
            if to_bpm < from_bpm { "Slowed" } else { "Sped up" },
            bar_span(from, to),
            if to_bpm < from_bpm { "rit." } else { "accel." }
        ))
    }
}

fn is_expression(kind: &midly::TrackEventKind<'_>) -> bool {
    matches!(
        kind,
        midly::TrackEventKind::Midi {
            message: midly::MidiMessage::Controller { controller, .. },
            ..
        } if controller.as_int() == 11
    )
}

fn expression_event(channel: u8, value: u8) -> midly::TrackEventKind<'static> {
    midly::TrackEventKind::Midi {
        channel: midly::num::u4::new(channel & 0x0f),
        message: midly::MidiMessage::Controller {
            controller: midly::num::u7::new(11),
            value: midly::num::u7::new(value.min(127)),
        },
    }
}

impl Score {
    /// Declare that the piece begins with a pickup of `beats`.
    ///
    /// MIDI has no field for an upbeat, so it is written the way notation
    /// programs write it: a short first bar, then the real signature. That
    /// keeps the bar numbers everyone else will read — bar 1 is the pickup,
    /// bar 2 is the first full bar — and survives a save, because it is just
    /// a time signature.
    pub fn set_pickup(&mut self, beats: f64) -> Result<Summary, String> {
        let (numerator, denominator) = {
            let bars = self.bars();
            // The signature the piece is really in is whatever follows the
            // pickup, or what bar 1 says if there is no pickup yet.
            let span = bars.spans.iter().find(|s| s.first_bar > 1).unwrap_or(&bars.spans[0]);
            (span.numerator, span.denominator)
        };
        let beat_ticks = (self.ppq * 4 / u32::from(denominator)).max(1);

        if beats <= 0.0 {
            self.time_signatures.retain(|t| t.tick != 0);
            self.time_signatures.insert(0, TimeSigChange { tick: 0, numerator, denominator });
            self.time_signatures.retain(|t| t.tick == 0 || t.numerator != numerator || t.denominator != denominator);
            return Ok("Removed the pickup; bar 1 is a full bar again".into());
        }
        if beats >= f64::from(numerator) {
            return Err(format!("A pickup must be shorter than one bar ({numerator} beats)."));
        }
        let pickup = (beats * f64::from(beat_ticks)).round() as u32;
        if pickup == 0 {
            return Err("The pickup is too short.".into());
        }

        self.time_signatures.retain(|t| t.tick != 0 && t.tick != pickup);
        // A first bar of exactly the pickup's length, then the real signature.
        self.time_signatures.push(TimeSigChange {
            tick: 0,
            numerator: (beats.round() as u8).max(1),
            denominator,
        });
        self.time_signatures.push(TimeSigChange { tick: pickup, numerator, denominator });
        self.time_signatures.sort_by_key(|t| t.tick);
        Ok(format!(
            "Made bar 1 a pickup of {beats} {} ({numerator}/{denominator} from bar 2)",
            if beats == 1.0 { "beat" } else { "beats" }
        ))
    }

    /// Whether the music starts partway through bar 1, which usually means
    /// the file was written without declaring its upbeat.
    pub fn looks_like_a_pickup(&self) -> Option<f64> {
        let bars = self.bars();
        let first = self.tracks.iter().filter_map(|t| t.notes.first().map(|n| n.start)).min()?;
        if first == 0 || first >= bars.bar_ticks(1) {
            return None;
        }
        let beats = u32::from(bars.time_signature(1).0.max(1));
        let beat_ticks = (bars.bar_ticks(1) / beats).max(1);
        // Only worth mentioning when it lands cleanly on a beat.
        let on_beat = first % beat_ticks == 0;
        on_beat.then(|| f64::from(bars.bar_ticks(1) - first) / f64::from(beat_ticks))
    }
}
