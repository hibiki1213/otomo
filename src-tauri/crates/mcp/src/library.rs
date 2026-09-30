//! The app's song library: the same SQLite file and the same
//! `songs/<hash>.mid` store the app writes (see `src-tauri/src/songs.rs`).
//!
//! Held open read-only, so no tool can damage what is already there. The one
//! exception is creating a song, which opens a second connection and only
//! ever inserts — everything else an edit does waits in `pending`.

use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use sha2::{Digest, Sha256};

pub struct Song {
    pub id: i64,
    pub title: String,
    pub file_name: String,
    pub hash: String,
    pub duration: f64,
    pub track_count: i64,
    pub favorite: bool,
}

pub struct Library {
    db: Connection,
    dir: PathBuf,
    root: PathBuf,
}

/// Where the app keeps its data. `OTOMO_DATA_DIR` overrides it, which is what
/// the tests use and what a second install would set.
pub fn data_dir() -> Result<PathBuf, String> {
    if let Ok(dir) = std::env::var("OTOMO_DATA_DIR") {
        return Ok(PathBuf::from(dir));
    }
    let home = std::env::var("HOME").map_err(|_| "HOME が設定されていません".to_string())?;
    Ok(PathBuf::from(home).join("Library/Application Support/io.github.hibiki1213.otomo"))
}

impl Library {
    pub fn open(dir: &Path) -> Result<Self, String> {
        let path = dir.join("library.db");
        if !path.exists() {
            return Err(format!(
                "ライブラリがまだありません（{}）。Otomo で一度 MIDI を開いてください。",
                path.display()
            ));
        }
        let db = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|e| format!("ライブラリを開けません: {e}"))?;
        Ok(Self { db, dir: dir.join("songs"), root: dir.to_path_buf() })
    }

    /// Most recently played first, favourites on top — the sidebar's order.
    pub fn list(&self, query: Option<&str>, limit: i64) -> Result<Vec<Song>, String> {
        let like = format!("%{}%", query.unwrap_or_default().trim().to_lowercase());
        let mut stmt = self
            .db
            .prepare(
                "SELECT id, title, file_name, hash, duration, track_count, favorite
                 FROM songs
                 WHERE ?1 = '%%' OR lower(title) LIKE ?1 OR lower(file_name) LIKE ?1
                 ORDER BY favorite DESC, coalesce(last_played, added_at) DESC
                 LIMIT ?2",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![like, limit], |r| {
                Ok(Song {
                    id: r.get(0)?,
                    title: r.get(1)?,
                    file_name: r.get(2)?,
                    hash: r.get(3)?,
                    duration: r.get(4)?,
                    track_count: r.get(5)?,
                    favorite: r.get::<_, i64>(6)? != 0,
                })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<rusqlite::Result<Vec<_>>>().map_err(|e| e.to_string())
    }

    pub fn get(&self, id: i64) -> Result<Song, String> {
        self.db
            .query_row(
                "SELECT id, title, file_name, hash, duration, track_count, favorite
                 FROM songs WHERE id = ?1",
                params![id],
                |r| {
                    Ok(Song {
                        id: r.get(0)?,
                        title: r.get(1)?,
                        file_name: r.get(2)?,
                        hash: r.get(3)?,
                        duration: r.get(4)?,
                        track_count: r.get(5)?,
                        favorite: r.get::<_, i64>(6)? != 0,
                    })
                },
            )
            .optional()
            .map_err(|e| e.to_string())?
            .ok_or_else(|| format!("song_id {id} はライブラリにありません。list_songs で確認してください。"))
    }

    pub fn bytes(&self, song: &Song) -> Result<Vec<u8>, String> {
        std::fs::read(self.dir.join(format!("{}.mid", song.hash)))
            .map_err(|e| format!("「{}」のファイルを読めません: {e}", song.title))
    }

    /// Add a new song to the library. Purely additive: nothing existing is
    /// touched, which is why this is the only write the server makes.
    pub fn create(&self, title: &str, bytes: &[u8]) -> Result<Song, String> {
        let title = title.trim();
        if title.is_empty() {
            return Err("曲名を指定してください。".into());
        }
        let hash = format!("{:x}", Sha256::digest(bytes));
        std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
        let path = self.dir.join(format!("{hash}.mid"));
        let tmp = path.with_extension("part");
        std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;

        let db = Connection::open(self.root.join("library.db"))
            .map_err(|e| format!("ライブラリに書き込めません: {e}"))?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        db.execute(
            "INSERT INTO songs (title, file_name, hash, bytes, added_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(hash) DO UPDATE SET title = excluded.title",
            params![title, format!("{title}.mid"), hash, bytes.len() as i64, now],
        )
        .map_err(|e| format!("ライブラリに書き込めません: {e}"))?;
        let id: i64 = db
            .query_row("SELECT id FROM songs WHERE hash = ?1", params![hash], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        self.get(id)
    }
}
