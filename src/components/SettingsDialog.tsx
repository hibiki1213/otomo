import { useEffect, useState, type ReactNode } from "react";
import { resolveResource } from "@tauri-apps/api/path";
import { openPath } from "@tauri-apps/plugin-opener";
import { useI18n, type LangPref } from "@/lib/i18n";
import { libraryAvailable } from "@/lib/songsApi";
import { applyTheme, readThemePref, type ThemePref } from "@/lib/theme";
import { appVersion, useUpdater } from "@/lib/updater";

/** Each language is listed in itself, so it can be found from either. */
const LANGUAGES: { value: Exclude<LangPref, "system">; label: string }[] = [
  { value: "ja", label: "日本語" },
  { value: "en", label: "English" },
];

/**
 * Appearance, language, the version with its update check, and the
 * licenses: what used to sit at the foot of the sidebar, in one place
 * (sidebar gear, Otomo → Settings…, ⌘,).
 */
export default function SettingsDialog({
  onClose,
  onCheckUpdates,
}: {
  onClose: () => void;
  /** Check for a new version; absent outside the app. */
  onCheckUpdates?: () => void;
}) {
  const { t, pref: langPref, setPref: setLangPref } = useI18n();
  const [themePref, setThemePref] = useState<ThemePref>(readThemePref);
  const updater = useUpdater().status.phase;
  const [version, setVersion] = useState<string | null>(null);

  useEffect(() => {
    if (libraryAvailable()) void appVersion().then(setVersion, () => {});
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const themes: { value: ThemePref; label: string }[] = [
    { value: "system", label: t.theme.system },
    { value: "light", label: t.theme.light },
    { value: "dark", label: t.theme.dark },
  ];
  const languages: { value: LangPref; label: string }[] = [
    { value: "system", label: t.language.system },
    ...LANGUAGES,
  ];

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div className="modal settings-modal" role="dialog" aria-label={t.settings.title} onClick={(e) => e.stopPropagation()}>
        <div className="modal-head">
          <h2>{t.settings.title}</h2>
          <button className="modal-close" onClick={onClose} aria-label={t.common.close}>
            ×
          </button>
        </div>

        <div className="settings-rows">
          <Row label={t.theme.label}>
            <div className="segmented-control">
              {themes.map((o) => (
                <button
                  key={o.value}
                  className={themePref === o.value ? "on" : ""}
                  onClick={() => {
                    setThemePref(o.value);
                    void applyTheme(o.value);
                  }}
                >
                  {o.label}
                </button>
              ))}
            </div>
          </Row>

          <Row label={t.language.label}>
            <div className="segmented-control">
              {languages.map((o) => (
                <button key={o.value} className={langPref === o.value ? "on" : ""} onClick={() => setLangPref(o.value)}>
                  {o.label}
                </button>
              ))}
            </div>
          </Row>

          {onCheckUpdates && (
            <Row label={t.settings.updates}>
              <span className="settings-value">{version && t.updates.version(version)}</span>
              <button className="btn-ghost" disabled={updater === "checking"} onClick={onCheckUpdates}>
                {updater === "checking"
                  ? t.updates.checking
                  : updater === "ready"
                    ? t.updates.restart
                    : t.updates.check}
              </button>
            </Row>
          )}

          {libraryAvailable() && (
            <Row label={t.settings.licenses}>
              <button
                className="btn-ghost"
                onClick={() =>
                  void resolveResource("THIRD_PARTY_LICENSES.html")
                    .then((p) => openPath(p))
                    .catch(console.error)
                }
              >
                {t.settings.showLicenses}
              </button>
            </Row>
          )}
        </div>
      </div>
    </div>
  );
}

function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="settings-row">
      <span className="settings-label">{label}</span>
      <div className="settings-control">{children}</div>
    </div>
  );
}
