import { useEffect, useState } from "react";
import {
  formatBytes,
  installLibrary,
  listLibraries,
  onLibraryProgress,
  removeLibrary,
  type LibraryProgress,
  type LibraryStatus,
} from "@/lib/instrumentsApi";
import type { TrackInstruments } from "@/lib/useTrackInstruments";

async function openExternal(url: string) {
  const { openUrl } = await import("@tauri-apps/plugin-opener");
  await openUrl(url);
}

/** Download / remove free SFZ libraries and control auto-assignment. */
export default function LibraryManager({
  instruments,
  onClose,
}: {
  instruments: TrackInstruments;
  onClose: () => void;
}) {
  const [libs, setLibs] = useState<LibraryStatus[]>([]);
  const [progress, setProgress] = useState<Record<string, LibraryProgress>>({});
  const [error, setError] = useState<string | null>(null);

  const refresh = async () => {
    try {
      setLibs(await listLibraries());
    } catch (e) {
      setError(String(e));
    }
  };

  useEffect(() => {
    void refresh();
    const off = onLibraryProgress((p) => {
      setProgress((prev) => ({ ...prev, [p.id]: p }));
      if (p.phase === "done" || p.phase === "error") {
        void refresh();
        void instruments.refreshCatalog();
      }
    });
    return () => {
      void off.then((f) => f());
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const install = async (id: string) => {
    setError(null);
    setLibs((prev) => prev.map((l) => (l.id === id ? { ...l, installing: true } : l)));
    try {
      await installLibrary(id);
      if (instruments.autoAssign) instruments.reassign();
    } catch (e) {
      setError(String(e));
    } finally {
      void refresh();
    }
  };

  const remove = async (lib: LibraryStatus) => {
    const { ask } = await import("@tauri-apps/plugin-dialog");
    const ok = await ask(`「${lib.name}」を削除しますか？（あとで再ダウンロードできます）`, {
      title: "音源ライブラリ",
      kind: "warning",
    });
    if (!ok) return;
    try {
      await removeLibrary(lib.id);
      await refresh();
      await instruments.refreshCatalog();
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div
        className="modal"
        role="dialog"
        aria-label="音源ライブラリ"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="modal-head">
          <h2>音源ライブラリ</h2>
          <button className="modal-close" onClick={onClose} aria-label="閉じる">
            ×
          </button>
        </div>

        <p className="modal-lead">
          無料の高品質な音源をダウンロードすると、曲の各トラックに合う楽器が自動で割り当てられます。
          割り当てられない楽器は GM 音源（SoundFont）で鳴ります。
        </p>

        <label className="modal-toggle">
          <input
            type="checkbox"
            checked={instruments.autoAssign}
            onChange={(e) => instruments.setAutoAssign(e.target.checked)}
          />
          曲を開いたときに自動で割り当てる
        </label>
        <div className="modal-actions">
          <button className="btn-ghost" onClick={instruments.reassign}>
            いまの曲に自動割り当て
          </button>
          <button className="btn-ghost" onClick={instruments.resetAll}>
            すべて GM 音源に戻す
          </button>
        </div>

        {error && <div className="error">{error}</div>}

        <ul className="lib-list">
          {libs.length === 0 && <li className="lib-empty">音源カタログを読み込み中…</li>}
          {libs.map((lib) => {
            const p = progress[lib.id];
            const busy = lib.installing;
            const pct =
              p && p.phase === "download" && p.total > 0
                ? Math.round((p.received / p.total) * 100)
                : null;
            return (
              <li key={lib.id} className="lib-item">
                <div className="lib-main">
                  <div className="lib-name">
                    {lib.name}
                    {lib.installed && <span className="lib-badge">インストール済み</span>}
                  </div>
                  <div className="lib-desc">{lib.description}</div>
                  <div className="lib-meta">
                    {formatBytes(lib.downloadSize)} · {lib.license} ·{" "}
                    <button className="link-btn" onClick={() => void openExternal(lib.homepage)}>
                      配布元
                    </button>
                  </div>
                  {lib.installed && lib.instruments.length > 0 && (
                    <div className="lib-instruments">
                      {lib.instruments.map((i) => i.name).join("、")}
                    </div>
                  )}
                  <div className="lib-credit">{lib.attribution}</div>
                  {busy && (
                    <div className="lib-progress">
                      <div
                        className="lib-bar"
                        style={{ width: `${pct ?? (p?.phase === "extract" ? 100 : 0)}%` }}
                      />
                      <span>
                        {p?.phase === "extract"
                          ? "展開中…"
                          : pct !== null
                          ? `ダウンロード中 ${pct}%`
                          : "準備中…"}
                      </span>
                    </div>
                  )}
                </div>
                <div className="lib-side">
                  {lib.installed ? (
                    <button className="btn-ghost" disabled={busy} onClick={() => void remove(lib)}>
                      削除
                    </button>
                  ) : (
                    <button className="btn-filled" disabled={busy} onClick={() => void install(lib.id)}>
                      {busy ? "処理中…" : "ダウンロード"}
                    </button>
                  )}
                </div>
              </li>
            );
          })}
        </ul>

        {instruments.plugins.length > 0 && (
          <p className="modal-foot">
            この Mac の AudioUnit 音源 {instruments.plugins.length} 件も、各トラックの「音源」から選べます。
          </p>
        )}
      </div>
    </div>
  );
}
