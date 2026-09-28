//! Sequencer and per-track instruments.
//!
//! Every track owns its sound source:
//! * [`Source::Sf2`] — the shared SoundFont synthesizer, on a synth channel of
//!   its own (track `i` = synth channel `i`), so its program, controllers and
//!   output are independent of other tracks even when they share a MIDI
//!   channel in the file;
//! * [`Source::External`] — any other instrument (SFZ, plugin) behind the
//!   [`Instrument`] trait.
//!
//! Controller and pitch-bend events are addressed to MIDI channels as in the
//! file and fanned out to every track on that channel.

use rustysynth::Synthesizer;

use super::song::{EventKind, Song};
use crate::spatial::TrackBuffers;

/// Upper bound on tracks (and synth channels); buffers are preallocated.
pub const MAX_TRACKS: usize = 256;
const PERCUSSION_CHANNEL: u8 = 9;
/// Controllers that form RPN/NRPN sequences; replayed in order on seek.
const PARAMETER_CCS: [u8; 6] = [6, 38, 98, 99, 100, 101];

/// A non-SoundFont instrument driven from the audio thread. Events carry a
/// frame offset (`delay`) within the next [`Instrument::render`] call.
pub trait Instrument: Send {
    fn note_on(&mut self, delay: usize, key: u8, velocity: u8);
    fn note_off(&mut self, delay: usize, key: u8);
    /// Controller (0xB0) or pitch bend (0xE0). Pan (CC10) is never sent:
    /// the spatial stage positions tracks.
    fn channel_message(&mut self, delay: usize, status: u8, data1: u8, data2: u8);
    fn all_notes_off(&mut self, immediate: bool);
    /// Back to default controller state (before a seek replays them).
    fn reset(&mut self);
    /// Render `left.len()` frames, overwriting the buffers. Returns false if
    /// the output is known to be silent.
    fn render(&mut self, left: &mut [f32], right: &mut [f32]) -> bool;
}

pub enum Source {
    Sf2,
    External(Box<dyn Instrument>),
}

pub struct Track {
    pub source: Source,
    pub midi_channel: u8,
    pub program: u8,
    pub muted: bool,
    /// Mixer level, linear (1 = as written). Set from the UI's fader.
    pub gain: f32,
    /// The level external instruments were last scaled by, so a fader move
    /// ramps across one block instead of clicking.
    applied_gain: f32,
    /// Current pan from CC10, -1 (left) … 1 (right).
    pub pan: f32,
    /// Sounding-note count per key, so mute/pause release exactly what this
    /// track started.
    active: [u8; 128],
}

impl Track {
    pub fn new(midi_channel: u8, program: u8) -> Self {
        Self {
            source: Source::Sf2,
            midi_channel,
            program,
            muted: false,
            gain: 1.0,
            applied_gain: 1.0,
            pan: 0.0,
            active: [0; 128],
        }
    }

    fn is_percussion(&self) -> bool {
        self.midi_channel == PERCUSSION_CHANNEL
    }
}

pub struct Player {
    pub synth: Option<Box<Synthesizer>>,
    pub song: Box<Song>,
    pub tracks: Vec<Track>,
    pub cursor: usize,
    pub position: u64,
    pub playing: bool,
    pub ended: bool,
}

/// Send one channel message to every track on `channel`. `delay` is the
/// frame offset within the engine block (used by external instruments).
#[allow(clippy::too_many_arguments)]
fn route_channel(
    song: &Song,
    tracks: &mut [Track],
    mut synth: Option<&mut Synthesizer>,
    delay: usize,
    channel: u8,
    status: u8,
    data1: u8,
    data2: u8,
) {
    for &t in &song.channel_tracks[usize::from(channel & 15)] {
        let Some(track) = tracks.get_mut(usize::from(t)) else { continue };
        let is_pan = status == 0xB0 && data1 == 10;
        if is_pan {
            track.pan = (f32::from(data2) - 64.0) / 63.0;
            track.pan = track.pan.clamp(-1.0, 1.0);
        }
        match &mut track.source {
            Source::Sf2 => {
                if let Some(s) = synth.as_deref_mut() {
                    s.process_midi_message(i32::from(t), i32::from(status), i32::from(data1), i32::from(data2));
                }
            }
            Source::External(inst) => {
                if !is_pan {
                    inst.channel_message(delay, status, data1, data2);
                }
            }
        }
    }
}

