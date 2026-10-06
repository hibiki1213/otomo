import { useEffect, useState } from "react";
import { useI18n } from "@/lib/i18n";
import {
  setSpatial,
  spatialOptions,
  type OutputMode,
  type SpatialConfig,
  type VenueInfo,
} from "@/lib/rustEngine";

const STORAGE_KEY = "otomo.spatial";
const OUTPUTS: OutputMode[] = ["speakers", "headphones"];

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
  const { t, lang } = useI18n();
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

  // Venue names come from the backend in the current language.
  useEffect(() => {
    if (venues.length === 0) return;
    spatialOptions()
      .then(({ venues }) => setVenues(venues))
      .catch((e) => console.error("spatial options", e));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [lang]);

  if (!config) return null;
  const isVenue = config.room !== "original" && config.room !== "dry";

  return (
    <div className="acoustics">
      <label className="acoustics-field">
        <span className="acoustics-label">{t.acoustics.venue}</span>
        <select
          value={config.room}
          onChange={(e) => apply({ ...config, room: e.target.value })}
        >
          <option value="original">{t.acoustics.original}</option>
          <option value="dry">{t.acoustics.dry}</option>
          {venues.map((v) => (
            <option key={v.id} value={v.id}>
              {t.acoustics.venueOption(v.name, v.rt60.toFixed(1))}
            </option>
          ))}
        </select>
      </label>

      <div className="acoustics-field">
        <span className="acoustics-label">{t.acoustics.seat}</span>
        <div className="segmented" aria-disabled={!isVenue}>
          {t.acoustics.seats.map((label, i) => (
            <button
              key={i}
              className={config.seat === i ? "on" : ""}
              disabled={!isVenue}
              onClick={() => apply({ ...config, seat: i })}
              title={
                isVenue ? t.acoustics.seatHint : t.acoustics.seatNeedsVenue
              }
            >
              {label}
            </button>
          ))}
        </div>
      </div>

      <div className="acoustics-field">
        <span className="acoustics-label">{t.acoustics.output}</span>
        <div className="segmented">
          {OUTPUTS.map((o) => (
            <button
              key={o}
              className={config.output === o ? "on" : ""}
              onClick={() => apply({ ...config, output: o })}
              title={o === "speakers" ? t.acoustics.speakersHint : t.acoustics.headphonesHint}
            >
              {o === "speakers" ? t.acoustics.speakers : t.acoustics.headphones}
            </button>
          ))}
        </div>
      </div>

      {building && <span className="acoustics-status">{t.acoustics.building}</span>}

      <button className="btn-ghost acoustics-library" onClick={onOpenLibrary}>
        {t.acoustics.libraries}
      </button>
    </div>
  );
}
