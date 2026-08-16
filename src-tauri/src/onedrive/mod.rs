//! Cliente do Microsoft Graph (OneDrive) — uma das implementações concretas
//! de `remote::RemoteProvider`.
//!
//! Escopo `Files.ReadWrite.AppFolder`: o app só enxerga sua própria pasta
//! especial, `/me/drive/special/approot` — equivalente ao `drive.file` do
//! Drive e à App Folder do Dropbox. Endereça por **path** dentro da approot
//! (sintaxe `approot:/<path>:`), como o Dropbox — `id`/`folder_id` no trait
//! são sempre esse path relativo (ex.: `PPSSPP/saves`), nunca o item ID
//! opaco do Graph, o que evita precisar de um cache de IDs.
//!
//! Sem um equivalente simples a `appProperties` do Drive, a atribuição de
//! dispositivo usa o índice compartilhado (`remote::device_index`), guardado
//! em `.slot2sync-index.json` na raiz da approot.
//!
//! Upload simples (`PUT .../content`): limite de 4 MB da API do Graph,
//! suficiente para saves/savestates típicos; arquivos maiores precisariam de
//! `createUploadSession` (não implementado). O mtime não vai junto do
//! upload — precisa um `PATCH` de `fileSystemInfo` logo depois (uma chamada
//! HTTP extra por upload, documentado onde acontece).

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::json;

use crate::auth::AuthManager;
use crate::constants::DRIVE_MAX_RETRIES;
use crate::error::{AppError, AppResult};
use crate::remote::device_index::{DeviceEntry, DeviceIndex, INDEX_FILE_NAME};
use crate::remote::http::{send_with_retry, RateLimiter};
use crate::remote::{BatchUploadOp, DeviceTag, RemoteFile, RemoteProvider};
use crate::sync::SyncCategory;

const GRAPH_BASE: &str = "https://graph.microsoft.com/v1.0";

pub struct OneDriveClient {
    http: reqwest::Client,
    auth: Arc<AuthManager>,
    base: String,
    limiter: RateLimiter,
}

impl OneDriveClient {
    pub fn new(http: reqwest::Client, auth: Arc<AuthManager>) -> Self {
        Self {
            http,
            auth,
            base: GRAPH_BASE.to_string(),
            limiter: RateLimiter::default(),
        }
    }

    #[cfg(test)]
    fn with_base_url(mut self, base: &str) -> Self {
        self.base = base.to_string();
        self
    }

    async fn send<F>(&self, op_name: &str, build: F) -> AppResult<reqwest::Response>
    where
        F: Fn(&str) -> reqwest::RequestBuilder,
    {
        send_with_retry(&self.auth, op_name, DRIVE_MAX_RETRIES, |_, _| false, build).await
    }

    /// URL do item pela sintaxe de path da approot. `""` = a própria raiz.
    fn item_url(&self, rel_path: &str) -> String {
        if rel_path.is_empty() {
            format!("{}/me/drive/special/approot", self.base)
        } else {
            format!("{}/me/drive/special/approot:/{rel_path}:", self.base)
        }
    }

    async fn get_item(&self, rel_path: &str) -> AppResult<Option<GraphItem>> {
        let url = self.item_url(rel_path);
        let result = self
            .send("drive.item.get", |token| {
                self.http.get(&url).bearer_auth(token)
            })
            .await;
        match result {
            Ok(response) => Ok(Some(response.json::<GraphItem>().await?)),
            Err(AppError::RemoteObjectNotFound(_)) => Ok(None),
            Err(err) => Err(err),
        }
    }

    async fn ensure_folder(&self, rel_path: &str) -> AppResult<()> {
        if self.get_item(rel_path).await?.is_some() {
            return Ok(());
        }
        let (parent, name) = match rel_path.rsplit_once('/') {
            Some((dir, name)) => (dir.to_string(), name),
            None => (String::new(), rel_path),
        };
        let url = format!("{}/children", self.item_url(&parent));
        let body = json!({
            "name": name,
            "folder": {},
            "@microsoft.graph.conflictBehavior": "fail",
        });
        let result = self
            .send("drive.item.create_folder", |token| {
                self.http.post(&url).bearer_auth(token).json(&body)
            })
            .await;
        match result {
            Ok(_) => Ok(()),
            // Corrida: outra chamada já criou a pasta entre o GET e o POST.
            Err(AppError::Other(msg)) if msg.contains("nameAlreadyExists") => Ok(()),
            Err(err) => Err(err),
        }
    }

