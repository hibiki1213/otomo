import { useCallback, useEffect, useRef, useState } from "react";
import { useI18n } from "./i18n";
import type { SongInfo } from "./midiEngine";
import {
  choiceKey,
  clearSongInstruments,
  listLibraries,
  listPlugins,
  listSynthPresets,
  listUserInstruments,
  loadSongInstruments,
  onInstrumentAssigned,
  saveSongInstrument,
  setTrackInstrument,
  suggestInstruments,
  type InstrumentChoice,
  type InstrumentEntry,
  type PluginInfo,
  type SavedInstrument,
} from "./instrumentsApi";

export interface TrackInstrumentState {
  choice: InstrumentChoice;
  /** Human-readable source name (empty for the SoundFont). */
  label: string;
  loading: boolean;
  error: string | null;
}

const AUTO_KEY = "otomo.autoAssign";
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
 * Per-track instrument assignment for the native engine: the built-in
 * synth, installed SFZ instruments, AudioUnit plugins, auto-assignment on
 * song load.
 *
 * A choice made by hand or by Claude is remembered for the song and comes
 * back when the song is opened again; auto-assignment only fills the tracks
 * nobody chose for.
 */
export function useTrackInstruments(
  song: SongInfo | null,
  songId: number | null,
  enabled: boolean
) {
  const { lang } = useI18n();
  const [states, setStates] = useState<TrackInstrumentState[]>([]);
  const [installed, setInstalled] = useState<InstrumentEntry[]>([]);
  const [plugins, setPlugins] = useState<PluginInfo[]>([]);
  const [autoAssign, setAutoAssignState] = useState(loadAuto);
  // Ignore results from a previous song.
  const songRef = useRef<SongInfo | null>(null);
  const songIdRef = useRef<number | null>(null);
  /** Tracks whose instrument is remembered for the song. */
  const savedRef = useRef<Set<number>>(new Set());

  const refreshCatalog = useCallback(async () => {
    if (!enabled) return;
    try {
      const [synths, libs, mine, plugs] = await Promise.all([
        listSynthPresets(),
        listLibraries(),
        listUserInstruments(),
        listPlugins(),
      ]);
      setInstalled([...synths, ...libs.flatMap((l) => l.instruments), ...mine]);
      setPlugins(plugs);
    } catch (e) {
      console.error("instrument catalog", e);
    }
    // Names come back in the current language, so a switch re-reads them.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [enabled, lang]);

  useEffect(() => {
    void refreshCatalog();
  }, [refreshCatalog]);

  const patch = (track: number, s: Partial<TrackInstrumentState>) =>
    setStates((prev) => {
      const next = [...prev];
      next[track] = { ...(next[track] ?? SF2), ...s };
      return next;
    });

  const remember = useCallback(
    (track: number, choice: InstrumentChoice, label: string) => {
      savedRef.current.add(track);
      const id = songIdRef.current;
      if (id == null) return;
      saveSongInstrument(id, track, choice, label).catch((e) =>
        console.error("remember instrument", e)
      );
    },
    []
  );

  /**
   * Play a track through `choice`. A person's choice is remembered for the
   * song; restoring and auto-assigning pass `remember: false`.
   */
  const apply = useCallback(
    async (
      track: number,
      choice: InstrumentChoice,
      label: string,
      opts: { remember?: boolean } = {}
    ) => {
      const forSong = songRef.current;
      patch(track, { choice, label, loading: true, error: null });
      try {
        await setTrackInstrument(track, choice);
        if (songRef.current !== forSong) return;
        patch(track, { loading: false });
        if (opts.remember ?? true) remember(track, choice, label);
      } catch (e) {
        if (songRef.current !== forSong) return;
        // Loading failed: the engine kept the previous source; show SF2.
        patch(track, { ...SF2, error: String(e) });
        void setTrackInstrument(track, { kind: "sf2" }).catch(() => undefined);
      }
    },
    [remember]
  );

  /** Auto-assign; `keepChosen` leaves remembered tracks alone. */
  const runAutoAssign = useCallback(
    async (target: SongInfo, keepChosen: boolean) => {
      const suggestions = await suggestInstruments(
        target.tracks.map((t) => ({
          program: t.program,
          percussion: t.isPercussion,
        }))
      );
      if (songRef.current !== target) return;
      await Promise.all(
        suggestions.map((s, i) => {
          if (keepChosen && savedRef.current.has(i)) return undefined;
          if (s) return apply(i, s.choice, s.name, { remember: false });
          // Nothing fits: a fresh auto-assignment means the SoundFont.
          return keepChosen
            ? undefined
            : apply(i, { kind: "sf2" }, "", { remember: false });
        })
      );
    },
    [apply]
  );

  // New song: everything starts on the SoundFont, then the remembered
  // choices come back and auto-assignment fills the rest.
  useEffect(() => {
    songRef.current = song;
    songIdRef.current = songId;
    savedRef.current = new Set();
    setStates(song ? song.tracks.map(() => SF2) : []);
    if (!song || !enabled) return;
    const target = song;
    void (async () => {
      let saved: SavedInstrument[] = [];
      if (songId != null) {
        try {
          saved = await loadSongInstruments(songId);
        } catch (e) {
          console.error("saved instruments", e);
        }
      }
      if (songRef.current !== target) return;
      const usable = saved.filter((s) => s.track < target.tracks.length);
      for (const s of usable) savedRef.current.add(s.track);
      await Promise.all([
        ...usable.map((s) =>
          apply(s.track, s.choice, s.label, { remember: false })
        ),
        autoAssign ? runAutoAssign(target, true) : undefined,
      ]);
    })().catch((e) => console.error("restore instruments", e));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [song, songId, enabled]);

  // Claude already switched the engine; show it and remember it.
  useEffect(() => {
    if (!enabled) return;
    const off = onInstrumentAssigned(({ track, choice, label }) => {
      const current = songRef.current;
      if (!current || track >= current.tracks.length) return;
      patch(track, { choice, label, loading: false, error: null });
      remember(track, choice, label);
    });
    return () => {
      void off.then((f) => f());
    };
  }, [enabled, remember]);

  const setAutoAssign = useCallback((on: boolean) => {
    setAutoAssignState(on);
    try {
      localStorage.setItem(AUTO_KEY, String(on));
    } catch {
      /* ignore */
    }
  }, []);

  /**
   * What a track plays through, named in the current language when it is in
   * the catalog (a remembered label is in whatever language it was saved in).
   */
  const labelOf = useCallback(
    (s: TrackInstrumentState) => {
      const key = choiceKey(s.choice);
      return installed.find((i) => choiceKey(i.choice) === key)?.name ?? s.label;
    },
    [installed]
  );

  return {
    states,
    labelOf,
    installed,
    plugins,
    autoAssign,
    setAutoAssign,
    apply,
    /** Start the current song over from auto-assignment, forgetting choices. */
    reassign: useCallback(() => {
      const target = songRef.current;
      if (!target) return;
      savedRef.current = new Set();
      const id = songIdRef.current;
      if (id != null) {
        clearSongInstruments(id).catch((e) =>
          console.error("forget instruments", e)
        );
      }
      void runAutoAssign(target, false);
    }, [runAutoAssign]),
    /** Auto-assign the tracks nobody chose for (e.g. after installing). */
    fillUnchosen: useCallback(() => {
      if (songRef.current) void runAutoAssign(songRef.current, true);
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
