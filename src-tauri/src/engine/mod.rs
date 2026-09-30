//! Native playback engine: a sample-accurate MIDI sequencer driving one
//! instrument per track (the SoundFont synthesizer, SFZ, …), followed by the
//! spatial stage (venue reverb, binaural), rendered to the OS audio device via
//! cpal.
//!
//! The audio thread owns all playback state. Tauri commands never lock it:
//! they send [`Command`]s over a channel (heavy objects — synthesizer, song,
//! instruments, room — are built beforehand on other threads) and read status
//! from atomics. Replaced objects are dropped on a janitor thread, so the
//! audio callback never blocks or frees large allocations.
//!
//! The song arrives from the frontend already parsed (see `rustEngine.ts`), so
//! track indices match the UI exactly. Holding the song here is also the
//! foundation for editing it from outside the UI later (MCP).

pub mod player;
pub mod song;
#[cfg(test)]
mod tests;

use std::any::Any;
use std::fs::File;
use std::io::BufReader;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};
use rustysynth::{SoundFont, Synthesizer, SynthesizerSettings};
use serde::Serialize;
use tauri::ipc::{InvokeBody, Request};
use tauri::{AppHandle, State};

use crate::asset_cache;
use crate::instruments::{
    sfz::{SfzInstrument, SfzSplit},
    InstrumentChoice,
};
use crate::spatial::hrtf::Hrtf;
use crate::spatial::{dsp::Fft, room, Scene, SpatialConfig, SpatialProcessor, TrackBuffers};
use player::{Player, Source, Track, MAX_TRACKS};
use song::Song;

/// Orchestral files routinely exceed rustysynth's default of 64 voices.
const MAX_POLYPHONY: usize = 256;
/// Headroom so dense arrangements plus reverb rarely need limiting.
const OUTPUT_GAIN: f32 = 0.8;
/// Limiter ceiling (≈ -0.9 dBFS): peaks are turned down smoothly before
/// they reach it instead of being clipped.
const LIMIT_CEILING: f32 = 0.9;
/// How fast the limiter lets go again, per block (~1 dB per 100 ms).
const LIMIT_RELEASE: f32 = 1.025;
/// Engine block in frames. Also the added output latency (~21 ms at 48 kHz),
/// irrelevant for a player; large blocks make long convolutions cheap.
pub const BLOCK: usize = 1024;

/* ---------- renderer: everything the audio callback runs ---------- */

pub struct LoadedSong {
    song: Box<Song>,
    tracks: Vec<Track>,
}

pub enum Command {
    SetSynth(Box<Synthesizer>),
    LoadSong(Box<LoadedSong>),
    Play,
    Pause,
    Stop,
    Seek(u64),
    MasterVolume(f32),
    Mute(usize, bool),
    Gain(usize, f32),
    Program(usize, u8),
    SetSource(usize, Source),
    SetScene(Box<Scene>),
}

/// Status published by the render thread and the device callback.
#[derive(Default)]
pub struct Shared {
    /// Song position (samples) of the audio at the ring buffer's write head.
    position: AtomicU64,
    playing: AtomicBool,
    ended: AtomicBool,
    /// Frames pushed into / pulled out of the ring buffer.
    written: AtomicU64,
    read: AtomicU64,
    /// Frames before this write index are stale (after a seek/stop) and are
    /// dropped by the device callback instead of played.
    flush_to: AtomicU64,
    stats: Stats,
    /// Latest two-second summary, for the UI.
    last_stats: Mutex<StatsSnapshot>,
}

impl Shared {
    /// Song position currently coming out of the speakers.
    fn audible_position(&self) -> u64 {
        let buffered = self.written.load(Ordering::Acquire).saturating_sub(self.read.load(Ordering::Acquire));
        self.position.load(Ordering::Acquire).saturating_sub(buffered)
    }
}

/// Rendering health since the last read (see [`Stats::take`]).
#[derive(Default)]
pub struct Stats {
    /// Sum of per-block load (render time / block duration), ×1000.
    load_sum: AtomicU64,
    blocks: AtomicU64,
    /// Worst block load, ×1000.
    load_max: AtomicU32,
    /// Device callbacks that found the ring buffer short (audible dropouts).
    overruns: AtomicU32,
    /// Blocks in which an instrument or the room produced NaN/inf samples
    /// (muted and the room reset); the last offending track is kept.
    invalid: AtomicU32,
    invalid_track: AtomicU32,
    /// Peak output level before the soft clipper (f32 bits).
    peak: AtomicU32,
    /// Samples the soft clipper had to bend.
    clipped: AtomicU64,
    /// Deepest limiter gain reduction (f32 bits, dB as a positive number).
    limited_db: AtomicU32,
    /// Most tracks producing sound in one block.
    active_tracks: AtomicU32,
    /// Peak of the instruments' mix before the room (f32 bits).
    input_peak: AtomicU32,
}

