import { useSyncExternalStore } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { Update } from "@tauri-apps/plugin-updater";

/**
 * In-app updates from GitHub Releases. The updater plugin fetches
 * `latest.json` from the latest published release (tauri.conf.json), checks
 * the download against the updater key, and swaps the app bundle in place;
 * the swapped-in version runs from the next launch, so a restart can wait.
 *
 * One flow for every entry point: the check on launch, the sidebar button
 * and the app menu's "Check for Updates…" (src-tauri/src/updates.rs).
 */

const REPO = "hibiki1213/otomo";

export type UpdatePhase =
  | { phase: "idle" }
  | { phase: "checking" }
  | { phase: "available"; version: string; current: string; notes: string }
  | { phase: "downloading"; version: string; received: number; total: number | null }
  | { phase: "ready"; version: string }
  /** blocked: running from a DMG or a translocated copy, which cannot be replaced. */
  | { phase: "failed"; version: string; blocked: boolean; message: string };

export interface UpdaterState {
  status: UpdatePhase;
  /** Whether the update dialog is showing; closing it never stops a download. */
  open: boolean;
}

/** What a check found, for the caller to tell the person when they asked. */
export type CheckResult = "available" | "latest" | "failed" | "busy";

let state: UpdaterState = { status: { phase: "idle" }, open: false };
let update: Update | null = null;
const listeners = new Set<() => void>();

function set(status: UpdatePhase, open = state.open) {
  state = { status, open };
  for (const listener of listeners) listener();
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function useUpdater(): UpdaterState {
  return useSyncExternalStore(subscribe, () => state);
}

/** Where to read about (or download by hand) a version. */
export function releaseUrl(version?: string): string {
  return version ? `https://github.com/${REPO}/releases/tag/v${version}` : `https://github.com/${REPO}/releases/latest`;
}

let versionPromise: Promise<string> | null = null;

/** This build's version, from tauri.conf.json. */
export function appVersion(): Promise<string> {
  versionPromise ??= import("@tauri-apps/api/app").then((m) => m.getVersion());
  return versionPromise;
}

/**
 * Ask GitHub whether there is a newer version, and open the dialog if so.
 * When an update is already found, downloading or installed, this only
 * brings the dialog back.
 */
export async function checkForUpdates(): Promise<CheckResult> {
  const { phase } = state.status;
  if (phase === "checking") return "busy";
  if (phase !== "idle") {
    set(state.status, true);
    return phase === "downloading" ? "busy" : "available";
  }
  set({ phase: "checking" });
  try {
    const { check } = await import("@tauri-apps/plugin-updater");
    const found = await check({ timeout: 30_000 });
    if (!found) {
      set({ phase: "idle" });
      return "latest";
    }
    update = found;
    set({ phase: "available", version: found.version, current: found.currentVersion, notes: found.body ?? "" }, true);
    return "available";
  } catch (e) {
    console.error("update check", e);
    set({ phase: "idle" });
    return "failed";
  }
}

/** Download and swap in the found update; the dialog follows along. */
export async function installUpdate() {
  if (state.status.phase !== "available" || !update) return;
  const { version } = update;
  if (await invoke<boolean>("update_blocked").catch(() => false)) {
    set({ phase: "failed", version, blocked: true, message: "" });
    return;
  }
  let received = 0;
  let total: number | null = null;
  let shown = -1;
  set({ phase: "downloading", version, received, total });
  try {
    await update.downloadAndInstall((event) => {
      if (event.event === "Started") {
        total = event.data.contentLength ?? null;
      } else if (event.event === "Progress") {
        received += event.data.chunkLength;
        // A re-render per percent, not per network chunk.
        const step = total ? Math.floor((received / total) * 100) : Math.floor(received / 1_000_000);
        if (step !== shown) {
          shown = step;
          set({ phase: "downloading", version, received, total });
        }
      }
    });
    // Reopen if the dialog was closed while downloading: the next step is theirs.
    set({ phase: "ready", version }, true);
  } catch (e) {
    console.error("update install", e);
    set({ phase: "failed", version, blocked: false, message: String(e) }, true);
  }
}

/** Hide the dialog. A found update is let go; one being downloaded or installed is kept. */
export function closeUpdateDialog() {
  const { phase } = state.status;
  if (phase === "available" || phase === "failed") {
    void update?.close().catch(() => {});
    update = null;
    set({ phase: "idle" }, false);
  } else {
    set(state.status, false);
  }
}

/** Start the installed version now. */
export async function restartToUpdate() {
  const { relaunch } = await import("@tauri-apps/plugin-process");
  await relaunch();
}
