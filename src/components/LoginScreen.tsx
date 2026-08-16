import { useCallback, useState } from "react";
import { Trans, useTranslation } from "react-i18next";
import { open as openDialog } from "@tauri-apps/plugin-dialog";

import { useErrorMessage } from "../lib/errors";
import {
  connectDropbox,
  connectGoogleDrive,
  connectLocalFolder,
  connectOneDrive,
  setDeviceName,
} from "../lib/ipc";
import { providerLabel } from "../lib/providerLabels";
import { usePlatform } from "../hooks/usePlatform";
import type { Theme } from "../hooks/useTheme";
import type { AuthStatus, ProviderKind } from "../types/ipc";
import { Button } from "./ui/Button";
import { Card } from "./ui/Card";

interface Props {
  /** Nome do dispositivo já salvo, usado para pré-preencher o campo. */
  initialDeviceName: string | null;
  /** Chamado com o novo status após o login concluir com sucesso. */
  onConnected: (status: AuthStatus) => void;
  theme: Theme;
  onToggleTheme: () => void;
}

const OAUTH_PROVIDERS = ["google_drive", "dropbox", "one_drive"] as const;

const OAUTH_CONNECT: Record<(typeof OAUTH_PROVIDERS)[number], () => Promise<AuthStatus>> = {
  google_drive: connectGoogleDrive,
  dropbox: connectDropbox,
  one_drive: connectOneDrive,
};

/**
 * Provedores com o backend pronto, mas ainda sem credenciais cadastradas nos
 * consoles externos ficam visíveis e desativados em vez de somem, sinalizando o que já está a
 * caminho sem deixar o usuário cair num fluxo OAuth que só falharia.
 */
const UNAVAILABLE_PROVIDERS = new Set<(typeof OAUTH_PROVIDERS)[number]>(["dropbox", "one_drive"]);

/**
 * Tela de login dedicada. É a única coisa renderizada enquanto o usuário não
 * está conectado — a tela principal só aparece depois que o login conclui.
 *
 * O nome do dispositivo é obrigatório: identifica esta máquina nos metadados de
 * sync no provedor escolhido e é gravado antes de concluir a autenticação.
 */
export function LoginScreen({ initialDeviceName, onConnected, theme, onToggleTheme }: Props) {
  const { t } = useTranslation();
  const errorMessage = useErrorMessage();
  const { isMobile } = usePlatform();
  const [device, setDevice] = useState(initialDeviceName ?? "");
  const [provider, setProvider] = useState<ProviderKind>("google_drive");
  const [folderPath, setFolderPath] = useState("");
  const [connecting, setConnecting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Pré-preenche com o nome já salvo, sem sobrescrever o que o usuário digita.
  // Ajuste durante o render (em vez de useEffect): initialDeviceName pode
  // chegar depois da primeira renderização (carregado de forma assíncrona).
  const [prevInitialDeviceName, setPrevInitialDeviceName] = useState(initialDeviceName);
  if (initialDeviceName !== prevInitialDeviceName) {
    setPrevInitialDeviceName(initialDeviceName);
    setDevice((cur) => cur || initialDeviceName || "");
  }

  const pickFolder = useCallback(async () => {
    const selected = await openDialog({ directory: true, multiple: false });
    if (typeof selected === "string") {
      setFolderPath(selected);
    }
  }, []);

  const handleConnect = useCallback(async () => {
    const name = device.trim();
    if (!name) return;
    setConnecting(true);
    setError(null);
    try {
      await setDeviceName(name);
      if (provider === "local_folder") {
        onConnected(await connectLocalFolder(folderPath.trim()));
      } else {
        onConnected(await OAUTH_CONNECT[provider]());
      }
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setConnecting(false);
    }
  }, [device, folderPath, provider, onConnected, errorMessage]);

  const canConnect =
    device.trim().length > 0 &&
    !connecting &&
    (provider !== "local_folder" || folderPath.trim().length > 0);

  return (
    <main className="login-screen">
      <Button variant="secondary" size="sm" className="theme-toggle" onClick={onToggleTheme}>
        {theme === "dark" ? t("app.switchToLightTheme") : t("app.switchToDarkTheme")}
      </Button>
      <Card as="div" padding="lg" className="login-card">
        <h1>Slot2Sync</h1>
        <p className="login-tagline">{t("login.tagline")}</p>

        <div className="field">
          <span>{t("login.providerLabel")}</span>
          <div className="provider-picker">
            {OAUTH_PROVIDERS.map((kind) => {
              const unavailable = UNAVAILABLE_PROVIDERS.has(kind);
              return (
                <Button
                  key={kind}
                  type="button"
                  variant={provider === kind ? "primary" : "secondary"}
                  size="sm"
                  disabled={unavailable}
                  title={unavailable ? t("login.comingSoon") : undefined}
                  onClick={() => setProvider(kind)}
                >
                  {providerLabel(kind, t)}
                  {unavailable ? <span className="muted"> ({t("login.comingSoon")})</span> : null}
                </Button>
              );
            })}
            {!isMobile ? (
              <Button
                type="button"
                variant={provider === "local_folder" ? "primary" : "secondary"}
                size="sm"
                onClick={() => setProvider("local_folder")}
              >
                {providerLabel("local_folder", t)}
              </Button>
            ) : null}
          </div>
        </div>

        {provider === "local_folder" ? (
          <label className="field">
            <span>{t("login.folderPathLabel")}</span>
            <div className="folder-path-row">
              <input
                type="text"
                value={folderPath}
                onChange={(e) => setFolderPath(e.target.value)}
                placeholder={t("login.folderPathPlaceholder")}
                disabled={connecting}
              />
              <Button type="button" variant="secondary" size="sm" onClick={() => void pickFolder()}>
                {t("login.selectFolder")}
              </Button>
            </div>
          </label>
        ) : (
          <p className="permission-note">
            <Trans i18nKey="login.permissionNote" components={{ strong: <strong /> }} />
          </p>
        )}

        <label className="field">
          <span>{t("device.nameLabel")}</span>
          <input
            type="text"
            value={device}
            onChange={(e) => setDevice(e.target.value)}
            placeholder={t("device.namePlaceholder")}
            disabled={connecting}
            maxLength={60}
            autoFocus
          />
        </label>

        <Button
          variant="primary"
          fullWidth
          onClick={() => void handleConnect()}
          disabled={!canConnect}
        >
          {connecting
            ? t("login.connecting")
            : provider === "local_folder"
              ? t("login.connectFolder")
              : t("login.connect", { provider: providerLabel(provider, t) })}
        </Button>

        {error ? <p className="error">{error}</p> : null}
      </Card>
    </main>
  );
}
