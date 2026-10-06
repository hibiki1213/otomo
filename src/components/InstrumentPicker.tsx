import {
  addUserInstrument,
  choiceKey,
  openPluginEditor,
  pickSfzFile,
} from "@/lib/instrumentsApi";
import { useI18n } from "@/lib/i18n";
import type { TrackInstruments } from "@/lib/useTrackInstruments";

const SF2_VALUE = "sf2";
const PICK_VALUE = "pick-sfz";

/** Sound source selector for one track (native engine). */
export default function InstrumentPicker({
  track,
  trackName,
  instruments,
}: {
  track: number;
  trackName: string;
  instruments: TrackInstruments;
}) {
  const { t } = useI18n();
  const state = instruments.states[track];
  if (!state) return null;
  const value = choiceKey(state.choice);
  const known = instruments.installed.some((i) => choiceKey(i.choice) === value);

  const onChange = async (v: string) => {
    if (v === SF2_VALUE) {
      void instruments.apply(track, { kind: "sf2" }, "");
    } else if (v === PICK_VALUE) {
      const path = await pickSfzFile();
      if (!path) return;
      try {
        // Kept in the user library, so it is offered again and Claude can use it.
        const entry = await addUserInstrument(path);
        void instruments.refreshCatalog();
        void instruments.apply(track, entry.choice, entry.name);
      } catch (e) {
        console.error("add instrument", e);
        const name = path.split("/").pop()?.replace(/\.sfz$/i, "") ?? path;
        void instruments.apply(track, { kind: "sfz", path }, name);
      }
    } else if (v.startsWith("au:")) {
      const id = v.slice(3);
      const p = instruments.plugins.find((x) => x.id === id);
      void instruments.apply(track, { kind: "audioUnit", id }, p?.name ?? id);
    } else {
      const entry = instruments.installed.find((i) => choiceKey(i.choice) === v);
      if (entry) void instruments.apply(track, entry.choice, entry.name);
    }
  };

  // Group installed instruments by library.
  const libraries = new Map<string, typeof instruments.installed>();
  for (const i of instruments.installed) {
    libraries.set(i.libraryName, [...(libraries.get(i.libraryName) ?? []), i]);
  }

  return (
    <div className="instrument-picker">
      <select
        value={value}
        disabled={state.loading}
        onChange={(e) => void onChange(e.target.value)}
        aria-label={t.picker.label}
        title={t.picker.title}
      >
        <option value={SF2_VALUE}>{t.picker.soundfont}</option>
        {[...libraries].map(([lib, list]) => (
          <optgroup key={lib} label={lib}>
            {list.map((i) => (
              <option key={choiceKey(i.choice)} value={choiceKey(i.choice)}>
                {i.name}
              </option>
            ))}
          </optgroup>
        ))}
        {state.choice.kind === "sfz" && !known && (
          <option value={value}>{t.picker.sfzFile(state.label)}</option>
        )}
        {instruments.plugins.length > 0 && (
          <optgroup label={t.picker.plugins}>
            {instruments.plugins.map((p) => (
              <option key={p.id} value={`au:${p.id}`}>
                {p.manufacturer ? `${p.manufacturer} – ` : ""}
                {p.name}
              </option>
            ))}
          </optgroup>
        )}
        <option value={PICK_VALUE}>{t.picker.pickSfz}</option>
      </select>
      {state.choice.kind === "audioUnit" && !state.loading && (
        <button
          className="picker-btn"
          onClick={() =>
            void openPluginEditor(track, `${state.label} — ${trackName}`).catch(
              (e) => console.error(e)
            )
          }
          title={t.picker.editorTitle}
        >
          {t.picker.editor}
        </button>
      )}
      {state.loading && <span className="picker-status">{t.common.loading}</span>}
      {state.error && (
        <span className="picker-error" title={state.error}>
          {t.picker.failed}
        </span>
      )}
    </div>
  );
}
