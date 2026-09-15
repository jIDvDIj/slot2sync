import { useState } from "react";
import { useTranslation } from "react-i18next";

import { Button } from "../components/ui/Button";
import { Dialog } from "../components/ui/Dialog";
import { InlineStatus } from "../components/ui/Form";
import { Icon } from "../components/ui/Icon";
import { currentLocale } from "../i18n";
import { useErrorMessage } from "../lib/errors";
import { installUpdate } from "../lib/ipc";
import type { UpdateInfo } from "../types/ipc";

interface UpdateNoticeProps {
  update: UpdateInfo;
  /** Icon-only pill, for narrow toolbars. */
  compact?: boolean;
}

function formatReleaseDate(date: string | null): string | null {
  if (!date) return null;
  const ms = Date.parse(date);
  return Number.isNaN(ms)
    ? null
    : new Date(ms).toLocaleDateString(currentLocale(), { dateStyle: "long" });
}

/** Small pill that stays out of the way; the release notes and install action live in its dialog. */
export function UpdateNotice({ update, compact = false }: UpdateNoticeProps) {
  const { t } = useTranslation();
  const errorMessage = useErrorMessage();
  const [open, setOpen] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const label = t("update.available", { version: update.version });
  const released = formatReleaseDate(update.date);

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
    <>
      <button
        type="button"
        className="update-notice"
        data-compact={compact || undefined}
        aria-haspopup="dialog"
        aria-label={compact ? label : undefined}
        title={label}
        onClick={() => setOpen(true)}
      >
        <Icon name="arrowDown" size={13} />
        {compact ? null : <span>{t("update.availableShort")}</span>}
      </button>

      <Dialog
        open={open}
        onClose={() => setOpen(false)}
        dismissible={!busy}
        title={t("update.title", { version: update.version })}
        description={released ? t("update.released", { date: released }) : undefined}
        footer={
          <>
            <Button variant="bordered" disabled={busy} onClick={() => setOpen(false)}>
              {t("update.later")}
            </Button>
            <Button
              variant="prominent"
              loading={busy}
              data-autofocus
              onClick={() => void install()}
            >
              {busy ? t("update.installing") : t("update.install")}
            </Button>
          </>
        }
      >
        {update.notes ? (
          <p className="update-notes selectable">{update.notes}</p>
        ) : (
          <p className="text-secondary">{t("update.noNotes")}</p>
        )}
        {error ? <InlineStatus tone="danger">{error}</InlineStatus> : null}
      </Dialog>
    </>
  );
}
