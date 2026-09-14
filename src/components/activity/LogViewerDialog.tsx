import { useCallback, useEffect, useId, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import { currentLocale } from "../../i18n";
import { useErrorMessage } from "../../lib/errors";
import { getLogs, setLogStreaming } from "../../lib/ipc";
import { EVT, type LogEntry } from "../../types/ipc";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";
import { EmptyState } from "../ui/EmptyState";
import { InlineStatus, Select } from "../ui/Form";

import "./Activity.css";

const INITIAL_LINES = 500;
const MAX_BUFFERED = 5000;
const PIN_THRESHOLD_PX = 32;

const LEVELS = ["ERROR", "WARN", "INFO", "DEBUG", "TRACE"] as const;
type Level = (typeof LEVELS)[number];

function atLeast(entry: LogEntry, minimum: Level): boolean {
  const index = LEVELS.indexOf(entry.level as Level);
  return index >= 0 && index <= LEVELS.indexOf(minimum);
}

function formatEntry(entry: LogEntry): string {
  return `${entry.timestamp} ${entry.level.padEnd(5)} ${entry.target}: ${entry.message}`;
}

function formatTime(timestamp: string): string {
  const date = new Date(timestamp);
  if (Number.isNaN(date.getTime())) return timestamp;
  return date.toLocaleTimeString(currentLocale(), {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

interface LogViewerDialogProps {
  open: boolean;
  onClose: () => void;
}

export function LogViewerDialog({ open, onClose }: LogViewerDialogProps) {
  const { t } = useTranslation();
  const errorMessage = useErrorMessage();
  const levelId = useId();
  const [entries, setEntries] = useState<LogEntry[]>([]);
  const [minimum, setMinimum] = useState<Level>("INFO");
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const viewRef = useRef<HTMLDivElement>(null);
  // Auto-scroll only while the reader hasn't scrolled up to read something.
  const pinnedRef = useRef(true);

  // The backend emits log events only while this window is open, so streaming
  // is switched on and off with it.
  useEffect(() => {
    if (!open) return;
    let active = true;
    let unlisten: UnlistenFn | null = null;
    pinnedRef.current = true;

    setLogStreaming(true).catch(() => {});
    getLogs(INITIAL_LINES)
      .then((loaded) => {
        if (active) setEntries(loaded);
      })
      .catch((err: unknown) => {
        if (active) setError(errorMessage(err));
      });

    listen<LogEntry>(EVT.LOG_ENTRY, (event) => {
      if (!active) return;
      setEntries((current) => {
        const next = [...current, event.payload];
        return next.length > MAX_BUFFERED ? next.slice(next.length - MAX_BUFFERED) : next;
      });
    })
      .then((fn) => {
        if (active) unlisten = fn;
        else fn();
      })
      .catch(() => {});

    return () => {
      active = false;
      unlisten?.();
      setLogStreaming(false).catch(() => {});
    };
  }, [open, errorMessage]);

  const visible = useMemo(
    () => entries.filter((entry) => atLeast(entry, minimum)),
    [entries, minimum],
  );

  useEffect(() => {
    const view = viewRef.current;
    if (view && pinnedRef.current) view.scrollTop = view.scrollHeight;
  }, [visible]);

  const onScroll = useCallback(() => {
    const view = viewRef.current;
    if (!view) return;
    pinnedRef.current = view.scrollHeight - view.scrollTop - view.clientHeight < PIN_THRESHOLD_PX;
  }, []);

  const copyAll = async () => {
    setError(null);
    try {
      await navigator.clipboard.writeText(visible.map(formatEntry).join("\n"));
      setCopied(true);
    } catch {
      // No clipboard permission: select the text so the keyboard shortcut still works.
      if (viewRef.current) window.getSelection()?.selectAllChildren(viewRef.current);
      setError(t("logViewer.copyFailed"));
    }
  };

  return (
    <Dialog
      open={open}
      onClose={onClose}
      size="large"
      title={t("logViewer.title")}
      description={t("logViewer.description")}
      footer={
        <Button variant="prominent" onClick={onClose}>
          {t("common.done")}
        </Button>
      }
    >
      <div className="log-controls">
        <label className="log-level-label" htmlFor={levelId}>
          {t("logViewer.levelLabel")}
        </label>
        <Select
          id={levelId}
          value={minimum}
          onChange={(event) => setMinimum(event.target.value as Level)}
        >
          {LEVELS.map((level) => (
            <option key={level} value={level}>
              {t("logViewer.levelAtLeast", { level })}
            </option>
          ))}
        </Select>
        <span className="log-controls-spacer" />
        {copied ? <InlineStatus tone="success">{t("logViewer.copied")}</InlineStatus> : null}
        <Button icon="copy" disabled={visible.length === 0} onClick={() => void copyAll()}>
          {t("logViewer.copyAll")}
        </Button>
      </div>

      {error ? <InlineStatus tone="danger">{error}</InlineStatus> : null}

      {visible.length === 0 ? (
        <div className="log-view log-view-empty">
          <EmptyState compact icon="document" title={t("logViewer.empty")} />
        </div>
      ) : (
        <div
          ref={viewRef}
          className="log-view selectable"
          role="log"
          aria-label={t("logViewer.logLabel")}
          tabIndex={0}
          onScroll={onScroll}
        >
          {visible.map((entry, index) => (
            <div key={`${entry.timestamp}-${index}`} className="log-line">
              <span className="log-time tabular" title={entry.timestamp}>
                {formatTime(entry.timestamp)}
              </span>
              <span className="log-level" data-level={entry.level}>
                {entry.level}
              </span>
              <span className="log-message">
                <span className="log-target">{entry.target}</span> {entry.message}
              </span>
            </div>
          ))}
        </div>
      )}
    </Dialog>
  );
}
