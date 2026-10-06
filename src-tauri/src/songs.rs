//! The song library: every MIDI the app has opened, kept in the app's own
//! storage so it stays openable after the original file is moved, renamed or
//! deleted — and so songs that never had a file (generated ones, later) live
//! in the same place.
//!
//! The file itself is copied to `songs/<sha256>.mid` and the row in
//! `library.db` carries what the UI shows: title, where it came from, how long
//! it is, when it was last played. Content addressing makes re-opening the
//! same file an update instead of a duplicate.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use sha2::{Digest, Sha256};
use tauri::ipc::Response;
use tauri::{AppHandle, Manager, State};

use crate::i18n::tr;
use crate::instruments::InstrumentChoice;

/// Opened once at startup and shared by every command.
pub struct SongLibrary {
    db: Mutex<Connection>,
    dir: PathBuf,
    /// Where the MCP server leaves edits for a person to approve.
    pending: PathBuf,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SongRecord {
    pub id: i64,
    /// Shown in the UI; starts as the file name and can be renamed.
    pub title: String,
    pub file_name: String,
    pub bytes: i64,
    /// Where it was imported from, if it came from a file on disk.
    pub source_path: Option<String>,
    /// Seconds; 0 until the song has been opened once.
    pub duration: f64,
    pub track_count: i64,
    pub added_at: i64,
    pub last_played: Option<i64>,
    pub play_count: i64,
    pub favorite: bool,
    /// An earlier version to step back to (⌘Z).
    pub can_undo: bool,
    /// A version that was undone, to step forward to again (⌘⇧Z).
    pub can_redo: bool,
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS songs (
    id          INTEGER PRIMARY KEY,
    title       TEXT    NOT NULL,
    file_name   TEXT    NOT NULL,
    hash        TEXT    NOT NULL UNIQUE,
    previous_hash TEXT,
    version     INTEGER,
    bytes       INTEGER NOT NULL,
    source_path TEXT,
    duration    REAL    NOT NULL DEFAULT 0,
    track_count INTEGER NOT NULL DEFAULT 0,
    added_at    INTEGER NOT NULL,
    last_played INTEGER,
    play_count  INTEGER NOT NULL DEFAULT 0,
    favorite    INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS songs_recent ON songs (last_played DESC, added_at DESC);
-- What each track was last set to play through, by a person or by Claude.
-- A playback setting, so it lives here rather than in the MIDI file.
CREATE TABLE IF NOT EXISTS song_instruments (
    song_id     INTEGER NOT NULL,
    track       INTEGER NOT NULL,
    choice      TEXT    NOT NULL,
    label       TEXT    NOT NULL,
    PRIMARY KEY (song_id, track)
);
-- Every version a song has had, oldest first: what undo and redo step
-- through. songs.version is the one in use, and songs.hash its file.
CREATE TABLE IF NOT EXISTS song_versions (
    id          INTEGER PRIMARY KEY,
    song_id     INTEGER NOT NULL,
    hash        TEXT    NOT NULL,
    -- What made this version: 'original', 'edit' (from before the history
    -- was kept), 'claude' (applied edits) or 'paste'.
    kind        TEXT    NOT NULL,
    -- How much: edits applied, notes pasted.
    count       INTEGER NOT NULL DEFAULT 0,
    created_at  INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS song_versions_song ON song_versions (song_id, id);
-- SFZ files the user picked from disk, so they can be offered again (and
-- named to Claude) like the instruments of a downloaded library.
CREATE TABLE IF NOT EXISTS user_instruments (
    id          INTEGER PRIMARY KEY,
    path        TEXT    NOT NULL UNIQUE,
    name        TEXT    NOT NULL,
    added_at    INTEGER NOT NULL
);
";

/// A track's remembered instrument.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SavedInstrument {
    pub track: usize,
    pub choice: InstrumentChoice,
    pub label: String,
}

/// An .sfz the user added from disk.
#[derive(Clone, Debug, PartialEq)]
pub struct UserInstrument {
    pub id: i64,
    pub path: String,
    pub name: String,
}

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

impl SongLibrary {
    /// Open (or create) the library under the app's data directory.
    pub fn open(app: &AppHandle) -> Result<Self, String> {
        let root = app.path().app_data_dir().map_err(|e| e.to_string())?;
        let dir = root.join("songs");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let db = Connection::open(root.join("library.db")).map_err(|e| e.to_string())?;
        // Survive a crash mid-write without losing the library.
        db.execute_batch("PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;")
            .map_err(|e| e.to_string())?;
        db.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
        // Libraries made before applied edits existed have no room to
        // remember what a song looked like beforehand; give them one.
        let _ = db.execute("ALTER TABLE songs ADD COLUMN previous_hash TEXT", []);
        // …and before the history, a place to say which version is in use.
        let _ = db.execute("ALTER TABLE songs ADD COLUMN version INTEGER", []);
        Ok(Self { db: Mutex::new(db), dir, pending: root.join("pending") })
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Connection>, String> {
        // Nothing is left half-written under the lock, so a poisoned lock is
        // still usable.
        Ok(self.db.lock().unwrap_or_else(|e| e.into_inner()))
    }

    fn path_for(&self, hash: &str) -> PathBuf {
        self.dir.join(format!("{hash}.mid"))
    }

    /// Copy `data` into the library. Importing the same content twice keeps
    /// the existing entry (and refreshes where it came from).
    pub fn import(&self, data: &[u8], file_name: &str, source_path: Option<&str>) -> Result<SongRecord, String> {
        if data.is_empty() {
            return Err("empty file".into());
        }
        let hash = format!("{:x}", Sha256::digest(data));
        let db = self.lock()?;
        let existing: Option<i64> = db
            .query_row("SELECT id FROM songs WHERE hash = ?1", params![hash], |r| r.get(0))
            .optional()
            .map_err(|e| e.to_string())?;
        if let Some(id) = existing {
            if let Some(src) = source_path {
                db.execute("UPDATE songs SET source_path = ?2 WHERE id = ?1", params![id, src])
                    .map_err(|e| e.to_string())?;
            }
            return get(&db, id);
        }

        let path = self.path_for(&hash);
        // Write-then-rename: a crash never leaves a truncated song behind.
        let tmp = path.with_extension("part");
        std::fs::write(&tmp, data).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;

        let title = Path::new(file_name).file_stem().map_or(file_name, |s| s.to_str().unwrap_or(file_name));
        db.execute(
            "INSERT INTO songs (title, file_name, hash, bytes, source_path, added_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![title, file_name, hash, data.len() as i64, source_path, now()],
        )
        .map_err(|e| e.to_string())?;
        get(&db, db.last_insert_rowid())
    }

    /// The instruments remembered for a song's tracks. Rows that no longer
    /// parse (a choice from an older build) are skipped, not fatal.
    pub fn instruments(&self, song: i64) -> Result<Vec<SavedInstrument>, String> {
        let db = self.lock()?;
        let mut stmt = db
            .prepare("SELECT track, choice, label FROM song_instruments WHERE song_id = ?1 ORDER BY track")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![song], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))
            .map_err(|e| e.to_string())?;
        Ok(rows
            .flatten()
            .filter_map(|(track, choice, label)| {
                Some(SavedInstrument {
                    track: usize::try_from(track).ok()?,
                    choice: serde_json::from_str(&choice).ok()?,
                    label,
                })
            })
            .collect())
    }

    pub fn set_instrument(&self, song: i64, track: usize, choice: &InstrumentChoice, label: &str) -> Result<(), String> {
        let choice = serde_json::to_string(choice).map_err(|e| e.to_string())?;
        self.lock()?
            .execute(
                "INSERT INTO song_instruments (song_id, track, choice, label) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (song_id, track) DO UPDATE SET choice = excluded.choice, label = excluded.label",
                params![song, track as i64, choice, label],
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    pub fn clear_instruments(&self, song: i64) -> Result<(), String> {
        self.lock()?
            .execute("DELETE FROM song_instruments WHERE song_id = ?1", params![song])
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    /// SFZ files the user has added, oldest first.
    pub fn user_instruments(&self) -> Result<Vec<UserInstrument>, String> {
        let db = self.lock()?;
        let mut stmt = db
            .prepare("SELECT id, path, name FROM user_instruments ORDER BY added_at, id")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| Ok(UserInstrument { id: r.get(0)?, path: r.get(1)?, name: r.get(2)? }))
            .map_err(|e| e.to_string())?;
        Ok(rows.flatten().collect())
    }

    /// Remember an .sfz picked from disk; adding the same file again keeps
    /// the existing entry.
    pub fn add_user_instrument(&self, path: &str, name: &str) -> Result<UserInstrument, String> {
        let db = self.lock()?;
        db.execute(
            "INSERT INTO user_instruments (path, name, added_at) VALUES (?1, ?2, ?3)
             ON CONFLICT (path) DO NOTHING",
            params![path, name, now()],
        )
        .map_err(|e| e.to_string())?;
        db.query_row("SELECT id, path, name FROM user_instruments WHERE path = ?1", params![path], |r| {
            Ok(UserInstrument { id: r.get(0)?, path: r.get(1)?, name: r.get(2)? })
        })
        .map_err(|e| e.to_string())
    }

    fn pending_path(&self, id: i64) -> PathBuf {
        self.pending.join(format!("{id}.mid"))
    }

    /// A song's title and MIDI bytes: what is saved, or what Claude staged.
    pub(crate) fn source(&self, id: i64, staged: bool) -> Result<(String, Vec<u8>), String> {
        let title = get(&*self.lock()?, id)?.title;
        let bytes = if staged {
            std::fs::read(self.pending_path(id))
                .map_err(|_| tr!("この曲に未適用の編集はありません", "This song has no pending edits"))?
        } else {
            self.read(id)?
        };
        Ok((title, bytes))
    }

    pub(crate) fn record(&self, id: i64) -> Result<SongRecord, String> {
        get(&*self.lock()?, id)
    }

    pub(crate) fn has_pending(&self, id: i64) -> bool {
        self.pending_path(id).exists()
    }

    /// Make `data` the song's saved version, as a new step in its history:
    /// what was undone before it can no longer be redone, as in any editor.
    /// `kind` and `count` say what made it, for "Undid: …".
    pub(crate) fn commit(&self, id: i64, data: &[u8], kind: &str, count: usize) -> Result<SongRecord, String> {
        if data.is_empty() {
            return Err(tr!("編集後のファイルが空です", "The edited file is empty"));
        }
        let hash = format!("{:x}", Sha256::digest(data));
        let path = self.path_for(&hash);
        let tmp = path.with_extension("part");
        std::fs::write(&tmp, data).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;

        let db = self.lock()?;
        let current = ensure_history(&db, id)?;
        db.execute("DELETE FROM song_versions WHERE song_id = ?1 AND id > ?2", params![id, current])
            .map_err(|e| e.to_string())?;
        db.execute(
            "INSERT INTO song_versions (song_id, hash, kind, count, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, hash, kind, count as i64, now()],
        )
        .map_err(|e| e.to_string())?;
        let version = db.last_insert_rowid();
        use_version(&db, id, version, &hash, data.len() as i64)?;
        // A long session's history is kept to the last HISTORY_LIMIT steps.
        db.execute(
            "DELETE FROM song_versions WHERE song_id = ?1 AND id NOT IN
               (SELECT id FROM song_versions WHERE song_id = ?1 ORDER BY id DESC LIMIT ?2)",
            params![id, HISTORY_LIMIT],
        )
        .map_err(|e| e.to_string())?;
        get(&db, id)
    }

    /// Step one version back (undo) or forward (redo). Returns the song and
    /// what the step took back or put back.
    pub(crate) fn step(&self, id: i64, back: bool) -> Result<(SongRecord, Step), String> {
        if self.has_pending(id) {
            return Err(tr!(
                "Claude の未適用の編集があります。先に適用か破棄をしてください。",
                "Claude has pending edits on this song. Apply or discard them first."
            ));
        }
        let db = self.lock()?;
        let current = ensure_history(&db, id)?;
        let sql = if back {
            "SELECT id, hash FROM song_versions WHERE song_id = ?1 AND id < ?2 ORDER BY id DESC LIMIT 1"
        } else {
            "SELECT id, hash FROM song_versions WHERE song_id = ?1 AND id > ?2 ORDER BY id ASC LIMIT 1"
        };
        let (target, hash): (i64, String) = db
            .query_row(sql, params![id, current], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()
            .map_err(|e| e.to_string())?
            .ok_or_else(|| {
                if back {
                    tr!("これ以上戻せません", "Nothing to undo")
                } else {
                    tr!("やり直せる操作はありません", "Nothing to redo")
                }
            })?;
        // Undo takes back what made the current version; redo puts back
        // what made the next one.
        let (kind, count): (String, i64) = db
            .query_row(
                "SELECT kind, count FROM song_versions WHERE id = ?1",
                params![if back { current } else { target }],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .map_err(|e| e.to_string())?;
        let bytes = std::fs::metadata(self.path_for(&hash))
            .map(|m| m.len() as i64)
            .map_err(|_| tr!("その版のファイルが見つかりません", "That version's file is missing"))?;
        use_version(&db, id, target, &hash, bytes)?;
        Ok((get(&db, id)?, Step { kind, count }))
    }

    /// Add a change to the song's staged edit, the way the MCP server stages
    /// Claude's (same files, same note), so it is applied or discarded with them.
    pub(crate) fn stage(&self, id: i64, data: &[u8], change: &str) -> Result<(), String> {
        std::fs::create_dir_all(&self.pending).map_err(|e| e.to_string())?;
        let path = self.pending_path(id);
        let tmp = path.with_extension("part");
        std::fs::write(&tmp, data).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;

        let note_path = self.pending.join(format!("{id}.json"));
        let mut note: serde_json::Value = std::fs::read_to_string(&note_path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_else(|| serde_json::json!({}));
        let title = get(&*self.lock()?, id)?.title;
        let mut changes: Vec<serde_json::Value> =
            note.get("changes").and_then(|c| c.as_array()).cloned().unwrap_or_default();
        changes.push(change.into());
        if changes.len() > 20 {
            changes.drain(..changes.len() - 20);
        }
        note["songId"] = id.into();
        note["title"] = title.into();
        note["changes"] = changes.into();
        note["updated"] = now().into();
        std::fs::write(&note_path, note.to_string()).map_err(|e| e.to_string())
    }

    fn read(&self, id: i64) -> Result<Vec<u8>, String> {
        let db = self.lock()?;
        let hash: String = db
            .query_row("SELECT hash FROM songs WHERE id = ?1", params![id], |r| r.get(0))
            .optional()
            .map_err(|e| e.to_string())?
            .ok_or("song not in the library")?;
        drop(db);
        std::fs::read(self.path_for(&hash)).map_err(|e| e.to_string())
    }
}

/// How many versions of a song are kept for undo.
const HISTORY_LIMIT: i64 = 100;

/// What an undo or redo stepped over, for the UI to name it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Step {
    kind: String,
    count: i64,
}

/// The song's current version in the history, starting the history the
/// first time it is needed: with the version an older build's "apply" kept
/// aside, if there is one, then the current one.
fn ensure_history(db: &Connection, id: i64) -> Result<i64, String> {
    let (hash, previous, version): (String, Option<String>, Option<i64>) = db
        .query_row("SELECT hash, previous_hash, version FROM songs WHERE id = ?1", params![id], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
        .map_err(|e| e.to_string())?;
    if let Some(v) = version {
        let known: bool = db
            .query_row("SELECT EXISTS (SELECT 1 FROM song_versions WHERE id = ?1 AND song_id = ?2)", params![v, id], |r| {
                r.get(0)
            })
            .map_err(|e| e.to_string())?;
        if known {
            return Ok(v);
        }
    }
    let insert = |hash: &str, kind: &str| -> Result<i64, String> {
        db.execute(
            "INSERT INTO song_versions (song_id, hash, kind, count, created_at) VALUES (?1, ?2, ?3, 0, ?4)",
            params![id, hash, kind, now()],
        )
        .map_err(|e| e.to_string())?;
        Ok(db.last_insert_rowid())
    };
    if let Some(previous) = &previous {
        insert(previous, "original")?;
    }
    let current = insert(&hash, if previous.is_some() { "edit" } else { "original" })?;
    db.execute("UPDATE songs SET version = ?2 WHERE id = ?1", params![id, current]).map_err(|e| e.to_string())?;
    Ok(current)
}

/// Point the song at a version. Duration and track count are what the old
/// version measured; opening the song again fills them in.
fn use_version(db: &Connection, id: i64, version: i64, hash: &str, bytes: i64) -> Result<(), String> {
    db.execute(
        "UPDATE songs
         SET hash = ?2, bytes = ?3, version = ?4, previous_hash = NULL, duration = 0, track_count = 0
         WHERE id = ?1",
        params![id, hash, bytes, version],
    )
    .map(|_| ())
    .map_err(|e| {
        if e.to_string().contains("UNIQUE") {
            tr!("同じ内容の曲がライブラリに別にあるため切り替えられません", "Another song in the library has exactly this content")
        } else {
            e.to_string()
        }
    })
}

const COLUMNS: &str = "id, title, file_name, bytes, source_path, duration, track_count,
                       added_at, last_played, play_count, favorite,
                       EXISTS (SELECT 1 FROM song_versions v WHERE v.song_id = songs.id AND v.id < songs.version)
                         OR (songs.version IS NULL AND previous_hash IS NOT NULL),
                       EXISTS (SELECT 1 FROM song_versions v WHERE v.song_id = songs.id AND v.id > songs.version)";

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<SongRecord> {
    Ok(SongRecord {
        id: r.get(0)?,
        title: r.get(1)?,
        file_name: r.get(2)?,
        bytes: r.get(3)?,
        source_path: r.get(4)?,
        duration: r.get(5)?,
        track_count: r.get(6)?,
        added_at: r.get(7)?,
        last_played: r.get(8)?,
        play_count: r.get(9)?,
        favorite: r.get::<_, i64>(10)? != 0,
        can_undo: r.get::<_, bool>(11)?,
        can_redo: r.get::<_, bool>(12)?,
    })
}

fn get(db: &Connection, id: i64) -> Result<SongRecord, String> {
    db.query_row(&format!("SELECT {COLUMNS} FROM songs WHERE id = ?1"), params![id], row)
        .map_err(|e| e.to_string())
}

/// An edit the MCP server staged, waiting for a person to approve it.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PendingEdit {
    pub song_id: i64,
    pub title: String,
    /// What the tools did, newest last.
    pub changes: Vec<String>,
    pub updated: i64,
}

/* ---------- commands ---------- */

/// Edits waiting for approval, newest first.
#[tauri::command]
pub fn songs_pending(library: State<'_, SongLibrary>) -> Result<Vec<PendingEdit>, String> {
    let Ok(entries) = std::fs::read_dir(&library.pending) else {
        return Ok(Vec::new());
    };
    let db = library.lock()?;
    let mut out: Vec<PendingEdit> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "json") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        let Ok(note) = serde_json::from_str::<serde_json::Value>(&text) else { continue };
        let Some(song_id) = note.get("songId").and_then(serde_json::Value::as_i64) else { continue };
        // The song may have been deleted while the edit waited.
        let Ok(song) = get(&db, song_id) else {
            continue;
        };
        out.push(PendingEdit {
            song_id,
            title: song.title,
            changes: note
                .get("changes")
                .and_then(serde_json::Value::as_array)
                .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect())
                .unwrap_or_default(),
            updated: note.get("updated").and_then(serde_json::Value::as_i64).unwrap_or(0),
        });
    }
    out.sort_by_key(|p| std::cmp::Reverse(p.updated));
    Ok(out)
}

/// The staged version's bytes, for previewing it before it is applied.
#[tauri::command]
pub fn songs_pending_data(library: State<'_, SongLibrary>, id: i64) -> Result<Response, String> {
    let bytes = std::fs::read(library.pending_path(id))
        .map_err(|_| tr!("この曲に未適用の編集はありません", "This song has no pending edits"))?;
    Ok(Response::new(bytes))
}

/// Accept a staged edit: it becomes the song, and the version it replaced is
/// kept so the change can be undone once.
#[tauri::command]
pub fn songs_apply_pending(library: State<'_, SongLibrary>, id: i64) -> Result<SongRecord, String> {
    let data = std::fs::read(library.pending_path(id))
        .map_err(|_| tr!("この曲に未適用の編集はありません", "This song has no pending edits"))?;
    let changes = pending_changes(&library, id).len();
    let record = library.commit(id, &data, "claude", changes)?;
    discard_pending(&library, id);
    Ok(record)
}

/// Throw a staged edit away.
#[tauri::command]
pub fn songs_discard_pending(library: State<'_, SongLibrary>, id: i64) -> Result<(), String> {
    discard_pending(&library, id);
    Ok(())
}

/// What a staged edit did, from its note.
fn pending_changes(library: &SongLibrary, id: i64) -> Vec<String> {
    std::fs::read_to_string(library.pending.join(format!("{id}.json")))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|note| note.get("changes").and_then(|c| c.as_array()).cloned())
        .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect())
        .unwrap_or_default()
}

fn discard_pending(library: &SongLibrary, id: i64) {
    let _ = std::fs::remove_file(library.pending_path(id));
    let _ = std::fs::remove_file(library.pending.join(format!("{id}.json")));
}

/// Undo: the song as it was before its last change (⌘Z).
#[tauri::command]
pub fn songs_undo(library: State<'_, SongLibrary>, id: i64) -> Result<Stepped, String> {
    let (record, step) = library.step(id, true)?;
    Ok(Stepped { record, step })
}

/// Redo: put back the change undone last (⌘⇧Z).
#[tauri::command]
pub fn songs_redo(library: State<'_, SongLibrary>, id: i64) -> Result<Stepped, String> {
    let (record, step) = library.step(id, false)?;
    Ok(Stepped { record, step })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stepped {
    record: SongRecord,
    step: Step,
}

/// The library, most recently played first. `query` filters by title or file
/// name; an empty query returns everything.
#[tauri::command]
pub fn songs_list(library: State<'_, SongLibrary>, query: Option<String>) -> Result<Vec<SongRecord>, String> {
    let db = library.lock()?;
    let like = format!("%{}%", query.unwrap_or_default().trim().to_lowercase());
    let mut stmt = db
        .prepare(&format!(
            "SELECT {COLUMNS} FROM songs
             WHERE ?1 = '%%' OR lower(title) LIKE ?1 OR lower(file_name) LIKE ?1
             ORDER BY favorite DESC, coalesce(last_played, added_at) DESC"
        ))
        .map_err(|e| e.to_string())?;
    let rows = stmt.query_map(params![like], row).map_err(|e| e.to_string())?;
    rows.collect::<rusqlite::Result<Vec<_>>>().map_err(|e| e.to_string())
}

/// One entry, by id.
#[tauri::command]
pub fn songs_get(library: State<'_, SongLibrary>, id: i64) -> Result<SongRecord, String> {
    let db = library.lock()?;
    get(&db, id)
}

/// Copy a MIDI file on disk into the library. A MusicXML score (.musicxml,
/// .xml, .mxl) is converted to MIDI on the way in, so everything downstream
/// only ever sees MIDI.
#[tauri::command]
pub async fn songs_import(library: State<'_, SongLibrary>, path: String) -> Result<SongRecord, String> {
    let p = PathBuf::from(&path);
    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let stem = p.file_stem().and_then(|n| n.to_str()).unwrap_or("song").to_owned();
    match ext.as_str() {
        "mid" | "midi" => {
            let data = tokio::fs::read(&p).await.map_err(|e| e.to_string())?;
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("song.mid").to_owned();
            library.import(&data, &name, Some(&path))
        }
        "musicxml" | "xml" | "mxl" => {
            let data = tokio::fs::read(&p).await.map_err(|e| e.to_string())?;
            if !otomo_score::is_musicxml(&data) {
                return Err(tr!("{stem}.{ext} は MusicXML の楽譜ではありません。", "{stem}.{ext} is not a MusicXML score."));
            }
            // The reader's errors are in English (they are Claude's too).
            let smf = otomo_score::read_musicxml(&data)
                .and_then(|read| read.score.to_smf())
                .map_err(|e| tr!("{stem}.{ext} を読み込めませんでした: {e}", "Couldn't import {stem}.{ext}: {e}"))?;
            library.import(&smf, &format!("{stem}.mid"), Some(&path))
        }
        _ => Err(tr!(
            "MIDI（.mid / .midi）か MusicXML（.musicxml / .xml / .mxl）のファイルを選択してください。",
            "Choose a MIDI (.mid / .midi) or MusicXML (.musicxml / .xml / .mxl) file."
        )),
    }
}

/// The stored MIDI bytes, as a raw body (no JSON encoding).
#[tauri::command]
pub fn songs_data(library: State<'_, SongLibrary>, id: i64) -> Result<Response, String> {
    Ok(Response::new(library.read(id)?))
}

/// Record a play, and save what opening the song revealed about it.
#[tauri::command]
pub fn songs_played(
    library: State<'_, SongLibrary>,
    id: i64,
    duration: f64,
    track_count: i64,
) -> Result<(), String> {
    let db = library.lock()?;
    db.execute(
        "UPDATE songs
         SET last_played = ?2, play_count = play_count + 1,
             duration = max(duration, ?3), track_count = max(track_count, ?4)
         WHERE id = ?1",
        params![id, now(), duration.max(0.0), track_count.max(0)],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn songs_rename(library: State<'_, SongLibrary>, id: i64, title: String) -> Result<(), String> {
    let title = title.trim();
    if title.is_empty() {
        return Err(tr!("名前を入力してください。", "Enter a name."));
    }
    library
        .lock()?
        .execute("UPDATE songs SET title = ?2 WHERE id = ?1", params![id, title])
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn songs_set_favorite(library: State<'_, SongLibrary>, id: i64, favorite: bool) -> Result<(), String> {
    library
        .lock()?
        .execute("UPDATE songs SET favorite = ?2 WHERE id = ?1", params![id, favorite as i64])
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// The instruments remembered for a song, to restore when it is opened.
#[tauri::command]
pub fn songs_instruments(library: State<'_, SongLibrary>, id: i64) -> Result<Vec<SavedInstrument>, String> {
    library.instruments(id)
}

/// Remember what a track of a song plays through.
#[tauri::command]
pub fn songs_set_instrument(
    library: State<'_, SongLibrary>,
    id: i64,
    track: usize,
    choice: InstrumentChoice,
    label: String,
) -> Result<(), String> {
    library.set_instrument(id, track, &choice, &label)
}

/// Forget every remembered instrument of a song (back to auto-assignment).
#[tauri::command]
pub fn songs_clear_instruments(library: State<'_, SongLibrary>, id: i64) -> Result<(), String> {
    library.clear_instruments(id)
}

/// Remove a song from the library and delete its stored copy.
#[tauri::command]
pub fn songs_remove(library: State<'_, SongLibrary>, id: i64) -> Result<(), String> {
    let db = library.lock()?;
    let hash: Option<String> = db
        .query_row("SELECT hash FROM songs WHERE id = ?1", params![id], |r| r.get(0))
        .optional()
        .map_err(|e| e.to_string())?;
    db.execute("DELETE FROM songs WHERE id = ?1", params![id]).map_err(|e| e.to_string())?;
    db.execute("DELETE FROM song_instruments WHERE song_id = ?1", params![id]).map_err(|e| e.to_string())?;
    db.execute("DELETE FROM song_versions WHERE song_id = ?1", params![id]).map_err(|e| e.to_string())?;
    drop(db);
    if let Some(hash) = hash {
        // The row is gone either way; a leftover file is harmless.
        let _ = std::fs::remove_file(library.path_for(&hash));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn library(dir: &Path) -> SongLibrary {
        std::fs::create_dir_all(dir.join("songs")).unwrap();
        let db = Connection::open(dir.join("library.db")).unwrap();
        db.execute_batch(SCHEMA).unwrap();
        SongLibrary { db: Mutex::new(db), dir: dir.join("songs"), pending: dir.join("pending") }
    }

    #[test]
    fn imports_deduplicates_and_survives_the_original() {
        let dir = std::env::temp_dir().join(format!("otomo-songs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let lib = library(&dir);

        let a = lib.import(b"MThd-a", "Prelude.mid", Some("/Users/x/Prelude.mid")).unwrap();
        assert_eq!(a.title, "Prelude", "title defaults to the file name");
        assert_eq!(a.play_count, 0);
        assert!(a.last_played.is_none());

        // The same bytes from a new location: one entry, updated source.
        let again = lib.import(b"MThd-a", "Prelude copy.mid", Some("/Volumes/usb/Prelude.mid")).unwrap();
        assert_eq!(again.id, a.id);
        assert_eq!(again.source_path.as_deref(), Some("/Volumes/usb/Prelude.mid"));

        let b = lib.import(b"MThd-b", "Fugue.mid", None).unwrap();
        assert_ne!(b.id, a.id);

        // Reading does not depend on the original file existing.
        assert_eq!(lib.read(a.id).unwrap(), b"MThd-a");
        assert_eq!(lib.read(b.id).unwrap(), b"MThd-b");
        assert!(lib.read(9999).is_err());
        assert!(lib.import(b"", "empty.mid", None).is_err());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn edits_can_be_undone_and_redone_step_by_step() {
        let dir = std::env::temp_dir().join(format!("otomo-songs-hist-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let lib = library(&dir);
        let song = lib.import(b"MThd-v1", "Song.mid", None).unwrap();
        assert!(!song.can_undo && !song.can_redo);

        lib.commit(song.id, b"MThd-v2", "paste", 12).unwrap();
        let after = lib.commit(song.id, b"MThd-v3", "claude", 2).unwrap();
        assert!(after.can_undo && !after.can_redo);
        assert_eq!(lib.read(song.id).unwrap(), b"MThd-v3");

        // Back twice, each step naming what it took back.
        let (rec, step) = lib.step(song.id, true).unwrap();
        assert_eq!((step.kind.as_str(), step.count), ("claude", 2));
        assert!(rec.can_undo && rec.can_redo);
        assert_eq!(lib.read(song.id).unwrap(), b"MThd-v2");
        let (rec, step) = lib.step(song.id, true).unwrap();
        assert_eq!((step.kind.as_str(), step.count), ("paste", 12));
        assert!(!rec.can_undo);
        assert_eq!(lib.read(song.id).unwrap(), b"MThd-v1");
        assert!(lib.step(song.id, true).is_err(), "nothing before the original");

        // Forward again, naming what it put back.
        let (_, step) = lib.step(song.id, false).unwrap();
        assert_eq!(step.kind, "paste");
        assert_eq!(lib.read(song.id).unwrap(), b"MThd-v2");

        // A new edit after an undo drops what could have been redone.
        let rec = lib.commit(song.id, b"MThd-v4", "paste", 3).unwrap();
        assert!(!rec.can_redo);
        assert!(lib.step(song.id, false).is_err());
        lib.step(song.id, true).unwrap();
        assert_eq!(lib.read(song.id).unwrap(), b"MThd-v2");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_edit_applied_by_an_older_build_can_still_be_undone() {
        let dir = std::env::temp_dir().join(format!("otomo-songs-old-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let lib = library(&dir);
        let old = lib.import(b"MThd-old", "Old.mid", None).unwrap();
        let new = lib.import(b"MThd-new", "Tmp.mid", None).unwrap();
        // What the old "apply" left: the new file in use, the old one aside.
        {
            let db = lib.lock().unwrap();
            let hash = |id: i64| -> String { db.query_row("SELECT hash FROM songs WHERE id = ?1", [id], |r| r.get(0)).unwrap() };
            let (h_old, h_new) = (hash(old.id), hash(new.id));
            db.execute("DELETE FROM songs WHERE id = ?1", [new.id]).unwrap();
            db.execute("UPDATE songs SET hash = ?2, previous_hash = ?3 WHERE id = ?1", params![old.id, h_new, h_old])
                .unwrap();
        }
        assert!(lib.record(old.id).unwrap().can_undo);
        lib.step(old.id, true).unwrap();
        assert_eq!(lib.read(old.id).unwrap(), b"MThd-old");
        lib.step(old.id, false).unwrap();
        assert_eq!(lib.read(old.id).unwrap(), b"MThd-new");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn undo_waits_for_claudes_pending_edits() {
        let dir = std::env::temp_dir().join(format!("otomo-songs-pend-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let lib = library(&dir);
        let song = lib.import(b"MThd-1", "S.mid", None).unwrap();
        lib.commit(song.id, b"MThd-2", "paste", 1).unwrap();
        lib.stage(song.id, b"MThd-3", "Claude did something").unwrap();
        assert!(lib.step(song.id, true).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn remembers_instruments_per_song() {
        let dir = std::env::temp_dir().join(format!("otomo-songs-inst-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let lib = library(&dir);
        let a = lib.import(b"MThd-a", "a.mid", None).unwrap();
        let b = lib.import(b"MThd-b", "b.mid", None).unwrap();

        let au = InstrumentChoice::AudioUnit { id: "aumu-abcd-Efgh".into() };
        lib.set_instrument(a.id, 2, &au, "Synth").unwrap();
        lib.set_instrument(a.id, 0, &InstrumentChoice::Sf2, "").unwrap();
        // Choosing again replaces, it does not add.
        let piano = InstrumentChoice::Sfz { path: "/x/piano.sfz".into(), gain_db: 0.0 };
        lib.set_instrument(a.id, 2, &piano, "Piano").unwrap();

        let saved = lib.instruments(a.id).unwrap();
        assert_eq!(
            saved,
            vec![
                SavedInstrument { track: 0, choice: InstrumentChoice::Sf2, label: String::new() },
                SavedInstrument { track: 2, choice: piano, label: "Piano".into() },
            ]
        );
        assert!(lib.instruments(b.id).unwrap().is_empty(), "other songs are untouched");

        lib.clear_instruments(a.id).unwrap();
        assert!(lib.instruments(a.id).unwrap().is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn user_instruments_are_kept_once() {
        let dir = std::env::temp_dir().join(format!("otomo-songs-user-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let lib = library(&dir);

        let first = lib.add_user_instrument("/sfz/Cello.sfz", "Cello").unwrap();
        let again = lib.add_user_instrument("/sfz/Cello.sfz", "Cello").unwrap();
        assert_eq!(first, again);
        lib.add_user_instrument("/sfz/Harp.sfz", "Harp").unwrap();
        let names: Vec<_> = lib.user_instruments().unwrap().into_iter().map(|i| i.name).collect();
        assert_eq!(names, ["Cello", "Harp"]);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
