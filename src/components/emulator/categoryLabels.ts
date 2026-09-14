import type { SyncedGame } from "../../types/ipc";

export const CATEGORY_LABEL = {
  saves: "categories.saves",
  savestates: "categories.savestates",
  config: "categories.config",
} as const satisfies Record<SyncedGame["categories"][number], string>;
