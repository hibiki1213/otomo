import { useEffect, useMemo, useRef, useState } from "react";
import type { ControlPoint, SongInfo } from "@/lib/midiEngine";
import { useI18n } from "@/lib/i18n";
import { ccName } from "@/lib/midiNames";

export const LANE_H = 92;

/** What the lane graphs: note velocities, pitch bend, pressure, or one controller. */
type LaneKind = "velocity" | "bend" | "pressure" | `cc${number}`;

// Controllers that are settings rather than performance (bank, RPN/NRPN,
// channel mode): one-off values, read in the event list instead.
const SETTINGS = new Set([0, 6, 32, 38, 96, 97, 98, 99, 100, 101, 120, 121, 122, 123, 124, 125, 126, 127]);
// The ones people reach for first, in that order; the rest follow by number.
const FIRST = [1, 11, 7, 10, 64, 91, 93, 74];
const KIND_KEY = "otomo.lane.kind";

interface Series {
  channel: number;
  color: string;
  muted: boolean;
  name: string;
  points: ControlPoint[];
}

function readKind(): LaneKind {
  try {
    return (localStorage.getItem(KIND_KEY) as LaneKind | null) ?? "velocity";
  } catch {
    return "velocity";
  }
}

/** Index of the last point at or before `t`, or -1. */
function lastAtOrBefore(points: { time: number }[], t: number): number {
  let lo = 0;
  let hi = points.length - 1;
  let found = -1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if (points[mid].time <= t) {
      found = mid;
      lo = mid + 1;
    } else hi = mid - 1;
  }
  return found;
}

/**
 * A graph under the piano roll, on the roll's time axis: velocity, pitch
 * bend, aftertouch or any controller the song uses, for one track or all.
 * Everything here is played; none of it shows in the notes themselves.
 */
