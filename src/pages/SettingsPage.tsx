import { useState } from "react";
import { useTranslation } from "react-i18next";

import { GeneralSettings } from "../components/settings/GeneralSettings";
import { NotificationSettings } from "../components/settings/NotificationSettings";
import { StorageSettings } from "../components/settings/StorageSettings";
import { SyncSettings } from "../components/settings/SyncSettings";
import { SegmentedControl } from "../components/ui/Form";
import { PageHeader } from "../components/ui/PageHeader";
import type { Appearance } from "../hooks/useAppearance";
import { usePlatform } from "../hooks/usePlatform";
import type { Settings } from "../types/ipc";

import "../components/settings/Settings.css";

export interface SettingsPageProps {
  settings: Settings;
  /** Reloads settings in the shell after any change. */
  onSaved: () => void;
  /** Signs out of the active provider; the shell returns to the sign-in screen. */
  onDisconnectProvider: () => void;
  appearance: Appearance;
  onAppearanceChange: (appearance: Appearance) => void;
}

type Section = "general" | "sync" | "notifications" | "storage";

export function SettingsPage({
  settings,
  onSaved,
  onDisconnectProvider,
  appearance,
  onAppearanceChange,
}: SettingsPageProps) {
  const { t } = useTranslation();
  const { isMobile } = usePlatform();
  const [section, setSection] = useState<Section>("general");

  return (
    <div className="settings-page">
      <div>
        <PageHeader title={t("nav.settings")} />
        <div className="settings-sections">
          <SegmentedControl
            aria-label={t("settings.sectionsLabel")}
            value={section}
            onChange={setSection}
            options={[
              { value: "general", label: t("settings.sections.general") },
              { value: "sync", label: t("settings.sections.sync") },
              { value: "notifications", label: t("settings.sections.notifications") },
              { value: "storage", label: t("settings.sections.storage") },
            ]}
          />
        </div>
      </div>

      <div className="settings-panel" key={section}>
        {section === "general" ? (
          <GeneralSettings
            settings={settings}
            onSaved={onSaved}
            appearance={appearance}
            onAppearanceChange={onAppearanceChange}
            isMobile={isMobile}
          />
        ) : section === "sync" ? (
          <SyncSettings settings={settings} onSaved={onSaved} isMobile={isMobile} />
        ) : section === "notifications" ? (
          <NotificationSettings settings={settings} onSaved={onSaved} />
        ) : (
          <StorageSettings
            settings={settings}
            onSaved={onSaved}
            onDisconnectProvider={onDisconnectProvider}
            isMobile={isMobile}
          />
        )}
      </div>
    </div>
  );
}
