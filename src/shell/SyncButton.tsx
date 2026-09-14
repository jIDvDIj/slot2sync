import { useEffect, useId, useState } from "react";
import { useTranslation } from "react-i18next";

import { Icon } from "../components/ui/Icon";
import { ariaShortcut, shortcutLabel } from "../hooks/useShortcut";
import { progressFraction } from "../lib/progress";
import type { SyncProgress } from "../types/ipc";
import { SHORTCUTS } from "./shortcuts";

const DONE_MOMENT_MS = 1800;

interface SyncButtonProps {
  syncing: boolean;
  progress: SyncProgress | null;
  onSync: () => void;
  compact: boolean;
}

/**
 * The app's one signature control: the button itself becomes the progress
 * indicator while a sync runs, then briefly confirms completion.
 */
export function SyncButton({ syncing, progress, onSync, compact }: SyncButtonProps) {
  const { t } = useTranslation();
  const [previousSyncing, setPreviousSyncing] = useState(syncing);
  const [justFinished, setJustFinished] = useState(false);
  if (syncing !== previousSyncing) {
    setPreviousSyncing(syncing);
    setJustFinished(!syncing);
  }

  useEffect(() => {
    if (!justFinished) return;
    const timer = window.setTimeout(() => setJustFinished(false), DONE_MOMENT_MS);
    return () => window.clearTimeout(timer);
  }, [justFinished]);

  const fraction = syncing ? progressFraction(progress) : null;
  const phase = syncing ? "syncing" : justFinished ? "done" : "idle";
  const label = syncing
    ? fraction === null
      ? t("syncBar.syncing")
      : t("syncBar.syncingPercent", { percent: Math.round(fraction * 100) })
    : justFinished
      ? t("syncBar.synced")
      : t("syncBar.syncNow");

  return (
    <>
      <button
        type="button"
        className="sync-button"
        data-phase={phase}
        data-compact={compact || undefined}
        onClick={syncing ? undefined : onSync}
        aria-disabled={syncing || undefined}
        aria-label={compact ? label : undefined}
        aria-keyshortcuts={ariaShortcut(SHORTCUTS.sync)}
        title={`${t("syncBar.syncNow")} (${shortcutLabel(SHORTCUTS.sync)})`}
      >
        <span className="sync-button-glyph" aria-hidden="true">
          {syncing ? (
            <ProgressRing fraction={fraction} />
          ) : justFinished ? (
            <Icon name="check" size={16} className="sync-button-check" />
          ) : (
            <Icon name="sync" size={16} />
          )}
        </span>
        {compact ? null : <span className="sync-button-label tabular">{label}</span>}
      </button>
      <span className="visually-hidden" role="status">
        {syncing ? t("syncBar.syncing") : justFinished ? t("syncBar.synced") : ""}
      </span>
    </>
  );
}

const RING_RADIUS = 7;
const RING_CIRCUMFERENCE = 2 * Math.PI * RING_RADIUS;

function ProgressRing({ fraction }: { fraction: number | null }) {
  // useId output contains characters that break `url(#…)` references.
  const gradientId = `ring-${useId().replace(/[^a-zA-Z0-9_-]/g, "")}`;
  const offset =
    fraction === null
      ? RING_CIRCUMFERENCE * 0.7
      : RING_CIRCUMFERENCE * (1 - Math.min(1, Math.max(0, fraction)));

  return (
    <svg
      className="progress-ring"
      data-indeterminate={fraction === null || undefined}
      width="18"
      height="18"
      viewBox="0 0 18 18"
    >
      <defs>
        <linearGradient id={gradientId} x1="0" y1="0" x2="1" y2="1">
          <stop offset="0%" className="progress-ring-stop-start" />
          <stop offset="55%" className="progress-ring-stop-middle" />
          <stop offset="100%" className="progress-ring-stop-end" />
        </linearGradient>
      </defs>
      <circle className="progress-ring-track" cx="9" cy="9" r={RING_RADIUS} />
      <circle
        className="progress-ring-value"
        cx="9"
        cy="9"
        r={RING_RADIUS}
        stroke={`url(#${gradientId})`}
        strokeDasharray={RING_CIRCUMFERENCE}
        strokeDashoffset={offset}
      />
    </svg>
  );
}
