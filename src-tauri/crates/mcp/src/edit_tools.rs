//! The editing half of the tool set.
//!
//! Every one of these follows the same shape: read the current version (the
//! staged one if an edit is already waiting), change one thing, stage the
//! result. Nothing here writes to the library — a person applies the change
//! in the app, having heard it.

use otomo_score::{program_number, KeyChange, Score, Voice};
use serde_json::{json, Value};

use crate::control;
use crate::library::{Library, Song};
use crate::pending;
use crate::rpc::{need_i64, need_str, opt_i64, opt_str, Tool};

pub fn tools() -> Vec<Tool> {
    vec![
        Tool {
            name: "create_song",
            description: "Create an empty song in the library, then write its contents with add_track and write_bars. \
                          For music that already exists (national anthems, classical pieces, folk songs, etc.), \
                          first search the web for MIDI or sheet music and import it with download instead.",
            schema: json!({
                "type": "object",
                "properties": {
                    "title": { "type": "string" },
                    "tempo": { "type": "number", "description": "BPM (default 120)" },
                    "time_signature": { "type": "string", "description": "\"4/4\" (default), \"3/4\", etc." },
                    "key": { "type": "string", "description": "\"D major\", \"B minor\", etc." }
                },
                "required": ["title"]
            }),
        },
        Tool {
            name: "write_bars",
            description: "Replace a range of bars in one track with exactly what you write. \
                          The notation is the same as read_bars: `pitch@beat:length` (append `v90` for velocity). \
                          Bars in the range that you leave out become empty. Repeating the same call does not \
                          double the notes.",
            schema: json!({
                "type": "object",
                "properties": {
                    "song_id": { "type": "integer" },
                    "track": { "type": "integer", "description": "Track number from get_song" },
                    "from_bar": { "type": "integer" },
                    "to_bar": { "type": "integer" },
                    "notes": {
                        "type": "string",
                        "description": "One bar per line, labelled with its bar number, e.g. `b12  C4@1:1 [E4 G4]@3:2`. \
                                        An empty string clears the range."
                    }
                },
                "required": ["song_id", "track", "from_bar", "to_bar", "notes"]
            }),
        },
        Tool {
            name: "add_track",
            description: "Add an instrument track. With percussion set to true it becomes a drum track (MIDI ch. 10).",
            schema: json!({
                "type": "object",
                "properties": {
                    "song_id": { "type": "integer" },
                    "instrument": {
                        "type": "string",
                        "description": "General MIDI instrument name (\"Flute\", \"French Horn\", \"String Ensemble 1\") or number 0–127"
                    },
                    "name": { "type": "string", "description": "Part name shown in the score (defaults to the instrument name)" },
                    "percussion": { "type": "boolean" }
                },
                "required": ["song_id", "instrument"]
            }),
        },
        Tool {
            name: "set_track",
            description: "Change a track's instrument or name.",
            schema: json!({
                "type": "object",
                "properties": {
                    "song_id": { "type": "integer" },
                    "track": { "type": "integer" },
                    "instrument": { "type": "string", "description": "General MIDI instrument name or number" },
                    "name": { "type": "string" }
                },
                "required": ["song_id", "track"]
            }),
        },
        Tool {
            name: "add_tracks",
            description: "Add several instruments at once, for setting up an ensemble in one go. \
                          Include \"Drums\" for a drum track.",
            schema: json!({
                "type": "object",
                "properties": {
                    "song_id": { "type": "integer" },
                    "instruments": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "e.g. [\"Flute\", \"Clarinet\", \"French Horn\", \"String Ensemble 1\", \"Drums\"]"
                    }
                },
                "required": ["song_id", "instruments"]
            }),
        },
        Tool {
            name: "write_score",
            description: "Write several tracks in one call, in the same format read_bars prints: \
                          a header such as `[0] Flute` switches to that track. \
                          Tracks without a header are left untouched. Use this for large ensembles.",
            schema: json!({
                "type": "object",
                "properties": {
                    "song_id": { "type": "integer" },
                    "from_bar": { "type": "integer" },
                    "to_bar": { "type": "integer" },
                    "notes": {
                        "type": "string",
                        "description": "Example:\n[0] Flute\nb1  C6@1:2 D6@3:2\nb2  E6@1:4\n\n[2] Cello\nb1  C3@1:4\nb2  G2@1:4"
                    }
                },
                "required": ["song_id", "from_bar", "to_bar", "notes"]
            }),
        },
        Tool {
            name: "copy_bars",
            description: "Copy a range of bars from one track to another — the core of orchestration work. \
                          transpose shifts the pitch; voice takes only the top line of the chords (melody) \
                          or the bottom line (bass). With merge set to true, the notes are layered on top \
                          instead of replacing what is there (octave doublings, etc.).",
            schema: json!({
                "type": "object",
                "properties": {
                    "song_id": { "type": "integer" },
                    "from_track": { "type": "integer" },
                    "to_track": { "type": "integer" },
                    "from_bar": { "type": "integer" },
                    "to_bar": { "type": "integer" },
                    "at_bar": { "type": "integer", "description": "First bar at the destination (defaults to the same bar)" },
                    "transpose": { "type": "integer", "description": "In semitones. 12 is an octave up" },
                    "voice": {
                        "type": "string",
                        "description": "\"all\" (default) / \"top\" (highest note of each chord = melody) / \"bottom\" (lowest note = bass)"
                    },
                    "merge": { "type": "boolean", "description": "Layer onto the destination instead of clearing it (default false)" }
                },
                "required": ["song_id", "from_track", "to_track", "from_bar", "to_bar"]
            }),
        },
        Tool {
            name: "remove_track",
            description: "Delete a whole track. Note that the tracks after it are renumbered (each moves down by one).",
            schema: json!({
                "type": "object",
                "properties": { "song_id": { "type": "integer" }, "track": { "type": "integer" } },
                "required": ["song_id", "track"]
            }),
        },
        Tool {
            name: "set_tempo",
            description: "Change the tempo. With from_bar, the change starts at that bar. \
                          With both to_bar and to_bpm, the tempo changes gradually over that span (a ritardando, etc.).",
            schema: json!({
                "type": "object",
                "properties": {
                    "song_id": { "type": "integer" },
                    "bpm": { "type": "number" },
                    "from_bar": { "type": "integer" },
                    "to_bar": { "type": "integer", "description": "Together with to_bpm, the tempo changes gradually over the range (rit. / accel.)" },
                    "to_bpm": { "type": "number" }
                },
                "required": ["song_id", "bpm"]
            }),
        },
        Tool {
            name: "set_time_signature",
            description: "Change the time signature. The bar numbers after from_bar change.",
            schema: json!({
                "type": "object",
                "properties": {
                    "song_id": { "type": "integer" },
                    "signature": { "type": "string", "description": "\"3/4\", \"6/8\", etc." },
                    "from_bar": { "type": "integer" }
                },
                "required": ["song_id", "signature"]
            }),
        },
        Tool {
            name: "set_key",
            description: "Set the key signature.",
            schema: json!({
                "type": "object",
                "properties": {
                    "song_id": { "type": "integer" },
                    "key": { "type": "string", "description": "\"D major\", \"B minor\", etc." }
                },
                "required": ["song_id", "key"]
            }),
        },
        Tool {
            name: "insert_bars",
            description: "Insert empty bars before the given bar. Everything after it moves later.",
            schema: json!({
                "type": "object",
                "properties": {
                    "song_id": { "type": "integer" },
                    "at_bar": { "type": "integer" },
                    "count": { "type": "integer" }
                },
                "required": ["song_id", "at_bar", "count"]
            }),
        },
        Tool {
            name: "delete_bars",
            description: "Remove bars from every track. Everything after them moves earlier to close the gap.",
            schema: json!({
                "type": "object",
                "properties": {
                    "song_id": { "type": "integer" },
                    "from_bar": { "type": "integer" },
                    "to_bar": { "type": "integer" }
                },
                "required": ["song_id", "from_bar", "to_bar"]
            }),
        },
        Tool {
            name: "set_dynamics",
            description: "Set the dynamics of a range of bars. With to_level it becomes a crescendo / diminuendo, \
                          and an expression (CC11) curve is written too, so that held notes swell as well.",
            schema: json!({
                "type": "object",
                "properties": {
                    "song_id": { "type": "integer" },
                    "track": { "type": "integer" },
                    "from_bar": { "type": "integer" },
                    "to_bar": { "type": "integer" },
                    "level": { "type": "string", "description": "pp / p / mp / mf / f / ff or 1–127" },
                    "to_level": { "type": "string", "description": "Omit for a constant level. When given, the dynamics move toward this level" }
                },
                "required": ["song_id", "track", "from_bar", "to_bar", "level"]
            }),
        },
        Tool {
            name: "set_pickup",
            description: "Make bar 1 a pickup (anacrusis). beats is the length of the pickup; 0 removes it. \
                          Note that the bar numbers after it shift by one.",
            schema: json!({
                "type": "object",
                "properties": {
                    "song_id": { "type": "integer" },
                    "beats": { "type": "number", "description": "Length of the pickup (in beats)" }
                },
                "required": ["song_id", "beats"]
            }),
        },
        Tool {
            name: "discard_changes",
            description: "Throw away the pending (unapplied) edits and go back to the version in the library.",
            schema: json!({
                "type": "object",
                "properties": { "song_id": { "type": "integer" } },
                "required": ["song_id"]
            }),
        },
    ]
}

