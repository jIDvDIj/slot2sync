import { useState } from "react";
import { useTranslation } from "react-i18next";

import { currentLocale } from "../i18n";
import { useErrorMessage } from "../lib/errors";
import { formatBytes } from "../lib/format";
import { resolveConflict, revealBackupPath } from "../lib/ipc";
import type { Conflict, ConflictResolution } from "../types/ipc";
import { usePlatform } from "../hooks/usePlatform";
import { Modal } from "./ui/Modal";

interface Props {
  emulator: string;
  conflicts: Conflict[];
  onClose: () => void;
  /** Recarrega a lista de conflitos no App após uma resolução. */
  onResolved: () => void;
}

function formatDate(ms: number): string {
  return new Date(ms).toLocaleString(currentLocale());
}

/** Modal de resolução de conflito de um emulador (uma ou mais entradas). */
export function ConflictModal({ emulator, conflicts, onClose, onResolved }: Props) {
  const { t } = useTranslation();
  const errorMessage = useErrorMessage();
  const { isMobile } = usePlatform();
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const showCopy = async (path: string) => {
    setError(null);
    try {
      await revealBackupPath(path);
    } catch (err) {
      setError(errorMessage(err));
    }
  };

  const resolve = async (c: Conflict, keep: ConflictResolution) => {
    setBusy(c.relPath);
    setError(null);
    try {
      await resolveConflict(c.emulator, c.category, c.relPath, keep);
      onResolved();
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setBusy(null);
    }
  };

  return (
    <Modal title={t("conflict.title", { emulator })} onClose={onClose}>
      <p className="muted">{t("conflict.intro")}</p>

      {conflicts.map((c) => (
        <div className="conflict-item" key={`${c.category}/${c.relPath}`}>
          <div className="conflict-path">
            {c.category} · {c.relPath}
          </div>
          {!isMobile && c.backupPath ? (
            <button className="secondary" onClick={() => void showCopy(c.backupPath as string)}>
              {t("conflict.openCopy")}
            </button>
          ) : null}
          <div className="conflict-sides">
            <div className="conflict-side">
              <div className="conflict-side-title">
                {t("conflict.thisDevice")}
                {c.localDevice ? ` · ${c.localDevice}` : ""}
              </div>
              <div className="muted">
                {formatDate(c.localMtimeMs)} · {formatBytes(c.localSize)}
              </div>
              <button disabled={busy === c.relPath} onClick={() => resolve(c, "local")}>
                {t("conflict.keepLocal")}
              </button>
            </div>
            <div className="conflict-side">
              <div className="conflict-side-title">
                {t("conflict.remote")}
                {c.remoteDevice ? ` · ${c.remoteDevice}` : ""}
              </div>
              <div className="muted">
                {formatDate(c.remoteMtimeMs)} · {formatBytes(c.remoteSize)}
              </div>
              <button disabled={busy === c.relPath} onClick={() => resolve(c, "remote")}>
                {t("conflict.keepRemote")}
              </button>
            </div>
          </div>
        </div>
      ))}
      {error ? <p className="error">{error}</p> : null}
    </Modal>
  );
}
