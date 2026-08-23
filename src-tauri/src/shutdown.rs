//! Desligamento gracioso: um [`CancellationToken`] compartilhado sinaliza a
//! todas as tasks de longa duração que é hora de parar, e um [`TaskTracker`]
//! permite esperar que elas realmente terminem antes de derrubar o processo.
//!
//! Sem isso, `app.exit(0)` matava o processo no meio de uma transferência: o
//! arquivo parcial ficava no disco e o manifesto não era atualizado. Agora o
//! menu "Sair" cancela o trabalho em andamento, dá até
//! [`SHUTDOWN_GRACE_SECS`](crate::constants::SHUTDOWN_GRACE_SECS) para as
//! tasks drenarem e só então encerra.

use std::time::Duration;

use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

/// Compartilhado via `AppState`. O `token` é o mesmo que o `SyncEngine`
/// consulta entre operações — ver [`crate::sync::SyncEngine::cancel_token`].
#[derive(Clone)]
pub struct ShutdownHandle {
    pub token: CancellationToken,
    pub tracker: TaskTracker,
}

impl ShutdownHandle {
    pub fn new(token: CancellationToken) -> Self {
        Self {
            token,
            tracker: TaskTracker::new(),
        }
    }

    /// Sinaliza o cancelamento e espera as tasks registradas terminarem, com
    /// teto de `grace`. Retorna `false` se o prazo estourou (alguma task não
    /// respondeu ao cancelamento) — o chamador encerra assim mesmo, mas o log
    /// registra o caso.
    pub async fn shutdown(&self, grace: Duration) -> bool {
        self.token.cancel();
        // `close()` é obrigatório: sem ele `wait()` bloqueia para sempre,
        // porque o tracker admite que novas tasks ainda serão registradas.
        self.tracker.close();
        tokio::time::timeout(grace, self.tracker.wait())
            .await
            .is_ok()
    }
}
