import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import { currentLocale } from "../i18n";
import { useErrorMessage } from "../lib/errors";
import { formatBytes } from "../lib/format";
import { listBackups, openBackupFolder, restoreVersion } from "../lib/ipc";
import { usePlatform } from "../hooks/usePlatform";
import type { BackupEntry } from "../types/ipc";
import { Modal } from "./ui/Modal";

interface Props {
  onClose: () => void;
}

/**
 * Histórico dos backups locais: lista as cópias que o Slot2Sync guardou antes de sobrescrever
 * arquivos, com filtro por texto. Restauração continua manual, pela pasta.
 */
export function BackupHistoryModal({ onClose }: Props) {
  const { t } = useTranslation();
  const errorMessage = useErrorMessage();
  const { isMobile } = usePlatform();
  // `null` = ainda carregando.
  const [entries, setEntries] = useState<BackupEntry[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [filter, setFilter] = useState("");
  const [busyPath, setBusyPath] = useState<string | null>(null);
  const [restoredPath, setRestoredPath] = useState<string | null>(null);

  /** Só entradas do histórico de versões (`history/`) são restauráveis pela UI
   * — o nome com carimbo permite localizar a versão e o arquivo original. */
  const isRestorable = (entry: BackupEntry) =>
    entry.run === "history" &&
    (entry.category === "saves" || entry.category === "savestates" || entry.category === "config");

  const restore = async (entry: BackupEntry) => {
    setBusyPath(entry.absPath);
    setError(null);
    setRestoredPath(null);
    try {
      await restoreVersion(
        entry.emulator,
        entry.category as "saves" | "savestates" | "config",
        entry.relPath,
      );
      setRestoredPath(entry.absPath);
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setBusyPath(null);
    }
  };

  useEffect(() => {
    listBackups()
      .then(setEntries)
      .catch((err) => setError(errorMessage(err)));
    // errorMessage é estável o suficiente; o fetch deve rodar uma única vez.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const filtered = useMemo(() => {
    if (!entries) return null;
    const needle = filter.trim().toLowerCase();
    if (!needle) return entries;
    return entries.filter((e) =>
      `${e.emulator}/${e.category}/${e.relPath}`.toLowerCase().includes(needle),
    );
  }, [entries, filter]);

  const openFolder = async () => {
    setError(null);
    try {
      await openBackupFolder();
    } catch (err) {
      setError(errorMessage(err));
    }
  };

  return (
    <Modal title={t("backupHistory.title")} onClose={onClose}>
      <p className="muted">{t("backupHistory.intro")}</p>

      <label className="field">
        <span>{t("backupHistory.filterLabel")}</span>
        <input
          type="text"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          placeholder={t("backupHistory.filterPlaceholder")}
        />
      </label>

      {filtered === null && !error ? (
        <p className="muted">{t("app.loading")}</p>
      ) : filtered && filtered.length === 0 ? (
        <p className="muted">{t("backupHistory.empty")}</p>
      ) : filtered ? (
        <div className="backup-list">
          {filtered.map((entry) => (
            <div className="backup-row" key={entry.absPath} title={entry.absPath}>
              <div className="backup-info">
                <span className="backup-path">
                  {entry.emulator} · {entry.category} · {entry.relPath}
                </span>
                <span className="backup-meta">
                  {entry.run} · {new Date(entry.modifiedAtMs).toLocaleString(currentLocale())}
                </span>
              </div>
              <span className="backup-size">{formatBytes(entry.sizeBytes)}</span>
              {isRestorable(entry) ? (
                <button
                  className="secondary"
                  disabled={busyPath !== null}
                  onClick={() => restore(entry)}
                >
                  {busyPath === entry.absPath
                    ? t("backupHistory.restoring")
                    : restoredPath === entry.absPath
                      ? t("backupHistory.restored")
                      : t("backupHistory.restore")}
                </button>
              ) : null}
            </div>
          ))}
        </div>
      ) : null}

      {!isMobile ? (
        <div className="settings-row">
          <button className="secondary" onClick={openFolder}>
            {t("settings.backups.open")}
          </button>
        </div>
      ) : null}
      {error ? <p className="error">{error}</p> : null}
    </Modal>
  );
}
