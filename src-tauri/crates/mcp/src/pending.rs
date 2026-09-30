//! Edits wait here until a person says yes.
//!
//! A tool call never changes the library. It reads the song (the staged
//! version if there is one, so a sequence of edits builds up), applies one
//! change, and writes the result to `pending/<song_id>.mid` next to a note of
//! what was done. The app shows that note with **適用 / 破棄**, and can play
//! the staged version so the change can be heard before it is kept.
//!
//! Plain MIDI on purpose: the app can load a pending file with exactly the
//! code it already uses for everything else.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use otomo_score::Score;
use serde_json::{json, Value};

use crate::library::{Library, Song};

pub fn dir(data_dir: &Path) -> PathBuf {
    data_dir.join("pending")
}

fn midi_path(data_dir: &Path, song_id: i64) -> PathBuf {
    dir(data_dir).join(format!("{song_id}.mid"))
}

fn note_path(data_dir: &Path, song_id: i64) -> PathBuf {
    dir(data_dir).join(format!("{song_id}.json"))
}

/// What is staged for one song.
pub struct Staged {
    pub changes: Vec<String>,
}

pub fn staged(data_dir: &Path, song_id: i64) -> Option<Staged> {
    if !midi_path(data_dir, song_id).exists() {
        return None;
    }
    let note: Value = std::fs::read_to_string(note_path(data_dir, song_id))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| json!({}));
    let changes = note
        .get("changes")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect())
        .unwrap_or_default();
    Some(Staged { changes })
}

/// The version to work from: the staged one if it exists, else the library's.
pub fn current(library: &Library, data_dir: &Path, song: &Song) -> Result<Score, String> {
    let bytes = match std::fs::read(midi_path(data_dir, song.id)) {
        Ok(bytes) => bytes,
        Err(_) => library.bytes(song)?,
    };
    Score::parse(&bytes)
}

/// Stage `score` as the new version of `song`, recording what changed.
pub fn stage(data_dir: &Path, song: &Song, score: &Score, change: &str) -> Result<String, String> {
    let dir = dir(data_dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;

    let bytes = score.to_smf()?;
    let path = midi_path(data_dir, song.id);
    // Write-then-rename, so the app never loads a half-written edit.
    let tmp = path.with_extension("part");
    std::fs::write(&tmp, &bytes).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;

    let mut changes = staged(data_dir, song.id).map(|s| s.changes).unwrap_or_default();
    changes.push(change.to_string());
    // The banner shows these; a long session should not grow without bound.
    if changes.len() > 20 {
        changes.drain(..changes.len() - 20);
    }
    let note = json!({
        "songId": song.id,
        "title": song.title,
        "changes": changes,
        "updated": SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
    });
    std::fs::write(note_path(data_dir, song.id), note.to_string()).map_err(|e| e.to_string())?;

    Ok(format!(
        "{change}\n（未適用。Otomo で「適用」を押すと確定します。play で試聴できます）"
    ))
}

pub fn discard(data_dir: &Path, song_id: i64) -> bool {
    let had = midi_path(data_dir, song_id).exists();
    let _ = std::fs::remove_file(midi_path(data_dir, song_id));
    let _ = std::fs::remove_file(note_path(data_dir, song_id));
    had
}
