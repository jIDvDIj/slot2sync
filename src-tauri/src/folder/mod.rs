//! `FolderProvider` — implementação de `remote::RemoteProvider` sobre uma
//! pasta local ou de rede (sem OAuth, sem API externa). O "provedor remoto"
//! aqui é literalmente um diretório escolhido pelo usuário — útil para quem
//! já sincroniza essa pasta por fora (Syncthing, OneDrive/Nextcloud
//! sincronizando a pasta, um compartilhamento de rede) ou só quer um backup
//! local simples.
//!
//! Estrutura espelha a do Drive: `<raiz>/Slot2Sync/<Emulador>/<categoria>/`.
//! `id`/`file_id`/`folder_id` são sempre o caminho absoluto do próprio
//! arquivo/pasta — string opaca do ponto de vista do trait, como os IDs do
//! Drive.
//!
//! Sem um equivalente a `appProperties` do Drive, a atribuição de dispositivo
//! usa o índice compartilhado (`remote::device_index`), num único arquivo
//! `.slot2sync-index.json` na raiz da pasta escolhida, chaveado pelo caminho
//! relativo a essa raiz (independente de emulador/categoria).

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use tokio::fs;

use crate::constants::{
    DRIVE_CONFIG_FOLDER, DRIVE_ROOT_FOLDER, DRIVE_SAVES_FOLDER, DRIVE_STATES_FOLDER, TMP_SUFFIX,
};
use crate::error::{AppError, AppResult};
use crate::remote::device_index::{DeviceEntry, DeviceIndex, INDEX_FILE_NAME};
use crate::remote::{BatchUploadOp, DeviceTag, RemoteFile, RemoteProvider};
use crate::sync::{sha256_hex, SyncCategory};

pub struct FolderProvider {
    root: PathBuf,
}

impl FolderProvider {
    /// `root` é a pasta escolhida pelo usuário — precisa existir e ser
    /// gravável; a validação acontece no comando `connect_local_folder`, não
    /// aqui (construtor infalível, como os demais clientes).
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn index_path(&self) -> PathBuf {
        self.root.join(INDEX_FILE_NAME)
    }

    async fn load_index(&self) -> DeviceIndex {
        match fs::read(self.index_path()).await {
            Ok(bytes) => DeviceIndex::parse(&bytes),
            Err(_) => DeviceIndex::default(),
        }
    }

    async fn save_index(&self, index: &DeviceIndex) -> AppResult<()> {
        write_atomic(&self.index_path(), &index.to_bytes()).await
    }

    /// Chave do índice de dispositivo: caminho relativo à raiz da pasta
    /// escolhida (não à categoria — um único índice cobre tudo).
    fn index_key(&self, abs_path: &Path) -> String {
        abs_path
            .strip_prefix(&self.root)
            .unwrap_or(abs_path)
            .to_string_lossy()
            .replace('\\', "/")
    }

    async fn stamp_device(&self, abs_path: &Path, device: DeviceTag<'_>) {
        if device.name.is_none() && device.id.is_none() {
            return;
        }
        let key = self.index_key(abs_path);
        let mut index = self.load_index().await;
        index.set(
            &key,
            DeviceEntry {
                device_name: device.name.map(str::to_string),
                device_id: device.id.map(str::to_string),
            },
        );
        if let Err(err) = self.save_index(&index).await {
            tracing::warn!(error = %err, "falha ao atualizar índice de dispositivo da pasta local");
        }
    }

    async fn remote_file_for(&self, abs_path: &Path, rel_path: String) -> AppResult<RemoteFile> {
        let metadata = fs::metadata(abs_path).await?;
        let content = fs::read(abs_path).await?;
        let index = self.load_index().await;
        let entry = index.get(&self.index_key(abs_path)).cloned();
        Ok(RemoteFile {
            id: abs_path.to_string_lossy().into_owned(),
            rel_path,
            modified_ms: metadata.modified().ok().map(system_time_ms),
            size_bytes: Some(metadata.len() as i64),
            hash: Some(sha256_hex(&content)),
            device_name: entry.as_ref().and_then(|e| e.device_name.clone()),
            device_id: entry.as_ref().and_then(|e| e.device_id.clone()),
        })
    }
}

/// Escreve com rename atômico (mesmo padrão `TMP_SUFFIX` usado pela escrita
/// local em `sync::storage`): evita que um crash a meio da escrita deixe um
/// arquivo corrompido no lugar do original.
async fn write_atomic(dest: &Path, content: &[u8]) -> AppResult<()> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent).await?;
    }
    let tmp = dest.with_file_name(format!(
        "{}{TMP_SUFFIX}",
        dest.file_name().unwrap_or_default().to_string_lossy()
    ));
    fs::write(&tmp, content).await?;
    fs::rename(&tmp, dest).await?;
    Ok(())
}

