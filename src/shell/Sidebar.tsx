import type { ReactNode } from "react";
import { useTranslation } from "react-i18next";

import logo from "../assets/logo.png";
import { Button } from "../components/ui/Button";
import { Icon, type IconName } from "../components/ui/Icon";
import { useEmulatorCategories } from "../hooks/useEmulatorCategories";
import { ariaShortcut, shortcutLabel, type Shortcut } from "../hooks/useShortcut";
import type { SyncState } from "../hooks/useSyncEvents";
import { cx } from "../lib/cx";
import { emulatorStatus } from "../lib/emulatorStatus";
import type { Route } from "../lib/navigation";
import { providerLabel } from "../lib/providerLabels";
import { STATUS_PRESENTATION } from "../lib/statusPresentation";
import type { Conflict, EmulatorProfile, PendingOp, ProviderKind } from "../types/ipc";
import { SHORTCUTS } from "./shortcuts";

interface SidebarProps {
  hidden: boolean;
  route: Route;
  onNavigate: (route: Route) => void;
  emulators: EmulatorProfile[];
  conflicts: Conflict[];
  pendingOps: PendingOp[];
  sync: SyncState;
  issueCount: number;
  deviceName: string | null;
  provider: ProviderKind | null;
  email: string | null;
  onAddEmulator: () => void;
}

export function Sidebar({
  hidden,
  route,
  onNavigate,
  emulators,
  conflicts,
  pendingOps,
  sync,
  issueCount,
  deviceName,
  provider,
  email,
  onAddEmulator,
}: SidebarProps) {
  const { t } = useTranslation();

  return (
    <aside className="sidebar" aria-label={t("nav.sidebar")} inert={hidden}>
      <div className="sidebar-header">
        <img src={logo} alt="" width={22} height={22} className="sidebar-logo" />
        <span className="sidebar-app-name">Slot2Sync</span>
      </div>

      <nav className="sidebar-scroll" aria-label={t("nav.sidebar")}>
        <ul role="list" className="sidebar-list">
          <li>
            <SidebarItem
              icon="grid"
              label={t("nav.overview")}
              current={route.name === "overview"}
              shortcut={SHORTCUTS.overview}
              onClick={() => onNavigate({ name: "overview" })}
            />
          </li>
          <li>
            <SidebarItem
              icon="activity"
              label={t("nav.activity")}
              current={route.name === "activity"}
              shortcut={SHORTCUTS.activity}
              onClick={() => onNavigate({ name: "activity" })}
              trailing={
                issueCount > 0 ? (
                  <span
                    className="sidebar-count tabular"
                    aria-label={t("nav.issues", { count: issueCount })}
                  >
                    {issueCount}
                  </span>
                ) : null
              }
            />
          </li>
        </ul>

        <div className="sidebar-section-header">
          <h2 className="sidebar-section-title">{t("nav.emulators")}</h2>
          <Button
            variant="plain"
            size="small"
            icon="plus"
            aria-label={t("nav.addEmulator")}
            title={`${t("nav.addEmulator")} (${shortcutLabel(SHORTCUTS.addEmulator)})`}
            aria-keyshortcuts={ariaShortcut(SHORTCUTS.addEmulator)}
            onClick={onAddEmulator}
          />
        </div>

        <ul role="list" className="sidebar-list">
          {emulators.map((profile) => (
            <li key={profile.name}>
              <SidebarEmulatorItem
                profile={profile}
                current={route.name === "emulator" && route.emulator === profile.name}
                running={sync.running.has(profile.name)}
                syncing={sync.progress?.emulator === profile.name}
                conflicts={conflicts.filter((c) => c.emulator === profile.name).length}
                pendingOps={pendingOps.filter((op) => op.emulator === profile.name)}
                onClick={() => onNavigate({ name: "emulator", emulator: profile.name })}
              />
            </li>
          ))}
          {emulators.length === 0 ? (
            <li>
              <SidebarItem icon="plus" label={t("nav.addEmulator")} muted onClick={onAddEmulator} />
            </li>
          ) : null}
        </ul>
      </nav>

      <div className="sidebar-footer">
        <SidebarItem
          icon="settings"
          label={t("nav.settings")}
          current={route.name === "settings"}
          shortcut={SHORTCUTS.settings}
          onClick={() => onNavigate({ name: "settings" })}
        />
        <div className="sidebar-account">
          <Icon name="laptop" size={16} />
          <div className="sidebar-account-text">
            <span className="truncate">{deviceName ?? t("nav.thisDevice")}</span>
            {provider ? (
              <span className="sidebar-account-detail truncate" title={email ?? undefined}>
                {providerLabel(provider, t)}
                {email ? ` · ${email}` : ""}
              </span>
            ) : null}
          </div>
        </div>
      </div>
    </aside>
  );
}

interface SidebarItemProps {
  icon: IconName;
  label: string;
  current?: boolean;
  onClick: () => void;
  trailing?: ReactNode;
  shortcut?: Shortcut;
  muted?: boolean;
  ariaLabel?: string;
}

function SidebarItem({
  icon,
  label,
  current,
  onClick,
  trailing,
  shortcut,
  muted,
  ariaLabel,
}: SidebarItemProps) {
  return (
    <button
      type="button"
      className={cx("sidebar-item", muted && "sidebar-item-muted")}
      aria-current={current ? "page" : undefined}
      aria-label={ariaLabel}
      aria-keyshortcuts={shortcut ? ariaShortcut(shortcut) : undefined}
      title={shortcut ? `${label} (${shortcutLabel(shortcut)})` : undefined}
      onClick={onClick}
    >
      <Icon name={icon} size={16} className="sidebar-item-icon" />
      <span className="sidebar-item-label truncate">{label}</span>
      {trailing}
    </button>
  );
}

interface SidebarEmulatorItemProps {
  profile: EmulatorProfile;
  current: boolean;
  running: boolean;
  syncing: boolean;
  conflicts: number;
  pendingOps: PendingOp[];
  onClick: () => void;
}

function SidebarEmulatorItem({
  profile,
  current,
  running,
  syncing,
  conflicts,
  pendingOps,
  onClick,
}: SidebarEmulatorItemProps) {
  const { t } = useTranslation();
  const categories = useEmulatorCategories(profile.name);
  const status = emulatorStatus({ running, syncing, conflicts, pendingOps, categories });
  const presentation = STATUS_PRESENTATION[status.kind];
  const statusText = t(presentation.labelKey);
  const quiet = status.kind === "idle";

  return (
    <SidebarItem
      icon="gamepad"
      label={profile.name}
      current={current}
      onClick={onClick}
      ariaLabel={`${profile.name}, ${statusText}`}
      trailing={
        quiet ? null : (
          <span className="sidebar-status" data-tone={presentation.tone} title={statusText}>
            <Icon
              name={presentation.icon}
              size={13}
              className={status.kind === "syncing" ? "icon-spin" : undefined}
            />
          </span>
        )
      }
    />
  );
}
