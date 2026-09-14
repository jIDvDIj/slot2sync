import { useId } from "react";
import { useTranslation } from "react-i18next";

import { useEmulatorCategories } from "../../hooks/useEmulatorCategories";
import { useEmulatorStats } from "../../hooks/useEmulatorStats";
import { useEmulatorSummary } from "../../hooks/useEmulatorSummary";
import { emulatorStatus } from "../../lib/emulatorStatus";
import { formatBytes } from "../../lib/format";
import { progressFraction } from "../../lib/progress";
import { STATUS_PRESENTATION } from "../../lib/statusPresentation";
import { formatRelativeTime } from "../../lib/time";
import type { EmulatorProfile, PendingOp, SyncProgress } from "../../types/ipc";
import { Icon } from "../ui/Icon";
import { ProgressBar } from "../ui/ProgressBar";
import { StatusLabel } from "../ui/StatusLabel";

interface EmulatorTileProps {
  profile: EmulatorProfile;
  running: boolean;
  /** Progress of the running sync, only when it concerns this emulator. */
  progress: SyncProgress | null;
  conflicts: number;
  pendingOps: PendingOp[];
  gameCount: number;
  now: number;
  onOpen: () => void;
}

export function EmulatorTile({
  profile,
  running,
  progress,
  conflicts,
  pendingOps,
  gameCount,
  now,
  onOpen,
}: EmulatorTileProps) {
  const { t } = useTranslation();
  const detailsId = useId();
  const summary = useEmulatorSummary(profile.name);
  const stats = useEmulatorStats(profile.name);
  const categories = useEmulatorCategories(profile.name);
  const status = emulatorStatus({
    running,
    syncing: progress !== null,
    conflicts,
    pendingOps,
    categories,
  });
  const presentation = STATUS_PRESENTATION[status.kind];
  const statusText = t(presentation.labelKey);

  return (
    <button
      type="button"
      className="emulator-tile"
      data-tone={presentation.tone}
      onClick={onOpen}
      aria-label={t("overview.tileLabel", { name: profile.name, status: statusText })}
      aria-describedby={detailsId}
    >
      <span className="emulator-tile-head">
        <span className="emulator-tile-name truncate">{profile.name}</span>
        <StatusLabel
          tone={presentation.tone}
          icon={presentation.icon}
          spinning={status.kind === "syncing"}
        >
          {statusText}
        </StatusLabel>
      </span>

      <span className="emulator-tile-details" id={detailsId}>
        <span className="emulator-tile-facts">
          <span className="emulator-tile-fact">
            <span className="emulator-tile-fact-label">
              <Icon name="laptop" size={13} />
              {t("overview.onThisDevice")}
            </span>
            <span className="emulator-tile-fact-value tabular">
              {summary
                ? t("overview.filesSize", {
                    count: summary.localFiles,
                    size: formatBytes(summary.localBytes),
                  })
                : "–"}
            </span>
          </span>
          <span className="emulator-tile-fact">
            <span className="emulator-tile-fact-label">
              <Icon name="cloud" size={13} />
              {t("overview.remote")}
            </span>
            <span className="emulator-tile-fact-value tabular">
              {summary
                ? t("overview.filesSize", {
                    count: summary.remoteFiles,
                    size: formatBytes(summary.remoteBytes),
                  })
                : "–"}
            </span>
          </span>
        </span>

        {summary && summary.needSync > 0 ? (
          <span className="emulator-tile-warning">
            <Icon name="clock" size={13} />
            {t("overview.outOfDate", { count: summary.needSync })}
          </span>
        ) : null}

        <span className="emulator-tile-foot">
          <span className="truncate">
            {stats?.lastSyncAtMs
              ? t("overview.syncedWhen", {
                  when: formatRelativeTime(t, stats.lastSyncAtMs, now),
                })
              : t("overview.neverSynced")}
          </span>
          {gameCount > 0 ? (
            <span className="tabular">{t("overview.games", { count: gameCount })}</span>
          ) : null}
        </span>
      </span>

      {progress ? (
        <ProgressBar value={progressFraction(progress)} label={statusText} size="thin" />
      ) : null}
    </button>
  );
}

export function AddEmulatorTile({ onClick }: { onClick: () => void }) {
  const { t } = useTranslation();
  return (
    <button type="button" className="emulator-tile emulator-tile-add" onClick={onClick}>
      <Icon name="plus" size={20} />
      <span>{t("nav.addEmulator")}</span>
    </button>
  );
}

export function SkeletonTile() {
  return (
    <div className="emulator-tile emulator-tile-skeleton" aria-hidden="true">
      <span className="skeleton-line skeleton-line-title" />
      <span className="skeleton-line" />
      <span className="skeleton-line skeleton-line-short" />
    </div>
  );
}
