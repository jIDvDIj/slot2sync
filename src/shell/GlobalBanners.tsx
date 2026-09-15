import { useTranslation } from "react-i18next";

import { Banner } from "../components/ui/Banner";
import type { AppPanicPayload } from "../types/ipc";

interface GlobalBannersProps {
  panic: AppPanicPayload | null;
  onDismissPanic: () => void;
}

export function GlobalBanners({ panic, onDismissPanic }: GlobalBannersProps) {
  return panic ? <PanicBanner panic={panic} onDismiss={onDismissPanic} /> : null;
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
