import { useState } from "react";
import { useTranslation } from "react-i18next";

import type { TFunction } from "i18next";

import { currentLocale } from "../i18n";
import { useErrorMessage } from "../lib/errors";
import { formatBytes, formatEta, formatRate } from "../lib/format";
import { openBackupFolder, syncNow } from "../lib/ipc";
import { useTransferRate } from "../hooks/useTransferRate";
import type { SyncState } from "../hooks/useSyncEvents";
import type { SyncProgress, SyncSummary } from "../types/ipc";
import { Button } from "./ui/Button";
import { NoticeBanner } from "./ui/NoticeBanner";

interface Props {
  state: SyncState;
}

/** Gatilhos automáticos do watcher — ganham indicador visual próprio. */
export function autoTriggerLabelKey(
  trigger: string | null,
): "sync.autoPreGame" | "sync.autoPostGame" | null {
  if (trigger === "emulator-start") return "sync.autoPreGame";
  if (trigger === "emulator-stop") return "sync.autoPostGame";
  return null;
}

function formatRelative(t: TFunction, atMs: number): string {
  const seconds = Math.round((Date.now() - atMs) / 1000);
  if (seconds < 10) return t("sync.justNow");
  if (seconds < 60) return t("sync.secondsAgo", { count: seconds });
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return t("sync.minutesAgo", { count: minutes });
  const hours = Math.round(minutes / 60);
  if (hours < 24) return t("sync.hoursAgo", { count: hours });
  return new Date(atMs).toLocaleString(currentLocale());
}

function summaryLine(t: TFunction, summary: SyncSummary): string {
  const parts = [`↑ ${summary.uploaded}`, `↓ ${summary.downloaded}`, `= ${summary.skipped}`];
  if (summary.queued > 0) parts.push(t("sync.queued", { count: summary.queued }));
  if (summary.failed > 0) parts.push(t("sync.failed", { count: summary.failed }));
  return `${parts.join(" · ")} (${(summary.durationMs / 1000).toFixed(1)}s)`;
}

/** Progresso ao vivo: arquivo atual, barra em bytes, velocidade e ETA. */
function LiveProgress({ progress, trigger }: { progress: SyncProgress; trigger: string | null }) {
  const { t } = useTranslation();
  const rate = useTransferRate(progress);
  const hasBytes = progress.bytesTotal > 0;
  const eta = hasBytes ? formatEta(progress.bytesTotal - progress.bytesDone, rate) : null;
  const autoLabelKey = autoTriggerLabelKey(trigger);

  return (
    <div className="sync-live">
      {autoLabelKey ? (
        <span className="sync-auto-indicator" title={t(autoLabelKey)}>
          {t(autoLabelKey)}
        </span>
      ) : null}
      <span className="sync-progress">
        {progress.emulator} · {progress.currentFile} ({progress.completed}/{progress.total})
      </span>
      <progress
        className="sync-bar"
        value={hasBytes ? progress.bytesDone : undefined}
        max={hasBytes ? progress.bytesTotal : undefined}
      />
      {hasBytes ? (
        <span className="sync-bytes muted">
          {formatBytes(progress.bytesDone)} / {formatBytes(progress.bytesTotal)}
          {rate > 0 ? ` · ${formatRate(rate)}` : ""}
          {eta ? ` · ~${eta}` : ""}
        </span>
      ) : null}
    </div>
  );
}

/** Barra de status: último sync, progresso ao vivo e sync manual. */
export function SyncStatus({ state }: Props) {
  const { t } = useTranslation();
  const errorMessage = useErrorMessage();
  const [busy, setBusy] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);
  const syncing = busy || state.phase === "syncing";

  const handleSync = async () => {
    setBusy(true);
    setActionError(null);
    try {
      await syncNow();
    } catch (err) {
      setActionError(errorMessage(err));
    } finally {
      setBusy(false);
    }
  };

  const handleOpenBackups = async () => {
    setActionError(null);
    try {
      await openBackupFolder();
    } catch (err) {
      setActionError(errorMessage(err));
    }
  };

  const backedUp = state.lastSync?.summary.backedUp ?? 0;

  return (
    <section className="sync-status">
      <div className="sync-row">
        <Button variant="primary" onClick={handleSync} disabled={syncing}>
          {syncing ? t("sync.syncing") : t("sync.syncNow")}
        </Button>
        <div className="sync-info">
          {syncing && state.progress ? (
            <LiveProgress progress={state.progress} trigger={state.trigger} />
          ) : state.lastSync ? (
            <span>
              {t("sync.lastSync", { when: formatRelative(t, state.lastSync.atMs) })} ·{" "}
              <span className="muted">{summaryLine(t, state.lastSync.summary)}</span>
            </span>
          ) : (
            <span className="muted">{t("sync.never")}</span>
          )}
        </div>
      </div>
      {backedUp > 0 && state.lastSync ? (
        <NoticeBanner id={`backup-run-${state.lastSync.atMs}`} tone="warning">
          <span>{t("sync.backupBanner", { count: backedUp })}</span>
          <Button variant="secondary" size="sm" onClick={handleOpenBackups}>
            {t("sync.openBackupFolder")}
          </Button>
        </NoticeBanner>
      ) : null}
      {actionError ? <p className="error">{actionError}</p> : null}
      {state.lastError ? (
        <p className="error">
          {t("sync.lastSyncError", {
            emulator: state.lastError.emulator ? ` (${state.lastError.emulator})` : "",
            message: state.lastError.message,
          })}
        </p>
      ) : null}
    </section>
  );
}
