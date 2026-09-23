//! Offline checks against a real SoundFont. Set `MIDISS_SF2` to a .sf2
//! path to run them (skipped otherwise); `MIDISS_WAV_DIR` also writes the
//! rendered test song per room/output to WAVs for listening.

//! Offline checks against a real SoundFont. Set `MIDISS_SF2` to a .sf2
//! path to run them (skipped otherwise); `MIDISS_WAV_DIR` also writes the
//! rendered test song per room/output to WAVs for listening.

use super::player::Instrument;
use super::song::{build_song, decode_song, ChannelEventData, NoteData, SongData, TrackData};
use super::*;
use crate::spatial::OutputMode;

const SR: u32 = 48_000;

fn hrtf() -> Arc<Hrtf> {
    Arc::new(Hrtf::load(SR, &mut Fft::new(BLOCK)))
}

fn renderer(config: SpatialConfig) -> Option<(Renderer, Receiver<Box<dyn Any + Send>>)> {
    let path = std::env::var("MIDISS_SF2").ok()?;
    let mut reader = BufReader::new(File::open(path).expect("open sf2"));
    let sf = Arc::new(SoundFont::new(&mut reader).expect("parse sf2"));
    let synth = new_synth(&sf, SR).expect("synth");
    let h = hrtf();
    let scene = Box::new(Scene::build(config, SR, BLOCK, &h).expect("scene"));
    let (tx, rx) = channel();
    let mut r = Renderer::new(SR, h, scene, Arc::new(Shared::default()), tx);
    r.handle(Command::SetSynth(Box::new(synth)));
    Some((r, rx))
}

fn note(track: u16, key: u8, start: f64, end: f64) -> NoteData {
    NoteData { track, key, velocity: 100, start, end }
}

fn cc(time: f64, channel: u8, status: u8, data1: u8, data2: u8) -> ChannelEventData {
    ChannelEventData { time, channel, status, data1, data2 }
}

/// Piano chord + drums + a violin sharing the piano's channel, with a
/// pitch bend, sustain pedal and volume change.
fn test_song() -> SongData {
    SongData {
        duration: 3.0,
        tracks: vec![
            TrackData { channel: 0, program: 0 },
            TrackData { channel: 9, program: 0 },
            TrackData { channel: 0, program: 40 },
        ],
        notes: vec![
            note(0, 60, 0.0, 0.5),
            note(0, 64, 0.0, 0.5),
            note(0, 67, 0.0, 0.5),
            note(1, 36, 0.0, 0.1),
            note(1, 42, 0.5, 0.6),
            note(1, 38, 1.0, 1.1),
            note(2, 72, 1.0, 2.5),
        ],
        channel_events: vec![
            cc(0.0, 0, 0xB0, 7, 110),
            cc(0.0, 0, 0xB0, 64, 127),
            cc(0.9, 0, 0xB0, 64, 0),
            cc(1.5, 0, 0xE0, 0, 96),
        ],
    }
}

/// Render until the song ends plus `tail` seconds; returns [left, right].
fn render_song(r: &mut Renderer, song: &SongData, tail: f64) -> [Vec<f32>; 2] {
    r.handle(Command::LoadSong(Box::new(load_song(song, SR))));
    play_loaded(r, tail)
}

/// Play the already-loaded song to its end plus `tail` seconds.
fn play_loaded(r: &mut Renderer, tail: f64) -> [Vec<f32>; 2] {
    r.handle(Command::Play);
    let mut out = [Vec::new(), Vec::new()];
    let mut buf = vec![0f32; 512 * 2];
    let mut after_end = 0usize;
    let limit = (tail * f64::from(SR)) as usize;
    while after_end < limit {
        r.render(&mut buf, 2);
        for f in buf.chunks(2) {
            out[0].push(f[0]);
            out[1].push(f[1]);
        }
        if r.player.ended {
            after_end += 512;
        }
        assert!(out[0].len() < 60 * SR as usize, "song never ended");
    }
    out
}

fn rms(samples: &[f32]) -> f32 {
    (samples.iter().map(|s| s * s).sum::<f32>() / samples.len().max(1) as f32).sqrt()
}

fn window(out: &[f32], from: f64, to: f64) -> &[f32] {
    let a = (from * f64::from(SR)) as usize;
    let b = ((to * f64::from(SR)) as usize).min(out.len());
    &out[a..b]
}

