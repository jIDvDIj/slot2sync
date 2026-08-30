//! `MockDrive` — implementação em memória do [`RemoteProvider`] para testes.
//!
//! Modela o Drive como pastas e arquivos num mapa, sem rede nem credenciais.
//! Além do contrato do trait, expõe helpers de fixture (`seed_category_file`,
//! `file_by_path`) e contadores/injeção de falha para asserções de fluxo
//! (`batch_calls`, `set_fail_next_batch`).

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;

use async_trait::async_trait;

use super::DriveFile;
use crate::constants::{DRIVE_APP_PROP_DEVICE, DRIVE_APP_PROP_DEVICE_ID, DRIVE_ROOT_FOLDER};
use crate::error::{AppError, AppResult};
use crate::remote::{BatchUploadOp, DeviceTag, RemoteFile, RemoteProvider};
use crate::sync::SyncCategory;

#[derive(Debug, Clone)]
struct Folder {
    name: String,
    parent: Option<String>,
}

#[derive(Debug, Clone)]
pub struct MockFile {
    pub name: String,
    pub parent: String,
    pub content: Vec<u8>,
    pub mtime_ms: i64,
    pub app_properties: HashMap<String, String>,
}

#[derive(Default)]
struct State {
    /// id → pasta. IDs determinísticos: `folder:<cache_key>`.
    folders: HashMap<String, Folder>,
    /// id → arquivo. IDs sequenciais: `file-N`.
    files: HashMap<String, MockFile>,
    next_file: u64,
}

#[derive(Default)]
pub struct MockDrive {
    state: Mutex<State>,
    /// Quantas vezes `upload_batch` foi chamado.
    pub batch_calls: AtomicU32,
    /// Quantos uploads per-file (`upload_new`) foram feitos.
    pub upload_new_calls: AtomicU32,
    fail_next_batch: AtomicBool,
    fail_downloads: AtomicBool,
    /// Gancho disparado no início de cada `download`, para o teste injetar um
    /// efeito colateral no meio do plano (ex.: cancelar o desligamento).
    #[allow(clippy::type_complexity)]
    on_download: Mutex<Option<Box<dyn Fn() + Send>>>,
}

impl MockDrive {
    pub fn new() -> Self {
        Self::default()
    }

    /// Faz o PRÓXIMO `upload_batch` falhar (uma vez) — exercita o fallback
    /// per-file do engine.
    pub fn set_fail_next_batch(&self) {
        self.fail_next_batch.store(true, Ordering::SeqCst);
    }

    /// Liga/desliga falha em TODOS os downloads — exercita a fila offline
    /// (`pending_ops`) do engine.
    pub fn set_fail_downloads(&self, fail: bool) {
        self.fail_downloads.store(fail, Ordering::SeqCst);
    }

    /// Registra um gancho executado no início de cada `download`. Permite ao
    /// teste agir enquanto o plano ainda está em execução.
    pub fn set_on_download(&self, hook: impl Fn() + Send + 'static) {
        *self.on_download.lock().unwrap() = Some(Box::new(hook));
    }

    fn folder_id(cache_key: &str) -> String {
        format!("folder:{cache_key}")
    }

    /// Garante a cadeia de pastas de `cache_key` (`"Slot2Sync/PPSSPP/saves"`),
    /// devolvendo o ID da última.
    fn ensure_chain(&self, cache_key: &str) -> String {
        let mut state = self.state.lock().unwrap();
        let mut key = String::new();
        let mut parent: Option<String> = None;
        for segment in cache_key.split('/').filter(|s| !s.is_empty()) {
            if !key.is_empty() {
                key.push('/');
            }
            key.push_str(segment);
            let id = Self::folder_id(&key);
            state.folders.entry(id.clone()).or_insert_with(|| Folder {
                name: segment.to_string(),
                parent: parent.clone(),
            });
            parent = Some(id);
        }
        parent.expect("cache_key vazio")
    }

    fn insert_file(
        &self,
        parent_id: &str,
        name: &str,
        content: Vec<u8>,
        mtime_ms: i64,
        device: DeviceTag<'_>,
    ) -> (String, DriveFile) {
        let mut props = HashMap::new();
        if let Some(n) = device.name {
            props.insert(DRIVE_APP_PROP_DEVICE.to_string(), n.to_string());
        }
        if let Some(id) = device.id {
            props.insert(DRIVE_APP_PROP_DEVICE_ID.to_string(), id.to_string());
        }
        let mut state = self.state.lock().unwrap();
        state.next_file += 1;
        let id = format!("file-{}", state.next_file);
        let file = MockFile {
            name: name.to_string(),
            parent: parent_id.to_string(),
            content,
            mtime_ms,
            app_properties: props,
        };
        state.files.insert(id.clone(), file.clone());
        (id.clone(), to_drive_file(&id, &file))
    }

