import { useState } from "react";
import { useTranslation } from "react-i18next";

import { Banner } from "../components/ui/Banner";
import { Button } from "../components/ui/Button";
import { NoticeBanner } from "../components/ui/NoticeBanner";
import type { SyncState } from "../hooks/useSyncEvents";
import { usePlatform } from "../hooks/usePlatform";
import { useErrorMessage } from "../lib/errors";
import { openBackupFolder } from "../lib/ipc";

interface MainBannersProps {
  sync: SyncState;
  actionError: string | null;
  onDismissActionError: () => void;
  onRetrySync: () => void;
}

export function MainBanners({
  sync,
  actionError,
  onDismissActionError,
  onRetrySync,
}: MainBannersProps) {
  const { t } = useTranslation();
  const errorMessage = useErrorMessage();
  const { isMobile } = usePlatform();
  const [dismissedErrorKey, setDismissedErrorKey] = useState<string | null>(null);
  const [folderError, setFolderError] = useState<string | null>(null);

  const lastError = sync.lastError;
  const errorKey = lastError ? `${lastError.emulator ?? ""}|${lastError.message}` : null;
  const backedUp = sync.lastSync?.summary.backedUp ?? 0;

  const showBackups = async () => {
    setFolderError(null);
    try {
      await openBackupFolder();
    } catch (err) {
      setFolderError(errorMessage(err));
    }
  };

  return (
    <>
      {lastError && errorKey !== dismissedErrorKey ? (
        <Banner
          tone="danger"
          title={
            lastError.emulator
              ? t("syncBar.errorTitleEmulator", { emulator: lastError.emulator })
              : t("syncBar.errorTitle")
          }
          onDismiss={() => setDismissedErrorKey(errorKey)}
          actions={
            <Button variant="bordered" size="small" icon="sync" onClick={onRetrySync}>
              {t("common.retry")}
            </Button>
          }
        >
          {lastError.message}
        </Banner>
      ) : null}

      {actionError ? (
        <Banner tone="danger" title={t("syncBar.errorTitle")} onDismiss={onDismissActionError}>
          {actionError}
        </Banner>
      ) : null}

      {backedUp > 0 && sync.lastSync ? (
        <NoticeBanner
          id={`backup-run-${sync.lastSync.atMs}`}
          tone="warning"
          actions={
            !isMobile ? (
              <Button
                variant="bordered"
                size="small"
                icon="folder"
                onClick={() => void showBackups()}
              >
                {t("syncBar.showBackups")}
              </Button>
            ) : undefined
          }
        >
          <p>{t("syncBar.backupNotice", { count: backedUp })}</p>
          {folderError ? <p className="text-danger">{folderError}</p> : null}
        </NoticeBanner>
      ) : null}
    </>
  );
}
