import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  MidiEngine,
  keySignatureLabel,
  type PlaybackEngine,
  type PlayerState,
  type SongInfo,
  type TimeSignatureEvent,
  type KeySignatureEvent,
  type MarkerEvent,
} from "@/lib/midiEngine";
import { gmInstrumentGroups, prettyInstrumentName } from "@/lib/gm";
import { messages, useI18n } from "@/lib/i18n";
import { RustEngine } from "@/lib/rustEngine";
import AcousticsBar from "@/components/AcousticsBar";
import InstrumentPicker from "@/components/InstrumentPicker";
import LibraryManager from "@/components/LibraryManager";
import PasteDialog from "@/components/PasteDialog";
import type { PasteRequest, RollClip } from "@/lib/songsApi";
import { useTrackInstruments } from "@/lib/useTrackInstruments";
import type { RollSelection } from "@/lib/rollSelection";
import { lyricLines, type ChordEvent, type LyricLine } from "@/lib/midiText";
import { drumName } from "@/lib/midiNames";
import { scoreChords, type ChordAt } from "@/lib/scoreApi";
import { libraryAvailable } from "@/lib/songsApi";
import ControllerLane from "@/components/ControllerLane";
import EventList from "@/components/EventList";
import {
  beatOfBar,
  beatPosition,
  beatToTime,
  bpmAtTime,
  buildBarMap,
  buildTempoMap,
  playheadBar,
  timeToBeat,
} from "@/lib/timeline";

/** A piano-roll range, before it knows which song entry it belongs to. */
export type RollSelectionDraft = Omit<RollSelection, "id" | "songId" | "songTitle" | "excluded">;
/** A copied range before the song it came from is attached. */
export type RollClipDraft = Omit<RollClip, "songId" | "songTitle" | "staged">;

type EngineKind = "native" | "web";

function createEngine(kind: EngineKind): PlaybackEngine {
  return kind === "native" ? new RustEngine() : new MidiEngine();
}

/* ---------- icons ---------- */

function Icon({
  path,
  size = 18,
}: {
  path: string;
  size?: number;
}) {
  return (
    <svg width={size} height={size} viewBox="0 0 24 24" fill="currentColor">
      <path d={path} />
    </svg>
  );
}

const ICON = {
  play: "M8 5.14v13.72a1 1 0 0 0 1.53.85l10.8-6.86a1 1 0 0 0 0-1.7L9.53 4.29A1 1 0 0 0 8 5.14z",
  pause:
    "M7 4.5h3a1 1 0 0 1 1 1v13a1 1 0 0 1-1 1H7a1 1 0 0 1-1-1v-13a1 1 0 0 1 1-1zm7 0h3a1 1 0 0 1 1 1v13a1 1 0 0 1-1 1h-3a1 1 0 0 1-1-1v-13a1 1 0 0 1 1-1z",
  back: "M6 6a1 1 0 0 1 2 0v4.6l9.4-6.4A1 1 0 0 1 19 5v14a1 1 0 0 1-1.6.8L8 13.4V18a1 1 0 1 1-2 0V6z",
  forward:
    "M18 6a1 1 0 0 0-2 0v4.6L6.6 4.2A1 1 0 0 0 5 5v14a1 1 0 0 0 1.6.8L16 13.4V18a1 1 0 1 0 2 0V6z",
  stop: "M6 6h12v12H6z",
  speaker:
    "M13 4.5 8.2 8.3H4.6a1 1 0 0 0-1 1v5.4a1 1 0 0 0 1 1h3.6L13 19.5a1 1 0 0 0 1.6-.8V5.3A1 1 0 0 0 13 4.5zm3.7 3a1 1 0 0 0-.1 1.5 4 4 0 0 1 0 6 1 1 0 1 0 1.4 1.5 6 6 0 0 0 0-9 1 1 0 0 0-1.3 0z",
  upload:
    "M12 3a1 1 0 0 1 .7.3l4 4a1 1 0 0 1-1.4 1.4L13 6.4V15a1 1 0 1 1-2 0V6.4L8.7 8.7a1 1 0 0 1-1.4-1.4l4-4A1 1 0 0 1 12 3zM5 14a1 1 0 0 1 1 1v3h12v-3a1 1 0 1 1 2 0v4a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1v-4a1 1 0 0 1 1-1z",
};

/* ---------- helpers ---------- */

function formatTime(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) seconds = 0;
  const m = Math.floor(seconds / 60);
  const s = Math.floor(seconds % 60);
  const ms = Math.floor((seconds - Math.floor(seconds)) * 100);
  return `${m.toString().padStart(2, "0")}:${s
    .toString()
    .padStart(2, "0")}.${ms.toString().padStart(2, "0")}`;
}

function formatDuration(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) seconds = 0;
  const m = Math.floor(seconds / 60);
  const s = Math.floor(seconds % 60);
  return `${m}:${s.toString().padStart(2, "0")}`;
}

/* ---------- component ---------- */

/** The song to play, taken from the library (see `songsApi`). */
export interface SongSource {
  id: number;
  /** Changes per request, so the same song can be re-opened at a new spot. */
  token: number;
  /** File name, as the MIDI parser sees it. */
  name: string;
  /** What the library calls it; shown in the toolbar. */
  title: string;
  buffer: ArrayBuffer;
  /** Where to start, in seconds (MCP asks for a bar; it arrives resolved). */
  startAt?: number;
  /** Stop here — or go back to `startAt` when looping. */
  stopAt?: number;
  loop?: boolean;
  /** Sound only these tracks (MCP's "let me hear just the horns"). */
  solo?: number[];
  autoplay?: boolean;
  /** True when this is an edit waiting for approval, not the saved song. */
  staged?: boolean;
}

