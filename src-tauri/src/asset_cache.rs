//! Disk cache for large remote audio assets (SoundFont, piano samples).
//!
//! The webview fetches the asset over HTTP the first time and hands the bytes
//! to `asset_cache_put`; later loads are served by `asset_cache_get` without
//! touching the network. Bytes travel as raw IPC bodies (no JSON encoding),
//! so a 31 MB SoundFont round-trips cheaply.

use std::path::PathBuf;

use tauri::ipc::{InvokeBody, Request, Response};
use tauri::{AppHandle, Manager};

const KEY_HEADER: &str = "x-asset-key";

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in bytes {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

pub(crate) fn cache_path(app: &AppHandle, key: &str) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_cache_dir()
        .map_err(|e| e.to_string())?
        .join("assets");
    Ok(dir.join(format!("{:016x}.bin", fnv1a64(key.as_bytes()))))
}

/// Whether an asset is cached, without transferring it.
#[tauri::command]
pub async fn asset_cache_has(app: AppHandle, key: String) -> Result<bool, String> {
    Ok(tokio::fs::try_exists(cache_path(&app, &key)?).await.unwrap_or(false))
}

/// Returns the cached bytes, or an empty body on a miss.
#[tauri::command]
pub async fn asset_cache_get(app: AppHandle, key: String) -> Result<Response, String> {
    let path = cache_path(&app, &key)?;
    match tokio::fs::read(&path).await {
        Ok(bytes) => Ok(Response::new(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Response::new(Vec::new())),
        Err(e) => Err(e.to_string()),
    }
}

/// Stores the raw request body under the key given in the `x-asset-key` header.
#[tauri::command]
pub async fn asset_cache_put(app: AppHandle, request: Request<'_>) -> Result<(), String> {
    let key = request
        .headers()
        .get(KEY_HEADER)
        .and_then(|v| v.to_str().ok())
        .ok_or("missing x-asset-key header")?
        .to_owned();
    let InvokeBody::Raw(bytes) = request.body() else {
        return Err("expected raw body".into());
    };
    if bytes.is_empty() {
        return Err("empty body".into());
    }

    let path = cache_path(&app, &key)?;
    if let Some(dir) = path.parent() {
        tokio::fs::create_dir_all(dir).await.map_err(|e| e.to_string())?;
    }
    // Write-then-rename so a crash mid-write never leaves a truncated asset.
    let tmp = path.with_extension("part");
    tokio::fs::write(&tmp, bytes).await.map_err(|e| e.to_string())?;
    tokio::fs::rename(&tmp, &path).await.map_err(|e| e.to_string())?;
    Ok(())
}
