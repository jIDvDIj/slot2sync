import { useTranslation } from "react-i18next";

import { Button } from "../components/ui/Button";
import { useNow } from "../hooks/useNow";
import { formatRelativeTime } from "../lib/time";
import type { LastSync, SyncProgress, UpdateInfo } from "../types/ipc";
import { SyncButton } from "./SyncButton";
import { UpdateNotice } from "./UpdateNotice";

interface ToolbarProps {
  title: string;
  scrolled: boolean;
  compact: boolean;
  sidebarHidden: boolean;
  onToggleSidebar: () => void;
  onBack?: () => void;
  syncing: boolean;
  progress: SyncProgress | null;
  lastSync: LastSync | null;
  onSync: () => void;
  update: UpdateInfo | null;
}

export function Toolbar({
  title,
  scrolled,
  compact,
  sidebarHidden,
  onToggleSidebar,
  onBack,
  syncing,
  progress,
  lastSync,
  onSync,
  update,
}: ToolbarProps) {
  const { t } = useTranslation();
  const now = useNow();

  return (
    <div className="toolbar" data-scrolled={scrolled || undefined}>
      {!compact ? (
        <Button
          variant="plain"
          icon="sidebar"
          aria-label={sidebarHidden ? t("nav.showSidebar") : t("nav.hideSidebar")}
          title={sidebarHidden ? t("nav.showSidebar") : t("nav.hideSidebar")}
          aria-expanded={!sidebarHidden}
          onClick={onToggleSidebar}
        />
      ) : null}
      {onBack ? (
        <Button variant="plain" icon="chevronLeft" onClick={onBack} className="toolbar-back">
          {t("nav.back")}
        </Button>
      ) : null}

      {/* The page's own <h1> is the heading; this echo is visual only. */}
      <span className="toolbar-title truncate" aria-hidden="true">
        {title}
      </span>

      <span className="toolbar-spacer" />

      {update ? <UpdateNotice update={update} compact={compact} /> : null}

      {!compact && !syncing ? (
        <span className="toolbar-meta">
          {lastSync
            ? t("syncBar.lastSynced", { when: formatRelativeTime(t, lastSync.atMs, now) })
            : t("syncBar.neverSynced")}
        </span>
      ) : null}

      <SyncButton syncing={syncing} progress={progress} onSync={onSync} compact={compact} />
    </div>
  );
}
