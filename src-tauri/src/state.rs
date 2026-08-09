//! Estado global gerenciado pelo Tauri, construído no `setup` (precisa do
//! `AppHandle` para o diretório de dados e para o engine emitir eventos) e
//! acessado pelos comandos via `tauri::State<AppState>`.

use std::sync::Arc;

use crate::auth::AuthManager;
use crate::storage::db::Db;
use crate::sync::{LastSyncStore, LocalStorage, SyncEngine};

pub struct AppState {
    pub auth: Arc<AuthManager>,
    pub db: Db,
    pub engine: Arc<SyncEngine>,
    pub last_sync: LastSyncStore,
    pub storage: Arc<dyn LocalStorage>,
}