fn category_dir_name(category: SyncCategory) -> &'static str {
    match category {
        SyncCategory::Saves => DRIVE_SAVES_FOLDER,
        SyncCategory::Savestates => DRIVE_STATES_FOLDER,
        SyncCategory::Config => DRIVE_CONFIG_FOLDER,
    }
}

#[async_trait]
impl RemoteProvider for FolderProvider {
    async fn ensure_root(&self) -> AppResult<String> {
        let path = self.root.join(DRIVE_ROOT_FOLDER);
        fs::create_dir_all(&path).await?;
        Ok(path.to_string_lossy().into_owned())
    }

    async fn ensure_category_folder(
        &self,
        emulator: &str,
        category: SyncCategory,
    ) -> AppResult<String> {
        let path = self
            .root
            .join(DRIVE_ROOT_FOLDER)
            .join(emulator)
            .join(category_dir_name(category));
        fs::create_dir_all(&path).await?;
        Ok(path.to_string_lossy().into_owned())
    }

    async fn ensure_subpath(
        &self,
        base_id: &str,
        _base_key: &str,
        rel_dir: &str,
    ) -> AppResult<String> {
        let path = Path::new(base_id).join(rel_dir);
        fs::create_dir_all(&path).await?;
        Ok(path.to_string_lossy().into_owned())
    }

    async fn list_tree(&self, folder_id: &str) -> AppResult<Vec<RemoteFile>> {
        let base = PathBuf::from(folder_id);
        let mut out = Vec::new();
        let mut pending = vec![base.clone()];
        while let Some(dir) = pending.pop() {
            let mut entries = match fs::read_dir(&dir).await {
                Ok(e) => e,
                Err(_) => continue,
            };
            while let Some(entry) = entries.next_entry().await? {
                let path = entry.path();
                let file_type = entry.file_type().await?;
                if file_type.is_dir() {
                    pending.push(path);
                    continue;
                }
                let name = entry.file_name().to_string_lossy().into_owned();
                if name == INDEX_FILE_NAME || name.ends_with(TMP_SUFFIX) {
                    continue;
                }
                let rel_path = path
                    .strip_prefix(&base)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/");
                out.push(self.remote_file_for(&path, rel_path).await?);
            }
        }
        Ok(out)
    }

    async fn find_child(&self, folder_id: &str, name: &str) -> AppResult<Option<RemoteFile>> {
        let path = Path::new(folder_id).join(name);
        if fs::metadata(&path).await.is_err() {
            return Ok(None);
        }
        Ok(Some(self.remote_file_for(&path, name.to_string()).await?))
    }

    async fn download(&self, file_id: &str) -> AppResult<Vec<u8>> {
        Ok(fs::read(file_id).await?)
    }

    async fn upload_new(
        &self,
        parent_id: &str,
        name: &str,
        content: Vec<u8>,
        mtime_ms: i64,
        device: DeviceTag<'_>,
    ) -> AppResult<RemoteFile> {
        let path = Path::new(parent_id).join(name);
        write_atomic(&path, &content).await?;
        set_mtime(&path, mtime_ms)?;
        self.stamp_device(&path, device).await;
        self.remote_file_for(&path, name.to_string()).await
    }

    async fn upload_existing(
        &self,
        file_id: &str,
        content: Vec<u8>,
        mtime_ms: i64,
        device: DeviceTag<'_>,
    ) -> AppResult<RemoteFile> {
        let path = PathBuf::from(file_id);
        write_atomic(&path, &content).await?;
        set_mtime(&path, mtime_ms)?;
        self.stamp_device(&path, device).await;
        self.remote_file_for(&path, String::new()).await
    }

    async fn upload_batch(&self, ops: Vec<BatchUploadOp>) -> AppResult<Vec<RemoteFile>> {
        // Sem ganho real de agrupar requests num filesystem — laço simples.
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
        let old_path = PathBuf::from(file_id);
        let new_dir = match add_parent {
            Some(dir) => PathBuf::from(dir),
            None => old_path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| self.root.clone()),
        };
        fs::create_dir_all(&new_dir).await?;
        let new_path = new_dir.join(new_name);
        fs::rename(&old_path, &new_path).await?;

        let mut index = self.load_index().await;
        index.rename(&self.index_key(&old_path), &self.index_key(&new_path));
        if let Err(err) = self.save_index(&index).await {
            tracing::warn!(error = %err, "falha ao atualizar índice de dispositivo após rename");
        }

        self.remote_file_for(&new_path, new_name.to_string()).await
    }

    async fn invalidate_folder_path(&self, _cache_key: &str) {
        // Sem cache — a leitura do FS já é barata e sempre consistente.
    }

    async fn clear_folder_cache(&self) {}
}

