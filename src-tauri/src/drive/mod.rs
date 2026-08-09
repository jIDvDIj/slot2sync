//! Cliente da API do Google Drive v3 via `reqwest`.
//!
//! - `client`: requisições autenticadas com retry exponencial + jitter;
//! - `folders`: criação idempotente (com cache) da estrutura
//!   `Slot2Sync/<Emulador>/{saves,savestates,config}`;
//! - `files`: listagem recursiva, download, upload multipart (≤5 MB) e
//!   resumable (>5 MB), sempre preservando o mtime original em `modifiedTime`.
//!
//! Escopo OAuth: `drive.file` — o app só enxerga o que ele mesmo criou.
//! Nunca deleta nada no Drive.

mod api;
mod client;
mod files;
mod folders;
// Mesmo cfg dos cenários que o consomem (sync::scenarios) — sem eles, o mock
// viraria dead code no build de teste do Windows.
#[cfg(all(test, desktop, not(windows)))]
pub(crate) mod mock;
// Helper compartilhado pelos testes de HTTP (drive::files, drive::client) —
// não depende do MockRuntime do tauri, então roda em todas as plataformas.
#[cfg(test)]
pub(crate) mod test_support;

pub use api::DriveApi;
pub use client::DriveClient;
pub use files::{BatchUploadOp, DeviceTag, DriveFile, RemoteFile};

pub(crate) const DRIVE_API_BASE: &str = "https://www.googleapis.com/drive/v3";
pub(crate) const DRIVE_UPLOAD_BASE: &str = "https://www.googleapis.com/upload/drive/v3";
/// Endpoint de batch: agrupa até 100 sub-requests em um `multipart/mixed`.
pub(crate) const DRIVE_BATCH_BASE: &str = "https://www.googleapis.com/batch/drive/v3";
pub(crate) const FOLDER_MIME_TYPE: &str = "application/vnd.google-apps.folder";
pub(crate) const OCTET_STREAM: &str = "application/octet-stream";

/// Acima disso o upload usa sessão resumable em vez de multipart.
pub(crate) use crate::constants::DRIVE_SIMPLE_UPLOAD_MAX_BYTES as SIMPLE_UPLOAD_MAX_BYTES;

pub(crate) const FILE_FIELDS: &str = "id,name,mimeType,modifiedTime,size,md5Checksum,appProperties";
pub(crate) const LIST_FIELDS: &str =
    "files(id,name,mimeType,modifiedTime,size,md5Checksum,appProperties),nextPageToken";

/// Converte epoch ms para o RFC 3339 aceito pela API em `modifiedTime`.
pub(crate) fn ms_to_rfc3339(ms: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(ms)
        .unwrap_or_else(chrono::Utc::now)
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}
