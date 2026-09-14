import { useState } from "react";
import { useTranslation } from "react-i18next";

import { useNow } from "../../hooks/useNow";
import { useErrorMessage } from "../../lib/errors";
import { bumpPendingOp, retryPendingOp } from "../../lib/ipc";
import { formatDateTime, formatRelativeTime } from "../../lib/time";
import type { PendingOp } from "../../types/ipc";
import { Button } from "../ui/Button";
import { FormRow, FormSection, InlineStatus } from "../ui/Form";
import { StatusLabel } from "../ui/StatusLabel";
import { CATEGORY_LABEL } from "./categoryLabels";

import "./Emulator.css";

interface PendingSectionProps {
  ops: PendingOp[];
  onSyncNow: () => Promise<void>;
  onChanged: () => void;
}

const RETRY_ALL = "all";

function opKey(op: PendingOp): string {
  return `${op.category}/${op.relPath}/${op.direction}`;
}

export function PendingSection({ ops, onSyncNow, onChanged }: PendingSectionProps) {
  const { t } = useTranslation();
  const errorMessage = useErrorMessage();
  const now = useNow();
  const [busyKey, setBusyKey] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const run = async (key: string, action?: () => Promise<void>) => {
    setBusyKey(key);
    setError(null);
    try {
      await action?.();
      await onSyncNow();
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setBusyKey(null);
      onChanged();
    }
  };

  return (
    <FormSection
      title={t("pending.heading")}
      description={t("pending.description")}
      footer={
        <div className="section-footer-row">
          <Button
            variant="bordered"
            icon="sync"
            loading={busyKey === RETRY_ALL}
            disabled={busyKey !== null}
            onClick={() => void run(RETRY_ALL)}
          >
            {t("pending.retryAll")}
          </Button>
          {error ? <InlineStatus tone="danger">{error}</InlineStatus> : null}
        </div>
      }
    >
      {ops.map((op) => {
        const key = opKey(op);
        const dead = op.nextRetryAtMs === null;
        return (
          <FormRow
            key={key}
            icon={op.direction === "upload" ? "arrowUp" : "arrowDown"}
            label={
              <span className="row-path emulator-mono" title={op.relPath}>
                {op.relPath}
              </span>
            }
            description={
              <span title={formatDateTime(op.enqueuedAtMs)}>
                {[
                  t(op.direction === "upload" ? "pending.upload" : "pending.download"),
                  t(CATEGORY_LABEL[op.category]),
                  t("pending.attempts", { count: op.attempts }),
                  t("pending.queued", { when: formatRelativeTime(t, op.enqueuedAtMs, now) }),
                ].join(" · ")}
              </span>
            }
            control={
              <>
                {dead ? (
                  <StatusLabel tone="danger" icon="error">
                    {t("pending.stopped")}
                  </StatusLabel>
                ) : null}
                {op.priority ? (
                  <StatusLabel tone="accent" icon="arrowUp">
                    {t("pending.prioritized")}
                  </StatusLabel>
                ) : null}
                {dead ? (
                  <Button
                    variant="bordered"
                    size="small"
                    loading={busyKey === key}
                    disabled={busyKey !== null}
                    onClick={() =>
                      void run(key, () => retryPendingOp(op.emulator, op.category, op.relPath))
                    }
                  >
                    {t("pending.retry")}
                  </Button>
                ) : null}
                {!op.priority ? (
                  <Button
                    variant="plain"
                    size="small"
                    loading={busyKey === `${key}:bump`}
                    disabled={busyKey !== null}
                    onClick={() =>
                      void run(`${key}:bump`, () =>
                        bumpPendingOp(op.emulator, op.category, op.relPath),
                      )
                    }
                  >
                    {t("pending.prioritize")}
                  </Button>
                ) : null}
              </>
            }
          >
            {op.lastError ? <span className="row-error">{op.lastError}</span> : null}
          </FormRow>
        );
      })}
    </FormSection>
  );
}
