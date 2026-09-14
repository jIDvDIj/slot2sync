import { useState } from "react";
import { useTranslation } from "react-i18next";

import { BackupHistoryDialog } from "../components/activity/BackupHistoryDialog";
import { LastSyncSummary } from "../components/activity/LastSyncSummary";
import { LogViewerDialog } from "../components/activity/LogViewerDialog";
import { Button } from "../components/ui/Button";
import { EmptyState } from "../components/ui/EmptyState";
import { FormRow, FormSection, InlineStatus } from "../components/ui/Form";
import { PageHeader } from "../components/ui/PageHeader";
import { StatusLabel } from "../components/ui/StatusLabel";
import { useNow } from "../hooks/useNow";
import { useRecentErrors } from "../hooks/useRecentErrors";
import type { SyncState } from "../hooks/useSyncEvents";
import { useErrorMessage } from "../lib/errors";
import { retryPendingOp } from "../lib/ipc";
import { formatDateTime, formatRelativeTime } from "../lib/time";
import type { Conflict, EmulatorProfile, PendingOp } from "../types/ipc";

import "../components/activity/Activity.css";

export interface ActivityPageProps {
  sync: SyncState;
  emulators: EmulatorProfile[];
  conflicts: Conflict[];
  pendingOps: PendingOp[];
  onPendingChanged: () => void;
  onOpenEmulator: (name: string) => void;
  onSyncNow: () => Promise<void>;
}

function opKey(op: PendingOp): string {
  return `${op.emulator}/${op.category}/${op.relPath}/${op.direction}`;
}

