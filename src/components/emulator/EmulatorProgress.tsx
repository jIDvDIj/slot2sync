import { useTranslation } from "react-i18next";

import { formatBytes } from "../../lib/format";
import { progressFraction } from "../../lib/progress";
import type { SyncProgress } from "../../types/ipc";
import { Icon } from "../ui/Icon";
import { ProgressBar } from "../ui/ProgressBar";

import "./Emulator.css";

export function EmulatorProgress({ progress }: { progress: SyncProgress }) {
  const { t } = useTranslation();
  const fraction = progressFraction(progress);
  const hasBytes = progress.bytesTotal > 0;

  const details = [
    hasBytes
      ? t("syncBar.bytesProgress", {
          done: formatBytes(progress.bytesDone),
          total: formatBytes(progress.bytesTotal),
        })
      : t("syncBar.filesProgress", { completed: progress.completed, total: progress.total }),
    fraction !== null
      ? t("emulatorPage.progressPercent", { percent: Math.round(fraction * 100) })
      : null,
  ].filter(Boolean);

  return (
    <section className="emulator-card" aria-label={t("syncBar.progressLabel")}>
      <div className="emulator-progress-row">
        <span className="emulator-progress-heading">
          <Icon name="sync" size={14} className="icon-spin" />
          {t("syncBar.syncingEmulator", { emulator: progress.emulator })}
        </span>
        <span className="emulator-progress-meta">{details.join(" · ")}</span>
      </div>
      <ProgressBar value={fraction} label={t("syncBar.progressLabel")} />
      <span className="emulator-progress-file emulator-mono row-path" title={progress.currentFile}>
        {progress.currentFile}
      </span>
    </section>
  );
}
