"use client";

import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import {
  MidiEngine,
  type PlayerState,
  type SongInfo,
} from "@/lib/midiEngine";

/* ---------- icons (SF Symbols inspired) ---------- */

function Icon({
  path,
  size = 24,
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
  back: "M11.5 12 20 6.2A1 1 0 0 0 19.4 4.4H19a1 1 0 0 0-.56.17L9 11V5.5a1 1 0 0 0-1.56-.83l-.06.04V5.5h-.01l-.02.01L7 5.5a1 1 0 0 0-.44.1L6 6v12l.56.4A1 1 0 0 0 7 18.5a1 1 0 0 0 1-1V13l9.44 6.43A1 1 0 0 0 18 20h.4a1 1 0 0 0 1.6-1.8L11.5 12z",
  forward:
    "M12.5 12 4 6.2A1 1 0 0 1 4.6 4.4H5a1 1 0 0 1 .56.17L15 11V5.5a1 1 0 0 1 2-.4V18.5a1 1 0 0 1-2 .4V13l-9.44 6.43A1 1 0 0 1 5 20h-.4a1 1 0 0 1-1.6-1.8L12.5 12z",
  speaker:
    "M13 4.5 8.2 8.3H4.6a1 1 0 0 0-1 1v5.4a1 1 0 0 0 1 1h3.6L13 19.5a1 1 0 0 0 1.6-.8V5.3A1 1 0 0 0 13 4.5zm3.7 3a1 1 0 0 0-.1 1.5 4 4 0 0 1 0 6 1 1 0 1 0 1.4 1.5 6 6 0 0 0 0-9 1 1 0 0 0-1.3 0z",
  speakerOff:
    "M13 4.5 8.2 8.3H4.6a1 1 0 0 0-1 1v5.4a1 1 0 0 0 1 1h3.6L13 19.5a1 1 0 0 0 1.6-.8V5.3A1 1 0 0 0 13 4.5zm4.3 4.8 1.8 1.8 1.8-1.8a1 1 0 1 1 1.4 1.4L20.5 12.5l1.8 1.8a1 1 0 0 1-1.4 1.4l-1.8-1.8-1.8 1.8a1 1 0 0 1-1.4-1.4l1.8-1.8-1.8-1.8a1 1 0 0 1 1.4-1.4z",
  note: "M9 17.5a3 3 0 1 1-2-2.83V7.2a1 1 0 0 1 .76-.97l9-2.2A1 1 0 0 1 18 5v9.5a3 3 0 1 1-2-2.83V8.78l-7 1.7v7.02z",
  upload:
    "M12 3a1 1 0 0 1 .7.3l4 4a1 1 0 0 1-1.4 1.4L13 6.4V15a1 1 0 1 1-2 0V6.4L8.7 8.7a1 1 0 0 1-1.4-1.4l4-4A1 1 0 0 1 12 3zM5 14a1 1 0 0 1 1 1v3h12v-3a1 1 0 1 1 2 0v4a1 1 0 0 1-1 1H5a1 1 0 0 1-1-1v-4a1 1 0 0 1 1-1z",
};

/* ---------- helpers ---------- */

function formatTime(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) seconds = 0;
  const m = Math.floor(seconds / 60);
  const s = Math.floor(seconds % 60);
  return `${m}:${s.toString().padStart(2, "0")}`;
}

const ROLL_HEIGHT = 220;

/* ---------- component ---------- */

