import { useCallback, useState } from "react";

export type Appearance = "system" | "light" | "dark";

const STORAGE_KEY = "slot2sync.appearance";

export function readStoredAppearance(): Appearance {
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    return stored === "light" || stored === "dark" ? stored : "system";
  } catch {
    return "system";
  }
}

export function applyAppearance(appearance: Appearance): void {
  const root = document.documentElement;
  if (appearance === "system") delete root.dataset.theme;
  else root.dataset.theme = appearance;
}

/** Display-only preference, so it lives in the webview and never reaches the backend. */
export function useAppearance() {
  const [appearance, setState] = useState<Appearance>(readStoredAppearance);

  const setAppearance = useCallback((next: Appearance) => {
    setState(next);
    applyAppearance(next);
    try {
      if (next === "system") localStorage.removeItem(STORAGE_KEY);
      else localStorage.setItem(STORAGE_KEY, next);
    } catch {
      // Storage blocked: the choice still applies for this session.
    }
  }, []);

  return { appearance, setAppearance };
}
