import { useState } from "react";
import { useTranslation } from "react-i18next";

import { currentLocale } from "../i18n";
import { useErrorMessage } from "../lib/errors";
import { bumpPendingOp, retryPendingOp, syncNow } from "../lib/ipc";
import type { PendingOp } from "../types/ipc";
import { Modal } from "./ui/Modal";

interface Props {
  emulator: string;
  ops: PendingOp[];
  onClose: () => void;
}

/**
 * Fila offline visível de um emulador: cada arquivo preso com direção, tentativas e o último erro.
 * As pendências são retentadas automaticamente a cada sync; o botão só
 * antecipa a próxima tentativa.
 */
export function PendingOpsModal({ emulator, ops, onClose }: Props) {
  const { t } = useTranslation();
  const errorMessage = useErrorMessage();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const retryNow = async () => {
    setBusy(true);
    setError(null);
    try {
      await syncNow();
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setBusy(false);
    }
  };

  /** Reativa uma pendência morta (zera tentativas/backoff) e sincroniza. */
  const retryFile = async (op: PendingOp) => {
    setBusy(true);
    setError(null);
    try {
      await retryPendingOp(op.emulator, op.category, op.relPath);
      await syncNow();
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setBusy(false);
    }
  };

  /** Marca a pendência como prioritária (lista primeiro), libera o backoff e
   * sincroniza já. */
  const bumpFile = async (op: PendingOp) => {
    setBusy(true);
    setError(null);
    try {
      await bumpPendingOp(op.emulator, op.category, op.relPath);
      await syncNow();
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Modal title={t("pending.title", { emulator })} onClose={onClose}>
      <p className="muted">{t("pending.intro")}</p>

      {ops.length === 0 ? (
        <p className="muted">{t("pending.empty")}</p>
      ) : (
        <div className="pending-list">
          {ops.map((op) => (
            <div className="pending-row" key={`${op.category}/${op.relPath}/${op.direction}`}>
              <span className="pending-path">
                {op.direction === "upload" ? "↑" : "↓"} {op.category} · {op.relPath}
              </span>
              <span className="pending-meta">
                <span>{t(op.direction === "upload" ? "pending.upload" : "pending.download")}</span>
                <span>{t("pending.attempts", { count: op.attempts })}</span>
                <span>{new Date(op.enqueuedAtMs).toLocaleString(currentLocale())}</span>
                {op.priority ? (
                  <span className="pending-priority">{t("pending.prioritized")}</span>
                ) : null}
                {op.nextRetryAtMs === null ? (
                  <span className="pending-error">{t("pending.dead")}</span>
                ) : null}
              </span>
              {op.lastError ? <span className="pending-error">{op.lastError}</span> : null}
              <span className="settings-row">
                {op.nextRetryAtMs === null ? (
                  <button onClick={() => retryFile(op)} disabled={busy}>
                    {t("pending.retryFile")}
                  </button>
                ) : null}
                {!op.priority ? (
                  <button className="secondary" onClick={() => bumpFile(op)} disabled={busy}>
                    {t("pending.bumpFile")}
                  </button>
                ) : null}
              </span>
            </div>
          ))}
        </div>
      )}

      <div className="settings-row">
        <button onClick={retryNow} disabled={busy || ops.length === 0}>
          {busy ? t("pending.retrying") : t("pending.retryNow")}
        </button>
      </div>
      {error ? <p className="error">{error}</p> : null}
    </Modal>
  );
}
