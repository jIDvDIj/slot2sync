import { useCallback, useId, useMemo, useState } from "react";
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
import { Button } from "./ui/Button";
import { Dialog } from "./ui/Dialog";
import { EmptyState } from "./ui/EmptyState";
import { Field, FormRow, FormSection, InlineStatus, TextField } from "./ui/Form";
import { Spinner } from "./ui/Spinner";

import "./AddEmulatorDialog.css";

export interface AddEmulatorDialogProps {
  open: boolean;
  /** Already configured emulators, filtered out of the recommendations. */
  existingNames: string[];
  onClose: () => void;
  /** Called after each successful addition. */
  onAdded: () => void;
}

export function AddEmulatorDialog({
  open,
  existingNames,
  onClose,
  onAdded,
}: AddEmulatorDialogProps) {
  const { t } = useTranslation();
  // The Dialog unmounts its children after closing, which resets the form and
  // restarts discovery the next time it opens.
  return (
    <Dialog
      open={open}
      onClose={onClose}
      size="large"
      title={t("addEmulator.title")}
      description={t("addEmulator.description")}
      footer={
        <Button variant="bordered" onClick={onClose}>
          {t("common.done")}
        </Button>
      }
    >
      <AddEmulatorContent existingNames={existingNames} onAdded={onAdded} />
    </Dialog>
  );
}

const SOURCE_LABEL_KEY = {
  dataDir: "addEmulator.sourceSavesFound",
  both: "addEmulator.sourceSavesFound",
  registry: "addEmulator.sourceInstalled",
} as const satisfies Record<DiscoveredEmulator["source"], string>;

/** Path of `child` relative to `root`, or `null` when it is not under the root. */
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

/** Known relative layouts, used to prefill the mobile form. */
function defaultPaths(name: string): { saves: string; states: string } {
  const n = name.toLowerCase();
  if (n.includes("ppsspp")) {
    return { saves: "PSP/SAVEDATA", states: "PSP/PPSSPP_STATE" };
  }
  if (n.includes("pcsx2")) {
    return { saves: "memcards", states: "sstates" };
  }
  if (n.includes("retroarch")) {
    return { saves: "saves", states: "states" };
  }
  return { saves: "", states: "" };
}

type Section = "found" | "folder";

