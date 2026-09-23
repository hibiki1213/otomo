import { useEffect, useState } from "react";
import {
  setSpatial,
  spatialOptions,
  type OutputMode,
  type SpatialConfig,
  type VenueInfo,
} from "@/lib/rustEngine";

const STORAGE_KEY = "midiss.spatial";
const SEATS = ["前方", "中央", "後方"];
const OUTPUTS: { id: OutputMode; label: string; hint: string }[] = [
  { id: "speakers", label: "スピーカー", hint: "スピーカー向けのステレオ" },
  {
    id: "headphones",
    label: "ヘッドホン",
    hint: "バイノーラル：頭の外、ステージの位置から聞こえるように再現",
  },
];

function loadSaved(): SpatialConfig | null {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    return raw ? (JSON.parse(raw) as SpatialConfig) : null;
  } catch {
    return null;
  }
}

function save(config: SpatialConfig) {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(config));
  } catch {
    /* ignore */
  }
}

/** Venue / seat / output selector for the native engine. */
export default function AcousticsBar({ onOpenLibrary }: { onOpenLibrary: () => void }) {
  const [venues, setVenues] = useState<VenueInfo[]>([]);
  const [config, setConfig] = useState<SpatialConfig | null>(null);
  const [building, setBuilding] = useState(false);

  const apply = async (next: SpatialConfig) => {
    setConfig(next);
    save(next);
    setBuilding(true);
    try {
      await setSpatial(next);
    } catch (e) {
      console.error("spatial", e);
    } finally {
      setBuilding(false);
    }
  };

  useEffect(() => {
    let cancelled = false;
    spatialOptions()
      .then(({ venues, current }) => {
        if (cancelled) return;
        setVenues(venues);
        const saved = loadSaved();
        const valid =
          saved &&
          (saved.room === "original" ||
            saved.room === "dry" ||
            venues.some((v) => v.id === saved.room));
        if (valid && JSON.stringify(saved) !== JSON.stringify(current)) {
          void apply(saved);
        } else {
          setConfig(current);
        }
      })
      .catch((e) => console.error("spatial options", e));
    return () => {
      cancelled = true;
    };
  }, []);

  if (!config) return null;
  const isVenue = config.room !== "original" && config.room !== "dry";

  return (
    <div className="acoustics">
      <label className="acoustics-field">
        <span className="acoustics-label">会場</span>
        <select
          value={config.room}
          onChange={(e) => apply({ ...config, room: e.target.value })}
        >
          <option value="original">オリジナル（MIDIの設定どおり）</option>
          <option value="dry">残響なし</option>
          {venues.map((v) => (
            <option key={v.id} value={v.id}>
              {v.name}（残響 {v.rt60.toFixed(1)} 秒）
            </option>
          ))}
        </select>
      </label>

      <div className="acoustics-field">
        <span className="acoustics-label">座席</span>
        <div className="segmented" aria-disabled={!isVenue}>
          {SEATS.map((label, i) => (
            <button
              key={label}
              className={config.seat === i ? "on" : ""}
              disabled={!isVenue}
              onClick={() => apply({ ...config, seat: i })}
              title={
                isVenue
                  ? "後ろの席ほど響きが多く、音が遠くなります"
                  : "会場を選ぶと座席を選べます"
              }
            >
              {label}
            </button>
          ))}
        </div>
      </div>

      <div className="acoustics-field">
        <span className="acoustics-label">出力</span>
        <div className="segmented">
          {OUTPUTS.map((o) => (
            <button
              key={o.id}
              className={config.output === o.id ? "on" : ""}
              onClick={() => apply({ ...config, output: o.id })}
              title={o.hint}
            >
              {o.label}
            </button>
          ))}
        </div>
      </div>

      {building && <span className="acoustics-status">響きを生成中…</span>}

      <button className="btn-ghost acoustics-library" onClick={onOpenLibrary}>
        音源ライブラリ…
      </button>
    </div>
  );
}
