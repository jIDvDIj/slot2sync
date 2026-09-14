import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import { currentLocale } from "../i18n";
import { filterBackups, groupBackups, type BackupGroup } from "../lib/backupGroups";
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
 * Histórico dos backups locais, agrupado por arquivo, com filtro por texto e
 * por faixa de datas.
 */
export function BackupHistoryModal({ onClose }: Props) {
  const { t } = useTranslation();
  const errorMessage = useErrorMessage();
  const { isMobile } = usePlatform();
  // `null` = ainda carregando.
  const [entries, setEntries] = useState<BackupEntry[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [filter, setFilter] = useState("");
  const [from, setFrom] = useState("");
  const [to, setTo] = useState("");
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
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

  const groups = useMemo(() => {
    if (!entries) return null;
    return groupBackups(filterBackups(entries, { text: filter, from, to }));
  }, [entries, filter, from, to]);

  const toggle = (key: string) =>
    setExpanded((current) => {
      const next = new Set(current);
      if (!next.delete(key)) next.add(key);
      return next;
    });

  const openFolder = async () => {
    setError(null);
    try {
      await openBackupFolder();
    } catch (err) {
      setError(errorMessage(err));
    }
  };

  const renderVersion = (entry: BackupEntry) => (
    <div className="backup-row backup-version" key={entry.absPath} title={entry.absPath}>
      <div className="backup-info">
        <span className="backup-meta">
          {entry.run} · {new Date(entry.modifiedAtMs).toLocaleString(currentLocale())}
        </span>
      </div>
      <span className="backup-size">{formatBytes(entry.sizeBytes)}</span>
      {isRestorable(entry) ? (
        <button className="secondary" disabled={busyPath !== null} onClick={() => restore(entry)}>
          {busyPath === entry.absPath
            ? t("backupHistory.restoring")
            : restoredPath === entry.absPath
              ? t("backupHistory.restored")
              : t("backupHistory.restore")}
        </button>
      ) : null}
    </div>
  );

  const renderGroup = (group: BackupGroup) => {
    const open = expanded.has(group.key);
    return (
      <div className="backup-group" key={group.key}>
        <button
          className="backup-group-head"
          aria-expanded={open}
          onClick={() => toggle(group.key)}
        >
          <span className="backup-path">
            {open ? "▾" : "▸"} {group.emulator} · {group.category} · {group.relPath}
          </span>
          <span className="backup-meta">
            {t("backupHistory.versions", { count: group.entries.length })} ·{" "}
            {new Date(group.newestAtMs).toLocaleString(currentLocale())}
          </span>
          <span className="backup-size">{formatBytes(group.totalBytes)}</span>
        </button>
        {open ? group.entries.map(renderVersion) : null}
      </div>
    );
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

      <div className="backup-range">
        <label className="field">
          <span>{t("backupHistory.fromLabel")}</span>
          <input
            type="date"
            value={from}
            max={to || undefined}
            onChange={(e) => setFrom(e.target.value)}
          />
        </label>
        <label className="field">
          <span>{t("backupHistory.toLabel")}</span>
          <input
            type="date"
            value={to}
            min={from || undefined}
            onChange={(e) => setTo(e.target.value)}
          />
        </label>
      </div>

      {groups === null && !error ? (
        <p className="muted">{t("app.loading")}</p>
      ) : groups && groups.length === 0 ? (
        <p className="muted">{t("backupHistory.empty")}</p>
      ) : groups ? (
        <div className="backup-list">{groups.map(renderGroup)}</div>
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