    async fn load_index(&self) -> DeviceIndex {
        match self.download(INDEX_FILE_NAME).await {
            Ok(bytes) => DeviceIndex::parse(&bytes),
            Err(_) => DeviceIndex::default(),
        }
    }

    async fn save_index(&self, index: &DeviceIndex) {
        if let Err(err) = self.put_content(INDEX_FILE_NAME, index.to_bytes()).await {
            tracing::warn!(error = %err, "falha ao atualizar índice de dispositivo do OneDrive");
        }
    }

    async fn stamp_device(&self, rel_path: &str, device: DeviceTag<'_>) {
        if device.name.is_none() && device.id.is_none() {
            return;
        }
        let mut index = self.load_index().await;
        index.set(
            rel_path,
            DeviceEntry {
                device_name: device.name.map(str::to_string),
                device_id: device.id.map(str::to_string),
            },
        );
        self.save_index(&index).await;
    }

    async fn put_content(&self, rel_path: &str, content: Vec<u8>) -> AppResult<GraphItem> {
        self.limiter.throttle(content.len(), 0).await;
        let url = format!("{}/content", self.item_url(rel_path));
        let response = self
            .send("drive.item.upload", |token| {
                self.http
                    .put(&url)
                    .bearer_auth(token)
                    .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
                    .body(content.clone())
            })
            .await?;
        Ok(response.json::<GraphItem>().await?)
    }

    /// `PATCH fileSystemInfo` — o Graph não aceita mtime no upload em si.
    async fn set_mtime(&self, rel_path: &str, mtime_ms: i64) -> AppResult<GraphItem> {
        let url = self.item_url(rel_path);
        let body = json!({ "fileSystemInfo": { "lastModifiedDateTime": ms_to_rfc3339(mtime_ms) } });
        let response = self
            .send("drive.item.patch_mtime", |token| {
                self.http.patch(&url).bearer_auth(token).json(&body)
            })
            .await?;
        Ok(response.json::<GraphItem>().await?)
    }

    async fn upload_and_stamp(
        &self,
        rel_path: &str,
        content: Vec<u8>,
        mtime_ms: i64,
        device: DeviceTag<'_>,
    ) -> AppResult<RemoteFile> {
        self.put_content(rel_path, content).await?;
        let item = self.set_mtime(rel_path, mtime_ms).await?;
        self.stamp_device(rel_path, device).await;
        Ok(self
            .to_remote_file(
                &item,
                rel_path.rsplit('/').next().unwrap_or(rel_path).to_string(),
            )
            .await)
    }

    async fn to_remote_file(&self, item: &GraphItem, rel_path: String) -> RemoteFile {
        let index = self.load_index().await;
        let key = item.path_key().unwrap_or_else(|| rel_path.clone());
        let device = index.get(&key).cloned();
        RemoteFile {
            id: item.id.clone(),
            rel_path,
            modified_ms: item.modified_ms(),
            size_bytes: item.size.map(|s| s as i64),
            hash: item.hashes.as_ref().and_then(|h| h.quick_xor_hash.clone()),
            device_name: device.as_ref().and_then(|e| e.device_name.clone()),
            device_id: device.as_ref().and_then(|e| e.device_id.clone()),
        }
    }
}

