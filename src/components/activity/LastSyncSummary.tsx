import type { TFunction } from "i18next";
import { useTranslation } from "react-i18next";

import { useNow } from "../../hooks/useNow";
import { formatDuration } from "../../lib/format";
import { formatRelativeTime } from "../../lib/time";
import type { LastSync } from "../../types/ipc";

const TRIGGER_KEYS = {
  manual: "activity.lastSync.triggers.manual",
  startup: "activity.lastSync.triggers.startup",
  shutdown: "activity.lastSync.triggers.shutdown",
  "emulator-start": "activity.lastSync.triggers.emulatorStart",
  "emulator-stop": "activity.lastSync.triggers.emulatorStop",
  scheduled: "activity.lastSync.triggers.scheduled",
  foreground: "activity.lastSync.triggers.foreground",
  background: "activity.lastSync.triggers.background",
  "file-change": "activity.lastSync.triggers.fileChange",
} as const;

function triggerLabel(t: TFunction, trigger: string): string {
  return trigger in TRIGGER_KEYS ? t(TRIGGER_KEYS[trigger as keyof typeof TRIGGER_KEYS]) : trigger;
}

type Tone = "danger" | "warning" | undefined;

export function LastSyncSummary({ lastSync }: { lastSync: LastSync }) {
  const { t } = useTranslation();
  const now = useNow();
  const { summary } = lastSync;

  const tiles: { label: string; value: string | number; tone?: Tone; always?: boolean }[] = [
    { label: t("activity.lastSync.uploaded"), value: summary.uploaded, always: true },
    { label: t("activity.lastSync.downloaded"), value: summary.downloaded, always: true },
    { label: t("activity.lastSync.unchanged"), value: summary.skipped, always: true },
    { label: t("activity.lastSync.failed"), value: summary.failed, tone: "danger" },
    { label: t("activity.lastSync.conflicts"), value: summary.conflicts, tone: "danger" },
    { label: t("activity.lastSync.queued"), value: summary.queued, tone: "warning" },
    { label: t("activity.lastSync.renamed"), value: summary.renamed },
    { label: t("activity.lastSync.backedUp"), value: summary.backedUp },
  ];

  return (
    <section className="form-section" aria-label={t("activity.lastSync.title")}>
      <div className="form-section-header">
        <h2 className="form-section-title">{t("activity.lastSync.title")}</h2>
        <p className="form-section-description">
          {t("activity.lastSync.detail", {
            when: formatRelativeTime(t, lastSync.atMs, now),
            trigger: triggerLabel(t, lastSync.trigger),
          })}
        </p>
      </div>
      <div className="stat-grid">
        {tiles
          .filter((tile) => tile.always || Number(tile.value) > 0)
          .map((tile) => (
            <div key={tile.label} className="stat-tile" data-tone={tile.tone}>
              <span className="stat-value">{tile.value}</span>
              <span className="stat-label">{tile.label}</span>
            </div>
          ))}
        <div className="stat-tile">
          <span className="stat-value">{formatDuration(summary.durationMs)}</span>
          <span className="stat-label">{t("activity.lastSync.duration")}</span>
        </div>
      </div>
      {summary.cancelled > 0 ? (
        <p className="form-section-footer">{t("activity.lastSync.incomplete")}</p>
      ) : null}
    </section>
  );
}
