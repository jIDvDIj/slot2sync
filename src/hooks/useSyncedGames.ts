import { useEffect, useState } from "react";

import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import { listSyncedGames } from "../lib/ipc";
import { EVT, type SyncedGame } from "../types/ipc";

/**
 * Jogos sincronizados, agregados do manifest no backend.
 * Carrega ao montar e recarrega a cada `sync:completed`, para refletir arquivos
 * novos sem exigir refresh manual.
 */
export function useSyncedGames(): SyncedGame[] {
  const [games, setGames] = useState<SyncedGame[]>([]);

  useEffect(() => {
    const load = () => {
      listSyncedGames()
        .then(setGames)
        .catch(() => {});
    };
    load();

    const sub: Promise<UnlistenFn> = listen(EVT.SYNC_COMPLETED, load);
    return () => {
      sub.then((unlisten) => unlisten()).catch(() => {});
    };
  }, []);

  return games;
}
