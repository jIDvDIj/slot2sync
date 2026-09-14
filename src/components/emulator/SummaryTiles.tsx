import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";

import { useEmulatorStats } from "../../hooks/useEmulatorStats";
import { useEmulatorSummary } from "../../hooks/useEmulatorSummary";
import { useNow } from "../../hooks/useNow";
import { formatBytes } from "../../lib/format";
import { formatDateTime, formatRelativeTime } from "../../lib/time";
import { Icon, type IconName } from "../ui/Icon";

import "./Emulator.css";

const PLACEHOLDER = "—";

export function SummaryTiles({ name }: { name: string }) {
  const { t } = useTranslation();
  const stats = useEmulatorStats(name);
  const summary = useEmulatorSummary(name);
  const now = useNow();

  const lastSyncAtMs = stats?.lastSyncAtMs ?? summary?.lastSyncAtMs ?? null;
  const loaded = stats !== null || summary !== null;
  const outOfDate = summary?.needSync ?? 0;

  return (
    <section aria-label={t("emulatorPage.summaryHeading")}>
      <dl className="summary-grid">
        <Tile
          icon="laptop"
          label={t("emulatorPage.localTitle")}
          value={summary ? t("emulatorPage.files", { count: summary.localFiles }) : PLACEHOLDER}
          detail={summary ? formatBytes(summary.localBytes) : null}
        />
        <Tile
          icon="cloud"
          label={t("emulatorPage.remoteTitle")}
          value={summary ? t("emulatorPage.files", { count: summary.remoteFiles }) : PLACEHOLDER}
          detail={summary ? formatBytes(summary.remoteBytes) : null}
        />
        <Tile
          icon={outOfDate > 0 ? "warning" : "checkCircle"}
          tone={outOfDate > 0 ? "warning" : undefined}
          label={t("emulatorPage.outOfDateTitle")}
          value={summary ? String(outOfDate) : PLACEHOLDER}
          detail={
            summary
              ? outOfDate > 0
                ? t("emulatorPage.outOfDateSome")
                : t("emulatorPage.outOfDateNone")
              : null
          }
        />
        <Tile
          icon="clock"
          label={t("emulatorPage.lastSyncTitle")}
          value={
            lastSyncAtMs !== null
              ? formatRelativeTime(t, lastSyncAtMs, now)
              : loaded
                ? t("emulatorPage.never")
                : PLACEHOLDER
          }
          title={lastSyncAtMs !== null ? formatDateTime(lastSyncAtMs) : undefined}
          detail={
            lastSyncAtMs !== null
              ? formatDateTime(lastSyncAtMs)
              : loaded
                ? t("emulatorPage.neverDetail")
                : null
          }
        />
        <Tile
          icon="sync"
          label={t("emulatorPage.transfersTitle")}
          value={
            stats
              ? t("emulatorPage.transfersValue", {
                  up: stats.totalUploads,
                  down: stats.totalDownloads,
                })
              : PLACEHOLDER
          }
          detail={
            stats
              ? t("emulatorPage.transfersDetail", {
                  up: formatBytes(stats.totalBytesUp),
                  down: formatBytes(stats.totalBytesDown),
                })
              : null
          }
          title={stats?.lastFile ?? undefined}
        />
      </dl>
    </section>
  );
}

interface TileProps {
  icon: IconName;
  label: string;
  value: ReactNode;
  detail: ReactNode;
  tone?: "warning";
  title?: string;
}

function Tile({ icon, label, value, detail, tone, title }: TileProps) {
  return (
    <div className="summary-tile" data-tone={tone} title={title}>
      <dt className="summary-tile-label">
        <Icon name={icon} size={14} />
        {label}
      </dt>
      <dd className="summary-tile-value">{value}</dd>
      {detail ? <dd className="summary-tile-detail">{detail}</dd> : null}
    </div>
  );
}
