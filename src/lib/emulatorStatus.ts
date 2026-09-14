import type { PendingOp, SyncCategories } from "../types/ipc";

/** Estado exclusivo de um emulador; as variantes estão em ordem de urgência. */
export type EmulatorStatus =
  | { kind: "conflict"; count: number }
  | { kind: "syncing" }
  | { kind: "failed"; count: number }
  | { kind: "pending"; count: number }
  | { kind: "paused" }
  | { kind: "running" }
  | { kind: "idle" };

export interface EmulatorStatusInput {
  /** Processo do emulador em execução (evento `emulator:status`). */
  running: boolean;
  /** Sync em curso neste emulador. */
  syncing: boolean;
  conflicts: number;
  pendingOps: PendingOp[];
  /** `null` enquanto as categorias não carregaram. */
  categories: SyncCategories | null;
}

/** Espelha `queue::PendingOp::is_dead`: só a ação do usuário reativa. */
function isDead(op: PendingOp): boolean {
  return op.nextRetryAtMs === null;
}

export function emulatorStatus({
  running,
  syncing,
  conflicts,
  pendingOps,
  categories,
}: EmulatorStatusInput): EmulatorStatus {
  if (conflicts > 0) return { kind: "conflict", count: conflicts };
  if (syncing) return { kind: "syncing" };

  const failed = pendingOps.filter(isDead).length;
  if (failed > 0) return { kind: "failed", count: failed };
  if (pendingOps.length > 0) return { kind: "pending", count: pendingOps.length };

  // Nenhuma categoria ativa: o emulador está configurado mas nada dele é
  // sincronizado, o que de fora parece "não funciona".
  if (categories && !categories.saves && !categories.savestates && !categories.config) {
    return { kind: "paused" };
  }

  return running ? { kind: "running" } : { kind: "idle" };
}
