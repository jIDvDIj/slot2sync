import { useId, useState } from "react";
import { useTranslation } from "react-i18next";

import { useErrorMessage } from "../../lib/errors";
import { setBandwidthLimits, setScanIntervalMinutes, setTriggers } from "../../lib/ipc";
import type { Settings, TriggerSettings } from "../../types/ipc";
import { FormRow, FormSection, InlineStatus } from "../ui/Form";
import { Switch } from "../ui/Switch";
import { NumberSettingRow } from "./SettingRows";

const TRIGGERS = [
  {
    key: "startup",
    labelKey: "settings.autoSync.startupLabel",
    descriptionKey: "settings.autoSync.startupDescription",
  },
  {
    key: "emulatorStart",
    labelKey: "settings.autoSync.emulatorStartLabel",
    descriptionKey: "settings.autoSync.emulatorStartDescription",
  },
  {
    key: "emulatorStop",
    labelKey: "settings.autoSync.emulatorStopLabel",
    descriptionKey: "settings.autoSync.emulatorStopDescription",
  },
] as const satisfies readonly {
  key: keyof TriggerSettings;
  labelKey: string;
  descriptionKey: string;
}[];

interface SyncSettingsProps {
  settings: Settings;
  onSaved: () => void;
  isMobile: boolean;
}

export function SyncSettings({ settings, onSaved, isMobile }: SyncSettingsProps) {
  const { t } = useTranslation();
  const errorMessage = useErrorMessage();
  const baseId = useId();
  const [triggers, setTriggerState] = useState<TriggerSettings>(settings.triggers);
  const [triggerError, setTriggerError] = useState<string | null>(null);

  const toggleTrigger = async (key: keyof TriggerSettings, enabled: boolean) => {
    const previous = triggers;
    const next = { ...triggers, [key]: enabled };
    setTriggerState(next);
    setTriggerError(null);
    try {
      await setTriggers(next);
      onSaved();
    } catch (err) {
      setTriggerError(errorMessage(err));
      setTriggerState(previous);
    }
  };

  return (
    <>
      <FormSection
        title={t("settings.autoSync.title")}
        footer={
          triggerError ? (
            <InlineStatus tone="danger">{triggerError}</InlineStatus>
          ) : (
            t("settings.autoSync.footer")
          )
        }
      >
        {TRIGGERS.map(({ key, labelKey, descriptionKey }) => (
          <FormRow
            key={key}
            label={t(labelKey)}
            description={t(descriptionKey)}
            htmlFor={`${baseId}-${key}`}
            control={
              <Switch
                id={`${baseId}-${key}`}
                checked={triggers[key]}
                onCheckedChange={(enabled) => void toggleTrigger(key, enabled)}
              />
            }
          />
        ))}
      </FormSection>

      {!isMobile ? (
        <FormSection title={t("settings.scan.title")} footer={t("settings.scan.description")}>
          <NumberSettingRow
            label={t("settings.scan.label")}
            description={t("settings.scan.off")}
            value={settings.scanIntervalMinutes}
            min={0}
            max={1440}
            unit={t("settings.scan.unit")}
            rangeMessage={t("settings.scan.range")}
            onCommit={async (minutes) => {
              await setScanIntervalMinutes(minutes);
              onSaved();
            }}
          />
        </FormSection>
      ) : null}

      <FormSection title={t("settings.bandwidth.title")} footer={t("settings.bandwidth.footer")}>
        <NumberSettingRow
          label={t("settings.bandwidth.uploadLabel")}
          value={settings.uploadKbps}
          min={0}
          unit={t("settings.bandwidth.unit")}
          rangeMessage={t("settings.bandwidth.range")}
          onCommit={async (upload) => {
            await setBandwidthLimits(upload, settings.downloadKbps);
            onSaved();
          }}
        />
        <NumberSettingRow
          label={t("settings.bandwidth.downloadLabel")}
          value={settings.downloadKbps}
          min={0}
          unit={t("settings.bandwidth.unit")}
          rangeMessage={t("settings.bandwidth.range")}
          onCommit={async (download) => {
            await setBandwidthLimits(settings.uploadKbps, download);
            onSaved();
          }}
        />
      </FormSection>
    </>
  );
}
