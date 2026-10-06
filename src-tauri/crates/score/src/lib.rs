//! The song as music rather than as MIDI: bars, beats, note names.
//!
//! The playback engine works in samples and the file works in ticks, but
//! neither is a good way to talk about a piece — "bar 17, beat 3" is. This
//! module is the shared vocabulary for the MCP server (and later, editing):
//! it parses a Standard MIDI File into a [`Score`] and renders passages as
//! compact text that reads back in the same form it is written.
//!
//! Track numbering deliberately matches what the app's UI shows: one track
//! per SMF track that contains notes, in file order.

mod edit;
mod gm;
mod harmony;
mod musicxml;
mod parse_text;
mod text;
mod write;

pub use edit::Voice;
pub use harmony::{analyse as analyse_harmony, Chord};
pub use gm::{drum_key, drum_name, program_name, program_number, range as program_range};
pub use musicxml::{is_musicxml, read_musicxml, MusicXml};
pub use parse_text::parse_note_name;
pub use text::{note_name, render_bars, suggest_grid, BarLine, Grid};

use midly::{MetaMessage, MidiMessage, Smf, Timing, TrackEventKind};

/// An event the score does not model, kept exactly as the file had it.
pub(crate) type Passthrough = Vec<(u32, TrackEventKind<'static>)>;

/// Assumed when a file states no tempo (the MIDI default).
const DEFAULT_BPM: f64 = 120.0;
const DEFAULT_US_PER_BEAT: u32 = 500_000;

#[derive(Debug, Clone, Copy)]
pub struct TempoChange {
    pub tick: u32,
    pub us_per_beat: u32,
}

impl TempoChange {
    pub fn bpm(&self) -> f64 {
        60_000_000.0 / f64::from(self.us_per_beat)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct TimeSigChange {
    pub tick: u32,
    pub numerator: u8,
    /// 4 = quarter, 8 = eighth, …
    pub denominator: u16,
}

#[derive(Debug, Clone, Copy)]
pub struct KeyChange {
    pub tick: u32,
    /// Sharps (positive) or flats (negative), as written in the file.
    pub sharps: i8,
    pub minor: bool,
}

const MAJOR_KEYS: [&str; 15] =
    ["Cb", "Gb", "Db", "Ab", "Eb", "Bb", "F", "C", "G", "D", "A", "E", "B", "F#", "C#"];
const MINOR_KEYS: [&str; 15] =
    ["Ab", "Eb", "Bb", "F", "C", "G", "D", "A", "E", "B", "F#", "C#", "G#", "D#", "A#"];

impl KeyChange {
    /// "D major", "B minor".
    pub fn name(&self) -> String {
        let i = (self.sharps.clamp(-7, 7) + 7) as usize;
        let tonic = if self.minor { MINOR_KEYS[i] } else { MAJOR_KEYS[i] };
        format!("{tonic} {}", if self.minor { "minor" } else { "major" })
    }

    /// The other direction: "Bb minor", "D", "f# minor", "B-flat major" →
    /// sharps and mode.
    pub fn parse(text: &str) -> Result<(i8, bool), String> {
        let text = text.trim();
        let spelled = spelled_accidentals(text);
        let (tonic, minor) = match spelled.split_once(char::is_whitespace) {
            Some((tonic, mode)) => {
                let mode = mode.trim().to_lowercase();
                let minor = match mode.as_str() {
                    "minor" | "min" | "m" | "短調" => true,
                    "major" | "maj" | "長調" => false,
                    _ => return Err(format!("{text}: write keys like \"D major\" or \"B minor\"")),
                };
                (tonic, minor)
            }
            // A bare tonic means major, as on a lead sheet.
            None => (spelled.as_str(), false),
        };
        let tonic = normalise_tonic(tonic);
        let table = if minor { MINOR_KEYS } else { MAJOR_KEYS };
        table
            .iter()
            .position(|k| *k == tonic)
            .map(|i| (i as i8 - 7, minor))
            .ok_or_else(|| format!("{text}: there is no key signature for that tonic"))
    }
}

/// "B-flat major", "F sharp minor" → "Bb major", "F# minor": the way sources
/// on the web spell keys out.
fn spelled_accidentals(text: &str) -> String {
    let mut out = text.to_string();
    for (word, sign) in [("-flat", "b"), (" flat", "b"), ("-sharp", "#"), (" sharp", "#")] {
        // ASCII lowering keeps byte offsets, so the match can be cut from `out`.
        if let Some(i) = out.to_ascii_lowercase().find(word) {
            out.replace_range(i..i + word.len(), sign);
        }
    }
    out
}

/// "bb" → "Bb", "f#" → "F#", accepting the ♯♭ signs too.
fn normalise_tonic(tonic: &str) -> String {
    let mut out = String::new();
    for (i, c) in tonic.chars().enumerate() {
        match (i, c) {
            (0, c) => out.extend(c.to_uppercase()),
            (_, '♯') => out.push('#'),
            (_, '♭') => out.push('b'),
            (_, 'B') => out.push('b'),
            (_, c) => out.push(c),
        }
    }
    out
}

#[derive(Debug, Clone, Copy)]
pub struct Note {
    pub start: u32,
    /// Ticks held. Never zero: a zero-length note would be inaudible.
    pub len: u32,
    pub key: u8,
    pub velocity: u8,
}

impl Note {
    pub fn end(&self) -> u32 {
        self.start + self.len
    }
}

#[derive(Debug, Clone)]
pub struct Track {
    /// From the file's track-name meta event; empty if it has none.
    pub name: String,
    pub channel: u8,
    /// GM program in effect at the start of the track.
    pub program: u8,
    /// Sorted by start tick, then by key.
    pub notes: Vec<Note>,
    /// Everything this track does that notes cannot say: the sustain pedal,
    /// expression, volume, pitch bend, later program changes. The score has
    /// no opinion about these, and carries them through untouched so that
    /// editing a few bars never flattens a performance.
    pub(crate) passthrough: Passthrough,
}

impl Track {
    pub fn is_percussion(&self) -> bool {
        self.channel == 9
    }
}

#[derive(Debug, Clone)]
pub struct Score {
    /// Ticks per quarter note.
    pub ppq: u32,
    /// Always starts with a change at tick 0.
    pub tempos: Vec<TempoChange>,
    /// Always starts with a change at tick 0.
    pub time_signatures: Vec<TimeSigChange>,
    pub key_signatures: Vec<KeyChange>,
    pub tracks: Vec<Track>,
    /// Tick the last note ends on.
    pub end_tick: u32,
    /// Title / copyright / other text the file carries, in file order.
    pub text: Vec<String>,
    /// Unmodelled events from the file's conductor track, or from any track
    /// with nothing to play. Written back with the tempo map.
    pub(crate) conductor_extra: Passthrough,
    /// Section names the file carries, as `(tick, text)`. Many files label
    /// their own structure, which is the cheapest map of a piece there is.
    pub markers: Vec<(u32, String)>,
}

impl Score {
    /// Parse a Standard MIDI File.
    pub fn parse(bytes: &[u8]) -> Result<Self, String> {
        let smf = Smf::parse(bytes).map_err(|e| format!("Could not read the MIDI file: {e}"))?;
        let ppq = match smf.header.timing {
            Timing::Metrical(t) => u32::from(t.as_int()),
            // SMPTE-timed files are vanishingly rare and would need a
            // different clock throughout; say so instead of guessing.
            Timing::Timecode(..) => {
                return Err("MIDI files timed in SMPTE timecode are not supported".into())
            }
        };
        if ppq == 0 {
            return Err("The MIDI file's resolution (ticks per beat) is 0".into());
        }

        let mut score = Score {
            ppq,
            tempos: Vec::new(),
            time_signatures: Vec::new(),
            key_signatures: Vec::new(),
            tracks: Vec::new(),
            end_tick: 0,
            text: Vec::new(),
            conductor_extra: Vec::new(),
            markers: Vec::new(),
        };

        for events in &smf.tracks {
            let mut tick: u32 = 0;
            let mut track = Track {
                name: String::new(),
                channel: 0,
                program: 0,
                notes: Vec::new(),
                passthrough: Vec::new(),
            };
            let mut channel_seen = false;
            let mut program_seen = false;
            // A part with no notes yet is still a part, as long as the file
            // says so: a name plus a channel message. Without this, a track
            // added but not yet written to would vanish on the next save.
            let mut addressed = false;
            // Sounding notes, by (channel, key), holding the start tick.
            let mut open: Vec<(u8, u8, u32, u8)> = Vec::new();

            for event in events {
                tick = tick.saturating_add(event.delta.as_int());
                match event.kind {
                    TrackEventKind::Meta(meta) => match meta {
                        MetaMessage::Tempo(us) => score.tempos.push(TempoChange {
                            tick,
                            us_per_beat: us.as_int().max(1),
                        }),
                        MetaMessage::TimeSignature(num, den_pow, ..) => {
                            score.time_signatures.push(TimeSigChange {
                                tick,
                                numerator: num.max(1),
                                denominator: 1u16 << den_pow.min(6),
                            })
                        }
                        MetaMessage::KeySignature(sharps, minor) => {
                            score.key_signatures.push(KeyChange { tick, sharps, minor })
                        }
                        MetaMessage::TrackName(name) => {
                            if track.name.is_empty() {
                                track.name = clean_text(name);
                            }
                        }
                        MetaMessage::Text(t) | MetaMessage::Copyright(t) => {
                            let cleaned = clean_text(t);
                            if !cleaned.is_empty() {
                                score.text.push(cleaned);
                            }
                            track.passthrough.push((tick, event.kind.to_static()));
                        }
                        MetaMessage::Marker(name) | MetaMessage::CuePoint(name) => {
                            let name = clean_text(name);
                            if !name.is_empty() {
                                score.markers.push((tick, name));
                            }
                            track.passthrough.push((tick, event.kind.to_static()));
                        }
                        // Our own writer closes each track; everything else
                        // the file says is kept as it stands.
                        MetaMessage::EndOfTrack => {}
                        _ => track.passthrough.push((tick, event.kind.to_static())),
                    },
                    TrackEventKind::Midi { channel, message } => {
                        let ch = channel.as_int();
                        addressed = true;
                        // A part with no notes yet still belongs to a channel,
                        // and for a drum kit that channel is the whole point.
                        if !channel_seen {
                            track.channel = ch;
                            channel_seen = true;
                        }
                        match message {
                            MidiMessage::NoteOn { key, vel } if vel.as_int() > 0 => {
                                open.push((ch, key.as_int(), tick, vel.as_int()));
                            }
                            // A note-on with velocity 0 is a note-off.
                            MidiMessage::NoteOn { key, .. } | MidiMessage::NoteOff { key, .. } => {
                                let key = key.as_int();
                                if let Some(i) =
                                    open.iter().rposition(|(c, k, ..)| *c == ch && *k == key)
                                {
                                    let (_, key, start, velocity) = open.remove(i);
                                    track.notes.push(Note {
                                        start,
                                        len: tick.saturating_sub(start).max(1),
                                        key,
                                        velocity,
                                    });
                                }
                            }
                            MidiMessage::ProgramChange { program } => {
                                // The program a track *starts* on is the one
                                // the app shows. One that arrives after the
                                // music has begun is a switch, and plays where
                                // the file put it.
                                let started = !track.notes.is_empty() || !open.is_empty();
                                if program_seen || started {
                                    track.passthrough.push((tick, event.kind.to_static()));
                                } else {
                                    track.program = program.as_int();
                                    program_seen = true;
                                }
                            }
                            _ => track.passthrough.push((tick, event.kind.to_static())),
                        }
                    }
                    _ => track.passthrough.push((tick, event.kind.to_static())),
                }
            }
            // Notes still held at the end run to the end of the track.
            for (_, key, start, velocity) in open {
                track.notes.push(Note {
                    start,
                    len: tick.saturating_sub(start).max(1),
                    key,
                    velocity,
                });
            }

            if track.notes.is_empty() && !(addressed && !track.name.is_empty()) {
                // Nothing to play, but it may still carry the file's markers
                // or a GM reset; those belong with the tempo map.
                score.conductor_extra.append(&mut track.passthrough);
                continue;
            }
            track.notes.sort_by_key(|n| (n.start, n.key));
            score.end_tick =
                score.end_tick.max(track.notes.iter().map(Note::end).max().unwrap_or(0));
            score.tracks.push(track);
        }

        score.markers.sort_by_key(|(tick, _)| *tick);
        score.markers.dedup();
        score.tempos.sort_by_key(|t| t.tick);
        score.time_signatures.sort_by_key(|t| t.tick);
        score.key_signatures.sort_by_key(|k| k.tick);
        if score.tempos.first().is_none_or(|t| t.tick > 0) {
            score.tempos.insert(0, TempoChange { tick: 0, us_per_beat: DEFAULT_US_PER_BEAT });
        }
        if score.time_signatures.first().is_none_or(|t| t.tick > 0) {
            score
                .time_signatures
                .insert(0, TimeSigChange { tick: 0, numerator: 4, denominator: 4 });
        }
        Ok(score)
    }

    /// Seconds from the start of the file to `tick`, following the tempo map.
    pub fn tick_seconds(&self, tick: u32) -> f64 {
        let mut seconds = 0.0;
        let mut at = 0u32;
        let mut us = DEFAULT_US_PER_BEAT;
        for change in &self.tempos {
            if change.tick >= tick {
                break;
            }
            seconds += ticks_to_seconds(change.tick - at, self.ppq, us);
            at = change.tick;
            us = change.us_per_beat;
        }
        seconds + ticks_to_seconds(tick.saturating_sub(at), self.ppq, us)
    }

    pub fn duration(&self) -> f64 {
        self.tick_seconds(self.end_tick)
    }

    /// The slowest and fastest tempo in the file, rounded.
    pub fn bpm_range(&self) -> (u32, u32) {
        let mut lo = f64::INFINITY;
        let mut hi = 0.0f64;
        for t in &self.tempos {
            lo = lo.min(t.bpm());
            hi = hi.max(t.bpm());
        }
        if hi == 0.0 {
            return (DEFAULT_BPM as u32, DEFAULT_BPM as u32);
        }
        (lo.round() as u32, hi.round() as u32)
    }

    pub fn bars(&self) -> BarMap {
        BarMap::new(self)
    }
}

fn ticks_to_seconds(ticks: u32, ppq: u32, us_per_beat: u32) -> f64 {
    f64::from(ticks) / f64::from(ppq) * f64::from(us_per_beat) / 1e6
}

/// Track names and text meta are bytes of unknown encoding; keep it printable.
fn clean_text(raw: &[u8]) -> String {
    String::from_utf8_lossy(raw)
        .chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .trim()
        .to_string()
}

/* ---------- bars ---------- */

/// A stretch of bars under one time signature.
#[derive(Debug, Clone, Copy)]
pub struct BarSpan {
    /// 1-based number of this span's first bar.
    pub first_bar: u32,
    pub start_tick: u32,
    pub numerator: u8,
    pub denominator: u16,
    /// Ticks in one beat, where the beat is the signature's denominator.
    pub beat_ticks: u32,
}

impl BarSpan {
    pub fn bar_ticks(&self) -> u32 {
        self.beat_ticks * u32::from(self.numerator)
    }
}

/// Bar numbering for a score. Bars are 1-based, beats within a bar are 1-based
/// too, so "bar 5, beat 1" is the downbeat of the fifth bar.
#[derive(Debug, Clone)]
pub struct BarMap {
    pub spans: Vec<BarSpan>,
    pub count: u32,
}

impl BarMap {
    fn new(score: &Score) -> Self {
        let mut spans: Vec<BarSpan> = Vec::new();
        for ts in &score.time_signatures {
            let beat_ticks = (score.ppq * 4 / u32::from(ts.denominator)).max(1);
            let (first_bar, start_tick) = match spans.last() {
                None => (1, 0),
                Some(prev) => {
                    // A signature change takes effect at the next bar line.
                    let elapsed = ts.tick.saturating_sub(prev.start_tick);
                    let bars = elapsed.div_ceil(prev.bar_ticks());
                    (prev.first_bar + bars, prev.start_tick + bars * prev.bar_ticks())
                }
            };
            let span =
                BarSpan { first_bar, start_tick, numerator: ts.numerator, denominator: ts.denominator, beat_ticks };
            // Two signatures inside one bar: the later one wins.
            if spans.last().is_some_and(|p| p.first_bar == first_bar) {
                spans.pop();
            }
            spans.push(span);
        }
        let last = *spans.last().expect("a score always has one time signature");
        let remaining = score.end_tick.saturating_sub(last.start_tick);
        let count = (last.first_bar + remaining.div_ceil(last.bar_ticks())).saturating_sub(1).max(1);
        Self { spans, count }
    }

    fn span_of_bar(&self, bar: u32) -> BarSpan {
        *self
            .spans
            .iter()
            .rev()
            .find(|s| s.first_bar <= bar)
            .unwrap_or(&self.spans[0])
    }

    fn span_of_tick(&self, tick: u32) -> BarSpan {
        *self
            .spans
            .iter()
            .rev()
            .find(|s| s.start_tick <= tick)
            .unwrap_or(&self.spans[0])
    }

    /// First tick of `bar` (1-based).
    pub fn bar_start(&self, bar: u32) -> u32 {
        let bar = bar.max(1);
        let span = self.span_of_bar(bar);
        span.start_tick + (bar - span.first_bar) * span.bar_ticks()
    }

    pub fn bar_ticks(&self, bar: u32) -> u32 {
        self.span_of_bar(bar.max(1)).bar_ticks()
    }

    pub fn time_signature(&self, bar: u32) -> (u8, u16) {
        let s = self.span_of_bar(bar.max(1));
        (s.numerator, s.denominator)
    }

    /// Bar number and 1-based beat (fractional) for a tick.
    pub fn position(&self, tick: u32) -> (u32, f64) {
        let span = self.span_of_tick(tick);
        let into = tick - span.start_tick;
        let bar = span.first_bar + into / span.bar_ticks();
        let beat = f64::from(into % span.bar_ticks()) / f64::from(span.beat_ticks) + 1.0;
        (bar, beat)
    }
}

#[cfg(test)]
mod tests;
