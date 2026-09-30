//! Connecting Otomo to Claude Desktop.
//!
//! Claude Desktop starts its MCP servers from a JSON file of its own, so
//! "connect" means adding one entry to that file — and nothing else. The
//! file belongs to another application and usually holds servers the user set
//! up themselves, so it is read, merged and written back: never replaced,
//! never rewritten from a template, and always copied aside first.
//!
//! Only `mcpServers.otomo` is ever touched.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::{json, Value};
use tauri::AppHandle;

/// The key this app owns in Claude Desktop's configuration.
const SERVER_KEY: &str = "otomo";
const SERVER_BIN: &str = "otomo-mcp";

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct McpStatus {
    /// Claude Desktop keeps its settings here, so it is installed.
    pub claude_found: bool,
    pub config_path: String,
    /// The MCP server binary, when it has been built.
    pub server_path: Option<String>,
    /// Whether Claude Desktop is already pointed at this build.
    pub connected: bool,
    /// Set when the entry points somewhere else — an older copy, usually.
    pub stale: bool,
    /// Why connecting is not possible right now, if it is not.
    pub blocked: Option<String>,
}

fn claude_dir() -> Result<PathBuf, String> {
    let home = std::env::var("HOME").map_err(|_| "HOME が設定されていません".to_string())?;
    Ok(PathBuf::from(home).join("Library/Application Support/Claude"))
}

fn config_path() -> Result<PathBuf, String> {
    Ok(claude_dir()?.join("claude_desktop_config.json"))
}

/// Where this build keeps its MCP server.
///
/// Installed, it sits beside the app's own executable. In development the
/// release build of it does too, one profile over — which is where
/// `pnpm mcp:build` puts it.
pub(crate) fn server_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    let beside = dir.join(SERVER_BIN);
    if beside.is_file() {
        return Some(beside);
    }
    if cfg!(debug_assertions) {
        let release = dir.parent()?.join("release").join(SERVER_BIN);
        if release.is_file() {
            return Some(release);
        }
    }
    None
}

/// Read the configuration, or say why it cannot be read.
///
/// A file that exists but does not parse is left alone: it is not ours to
/// repair, and overwriting it would cost the user every other server.
fn read_config(path: &Path) -> Result<Option<Value>, String> {
    match std::fs::read_to_string(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("設定ファイルを読めません: {e}")),
        Ok(text) if text.trim().is_empty() => Ok(None),
        Ok(text) => serde_json::from_str(&text).map(Some).map_err(|e| {
            format!(
                "Claude Desktop の設定ファイルが壊れているようです（{e}）。\
                 中身を確認してから、もう一度お試しください。"
            )
        }),
    }
}

fn entry_command(config: &Value) -> Option<&str> {
    config.get("mcpServers")?.get(SERVER_KEY)?.get("command")?.as_str()
}

#[tauri::command]
pub fn mcp_status(_app: AppHandle) -> Result<McpStatus, String> {
    let path = config_path()?;
    let server = server_path();
    let config = read_config(&path);

    let mut status = McpStatus {
        claude_found: claude_dir().map(|d| d.is_dir()).unwrap_or(false),
        config_path: path.display().to_string(),
        server_path: server.as_ref().map(|p| p.display().to_string()),
        connected: false,
        stale: false,
        blocked: None,
    };
    match config {
        Err(e) => status.blocked = Some(e),
        Ok(config) => {
            let current = config.as_ref().and_then(entry_command);
            let wanted = server.as_ref().map(|p| p.display().to_string());
            status.connected = current.is_some() && current.map(str::to_owned) == wanted;
            status.stale = current.is_some() && !status.connected;
        }
    }
    if status.blocked.is_none() {
        if !status.claude_found {
            status.blocked = Some(
                "Claude Desktop が見つかりません。インストールして一度起動してください。".into(),
            );
        } else if server.is_none() {
            status.blocked = Some(
                "MCP サーバーがまだビルドされていません。`pnpm mcp:build` を実行してください。"
                    .into(),
            );
        }
    }
    Ok(status)
}

/// Add (or update) this app's entry in Claude Desktop's configuration.
#[tauri::command]
pub fn mcp_connect(app: AppHandle) -> Result<McpStatus, String> {
    let path = config_path()?;
    let server = server_path()
        .ok_or("MCP サーバーが見つかりません。`pnpm mcp:build` を実行してください。")?;
    if let Some(reason) = mcp_status(app.clone())?.blocked {
        return Err(reason);
    }

    let mut config = read_config(&path)?.unwrap_or_else(|| json!({}));
    if !config.is_object() {
        return Err("Claude Desktop の設定ファイルの形式が想定と違います。".into());
    }
    let servers = config
        .as_object_mut()
        .expect("checked above")
        .entry("mcpServers")
        .or_insert_with(|| json!({}));
    if !servers.is_object() {
        return Err("設定ファイルの mcpServers が想定と違う形式です。".into());
    }
    servers
        .as_object_mut()
        .expect("checked above")
        .insert(SERVER_KEY.to_string(), json!({ "command": server.display().to_string() }));

    write_config(&path, &config)?;
    mcp_status(app)
}

