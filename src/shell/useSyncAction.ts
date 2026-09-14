import { useCallback, useRef, useState } from "react";

import { useErrorMessage } from "../lib/errors";
import { syncNow } from "../lib/ipc";

/** Manual sync shared by the toolbar button, the shortcut and the pages' retry actions. */
export function useSyncAction() {
  const errorMessage = useErrorMessage();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const inFlight = useRef(false);

  const run = useCallback(async () => {
    if (inFlight.current) return;
    inFlight.current = true;
    setBusy(true);
    setError(null);
    try {
      await syncNow();
    } catch (err) {
      setError(errorMessage(err));
    } finally {
      inFlight.current = false;
      setBusy(false);
    }
  }, [errorMessage]);

  const dismissError = useCallback(() => setError(null), []);

  return { busy, error, run, dismissError };
}
