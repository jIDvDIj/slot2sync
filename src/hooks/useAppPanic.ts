import { useCallback, useEffect, useState } from "react";

import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import { EVT, type AppPanicPayload } from "../types/ipc";

/**
 * Último panic capturado pelo hook global do backend. Um panic derruba só a
 * task onde ocorreu — o app segue vivo na bandeja —, então sem este aviso o
 * usuário não teria sinal nenhum de que algo falhou.
 */
export function useAppPanic(): { panic: AppPanicPayload | null; dismiss: () => void } {
  const [panic, setPanic] = useState<AppPanicPayload | null>(null);

  const dismiss = useCallback(() => setPanic(null), []);

  useEffect(() => {
    const subscription: Promise<UnlistenFn> = listen<AppPanicPayload>(EVT.APP_PANIC, (event) =>
      setPanic(event.payload),
    );
    return () => {
      subscription.then((unlisten) => unlisten()).catch(() => {});
    };
  }, []);

  return { panic, dismiss };
}
