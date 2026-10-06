import { createContext, useCallback, useContext, useState, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { en, type Messages } from "@/locales/en";
import { ja } from "@/locales/ja";

export type Lang = "ja" | "en";
/** What the person picked; "system" follows the Mac's language. */
export type LangPref = Lang | "system";

const DICTS: Record<Lang, Messages> = { ja, en };
const PREF_KEY = "otomo.lang";
const inApp = () => "__TAURI_INTERNALS__" in window;

let system: Lang = toLang(navigator.language);
let current: Lang = "ja";

/** "ja-JP" → ja; anything we have no translation for → en. */
function toLang(tag: string | undefined): Lang {
  return tag?.toLowerCase().startsWith("ja") ? "ja" : "en";
}

function readPref(): LangPref {
  try {
    const v = localStorage.getItem(PREF_KEY);
    return v === "ja" || v === "en" ? v : "system";
  } catch {
    return "system";
  }
}

function writePref(pref: LangPref) {
  try {
    if (pref === "system") localStorage.removeItem(PREF_KEY);
    else localStorage.setItem(PREF_KEY, pref);
  } catch {
    /* ignore */
  }
}

const resolve = (pref: LangPref): Lang => (pref === "system" ? system : pref);

/**
 * Make `lang` current here and in the backend, which words its errors and
 * catalog names to match. Finishes before React re-renders, so anything
 * re-fetched for the new language comes back in it.
 */
async function activate(lang: Lang) {
  current = lang;
  document.documentElement.lang = lang;
  if (inApp()) await invoke("set_language", { lang }).catch((e) => console.error("set_language", e));
}

/**
 * Settle the language before the first render. The system language comes
 * from the backend: in the app's web view `navigator.language` need not be
 * the Mac's (WebKit weighs it against the bundle's localizations).
 */
export async function initLanguage() {
  if (inApp()) {
    try {
      system = toLang(await invoke<string>("system_language"));
    } catch (e) {
      console.error("system_language", e);
    }
  }
  await activate(resolve(readPref()));
}

/**
 * Strings for code outside React (parsers, API helpers). Read it when the
 * text is made, not at import time, so it follows a switch.
 */
export function messages(): Messages {
  return DICTS[current];
}

interface I18n {
  t: Messages;
  lang: Lang;
  pref: LangPref;
  setPref: (pref: LangPref) => void;
}

const Ctx = createContext<I18n | null>(null);

export function I18nProvider({ children }: { children: ReactNode }) {
  const [pref, setPrefState] = useState<LangPref>(readPref);
  const [lang, setLang] = useState<Lang>(current);

  const setPref = useCallback((next: LangPref) => {
    writePref(next);
    const target = resolve(next);
    void activate(target).then(() => {
      setPrefState(next);
      setLang(target);
    });
  }, []);

  return <Ctx.Provider value={{ t: DICTS[lang], lang, pref, setPref }}>{children}</Ctx.Provider>;
}

export function useI18n(): I18n {
  const ctx = useContext(Ctx);
  if (!ctx) throw new Error("useI18n outside I18nProvider");
  return ctx;
}
