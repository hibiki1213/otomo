import { useEffect, useMemo, useRef, useState } from "react";
import { engrave, engraveSupported, type Engraved, type Paper } from "@/lib/engrave";
import { useI18n } from "@/lib/i18n";
import {
  fileStem,
  scoreMusicXml,
  scoreTracks,
  writePdf,
  writeTextFile,
  type ScoreTrack,
} from "@/lib/scoreApi";

type Format = "pdf" | "musicxml";
type Contents = "score" | "parts";
type PartFiles = "separate" | "combined";

export interface ExportTarget {
  id: number;
  title: string;
  /** Export Claude's staged version (what is being previewed). */
  staged: boolean;
}

const PREFS_KEY = "otomo.export";
const PREVIEW_BARS = 24;

interface Prefs {
  format: Format;
  partFiles: PartFiles;
  paper: Paper;
}

function loadPrefs(): Prefs {
  // Letter where it is the norm; A4 everywhere else.
  const letter = /^en-(US|CA)|^es-(MX|US)/.test(navigator.language);
  const fallback: Prefs = { format: "pdf", partFiles: "separate", paper: letter ? "letter" : "a4" };
  try {
    return { ...fallback, ...(JSON.parse(localStorage.getItem(PREFS_KEY) ?? "{}") as Partial<Prefs>) };
  } catch {
    return fallback;
  }
}

function savePrefs(prefs: Prefs) {
  try {
    localStorage.setItem(PREFS_KEY, JSON.stringify(prefs));
  } catch {
    /* only the convenience is lost */
  }
}

/**
 * Sheet music out: the full score or one part per instrument, as PDF or
 * MusicXML. The preview is the same engraving the PDF is made from, so what
 * is shown is what is saved.
 */
