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
import { useI18n } from "@/lib/i18n";
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
  const { t, lang } = useI18n();
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

  // Names and descriptions come from the backend in the current language.
  useEffect(() => {
    if (libs.length > 0) void refresh();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [lang]);

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
      if (instruments.autoAssign) instruments.fillUnchosen();
    } catch (e) {
      setError(String(e));
    } finally {
      void refresh();
    }
  };

  const remove = async (lib: LibraryStatus) => {
    const { ask } = await import("@tauri-apps/plugin-dialog");
    const ok = await ask(t.libraries.confirmRemove(lib.name), {
      title: t.libraries.title,
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
        aria-label={t.libraries.title}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="modal-head">
          <h2>{t.libraries.title}</h2>
          <button className="modal-close" onClick={onClose} aria-label={t.common.close}>
            ×
          </button>
        </div>

        <p className="modal-lead">{t.libraries.lead}</p>

        <label className="modal-toggle">
          <input
            type="checkbox"
            checked={instruments.autoAssign}
            onChange={(e) => instruments.setAutoAssign(e.target.checked)}
          />
          {t.libraries.autoAssign}
        </label>
        <div className="modal-actions">
          <button className="btn-ghost" onClick={instruments.reassign}>
            {t.libraries.assignNow}
          </button>
          <button className="btn-ghost" onClick={instruments.resetAll}>
            {t.libraries.resetAll}
          </button>
        </div>

        {error && <div className="error">{error}</div>}

        <ul className="lib-list">
          {libs.length === 0 && <li className="lib-empty">{t.libraries.loadingCatalog}</li>}
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
                    {lib.installed && <span className="lib-badge">{t.libraries.installed}</span>}
                  </div>
                  <div className="lib-desc">{lib.description}</div>
                  <div className="lib-meta">
                    {formatBytes(lib.downloadSize)} · {lib.license} ·{" "}
                    <button className="link-btn" onClick={() => void openExternal(lib.homepage)}>
                      {t.libraries.homepage}
                    </button>
                  </div>
                  {lib.installed && lib.instruments.length > 0 && (
                    <div className="lib-instruments">
                      {lib.instruments.map((i) => i.name).join(t.libraries.listSeparator)}
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
                          ? t.libraries.extracting
                          : pct !== null
                          ? t.libraries.downloading(pct)
                          : t.libraries.preparing}
                      </span>
                    </div>
                  )}
                </div>
                <div className="lib-side">
                  {lib.installed ? (
                    <button className="btn-ghost" disabled={busy} onClick={() => void remove(lib)}>
                      {t.libraries.remove}
                    </button>
                  ) : (
                    <button className="btn-filled" disabled={busy} onClick={() => void install(lib.id)}>
                      {busy ? t.libraries.working : t.libraries.download}
                    </button>
                  )}
                </div>
              </li>
            );
          })}
        </ul>

        {instruments.plugins.length > 0 && (
          <p className="modal-foot">{t.libraries.plugins(instruments.plugins.length)}</p>
        )}
      </div>
    </div>
  );
}
