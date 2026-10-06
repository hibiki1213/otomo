//! Lifting a passage out of one song and setting it into another.
//!
//! Arranging usually starts from something that exists: the melody of one
//! song, the groove of another. A [`Clip`] is one track's notes over a span,
//! with the pedal and expression that shape them, measured in the source's
//! ticks. Pasting rescales those to the destination, so the music lands on
//! the same beats whatever the two files' resolutions or tempos — a passage
//! is positioned by beats, not by seconds.

use midly::num::{u4, u7};
use midly::{MidiMessage, TrackEventKind};

use super::edit::{pick_voice, Summary, Voice};
use super::{Note, Score};

/// Controllers that belong to the passage rather than to the mix: sustain
/// pedal, expression, modulation. Volume and pan stay with the destination.
const SHAPING: [u8; 3] = [64, 11, 1];

const SUSTAIN: u8 = 64;

/// One track's notes over a span, ready to paste.
#[derive(Debug, Clone)]
pub struct Clip {
    ppq: u32,
    /// Ticks the clip covers; pasting replaces this much of the destination.
    len: u32,
    /// Starts relative to the clip.
    notes: Vec<Note>,
    /// (tick, controller, value), relative to the clip.
    controls: Vec<(u32, u8, u8)>,
    /// The source track's name and instrument, for a new track made from it.
    pub name: String,
    pub program: u8,
    pub percussion: bool,
    /// The key the source is in where the clip starts.
    pub key: Option<(i8, bool)>,
}

impl Clip {
    pub fn note_count(&self) -> usize {
        self.notes.len()
    }
}

fn controller(kind: &TrackEventKind<'_>) -> Option<(u8, u8)> {
    match kind {
        TrackEventKind::Midi { message: MidiMessage::Controller { controller, value }, .. } => {
            Some((controller.as_int(), value.as_int()))
        }
        _ => None,
    }
}

impl Score {
    /// The notes of `track` that start in `start..end` (ticks) and lie
    /// within `keys`, as a clip. `voice` can take just the melody or the bass.
    /// The pedal and expression come too when no note of the span was left out.
    pub fn clip(&self, track: usize, start: u32, end: u32, keys: (u8, u8), voice: Voice) -> Result<Clip, String> {
        let part = self.tracks.get(track).ok_or_else(|| format!("There is no track {track}."))?;
        if end <= start {
            return Err("The range to copy is empty.".into());
        }
        let in_span: Vec<Note> = part.notes.iter().filter(|n| n.start >= start && n.start < end).copied().collect();
        let taken: Vec<Note> = in_span.iter().filter(|n| (keys.0..=keys.1).contains(&n.key)).copied().collect();
        let notes = pick_voice(&taken, voice, self.ppq)
            .into_iter()
            .map(|n| Note { start: n.start - start, ..n })
            .collect();

        // The pedal and expression as they stand when the clip begins, what
        // happens inside it, and the pedal let up at its end: a passage
        // pasted elsewhere must not leave the sustain down.
        let mut controls: Vec<(u32, u8, u8)> = Vec::new();
        // Only when the whole part comes along: the pedal of a piano part
        // would smear a melody lifted out of it.
        let whole = taken.len() == in_span.len();
        if whole && !part.is_percussion() {
            for c in SHAPING {
                let before = part
                    .passthrough
                    .iter()
                    .filter(|(t, _)| *t < start)
                    .filter_map(|(_, k)| controller(k))
                    .filter(|(n, _)| *n == c)
                    .last();
                if let Some((_, value)) = before {
                    if c != SUSTAIN || value >= 64 {
                        controls.push((0, c, value));
                    }
                }
            }
            for (tick, kind) in &part.passthrough {
                if let Some((c, value)) = controller(kind) {
                    if *tick >= start && *tick < end && SHAPING.contains(&c) {
                        controls.push((tick - start, c, value));
                    }
                }
            }
            let held = controls.iter().filter(|(_, c, _)| *c == SUSTAIN).last().is_some_and(|(_, _, v)| *v >= 64);
            if held {
                controls.push((end - start, SUSTAIN, 0));
            }
        }

        let key = self.key_signatures.iter().rev().find(|k| k.tick <= start).map(|k| (k.sharps, k.minor));
        Ok(Clip {
            ppq: self.ppq,
            len: end - start,
            notes,
            controls,
            name: part.display_name(),
            program: part.program,
            percussion: part.is_percussion(),
            key,
        })
    }

