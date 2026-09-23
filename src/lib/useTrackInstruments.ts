import { useCallback, useEffect, useRef, useState } from "react";
import type { SongInfo } from "./midiEngine";
import {
  listLibraries,
  listPlugins,
  setTrackInstrument,
  suggestInstruments,
  type InstrumentChoice,
  type InstrumentEntry,
  type PluginInfo,
} from "./instrumentsApi";

export interface TrackInstrumentState {
  choice: InstrumentChoice;
  /** Human-readable source name (empty for the SoundFont). */
  label: string;
  loading: boolean;
  error: string | null;
}

const AUTO_KEY = "midiss.autoAssign";
const SF2: TrackInstrumentState = {
  choice: { kind: "sf2" },
  label: "",
  loading: false,
  error: null,
};

function loadAuto(): boolean {
  try {
    return localStorage.getItem(AUTO_KEY) !== "false";
  } catch {
    return true;
  }
}

/**
 * Per-track instrument assignment for the native engine: installed SFZ
 * instruments, AudioUnit plugins, auto-assignment on song load.
 */
export function useTrackInstruments(song: SongInfo | null, enabled: boolean) {
  const [states, setStates] = useState<TrackInstrumentState[]>([]);
  const [installed, setInstalled] = useState<InstrumentEntry[]>([]);
  const [plugins, setPlugins] = useState<PluginInfo[]>([]);
  const [autoAssign, setAutoAssignState] = useState(loadAuto);
  // Ignore results from a previous song.
  const songRef = useRef<SongInfo | null>(null);

  const refreshCatalog = useCallback(async () => {
    if (!enabled) return;
    try {
      const [libs, plugs] = await Promise.all([listLibraries(), listPlugins()]);
      setInstalled(libs.flatMap((l) => l.instruments));
      setPlugins(plugs);
    } catch (e) {
      console.error("instrument catalog", e);
    }
  }, [enabled]);

  useEffect(() => {
    void refreshCatalog();
  }, [refreshCatalog]);

  const patch = (track: number, s: Partial<TrackInstrumentState>) =>
    setStates((prev) => {
      const next = [...prev];
      next[track] = { ...(next[track] ?? SF2), ...s };
      return next;
    });

  const apply = useCallback(
    async (track: number, choice: InstrumentChoice, label: string) => {
      const forSong = songRef.current;
      patch(track, { choice, label, loading: true, error: null });
      try {
        await setTrackInstrument(track, choice);
        if (songRef.current === forSong) patch(track, { loading: false });
      } catch (e) {
        if (songRef.current !== forSong) return;
        // Loading failed: the engine kept the previous source; show SF2.
        patch(track, { ...SF2, error: String(e) });
        void setTrackInstrument(track, { kind: "sf2" }).catch(() => undefined);
      }
    },
    []
  );

  const runAutoAssign = useCallback(
    async (target: SongInfo) => {
      const suggestions = await suggestInstruments(
        target.tracks.map((t) => ({
          program: t.program,
          percussion: t.isPercussion,
        }))
      );
      await Promise.all(
        suggestions.map((s, i) =>
          s ? apply(i, s.choice, s.name) : undefined
        )
      );
    },
    [apply]
  );

  // New song: everything starts on the SoundFont, then auto-assign.
  useEffect(() => {
    songRef.current = song;
    setStates(song ? song.tracks.map(() => SF2) : []);
    if (song && enabled && autoAssign) {
      runAutoAssign(song).catch((e) => console.error("auto assign", e));
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [song, enabled]);

  const setAutoAssign = useCallback((on: boolean) => {
    setAutoAssignState(on);
    try {
      localStorage.setItem(AUTO_KEY, String(on));
    } catch {
      /* ignore */
    }
  }, []);

  return {
    states,
    installed,
    plugins,
    autoAssign,
    setAutoAssign,
    apply,
    /** Re-run auto-assignment on the current song (e.g. after installing). */
    reassign: useCallback(() => {
      if (songRef.current) void runAutoAssign(songRef.current);
    }, [runAutoAssign]),
    resetAll: useCallback(() => {
      const s = songRef.current;
      if (!s) return;
      s.tracks.forEach((_, i) => void apply(i, { kind: "sf2" }, ""));
    }, [apply]),
    refreshCatalog,
  };
}

export type TrackInstruments = ReturnType<typeof useTrackInstruments>;