#[derive(Serialize, Clone, Copy, Default)]
#[serde(rename_all = "camelCase")]
pub struct StatsSnapshot {
    /// Average and worst audio-thread load, 1.0 = all of the available time.
    load_avg: f32,
    load_max: f32,
    overruns: u32,
    /// Peak level in dBFS before soft clipping.
    peak_db: f32,
    clipped: u64,
    /// Deepest limiter gain reduction in dB (0 = untouched).
    limited_db: f32,
    active_tracks: u32,
    /// Peak of the instruments' mix before the room, dBFS.
    input_peak_db: f32,
    invalid: u32,
    invalid_track: u32,
}

impl Stats {
    fn record_input(&self, active_tracks: u32, input_peak: f32) {
        self.active_tracks.fetch_max(active_tracks, Ordering::Relaxed);
        self.input_peak.fetch_max(input_peak.to_bits(), Ordering::Relaxed);
    }

    fn record(&self, load: f32, peak: f32, clipped: u64, limited_db: f32) {
        self.limited_db.fetch_max(limited_db.max(0.0).to_bits(), Ordering::Relaxed);
        let l = (load * 1000.0) as u64;
        self.load_sum.fetch_add(l, Ordering::Relaxed);
        self.blocks.fetch_add(1, Ordering::Relaxed);
        self.load_max.fetch_max(l as u32, Ordering::Relaxed);
        // Positive floats order like their bit patterns.
        self.peak.fetch_max(peak.to_bits(), Ordering::Relaxed);
        self.clipped.fetch_add(clipped, Ordering::Relaxed);
    }

    /// Read and reset.
    pub fn take(&self) -> StatsSnapshot {
        let blocks = self.blocks.swap(0, Ordering::Relaxed).max(1);
        let peak = f32::from_bits(self.peak.swap(0, Ordering::Relaxed));
        StatsSnapshot {
            load_avg: self.load_sum.swap(0, Ordering::Relaxed) as f32 / 1000.0 / blocks as f32,
            load_max: self.load_max.swap(0, Ordering::Relaxed) as f32 / 1000.0,
            overruns: self.overruns.swap(0, Ordering::Relaxed),
            peak_db: 20.0 * peak.max(1e-6).log10(),
            clipped: self.clipped.swap(0, Ordering::Relaxed),
            limited_db: f32::from_bits(self.limited_db.swap(0, Ordering::Relaxed)),
            active_tracks: self.active_tracks.swap(0, Ordering::Relaxed),
            input_peak_db: 20.0 * f32::from_bits(self.input_peak.swap(0, Ordering::Relaxed)).max(1e-6).log10(),
            invalid: self.invalid.swap(0, Ordering::Relaxed),
            invalid_track: self.invalid_track.load(Ordering::Relaxed),
        }
    }
}

pub struct Renderer {
    player: Player,
    spatial: SpatialProcessor,
    buffers: TrackBuffers,
    /// Block being output (already limited).
    out: [Vec<f32>; 2],
    /// Next block, rendered one block ahead so the limiter can see it coming.
    ahead: [Vec<f32>; 2],
    limiter_gain: f32,
    /// Read position into `out` for [`Renderer::render`] (tests).
    #[cfg(test)]
    read: usize,
    master_volume: f32,
    /// Real-time duration of one engine block.
    block_seconds: f32,
    shared: Arc<Shared>,
    trash: Sender<Box<dyn Any + Send>>,
}

impl Renderer {
    fn new(
        sample_rate: u32,
        hrtf: Arc<Hrtf>,
        scene: Box<Scene>,
        shared: Arc<Shared>,
        trash: Sender<Box<dyn Any + Send>>,
    ) -> Self {
        Self {
            player: Player::new(),
            spatial: SpatialProcessor::new(BLOCK, MAX_TRACKS, hrtf, scene),
            buffers: TrackBuffers::new(BLOCK, MAX_TRACKS),
            out: [vec![0.0; BLOCK], vec![0.0; BLOCK]],
            ahead: [vec![0.0; BLOCK], vec![0.0; BLOCK]],
            limiter_gain: 1.0,
            #[cfg(test)]
            read: BLOCK,
            master_volume: 0.9,
            block_seconds: BLOCK as f32 / sample_rate as f32,
            shared,
            trash,
        }
    }

    #[cfg(test)]
    fn spatial_hrtf(&self) -> Arc<Hrtf> {
        self.spatial.hrtf()
    }

    fn discard<T: Any + Send>(&self, value: T) {
        // If the janitor is gone we are shutting down; dropping here is fine.
        let _ = self.trash.send(Box::new(value));
    }