/// Take this app's entry out again, leaving every other server alone.
#[tauri::command]
pub fn mcp_disconnect(app: AppHandle) -> Result<McpStatus, String> {
    let path = config_path()?;
    let Some(mut config) = read_config(&path)? else {
        return mcp_status(app);
    };
    let removed = config
        .get_mut("mcpServers")
        .and_then(Value::as_object_mut)
        .map(|servers| servers.remove(SERVER_KEY).is_some())
        .unwrap_or(false);
    if removed {
        write_config(&path, &config)?;
    }
    mcp_status(app)
}

/// Keep a copy of what was there, then write the new file in one step.
fn write_config(path: &Path, config: &Value) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    if path.exists() {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let backup = path.with_file_name(format!("claude_desktop_config.{stamp}.backup.json"));
        std::fs::copy(path, &backup)
            .map_err(|e| format!("バックアップを作れませんでした: {e}"))?;
        prune_backups(path);
    }
    let text = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    // Write-then-rename: Claude Desktop never sees a half-written file.
    let tmp = path.with_extension("json.part");
    std::fs::write(&tmp, text.as_bytes()).map_err(|e| format!("書き込めませんでした: {e}"))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("書き込めませんでした: {e}"))?;
    Ok(())
}

/// Keep the five most recent backups; older ones are noise in a folder that
/// is not ours.
fn prune_backups(path: &Path) {
    let Some(dir) = path.parent() else { return };
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut backups: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("claude_desktop_config.") && n.ends_with(".backup.json"))
        })
        .collect();
    backups.sort();
    for old in backups.iter().rev().skip(5) {
        let _ = std::fs::remove_file(old);
    }
}

/// Only for the tests below: the merge, without touching the real file.
#[cfg(test)]
fn merged(existing: Option<Value>, command: &str) -> Value {
    let mut config = existing.unwrap_or_else(|| json!({}));
    let servers = config.as_object_mut().unwrap().entry("mcpServers").or_insert_with(|| json!({}));
    servers
        .as_object_mut()
        .unwrap()
        .insert(SERVER_KEY.to_string(), json!({ "command": command }));
    config
}

#[cfg(test)]
fn servers_of(config: &Value) -> &serde_json::Map<String, Value> {
    config.get("mcpServers").and_then(Value::as_object).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connecting_leaves_every_other_server_alone() {
        let existing = json!({
            "mcpServers": {
                "freee-mcp": { "command": "/opt/freee", "args": ["--serve"] }
            },
            "preferences": { "theme": "dark" }
        });
        let after = merged(Some(existing), "/Applications/Otomo.app/Contents/MacOS/otomo-mcp");

        let servers = servers_of(&after);
        assert_eq!(servers.len(), 2);
        assert_eq!(servers["freee-mcp"]["command"], "/opt/freee");
        assert_eq!(servers["freee-mcp"]["args"][0], "--serve", "their arguments survive");
        assert_eq!(
            servers["otomo"]["command"],
            "/Applications/Otomo.app/Contents/MacOS/otomo-mcp"
        );
        assert_eq!(after["preferences"]["theme"], "dark", "the rest of the file survives");
    }

    #[test]
    fn connecting_again_updates_the_path_rather_than_adding_a_second_entry() {
        let first = merged(None, "/old/otomo-mcp");
        let second = merged(Some(first), "/new/otomo-mcp");
        let servers = servers_of(&second);
        assert_eq!(servers.len(), 1);
        assert_eq!(servers["otomo"]["command"], "/new/otomo-mcp");
    }

    #[test]
    fn a_broken_config_is_reported_not_overwritten() {
        let dir = std::env::temp_dir().join(format!("otomo-mcp-cfg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("claude_desktop_config.json");
        std::fs::write(&path, "{ this is not json").unwrap();

        let err = read_config(&path).unwrap_err();
        assert!(err.contains("壊れている"), "{err}");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "{ this is not json",
            "the file is left exactly as it was"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn writing_keeps_a_copy_of_what_was_there() {
        let dir = std::env::temp_dir().join(format!("otomo-mcp-bak-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("claude_desktop_config.json");
        std::fs::write(&path, r#"{"mcpServers":{"theirs":{"command":"/x"}}}"#).unwrap();

        write_config(&path, &merged(read_config(&path).unwrap(), "/new")).unwrap();

        let after: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(servers_of(&after).len(), 2);

        let backups: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".backup.json"))
            .collect();
        assert_eq!(backups.len(), 1, "one backup: {backups:?}");
        let saved: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join(&backups[0])).unwrap()).unwrap();
        assert_eq!(servers_of(&saved).len(), 1, "the backup is the file from before");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
