//! Barramento interno de eventos: um canal [`tokio::sync::broadcast`] pelo qual
//! o backend publica o que aconteceu, sem saber quem vai reagir.
//!
//! Antes, cada produtor precisava de um `AppHandle` para chamar `emit` e
//! `notification()` por conta própria. Isso obrigava o [`SyncEngine`] a ser
//! genérico sobre o runtime do Tauri só para carregar esse handle, e o genérico
//! vazava para o `AppState` e para todo teste que quisesse instanciá-lo.
//!
//! Agora os produtores publicam [`AppEvent`] e uma única task em `lib.rs`
//! traduz cada evento em `emit` para o frontend e, quando for o caso, em
//! notificação nativa do SO. O engine não conhece mais o Tauri.
//!
//! **Canal com perda:** `broadcast` descarta as mensagens mais antigas quando
//! um assinante fica para trás (`RecvError::Lagged`). Isso é aceitável para
//! eventos de UI, em que só o estado mais recente importa — e é o motivo de o
//! disparo de sync do watcher continuar num `mpsc` dedicado, e não aqui: perder
//! um "emulador fechou" significaria perder uma sincronização.
//!
//! [`SyncEngine`]: crate::sync::SyncEngine

use tokio::sync::broadcast;

use crate::storage::conflicts::Conflict;
use crate::sync::{SyncError, SyncProgress, SyncStarted, SyncStateChanged, SyncSummary};

/// Capacidade do canal. Um sync emite no máximo um retrato de progresso a cada
/// 500ms, então a folga aqui é para rajadas de conflito/erro no início de uma
/// sincronização grande.
const BUS_CAPACITY: usize = 64;

/// Notificação nativa do SO pedida por um produtor. O gating por
/// [`NotificationLevel`](crate::storage::settings::NotificationLevel) é feito
/// por quem publica — quem consome só exibe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeNotification {
    pub title: String,
    pub body: String,
}

/// Tudo que o backend comunica para fora de si mesmo. As variantes que o
/// frontend enxerga carregam exatamente o payload já espelhado em
/// `src/types/ipc.ts`.
#[derive(Debug, Clone)]
pub enum AppEvent {
    SyncStarted(SyncStarted),
    SyncProgress(SyncProgress),
    SyncCompleted(SyncSummary),
    SyncCancelled(SyncSummary),
    SyncError(SyncError),
    SyncConflict(Box<Conflict>),
    SyncStateChanged(SyncStateChanged),
    AuthStatus(Box<crate::auth::AuthStatus>),
    EmulatorStatus {
        emulator: String,
        running: bool,
    },
    /// Pedido de notificação nativa, independente do evento de UI que a
    /// acompanha — nem toda notificação tem um evento correspondente na tela.
    Notify(NativeNotification),
}

/// Ponta de publicação do barramento. Clonável e barato de passar adiante;
/// publicar sem nenhum assinante ativo é um no-op silencioso.
#[derive(Clone)]
pub struct EventBus {
    tx: broadcast::Sender<AppEvent>,
}

impl EventBus {
    pub fn new() -> Self {
        let (tx, _rx) = broadcast::channel(BUS_CAPACITY);
        Self { tx }
    }

    /// Publica um evento. O erro de "nenhum assinante" é esperado (acontece
    /// antes de `lib.rs` subir a ponte, e nos testes) e não é propagado.
    pub fn publish(&self, event: AppEvent) {
        let _ = self.tx.send(event);
    }

    /// Atalho para [`AppEvent::Notify`].
    pub fn notify(&self, title: impl Into<String>, body: impl Into<String>) {
        self.publish(AppEvent::Notify(NativeNotification {
            title: title.into(),
            body: body.into(),
        }));
    }

    pub fn subscribe(&self) -> broadcast::Receiver<AppEvent> {
        self.tx.subscribe()
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notification(title: &str) -> AppEvent {
        AppEvent::Notify(NativeNotification {
            title: title.into(),
            body: String::new(),
        })
    }

    #[test]
    fn publicar_sem_assinante_nao_falha() {
        let bus = EventBus::new();
        // Acontece de verdade: o engine pode publicar antes de `lib.rs` subir
        // a ponte, e nos testes de cenário ninguém assina.
        bus.publish(notification("sem ninguém ouvindo"));
    }

    #[test]
    fn assinante_recebe_o_que_foi_publicado_depois_de_assinar() {
        let bus = EventBus::new();
        let mut rx = bus.subscribe();

        bus.notify("titulo", "corpo");

        let received = rx.try_recv().unwrap();
        match received {
            AppEvent::Notify(n) => {
                assert_eq!(n.title, "titulo");
                assert_eq!(n.body, "corpo");
            }
            other => panic!("evento inesperado: {other:?}"),
        }
    }

    #[test]
    fn todos_os_assinantes_recebem_o_mesmo_evento() {
        let bus = EventBus::new();
        let (mut a, mut b) = (bus.subscribe(), bus.subscribe());

        bus.publish(notification("difundido"));

        for rx in [&mut a, &mut b] {
            assert!(matches!(rx.try_recv(), Ok(AppEvent::Notify(_))));
        }
    }

    /// `Default` existe para o barramento poder ser construído por derive em
    /// quem o contém; precisa ser o mesmo canal que `new`.
    #[test]
    fn default_produz_um_barramento_utilizavel() {
        let bus = EventBus::default();
        let mut rx = bus.subscribe();

        bus.publish(notification("via default"));

        assert!(matches!(rx.try_recv(), Ok(AppEvent::Notify(_))));
    }

    /// O canal é de perda declarada: assinante que não drena perde as
    /// mensagens mais antigas e recebe `Lagged`. A ponte em `lib.rs` trata
    /// esse caso registrando quantas se perderam e seguindo em frente.
    #[test]
    fn assinante_lento_recebe_lagged_em_vez_de_travar_o_produtor() {
        use tokio::sync::broadcast::error::TryRecvError;

        let bus = EventBus::new();
        let mut rx = bus.subscribe();

        for i in 0..BUS_CAPACITY + 10 {
            bus.publish(notification(&i.to_string()));
        }

        assert!(matches!(rx.try_recv(), Err(TryRecvError::Lagged(_))));
    }
}