pub fn is_edit(name: &str) -> bool {
    tools().iter().any(|t| t.name == name)
}

/// Run an editing tool. `open` hands back the song and the version to edit.
pub fn call(
    name: &str,
    args: &Value,
    library: &Library,
    data_dir: &std::path::Path,
) -> Result<String, String> {
    if name == "create_song" {
        return create_song(args, library, data_dir);
    }
    let song = library.get(need_i64(args, "song_id")?)?;
    if name == "discard_changes" {
        return Ok(if pending::discard(data_dir, song.id) {
            notify(data_dir, song.id);
            format!("Discarded the pending edits to \"{}\".", song.title)
        } else {
            format!("\"{}\" has no pending edits.", song.title)
        });
    }

    let mut score = pending::current(library, data_dir, &song)?;
    let summary = apply(name, args, &mut score)?;
    let staged = pending::stage(data_dir, &song, &score, &summary)?;
    notify(data_dir, song.id);
    Ok(staged)
}

/// Tell the app an edit is waiting, so its banner appears at once. Best
/// effort: with the app closed the files are still there to be found later.
fn notify(data_dir: &std::path::Path, song_id: i64) {
    let _ = control::send(data_dir, json!({ "cmd": "staged", "songId": song_id }));
}

fn apply(name: &str, args: &Value, score: &mut Score) -> Result<String, String> {
    match name {
        "write_bars" => score.write_bars(
            track_of(args)?,
            need_bar(args, "from_bar")?,
            need_bar(args, "to_bar")?,
            need_str(args, "notes")?,
        ),
        "add_track" => {
            let percussion = args.get("percussion").and_then(Value::as_bool).unwrap_or(false);
            let instrument = instrument_of(args, "instrument")?.unwrap_or(0);
            // An unnamed part is called after what it plays, which reads
            // better in the app than "Track 4".
            let name = opt_str(args, "name")
                .unwrap_or_else(|| otomo_score::program_name(instrument).to_string());
            Ok(score.add_track(instrument, &name, percussion))
        }
        "add_tracks" => {
            let list = args
                .get("instruments")
                .and_then(Value::as_array)
                .ok_or("instruments is required (an array of instrument names)")?;
            if list.is_empty() {
                return Err("Give at least one instrument.".into());
            }
            let mut done = Vec::new();
            for item in list {
                let text = item.as_str().ok_or("instruments must be an array of strings")?;
                let percussion = text.trim().eq_ignore_ascii_case("drums")
                    || text.trim().eq_ignore_ascii_case("percussion");
                let program = if percussion { 0 } else { program_number(text)? };
                let name = if percussion {
                    "Drums".to_string()
                } else {
                    otomo_score::program_name(program).to_string()
                };
                done.push(score.add_track(program, &name, percussion));
            }
            Ok(done.join("\n"))
        }
        "set_track" => score.set_track(
            track_of(args)?,
            instrument_of(args, "instrument")?,
            opt_str(args, "name").as_deref(),
        ),
        "copy_bars" => score.copy_bars(
            need_usize(args, "from_track")?,
            need_usize(args, "to_track")?,
            need_bar(args, "from_bar")?,
            need_bar(args, "to_bar")?,
            match opt_i64(args, "at_bar") {
                Some(_) => need_bar(args, "at_bar")?,
                None => need_bar(args, "from_bar")?,
            },
            opt_i64(args, "transpose").unwrap_or(0).clamp(-48, 48) as i32,
            match opt_str(args, "voice") {
                Some(text) => Voice::parse(&text)?,
                None => Voice::All,
            },
            args.get("merge").and_then(Value::as_bool).unwrap_or(false),
        ),
        "write_score" => score.write_score(
            need_bar(args, "from_bar")?,
            need_bar(args, "to_bar")?,
            need_str(args, "notes")?,
        ),
        "remove_track" => score.remove_track(track_of(args)?),
        "set_tempo" => {
            let bpm = args.get("bpm").and_then(Value::as_f64).ok_or("bpm is required (a number)")?;
            let from = opt_i64(args, "from_bar").unwrap_or(1).max(1) as u32;
            // A second tempo and a range make it a curve rather than a step.
            match (args.get("to_bpm").and_then(Value::as_f64), opt_i64(args, "to_bar")) {
                (Some(to_bpm), Some(_)) => score.tempo_curve(from, need_bar(args, "to_bar")?, bpm, to_bpm),
                _ => score.set_tempo(bpm, from),
            }
        }
        "set_dynamics" => score.set_dynamics(
            track_of(args)?,
            need_bar(args, "from_bar")?,
            need_bar(args, "to_bar")?,
            need_str(args, "level")?,
            opt_str(args, "to_level").as_deref(),
        ),
        "set_pickup" => score.set_pickup(
            args.get("beats").and_then(Value::as_f64).ok_or("beats is required (a number of beats)")?,
        ),
        "set_time_signature" => {
            let (num, den) = parse_signature(need_str(args, "signature")?)?;
            score.set_time_signature(num, den, opt_i64(args, "from_bar").unwrap_or(1).max(1) as u32)
        }
        "set_key" => {
            let (sharps, minor) = KeyChange::parse(need_str(args, "key")?)?;
            score.set_key(sharps, minor)
        }
        "insert_bars" => score.insert_bars(
            need_bar(args, "at_bar")?,
            need_i64(args, "count")?.clamp(0, 512) as u32,
        ),
        "delete_bars" => score.delete_bars(need_bar(args, "from_bar")?, need_bar(args, "to_bar")?),
        other => Err(format!("Unknown tool: {other}")),
    }
}

