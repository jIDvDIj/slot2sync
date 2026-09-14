import { useEffect, useId, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import { usePlatform } from "../../hooks/usePlatform";
import { filterBackups, groupBackups, type BackupGroup } from "../../lib/backupGroups";
import { useErrorMessage } from "../../lib/errors";
import { formatBytes } from "../../lib/format";
import { listBackups, openBackupFolder, restoreVersion } from "../../lib/ipc";
import { formatDateTime } from "../../lib/time";
import type { BackupEntry, PendingOp } from "../../types/ipc";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";
import { EmptyState } from "../ui/EmptyState";
import { Field, InlineStatus, TextField } from "../ui/Form";
import { Icon } from "../ui/Icon";
import { Spinner } from "../ui/Spinner";

import "./Activity.css";

type Category = PendingOp["category"];

/** Only archived history entries carry the stamped name needed to find the original file. */
function restorableCategory(entry: BackupEntry): Category | null {
  if (entry.run !== "history") return null;
  return entry.category === "saves" ||
    entry.category === "savestates" ||
    entry.category === "config"
    ? entry.category
    : null;
}

interface BackupHistoryDialogProps {
  open: boolean;
  onClose: () => void;
}

export function BackupHistoryDialog({ open, onClose }: BackupHistoryDialogProps) {
  const { t } = useTranslation();
  const errorMessage = useErrorMessage();
  const { isMobile } = usePlatform();
  const searchId = useId();
  const fromId = useId();
  const toId = useId();
  const [entries, setEntries] = useState<BackupEntry[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [search, setSearch] = useState("");
  const [from, setFrom] = useState("");
  const [to, setTo] = useState("");
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [busyPath, setBusyPath] = useState<string | null>(null);
  const [restored, setRestored] = useState<Set<string>>(new Set());

  useEffect(() => {
    if (!open) return;
    let active = true;
    listBackups()
      .then((loaded) => {
        if (active) setEntries(loaded);
      })
      .catch((err: unknown) => {
        if (active) setError(errorMessage(err));
      });
    return () => {
      active = false;
    };
  }, [open, errorMessage]);

  const groups = useMemo(
    () => (entries ? groupBackups(filterBackups(entries, { text: search, from, to })) : null),
    [entries, search, from, to],
  );

  const toggle = (key: string) =>
    setExpanded((current) => {
      const next = new Set(current);
      if (!next.delete(key)) next.add(key);
      return next;
    });

  const restore = async (entry: BackupEntry, category: Category) => {
    setBusyPath(entry.absPath);
    setError(null);
    try {
      await restoreVersion(entry.emulator, category, entry.relPath);
      setRestored((current) => new Set(current).add(entry.absPath));
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setBusyPath(null);
    }
  };

  const showFolder = async () => {
    setError(null);
    try {
      await openBackupFolder();
    } catch (err) {
      setError(errorMessage(err));
    }
  };

  const filtering = search.trim() !== "" || from !== "" || to !== "";

  const renderGroup = (group: BackupGroup) => {
    const isOpen = expanded.has(group.key);
    const panelId = `${searchId}-${group.key}`;
    return (
      <li key={group.key} className="backup-group">
        <button
          type="button"
          className="backup-group-head"
          aria-expanded={isOpen}
          aria-controls={panelId}
          onClick={() => toggle(group.key)}
        >
          <Icon name="chevronRight" size={14} className="backup-chevron" />
          <span className="backup-group-text">
            <span className="backup-path truncate" title={group.relPath}>
              {group.relPath}
            </span>
            <span className="backup-meta truncate">
              {group.emulator} · {group.category} ·{" "}
              {t("backupHistory.versions", { count: group.entries.length })}
            </span>
          </span>
          <span className="backup-side tabular">
            <span>{formatDateTime(group.newestAtMs)}</span>
            <span className="backup-meta">{formatBytes(group.totalBytes)}</span>
          </span>
        </button>
        {isOpen ? (
          <ul role="list" id={panelId} className="backup-versions">
            {group.entries.map((entry) => {
              const category = restorableCategory(entry);
              return (
                <li key={entry.absPath} className="backup-version" title={entry.absPath}>
                  <span className="backup-version-text tabular">
                    <span>{formatDateTime(entry.modifiedAtMs)}</span>
                    <span className="backup-meta">
                      {entry.run} · {formatBytes(entry.sizeBytes)}
                    </span>
                  </span>
                  {category ? (
                    restored.has(entry.absPath) ? (
                      <InlineStatus tone="success">{t("backupHistory.restored")}</InlineStatus>
                    ) : (
                      <Button
                        size="small"
                        icon="restore"
                        loading={busyPath === entry.absPath}
                        disabled={busyPath !== null}
                        onClick={() => void restore(entry, category)}
                      >
                        {t("backupHistory.restore")}
                      </Button>
                    )
                  ) : null}
                </li>
              );
            })}
          </ul>
        ) : null}
      </li>
    );
  };

  return (
    <Dialog
      open={open}
      onClose={onClose}
      size="large"
      title={t("backupHistory.title")}
      description={t("backupHistory.description")}
      footer={
        <>
          {!isMobile ? (
            <Button icon="folder" onClick={() => void showFolder()}>
              {t("settings.backups.showInFolder")}
            </Button>
          ) : null}
          <Button variant="prominent" onClick={onClose}>
            {t("common.done")}
          </Button>
        </>
      }
    >
      <div className="backup-filters">
        <Field label={t("backupHistory.searchLabel")} htmlFor={searchId}>
          <span className="search-field">
            <Icon name="search" size={14} className="search-field-icon" />
            <TextField
              id={searchId}
              type="search"
              value={search}
              placeholder={t("backupHistory.searchPlaceholder")}
              spellCheck={false}
              onChange={(event) => setSearch(event.target.value)}
            />
          </span>
        </Field>
        <Field label={t("backupHistory.fromLabel")} htmlFor={fromId}>
          <TextField
            id={fromId}
            type="date"
            value={from}
            max={to || undefined}
            onChange={(event) => setFrom(event.target.value)}
          />
        </Field>
        <Field label={t("backupHistory.toLabel")} htmlFor={toId}>
          <TextField
            id={toId}
            type="date"
            value={to}
            min={from || undefined}
            onChange={(event) => setTo(event.target.value)}
          />
        </Field>
      </div>

      {error ? <InlineStatus tone="danger">{error}</InlineStatus> : null}

      {groups === null ? (
        error ? null : (
          <div className="dialog-loading">
            <Spinner label={t("common.loading")} />
          </div>
        )
      ) : groups.length === 0 ? (
        <EmptyState
          compact
          icon="archive"
          title={filtering ? t("backupHistory.noMatches") : t("backupHistory.empty")}
          message={filtering ? undefined : t("backupHistory.emptyMessage")}
        />
      ) : (
        <ul role="list" className="backup-list">
          {groups.map(renderGroup)}
        </ul>
      )}
    </Dialog>
  );
}
