import { useCallback, useRef, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";

import logo from "./assets/logo.png";
import { AddEmulatorDialog } from "./components/AddEmulatorDialog";
import { LoginScreen } from "./components/LoginScreen";
import { Spinner } from "./components/ui/Spinner";
import { useAppearance } from "./hooks/useAppearance";
import { useAppPanic } from "./hooks/useAppPanic";
import { useAuth } from "./hooks/useAuth";
import { useConflicts } from "./hooks/useConflicts";
import { useEmulators } from "./hooks/useEmulators";
import { useMediaQuery } from "./hooks/useMediaQuery";
import { usePendingOps } from "./hooks/usePendingOps";
import { useSettings } from "./hooks/useSettings";
import { useShortcut } from "./hooks/useShortcut";
import { useSyncedGames } from "./hooks/useSyncedGames";
import { useSyncEvents } from "./hooks/useSyncEvents";
import { useUpdate } from "./hooks/useUpdate";
import { routeKey, type Route } from "./lib/navigation";
import { ActivityPage } from "./pages/ActivityPage";
import { EmulatorPage } from "./pages/EmulatorPage";
import { OverviewPage } from "./pages/OverviewPage";
import { SettingsPage } from "./pages/SettingsPage";
import { GlobalBanners } from "./shell/GlobalBanners";
import { MainBanners } from "./shell/MainBanners";
import { SHORTCUTS } from "./shell/shortcuts";
import { Sidebar } from "./shell/Sidebar";
import { SyncActivity } from "./shell/SyncActivity";
import { TabBar } from "./shell/TabBar";
import { Toolbar } from "./shell/Toolbar";
import { usePersistentFlag } from "./shell/usePersistentFlag";
import { useSyncAction } from "./shell/useSyncAction";

import "./shell/Shell.css";

function App() {
  const { t } = useTranslation();
  const auth = useAuth();
  const { settings, reload: reloadSettings } = useSettings();
  const appearance = useAppearance();
  const { panic, dismiss: dismissPanic } = useAppPanic();
  const { update, dismiss: dismissUpdate } = useUpdate();

  const banners = (
    <GlobalBanners
      panic={panic}
      onDismissPanic={dismissPanic}
      update={update}
      onDismissUpdate={dismissUpdate}
    />
  );

  if (auth.loading) {
    return (
      <main className="launch-screen">
        <img src={logo} alt="" width={64} height={64} className="launch-logo" />
        <div className="launch-status">
          <Spinner />
          <p className="text-secondary">{t("app.checkingConnection")}</p>
        </div>
      </main>
    );
  }

  if (!auth.connected) {
    return (
      <>
        <div className="floating-banners">{banners}</div>
        <LoginScreen
          initialDeviceName={settings?.deviceName ?? null}
          onConnected={(status) => {
            auth.setStatus(status);
            void reloadSettings();
          }}
        />
      </>
    );
  }

  return (
    <MainScreen
      auth={auth}
      settings={settings}
      reloadSettings={reloadSettings}
      appearance={appearance}
      banners={banners}
    />
  );
}

interface MainScreenProps {
  auth: ReturnType<typeof useAuth>;
  settings: ReturnType<typeof useSettings>["settings"];
  reloadSettings: () => Promise<void>;
  appearance: ReturnType<typeof useAppearance>;
  banners: ReactNode;
}

/** Mounted only once connected, so sync and emulator hooks never run on the sign-in screen. */
function MainScreen({ auth, settings, reloadSettings, appearance, banners }: MainScreenProps) {
  const { t } = useTranslation();
  const sync = useSyncEvents();
  const { emulators, loading, error, refresh, remove } = useEmulators();
  const { conflicts, reload: reloadConflicts } = useConflicts();
  const { ops: pendingOps, reload: reloadPending } = usePendingOps();
  const games = useSyncedGames();
  const syncAction = useSyncAction();
  const compact = useMediaQuery("(max-width: 699px)");
  const [sidebarHidden, setSidebarHidden] = usePersistentFlag("slot2sync.sidebarHidden");
  const [route, setRoute] = useState<Route>({ name: "overview" });
  const [addOpen, setAddOpen] = useState(false);
  const [scrolled, setScrolled] = useState(false);
  const scrollRef = useRef<HTMLDivElement>(null);

  const navigate = useCallback((next: Route) => {
    setRoute(next);
    setScrolled(false);
    scrollRef.current?.scrollTo({ top: 0 });
  }, []);

  const activeRoute: Route =
    route.name === "emulator" && !loading && !emulators.some((e) => e.name === route.emulator)
      ? { name: "overview" }
      : route;
  const selectedProfile =
    activeRoute.name === "emulator"
      ? (emulators.find((e) => e.name === activeRoute.emulator) ?? null)
      : null;

  const syncing = syncAction.busy || sync.phase === "syncing";
  const issueCount = conflicts.length + pendingOps.filter((op) => op.nextRetryAtMs === null).length;

  useShortcut(SHORTCUTS.sync, () => void syncAction.run());
  useShortcut(SHORTCUTS.settings, () => navigate({ name: "settings" }));
  useShortcut(SHORTCUTS.addEmulator, () => setAddOpen(true));
  useShortcut(SHORTCUTS.overview, () => navigate({ name: "overview" }));
  useShortcut(SHORTCUTS.activity, () => navigate({ name: "activity" }));

  const openEmulator = (name: string) => navigate({ name: "emulator", emulator: name });
  const openAddEmulator = () => setAddOpen(true);

  let title: string;
  let page: ReactNode;
  switch (activeRoute.name) {
    case "overview":
      title = t("nav.overview");
      page = (
        <OverviewPage
          emulators={emulators}
          loading={loading}
          error={error}
          sync={sync}
          conflicts={conflicts}
          pendingOps={pendingOps}
          games={games}
          onOpenEmulator={openEmulator}
          onAddEmulator={openAddEmulator}
          onOpenActivity={() => navigate({ name: "activity" })}
        />
      );
      break;
    case "emulator":
      title = activeRoute.emulator;
      page = selectedProfile ? (
        <EmulatorPage
          key={selectedProfile.name}
          profile={selectedProfile}
          running={sync.running.has(selectedProfile.name)}
          syncing={syncing}
          progress={sync.progress}
          trigger={sync.trigger}
          conflicts={conflicts.filter((c) => c.emulator === selectedProfile.name)}
          pendingOps={pendingOps.filter((op) => op.emulator === selectedProfile.name)}
          games={games.filter((g) => g.emulator === selectedProfile.name)}
          onRemove={async (name) => {
            await remove(name);
            navigate({ name: "overview" });
          }}
          onConflictResolved={reloadConflicts}
          onPendingChanged={reloadPending}
          onSyncNow={syncAction.run}
        />
      ) : (
        <PageLoading />
      );
      break;
    case "activity":
      title = t("nav.activity");
      page = (
        <ActivityPage
          sync={sync}
          emulators={emulators}
          conflicts={conflicts}
          pendingOps={pendingOps}
          onPendingChanged={reloadPending}
          onOpenEmulator={openEmulator}
          onSyncNow={syncAction.run}
        />
      );
      break;
    case "settings":
      title = t("nav.settings");
      page = settings ? (
        <SettingsPage
          settings={settings}
          onSaved={() => void reloadSettings()}
          onDisconnectProvider={() => void auth.disconnect()}
          appearance={appearance.appearance}
          onAppearanceChange={appearance.setAppearance}
        />
      ) : (
        <PageLoading />
      );
      break;
  }

  return (
    <div
      className="app-shell"
      data-compact={compact || undefined}
      data-sidebar={!compact && sidebarHidden ? "hidden" : undefined}
    >
      {!compact ? (
        <Sidebar
          hidden={sidebarHidden}
          route={activeRoute}
          onNavigate={navigate}
          emulators={emulators}
          conflicts={conflicts}
          pendingOps={pendingOps}
          sync={sync}
          issueCount={issueCount}
          deviceName={settings?.deviceName ?? null}
          provider={settings?.storageProvider ?? null}
          email={auth.status?.email ?? null}
          onAddEmulator={openAddEmulator}
        />
      ) : null}

      <div className="main-column">
        <div
          className="main-scroll"
          ref={scrollRef}
          onScroll={(event) => setScrolled(event.currentTarget.scrollTop > 24)}
        >
          <Toolbar
            title={title}
            scrolled={scrolled}
            compact={compact}
            sidebarHidden={sidebarHidden}
            onToggleSidebar={() => setSidebarHidden(!sidebarHidden)}
            onBack={
              compact && activeRoute.name === "emulator"
                ? () => navigate({ name: "overview" })
                : undefined
            }
            syncing={syncing}
            progress={sync.progress}
            lastSync={sync.lastSync}
            onSync={() => void syncAction.run()}
          />

          <main className="content">
            <div className="banner-stack">
              {banners}
              <MainBanners
                sync={sync}
                actionError={syncAction.error}
                onDismissActionError={syncAction.dismissError}
                onRetrySync={() => void syncAction.run()}
              />
            </div>
            {/* The emulator page renders its own progress for that emulator. */}
            <SyncActivity
              syncing={
                syncing &&
                !(
                  activeRoute.name === "emulator" &&
                  sync.progress?.emulator === activeRoute.emulator
                )
              }
              progress={sync.progress}
              trigger={sync.trigger}
            />
            <div className="page-transition" key={routeKey(activeRoute)}>
              {page}
            </div>
          </main>
        </div>

        {compact ? (
          <TabBar route={activeRoute} onNavigate={navigate} issueCount={issueCount} />
        ) : null}
      </div>

      <AddEmulatorDialog
        open={addOpen}
        existingNames={emulators.map((e) => e.name)}
        onClose={() => setAddOpen(false)}
        onAdded={() => void refresh()}
      />
    </div>
  );
}

function PageLoading() {
  return (
    <div className="page-loading">
      <Spinner size="large" />
    </div>
  );
}

export default App;
