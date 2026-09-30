//! Talking to Claude from inside the app.
//!
//! Each question starts a headless `claude -p` whose tools are this app's own
//! MCP server — the same `otomo-mcp` Claude Desktop uses — and web search, so
//! an existing piece is looked up rather than recalled. So an edit made
//! from the chat panel arrives exactly like one made from Claude Desktop: it is
//! staged, the banner appears, and nothing is kept until someone presses 適用.
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
use std::path::PathBuf;
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager, State};

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
    /// Conversations someone pressed 止める on, so the ending reads as that
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

/// What the app has open, so "この曲" means something.
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

/// Where Claude Code is. An app started from the Finder gets a bare PATH, so
/// the usual install locations are looked at by hand before PATH is.
fn claude_path() -> Option<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_default();
    let known = [
        format!("{home}/.local/bin/claude"),
        format!("{home}/.claude/local/claude"),
        "/opt/homebrew/bin/claude".to_string(),
        "/usr/local/bin/claude".to_string(),
    ];
    if let Some(p) = known.iter().map(PathBuf::from).find(|p| p.is_file()) {
        return Some(p);
    }
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths).map(|d| d.join("claude")).find(|p| p.is_file())
    })
}

fn system_prompt() -> &'static str {
    "あなたは Otomo（MIDI の作曲・再生アプリ）に組み込まれたアシスタントです。\
ユーザーはアプリの右パネルからあなたと話していて、画面には曲とピアノロールが見えています。\n\n\
- 曲を調べる・作る・直す・鳴らすのは Otomo のツールで行う。\n\
- 既にある曲（国歌・クラシック・民謡・スタンダード・有名曲など）を頼まれたら、記憶だけで書かない。\
まず WebSearch で楽譜ファイルを探し、見つかったら download で取り込む。\
探す順は、MusicXML（.musicxml / .mxl：楽譜そのもので最も正確）、MIDI、ABC 記法、PDF・画像の楽譜。\
Wikimedia Commons、Mutopia Project、IMSLP、abcnotation.com など、パブリックドメインや自由なライセンスの出典を優先する。\n\
- 取り込んだら get_song と read_bars で中身を確かめてから、頼まれた編成やアレンジに直す。どこから取ったかを返事で伝える。\n\
- PDF や画像の楽譜しかなければ、download でページを見て書き起こす。読み取りに自信のない箇所は返事で伝える。\n\
- どれも見つからなければ、WebFetch で調・拍子・テンポ・構成・旋律を調べてから書く。\
それでも記憶に頼った部分があれば、そう伝える。\n\
- 編集はすぐには保存されない。「未適用の編集」としてアプリに出て、ユーザーが試聴して「適用」を押すと確定する。\
編集したら、何を変えたかを一言で伝える。\n\
- 音を確かめてほしいときは play で鳴らす（範囲や繰り返しも使える）。\n\
- 各メッセージの頭に [アプリの状態] が付く。「この曲」「いまの曲」はそこに書かれた曲を指す。\n\
- 返事は日本語で、短く。音楽用語は使ってよいが、長い説明はしない。\n"
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

#[tauri::command]
pub fn chat_status() -> ChatStatus {
    let claude = claude_path();
    let server = crate::mcp_setup::server_path();
    let blocked = if claude.is_none() {
        Some("Claude Code が見つかりません。インストールしてログインしてください。".into())
    } else if server.is_none() {
        Some("MCP サーバーがまだビルドされていません。`pnpm mcp:build` を実行してください。".into())
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
    model: Option<String>,
    effort: Option<String>,
) -> Result<Conversation, String> {
    let text = text.trim().to_string();
    if text.is_empty() {
        return Err("メッセージが空です".into());
    }
    let status = chat_status();
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
                params![short(&text, 40), model, effort, t],
            )
            .map_err(|e| e.to_string())?;
            db.last_insert_rowid()
        }
    };
    if chat.runs.lock().unwrap_or_else(|e| e.into_inner()).children.contains_key(&conv_id) {
        return Err("まだ返事を待っています".into());
    }
    let (session_id, conv_model, conv_effort): (Option<String>, Option<String>, Option<String>) = db
        .query_row(
            "SELECT session_id, model, effort FROM chat_conv WHERE id = ?1",
            params![conv_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?
        .ok_or("会話が見つかりません")?;
    let model = model.or(conv_model);
    let effort = effort.or(conv_effort).filter(|_| takes_effort(model.as_deref()));
    add_message(
        &db,
        conv_id,
        &Message {
            id: 0,
            role: "user".into(),
            parts: vec![Part::Text { text: text.clone() }],
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
        Some(s) => format!("[アプリの状態] いま開いている曲: 「{}」（song_id: {}）", s.title, s.id),
        None => "[アプリの状態] 曲は開いていません".to_string(),
    };
    let prompt = format!("{context}\n\n{text}");

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

    let mut cmd = Command::new(claude);
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
        .args(["--append-system-prompt", system_prompt()])
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
    // Started from inside a Claude Code session (as in development), the child
    // would otherwise inherit that session's identity and act as part of it.
    for (k, _) in std::env::vars() {
        if k.starts_with("CLAUDE") {
            cmd.env_remove(&k);
        }
    }
    cmd.env("CLAUDE_CODE_ENTRYPOINT", "otomo-chat");

    let mut child = cmd.spawn().map_err(|e| format!("Claude を起動できませんでした: {e}"))?;
    let mut stdin = child.stdin.take().ok_or("stdin を開けませんでした")?;
    stdin
        .write_all(prompt.as_bytes())
        .map_err(|e| format!("メッセージを渡せませんでした: {e}"))?;
    drop(stdin);
    let stdout = child.stdout.take().ok_or("stdout を開けませんでした")?;
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
                        .unwrap_or("不明なエラー");
                    out.error = Some(msg.to_string());
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
            "返事が返ってきませんでした".into()
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
}
