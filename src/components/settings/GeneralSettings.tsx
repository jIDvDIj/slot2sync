import { useId, useState } from "react";
import { useTranslation } from "react-i18next";

import type { Appearance } from "../../hooks/useAppearance";
import { SUPPORTED_LANGUAGES, changeLanguage, type LanguageCode } from "../../i18n";
import { useErrorMessage } from "../../lib/errors";
import { setAutostart, setDeviceName } from "../../lib/ipc";
import type { Settings } from "../../types/ipc";
import { FormRow, FormSection, InlineStatus, SegmentedControl, Select } from "../ui/Form";
import { Switch } from "../ui/Switch";
import { TextSettingRow } from "./SettingRows";

interface GeneralSettingsProps {
  settings: Settings;
  onSaved: () => void;
  appearance: Appearance;
  onAppearanceChange: (appearance: Appearance) => void;
  isMobile: boolean;
}

export function GeneralSettings({
  settings,
  onSaved,
  appearance,
  onAppearanceChange,
  isMobile,
}: GeneralSettingsProps) {
  const { t, i18n } = useTranslation();
  const errorMessage = useErrorMessage();
  const languageId = useId();
  const startupId = useId();
  const [autostart, setAutostartState] = useState(settings.autostart);
  const [autostartError, setAutostartError] = useState<string | null>(null);

  const toggleAutostart = async (next: boolean) => {
    setAutostartState(next);
    setAutostartError(null);
    try {
      await setAutostart(next);
      onSaved();
    } catch (err) {
      setAutostartError(errorMessage(err));
      setAutostartState(!next);
    }
  };

  return (
    <>
      <FormSection title={t("settings.interface.title")}>
        <FormRow
          label={t("settings.interface.appearanceLabel")}
          description={t("settings.interface.appearanceDescription")}
          control={
            <SegmentedControl
              aria-label={t("settings.interface.appearanceLabel")}
              value={appearance}
              onChange={onAppearanceChange}
              options={[
                { value: "system", label: t("settings.interface.system") },
                { value: "light", label: t("settings.interface.light") },
                { value: "dark", label: t("settings.interface.dark") },
              ]}
            />
          }
        />
        <FormRow
          label={t("settings.interface.languageLabel")}
          htmlFor={languageId}
          control={
            <Select
              id={languageId}
              value={i18n.language}
              onChange={(event) => void changeLanguage(event.target.value as LanguageCode)}
            >
              {SUPPORTED_LANGUAGES.map(({ code, label }) => (
                <option key={code} value={code}>
                  {label}
                </option>
              ))}
            </Select>
          }
        />
      </FormSection>

      <FormSection title={t("settings.device.title")}>
        <TextSettingRow
          label={t("settings.device.nameLabel")}
          description={t("settings.device.nameDescription")}
          value={settings.deviceName ?? ""}
          placeholder={t("settings.device.namePlaceholder")}
          maxLength={60}
          emptyMessage={t("settings.device.nameEmpty")}
          onCommit={async (name) => {
            await setDeviceName(name);
            onSaved();
          }}
        />
        {!isMobile ? (
          <FormRow
            label={t("settings.device.startupLabel")}
            description={t("settings.device.startupDescription")}
            htmlFor={startupId}
            control={
              <Switch
                id={startupId}
                checked={autostart}
                onCheckedChange={(next) => void toggleAutostart(next)}
              />
            }
          >
            {autostartError ? <InlineStatus tone="danger">{autostartError}</InlineStatus> : null}
          </FormRow>
        ) : null}
      </FormSection>
    </>
  );
}
