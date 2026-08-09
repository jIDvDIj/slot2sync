//! Estado global gerenciado pelo Tauri, construído no `setup` (precisa do
//! `AppHandle` para o diretório de dados e para o engine emitir eventos) e
//! acessado pelos comandos via `tauri::State<AppState>`.

use std::sync::{Arc, RwLock};

use crate::auth::AuthManager;
use crate::secrets::SecretStore;
use crate::storage::db::Db;
use crate::sync::{LastSyncStore, LocalStorage, SyncEngine};

pub struct AppState {
    /// `None` quando nenhum provedor OAuth está configurado (primeiro uso, ou
    /// provedor ativo é `LocalFolder`, que não usa `AuthManager`). Trocável em
    /// tempo de execução — ver `commands::connect_*`/`disconnect_provider`.
    pub auth: RwLock<Option<Arc<AuthManager>>>,
    pub db: Db,
    pub engine: Arc<SyncEngine>,
    pub last_sync: LastSyncStore,
    pub storage: Arc<dyn LocalStorage>,
    /// Cliente HTTP e store de segredos compartilhados — os comandos
    /// `connect_*` precisam deles para montar um `AuthManager`/provedor novo
    /// na primeira conexão ou ao trocar de provedor.
    pub http: reqwest::Client,
    pub secrets: Arc<dyn SecretStore>,
}