export function ActivityPage({
  sync,
  conflicts,
  pendingOps,
  onPendingChanged,
  onOpenEmulator,
  onSyncNow,
}: ActivityPageProps) {
  const { t } = useTranslation();
  const errorMessage = useErrorMessage();
  const now = useNow();
  const { errors, clear } = useRecentErrors();
  const [historyOpen, setHistoryOpen] = useState(false);
  const [logOpen, setLogOpen] = useState(false);
  const [retrying, setRetrying] = useState<string | null>(null);
  const [retryErrors, setRetryErrors] = useState<Record<string, string>>({});
  const [clearError, setClearError] = useState<string | null>(null);

  const conflictCounts = new Map<string, number>();
  for (const conflict of conflicts) {
    conflictCounts.set(conflict.emulator, (conflictCounts.get(conflict.emulator) ?? 0) + 1);
  }
  const failed = pendingOps.filter((op) => op.nextRetryAtMs === null);
  const waiting = pendingOps.filter((op) => op.nextRetryAtMs !== null);
  const needsAttention = conflictCounts.size > 0 || failed.length > 0;
  const nothingToShow = !needsAttention && errors.length === 0 && waiting.length === 0;

  const direction = (op: PendingOp) =>
    op.direction === "upload" ? t("activity.attention.upload") : t("activity.attention.download");

  const retry = async (op: PendingOp) => {
    const key = opKey(op);
    setRetrying(key);
    setRetryErrors((current) => {
      const next = { ...current };
      delete next[key];
      return next;
    });
    try {
      await retryPendingOp(op.emulator, op.category, op.relPath);
      onPendingChanged();
      await onSyncNow();
    } catch (err) {
      setRetryErrors((current) => ({ ...current, [key]: errorMessage(err) }));
    } finally {
      setRetrying(null);
    }
  };

  const clearAll = async () => {
    setClearError(null);
    try {
      await clear();
    } catch (err) {
      setClearError(errorMessage(err));
    }
  };

  return (
    <div className="activity-page">
      <PageHeader
        title={t("nav.activity")}
        subtitle={
          sync.lastSync
            ? t("syncBar.lastSynced", { when: formatRelativeTime(t, sync.lastSync.atMs, now) })
            : t("syncBar.neverSynced")
        }
      />

      {needsAttention ? (
        <FormSection title={t("activity.attention.title")}>
          {[...conflictCounts].map(([emulator, count]) => (
            <FormRow
              key={`conflict-${emulator}`}
              icon="warning"
              label={emulator}
              description={t("activity.attention.conflicts", { count })}
              control={
                <>
                  <StatusLabel tone="danger" icon="warning">
                    {t("status.conflict")}
                  </StatusLabel>
                  <Button onClick={() => onOpenEmulator(emulator)}>
                    {t("activity.attention.resolve")}
                  </Button>
                </>
              }
            />
          ))}
          {failed.map((op) => {
            const key = opKey(op);
            return (
              <FormRow
                key={key}
                icon="error"
                label={<span className="activity-path">{op.relPath}</span>}
                description={t("activity.attention.failedDetail", {
                  emulator: op.emulator,
                  direction: direction(op),
                  count: op.attempts,
                })}
                control={
                  <Button
                    icon="sync"
                    loading={retrying === key}
                    disabled={retrying !== null}
                    onClick={() => void retry(op)}
                  >
                    {t("activity.attention.retry")}
                  </Button>
                }
              >
                {op.lastError ? <p className="activity-error-message">{op.lastError}</p> : null}
                {retryErrors[key] ? (
                  <InlineStatus tone="danger">{retryErrors[key]}</InlineStatus>
                ) : null}
              </FormRow>
            );
          })}
        </FormSection>
      ) : null}

      {nothingToShow ? (
        <EmptyState
          icon="checkCircle"
          title={t("activity.emptyTitle")}
          message={t("activity.emptyMessage")}
        />
      ) : null}

      {sync.lastSync ? <LastSyncSummary lastSync={sync.lastSync} /> : null}

      {errors.length > 0 ? (
        <FormSection
          title={t("activity.errors.title")}
          footer={
            clearError ? (
              <InlineStatus tone="danger">{clearError}</InlineStatus>
            ) : (
              t("activity.errors.footer")
            )
          }
        >
          {errors.map((entry, index) => (
            <FormRow
              key={`${entry.atMs}-${index}`}
              label={entry.emulator ?? t("activity.errors.general")}
              description={formatDateTime(entry.atMs)}
              control={
                index === 0 ? (
                  <Button variant="plain" onClick={() => void clearAll()}>
                    {t("activity.errors.clear")}
                  </Button>
                ) : undefined
              }
            >
              <p className="activity-error-message">{entry.message}</p>
            </FormRow>
          ))}
        </FormSection>
      ) : null}

      {waiting.length > 0 ? (
        <FormSection title={t("activity.queue.title")} footer={t("activity.queue.footer")}>
          {waiting.map((op) => (
            <FormRow
              key={opKey(op)}
              icon={op.direction === "upload" ? "arrowUp" : "arrowDown"}
              label={<span className="activity-path">{op.relPath}</span>}
              description={t("activity.queue.detail", {
                emulator: op.emulator,
                direction: direction(op),
                count: op.attempts,
              })}
              control={
                <>
                  {op.priority ? (
                    <StatusLabel tone="accent" icon="arrowUp">
                      {t("activity.queue.prioritized")}
                    </StatusLabel>
                  ) : null}
                  {op.nextRetryAtMs ? (
                    <span className="text-callout text-secondary tabular">
                      {t("activity.queue.nextRetry", { when: formatDateTime(op.nextRetryAtMs) })}
                    </span>
                  ) : null}
                </>
              }
            />
          ))}
        </FormSection>
      ) : null}

      <FormSection title={t("activity.tools.title")}>
        <FormRow
          icon="archive"
          label={t("settings.backups.historyLabel")}
          description={t("settings.backups.historyDescription")}
          control={
            <Button onClick={() => setHistoryOpen(true)}>{t("settings.backups.history")}</Button>
          }
        />
        <FormRow
          icon="document"
          label={t("settings.diagnostics.logLabel")}
          description={t("settings.diagnostics.logDescription")}
          control={
            <Button onClick={() => setLogOpen(true)}>{t("settings.diagnostics.showLog")}</Button>
          }
        />
      </FormSection>

      <BackupHistoryDialog open={historyOpen} onClose={() => setHistoryOpen(false)} />
      <LogViewerDialog open={logOpen} onClose={() => setLogOpen(false)} />
    </div>
  );
}
