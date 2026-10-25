import { useEffect, type ReactNode } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import ChatMarkdown from "@/components/ChatMarkdown";
import { useI18n } from "@/lib/i18n";
import { closeUpdateDialog, installUpdate, releaseUrl, restartToUpdate, useUpdater } from "@/lib/updater";

const mb = (bytes: number) => (bytes / 1_000_000).toFixed(1);

/**
 * The update, from "a new version is available" to "restart to update".
 * `busy` is whether a restart now would cut something off (playback, a
 * Claude run, an edit waiting for a decision); then restarting is offered
 * but not pressed, since the new version also starts on the next launch.
 */
export default function UpdateDialog({ busy }: { busy: boolean }) {
  const { t } = useI18n();
  const { status, open } = useUpdater();
  const visible = open && status.phase !== "idle" && status.phase !== "checking";

  useEffect(() => {
    if (!visible) return;
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && closeUpdateDialog();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [visible]);

  if (!visible) return null;
  const openRelease = (version?: string) => void openUrl(releaseUrl(version)).catch(console.error);

  let title: string;
  let body: ReactNode;
  let actions: ReactNode;
  switch (status.phase) {
    case "available":
      title = t.updates.availableTitle;
      body = (
        <>
          <p className="modal-lead">{t.updates.available(status.version, status.current)}</p>
          {status.notes.trim() && (
            <div className="update-notes">
              <ChatMarkdown text={status.notes} />
            </div>
          )}
          <button className="link-btn update-link" onClick={() => openRelease(status.version)}>
            {t.updates.viewRelease}
          </button>
        </>
      );
      actions = (
        <>
          <button className="btn-ghost" onClick={closeUpdateDialog}>
            {t.updates.later}
          </button>
          <button className="btn-filled" onClick={() => void installUpdate()}>
            {t.updates.update}
          </button>
        </>
      );
      break;
    case "downloading": {
      const percent = status.total ? Math.min(100, (status.received / status.total) * 100) : null;
      title = t.updates.downloadingTitle;
      body = (
        <>
          <div
            className={`update-progress${percent == null ? " is-indeterminate" : ""}`}
            role="progressbar"
            aria-valuemin={0}
            aria-valuemax={100}
            aria-valuenow={percent ?? undefined}
          >
            <span style={percent == null ? undefined : { width: `${percent}%` }} />
          </div>
          <p className="modal-foot">
            {t.updates.downloaded(mb(status.received), status.total ? mb(status.total) : null)}
          </p>
        </>
      );
      actions = (
        <button className="btn-ghost" onClick={closeUpdateDialog}>
          {t.updates.hideWhileDownloading}
        </button>
      );
      break;
    }
    case "ready":
      title = t.updates.readyTitle;
      body = (
        <>
          <p className="modal-lead">{t.updates.ready(status.version)}</p>
          {busy && <p className="modal-foot">{t.updates.busy}</p>}
        </>
      );
      actions = busy ? (
        <>
          <button className="btn-ghost" onClick={() => void restartToUpdate()}>
            {t.updates.restart}
          </button>
          <button className="btn-filled" onClick={closeUpdateDialog}>
            {t.updates.later}
          </button>
        </>
      ) : (
        <>
          <button className="btn-ghost" onClick={closeUpdateDialog}>
            {t.updates.later}
          </button>
          <button className="btn-filled" onClick={() => void restartToUpdate()}>
            {t.updates.restart}
          </button>
        </>
      );
      break;
    case "failed":
      title = status.blocked ? t.updates.blockedTitle : t.updates.failedTitle;
      body = (
        <>
          <p className="modal-lead">{status.blocked ? t.updates.blocked : t.updates.failed}</p>
          {status.message && <p className="update-error">{status.message}</p>}
          {!status.blocked && (
            <button className="link-btn update-link" onClick={() => openRelease(status.version)}>
              {t.updates.openDownload}
            </button>
          )}
        </>
      );
      actions = (
        <button className="btn-filled" onClick={closeUpdateDialog}>
          {t.common.close}
        </button>
      );
      break;
  }

  return (
    <div className="modal-backdrop" onClick={closeUpdateDialog}>
      <div className="modal update-modal" role="dialog" aria-label={title} onClick={(e) => e.stopPropagation()}>
        <div className="modal-head">
          <h2>{title}</h2>
        </div>
        {body}
        <div className="update-actions">{actions}</div>
      </div>
    </div>
  );
}
