import { useState } from "react";
import { useTranslation } from "react-i18next";

import { useErrorMessage } from "../../lib/errors";
import { Button } from "../ui/Button";
import { ConfirmDialog } from "../ui/Dialog";
import { FormRow, FormSection, InlineStatus } from "../ui/Form";

interface RemoveSectionProps {
  name: string;
  onRemove: (name: string) => Promise<void>;
}

export function RemoveSection({ name, onRemove }: RemoveSectionProps) {
  const { t } = useTranslation();
  const errorMessage = useErrorMessage();
  const [confirming, setConfirming] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const remove = async () => {
    setBusy(true);
    setError(null);
    try {
      // On success the shell navigates away and this component unmounts.
      await onRemove(name);
    } catch (err) {
      setError(errorMessage(err));
      setBusy(false);
      setConfirming(false);
    }
  };

  return (
    <FormSection title={t("remove.heading")}>
      <FormRow
        icon="trash"
        label={t("remove.heading")}
        description={t("remove.description")}
        control={
          <Button variant="destructive" onClick={() => setConfirming(true)}>
            {t("remove.button")}
          </Button>
        }
      >
        {error ? <InlineStatus tone="danger">{error}</InlineStatus> : null}
      </FormRow>
      <ConfirmDialog
        open={confirming}
        destructive
        busy={busy}
        title={t("remove.confirmTitle", { emulator: name })}
        message={t("remove.confirmMessage")}
        confirmLabel={t("remove.confirm")}
        onConfirm={() => void remove()}
        onCancel={() => setConfirming(false)}
      />
    </FormSection>
  );
}
