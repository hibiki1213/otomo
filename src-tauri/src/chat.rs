//! Talking to Claude from inside the app.
//!
//! Each question starts a headless `claude -p` whose tools are this app's own
//! MCP server — the same `otomo-mcp` Claude Desktop uses — and web search, so
//! an existing piece is looked up rather than recalled. So an edit made
//! from the chat panel arrives exactly like one made from Claude Desktop: it is
//! staged, the banner appears, and nothing is kept until someone presses Apply.
//! Playback works the same way, through the control socket.
//!
//! The child runs with `--output-format stream-json`; text, tool calls and
//! tool results are forwarded to the UI as they happen, so a long composition
//! is something you watch (and can stop) rather than a spinner.
//!
//! Conversations live in the library database. Follow-ups reuse the Claude
//! session through `--resume`, which needs the same working directory every
//! time — hence the fixed `chat/` folder under the app's data.

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::i18n::tr;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS chat_conv (
    id          INTEGER PRIMARY KEY,
    title       TEXT    NOT NULL,
    session_id  TEXT,
    model       TEXT,
    effort      TEXT,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS chat_msg (
    id          INTEGER PRIMARY KEY,
    conv_id     INTEGER NOT NULL REFERENCES chat_conv(id) ON DELETE CASCADE,
    role        TEXT    NOT NULL,
    parts       TEXT    NOT NULL,
    created_at  INTEGER NOT NULL,
    model       TEXT,
    effort      TEXT,
    cost_usd    REAL,
    duration_ms INTEGER,
    error       TEXT,
    stopped     INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS chat_msg_conv ON chat_msg (conv_id, id);
";

/// The tools the model may call: this app's MCP server, and Claude Code's
/// own web tools for finding the music.
const SERVER_KEY: &str = "otomo";
const WEB_TOOLS: &str = "WebSearch,WebFetch";

/// What `claude --effort` accepts, lowest first.
const EFFORTS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];

/// A level `claude` accepts, or None (= the model's own default).
fn effort_level(effort: Option<String>) -> Option<String> {
    effort.filter(|e| EFFORTS.contains(&e.as_str()))
}

/// Haiku has no effort control, so asking for one only risks a failed run.
fn takes_effort(model: Option<&str>) -> bool {
    !model.is_some_and(|m| m.contains("haiku"))
}

pub struct ChatState {
    db: Mutex<Connection>,
    /// Working directory for every child, so `--resume` finds its session.
    cwd: PathBuf,
    data_dir: PathBuf,
    runs: Arc<Mutex<Runs>>,
}

#[derive(Default)]
struct Runs {
    children: HashMap<i64, Child>,
    /// Conversations someone pressed Stop on, so the ending reads as that
    /// rather than as a failure.
    stopped: HashSet<i64>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Conversation {
    pub id: i64,
    pub title: String,
    /// None = whatever `claude` picks on its own.
    pub model: Option<String>,
    /// None = the model's default effort.
    pub effort: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

const CONV_COLUMNS: &str = "id, title, model, effort, created_at, updated_at";

fn conversation(r: &rusqlite::Row) -> rusqlite::Result<Conversation> {
    Ok(Conversation {
        id: r.get(0)?,
        title: r.get(1)?,
        model: r.get(2)?,
        effort: r.get(3)?,
        created_at: r.get(4)?,
        updated_at: r.get(5)?,
    })
}

/// One piece of a message, in the order it happened.
#[derive(Serialize, Deserialize, Clone)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Part {
    Text { text: String },
    #[serde(rename_all = "camelCase")]
    Tool {
        id: String,
        /// Without the `mcp__otomo__` prefix.
        name: String,
        input: Value,
        /// "running", "ok" or "error".
        status: String,
        result: Option<String>,
    },
    /// A piano-roll range the user attached: the chip it showed as, and the
    /// description Claude was given.
    Selection { label: String, context: String },
}

/// A piano-roll range attached to a message.
#[derive(Deserialize)]
pub struct SelectionInput {
    pub label: String,
    pub context: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub id: i64,
    pub role: String,
    pub parts: Vec<Part>,
    pub created_at: i64,
    pub model: Option<String>,
    /// The level asked for, not one `claude` reports back.
    pub effort: Option<String>,
    pub cost_usd: Option<f64>,
    pub duration_ms: Option<i64>,
    pub error: Option<String>,
    pub stopped: bool,
}

/// What the app has open, so "this song" means something.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SongContext {
    pub id: i64,
    pub title: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ChatStatus {
    pub claude_path: Option<String>,
    pub server_path: Option<String>,
    /// Why the panel cannot be used right now, if it cannot.
    pub blocked: Option<String>,
}

/* ---------- events ---------- */

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Delta {
    conv_id: i64,
    text: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ToolEvent {
    conv_id: i64,
    id: String,
    name: Option<String>,
    input: Option<Value>,
    status: Option<String>,
    result: Option<String>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Done {
    conv_id: i64,
    message: Message,
}

fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn short(s: &str, max: usize) -> String {
    let t = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if t.chars().count() <= max {
        return t;
    }
    t.chars().take(max).collect::<String>() + "…"
}

/// The PATH to search and to hand the child. An app started from the Finder
/// gets only `/usr/bin:/bin:…`, without what `.zshrc` adds (nvm, Volta, npm's
/// global bin…), so the login shell is asked once and its answer goes first.
/// The npm build of `claude` is `#!/usr/bin/env node`, so this is also how its
/// node is found.
fn search_path() -> &'static str {
    static PATH: OnceLock<String> = OnceLock::new();
    PATH.get_or_init(|| {
        let mut dirs: Vec<PathBuf> = Vec::new();
        if let Some(login) = login_shell_path() {
            dirs.extend(std::env::split_paths(&login));
        }
        if let Some(own) = std::env::var_os("PATH") {
            dirs.extend(std::env::split_paths(&own));
        }
        let mut seen = HashSet::new();
        dirs.retain(|d| !d.as_os_str().is_empty() && seen.insert(d.clone()));
        std::env::join_paths(dirs).map(|p| p.to_string_lossy().into_owned()).unwrap_or_default()
    })
}

/// PATH as `$SHELL -ilc` sees it. An interactive shell may print things while
/// it starts, so the value comes between markers; a shell setup that hangs, or
/// leaves something holding the pipe open, costs five seconds and no more.
fn login_shell_path() -> Option<String> {
    const MARK: &str = "__OTOMO_PATH__";
    let shell = std::env::var("SHELL").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "/bin/zsh".into());
    let mut child = Command::new(shell)
        .args(["-ilc", &format!("printf '{MARK}%s{MARK}' \"$PATH\"")])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut out = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            match std::io::Read::read(&mut stdout, &mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => out.extend_from_slice(&buf[..n]),
            }
            let text = String::from_utf8_lossy(&out);
            if let Some((_, rest)) = text.split_once(MARK) {
                if let Some((path, _)) = rest.split_once(MARK) {
                    let _ = tx.send(path.to_string());
                    return;
                }
            }
        }
    });
    let path = rx.recv_timeout(std::time::Duration::from_secs(5)).ok();
    let _ = child.kill();
    let _ = child.wait();
    path.filter(|p| !p.is_empty())
}