fn create_song(args: &Value, library: &Library, data_dir: &std::path::Path) -> Result<String, String> {
    let (num, den) = match opt_str(args, "time_signature") {
        Some(text) => parse_signature(&text)?,
        None => (4, 4),
    };
    let bpm = args.get("tempo").and_then(Value::as_f64).unwrap_or(120.0);
    if !(10.0..=400.0).contains(&bpm) {
        return Err("The tempo must be between 10 and 400 BPM.".into());
    }
    let mut score = Score::blank(bpm, num, den);
    if let Some(key) = opt_str(args, "key") {
        let (sharps, minor) = KeyChange::parse(&key)?;
        score.set_key(sharps, minor)?;
    }
    let title = need_str(args, "title")?;
    let song = library.create(title, &score.to_smf()?)?;
    let _ = data_dir;
    Ok(format!(
        "Created \"{title}\" (song_id {}, {bpm:.0} BPM, {num}/{den}). \
         Add instruments with add_track, then write with write_bars.",
        song.id
    ))
}

fn track_of(args: &Value) -> Result<usize, String> {
    need_usize(args, "track")
}

fn need_usize(args: &Value, key: &str) -> Result<usize, String> {
    usize::try_from(need_i64(args, key)?).map_err(|_| format!("{key} must be 0 or more"))
}

