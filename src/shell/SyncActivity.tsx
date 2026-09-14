import { useState } from "react";
import { useTranslation } from "react-i18next";

import { Icon, type IconName } from "../components/ui/Icon";
import { ProgressBar } from "../components/ui/ProgressBar";
import { useTransferRate } from "../hooks/useTransferRate";
import { formatBytes, formatEta, formatRate } from "../lib/format";
import { autoTriggerLabelKey, progressFraction } from "../lib/progress";
import type { SyncProgress } from "../types/ipc";

interface SyncActivityProps {
  syncing: boolean;
  progress: SyncProgress | null;
  trigger: string | null;
}

/** Live detail under the toolbar while a sync runs; collapses when it ends. */
export function SyncActivity({ syncing, progress, trigger }: SyncActivityProps) {
  // Keep the last snapshot so the card doesn't empty out while it collapses.
  const [shown, setShown] = useState(progress);
  if (progress && progress !== shown) setShown(progress);

  const open = syncing && progress !== null;

  return (
    <div className="sync-activity" data-open={open || undefined} aria-hidden={!open} inert={!open}>
      <div className="sync-activity-clip">
        {shown ? <SyncActivityCard progress={shown} trigger={trigger} /> : null}
      </div>
    </div>
  );
}

function SyncActivityCard({
  progress,
  trigger,
}: {
  progress: SyncProgress;
  trigger: string | null;
}) {
  const { t } = useTranslation();
  const rate = useTransferRate(progress);
  const fraction = progressFraction(progress);
  const autoKey = autoTriggerLabelKey(trigger);
  const hasBytes = progress.bytesTotal > 0;
  const eta = hasBytes ? formatEta(progress.bytesTotal - progress.bytesDone, rate) : null;

  const icon: IconName =
    trigger === "emulator-start" ? "arrowDown" : trigger === "emulator-stop" ? "arrowUp" : "sync";

  const details = [
    hasBytes
      ? t("syncBar.bytesProgress", {
          done: formatBytes(progress.bytesDone),
          total: formatBytes(progress.bytesTotal),
        })
      : t("syncBar.filesProgress", { completed: progress.completed, total: progress.total }),
    rate > 0 ? formatRate(rate) : null,
    eta ? t("syncBar.timeLeft", { eta }) : null,
  ].filter(Boolean);

  return (
    <section className="sync-activity-card" aria-label={t("syncBar.progressLabel")}>
      <div className="sync-activity-row">
        <span className="sync-activity-heading">
          <Icon name={icon} size={14} className="sync-activity-icon" />
          <span className="truncate">
            {autoKey ? t(autoKey) : t("syncBar.syncingEmulator", { emulator: progress.emulator })}
          </span>
        </span>
        <span className="sync-activity-meta tabular">{details.join(" · ")}</span>
      </div>
      <ProgressBar value={fraction} label={t("syncBar.progressLabel")} size="thin" />
      <span className="sync-activity-file truncate" title={progress.currentFile}>
        {progress.emulator} · {progress.currentFile}
      </span>
    </section>
  );
}