/// Where Claude Code is: the official installer's and Homebrew's places
/// first, then the login shell's PATH, then where npm and friends usually put
/// it, for people whose shell doesn't add those to PATH.
fn claude_path() -> Option<PathBuf> {
    let home = PathBuf::from(std::env::var("HOME").unwrap_or_default());
    let known = [
        home.join(".local/bin/claude"),
        home.join(".claude/local/claude"),
        PathBuf::from("/opt/homebrew/bin/claude"),
        PathBuf::from("/usr/local/bin/claude"),
    ];
    if let Some(p) = known.into_iter().find(|p| p.is_file()) {
        return Some(p);
    }
    if let Some(p) = std::env::split_paths(search_path()).map(|d| d.join("claude")).find(|p| p.is_file()) {
        return Some(p);
    }
    let mut extra = vec![
        home.join(".npm-global/bin/claude"),
        home.join(".volta/bin/claude"),
        home.join(".bun/bin/claude"),
        home.join(".yarn/bin/claude"),
    ];
    if let Ok(versions) = std::fs::read_dir(home.join(".nvm/versions/node")) {
        let names = versions.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        let dir = home.join(".nvm/versions/node");
        extra.extend(newest_first(names).into_iter().map(|v| dir.join(v).join("bin/claude")));
    }
    extra.into_iter().find(|p| p.is_file())
}

/// nvm's version folders (`v20.11.1`), newest first by number, so v9 comes
/// after v20. Anything that isn't a version goes last.
fn newest_first(mut names: Vec<String>) -> Vec<String> {
    fn version(name: &str) -> Option<Vec<u64>> {
        name.strip_prefix('v')?.split('.').map(|n| n.parse().ok()).collect()
    }
    names.sort_by(|a, b| version(b).cmp(&version(a)).then_with(|| a.cmp(b)));
    names
}

/// The child's PATH: `claude`'s own folder (where an npm install keeps its
/// node), then the search path.
fn child_path(claude: &Path) -> String {
    let mut dirs: Vec<PathBuf> = claude.parent().map(Path::to_path_buf).into_iter().collect();
    dirs.extend(std::env::split_paths(search_path()));
    std::env::join_paths(dirs).map(|p| p.to_string_lossy().into_owned()).unwrap_or_default()
}