impl Player {
    pub fn new() -> Self {
        Self {
            synth: None,
            song: Box::new(Song::empty()),
            tracks: Vec::new(),
            cursor: 0,
            position: 0,
            playing: false,
            ended: false,
        }
    }

    /// Point synth channels at this song's tracks (percussion flags).
    fn configure_synth(&mut self) {
        let Some(synth) = self.synth.as_mut() else { return };
        for ch in 0..synth.channel_count() {
            let percussion = self.tracks.get(ch).is_some_and(|t| t.is_percussion());
            synth.set_percussion_channel(ch, percussion);
            synth.set_channel_gain(ch, self.tracks.get(ch).map_or(1.0, |t| t.gain));
        }
    }

    pub fn set_synth(&mut self, synth: Box<Synthesizer>) -> Option<Box<Synthesizer>> {
        let old = self.synth.replace(synth);
        self.configure_synth();
        self.seek(self.position);
        old
    }

    /// Install a new song; returns the previous tracks for disposal.
    pub fn load(&mut self, song: Box<Song>, tracks: Vec<Track>) -> (Box<Song>, Vec<Track>) {
        self.playing = false;
        let old_song = std::mem::replace(&mut self.song, song);
        let old_tracks = std::mem::replace(&mut self.tracks, tracks);
        self.configure_synth();
        self.seek(0);
        (old_song, old_tracks)
    }

    fn send_program(&mut self, t: usize) {
        let (Some(synth), Some(track)) = (self.synth.as_mut(), self.tracks.get(t)) else { return };
        if let Source::Sf2 = track.source {
            synth.process_midi_message(t as i32, 0xC0, i32::from(track.program), 0);
        }
    }

    /// Jump to `target` samples: silence everything, reset controllers, then
    /// restore controller state as of `target`. Only the last value of each
    /// controller is sent (RPN/NRPN sequences are replayed in order), which
    /// keeps seeks cheap on files with hundreds of thousands of events.
    pub fn seek(&mut self, target: u64) {
        let target = target.min(self.song.duration);
        if let Some(synth) = self.synth.as_mut() {
            synth.reset();
        }
        for track in &mut self.tracks {
            track.active = [0; 128];
            track.pan = 0.0;
            if let Source::External(inst) = &mut track.source {
                inst.reset();
            }
        }
        for t in 0..self.tracks.len() {
            self.send_program(t);
        }

        let mut last_cc = [[u32::MAX; 128]; 16];
        let mut last_bend = [u32::MAX; 16];
        let mut i = 0;
        while i < self.song.events.len() && self.song.events[i].at < target {
            if let EventKind::Channel { channel, status, data1, data2 } = self.song.events[i].kind {
                let ch = usize::from(channel & 15);
                match status {
                    0xB0 if PARAMETER_CCS.contains(&data1) => route_channel(
                        &self.song,
                        &mut self.tracks,
                        self.synth.as_deref_mut(),
                        0,
                        channel,
                        status,
                        data1,
                        data2,
                    ),
                    0xB0 => last_cc[ch][usize::from(data1 & 0x7f)] = i as u32,
                    0xE0 => last_bend[ch] = i as u32,
                    _ => {}
                }
            }
            i += 1;
        }
        self.cursor = i;
        // Bank select first so later program-dependent state lands right.
        let order = [0usize, 32].into_iter().chain((1..128).filter(|&c| c != 32));
        let order: Vec<usize> = order.collect();
        for ch in 0..16 {
            for &cc in &order {
                self.replay(last_cc[ch][cc]);
            }
            self.replay(last_bend[ch]);
        }
        self.position = target;
        self.ended = false;
    }

    fn replay(&mut self, index: u32) {
        if index == u32::MAX {
            return;
        }
        if let EventKind::Channel { channel, status, data1, data2 } = self.song.events[index as usize].kind {
            route_channel(&self.song, &mut self.tracks, self.synth.as_deref_mut(), 0, channel, status, data1, data2);
        }
    }

