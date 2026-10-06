import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useI18n } from "@/lib/i18n";
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
  const { t } = useI18n();
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
    ? t.connection.connected
    : status.stale
      ? t.connection.update
      : t.connection.connect;

  return (
    <div className="claude-connect">
      <button
        className={`connect-btn${status.connected ? " on" : ""}`}
        disabled={busy || (!!status.blocked && !status.connected)}
        title={status.blocked ?? status.configPath}
        onClick={() =>
          status.connected
            ? void run("mcp_disconnect", t.connection.didDisconnect)
            : void run("mcp_connect", t.connection.didConnect)
        }
      >
        <span className={`connect-dot${status.connected ? " on" : ""}`} />
        {busy ? t.connection.working : label}
      </button>

      {status.connected && !busy && (
        <p className="connect-note">{t.connection.clickToDisconnect}</p>
      )}
      {status.stale && !busy && (
        <p className="connect-note">{t.connection.stale}</p>
      )}
      {error && <p className="connect-error">{error}</p>}
      {note && !error && <p className="connect-note">{note}</p>}
      {status.blocked && !status.connected && !note && (
        <p className="connect-note">{status.blocked}</p>
      )}
    </div>
  );
}