export default function ControllerLane({
  song,
  muted,
  scrollX,
  pxPerSec,
  viewportW,
  dark,
  barTimes,
  getLivePosition,
  onSeekTime,
}: {
  song: SongInfo;
  muted: boolean[];
  scrollX: number;
  pxPerSec: number;
  viewportW: number;
  dark: boolean;
  /** Where the bar lines are, in seconds. */
  barTimes: number[];
  getLivePosition: () => number;
  onSeekTime: (seconds: number) => void;
}) {
  const { t } = useI18n();
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const playheadRef = useRef<HTMLDivElement | null>(null);
  const [kind, setKind] = useState<LaneKind>(readKind);
  const [track, setTrack] = useState<number | "all">("all");
  const [hover, setHover] = useState<{ x: number; y: number } | null>(null);

  useEffect(() => setTrack("all"), [song]);
  useEffect(() => {
    try {
      localStorage.setItem(KIND_KEY, kind);
    } catch {
      /* ignore */
    }
  }, [kind]);

  // Channels the song's tracks play on; controllers belong to channels.
  const channelTracks = useMemo(() => {
    const map = new Map<number, number[]>();
    for (const tr of song.tracks) map.set(tr.channel, [...(map.get(tr.channel) ?? []), tr.index]);
    return map;
  }, [song]);

  const pointsOf = (k: LaneKind, channel: number): ControlPoint[] => {
    const c = song.channels[channel];
    if (!c) return [];
    if (k === "bend") return c.bend;
    if (k === "pressure") return c.pressure;
    if (k.startsWith("cc")) return c.cc[Number(k.slice(2))] ?? [];
    return [];
  };

  // Every kind with something to show, with how many events it has.
  const kinds = useMemo(() => {
    const total = (k: LaneKind) => {
      let n = 0;
      for (const ch of channelTracks.keys()) n += pointsOf(k, ch).length;
      return n;
    };
    const ccs = new Set<number>();
    for (const ch of channelTracks.keys()) {
      for (const n of Object.keys(song.channels[ch]?.cc ?? {})) {
        if (!SETTINGS.has(Number(n))) ccs.add(Number(n));
      }
    }
    const ordered = [
      ...FIRST.filter((n) => ccs.has(n)),
      ...[...ccs].filter((n) => !FIRST.includes(n)).sort((a, b) => a - b),
    ];
    const list: { kind: LaneKind; label: string; count: number }[] = [
      { kind: "velocity", label: t.lane.velocity, count: song.notes.length },
      { kind: "bend", label: t.lane.bend, count: total("bend") },
      ...ordered.map((n) => ({ kind: `cc${n}` as LaneKind, label: t.lane.cc(n, ccName(n)), count: total(`cc${n}`) })),
      { kind: "pressure", label: t.lane.pressure, count: total("pressure") },
    ];
    return list.filter((k) => k.kind === "velocity" || k.count > 0);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [song, channelTracks, t]);

  const active: LaneKind = kinds.some((k) => k.kind === kind) ? kind : "velocity";

  // The curves to draw: one per channel, coloured after its first track.
  const series = useMemo<Series[]>(() => {
    if (active === "velocity") return [];
    const channels =
      track === "all" ? [...channelTracks.keys()] : [song.tracks[track]?.channel].filter((c) => c != null);
    return channels
      .map((channel) => {
        const tracks = channelTracks.get(channel) ?? [];
        const lead = song.tracks[track === "all" ? tracks[0] : track];
        return {
          channel,
          color: lead?.color ?? "#888",
          muted: tracks.every((i) => muted[i]),
          name: lead?.name ?? `Ch ${channel + 1}`,
          points: pointsOf(active, channel),
        };
      })
      .filter((s) => s.points.length > 0);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active, track, song, channelTracks, muted]);

  // Value range: MIDI's 0–127, or ± the widest bend in sight.
  const range = useMemo(() => {
    if (active !== "bend") return { lo: 0, hi: 127, centre: active === "cc10" ? 64 : null };
    let widest = 2;
    for (const s of series) for (const p of s.points) widest = Math.max(widest, Math.abs(p.value));
    const r = Math.ceil(widest);
    return { lo: -r, hi: r, centre: 0 };
  }, [active, series]);

  const empty =
    active === "velocity"
      ? track !== "all" && !song.notes.some((n) => n.trackIndex === track)
      : series.length === 0;

  /* draw */
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const dpr = Math.min(window.devicePixelRatio || 1, 2);
    const w = viewportW;
    const h = LANE_H;
    if (canvas.width !== w * dpr || canvas.height !== h * dpr) {
      canvas.width = w * dpr;
      canvas.height = h * dpr;
    }
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);

    const bg = dark ? "#0d0e11" : "#eef0f3";
    const grid = dark ? "rgba(255,255,255,0.06)" : "rgba(0,0,0,0.07)";
    const label = dark ? "rgba(255,255,255,0.45)" : "#737373";
    const mutedColor = dark ? "#3a3c40" : "#bdc0c5";
    const pad = 6;
    const yOf = (v: number) => pad + (1 - (v - range.lo) / (range.hi - range.lo)) * (h - pad * 2);
    const startT = scrollX / pxPerSec;
    const endT = (scrollX + w) / pxPerSec;

    ctx.fillStyle = bg;
    ctx.fillRect(0, 0, w, h);

    // Bar lines and the guide lines.
    ctx.strokeStyle = grid;
    ctx.lineWidth = 1;
    for (const bt of barTimes) {
      if (bt < startT || bt > endT) continue;
      const x = Math.round(bt * pxPerSec - scrollX) + 0.5;
      ctx.beginPath();
      ctx.moveTo(x, 0);
      ctx.lineTo(x, h);
      ctx.stroke();
    }
    for (const v of [range.lo, range.centre ?? (range.lo + range.hi) / 2, range.hi]) {
      const y = Math.round(yOf(v)) + 0.5;
      ctx.setLineDash(v === range.centre ? [3, 3] : []);
      ctx.beginPath();
      ctx.moveTo(0, y);
      ctx.lineTo(w, y);
      ctx.stroke();
    }
    ctx.setLineDash([]);

    if (active === "velocity") {
      // A stem per note, as tall as it is loud.
      let i = lastAtOrBefore(song.notes, startT - 0.05);
      i = Math.max(0, i);
      for (; i < song.notes.length; i++) {
        const n = song.notes[i];
        if (n.time > endT) break;
        if (track !== "all" && n.trackIndex !== track) continue;
        const x = Math.round(n.time * pxPerSec - scrollX) + 0.5;
        const y = yOf(n.velocity * 127);
        const isMuted = muted[n.trackIndex];
        ctx.strokeStyle = isMuted ? mutedColor : song.tracks[n.trackIndex].color;
        ctx.fillStyle = ctx.strokeStyle;
        ctx.globalAlpha = isMuted ? 0.4 : 0.85;
        ctx.beginPath();
        ctx.moveTo(x, h - pad);
        ctx.lineTo(x, y);
        ctx.stroke();
        ctx.fillRect(x - 1.5, y - 1.5, 3, 3);
      }
      ctx.globalAlpha = 1;
    } else {
      const single = series.length === 1;
      for (const s of series) {
        const pts = s.points;
        const from = Math.max(0, lastAtOrBefore(pts, startT));
        if (pts[from].time > endT) continue;
        ctx.beginPath();
        let firstX = 0;
        let prevY = 0;
        let lastX = 0;
        for (let i = from; i < pts.length; i++) {
          const x = Math.max(-1, pts[i].time * pxPerSec - scrollX);
          if (x > w + 1) break;
          const y = yOf(pts[i].value);
          if (i === from) {
            ctx.moveTo(x, y);
            firstX = x;
          } else {
            ctx.lineTo(x, prevY);
            ctx.lineTo(x, y);
          }
          prevY = y;
          lastX = x;
        }
        // The last value holds until the next change, or the end.
        const next = lastAtOrBefore(pts, endT) + 1;
        const holdTo = next < pts.length ? pts[next].time : song.duration;
        lastX = Math.min(w + 1, holdTo * pxPerSec - scrollX);
        ctx.lineTo(lastX, prevY);
        const color = s.muted ? mutedColor : s.color;
        ctx.strokeStyle = color;
        ctx.lineWidth = single ? 1.5 : 1.25;
        ctx.globalAlpha = s.muted ? 0.5 : 0.95;
        ctx.stroke();
        if (single) {
          const base = yOf(range.centre ?? range.lo);
          ctx.lineTo(lastX, base);
          ctx.lineTo(firstX, base);
          ctx.closePath();
          ctx.fillStyle = color;
          ctx.globalAlpha = 0.14;
          ctx.fill();
        }
        ctx.globalAlpha = 1;
      }
    }

    // Scale, on the left.
    ctx.font = '9px ui-monospace, SFMono-Regular, Menlo, monospace';
    ctx.fillStyle = label;
    ctx.textBaseline = "middle";
    const scale =
      active === "bend"
        ? [`+${range.hi}`, "0", `${range.lo}`]
        : active === "cc10"
        ? [t.lane.right, t.lane.centre, t.lane.left]
        : ["127", "64", "0"];
    ctx.fillText(scale[0], 4, pad + 4);
    ctx.fillText(scale[1], 4, h / 2);
    ctx.fillText(scale[2], 4, h - pad - 4);

    if (empty) {
      ctx.textAlign = "center";
      ctx.font = '11px -apple-system, BlinkMacSystemFont, system-ui, sans-serif';
      ctx.fillText(t.lane.none, w / 2, h / 2);
      ctx.textAlign = "start";
    }
  }, [active, series, range, track, song, muted, scrollX, pxPerSec, viewportW, dark, barTimes, empty, t]);

  /* playhead */
  useEffect(() => {
    const ph = playheadRef.current;
    if (!ph) return;
    let raf = 0;
    const tick = () => {
      const x = getLivePosition() * pxPerSec - scrollX;
      ph.style.transform = `translateX(${x}px)`;
      ph.style.opacity = x >= -2 && x <= viewportW + 2 ? "1" : "0";
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [pxPerSec, scrollX, viewportW, getLivePosition]);

  // The value under the pointer: the nearest stem, or the nearest curve.
  const readout = useMemo(() => {
    if (!hover) return null;
    const time = (scrollX + hover.x) / pxPerSec;
    if (active === "velocity") {
      let best: { d: number; text: string } | null = null;
      const from = Math.max(0, lastAtOrBefore(song.notes, time - 8 / pxPerSec));
      for (let i = from; i < song.notes.length; i++) {
        const n = song.notes[i];
        const d = Math.abs(n.time - time) * pxPerSec;
        if (n.time - time > 8 / pxPerSec) break;
        if (track !== "all" && n.trackIndex !== track) continue;
        if (d <= 6 && (!best || d < best.d)) {
          best = { d, text: `${song.tracks[n.trackIndex].name} · ${Math.round(n.velocity * 127)}` };
        }
      }
      return best?.text ?? null;
    }
    const h = LANE_H;
    const pad = 6;
    let best: { d: number; text: string } | null = null;
    for (const s of series) {
      const i = lastAtOrBefore(s.points, time);
      if (i < 0) continue;
      const v = s.points[i].value;
      const y = pad + (1 - (v - range.lo) / (range.hi - range.lo)) * (h - pad * 2);
      const d = Math.abs(y - hover.y);
      if (!best || d < best.d) best = { d, text: `${s.name} · ${formatValue(active, v, t)}` };
    }
    return best?.text ?? null;
  }, [hover, active, series, range, song, track, scrollX, pxPerSec, t]);

  return (
    <div className="lane-shell">
      <div className="lane-head">
        <select
          value={active}
          onChange={(e) => setKind(e.target.value as LaneKind)}
          aria-label={t.lane.kindLabel}
        >
          {kinds.map((k) => (
            <option key={k.kind} value={k.kind}>
              {t.lane.counted(k.label, k.count)}
            </option>
          ))}
        </select>
        <select
          value={track}
          onChange={(e) => setTrack(e.target.value === "all" ? "all" : Number(e.target.value))}
          aria-label={t.lane.trackLabel}
        >
          <option value="all">{t.lane.allTracks}</option>
          {song.tracks.map((tr) => (
            <option key={tr.index} value={tr.index}>
              {tr.name} · {tr.instrument}
            </option>
          ))}
        </select>
        <div style={{ flex: 1 }} />
        {readout && <span className="lane-readout">{readout}</span>}
      </div>
      <div
        className="lane-main"
        style={{ height: LANE_H }}
        onPointerMove={(e) => {
          const r = e.currentTarget.getBoundingClientRect();
          setHover({ x: e.clientX - r.left, y: e.clientY - r.top });
        }}
        onPointerLeave={() => setHover(null)}
        onPointerDown={(e) => {
          const r = e.currentTarget.getBoundingClientRect();
          onSeekTime(Math.max(0, Math.min(song.duration, (scrollX + e.clientX - r.left) / pxPerSec)));
        }}
      >
        <canvas ref={canvasRef} />
        <div ref={playheadRef} className="lane-playhead" />
      </div>
    </div>
  );
}

function formatValue(kind: LaneKind, v: number, t: ReturnType<typeof useI18n>["t"]): string {
  if (kind === "bend") {
    const r = Math.round(v * 100) / 100;
    return t.lane.semitones(r > 0 ? `+${r}` : String(r));
  }
  if (kind === "cc10") {
    if (v === 64) return t.lane.centre;
    return v < 64 ? `${t.lane.left}${64 - v}` : `${t.lane.right}${v - 64}`;
  }
  return String(v);
}
