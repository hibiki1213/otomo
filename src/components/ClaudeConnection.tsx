import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { libraryAvailable } from "@/lib/songsApi";

/** See `src-tauri/src/mcp_setup.rs`. */
interface McpStatus {
  claudeFound: boolean;
  configPath: string;
  serverPath: string | null;
  connected: boolean;
  /** Claude Desktop points at a different build of the server. */
  stale: boolean;
  blocked: string | null;
}

/**
 * Connecting the library to Claude Desktop: one entry in its configuration,
 * added on a click and removable the same way.
 *
 * The button reports what it will do before it does it, because it writes to
 * another application's settings — and says afterwards that Claude Desktop
 * has to be restarted, which is the step people otherwise miss.
 */
export default function ClaudeConnection() {
  const [status, setStatus] = useState<McpStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    if (!libraryAvailable()) return;
    try {
      setStatus(await invoke<McpStatus>("mcp_status"));
    } catch (e) {
      console.error("mcp status", e);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const run = async (command: "mcp_connect" | "mcp_disconnect", after: string) => {
    setBusy(true);
    setError(null);
    setNote(null);
    try {
      setStatus(await invoke<McpStatus>(command));
      setNote(after);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  if (!status) return null;

  const label = status.connected
    ? "Claude と接続済み"
    : status.stale
      ? "接続を更新"
      : "Claude と接続";

  return (
    <div className="claude-connect">
      <button
        className={`connect-btn${status.connected ? " on" : ""}`}
        disabled={busy || (!!status.blocked && !status.connected)}
        title={status.blocked ?? status.configPath}
        onClick={() =>
          status.connected
            ? void run("mcp_disconnect", "接続を解除しました。Claude Desktop を再起動すると反映されます。")
            : void run("mcp_connect", "接続しました。Claude Desktop を再起動してください。")
        }
      >
        <span className={`connect-dot${status.connected ? " on" : ""}`} />
        {busy ? "設定中…" : label}
      </button>

      {status.connected && !busy && (
        <p className="connect-note">クリックで解除</p>
      )}
      {status.stale && !busy && (
        <p className="connect-note">
          別の場所のサーバーが登録されています。押すとこのアプリのものに直します。
        </p>
      )}
      {error && <p className="connect-error">{error}</p>}
      {note && !error && <p className="connect-note">{note}</p>}
      {status.blocked && !status.connected && !note && (
        <p className="connect-note">{status.blocked}</p>
      )}
    </div>
  );
}