export default function Player({
  source,
  onLoaded,
  onStateChange,
  onOpenFiles,
  stopRequest,
  onAttachSelection,
  onExport,
  clip,
  onCopy,
  onPaste,
  pasteStaged,
  onHistory,
}: {
  source: SongSource | null;
  /** Reports what opening the song revealed, for the library entry. */
  onLoaded: (id: number, info: SongInfo) => void;
  onStateChange: (state: PlayerState) => void;
  onOpenFiles: () => void;
  /** Changes when something outside the UI asks playback to stop (MCP). */
  stopRequest: number;
  /** Hands a piano-roll range to the chat; absent when there is no chat. */
  onAttachSelection?: (sel: Omit<RollSelection, "id" | "excluded">) => void;
  /** Opens the score export; absent outside the app (no library). */
  onExport?: () => void;
  /** What ⌘V would paste, copied from this song or another. */
  clip?: RollClip | null;
  onCopy?: (clip: RollClip) => void;
  /** Resolves true once pasted. */
  onPaste?: (request: PasteRequest) => Promise<boolean>;
  /** Claude has edits waiting on the open song (a paste joins them). */
  pasteStaged?: boolean;
  /** ⌘Z / ⌘⇧Z on the open song; `seconds` is where to come back to. */
  onHistory?: (songId: number, back: boolean, seconds: number) => void;
}) {
  const { t } = useI18n();
  const engineRef = useRef<PlaybackEngine | null>(null);
  const lastSongRef = useRef<{
    buffer: ArrayBuffer;
    name: string;
    id: number;
  } | null>(null);
  const [engineKind, setEngineKind] = useState<EngineKind>(() =>
    RustEngine.available() ? "native" : "web"
  );

  const [song, setSong] = useState<SongInfo | null>(null);
  /** The file behind `song`, for the event list. */
  const [songBuffer, setSongBuffer] = useState<ArrayBuffer | null>(null);
  /** Library id of the song on screen (instruments are remembered per song). */
  const [songId, setSongId] = useState<number | null>(null);
  const [state, setState] = useState<PlayerState>("idle");
  const [position, setPosition] = useState(0);
  const [volume, setVolume] = useState(0.9);
  const [muted, setMuted] = useState<boolean[]>([]);
  const [soloed, setSoloed] = useState<boolean[]>([]);
  /** Per-track fader, in dB (FADER_MIN = off). */
  const [gainsDb, setGainsDb] = useState<number[]>([]);
  const mixSongRef = useRef<number | null>(null);
  // What the engine was last told, so only changes are sent. Reset whenever
  // a song is loaded, because loading resets the engine's tracks.
  const sentMuted = useRef<boolean[]>([]);
  const sentGain = useRef<number[]>([]);
  const [trackPrograms, setTrackPrograms] = useState<number[]>([]);
  const [loadingTracks, setLoadingTracks] = useState<Set<number>>(new Set());
  const [error, setError] = useState<string | null>(null);
  const [progressLabel, setProgressLabel] = useState("");
  const [primaryView, setPrimaryView] = useState<PrimaryView>("roll");
  const [showKeys, setShowKeys] = useState(false);
  const [showLibrary, setShowLibrary] = useState(false);

  const getLivePosition = useCallback(
    () => engineRef.current?.position() ?? 0,
    []
  );

  const volumeRef = useRef(volume);
  volumeRef.current = volume;

  const onLoadedRef = useRef(onLoaded);
  onLoadedRef.current = onLoaded;
  const onStateChangeRef = useRef(onStateChange);
  onStateChangeRef.current = onStateChange;

  const loadBuffer = useCallback(
    async (
      buffer: ArrayBuffer,
      name: string,
      id: number,
      startAt?: number,
      autoplay?: boolean,
      solo?: number[]
    ) => {
      const engine = engineRef.current;
      if (!engine) return;
      setError(null);
      setProgressLabel(messages().player.loadingSounds);
      try {
        lastSongRef.current = { buffer, name, id };
        const info = await engine.load(buffer, name);
        if (engineRef.current !== engine) return;
        setSong(info);
        setSongBuffer(buffer);
        setSongId(id);
        // Loading gave the engine fresh, unmuted, full-level tracks.
        sentMuted.current = info.tracks.map(() => false);
        sentGain.current = info.tracks.map(() => 1);
        mixSongRef.current = id;
        setMuted(info.tracks.map(() => false));
        // A solo request (MCP's "just the horns") is an ordinary solo, so
        // the S buttons show it and one click undoes it.
        setSoloed(info.tracks.map((_, i) => !!solo && solo.includes(i)));
        const savedMix = loadMix(id);
        setGainsDb(info.tracks.map((_, i) => savedMix[i] ?? 0));
        setTrackPrograms(info.tracks.map((t) => t.program));
        setLoadingTracks(new Set());
        setPosition(0);
        engine.setMasterVolume(volumeRef.current);
        if (startAt) {
          engine.seek(startAt);
          setPosition(startAt);
        }
        if (autoplay) engine.play();
        onLoadedRef.current(id, info);
      } catch (e) {
        console.error(e);
        setError(messages().player.loadFailed);
        setState("idle");
      }
    },
    []
  );

  // (Re)create the engine when the backend changes; reload the current song
  // into the new engine so the two can be A/B compared.
  useEffect(() => {
    const engine = createEngine(engineKind);
    engine.onState = (s) => {
      setState(s);
      onStateChangeRef.current(s);
    };
    engine.onTick = (p) => setPosition(p);
    engineRef.current = engine;
    const last = lastSongRef.current;
    if (last) void loadBuffer(last.buffer, last.name, last.id);
    return () => engine.dispose();
  }, [engineKind, loadBuffer]);

  // A different song was picked in the library. Keyed on the request token,
  // so editing the entry (a rename, say) never interrupts what is playing.
  const sourceRef = useRef(source);
  sourceRef.current = source;
  useEffect(() => {
    const s = sourceRef.current;
    if (s) void loadBuffer(s.buffer, s.name, s.id, s.startAt, s.autoplay, s.solo);
  }, [source?.token, loadBuffer]);

  // A passage on repeat is how a correction gets checked, so the range is
  // watched here rather than being left to the engine.
  useEffect(() => {
    const stopAt = source?.stopAt;
    if (stopAt === undefined || position < stopAt) return;
    const engine = engineRef.current;
    if (!engine || engine.state !== "playing") return;
    if (source?.loop) engine.seek(source.startAt ?? 0);
    else engine.pause();
  }, [position, source?.stopAt, source?.loop, source?.startAt]);

  const togglePlay = useCallback(() => {
    const engine = engineRef.current;
    if (!engine || !song) return;
    if (engine.state === "playing") engine.pause();
    else engine.play();
  }, [song]);

  const stop = useCallback(() => {
    const engine = engineRef.current;
    if (!engine || !song) return;
    engine.stop();
  }, [song]);

  const skip = useCallback(
    (delta: number) => {
      const engine = engineRef.current;
      if (!engine || !song) return;
      engine.seek(engine.position() + delta);
    },
    [song]
  );

  const seekTo = useCallback(
    (ratio: number) => {
      const engine = engineRef.current;
      if (!engine || !song) return;
      engine.seek(ratio * song.duration);
    },
    [song]
  );

  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (!song) return;
      const tag = (e.target as HTMLElement)?.tagName;
      if (tag === "INPUT" || tag === "TEXTAREA") return;
      if (e.code === "Space") {
        e.preventDefault();
        togglePlay();
      } else if (e.code === "Home") {
        e.preventDefault();
        engineRef.current?.seek(0);
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [song, togglePlay]);

  useEffect(() => {
    if (stopRequest > 0) engineRef.current?.pause();
  }, [stopRequest]);

  const toggleMute = useCallback((index: number) => {
    setMuted((prev) => {
      const next = [...prev];
      next[index] = !next[index];
      return next;
    });
  }, []);

  const toggleSolo = useCallback((index: number, exclusive: boolean) => {
    setSoloed((prev) => {
      // ⌥-click: this track alone, as in most DAWs.
      if (exclusive) return prev.map((_, i) => i === index && !prev[index]);
      const next = [...prev];
      next[index] = !next[index];
      return next;
    });
  }, []);

  const setTrackGainDb = useCallback((index: number, db: number) => {
    setGainsDb((prev) => {
      const next = [...prev];
      next[index] = db;
      if (mixSongRef.current != null) saveMix(mixSongRef.current, next);
      return next;
    });
  }, []);

  // Audible = not muted, and soloed if anything is.
  const silenced = useMemo(() => {
    const anySolo = soloed.some(Boolean);
    return muted.map((m, i) => m || (anySolo && !soloed[i]));
  }, [muted, soloed]);

  useEffect(() => {
    const engine = engineRef.current;
    if (!engine) return;
    silenced.forEach((m, i) => {
      if (sentMuted.current[i] !== m) {
        engine.setTrackMuted(i, m);
        sentMuted.current[i] = m;
      }
    });
  }, [silenced]);

  useEffect(() => {
    const engine = engineRef.current;
    if (!engine) return;
    gainsDb.forEach((db, i) => {
      const g = dbToGain(db);
      if (sentGain.current[i] !== g) {
        engine.setTrackGain(i, g);
        sentGain.current[i] = g;
      }
    });
  }, [gainsDb]);

  const onVolume = useCallback((v: number) => {
    setVolume(v);
    engineRef.current?.setMasterVolume(v);
  }, []);

  const changeTrackProgram = useCallback(
    async (trackIndex: number, program: number) => {
      const engine = engineRef.current;
      if (!engine) return;
      setTrackPrograms((prev) => {
        const next = [...prev];
        next[trackIndex] = program;
        return next;
      });
      setLoadingTracks((prev) => {
        const next = new Set(prev);
        next.add(trackIndex);
        return next;
      });
      try {
        await engine.setTrackProgram(trackIndex, program);
      } finally {
        setLoadingTracks((prev) => {
          const next = new Set(prev);
          next.delete(trackIndex);
          return next;
        });
      }
    },
    []
  );

  // Family names are in the current language.
  const instrumentGroups = useMemo(() => gmInstrumentGroups(), [t]);
  const instruments = useTrackInstruments(song, songId, engineKind === "native");

  const attachSelection = useMemo(() => {
    if (!onAttachSelection) return undefined;
    return (draft: RollSelectionDraft) =>
      onAttachSelection({
        ...draft,
        songId,
        songTitle: source?.title ?? song?.name.replace(/\.midi?$/i, "") ?? "",
      });
  }, [onAttachSelection, songId, source, song]);

  // Chords from the notes, for files that name none (read by the Rust side).
  const staged = source?.staged === true;
  const estimateChords = useMemo(
    () => (songId != null && libraryAvailable() ? () => scoreChords(songId, staged) : undefined),
    [songId, staged]
  );

  const copySelection = useMemo(() => {
    if (!onCopy || songId == null) return undefined;
    return (draft: RollClipDraft) =>
      onCopy({
        ...draft,
        songId,
        songTitle: source?.title ?? song?.name.replace(/\.midi?$/i, "") ?? "",
        staged: source?.staged === true,
      });
  }, [onCopy, songId, source, song]);

  // ⌘V: paste what was copied, at the playhead's bar.
  const [pasting, setPasting] = useState(false);
  useEffect(() => {
    if (!clip || !onPaste || !song || songId == null) return;
    const onKey = (e: KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey) || e.key.toLowerCase() !== "v") return;
      const el = e.target as HTMLElement | null;
      if (el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.isContentEditable)) return;
      e.preventDefault();
      setPasting(true);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [clip, onPaste, song, songId]);

  // ⌘Z undoes the song's last change, ⌘⇧Z redoes it. In a text field the
  // keys are the field's own.
  const positionRef = useRef(position);
  positionRef.current = position;
  useEffect(() => {
    if (!onHistory || songId == null) return;
    const onKey = (e: KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey) || e.key.toLowerCase() !== "z" || e.altKey) return;
      const el = e.target as HTMLElement | null;
      if (el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.isContentEditable)) return;
      if (document.querySelector(".modal-backdrop")) return;
      e.preventDefault();
      onHistory(songId, !e.shiftKey, positionRef.current);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onHistory, songId]);

  const trackList = useMemo(() => {
    if (!song) return null;
    return (
      <div className="tracks">
        <div className="tracks-head">{t.player.tracks(song.tracks.length)}</div>
        {song.tracks.map((track) => {
          const currentProgram =
            trackPrograms[track.index] ?? track.program;
          const isLoading = loadingTracks.has(track.index);
          const source = instruments.states[track.index];
          const usesSf2 = !source || source.choice.kind === "sf2";
          const instrumentLabel = !usesSf2
            ? instruments.labelOf(source)
            : track.isPercussion
            ? track.instrument
            : prettyInstrumentName(currentProgram);
          return (
            <div className="track-row" key={track.index}>
              <span
                className="track-swatch"
                style={{
                  background: track.color,
                  opacity: silenced[track.index] ? 0.3 : 1,
                }}
              />
              <div className="track-info">
                <div
                  className="track-name"
                  style={{ opacity: silenced[track.index] ? 0.5 : 1 }}
                >
                  {track.name}
                </div>
                <div className="track-detail">
                  {instrumentLabel}
                  {isLoading && (
                    <span style={{ color: "var(--accent)", marginLeft: 6 }}>
                      {t.common.loading}
                    </span>
                  )}
                  {" · "}
                  CH {track.channel + 1}
                  {" · "}
                  {track.noteCount} notes
                </div>
                {engineKind === "native" && (
                  <InstrumentPicker
                    track={track.index}
                    trackName={track.name}
                    instruments={instruments}
                  />
                )}
                {!track.isPercussion && usesSf2 && (
                  <select
                    className="track-instrument-select"
                    value={currentProgram}
                    disabled={isLoading}
                    onChange={(e) =>
                      changeTrackProgram(
                        track.index,
                        parseInt(e.target.value, 10)
                      )
                    }
                    aria-label={t.player.changeInstrument}
                    title={t.player.changeInstrumentTitle}
                  >
                    {instrumentGroups.map((group) => (
                      <optgroup key={group.family} label={group.family}>
                        {group.instruments.map((inst) => (
                          <option key={inst.program} value={inst.program}>
                            {inst.name}
                          </option>
                        ))}
                      </optgroup>
                    ))}
                  </select>
                )}
              </div>
              <Fader
                db={gainsDb[track.index] ?? 0}
                onChange={(db) => setTrackGainDb(track.index, db)}
                label={t.player.volumeOf(track.name)}
              />
              <button
                className={`solo-btn${soloed[track.index] ? " on" : ""}`}
                onClick={(e) => toggleSolo(track.index, e.altKey)}
                aria-label={soloed[track.index] ? t.player.unsolo : t.player.solo}
                title={t.player.soloTitle}
              >
                S
              </button>
              <button
                className={`mute-btn${muted[track.index] ? " muted" : ""}`}
                onClick={() => toggleMute(track.index)}
                aria-label={muted[track.index] ? t.player.unmute : t.player.mute}
                title={muted[track.index] ? t.player.muted : t.player.mute}
              >
                M
              </button>
            </div>
          );
        })}
      </div>
    );
  }, [
    song,
    muted,
    soloed,
    silenced,
    gainsDb,
    toggleMute,
    toggleSolo,
    setTrackGainDb,
    trackPrograms,
    loadingTracks,
    instrumentGroups,
    changeTrackProgram,
    engineKind,
    instruments,
    t,
  ]);

  const isPlaying = state === "playing";

  const songTempoMap = useMemo(
    () => (song ? buildTempoMap(song.tempos, song.duration) : null),
    [song]
  );
  const currentBpm = useMemo(() => {
    if (!song) return 0;
    if (!songTempoMap || song.tempos.length <= 1) return song.bpm;
    return Math.round(bpmAtTime(songTempoMap, position));
  }, [song, songTempoMap, position]);

  const currentTimeSig = useMemo<TimeSignatureEvent | null>(() => {
    if (!song) return null;
    let cur = song.timeSignatures[0] ?? null;
    for (const t of song.timeSignatures) {
      if (t.time <= position + 0.0001) cur = t;
      else break;
    }
    return cur;
  }, [song, position]);

  const currentKey = useMemo<KeySignatureEvent | null>(() => {
    if (!song || song.keySignatures.length === 0) return null;
    let cur = song.keySignatures[0];
    for (const k of song.keySignatures) {
      if (k.time <= position + 0.0001) cur = k;
      else break;
    }
    return cur;
  }, [song, position]);

  return (
    <div className="card">
      {!song && state !== "loading" && (
        <>
          <DropZone onPick={onOpenFiles} />
          {error && <div className="error">{error}</div>}
        </>
      )}

      {state === "loading" && (
        <div className="loading">
          <div className="spinner" />
          <p>{progressLabel}</p>
        </div>
      )}

      {song && state !== "loading" && (
        <>
          <Toolbar
            song={song}
            title={source?.title ?? song.name.replace(/\.midi?$/i, "")}
            position={position}
            isPlaying={isPlaying}
            volume={volume}
            currentBpm={currentBpm}
            currentTimeSig={currentTimeSig}
            currentKey={currentKey}
            onTogglePlay={togglePlay}
            onStop={stop}
            onSkip={skip}
            onVolume={onVolume}
            onOpen={onOpenFiles}
            onExport={onExport}
          />

          {engineKind === "native" && (
            <AcousticsBar onOpenLibrary={() => setShowLibrary(true)} />
          )}

          <ViewSwitcher
            primary={primaryView}
            onPrimaryChange={setPrimaryView}
            showKeys={showKeys}
            onToggleKeys={() => setShowKeys((v) => !v)}
            song={song}
            position={position}
            isPlaying={isPlaying}
            muted={silenced}
            onSeek={seekTo}
            getLivePosition={getLivePosition}
            onAttach={attachSelection}
            onCopy={copySelection}
            estimateChords={estimateChords}
            buffer={songBuffer}
          />

          {trackList}

          {song.textMeta.length > 0 && (
            <div className="meta-strip">
              {song.textMeta.slice(0, 3).map((t, i) => (
                <span key={i} className="meta-line">
                  {t}
                </span>
              ))}
            </div>
          )}

          {error && <div className="error">{error}</div>}

          <div className="statusbar">
            <span className="kbd-hint">
              <kbd>Space</kbd> {t.player.hintPlay}
              <span style={{ opacity: 0.5, margin: "0 6px" }}>·</span>
              <kbd>Cmd</kbd>+<kbd>Wheel</kbd> {t.player.hintZoom}
              <span style={{ opacity: 0.5, margin: "0 6px" }}>·</span>
              <kbd>Wheel</kbd> {t.player.hintScroll}
            </span>
            {RustEngine.available() && (
              <label className="engine-select">
                {t.player.engine}
                <select
                  value={engineKind}
                  onChange={(e) => setEngineKind(e.target.value as EngineKind)}
                >
                  <option value="native">{t.player.engineNative}</option>
                  <option value="web">{t.player.engineWeb}</option>
                </select>
              </label>
            )}
          </div>
        </>
      )}

      {pasting && clip && onPaste && song && songId != null && (
        <PasteDialog
          clip={clip}
          songId={songId}
          tracks={song.tracks.map((tr) => ({ index: tr.index, name: tr.name }))}
          defaultBar={playheadBar(song, position)}
          barCount={buildBarMap(song.timeSignatures, buildTempoMap(song.tempos, song.duration)).bars.length}
          staged={pasteStaged === true}
          toBeat={(bar, beat) => beatOfBar(song, bar) + beat - 1}
          onPaste={onPaste}
          onClose={() => setPasting(false)}
        />
      )}

      {showLibrary && engineKind === "native" && (
        <LibraryManager
          instruments={instruments}
          onClose={() => setShowLibrary(false)}
        />
      )}

    </div>
  );
}

/* ---------- drop zone ---------- */

function DropZone({ onPick }: { onPick: () => void }) {
  const { t } = useI18n();
  return (
    <div className="dropzone">
      <div className="dropzone-icon">
        <Icon path={ICON.upload} size={26} />
      </div>
      <h2>{t.player.dropTitle}</h2>
      <p>{t.player.dropSub}</p>
      <button className="btn-filled" onClick={onPick}>
        {t.player.chooseFile}
      </button>
    </div>
  );
}

/* ---------- track faders ---------- */

/** Bottom of the fader: silence. */
const FADER_MIN = -48;
const FADER_MAX = 6;

function dbToGain(db: number): number {
  return db <= FADER_MIN ? 0 : Math.pow(10, db / 20);
}

function dbLabel(db: number): string {
  if (db <= FADER_MIN) return "−∞";
  if (Math.abs(db) < 0.05) return "0.0";
  return `${db > 0 ? "+" : "−"}${Math.abs(db).toFixed(1)}`;
}

const MIX_KEY = (id: number) => `otomo.mix.${id}`;

/** A song's fader positions, kept per song in this browser profile. */
function loadMix(id: number): number[] {
  try {
    const raw = localStorage.getItem(MIX_KEY(id));
    const parsed: unknown = raw ? JSON.parse(raw) : [];
    return Array.isArray(parsed) ? parsed.map((v) => (typeof v === "number" ? v : 0)) : [];
  } catch {
    return [];
  }
}

function saveMix(id: number, db: number[]) {
  try {
    if (db.every((v) => v === 0)) localStorage.removeItem(MIX_KEY(id));
    else localStorage.setItem(MIX_KEY(id), JSON.stringify(db));
  } catch {
    /* only the memory of the mix is lost */
  }
}

/**
 * One track's level, in dB. Double-click puts it back to 0 dB; the bar
 * fills from the left so levels can be compared down the list at a glance.
 */
function Fader({
  db,
  onChange,
  label,
}: {
  db: number;
  onChange: (db: number) => void;
  label: string;
}) {
  const { t } = useI18n();
  const fill = ((db - FADER_MIN) / (FADER_MAX - FADER_MIN)) * 100;
  const unity = ((0 - FADER_MIN) / (FADER_MAX - FADER_MIN)) * 100;
  return (
    <div className="fader" title={t.player.faderReset}>
      <div className="fader-track">
        <div className="fader-fill" style={{ width: `${fill}%` }} />
        <div className="fader-unity" style={{ left: `${unity}%` }} />
        <input
          type="range"
          min={FADER_MIN}
          max={FADER_MAX}
          step={0.5}
          value={db}
          aria-label={label}
          aria-valuetext={`${dbLabel(db)} dB`}
          onChange={(e) => onChange(Number(e.target.value))}
          onDoubleClick={() => onChange(0)}
        />
      </div>
      <span className={`fader-value${db === 0 ? "" : " moved"}`}>{dbLabel(db)}</span>
    </div>
  );
}

/* ---------- top toolbar ---------- */

