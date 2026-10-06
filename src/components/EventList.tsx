import { useEffect, useMemo, useRef, useState } from "react";
import type { SongInfo } from "@/lib/midiEngine";
import { listEvents, type EventCategory, type EventListing } from "@/lib/midiEvents";
import { useI18n } from "@/lib/i18n";
import { beatPosition, buildBarMap, buildTempoMap } from "@/lib/timeline";

const ROW_H = 22;
const VIEW_H = 420;
const OVERSCAN = 12;
const CATEGORIES: EventCategory[] = ["note", "control", "bend", "program", "sysex", "meta"];
/** Notes start hidden: they are what the piano roll already shows. */
const DEFAULT_SHOWN = new Set<EventCategory>(["control", "bend", "program", "sysex", "meta"]);
const NO_CHANNEL = -1;

/**
 * Every event in the file as a table: what the piano roll cannot draw (bank
 * selects, NRPNs, SysEx, text) is at least readable here. Click a row to
 * play from it.
 */
export default function EventList({
  song,
  buffer,
  position,
  isPlaying,
  onSeekTime,
}: {
  song: SongInfo;
  buffer: ArrayBuffer;
  position: number;
  isPlaying: boolean;
  onSeekTime: (seconds: number) => void;
}) {
  const { t, lang } = useI18n();
  const [listing, setListing] = useState<EventListing | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [shown, setShown] = useState(DEFAULT_SHOWN);
  const [channel, setChannel] = useState<number | "all">("all");
  const [follow, setFollow] = useState(true);
  const [scrollTop, setScrollTop] = useState(0);
  const scrollRef = useRef<HTMLDivElement | null>(null);

  // Re-read on a new file, and in a new language (the rows are worded).
  useEffect(() => {
    let cancelled = false;
    setListing(null);
    setError(null);
    listEvents(buffer)
      .then((l) => !cancelled && setListing(l))
      .catch((e) => !cancelled && setError(String(e)));
    return () => {
      cancelled = true;
    };
  }, [buffer, lang]);

  const bars = useMemo(
    () => buildBarMap(song.timeSignatures, buildTempoMap(song.tempos, song.duration)).bars,
    [song]
  );

  // The channels in use, named after a track that plays on each.
  const channels = useMemo(() => {
    const used = new Set<number>();
    for (const r of listing?.rows ?? []) if (r.channel != null) used.add(r.channel);
    return [...used].sort((a, b) => a - b).map((ch) => {
      const track = song.tracks.find((tr) => tr.channel === ch);
      return { channel: ch, name: track?.name ?? "", color: track?.color ?? "var(--text-faint)" };
    });
  }, [listing, song]);
  const colorOf = useMemo(() => new Map(channels.map((c) => [c.channel, c.color])), [channels]);

  const counts = useMemo(() => {
    const c = Object.fromEntries(CATEGORIES.map((k) => [k, 0])) as Record<EventCategory, number>;
    for (const r of listing?.rows ?? []) {
      if (channel === "all" || (r.channel ?? NO_CHANNEL) === channel) c[r.category]++;
    }
    return c;
  }, [listing, channel]);

  const rows = useMemo(
    () =>
      (listing?.rows ?? []).filter(
        (r) => shown.has(r.category) && (channel === "all" || (r.channel ?? NO_CHANNEL) === channel)
      ),
    [listing, shown, channel]
  );

  // The row at the playhead: the last one that has started.
  const current = useMemo(() => {
    let lo = 0;
    let hi = rows.length - 1;
    let found = -1;
    while (lo <= hi) {
      const mid = (lo + hi) >> 1;
      if (rows[mid].time <= position + 1e-4) {
        found = mid;
        lo = mid + 1;
      } else hi = mid - 1;
    }
    return found;
  }, [rows, position]);

  // Keep it in view while playing, without fighting a hand on the scroll bar.
  useEffect(() => {
    const el = scrollRef.current;
    if (!el || !follow || !isPlaying || current < 0) return;
    const top = current * ROW_H;
    const bodyH = el.clientHeight - ROW_H;
    if (top < el.scrollTop || top + ROW_H > el.scrollTop + bodyH) {
      el.scrollTop = Math.max(0, top - bodyH / 3);
    }
  }, [current, follow, isPlaying]);

  const toggle = (k: EventCategory) =>
    setShown((prev) => {
      const next = new Set(prev);
      if (next.has(k)) next.delete(k);
      else next.add(k);
      return next;
    });

  const label: Record<EventCategory, string> = {
    note: t.events.notes,
    control: t.events.controllers,
    bend: t.events.bends,
    program: t.events.programs,
    sysex: t.events.sysex,
    meta: t.events.meta,
  };

  const first = Math.max(0, Math.floor(scrollTop / ROW_H) - OVERSCAN);
  const last = Math.min(rows.length, Math.ceil((scrollTop + VIEW_H) / ROW_H) + OVERSCAN);

  return (
    <div className="events-view">
      <div className="events-toolbar">
        {CATEGORIES.map((k) => (
          <button
            key={k}
            className={`toggle-chip${shown.has(k) ? " on" : ""}`}
            onClick={() => toggle(k)}
            disabled={counts[k] === 0}
          >
            {label[k]} <span className="events-count">{counts[k].toLocaleString()}</span>
          </button>
        ))}
        <select
          className="events-channel"
          value={channel}
          onChange={(e) => setChannel(e.target.value === "all" ? "all" : Number(e.target.value))}
        >
          <option value="all">{t.events.allChannels}</option>
          {channels.map((c) => (
            <option key={c.channel} value={c.channel}>
              {t.events.channel(c.channel + 1, c.name)}
            </option>
          ))}
          <option value={NO_CHANNEL}>{t.events.noChannel}</option>
        </select>
        <div style={{ flex: 1 }} />
        {listing && <span className="events-shown">{t.events.shown(rows.length, listing.rows.length)}</span>}
        <button
          className={`toggle-chip${follow ? " on" : ""}`}
          onClick={() => setFollow((f) => !f)}
          title={t.events.followTitle}
        >
          {t.events.follow}
        </button>
      </div>

      <div
        ref={scrollRef}
        className="events-table"
        style={{ height: VIEW_H }}
        onScroll={(e) => setScrollTop(e.currentTarget.scrollTop)}
      >
        <div className="events-row events-head">
          <span>{t.events.position}</span>
          <span>{t.events.time}</span>
          <span>Ch</span>
          <span>{t.events.type}</span>
          <span>{t.events.detail}</span>
        </div>
        {!listing && <div className="events-empty">{error ? t.events.failed(error) : t.events.loading}</div>}
        {listing && rows.length === 0 && <div className="events-empty">{t.events.empty}</div>}
        <div style={{ height: rows.length * ROW_H, position: "relative" }}>
          {rows.slice(first, last).map((r, i) => {
            const index = first + i;
            const at = beatPosition(bars, r.beat);
            return (
              <div
                key={index}
                className={`events-row is-${r.category}${index === current ? " current" : ""}`}
                style={{ top: index * ROW_H }}
                onClick={() => onSeekTime(r.time)}
                title={t.events.track(r.track + 1, listing?.trackNames[r.track] ?? "")}
              >
                <span className="events-mono">
                  {at.bar}:{fmtBeat(at.beat)}
                </span>
                <span className="events-mono">{fmtTime(r.time)}</span>
                <span className="events-ch">
                  {r.channel != null && (
                    <>
                      <i style={{ background: colorOf.get(r.channel) }} />
                      {r.channel + 1}
                    </>
                  )}
                </span>
                <span className="events-type">{r.type}</span>
                <span className="events-detail">{r.detail}</span>
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}

function fmtBeat(beat: number): string {
  return (Math.round(beat * 100) / 100).toFixed(2);
}

function fmtTime(seconds: number): string {
  const m = Math.floor(seconds / 60);
  const s = seconds - m * 60;
  return `${m}:${s.toFixed(2).padStart(5, "0")}`;
}
