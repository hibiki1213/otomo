import { useI18n } from "@/lib/i18n";
import type { PendingEdit } from "@/lib/songsApi";
import type { SongLibrary } from "@/lib/useSongLibrary";

/**
 * What Claude changed, and the decision it is waiting on. Edits made over MCP
 * never touch the library on their own: they sit here until someone has heard
 * them and said yes.
 */
export default function PendingBar({
  edit,
  library,
  previewing,
}: {
  edit: PendingEdit;
  library: SongLibrary;
  /** Whether the player is currently sounding the staged version. */
  previewing: boolean;
}) {
  const { t } = useI18n();
  return (
    <div className="pending-bar">
      <div className="pending-text">
        <strong>{t.pending.title}</strong>
        <ul>
          {edit.changes.slice(-4).map((change, i) => (
            <li key={i}>{change.split("\n")[0]}</li>
          ))}
        </ul>
      </div>
      <div className="pending-actions">
        <button
          className="btn-quiet"
          onClick={() => void library.previewPending(edit.songId)}
          disabled={previewing}
        >
          {previewing ? t.common.previewing : t.common.preview}
        </button>
        <button className="btn-quiet" onClick={() => void library.discardEdit(edit.songId)}>
          {t.common.discard}
        </button>
        <button className="btn-filled" onClick={() => void library.applyEdit(edit.songId)}>
          {t.common.apply}
        </button>
      </div>
    </div>
  );
}