function Toolbar({
  song,
  title,
  position,
  isPlaying,
  volume,
  currentBpm,
  currentTimeSig,
  currentKey,
  onTogglePlay,
  onStop,
  onSkip,
  onVolume,
  onOpen,
  onExport,
}: {
  song: SongInfo;
  /** The library's name for the song, which the user can rename. */
  title: string;
  position: number;
  isPlaying: boolean;
  volume: number;
  currentBpm: number;
  currentTimeSig: TimeSignatureEvent | null;
  currentKey: KeySignatureEvent | null;
  onTogglePlay: () => void;
  onStop: () => void;
  onSkip: (delta: number) => void;
  onVolume: (v: number) => void;
  onOpen: () => void;
  onExport?: () => void;
}) {
  const { t } = useI18n();
  const tempoVaries = song.bpmMin !== song.bpmMax;
  const tsVaries = song.timeSignatures.length > 1;
  const keyVaries = song.keySignatures.length > 1;
  return (
    <div className="toolbar" data-tauri-drag-region>
      <div className="toolbar-group">
        <button
          className="transport-btn"
          onClick={() => onSkip(-5)}
          aria-label={t.player.back5}
          title={t.player.back5}
        >
          <Icon path={ICON.back} size={16} />
        </button>
        <button
          className="transport-btn primary"
          onClick={onTogglePlay}
          aria-label={isPlaying ? t.player.pause : t.player.play}
        >
          <Icon path={isPlaying ? ICON.pause : ICON.play} size={18} />
        </button>
        <button
          className="transport-btn"
          onClick={onStop}
          aria-label={t.player.stop}
          title={t.player.stop}
        >
          <Icon path={ICON.stop} size={12} />
        </button>
        <button
          className="transport-btn"
          onClick={() => onSkip(5)}
          aria-label={t.player.forward5}
          title={t.player.forward5}
        >
          <Icon path={ICON.forward} size={16} />
        </button>
      </div>

      <div className="toolbar-sep" />

      <div className="time-display">
        {formatTime(position)}
        <span className="total">/ {formatDuration(song.duration)}</span>
      </div>

      <div className="toolbar-sep" />

      <div className="song-title">
        <div className="song-name">{title}</div>
        <div className="song-meta">
          {tempoVaries ? (
            <>
              <span style={{ color: "var(--accent)" }}>{currentBpm}</span>
              <span style={{ opacity: 0.6 }}> BPM</span>
              <span style={{ opacity: 0.5, fontSize: 10, marginLeft: 4 }}>
                ({song.bpmMin}–{song.bpmMax})
              </span>
            </>
          ) : (
            <>{song.bpm} BPM</>
          )}
          {currentTimeSig && (
            <>
              <span className="dot">·</span>
              <span
                style={{
                  color: tsVaries ? "var(--accent)" : undefined,
                }}
              >
                {currentTimeSig.numerator}/{currentTimeSig.denominator}
              </span>
            </>
          )}
          {currentKey && (
            <>
              <span className="dot">·</span>
              <span
                style={{
                  color: keyVaries ? "var(--accent)" : undefined,
                }}
              >
                {keySignatureLabel(currentKey)}
              </span>
            </>
          )}
          <span className="dot">·</span>
          {song.tracks.length} TRK
          <span className="dot">·</span>
          {song.notes.length} NOTES
        </div>
      </div>

      <div className="toolbar-sep" />

      <div className="volume-cluster">
        <Icon path={ICON.speaker} size={15} />
        <input
          type="range"
          min={0}
          max={1}
          step={0.01}
          value={volume}
          onChange={(e) => onVolume(Number(e.target.value))}
          aria-label={t.player.volume}
        />
      </div>

      <div className="toolbar-sep" />

      <button className="btn-ghost" onClick={onOpen}>
        {t.player.open}
      </button>
      {onExport && (
        <button className="btn-ghost" onClick={onExport} title={t.player.exportScoreTitle}>
          {t.player.exportScore}
        </button>
      )}
    </div>
  );
}

/* ---------- piano roll ---------- */

const RULER_H = 38;
const ROLL_TOTAL_H = 360;
const ROLL_NOTES_H = ROLL_TOTAL_H - RULER_H;
const OVERVIEW_H = 64;
/** The chord and lyric lanes under the ruler. */
const TEXT_LANE_H = 20;
/** Drum rows: as tall as fits this, within the row limits. */
const DRUMS_MAX_H = 180;
const DRUM_ROW_MIN = 9;
const DRUM_ROW_MAX = 15;
/** Where the playhead line starts (the ruler's flag sits above). */
const PLAYHEAD_TOP = 34;
const LANES_KEY = "otomo.roll.lanes";

/** Which extra lanes and drawings the roll shows; remembered per viewer. */
interface RollLanes {
  chords: boolean;
  lyrics: boolean;
  bends: boolean;
  controllers: boolean;
}

function readLanes(): RollLanes {
  const all = { chords: true, lyrics: true, bends: true, controllers: true };
  try {
    return { ...all, ...JSON.parse(localStorage.getItem(LANES_KEY) ?? "{}") };
  } catch {
    return all;
  }
}

/** The roll's vertical layout: ruler, text lanes, pitched notes, drum rows. */
interface RollLayout {
  chordsTop: number;
  lyricsTop: number;
  notesTop: number;
  notesH: number;
  drumTop: number;
  drumRowH: number;
  /** Drum keys from the top row down (highest first, as on a keyboard). */
  drumKeys: number[];
  totalH: number;
}

/** Index of the last item at or before `t`, or -1. */
function lastAtOrBefore(items: { time: number }[], t: number): number {
  let lo = 0;
  let hi = items.length - 1;
  let found = -1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if (items[mid].time <= t) {
      found = mid;
      lo = mid + 1;
    } else hi = mid - 1;
  }
  return found;
}

/** What the text lanes need to know about the view. */
interface LaneView {
  startT: number;
  endT: number;
  scrollX: number;
  pxPerSec: number;
  w: number;
  dark: boolean;
  position: number;
}

const LANE_FONT = '11px -apple-system, BlinkMacSystemFont, "SF Pro Text", system-ui, sans-serif';

/** Draw `text` within `room` pixels, cut short with "…" if it does not fit. */
function fitText(ctx: CanvasRenderingContext2D, text: string, x: number, y: number, room: number) {
  if (ctx.measureText(text).width <= room) {
    ctx.fillText(text, x, y);
    return;
  }
  let lo = 0;
  let hi = text.length;
  while (lo < hi) {
    const mid = (lo + hi + 1) >> 1;
    if (ctx.measureText(text.slice(0, mid) + "…").width <= room) lo = mid;
    else hi = mid - 1;
  }
  if (lo > 0) ctx.fillText(text.slice(0, lo) + "…", x, y);
}

/** Chord symbols, each from where it starts to the next; the one playing lit. */
function drawChordLane(
  ctx: CanvasRenderingContext2D,
  v: LaneView,
  top: number,
  chords: ChordEvent[],
  info: { estimated: boolean; pending: boolean; tag: string; pendingText: string; duration: number }
) {
  const h = TEXT_LANE_H;
  ctx.save();
  ctx.fillStyle = v.dark ? "#101215" : "#e9ebee";
  ctx.fillRect(0, top, v.w, h);
  ctx.font = `600 ${LANE_FONT}`;
  ctx.textBaseline = "middle";
  const ink = v.dark ? "#e8eaed" : "#1a1a1a";
  const faint = v.dark ? "rgba(255,255,255,0.4)" : "rgba(0,0,0,0.45)";
  const tick = v.dark ? "rgba(255,255,255,0.18)" : "rgba(0,0,0,0.2)";
  for (let i = Math.max(0, lastAtOrBefore(chords, v.startT)); i < chords.length; i++) {
    const c = chords[i];
    if (c.time > v.endT) break;
    const end = i + 1 < chords.length ? chords[i + 1].time : info.duration;
    if (c.name === "—") continue;
    const x0 = c.time * v.pxPerSec - v.scrollX;
    const x1 = end * v.pxPerSec - v.scrollX;
    if (x1 < 0) continue;
    const playing = c.time <= v.position && v.position < end;
    if (playing) {
      ctx.fillStyle = "rgba(245,166,35,0.18)";
      ctx.fillRect(x0, top, x1 - x0, h);
    }
    ctx.fillStyle = tick;
    ctx.fillRect(Math.round(x0), top + 3, 1, h - 6);
    // A chord that started off-screen keeps its name at the left edge.
    const tx = Math.max(x0 + 4, 4);
    const room = x1 - tx - 3;
    if (room < 10) continue;
    ctx.fillStyle = c.name === "?" ? faint : playing ? "#f5a623" : ink;
    fitText(ctx, c.name, tx, top + h / 2, room);
  }
  // Say when the chords are a reading of the notes, not the file's own.
  if (info.estimated || info.pending) {
    ctx.font = LANE_FONT;
    const label = info.pending ? info.pendingText : info.tag;
    const lw = ctx.measureText(label).width + 10;
    ctx.fillStyle = v.dark ? "rgba(16,18,21,0.9)" : "rgba(233,235,238,0.92)";
    ctx.fillRect(v.w - lw - 4, top + 2, lw, h - 4);
    ctx.fillStyle = faint;
    ctx.fillText(label, v.w - lw + 1, top + h / 2);
  }
  ctx.restore();
}

/**
 * Lyrics: each syllable where it is sung when the line has room, otherwise
 * the line as one string from where it starts. Sung syllables are lit.
 */
function drawLyricLane(
  ctx: CanvasRenderingContext2D,
  v: LaneView,
  top: number,
  lines: LyricLine[],
  duration: number
) {
  const h = TEXT_LANE_H;
  ctx.save();
  ctx.fillStyle = v.dark ? "#0e0f12" : "#eef0f2";
  ctx.fillRect(0, top, v.w, h);
  ctx.font = LANE_FONT;
  ctx.textBaseline = "middle";
  const sung = v.dark ? "#d7b8f3" : "#6a2ca8";
  const coming = v.dark ? "rgba(255,255,255,0.55)" : "rgba(0,0,0,0.55)";
  const y = top + h / 2;
  for (let i = Math.max(0, lastAtOrBefore(lines, v.startT)); i < lines.length; i++) {
    const line = lines[i];
    if (line.time > v.endT) break;
    const lineEnd = i + 1 < lines.length ? lines[i + 1].time : duration;
    const x0 = line.time * v.pxPerSec - v.scrollX;
    const x1 = lineEnd * v.pxPerSec - v.scrollX;
    if (x1 < 0) continue;
    const xs = line.syllables.map((s) => s.time * v.pxPerSec - v.scrollX);
    const fits = line.syllables.every((s, k) => {
      const next = k + 1 < xs.length ? xs[k + 1] : x1;
      return ctx.measureText(s.text).width <= next - xs[k] + 1;
    });
    if (fits) {
      line.syllables.forEach((s, k) => {
        ctx.fillStyle = s.time <= v.position ? sung : coming;
        ctx.fillText(s.text, xs[k], y);
      });
    } else {
      const text = line.syllables.map((s) => s.text).join("");
      const tx = Math.max(x0, 2);
      const room = x1 - tx - 4;
      if (room < 12) continue;
      ctx.fillStyle = line.time <= v.position ? sung : coming;
      fitText(ctx, text, tx, y, room);
    }
  }
  ctx.restore();
}

const MIN_PX_PER_SEC = 6;
const MAX_PX_PER_SEC = 480;
const FOLLOW_EDGE = 0.65;