    fn handle(&mut self, cmd: Command) {
        let p = &mut self.player;
        match cmd {
            Command::SetSynth(mut synth) => {
                synth.set_reverb_enabled(self.spatial.config().is_original());
                if let Some(old) = p.set_synth(synth) {
                    self.discard(old);
                }
            }
            Command::LoadSong(loaded) => {
                let LoadedSong { song, tracks } = *loaded;
                let old = p.load(song, tracks);
                self.discard(old);
                self.spatial.reset();
                self.flush();
            }
            Command::Play => {
                if p.ended || p.position >= p.song.duration {
                    p.seek(0);
                }
                p.playing = true;
            }
            Command::Pause => {
                p.playing = false;
                p.release_all();
                self.flush();
            }
            Command::Stop => {
                p.playing = false;
                p.release_all();
                p.seek(0);
                self.flush();
            }
            Command::Seek(target) => {
                p.seek(target);
                self.spatial.reset();
                self.flush();
            }
            Command::MasterVolume(v) => self.master_volume = v.clamp(0.0, 1.0),
            Command::Mute(track, muted) => p.set_muted(track, muted),
            Command::Gain(track, gain) => p.set_gain(track, gain),
            Command::Program(track, program) => p.set_program(track, program),
            Command::SetSource(track, source) => {
                if let Some(old) = p.set_source(track, source) {
                    self.discard(old);
                }
            }
            Command::SetScene(scene) => {
                if let Some(synth) = p.synth.as_mut() {
                    synth.set_reverb_enabled(scene.config.is_original());
                }
                let old = self.spatial.set_scene(scene);
                self.discard(old);
            }
        }
        self.publish();
    }

    /// Drop audio rendered before a seek/stop/pause: whatever is queued in
    /// the ring buffer, and the limiter's look-ahead block.
    fn flush(&mut self) {
        let s = &self.shared;
        s.flush_to.store(s.written.load(Ordering::Relaxed), Ordering::Release);
        for b in self.out.iter_mut().chain(self.ahead.iter_mut()) {
            b.fill(0.0);
        }
    }

    fn publish(&self) {
        let p = &self.player;
        // `out` (the block about to be queued) lags the sequencer by the
        // look-ahead block.
        self.shared.position.store(p.position.saturating_sub(BLOCK as u64), Ordering::Release);
        self.shared.playing.store(p.playing, Ordering::Relaxed);
        self.shared.ended.store(p.ended, Ordering::Relaxed);
    }

    fn next_block(&mut self) {
        let started = Instant::now();
        // The previous look-ahead block becomes the one to output, and a new
        // block is rendered ahead of it.
        std::mem::swap(&mut self.out, &mut self.ahead);
        self.player.render(&mut self.buffers);
        self.sanitize_input();
        let b = &self.buffers;
        let active = b.active[..b.count].iter().filter(|a| **a).count() as u32;
        let input_peak = b.mix.iter().flatten().fold(0f32, |m, v| m.max(v.abs()));
        self.shared.stats.record_input(active, input_peak);
        self.spatial.process(&self.buffers, &mut self.ahead);
        if self.ahead.iter().flatten().any(|v| !v.is_finite()) {
            // Never let NaN/inf reach the reverb history or the speakers.
            for b in &mut self.ahead {
                b.fill(0.0);
            }
            self.spatial.reset();
            self.shared.stats.invalid.fetch_add(1, Ordering::Relaxed);
        }
        let gain = self.master_volume * OUTPUT_GAIN;
        for v in self.ahead.iter_mut().flatten() {
            *v *= gain;
        }

        // Look-ahead limiter: over this block, ramp the gain to what both this
        // block and the next need. The gain at the start already respects this
        // block (it was the previous ramp's target), and a linear ramp between
        // two values that both do stays within it — so no sample passes the
        // ceiling, and nothing is clipped.
        let block_peak = |b: &[Vec<f32>; 2]| b.iter().flatten().fold(0f32, |m, v| m.max(v.abs()));
        let need = |peak: f32| if peak > LIMIT_CEILING { LIMIT_CEILING / peak } else { 1.0 };
        let target = need(block_peak(&self.out)).min(need(block_peak(&self.ahead)));
        let target = target.min(self.limiter_gain * LIMIT_RELEASE);
        let start = self.limiter_gain;
        let step = (target - start) / BLOCK as f32;
        let (mut peak, mut clipped) = (0f32, 0u64);
        for ch in &mut self.out {
            for (i, v) in ch.iter_mut().enumerate() {
                let x = *v * (start + step * (i + 1) as f32);
                let a = x.abs();
                peak = peak.max(a);
                if a > SOFT_CLIP_KNEE {
                    clipped += 1;
                }
                // Safety net only; the limiter keeps levels below the knee.
                *v = soft_clip(x);
            }
        }
        self.limiter_gain = target;
        #[cfg(test)]
        {
            self.read = 0;
        }
        let load = started.elapsed().as_secs_f32() / self.block_seconds;
        let limited_db = -20.0 * start.min(target).log10();
        self.shared.stats.record(load, peak, clipped, limited_db);
    }