function AddEmulatorContent({
  existingNames,
  onAdded,
}: Pick<AddEmulatorDialogProps, "existingNames" | "onAdded">) {
  const { t } = useTranslation();
  const errorMessage = useErrorMessage();
  const { isMobile } = usePlatform();
  const nameId = useId();
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<{ section: Section; message: string } | null>(null);
  const [added, setAdded] = useState<Set<string>>(new Set());

  const [root, setRoot] = useState<string | null>(null);
  const [detected, setDetected] = useState<EmulatorProfile | null>(null);
  const [needsManual, setNeedsManual] = useState(false);
  const [folderAdded, setFolderAdded] = useState<string | null>(null);

  const [manualName, setManualName] = useState("");
  const [savesRel, setSavesRel] = useState("");
  const [statesRel, setStatesRel] = useState("");
  const [excludeText, setExcludeText] = useState("");

  const resetFolder = () => {
    setRoot(null);
    setDetected(null);
    setNeedsManual(false);
    setFolderAdded(null);
    setManualName("");
    setSavesRel("");
    setStatesRel("");
    setExcludeText("");
  };

  const wrap = useCallback(
    async (key: string, section: Section, fn: () => Promise<void>) => {
      setBusy(key);
      setError(null);
      try {
        await fn();
      } catch (err) {
        setError({ section, message: errorMessage(err) });
      } finally {
        setBusy(null);
      }
    },
    [errorMessage],
  );

  const discovery = useDiscovery();
  // Keep just-added entries visible so their "Added" confirmation doesn't vanish
  // when the refreshed list starts filtering them out.
  const recommendations = useMemo(
    () => discovery.discovered.filter((d) => !existingNames.includes(d.name) || added.has(d.name)),
    [discovery.discovered, existingNames, added],
  );

  const addRecommended = (d: DiscoveredEmulator) =>
    wrap(`rec:${d.name}`, "found", async () => {
      if (!d.profile) return;
      await addEmulator(d.profile.rootPath);
      setAdded((prev) => new Set(prev).add(d.name));
      onAdded();
    });

  const pickRootDesktop = async () => {
    const selected = await openDialog({
      directory: true,
      multiple: false,
      title: t("addEmulator.pickRootTitle"),
    });
    if (typeof selected !== "string") return;
    resetFolder();
    setRoot(selected);
    await wrap("detect", "folder", async () => {
      const profile = await detectEmulator(selected);
      if (profile) setDetected(profile);
      else setNeedsManual(true);
    });
  };

  const pickRootMobile = () =>
    wrap("detect", "folder", async () => {
      const tree = await pickEmulatorFolder();
      resetFolder();
      setRoot(tree);
      const profile = await detectEmulatorMobile(tree);
      if (profile) setDetected(profile);
      else setNeedsManual(true);
    });

  const addDetected = () =>
    wrap("add-detected", "folder", async () => {
      if (!root || !detected) return;
      await addEmulator(root);
      onAdded();
      const name = detected.name;
      resetFolder();
      setFolderAdded(name);
    });

  const pickSub = async (setter: (value: string) => void) => {
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
      setError({ section: "folder", message: t("addEmulator.subfolderError") });
      return;
    }
    setError(null);
    setter(rel);
  };

  const addManual = () =>
    wrap("add-manual", "folder", async () => {
      if (!root) return;
      const name = manualName.trim();
      await addEmulatorManual(
        name,
        root,
        savesRel ? [savesRel] : [],
        statesRel ? [statesRel] : [],
        // A categoria config está desligada no backend: um emulador fora do
        // catálogo não ganha pasta de configuração.
        [],
        excludeText
          .split(",")
          .map((pattern) => pattern.trim())
          .filter((pattern) => pattern.length > 0),
      );
      onAdded();
      resetFolder();
      setFolderAdded(name);
    });

  const onNameChange = (name: string) => {
    setManualName(name);
    if (isMobile && !savesRel && !statesRel) {
      const defaults = defaultPaths(name);
      setSavesRel(defaults.saves);
      setStatesRel(defaults.states);
    }
  };

  const manualIncomplete = manualName.trim() === "" || (!savesRel && !statesRel);

  const pathRows = [
    { key: "saves", label: t("categories.saves"), value: savesRel, set: setSavesRel },
    { key: "savestates", label: t("categories.savestates"), value: statesRel, set: setStatesRel },
  ] as const;

  return (
    <>
      {!isMobile ? (
        <FormSection title={t("addEmulator.foundTitle")}>
          {discovery.loading ? (
            <div className="add-emulator-searching">
              <Spinner />
              <span>{t("addEmulator.searching")}</span>
            </div>
          ) : discovery.error ? (
            <FormRow label={<InlineStatus tone="danger">{discovery.error}</InlineStatus>} />
          ) : recommendations.length === 0 ? (
            <EmptyState
              compact
              icon="search"
              title={t("addEmulator.noneFound")}
              message={t("addEmulator.noneFoundMessage")}
            />
          ) : (
            recommendations.map((d) => (
              <FormRow
                key={d.name}
                icon="gamepad"
                label={d.name}
                description={
                  d.profile ? t(SOURCE_LABEL_KEY[d.source]) : t("addEmulator.installedNoSaves")
                }
                control={
                  added.has(d.name) ? (
                    <InlineStatus tone="success">{t("addEmulator.added")}</InlineStatus>
                  ) : d.profile ? (
                    <Button
                      variant="bordered"
                      size="small"
                      loading={busy === `rec:${d.name}`}
                      disabled={busy !== null}
                      onClick={() => void addRecommended(d)}
                    >
                      {t("addEmulator.add")}
                    </Button>
                  ) : (
                    <span className="add-emulator-hint">{t("addEmulator.openOnce")}</span>
                  )
                }
              />
            ))
          )}
        </FormSection>
      ) : null}
      {error?.section === "found" ? (
        <InlineStatus tone="danger">{error.message}</InlineStatus>
      ) : null}

      <FormSection
        title={t("addEmulator.folderTitle")}
        description={isMobile ? t("addEmulator.folderHintMobile") : t("addEmulator.folderHint")}
      >
        <FormRow
          icon="folder"
          label={
            root ? (
              isMobile ? (
                t("addEmulator.folderGranted")
              ) : (
                <span className="add-emulator-path truncate" title={root}>
                  {root}
                </span>
              )
            ) : (
              <span className="add-emulator-hint">{t("addEmulator.noFolder")}</span>
            )
          }
          control={
            <Button
              variant="bordered"
              loading={busy === "detect"}
              disabled={busy !== null && busy !== "detect"}
              onClick={() => void (isMobile ? pickRootMobile() : pickRootDesktop())}
            >
              {t("addEmulator.chooseFolder")}
            </Button>
          }
        />
        {detected ? (
          <FormRow
            icon="checkCircle"
            label={detected.name}
            description={t("addEmulator.detectedHere")}
            control={
              <Button
                variant="prominent"
                loading={busy === "add-detected"}
                disabled={busy !== null}
                onClick={() => void addDetected()}
              >
                {t("addEmulator.add")}
              </Button>
            }
          />
        ) : null}
        {folderAdded ? (
          <FormRow
            icon="checkCircle"
            label={folderAdded}
            control={<InlineStatus tone="success">{t("addEmulator.added")}</InlineStatus>}
          />
        ) : null}
      </FormSection>

      {needsManual ? (
        <div className="add-emulator-manual">
          <p className="add-emulator-hint">{t("addEmulator.manualIntro")}</p>
          <Field label={t("addEmulator.nameLabel")} htmlFor={nameId}>
            <TextField
              id={nameId}
              value={manualName}
              onChange={(e) => onNameChange(e.target.value)}
              placeholder={t("addEmulator.namePlaceholder")}
              autoComplete="off"
            />
          </Field>
          <FormSection title={t("addEmulator.foldersTitle")}>
            {pathRows.map((row) => (
              <FormRow
                key={row.key}
                label={row.label}
                htmlFor={isMobile ? `${nameId}-${row.key}` : undefined}
                control={
                  isMobile ? (
                    <TextField
                      id={`${nameId}-${row.key}`}
                      className="add-emulator-path-input"
                      value={row.value}
                      onChange={(e) => row.set(e.target.value)}
                      placeholder={t("addEmulator.relativePathPlaceholder")}
                      autoComplete="off"
                    />
                  ) : (
                    <Button variant="bordered" size="small" onClick={() => void pickSub(row.set)}>
                      {row.value ? (
                        <span className="add-emulator-path">{row.value}</span>
                      ) : (
                        t("addEmulator.chooseSubfolder")
                      )}
                    </Button>
                  )
                }
              />
            ))}
          </FormSection>
          <Field
            label={t("addEmulator.ignoreLabel")}
            htmlFor={`${nameId}-exclude`}
            hint={t("addEmulator.ignoreHint")}
          >
            <TextField
              id={`${nameId}-exclude`}
              value={excludeText}
              onChange={(e) => setExcludeText(e.target.value)}
              placeholder={t("addEmulator.ignorePlaceholder")}
              autoComplete="off"
              spellCheck={false}
            />
          </Field>
          <div className="add-emulator-actions">
            {manualIncomplete ? (
              <span className="add-emulator-hint">{t("addEmulator.manualIncomplete")}</span>
            ) : null}
            <Button
              variant="prominent"
              loading={busy === "add-manual"}
              disabled={busy !== null || manualIncomplete}
              onClick={() => void addManual()}
            >
              {t("addEmulator.addManual")}
            </Button>
          </div>
        </div>
      ) : null}

      {error?.section === "folder" ? (
        <InlineStatus tone="danger">{error.message}</InlineStatus>
      ) : null}
    </>
  );
}