export default function Player() {
  const engineRef = useRef<MidiEngine | null>(null);
  const fileInputRef = useRef<HTMLInputElement | null>(null);

  const [song, setSong] = useState<SongInfo | null>(null);
  const [state, setState] = useState<PlayerState>("idle");
  const [position, setPosition] = useState(0);
  const [volume, setVolume] = useState(0.9);
  const [muted, setMuted] = useState<boolean[]>([]);
  const [dragging, setDragging] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [progressLabel, setProgressLabel] = useState("");

  /* engine lifecycle */
  useEffect(() => {
    const engine = new MidiEngine();
    engine.onState = (s) => setState(s);
    engine.onTick = (p) => setPosition(p);
    engineRef.current = engine;
    return () => engine.dispose();
  }, []);

  const loadFile = useCallback(async (file: File) => {
    const engine = engineRef.current;
    if (!engine) return;
    if (!/\.midi?$/i.test(file.name)) {
      setError("MIDI ファイル（.mid / .midi）を選択してください。");
      return;
    }
    setError(null);
    setProgressLabel("ファイルを解析しています…");
    try {
      const buffer = await file.arrayBuffer();
      setProgressLabel("楽器の音源を読み込んでいます…");
      const info = await engine.load(buffer, file.name);
      setSong(info);
      setMuted(new Array(info.tracks.length).fill(false));
      setPosition(0);
      engine.setMasterVolume(volume);
    } catch {
      setError("このファイルを読み込めませんでした。MIDI 形式をご確認ください。");
      setState("idle");
    }
  }, [volume]);

  /* drag & drop */
  const onDrop = useCallback(
    (e: React.DragEvent) => {
      e.preventDefault();
      setDragging(false);
      const file = e.dataTransfer.files[0];
      if (file) loadFile(file);
    },
    [loadFile]
  );

  /* transport */
  const togglePlay = useCallback(() => {
    const engine = engineRef.current;
    if (!engine || !song) return;
    if (engine.state === "playing") engine.pause();
    else engine.play();
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

  /* keyboard: space toggles playback */
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.code !== "Space" || !song) return;
      const tag = (e.target as HTMLElement)?.tagName;
      if (tag === "INPUT" || tag === "TEXTAREA") return;
      e.preventDefault();
      togglePlay();
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [song, togglePlay]);

  const toggleMute = useCallback((index: number) => {
    setMuted((prev) => {
      const next = [...prev];
      next[index] = !next[index];
      engineRef.current?.setTrackMuted(index, next[index]);
      return next;
    });
  }, []);

  const onVolume = useCallback((v: number) => {
    setVolume(v);
    engineRef.current?.setMasterVolume(v);
  }, []);

  // Memoized so the track list does not re-render on every playback tick.
  const trackList = useMemo(() => {
    if (!song) return null;
    return (
      <div className="tracks">
        <div className="tracks-head">トラック · {song.tracks.length}</div>
        {song.tracks.map((track) => (
          <div className="track-row" key={track.index}>
            <span
              className="track-swatch"
              style={{
                background: track.color,
                opacity: muted[track.index] ? 0.3 : 1,
              }}
            />
            <div className="track-info">
              <div
                className="track-name"
                style={{ opacity: muted[track.index] ? 0.5 : 1 }}
              >
                {track.name}
              </div>
              <div className="track-detail">
                {track.instrument} · {track.noteCount} ノート
              </div>
            </div>
            <button
              className={`mute-btn${muted[track.index] ? " muted" : ""}`}
              onClick={() => toggleMute(track.index)}
              aria-label={muted[track.index] ? "ミュート解除" : "ミュート"}
            >
              <Icon
                path={muted[track.index] ? ICON.speakerOff : ICON.speaker}
                size={19}
              />
            </button>
          </div>
        ))}
      </div>
    );
  }, [song, muted, toggleMute]);

  const isPlaying = state === "playing";
  const progress = song && song.duration > 0 ? position / song.duration : 0;

  return (
    <div className="card">
      {!song && state !== "loading" && (
        <>
          <DropZone
            dragging={dragging}
            setDragging={setDragging}
            onDrop={onDrop}
            onPick={() => fileInputRef.current?.click()}
          />
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
          <NowPlaying song={song} position={position} />

          <PianoRoll
            song={song}
            position={position}
            muted={muted}
            onSeek={seekTo}
          />

          <div className="transport">
            <Scrubber
              progress={progress}
              position={position}
              duration={song.duration}
              onSeek={seekTo}
            />
            <div className="controls">
              <button
                className="control-btn"
                onClick={() => skip(-10)}
                aria-label="10秒戻る"
              >
                <Icon path={ICON.back} size={22} />
              </button>
              <button
                className="control-btn primary"
                onClick={togglePlay}
                aria-label={isPlaying ? "一時停止" : "再生"}
              >
                <Icon path={isPlaying ? ICON.pause : ICON.play} size={30} />
              </button>
              <button
                className="control-btn"
                onClick={() => skip(10)}
                aria-label="10秒進む"
              >
                <Icon path={ICON.forward} size={22} />
              </button>
            </div>
            <div className="volume">
              <Icon path={ICON.speaker} size={17} />
              <input
                type="range"
                min={0}
                max={1}
                step={0.01}
                value={volume}
                onChange={(e) => onVolume(Number(e.target.value))}
                aria-label="音量"
              />
            </div>
          </div>

          {trackList}

          {error && <div className="error">{error}</div>}

          <div className="statusbar">
            <span style={{ fontSize: 12, color: "var(--text-tertiary)" }}>
              スペースキーで再生 / 一時停止
            </span>
            <button
              className="btn-tinted"
              onClick={() => fileInputRef.current?.click()}
            >
              別のファイルを開く
            </button>
          </div>
        </>
      )}

      <input
        ref={fileInputRef}
        type="file"
        accept=".mid,.midi,audio/midi"
        style={{ display: "none" }}
        onChange={(e) => {
          const file = e.target.files?.[0];
          if (file) loadFile(file);
          e.target.value = "";
        }}
      />
    </div>
  );
}

