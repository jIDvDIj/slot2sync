//! Cliente da API v2 do Dropbox — uma das implementações concretas de
//! `remote::RemoteProvider`.
//!
//! Dropbox endereça tudo por **path** (não por ID de pasta como o Drive), o
//! que simplifica bastante `ensure_*`: não há cache de IDs, o path já É o
//! identificador estável. `id`/`folder_id` no trait são sempre o path
//! Dropbox do objeto (ex.: `/Slot2Sync/PPSSPP/saves/save.bin`).
//!
//! Sem um equivalente simples a `appProperties` do Drive, a atribuição de
//! dispositivo usa o índice compartilhado (`remote::device_index`), guardado
//! em `/Slot2Sync/.slot2sync-index.json` no próprio Dropbox.
//!
//! Escopo de acesso: a App Folder do app registrado no App Console do
//! Dropbox — equivalente ao `drive.file` do Drive (o app só enxerga sua
//! própria pasta). Upload simples (sem sessão em chunks): arquivos até
//! ~150 MB, suficiente para saves/savestates típicos; arquivos maiores
//! precisariam de `upload_session` (não implementado).

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::json;

use crate::auth::AuthManager;
use crate::constants::{DRIVE_MAX_RETRIES, DRIVE_ROOT_FOLDER};
use crate::error::{AppError, AppResult};
use crate::remote::device_index::{DeviceEntry, DeviceIndex, INDEX_FILE_NAME};
use crate::remote::http::{send_with_retry, RateLimiter};
use crate::remote::{BatchUploadOp, DeviceTag, RemoteFile, RemoteProvider};
use crate::sync::SyncCategory;

const API_BASE: &str = "https://api.dropboxapi.com/2";
const CONTENT_BASE: &str = "https://content.dropboxapi.com/2";

pub struct DropboxClient {
    http: reqwest::Client,
    auth: Arc<AuthManager>,
    api_base: String,
    content_base: String,
    limiter: RateLimiter,
}

impl DropboxClient {
    pub fn new(http: reqwest::Client, auth: Arc<AuthManager>) -> Self {
        Self {
            http,
            auth,
            api_base: API_BASE.to_string(),
            content_base: CONTENT_BASE.to_string(),
            limiter: RateLimiter::default(),
        }
    }

    #[cfg(test)]
    fn with_base_url(mut self, base: &str) -> Self {
        self.api_base = base.to_string();
        self.content_base = base.to_string();
        self
    }

    async fn send<F>(&self, op_name: &str, build: F) -> AppResult<reqwest::Response>
    where
        F: Fn(&str) -> reqwest::RequestBuilder,
    {
        // Dropbox sinaliza rate limit com 429 + `Retry-After`, já coberto pelo
        // caminho genérico de `remote::http`; nenhuma regra extra necessária.
        send_with_retry(&self.auth, op_name, DRIVE_MAX_RETRIES, |_, _| false, build).await
    }

    fn root_path() -> String {
        format!("/{DRIVE_ROOT_FOLDER}")
    }

    async fn create_folder(&self, path: &str) -> AppResult<()> {
        let url = format!("{}/files/create_folder_v2", self.api_base);
        let body = json!({ "path": path, "autorename": false });
        let result = self
            .send("files.create_folder_v2", |token| {
                self.http.post(&url).bearer_auth(token).json(&body)
            })
            .await;
        match result {
            Ok(_) => Ok(()),
            // Já existe: idempotente, tolera.
            Err(AppError::Other(msg)) if msg.contains("path/conflict") => Ok(()),
            Err(err) => Err(err),
        }
    }

    async fn get_metadata(&self, path: &str) -> AppResult<Option<DropboxEntry>> {
        let url = format!("{}/files/get_metadata", self.api_base);
        let body = json!({ "path": path });
        let result = self
            .send("files.get_metadata", |token| {
                self.http.post(&url).bearer_auth(token).json(&body)
            })
            .await;
        match result {
            Ok(response) => Ok(Some(response.json::<DropboxEntry>().await?)),
            Err(AppError::Other(msg)) if msg.contains("path/not_found") => Ok(None),
            Err(AppError::RemoteObjectNotFound(_)) => Ok(None),
            Err(err) => Err(err),
        }
    }

    async fn load_index(&self) -> DeviceIndex {
        let path = format!("{}/{INDEX_FILE_NAME}", Self::root_path());
        match self.download(&path).await {
            Ok(bytes) => DeviceIndex::parse(&bytes),
            Err(_) => DeviceIndex::default(),
        }
    }

