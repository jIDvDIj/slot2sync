import { useRef, useState, type KeyboardEvent } from "react";
import { useTranslation } from "react-i18next";

import { useErrorMessage } from "../../lib/errors";
import { setNotificationLevel } from "../../lib/ipc";
import type { NotificationLevel, Settings } from "../../types/ipc";
import { FormSection, InlineStatus } from "../ui/Form";
import { Icon } from "../ui/Icon";

const OPTIONS = [
  {
    value: "all",
    labelKey: "settings.notifications.all",
    descriptionKey: "settings.notifications.allDescription",
  },
  {
    value: "errors_only",
    labelKey: "settings.notifications.errorsOnly",
    descriptionKey: "settings.notifications.errorsOnlyDescription",
  },
  {
    value: "none",
    labelKey: "settings.notifications.none",
    descriptionKey: "settings.notifications.noneDescription",
  },
] as const satisfies readonly {
  value: NotificationLevel;
  labelKey: string;
  descriptionKey: string;
}[];

interface NotificationSettingsProps {
  settings: Settings;
  onSaved: () => void;
}

export function NotificationSettings({ settings, onSaved }: NotificationSettingsProps) {
  const { t } = useTranslation();
  const errorMessage = useErrorMessage();
  const [level, setLevel] = useState<NotificationLevel>(settings.notificationLevel);
  const [error, setError] = useState<string | null>(null);
  const refs = useRef<(HTMLButtonElement | null)[]>([]);

  const choose = async (next: NotificationLevel) => {
    if (next === level) return;
    const previous = level;
    setLevel(next);
    setError(null);
    try {
      await setNotificationLevel(next);
      onSaved();
    } catch (err) {
      setError(errorMessage(err));
      setLevel(previous);
    }
  };

  const onKeyDown = (event: KeyboardEvent<HTMLButtonElement>, position: number) => {
    const step =
      event.key === "ArrowDown" || event.key === "ArrowRight"
        ? 1
        : event.key === "ArrowUp" || event.key === "ArrowLeft"
          ? -1
          : 0;
    if (step === 0) return;
    event.preventDefault();
    const next = (position + step + OPTIONS.length) % OPTIONS.length;
    refs.current[next]?.focus();
    void choose(OPTIONS[next].value);
  };

  return (
    <FormSection
      title={t("settings.notifications.title")}
      footer={
        error ? (
          <InlineStatus tone="danger">{error}</InlineStatus>
        ) : (
          t("settings.notifications.footer")
        )
      }
    >
      <div role="radiogroup" aria-label={t("settings.notifications.levelLabel")}>
        {OPTIONS.map(({ value, labelKey, descriptionKey }, position) => {
          const checked = level === value;
          return (
            <button
              key={value}
              ref={(element) => {
                refs.current[position] = element;
              }}
              type="button"
              role="radio"
              aria-checked={checked}
              tabIndex={checked ? 0 : -1}
              className="form-row choice-row"
              onClick={() => void choose(value)}
              onKeyDown={(event) => onKeyDown(event, position)}
            >
              <span className="form-row-main">
                <span className="form-row-text">
                  <span className="form-row-label">{t(labelKey)}</span>
                  <span className="form-row-description">{t(descriptionKey)}</span>
                </span>
                <Icon name="check" size={16} className="choice-row-check" />
              </span>
            </button>
          );
        })}
      </div>
    </FormSection>
  );
}
