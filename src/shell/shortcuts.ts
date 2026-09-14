import type { Shortcut } from "../hooks/useShortcut";

export const SHORTCUTS = {
  sync: { key: "r", mod: true },
  settings: { key: ",", mod: true },
  addEmulator: { key: "n", mod: true },
  overview: { key: "1", mod: true },
  activity: { key: "2", mod: true },
} as const satisfies Record<string, Shortcut>;