fn ms_to_rfc3339(ms: i64) -> String {
    DateTime::<Utc>::from_timestamp_millis(ms)
        .unwrap_or_else(Utc::now)
        .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

#[derive(Debug, Clone, Deserialize, Default)]
struct GraphHashes {
    #[serde(rename = "quickXorHash", default)]
    quick_xor_hash: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct FileSystemInfo {
    #[serde(rename = "lastModifiedDateTime", default)]
    last_modified_date_time: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct GraphItem {
    id: String,
    name: String,
    #[serde(default)]
    size: Option<u64>,
    #[serde(default)]
    folder: Option<serde_json::Value>,
    #[serde(rename = "fileSystemInfo", default)]
    file_system_info: Option<FileSystemInfo>,
    #[serde(default)]
    hashes: Option<GraphHashes>,
    #[serde(rename = "parentReference", default)]
    parent_reference: Option<ParentReference>,
}

#[derive(Debug, Clone, Deserialize)]
struct ParentReference {
    #[serde(default)]
    path: Option<String>,
}

impl GraphItem {
    fn is_folder(&self) -> bool {
        self.folder.is_some()
    }

    fn modified_ms(&self) -> Option<i64> {
        self.file_system_info
            .as_ref()
            .and_then(|f| f.last_modified_date_time.as_deref())
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|dt| dt.timestamp_millis())
    }

    /// Chave do índice de dispositivo: path relativo à approot, reconstruído
    /// de `parentReference.path` (que vem como
    /// `/drive/root:/Aplicativos/Slot2Sync/PPSSPP/saves`) + nome.
    fn path_key(&self) -> Option<String> {
        let parent = self.parent_reference.as_ref()?.path.as_deref()?;
        let after_approot = parent.split("approot").nth(1).unwrap_or("");
        let trimmed = after_approot
            .trim_start_matches(':')
            .trim_start_matches('/');
        Some(if trimmed.is_empty() {
            self.name.clone()
        } else {
            format!("{trimmed}/{}", self.name)
        })
    }
}

#[derive(Debug, Deserialize)]
struct ChildrenResult {
    value: Vec<GraphItem>,
    #[serde(rename = "@odata.nextLink", default)]
    next_link: Option<String>,
}

#[async_trait]
impl RemoteProvider for OneDriveClient {
    async fn ensure_root(&self) -> AppResult<String> {
        // A approot já existe por definição (é a pasta especial do app) —
        // nada a criar. Path relativo vazio = a própria raiz.
        Ok(String::new())
    }

    async fn ensure_category_folder(
        &self,
        emulator: &str,
        category: SyncCategory,
    ) -> AppResult<String> {
        let emulator_path = emulator.to_string();
        self.ensure_folder(&emulator_path).await?;
        let category_path = format!("{emulator_path}/{}", category.as_str());
        self.ensure_folder(&category_path).await?;
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
            current = if current.is_empty() {
                segment.to_string()
            } else {
                format!("{current}/{segment}")
            };
            self.ensure_folder(&current).await?;
        }
        Ok(current)
    }

    async fn list_tree(&self, folder_id: &str) -> AppResult<Vec<RemoteFile>> {
        let mut out = Vec::new();
        let mut pending = vec![folder_id.to_string()];
        while let Some(dir) = pending.pop() {
            let mut url = format!("{}/children", self.item_url(&dir));
            loop {
                let response = self
                    .send("drive.item.children", |token| {
                        self.http.get(&url).bearer_auth(token)
                    })
                    .await?;
                let page: ChildrenResult = response.json().await?;
                for item in &page.value {
                    if item.is_folder() {
                        let child_path = if dir.is_empty() {
                            item.name.clone()
                        } else {
                            format!("{dir}/{}", item.name)
                        };
                        pending.push(child_path);
                        continue;
                    }
                    if item.name == INDEX_FILE_NAME {
                        continue;
                    }
                    let rel_path = item
                        .path_key()
                        .and_then(|full| full.strip_prefix(folder_id).map(str::to_string))
                        .map(|s| s.trim_start_matches('/').to_string())
                        .unwrap_or_else(|| item.name.clone());
                    out.push(self.to_remote_file(item, rel_path).await);
                }
                match page.next_link {
                    Some(next) => url = next,
                    None => break,
                }
            }
        }
        Ok(out)
    }

    async fn find_child(&self, folder_id: &str, name: &str) -> AppResult<Option<RemoteFile>> {
        let rel_path = if folder_id.is_empty() {
            name.to_string()
        } else {
            format!("{folder_id}/{name}")
        };
        match self.get_item(&rel_path).await? {
            Some(item) if !item.is_folder() => {
                Some(self.to_remote_file(&item, name.to_string()).await)
            }
            _ => None,
        }
        .map(Ok)
        .transpose()
    }

    async fn download(&self, file_id: &str) -> AppResult<Vec<u8>> {
        let url = format!("{}/content", self.item_url(file_id));
        let response = self
            .send("drive.item.download", |token| {
                self.http.get(&url).bearer_auth(token)
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
        let rel_path = if parent_id.is_empty() {
            name.to_string()
        } else {
            format!("{parent_id}/{name}")
        };
        self.upload_and_stamp(&rel_path, content, mtime_ms, device)
            .await
    }

    async fn upload_existing(
        &self,
        file_id: &str,
        content: Vec<u8>,
        mtime_ms: i64,
        device: DeviceTag<'_>,
    ) -> AppResult<RemoteFile> {
        self.upload_and_stamp(file_id, content, mtime_ms, device)
            .await
    }

    async fn upload_batch(&self, ops: Vec<BatchUploadOp>) -> AppResult<Vec<RemoteFile>> {
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
        let parent = add_parent.map(str::to_string).unwrap_or_else(|| {
            file_id
                .rsplit_once('/')
                .map(|(dir, _)| dir.to_string())
                .unwrap_or_default()
        });
        let new_rel_path = if parent.is_empty() {
            new_name.to_string()
        } else {
            format!("{parent}/{new_name}")
        };

        let url = self.item_url(file_id);
        let body = json!({ "name": new_name });
        let response = self
            .send("drive.item.rename", |token| {
                self.http.patch(&url).bearer_auth(token).json(&body)
            })
            .await?;
        let item: GraphItem = response.json().await?;

        let mut index = self.load_index().await;
        index.rename(file_id, &new_rel_path);
        self.save_index(&index).await;

        Ok(self.to_remote_file(&item, new_name.to_string()).await)
    }

    async fn invalidate_folder_path(&self, _cache_key: &str) {}

    async fn clear_folder_cache(&self) {}
}

#[cfg(test)]
mod tests {
    use wiremock::matchers::{method, path_regex};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::secrets::MemSecrets;

    async fn client_against(server: &MockServer) -> OneDriveClient {
        let secrets: Arc<dyn crate::secrets::SecretStore> = Arc::new(MemSecrets::default());
        let auth = Arc::new(AuthManager::new(reqwest::Client::new(), secrets));
        auth.set_test_access_token("tok-teste").await;
        OneDriveClient::new(reqwest::Client::new(), auth).with_base_url(&server.uri())
    }

    #[tokio::test]
    async fn download_retorna_os_bytes() {
        let server = MockServer::start().await;
        let client = client_against(&server).await;

        Mock::given(method("GET"))
            .and(path_regex(r".*/content$"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"conteudo".to_vec()))
            .mount(&server)
            .await;

        assert_eq!(
            client.download("PPSSPP/saves/save.bin").await.unwrap(),
            b"conteudo"
        );
    }

    #[tokio::test]
    async fn list_tree_ignora_o_indice_de_dispositivo() {
        let server = MockServer::start().await;
        let client = client_against(&server).await;

        Mock::given(method("GET"))
            .and(path_regex(r".*/children$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "value": [
                    {"id": "1", "name": "save.bin", "size": 10, "parentReference": {"path": "/drive/root:/Aplicativos/Slot2Sync/PPSSPP/saves"}},
                    {"id": "2", "name": ".slot2sync-index.json", "size": 2, "parentReference": {"path": "/drive/root:/Aplicativos/Slot2Sync/PPSSPP/saves"}},
                ]
            })))
            .mount(&server)
            .await;

        let tree = client.list_tree("PPSSPP/saves").await.unwrap();
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].rel_path, "save.bin");
    }

    #[tokio::test]
    async fn ensure_root_e_no_op_e_retorna_caminho_vazio() {
        // A approot já existe por definição — não faz nenhuma chamada HTTP.
        let server = MockServer::start().await;
        let client = client_against(&server).await;
        assert_eq!(client.ensure_root().await.unwrap(), "");
    }

    #[tokio::test]
    async fn ensure_category_folder_cria_emulador_e_categoria() {
        let server = MockServer::start().await;
        let client = client_against(&server).await;

        Mock::given(method("GET"))
            .and(path_regex(r".*/approot.*"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/children$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "id:folder", "name": "saves", "folder": {},
            })))
            .mount(&server)
            .await;

        let category_path = client
            .ensure_category_folder("PPSSPP", SyncCategory::Saves)
            .await
            .unwrap();
        assert_eq!(category_path, "PPSSPP/saves");
    }

    #[tokio::test]
    async fn ensure_folder_tolera_corrida_de_criacao() {
        let server = MockServer::start().await;
        let client = client_against(&server).await;

        Mock::given(method("GET"))
            .and(path_regex(r".*/approot.*"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/children$"))
            .respond_with(
                ResponseTemplate::new(409).set_body_string(
                    r#"{"error": {"code": "nameAlreadyExists", "message": "..."}}"#,
                ),
            )
            .mount(&server)
            .await;

        let category_path = client
            .ensure_category_folder("PPSSPP", SyncCategory::Saves)
            .await
            .unwrap();
        assert_eq!(category_path, "PPSSPP/saves");
    }

    #[tokio::test]
    async fn ensure_subpath_cria_a_cadeia_de_subpastas() {
        let server = MockServer::start().await;
        let client = client_against(&server).await;

        Mock::given(method("GET"))
            .and(path_regex(r".*/approot.*"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path_regex(r".*/children$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "id:folder", "name": "x", "folder": {},
            })))
            .mount(&server)
            .await;

        let leaf = client
            .ensure_subpath("PPSSPP/saves", "", "jogo/slot1")
            .await
            .unwrap();
        assert_eq!(leaf, "PPSSPP/saves/jogo/slot1");
    }

    #[tokio::test]
    async fn find_child_retorna_none_quando_nao_encontrado() {
        let server = MockServer::start().await;
        let client = client_against(&server).await;

        Mock::given(method("GET"))
            .and(path_regex(r".*/approot.*"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let found = client.find_child("PPSSPP/saves", "save.bin").await.unwrap();
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn find_child_ignora_pastas_com_o_mesmo_nome() {
        let server = MockServer::start().await;
        let client = client_against(&server).await;

        Mock::given(method("GET"))
            .and(path_regex(r".*/approot.*"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "id:folder1", "name": "jogo", "folder": {},
            })))
            .mount(&server)
            .await;

        let found = client.find_child("PPSSPP/saves", "jogo").await.unwrap();
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn find_child_retorna_o_arquivo_quando_encontrado() {
        let server = MockServer::start().await;
        let client = client_against(&server).await;

        Mock::given(method("GET"))
            .and(path_regex(r".*/approot.*"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "id:file1", "name": "save.bin", "size": 10,
            })))
            .mount(&server)
            .await;

        let found = client
            .find_child("PPSSPP/saves", "save.bin")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(found.id, "id:file1");
        assert_eq!(found.rel_path, "save.bin");
    }

    #[tokio::test]
    async fn upload_existing_atualiza_conteudo_e_mtime() {
        let server = MockServer::start().await;
        let client = client_against(&server).await;

        Mock::given(method("PUT"))
            .and(path_regex(r".*/content$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "id:file1", "name": "save.bin", "size": 6,
            })))
            .mount(&server)
            .await;
        Mock::given(method("PATCH"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "id:file1",
                "name": "save.bin",
                "size": 6,
                "fileSystemInfo": {"lastModifiedDateTime": "2023-11-14T22:13:20Z"},
            })))
            .mount(&server)
            .await;

        let file = client
            .upload_existing(
                "PPSSPP/saves/save.bin",
                b"novos!".to_vec(),
                1_700_000_000_000,
                DeviceTag::default(),
            )
            .await
            .unwrap();
        assert_eq!(file.id, "id:file1");
        assert_eq!(file.modified_ms, Some(1_700_000_000_000));
    }

    #[tokio::test]
    async fn upload_new_com_device_tag_atualiza_o_indice_sem_erro() {
        let server = MockServer::start().await;
        let client = client_against(&server).await;

        Mock::given(method("PUT"))
            .and(path_regex(r".*/content$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "id:new1", "name": "save.bin", "size": 5,
            })))
            .mount(&server)
            .await;
        Mock::given(method("PATCH"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "id:new1", "name": "save.bin", "size": 5,
            })))
            .mount(&server)
            .await;
        // `stamp_device` primeiro tenta baixar o índice existente.
        Mock::given(method("GET"))
            .and(path_regex(r".*/content$"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let file = client
            .upload_new(
                "PPSSPP/saves",
                "save.bin",
                b"dados".to_vec(),
                1_700_000_000_000,
                DeviceTag {
                    name: Some("PC-1"),
                    id: Some("dev-1"),
                },
            )
            .await
            .unwrap();
        assert_eq!(file.id, "id:new1");
    }

    #[tokio::test]
    async fn upload_batch_envia_cada_item_e_preserva_a_ordem() {
        let server = MockServer::start().await;
        let client = client_against(&server).await;

        Mock::given(method("PUT"))
            .and(path_regex(r".*/content$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "id:batch", "name": "x", "size": 1,
            })))
            .mount(&server)
            .await;
        Mock::given(method("PATCH"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "id:batch", "name": "x", "size": 1,
            })))
            .mount(&server)
            .await;

        let ops = vec![
            BatchUploadOp {
                parent_id: "PPSSPP/saves".into(),
                name: "a.bin".into(),
                content: b"a".to_vec(),
                mtime_ms: 1,
                device_name: None,
                device_id: None,
            },
            BatchUploadOp {
                parent_id: "PPSSPP/saves".into(),
                name: "b.bin".into(),
                content: b"b".to_vec(),
                mtime_ms: 2,
                device_name: None,
                device_id: None,
            },
        ];
        let files = client.upload_batch(ops).await.unwrap();
        assert_eq!(files.len(), 2);
    }

    #[tokio::test]
    async fn rename_file_atualiza_o_indice_e_devolve_o_novo_nome() {
        let server = MockServer::start().await;
        let client = client_against(&server).await;

        Mock::given(method("PATCH"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "id:renamed", "name": "save2.bin", "size": 5,
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path_regex(r".*/content$"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        Mock::given(method("PUT"))
            .and(path_regex(r".*/content$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "id:index", "name": INDEX_FILE_NAME, "size": 2,
            })))
            .mount(&server)
            .await;

        let file = client
            .rename_file("PPSSPP/saves/save.bin", "save2.bin", None, None)
            .await
            .unwrap();
        assert_eq!(file.id, "id:renamed");
        assert_eq!(file.rel_path, "save2.bin");
    }

    #[test]
    fn ms_to_rfc3339_formata_timestamp_em_utc() {
        assert_eq!(ms_to_rfc3339(1_700_000_000_000), "2023-11-14T22:13:20Z");
    }

    fn item(
        folder: bool,
        fsi: Option<FileSystemInfo>,
        parent: Option<ParentReference>,
    ) -> GraphItem {
        GraphItem {
            id: "id:1".into(),
            name: "save.bin".into(),
            size: Some(10),
            folder: folder.then(|| serde_json::json!({})),
            file_system_info: fsi,
            hashes: None,
            parent_reference: parent,
        }
    }

    #[test]
    fn graph_item_is_folder_reflete_o_campo_folder() {
        assert!(!item(false, None, None).is_folder());
        assert!(item(true, None, None).is_folder());
    }

    #[test]
    fn graph_item_modified_ms_le_o_file_system_info() {
        assert_eq!(item(false, None, None).modified_ms(), None);
        let with_fsi = item(
            false,
            Some(FileSystemInfo {
                last_modified_date_time: Some("2023-11-14T22:13:20Z".into()),
            }),
            None,
        );
        assert_eq!(with_fsi.modified_ms(), Some(1_700_000_000_000));
    }

    #[test]
    fn graph_item_path_key_none_sem_parent_reference() {
        assert_eq!(item(false, None, None).path_key(), None);
    }

    #[test]
    fn graph_item_path_key_reconstroi_o_caminho_quando_o_marcador_approot_esta_presente() {
        let with_parent = item(
            false,
            None,
            Some(ParentReference {
                path: Some("/drive/special/approot:/PPSSPP/saves".into()),
            }),
        );
        assert_eq!(
            with_parent.path_key().as_deref(),
            Some("PPSSPP/saves/save.bin")
        );
    }

    #[test]
    fn graph_item_path_key_degrada_para_o_nome_sem_o_marcador_approot() {
        // Comportamento real observado (ver `list_tree_ignora_o_indice_de_dispositivo`):
        // quando o path do Graph não contém "approot" literalmente, a função
        // degrada para só o nome do item.
        let with_parent = item(
            false,
            None,
            Some(ParentReference {
                path: Some("/drive/root:/Aplicativos/Slot2Sync/PPSSPP/saves".into()),
            }),
        );
        assert_eq!(with_parent.path_key().as_deref(), Some("save.bin"));
    }
}