function PianoRoll({
  song,
  position,
  isPlaying,
  muted,
  onSeek,
  getLivePosition,
  onAttach,
  onCopy,
  estimateChords,
}: {
  song: SongInfo;
  position: number;
  isPlaying: boolean;
  muted: boolean[];
  onSeek: (ratio: number) => void;
  getLivePosition: () => number;
  /** Drag in the notes area selects a range for this; a click still seeks. */
  onAttach?: (sel: RollSelectionDraft) => void;
  /** …or to copy it (⌘C). */
  onCopy?: (clip: RollClipDraft) => void;
  /** Works out the chords when the file names none (library songs only). */
  estimateChords?: () => Promise<ChordAt[]>;
}) {
  const { t } = useI18n();
  const stageRef = useRef<HTMLDivElement | null>(null);
  const mainRef = useRef<HTMLDivElement | null>(null);
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const playheadRef = useRef<HTMLDivElement | null>(null);
  const overviewWrapRef = useRef<HTMLDivElement | null>(null);
  const overviewRef = useRef<HTMLCanvasElement | null>(null);

  const [viewportW, setViewportW] = useState(800);
  const [overviewW, setOverviewW] = useState(800);
  const [pxPerSec, setPxPerSec] = useState(40);
  const [scrollX, setScrollX] = useState(0);
  const [follow, setFollow] = useState(true);
  const [dark, setDark] = useState(true);

  const [lanes, setLanes] = useState<RollLanes>(readLanes);
  useEffect(() => {
    try {
      localStorage.setItem(LANES_KEY, JSON.stringify(lanes));
    } catch {
      /* ignore */
    }
  }, [lanes]);
  const toggleLane = (key: keyof RollLanes) => setLanes((l) => ({ ...l, [key]: !l[key] }));

  // Chords: the file's own chart, else worked out from the notes on demand.
  const [estimated, setEstimated] = useState<{ song: SongInfo; chords: ChordAt[] | null } | null>(null);
  const wantEstimate = lanes.chords && song.chords.length === 0 && !!estimateChords;
  useEffect(() => {
    if (!wantEstimate || !estimateChords || estimated?.song === song) return;
    let cancelled = false;
    setEstimated({ song, chords: null });
    estimateChords()
      .then((chords) => !cancelled && setEstimated({ song, chords }))
      .catch((e) => {
        console.error("score_chords", e);
        if (!cancelled) setEstimated({ song, chords: [] });
      });
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [wantEstimate, song, estimateChords]);
  const chordList: ChordEvent[] =
    song.chords.length > 0 ? song.chords : estimated?.song === song ? estimated.chords ?? [] : [];
  const chordsEstimated = song.chords.length === 0;
  const chordsPending = wantEstimate && (estimated?.song !== song || estimated.chords === null);
  const showChords = lanes.chords && (song.chords.length > 0 || !!estimateChords);

  const lyrics = useMemo(
    () => lyricLines(song.markers.filter((m) => m.kind === "lyrics")),
    [song]
  );
  const showLyrics = lanes.lyrics && lyrics.length > 0;
  const hasBends = useMemo(() => song.channels.some((c) => c.bend.length > 0), [song]);

  // Drums get rows of their own; the keyboard is for pitched notes.
  const isDrum = useMemo(() => song.tracks.map((tr) => tr.isPercussion), [song]);
  const padded = useMemo(() => {
    let min = 127;
    let max = 0;
    for (const n of song.notes) {
      if (isDrum[n.trackIndex]) continue;
      if (n.midi < min) min = n.midi;
      if (n.midi > max) max = n.midi;
    }
    if (min > max) {
      min = 48;
      max = 72;
    }
    const lo = Math.max(0, min - 2);
    const hi = Math.min(127, max + 2);
    return { lo, hi, range: Math.max(12, hi - lo) };
  }, [song, isDrum]);

  const layout = useMemo<RollLayout>(() => {
    const keys = new Set<number>();
    let pitched = false;
    for (const n of song.notes) {
      if (isDrum[n.trackIndex]) keys.add(n.midi);
      else pitched = true;
    }
    const drumKeys = [...keys].sort((a, b) => b - a);
    const chordsTop = RULER_H;
    const lyricsTop = chordsTop + (showChords ? TEXT_LANE_H : 0);
    const notesTop = lyricsTop + (showLyrics ? TEXT_LANE_H : 0);
    const notesH = pitched || drumKeys.length === 0 ? ROLL_NOTES_H : 0;
    const drumTop = notesTop + notesH;
    const maxRow = notesH === 0 ? DRUM_ROW_MAX + 4 : DRUM_ROW_MAX;
    const drumRowH = drumKeys.length
      ? Math.max(DRUM_ROW_MIN, Math.min(maxRow, Math.floor(DRUMS_MAX_H / drumKeys.length)))
      : 0;
    return {
      chordsTop,
      lyricsTop,
      notesTop,
      notesH,
      drumTop,
      drumRowH,
      drumKeys,
      totalH: drumTop + drumKeys.length * drumRowH,
    };
  }, [song, isDrum, showChords, showLyrics]);

  const tempoMap = useMemo(
    () => buildTempoMap(song.tempos, song.duration),
    [song]
  );
  const barMap = useMemo(
    () => buildBarMap(song.timeSignatures, tempoMap),
    [song, tempoMap]
  );
  const barTimes = useMemo(
    () => barMap.bars.map((b) => beatToTime(tempoMap, b.beat)),
    [barMap, tempoMap]
  );

  /* range selection: whole beats × whole keys (or drum rows), to hand to Claude */
  type Pane = "keys" | "drums";
  const [sel, setSel] = useState<{ b0: number; b1: number; lo: number; hi: number; pane: Pane } | null>(null);
  const selDragRef = useRef<{ beat: number; midi: number; pane: Pane; x: number; y: number; moved: boolean } | null>(null);
  useEffect(() => setSel(null), [song]);
  useEffect(() => {
    if (!sel) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && setSel(null);
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [sel]);

  const contentW = song.duration * pxPerSec;
  const maxScroll = Math.max(0, contentW - viewportW);

  const fitPxPerSec = useMemo(
    () => Math.max(MIN_PX_PER_SEC, viewportW / Math.max(0.1, song.duration)),
    [viewportW, song.duration]
  );

  /* reset on song change */
  useEffect(() => {
    const initial = Math.max(48, viewportW / Math.max(0.1, song.duration));
    setPxPerSec(Math.min(MAX_PX_PER_SEC, initial));
    setScrollX(0);
    setFollow(true);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [song]);

  /* track main width */
  useEffect(() => {
    const el = mainRef.current;
    if (!el) return;
    const update = () => setViewportW(el.clientWidth || 800);
    update();
    const obs = new ResizeObserver(update);
    obs.observe(el);
    return () => obs.disconnect();
  }, []);

  /* track overview width */
  useEffect(() => {
    const el = overviewWrapRef.current;
    if (!el) return;
    const update = () => setOverviewW(el.clientWidth || 800);
    update();
    const obs = new ResizeObserver(update);
    obs.observe(el);
    return () => obs.disconnect();
  }, []);

  /* dark mode */
  useEffect(() => {
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    setDark(mq.matches);
    const handler = (e: MediaQueryListEvent) => setDark(e.matches);
    mq.addEventListener("change", handler);
    return () => mq.removeEventListener("change", handler);
  }, []);

  /* clamp scroll when content shrinks */
  useEffect(() => {
    setScrollX((s) => Math.max(0, Math.min(maxScroll, s)));
  }, [maxScroll]);

  /* follow playhead during playback */
  useEffect(() => {
    if (!isPlaying || !follow) return;
    const playX = position * pxPerSec;
    const leftEdge = scrollX;
    const rightEdge = scrollX + viewportW;
    const triggerRight = scrollX + viewportW * FOLLOW_EDGE;
    if (playX > triggerRight) {
      const next = Math.min(
        maxScroll,
        playX - viewportW * (1 - FOLLOW_EDGE) * 0.5
      );
      setScrollX(next);
    } else if (playX < leftEdge - 4 || playX > rightEdge + 4) {
      setScrollX(Math.max(0, Math.min(maxScroll, playX - viewportW * 0.3)));
    }
  }, [position, isPlaying, follow, pxPerSec, viewportW, maxScroll, scrollX]);

  /* wheel: scroll + cmd/ctrl to zoom (over the notes or the lane below) */
  useEffect(() => {
    const el = stageRef.current;
    if (!el) return;
    const handler = (e: WheelEvent) => {
      if (e.ctrlKey || e.metaKey) {
        e.preventDefault();
        const rect = el.getBoundingClientRect();
        const cursorX = e.clientX - rect.left;
        const timeAtCursor = (scrollX + cursorX) / pxPerSec;
        const factor = Math.pow(1.0018, -e.deltaY);
        const next = Math.max(
          MIN_PX_PER_SEC,
          Math.min(MAX_PX_PER_SEC, pxPerSec * factor)
        );
        const newContentW = song.duration * next;
        const newMaxScroll = Math.max(0, newContentW - viewportW);
        const newScrollX = Math.max(
          0,
          Math.min(newMaxScroll, timeAtCursor * next - cursorX)
        );
        setPxPerSec(next);
        setScrollX(newScrollX);
        setFollow(false);
      } else {
        const dx = Math.abs(e.deltaX) > Math.abs(e.deltaY) ? e.deltaX : e.deltaY;
        if (Math.abs(dx) < 0.1) return;
        e.preventDefault();
        setScrollX((prev) =>
          Math.max(0, Math.min(maxScroll, prev + dx))
        );
        if (Math.abs(dx) > 2) setFollow(false);
      }
    };
    el.addEventListener("wheel", handler, { passive: false });
    return () => el.removeEventListener("wheel", handler);
  }, [scrollX, pxPerSec, viewportW, maxScroll, song.duration]);

  /* render main canvas */
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    const w = viewportW;
    const h = layout.totalH;
    if (canvas.width !== w * dpr || canvas.height !== h * dpr) {
      canvas.width = w * dpr;
      canvas.height = h * dpr;
    }
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);

    const rollBg = dark ? "#0a0b0d" : "#f4f5f7";
    const rulerBg = dark ? "#15171a" : "#eceef1";
    const rulerText = dark ? "#8a8e95" : "#5d6168";
    const rulerTick = dark ? "rgba(255,255,255,0.18)" : "rgba(0,0,0,0.25)";
    const octaveLine = dark ? "rgba(255,255,255,0.05)" : "rgba(0,0,0,0.05)";
    const beatLine = dark ? "rgba(255,255,255,0.03)" : "rgba(0,0,0,0.03)";
    const barLine = dark ? "rgba(255,255,255,0.08)" : "rgba(0,0,0,0.08)";
    const playhead = "#f5a623";
    const mutedColor = dark ? "#3a3c40" : "#bdc0c5";

    ctx.fillStyle = rollBg;
    ctx.fillRect(0, 0, w, h);

    /* ruler bg */
    ctx.fillStyle = rulerBg;
    ctx.fillRect(0, 0, w, RULER_H);
    ctx.strokeStyle = dark ? "rgba(255,255,255,0.06)" : "rgba(0,0,0,0.08)";
    ctx.beginPath();
    ctx.moveTo(0, RULER_H - 0.5);
    ctx.lineTo(w, RULER_H - 0.5);
    ctx.stroke();

    /* ruler ticks — choose stride so labels are readable */
    const targetPx = 80;
    const desiredSec = targetPx / pxPerSec;
    const steps = [0.1, 0.25, 0.5, 1, 2, 5, 10, 15, 30, 60, 120, 300];
    const stepSec = steps.find((s) => s >= desiredSec) ?? 600;
    const startT = scrollX / pxPerSec;
    const endT = (scrollX + w) / pxPerSec;
    const firstTick = Math.floor(startT / stepSec) * stepSec;

    ctx.fillStyle = rulerText;
    ctx.strokeStyle = rulerTick;
    ctx.lineWidth = 1;
    ctx.font =
      '11px ui-monospace, SFMono-Regular, Menlo, Consolas, monospace';
    ctx.textBaseline = "middle";
    for (let t = firstTick; t <= endT + stepSec; t += stepSec) {
      if (t < 0) continue;
      const x = Math.round(t * pxPerSec - scrollX) + 0.5;
      ctx.beginPath();
      ctx.moveTo(x, RULER_H - 8);
      ctx.lineTo(x, RULER_H - 1);
      ctx.stroke();
      const label =
        stepSec < 1
          ? `${t.toFixed(1)}s`
          : `${Math.floor(t / 60)}:${Math.floor(t % 60)
              .toString()
              .padStart(2, "0")}`;
      ctx.fillText(label, x + 4, 22);
      /* sub ticks */
      const subStep = stepSec / 4;
      for (let s = 1; s < 4; s++) {
        const sx = Math.round((t + s * subStep) * pxPerSec - scrollX) + 0.5;
        ctx.beginPath();
        ctx.moveTo(sx, RULER_H - 4);
        ctx.lineTo(sx, RULER_H - 1);
        ctx.stroke();
      }
    }

    /* horizontal grid: octaves */
    const noteH = layout.notesH / padded.range;
    const notesBottom = layout.notesTop + layout.notesH;
    ctx.strokeStyle = octaveLine;
    ctx.lineWidth = 1;
    for (let m = Math.ceil(padded.lo / 12) * 12; m <= padded.hi && layout.notesH > 0; m += 12) {
      const y = notesBottom - (m - padded.lo) * noteH + 0.5;
      ctx.beginPath();
      ctx.moveTo(0, y);
      ctx.lineTo(w, y);
      ctx.stroke();
    }

    /* vertical grid: bars + beats from tempo map (handles tempo changes) */
    const visibleStartBeat = timeToBeat(tempoMap, Math.max(0, startT));
    const visibleEndBeat = timeToBeat(
      tempoMap,
      Math.min(song.duration, endT)
    );
    // Use average pxPerBeat across visible range to decide stride.
    const avgPxPerBeat =
      visibleEndBeat > visibleStartBeat
        ? w / (visibleEndBeat - visibleStartBeat)
        : 24;
    let beatStride = 1;
    if (avgPxPerBeat < 6) beatStride = 16;
    else if (avgPxPerBeat < 12) beatStride = 8;
    else if (avgPxPerBeat < 24) beatStride = 4;
    else if (avgPxPerBeat < 48) beatStride = 2;

    const firstBeatI =
      Math.floor(visibleStartBeat / beatStride) * beatStride;
    const lastBeatI = Math.ceil(visibleEndBeat) + beatStride;
    // Bar boundaries inside visible range — for highlight.
    const barBeats = new Set<number>();
    for (const bar of barMap.bars) {
      if (bar.beat >= firstBeatI - 0.5 && bar.beat <= lastBeatI + 0.5) {
        barBeats.add(Math.round(bar.beat));
      }
    }
    for (let b = firstBeatI; b <= lastBeatI; b += beatStride) {
      const t = beatToTime(tempoMap, b);
      const x = Math.round(t * pxPerSec - scrollX) + 0.5;
      const isBar = barBeats.has(Math.round(b));
      ctx.strokeStyle = isBar ? barLine : beatLine;
      ctx.beginPath();
      ctx.moveTo(x, RULER_H);
      ctx.lineTo(x, h);
      ctx.stroke();
    }

    /* tempo change markers — vertical dotted line + small label in ruler */
    if (tempoMap.segments.length > 1) {
      ctx.save();
      ctx.setLineDash([2, 3]);
      ctx.strokeStyle = dark
        ? "rgba(245,166,35,0.45)"
        : "rgba(216,140,16,0.55)";
      ctx.fillStyle = dark ? "#f5a623" : "#b8770a";
      ctx.font =
        '10px ui-monospace, SFMono-Regular, Menlo, Consolas, monospace';
      ctx.textBaseline = "top";
      for (let i = 1; i < tempoMap.segments.length; i++) {
        const seg = tempoMap.segments[i];
        if (seg.startTime < startT || seg.startTime > endT) continue;
        const x = Math.round(seg.startTime * pxPerSec - scrollX) + 0.5;
        ctx.beginPath();
        ctx.moveTo(x, RULER_H);
        ctx.lineTo(x, h);
        ctx.stroke();
        const label = `♩=${Math.round(seg.bpm)}`;
        ctx.fillText(label, x + 3, 11);
      }
      ctx.restore();
    }

    /* key-signature change markers — small vertical line + key label */
    if (song.keySignatures.length > 1) {
      ctx.save();
      ctx.strokeStyle = dark
        ? "rgba(100,210,255,0.45)"
        : "rgba(0,122,170,0.5)";
      ctx.fillStyle = dark ? "#64d2ff" : "#006e98";
      ctx.font =
        '10px ui-monospace, SFMono-Regular, Menlo, Consolas, monospace';
      ctx.textBaseline = "top";
      for (let i = 1; i < song.keySignatures.length; i++) {
        const k = song.keySignatures[i];
        if (k.time < startT || k.time > endT) continue;
        const x = Math.round(k.time * pxPerSec - scrollX) + 0.5;
        ctx.beginPath();
        ctx.moveTo(x, RULER_H - 14);
        ctx.lineTo(x, RULER_H);
        ctx.stroke();
        ctx.fillText(k.key, x + 3, 22);
      }
      ctx.restore();
    }

    /* time-signature change markers */
    if (song.timeSignatures.length > 1) {
      ctx.save();
      ctx.strokeStyle = dark
        ? "rgba(255,159,10,0.5)"
        : "rgba(200,100,0,0.6)";
      ctx.fillStyle = dark ? "#ff9f0a" : "#a55a00";
      ctx.font =
        'bold 10px ui-monospace, SFMono-Regular, Menlo, Consolas, monospace';
      ctx.textBaseline = "top";
      for (let i = 1; i < song.timeSignatures.length; i++) {
        const ts = song.timeSignatures[i];
        if (ts.time < startT || ts.time > endT) continue;
        const x = Math.round(ts.time * pxPerSec - scrollX) + 0.5;
        ctx.beginPath();
        ctx.moveTo(x, RULER_H - 14);
        ctx.lineTo(x, RULER_H);
        ctx.stroke();
        ctx.fillText(`${ts.numerator}/${ts.denominator}`, x + 3, 11);
      }
      ctx.restore();
    }

    /* markers (rehearsal marks, cue points) — flags on the ruler; lyrics
     * have a lane of their own */
    if (song.markers.length > 0) {
      ctx.save();
      ctx.font =
        '10px -apple-system, BlinkMacSystemFont, "SF Pro Text", system-ui, sans-serif';
      ctx.textBaseline = "top";
      for (const m of song.markers) {
        if (m.kind === "lyrics") continue;
        if (m.time < startT - 1 || m.time > endT + 1) continue;
        const x = Math.round(m.time * pxPerSec - scrollX) + 0.5;
        const color =
          m.kind === "cuePoint"
            ? dark
              ? "#30d158"
              : "#1a7d3a"
            : dark
            ? "#ff9f0a"
            : "#a55a00";
        ctx.fillStyle = color;
        ctx.strokeStyle = color;
        ctx.lineWidth = 1;
        // flag shape
        ctx.beginPath();
        ctx.moveTo(x, 0);
        ctx.lineTo(x + 7, 0);
        ctx.lineTo(x + 5, 4);
        ctx.lineTo(x + 7, 8);
        ctx.lineTo(x, 8);
        ctx.closePath();
        ctx.fill();
        // line down through ruler
        ctx.beginPath();
        ctx.moveTo(x, 8);
        ctx.lineTo(x, RULER_H);
        ctx.stroke();
        // label clipped
        const label = m.text.length > 18 ? m.text.slice(0, 17) + "…" : m.text;
        ctx.fillStyle = dark ? "#e8eaed" : "#1a1a1a";
        ctx.fillText(label, x + 9, 0);
      }
      ctx.restore();
    }

    const lane = { startT, endT, scrollX, pxPerSec, w, dark, position };
    if (showChords) {
      drawChordLane(ctx, lane, layout.chordsTop, chordList, {
        estimated: chordsEstimated,
        pending: chordsPending,
        tag: t.roll.estimated,
        pendingText: t.roll.estimating,
        duration: song.duration,
      });
    }
    if (showLyrics) drawLyricLane(ctx, lane, layout.lyricsTop, lyrics, song.duration);

    /* drum rows: one per sound, named, under the keyboard */
    const drumRow = new Map(layout.drumKeys.map((k, i) => [k, i]));
    if (layout.drumKeys.length > 0) {
      for (let i = 0; i < layout.drumKeys.length; i++) {
        if (i % 2 === 1) continue;
        ctx.fillStyle = dark ? "rgba(255,255,255,0.025)" : "rgba(0,0,0,0.03)";
        ctx.fillRect(0, layout.drumTop + i * layout.drumRowH, w, layout.drumRowH);
      }
      ctx.strokeStyle = dark ? "rgba(255,255,255,0.12)" : "rgba(0,0,0,0.14)";
      ctx.beginPath();
      ctx.moveTo(0, layout.drumTop + 0.5);
      ctx.lineTo(w, layout.drumTop + 0.5);
      ctx.stroke();
    }

    /* notes — visible only */
    const playT = position;
    const rectH = Math.max(2, noteH - 0.5);
    for (const note of song.notes) {
      if (note.time + note.duration < startT) continue;
      if (note.time > endT) continue;
      const track = song.tracks[note.trackIndex];
      const isMuted = muted[note.trackIndex];
      const x = note.time * pxPerSec - scrollX;
      const isPast = note.time < playT;
      const color = isMuted ? mutedColor : track.color;

      if (track.isPercussion) {
        // A hit has no length worth drawing: a diamond where it lands.
        const row = drumRow.get(note.midi);
        if (row === undefined) continue;
        const isActive = !isMuted && note.time <= playT && playT - note.time < 0.12;
        const cy = layout.drumTop + (row + 0.5) * layout.drumRowH;
        const r = Math.max(2.5, Math.min(layout.drumRowH * 0.42, 5.5));
        ctx.globalAlpha = isMuted ? 0.28 : isActive ? 1 : isPast ? 0.55 + note.velocity * 0.45 : 0.32 + note.velocity * 0.4;
        ctx.fillStyle = color;
        ctx.beginPath();
        ctx.moveTo(x, cy - r);
        ctx.lineTo(x + r, cy);
        ctx.lineTo(x, cy + r);
        ctx.lineTo(x - r, cy);
        ctx.closePath();
        ctx.fill();
        if (isActive) {
          ctx.globalAlpha = 0.6;
          ctx.strokeStyle = "#ffffff";
          ctx.lineWidth = 1;
          ctx.stroke();
        }
        continue;
      }
      if (layout.notesH === 0) continue;

      const noteW = Math.max(2, note.duration * pxPerSec - 0.5);
      const y = notesBottom - (note.midi - padded.lo + 1) * noteH;
      const isActive =
        !isMuted && note.time <= playT && note.time + note.duration > playT;
      let alpha: number;
      if (isMuted) alpha = 0.28;
      else if (isActive) alpha = 1;
      else if (isPast) alpha = 0.55 + note.velocity * 0.45;
      else alpha = 0.32 + note.velocity * 0.28;
      ctx.fillStyle = color;

      // Bent notes follow the bend; a faint outline keeps the written pitch.
      const bends = lanes.bends ? song.channels[track.channel]?.bend : undefined;
      if (bends && bends.length > 0) {
        const end = note.time + note.duration;
        let i = lastAtOrBefore(bends, note.time);
        let v = i >= 0 ? bends[i].value : 0;
        if (Math.abs(v) > 0.01 || (i + 1 < bends.length && bends[i + 1].time < end)) {
          ctx.globalAlpha = alpha * 0.5;
          ctx.strokeStyle = color;
          ctx.lineWidth = 1;
          ctx.strokeRect(x + 0.5, y + 0.5, Math.max(1, noteW - 1), Math.max(1, rectH - 1));
          ctx.globalAlpha = alpha;
          let from = note.time;
          for (;;) {
            const next = i + 1 < bends.length && bends[i + 1].time < end ? bends[i + 1].time : end;
            const xa = from * pxPerSec - scrollX;
            const xb = next * pxPerSec - scrollX;
            ctx.fillRect(xa, y - v * noteH, Math.max(1, xb - xa + 0.5), rectH);
            if (next >= end) break;
            i++;
            from = next;
            v = bends[i].value;
          }
          if (isActive) {
            ctx.globalAlpha = 0.5;
            ctx.strokeStyle = "#ffffff";
            ctx.strokeRect(x, y, noteW, rectH);
          }
          continue;
        }
      }

      ctx.globalAlpha = alpha;
      const r = Math.min(2, rectH / 2);
      if (typeof ctx.roundRect === "function") {
        ctx.beginPath();
        ctx.roundRect(x, y, noteW, rectH, r);
        ctx.fill();
      } else {
        ctx.fillRect(x, y, noteW, rectH);
      }
      if (isActive) {
        ctx.globalAlpha = 0.5;
        ctx.strokeStyle = "#ffffff";
        ctx.lineWidth = 1;
        ctx.beginPath();
        ctx.rect(x, y, noteW, rectH);
        ctx.stroke();
      }
    }
    ctx.globalAlpha = 1;

    /* drum names, pinned to the left edge over the hits */
    if (layout.drumKeys.length > 0) {
      ctx.save();
      const size = Math.min(11, layout.drumRowH - 1);
      ctx.font = `${size}px -apple-system, BlinkMacSystemFont, "SF Pro Text", system-ui, sans-serif`;
      ctx.textBaseline = "middle";
      const names = layout.drumKeys.map((k) => drumName(k));
      const labelW = Math.min(150, Math.max(...names.map((n) => ctx.measureText(n).width)) + 12);
      ctx.globalAlpha = 0.82;
      ctx.fillStyle = rollBg;
      ctx.fillRect(0, layout.drumTop + 1, labelW, layout.drumKeys.length * layout.drumRowH - 1);
      ctx.globalAlpha = 1;
      ctx.fillStyle = rulerText;
      names.forEach((name, i) => {
        fitText(ctx, name, 6, layout.drumTop + (i + 0.5) * layout.drumRowH, labelW - 10);
      });
      ctx.restore();
    }

    /* Playhead is drawn by a DOM overlay (see effect below) so it can
     * animate smoothly at 60fps without re-running this whole render. */
  }, [
    position,
    viewportW,
    song,
    dark,
    scrollX,
    pxPerSec,
    muted,
    padded,
    layout,
    lanes.bends,
    showChords,
    showLyrics,
    chordList,
    chordsEstimated,
    chordsPending,
    lyrics,
    t,
  ]);

  /* Playhead overlay — updated via rAF reading live position. */
  useEffect(() => {
    const ph = playheadRef.current;
    if (!ph) return;
    let raf = 0;
    const tick = () => {
      const pos = getLivePosition();
      const playX = pos * pxPerSec - scrollX;
      const visible = playX >= -2 && playX <= viewportW + 2;
      ph.style.transform = `translateX(${playX}px)`;
      ph.style.opacity = visible ? "1" : "0";
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [pxPerSec, scrollX, viewportW, getLivePosition]);

  /* render overview canvas (cached, redraw only when notes/mute/zoom basics change) */
  useEffect(() => {
    const canvas = overviewRef.current;
    if (!canvas) return;
    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    const w = overviewW;
    const h = OVERVIEW_H;
    if (canvas.width !== w * dpr || canvas.height !== h * dpr) {
      canvas.width = w * dpr;
      canvas.height = h * dpr;
    }
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, h);

    const pxPerSecMini = w / Math.max(0.1, song.duration);
    // Drums along the bottom edge, pitched notes above.
    const drumBand = layout.drumKeys.length > 0 ? 6 : 0;
    const noteH = (h - drumBand) / padded.range;

    for (const note of song.notes) {
      const isMuted = muted[note.trackIndex];
      const x = note.time * pxPerSecMini;
      if (isDrum[note.trackIndex]) {
        ctx.globalAlpha = isMuted ? 0.16 : 0.45;
        ctx.fillStyle = isMuted ? (dark ? "#3a3c40" : "#bdc0c5") : song.tracks[note.trackIndex].color;
        ctx.fillRect(x, h - drumBand + 1, 1, drumBand - 1);
        continue;
      }
      const noteW = Math.max(1, note.duration * pxPerSecMini);
      const y = h - drumBand - (note.midi - padded.lo + 1) * noteH;
      const rectH = Math.max(1, noteH);
      ctx.globalAlpha = isMuted ? 0.16 : 0.6;
      ctx.fillStyle = isMuted
        ? dark
          ? "#3a3c40"
          : "#bdc0c5"
        : song.tracks[note.trackIndex].color;
      ctx.fillRect(x, y, noteW, rectH);
    }
    ctx.globalAlpha = 1;
  }, [overviewW, song, muted, padded, dark, isDrum, layout.drumKeys.length]);

  /* viewport window & playhead on overview */
  const overviewWindow = useMemo(() => {
    const total = Math.max(0.001, song.duration);
    const left = (scrollX / pxPerSec / total) * overviewW;
    const width = (viewportW / pxPerSec / total) * overviewW;
    return { left, width: Math.min(overviewW, Math.max(2, width)) };
  }, [scrollX, pxPerSec, viewportW, overviewW, song.duration]);

  const overviewPlayheadX = useMemo(
    () => (position / Math.max(0.001, song.duration)) * overviewW,
    [position, song.duration, overviewW]
  );

  /* interaction helpers */
  const seekFromMain = useCallback(
    (clientX: number) => {
      const el = mainRef.current;
      if (!el) return;
      const rect = el.getBoundingClientRect();
      const x = clientX - rect.left;
      const t = (scrollX + x) / pxPerSec;
      onSeek(Math.max(0, Math.min(1, t / song.duration)));
    },
    [scrollX, pxPerSec, onSeek, song.duration]
  );

  /**
   * Where a pointer is, in beats and keys: a key row in the pitched area or
   * a drum row below it. `pane` keeps a drag in the pane it started in.
   */
  const pointAt = useCallback(
    (clientX: number, clientY: number, pane?: Pane) => {
      const el = mainRef.current;
      if (!el) return null;
      const rect = el.getBoundingClientRect();
      const x = clientX - rect.left;
      const y = clientY - rect.top;
      const t = Math.max(0, Math.min(song.duration, (scrollX + x) / pxPerSec));
      const beat = timeToBeat(tempoMap, t);
      const inDrums = layout.drumKeys.length > 0 && (pane ? pane === "drums" : y >= layout.drumTop);
      if (inDrums) {
        const row = Math.floor((y - layout.drumTop) / layout.drumRowH);
        const midi = layout.drumKeys[Math.max(0, Math.min(layout.drumKeys.length - 1, row))];
        return { y, beat, midi, pane: "drums" as Pane, inNotes: y >= layout.drumTop };
      }
      const noteH = layout.notesH / padded.range;
      const row = noteH > 0 ? Math.floor((layout.notesTop + layout.notesH - y) / noteH) : 0;
      const midi = Math.max(0, Math.min(127, padded.lo + Math.max(0, Math.min(padded.range - 1, row))));
      return { y, beat, midi, pane: "keys" as Pane, inNotes: y >= layout.notesTop && layout.notesH > 0 };
    },
    [scrollX, pxPerSec, song.duration, padded, tempoMap, layout]
  );

  /** Whether a note belongs to the pane a selection was drawn in. */
  const inPane = (trackIndex: number, pane: Pane) => isDrum[trackIndex] === (pane === "drums");

  const copySelection = () => {
    if (!sel || !onCopy) return;
    const t0 = beatToTime(tempoMap, sel.b0);
    const t1 = beatToTime(tempoMap, sel.b1);
    const used = new Map<number, number>();
    for (const n of song.notes) {
      if (n.time < t0 - 1e-6) continue;
      if (n.time >= t1 - 1e-6) break;
      if (n.midi < sel.lo || n.midi > sel.hi || !inPane(n.trackIndex, sel.pane)) continue;
      used.set(n.trackIndex, (used.get(n.trackIndex) ?? 0) + 1);
    }
    if (used.size === 0) return;
    onCopy({
      startBeat: sel.b0,
      endBeat: sel.b1,
      start: beatPosition(barMap.bars, sel.b0),
      end: beatPosition(barMap.bars, sel.b1 - 1),
      lo: sel.lo,
      hi: sel.hi,
      tracks: song.tracks
        .filter((tr) => used.has(tr.index))
        .map((tr) => ({ index: tr.index, name: tr.name || tr.instrument })),
      noteCount: [...used.values()].reduce((a, b) => a + b, 0),
    });
  };

  // ⌘C copies the selection.
  const copyRef = useRef(copySelection);
  copyRef.current = copySelection;
  useEffect(() => {
    if (!sel || !onCopy) return;
    const onKey = (e: KeyboardEvent) => {
      if (!(e.metaKey || e.ctrlKey) || e.key.toLowerCase() !== "c") return;
      const el = e.target as HTMLElement | null;
      if (el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.isContentEditable)) return;
      if (window.getSelection()?.toString()) return;
      e.preventDefault();
      copyRef.current();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [sel, onCopy]);

  const attachSelection = () => {
    if (!sel || !onAttach) return;
    const t0 = beatToTime(tempoMap, sel.b0);
    const t1 = beatToTime(tempoMap, sel.b1);
    const notes: RollSelectionDraft["notes"] = [];
    const used = new Set<number>();
    // Notes that start inside the range; song.notes is in time order.
    for (const n of song.notes) {
      if (n.time < t0 - 1e-6) continue;
      if (n.time >= t1 - 1e-6) break;
      if (n.midi < sel.lo || n.midi > sel.hi || !inPane(n.trackIndex, sel.pane)) continue;
      const nb = timeToBeat(tempoMap, n.time);
      notes.push({
        track: n.trackIndex,
        midi: n.midi,
        at: beatPosition(barMap.bars, nb),
        beats: timeToBeat(tempoMap, n.time + n.duration) - nb,
      });
      used.add(n.trackIndex);
    }
    notes.sort((a, b) => a.track - b.track);
    onAttach({
      start: beatPosition(barMap.bars, sel.b0),
      end: beatPosition(barMap.bars, sel.b1 - 1),
      lo: sel.lo,
      hi: sel.hi,
      tracks: song.tracks
        .filter((t) => used.has(t.index))
        .map((t) => ({ index: t.index, name: t.name || t.instrument })),
      notes,
    });
    setSel(null);
  };

  const selBox = useMemo(() => {
    if (!sel) return null;
    const x0 = beatToTime(tempoMap, sel.b0) * pxPerSec - scrollX;
    const x1 = beatToTime(tempoMap, sel.b1) * pxPerSec - scrollX;
    const box = { left: x0, width: Math.max(2, x1 - x0) };
    if (sel.pane === "drums") {
      // Rows run from the highest key down.
      const top = layout.drumKeys.indexOf(sel.hi);
      const bottom = layout.drumKeys.indexOf(sel.lo);
      return {
        ...box,
        top: layout.drumTop + top * layout.drumRowH,
        height: (bottom - top + 1) * layout.drumRowH,
      };
    }
    const noteH = layout.notesH / padded.range;
    return {
      ...box,
      top: layout.notesTop + layout.notesH - (sel.hi - padded.lo + 1) * noteH,
      height: (sel.hi - sel.lo + 1) * noteH,
    };
  }, [sel, tempoMap, pxPerSec, scrollX, padded, layout]);

  const draggingRef = useRef<null | "main" | "select" | "ov-scrub" | "ov-pan">(null);
  const panOriginRef = useRef({ x: 0, scroll: 0 });

  useEffect(() => {
    const move = (e: PointerEvent) => {
      const mode = draggingRef.current;
      if (!mode) return;
      if (mode === "main") {
        seekFromMain(e.clientX);
      } else if (mode === "select") {
        const d = selDragRef.current;
        const p = d && pointAt(e.clientX, e.clientY, d.pane);
        if (!d || !p) return;
        if (!d.moved && Math.hypot(e.clientX - d.x, e.clientY - d.y) < 4) return;
        d.moved = true;
        const total = Math.max(1, Math.ceil(tempoMap.totalBeats - 1e-6));
        const b0 = Math.max(0, Math.min(total - 1, Math.floor(Math.min(d.beat, p.beat))));
        const b1 = Math.max(b0 + 1, Math.min(total, Math.ceil(Math.max(d.beat, p.beat))));
        setSel({ b0, b1, lo: Math.min(d.midi, p.midi), hi: Math.max(d.midi, p.midi), pane: d.pane });
      } else if (mode === "ov-scrub") {
        const el = overviewWrapRef.current;
        if (!el) return;
        const rect = el.getBoundingClientRect();
        const ratio = (e.clientX - rect.left) / rect.width;
        const t = Math.max(0, Math.min(1, ratio)) * song.duration;
        const next = Math.max(
          0,
          Math.min(maxScroll, t * pxPerSec - viewportW / 2)
        );
        setScrollX(next);
        setFollow(false);
      } else if (mode === "ov-pan") {
        const el = overviewWrapRef.current;
        if (!el) return;
        const rect = el.getBoundingClientRect();
        const dx = e.clientX - panOriginRef.current.x;
        const ratio = dx / rect.width;
        const next = Math.max(
          0,
          Math.min(maxScroll, panOriginRef.current.scroll + ratio * contentW)
        );
        setScrollX(next);
        setFollow(false);
      }
    };
    const up = () => {
      // A press in the notes area that never moved is a click: seek there.
      const d = selDragRef.current;
      if (draggingRef.current === "select" && d && !d.moved) {
        seekFromMain(d.x);
        setSel(null);
      }
      selDragRef.current = null;
      draggingRef.current = null;
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    window.addEventListener("pointercancel", up);
    return () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("pointercancel", up);
    };
  }, [seekFromMain, pointAt, tempoMap, maxScroll, pxPerSec, viewportW, contentW, song.duration]);

  /* zoom controls */
  const zoomBy = (factor: number) => {
    setPxPerSec((p) =>
      Math.max(MIN_PX_PER_SEC, Math.min(MAX_PX_PER_SEC, p * factor))
    );
    setFollow(false);
  };
  const fitAll = () => {
    setPxPerSec(fitPxPerSec);
    setScrollX(0);
  };

  const zoomReadout = `${Math.round(pxPerSec)} px/s`;

  return (
    <div className="roll-shell">
      <div className="roll-toolbar">
        <span
          style={{
            fontSize: 10,
            fontWeight: 700,
            textTransform: "uppercase",
            letterSpacing: "0.1em",
            color: "var(--text-faint)",
            whiteSpace: "nowrap",
          }}
        >
          Piano Roll
        </span>
        {onAttach && (
          <span className="roll-hint">{t.roll.hint}</span>
        )}
        <div style={{ flex: 1 }} />
        <div className="roll-lane-toggles">
          {(song.chords.length > 0 || estimateChords) && (
            <button
              className={`toggle-chip${lanes.chords ? " on" : ""}`}
              onClick={() => toggleLane("chords")}
              title={t.roll.chordsTitle}
            >
              {t.roll.chords}
            </button>
          )}
          {lyrics.length > 0 && (
            <button
              className={`toggle-chip${lanes.lyrics ? " on" : ""}`}
              onClick={() => toggleLane("lyrics")}
              title={t.roll.lyricsTitle}
            >
              {t.roll.lyrics}
            </button>
          )}
          {hasBends && (
            <button
              className={`toggle-chip${lanes.bends ? " on" : ""}`}
              onClick={() => toggleLane("bends")}
              title={t.roll.bendsTitle}
            >
              {t.roll.bends}
            </button>
          )}
          <button
            className={`toggle-chip${lanes.controllers ? " on" : ""}`}
            onClick={() => toggleLane("controllers")}
            title={t.roll.lanesTitle}
          >
            {t.roll.lanes}
          </button>
        </div>
        <button
          className={`toggle-chip${follow ? " on" : ""}`}
          onClick={() => setFollow((f) => !f)}
          title={t.roll.follow}
        >
          Follow
        </button>
        <div style={{ display: "flex", alignItems: "center", gap: 2 }}>
          <button
            className="zoom-btn"
            onClick={() => zoomBy(1 / 1.5)}
            aria-label={t.roll.zoomOut}
          >
            −
          </button>
          <span className="zoom-readout">{zoomReadout}</span>
          <button
            className="zoom-btn"
            onClick={() => zoomBy(1.5)}
            aria-label={t.roll.zoomIn}
          >
            +
          </button>
        </div>
        <button className="toggle-chip" onClick={fitAll} title={t.roll.fitAll}>
          Fit
        </button>
      </div>

      <div ref={stageRef}>
        <div
          ref={mainRef}
          className="roll-main"
          style={{ height: layout.totalH }}
          onPointerDown={(e) => {
            (e.target as Element).setPointerCapture?.(e.pointerId);
            const p = onAttach || onCopy ? pointAt(e.clientX, e.clientY) : null;
            if (p && p.inNotes) {
              selDragRef.current = { beat: p.beat, midi: p.midi, pane: p.pane, x: e.clientX, y: e.clientY, moved: false };
              draggingRef.current = "select";
              return;
            }
            draggingRef.current = "main";
            seekFromMain(e.clientX);
          }}
        >
          <canvas ref={canvasRef} />
          <div ref={playheadRef} className="roll-playhead" style={{ height: layout.totalH - PLAYHEAD_TOP }} />
          {selBox && (
            <div className="roll-selection" style={selBox}>
              <div className="roll-selection-actions" onPointerDown={(e) => e.stopPropagation()}>
                {onCopy && (
                  <button className="roll-selection-btn is-quiet" onClick={copySelection} title={t.roll.copyTitle}>
                    {t.roll.copy}
                  </button>
                )}
                {onAttach && (
                  <button className="roll-selection-btn" onClick={attachSelection} title={t.roll.attachTitle}>
                    {t.roll.attach}
                  </button>
                )}
              </div>
            </div>
          )}
        </div>

        {lanes.controllers && (
          <ControllerLane
            song={song}
            muted={muted}
            scrollX={scrollX}
            pxPerSec={pxPerSec}
            viewportW={viewportW}
            dark={dark}
            barTimes={barTimes}
            getLivePosition={getLivePosition}
            onSeekTime={(t) => onSeek(t / Math.max(0.001, song.duration))}
          />
        )}
      </div>

      <div
        ref={overviewWrapRef}
        className="roll-overview"
        onPointerDown={(e) => {
          const rect = (
            e.currentTarget as HTMLDivElement
          ).getBoundingClientRect();
          const xRel = e.clientX - rect.left;
          const inWindow =
            xRel >= overviewWindow.left &&
            xRel <= overviewWindow.left + overviewWindow.width;
          (e.target as Element).setPointerCapture?.(e.pointerId);
          if (inWindow) {
            draggingRef.current = "ov-pan";
            panOriginRef.current = { x: e.clientX, scroll: scrollX };
          } else {
            draggingRef.current = "ov-scrub";
            const ratio = xRel / rect.width;
            const t = Math.max(0, Math.min(1, ratio)) * song.duration;
            const next = Math.max(
              0,
              Math.min(maxScroll, t * pxPerSec - viewportW / 2)
            );
            setScrollX(next);
            setFollow(false);
          }
        }}
      >
        <canvas ref={overviewRef} />
        <div
          className="viewport-window"
          style={{
            left: `${overviewWindow.left}px`,
            width: `${overviewWindow.width}px`,
          }}
        />
        <div
          style={{
            position: "absolute",
            top: 0,
            bottom: 0,
            left: `${overviewPlayheadX}px`,
            width: 1,
            background: "#f5a623",
            opacity: 0.9,
            pointerEvents: "none",
          }}
        />
      </div>
    </div>
  );
}

/* ---------- view switcher ---------- */

type PrimaryView = "roll" | "score" | "events";

function ViewSwitcher({
  primary,
  onPrimaryChange,
  showKeys,
  onToggleKeys,
  song,
  position,
  isPlaying,
  muted,
  onSeek,
  getLivePosition,
  onAttach,
  onCopy,
  estimateChords,
  buffer,
}: {
  primary: PrimaryView;
  onPrimaryChange: (v: PrimaryView) => void;
  showKeys: boolean;
  onToggleKeys: () => void;
  song: SongInfo;
  position: number;
  isPlaying: boolean;
  muted: boolean[];
  onSeek: (ratio: number) => void;
  getLivePosition: () => number;
  onAttach?: (sel: RollSelectionDraft) => void;
  onCopy?: (clip: RollClipDraft) => void;
  estimateChords?: () => Promise<ChordAt[]>;
  /** The file on screen, for the event list. */
  buffer: ArrayBuffer | null;
}) {
  const { t } = useI18n();
  return (
    <div className="view-shell">
      <div className="view-tabs">
        <button
          className={`view-tab${primary === "roll" ? " active" : ""}`}
          onClick={() => onPrimaryChange("roll")}
        >
          Piano Roll
        </button>
        <button
          className={`view-tab${primary === "score" ? " active" : ""}`}
          onClick={() => onPrimaryChange("score")}
        >
          Score
        </button>
        {buffer && (
          <button
            className={`view-tab${primary === "events" ? " active" : ""}`}
            onClick={() => onPrimaryChange("events")}
          >
            Events
          </button>
        )}
        <div style={{ flex: 1 }} />
        <button
          className={`view-toggle${showKeys ? " on" : ""}`}
          onClick={onToggleKeys}
          title={showKeys ? t.roll.hideKeys : t.roll.showKeys}
        >
          {showKeys ? "▾" : "▸"} Keyboard
        </button>
      </div>
      {primary === "roll" && (
        <PianoRoll
          song={song}
          position={position}
          isPlaying={isPlaying}
          muted={muted}
          onSeek={onSeek}
          getLivePosition={getLivePosition}
          onAttach={onAttach}
          onCopy={onCopy}
          estimateChords={estimateChords}
        />
      )}
      {primary === "events" && buffer && (
        <EventList
          song={song}
          buffer={buffer}
          position={position}
          isPlaying={isPlaying}
          onSeekTime={(t) => onSeek(t / Math.max(0.001, song.duration))}
        />
      )}
      {primary === "score" && (
        <ScoreView
          song={song}
          muted={muted}
          onSeek={onSeek}
          getLivePosition={getLivePosition}
        />
      )}
      {showKeys && (
        <KeyboardView
          song={song}
          muted={muted}
          onSeek={onSeek}
          getLivePosition={getLivePosition}
        />
      )}
    </div>
  );
}

/* ---------- keyboard view (Synthesia-style) ---------- */

const KEYBOARD_H = 110;
const FALL_H = 320;
const FALL_LOOKAHEAD = 4.5;
/** Onset flash duration in seconds; longer = more visible re-attacks. */
const ATTACK_FLASH = 0.13;

function isBlackKey(midi: number): boolean {
  const n = midi % 12;
  return n === 1 || n === 3 || n === 6 || n === 8 || n === 10;
}

/** First index `i` such that `notes[i].time >= t`. */
function lowerBoundByTime(
  notes: { time: number }[],
  t: number
): number {
  let lo = 0;
  let hi = notes.length;
  while (lo < hi) {
    const mid = (lo + hi) >>> 1;
    if (notes[mid].time < t) lo = mid + 1;
    else hi = mid;
  }
  return lo;
}

/** First index `i` such that `notes[i].time > t`. */
function upperBoundByTime(
  notes: { time: number }[],
  t: number
): number {
  let lo = 0;
  let hi = notes.length;
  while (lo < hi) {
    const mid = (lo + hi) >>> 1;
    if (notes[mid].time <= t) lo = mid + 1;
    else hi = mid;
  }
  return lo;
}

function KeyboardView({
  song,
  muted,
  onSeek,
  getLivePosition,
}: {
  song: SongInfo;
  muted: boolean[];
  onSeek: (ratio: number) => void;
  getLivePosition: () => number;
}) {
  const wrapRef = useRef<HTMLDivElement | null>(null);
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const mutedRef = useRef(muted);
  mutedRef.current = muted;
  const [width, setWidth] = useState(800);
  const [dark, setDark] = useState(true);

  useEffect(() => {
    const el = wrapRef.current;
    if (!el) return;
    const update = () => setWidth(el.clientWidth || 800);
    update();
    const obs = new ResizeObserver(update);
    obs.observe(el);
    return () => obs.disconnect();
  }, []);

  useEffect(() => {
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    setDark(mq.matches);
    const handler = (e: MediaQueryListEvent) => setDark(e.matches);
    mq.addEventListener("change", handler);
    return () => mq.removeEventListener("change", handler);
  }, []);

  const range = useMemo(() => {
    const lo = Math.max(
      21,
      Math.min(108, Math.floor(song.minMidi / 12) * 12)
    );
    const hi = Math.min(
      108,
      Math.max(lo + 24, Math.ceil((song.maxMidi + 1) / 12) * 12 - 1)
    );
    let whiteCount = 0;
    const whiteIndex: number[] = new Array(128).fill(-1);
    for (let m = lo; m <= hi; m++) {
      if (!isBlackKey(m)) {
        whiteIndex[m] = whiteCount;
        whiteCount++;
      }
    }
    return { lo, hi, whiteCount, whiteIndex };
  }, [song]);

  const xForKey = useCallback(
    (midi: number) => {
      const w = width;
      const whiteW = w / range.whiteCount;
      if (!isBlackKey(midi)) {
        return range.whiteIndex[midi] * whiteW;
      }
      const prevWhite = range.whiteIndex[midi - 1];
      return (prevWhite + 1) * whiteW - whiteW * 0.32;
    },
    [width, range]
  );

  const keyWidth = useCallback(
    (midi: number) => {
      const w = width;
      const whiteW = w / range.whiteCount;
      return isBlackKey(midi) ? whiteW * 0.64 : whiteW;
    },
    [width, range]
  );

  /* requestAnimationFrame loop — reads live position each frame for smooth
   * 60fps animation independent of the engine's UI-tick rate. */
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    const w = width;
    const totalH = FALL_H + KEYBOARD_H;
    if (canvas.width !== w * dpr || canvas.height !== totalH * dpr) {
      canvas.width = w * dpr;
      canvas.height = totalH * dpr;
    }
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    const notes = song.notes;
    const tracks = song.tracks;
    const maxDur = Math.max(0.5, song.maxNoteDuration);

    const bg = dark ? "#0a0b0d" : "#f4f5f7";
    const cline = dark ? "rgba(255,255,255,0.05)" : "rgba(0,0,0,0.05)";
    const nowLine = dark ? "rgba(245,166,35,0.55)" : "rgba(216,140,16,0.6)";
    const kbBg = dark ? "#1a1c20" : "#cdd0d4";
    const whiteKey = dark ? "#f0f0f3" : "#ffffff";
    const whiteEdge = dark ? "#0a0b0d" : "#7a7d83";
    const blackKey = dark ? "#15171a" : "#1a1c20";
    const cLabelDim = dark ? "#6d7077" : "#5d6168";

    let raf = 0;

    const draw = () => {
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);

      ctx.fillStyle = bg;
      ctx.fillRect(0, 0, w, totalH);

      ctx.strokeStyle = cline;
      ctx.lineWidth = 1;
      for (let m = range.lo; m <= range.hi; m++) {
        if (m % 12 === 0) {
          const x = xForKey(m);
          ctx.beginPath();
          ctx.moveTo(x + 0.5, 0);
          ctx.lineTo(x + 0.5, FALL_H);
          ctx.stroke();
        }
      }

      ctx.strokeStyle = nowLine;
      ctx.lineWidth = 1.5;
      ctx.beginPath();
      ctx.moveTo(0, FALL_H);
      ctx.lineTo(w, FALL_H);
      ctx.stroke();

      const pos = getLivePosition();
      const startT = pos;
      const endT = pos + FALL_LOOKAHEAD;
      const activeKeys = new Map<number, string>();
      /** midi → max flash alpha (0..1) from the most recent onset within
       *  ATTACK_FLASH seconds. */
      const attackFlash = new Map<number, number>();
      const currentMuted = mutedRef.current;

      // Window into the sorted notes array: notes that could possibly
      // overlap [startT, endT] have time in [startT - maxDur, endT].
      const lo = lowerBoundByTime(notes, startT - maxDur);
      const hi = upperBoundByTime(notes, endT);

      for (let i = lo; i < hi; i++) {
        const note = notes[i];
        if (currentMuted[note.trackIndex]) continue;
        const noteEnd = note.time + note.duration;
        if (noteEnd < startT) continue;
        const color = tracks[note.trackIndex].color;
        const yEnd = FALL_H - ((noteEnd - startT) / FALL_LOOKAHEAD) * FALL_H;
        const yStart = FALL_H - ((note.time - startT) / FALL_LOOKAHEAD) * FALL_H;
        const drawY = Math.max(0, yEnd);
        const drawH = Math.max(2, Math.min(FALL_H, yStart) - drawY);
        const x = xForKey(note.midi);
        const kw = keyWidth(note.midi);
        const padX = kw * 0.08;
        ctx.fillStyle = color;
        ctx.globalAlpha = 0.55 + note.velocity * 0.45;
        const r = Math.min(4, kw * 0.18);
        if (typeof ctx.roundRect === "function") {
          ctx.beginPath();
          ctx.roundRect(x + padX, drawY, kw - 2 * padX, drawH, r);
          ctx.fill();
        } else {
          ctx.fillRect(x + padX, drawY, kw - 2 * padX, drawH);
        }
        if (note.time <= startT && noteEnd > startT) {
          activeKeys.set(note.midi, color);
          ctx.globalAlpha = 0.45;
          ctx.fillRect(x, FALL_H - 6, kw, 6);
        }
        /* Onset flash: any note that started within the last ATTACK_FLASH
         * seconds gets a fading bright overlay on its key. Re-attacks at
         * the same pitch refresh the flash. */
        if (note.time <= startT && note.time >= startT - ATTACK_FLASH) {
          const age = startT - note.time;
          const alpha =
            Math.max(0, 1 - age / ATTACK_FLASH) * (0.6 + note.velocity * 0.4);
          const prev = attackFlash.get(note.midi) ?? 0;
          if (alpha > prev) attackFlash.set(note.midi, alpha);
        }
      }
      ctx.globalAlpha = 1;

      const kbY = FALL_H;
      ctx.fillStyle = kbBg;
      ctx.fillRect(0, kbY, w, KEYBOARD_H);

      for (let m = range.lo; m <= range.hi; m++) {
        if (isBlackKey(m)) continue;
        const x = xForKey(m);
        const kw = keyWidth(m);
        const active = activeKeys.get(m);
        ctx.fillStyle = active ? active : whiteKey;
        ctx.fillRect(x + 0.5, kbY, kw - 1, KEYBOARD_H);
        const flash = attackFlash.get(m);
        if (flash) {
          ctx.globalAlpha = flash;
          ctx.fillStyle = "#ffffff";
          ctx.fillRect(x + 0.5, kbY, kw - 1, KEYBOARD_H);
          ctx.globalAlpha = 1;
        }
        ctx.strokeStyle = whiteEdge;
        ctx.lineWidth = 1;
        ctx.strokeRect(x + 0.5, kbY + 0.5, kw - 1, KEYBOARD_H - 1);
        if (m % 12 === 0) {
          ctx.fillStyle = active ? "#1a1306" : cLabelDim;
          ctx.font =
            '9px ui-monospace, SFMono-Regular, Menlo, Consolas, monospace';
          ctx.textBaseline = "alphabetic";
          ctx.fillText(`C${m / 12 - 1}`, x + 3, kbY + KEYBOARD_H - 4);
        }
      }
      for (let m = range.lo; m <= range.hi; m++) {
        if (!isBlackKey(m)) continue;
        const x = xForKey(m);
        const kw = keyWidth(m);
        const active = activeKeys.get(m);
        ctx.fillStyle = active ? active : blackKey;
        const bkH = KEYBOARD_H * 0.62;
        ctx.fillRect(x, kbY, kw, bkH);
        const flash = attackFlash.get(m);
        if (flash) {
          ctx.globalAlpha = flash;
          ctx.fillStyle = "#ffffff";
          ctx.fillRect(x, kbY, kw, bkH);
          ctx.globalAlpha = 1;
        }
        if (active) {
          ctx.strokeStyle = "rgba(255,255,255,0.3)";
          ctx.lineWidth = 1;
          ctx.strokeRect(x + 0.5, kbY + 0.5, kw - 1, bkH - 1);
        }
      }

      raf = requestAnimationFrame(draw);
    };
    raf = requestAnimationFrame(draw);
    return () => cancelAnimationFrame(raf);
  }, [width, song, dark, range, xForKey, keyWidth, getLivePosition]);

  const onPointerDown = useCallback(
    (e: React.PointerEvent) => {
      const el = wrapRef.current;
      if (!el) return;
      const rect = el.getBoundingClientRect();
      const y = e.clientY - rect.top;
      if (y > FALL_H) return;
      const dt = ((FALL_H - y) / FALL_H) * FALL_LOOKAHEAD;
      const target = Math.max(0, getLivePosition() + dt);
      onSeek(Math.min(1, target / song.duration));
    },
    [getLivePosition, song.duration, onSeek]
  );

  return (
    <div
      ref={wrapRef}
      className="keys-view"
      onPointerDown={onPointerDown}
      style={{ height: FALL_H + KEYBOARD_H }}
    >
      <canvas ref={canvasRef} />
    </div>
  );
}

