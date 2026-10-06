//! Turning a [`Score`] back into a Standard MIDI File.
//!
//! Written as format 1: a conductor track carrying tempo, time and key
//! signatures, then one track per part. That is the shape the app's parser
//! (and every other reader) expects, and it keeps track numbering stable
//! across a read-edit-write round trip.

use midly::num::{u15, u24, u28, u4, u7};
use midly::{Format, Header, MetaMessage, MidiMessage, Smf, Timing, TrackEvent, TrackEventKind};

use super::{Note, Score};

impl Score {
    /// Serialise to a Standard MIDI File.
    pub fn to_smf(&self) -> Result<Vec<u8>, String> {
        // midly borrows event payloads, so the track names have to outlive
        // the file being built. Declaring them first does that.
        let names: Vec<Vec<u8>> =
            self.tracks.iter().map(|t| t.name.clone().into_bytes()).collect();

        let mut smf = Smf::new(Header::new(
            Format::Parallel,
            Timing::Metrical(u15::new(self.ppq.min(0x7fff) as u16)),
        ));
        smf.tracks.push(self.conductor());
        for (track, name) in self.tracks.iter().zip(&names) {
            smf.tracks.push(part(track, name));
        }
        let mut out = Vec::new();
        smf.write_std(&mut out).map_err(|e| format!("Could not write the MIDI file: {e}"))?;
        Ok(out)
    }

    fn conductor(&self) -> Vec<TrackEvent<'static>> {
        let mut timed: Vec<(u32, TrackEventKind<'static>)> = Vec::new();
        for t in &self.tempos {
            timed.push((
                t.tick,
                TrackEventKind::Meta(MetaMessage::Tempo(u24::new(t.us_per_beat.min(0xff_ffff)))),
            ));
        }
        for ts in &self.time_signatures {
            // The meta event stores the denominator as a power of two.
            let power = ts.denominator.max(1).trailing_zeros().min(6) as u8;
            timed.push((
                ts.tick,
                TrackEventKind::Meta(MetaMessage::TimeSignature(ts.numerator, power, 24, 8)),
            ));
        }
        for k in &self.key_signatures {
            timed.push((k.tick, TrackEventKind::Meta(MetaMessage::KeySignature(k.sharps, k.minor))));
        }
        timed.extend(self.conductor_extra.iter().cloned());
        // Keep the file's length even when the last bars are silent.
        finish(timed, self.end_tick)
    }
}

fn part<'a>(track: &'a super::Track, name: &'a [u8]) -> Vec<TrackEvent<'a>> {
    let channel = u4::new(track.channel & 0x0f);
    let mut timed: Vec<(u32, TrackEventKind<'a>)> = Vec::new();
    if !name.is_empty() {
        timed.push((0, TrackEventKind::Meta(MetaMessage::TrackName(name))));
    }
    timed.push((
        0,
        TrackEventKind::Midi {
            channel,
            message: MidiMessage::ProgramChange { program: u7::new(track.program & 0x7f) },
        },
    ));
    timed.extend(track.passthrough.iter().cloned());
    for note in &track.notes {
        timed.push((
            note.start,
            TrackEventKind::Midi {
                channel,
                message: MidiMessage::NoteOn {
                    key: u7::new(note.key & 0x7f),
                    vel: u7::new(note.velocity.clamp(1, 127)),
                },
            },
        ));
        timed.push((
            note.end(),
            TrackEventKind::Midi {
                channel,
                message: MidiMessage::NoteOff { key: u7::new(note.key & 0x7f), vel: u7::new(64) },
            },
        ));
    }
    let end = track
        .notes
        .iter()
        .map(Note::end)
        .chain(track.passthrough.iter().map(|(tick, _)| *tick))
        .max()
        .unwrap_or(0);
    finish(timed, end)
}

/// Order by tick — releases before presses, so a repeated note reads as two
/// notes — then convert absolute ticks to deltas and close the track.
fn finish<'a>(mut timed: Vec<(u32, TrackEventKind<'a>)>, end: u32) -> Vec<TrackEvent<'a>> {
    timed.sort_by_key(|(tick, kind)| (*tick, order(kind)));
    let end = timed.last().map_or(end, |(tick, _)| end.max(*tick));

    let mut last = 0u32;
    let mut out = Vec::with_capacity(timed.len() + 1);
    for (tick, kind) in timed {
        out.push(TrackEvent { delta: u28::new(tick - last), kind });
        last = tick;
    }
    out.push(TrackEvent {
        delta: u28::new(end - last),
        kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
    });
    out
}

fn order(kind: &TrackEventKind<'_>) -> u8 {
    match kind {
        TrackEventKind::Meta(_) => 0,
        TrackEventKind::Midi { message: MidiMessage::NoteOff { .. }, .. } => 1,
        TrackEventKind::Midi { message: MidiMessage::NoteOn { .. }, .. } => 3,
        _ => 2,
    }
}