/// `claude`, ready to run as the app's child: with a PATH it can find node by,
/// and none of the parent's `CLAUDE*` variables — started from inside a Claude
/// Code session (as in development), it would otherwise inherit that session's
/// identity and act as part of it.
fn claude_command(claude: &Path) -> Command {
    let mut cmd = Command::new(claude);
    cmd.env("PATH", child_path(claude));
    for (k, _) in std::env::vars() {
        if k.starts_with("CLAUDE") {
            cmd.env_remove(&k);
        }
    }
    cmd.env("CLAUDE_CODE_ENTRYPOINT", "otomo-chat");
    cmd
}

/// English like everything else Claude reads; the reply follows the person.
fn system_prompt() -> String {
    let ui = if crate::i18n::english() { "English" } else { "Japanese" };
    format!(
        "You are the assistant built into Otomo, an app for composing and playing MIDI. \
The user is talking to you from the app's right-hand panel, with the song and its piano roll on screen.\n\n\
- Look into, create, fix and play songs with the Otomo tools.\n\
- When asked for a piece that already exists (national anthems, classical works, folk songs, standards, well-known songs), \
don't write it from memory. First search for a score file with WebSearch, and if you find one, import it with download. \
Look in this order: MusicXML (.musicxml / .mxl — the score itself, the most accurate), MIDI, ABC notation, then PDF or image scores. \
Prefer public-domain and freely licensed sources such as Wikimedia Commons, the Mutopia Project, IMSLP and abcnotation.com.\n\
- After importing, check its contents with get_song and read_bars before rearranging it for the requested instrumentation or arrangement. \
Say in your reply where you got it.\n\
- If only a PDF or image score exists, view its pages with download and transcribe them. Say which passages you were unsure of reading.\n\
- If none of those turn up, look up the key, time signature, tempo, form and melody with WebFetch before writing. \
If any part still relied on memory, say so.\n\
- Edits are not saved right away. They appear in the app as pending edits; they are kept once the user has previewed them and pressed Apply. \
After editing, say in a sentence what you changed.\n\
- When you want the user to hear something, play it with play (ranges and looping work too).\n\
- Each message starts with [App state]. \"This song\" and \"the current song\" mean the song named there.\n\
- A message may carry [Selection N]: a time × pitch range the user drew on the piano roll. \
\"Here\", \"this part\" and \"this register\" mean it; when there are several, they are told apart by number. \
When fixing, change mainly what is inside the range, and touch what is outside it (neighbouring bars, notes outside the pitch range) only as much as the music needs. \
write_bars rewrites whole bars, so write notes outside the range in the same bars back exactly as read_bars gave them. \
If the range has no notes, you may write new ones into it (if no track is implied, pick one that fits or create one).\n\
- Reply briefly, in the language the user writes in (the app is set to {ui}). Musical terms are fine; long explanations are not.\n"
    )
}