    /// Mute any track (and the mix) that produced NaN/inf this block.
    fn sanitize_input(&mut self) {
        let b = &mut self.buffers;
        let mut bad = false;
        for t in 0..b.count {
            if b.active[t] && b.tracks[t].iter().flatten().any(|v| !v.is_finite()) {
                for ch in &mut b.tracks[t] {
                    ch.fill(0.0);
                }
                self.shared.stats.invalid_track.store(t as u32, Ordering::Relaxed);
                bad = true;
            }
        }
        if bad || b.mix.iter().chain(b.fx.iter()).flatten().any(|v| !v.is_finite()) {
            for ch in b.mix.iter_mut().chain(b.fx.iter_mut()) {
                for v in ch.iter_mut() {
                    if !v.is_finite() {
                        *v = 0.0;
                    }
                }
            }
            self.shared.stats.invalid.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Render one block and queue it for the device (render thread).
    fn produce(&mut self, ring: &mut rtrb::Producer<f32>) {
        self.next_block();
        if let Ok(mut chunk) = ring.write_chunk(BLOCK * 2) {
            let (a, b) = chunk.as_mut_slices();
            for (i, slot) in a.iter_mut().chain(b.iter_mut()).enumerate() {
                *slot = self.out[i & 1][i >> 1];
            }
            chunk.commit_all();
        }
        self.shared.written.fetch_add(BLOCK as u64, Ordering::Release);
        self.publish();
    }

    /// Render straight into an interleaved buffer (tests; the app goes
    /// through the render thread and ring buffer instead).
    #[cfg(test)]
    fn render<T: SizedSample + FromSample<f32>>(&mut self, data: &mut [T], channels: usize) {
        for frame in data.chunks_mut(channels.max(1)) {
            if self.read >= BLOCK {
                self.next_block();
            }
            let (l, r) = (self.out[0][self.read], self.out[1][self.read]);
            self.read += 1;
            for (c, sample) in frame.iter_mut().enumerate() {
                *sample = T::from_sample(match c {
                    0 => l,
                    1 => r,
                    _ => 0.0,
                });
            }
        }
        self.publish();
    }

}

const SOFT_CLIP_KNEE: f32 = 0.95;

/// Transparent below -0.45 dBFS, then a smooth knee that never exceeds ±1.
fn soft_clip(x: f32) -> f32 {
    const KNEE: f32 = SOFT_CLIP_KNEE;
    let a = x.abs();
    if a <= KNEE {
        x
    } else {
        let over = (a - KNEE) / (1.0 - KNEE);
        (KNEE + (1.0 - KNEE) * over.tanh()).copysign(x)
    }
}

fn new_synth(sound_font: &Arc<SoundFont>, sample_rate: u32) -> Result<Synthesizer, String> {
    let mut settings = SynthesizerSettings::new(sample_rate as i32);
    settings.maximum_polyphony = MAX_POLYPHONY;
    // One synth channel per track.
    settings.channel_count = MAX_TRACKS;
    Synthesizer::new(sound_font, &settings).map_err(|e| e.to_string())
}

fn load_song(song: &song::SongData, sample_rate: u32) -> LoadedSong {
    let song = song::build_song(song, sample_rate, MAX_TRACKS);
    let tracks = song.tracks.iter().map(|t| Track::new(t.midi_channel, t.program)).collect();
    LoadedSong { song: Box::new(song), tracks }
}

/* ---------- audio device ---------- */

pub struct EngineHandle {
    commands: Mutex<Sender<Command>>,
    shared: Arc<Shared>,
    sample_rate: u32,
    hrtf: Arc<Hrtf>,
    spatial: Mutex<SpatialConfig>,
    /// Latest requested room; stale builds that finish late are discarded.
    spatial_generation: AtomicU64,
    /// Bumped on every song load, so instruments that finish loading after
    /// the song changed are not installed into the wrong track.
    song_generation: AtomicU64,
    /// MIDI channel of each track of the current song.
    track_channels: Mutex<Vec<u8>>,
    /// Plugins per track, so their editors can be opened.
    #[cfg(target_os = "macos")]
    audio_units: Mutex<HashMap<usize, crate::instruments::audio_unit::SharedAudioUnit>>,
}

impl EngineHandle {
    pub fn send(&self, cmd: Command) -> Result<(), String> {
        self.commands
            .lock()
            .map_err(|_| "engine poisoned")?
            .send(cmd)
            .map_err(|_| "audio thread stopped".to_string())
    }

    fn to_samples(&self, seconds: f64) -> u64 {
        (seconds.max(0.0) * f64::from(self.sample_rate)).round() as u64
    }
}

/// Blocks queued ahead of the device (~85 ms at 48 kHz). Absorbs render-time
/// spikes; a player doesn't mind the latency.
const RING_BLOCKS: usize = 4;

/// The device callback only copies queued audio out of the ring buffer; all
/// rendering happens on the render thread.
fn run_stream<T: SizedSample + FromSample<f32>>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    mut ring: rtrb::Consumer<f32>,
    shared: Arc<Shared>,
) -> Result<cpal::Stream, String> {
    let channels = usize::from(config.channels).max(1);
    let logged = AtomicBool::new(false);
    let stream = device
        .build_output_stream(
            config,
            move |data: &mut [T], _| {
                if !logged.swap(true, Ordering::Relaxed) {
                    eprintln!("[engine] device buffer {} frames", data.len() / channels);
                }
                pull(&mut ring, &shared, data, channels);
            },
            |err| eprintln!("[engine] audio stream error: {err}"),
            None,
        )
        .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    Ok(stream)
}

/// Device side: copy queued audio into an interleaved device buffer,
/// skipping audio made stale by a seek/stop; count a dropout when short.
fn pull<T: SizedSample + FromSample<f32>>(ring: &mut rtrb::Consumer<f32>, shared: &Shared, data: &mut [T], channels: usize) {
    let frames = data.len() / channels;
    let mut read = shared.read.load(Ordering::Relaxed);
    let flush_to = shared.flush_to.load(Ordering::Acquire);
    if read < flush_to {
        let skip = ((flush_to - read) as usize).min(ring.slots() / 2);
        if let Ok(chunk) = ring.read_chunk(skip * 2) {
            chunk.commit_all();
        }
        read += skip as u64;
    }
    let n = frames.min(ring.slots() / 2);
    if let Ok(chunk) = ring.read_chunk(n * 2) {
        let (a, b) = chunk.as_slices();
        let mut samples = a.iter().chain(b.iter());
        for frame in data[..n * channels].chunks_mut(channels) {
            let l = *samples.next().unwrap_or(&0.0);
            let r = *samples.next().unwrap_or(&0.0);
            for (c, out) in frame.iter_mut().enumerate() {
                *out = T::from_sample(match c {
                    0 => l,
                    1 => r,
                    _ => 0.0,
                });
            }
        }
        chunk.commit_all();
    }
    data[n * channels..].fill(T::EQUILIBRIUM);
    // Right after a flush the ring is legitimately empty until the render
    // thread catches up; that gap is not a dropout.
    let refilling = shared.written.load(Ordering::Acquire) <= flush_to;
    if n < frames && !refilling && shared.playing.load(Ordering::Relaxed) {
        shared.stats.overruns.fetch_add(1, Ordering::Relaxed);
    }
    shared.read.store(read + n as u64, Ordering::Release);
}

/// Keep the ring buffer full, applying commands between blocks.
fn render_loop(mut renderer: Renderer, commands: Receiver<Command>, mut ring: rtrb::Producer<f32>) {
    #[cfg(target_os = "macos")]
    // SAFETY: plain syscall on the current thread.
    unsafe {
        libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_USER_INTERACTIVE, 0);
    }
    loop {
        loop {
            match commands.try_recv() {
                Ok(cmd) => renderer.handle(cmd),
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => return,
            }
        }
        if ring.slots() >= BLOCK * 2 {
            renderer.produce(&mut ring);
            continue;
        }
        // Full: wait for room, waking early for commands.
        match commands.recv_timeout(Duration::from_millis(2)) {
            Ok(cmd) => renderer.handle(cmd),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// Open the default output device on a dedicated thread (cpal streams are not
/// `Send` on every platform) and keep it alive for the app's lifetime.
pub fn start() -> Result<EngineHandle, String> {
    let (trash_tx, trash_rx) = channel::<Box<dyn Any + Send>>();
    std::thread::Builder::new()
        .name("otomo-janitor".into())
        .spawn(move || {
            for garbage in trash_rx {
                drop(garbage);
            }
        })
        .map_err(|e| e.to_string())?;

    let (tx, rx) = std::sync::mpsc::channel::<Result<EngineHandle, String>>();
    std::thread::Builder::new()
        .name("otomo-audio".into())
        .spawn(move || {
            let result = (|| {
                let host = cpal::default_host();
                let device = host.default_output_device().ok_or("no audio output device")?;
                let supported = device.default_output_config().map_err(|e| e.to_string())?;
                let format = supported.sample_format();
                let config: cpal::StreamConfig = supported.into();
                let sample_rate = config.sample_rate;

                let hrtf = Arc::new(Hrtf::load(sample_rate, &mut Fft::new(BLOCK)));
                let spatial = SpatialConfig::default();
                let scene = Box::new(Scene::build(spatial.clone(), sample_rate, BLOCK, &hrtf)?);
                let shared = Arc::new(Shared::default());
                let renderer = Renderer::new(sample_rate, hrtf.clone(), scene, shared.clone(), trash_tx);
                spawn_monitor(shared.clone());
                let (cmd_tx, cmd_rx) = channel();
                let (producer, consumer) = rtrb::RingBuffer::new(BLOCK * 2 * RING_BLOCKS);
                std::thread::Builder::new()
                    .name("otomo-render".into())
                    .spawn(move || render_loop(renderer, cmd_rx, producer))
                    .map_err(|e| e.to_string())?;

                let s = shared.clone();
                let stream = match format {
                    cpal::SampleFormat::F32 => run_stream::<f32>(&device, &config, consumer, s),
                    cpal::SampleFormat::I16 => run_stream::<i16>(&device, &config, consumer, s),
                    cpal::SampleFormat::I32 => run_stream::<i32>(&device, &config, consumer, s),
                    cpal::SampleFormat::U16 => run_stream::<u16>(&device, &config, consumer, s),
                    other => Err(format!("unsupported sample format {other}")),
                }?;
                let handle = EngineHandle {
                    commands: Mutex::new(cmd_tx),
                    shared,
                    sample_rate,
                    hrtf,
                    spatial: Mutex::new(spatial),
                    spatial_generation: AtomicU64::new(0),
                    song_generation: AtomicU64::new(0),
                    track_channels: Mutex::new(Vec::new()),
                    #[cfg(target_os = "macos")]
                    audio_units: Mutex::new(HashMap::new()),
                };
                Ok::<_, String>((handle, stream))
            })();
            match result {
                Ok((handle, stream)) => {
                    let _ = tx.send(Ok(handle));
                    // Keep the stream alive; parking forever is cheap.
                    let _stream = stream;
                    loop {
                        std::thread::park();
                    }
                }
                Err(e) => {
                    let _ = tx.send(Err(e));
                }
            }
        })
        .map_err(|e| e.to_string())?;
    rx.recv().map_err(|e| e.to_string())?
}

/// Log rendering health every two seconds while playing, so problems like
/// dropouts (overruns) or constant limiting (clipped) show up in the log.
fn spawn_monitor(shared: Arc<Shared>) {
    let _ = std::thread::Builder::new().name("otomo-monitor".into()).spawn(move || loop {
        std::thread::sleep(Duration::from_secs(2));
        let s = shared.stats.take();
        *shared.last_stats.lock().unwrap_or_else(|e| e.into_inner()) = s;
        if shared.playing.load(Ordering::Relaxed) {
            eprintln!(
                "[engine] load avg {:.0}% max {:.0}% | dropouts {} | tracks sounding {} | mix in {:+.1} dBFS -> out {:+.1} dBFS | limiter -{:.1} dB | clipped {}",
                s.load_avg * 100.0,
                s.load_max * 100.0,
                s.overruns,
                s.active_tracks,
                s.input_peak_db,
                s.peak_db,
                s.limited_db,
                s.clipped
            );
            if s.invalid > 0 {
                eprintln!(
                    "[engine] WARNING: invalid (NaN/inf) audio in {} blocks, muted and reset (last from track {})",
                    s.invalid, s.invalid_track
                );
            }
        }
    });
}

/* ---------- commands ---------- */

/// Load the SoundFont previously stored in the asset cache under `key`.
#[tauri::command]
pub async fn engine_load_soundfont(app: AppHandle, handle: State<'_, EngineHandle>, key: String) -> Result<(), String> {
    let path = asset_cache::cache_path(&app, &key)?;
    let sample_rate = handle.sample_rate;
    let synth = tauri::async_runtime::spawn_blocking(move || {
        let mut reader = BufReader::new(File::open(&path).map_err(|e| e.to_string())?);
        let sound_font = Arc::new(SoundFont::new(&mut reader).map_err(|e| e.to_string())?);
        new_synth(&sound_font, sample_rate)
    })
    .await
    .map_err(|e| e.to_string())??;
    handle.send(Command::SetSynth(Box::new(synth)))
}

/// Takes the song as a raw binary body (see `song::decode_song`).
#[tauri::command]
pub async fn engine_load_song(handle: State<'_, EngineHandle>, request: Request<'_>) -> Result<(), String> {
    let InvokeBody::Raw(bytes) = request.body() else {
        return Err("expected raw body".into());
    };
    let loaded = load_song(&song::decode_song(bytes)?, handle.sample_rate);
    eprintln!("[engine] song loaded: {} tracks, {} events", loaded.tracks.len(), loaded.song.events.len());
    handle.song_generation.fetch_add(1, Ordering::SeqCst);
    *handle.track_channels.lock().map_err(|_| "poisoned")? = loaded.tracks.iter().map(|t| t.midi_channel).collect();
    #[cfg(target_os = "macos")]
    handle.audio_units.lock().map_err(|_| "poisoned")?.clear();
    handle.send(Command::LoadSong(Box::new(loaded)))
}

/// Choose what a track plays through, from a thread that is not async — the
/// MCP control socket. Blocks until the instrument is live, which is what the
/// caller wants: it is about to ask for the track to be played.
pub fn assign_instrument(
    handle: &State<'_, EngineHandle>,
    track: usize,
    choice: InstrumentChoice,
) -> Result<(), String> {
    let generation = handle.song_generation.load(Ordering::SeqCst);
    let sample_rate = handle.sample_rate;
    tauri::async_runtime::block_on(set_track_instrument(handle, track, choice, generation, sample_rate))
}

/// Choose what a track plays through. SFZ files load on a worker thread;
/// resolves once the instrument is live (or failed to load).
#[tauri::command]
pub async fn engine_set_track_instrument(
    handle: State<'_, EngineHandle>,
    track: usize,
    choice: InstrumentChoice,
) -> Result<(), String> {
    let generation = handle.song_generation.load(Ordering::SeqCst);
    let sample_rate = handle.sample_rate;
    let started = Instant::now();
    let label = match &choice {
        InstrumentChoice::Sf2 => "SoundFont".to_string(),
        InstrumentChoice::Sfz { path, .. } => path.rsplit('/').next().unwrap_or(path).to_string(),
        InstrumentChoice::SfzSplit { parts, .. } => format!("split of {}", parts.len()),
        InstrumentChoice::AudioUnit { id } => format!("AudioUnit {id}"),
        InstrumentChoice::Synth { preset } => format!("synth {preset}"),
    };
    let result = set_track_instrument(&handle, track, choice, generation, sample_rate).await;
    match &result {
        Ok(()) => eprintln!("[engine] track {track} -> {label} ({:.0} ms)", started.elapsed().as_secs_f64() * 1e3),
        Err(e) => eprintln!("[engine] track {track} -> {label} FAILED: {e}"),
    }
    result
}

async fn set_track_instrument(
    handle: &State<'_, EngineHandle>,
    track: usize,
    choice: InstrumentChoice,
    generation: u64,
    sample_rate: u32,
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let mut audio_unit = None;
    let source = match choice {
        InstrumentChoice::Sf2 => Source::Sf2,
        InstrumentChoice::Sfz { path, gain_db } => {
            let inst = tauri::async_runtime::spawn_blocking(move || {
                let mut inst = SfzInstrument::load(path.as_ref(), sample_rate)?;
                inst.set_gain_db(gain_db);
                Ok::<_, String>(inst)
            })
            .await
            .map_err(|e| e.to_string())??;
            Source::External(Box::new(inst))
        }
        InstrumentChoice::SfzSplit { parts, gain_db } => {
            let parts: Vec<_> = parts.into_iter().map(|p| (p.lo, p.hi, std::path::PathBuf::from(p.path))).collect();
            let inst = tauri::async_runtime::spawn_blocking(move || {
                let mut inst = SfzSplit::load(&parts, sample_rate)?;
                inst.set_gain_db(gain_db);
                Ok::<_, String>(inst)
            })
            .await
            .map_err(|e| e.to_string())??;
            Source::External(Box::new(inst))
        }
        InstrumentChoice::Synth { preset } => {
            Source::External(Box::new(crate::instruments::synth::SynthInstrument::load(&preset, sample_rate)?))
        }
        #[cfg(target_os = "macos")]
        InstrumentChoice::AudioUnit { id } => {
            use crate::instruments::audio_unit::AudioUnitInstrument;
            let channel = handle.track_channels.lock().map_err(|_| "poisoned")?.get(track).copied().unwrap_or(0);
            let inst = tauri::async_runtime::spawn_blocking(move || AudioUnitInstrument::load(&id, sample_rate, channel))
                .await
                .map_err(|e| e.to_string())??;
            eprintln!("[engine] AudioUnit loaded on track {track} (isolated in its own process: {})", inst.out_of_process);
            audio_unit = Some(inst.shared());
            Source::External(Box::new(inst))
        }
        #[cfg(not(target_os = "macos"))]
        InstrumentChoice::AudioUnit { .. } => return Err("AudioUnit is only available on macOS".into()),
    };
    if handle.song_generation.load(Ordering::SeqCst) != generation {
        return Err("song changed while loading".into());
    }
    #[cfg(target_os = "macos")]
    {
        let mut units = handle.audio_units.lock().map_err(|_| "poisoned")?;
        match audio_unit {
            Some(unit) => units.insert(track, unit),
            None => units.remove(&track),
        };
    }
    handle.send(Command::SetSource(track, source))
}

/// Installed instrument plugins.
#[tauri::command]
pub fn plugin_list() -> Vec<serde_json::Value> {
    #[cfg(target_os = "macos")]
    {
        crate::instruments::audio_unit::list_instruments()
            .into_iter()
            .filter_map(|p| serde_json::to_value(p).ok())
            .collect()
    }
    #[cfg(not(target_os = "macos"))]
    Vec::new()
}

/// Open the editor window of the plugin playing `track`.
#[tauri::command]
pub fn plugin_open_editor(app: AppHandle, handle: State<'_, EngineHandle>, track: usize, title: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let unit = handle.audio_units.lock().map_err(|_| "poisoned")?.get(&track).cloned().ok_or("no plugin on this track")?;
        app.run_on_main_thread(move || {
            if let Some(mtm) = objc2::MainThreadMarker::new() {
                crate::instruments::audio_unit::open_editor(mtm, unit, title);
            }
        })
        .map_err(|e| e.to_string())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, handle, track, title);
        Err("plugins are only available on macOS".into())
    }
}

#[tauri::command]
pub fn engine_play(handle: State<'_, EngineHandle>) -> Result<(), String> {
    handle.send(Command::Play)
}

#[tauri::command]
pub fn engine_pause(handle: State<'_, EngineHandle>) -> Result<(), String> {
    handle.send(Command::Pause)
}

#[tauri::command]
pub fn engine_stop(handle: State<'_, EngineHandle>) -> Result<(), String> {
    handle.send(Command::Stop)
}

#[tauri::command]
pub fn engine_seek(handle: State<'_, EngineHandle>, seconds: f64) -> Result<(), String> {
    handle.send(Command::Seek(handle.to_samples(seconds)))
}

#[tauri::command]
pub fn engine_set_master_volume(handle: State<'_, EngineHandle>, value: f32) -> Result<(), String> {
    handle.send(Command::MasterVolume(value))
}

#[tauri::command]
pub fn engine_set_track_muted(handle: State<'_, EngineHandle>, track: usize, muted: bool) -> Result<(), String> {
    handle.send(Command::Mute(track, muted))
}

/// SoundFont program for a track; takes effect immediately.
/// A track's fader, as a linear gain (1 = unchanged).
#[tauri::command]
pub fn engine_set_track_gain(handle: State<'_, EngineHandle>, track: usize, gain: f32) -> Result<(), String> {
    handle.send(Command::Gain(track, gain))
}

#[tauri::command]
pub fn engine_set_track_program(handle: State<'_, EngineHandle>, track: usize, program: u8) -> Result<(), String> {
    handle.send(Command::Program(track, program))
}

#[derive(Serialize)]
pub struct EngineStatus {
    position: f64,
    playing: bool,
    ended: bool,
    stats: StatsSnapshot,
}

#[tauri::command]
pub fn engine_status(handle: State<'_, EngineHandle>) -> EngineStatus {
    let s = &handle.shared;
    EngineStatus {
        position: s.audible_position() as f64 / f64::from(handle.sample_rate),
        playing: s.playing.load(Ordering::Relaxed),
        ended: s.ended.load(Ordering::Relaxed),
        stats: *s.last_stats.lock().unwrap_or_else(|e| e.into_inner()),
    }
}

#[derive(Serialize)]
pub struct SpatialOptions {
    venues: Vec<room::VenueInfo>,
    current: SpatialConfig,
}

#[tauri::command]
pub fn engine_spatial_options(handle: State<'_, EngineHandle>) -> Result<SpatialOptions, String> {
    let current = handle.spatial.lock().map_err(|_| "engine poisoned")?.clone();
    Ok(SpatialOptions { venues: room::venue_list(), current })
}

/// Switch room / seat / output. The room response is generated on a worker
/// thread and handed to the audio thread when ready.
#[tauri::command]
pub async fn engine_set_spatial(handle: State<'_, EngineHandle>, config: SpatialConfig) -> Result<(), String> {
    let generation = handle.spatial_generation.fetch_add(1, Ordering::SeqCst) + 1;
    *handle.spatial.lock().map_err(|_| "engine poisoned")? = config.clone();
    let (hrtf, sample_rate) = (handle.hrtf.clone(), handle.sample_rate);
    let started = Instant::now();
    let label = format!("{} / seat {} / {:?}", config.room, config.seat, config.output);
    let scene = tauri::async_runtime::spawn_blocking(move || Scene::build(config, sample_rate, BLOCK, &hrtf))
        .await
        .map_err(|e| e.to_string())??;
    if handle.spatial_generation.load(Ordering::SeqCst) == generation {
        eprintln!("[engine] room -> {label} (built in {:.0} ms)", started.elapsed().as_secs_f64() * 1e3);
        handle.send(Command::SetScene(Box::new(scene)))?;
    } else {
        eprintln!("[engine] room -> {label} superseded, dropped");
    }
    Ok(())
}
