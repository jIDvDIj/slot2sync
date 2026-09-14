import { useTranslation } from "react-i18next";

import { AddEmulatorTile, EmulatorTile, SkeletonTile } from "../components/overview/EmulatorTile";
import { RecentGames } from "../components/overview/RecentGames";
import { Banner } from "../components/ui/Banner";
import { Button } from "../components/ui/Button";
import { EmptyState } from "../components/ui/EmptyState";
import { PageHeader } from "../components/ui/PageHeader";
import { useNow } from "../hooks/useNow";
import type { SyncState } from "../hooks/useSyncEvents";
import { formatRelativeTime } from "../lib/time";
import type { Conflict, EmulatorProfile, PendingOp, SyncedGame } from "../types/ipc";

import "../components/overview/Overview.css";

export interface OverviewPageProps {
  emulators: EmulatorProfile[];
  loading: boolean;
  error: string | null;
  sync: SyncState;
  conflicts: Conflict[];
  pendingOps: PendingOp[];
  games: SyncedGame[];
  onOpenEmulator: (name: string) => void;
  onAddEmulator: () => void;
  onOpenActivity: () => void;
}

const SKELETON_COUNT = 3;

export function OverviewPage({
  emulators,
  loading,
  error,
  sync,
  conflicts,
  pendingOps,
  games,
  onOpenEmulator,
  onAddEmulator,
  onOpenActivity,
}: OverviewPageProps) {
  const { t } = useTranslation();
  const now = useNow();

  const failedCount = pendingOps.filter((op) => op.nextRetryAtMs === null).length;
  const attentionCount = conflicts.length + failedCount;
  const attentionParts = [
    conflicts.length > 0 ? t("overview.conflicts", { count: conflicts.length }) : null,
    failedCount > 0 ? t("overview.failedTransfers", { count: failedCount }) : null,
  ].filter(Boolean);

  const subtitle = sync.lastSync
    ? t("syncBar.lastSynced", { when: formatRelativeTime(t, sync.lastSync.atMs, now) })
    : t("syncBar.neverSynced");

  const showEmpty = !loading && !error && emulators.length === 0;

  return (
    <div className="overview">
      <div>
        <PageHeader
          title={t("nav.overview")}
          subtitle={subtitle}
          accessory={
            emulators.length > 0 ? (
              <Button variant="bordered" icon="plus" onClick={onAddEmulator}>
                {t("nav.addEmulator")}
              </Button>
            ) : null
          }
        />
        <div className="overview-banners">
          {attentionCount > 0 ? (
            <Banner
              tone={conflicts.length > 0 ? "danger" : "warning"}
              title={t("nav.issues", { count: attentionCount })}
              actions={
                <Button variant="bordered" size="small" onClick={onOpenActivity}>
                  {t("overview.review")}
                </Button>
              }
            >
              {attentionParts.join(" · ")}
            </Banner>
          ) : null}
          {error ? (
            <Banner tone="danger" title={t("overview.loadErrorTitle")}>
              {error}
            </Banner>
          ) : null}
        </div>
      </div>

      {showEmpty ? (
        <EmptyState
          icon="gamepad"
          title={t("overview.emptyTitle")}
          message={t("overview.emptyMessage")}
          action={
            <Button variant="prominent" icon="plus" onClick={onAddEmulator}>
              {t("nav.addEmulator")}
            </Button>
          }
        />
      ) : (
        <section className="overview-section" aria-labelledby="overview-emulators-heading">
          <h2 id="overview-emulators-heading" className="overview-section-title">
            {t("overview.emulatorsHeading")}
          </h2>
          <div className="emulator-grid" aria-busy={loading || undefined}>
            {loading
              ? Array.from({ length: SKELETON_COUNT }, (_, index) => <SkeletonTile key={index} />)
              : emulators.map((profile) => (
                  <EmulatorTile
                    key={profile.name}
                    profile={profile}
                    running={sync.running.has(profile.name)}
                    progress={sync.progress?.emulator === profile.name ? sync.progress : null}
                    conflicts={conflicts.filter((c) => c.emulator === profile.name).length}
                    pendingOps={pendingOps.filter((op) => op.emulator === profile.name)}
                    gameCount={games.filter((g) => g.emulator === profile.name).length}
                    now={now}
                    onOpen={() => onOpenEmulator(profile.name)}
                  />
                ))}
            {!loading ? <AddEmulatorTile onClick={onAddEmulator} /> : null}
          </div>
        </section>
      )}

      <RecentGames games={games} now={now} onOpenEmulator={onOpenEmulator} />
    </div>
  );
}
