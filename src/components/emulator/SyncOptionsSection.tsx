import { useId, useState } from "react";
import { useTranslation } from "react-i18next";

import { useErrorMessage } from "../../lib/errors";
import { setEmulatorCategories, setExcludePatterns } from "../../lib/ipc";
import type { EmulatorProfile, SyncCategories } from "../../types/ipc";
import { Button } from "../ui/Button";
import { FormRow, FormSection, InlineStatus, TextField } from "../ui/Form";
import { Switch } from "../ui/Switch";

import "./Emulator.css";

// "config" is left out on purpose: the backend keeps it permanently disabled
// and the UI must not be able to turn it back on.
const CATEGORY_ROWS = [
  { key: "saves", labelKey: "options.saves", hintKey: "options.savesHint" },
  { key: "savestates", labelKey: "options.savestates", hintKey: "options.savestatesHint" },
] as const satisfies readonly {
  key: keyof SyncCategories;
  labelKey: string;
  hintKey: string;
}[];

function parsePatterns(text: string): string[] {
  return text
    .split(",")
    .map((pattern) => pattern.trim())
    .filter((pattern) => pattern.length > 0);
}

interface SyncOptionsSectionProps {
  profile: EmulatorProfile;
  /** `null` while loading. */
  categories: SyncCategories | null;
  onCategoriesChange: (categories: SyncCategories) => void;
}

export function SyncOptionsSection({
  profile,
  categories,
  onCategoriesChange,
}: SyncOptionsSectionProps) {
  const { t } = useTranslation();
  const errorMessage = useErrorMessage();
  const idPrefix = useId();
  const patternsId = `${idPrefix}-patterns`;

  const [categoryError, setCategoryError] = useState<string | null>(null);
  const [savedPatterns, setSavedPatterns] = useState(profile.excludePatterns.join(", "));
  const [patterns, setPatterns] = useState(savedPatterns);
  const [patternsBusy, setPatternsBusy] = useState(false);
  const [patternsSaved, setPatternsSaved] = useState(false);
  const [patternsError, setPatternsError] = useState<string | null>(null);

  const patternsDirty =
    parsePatterns(patterns).join(", ") !== parsePatterns(savedPatterns).join(", ");
  const allOff = categories !== null && !categories.saves && !categories.savestates;

  const toggle = async (key: keyof SyncCategories, value: boolean) => {
    if (!categories) return;
    const previous = categories;
    const next = { ...categories, [key]: value };
    onCategoriesChange(next);
    setCategoryError(null);
    try {
      await setEmulatorCategories(profile.name, next);
    } catch (err) {
      onCategoriesChange(previous);
      setCategoryError(errorMessage(err));
    }
  };

  const savePatterns = async () => {
    const list = parsePatterns(patterns);
    setPatternsBusy(true);
    setPatternsError(null);
    setPatternsSaved(false);
    try {
      await setExcludePatterns(profile.name, list);
      setSavedPatterns(list.join(", "));
      setPatterns(list.join(", "));
      setPatternsSaved(true);
    } catch (err) {
      setPatternsError(errorMessage(err));
    } finally {
      setPatternsBusy(false);
    }
  };

  return (
    <FormSection
      title={t("options.heading")}
      description={t("options.description")}
      footer={allOff ? t("options.allOff") : undefined}
    >
      {CATEGORY_ROWS.map(({ key, labelKey, hintKey }) => {
        const id = `${idPrefix}-${key}`;
        return (
          <FormRow
            key={key}
            label={t(labelKey)}
            description={t(hintKey)}
            htmlFor={id}
            control={
              <Switch
                id={id}
                checked={categories ? categories[key] : true}
                disabled={!categories}
                onCheckedChange={(value) => void toggle(key, value)}
              />
            }
          />
        );
      })}
      {categoryError ? (
        <FormRow label={<InlineStatus tone="danger">{categoryError}</InlineStatus>} />
      ) : null}
      <FormRow
        label={t("options.ignoreLabel")}
        description={t("options.ignoreHint")}
        htmlFor={patternsId}
      >
        <form
          className="patterns-row"
          onSubmit={(event) => {
            event.preventDefault();
            if (patternsDirty && !patternsBusy) void savePatterns();
          }}
        >
          <TextField
            id={patternsId}
            className="emulator-mono"
            value={patterns}
            placeholder={t("options.ignorePlaceholder")}
            spellCheck={false}
            invalid={patternsError !== null}
            onChange={(event) => {
              setPatterns(event.target.value);
              setPatternsSaved(false);
            }}
          />
          <Button type="submit" variant="bordered" loading={patternsBusy} disabled={!patternsDirty}>
            {t("common.save")}
          </Button>
        </form>
        {patternsError ? (
          <InlineStatus tone="danger">{patternsError}</InlineStatus>
        ) : patternsSaved && !patternsDirty ? (
          <InlineStatus tone="success">{t("common.saved")}</InlineStatus>
        ) : null}
      </FormRow>
    </FormSection>
  );
}
