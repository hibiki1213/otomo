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

/// Opened once at startup and shared by every command.
pub struct SongLibrary {
    db: Mutex<Connection>,
    dir: PathBuf,
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
}

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS songs (
    id          INTEGER PRIMARY KEY,
    title       TEXT    NOT NULL,
    file_name   TEXT    NOT NULL,
    hash        TEXT    NOT NULL UNIQUE,
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
";

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
        Ok(Self { db: Mutex::new(db), dir })
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

const COLUMNS: &str = "id, title, file_name, bytes, source_path, duration, track_count,
                       added_at, last_played, play_count, favorite";

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
    })
}

fn get(db: &Connection, id: i64) -> Result<SongRecord, String> {
    db.query_row(&format!("SELECT {COLUMNS} FROM songs WHERE id = ?1"), params![id], row)
        .map_err(|e| e.to_string())
}

/* ---------- commands ---------- */

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

/// Copy a MIDI file on disk into the library.
#[tauri::command]
pub async fn songs_import(library: State<'_, SongLibrary>, path: String) -> Result<SongRecord, String> {
    let p = PathBuf::from(&path);
    if !p.extension().is_some_and(|e| e.eq_ignore_ascii_case("mid") || e.eq_ignore_ascii_case("midi")) {
        return Err("MIDI ファイル（.mid / .midi）を選択してください。".into());
    }
    let data = tokio::fs::read(&p).await.map_err(|e| e.to_string())?;
    let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("song.mid").to_owned();
    library.import(&data, &name, Some(&path))
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
        return Err("名前を入力してください。".into());
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

/// Remove a song from the library and delete its stored copy.
#[tauri::command]
pub fn songs_remove(library: State<'_, SongLibrary>, id: i64) -> Result<(), String> {
    let db = library.lock()?;
    let hash: Option<String> = db
        .query_row("SELECT hash FROM songs WHERE id = ?1", params![id], |r| r.get(0))
        .optional()
        .map_err(|e| e.to_string())?;
    db.execute("DELETE FROM songs WHERE id = ?1", params![id]).map_err(|e| e.to_string())?;
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
        SongLibrary { db: Mutex::new(db), dir: dir.join("songs") }
    }

    #[test]
    fn imports_deduplicates_and_survives_the_original() {
        let dir = std::env::temp_dir().join(format!("midiss-songs-{}", std::process::id()));
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
}
