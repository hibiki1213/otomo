//! Talking to the running app.
//!
//! The app listens on a Unix socket in its data directory; one JSON line in,
//! one JSON line out. Only the filesystem guards it, which is the right level
//! for something that just starts and stops playback on this machine.
//!
//! The app not being open is an ordinary outcome, not a failure of the
//! server: reading and analysing songs works either way.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use serde_json::{json, Value};

pub const SOCKET_NAME: &str = "control.sock";

/// Send one command and return what the app said.
pub fn send(dir: &Path, command: Value) -> Result<Value, String> {
    send_within(dir, command, Duration::from_secs(3))
}

/// Some commands do real work before they can answer — loading a sampled
/// instrument, for one — so they are given longer than a glance.
pub fn send_within(dir: &Path, command: Value, wait: Duration) -> Result<Value, String> {
    let path = dir.join(SOCKET_NAME);
    let mut stream = UnixStream::connect(&path)
        .map_err(|_| "Otomo が起動していません。アプリを開いてからもう一度お試しください。".to_string())?;
    // The app answers within the window or not at all; never hang the model.
    let timeout = Some(wait);
    let _ = stream.set_read_timeout(timeout);
    let _ = stream.set_write_timeout(timeout);

    writeln!(stream, "{command}").map_err(|e| format!("Otomo に送れません: {e}"))?;
    stream.flush().map_err(|e| e.to_string())?;

    let mut line = String::new();
    BufReader::new(&stream)
        .read_line(&mut line)
        .map_err(|e| format!("Otomo が応答しません: {e}"))?;
    let reply: Value = serde_json::from_str(line.trim())
        .map_err(|e| format!("Otomo の応答を解釈できません: {e}"))?;
    if reply.get("ok").and_then(Value::as_bool) == Some(true) {
        Ok(reply)
    } else {
        Err(reply
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("Otomo がコマンドを実行できませんでした")
            .to_string())
    }
}

#[allow(clippy::too_many_arguments)]
pub fn play(
    dir: &Path,
    song_id: i64,
    seconds: f64,
    until: Option<f64>,
    looping: bool,
    solo: Option<Vec<usize>>,
    pending: bool,
) -> Result<(), String> {
    send(
        dir,
        json!({
            "cmd": "play",
            "songId": song_id,
            "seconds": seconds,
            "until": until,
            "loop": looping,
            "solo": solo,
            "pending": pending,
        }),
    )
    .map(|_| ())
}

/// Ask the app which instruments it can play a part through (its installed
/// SFZ libraries and audio plugins). Needs the app to be running, because
/// only it knows what is installed.
pub fn instruments(dir: &Path) -> Result<Value, String> {
    send(dir, json!({ "cmd": "instruments" }))
}

/// Play a track through one of those instruments, for the song on screen.
pub fn set_playback(dir: &Path, track: usize, instrument: &str) -> Result<Value, String> {
    send_within(
        dir,
        json!({ "cmd": "setInstrument", "track": track, "instrument": instrument }),
        // A gigabyte of samples takes a moment to reach the audio thread.
        Duration::from_secs(30),
    )
}

pub fn stop(dir: &Path) -> Result<(), String> {
    send(dir, json!({ "cmd": "stop" })).map(|_| ())
}
