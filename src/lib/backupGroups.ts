import type { BackupEntry } from "../types/ipc";

export interface BackupGroup {
  key: string;
  emulator: string;
  category: string;
  /** Caminho do arquivo original, sem carimbo de versão. */
  relPath: string;
  /** Versões do arquivo, mais recentes primeiro. */
  entries: BackupEntry[];
  newestAtMs: number;
  totalBytes: number;
}

export interface BackupFilter {
  /** Casado contra `emulador/categoria/caminho`, sem distinção de caixa. */
  text: string;
  /** `YYYY-MM-DD` do `<input type="date">`; vazio = sem limite. */
  from: string;
  to: string;
}

/** Início do dia local de `YYYY-MM-DD`; `null` se vazio ou inválido. */
function dayStartMs(date: string): number | null {
  if (!date) return null;
  const ms = new Date(`${date}T00:00:00`).getTime();
  return Number.isNaN(ms) ? null : ms;
}

/** Instante seguinte ao fim do dia local, para comparar com `<`. */
function dayEndMs(date: string): number | null {
  const start = dayStartMs(date);
  return start === null ? null : start + 24 * 60 * 60 * 1000;
}

export function filterBackups(entries: BackupEntry[], filter: BackupFilter): BackupEntry[] {
  const needle = filter.text.trim().toLowerCase();
  const from = dayStartMs(filter.from);
  const to = dayEndMs(filter.to);

  return entries.filter((entry) => {
    if (from !== null && entry.modifiedAtMs < from) return false;
    if (to !== null && entry.modifiedAtMs >= to) return false;
    if (!needle) return true;
    return `${entry.emulator}/${entry.category}/${entry.originalRelPath}`
      .toLowerCase()
      .includes(needle);
  });
}

/**
 * Agrupa as versões de um mesmo arquivo. Grupos e versões saem do mais
 * recente para o mais antigo.
 */
export function groupBackups(entries: BackupEntry[]): BackupGroup[] {
  const groups = new Map<string, BackupGroup>();

  for (const entry of entries) {
    const key = `${entry.emulator}/${entry.category}/${entry.originalRelPath}`;
    const group = groups.get(key);
    if (group) {
      group.entries.push(entry);
      group.newestAtMs = Math.max(group.newestAtMs, entry.modifiedAtMs);
      group.totalBytes += entry.sizeBytes;
    } else {
      groups.set(key, {
        key,
        emulator: entry.emulator,
        category: entry.category,
        relPath: entry.originalRelPath,
        entries: [entry],
        newestAtMs: entry.modifiedAtMs,
        totalBytes: entry.sizeBytes,
      });
    }
  }

  const out = [...groups.values()];
  for (const group of out) {
    group.entries.sort((a, b) => b.modifiedAtMs - a.modifiedAtMs);
  }
  out.sort((a, b) => b.newestAtMs - a.newestAtMs);
  return out;
}
