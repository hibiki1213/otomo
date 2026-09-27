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
  return (
    <div className="pending-bar">
      <div className="pending-text">
        <strong>Claude が編集しました</strong>
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
          {previewing ? "試聴中" : "試聴"}
        </button>
        <button className="btn-quiet" onClick={() => void library.discardEdit(edit.songId)}>
          破棄
        </button>
        <button className="btn-filled" onClick={() => void library.applyEdit(edit.songId)}>
          適用
        </button>
      </div>
    </div>
  );
}