/* ---------- drop zone ---------- */

function DropZone({
  dragging,
  setDragging,
  onDrop,
  onPick,
}: {
  dragging: boolean;
  setDragging: (v: boolean) => void;
  onDrop: (e: React.DragEvent) => void;
  onPick: () => void;
}) {
  return (
    <div
      className={`dropzone${dragging ? " dragging" : ""}`}
      onDragOver={(e) => {
        e.preventDefault();
        setDragging(true);
      }}
      onDragLeave={() => setDragging(false)}
      onDrop={onDrop}
    >
      <div className="dropzone-icon">
        <Icon path={ICON.upload} size={28} />
      </div>
      <h2>MIDI ファイルをドロップ</h2>
      <p>.mid / .midi 形式に対応しています</p>
      <button className="btn-filled" onClick={onPick}>
        ファイルを選択
      </button>
    </div>
  );
}

/* ---------- now playing header ---------- */

function NowPlaying({
  song,
  position,
}: {
  song: SongInfo;
  position: number;
}) {
  return (
    <div className="nowplaying">
      <div className="nowplaying-art">
        <Icon path={ICON.note} size={26} />
      </div>
      <div className="nowplaying-meta">
        <div className="nowplaying-title">
          {song.name.replace(/\.midi?$/i, "")}
        </div>
        <div className="nowplaying-sub">
          {song.bpm} BPM
          <span className="dot">·</span>
          {song.tracks.length} トラック
          <span className="dot">·</span>
          {formatTime(position)} / {formatTime(song.duration)}
        </div>
      </div>
    </div>
  );
}

/* ---------- scrubber ---------- */

function Scrubber({
  progress,
  position,
  duration,
  onSeek,
}: {
  progress: number;
  position: number;
  duration: number;
  onSeek: (ratio: number) => void;
}) {
  const trackRef = useRef<HTMLDivElement | null>(null);
  const draggingRef = useRef(false);

  const seekFromEvent = useCallback(
    (clientX: number) => {
      const el = trackRef.current;
      if (!el) return;
      const rect = el.getBoundingClientRect();
      const ratio = (clientX - rect.left) / rect.width;
      onSeek(Math.max(0, Math.min(1, ratio)));
    },
    [onSeek]
  );

  useEffect(() => {
    const move = (e: PointerEvent) => {
      if (draggingRef.current) seekFromEvent(e.clientX);
    };
    const up = () => {
      draggingRef.current = false;
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    return () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
    };
  }, [seekFromEvent]);

  const pct = `${Math.max(0, Math.min(1, progress)) * 100}%`;

  return (
    <div className="scrubber">
      <span>{formatTime(position)}</span>
      <div
        className="scrubber-track"
        ref={trackRef}
        onPointerDown={(e) => {
          draggingRef.current = true;
          seekFromEvent(e.clientX);
        }}
      >
        <div className="scrubber-fill" style={{ width: pct }} />
        <div className="scrubber-knob" style={{ left: pct }} />
      </div>
      <span>{formatTime(duration)}</span>
    </div>
  );
}

/* ---------- piano roll visualization ---------- */