impl ChatState {
    pub fn open(app: &AppHandle) -> Result<Self, String> {
        let root = app.path().app_data_dir().map_err(|e| e.to_string())?;
        let cwd = root.join("chat");
        std::fs::create_dir_all(&cwd).map_err(|e| e.to_string())?;
        let db = Connection::open(root.join("library.db")).map_err(|e| e.to_string())?;
        db.execute_batch(
            "PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 3000;",
        )
        .map_err(|e| e.to_string())?;
        db.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
        add_column(&db, "chat_conv", "effort", "TEXT")?;
        add_column(&db, "chat_msg", "effort", "TEXT")?;
        // Asking the login shell for PATH can take a few seconds; get it done
        // before the panel needs it.
        std::thread::spawn(|| {
            search_path();
        });
        Ok(Self { db: Mutex::new(db), cwd, data_dir: root, runs: Default::default() })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.db.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Columns added after the tables first shipped; `CREATE TABLE IF NOT EXISTS`
/// leaves an older database without them.
fn add_column(db: &Connection, table: &str, column: &str, decl: &str) -> Result<(), String> {
    let present: i64 = db
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info(?1) WHERE name = ?2",
            params![table, column],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    if present == 0 {
        db.execute_batch(&format!("ALTER TABLE {table} ADD COLUMN {column} {decl}"))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn load_messages(db: &Connection, conv_id: i64) -> Result<Vec<Message>, String> {
    let mut stmt = db
        .prepare(
            "SELECT id, role, parts, created_at, model, effort, cost_usd, duration_ms, error, stopped
             FROM chat_msg WHERE conv_id = ?1 ORDER BY id",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![conv_id], |r| {
            let parts: String = r.get(2)?;
            Ok(Message {
                id: r.get(0)?,
                role: r.get(1)?,
                parts: serde_json::from_str(&parts).unwrap_or_default(),
                created_at: r.get(3)?,
                model: r.get(4)?,
                effort: r.get(5)?,
                cost_usd: r.get(6)?,
                duration_ms: r.get(7)?,
                error: r.get(8)?,
                stopped: r.get::<_, i64>(9)? != 0,
            })
        })
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<_, _>>().map_err(|e| e.to_string())
}

fn add_message(db: &Connection, conv_id: i64, m: &Message) -> Result<i64, String> {
    let parts = serde_json::to_string(&m.parts).map_err(|e| e.to_string())?;
    db.execute(
        "INSERT INTO chat_msg (conv_id, role, parts, created_at, model, effort, cost_usd, duration_ms, error, stopped)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        params![
            conv_id,
            m.role,
            parts,
            m.created_at,
            m.model,
            m.effort,
            m.cost_usd,
            m.duration_ms,
            m.error,
            m.stopped as i64
        ],
    )
    .map_err(|e| e.to_string())?;
    db.execute("UPDATE chat_conv SET updated_at = ?2 WHERE id = ?1", params![conv_id, now()])
        .map_err(|e| e.to_string())?;
    Ok(db.last_insert_rowid())
}

/* ---------- commands ---------- */

/// Async so that the first call, which may wait on the login shell for up to
/// five seconds, doesn't hold up the window.
#[tauri::command]
pub async fn chat_status() -> ChatStatus {
    status()
}

fn status() -> ChatStatus {
    let claude = claude_path();
    let server = crate::mcp_setup::server_path();
    let blocked = if claude.is_none() {
        Some(tr!(
            "Claude Code が見つかりません。ターミナルで `curl -fsSL https://claude.ai/install.sh | bash` を実行してインストールし、`claude` を一度起動してログインしてから、Otomo に戻ってください。",
            "Claude Code wasn't found. Install it by running `curl -fsSL https://claude.ai/install.sh | bash` in Terminal, start `claude` once to log in, then come back to Otomo."
        ))
    } else if server.is_none() {
        Some(crate::mcp_setup::server_missing())
    } else {
        None
    };
    ChatStatus {
        claude_path: claude.map(|p| p.display().to_string()),
        server_path: server.map(|p| p.display().to_string()),
        blocked,
    }
}

#[tauri::command]
pub fn chat_list(chat: State<'_, ChatState>) -> Result<Vec<Conversation>, String> {
    let db = chat.lock();
    let mut stmt = db
        .prepare(&format!(
            "SELECT {CONV_COLUMNS} FROM chat_conv ORDER BY updated_at DESC, id DESC"
        ))
        .map_err(|e| e.to_string())?;
    let rows = stmt.query_map([], conversation).map_err(|e| e.to_string())?;
    rows.collect::<Result<_, _>>().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn chat_messages(chat: State<'_, ChatState>, conv_id: i64) -> Result<Vec<Message>, String> {
    load_messages(&chat.lock(), conv_id)
}

/// Conversations with an answer still coming, so a remounted panel can pick
/// the stream up again.
#[tauri::command]
pub fn chat_running(chat: State<'_, ChatState>) -> Vec<i64> {
    let runs = chat.runs.lock().unwrap_or_else(|e| e.into_inner());
    runs.children.keys().copied().collect()
}

#[tauri::command]
pub fn chat_delete(chat: State<'_, ChatState>, conv_id: i64) -> Result<(), String> {
    stop(&chat, conv_id);
    let db = chat.lock();
    db.execute("DELETE FROM chat_msg WHERE conv_id = ?1", params![conv_id])
        .map_err(|e| e.to_string())?;
    db.execute("DELETE FROM chat_conv WHERE id = ?1", params![conv_id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn chat_set_model(
    chat: State<'_, ChatState>,
    conv_id: i64,
    model: Option<String>,
) -> Result<(), String> {
    let model = model.filter(|m| !m.trim().is_empty());
    chat.lock()
        .execute("UPDATE chat_conv SET model = ?2 WHERE id = ?1", params![conv_id, model])
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn chat_set_effort(
    chat: State<'_, ChatState>,
    conv_id: i64,
    effort: Option<String>,
) -> Result<(), String> {
    chat.lock()
        .execute(
            "UPDATE chat_conv SET effort = ?2 WHERE id = ?1",
            params![conv_id, effort_level(effort)],
        )
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn chat_stop(chat: State<'_, ChatState>, conv_id: i64) {
    stop(&chat, conv_id);
}

fn stop(chat: &ChatState, conv_id: i64) {
    let mut runs = chat.runs.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(child) = runs.children.get_mut(&conv_id) {
        let _ = child.kill();
        runs.stopped.insert(conv_id);
    }
}

/// The `claude auth login` under way, so it can be cancelled and not started
/// twice.
static LOGIN: Mutex<Option<Child>> = Mutex::new(None);

/// How long a sign-in left open in the browser is waited for.
const LOGIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10 * 60);

/// Sign Claude Code in again, for when a run fails on an expired login.
/// `claude auth login` opens the browser itself and exits once the sign-in
/// comes back; any link it prints goes out as `chat://login-url`, in case the
/// browser didn't open. Ok(false) means it was cancelled.
#[tauri::command]
pub async fn chat_login(app: AppHandle) -> Result<bool, String> {
    tauri::async_runtime::spawn_blocking(move || login(&app))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn chat_login_cancel() {
    if let Some(child) = LOGIN.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
        let _ = child.kill();
    }
}

fn login(app: &AppHandle) -> Result<bool, String> {
    let claude = claude_path().ok_or_else(|| status().blocked.unwrap_or_default())?;
    let output = Arc::new(Mutex::new(String::new()));
    {
        let mut slot = LOGIN.lock().unwrap_or_else(|e| e.into_inner());
        if slot.is_some() {
            return Err(tr!("ログインはもう始まっています", "Signing in has already started"));
        }
        let mut child = claude_command(&claude)
            .args(["auth", "login"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| tr!("Claude を起動できませんでした: {e}", "Couldn't start Claude: {e}"))?;
        let pipes: [Option<Box<dyn std::io::Read + Send>>; 2] = [
            child.stdout.take().map(|p| Box::new(p) as _),
            child.stderr.take().map(|p| Box::new(p) as _),
        ];
        for pipe in pipes.into_iter().flatten() {
            let (app, output) = (app.clone(), output.clone());
            std::thread::spawn(move || {
                for line in BufReader::new(pipe).lines().map_while(Result::ok) {
                    if let Some(url) = line.split_whitespace().find(|w| w.starts_with("https://")) {
                        let _ = app.emit("chat://login-url", url.to_string());
                    }
                    let mut out = output.lock().unwrap_or_else(|e| e.into_inner());
                    out.push_str(&line);
                    out.push('\n');
                }
            });
        }
        *slot = Some(child);
    }

    let started = std::time::Instant::now();
    let exit = loop {
        let mut slot = LOGIN.lock().unwrap_or_else(|e| e.into_inner());
        let child = slot.as_mut().expect("only this function clears it");
        match child.try_wait() {
            Ok(Some(exit)) => {
                *slot = None;
                break Some(exit);
            }
            Ok(None) if started.elapsed() < LOGIN_TIMEOUT => {}
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                *slot = None;
                break None;
            }
        }
        drop(slot);
        std::thread::sleep(std::time::Duration::from_millis(200));
    };
    match exit {
        Some(exit) if exit.success() => Ok(true),
        // Killed: Cancel was pressed.
        Some(exit) if exit.code().is_none() => Ok(false),
        Some(_) => {
            let out = output.lock().unwrap_or_else(|e| e.into_inner());
            Err(if out.trim().is_empty() {
                tr!("ログインできませんでした", "Couldn't sign in")
            } else {
                short(&out, 300)
            })
        }
        None => Err(tr!(
            "ログインが時間内に終わりませんでした。もう一度お試しください。",
            "Signing in took too long. Please try again."
        )),
    }
}

/// Ask something. Returns the conversation id as soon as the child is
/// running; the answer arrives as `chat://…` events and ends with
/// `chat://done`.
#[tauri::command]
pub fn chat_send(
    app: AppHandle,
    chat: State<'_, ChatState>,
    conv_id: Option<i64>,
    text: String,
    song: Option<SongContext>,
    selections: Option<Vec<SelectionInput>>,
    model: Option<String>,
    effort: Option<String>,
) -> Result<Conversation, String> {
    let text = text.trim().to_string();
    let selections = selections.unwrap_or_default();
    if text.is_empty() && selections.is_empty() {
        return Err(tr!("メッセージが空です", "The message is empty"));
    }
    let status = status();
    if let Some(reason) = status.blocked {
        return Err(reason);
    }
    let claude = PathBuf::from(status.claude_path.expect("checked above"));
    let server = status.server_path.expect("checked above");
    let model = model.filter(|m| !m.trim().is_empty());
    let effort = effort_level(effort);

    let db = chat.lock();
    let conv_id = match conv_id {
        Some(id) => id,
        None => {
            let t = now();
            db.execute(
                "INSERT INTO chat_conv (title, model, effort, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?4)",
                params![
                    short(if text.is_empty() { &selections[0].label } else { &text }, 40),
                    model,
                    effort,
                    t
                ],
            )
            .map_err(|e| e.to_string())?;
            db.last_insert_rowid()
        }
    };
    if chat.runs.lock().unwrap_or_else(|e| e.into_inner()).children.contains_key(&conv_id) {
        return Err(tr!("まだ返事を待っています", "Still waiting for the reply"));
    }
    let (session_id, conv_model, conv_effort): (Option<String>, Option<String>, Option<String>) = db
        .query_row(
            "SELECT session_id, model, effort FROM chat_conv WHERE id = ?1",
            params![conv_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .ok_or_else(|| tr!("会話が見つかりません", "Conversation not found"))?;
    let model = model.or(conv_model);
    let effort = effort.or(conv_effort).filter(|_| takes_effort(model.as_deref()));
    add_message(
        &db,
        conv_id,
        &Message {
            id: 0,
            role: "user".into(),
            parts: selections
                .iter()
                .map(|s| Part::Selection { label: s.label.clone(), context: s.context.clone() })
                .chain((!text.is_empty()).then(|| Part::Text { text: text.clone() }))
                .collect(),
            created_at: now(),
            model: None,
            effort: None,
            cost_usd: None,
            duration_ms: None,
            error: None,
            stopped: false,
        },
    )?;
    drop(db);

    let context = match &song {
        Some(s) => format!("[App state] Open song: \"{}\" (song_id: {})", s.title, s.id),
        None => "[App state] No song is open".to_string(),
    };
    let mut prompt = context;
    for s in &selections {
        prompt.push_str("\n\n");
        prompt.push_str(&s.context);
    }
    prompt.push_str("\n\n");
    prompt.push_str(if text.is_empty() {
        "(No instruction was written. Look at the selection and briefly suggest how it could be improved.)"
    } else {
        text.as_str()
    });

    // The data directory is passed explicitly so the server always works on
    // the library this app is showing.
    let mcp = json!({
        "mcpServers": {
            SERVER_KEY: {
                "command": server,
                "env": { "OTOMO_DATA_DIR": chat.data_dir.display().to_string() }
            }
        }
    });

    let mut cmd = claude_command(&claude);
    cmd.arg("-p")
        .args(["--output-format", "stream-json", "--include-partial-messages", "--verbose"])
        // No shell and no file access: Otomo, plus the web to look a piece
        // up instead of writing it from memory.
        .args(["--tools", WEB_TOOLS])
        .args(["--strict-mcp-config", "--mcp-config"])
        .arg(mcp.to_string())
        .args(["--allowedTools", &format!("mcp__{SERVER_KEY},{WEB_TOOLS}")])
        // The user's settings carry hooks and permissions meant for their
        // coding sessions, not for this panel.
        .args(["--setting-sources", ""])
        .args(["--append-system-prompt", &system_prompt()])
        .current_dir(&chat.cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(id) = &session_id {
        cmd.args(["--resume", id]);
    }
    if let Some(m) = &model {
        cmd.args(["--model", m]);
    }
    if let Some(e) = &effort {
        cmd.args(["--effort", e]);
    }

    let mut child = cmd.spawn().map_err(|e| tr!("Claude を起動できませんでした: {e}", "Couldn't start Claude: {e}"))?;
    let mut stdin = child.stdin.take().ok_or("couldn't open stdin")?;
    stdin
        .write_all(prompt.as_bytes())
        .map_err(|e| tr!("メッセージを渡せませんでした: {e}", "Couldn't pass the message on: {e}"))?;
    drop(stdin);
    let stdout = child.stdout.take().ok_or("couldn't open stdout")?;
    let stderr = child.stderr.take();

    // Drain stderr on its own thread; a full pipe would stall the child.
    let errbuf = Arc::new(Mutex::new(String::new()));
    if let Some(err) = stderr {
        let errbuf = errbuf.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(err).lines().map_while(Result::ok) {
                let mut b = errbuf.lock().unwrap_or_else(|e| e.into_inner());
                b.push_str(&line);
                b.push('\n');
            }
        });
    }

    let runs = chat.runs.clone();
    {
        let mut r = runs.lock().unwrap_or_else(|e| e.into_inner());
        r.stopped.remove(&conv_id);
        r.children.insert(conv_id, child);
    }

    let app2 = app.clone();
    std::thread::spawn(move || {
        let outcome = Outcome { effort, ..pump(&app2, conv_id, stdout) };
        finish(&app2, &runs, conv_id, outcome, &errbuf.lock().unwrap_or_else(|e| e.into_inner()));
    });

    chat.lock()
        .query_row(
            &format!("SELECT {CONV_COLUMNS} FROM chat_conv WHERE id = ?1"),
            params![conv_id],
            conversation,
        )
        .map_err(|e| e.to_string())
}

/* ---------- streaming ---------- */

#[derive(Default)]
struct Outcome {
    parts: Vec<Part>,
    model: Option<String>,
    effort: Option<String>,
    cost_usd: Option<f64>,
    duration_ms: Option<i64>,
    error: Option<String>,
    finished: bool,
}

fn tool_name(full: &str) -> String {
    full.strip_prefix(&format!("mcp__{SERVER_KEY}__")).unwrap_or(full).to_string()
}

/// The text of a tool result, which comes either as a string or as blocks.
fn result_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .filter_map(|b| b.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// Read the NDJSON stream, forwarding what happens and building the message
/// that gets saved.
fn pump(app: &AppHandle, conv_id: i64, stdout: ChildStdout) -> Outcome {
    let mut out = Outcome::default();
    let tool_event = |id: &str| ToolEvent {
        conv_id,
        id: id.to_string(),
        name: None,
        input: None,
        status: None,
        result: None,
    };

    for line in BufReader::new(stdout).lines().map_while(Result::ok) {
        let Ok(v) = serde_json::from_str::<Value>(line.trim()) else { continue };
        match v.get("type").and_then(Value::as_str).unwrap_or("") {
            // The session id comes first. Keep it straight away, so a
            // follow-up can resume even if this run is stopped.
            "system" if v.get("subtype").and_then(Value::as_str) == Some("init") => {
                if let Some(sid) = v.get("session_id").and_then(Value::as_str) {
                    if let Some(chat) = app.try_state::<ChatState>() {
                        let _ = chat.lock().execute(
                            "UPDATE chat_conv SET session_id = ?2 WHERE id = ?1",
                            params![conv_id, sid],
                        );
                    }
                }
            }
            "stream_event" => {
                let Some(ev) = v.get("event") else { continue };
                match ev.get("type").and_then(Value::as_str).unwrap_or("") {
                    "message_start" => {
                        if let Some(m) = ev.pointer("/message/model").and_then(Value::as_str) {
                            out.model = Some(m.to_string());
                        }
                    }
                    "content_block_start" => {
                        let block = &ev["content_block"];
                        if block.get("type").and_then(Value::as_str) == Some("tool_use") {
                            let id = block["id"].as_str().unwrap_or_default().to_string();
                            let name = tool_name(block["name"].as_str().unwrap_or_default());
                            out.parts.push(Part::Tool {
                                id: id.clone(),
                                name: name.clone(),
                                input: json!({}),
                                status: "running".into(),
                                result: None,
                            });
                            let _ = app.emit(
                                "chat://tool",
                                ToolEvent { name: Some(name), status: Some("running".into()), ..tool_event(&id) },
                            );
                        }
                    }
                    "content_block_delta" => {
                        let delta = &ev["delta"];
                        if delta.get("type").and_then(Value::as_str) == Some("text_delta") {
                            let t = delta["text"].as_str().unwrap_or_default();
                            if t.is_empty() {
                                continue;
                            }
                            match out.parts.last_mut() {
                                Some(Part::Text { text }) => text.push_str(t),
                                _ => out.parts.push(Part::Text { text: t.to_string() }),
                            }
                            let _ = app.emit("chat://delta", Delta { conv_id, text: t.to_string() });
                        }
                    }
                    _ => {}
                }
            }
            // Whole blocks, after the fact: the one place a tool's full input
            // is available.
            "assistant" => {
                let blocks = v.pointer("/message/content").and_then(Value::as_array);
                for b in blocks.into_iter().flatten() {
                    if b.get("type").and_then(Value::as_str) != Some("tool_use") {
                        continue;
                    }
                    let id = b["id"].as_str().unwrap_or_default();
                    for p in out.parts.iter_mut() {
                        if let Part::Tool { id: pid, input, .. } = p {
                            if pid == id {
                                *input = b["input"].clone();
                            }
                        }
                    }
                    let _ = app.emit(
                        "chat://tool",
                        ToolEvent { input: Some(b["input"].clone()), ..tool_event(id) },
                    );
                }
            }
            "user" => {
                let blocks = v.pointer("/message/content").and_then(Value::as_array);
                for b in blocks.into_iter().flatten() {
                    if b.get("type").and_then(Value::as_str) != Some("tool_result") {
                        continue;
                    }
                    let id = b["tool_use_id"].as_str().unwrap_or_default();
                    let failed = b.get("is_error").and_then(Value::as_bool).unwrap_or(false);
                    let text = result_text(b.get("content"));
                    let state = if failed { "error" } else { "ok" };
                    for p in out.parts.iter_mut() {
                        if let Part::Tool { id: pid, status, result, .. } = p {
                            if pid == id {
                                *status = state.into();
                                *result = Some(text.clone());
                            }
                        }
                    }
                    let _ = app.emit(
                        "chat://tool",
                        ToolEvent {
                            status: Some(state.into()),
                            result: Some(text),
                            ..tool_event(id)
                        },
                    );
                }
            }
            "result" => {
                out.finished = true;
                out.duration_ms = v.get("duration_ms").and_then(Value::as_i64);
                out.cost_usd = v.get("total_cost_usd").and_then(Value::as_f64);
                if v.get("is_error").and_then(Value::as_bool).unwrap_or(false) {
                    let msg = v
                        .get("result")
                        .and_then(Value::as_str)
                        .or_else(|| v.get("subtype").and_then(Value::as_str))
                        .map(str::to_string)
                        .unwrap_or_else(|| tr!("不明なエラー", "Unknown error"));
                    out.error = Some(msg);
                }
                // When nothing streamed (one-block answers), the final text
                // is only here.
                if out.parts.is_empty() {
                    if let Some(r) = v.get("result").and_then(Value::as_str).filter(|r| !r.is_empty()) {
                        if out.error.is_none() {
                            out.parts.push(Part::Text { text: r.to_string() });
                        }
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// Whether `claude` refused the command line itself: an older version that
/// doesn't know a flag this app passes (`--effort`, `--tools`…).
fn too_old(stderr: &str) -> bool {
    let s = stderr.to_ascii_lowercase();
    s.contains("unknown option") || s.contains("unknown argument") || s.contains("unexpected argument")
}

/// Reap the child, save what was said, and tell the UI it is over.
fn finish(app: &AppHandle, runs: &Mutex<Runs>, conv_id: i64, mut out: Outcome, stderr: &str) {
    let stopped = {
        let mut r = runs.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(mut child) = r.children.remove(&conv_id) {
            let _ = child.wait();
        }
        r.stopped.remove(&conv_id)
    };

    // A tool that never answered was cut off; it should not read as running.
    for p in out.parts.iter_mut() {
        if let Part::Tool { status, .. } = p {
            if status == "running" {
                *status = "error".into();
            }
        }
    }
    if !stopped && !out.finished && out.error.is_none() {
        out.error = Some(if stderr.trim().is_empty() {
            tr!("返事が返ってきませんでした", "No reply came back")
        } else if too_old(stderr) {
            tr!(
                "Claude Code が古いようです。ターミナルで `claude update` を実行してから、もう一度送ってください。\n（{}）",
                "Claude Code looks out of date. Run `claude update` in Terminal, then send again.\n({})",
                short(stderr, 200)
            )
        } else {
            short(stderr, 300)
        });
    }

    let mut message = Message {
        id: 0,
        role: "assistant".into(),
        parts: out.parts,
        created_at: now(),
        model: out.model,
        effort: out.effort,
        cost_usd: out.cost_usd,
        duration_ms: out.duration_ms,
        error: out.error,
        stopped,
    };
    if let Some(chat) = app.try_state::<ChatState>() {
        match add_message(&chat.lock(), conv_id, &message) {
            Ok(id) => message.id = id,
            Err(e) => log::error!("chat: could not save the answer: {e}"),
        }
    }
    let _ = app.emit("chat://done", Done { conv_id, message });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A database from before effort existed opens, gains the column once,
    /// and keeps what it held.
    #[test]
    fn old_database_gains_effort() {
        let db = Connection::open_in_memory().unwrap();
        db.execute_batch(
            "CREATE TABLE chat_conv (id INTEGER PRIMARY KEY, title TEXT NOT NULL, session_id TEXT,
                 model TEXT, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
             INSERT INTO chat_conv (title, model, created_at, updated_at) VALUES ('old', 'opus', 1, 1);",
        )
        .unwrap();
        db.execute_batch(SCHEMA).unwrap();
        for _ in 0..2 {
            add_column(&db, "chat_conv", "effort", "TEXT").unwrap();
            add_column(&db, "chat_msg", "effort", "TEXT").unwrap();
        }
        let conv = db
            .query_row(&format!("SELECT {CONV_COLUMNS} FROM chat_conv"), [], conversation)
            .unwrap();
        assert_eq!((conv.title.as_str(), conv.model.as_deref(), conv.effort), ("old", Some("opus"), None));
        assert!(load_messages(&db, conv.id).unwrap().is_empty());
    }

    #[test]
    fn effort_is_checked_and_skipped_for_haiku() {
        assert_eq!(effort_level(Some("xhigh".into())).as_deref(), Some("xhigh"));
        assert_eq!(effort_level(Some("ultra".into())), None);
        assert!(takes_effort(None));
        assert!(takes_effort(Some("fable")));
        assert!(!takes_effort(Some("haiku")));
        assert!(!takes_effort(Some("claude-haiku-4-5-20251001")));
    }

    #[test]
    fn an_unknown_flag_reads_as_an_old_claude() {
        assert!(too_old("error: unknown option '--effort'\n"));
        assert!(!too_old("Failed to authenticate: OAuth session expired"));
    }

    #[test]
    fn nvm_versions_go_newest_first_by_number() {
        let names = ["v9.11.2", "v20.11.1", "default", "v20.9.0", "v18.19.0"].map(String::from).to_vec();
        assert_eq!(newest_first(names), ["v20.11.1", "v20.9.0", "v18.19.0", "v9.11.2", "default"]);
    }

    /// Machine-dependent, so run by hand, ideally with the bare environment
    /// the Finder gives an app:
    /// `env -i HOME=$HOME SHELL=$SHELL USER=$USER PATH=/usr/bin:/bin:/usr/sbin:/sbin <test binary> --ignored finds_claude_from_a_bare_path --nocapture`
    #[test]
    #[ignore]
    fn finds_claude_from_a_bare_path() {
        eprintln!("PATH        {:?}", std::env::var("PATH"));
        eprintln!("search PATH {}", search_path());
        let claude = claude_path().expect("claude not found");
        eprintln!("claude      {}", claude.display());
        eprintln!("child PATH  {}", child_path(&claude));
        let out = Command::new(&claude)
            .arg("--version")
            .env_clear()
            .env("HOME", std::env::var("HOME").unwrap())
            .env("PATH", child_path(&claude))
            .output()
            .unwrap();
        eprintln!("--version   {}", String::from_utf8_lossy(&out.stdout).trim());
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    }
}