    /// Set `clip` into track `to` at tick `at` — or, with no track given,
    /// into a new one playing the clip's instrument. Unless `merge`, what the
    /// destination track had over the clip's length is replaced. Returns the
    /// track the clip went into.
    pub fn paste(&mut self, clip: &Clip, to: Option<usize>, at: u32, transpose: i32, merge: bool) -> Result<usize, String> {
        for n in &clip.notes {
            let key = i32::from(n.key) + transpose;
            if !(0..=127).contains(&key) {
                return Err(format!(
                    "Transposing by {transpose} semitones goes out of range ({} would fall outside MIDI's range).",
                    crate::note_name(n.key)
                ));
            }
        }
        let (ppq, from_ppq) = (u64::from(self.ppq), u64::from(clip.ppq.max(1)));
        let scale = move |t: u32| -> u32 { ((u64::from(t) * ppq + from_ppq / 2) / from_ppq) as u32 };
        let len = scale(clip.len);
        let (track, fresh) = match to {
            Some(t) if t < self.tracks.len() => (t, false),
            Some(t) => return Err(format!("There is no track {t}.")),
            None => {
                self.add_track(clip.program, &clip.name, clip.percussion);
                (self.tracks.len() - 1, true)
            }
        };

        let moved: Vec<Note> = clip
            .notes
            .iter()
            .map(|n| Note {
                start: at + scale(n.start),
                len: scale(n.len).max(1),
                key: (i32::from(n.key) + transpose) as u8,
                velocity: n.velocity,
            })
            .collect();
        let part = &mut self.tracks[track];
        if !merge {
            part.notes.retain(|n| n.start < at || n.start >= at + len);
        }
        part.notes.extend(moved);
        part.notes.sort_by_key(|n| (n.start, n.key));

        // The pedal only means the same thing on the same instrument; on a
        // layered part it would fight the one already there.
        if !clip.controls.is_empty() && !merge && (fresh || part.program == clip.program) {
            let channel = part.channel;
            part.passthrough.retain(|(tick, kind)| {
                !(*tick >= at && *tick <= at + len && controller(kind).is_some_and(|(c, _)| SHAPING.contains(&c)))
            });
            for &(tick, c, value) in &clip.controls {
                let message = MidiMessage::Controller { controller: u7::new(c), value: u7::new(value.min(127)) };
                part.passthrough.push((at + scale(tick), TrackEventKind::Midi { channel: u4::new(channel & 0x0f), message }));
            }
            part.passthrough.sort_by_key(|(tick, _)| *tick);
        }
        self.recount();
        Ok(track)
    }

