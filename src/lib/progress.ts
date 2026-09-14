import type { SyncProgress } from "../types/ipc";

/** 0–1 by bytes when known, by file count otherwise; `null` when nothing is measurable yet. */
export function progressFraction(progress: SyncProgress | null): number | null {
  if (!progress) return null;
  if (progress.bytesTotal > 0) return progress.bytesDone / progress.bytesTotal;
  if (progress.total > 0) return progress.completed / progress.total;
  return null;
}

/** Watcher-driven syncs get their own wording so people know why a sync started. */
export function autoTriggerLabelKey(
  trigger: string | null,
): "syncBar.autoPreGame" | "syncBar.autoPostGame" | null {
  if (trigger === "emulator-start") return "syncBar.autoPreGame";
  if (trigger === "emulator-stop") return "syncBar.autoPostGame";
  return null;
}