export default function ExportDialog({ target, onClose }: { target: ExportTarget; onClose: () => void }) {
  const { t } = useI18n();
  const canEngrave = useMemo(engraveSupported, []);
  const [prefs, setPrefs] = useState<Prefs>(() => {
    const p = loadPrefs();
    return canEngrave ? p : { ...p, format: "musicxml" };
  });
  const [contents, setContents] = useState<Contents>("score");
  // Scores are usually read at concert pitch, parts never are.
  const [transposedScore, setTransposedScore] = useState(false);
  const [transposedParts, setTransposedParts] = useState(true);
  const [tracks, setTracks] = useState<ScoreTrack[] | null>(null);
  const [selected, setSelected] = useState<Set<number>>(new Set());
  const [preview, setPreview] = useState<{ pages: string[]; label: string } | null>(null);
  const [previewBusy, setPreviewBusy] = useState(false);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState<string[] | null>(null);

  const transposed = contents === "parts" ? transposedParts : transposedScore;
  const update = (patch: Partial<Prefs>) =>
    setPrefs((p) => {
      const next = { ...p, ...patch };
      savePrefs(next);
      return next;
    });

  useEffect(() => {
    scoreTracks(target.id, target.staged)
      .then((list) => {
        setTracks(list);
        setSelected(new Set(list.map((tr) => tr.index)));
      })
      .catch((e) => setError(String(e)));
  }, [target.id, target.staged]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && !busy && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose, busy]);

  const chosen = useMemo(
    () => (tracks ?? []).filter((tr) => selected.has(tr.index)),
    [tracks, selected]
  );

  // The preview follows the options, a moment after they settle.
  const previewRun = useRef(0);
  useEffect(() => {
    if (!canEngrave || chosen.length === 0) {
      setPreview(null);
      return;
    }
    const run = ++previewRun.current;
    const timer = window.setTimeout(async () => {
      setPreviewBusy(true);
      try {
        const parts = contents === "parts";
        const shown = parts ? [chosen[0]] : chosen;
        const xml = await scoreMusicXml({
          id: target.id,
          staged: target.staged,
          tracks: shown.map((tr) => tr.index),
          transposed,
          multiRests: parts,
          // Enough for the first pages; a whole symphony would take seconds.
          maxBars: PREVIEW_BARS,
        });
        const engraved = await engrave(xml, { paper: prefs.paper, kind: parts ? "part" : "score" });
        if (run !== previewRun.current) return;
        setPreview({ pages: engraved.pages, label: parts ? t.exporter.previewPart(chosen[0].name) : "" });
        setError(null);
      } catch (e) {
        if (run === previewRun.current) setError(t.exporter.failed(String(e)));
      } finally {
        if (run === previewRun.current) setPreviewBusy(false);
      }
    }, 250);
    return () => window.clearTimeout(timer);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [chosen, contents, transposed, prefs.paper, canEngrave, target.id, target.staged]);

  const request = (trackList: ScoreTrack[], parts: boolean) =>
    scoreMusicXml({
      id: target.id,
      staged: target.staged,
      tracks: trackList.map((tr) => tr.index),
      transposed,
      multiRests: parts,
    });

  const run = async () => {
    const { save, open } = await import("@tauri-apps/plugin-dialog");
    const title = fileStem(target.title);
    const ext = prefs.format === "pdf" ? "pdf" : "musicxml";
    const filters = [{ name: prefs.format === "pdf" ? "PDF" : "MusicXML", extensions: [ext] }];
    const oneFile = contents === "score" || (prefs.format === "pdf" && prefs.partFiles === "combined");

    let destination: string | null;
    if (oneFile) {
      const name = contents === "score" ? title : `${title} (${t.exporter.partsSuffix})`;
      destination = await save({ defaultPath: `${name}.${ext}`, filters });
    } else {
      const picked = await open({ directory: true, multiple: false, title: t.exporter.chooseFolder });
      destination = typeof picked === "string" ? picked : null;
    }
    if (!destination) return;

    setError(null);
    setSaved(null);
    const written: string[] = [];
    try {
      if (contents === "score") {
        setBusy(t.exporter.working(1, 1));
        const xml = await request(chosen, false);
        if (prefs.format === "pdf") {
          await writePdf(destination, await engrave(xml, { paper: prefs.paper, kind: "score" }));
        } else {
          await writeTextFile(destination, xml);
        }
        written.push(destination);
      } else if (oneFile) {
        // Every part, one after another, each from a fresh page.
        const pages: string[] = [];
        let size: Engraved | undefined;
        for (const [n, tr] of chosen.entries()) {
          setBusy(t.exporter.working(n + 1, chosen.length));
          const part = await engrave(await request([tr], true), { paper: prefs.paper, kind: "part" });
          pages.push(...part.pages);
          size ??= part;
        }
        if (size) await writePdf(destination, { ...size, pages });
        written.push(destination);
      } else {
        const names = new Map<string, number>();
        for (const [n, tr] of chosen.entries()) {
          setBusy(t.exporter.working(n + 1, chosen.length));
          // Two tracks with one name ("Violin", "Violin") get numbered.
          const base = fileStem(`${title} - ${tr.name}`);
          const seen = names.get(base) ?? 0;
          names.set(base, seen + 1);
          const path = `${destination}/${seen ? `${base} ${seen + 1}` : base}.${ext}`;
          const xml = await request([tr], true);
          if (prefs.format === "pdf") {
            await writePdf(path, await engrave(xml, { paper: prefs.paper, kind: "part" }));
          } else {
            await writeTextFile(path, xml);
          }
          written.push(path);
        }
      }
      setSaved(written);
    } catch (e) {
      setError(t.exporter.failed(String(e)));
    } finally {
      setBusy(null);
    }
  };

  const reveal = async () => {
    if (!saved?.length) return;
    const { revealItemInDir } = await import("@tauri-apps/plugin-opener");
    await revealItemInDir(saved[0]).catch(() => undefined);
  };

  const toggle = (index: number) =>
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(index)) next.delete(index);
      else next.add(index);
      return next;
    });

  return (
    <div className="modal-backdrop" onClick={() => !busy && onClose()}>
      <div
        className="modal export-modal"
        role="dialog"
        aria-label={t.exporter.title}
        onClick={(e) => e.stopPropagation()}
      >
        <div className="export-options">
          <div className="modal-head">
            <h2>{t.exporter.title}</h2>
          </div>
          <p className="export-song">{target.title}</p>
          {target.staged && <p className="export-note">{t.exporter.staged}</p>}

          <div className="export-field">
            <span className="export-label">{t.exporter.format}</span>
            <div className="segmented">
              <button
                className={prefs.format === "pdf" ? "on" : ""}
                disabled={!canEngrave}
                onClick={() => update({ format: "pdf" })}
              >
                PDF
              </button>
              <button
                className={prefs.format === "musicxml" ? "on" : ""}
                onClick={() => update({ format: "musicxml" })}
              >
                MusicXML
              </button>
            </div>
            {prefs.format === "musicxml" && <p className="export-note">{t.exporter.musicxmlHint}</p>}
            {!canEngrave && <p className="export-note">{t.exporter.noEngraving}</p>}
          </div>

          <div className="export-field">
            <span className="export-label">{t.exporter.contents}</span>
            <div className="segmented">
              <button className={contents === "score" ? "on" : ""} onClick={() => setContents("score")}>
                {t.exporter.score}
              </button>
              <button className={contents === "parts" ? "on" : ""} onClick={() => setContents("parts")}>
                {t.exporter.parts}
              </button>
            </div>
            {contents === "parts" && prefs.format === "pdf" && (
              <div className="export-radios">
                {(["separate", "combined"] as const).map((v) => (
                  <label key={v}>
                    <input
                      type="radio"
                      name="part-files"
                      checked={prefs.partFiles === v}
                      onChange={() => update({ partFiles: v })}
                    />
                    {v === "separate" ? t.exporter.separate : t.exporter.combined}
                  </label>
                ))}
              </div>
            )}
          </div>

          <div className="export-field export-tracks-field">
            <div className="export-label-row">
              <span className="export-label">{t.exporter.instruments}</span>
              <button className="link-btn" onClick={() => setSelected(new Set((tracks ?? []).map((tr) => tr.index)))}>
                {t.exporter.all}
              </button>
              <button className="link-btn" onClick={() => setSelected(new Set())}>
                {t.exporter.none}
              </button>
            </div>
            <ul className="export-tracks">
              {(tracks ?? []).map((tr) => (
                <li key={tr.index}>
                  <label>
                    <input type="checkbox" checked={selected.has(tr.index)} onChange={() => toggle(tr.index)} />
                    <span className="export-track-name">{tr.name}</span>
                    {tr.name !== tr.instrument && (
                      <span className="export-track-inst">{tr.percussion ? t.midi.drums : tr.instrument}</span>
                    )}
                  </label>
                </li>
              ))}
            </ul>
          </div>

          <label className="modal-toggle export-transpose">
            <input
              type="checkbox"
              checked={transposed}
              onChange={(e) =>
                contents === "parts"
                  ? setTransposedParts(e.target.checked)
                  : setTransposedScore(e.target.checked)
              }
            />
            <span>
              {t.exporter.transposed}
              <span className="export-note">{t.exporter.transposedHint}</span>
            </span>
          </label>

          {prefs.format === "pdf" && (
            <div className="export-field">
              <span className="export-label">{t.exporter.paper}</span>
              <div className="segmented">
                {(["a4", "letter"] as const).map((p) => (
                  <button key={p} className={prefs.paper === p ? "on" : ""} onClick={() => update({ paper: p })}>
                    {p === "a4" ? t.exporter.a4 : t.exporter.letter}
                  </button>
                ))}
              </div>
            </div>
          )}

          {error && <div className="error export-error">{error}</div>}
          {saved && (
            <div className="export-saved">
              {t.exporter.done(saved.length)}
              <button className="link-btn" onClick={() => void reveal()}>
                {t.exporter.reveal}
              </button>
            </div>
          )}

          <div className="export-actions">
            <button className="btn-quiet" onClick={onClose} disabled={!!busy}>
              {saved ? t.common.close : t.exporter.cancel}
            </button>
            <button
              className="btn-filled"
              disabled={!!busy || chosen.length === 0 || !tracks}
              onClick={() => void run()}
            >
              {busy ?? t.exporter.export}
            </button>
          </div>
        </div>

        <div className="export-preview">
          <div className="export-preview-head">
            <span>{t.exporter.preview}</span>
            {preview?.label && <span className="export-preview-label">{preview.label}</span>}
            {preview && <span className="export-preview-count">{t.exporter.openingOnly}</span>}
            {previewBusy && <span className="export-preview-busy">{t.exporter.engraving}</span>}
          </div>
          <div className={`export-pages${previewBusy ? " is-stale" : ""}`}>
            {chosen.length === 0 && <p className="export-empty">{t.exporter.nothingSelected}</p>}
            {!canEngrave && <p className="export-empty">{t.exporter.noEngraving}</p>}
            {preview?.pages.map((svg, i) => <PageImage key={i} svg={svg} paper={prefs.paper} />)}
          </div>
        </div>
      </div>
    </div>
  );
}

/** One engraved page, shown as a sheet of paper. */
function PageImage({ svg, paper }: { svg: string; paper: Paper }) {
  const url = useMemo(() => URL.createObjectURL(new Blob([svg], { type: "image/svg+xml" })), [svg]);
  useEffect(() => () => URL.revokeObjectURL(url), [url]);
  return <img className={`export-page is-${paper}`} src={url} alt="" />;
}