function PianoRoll({
  song,
  position,
  muted,
  onSeek,
}: {
  song: SongInfo;
  position: number;
  muted: boolean[];
  onSeek: (ratio: number) => void;
}) {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const layersRef = useRef<{
    dim: HTMLCanvasElement;
    bright: HTMLCanvasElement;
    width: number;
  } | null>(null);
  const [width, setWidth] = useState(640);
  const [dark, setDark] = useState(false);

  /* track container width */
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const update = () => setWidth(canvas.clientWidth || 640);
    update();
    const observer = new ResizeObserver(update);
    observer.observe(canvas);
    return () => observer.disconnect();
  }, []);

  /* track color scheme */
  useEffect(() => {
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    setDark(mq.matches);
    const handler = (e: MediaQueryListEvent) => setDark(e.matches);
    mq.addEventListener("change", handler);
    return () => mq.removeEventListener("change", handler);
  }, []);

  const padded = useMemo(() => {
    const lo = Math.max(0, song.minMidi - 2);
    const hi = Math.min(127, song.maxMidi + 2);
    return { lo, hi, range: Math.max(12, hi - lo) };
  }, [song]);

  /* build cached note layers */
  useEffect(() => {
    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    const w = width;
    const h = ROLL_HEIGHT;

    const makeLayer = (bright: boolean) => {
      const layer = document.createElement("canvas");
      layer.width = w * dpr;
      layer.height = h * dpr;
      const ctx = layer.getContext("2d");
      if (!ctx) return layer;
      ctx.scale(dpr, dpr);

      const pxPerSec = w / Math.max(0.001, song.duration);
      const noteH = h / padded.range;
      const grey = dark ? "#48484a" : "#c7c7cc";

      // octave guide lines
      ctx.strokeStyle = dark
        ? "rgba(255,255,255,0.05)"
        : "rgba(0,0,0,0.05)";
      ctx.lineWidth = 1;
      for (let m = Math.ceil(padded.lo / 12) * 12; m <= padded.hi; m += 12) {
        const y = h - (m - padded.lo) * noteH;
        ctx.beginPath();
        ctx.moveTo(0, y);
        ctx.lineTo(w, y);
        ctx.stroke();
      }

      for (const note of song.notes) {
        const isMuted = muted[note.trackIndex];
        const x = note.time * pxPerSec;
        const noteW = Math.max(2.5, note.duration * pxPerSec - 1);
        const y = h - (note.midi - padded.lo + 1) * noteH;
        const rectH = Math.max(2.5, noteH - 1);
        const color = isMuted
          ? grey
          : song.tracks[note.trackIndex].color;
        ctx.globalAlpha = bright && !isMuted ? 0.42 + note.velocity * 0.58 : 0.26;
        ctx.fillStyle = color;
        ctx.beginPath();
        ctx.roundRect(x, y, noteW, rectH, Math.min(3, rectH / 2));
        ctx.fill();
      }
      ctx.globalAlpha = 1;
      return layer;
    };

    layersRef.current = {
      dim: makeLayer(false),
      bright: makeLayer(true),
      width: w,
    };
  }, [song, width, muted, dark, padded]);

  /* composite per frame */
  useEffect(() => {
    const canvas = canvasRef.current;
    const layers = layersRef.current;
    if (!canvas || !layers) return;
    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    const w = width;
    const h = ROLL_HEIGHT;
    if (canvas.width !== w * dpr || canvas.height !== h * dpr) {
      canvas.width = w * dpr;
      canvas.height = h * dpr;
    }
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, h);

    ctx.drawImage(layers.dim, 0, 0, w, h);

    const playX = Math.max(
      0,
      Math.min(1, position / Math.max(0.001, song.duration))
    ) * w;

    ctx.save();
    ctx.beginPath();
    ctx.rect(0, 0, playX, h);
    ctx.clip();
    ctx.drawImage(layers.bright, 0, 0, w, h);
    ctx.restore();

    // playhead
    ctx.strokeStyle = dark ? "#0a84ff" : "#0071e3";
    ctx.lineWidth = 2;
    ctx.beginPath();
    ctx.moveTo(playX, 0);
    ctx.lineTo(playX, h);
    ctx.stroke();
    ctx.fillStyle = ctx.strokeStyle;
    ctx.beginPath();
    ctx.arc(playX, 0, 4.5, 0, Math.PI * 2);
    ctx.fill();
  }, [position, width, song, dark]);

  return (
    <div className="roll">
      <canvas
        ref={canvasRef}
        className="roll-canvas"
        onClick={(e) => {
          const rect = e.currentTarget.getBoundingClientRect();
          onSeek((e.clientX - rect.left) / rect.width);
        }}
      />
    </div>
  );
}
