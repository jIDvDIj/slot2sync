import { useCallback, useEffect, useState } from "react";

import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import { checkForUpdates } from "../lib/ipc";
import { EVT, type UpdateInfo } from "../types/ipc";

/**
 * Versão nova disponível. A checagem roda uma vez ao abrir o app; falhas são
 * engolidas de propósito: não poder atualizar não é um erro que o usuário
 * precise ver ao iniciar.
 */
export function useUpdate(): { update: UpdateInfo | null; dismiss: () => void } {
  const [update, setUpdate] = useState<UpdateInfo | null>(null);

  const dismiss = useCallback(() => setUpdate(null), []);

  useEffect(() => {
    let active = true;
    checkForUpdates()
      .then((found) => {
        if (active && found) setUpdate(found);
      })
      .catch(() => {});

    const subscription: Promise<UnlistenFn> = listen<UpdateInfo>(EVT.UPDATE_AVAILABLE, (event) => {
      if (active) setUpdate(event.payload);
    });
    return () => {
      active = false;
      subscription.then((unlisten) => unlisten()).catch(() => {});
    };
  }, []);

  return { update, dismiss };
}
