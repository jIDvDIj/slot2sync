import { useState } from "react";
import { useTranslation } from "react-i18next";

import { usePlatform } from "../../hooks/usePlatform";
import { useErrorMessage } from "../../lib/errors";
import { formatBytes } from "../../lib/format";
import { resolveConflict, revealBackupPath } from "../../lib/ipc";
import { formatDateTime } from "../../lib/time";
import type { Conflict, ConflictResolution } from "../../types/ipc";
import { Banner } from "../ui/Banner";
import { Button } from "../ui/Button";
import { InlineStatus } from "../ui/Form";
import { Icon } from "../ui/Icon";
import { StatusLabel } from "../ui/StatusLabel";
import { CATEGORY_LABEL } from "./categoryLabels";

import "./Emulator.css";

interface ConflictSectionProps {
  emulator: string;
  conflicts: Conflict[];
  onResolved: () => void;
}

function conflictKey(conflict: Conflict): string {
  return `${conflict.category}/${conflict.relPath}`;
}

export function ConflictSection({ emulator, conflicts, onResolved }: ConflictSectionProps) {
  const { t } = useTranslation();
  const errorMessage = useErrorMessage();
  const [busy, setBusy] = useState<{ key: string; keep: ConflictResolution } | null>(null);
  const [error, setError] = useState<string | null>(null);

  const resolve = async (conflict: Conflict, keep: ConflictResolution) => {
    setBusy({ key: conflictKey(conflict), keep });
    setError(null);
    try {
      await resolveConflict(conflict.emulator, conflict.category, conflict.relPath, keep);
      onResolved();
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setBusy(null);
    }
  };

  const showCopy = async (path: string) => {
    setError(null);
    try {
      await revealBackupPath(path);
    } catch (err) {
      setError(errorMessage(err));
    }
  };

  return (
    <section className="conflict-section">
      <Banner tone="danger" title={t("conflict.bannerTitle", { count: conflicts.length })}>
        {t("conflict.bannerBody", { emulator })}
      </Banner>
      {conflicts.map((conflict) => {
        const key = conflictKey(conflict);
        return (
          <ConflictCard
            key={key}
            conflict={conflict}
            busyKeep={busy?.key === key ? busy.keep : null}
            disabled={busy !== null}
            onKeep={(keep) => void resolve(conflict, keep)}
            onShowCopy={(path) => void showCopy(path)}
          />
        );
      })}
      {error ? <InlineStatus tone="danger">{error}</InlineStatus> : null}
    </section>
  );
}

interface ConflictCardProps {
  conflict: Conflict;
  busyKeep: ConflictResolution | null;
  disabled: boolean;
  onKeep: (keep: ConflictResolution) => void;
  onShowCopy: (path: string) => void;
}

function ConflictCard({ conflict, busyKeep, disabled, onKeep, onShowCopy }: ConflictCardProps) {
  const { t } = useTranslation();
  const { isMobile } = usePlatform();
  const localNewer = conflict.localMtimeMs > conflict.remoteMtimeMs;
  const remoteNewer = conflict.remoteMtimeMs > conflict.localMtimeMs;

  return (
    <article className="emulator-card">
      <div className="conflict-card-head">
        <span className="conflict-path emulator-mono row-path" title={conflict.relPath}>
          {conflict.relPath}
        </span>
        <StatusLabel>{t(CATEGORY_LABEL[conflict.category])}</StatusLabel>
      </div>

      <div className="conflict-sides">
        <ConflictSide
          icon="laptop"
          title={t("conflict.thisDevice")}
          device={conflict.localDevice}
          mtimeMs={conflict.localMtimeMs}
          size={conflict.localSize}
          newer={localNewer}
          busy={busyKeep === "local"}
          disabled={disabled}
          actionLabel={t("conflict.keepLocalLabel", { file: conflict.relPath })}
          onKeep={() => onKeep("local")}
        />
        <ConflictSide
          icon="cloud"
          title={t("conflict.remote")}
          device={conflict.remoteDevice}
          mtimeMs={conflict.remoteMtimeMs}
          size={conflict.remoteSize}
          newer={remoteNewer}
          busy={busyKeep === "remote"}
          disabled={disabled}
          actionLabel={t("conflict.keepRemoteLabel", { file: conflict.relPath })}
          onKeep={() => onKeep("remote")}
        />
      </div>

      {!isMobile && conflict.backupPath ? (
        <div className="conflict-card-foot">
          <Button
            variant="plain"
            size="small"
            icon="folder"
            onClick={() => onShowCopy(conflict.backupPath as string)}
          >
            {t("conflict.showCopy")}
          </Button>
        </div>
      ) : null}
    </article>
  );
}

interface ConflictSideProps {
  icon: "laptop" | "cloud";
  title: string;
  device: string | null;
  mtimeMs: number;
  size: number;
  newer: boolean;
  busy: boolean;
  disabled: boolean;
  actionLabel: string;
  onKeep: () => void;
}

function ConflictSide({
  icon,
  title,
  device,
  mtimeMs,
  size,
  newer,
  busy,
  disabled,
  actionLabel,
  onKeep,
}: ConflictSideProps) {
  const { t } = useTranslation();
  return (
    <div className="conflict-side" data-newer={newer || undefined}>
      <div className="conflict-side-title">
        <Icon name={icon} size={16} />
        <span>{title}</span>
        {newer ? (
          <StatusLabel tone="accent" icon="clock">
            {t("conflict.newer")}
          </StatusLabel>
        ) : null}
      </div>
      <span className="conflict-side-meta">{device ?? t("conflict.unknownDevice")}</span>
      <span className="conflict-side-meta">
        {formatDateTime(mtimeMs)} · {formatBytes(size)}
      </span>
      <Button
        variant="bordered"
        loading={busy}
        disabled={disabled}
        aria-label={actionLabel}
        onClick={onKeep}
      >
        {t("conflict.keepThis")}
      </Button>
    </div>
  );
}