    /// Bring one track's instrument up to the current controller state
    /// (after its source was swapped mid-song).
    pub fn resync_track(&mut self, t: usize) {
        let Some(track) = self.tracks.get(t) else { return };
        let channel = track.midi_channel;
        self.send_program(t);
        let mut last_cc = [u32::MAX; 128];
        let mut last_bend = u32::MAX;
        for (i, ev) in self.song.events[..self.cursor].iter().enumerate() {
            if let EventKind::Channel { channel: c, status, data1, .. } = ev.kind {
                if c != channel {
                    continue;
                }
                match status {
                    0xB0 => last_cc[usize::from(data1 & 0x7f)] = i as u32,
                    0xE0 => last_bend = i as u32,
                    _ => {}
                }
            }
        }
        let track = &mut self.tracks[t];
        let mut send = |index: u32| {
            if index == u32::MAX {
                return;
            }
            if let EventKind::Channel { status, data1, data2, .. } = self.song.events[index as usize].kind {
                if status == 0xB0 && data1 == 10 {
                    track.pan = ((f32::from(data2) - 64.0) / 63.0).clamp(-1.0, 1.0);
                    return;
                }
                match &mut track.source {
                    Source::Sf2 => {
                        if let Some(s) = self.synth.as_deref_mut() {
                            s.process_midi_message(t as i32, i32::from(status), i32::from(data1), i32::from(data2));
                        }
                    }
                    Source::External(inst) => inst.channel_message(0, status, data1, data2),
                }
            }
        };
        for cc in last_cc {
            send(cc);
        }
        send(last_bend);
    }

    fn release_track(&mut self, t: usize, immediate: bool) {
        let Some(track) = self.tracks.get_mut(t) else { return };
        track.active = [0; 128];
        match &mut track.source {
            Source::Sf2 => {
                if let Some(s) = self.synth.as_mut() {
                    s.note_off_all_channel(t as i32, immediate);
                }
            }
            Source::External(inst) => inst.all_notes_off(immediate),
        }
    }

    pub fn release_all(&mut self) {
        for t in 0..self.tracks.len() {
            self.release_track(t, false);
        }
    }

    pub fn set_muted(&mut self, t: usize, muted: bool) {
        let Some(track) = self.tracks.get_mut(t) else { return };
        track.muted = muted;
        if muted {
            self.release_track(t, false);
        }
    }

    /// Set a track's mixer level (linear). The SoundFont applies it per voice;
    /// other instruments are scaled after rendering.
    pub fn set_gain(&mut self, t: usize, gain: f32) {
        let Some(track) = self.tracks.get_mut(t) else { return };
        let gain = if gain.is_finite() { gain.clamp(0.0, 4.0) } else { 1.0 };
        track.gain = gain;
        if let Some(synth) = self.synth.as_mut() {
            synth.set_channel_gain(t, gain);
        }
    }

    pub fn set_program(&mut self, t: usize, program: u8) {
        let Some(track) = self.tracks.get_mut(t) else { return };
        if track.is_percussion() {
            return;
        }
        track.program = program.min(127);
        self.send_program(t);
    }

    /// Swap a track's source; returns the old one for disposal.
    pub fn set_source(&mut self, t: usize, source: Source) -> Option<Source> {
        if t >= self.tracks.len() {
            return Some(source);
        }
        self.release_track(t, true);
        let old = std::mem::replace(&mut self.tracks[t].source, source);
        self.resync_track(t);
        Some(old)
    }

    /// `delay`: frame offset of the event within the current engine block.
    fn dispatch(&mut self, kind: EventKind, delay: usize) {
        match kind {
            EventKind::NoteOn { track, key, velocity } => {
                let t = usize::from(track);
                let Some(tr) = self.tracks.get_mut(t) else { return };
                if tr.muted {
                    return;
                }
                match &mut tr.source {
                    Source::Sf2 => {
                        let Some(s) = self.synth.as_mut() else { return };
                        s.note_on(t as i32, i32::from(key), i32::from(velocity));
                    }
                    Source::External(inst) => inst.note_on(delay, key, velocity),
                }
                let slot = &mut tr.active[usize::from(key)];
                *slot = slot.saturating_add(1);
            }
            EventKind::NoteOff { track, key } => {
                let t = usize::from(track);
                let Some(tr) = self.tracks.get_mut(t) else { return };
                let slot = &mut tr.active[usize::from(key)];
                if *slot == 0 {
                    return;
                }
                *slot -= 1;
                // Overlapping notes on the same key: release on the last off.
                if *slot > 0 {
                    return;
                }
                match &mut tr.source {
                    Source::Sf2 => {
                        if let Some(s) = self.synth.as_mut() {
                            s.note_off(t as i32, i32::from(key));
                        }
                    }
                    Source::External(inst) => inst.note_off(delay, key),
                }
            }
            EventKind::Channel { channel, status, data1, data2 } => {
                route_channel(
                    &self.song,
                    &mut self.tracks,
                    self.synth.as_deref_mut(),
                    delay,
                    channel,
                    status,
                    data1,
                    data2,
                );
            }
        }
    }

