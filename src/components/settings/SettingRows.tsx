import { useEffect, useId, useState, type KeyboardEvent, type ReactNode } from "react";
import { useTranslation } from "react-i18next";

import { useErrorMessage } from "../../lib/errors";
import { FormRow, InlineStatus, TextField } from "../ui/Form";

const SAVED_VISIBLE_MS = 2500;

/** "Saved" confirmation that fades out on its own. */
function useSavedFlag(): [boolean, (value: boolean) => void] {
  const [saved, setSaved] = useState(false);
  useEffect(() => {
    if (!saved) return;
    const timer = window.setTimeout(() => setSaved(false), SAVED_VISIBLE_MS);
    return () => window.clearTimeout(timer);
  }, [saved]);
  return [saved, setSaved];
}

function blurOnEnter(event: KeyboardEvent<HTMLInputElement>, revert: () => void) {
  if (event.key === "Enter") {
    event.currentTarget.blur();
  } else if (event.key === "Escape") {
    revert();
    event.currentTarget.blur();
  }
}

interface NumberSettingRowProps {
  label: ReactNode;
  description?: ReactNode;
  value: number;
  min: number;
  max?: number;
  unit: string;
  rangeMessage: string;
  onCommit: (value: number) => Promise<void>;
}

/** Numeric setting that validates inline and applies on blur or Enter. */
export function NumberSettingRow({
  label,
  description,
  value,
  min,
  max,
  unit,
  rangeMessage,
  onCommit,
}: NumberSettingRowProps) {
  const { t } = useTranslation();
  const errorMessage = useErrorMessage();
  const id = useId();
  const [text, setText] = useState(String(value));
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useSavedFlag();

  const [previousValue, setPreviousValue] = useState(value);
  if (value !== previousValue) {
    setPreviousValue(value);
    setText(String(value));
  }

  const commit = async () => {
    const trimmed = text.trim();
    const parsed = Number(trimmed);
    const valid = /^\d+$/.test(trimmed) && parsed >= min && (max === undefined || parsed <= max);
    if (!valid) {
      setError(rangeMessage);
      return;
    }
    setError(null);
    if (parsed === value) return;
    try {
      await onCommit(parsed);
      setSaved(true);
    } catch (err) {
      setError(errorMessage(err));
    }
  };

  return (
    <FormRow
      label={label}
      description={description}
      htmlFor={id}
      control={
        <span className="setting-input-group">
          <TextField
            id={id}
            type="number"
            inputMode="numeric"
            min={min}
            max={max}
            className="setting-number"
            value={text}
            invalid={error !== null}
            onChange={(event) => {
              setText(event.target.value);
              setSaved(false);
            }}
            onBlur={() => void commit()}
            onKeyDown={(event) =>
              blurOnEnter(event, () => {
                setText(String(value));
                setError(null);
              })
            }
          />
          <span className="setting-unit">{unit}</span>
        </span>
      }
    >
      {error ? <InlineStatus tone="danger">{error}</InlineStatus> : null}
      {saved ? <InlineStatus tone="success">{t("settings.saved")}</InlineStatus> : null}
    </FormRow>
  );
}

interface TextSettingRowProps {
  label: ReactNode;
  description?: ReactNode;
  value: string;
  placeholder?: string;
  maxLength?: number;
  emptyMessage: string;
  onCommit: (value: string) => Promise<void>;
}

/** Text setting that refuses empty values and applies on blur or Enter. */
export function TextSettingRow({
  label,
  description,
  value,
  placeholder,
  maxLength,
  emptyMessage,
  onCommit,
}: TextSettingRowProps) {
  const { t } = useTranslation();
  const errorMessage = useErrorMessage();
  const id = useId();
  const [text, setText] = useState(value);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useSavedFlag();

  const [previousValue, setPreviousValue] = useState(value);
  if (value !== previousValue) {
    setPreviousValue(value);
    setText(value);
  }

  const commit = async () => {
    const trimmed = text.trim();
    if (!trimmed) {
      setError(emptyMessage);
      return;
    }
    setError(null);
    if (trimmed === value) return;
    try {
      await onCommit(trimmed);
      setSaved(true);
    } catch (err) {
      setError(errorMessage(err));
    }
  };

  return (
    <FormRow
      label={label}
      description={description}
      htmlFor={id}
      control={
        <TextField
          id={id}
          className="setting-text"
          value={text}
          placeholder={placeholder}
          maxLength={maxLength}
          invalid={error !== null}
          onChange={(event) => {
            setText(event.target.value);
            setSaved(false);
          }}
          onBlur={() => void commit()}
          onKeyDown={(event) =>
            blurOnEnter(event, () => {
              setText(value);
              setError(null);
            })
          }
        />
      }
    >
      {error ? <InlineStatus tone="danger">{error}</InlineStatus> : null}
      {saved ? <InlineStatus tone="success">{t("settings.saved")}</InlineStatus> : null}
    </FormRow>
  );
}
