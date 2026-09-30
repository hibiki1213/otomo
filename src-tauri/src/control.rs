//! A small door into the running app, for the MCP server.
//!
//! One Unix socket in the app's data directory; one JSON line in, one JSON
//! line out. Only the filesystem guards it, which fits what it can do: start
//! and stop playback of songs already in this user's library. It never
//! touches files and never runs anything.
//!
//! Commands are forwarded to the UI as events, because the UI owns loading
//! (it parses the MIDI and drives the engine). `seconds` is resolved on the
//! server's side from a bar number, so both ends agree on where bar 17 is.
//!
//! Nothing here applies an edit. The server can only ask for a song to be
//! played and say that an edit is waiting; approving it is the user's click.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;

use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter, Manager};

const SOCKET_NAME: &str = "control.sock";
/// The UI listens for this and acts on the payload.
pub const EVENT: &str = "otomo://control";

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Control {
    /// "play", "stop", "staged" (an edit is waiting) or "instrument".
    pub cmd: String,
    pub song_id: Option<i64>,
    pub seconds: Option<f64>,
    /// For "play": stop here, and loop back when asked.
    pub until: Option<f64>,
    pub looping: bool,
    /// For "play": sound only these tracks.
    pub solo: Option<Vec<usize>>,
    /// For "play": listen to the staged edit rather than the saved song.
    pub pending: bool,
}

impl Control {
    fn of(cmd: &str) -> Self {
        Control {
            cmd: cmd.into(),
            song_id: None,
            seconds: None,
            until: None,
            looping: false,
            solo: None,
            pending: false,
        }
    }
}

/// Start listening. A failure here only costs the app its MCP connection, so
/// it is reported and otherwise ignored.
pub fn start(app: &AppHandle) {
    let path = match socket_path(app) {
        Ok(p) => p,
        Err(e) => {
            log::warn!("control socket unavailable: {e}");
            return;
        }
    };
    // A previous run that was killed leaves the file behind; the socket is
    // not a lock, so taking it over is the right thing.
    let _ = std::fs::remove_file(&path);
    let listener = match UnixListener::bind(&path) {
        Ok(l) => l,
        Err(e) => {
            log::warn!("control socket unavailable: {e}");
            return;
        }
    };
    log::info!("control socket at {}", path.display());

    let app = app.clone();
    let _ = std::thread::Builder::new().name("otomo-control".into()).spawn(move || {
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => serve(&app, stream),
                Err(e) => log::warn!("control connection failed: {e}"),
            }
        }
    });
}

fn socket_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join(SOCKET_NAME))
}

fn serve(app: &AppHandle, stream: UnixStream) {
    let mut line = String::new();
    if BufReader::new(&stream).read_line(&mut line).is_err() {
        return;
    }
    let reply = match handle(app, &line) {
        Ok(extra) => {
            let mut reply = serde_json::json!({ "ok": true });
            if let (Some(map), Some(extra)) = (reply.as_object_mut(), extra.as_object()) {
                map.extend(extra.clone());
            }
            reply
        }
        Err(e) => serde_json::json!({ "ok": false, "error": e }),
    };
    let mut stream = stream;
    let _ = writeln!(stream, "{reply}");
    let _ = stream.flush();
}

fn handle(app: &AppHandle, line: &str) -> Result<Value, String> {
    let message: Value = serde_json::from_str(line.trim()).map_err(|e| e.to_string())?;
    let cmd = message.get("cmd").and_then(Value::as_str).unwrap_or_default();
    let nothing = serde_json::json!({});
    match cmd {
        "play" => {
            let song_id = message
                .get("songId")
                .and_then(Value::as_i64)
                .ok_or("play needs songId")?;
            app.emit(
                EVENT,
                Control {
                    song_id: Some(song_id),
                    seconds: message.get("seconds").and_then(Value::as_f64),
                    until: message.get("until").and_then(Value::as_f64),
                    looping: message.get("loop").and_then(Value::as_bool).unwrap_or(false),
                    solo: message.get("solo").and_then(Value::as_array).map(|list| {
                        list.iter().filter_map(Value::as_u64).map(|n| n as usize).collect()
                    }),
                    pending: message.get("pending").and_then(Value::as_bool).unwrap_or(false),
                    ..Control::of("play")
                },
            )
            .map(|()| nothing)
            .map_err(|e| e.to_string())
        }
        "stop" => app.emit(EVENT, Control::of("stop")).map(|()| nothing).map_err(|e| e.to_string()),
        // An edit was staged: the UI shows it for approval.
        "staged" => app
            .emit(
                EVENT,
                Control {
                    song_id: message.get("songId").and_then(Value::as_i64),
                    pending: true,
                    ..Control::of("staged")
                },
            )
            .map(|()| nothing)
            .map_err(|e| e.to_string()),
        // What this machine can play a part through. Answered here rather
        // than by the UI, because the installed libraries are Rust's to know.
        "instruments" => Ok(serde_json::json!({ "instruments": crate::library::installed(app) })),
        "setInstrument" => {
            let track = message
                .get("track")
                .and_then(Value::as_u64)
                .ok_or("setInstrument needs track")? as usize;
            let id = message
                .get("instrument")
                .and_then(Value::as_str)
                .ok_or("setInstrument needs instrument")?;
            let message = crate::library::use_instrument(app, track, id)?;
            Ok(serde_json::json!({ "message": message }))
        }
        other => Err(format!("unknown command: {other}")),
    }
}
