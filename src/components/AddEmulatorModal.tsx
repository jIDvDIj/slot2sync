import { useCallback, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";

import { open as openDialog } from "@tauri-apps/plugin-dialog";

import { useDiscovery } from "../hooks/useDiscovery";
import { usePlatform } from "../hooks/usePlatform";
import { useErrorMessage } from "../lib/errors";
import {
  addEmulator,
  addEmulatorManual,
  detectEmulator,
  detectEmulatorMobile,
  pickEmulatorFolder,
} from "../lib/ipc";
import type { DiscoveredEmulator, EmulatorProfile } from "../types/ipc";
import { Modal } from "./ui/Modal";

interface Props {
  /** Emuladores já configurados — filtrados das recomendações. */
  existingNames: string[];
  onClose: () => void;
  /** Chamado após cada adição bem-sucedida (recarrega a lista no App). */
  onAdded: () => void;
}

/** Chave de tradução do rótulo curto da origem de uma sugestão com saves. */
const SOURCE_LABEL_KEY = {
  dataDir: "addEmulator.sourceSavesFound",
  both: "addEmulator.sourceSavesFound",
  registry: "addEmulator.sourceInstalled",
} as const satisfies Record<DiscoveredEmulator["source"], string>;

/** Caminho de `child` relativo a `root`, ou `null` se não estiver sob a raiz. */
function relativeUnder(root: string, child: string): string | null {
  const trim = (s: string) => s.replace(/[\\/]+$/, "");
  const r = trim(root);
  const c = trim(child);
  if (c === r) return "";
  if (c.startsWith(`${r}/`) || c.startsWith(`${r}\\`)) {
    return c.slice(r.length + 1);
  }
  return null;
}

/** Caminhos padrão por nome de emulador (mobile). */
function defaultPaths(name: string): { saves: string; states: string; config: string } {
  const n = name.toLowerCase();
  if (n.includes("ppsspp")) {
    return { saves: "PSP/SAVEDATA", states: "PSP/PPSSPP_STATE", config: "PSP/SYSTEM" };
  }
  if (n.includes("pcsx2")) {
    return { saves: "memcards", states: "sstates", config: "inis" };
  }
  return { saves: "", states: "", config: "" };
}

/**
 * Modal de adição de emulador com as três vias: recomendados (descoberta
 * automática), detecção por pasta e configuração manual (fallback).
 * No mobile exibe apenas o fluxo de concessão de pasta (SAF) + formulário
 * manual com caminhos padrão por emulador.
 */
export function AddEmulatorModal({ existingNames, onClose, onAdded }: Props) {
  const { t } = useTranslation();
  const errorMessage = useErrorMessage();
  const { isMobile } = usePlatform();
  const discovery = useDiscovery();
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Fluxo "apontar pasta": raiz escolhida e resultado da detecção automática.
  const [root, setRoot] = useState<string | null>(null);
  const [detected, setDetected] = useState<EmulatorProfile | null>(null);
  const [needsManual, setNeedsManual] = useState(false);

  // Campos do formulário manual.
  const [manualName, setManualName] = useState("");
  const [savesRel, setSavesRel] = useState("");
  const [statesRel, setStatesRel] = useState("");
  const [configRel, setConfigRel] = useState("");

  const recommendations = useMemo(
    () => discovery.discovered.filter((d) => !existingNames.includes(d.name)),
    [discovery.discovered, existingNames],
  );

  const resetManual = () => {
    setRoot(null);
    setDetected(null);
    setNeedsManual(false);
    setManualName("");
    setSavesRel("");
    setStatesRel("");
    setConfigRel("");
  };

  const wrap = useCallback(
    async (key: string, fn: () => Promise<void>) => {
      setBusy(key);
      setError(null);
      try {
        await fn();
      } catch (err) {
        setError(errorMessage(err));
      } finally {
        setBusy(null);
      }
    },
    [errorMessage],
  );

  const addRecommended = (d: DiscoveredEmulator) =>
    wrap(`rec:${d.name}`, async () => {
      if (!d.profile) return;
      await addEmulator(d.profile.rootPath);
      onAdded();
    });

  const pickRootDesktop = async () => {
    const selected = await openDialog({
      directory: true,
      multiple: false,
      title: t("addEmulator.pickRootTitle"),
    });
    if (typeof selected !== "string") return;
    resetManual();
    setRoot(selected);
    await wrap("detect", async () => {
      const profile = await detectEmulator(selected);
      if (profile) {
        setDetected(profile);
      } else {
        setNeedsManual(true);
      }
    });
  };

  // Mobile: abre o seletor SAF e tenta reconhecer o emulador via plugin
  // nativo (mesmo catálogo do desktop, checagem de pasta por chamada SAF em
  // vez de is_dir()); cai no formulário manual se não reconhecer.
  const pickRootMobile = async () => {
    await wrap("detect", async () => {
      const tree = await pickEmulatorFolder();
      resetManual();
      setRoot(tree);
      const profile = await detectEmulatorMobile(tree);
      if (profile) {
        setDetected(profile);
      } else {
        setNeedsManual(true);
      }
    });
  };

  const pickRoot = isMobile ? pickRootMobile : pickRootDesktop;

  const addDetected = () =>
    wrap("add-detected", async () => {
      if (!root) return;
      await addEmulator(root);
      onAdded();
      resetManual();
    });

  // Desktop: seleciona subpasta via dialog e calcula o caminho relativo.
  const pickSub = async (setter: (v: string) => void) => {
    if (!root) return;
    const selected = await openDialog({
      directory: true,
      multiple: false,
      defaultPath: root,
      title: t("addEmulator.pickSubTitle"),
    });
    if (typeof selected !== "string") return;
    const rel = relativeUnder(root, selected);
    if (!rel) {
      setError(t("addEmulator.subfolderError"));
      return;
    }
    setError(null);
    setter(rel);
  };

  const addManual = () =>
    wrap("add-manual", async () => {
      if (!root) return;
      await addEmulatorManual(
        manualName,
        root,
        savesRel ? [savesRel] : [],
        statesRel ? [statesRel] : [],
        configRel ? [configRel] : [],
      );
      onAdded();
      resetManual();
    });

  // Quando o nome muda no mobile, preenche os paths padrão se ainda estiverem vazios.
  const onNameChange = (name: string) => {
    setManualName(name);
    if (isMobile && !savesRel && !statesRel && !configRel) {
      const defaults = defaultPaths(name);
      setSavesRel(defaults.saves);
      setStatesRel(defaults.states);
      setConfigRel(defaults.config);
    }
  };

  const manualIncomplete = manualName.trim() === "" || (!savesRel && !statesRel && !configRel);

  return (
    <Modal title={t("addEmulator.title")} onClose={onClose}>
      {/* Seção de recomendados — apenas no desktop (requer scan de filesystem). */}
      {!isMobile ? (
        <section className="settings-section">
          <h3>{t("addEmulator.recommended")}</h3>
          {discovery.loading ? (
            <p className="muted">{t("addEmulator.searching")}</p>
          ) : discovery.error ? (
            <p className="error">{discovery.error}</p>
          ) : recommendations.length === 0 ? (
            <p className="muted">{t("addEmulator.noneDetected")}</p>
          ) : (
            <div className="discovery-list">
              {recommendations.map((d) => (
                <div className="discovery-row" key={d.name}>
                  <div className="discovery-info">
                    <span className="discovery-name">{d.name}</span>
                    <span className="muted discovery-meta">
                      {d.profile
                        ? t(SOURCE_LABEL_KEY[d.source])
                        : t("addEmulator.installedNoSaves")}
                    </span>
                  </div>
                  {d.profile ? (
                    <button disabled={busy !== null} onClick={() => addRecommended(d)}>
                      {busy === `rec:${d.name}` ? t("addEmulator.adding") : t("common.add")}
                    </button>
                  ) : (
                    <span className="muted discovery-hint">{t("addEmulator.openOnce")}</span>
                  )}
                </div>
              ))}
            </div>
          )}
        </section>
      ) : null}

      <section className="settings-section">
        <h3>{t("addEmulator.pickFolder")}</h3>
        <p className="muted">
          {isMobile ? t("addEmulator.pickFolderHintMobile") : t("addEmulator.pickFolderHint")}
        </p>
        <div className="settings-row">
          <button className="secondary" disabled={busy === "detect"} onClick={pickRoot}>
            {busy === "detect" ? t("addEmulator.detecting") : t("addEmulator.selectFolder")}
          </button>
          {root ? (
            <span className="muted discovery-meta" title={root}>
              {isMobile ? t("addEmulator.folderGranted") : root}
            </span>
          ) : null}
        </div>

        {detected ? (
          <div className="discovery-row">
            <div className="discovery-info">
              <span className="discovery-name">{detected.name}</span>
              <span className="muted discovery-meta">{t("addEmulator.detectedHere")}</span>
            </div>
            <button disabled={busy !== null} onClick={addDetected}>
              {busy === "add-detected" ? t("addEmulator.adding") : t("common.add")}
            </button>
          </div>
        ) : null}

        {needsManual ? (
          <div className="manual-form">
            <p className="muted">{t("addEmulator.manualIntro")}</p>
            <label className="manual-field">
              <span>{t("addEmulator.nameLabel")}</span>
              <input
                value={manualName}
                onChange={(e) => onNameChange(e.target.value)}
                placeholder={t("addEmulator.namePlaceholder")}
              />
            </label>
            {isMobile ? (
              <>
                <MobilePathInput
                  label={t("settings.categories.saves")}
                  value={savesRel}
                  onChange={setSavesRel}
                />
                <MobilePathInput
                  label={t("settings.categories.savestates")}
                  value={statesRel}
                  onChange={setStatesRel}
                />
                <MobilePathInput
                  label={t("settings.categories.config")}
                  value={configRel}
                  onChange={setConfigRel}
                />
              </>
            ) : (
              <>
                <ManualPathRow
                  label={t("settings.categories.saves")}
                  value={savesRel}
                  onPick={() => pickSub(setSavesRel)}
                />
                <ManualPathRow
                  label={t("settings.categories.savestates")}
                  value={statesRel}
                  onPick={() => pickSub(setStatesRel)}
                />
                <ManualPathRow
                  label={t("settings.categories.config")}
                  value={configRel}
                  onPick={() => pickSub(setConfigRel)}
                />
              </>
            )}
            <button disabled={busy !== null || manualIncomplete} onClick={addManual}>
              {busy === "add-manual" ? t("addEmulator.adding") : t("addEmulator.addManual")}
            </button>
          </div>
        ) : null}
      </section>

      {error ? <p className="error">{error}</p> : null}
    </Modal>
  );
}

interface ManualPathRowProps {
  label: string;
  value: string;
  onPick: () => void;
}

/** Linha do formulário manual no desktop: rótulo + botão de seleção de subpasta. */
function ManualPathRow({ label, value, onPick }: ManualPathRowProps) {
  const { t } = useTranslation();
  return (
    <div className="manual-path-row">
      <span className="manual-path-label">{label}</span>
      <button className="secondary" onClick={onPick}>
        {value || t("addEmulator.selectSubfolder")}
      </button>
    </div>
  );
}

interface MobilePathInputProps {
  label: string;
  value: string;
  onChange: (v: string) => void;
}

/** Linha do formulário manual no mobile: rótulo + campo de texto (caminho relativo). */
function MobilePathInput({ label, value, onChange }: MobilePathInputProps) {
  const { t } = useTranslation();
  return (
    <label className="manual-field">
      <span>{label}</span>
      <input
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder={t("addEmulator.relativePathPlaceholder")}
      />
    </label>
  );
}
