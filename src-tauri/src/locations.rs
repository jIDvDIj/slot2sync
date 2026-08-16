//! Resolução centralizada dos caminhos de dados locais do app. Antes disso,
//! `app.path().app_data_dir()` era chamado inline em vários comandos —
//! sempre seguido do mesmo `.join(LOCAL_BACKUP_DIR)` ou `.join(LOCAL_DB_FILE)`
//! e do mesmo tratamento de erro. Se um caminho precisar mudar, muda num
//! lugar só.

use std::path::PathBuf;

use tauri::{AppHandle, Manager, Runtime};

use crate::constants::{LOCAL_BACKUP_DIR, LOCAL_DB_FILE};
use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppPath {
    /// Arquivo do banco SQLite local (`storage::db`).
    Database,
    /// Pasta raiz de backups: primeiro sync, cópias de conflito e histórico
    /// de versões (`versioning`, `backups`).
    BackupDir,
    /// Pasta de logs rotacionados (`tracing-appender`). Base diferente de
    /// `Database`/`BackupDir` — o SO não garante que logs e dados do app
    /// fiquem sob o mesmo diretório.
    LogDir,
}

impl AppPath {
    pub fn resolve<R: Runtime>(self, app: &AppHandle<R>) -> AppResult<PathBuf> {
        match self {
            AppPath::Database => Ok(data_dir(app)?.join(LOCAL_DB_FILE)),
            AppPath::BackupDir => Ok(data_dir(app)?.join(LOCAL_BACKUP_DIR)),
            AppPath::LogDir => app
                .path()
                .app_log_dir()
                .map_err(|e| AppError::Other(format!("diretório de logs indisponível: {e}"))),
        }
    }
}

fn data_dir<R: Runtime>(app: &AppHandle<R>) -> AppResult<PathBuf> {
    app.path()
        .app_data_dir()
        .map_err(|e| AppError::Other(format!("diretório de dados indisponível: {e}")))
}