fn system_time_ms(time: std::time::SystemTime) -> i64 {
    time.duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn set_mtime(path: &Path, mtime_ms: i64) -> AppResult<()> {
    let secs = mtime_ms.div_euclid(1000);
    let nanos = (mtime_ms.rem_euclid(1000) * 1_000_000) as u32;
    let time = filetime::FileTime::from_unix_time(secs, nanos);
    filetime::set_file_mtime(path, time)
        .map_err(|e| AppError::Other(format!("falha ao ajustar mtime de {}: {e}", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tag<'a>() -> DeviceTag<'a> {
        DeviceTag {
            name: Some("PC Gamer"),
            id: Some("dev-1"),
        }
    }

    #[tokio::test]
    async fn ensure_root_e_category_folder_criam_a_arvore() {
        let tmp = tempfile::tempdir().unwrap();
        let provider = FolderProvider::new(tmp.path().to_path_buf());

        let root = provider.ensure_root().await.unwrap();
        assert!(Path::new(&root).is_dir());

        let cat = provider
            .ensure_category_folder("PPSSPP", SyncCategory::Saves)
            .await
            .unwrap();
        assert!(Path::new(&cat).ends_with("PPSSPP/saves"));
        assert!(Path::new(&cat).is_dir());
    }

    #[tokio::test]
    async fn upload_new_download_e_list_tree_fazem_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let provider = FolderProvider::new(tmp.path().to_path_buf());
        let cat = provider
            .ensure_category_folder("PPSSPP", SyncCategory::Saves)
            .await
            .unwrap();

        let uploaded = provider
            .upload_new(
                &cat,
                "save.bin",
                b"dados".to_vec(),
                1_700_000_000_000,
                tag(),
            )
            .await
            .unwrap();
        assert_eq!(uploaded.rel_path, "save.bin");
        assert_eq!(uploaded.device_name.as_deref(), Some("PC Gamer"));

        let content = provider.download(&uploaded.id).await.unwrap();
        assert_eq!(content, b"dados");

        let tree = provider.list_tree(&cat).await.unwrap();
        assert_eq!(tree.len(), 1);
        assert_eq!(tree[0].rel_path, "save.bin");
        // O índice de dispositivo não aparece como um arquivo sincronizável.
        assert!(!tree.iter().any(|f| f.rel_path.contains("slot2sync-index")));
    }

    #[tokio::test]
    async fn find_child_encontra_e_nao_encontra() {
        let tmp = tempfile::tempdir().unwrap();
        let provider = FolderProvider::new(tmp.path().to_path_buf());
        let cat = provider
            .ensure_category_folder("PPSSPP", SyncCategory::Saves)
            .await
            .unwrap();
        provider
            .upload_new(&cat, "save.bin", b"x".to_vec(), 1_700_000_000_000, tag())
            .await
            .unwrap();

        assert!(provider
            .find_child(&cat, "save.bin")
            .await
            .unwrap()
            .is_some());
        assert!(provider
            .find_child(&cat, "nao-existe.bin")
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn rename_file_move_conteudo_e_indice() {
        let tmp = tempfile::tempdir().unwrap();
        let provider = FolderProvider::new(tmp.path().to_path_buf());
        let cat = provider
            .ensure_category_folder("PPSSPP", SyncCategory::Saves)
            .await
            .unwrap();
        let uploaded = provider
            .upload_new(
                &cat,
                "antigo.bin",
                b"dados".to_vec(),
                1_700_000_000_000,
                tag(),
            )
            .await
            .unwrap();

        let renamed = provider
            .rename_file(&uploaded.id, "novo.bin", Some(&cat), None)
            .await
            .unwrap();

        assert_eq!(renamed.rel_path, "novo.bin");
        assert_eq!(renamed.device_name.as_deref(), Some("PC Gamer"));
        assert!(provider
            .find_child(&cat, "antigo.bin")
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn upload_existing_sobrescreve_preservando_mtime() {
        let tmp = tempfile::tempdir().unwrap();
        let provider = FolderProvider::new(tmp.path().to_path_buf());
        let cat = provider
            .ensure_category_folder("PPSSPP", SyncCategory::Saves)
            .await
            .unwrap();
        let uploaded = provider
            .upload_new(&cat, "save.bin", b"v1".to_vec(), 1_700_000_000_000, tag())
            .await
            .unwrap();

        let updated = provider
            .upload_existing(&uploaded.id, b"v2".to_vec(), 1_700_000_100_000, tag())
            .await
            .unwrap();

        assert_eq!(provider.download(&updated.id).await.unwrap(), b"v2");
        assert_eq!(updated.modified_ms, Some(1_700_000_100_000));
    }
}
