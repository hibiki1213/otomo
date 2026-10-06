import { useI18n, type LangPref } from "@/lib/i18n";

/** Each language is listed in itself, so it can be found from either. */
const NATIVE: { value: Exclude<LangPref, "system">; label: string }[] = [
  { value: "ja", label: "日本語" },
  { value: "en", label: "English" },
];

/** The display language, at the foot of the sidebar. */
export default function LanguageSelect() {
  const { t, pref, setPref } = useI18n();
  return (
    <label className="lang-select" title={t.language.label}>
      <GlobeIcon />
      <select
        value={pref}
        onChange={(e) => setPref(e.target.value as LangPref)}
        aria-label={t.language.label}
      >
        <option value="system">{t.language.system}</option>
        {NATIVE.map((l) => (
          <option key={l.value} value={l.value}>
            {l.label}
          </option>
        ))}
      </select>
    </label>
  );
}

function GlobeIcon() {
  return (
    <svg
      width="13"
      height="13"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth={1.6}
      strokeLinecap="round"
      aria-hidden
    >
      <circle cx="12" cy="12" r="8.5" />
      <path d="M3.5 12h17M12 3.5c2.4 2.3 3.6 5.1 3.6 8.5s-1.2 6.2-3.6 8.5c-2.4-2.3-3.6-5.1-3.6-8.5s1.2-6.2 3.6-8.5z" />
    </svg>
  );
}
