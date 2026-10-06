import { useEffect, useState } from "react";
import { useI18n } from "@/lib/i18n";
import type { PasteRequest, RollClip } from "@/lib/songsApi";

/**
 * Where a copied range goes: the bar and beat, which track each copied track
 * lands on (a new one by default when it comes from another song), and
 * whether it replaces what is there.
 */
export default function PasteDialog({
  clip,
  songId,
  tracks,
  defaultBar,
  barCount,
  staged,
  toBeat,
  onPaste,
  onClose,
}: {
  clip: RollClip;
  songId: number;
  /** The destination's tracks. */
  tracks: { index: number; name: string }[];
  /** Bar at the playhead. */
  defaultBar: number;
  barCount: number;
  /** Claude has edits waiting on this song. */
  staged: boolean;
  /** Bar and beat as shown, to quarter notes from the start. */
  toBeat: (bar: number, beat: number) => number;
  onPaste: (request: PasteRequest) => Promise<boolean>;
  onClose: () => void;
}) {
  const { t } = useI18n();
  const sameSong = clip.songId === songId;
  const [bar, setBar] = useState(defaultBar);
  const [beat, setBeat] = useState(clip.start.beat);
  const [targets, setTargets] = useState<(number | null)[]>(() =>
    // Within one song a track pastes onto itself; from another, onto a new one.
    clip.tracks.map((tr) => (sameSong && tracks.some((d) => d.index === tr.index) ? tr.index : null))
  );
  const [transpose, setTranspose] = useState(0);
  const [merge, setMerge] = useState(false);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && !busy && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose, busy]);

  const range =
    clip.start.bar === clip.end.bar
      ? t.selection.withinBar(clip.start.bar, fmt(clip.start.beat), fmt(clip.end.beat))
      : t.selection.acrossBars(clip.start.bar, fmt(clip.start.beat), clip.end.bar, fmt(clip.end.beat));

  const submit = async () => {
    setBusy(true);
    const ok = await onPaste({
      fromId: clip.songId,
      fromStaged: clip.staged,
      startBeat: clip.startBeat,
      endBeat: clip.endBeat,
      lo: clip.lo,
      hi: clip.hi,
      parts: clip.tracks.map((tr, i) => ({ from: tr.index, to: targets[i] })),
      toId: songId,
      atBeat: toBeat(Math.max(1, Math.round(bar)), Math.max(1, beat)),
      atBar: Math.max(1, Math.round(bar)),
      transpose,
      merge,
    });
    setBusy(false);
    if (ok) onClose();
  };

  return (
    <div className="modal-backdrop" onClick={() => !busy && onClose()}>
      <div className="modal paste-modal" role="dialog" aria-label={t.paste.title} onClick={(e) => e.stopPropagation()}>
        <div className="modal-head">
          <h2>{t.paste.title}</h2>
        </div>
        <p className="paste-source">
          {t.paste.from(clip.songTitle)} · {range} · {t.selection.notes(clip.noteCount)}
        </p>

        <div className="paste-grid">
          <span className="export-label">{t.paste.position}</span>
          <div className="paste-position">
            <label>
              {t.paste.bar}
              <input
                type="number"
                min={1}
                max={barCount + 64}
                value={bar}
                onChange={(e) => setBar(Number(e.target.value) || 1)}
                autoFocus
              />
            </label>
            <label>
              {t.paste.beat}
              <input
                type="number"
                min={1}
                step={0.5}
                value={beat}
                onChange={(e) => setBeat(Number(e.target.value) || 1)}
              />
            </label>
          </div>

          <span className="export-label">{t.paste.tracks}</span>
          <ul className="paste-tracks">
            {clip.tracks.map((tr, i) => (
              <li key={tr.index}>
                <span className="paste-from">{tr.name}</span>
                <span className="paste-arrow">→</span>
                <select
                  value={targets[i] ?? "new"}
                  onChange={(e) =>
                    setTargets((prev) =>
                      prev.map((v, k) => (k === i ? (e.target.value === "new" ? null : Number(e.target.value)) : v))
                    )
                  }
                >
                  <option value="new">{t.paste.newTrack}</option>
                  {tracks.map((d) => (
                    <option key={d.index} value={d.index}>
                      {d.index}: {d.name}
                    </option>
                  ))}
                </select>
              </li>
            ))}
          </ul>

          <span className="export-label">{t.paste.transpose}</span>
          <label className="paste-transpose">
            <input
              type="number"
              min={-24}
              max={24}
              value={transpose}
              onChange={(e) => setTranspose(Math.max(-24, Math.min(24, Math.round(Number(e.target.value) || 0))))}
            />
            {t.paste.semitones}
          </label>
        </div>

        <label className="modal-toggle">
          <input type="checkbox" checked={merge} onChange={(e) => setMerge(e.target.checked)} />
          {t.paste.merge}
        </label>
        {staged && <p className="export-note">{t.paste.staged}</p>}

        <div className="export-actions">
          <button className="btn-quiet" onClick={onClose} disabled={busy}>
            {t.paste.cancel}
          </button>
          <button className="btn-filled" onClick={() => void submit()} disabled={busy}>
            {t.paste.paste}
          </button>
        </div>
      </div>
    </div>
  );
}

function fmt(n: number): string {
  return String(Math.round(n * 100) / 100);
}