    /// Render one engine block into `out`.
    ///
    /// The SoundFont synth renders in its own small blocks, with events
    /// applied between them. External instruments get their events with a
    /// frame offset and render the whole block in one call: plugins and sfizz
    /// have a fixed cost per call, which at 16 calls per block × dozens of
    /// instruments was enough to overload the audio thread.
    pub fn render(&mut self, out: &mut TrackBuffers) {
        out.clear();
        let block = out.block();
        let sub = self.synth.as_ref().map_or(64, |s| s.get_block_size());
        for b in 0..block / sub {
            if self.playing {
                while let Some(ev) = self.song.events.get(self.cursor) {
                    if ev.at > self.position {
                        break;
                    }
                    let kind = ev.kind;
                    self.cursor += 1;
                    self.dispatch(kind, b * sub);
                }
            }

            let range = b * sub..(b + 1) * sub;
            if let Some(synth) = self.synth.as_mut() {
                synth.render_block_split();
                let (l, r) = synth.split_mix();
                out.mix[0][range.clone()].copy_from_slice(l);
                out.mix[1][range.clone()].copy_from_slice(r);
                let (l, r) = synth.split_effects();
                out.fx[0][range.clone()].copy_from_slice(l);
                out.fx[1][range.clone()].copy_from_slice(r);
            }
            if let Some(synth) = self.synth.as_ref() {
                for (t, track) in self.tracks.iter().enumerate() {
                    if let (Source::Sf2, Some((sl, sr))) = (&track.source, synth.split_channel(t)) {
                        let [l, r] = &mut out.tracks[t];
                        l[range.clone()].copy_from_slice(sl);
                        r[range.clone()].copy_from_slice(sr);
                        out.active[t] = true;
                    }
                }
            }

            if self.playing {
                self.position += sub as u64;
                if self.position >= self.song.duration && self.cursor >= self.song.events.len() {
                    // Let release tails ring out; the frontend sees `ended`.
                    self.playing = false;
                    self.ended = true;
                    self.position = self.song.duration;
                }
            }
        }

        for (t, track) in self.tracks.iter_mut().enumerate() {
            if let Source::External(inst) = &mut track.source {
                let [l, r] = &mut out.tracks[t];
                if inst.render(&mut l[..block], &mut r[..block]) {
                    out.active[t] = true;
                    let (from, to) = (track.applied_gain, track.gain);
                    if (from - 1.0).abs() > f32::EPSILON || (to - 1.0).abs() > f32::EPSILON {
                        let step = (to - from) / block as f32;
                        for i in 0..block {
                            let g = from + step * i as f32;
                            l[i] *= g;
                            r[i] *= g;
                        }
                    }
                }
                track.applied_gain = track.gain;
            }
        }

        out.count = self.tracks.len();
        for (t, track) in self.tracks.iter().enumerate() {
            out.pan[t] = track.pan;
            // External instruments are not part of the synth's mix: add them
            // with a stereo balance pan for the speaker / original renderings.
            if let Source::External(_) = track.source {
                if out.active[t] {
                    let p = track.pan;
                    // Moving right fades the left side out and folds it into
                    // the right, and vice versa.
                    let (ll, lr) = (1.0 - p.max(0.0), p.max(0.0));
                    let (rr, rl) = (1.0 + p.min(0.0), -p.min(0.0));
                    let [l, r] = &out.tracks[t];
                    for i in 0..block {
                        out.mix[0][i] += l[i] * ll + r[i] * rl;
                        out.mix[1][i] += r[i] * rr + l[i] * lr;
                    }
                }
            }
        }
    }
}
