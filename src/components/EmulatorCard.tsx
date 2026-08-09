import { useState } from "react";
import { useTranslation } from "react-i18next";

import { currentLocale } from "../i18n";
import { useEmulatorStats } from "../hooks/useEmulatorStats";
import { useErrorMessage } from "../lib/errors";
import { formatBytes } from "../lib/format";
import type { Conflict, EmulatorProfile, PendingOp, SyncedGame, SyncProgress } from "../types/ipc";
import { ConflictModal } from "./ConflictModal";
import { GameList } from "./GameList";
import { PendingOpsModal } from "./PendingOpsModal";
import { autoTriggerLabelKey } from "./SyncStatus";
import { Badge } from "./ui/Badge";
import { Button } from "./ui/Button";
import { Card } from "./ui/Card";

interface Props {
  profile: EmulatorProfile;
  running: boolean;
  /** Conflitos pendentes deste emulador (bloqueiam o sync dele). */
  conflicts: Conflict[];
  /** Arquivos deste emulador presos na fila offline (retentados a cada sync). */
  pendingOps: PendingOp[];
  /** Progresso do sync em curso (qualquer emulador); o card filtra pelo nome. */
  progress: SyncProgress | null;
  /** Gatilho do sync em curso — tooltip do badge nos syncs automáticos. */
  trigger: string | null;
  /** Jogos sincronizados deste emulador. */
  games: SyncedGame[];
  onRemove: (name: string) => Promise<void>;
  /** Recarrega a lista de conflitos após uma resolução. */
  onConflictResolved: () => void;
}

/**
 * Card de um emulador configurado: nome, pasta, estado (conflito /
 * sincronizando / rodando / parado + pendências), progresso do sync em curso,
 * jogos e remoção.
 */
export function EmulatorCard({
  profile,
  running,
  conflicts,
  pendingOps,
  progress,
  trigger,
  games,
  onRemove,
  onConflictResolved,
}: Props) {
  const { t } = useTranslation();
  const errorMessage = useErrorMessage();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [showConflicts, setShowConflicts] = useState(false);
  const [showPending, setShowPending] = useState(false);
  const [showGames, setShowGames] = useState(false);
  const stats = useEmulatorStats(profile.name);

  const handleRemove = async () => {
    setBusy(true);
    setError(null);
    try {
      await onRemove(profile.name);
    } catch (err) {
      setError(errorMessage(err));
      setBusy(false);
    }
  };

  const hasConflict = conflicts.length > 0;
  const cardProgress = progress?.emulator === profile.name ? progress : null;
  const syncingTitleKey = autoTriggerLabelKey(trigger);
  const hasBytes = (cardProgress?.bytesTotal ?? 0) > 0;
  const pct = cardProgress
    ? Math.round(
        hasBytes
          ? (cardProgress.bytesDone / cardProgress.bytesTotal) * 100
          : cardProgress.total > 0
            ? (cardProgress.completed / cardProgress.total) * 100
            : 0,
      )
    : 0;

  return (
    <Card
      as="article"
      padding="md"
      tone={hasConflict ? "danger-outline" : "default"}
      className={`emulator-card${hasConflict ? " has-conflict" : ""}`}
    >
      <div className="emulator-head">
        <span className="emulator-name">{profile.name}</span>
        <span className="emulator-badges">
          {pendingOps.length > 0 ? (
            <Badge tone="warning" as="button" onClick={() => setShowPending(true)}>
              {t("emulator.pendingBadge", { count: pendingOps.length })}
            </Badge>
          ) : null}
          {hasConflict ? (
            <Badge tone="danger">{t("emulator.conflictBadge")}</Badge>
          ) : cardProgress ? (
            <Badge
              tone="info"
              className="rs-badge-pulse"
              title={syncingTitleKey ? t(syncingTitleKey) : undefined}
            >
              {t("emulator.syncing")}
            </Badge>
          ) : (
            <Badge tone={running ? "success" : "neutral"}>
              {running ? t("emulator.running") : t("emulator.idle")}
            </Badge>
          )}
        </span>
      </div>
      <p className="emulator-path" title={profile.rootPath}>
        {profile.rootPath}
      </p>

      {stats?.lastSyncAtMs ? (
        <p className="muted emulator-stats" title={stats.lastFile ?? undefined}>
          {t("emulator.statsLine", {
            when: new Date(stats.lastSyncAtMs).toLocaleString(currentLocale()),
            up: stats.totalUploads,
            down: stats.totalDownloads,
          })}
        </p>
      ) : null}

      {cardProgress ? (
        <div className="emulator-progress">
          <progress
            className="sync-bar"
            value={hasBytes ? cardProgress.bytesDone : cardProgress.completed}
            max={hasBytes ? cardProgress.bytesTotal : Math.max(cardProgress.total, 1)}
          />
          <span className="emulator-progress-pct">
            {hasBytes
              ? `${formatBytes(cardProgress.bytesDone)} / ${formatBytes(cardProgress.bytesTotal)} · ${pct}%`
              : `${pct}%`}
          </span>
        </div>
      ) : null}

      {games.length > 0 ? (
        <div className="emulator-games">
          <Button
            variant="ghost"
            size="sm"
            className="games-toggle"
            onClick={() => setShowGames((v) => !v)}
          >
            {showGames ? t("emulator.hideGames") : t("emulator.games", { count: games.length })}
          </Button>
          {showGames ? <GameList games={games} /> : null}
        </div>
      ) : null}

      <div className="emulator-foot">
        {hasConflict ? (
          <Button variant="primary" size="sm" onClick={() => setShowConflicts(true)}>
            {t("emulator.resolveConflict", { count: conflicts.length })}
          </Button>
        ) : null}
        <Button variant="secondary" size="sm" onClick={handleRemove} disabled={busy}>
          {busy ? t("emulator.removing") : t("emulator.remove")}
        </Button>
        {error ? <span className="error">{error}</span> : null}
      </div>

      {showConflicts && hasConflict ? (
        <ConflictModal
          emulator={profile.name}
          conflicts={conflicts}
          onClose={() => setShowConflicts(false)}
          onResolved={onConflictResolved}
        />
      ) : null}

      {showPending ? (
        <PendingOpsModal
          emulator={profile.name}
          ops={pendingOps}
          onClose={() => setShowPending(false)}
        />
      ) : null}
    </Card>
  );
}
