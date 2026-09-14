import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import { useErrorMessage } from "../lib/errors";
import { getLogs, setLogStreaming } from "../lib/ipc";
import { EVT, type LogEntry } from "../types/ipc";
import { Modal } from "./ui/Modal";

interface Props {
  onClose: () => void;
}

/** Linhas carregadas do arquivo ao abrir e teto do buffer em memória. */
const INITIAL_LINES = 500;
const MAX_BUFFERED = 5000;

const LEVELS = ["ERROR", "WARN", "INFO", "DEBUG", "TRACE"] as const;
type Level = (typeof LEVELS)[number];

/** Severidade mínima escolhida no filtro; menor índice = mais severo. */
function atLeast(entry: LogEntry, minimum: Level): boolean {
  const index = LEVELS.indexOf(entry.level as Level);
  return index >= 0 && index <= LEVELS.indexOf(minimum);
}

function formatEntry(entry: LogEntry): string {
  return `${entry.timestamp} ${entry.level.padEnd(5)} ${entry.target}: ${entry.message}`;
}

/**
 * Log da sessão em tempo real. O streaming é ligado no backend só enquanto
 * esta janela está aberta.
 */
export function LogViewerModal({ onClose }: Props) {
  const { t } = useTranslation();
  const errorMessage = useErrorMessage();
  const [entries, setEntries] = useState<LogEntry[]>([]);
  const [minimum, setMinimum] = useState<Level>("INFO");
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const areaRef = useRef<HTMLTextAreaElement>(null);
  /** Só rola sozinho enquanto o usuário não subiu para ler algo. */
  const pinnedRef = useRef(true);

  useEffect(() => {
    let active = true;
    let unlisten: UnlistenFn | null = null;

    setLogStreaming(true).catch(() => {});
    getLogs(INITIAL_LINES)
      .then((loaded) => {
        if (active) setEntries(loaded);
      })
      .catch((err) => setError(errorMessage(err)));

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
    // errorMessage é estável; a assinatura deve durar a vida da janela.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const text = useMemo(
    () =>
      entries
        .filter((entry) => atLeast(entry, minimum))
        .map(formatEntry)
        .join("\n"),
    [entries, minimum],
  );

  useEffect(() => {
    const area = areaRef.current;
    if (area && pinnedRef.current) area.scrollTop = area.scrollHeight;
  }, [text]);

  const onScroll = useCallback(() => {
    const area = areaRef.current;
    if (!area) return;
    const distance = area.scrollHeight - area.scrollTop - area.clientHeight;
    pinnedRef.current = distance < 32;
  }, []);

  const copyAll = async () => {
    setError(null);
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
    } catch {
      // Sem permissão de área de transferência: seleciona para o usuário
      // copiar com o atalho do teclado.
      areaRef.current?.select();
      setError(t("logViewer.copyFailed"));
    }
  };

  return (
    <Modal title={t("logViewer.title")} onClose={onClose}>
      <p className="muted">{t("logViewer.intro")}</p>

      <div className="settings-row log-controls">
        <label className="field">
          <span>{t("logViewer.levelLabel")}</span>
          <select value={minimum} onChange={(e) => setMinimum(e.target.value as Level)}>
            {LEVELS.map((level) => (
              <option key={level} value={level}>
                {level}
              </option>
            ))}
          </select>
        </label>
        <button className="secondary" onClick={copyAll} disabled={text.length === 0}>
          {copied ? t("logViewer.copied") : t("logViewer.copyAll")}
        </button>
      </div>

      <textarea
        className="log-area"
        ref={areaRef}
        onScroll={onScroll}
        readOnly
        spellCheck={false}
        value={text}
        placeholder={t("logViewer.empty")}
      />

      {error ? <p className="error">{error}</p> : null}
    </Modal>
  );
}
