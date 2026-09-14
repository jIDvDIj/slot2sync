import { useEffect, useState } from "react";

import { getEmulatorCategories } from "../lib/ipc";
import type { SyncCategories } from "../types/ipc";

/** `null` enquanto carrega ou se a leitura falhar. */
export function useEmulatorCategories(name: string): SyncCategories | null {
  const [categories, setCategories] = useState<SyncCategories | null>(null);

  useEffect(() => {
    let active = true;
    getEmulatorCategories(name)
      .then((c) => {
        if (active) setCategories(c);
      })
      .catch(() => {
        if (active) setCategories(null);
      });
    return () => {
      active = false;
    };
  }, [name]);

  return categories;
}
