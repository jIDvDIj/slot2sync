import { useState } from "react";
import { useTranslation } from "react-i18next";

import { SUPPORTED_LANGUAGES, changeLanguage, type LanguageCode } from "../i18n";
import { useErrorMessage } from "../lib/errors";
import {
  openBackupFolder,
  setAutostart,
  setBackupRetentionDays,
  setDeviceName,
  setBandwidthLimits,
  setMaxBackupVersions,
  setNotificationLevel,
  setScanIntervalMinutes,
} from "../lib/ipc";
import type { EmulatorProfile, NotificationLevel, Settings } from "../types/ipc";
import { usePlatform } from "../hooks/usePlatform";
import { BackupHistoryModal } from "./BackupHistoryModal";
import { CategorySettings } from "./CategorySettings";
import { TriggerSettingsSection } from "./TriggerSettings";
import { Modal } from "./ui/Modal";

const NOTIFICATION_OPTIONS = [
  { value: "all", labelKey: "settings.notif.all" },
  { value: "errors_only", labelKey: "settings.notif.errorsOnly" },
  { value: "none", labelKey: "settings.notif.none" },
] as const satisfies readonly { value: NotificationLevel; labelKey: string }[];

/** Abas do modal — conteúdo relacionado agrupado em vez de um scroll único. */
const TABS = [
  { id: "general", labelKey: "settings.tabs.general" },
  { id: "sync", labelKey: "settings.tabs.sync" },
  { id: "notifications", labelKey: "settings.tabs.notifications" },
  { id: "backups", labelKey: "settings.tabs.backups" },
] as const;

type TabId = (typeof TABS)[number]["id"];

interface Props {
  settings: Settings;
  emulators: EmulatorProfile[];
  onClose: () => void;
  /** Recarrega as settings no App após qualquer alteração. */
  onSaved: () => void;
}

/**
 * Modal de configurações, organizado em abas: Geral (idioma,
 * dispositivo, inicialização), Sincronização (gatilhos, categorias),
 * Notificações e Backups (pasta + histórico).
 */