/* ---------- score view (VexFlow) ---------- */

const SCORE_BARS_MIN = 1;
const SCORE_BARS_MAX = 16;
const SCORE_BARS_DEFAULT = 4;

function ScoreView({
  song,
  muted,
  onSeek: _onSeek,
  getLivePosition,
}: {
  song: SongInfo;
  muted: boolean[];
  onSeek: (ratio: number) => void;
  getLivePosition: () => number;
}) {
  const { t } = useI18n();
  void _onSeek;
  const wrapRef = useRef<HTMLDivElement | null>(null);
  const mutedRef = useRef(muted);
  mutedRef.current = muted;
  const [width, setWidth] = useState(800);
  const [vex, setVex] = useState<typeof import("vexflow") | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [currentBarIdx, setCurrentBarIdx] = useState(0);
  /** Tracks visible in the score view. `null` = all tracks. */
  const [scoreTracks, setScoreTracks] = useState<Set<number> | null>(null);
  const [barsPerPage, setBarsPerPage] = useState(SCORE_BARS_DEFAULT);
  /** Per-note x positions captured from VexFlow after each static render.
   * Used by the playhead rAF to interpolate position exactly along the
   * laid-out beats, regardless of clef/key-sig widths or tempo changes. */
  const noteAnchorsRef = useRef<{ beat: number; x: number }[]>([]);
  /** Playhead SVG element, re-created on every static render because VexFlow
   * clears the host SVG. Updated by the rAF effect via setAttribute. */
  const playheadSvgRef = useRef<SVGLineElement | null>(null);

  useEffect(() => {
    let cancelled = false;
    import("vexflow")
      .then((m) => {
        if (!cancelled) setVex(m);
      })
      .catch((e) => {
        if (!cancelled) setError(String(e));
      });
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    const el = wrapRef.current;
    if (!el) return;
    const update = () => setWidth(el.clientWidth || 800);
    update();
    const obs = new ResizeObserver(update);
    obs.observe(el);
    return () => obs.disconnect();
  }, []);

  const tempoMap = useMemo(
    () => buildTempoMap(song.tempos, song.duration),
    [song]
  );
  const barMap = useMemo(
    () => buildBarMap(song.timeSignatures, tempoMap),
    [song, tempoMap]
  );

  /* Track the current bar via rAF — only re-render the score body when
   * we cross into a new bar, NOT on every position tick. */
  useEffect(() => {
    let raf = 0;
    const tick = () => {
      const pos = getLivePosition();
      const curBeat = timeToBeat(tempoMap, Math.max(0, pos));
      let idx = 0;
      for (let i = 0; i < barMap.bars.length; i++) {
        if (barMap.bars[i].beat <= curBeat + 1e-6) idx = i;
        else break;
      }
      setCurrentBarIdx((prev) => (prev === idx ? prev : idx));
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [tempoMap, barMap, getLivePosition]);

  /* Snap the window to a "page" of `barsPerPage` bars so that the playhead
   * moves across the visible bars instead of every bar shifting at once. */
  const win = useMemo(() => {
    const totalBars = barMap.bars.length;
    const cur = Math.min(currentBarIdx, totalBars - 1);
    const pageStartIdx =
      Math.floor(cur / barsPerPage) * barsPerPage;
    const safeStart = Math.min(pageStartIdx, totalBars - 1);
    const startBar = barMap.bars[safeStart];
    const endBarIdx = Math.min(totalBars - 1, safeStart + barsPerPage);
    const startBeat = startBar.beat;
    const endBeat =
      endBarIdx < totalBars - 1
        ? barMap.bars[endBarIdx].beat
        : tempoMap.totalBeats;
    const startTime = beatToTime(tempoMap, startBeat);
    const endTime = beatToTime(tempoMap, endBeat);
    return {
      barIdx: safeStart,
      startBar,
      startBeat,
      endBeat,
      startTime,
      endTime,
    };
  }, [currentBarIdx, barsPerPage, tempoMap, barMap]);

  const currentKey = useMemo(() => {
    if (!song.keySignatures.length) return null;
    let cur = song.keySignatures[0];
    for (const k of song.keySignatures) {
      if (k.time <= win.startTime + 1e-6) cur = k;
      else break;
    }
    return cur;
  }, [song.keySignatures, win.startTime]);

  const currentTs = useMemo(() => {
    let cur = song.timeSignatures[0];
    for (const t of song.timeSignatures) {
      if (t.time <= win.startTime + 1e-6) cur = t;
      else break;
    }
    return cur;
  }, [song.timeSignatures, win.startTime]);

  useEffect(() => {
    const el = wrapRef.current;
    if (!el || !vex || !currentTs) return;
    el.innerHTML = "";

    try {
      const {
        Renderer,
        Stave,
        StaveNote,
        Voice,
        Formatter,
        Accidental,
        StaveConnector,
      } = vex;

      // Compute slot count up-front so we can size the SVG appropriately.
      const totalBeats0 = win.endBeat - win.startBeat;
      const slotsPerBeat = 4;
      const totalSlots = Math.max(1, Math.round(totalBeats0 * slotsPerBeat));

      const renderer = new Renderer(el, Renderer.Backends.SVG);
      const h = 320;
      // VexFlow has a minimum width per note (accidentals, ledger lines etc.)
      // For dense pages this exceeds the viewport; we render at a generous
      // natural width then scale the SVG via viewBox to fit the container.
      const minPxPerSlot = 22;
      const headerW = 130;
      const naturalWidth = Math.max(
        width,
        Math.round(headerW + totalSlots * minPxPerSlot + 40)
      );
      renderer.resize(naturalWidth, h);
      const ctx = renderer.getContext();
      ctx.setFillStyle("#14161a");
      ctx.setStrokeStyle("#14161a");

      const padding = 16;
      const staveWidth = naturalWidth - padding * 2;
      const trebleStave = new Stave(padding, 70, staveWidth);
      const bassStave = new Stave(padding, 180, staveWidth);
      trebleStave.addClef("treble");
      bassStave.addClef("bass");
      const tsStr = `${currentTs.numerator}/${currentTs.denominator}`;
      trebleStave.addTimeSignature(tsStr);
      bassStave.addTimeSignature(tsStr);
      if (currentKey) {
        const k = vexKeyName(currentKey.key, currentKey.scale);
        try {
          trebleStave.addKeySignature(k);
          bassStave.addKeySignature(k);
        } catch {
          /* unsupported key */
        }
      }
      trebleStave.setContext(ctx).draw();
      bassStave.setContext(ctx).draw();
      new StaveConnector(trebleStave, bassStave)
        .setType(3)
        .setContext(ctx)
        .draw();
      new StaveConnector(trebleStave, bassStave)
        .setType(1)
        .setContext(ctx)
        .draw();

      type Slot = { treble: number[]; bass: number[] };
      const slots: Slot[] = Array.from({ length: totalSlots }, () => ({
        treble: [],
        bass: [],
      }));

      for (const note of song.notes) {
        if (muted[note.trackIndex]) continue;
        if (scoreTracks && !scoreTracks.has(note.trackIndex)) continue;
        // Only notes whose onset falls *inside* the window get drawn. Notes
        // tied from a previous bar are not re-attacked in notation; including
        // them here would clamp to slot 0 and falsely thicken the first beat.
        if (note.time < win.startTime - 1e-4) continue;
        if (note.time > win.endTime - 1e-4) continue;
        const beatOff = timeToBeat(tempoMap, note.time) - win.startBeat;
        const slot = Math.max(
          0,
          Math.min(totalSlots - 1, Math.round(beatOff * slotsPerBeat))
        );
        const target =
          note.midi >= 60 ? slots[slot].treble : slots[slot].bass;
        if (!target.includes(note.midi)) target.push(note.midi);
      }

      /** Tickables paired with their start slot, so we can correlate them
       *  back to beat positions after formatting. */
      const makeNotes = (
        clef: "treble" | "bass",
        getMidis: (s: Slot) => number[]
      ) => {
        const result: {
          note: InstanceType<typeof StaveNote>;
          startSlot: number;
        }[] = [];
        let i = 0;
        while (i < totalSlots) {
          const midis = getMidis(slots[i]);
          const startSlot = i;
          if (midis.length === 0) {
            let run = 1;
            while (
              i + run < totalSlots &&
              getMidis(slots[i + run]).length === 0 &&
              run < 4
            )
              run++;
            // Cap to power-of-2 so the StaveNote duration matches the slots
            // it consumes (no orphan 3-slot dotted notes).
            if (run === 3) run = 2;
            const dur = durationFromSixteenths(run);
            result.push({
              note: new StaveNote({
                clef,
                keys: [clef === "treble" ? "b/4" : "d/3"],
                duration: dur + "r",
              }),
              startSlot,
            });
            i += run;
          } else {
            let run = 1;
            while (
              i + run < totalSlots &&
              getMidis(slots[i + run]).length === 0 &&
              run < 4
            )
              run++;
            if (run === 3) run = 2;
            const dur = durationFromSixteenths(run);
            const sorted = midis.slice().sort((a, b) => a - b);
            const keys = sorted.map(midiToVex);
            const note = new StaveNote({ clef, keys, duration: dur });
            sorted.forEach((m, idx) => {
              const acc = accidentalFor(m);
              if (acc) note.addModifier(new Accidental(acc), idx);
            });
            result.push({ note, startSlot });
            i += run;
          }
        }
        return result;
      };

      const trebleEntries = makeNotes("treble", (s) => s.treble);
      const bassEntries = makeNotes("bass", (s) => s.bass);
      const trebleNotes = trebleEntries.map((e) => e.note);
      const bassNotes = bassEntries.map((e) => e.note);

      const numBeats = totalSlots / 4;
      const voiceTreble = new Voice({ numBeats, beatValue: 4 });
      voiceTreble.setMode(Voice.Mode.SOFT).addTickables(trebleNotes);
      const voiceBass = new Voice({ numBeats, beatValue: 4 });
      voiceBass.setMode(Voice.Mode.SOFT).addTickables(bassNotes);

      new Formatter()
        .joinVoices([voiceTreble])
        .joinVoices([voiceBass])
        .format([voiceTreble, voiceBass], staveWidth - 90);
      voiceTreble.draw(ctx, trebleStave);
      voiceBass.draw(ctx, bassStave);

      /* Build playhead anchors: (beat, x) pairs sampled from the formatted
       * notes. We merge treble + bass entries by slot, then sort by beat. */
      const anchorMap = new Map<number, number>();
      const captureFromEntries = (
        entries: { note: InstanceType<typeof StaveNote>; startSlot: number }[]
      ) => {
        for (const e of entries) {
          const beat = win.startBeat + e.startSlot / slotsPerBeat;
          const x = e.note.getAbsoluteX();
          if (!anchorMap.has(beat) || (anchorMap.get(beat) ?? 0) > x) {
            anchorMap.set(beat, x);
          }
        }
      };
      captureFromEntries(trebleEntries);
      captureFromEntries(bassEntries);
      const anchors = Array.from(anchorMap.entries())
        .map(([beat, x]) => ({ beat, x }))
        .sort((a, b) => a.beat - b.beat);
      // Bookend with the stave's actual note end so the playhead reaches the
      // right edge cleanly at the end of the page.
      const endX = trebleStave.getNoteEndX();
      if (anchors.length === 0) {
        anchors.push({ beat: win.startBeat, x: trebleStave.getNoteStartX() });
      }
      anchors.push({ beat: win.endBeat, x: endX });
      noteAnchorsRef.current = anchors;

      const svg = el.querySelector("svg");
      if (svg) {
        const bar = document.createElementNS(
          "http://www.w3.org/2000/svg",
          "text"
        );
        bar.setAttribute("x", "6");
        bar.setAttribute("y", "16");
        bar.setAttribute("fill", "#7a7d83");
        bar.setAttribute("font-size", "10");
        bar.setAttribute(
          "font-family",
          "ui-monospace, SFMono-Regular, Menlo, monospace"
        );
        bar.textContent = `Bar ${win.startBar.bar + 1}`;
        svg.appendChild(bar);

        /* Playhead line — inside the SVG so it scales with the viewBox. */
        const ph = document.createElementNS(
          "http://www.w3.org/2000/svg",
          "line"
        );
        ph.setAttribute("x1", "0");
        ph.setAttribute("x2", "0");
        ph.setAttribute("y1", "30");
        ph.setAttribute("y2", "290");
        ph.setAttribute("stroke", "#f5a623");
        ph.setAttribute("stroke-width", "1.5");
        ph.setAttribute("opacity", "0.75");
        ph.setAttribute("pointer-events", "none");
        svg.appendChild(ph);
        playheadSvgRef.current = ph;

        /* Map the natural-coordinate SVG to the container via viewBox, so
         * dense pages shrink to fit instead of overflowing horizontally. */
        svg.setAttribute("viewBox", `0 0 ${naturalWidth} ${h}`);
        svg.setAttribute("preserveAspectRatio", "xMinYMin meet");
        svg.removeAttribute("width");
        svg.removeAttribute("height");
        (svg as unknown as HTMLElement).style.width = "100%";
        (svg as unknown as HTMLElement).style.height = "auto";
      }
    } catch (e) {
      setError(String(e));
    }
  }, [vex, width, win, song, muted, tempoMap, currentTs, currentKey, scoreTracks]);

  /* Playhead animation — updates the SVG line via rAF.
   *
   * Position is interpolated between *actual note x positions* captured from
   * VexFlow after layout. The playhead lives inside the SVG so it scales
   * automatically when the viewBox shrinks dense pages to fit the viewport. */
  useEffect(() => {
    let raf = 0;
    const tick = () => {
      const ph = playheadSvgRef.current;
      if (!ph) {
        raf = requestAnimationFrame(tick);
        return;
      }
      const pos = getLivePosition();
      const curBeat = timeToBeat(tempoMap, Math.max(0, pos));
      const anchors = noteAnchorsRef.current;
      let phX: number;
      if (anchors.length === 0) {
        phX = 0;
      } else if (curBeat <= anchors[0].beat) {
        phX = anchors[0].x;
      } else if (curBeat >= anchors[anchors.length - 1].beat) {
        phX = anchors[anchors.length - 1].x;
      } else {
        let lo = 0;
        let hi = anchors.length - 1;
        while (lo < hi - 1) {
          const mid = (lo + hi) >>> 1;
          if (anchors[mid].beat <= curBeat) lo = mid;
          else hi = mid;
        }
        const a = anchors[lo];
        const b = anchors[hi];
        const t = (curBeat - a.beat) / Math.max(1e-6, b.beat - a.beat);
        phX = a.x + (b.x - a.x) * t;
      }
      const xs = String(phX);
      ph.setAttribute("x1", xs);
      ph.setAttribute("x2", xs);
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [tempoMap, getLivePosition]);

  const allOn = scoreTracks === null;
  const toggleTrack = (idx: number) => {
    setScoreTracks((prev) => {
      // From "All", first click solos the clicked track. Subsequent clicks
      // toggle. Empty selection or full selection collapses back to "All".
      if (prev === null) return new Set([idx]);
      const next = new Set(prev);
      if (next.has(idx)) next.delete(idx);
      else next.add(idx);
      if (next.size === 0) return null;
      if (next.size === song.tracks.length) return null;
      return next;
    });
  };

  return (
    <div className="score-view">
      <div className="score-track-bar">
        <button
          className={`track-chip${allOn ? " on" : ""}`}
          onClick={() => setScoreTracks(null)}
        >
          All
        </button>
        {song.tracks.map((t) => {
          const on = allOn || (scoreTracks?.has(t.index) ?? false);
          return (
            <button
              key={t.index}
              className={`track-chip${on ? " on" : ""}`}
              onClick={() => toggleTrack(t.index)}
              style={
                on
                  ? ({
                      "--chip-color": t.color,
                      borderColor: t.color,
                      color: t.color,
                    } as React.CSSProperties)
                  : undefined
              }
              title={t.name}
            >
              <span
                className="track-chip-dot"
                style={{ background: t.color }}
              />
              {t.name.length > 14 ? t.name.slice(0, 13) + "…" : t.name}
            </button>
          );
        })}
        <div style={{ flex: 1 }} />
        <div className="bars-control">
          <button
            className="zoom-btn"
            onClick={() =>
              setBarsPerPage((n) => Math.max(SCORE_BARS_MIN, n - 1))
            }
            aria-label={t.score.fewerBars}
            disabled={barsPerPage <= SCORE_BARS_MIN}
          >
            −
          </button>
          <span className="zoom-readout">{barsPerPage} bars</span>
          <button
            className="zoom-btn"
            onClick={() =>
              setBarsPerPage((n) => Math.min(SCORE_BARS_MAX, n + 1))
            }
            aria-label={t.score.moreBars}
            disabled={barsPerPage >= SCORE_BARS_MAX}
          >
            +
          </button>
        </div>
      </div>
      {error && (
        <div className="error" style={{ margin: 12 }}>
          {t.score.renderFailed(error)}
        </div>
      )}
      {!vex && !error && (
        <div className="loading" style={{ padding: 32 }}>
          <div className="spinner" />
          <p>{t.score.loadingEngine}</p>
        </div>
      )}
      <div className="score-stage">
        <div ref={wrapRef} className="score-host" />
      </div>
    </div>
  );
}

/* ---------- VexFlow helpers ---------- */

function durationFromSixteenths(n: number): string {
  if (n >= 4) return "q";
  if (n === 3) return "8";
  if (n === 2) return "8";
  return "16";
}

function midiToVex(midi: number): string {
  const names = [
    "c",
    "c#",
    "d",
    "d#",
    "e",
    "f",
    "f#",
    "g",
    "g#",
    "a",
    "a#",
    "b",
  ];
  const name = names[midi % 12];
  const octave = Math.floor(midi / 12) - 1;
  return `${name}/${octave}`;
}

function accidentalFor(midi: number): string | null {
  const n = midi % 12;
  if (n === 1 || n === 3 || n === 6 || n === 8 || n === 10) return "#";
  return null;
}

const VEX_KEY_MAP: Record<string, string> = {
  C: "C",
  G: "G",
  D: "D",
  A: "A",
  E: "E",
  B: "B",
  "F#": "F#",
  "C#": "C#",
  F: "F",
  Bb: "Bb",
  Eb: "Eb",
  Ab: "Ab",
  Db: "Db",
  Gb: "Gb",
  Cb: "Cb",
};

const VEX_MINOR_MAP: Record<string, string> = {
  A: "Am",
  E: "Em",
  B: "Bm",
  "F#": "F#m",
  "C#": "C#m",
  "G#": "G#m",
  "D#": "D#m",
  "A#": "A#m",
  D: "Dm",
  G: "Gm",
  C: "Cm",
  F: "Fm",
  Bb: "Bbm",
  Eb: "Ebm",
  Ab: "Abm",
};

function vexKeyName(key: string, scale: "major" | "minor"): string {
  if (scale === "minor") return VEX_MINOR_MAP[key] ?? "Am";
  return VEX_KEY_MAP[key] ?? "C";
}
