import { useState } from "react";
import { useTranslation } from "react-i18next";

import { useErrorMessage } from "../lib/errors";
import { installUpdate } from "../lib/ipc";
import type { UpdateInfo } from "../types/ipc";
import { Button } from "./ui/Button";

interface Props {
  update: UpdateInfo;
  onDismiss: () => void;
}

/** Aviso de versão nova, com instalação e reinício em um clique. */
export function UpdateBanner({ update, onDismiss }: Props) {
  const { t } = useTranslation();
  const errorMessage = useErrorMessage();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const install = async () => {
    setBusy(true);
    setError(null);
    try {
      // Em caso de sucesso o app reinicia e nada abaixo desta linha roda.
      await installUpdate();
    } catch (err) {
      setError(errorMessage(err));
      setBusy(false);
    }
  };

  return (
    <div className="update-banner" role="status">
      <div>
        <strong>{t("update.title", { version: update.version })}</strong>
        {update.notes ? <p className="muted">{update.notes}</p> : null}
        {error ? <p className="error">{error}</p> : null}
      </div>
      <div className="update-actions">
        <Button variant="primary" size="sm" onClick={install} disabled={busy}>
          {busy ? t("update.installing") : t("update.install")}
        </Button>
        <Button variant="secondary" size="sm" onClick={onDismiss} disabled={busy}>
          {t("update.later")}
        </Button>
      </div>
    </div>
  );
}