    // Não passa por `upload_bytes`/`stamp_device` de propósito: o próprio
    // índice não tem entrada de dispositivo, e isso criaria uma recursão
    // (stamp_device → save_index → upload_bytes → stamp_device → ...).
    async fn save_index(&self, index: &DeviceIndex) {
        let path = format!("{}/{INDEX_FILE_NAME}", Self::root_path());
        if let Err(err) = self.raw_upload_bytes(&path, index.to_bytes(), None).await {
            tracing::warn!(error = %err, "falha ao atualizar índice de dispositivo do Dropbox");
        }
    }

    async fn stamp_device(&self, path: &str, device: DeviceTag<'_>) {
        if device.name.is_none() && device.id.is_none() {
            return;
        }
        let mut index = self.load_index().await;
        index.set(
            path,
            DeviceEntry {
                device_name: device.name.map(str::to_string),
                device_id: device.id.map(str::to_string),
            },
        );
        self.save_index(&index).await;
    }

    // Nota: recarrega o índice a cada chamada (um download extra por
    // arquivo listado) — simples e correto, mas não o ideal para pastas com
    // muitos arquivos. Otimização futura: `list_tree` poderia carregar uma
    // vez e reusar entre as entradas da página.
    async fn to_remote_file(&self, entry: &DropboxEntry, rel_path: String) -> RemoteFile {
        let index = self.load_index().await;
        let device = index.get(&entry.path_lower_or(&entry.id)).cloned();
        RemoteFile {
            id: entry.id.clone(),
            rel_path,
            modified_ms: entry.client_modified_ms(),
            size_bytes: entry.size.map(|s| s as i64),
            hash: entry.content_hash.clone(),
            device_name: device.as_ref().and_then(|e| e.device_name.clone()),
            device_id: device.as_ref().and_then(|e| e.device_id.clone()),
        }
    }

    async fn raw_upload_bytes(
        &self,
        path: &str,
        content: Vec<u8>,
        mtime_ms: Option<i64>,
    ) -> AppResult<DropboxEntry> {
        self.limiter
            .throttle(
                content.len(),
                0, /* limites globais aplicados no engine */
            )
            .await;
        let mut arg = json!({
            "path": path,
            "mode": "overwrite",
            "autorename": false,
            "mute": true,
        });
        if let Some(ms) = mtime_ms {
            arg["client_modified"] = json!(ms_to_rfc3339(ms));
        }
        let arg_header = serde_json::to_string(&arg)?;
        let url = format!("{}/files/upload", self.content_base);
        let response = self
            .send("files.upload", |token| {
                self.http
                    .post(&url)
                    .bearer_auth(token)
                    .header("Dropbox-API-Arg", arg_header.clone())
                    .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
                    .body(content.clone())
            })
            .await?;
        Ok(response.json::<DropboxEntry>().await?)
    }

    async fn upload_bytes(
        &self,
        path: &str,
        content: Vec<u8>,
        mtime_ms: Option<i64>,
        device: DeviceTag<'_>,
    ) -> AppResult<DropboxEntry> {
        let entry = self.raw_upload_bytes(path, content, mtime_ms).await?;
        self.stamp_device(path, device).await;
        Ok(entry)
    }
}

fn ms_to_rfc3339(ms: i64) -> String {
    DateTime::<Utc>::from_timestamp_millis(ms)
        .unwrap_or_else(Utc::now)
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

#[derive(Debug, Clone, Deserialize)]
struct DropboxEntry {
    #[serde(rename = ".tag")]
    tag: String,
    id: String,
    name: String,
    #[serde(default)]
    path_display: Option<String>,
    #[serde(default)]
    path_lower: Option<String>,
    #[serde(default)]
    client_modified: Option<String>,
    #[serde(default)]
    size: Option<u64>,
    #[serde(default)]
    content_hash: Option<String>,
}

impl DropboxEntry {
    fn is_folder(&self) -> bool {
        self.tag == "folder"
    }

    fn client_modified_ms(&self) -> Option<i64> {
        self.client_modified
            .as_deref()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.timestamp_millis())
    }

    fn path_lower_or<'a>(&'a self, fallback: &'a str) -> String {
        self.path_lower
            .clone()
            .or_else(|| self.path_display.clone())
            .unwrap_or_else(|| fallback.to_string())
    }
}

#[derive(Debug, Deserialize)]
struct ListFolderResult {
    entries: Vec<DropboxEntry>,
    cursor: String,
    has_more: bool,
}

#[async_trait]
impl RemoteProvider for DropboxClient {
    async fn ensure_root(&self) -> AppResult<String> {
        let path = Self::root_path();
        self.create_folder(&path).await?;
        Ok(path)
    }