/// An instrument given by name or by number; absent means "leave it alone".
fn instrument_of(args: &Value, key: &str) -> Result<Option<u8>, String> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => Ok(Some(n.as_i64().unwrap_or(0).clamp(0, 127) as u8)),
        Some(Value::String(text)) => program_number(text).map(Some),
        Some(_) => Err(format!("{key} must be an instrument name or a number 0–127")),
    }
}

fn need_bar(args: &Value, key: &str) -> Result<u32, String> {
    let bar = need_i64(args, key)?;
    u32::try_from(bar)
        .ok()
        .filter(|b| *b >= 1)
        .ok_or_else(|| format!("{key} must be a bar number of 1 or more"))
}

/// "6/8" → (6, 8).
fn parse_signature(text: &str) -> Result<(u8, u16), String> {
    let (num, den) = text
        .trim()
        .split_once('/')
        .ok_or_else(|| format!("{text}: write time signatures like \"4/4\""))?;
    let num: u8 = num.trim().parse().map_err(|_| format!("{text}: the numerator is not a number"))?;
    let den: u16 = den.trim().parse().map_err(|_| format!("{text}: the denominator is not a number"))?;
    Ok((num, den))
}

/// What is staged, for `get_song` and the `pending_changes` tool.
pub fn describe_staged(data_dir: &std::path::Path, song: &Song) -> Option<String> {
    let staged = pending::staged(data_dir, song.id)?;
    let mut out = format!("\nPending edits (confirmed when the user presses Apply in Otomo):\n");
    for change in &staged.changes {
        out.push_str(&format!("  - {}\n", change.lines().next().unwrap_or(change)));
    }
    Some(out)
}
