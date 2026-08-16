//! `impl remote::RemoteProvider for DriveClient` — delegação simples para os
//! métodos inerentes de `DriveClient` (já retornam os tipos genéricos de
//! `remote`), permitindo ao `SyncEngine` depender só do trait.

use async_trait::async_trait;

use super::DriveClient;
use crate::error::AppResult;
use crate::remote::{BatchUploadOp, DeviceTag, RemoteFile, RemoteProvider};
use crate::sync::SyncCategory;

#[async_trait]
impl RemoteProvider for DriveClient {
    async fn ensure_root(&self) -> AppResult<String> {
        DriveClient::ensure_root(self).await
    }

    async fn ensure_category_folder(
        &self,
        emulator: &str,
        category: SyncCategory,
    ) -> AppResult<String> {
        DriveClient::ensure_category_folder(self, emulator, category).await
    }

    async fn ensure_subpath(
        &self,
        base_id: &str,
        base_key: &str,
        rel_dir: &str,
    ) -> AppResult<String> {
        DriveClient::ensure_subpath(self, base_id, base_key, rel_dir).await
    }

    async fn list_tree(&self, folder_id: &str) -> AppResult<Vec<RemoteFile>> {
        DriveClient::list_tree(self, folder_id).await
    }

    async fn find_child(&self, folder_id: &str, name: &str) -> AppResult<Option<RemoteFile>> {
        DriveClient::find_child(self, folder_id, name).await
    }

    async fn download(&self, file_id: &str) -> AppResult<Vec<u8>> {
        DriveClient::download(self, file_id).await
    }

    async fn upload_new(
        &self,
        parent_id: &str,
        name: &str,
        content: Vec<u8>,
        mtime_ms: i64,
        device: DeviceTag<'_>,
    ) -> AppResult<RemoteFile> {
        DriveClient::upload_new(self, parent_id, name, content, mtime_ms, device).await
    }

    async fn upload_existing(
        &self,
        file_id: &str,
        content: Vec<u8>,
        mtime_ms: i64,
        device: DeviceTag<'_>,
    ) -> AppResult<RemoteFile> {
        DriveClient::upload_existing(self, file_id, content, mtime_ms, device).await
    }

    async fn upload_batch(&self, ops: Vec<BatchUploadOp>) -> AppResult<Vec<RemoteFile>> {
        DriveClient::upload_batch(self, ops).await
    }

    async fn rename_file(
        &self,
        file_id: &str,
        new_name: &str,
        add_parent: Option<&str>,
        remove_parent: Option<&str>,
    ) -> AppResult<RemoteFile> {
        DriveClient::rename_file(self, file_id, new_name, add_parent, remove_parent).await
    }

    async fn invalidate_folder_path(&self, cache_key: &str) {
        DriveClient::invalidate_folder_path(self, cache_key).await
    }

    async fn clear_folder_cache(&self) {
        DriveClient::clear_folder_cache(self).await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::auth::AuthManager;
    use crate::drive::test_support::client_against;
    use crate::secrets::MemSecrets;
    use crate::storage::db::Db;
    use crate::storage::drive_folders;

    /// Exercita a delegação do trait para o `DriveClient` real nos métodos que
    /// dependem de HTTP (`list_tree`/`find_child`/`download`), contra o mesmo
    /// `MockServer` usado por `drive::files`. Uma delegação trocada (copy/paste
    /// para o método errado) falharia aqui.
    #[tokio::test]
    async fn delegacao_dos_metodos_http_para_o_cliente_real() {
        let server = MockServer::start().await;
        let client = client_against(&server).await;
        let api: &dyn RemoteProvider = &client;

        Mock::given(method("GET"))
            .and(path("/drive/v3/files"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "files": [{"id": "f1", "name": "save.bin", "mimeType": "application/octet-stream"}]
            })))
            .mount(&server)
            .await;
        let tree = api.list_tree("root-id").await.unwrap();
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].rel_path, "save.bin");

        let found = api.find_child("root-id", "save.bin").await.unwrap();
        assert_eq!(found.unwrap().id, "f1");

        Mock::given(method("GET"))
            .and(path("/drive/v3/files/f1"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"conteudo".to_vec()))
            .mount(&server)
            .await;
        assert_eq!(api.download("f1").await.unwrap(), b"conteudo");
    }

    /// Exercita a delegação do trait para o `DriveClient` real pelos caminhos
    /// que dispensam rede (cache persistido + batch vazio). Uma delegação
    /// trocada (copy/paste para o método errado) falharia aqui. Os métodos
    /// restantes de upload/rename exigem HTTP real e ficam para os testes
    /// atrás da feature `integration-tests`.
    #[tokio::test]
    async fn delegacao_para_o_cliente_real_nos_caminhos_sem_rede() {
        let db = Db::open_in_memory().unwrap();
        db.with_sync(|conn| {
            for (key, id) in [
                ("Slot2Sync", "id-root"),
                ("Slot2Sync/PPSSPP", "id-emu"),
                ("Slot2Sync/PPSSPP/saves", "id-saves"),
                ("Slot2Sync/PPSSPP/saves/jogo", "id-jogo"),
            ] {
                drive_folders::upsert(conn, key, id)?;
            }
            Ok(())
        });
        let auth = Arc::new(AuthManager::new(
            reqwest::Client::new(),
            Arc::new(MemSecrets::default()),
        ));
        let client = DriveClient::new(reqwest::Client::new(), auth, db);
        let api: &dyn RemoteProvider = &client;

        assert_eq!(api.ensure_root().await.unwrap(), "id-root");
        assert_eq!(
            api.ensure_category_folder("PPSSPP", SyncCategory::Saves)
                .await
                .unwrap(),
            "id-saves"
        );
        assert_eq!(
            api.ensure_subpath("id-saves", "Slot2Sync/PPSSPP/saves", "jogo")
                .await
                .unwrap(),
            "id-jogo"
        );
        assert!(api.upload_batch(Vec::new()).await.unwrap().is_empty());

        api.invalidate_folder_path("Slot2Sync/PPSSPP").await;
        api.clear_folder_cache().await;
        assert!(client.folder_cache.read().await.is_empty());
    }
}
