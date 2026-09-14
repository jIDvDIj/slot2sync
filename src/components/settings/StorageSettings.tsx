import { useState } from "react";
import { useTranslation } from "react-i18next";

import { useErrorMessage } from "../../lib/errors";
import {
  exportDiagnostics,
  openBackupFolder,
  setBackupRetentionDays,
  setMaxBackupVersions,
} from "../../lib/ipc";
import { providerLabel } from "../../lib/providerLabels";
import type { Settings } from "../../types/ipc";
import { BackupHistoryDialog } from "../activity/BackupHistoryDialog";
import { LogViewerDialog } from "../activity/LogViewerDialog";
import { Button } from "../ui/Button";
import { ConfirmDialog } from "../ui/Dialog";
import { FormRow, FormSection, InlineStatus } from "../ui/Form";
import { NumberSettingRow } from "./SettingRows";

interface StorageSettingsProps {
  settings: Settings;
  onSaved: () => void;
  onDisconnectProvider: () => void;
  isMobile: boolean;
}

export function StorageSettings({
  settings,
  onSaved,
  onDisconnectProvider,
  isMobile,
}: StorageSettingsProps) {
  const { t } = useTranslation();
  const errorMessage = useErrorMessage();
  const [confirmChange, setConfirmChange] = useState(false);
  const [confirmSignOut, setConfirmSignOut] = useState(false);
  const [historyOpen, setHistoryOpen] = useState(false);
  const [logOpen, setLogOpen] = useState(false);
  const [folderError, setFolderError] = useState<string | null>(null);
  const [exporting, setExporting] = useState(false);
  const [exportedPath, setExportedPath] = useState<string | null>(null);
  const [exportError, setExportError] = useState<string | null>(null);

  const provider = settings.storageProvider;
  const providerDetail = provider === "local_folder" ? settings.folderProviderPath : null;

  const showBackupFolder = async () => {
    setFolderError(null);
    try {
      await openBackupFolder();
    } catch (err) {
      setFolderError(errorMessage(err));
    }
  };

  const exportFile = async () => {
    setExporting(true);
    setExportError(null);
    setExportedPath(null);
    try {
      setExportedPath(await exportDiagnostics());
    } catch (err) {
      setExportError(errorMessage(err));
    } finally {
      setExporting(false);
    }
  };

  return (
    <>
      <FormSection title={t("settings.provider.title")}>
        <FormRow
          icon={provider === "local_folder" ? "folder" : "cloud"}
          label={provider ? providerLabel(provider, t) : t("settings.provider.none")}
          description={
            providerDetail ? <span className="selectable">{providerDetail}</span> : undefined
          }
          control={
            <Button onClick={() => setConfirmChange(true)}>{t("settings.provider.change")}</Button>
          }
        />
        <FormRow
          label={t("signOut.button")}
          description={t("signOut.message")}
          control={
            <Button variant="destructive" icon="power" onClick={() => setConfirmSignOut(true)}>
              {t("signOut.button")}
            </Button>
          }
        />
      </FormSection>

      <FormSection
        title={t("settings.backups.title")}
        footer={
          folderError ? (
            <InlineStatus tone="danger">{folderError}</InlineStatus>
          ) : (
            t("settings.backups.footer")
          )
        }
      >
        <NumberSettingRow
          label={t("settings.backups.retentionLabel")}
          description={t("settings.backups.retentionDescription")}
          value={settings.backupRetentionDays}
          min={0}
          max={3650}
          unit={t("settings.backups.retentionUnit")}
          rangeMessage={t("settings.backups.retentionRange")}
          onCommit={async (days) => {
            await setBackupRetentionDays(days);
            onSaved();
          }}
        />
        <NumberSettingRow
          label={t("settings.backups.versionsLabel")}
          description={t("settings.backups.versionsDescription")}
          value={settings.maxBackupVersions}
          min={1}
          max={50}
          unit=""
          rangeMessage={t("settings.backups.versionsRange")}
          onCommit={async (versions) => {
            await setMaxBackupVersions(versions);
            onSaved();
          }}
        />
        <FormRow
          label={t("settings.backups.historyLabel")}
          description={t("settings.backups.historyDescription")}
          control={
            <>
              {!isMobile ? (
                <Button variant="plain" onClick={() => void showBackupFolder()}>
                  {t("settings.backups.showInFolder")}
                </Button>
              ) : null}
              <Button onClick={() => setHistoryOpen(true)}>{t("settings.backups.history")}</Button>
            </>
          }
        />
      </FormSection>

      <FormSection
        title={t("settings.diagnostics.title")}
        footer={t("settings.diagnostics.footer")}
      >
        <FormRow
          label={t("settings.diagnostics.logLabel")}
          description={t("settings.diagnostics.logDescription")}
          control={
            <Button onClick={() => setLogOpen(true)}>{t("settings.diagnostics.showLog")}</Button>
          }
        />
        {!isMobile ? (
          <FormRow
            label={t("settings.diagnostics.exportLabel")}
            control={
              <Button loading={exporting} onClick={() => void exportFile()}>
                {exporting ? t("settings.diagnostics.exporting") : t("settings.diagnostics.export")}
              </Button>
            }
          >
            {exportedPath ? (
              <InlineStatus tone="success">
                <span className="selectable">
                  {t("settings.diagnostics.exported", { path: exportedPath })}
                </span>
              </InlineStatus>
            ) : null}
            {exportError ? <InlineStatus tone="danger">{exportError}</InlineStatus> : null}
          </FormRow>
        ) : null}
      </FormSection>

      <ConfirmDialog
        open={confirmChange}
        title={t("settings.provider.confirmTitle")}
        message={t("settings.provider.confirmMessage")}
        confirmLabel={t("settings.provider.confirm")}
        onCancel={() => setConfirmChange(false)}
        onConfirm={() => {
          setConfirmChange(false);
          onDisconnectProvider();
        }}
      />
      <ConfirmDialog
        open={confirmSignOut}
        title={t("signOut.title")}
        message={t("signOut.message")}
        confirmLabel={t("signOut.confirm")}
        destructive
        onCancel={() => setConfirmSignOut(false)}
        onConfirm={() => {
          setConfirmSignOut(false);
          onDisconnectProvider();
        }}
      />
      <BackupHistoryDialog open={historyOpen} onClose={() => setHistoryOpen(false)} />
      <LogViewerDialog open={logOpen} onClose={() => setLogOpen(false)} />
    </>
  );
}
