//! `RemoteProvider` — a porta do `SyncEngine` para o storage remoto.
//!
//! O engine depende deste trait, nunca de um cliente concreto: em produção
//! uma das quatro implementações (`drive`, `dropbox`, `onedrive`, `folder`) o
//! satisfaz; nos testes, `MockRemote` opera sobre um mapa em memória,
//! permitindo exercitar o engine de ponta a ponta sem rede e sem credenciais.
//!
//! Generaliza o antigo `drive::DriveApi`: mesmas operações, mas com um
//! `RemoteFile` achatado (sem depender de um tipo concreto de um provedor
//! específico) e um `ProviderKind` para identificar qual provedor está ativo.

pub mod device_index;
pub mod http;

use async_trait::async_trait;

use crate::error::AppResult;
use crate::sync::SyncCategory;

/// Qual provedor de storage está configurado. Persistido em `Settings` e
/// usado para escolher a implementação de `RemoteProvider` no `setup()` (ou
/// ao conectar interativamente pela primeira vez).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    GoogleDrive,
    Dropbox,
    OneDrive,
    LocalFolder,
}

impl ProviderKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ProviderKind::GoogleDrive => "google_drive",
            ProviderKind::Dropbox => "dropbox",
            ProviderKind::OneDrive => "one_drive",
            ProviderKind::LocalFolder => "local_folder",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "google_drive" => Some(ProviderKind::GoogleDrive),
            "dropbox" => Some(ProviderKind::Dropbox),
            "one_drive" => Some(ProviderKind::OneDrive),
            "local_folder" => Some(ProviderKind::LocalFolder),
            _ => None,
        }
    }
}

/// Arquivo remoto com caminho relativo à pasta de categoria (separador `/`).
/// Achatado — cada provedor converte o shape da sua própria API para este
/// formato na borda (ver `drive::api`, `dropbox::provider`, etc.).
#[derive(Debug, Clone, Default)]
pub struct RemoteFile {
    /// Identificador opaco do provedor: ID (Drive/OneDrive) ou o próprio path
    /// (Dropbox/pasta local) — nunca interpretado fora da implementação dona.
    pub id: String,
    pub rel_path: String,
    pub modified_ms: Option<i64>,
    pub size_bytes: Option<i64>,
    /// Hash de integridade pós-download, no formato nativo do provedor
    /// (MD5 no Drive, `content_hash` no Dropbox, `quickXorHash` no OneDrive,
    /// SHA-256 na pasta local) — comparado só contra o mesmo provedor, nunca
    /// entre provedores diferentes.
    pub hash: Option<String>,
    /// Nome amigável do dispositivo que publicou esta versão (para exibição).
    pub device_name: Option<String>,
    /// ID estável do dispositivo que publicou esta versão (detecção de
    /// conflito entre dispositivos).
    pub device_id: Option<String>,
}

/// Identidade do dispositivo estampada em cada upload: `name` (amigável,
/// exibição) e `id` (UUID estável do keyring, detecção de conflito). Ambos
/// opcionais — degradam para ausência.
#[derive(Debug, Clone, Copy, Default)]
pub struct DeviceTag<'a> {
    pub name: Option<&'a str>,
    pub id: Option<&'a str>,
}

/// Um upload de arquivo **novo** agrupável em lote. Restrito a arquivos
/// pequenos — cada provedor decide seu próprio limite. Possui os dados (sem
/// lifetimes) para acumular numa `Vec` entre awaits no engine.
#[derive(Debug, Clone)]
pub struct BatchUploadOp {
    pub parent_id: String,
    pub name: String,
    pub content: Vec<u8>,
    pub mtime_ms: i64,
    pub device_name: Option<String>,
    pub device_id: Option<String>,
}

/// Operações de storage remoto das quais o `SyncEngine` depende. Espelha 1:1
/// os métodos consumidos pelo engine — novas necessidades entram aqui
/// primeiro, mantendo todas as implementações (e o mock) em sincronia.
#[async_trait]
pub trait RemoteProvider: Send + Sync {
    /// Garante a pasta raiz do app no provedor e retorna seu ID/path.
    async fn ensure_root(&self) -> AppResult<String>;

    /// Garante `<raiz>/<emulator>/<categoria>` e retorna o ID/path da categoria.
    async fn ensure_category_folder(
        &self,
        emulator: &str,
        category: SyncCategory,
    ) -> AppResult<String>;

    /// Garante a cadeia de subpastas `rel_dir` (separador `/`) sob `base_id`.
    async fn ensure_subpath(
        &self,
        base_id: &str,
        base_key: &str,
        rel_dir: &str,
    ) -> AppResult<String>;

    /// Lista recursivamente os arquivos sob `folder_id`, com caminhos relativos.
    async fn list_tree(&self, folder_id: &str) -> AppResult<Vec<RemoteFile>>;

    /// Filho direto por nome (sem recursão).
    async fn find_child(&self, folder_id: &str, name: &str) -> AppResult<Option<RemoteFile>>;

    /// Baixa o conteúdo inteiro de um arquivo.
    async fn download(&self, file_id: &str) -> AppResult<Vec<u8>>;

    /// Cria um arquivo novo preservando o mtime e marcando o dispositivo.
    async fn upload_new(
        &self,
        parent_id: &str,
        name: &str,
        content: Vec<u8>,
        mtime_ms: i64,
        device: DeviceTag<'_>,
    ) -> AppResult<RemoteFile>;

    /// Atualiza o conteúdo de um arquivo existente preservando o mtime.
    async fn upload_existing(
        &self,
        file_id: &str,
        content: Vec<u8>,
        mtime_ms: i64,
        device: DeviceTag<'_>,
    ) -> AppResult<RemoteFile>;

    /// Envia arquivos novos e pequenos agrupados quando o provedor suportar;
    /// caso contrário, cada implementação pode simplesmente iterar
    /// `upload_new`. Retorna os `RemoteFile` na MESMA ordem das operações.
    async fn upload_batch(&self, ops: Vec<BatchUploadOp>) -> AppResult<Vec<RemoteFile>>;

    /// Renomeia (e opcionalmente move) um arquivo sem reenviar conteúdo.
    /// Usado pela detecção de renomeação por hash.
    async fn rename_file(
        &self,
        file_id: &str,
        new_name: &str,
        add_parent: Option<&str>,
        remove_parent: Option<&str>,
    ) -> AppResult<RemoteFile>;

    /// Invalida um caminho lógico de pasta e sua subárvore no cache (se houver).
    async fn invalidate_folder_path(&self, cache_key: &str);

    /// Zera todo o cache de pastas (logout/troca de conta), se houver.
    async fn clear_folder_cache(&self);
}