    async fn ensure_category_folder(
        &self,
        emulator: &str,
        category: SyncCategory,
    ) -> AppResult<String> {
        self.ensure_root().await?;
        let emulator_path = format!("{}/{emulator}", Self::root_path());
        self.create_folder(&emulator_path).await?;
        let category_path = format!("{emulator_path}/{}", category.as_str());
        self.create_folder(&category_path).await?;
        Ok(category_path)
    }

    async fn ensure_subpath(
        &self,
        base_id: &str,
        _base_key: &str,
        rel_dir: &str,
    ) -> AppResult<String> {
        let mut current = base_id.to_string();
        for segment in rel_dir.split('/').filter(|s| !s.is_empty()) {
            current = format!("{current}/{segment}");
            self.create_folder(&current).await?;
        }
        Ok(current)
    }

    async fn list_tree(&self, folder_id: &str) -> AppResult<Vec<RemoteFile>> {
        let url = format!("{}/files/list_folder", self.api_base);
        let body = json!({ "path": folder_id, "recursive": true });
        let response = self
            .send("files.list_folder", |token| {
                self.http.post(&url).bearer_auth(token).json(&body)
            })
            .await?;
        let mut page: ListFolderResult = response.json().await?;

        let mut out = Vec::new();
        loop {
            for entry in &page.entries {
                if entry.is_folder() {
                    continue;
                }
                if entry.name == INDEX_FILE_NAME {
                    continue;
                }
                let full_path = entry
                    .path_display
                    .clone()
                    .unwrap_or_else(|| format!("{folder_id}/{}", entry.name));
                let rel_path = full_path
                    .strip_prefix(folder_id)
                    .unwrap_or(&full_path)
                    .trim_start_matches('/')
                    .to_string();
                out.push(self.to_remote_file(entry, rel_path).await);
            }
            if !page.has_more {
                break;
            }
            let continue_url = format!("{}/files/list_folder/continue", self.api_base);
            let cursor = page.cursor.clone();
            let response = self
                .send("files.list_folder.continue", |token| {
                    self.http
                        .post(&continue_url)
                        .bearer_auth(token)
                        .json(&json!({ "cursor": cursor }))
                })
                .await?;
            page = response.json().await?;
        }
        Ok(out)
    }

    async fn find_child(&self, folder_id: &str, name: &str) -> AppResult<Option<RemoteFile>> {
        let path = format!("{folder_id}/{name}");
        match self.get_metadata(&path).await? {
            Some(entry) if !entry.is_folder() => {
                Some(self.to_remote_file(&entry, name.to_string()).await)
            }
            _ => None,
        }
        .map(Ok)
        .transpose()
    }

    async fn download(&self, file_id: &str) -> AppResult<Vec<u8>> {
        let url = format!("{}/files/download", self.content_base);
        let arg = serde_json::to_string(&json!({ "path": file_id }))?;
        let response = self
            .send("files.download", |token| {
                self.http
                    .post(&url)
                    .bearer_auth(token)
                    .header("Dropbox-API-Arg", arg.clone())
            })
            .await?;
        let content = response.bytes().await?.to_vec();
        self.limiter.throttle(content.len(), 0).await;
        Ok(content)
    }

    async fn upload_new(
        &self,
        parent_id: &str,
        name: &str,
        content: Vec<u8>,
        mtime_ms: i64,
        device: DeviceTag<'_>,
    ) -> AppResult<RemoteFile> {
        let path = format!("{parent_id}/{name}");
        let entry = self
            .upload_bytes(&path, content, Some(mtime_ms), device)
            .await?;
        Ok(self.to_remote_file(&entry, name.to_string()).await)
    }

    async fn upload_existing(
        &self,
        file_id: &str,
        content: Vec<u8>,
        mtime_ms: i64,
        device: DeviceTag<'_>,
    ) -> AppResult<RemoteFile> {
        let entry = self
            .upload_bytes(file_id, content, Some(mtime_ms), device)
            .await?;
        Ok(self.to_remote_file(&entry, String::new()).await)
    }

    async fn upload_batch(&self, ops: Vec<BatchUploadOp>) -> AppResult<Vec<RemoteFile>> {
        // A API de batch do Dropbox é assíncrona (job polling) — não vale o
        // overhead para o volume típico de um sync; laço simples per-file.
        let mut out = Vec::with_capacity(ops.len());
        for op in ops {
            let tag = DeviceTag {
                name: op.device_name.as_deref(),
                id: op.device_id.as_deref(),
            };
            out.push(
                self.upload_new(&op.parent_id, &op.name, op.content, op.mtime_ms, tag)
                    .await?,
            );
        }
        Ok(out)
    }

