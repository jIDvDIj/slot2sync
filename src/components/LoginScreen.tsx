import { useCallback, useState } from "react";
import { Trans, useTranslation } from "react-i18next";

import { useErrorMessage } from "../lib/errors";
import { connectGoogleDrive, setDeviceName } from "../lib/ipc";
import type { Theme } from "../hooks/useTheme";
import type { AuthStatus } from "../types/ipc";
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

/**
 * Tela de login dedicada. É a única coisa renderizada enquanto o usuário não
 * está conectado — a tela principal só aparece depois que o login conclui.
 *
 * O nome do dispositivo é obrigatório: identifica esta máquina nos metadados de
 * sync no Drive e é gravado antes de concluir a autenticação.
 */
export function LoginScreen({ initialDeviceName, onConnected, theme, onToggleTheme }: Props) {
  const { t } = useTranslation();
  const errorMessage = useErrorMessage();
  const [device, setDevice] = useState(initialDeviceName ?? "");
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

  const handleConnect = useCallback(async () => {
    const name = device.trim();
    if (!name) return;
    setConnecting(true);
    setError(null);
    try {
      await setDeviceName(name);
      onConnected(await connectGoogleDrive());
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      setConnecting(false);
    }
  }, [device, onConnected]);

  const canConnect = device.trim().length > 0 && !connecting;

  return (
    <main className="login-screen">
      <Button variant="secondary" size="sm" className="theme-toggle" onClick={onToggleTheme}>
        {theme === "dark" ? t("app.switchToLightTheme") : t("app.switchToDarkTheme")}
      </Button>
      <Card as="div" padding="lg" className="login-card">
        <h1>Slot2Sync</h1>
        <p className="login-tagline">{t("login.tagline")}</p>

        <p className="permission-note">
          <Trans i18nKey="login.permissionNote" components={{ strong: <strong /> }} />
        </p>

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

        <Button variant="primary" fullWidth onClick={handleConnect} disabled={!canConnect}>
          {connecting ? t("login.connecting") : t("login.connect")}
        </Button>

        {error ? <p className="error">{error}</p> : null}
      </Card>
    </main>
  );
}
