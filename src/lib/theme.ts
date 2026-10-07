/**
 * Light or dark, or whatever the Mac is set to. The choice is applied to the
 * window itself: the web view follows its window's appearance, so the CSS's
 * `prefers-color-scheme` and the canvases' `matchMedia` all switch with it,
 * and so does the sidebar's vibrancy.
 */
export type ThemePref = "system" | "light" | "dark";

const PREF_KEY = "otomo.theme";
const inApp = () => "__TAURI_INTERNALS__" in window;

export function readThemePref(): ThemePref {
  try {
    const v = localStorage.getItem(PREF_KEY);
    return v === "light" || v === "dark" ? v : "system";
  } catch {
    return "system";
  }
}

export async function applyTheme(pref: ThemePref) {
  try {
    if (pref === "system") localStorage.removeItem(PREF_KEY);
    else localStorage.setItem(PREF_KEY, pref);
  } catch {
    /* only remembering it is lost */
  }
  if (!inApp()) return;
  try {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    await getCurrentWindow().setTheme(pref === "system" ? null : pref);
  } catch (e) {
    console.error("set theme", e);
  }
}
