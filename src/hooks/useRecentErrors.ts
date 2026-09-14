import { useCallback, useEffect, useState } from "react";

import { listen, type UnlistenFn } from "@tauri-apps/api/event";

import { clearErrors, getRecentErrors } from "../lib/ipc";
import { EVT, type ErrorEntry } from "../types/ipc";

/** In-memory error history from the backend, newest first. */
export function useRecentErrors() {
  const [errors, setErrors] = useState<ErrorEntry[]>([]);

  const reload = useCallback(() => {
    getRecentErrors()
      .then((entries) => setErrors([...entries].reverse()))
      .catch(() => {});
  }, []);

  useEffect(() => {
    reload();
    const subscriptions: Promise<UnlistenFn>[] = [
      listen(EVT.SYNC_ERROR, reload),
      listen(EVT.SYNC_COMPLETED, reload),
    ];
    return () => {
      subscriptions.forEach((promise) => promise.then((unlisten) => unlisten()).catch(() => {}));
    };
  }, [reload]);

  const clear = useCallback(async () => {
    await clearErrors();
    setErrors([]);
  }, []);

  return { errors, clear };
}