    /// Fixture: planta um arquivo remoto sob a categoria de um emulador,
    /// criando as subpastas de `rel_path` conforme necessário.
    pub fn seed_category_file(
        &self,
        emulator: &str,
        category: SyncCategory,
        rel_path: &str,
        content: &[u8],
        mtime_ms: i64,
        device_id: Option<&str>,
    ) -> String {
        let base_key = format!("{DRIVE_ROOT_FOLDER}/{emulator}/{}", category.as_str());
        let (dir, name) = match rel_path.rsplit_once('/') {
            Some((dir, name)) => (Some(dir), name),
            None => (None, rel_path),
        };
        let parent = match dir {
            Some(dir) => self.ensure_chain(&format!("{base_key}/{dir}")),
            None => self.ensure_chain(&base_key),
        };
        let tag = DeviceTag {
            name: None,
            id: device_id,
        };
        self.insert_file(&parent, name, content.to_vec(), mtime_ms, tag)
            .0
    }

    /// Fixture: sobrescreve um arquivo existente como se OUTRO dispositivo
    /// tivesse publicado uma versão nova.
    pub fn overwrite_as_device(
        &self,
        emulator: &str,
        category: SyncCategory,
        rel_path: &str,
        content: &[u8],
        mtime_ms: i64,
        device_id: &str,
    ) {
        let (parent, name) = self.locate(emulator, category, rel_path);
        let mut state = self.state.lock().unwrap();
        let file = state
            .files
            .values_mut()
            .find(|f| f.parent == parent && f.name == name)
            .expect("arquivo inexistente no mock");
        file.content = content.to_vec();
        file.mtime_ms = mtime_ms;
        file.app_properties
            .insert(DRIVE_APP_PROP_DEVICE_ID.to_string(), device_id.to_string());
    }

    /// Arquivo sob a categoria de um emulador, por caminho relativo.
    pub fn file_by_path(
        &self,
        emulator: &str,
        category: SyncCategory,
        rel_path: &str,
    ) -> Option<MockFile> {
        let (parent, name) = self.locate(emulator, category, rel_path);
        let state = self.state.lock().unwrap();
        state
            .files
            .values()
            .find(|f| f.parent == parent && f.name == name)
            .cloned()
    }

    /// `(id da pasta-mãe, nome do arquivo)` de um caminho relativo à categoria.
    fn locate(&self, emulator: &str, category: SyncCategory, rel_path: &str) -> (String, String) {
        let key = format!(
            "{DRIVE_ROOT_FOLDER}/{emulator}/{}/{rel_path}",
            category.as_str()
        );
        let (dir, name) = key.rsplit_once('/').expect("caminho sem nome");
        (Self::folder_id(dir), name.to_string())
    }

    /// Arquivo direto na raiz `Slot2Sync/` (ex.: `sync_manifest.json`).
    pub fn root_file(&self, name: &str) -> Option<MockFile> {
        let parent = Self::folder_id(DRIVE_ROOT_FOLDER);
        let state = self.state.lock().unwrap();
        state
            .files
            .values()
            .find(|f| f.parent == parent && f.name == name)
            .cloned()
    }
}

fn to_drive_file(id: &str, file: &MockFile) -> DriveFile {
    DriveFile {
        id: id.to_string(),
        name: file.name.clone(),
        mime_type: super::OCTET_STREAM.to_string(),
        modified_time: chrono::DateTime::from_timestamp_millis(file.mtime_ms),
        size: Some(file.content.len().to_string()),
        // Mesmo contrato da API real: o Drive calcula e devolve o MD5.
        md5_checksum: Some(crate::sync::md5_hex(&file.content)),
        app_properties: file.app_properties.clone(),
    }
}

#[async_trait]
impl RemoteProvider for MockDrive {
    async fn ensure_root(&self) -> AppResult<String> {
        Ok(self.ensure_chain(DRIVE_ROOT_FOLDER))
    }

    async fn ensure_category_folder(
        &self,
        emulator: &str,
        category: SyncCategory,
    ) -> AppResult<String> {
        Ok(self.ensure_chain(&format!(
            "{DRIVE_ROOT_FOLDER}/{emulator}/{}",
            category.as_str()
        )))
    }

    async fn ensure_subpath(
        &self,
        _base_id: &str,
        base_key: &str,
        rel_dir: &str,
    ) -> AppResult<String> {
        Ok(self.ensure_chain(&format!("{base_key}/{rel_dir}")))
    }

