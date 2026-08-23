//! Nomes dos eventos Tauri emitidos pelo backend. O frontend espelha estes
//! valores em `src/types/ipc.ts` (objeto `EVT`).
//!
//! Quem produz um evento não chama `emit` diretamente: publica um
//! [`bus::AppEvent`] no barramento interno, e a ponte em `lib.rs` traduz para
//! estes nomes. Ver [`bus`].

#![allow(dead_code)]

pub mod bus;

pub const EVT_SYNC_STARTED: &str = "sync:started";
pub const EVT_SYNC_PROGRESS: &str = "sync:progress";
pub const EVT_SYNC_COMPLETED: &str = "sync:completed";
pub const EVT_SYNC_ERROR: &str = "sync:error";
/// Conflito detectado: ambos os lados mudaram desde o último sync.
pub const EVT_SYNC_CONFLICT: &str = "sync:conflict";
/// Transição de `SyncState` — o frontend pode renderizar o estado atual
/// deterministicamente a partir deste evento em vez de acumular os eventos
/// discretos acima. Ver `sync::SyncState`.
pub const EVT_SYNC_STATE_CHANGED: &str = "sync:state-changed";
pub const EVT_AUTH_STATUS: &str = "auth:status";
pub const EVT_EMULATOR_STATUS: &str = "emulator:status";
/// Panic capturado pelo hook global: o app segue vivo (outras threads não são
/// derrubadas), mas a UI avisa que algo falhou de forma inesperada.
pub const EVT_APP_PANIC: &str = "app:panic";

/// Sync interrompido pelo desligamento do app (menu "Sair"): as operações que
/// faltavam não foram executadas.
pub const EVT_SYNC_CANCELLED: &str = "sync:cancelled";