    async fn rename_file(
        &self,
        file_id: &str,
        new_name: &str,
        add_parent: Option<&str>,
        _remove_parent: Option<&str>,
    ) -> AppResult<RemoteFile> {
        let parent = add_parent
            .map(str::to_string)
            .or_else(|| file_id.rsplit_once('/').map(|(dir, _)| dir.to_string()))
            .unwrap_or_else(Self::root_path);
        let to_path = format!("{parent}/{new_name}");
        let url = format!("{}/files/move_v2", self.api_base);
        let body = json!({ "from_path": file_id, "to_path": to_path, "autorename": false });
        let response = self
            .send("files.move_v2", |token| {
                self.http.post(&url).bearer_auth(token).json(&body)
            })
            .await?;

        #[derive(Deserialize)]
        struct MoveResult {
            metadata: DropboxEntry,
        }
        let result: MoveResult = response.json().await?;

        let mut index = self.load_index().await;
        index.rename(file_id, &to_path);
        self.save_index(&index).await;

        Ok(self
            .to_remote_file(&result.metadata, new_name.to_string())
            .await)
    }

    async fn invalidate_folder_path(&self, _cache_key: &str) {}

    async fn clear_folder_cache(&self) {}
}

#[cfg(test)]
mod tests {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::secrets::MemSecrets;

    async fn client_against(server: &MockServer) -> DropboxClient {
        let secrets: Arc<dyn crate::secrets::SecretStore> = Arc::new(MemSecrets::default());
        let auth = Arc::new(AuthManager::new(reqwest::Client::new(), secrets));
        auth.set_test_access_token("tok-teste").await;
        DropboxClient::new(reqwest::Client::new(), auth).with_base_url(&server.uri())
    }

    #[tokio::test]
    async fn list_tree_ignora_pastas_e_o_indice_de_dispositivo() {
        let server = MockServer::start().await;
        let client = client_against(&server).await;

        Mock::given(method("POST"))
            .and(path("/files/list_folder"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "entries": [
                    {".tag": "folder", "id": "id:folder1", "name": "jogo", "path_display": "/Slot2Sync/PPSSPP/saves/jogo"},
                    {".tag": "file", "id": "id:file1", "name": "save.bin", "path_display": "/Slot2Sync/PPSSPP/saves/save.bin", "size": 10},
                    {".tag": "file", "id": "id:file2", "name": ".slot2sync-index.json", "path_display": "/Slot2Sync/PPSSPP/saves/.slot2sync-index.json", "size": 2},
                ],
                "cursor": "c1",
                "has_more": false,
            })))
            .mount(&server)
            .await;

        let tree = client.list_tree("/Slot2Sync/PPSSPP/saves").await.unwrap();
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].rel_path, "save.bin");
    }

    #[tokio::test]
    async fn download_retorna_os_bytes() {
        let server = MockServer::start().await;
        let client = client_against(&server).await;

        Mock::given(method("POST"))
            .and(path("/files/download"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"conteudo".to_vec()))
            .mount(&server)
            .await;

        assert_eq!(
            client
                .download("/Slot2Sync/PPSSPP/saves/save.bin")
                .await
                .unwrap(),
            b"conteudo"
        );
    }

    #[tokio::test]
    async fn upload_new_envia_dropbox_api_arg_e_retorna_remote_file() {
        let server = MockServer::start().await;
        let client = client_against(&server).await;

        Mock::given(method("POST"))
            .and(path("/files/upload"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                ".tag": "file",
                "id": "id:new1",
                "name": "save.bin",
                "path_display": "/Slot2Sync/PPSSPP/saves/save.bin",
                "size": 5,
                "content_hash": "abc123",
            })))
            .mount(&server)
            .await;

        let file = client
            .upload_new(
                "/Slot2Sync/PPSSPP/saves",
                "save.bin",
                b"dados".to_vec(),
                1_700_000_000_000,
                DeviceTag::default(),
            )
            .await
            .unwrap();
        assert_eq!(file.id, "id:new1");
        assert_eq!(file.hash.as_deref(), Some("abc123"));
    }

    #[tokio::test]
    async fn ensure_root_tolera_pasta_ja_existente() {
        let server = MockServer::start().await;
        let client = client_against(&server).await;

        Mock::given(method("POST"))
            .and(path("/files/create_folder_v2"))
            .respond_with(ResponseTemplate::new(409).set_body_string(
                r#"{"error_summary": "path/conflict/folder/...", "error": {".tag": "path", "path": {".tag": "conflict"}}}"#,
            ))
            .mount(&server)
            .await;

        assert_eq!(client.ensure_root().await.unwrap(), "/Slot2Sync");
    }
}
