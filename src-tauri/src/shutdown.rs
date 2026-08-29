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

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    use super::*;

    /// Caminho feliz: as tasks registradas observam o cancelamento, terminam,
    /// e o `shutdown` devolve `true` dentro do prazo.
    #[tokio::test]
    async fn espera_as_tasks_registradas_terminarem() {
        let handle = ShutdownHandle::new(CancellationToken::new());

        let token = handle.token.clone();
        let terminou = Arc::new(AtomicBool::new(false));
        let flag = terminou.clone();
        handle.tracker.spawn(async move {
            token.cancelled().await;
            flag.store(true, Ordering::SeqCst);
        });

        assert!(handle.shutdown(Duration::from_secs(5)).await);
        assert!(
            terminou.load(Ordering::SeqCst),
            "a task deveria ter rodado até o fim antes de o shutdown retornar"
        );
    }

    /// Task que ignora o cancelamento não pode prender a saída do app: o
    /// prazo estoura, `shutdown` devolve `false` e quem chama encerra assim
    /// mesmo (registrando o caso no log).
    #[tokio::test]
    async fn devolve_false_quando_a_task_ignora_o_cancelamento() {
        let handle = ShutdownHandle::new(CancellationToken::new());
        handle
            .tracker
            .spawn(async { tokio::time::sleep(Duration::from_secs(30)).await });

        assert!(!handle.shutdown(Duration::from_millis(50)).await);
    }

    /// Sem nenhuma task registrada, o shutdown retorna de imediato — é o caso
    /// do app que ainda não terminou de subir quando o usuário manda sair.
    #[tokio::test]
    async fn sem_tasks_registradas_retorna_na_hora() {
        let handle = ShutdownHandle::new(CancellationToken::new());
        assert!(handle.shutdown(Duration::from_millis(50)).await);
    }

    /// Regressão: as tasks longas são registradas no `setup()` do Tauri, que
    /// roda na thread main FORA do contexto do runtime. `TaskTracker::spawn`
    /// chama `tokio::spawn` por dentro e entra em pânico ali ("there is no
    /// reactor running"); `track_future` só embrulha o future, deixando o
    /// spawn para o `tauri::async_runtime`. Este teste é deliberadamente
    /// `#[test]`, e não `#[tokio::test]`: sem runtime é justamente a condição
    /// que reproduzia o pânico.
    #[test]
    fn track_future_pode_ser_montado_sem_runtime_ativo() {
        let handle = ShutdownHandle::new(CancellationToken::new());
        let _tracked = handle.tracker.track_future(async {});
    }

    /// A troca de `spawn` por `track_future` não pode afetar a espera: uma
    /// future rastreada ainda segura o `shutdown` até terminar.
    #[tokio::test]
    async fn future_rastreada_e_esperada_como_uma_task_spawnada() {
        let handle = ShutdownHandle::new(CancellationToken::new());

        let token = handle.token.clone();
        let terminou = Arc::new(AtomicBool::new(false));
        let flag = terminou.clone();
        let tracked = handle.tracker.track_future(async move {
            token.cancelled().await;
            flag.store(true, Ordering::SeqCst);
        });
        tokio::spawn(tracked);

        assert!(handle.shutdown(Duration::from_secs(5)).await);
        assert!(terminou.load(Ordering::SeqCst));
    }

    /// O token é compartilhado por clonagem: cancelar pelo handle sinaliza
    /// quem guardou uma cópia (o `SyncEngine` guarda a dele).
    #[tokio::test]
    async fn o_cancelamento_alcanca_clones_do_token() {
        let token = CancellationToken::new();
        let copia = token.clone();
        let handle = ShutdownHandle::new(token);

        assert!(!copia.is_cancelled());
        handle.shutdown(Duration::from_millis(50)).await;

        assert!(copia.is_cancelled());
    }
}
