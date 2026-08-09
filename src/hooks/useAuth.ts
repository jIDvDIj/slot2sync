import { useCallback, useEffect, useState } from "react";

import { useErrorMessage } from "../lib/errors";
import { disconnectProvider, getAuthStatus } from "../lib/ipc";
import type { AuthStatus } from "../types/ipc";

/**
 * Estado de autenticação no nível do App. É ele que decide qual tela renderizar:
 * enquanto `loading`, mostra o spinner; sem `connected`, a tela de login; com
 * `connected`, a tela principal.
 */
export function useAuth() {
  const errorMessage = useErrorMessage();
  const [status, setStatus] = useState<AuthStatus | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    getAuthStatus()
      .then(setStatus)
      .catch((err: unknown) => setError(errorMessage(err)))
      .finally(() => setLoading(false));
  }, [errorMessage]);

  const disconnect = useCallback(async () => {
    setError(null);
    try {
      setStatus(await disconnectProvider());
    } catch (err) {
      setError(errorMessage(err));
    }
  }, [errorMessage]);

  return {
    status,
    loading,
    error,
    connected: status?.connected ?? false,
    /** Atualiza o status após um login bem-sucedido na tela de login. */
    setStatus,
    disconnect,
  };
}