    /// Copy bars `from..=to` of `track` in `source` to bar `at` of this
    /// score, into `to_track` or a new track. The MCP tool's copy between songs.
    #[allow(clippy::too_many_arguments)]
    pub fn copy_from(
        &mut self,
        source: &Score,
        source_title: &str,
        track: usize,
        from: u32,
        to: u32,
        to_track: Option<usize>,
        at: u32,
        transpose: i32,
        voice: Voice,
        merge: bool,
    ) -> Result<Summary, String> {
        if to < from {
            return Err("to_bar must be at least from_bar.".into());
        }
        let src = source.bars();
        if from > src.count {
            return Err(format!("\"{source_title}\" ends at bar {}.", src.count));
        }
        let (start, end) = (src.bar_start(from), src.bar_start(to) + src.bar_ticks(to));
        let clip = source.clip(track, start, end, (0, 127), voice)?;
        if clip.note_count() == 0 {
            return Err(format!("Track {track} of \"{source_title}\" has no notes in {}.", super::edit::bar_span(from, to)));
        }
        let at_tick = self.bars().bar_start(at.max(1));
        let into = self.paste(&clip, to_track, at_tick, transpose, merge)?;

        let mut notes = Vec::new();
        let here = self.key_signatures.iter().rev().find(|k| k.tick <= at_tick).map(|k| (k.sharps, k.minor));
        if transpose == 0 && clip.key.unwrap_or((0, false)) != here.unwrap_or((0, false)) && !clip.percussion {
            let name = |k: Option<(i8, bool)>| {
                k.map_or_else(|| "C major".to_string(), |(s, m)| super::KeyChange { tick: 0, sharps: s, minor: m }.name())
            };
            notes.push(format!(
                "the source is in {}, this song in {} — copy again with transpose to change key",
                name(clip.key),
                name(here)
            ));
        }
        let (src_meter, dst_meter) = (src.time_signature(from), self.bars().time_signature(at.max(1)));
        if src_meter != dst_meter {
            notes.push(format!(
                "the source is in {}/{}, this song in {}/{}: placed beat for beat",
                src_meter.0, src_meter.1, dst_meter.0, dst_meter.1
            ));
        }
        let end_bar = at.max(1) + (to - from);
        Ok(format!(
            "{} {} from {} of track {track} \"{}\" in \"{source_title}\" to {} \"{}\", starting at bar {at}{}{}{}",
            if merge { "Layered" } else { "Copied" },
            super::edit::note_count(clip.note_count()),
            super::edit::bar_span(from, to),
            clip.name,
            if to_track.is_some() { format!("track {into}") } else { format!("new track {into}") },
            self.tracks[into].display_name(),
            match transpose {
                0 => String::new(),
                n => format!(", transposed {n:+} semitones"),
            },
            if notes.is_empty() { String::new() } else { format!(" ({})", notes.join("; ")) },
            self.out_of_range(into, at.max(1), end_bar),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song(ppq: u32) -> Score {
        let mut s = Score::blank(100.0, 4, 4);
        s.ppq = ppq;
        s
    }

    fn sustain(channel: u8, value: u8) -> TrackEventKind<'static> {
        TrackEventKind::Midi {
            channel: u4::new(channel),
            message: MidiMessage::Controller { controller: u7::new(SUSTAIN), value: u7::new(value) },
        }
    }

    #[test]
    fn a_passage_lands_on_the_same_beats_in_another_song() {
        // The source counts 96 ticks a beat, the destination 480.
        let mut source = song(96);
        source.add_track(0, "Piano", false);
        source.write_bars(0, 1, 3, "b2  C4@1:1 E4@2.5:0.5 G4@3:2\nb3  C5@1:4").unwrap();
        let mut dest = Score::blank(140.0, 4, 4);
        dest.add_track(48, "Strings", false);
        dest.write_bars(0, 1, 1, "C3@1:4").unwrap();

        let done = dest.copy_from(&source, "Sketch", 0, 2, 2, None, 5, 0, Voice::All, false).unwrap();
        assert!(done.contains("new track 1 \"Piano\""), "{done}");
        let piano = &dest.tracks[1];
        assert_eq!(piano.program, 0);
        let at = dest.bars().bar_start(5);
        let got: Vec<(u32, u32, u8)> = piano.notes.iter().map(|n| (n.start - at, n.len, n.key)).collect();
        assert_eq!(got, [(0, 480, 60), (720, 240, 64), (960, 960, 67)]);
        assert_eq!(dest.tracks[0].notes.len(), 1, "the strings are untouched");
    }

    #[test]
    fn replacing_clears_the_span_and_merging_keeps_it() {
        let mut source = song(480);
        source.add_track(73, "Flute", false);
        source.write_bars(0, 1, 1, "C5@1:1 D5@2:1").unwrap();
        let mut dest = song(480);
        dest.add_track(73, "Flute", false);
        dest.write_bars(0, 1, 2, "b1  G4@1:4\nb2  A4@1:4").unwrap();

        let mut replaced = dest.clone();
        replaced.copy_from(&source, "S", 0, 1, 1, Some(0), 1, 0, Voice::All, false).unwrap();
        let keys: Vec<u8> = replaced.tracks[0].notes.iter().map(|n| n.key).collect();
        assert_eq!(keys, [72, 74, 69], "bar 1 replaced, bar 2 kept");

        dest.copy_from(&source, "S", 0, 1, 1, Some(0), 1, 12, Voice::All, true).unwrap();
        let keys: Vec<u8> = dest.tracks[0].notes.iter().map(|n| n.key).collect();
        assert_eq!(keys, [67, 84, 86, 69], "layered an octave up");
    }

    #[test]
    fn the_pedal_comes_along_and_is_let_up() {
        let mut source = song(480);
        source.add_track(0, "Piano", false);
        source.write_bars(0, 1, 3, "b1  C4@1:4\nb2  E4@1:4\nb3  G4@1:4").unwrap();
        // Pedal down in bar 1, held through bar 2, up in bar 3.
        let ch = source.tracks[0].channel;
        source.tracks[0].passthrough = vec![(0, sustain(ch, 127)), (1920 * 2 + 100, sustain(ch, 0))];

        let mut dest = song(480);
        dest.copy_from(&source, "S", 0, 2, 2, None, 1, 0, Voice::All, false).unwrap();
        let ch = dest.tracks[0].channel;
        let pedal: Vec<(u32, u8)> = dest.tracks[0]
            .passthrough
            .iter()
            .filter_map(|(t, k)| controller(k).filter(|(c, _)| *c == SUSTAIN).map(|(_, v)| (*t, v)))
            .collect();
        assert_eq!(pedal, [(0, 127), (1920, 0)], "down from the start, up at the end");
        assert!(dest.tracks[0].passthrough.iter().all(|(_, k)| matches!(k, TrackEventKind::Midi { channel, .. } if channel.as_int() == ch)));
    }

    #[test]
    fn a_key_mismatch_is_mentioned() {
        let mut source = song(480);
        source.set_key(2, false).unwrap();
        source.add_track(73, "Flute", false);
        source.write_bars(0, 1, 1, "D5@1:4").unwrap();
        let mut dest = song(480);
        dest.set_key(-1, false).unwrap();
        let done = dest.copy_from(&source, "S", 0, 1, 1, None, 1, 0, Voice::All, false).unwrap();
        assert!(done.contains("D major") && done.contains("F major"), "{done}");
        let quiet = dest.copy_from(&source, "S", 0, 1, 1, None, 1, 3, Voice::All, false).unwrap();
        assert!(!quiet.contains("major"), "{quiet}");
    }

    #[test]
    fn a_selection_takes_only_its_keys_and_starts_mid_bar() {
        let mut source = song(480);
        source.add_track(0, "Piano", false);
        source.write_bars(0, 1, 1, "[C3 C5]@1:1 [D3 D5]@3:1").unwrap();
        // From beat 3, upper notes only.
        let clip = source.clip(0, 960, 1920, (60, 127), Voice::All).unwrap();
        assert_eq!(clip.note_count(), 1);
        let mut dest = song(480);
        dest.add_track(0, "Piano", false);
        let track = dest.paste(&clip, Some(0), 480, 0, false).unwrap();
        let got: Vec<(u32, u8)> = dest.tracks[track].notes.iter().map(|n| (n.start, n.key)).collect();
        assert_eq!(got, [(480, 74)]);
    }

    #[test]
    fn an_excerpt_starts_with_what_was_in_effect() {
        let mut s = song(480);
        s.add_track(0, "Piano", false);
        s.add_track(32, "Bass", false);
        s.set_tempo(80.0, 3).unwrap();
        s.set_time_signature(3, 4, 3).unwrap();
        s.write_bars(0, 1, 6, "b1  C4@1:4\nb4  E4@1:3\nb6  G4@1:3").unwrap();
        s.write_bars(1, 1, 6, "b1  C2@1:4\nb6  G2@1:3").unwrap();
        let part = s.excerpt(Some(&[0]), 4, 5).unwrap();
        assert_eq!(part.tracks.len(), 1);
        assert_eq!(part.tracks[0].notes.iter().map(|n| (n.start, n.key)).collect::<Vec<_>>(), [(0, 64)]);
        assert_eq!(part.tempos.len(), 1, "{:?}", part.tempos);
        assert_eq!(part.tempos[0].bpm().round(), 80.0);
        assert_eq!(part.bars().time_signature(1), (3, 4));
    }
}
