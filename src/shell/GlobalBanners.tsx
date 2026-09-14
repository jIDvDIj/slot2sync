import { useState } from "react";
import { useTranslation } from "react-i18next";

import { Banner } from "../components/ui/Banner";
import { Button } from "../components/ui/Button";
import { InlineStatus } from "../components/ui/Form";
import { useErrorMessage } from "../lib/errors";
import { installUpdate } from "../lib/ipc";
import type { AppPanicPayload, UpdateInfo } from "../types/ipc";

interface GlobalBannersProps {
  panic: AppPanicPayload | null;
  onDismissPanic: () => void;
  update: UpdateInfo | null;
  onDismissUpdate: () => void;
}

export function GlobalBanners({
  panic,
  onDismissPanic,
  update,
  onDismissUpdate,
}: GlobalBannersProps) {
  return (
    <>
      {panic ? <PanicBanner panic={panic} onDismiss={onDismissPanic} /> : null}
      {update ? <UpdateBanner update={update} onDismiss={onDismissUpdate} /> : null}
    </>
  );
}

/** A panic only kills the task it hit and the app keeps running in the tray; without this nobody would know. */
function PanicBanner({ panic, onDismiss }: { panic: AppPanicPayload; onDismiss: () => void }) {
  const { t } = useTranslation();
  return (
    <Banner tone="danger" title={t("panic.title")} onDismiss={onDismiss}>
      <p>{t("panic.body")}</p>
      <details className="disclosure">
        <summary>{t("panic.details")}</summary>
        <code className="text-mono">
          {panic.message}
          {panic.location ? ` (${t("panic.at")} ${panic.location})` : ""}
        </code>
      </details>
    </Banner>
  );
}

function UpdateBanner({ update, onDismiss }: { update: UpdateInfo; onDismiss: () => void }) {
  const { t } = useTranslation();
  const errorMessage = useErrorMessage();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const install = async () => {
    setBusy(true);
    setError(null);
    try {
      // On success the app restarts and nothing after this line runs.
      await installUpdate();
    } catch (err) {
      setError(errorMessage(err));
      setBusy(false);
    }
  };

  return (
    <Banner
      tone="info"
      title={t("update.title", { version: update.version })}
      actions={
        <>
          <Button variant="prominent" size="small" loading={busy} onClick={() => void install()}>
            {busy ? t("update.installing") : t("update.install")}
          </Button>
          <Button variant="bordered" size="small" disabled={busy} onClick={onDismiss}>
            {t("update.later")}
          </Button>
        </>
      }
    >
      {update.notes ? <p className="update-notes">{update.notes}</p> : null}
      {error ? <InlineStatus tone="danger">{error}</InlineStatus> : null}
    </Banner>
  );
}