fn write_wav(path: &str, out: &[Vec<f32>; 2]) {
    let data: Vec<u8> = out[0]
        .iter()
        .zip(&out[1])
        .flat_map(|(l, r)| [*l, *r])
        .flat_map(|s| ((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes())
        .collect();
    let mut w = Vec::new();
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&SR.to_le_bytes());
    w.extend_from_slice(&(SR * 4).to_le_bytes());
    w.extend_from_slice(&4u16.to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&(data.len() as u32).to_le_bytes());
    w.extend_from_slice(&data);
    std::fs::write(path, w).expect("write wav");
}

fn config(room: &str, seat: usize, output: OutputMode) -> SpatialConfig {
    SpatialConfig { room: room.into(), seat, output }
}

#[test]
fn decodes_packed_song() {
    let mut b = Vec::new();
    b.extend_from_slice(&2.5f64.to_le_bytes());
    for n in [1u32, 1, 1] {
        b.extend_from_slice(&n.to_le_bytes());
    }
    b.extend_from_slice(&[9, 25]);
    b.extend_from_slice(&0.5f64.to_le_bytes());
    b.extend_from_slice(&1.0f64.to_le_bytes());
    b.extend_from_slice(&0u16.to_le_bytes());
    b.extend_from_slice(&[36, 100]);
    b.extend_from_slice(&0.25f64.to_le_bytes());
    b.extend_from_slice(&[9, 0xB0, 7, 90]);

    let song = decode_song(&b).expect("decode");
    assert_eq!(song.duration, 2.5);
    assert_eq!((song.tracks[0].channel, song.tracks[0].program), (9, 25));
    let n = &song.notes[0];
    assert_eq!((n.start, n.end, n.track, n.key, n.velocity), (0.5, 1.0, 0, 36, 100));
    let c = &song.channel_events[0];
    assert_eq!((c.time, c.channel, c.status, c.data1, c.data2), (0.25, 9, 0xB0, 7, 90));

    assert!(decode_song(&b[..b.len() - 1]).is_err(), "truncated input rejected");
}

#[test]
fn events_sorted_with_release_before_attack() {
    let song = SongData {
        duration: 1.0,
        tracks: vec![TrackData { channel: 0, program: 0 }],
        notes: vec![note(0, 60, 0.0, 0.5), note(0, 60, 0.5, 1.0)],
        channel_events: vec![cc(0.5, 0, 0xB0, 7, 100), cc(0.5, 0, 0xC0, 5, 0)],
    };
    let order: Vec<u8> = build_song(&song, SR, MAX_TRACKS).events.iter().map(|e| e.kind.order()).collect();
    // Program change is dropped; at t=0.5: off, controller, on.
    assert_eq!(order, vec![2, 0, 1, 2, 0]);
}

#[test]
fn split_mix_matches_plain_render() {
    // The per-channel split must not change what "original" sounds like.
    let Ok(path) = std::env::var("MIDISS_SF2") else { return };
    let sf = Arc::new(SoundFont::new(&mut BufReader::new(File::open(path).unwrap())).unwrap());
    let settings = SynthesizerSettings::new(SR as i32);
    let mut a = Synthesizer::new(&sf, &settings).unwrap();
    let mut b = Synthesizer::new(&sf, &settings).unwrap();
    for s in [&mut a, &mut b] {
        s.process_midi_message(0, 0xB0, 10, 20);
        s.note_on(0, 60, 100);
        s.note_on(9, 38, 100);
    }
    let bs = a.get_block_size();
    let (mut l, mut r) = (vec![0f32; bs * 50], vec![0f32; bs * 50]);
    a.render(&mut l, &mut r);
    let mut max_err = 0f32;
    let mut dry = 0f32;
    for blk in 0..50 {
        b.render_block_split();
        let (ml, mr) = b.split_mix();
        for i in 0..bs {
            max_err = max_err.max((ml[i] - l[blk * bs + i]).abs()).max((mr[i] - r[blk * bs + i]).abs());
        }
        if let Some((cl, _)) = b.split_channel(0) {
            dry += cl.iter().map(|v| v.abs()).sum::<f32>();
        }
    }
    assert!(max_err < 1e-6, "split mix differs: {max_err}");
    assert!(dry > 0.0, "channel 0 dry output present");
    assert!(b.split_channel(5).is_none(), "silent channel reports None");
}

#[test]
fn plays_to_end_in_every_room_and_output() {
    let wav_dir = std::env::var("MIDISS_WAV_DIR").ok();
    let mut rooms = vec!["original".to_string(), "dry".to_string()];
    rooms.extend(room::VENUES.iter().map(|v| v.id.to_string()));
    for room_id in &rooms {
        for output in [OutputMode::Speakers, OutputMode::Headphones] {
            let Some((mut r, _trash)) = renderer(config(room_id, 1, output)) else {
                eprintln!("MIDISS_SF2 not set; skipping");
                return;
            };
            let out = render_song(&mut r, &test_song(), 3.0);
            let all = || out.iter().flatten();
            assert!(all().all(|s| s.is_finite()));
            let peak = all().fold(0f32, |m, s| m.max(s.abs()));
            assert!(peak <= 1.0, "{room_id} {output:?}: peak {peak}");
            let early = rms(window(&out[0], 0.0, 0.4)) + rms(window(&out[1], 0.0, 0.4));
            let violin = rms(window(&out[0], 1.2, 2.4)) + rms(window(&out[1], 1.2, 2.4));
            let tail = rms(window(&out[0], 3.3, 3.8)) + rms(window(&out[1], 3.3, 3.8));
            eprintln!(
                "{room_id:>10} {output:>10?}: peak {peak:.2} early {early:.3} violin {violin:.3} tail {tail:.4}"
            );
            assert!(early > 0.01 && violin > 0.005, "{room_id} {output:?} too quiet");
            if let Some(dir) = &wav_dir {
                write_wav(&format!("{dir}/{room_id}-{output:?}.wav").to_lowercase(), &out);
            }
        }
    }
}

#[test]
fn venue_reverb_outlasts_dry() {
    let tail_of = |room_id: &str| -> Option<f32> {
        let (mut r, _t) = renderer(config(room_id, 2, OutputMode::Speakers))?;
        let out = render_song(&mut r, &test_song(), 2.0);
        Some(rms(window(&out[0], 3.4, 4.0)))
    };
    let (Some(dry), Some(hall)) = (tail_of("dry"), tail_of("concert")) else { return };
    assert!(hall > 10.0 * dry.max(1e-6), "hall tail {hall} vs dry {dry}");
}

#[test]
fn binaural_places_panned_channel_on_its_side() {
    let Some((mut r, _t)) = renderer(config("dry", 1, OutputMode::Headphones)) else { return };
    let song = SongData {
        duration: 1.0,
        tracks: vec![TrackData { channel: 0, program: 0 }],
        notes: vec![note(0, 60, 0.0, 0.8)],
        // Hard left.
        channel_events: vec![cc(0.0, 0, 0xB0, 10, 0)],
    };
    let out = render_song(&mut r, &song, 0.5);
    let (l, rr) = (rms(&out[0]), rms(&out[1]));
    assert!(l > 1.3 * rr, "left {l} right {rr}");
    // At piano pitches the ears mostly localise by arrival time: the right
    // ear should lag the left by a few hundred microseconds.
    let (a, b) = (window(&out[0], 0.1, 0.6), window(&out[1], 0.1, 0.6));
    let xcorr = |lag: usize| -> f32 { a.iter().zip(&b[lag..]).map(|(x, y)| x * y).sum() };
    let lag = (0..40).max_by(|&i, &j| xcorr(i).total_cmp(&xcorr(j))).unwrap();
    let lead_right = (0..40).map(|lag| b.iter().zip(&a[lag..]).map(|(x, y)| x * y).sum::<f32>());
    assert!(xcorr(lag) > lead_right.fold(f32::MIN, f32::max), "left ear should lead");
    assert!(lag >= 8, "interaural delay {lag} samples");
}

#[test]
fn muted_track_is_silent() {
    let Some((mut r, _t)) = renderer(SpatialConfig::default()) else { return };
    let song = SongData {
        duration: 1.0,
        tracks: vec![TrackData { channel: 0, program: 0 }],
        notes: vec![note(0, 60, 0.0, 0.8)],
        channel_events: vec![],
    };
    r.handle(Command::LoadSong(Box::new(load_song(&song, SR))));
    r.handle(Command::Mute(0, true));
    r.handle(Command::Play);
    let mut buf = vec![0f32; 1024];
    let mut energy = 0.0;
    for _ in 0..100 {
        r.render(&mut buf, 2);
        energy += buf.iter().map(|v| v * v).sum::<f32>();
    }
    assert!(energy < 1e-6, "muted output should be silent");
}

#[test]
fn seek_lines_up_cursor_and_position() {
    let Some((mut r, _t)) = renderer(SpatialConfig::default()) else { return };
    r.handle(Command::LoadSong(Box::new(load_song(&test_song(), SR))));
    r.handle(Command::Seek((1.2 * f64::from(SR)) as u64));
    let p = &r.player;
    assert_eq!(p.position, (1.2 * f64::from(SR)) as u64);
    assert!(p.song.events[..p.cursor].iter().all(|e| e.at < p.position));
    assert!(p.song.events[p.cursor..].iter().all(|e| e.at >= p.position));
}

/// 64 tracks x 4-note chords every 250 ms, from the cheapest to the most
/// expensive path. Run with --release for real numbers.
#[test]
fn dense_arrangement_realtime_factor() {
    for (room_id, output) in [
        ("original", OutputMode::Speakers),
        ("dry", OutputMode::Headphones),
        ("cathedral", OutputMode::Speakers),
        ("cathedral", OutputMode::Headphones),
    ] {
        let Some((mut r, _t)) = renderer(config(room_id, 2, output)) else { return };
        let factor = dense_factor(&mut r);
        eprintln!("dense arrangement, {room_id} {output:?}: {factor:.1}x realtime");
    }
}

fn dense_factor(r: &mut Renderer) -> f64 {
    let mut tracks = Vec::new();
    let mut notes = Vec::new();
    for t in 0..64u16 {
        let ch = (t % 15) as u8;
        let ch = if ch >= 9 { ch + 1 } else { ch };
        tracks.push(TrackData { channel: ch, program: (t * 2 % 128) as u8 });
        for step in 0..40 {
            let start = f64::from(step) * 0.25;
            for k in 0..4u8 {
                notes.push(note(t, 48 + (t as u8 % 24) + k * 4, start, start + 0.6));
            }
        }
    }
    let channel_events = (0..16u8).filter(|&c| c != 9).map(|c| cc(0.0, c, 0xB0, 10, c * 8)).collect();
    let song = SongData { duration: 10.0, tracks, notes, channel_events };
    let t0 = std::time::Instant::now();
    let out = render_song(r, &song, 0.1);
    let secs = out[0].len() as f64 / f64::from(SR);
    assert!(out.iter().flatten().all(|s| s.is_finite()));
    secs / t0.elapsed().as_secs_f64()
}

/// Records what it receives and plays a sine per held key.
struct TestInstrument {
    log: Arc<Mutex<Vec<String>>>,
    held: Vec<(u8, f32)>,
}

impl Instrument for TestInstrument {
    fn note_on(&mut self, _delay: usize, key: u8, velocity: u8) {
        self.log.lock().unwrap().push(format!("on {key} {velocity}"));
        self.held.push((key, 0.0));
    }
    fn note_off(&mut self, _delay: usize, key: u8) {
        self.log.lock().unwrap().push(format!("off {key}"));
        self.held.retain(|(k, _)| *k != key);
    }
    fn channel_message(&mut self, _delay: usize, status: u8, data1: u8, data2: u8) {
        self.log.lock().unwrap().push(format!("msg {status:02x} {data1} {data2}"));
    }
    fn all_notes_off(&mut self, _immediate: bool) {
        self.held.clear();
    }
    fn reset(&mut self) {
        self.log.lock().unwrap().push("reset".into());
    }
    fn render(&mut self, left: &mut [f32], right: &mut [f32]) -> bool {
        left.fill(0.0);
        right.fill(0.0);
        for (key, phase) in &mut self.held {
            let step = 440.0 * 2f32.powf((f32::from(*key) - 69.0) / 12.0) / SR as f32;
            for (l, r) in left.iter_mut().zip(right.iter_mut()) {
                let v = (*phase * std::f32::consts::TAU).sin() * 0.2;
                *l += v;
                *r += v;
                *phase = (*phase + step).fract();
            }
        }
        !self.held.is_empty()
    }
}

#[test]
fn tracks_sharing_a_midi_channel_play_independently() {
    let Some((mut r, _t)) = renderer(SpatialConfig::default()) else { return };
    let song = SongData {
        duration: 1.0,
        tracks: vec![TrackData { channel: 0, program: 0 }, TrackData { channel: 0, program: 40 }],
        notes: vec![note(0, 60, 0.0, 0.8), note(1, 72, 0.0, 0.8)],
        channel_events: vec![],
    };
    r.handle(Command::LoadSong(Box::new(load_song(&song, SR))));
    r.handle(Command::Play);
    let mut buf = vec![0f32; 2048];
    r.render(&mut buf, 2);
    r.render(&mut buf, 2);
    assert!(r.buffers.active[0] && r.buffers.active[1], "both tracks sound");
    let e = |r: &Renderer, t: usize| r.buffers.tracks[t][0].iter().map(|v| v * v).sum::<f32>();
    assert!(e(&r, 0) > 0.0 && e(&r, 1) > 0.0);
    // Muting one leaves the other.
    r.handle(Command::Mute(0, true));
    for _ in 0..40 {
        r.render(&mut buf, 2);
    }
    assert!(!r.buffers.active[0] || e(&r, 0) < 1e-6, "muted track released");
    assert!(r.buffers.active[1]);
}

#[test]
fn external_instrument_receives_notes_and_controllers_but_not_pan() {
    let Some((mut r, _t)) = renderer(SpatialConfig::default()) else { return };
    let song = SongData {
        duration: 1.0,
        tracks: vec![TrackData { channel: 3, program: 0 }],
        notes: vec![note(0, 69, 0.1, 0.5)],
        channel_events: vec![cc(0.0, 3, 0xB0, 7, 90), cc(0.0, 3, 0xB0, 10, 0), cc(0.2, 3, 0xE0, 0, 80)],
    };
    r.handle(Command::LoadSong(Box::new(load_song(&song, SR))));
    let log = Arc::new(Mutex::new(Vec::new()));
    r.handle(Command::SetSource(0, Source::External(Box::new(TestInstrument { log: log.clone(), held: vec![] }))));
    let out = play_loaded(&mut r, 0.2);
    let log = log.lock().unwrap().clone();
    assert!(log.contains(&"msg b0 7 90".to_string()), "{log:?}");
    assert!(log.contains(&"on 69 100".to_string()));
    assert!(log.contains(&"off 69".to_string()));
    assert!(log.contains(&"msg e0 0 80".to_string()));
    assert!(!log.iter().any(|m| m.starts_with("msg b0 10 ")), "pan must not reach the instrument");
    // Hard-left pan is applied by the engine in the speaker mix.
    let (l, rr) = (rms(&out[0]), rms(&out[1]));
    assert!(l > 0.01 && rr < l * 0.05, "left {l} right {rr}");
}

#[test]
fn compacted_seek_restores_controller_state() {
    // A volume ramp before the note: seeking past it must land on the last
    // value, exactly like a song that only contains that final value.
    let ramp: Vec<ChannelEventData> = (0..=100).map(|i| cc(i as f64 * 0.01, 0, 0xB0, 7, (127 - i) as u8)).collect();
    let with_ramp = SongData {
        duration: 2.0,
        tracks: vec![TrackData { channel: 0, program: 0 }],
        notes: vec![note(0, 60, 1.5, 1.9)],
        channel_events: ramp,
    };
    let final_only = SongData {
        duration: 2.0,
        tracks: vec![TrackData { channel: 0, program: 0 }],
        notes: vec![note(0, 60, 1.5, 1.9)],
        channel_events: vec![cc(0.0, 0, 0xB0, 7, 27)],
    };
    let level = |song: &SongData| -> Option<f32> {
        let (mut r, _t) = renderer(SpatialConfig::default())?;
        r.handle(Command::LoadSong(Box::new(load_song(song, SR))));
        r.handle(Command::Seek((1.4 * f64::from(SR)) as u64));
        r.handle(Command::Play);
        let mut buf = vec![0f32; 1024];
        let mut energy = 0.0;
        for _ in 0..60 {
            r.render(&mut buf, 2);
            energy += buf.iter().map(|v| v * v).sum::<f32>();
        }
        Some(energy)
    };
    let (Some(a), Some(b)) = (level(&with_ramp), level(&final_only)) else { return };
    assert!(a > 0.0 && (a / b - 1.0).abs() < 0.01, "ramp {a} vs final {b}");
}

#[test]
#[ignore]
fn print_soundfont_note_energy() {
    // Reference level for comparing SFZ libraries (same measurement as
    // library::network_tests): one C4 at velocity 110, 20 blocks of 1024.
    let Ok(path) = std::env::var("MIDISS_SF2") else { return };
    let sf = Arc::new(SoundFont::new(&mut BufReader::new(File::open(path).unwrap())).unwrap());
    let mut s = new_synth(&sf, SR).unwrap();
    for (program, key, ch) in [(0u8, 60, 0), (33, 60, 1), (66, 60, 2), (16, 60, 3), (25, 60, 4), (0, 38, 9)] {
        s.process_midi_message(ch, 0xC0, i32::from(program), 0);
        s.note_on(ch, key, 110);
        let mut energy = 0.0;
        for _ in 0..20 {
            let (mut l, mut r) = (vec![0f32; 1024], vec![0f32; 1024]);
            s.render(&mut l, &mut r);
            energy += l.iter().chain(&r).map(|v| v * v).sum::<f32>();
        }
        s.note_off_all(true);
        eprintln!("program {program} ch {ch}: energy {energy:.3}");
    }
}

#[test]
#[ignore]
fn print_soundfont_reference_levels() {
    // Same measurement as instruments::sfz::bench (40 blocks).
    let Ok(path) = std::env::var("MIDISS_SF2") else { return };
    let sf = Arc::new(SoundFont::new(&mut BufReader::new(File::open(path).unwrap())).unwrap());
    let mut s = new_synth(&sf, SR).unwrap();
    let cases: [(&str, u8, &[i32], i32); 7] = [
        ("strings 48 chord", 48, &[48, 60, 67, 76], 100),
        ("piano 0 C4", 0, &[60], 110),
        ("trumpet 56", 56, &[67], 110),
        ("flute 73", 73, &[72], 110),
        ("horn 60", 60, &[60], 110),
        ("timpani 47", 47, &[48], 110),
        ("harp 46", 46, &[60], 110),
    ];
    for (i, (name, program, keys, vel)) in cases.iter().enumerate() {
        let ch = i as i32;
        s.process_midi_message(ch, 0xC0, i32::from(*program), 0);
        for &k in *keys {
            s.note_on(ch, k, *vel);
        }
        let (mut e, mut peak) = (0.0, 0f32);
        for _ in 0..40 {
            let (mut l, mut r) = (vec![0f32; 1024], vec![0f32; 1024]);
            s.render(&mut l, &mut r);
            e += l.iter().chain(&r).map(|v| v * v).sum::<f32>();
            peak = l.iter().chain(&r).fold(peak, |m, v| m.max(v.abs()));
        }
        s.note_off_all(true);
        eprintln!("{name}: energy {e:.1} peak {peak:.3}");
    }
}

/// test.mid-like load with the installed VSCO library: 50 tracks, 20 of them
/// string sections (4 sfizz each), the rest single VSCO instruments.
/// `cargo test [--release] -- --ignored orchestra_load --nocapture`
#[test]
#[ignore]
fn orchestra_load() {
    use crate::instruments::sfz::{SfzInstrument, SfzSplit};
    let vsco = std::path::PathBuf::from(std::env::var("HOME").unwrap())
        .join("Library/Application Support/com.midiss.app/libraries/vsco2-ce");
    if !vsco.exists() {
        return;
    }
    let Some((mut r, _t)) = renderer(SpatialConfig::default()) else { return };
    let tracks: Vec<TrackData> = (0..50).map(|i| TrackData { channel: (i % 15) as u8, program: 48 }).collect();
    let mut notes = Vec::new();
    for t in 0..50u16 {
        for step in 0..20 {
            let start = f64::from(step) * 0.5;
            for k in [48u8, 55, 60, 64] {
                notes.push(NoteData { track: t, key: k + (t % 12) as u8, velocity: 90, start, end: start + 0.45 });
            }
        }
    }
    let song = SongData { duration: 10.0, tracks, notes, channel_events: vec![] };
    r.handle(Command::LoadSong(Box::new(load_song(&song, SR))));
    let singles = ["FluteSusVib.sfz", "OboeSusVib.sfz", "ClarinetSus.sfz", "BassoonSus.sfz", "FHornSus.sfz", "TrumpetSusVib.sfz", "TromboneSus.sfz", "TubaSus.sfz", "Timpani.sfz", "Harp.sfz"];
    let t0 = std::time::Instant::now();
    for t in 0..50usize {
        let inst: Box<dyn Instrument> = if t < 20 {
            let parts = [
                (0u8, 38u8, vsco.join("ContrabassSusVB.sfz")),
                (39, 54, vsco.join("CelloEnsSusVib.sfz")),
                (55, 61, vsco.join("ViolaEnsSusVib.sfz")),
                (62, 127, vsco.join("ViolinEnsSusVib.sfz")),
            ];
            Box::new(SfzSplit::load(&parts, SR).unwrap())
        } else {
            Box::new(SfzInstrument::load(&vsco.join(singles[t % singles.len()]), SR).unwrap())
        };
        r.handle(Command::SetSource(t, Source::External(inst)));
    }
    eprintln!("loaded 110 sfizz instances in {:?}", t0.elapsed());
    let _ = r.shared.stats.take();
    let t0 = std::time::Instant::now();
    let out = play_loaded(&mut r, 0.5);
    let secs = out[0].len() as f64 / f64::from(SR);
    let s = r.shared.stats.take();
    eprintln!(
        "{secs:.1}s audio in {:.2}s ({:.1}x realtime) | load avg {:.0}% max {:.0}% overruns {} | peak {:+.1} dBFS clipped {}",
        t0.elapsed().as_secs_f64(),
        secs / t0.elapsed().as_secs_f64(),
        s.load_avg * 100.0,
        s.load_max * 100.0,
        s.overruns,
        s.peak_db,
        s.clipped
    );
}

/// Loudness of a sustained, low-heavy passage (SoundFont strings + bass)
/// in each room, relative to "original". Reveals rooms whose reverb builds
/// up beyond the loudness compensation.
#[test]
#[ignore]
fn room_loudness() {
    let mut rooms = vec!["original".to_string(), "dry".to_string()];
    rooms.extend(room::VENUES.iter().map(|v| v.id.to_string()));
    let song = SongData {
        duration: 8.0,
        tracks: vec![
            TrackData { channel: 0, program: 48 },
            TrackData { channel: 1, program: 32 },
            TrackData { channel: 2, program: 61 },
        ],
        notes: (0..8)
            .flat_map(|i| {
                let t = f64::from(i);
                vec![
                    NoteData { track: 0, key: 48, velocity: 110, start: t, end: t + 0.95 },
                    NoteData { track: 0, key: 55, velocity: 110, start: t, end: t + 0.95 },
                    NoteData { track: 0, key: 64, velocity: 110, start: t, end: t + 0.95 },
                    NoteData { track: 1, key: 36, velocity: 120, start: t, end: t + 0.5 },
                    NoteData { track: 2, key: 60, velocity: 110, start: t, end: t + 0.9 },
                ]
            })
            .collect(),
        channel_events: vec![],
    };
    for output in [OutputMode::Speakers, OutputMode::Headphones] {
        for room_id in &rooms {
            for seat in [1usize, 2] {
                let Some((mut r, _t)) = renderer(config(room_id, seat, output)) else { return };
                let out = render_song(&mut r, &song, 1.0);
                let tail = window(&out[0], 5.0, 8.0);
                let rms_db = 20.0 * rms(tail).max(1e-9).log10();
                let peak = out.iter().flatten().fold(0f32, |m, v| m.max(v.abs()));
                eprintln!(
                    "LOUD {output:?} {room_id:>9} seat {seat}: rms {rms_db:+.1} dB  peak {:+.1} dBFS",
                    20.0 * peak.log10()
                );
            }
        }
    }
}

#[test]
fn limiter_prevents_clipping() {
    // 16 tracks of loud chords at full volume: far beyond full scale.
    let Some((mut r, _t)) = renderer(config("cathedral", 2, OutputMode::Speakers)) else { return };
    r.handle(Command::MasterVolume(1.0));
    let tracks = (0..16).map(|i| TrackData { channel: i as u8 % 9, program: 48 }).collect();
    let notes = (0..16u16)
        .flat_map(|t| [36u8, 48, 55, 60, 64, 67].map(|k| note(t, k, 0.2, 2.5)))
        .map(|mut n| {
            n.velocity = 127;
            n
        })
        .collect();
    let song = SongData { duration: 3.0, tracks, notes, channel_events: vec![] };
    let out = render_song(&mut r, &song, 1.0);
    let peak = out.iter().flatten().fold(0f32, |m, v| m.max(v.abs()));
    let s = r.shared.stats.take();
    eprintln!("peak {peak:.3}, limiter -{:.1} dB, clipped {}", s.limited_db, s.clipped);
    assert!(peak <= LIMIT_CEILING + 1e-3, "peak {peak}");
    assert_eq!(s.clipped, 0);
    assert!(s.limited_db > 3.0, "limiter should have engaged");
}

/// Switch to a venue while playing (as the UI does) and check the output
/// stays finite and audible.
#[test]
fn switching_room_while_playing_keeps_sound() {
    for output in [OutputMode::Speakers, OutputMode::Headphones] {
        let Some((mut r, _t)) = renderer(SpatialConfig::default()) else { return };
        let song = SongData {
            duration: 6.0,
            tracks: vec![TrackData { channel: 0, program: 48 }, TrackData { channel: 9, program: 0 }],
            notes: (0..12)
                .flat_map(|i| {
                    let t = f64::from(i) * 0.5;
                    [note(0, 60, t, t + 0.45), note(0, 64, t, t + 0.45), note(1, 36, t, t + 0.1)]
                })
                .collect(),
            channel_events: vec![],
        };
        r.handle(Command::LoadSong(Box::new(load_song(&song, SR))));
        r.handle(Command::Play);
        let mut buf = vec![0f32; 1024];
        let mut chunk_rms = Vec::new();
        for i in 0..240 {
            if i == 60 {
                let h = r.spatial_hrtf();
                let scene = Scene::build(config("cathedral", 1, output), SR, BLOCK, &h).unwrap();
                r.handle(Command::SetScene(Box::new(scene)));
            }
            r.render(&mut buf, 2);
            assert!(buf.iter().all(|v| v.is_finite()), "{output:?}: non-finite output at chunk {i}");
            chunk_rms.push(rms(&buf));
        }
        let before = chunk_rms[20..60].iter().sum::<f32>() / 40.0;
        let after = chunk_rms[80..200].iter().sum::<f32>() / 120.0;
        eprintln!("{output:?}: rms before {before:.4} after {after:.4}, limiter gain {:.3}", r.limiter_gain);
        assert!(after > 0.3 * before, "{output:?}: sound dropped after switching ({before} -> {after})");
    }
}

/// Same as above with VSCO (SFZ) instruments, as in the app.
#[test]
#[ignore]
fn switching_room_with_sfz_keeps_sound() {
    use crate::instruments::sfz::{SfzInstrument, SfzSplit};
    let vsco = std::path::PathBuf::from(std::env::var("HOME").unwrap())
        .join("Library/Application Support/com.midiss.app/libraries/vsco2-ce");
    if !vsco.exists() {
        return;
    }
    for output in [OutputMode::Speakers, OutputMode::Headphones] {
        let Some((mut r, _t)) = renderer(SpatialConfig::default()) else { return };
        let song = SongData {
            duration: 8.0,
            tracks: vec![TrackData { channel: 0, program: 48 }, TrackData { channel: 1, program: 73 }],
            notes: (0..16)
                .flat_map(|i| {
                    let t = f64::from(i) * 0.5;
                    [note(0, 48, t, t + 0.45), note(0, 64, t, t + 0.45), note(1, 72, t, t + 0.45)]
                })
                .collect(),
            channel_events: vec![cc(0.0, 0, 0xB0, 7, 100), cc(0.0, 1, 0xB0, 7, 100)],
        };
        r.handle(Command::LoadSong(Box::new(load_song(&song, SR))));
        let parts = [
            (0u8, 38u8, vsco.join("ContrabassSusVB.sfz")),
            (39, 54, vsco.join("CelloEnsSusVib.sfz")),
            (55, 61, vsco.join("ViolaEnsSusVib.sfz")),
            (62, 127, vsco.join("ViolinEnsSusVib.sfz")),
        ];
        r.handle(Command::SetSource(0, Source::External(Box::new(SfzSplit::load(&parts, SR).unwrap()))));
        r.handle(Command::SetSource(1, Source::External(Box::new(SfzInstrument::load(&vsco.join("FluteSusVib.sfz"), SR).unwrap()))));
        r.handle(Command::Play);
        let mut buf = vec![0f32; 1024];
        let mut chunk_rms = Vec::new();
        for i in 0..300 {
            if i == 80 {
                let h = r.spatial_hrtf();
                let scene = Scene::build(config("cathedral", 1, output), SR, BLOCK, &h).unwrap();
                r.handle(Command::SetScene(Box::new(scene)));
            }
            r.render(&mut buf, 2);
            assert!(buf.iter().all(|v| v.is_finite()), "{output:?}: non-finite at chunk {i}");
            chunk_rms.push(rms(&buf));
        }
        let before = chunk_rms[30..80].iter().sum::<f32>() / 50.0;
        let after = chunk_rms[100..250].iter().sum::<f32>() / 150.0;
        eprintln!("SFZ {output:?}: rms before {before:.4} after {after:.4}, limiter gain {:.3}", r.limiter_gain);
        assert!(after > 0.3 * before, "{output:?}: sound dropped ({before} -> {after})");
    }
}


/// The app's path: render thread → ring buffer → device pulls.
#[test]
fn render_thread_and_ring_buffer() {
    let Some((r, _t)) = renderer(SpatialConfig::default()) else { return };
    let shared = r.shared.clone();
    let (tx, rx) = channel();
    let (producer, mut consumer) = rtrb::RingBuffer::new(BLOCK * 2 * RING_BLOCKS);
    let thread = std::thread::spawn(move || render_loop(r, rx, producer));

    let song = SongData {
        duration: 4.0,
        tracks: vec![TrackData { channel: 0, program: 0 }],
        notes: (0..8).map(|i| note(0, 60, f64::from(i) * 0.5, f64::from(i) * 0.5 + 0.4)).collect(),
        channel_events: vec![],
    };
    tx.send(Command::LoadSong(Box::new(load_song(&song, SR)))).unwrap();
    tx.send(Command::Play).unwrap();

    // Pull like a device with 512-frame buffers, pacing only on availability.
    let pull_seconds = |consumer: &mut rtrb::Consumer<f32>, secs: f64| -> f32 {
        let mut buf = vec![0f32; 512 * 2];
        let mut energy = 0.0;
        let mut pulled = 0usize;
        while (pulled as f64) < secs * f64::from(SR) {
            while consumer.slots() < 1024 {
                std::thread::sleep(Duration::from_millis(1));
            }
            pull(consumer, &shared, &mut buf, 2);
            energy += buf.iter().map(|v| v * v).sum::<f32>();
            pulled += 512;
        }
        energy
    };
    let e = pull_seconds(&mut consumer, 1.0);
    assert!(e > 1.0, "audio flows through the ring ({e})");
    let pos = shared.audible_position() as f64 / f64::from(SR);
    assert!((0.8..1.2).contains(&pos), "audible position {pos}");

    // Seek: stale queued audio is skipped and the position jumps at once.
    tx.send(Command::Seek((3.0 * f64::from(SR)) as u64)).unwrap();
    std::thread::sleep(Duration::from_millis(20));
    pull_seconds(&mut consumer, 0.1);
    let pos = shared.audible_position() as f64 / f64::from(SR);
    assert!((2.9..3.3).contains(&pos), "position after seek {pos}");
    assert_eq!(shared.stats.overruns.load(Ordering::Relaxed), 0, "no dropouts when pulled after data is ready");

    drop(tx);
    thread.join().unwrap();
}
