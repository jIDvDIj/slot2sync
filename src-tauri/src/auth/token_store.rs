//! Persistência do refresh token via `SecretStore`.
//!
//! Desktop: keyring nativo do SO. Mobile: tabela `secrets` do SQLite privado.
//! As operações são bloqueantes; os chamadores async devem envolvê-las em
//! `tokio::task::spawn_blocking`. A chave do keyring é por provedor (ver
//! `constants::KEYRING_*_REFRESH_TOKEN_KEY`) — cada `AuthManager` guarda a sua.

use serde::{Deserialize, Serialize};

use crate::error::AppResult;
use crate::secrets::SecretStore;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredAuth {
    pub refresh_token: String,
    pub email: Option<String>,
}

pub struct TokenStore;

impl TokenStore {
    pub fn save(key: &str, auth: &StoredAuth, secrets: &dyn SecretStore) -> AppResult<()> {
        let json = serde_json::to_string(auth)?;
        secrets.set(key, &json)?;
        Ok(())
    }

    pub fn load(key: &str, secrets: &dyn SecretStore) -> AppResult<Option<StoredAuth>> {
        match secrets.get(key)? {
            Some(json) => Ok(serde_json::from_str(&json).ok()),
            None => Ok(None),
        }
    }

    pub fn clear(key: &str, secrets: &dyn SecretStore) -> AppResult<()> {
        secrets.delete(key)?;
        Ok(())
    }
}
