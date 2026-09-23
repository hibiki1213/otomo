//! Song data: the packed format sent by the frontend and the flattened,
//! sample-timed event list the player consumes.

/// A parsed song. Crosses IPC as a packed little-endian binary body (see
/// `decode_song` / `encodeSong` in rustEngine.ts): dense files carry hundreds
/// of thousands of controller events, far too many for JSON.
pub struct SongData {
    pub duration: f64,
    pub tracks: Vec<TrackData>,
    pub notes: Vec<NoteData>,
    pub channel_events: Vec<ChannelEventData>,
}

pub struct TrackData {
    /// MIDI channel the track's events belong to (9 = GM percussion).
    pub channel: u8,
    pub program: u8,
}

pub struct NoteData {
    pub track: u16,
    pub key: u8,
    pub velocity: u8,
    pub start: f64,
    pub end: f64,
}

/// A raw channel voice message (controller, pitch bend, …) at an absolute time.
pub struct ChannelEventData {
    pub time: f64,
    pub channel: u8,
    pub status: u8,
    pub data1: u8,
    pub data2: u8,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EventKind {
    // Declaration order is the tie-break order for simultaneous events:
    // release old notes, then apply controllers, then start new notes.
    NoteOff { track: u16, key: u8 },
    /// Addressed to a MIDI channel: every track on that channel receives it.
    Channel { channel: u8, status: u8, data1: u8, data2: u8 },
    NoteOn { track: u16, key: u8, velocity: u8 },
}

impl EventKind {
    pub fn order(&self) -> u8 {
        match self {
            EventKind::NoteOff { .. } => 0,
            EventKind::Channel { .. } => 1,
            EventKind::NoteOn { .. } => 2,
        }
    }
}

pub struct Event {
    /// Absolute position in output samples.
    pub at: u64,
    pub kind: EventKind,
}

pub struct TrackInfo {
    pub midi_channel: u8,
    pub program: u8,
}

pub struct Song {
    pub events: Vec<Event>,
    pub tracks: Vec<TrackInfo>,
    /// Tracks listening on each MIDI channel.
    pub channel_tracks: [Vec<u16>; 16],
    pub duration: u64,
}

impl Song {
    pub fn empty() -> Self {
        Self { events: Vec::new(), tracks: Vec::new(), channel_tracks: Default::default(), duration: 0 }
    }
}

struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl Reader<'_> {
    fn take<const N: usize>(&mut self) -> Result<[u8; N], String> {
        let end = self.pos + N;
        let bytes = self.buf.get(self.pos..end).ok_or("truncated song data")?;
        self.pos = end;
        Ok(bytes.try_into().expect("slice length"))
    }
    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take::<1>()?[0])
    }
    fn u16(&mut self) -> Result<u16, String> {
        Ok(u16::from_le_bytes(self.take()?))
    }
    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take()?))
    }
    fn f64(&mut self) -> Result<f64, String> {
        Ok(f64::from_le_bytes(self.take()?))
    }
}

/// Layout (little-endian):
/// `f64 duration, u32 tracks, u32 notes, u32 events`, then
/// tracks `[u8 channel, u8 program]`,
/// notes `[f64 start, f64 end, u16 track, u8 key, u8 velocity]`,
/// events `[f64 time, u8 channel, u8 status, u8 data1, u8 data2]`.
pub fn decode_song(buf: &[u8]) -> Result<SongData, String> {
    let mut r = Reader { buf, pos: 0 };
    let duration = r.f64()?;
    let (nt, nn, ne) = (r.u32()? as usize, r.u32()? as usize, r.u32()? as usize);
    let expected = 20 + nt * 2 + nn * 20 + ne * 12;
    if buf.len() != expected {
        return Err(format!("song data size {} != expected {expected}", buf.len()));
    }
    let tracks = (0..nt)
        .map(|_| Ok(TrackData { channel: r.u8()?, program: r.u8()? }))
        .collect::<Result<_, String>>()?;
    let notes = (0..nn)
        .map(|_| {
            Ok(NoteData { start: r.f64()?, end: r.f64()?, track: r.u16()?, key: r.u8()?, velocity: r.u8()? })
        })
        .collect::<Result<_, String>>()?;
    let channel_events = (0..ne)
        .map(|_| {
            Ok(ChannelEventData {
                time: r.f64()?,
                channel: r.u8()?,
                status: r.u8()?,
                data1: r.u8()?,
                data2: r.u8()?,
            })
        })
        .collect::<Result<_, String>>()?;
    Ok(SongData { duration, tracks, notes, channel_events })
}

/// Flatten a song into a time-sorted event list (in samples). Tracks beyond
/// `max_tracks` are dropped.
pub fn build_song(song: &SongData, sample_rate: u32, max_tracks: usize) -> Song {
    let sr = f64::from(sample_rate);
    let at = |t: f64| (t.max(0.0) * sr).round() as u64;
    let track_count = song.tracks.len().min(max_tracks);

    let mut events = Vec::with_capacity(song.notes.len() * 2 + song.channel_events.len());
    for n in &song.notes {
        if n.velocity == 0 || usize::from(n.track) >= track_count {
            continue;
        }
        events.push(Event {
            at: at(n.start),
            kind: EventKind::NoteOn { track: n.track, key: n.key & 0x7f, velocity: n.velocity.min(127) },
        });
        events.push(Event { at: at(n.end.max(n.start)), kind: EventKind::NoteOff { track: n.track, key: n.key & 0x7f } });
    }
    for c in &song.channel_events {
        // Program changes are applied from the track program instead.
        if c.channel >= 16 || c.status == 0xC0 {
            continue;
        }
        events.push(Event {
            at: at(c.time),
            kind: EventKind::Channel { channel: c.channel, status: c.status, data1: c.data1, data2: c.data2 },
        });
    }
    events.sort_by_key(|e| (e.at, e.kind.order()));

    let tracks: Vec<TrackInfo> = song.tracks[..track_count]
        .iter()
        .map(|t| TrackInfo { midi_channel: t.channel.min(15), program: t.program.min(127) })
        .collect();
    let mut channel_tracks: [Vec<u16>; 16] = Default::default();
    for (i, t) in tracks.iter().enumerate() {
        channel_tracks[usize::from(t.midi_channel)].push(i as u16);
    }
    Song { events, tracks, channel_tracks, duration: at(song.duration) }
}