    async fn list_tree(&self, folder_id: &str) -> AppResult<Vec<RemoteFile>> {
        let state = self.state.lock().unwrap();
        // Caminho de cada pasta até `folder_id` (None = fora da subárvore).
        fn rel_prefix(
            folders: &HashMap<String, Folder>,
            target: &str,
            mut id: String,
        ) -> Option<String> {
            let mut parts: Vec<String> = Vec::new();
            loop {
                if id == target {
                    parts.reverse();
                    let mut prefix = parts.join("/");
                    if !prefix.is_empty() {
                        prefix.push('/');
                    }
                    return Some(prefix);
                }
                let folder = folders.get(&id)?;
                parts.push(folder.name.clone());
                id = folder.parent.clone()?;
            }
        }

        let mut out = Vec::new();
        for (id, file) in &state.files {
            if let Some(prefix) = rel_prefix(&state.folders, folder_id, file.parent.clone()) {
                let rel_path = format!("{prefix}{}", file.name);
                out.push(to_drive_file(id, file).to_remote(rel_path));
            }
        }
        Ok(out)
    }

    async fn find_child(&self, folder_id: &str, name: &str) -> AppResult<Option<RemoteFile>> {
        let state = self.state.lock().unwrap();
        Ok(state
            .files
            .iter()
            .find(|(_, f)| f.parent == folder_id && f.name == name)
            .map(|(id, f)| to_drive_file(id, f).to_remote(name.to_string())))
    }

    async fn download(&self, file_id: &str) -> AppResult<Vec<u8>> {
        if let Some(hook) = self.on_download.lock().unwrap().as_ref() {
            hook();
        }
        if self.fail_downloads.load(Ordering::SeqCst) {
            // FileBusy é um erro retryable para o engine (Network exigiria
            // construir um reqwest::Error, que não tem construtor público).
            return Err(AppError::FileBusy(format!("mock: download de {file_id}")));
        }
        let state = self.state.lock().unwrap();
        state
            .files
            .get(file_id)
            .map(|f| f.content.clone())
            .ok_or_else(|| AppError::RemoteObjectNotFound(format!("mock: {file_id}")))
    }

    async fn upload_new(
        &self,
        parent_id: &str,
        name: &str,
        content: Vec<u8>,
        mtime_ms: i64,
        device: DeviceTag<'_>,
    ) -> AppResult<RemoteFile> {
        self.upload_new_calls.fetch_add(1, Ordering::SeqCst);
        let (_id, file) = self.insert_file(parent_id, name, content, mtime_ms, device);
        Ok(file.to_remote(name.to_string()))
    }

    async fn upload_existing(
        &self,
        file_id: &str,
        content: Vec<u8>,
        mtime_ms: i64,
        device: DeviceTag<'_>,
    ) -> AppResult<RemoteFile> {
        let mut state = self.state.lock().unwrap();
        let file = state
            .files
            .get_mut(file_id)
            .ok_or_else(|| AppError::RemoteObjectNotFound(format!("mock: {file_id}")))?;
        file.content = content;
        file.mtime_ms = mtime_ms;
        if let Some(n) = device.name {
            file.app_properties
                .insert(DRIVE_APP_PROP_DEVICE.to_string(), n.to_string());
        }
        if let Some(id) = device.id {
            file.app_properties
                .insert(DRIVE_APP_PROP_DEVICE_ID.to_string(), id.to_string());
        }
        let file = file.clone();
        Ok(to_drive_file(file_id, &file).to_remote(String::new()))
    }

    async fn upload_batch(&self, ops: Vec<BatchUploadOp>) -> AppResult<Vec<RemoteFile>> {
        self.batch_calls.fetch_add(1, Ordering::SeqCst);
        if self.fail_next_batch.swap(false, Ordering::SeqCst) {
            return Err(AppError::Other("mock: batch falhou de propósito".into()));
        }
        let mut out = Vec::with_capacity(ops.len());
        for op in ops {
            let tag = DeviceTag {
                name: op.device_name.as_deref(),
                id: op.device_id.as_deref(),
            };
            let name = op.name.clone();
            let (_id, file) =
                self.insert_file(&op.parent_id, &op.name, op.content, op.mtime_ms, tag);
            out.push(file.to_remote(name));
        }
        Ok(out)
    }

    async fn rename_file(
        &self,
        file_id: &str,
        new_name: &str,
        add_parent: Option<&str>,
        remove_parent: Option<&str>,
    ) -> AppResult<RemoteFile> {
        let mut state = self.state.lock().unwrap();
        let file = state
            .files
            .get_mut(file_id)
            .ok_or_else(|| AppError::RemoteObjectNotFound(format!("mock: {file_id}")))?;
        file.name = new_name.to_string();
        if let Some(parent) = add_parent {
            file.parent = parent.to_string();
        }
        let _ = remove_parent;
        let file = file.clone();
        Ok(to_drive_file(file_id, &file).to_remote(new_name.to_string()))
    }

    async fn invalidate_folder_path(&self, _cache_key: &str) {}

    async fn clear_folder_cache(&self) {}
}
