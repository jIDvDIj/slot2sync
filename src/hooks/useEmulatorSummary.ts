import { useEffect, useState } from "react";

import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import { getEmulatorSummary } from "../lib/ipc";
import { EVT, type EmulatorSummary } from "../types/ipc";

/**
 * Retrato do emulador (volume local, volume remoto conhecido, quanto está fora
 * de dia), recarregado ao fim de cada sync. `null` enquanto carrega ou quando a
 * varredura falha — o card simplesmente omite a linha, como em
 * {@link useEmulatorStats}.
 */
export function useEmulatorSummary(name: string): EmulatorSummary | null {
  const [summary, setSummary] = useState<EmulatorSummary | null>(null);

  useEffect(() => {
    let active = true;
    const reload = () => {
      getEmulatorSummary(name)
        .then((s) => {
          if (active) setSummary(s);
        })
        .catch(() => {
          if (active) setSummary(null);
        });
    };
    reload();
    const subscription: Promise<UnlistenFn> = listen(EVT.SYNC_COMPLETED, reload);
    return () => {
      active = false;
      subscription.then((unlisten) => unlisten()).catch(() => {});
    };
  }, [name]);

  return summary;
}
