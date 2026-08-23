import { useState } from "react";
import { useTranslation } from "react-i18next";

import type { AppPanicPayload } from "./types/ipc";

import { AccountStatus } from "./components/AccountStatus";
import { AddEmulator } from "./components/AddEmulator";
import { EmulatorCard } from "./components/EmulatorCard";
import { LoginScreen } from "./components/LoginScreen";
import { SettingsModal } from "./components/SettingsModal";
import { SyncStatus } from "./components/SyncStatus";
import { Button } from "./components/ui/Button";
import { useAppPanic } from "./hooks/useAppPanic";
import { useAuth } from "./hooks/useAuth";
import { useConflicts } from "./hooks/useConflicts";
import { useEmulators } from "./hooks/useEmulators";
import { usePendingOps } from "./hooks/usePendingOps";
import { useSettings } from "./hooks/useSettings";
import { useSyncedGames } from "./hooks/useSyncedGames";
import { useSyncEvents } from "./hooks/useSyncEvents";
import { useTheme } from "./hooks/useTheme";
import "./App.css";

function App() {
  const { t } = useTranslation();
  const auth = useAuth();
  const { settings, reload: reloadSettings } = useSettings();
  const theme = useTheme();
  const { panic, dismiss: dismissPanic } = useAppPanic();

  const panicBanner = panic ? <PanicBanner panic={panic} onDismiss={dismissPanic} /> : null;

  // Enquanto o status de auth não chega, não decide qual tela mostrar.
  if (auth.loading) {
    return (
      <main className="login-screen">
        {panicBanner}
        <p className="muted">{t("app.checkingConnection")}</p>
      </main>
    );
  }

  // Sem login, a única tela acessível é a de login.
  if (!auth.connected) {
    return (
      <>
        {panicBanner}
        <LoginScreen
          initialDeviceName={settings?.deviceName ?? null}
          onConnected={(status) => {
            auth.setStatus(status);
            reloadSettings();
          }}
          theme={theme.theme}
          onToggleTheme={theme.toggle}
        />
      </>
    );
  }

  return (
    <>
      {panicBanner}
      <MainScreen auth={auth} settings={settings} reloadSettings={reloadSettings} theme={theme} />
    </>
  );
}

interface PanicBannerProps {
  panic: AppPanicPayload;
  onDismiss: () => void;
}

/** Aviso persistente de panic — o backend segue vivo, mas algo quebrou. */
function PanicBanner({ panic, onDismiss }: PanicBannerProps) {
  const { t } = useTranslation();
  return (
    <div className="panic-banner" role="alert">
      <div>
        <strong>{t("panic.title")}</strong>
        <p>{t("panic.body")}</p>
        <code>
          {panic.message}
          {panic.location ? ` (${t("panic.at")} ${panic.location})` : ""}
        </code>
      </div>
      <Button variant="secondary" size="sm" onClick={onDismiss}>
        {t("common.dismiss")}
      </Button>
    </div>
  );
}

interface MainScreenProps {
  auth: ReturnType<typeof useAuth>;
  settings: ReturnType<typeof useSettings>["settings"];
  reloadSettings: () => void;
  theme: ReturnType<typeof useTheme>;
}

/**
 * Tela principal — só montada quando o usuário está conectado. Os hooks de
 * emuladores/sync/conflitos vivem aqui para não rodar na tela de login.
 */
function MainScreen({ auth, settings, reloadSettings, theme }: MainScreenProps) {
  const { t } = useTranslation();
  const sync = useSyncEvents();
  const { emulators, loading, error, refresh, remove } = useEmulators();
  const { conflicts, reload: reloadConflicts } = useConflicts();
  const { ops: pendingOps } = usePendingOps();
  const games = useSyncedGames();
  const [showSettings, setShowSettings] = useState(false);

  return (
    <main className="app">
      <header className="app-header">
        <h1>Slot2Sync</h1>
        <div className="header-actions">
          <AccountStatus
            email={auth.status?.email ?? null}
            deviceName={settings?.deviceName ?? null}
            onDisconnect={auth.disconnect}
            error={auth.error}
          />
          <Button variant="secondary" size="sm" onClick={theme.toggle}>
            {theme.theme === "dark" ? t("app.switchToLightTheme") : t("app.switchToDarkTheme")}
          </Button>
          <Button variant="secondary" size="sm" onClick={() => setShowSettings(true)}>
            {t("app.settings")}
          </Button>
        </div>
      </header>

      <section className="emulators">
        <div className="section-head">
          <h2>{t("app.emulators")}</h2>
          <AddEmulator onAdded={refresh} existingNames={emulators.map((e) => e.name)} />
        </div>

        {loading ? (
          <p className="muted">{t("app.loading")}</p>
        ) : error ? (
          <p className="error">{error}</p>
        ) : emulators.length === 0 ? (
          <p className="muted empty">{t("app.noEmulators")}</p>
        ) : (
          <div className="emulator-grid">
            {emulators.map((profile) => (
              <EmulatorCard
                key={profile.name}
                profile={profile}
                running={sync.running.has(profile.name)}
                conflicts={conflicts.filter((c) => c.emulator === profile.name)}
                pendingOps={pendingOps.filter((op) => op.emulator === profile.name)}
                progress={sync.progress}
                trigger={sync.trigger}
                games={games.filter((g) => g.emulator === profile.name)}
                onRemove={remove}
                onConflictResolved={reloadConflicts}
              />
            ))}
          </div>
        )}
      </section>

      <SyncStatus state={sync} />

      {showSettings && settings ? (
        <SettingsModal
          settings={settings}
          emulators={emulators}
          onClose={() => setShowSettings(false)}
          onSaved={reloadSettings}
          onDisconnectProvider={() => {
            setShowSettings(false);
            void auth.disconnect();
          }}
        />
      ) : null}
    </main>
  );
}

export default App;