export function SettingsModal({ settings, emulators, onClose, onSaved }: Props) {
  const { t, i18n } = useTranslation();
  const errorMessage = useErrorMessage();
  const { isMobile } = usePlatform();
  const [tab, setTab] = useState<TabId>("general");
  const [device, setDevice] = useState(settings.deviceName ?? "");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);
  const [notifLevel, setNotifLevel] = useState<NotificationLevel>(settings.notificationLevel);
  const [notifError, setNotifError] = useState<string | null>(null);
  const [autostart, setAutostartState] = useState(settings.autostart);
  const [autostartError, setAutostartError] = useState<string | null>(null);
  const [backupError, setBackupError] = useState<string | null>(null);
  const [showBackupHistory, setShowBackupHistory] = useState(false);
  const [retentionDays, setRetentionDays] = useState(String(settings.backupRetentionDays));
  const [retentionSaved, setRetentionSaved] = useState(false);

  const retentionDirty = retentionDays !== String(settings.backupRetentionDays);
  const [maxVersions, setMaxVersions] = useState(String(settings.maxBackupVersions));
  const [versionsSaved, setVersionsSaved] = useState(false);
  const versionsDirty = maxVersions !== String(settings.maxBackupVersions);

  const saveMaxVersions = async () => {
    const versions = Number.parseInt(maxVersions, 10);
    if (Number.isNaN(versions) || versions < 1) return;
    setBackupError(null);
    setVersionsSaved(false);
    try {
      await setMaxBackupVersions(versions);
      onSaved();
      setVersionsSaved(true);
    } catch (err) {
      setBackupError(errorMessage(err));
    }
  };

  const [scanInterval, setScanInterval] = useState(String(settings.scanIntervalMinutes));
  const [scanSaved, setScanSaved] = useState(false);
  const [scanError, setScanError] = useState<string | null>(null);
  const scanDirty = scanInterval !== String(settings.scanIntervalMinutes);

  const [uploadKbps, setUploadKbps] = useState(String(settings.uploadKbps));
  const [downloadKbps, setDownloadKbps] = useState(String(settings.downloadKbps));
  const [bandwidthSaved, setBandwidthSaved] = useState(false);
  const bandwidthDirty =
    uploadKbps !== String(settings.uploadKbps) || downloadKbps !== String(settings.downloadKbps);

  const saveBandwidth = async () => {
    const up = Number.parseInt(uploadKbps, 10);
    const down = Number.parseInt(downloadKbps, 10);
    if (Number.isNaN(up) || Number.isNaN(down) || up < 0 || down < 0) return;
    setScanError(null);
    setBandwidthSaved(false);
    try {
      await setBandwidthLimits(up, down);
      onSaved();
      setBandwidthSaved(true);
    } catch (err) {
      setScanError(errorMessage(err));
    }
  };

  const saveScanInterval = async () => {
    const minutes = Number.parseInt(scanInterval, 10);
    if (Number.isNaN(minutes) || minutes < 0) return;
    setScanError(null);
    setScanSaved(false);
    try {
      await setScanIntervalMinutes(minutes);
      onSaved();
      setScanSaved(true);
    } catch (err) {
      setScanError(errorMessage(err));
    }
  };

  const saveRetention = async () => {
    const days = Number.parseInt(retentionDays, 10);
    if (Number.isNaN(days) || days < 0) return;
    setBackupError(null);
    setRetentionSaved(false);
    try {
      await setBackupRetentionDays(days);
      onSaved();
      setRetentionSaved(true);
    } catch (err) {
      setBackupError(errorMessage(err));
    }
  };

  const openBackups = async () => {
    setBackupError(null);
    try {
      await openBackupFolder();
    } catch (err) {
      setBackupError(errorMessage(err));
    }
  };

  const toggleAutostart = async () => {
    const next = !autostart;
    setAutostartState(next); // otimista
    setAutostartError(null);
    try {
      await setAutostart(next);
      onSaved();
    } catch (err) {
      setAutostartError(errorMessage(err));
      setAutostartState(!next); // reverte em falha
    }
  };

  const changeNotifLevel = async (level: NotificationLevel) => {
    const prev = notifLevel;
    setNotifLevel(level); // otimista
    setNotifError(null);
    try {
      await setNotificationLevel(level);
      onSaved();
    } catch (err) {
      setNotifError(errorMessage(err));
      setNotifLevel(prev); // reverte em falha
    }
  };

  const dirty = device.trim() !== (settings.deviceName ?? "");

  const saveDevice = async () => {
    const name = device.trim();
    if (!name) return;
    setBusy(true);
    setError(null);
    setSaved(false);
    try {
      await setDeviceName(name);
      onSaved();
      setSaved(true);
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Modal title={t("settings.title")} onClose={onClose}>
      <div className="modal-tabs" role="tablist">
        {TABS.map(({ id, labelKey }) => (
          <button
            key={id}
            role="tab"
            aria-selected={tab === id}
            className={`modal-tab${tab === id ? " active" : ""}`}
            onClick={() => setTab(id)}
          >
            {t(labelKey)}
          </button>
        ))}
      </div>

      {tab === "general" ? (
        <>
          <section className="settings-section">
            <h3>{t("settings.language.heading")}</h3>
            <p className="muted">{t("settings.language.hint")}</p>
            <label className="field">
              <span>{t("settings.language.label")}</span>
              <select
                value={i18n.language}
                onChange={(e) => void changeLanguage(e.target.value as LanguageCode)}
              >
                {SUPPORTED_LANGUAGES.map(({ code, label }) => (
                  <option key={code} value={code}>
                    {label}
                  </option>
                ))}
              </select>
            </label>
          </section>

          <section className="settings-section">
            <h3>{t("settings.device.heading")}</h3>
            <p className="muted">{t("settings.device.hint")}</p>
            <label className="field">
              <span>{t("device.nameLabel")}</span>
              <input
                type="text"
                value={device}
                onChange={(e) => {
                  setDevice(e.target.value);
                  setSaved(false);
                }}
                placeholder={t("device.namePlaceholder")}
                maxLength={60}
              />
            </label>
            <div className="settings-row">
              <button onClick={saveDevice} disabled={busy || !dirty || device.trim().length === 0}>
                {busy ? t("settings.device.saving") : t("settings.device.save")}
              </button>
              {saved && !dirty ? (
                <span className="saved-hint">{t("settings.device.saved")}</span>
              ) : null}
            </div>
            {error ? <p className="error">{error}</p> : null}
          </section>

          {!isMobile ? (
            <section className="settings-section">
              <h3>{t("settings.startup.heading")}</h3>
              <p className="muted">{t("settings.startup.hint")}</p>
              <div className="trigger-list">
                <label className="trigger-row">
                  <input type="checkbox" checked={autostart} onChange={toggleAutostart} />
                  <span className="trigger-text">
                    <span className="trigger-label">{t("settings.startup.label")}</span>
                    <span className="muted">{t("settings.startup.sublabel")}</span>
                  </span>
                </label>
                {autostartError ? <p className="error">{autostartError}</p> : null}
              </div>
            </section>
          ) : null}
        </>
      ) : null}

      {tab === "sync" ? (
        <>
          <section className="settings-section">
            <h3>{t("settings.autoSync.heading")}</h3>
            <p className="muted">{t("settings.autoSync.hint")}</p>
            <TriggerSettingsSection triggers={settings.triggers} onChanged={onSaved} />
          </section>

          <section className="settings-section">
            <h3>{t("settings.categories.heading")}</h3>
            <p className="muted">{t("settings.categories.hint")}</p>
            <CategorySettings emulators={emulators} />
          </section>

          <section className="settings-section">
            <h3>{t("settings.bandwidth.heading")}</h3>
            <p className="muted">{t("settings.bandwidth.hint")}</p>
            <label className="field">
              <span>{t("settings.bandwidth.uploadLabel")}</span>
              <input
                type="number"
                min={0}
                value={uploadKbps}
                onChange={(e) => {
                  setUploadKbps(e.target.value);
                  setBandwidthSaved(false);
                }}
              />
            </label>
            <label className="field">
              <span>{t("settings.bandwidth.downloadLabel")}</span>
              <input
                type="number"
                min={0}
                value={downloadKbps}
                onChange={(e) => {
                  setDownloadKbps(e.target.value);
                  setBandwidthSaved(false);
                }}
              />
            </label>
            <div className="settings-row">
              <button onClick={saveBandwidth} disabled={!bandwidthDirty}>
                {t("settings.device.save")}
              </button>
              {bandwidthSaved && !bandwidthDirty ? (
                <span className="saved-hint">{t("settings.bandwidth.saved")}</span>
              ) : null}
            </div>
          </section>

          {!isMobile ? (
            <section className="settings-section">
              <h3>{t("settings.scan.heading")}</h3>
              <p className="muted">{t("settings.scan.hint")}</p>
              <label className="field">
                <span>{t("settings.scan.label")}</span>
                <input
                  type="number"
                  min={0}
                  max={1440}
                  value={scanInterval}
                  onChange={(e) => {
                    setScanInterval(e.target.value);
                    setScanSaved(false);
                  }}
                />
              </label>
              <div className="settings-row">
                <button onClick={saveScanInterval} disabled={!scanDirty}>
                  {t("settings.device.save")}
                </button>
                {scanSaved && !scanDirty ? (
                  <span className="saved-hint">{t("settings.scan.saved")}</span>
                ) : null}
              </div>
              {scanError ? <p className="error">{scanError}</p> : null}
            </section>
          ) : null}
        </>
      ) : null}

      {tab === "notifications" ? (
        <section className="settings-section">
          <h3>{t("settings.notif.heading")}</h3>
          <p className="muted">{t("settings.notif.hint")}</p>
          <label className="field">
            <span>{t("settings.notif.label")}</span>
            <select
              value={notifLevel}
              onChange={(e) => changeNotifLevel(e.target.value as NotificationLevel)}
            >
              {NOTIFICATION_OPTIONS.map(({ value, labelKey }) => (
                <option key={value} value={value}>
                  {t(labelKey)}
                </option>
              ))}
            </select>
          </label>
          {notifError ? <p className="error">{notifError}</p> : null}
        </section>
      ) : null}

      {tab === "backups" ? (
        <section className="settings-section">
          <h3>{t("settings.backups.heading")}</h3>
          <p className="muted">{t("settings.backups.hint")}</p>
          <div className="settings-row">
            <button className="secondary" onClick={() => setShowBackupHistory(true)}>
              {t("settings.backups.history")}
            </button>
            {!isMobile ? (
              <button className="secondary" onClick={openBackups}>
                {t("settings.backups.open")}
              </button>
            ) : null}
          </div>
          <label className="field">
            <span>{t("settings.backups.retentionLabel")}</span>
            <input
              type="number"
              min={0}
              max={3650}
              value={retentionDays}
              onChange={(e) => {
                setRetentionDays(e.target.value);
                setRetentionSaved(false);
              }}
            />
          </label>
          <p className="muted">{t("settings.backups.retentionHint")}</p>
          <div className="settings-row">
            <button onClick={saveRetention} disabled={!retentionDirty}>
              {t("settings.device.save")}
            </button>
            {retentionSaved && !retentionDirty ? (
              <span className="saved-hint">{t("settings.backups.retentionSaved")}</span>
            ) : null}
          </div>
          <label className="field">
            <span>{t("settings.backups.versionsLabel")}</span>
            <input
              type="number"
              min={1}
              max={50}
              value={maxVersions}
              onChange={(e) => {
                setMaxVersions(e.target.value);
                setVersionsSaved(false);
              }}
            />
          </label>
          <p className="muted">{t("settings.backups.versionsHint")}</p>
          <div className="settings-row">
            <button onClick={saveMaxVersions} disabled={!versionsDirty}>
              {t("settings.device.save")}
            </button>
            {versionsSaved && !versionsDirty ? (
              <span className="saved-hint">{t("settings.backups.retentionSaved")}</span>
            ) : null}
          </div>
          {backupError ? <p className="error">{backupError}</p> : null}
        </section>
      ) : null}

      {showBackupHistory ? (
        <BackupHistoryModal onClose={() => setShowBackupHistory(false)} />
      ) : null}
    </Modal>
  );
}
