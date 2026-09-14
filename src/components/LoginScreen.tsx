import { useCallback, useId, useState, type FormEvent } from "react";
import { Trans, useTranslation } from "react-i18next";
import { open as openDialog } from "@tauri-apps/plugin-dialog";

import logo from "../assets/logo.png";
import { usePlatform } from "../hooks/usePlatform";
import { useErrorMessage } from "../lib/errors";
import {
  connectDropbox,
  connectGoogleDrive,
  connectLocalFolder,
  connectOneDrive,
  setDeviceName,
} from "../lib/ipc";
import { providerLabel } from "../lib/providerLabels";
import type { AuthStatus, ProviderKind } from "../types/ipc";
import { Button } from "./ui/Button";
import { Field, InlineStatus, TextField } from "./ui/Form";
import { Icon } from "./ui/Icon";
import { Spinner } from "./ui/Spinner";
import { StatusLabel } from "./ui/StatusLabel";

import "./LoginScreen.css";

export interface LoginScreenProps {
  /** Device name already saved, used to prefill the field. */
  initialDeviceName: string | null;
  /** Called with the new status once sign-in completes. */
  onConnected: (status: AuthStatus) => void;
}

const OAUTH_PROVIDERS = ["google_drive", "dropbox", "one_drive"] as const;
type OAuthProvider = (typeof OAUTH_PROVIDERS)[number];

const OAUTH_CONNECT: Record<OAuthProvider, () => Promise<AuthStatus>> = {
  google_drive: connectGoogleDrive,
  dropbox: connectDropbox,
  one_drive: connectOneDrive,
};

/**
 * Backend support exists but the external consoles have no credentials yet:
 * shown disabled so people see what's coming without entering an OAuth flow that would fail.
 */
const UNAVAILABLE_PROVIDERS = new Set<OAuthProvider>(["dropbox", "one_drive"]);

export function LoginScreen({ initialDeviceName, onConnected }: LoginScreenProps) {
  const { t } = useTranslation();
  const errorMessage = useErrorMessage();
  const { isMobile } = usePlatform();
  const [device, setDevice] = useState(initialDeviceName ?? "");
  const [provider, setProvider] = useState<ProviderKind>("google_drive");
  const [folderPath, setFolderPath] = useState("");
  const [connecting, setConnecting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const folderId = useId();
  const deviceId = useId();
  const pickerLabelId = useId();

  // The saved name can arrive after the first render; adjust during render without overwriting typing.
  const [prevInitialDeviceName, setPrevInitialDeviceName] = useState(initialDeviceName);
  if (initialDeviceName !== prevInitialDeviceName) {
    setPrevInitialDeviceName(initialDeviceName);
    setDevice((current) => current || initialDeviceName || "");
  }

  const isFolder = provider === "local_folder";

  const pickFolder = useCallback(async () => {
    const selected = await openDialog({
      directory: true,
      multiple: false,
      title: t("login.folderPickerTitle"),
    });
    if (typeof selected === "string") setFolderPath(selected);
  }, [t]);

  const canConnect =
    device.trim().length > 0 && !connecting && (!isFolder || folderPath.trim().length > 0);

  const handleSubmit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    const name = device.trim();
    if (!canConnect) return;
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
  };

  const options: { kind: ProviderKind; hint: string; unavailable: boolean }[] = [
    ...OAUTH_PROVIDERS.map((kind) => ({
      kind,
      hint: kind === "google_drive" ? t("login.googleDriveHint") : t("login.cloudHint"),
      unavailable: UNAVAILABLE_PROVIDERS.has(kind),
    })),
    ...(isMobile
      ? []
      : [{ kind: "local_folder" as const, hint: t("login.localFolderHint"), unavailable: false }]),
  ];

  return (
    <main className="login">
      <div className="login-card">
        <header className="login-header">
          <span className="login-mark">
            <img src={logo} alt="" width={72} height={72} />
          </span>
          <h1 className="login-title">Slot2Sync</h1>
          <p className="login-tagline">{t("login.tagline")}</p>
        </header>

        <form className="login-form" onSubmit={(event) => void handleSubmit(event)}>
          <fieldset className="login-providers" disabled={connecting}>
            <legend id={pickerLabelId} className="field-label">
              {t("login.providerLabel")}
            </legend>
            <div className="login-provider-group" role="radiogroup" aria-labelledby={pickerLabelId}>
              {options.map(({ kind, hint, unavailable }) => (
                <label
                  key={kind}
                  className="login-provider"
                  data-selected={provider === kind || undefined}
                  data-unavailable={unavailable || undefined}
                >
                  <input
                    type="radio"
                    name="provider"
                    className="login-provider-input"
                    value={kind}
                    checked={provider === kind}
                    disabled={unavailable}
                    onChange={() => setProvider(kind)}
                  />
                  <Icon
                    name={kind === "local_folder" ? "folder" : "cloud"}
                    size={20}
                    className="login-provider-icon"
                  />
                  <span className="login-provider-text">
                    <span className="login-provider-name">{providerLabel(kind, t)}</span>
                    <span className="login-provider-hint">{hint}</span>
                  </span>
                  {unavailable ? (
                    <StatusLabel icon="clock">{t("login.comingSoon")}</StatusLabel>
                  ) : (
                    <Icon name="check" size={16} className="login-provider-check" />
                  )}
                </label>
              ))}
            </div>
          </fieldset>

          {isFolder ? (
            <Field label={t("login.folderPathLabel")} htmlFor={folderId}>
              <div className="login-folder-row">
                <TextField
                  id={folderId}
                  value={folderPath}
                  onChange={(event) => setFolderPath(event.target.value)}
                  placeholder={t("login.folderPathPlaceholder")}
                  disabled={connecting}
                  spellCheck={false}
                />
                <Button icon="folder" disabled={connecting} onClick={() => void pickFolder()}>
                  {t("login.chooseFolder")}
                </Button>
              </div>
            </Field>
          ) : null}

          <Field label={t("device.nameLabel")} htmlFor={deviceId} hint={t("login.deviceHint")}>
            <TextField
              id={deviceId}
              value={device}
              onChange={(event) => setDevice(event.target.value)}
              placeholder={t("device.namePlaceholder")}
              disabled={connecting}
              maxLength={60}
              required
              autoFocus
            />
          </Field>

          {isFolder ? null : (
            <p className="login-privacy">
              <Icon name="lock" size={14} className="login-privacy-icon" />
              <span>
                <Trans i18nKey="login.permissionNote" components={{ strong: <strong /> }} />
              </span>
            </p>
          )}

          <div className="login-actions">
            <Button
              type="submit"
              variant="prominent"
              size="large"
              fullWidth
              loading={connecting}
              disabled={!canConnect}
            >
              {isFolder
                ? t("login.connectFolder")
                : t("login.continueWith", { provider: providerLabel(provider, t) })}
            </Button>
            {connecting ? (
              <p className="login-waiting" role="status">
                <Spinner size="small" />
                {isFolder ? t("login.connectingFolder") : t("login.waitingForBrowser")}
              </p>
            ) : null}
            {error ? <InlineStatus tone="danger">{error}</InlineStatus> : null}
          </div>
        </form>
      </div>
    </main>
  );
}
