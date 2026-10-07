import { useState } from "react";
import { useI18n } from "@/lib/i18n";
import { applyTheme, readThemePref, type ThemePref } from "@/lib/theme";

/** Light, dark or automatic, beside the language at the foot of the sidebar. */
export default function ThemeSelect() {
  const { t } = useI18n();
  const [pref, setPref] = useState<ThemePref>(readThemePref);
  return (
    <label className="lang-select" title={t.theme.label}>
      <ThemeIcon />
      <select
        value={pref}
        onChange={(e) => {
          const next = e.target.value as ThemePref;
          setPref(next);
          void applyTheme(next);
        }}
        aria-label={t.theme.label}
      >
        <option value="system">{t.theme.system}</option>
        <option value="light">{t.theme.light}</option>
        <option value="dark">{t.theme.dark}</option>
      </select>
    </label>
  );
}

function ThemeIcon() {
  // Half light, half dark.
  return (
    <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={1.6} aria-hidden>
      <circle cx="12" cy="12" r="8.5" />
      <path d="M12 3.5a8.5 8.5 0 0 1 0 17z" fill="currentColor" stroke="none" />
    </svg>
  );
}
