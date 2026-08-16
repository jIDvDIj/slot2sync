//! Interface Rust↔plugin do armazenamento de saves no mobile.
//!
//! No mobile não há caminho de filesystem para os saves de **outro** app: o
//! acesso passa por uma concessão de pasta do usuário — Storage Access Framework
//! (Android) ou document picker + security-scoped bookmark (iOS) — exposta por
//! um plugin nativo. Este módulo define:
//!
//! - o **contrato** das chamadas ao plugin (structs de request/response serde);
//! - a ponte [`PluginBridge`] — o único ponto que toca o runtime nativo;
//! - [`MobileStorage`], que implementa [`LocalStorage`] traduzindo cada operação
//!   numa chamada de comando ao plugin.
//!
//! O lado nativo (Kotlin/Swift) deve implementar os comandos `listFiles`, `stat`,
//! `exists`, `read`, `write` e `copy`.
//!
//! ## Locadores no mobile
//!
//! Um [`FileLoc`] mobile carrega um [`DocRef`] (`{ tree, rel }`) serializado: a
//! "árvore" concedida (URI do SAF / bookmark) + o caminho relativo dentro dela.
//! As pastas-base que o engine constrói via `FileLoc::from_path` (a partir do
//! `root_path` do perfil) são tratadas como a árvore concedida — ou seja, no
//! mobile o `root_path` do emulador guarda a URI da pasta concedida.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use base64::Engine as _;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use super::diff::LocalFile;
use super::storage::{FileLoc, LocalStorage};
use crate::error::{AppError, AppResult};

const CMD_LIST: &str = "listFiles";
const CMD_STAT: &str = "stat";
const CMD_EXISTS: &str = "exists";
const CMD_READ: &str = "read";
const CMD_WRITE: &str = "write";
const CMD_COPY: &str = "copy";

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

/// Referência a um documento na pasta concedida: árvore (URI do SAF/bookmark) +
/// caminho relativo. É o conteúdo (serializado) de um [`FileLoc`] mobile.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct DocRef {
    tree: String,
    rel: String,
}

// --- Contrato dos comandos do plugin (espelhado no lado nativo) ---

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ListRequest {
    tree: String,
    /// Pasta-base (relativa à árvore) a varrer; `rel` das entradas é relativo a ela.
    base: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListEntry {
    rel: String,
    mtime_ms: i64,
    size: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListResponse {
    entries: Vec<ListEntry>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DocRequest {
    tree: String,
    rel: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct StatResponse {
    mtime_ms: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExistsResponse {
    exists: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReadResponse {
    data_base64: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WriteRequest {
    tree: String,
    rel: String,
    data_base64: String,
    mtime_ms: Option<i64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CopyRequest {
    src_tree: String,
    src_rel: String,
    dest_tree: String,
    dest_rel: String,
}

/// Ponte de baixo nível para o plugin nativo. A implementação concreta liga isto
/// ao `PluginHandle::run_mobile_plugin` (ver [`init`]).
#[async_trait]
pub trait PluginBridge: Send + Sync {
    async fn invoke(
        &self,
        command: &str,
        payload: serde_json::Value,
    ) -> AppResult<serde_json::Value>;
}

#[async_trait]
impl PluginBridge for Arc<dyn PluginBridge> {
    async fn invoke(
        &self,
        command: &str,
        payload: serde_json::Value,
    ) -> AppResult<serde_json::Value> {
        (**self).invoke(command, payload).await
    }
}

/// [`LocalStorage`] sobre o plugin nativo de armazenamento concedido.
pub struct MobileStorage<B: PluginBridge> {
    bridge: B,
}

impl<B: PluginBridge> MobileStorage<B> {
    pub fn new(bridge: B) -> Self {
        Self { bridge }
    }

    async fn call<Req: Serialize, Res: DeserializeOwned>(
        &self,
        command: &str,
        req: Req,
    ) -> AppResult<Res> {
        let payload = serde_json::to_value(req)?;
        let resp = self.bridge.invoke(command, payload).await?;
        serde_json::from_value(resp)
            .map_err(|e| AppError::Other(format!("resposta do plugin inválida ({command}): {e}")))
    }
}

/// Extrai o [`DocRef`] de um locador. Locadores de "caminho" (raízes/bases que o
/// engine constrói via `from_path`) viram a árvore concedida com `rel` vazio.
fn loc_to_docref(loc: &FileLoc) -> DocRef {
    match loc.as_doc() {
        Some(doc) => serde_json::from_str(doc).unwrap_or(DocRef {
            tree: doc.to_string(),
            rel: String::new(),
        }),
        None => DocRef {
            tree: loc.to_string(),
            rel: String::new(),
        },
    }
}

fn docref_to_loc(d: &DocRef) -> FileLoc {
    FileLoc::doc(serde_json::to_string(d).unwrap_or_default())
}

/// Constrói o [`FileLoc`] de um documento sob a árvore `tree`, no caminho
/// relativo `rel`. Usado pela detecção automática mobile
/// (`commands::detect_emulator_mobile`), que precisa checar existência de
/// pastas candidatas fora do fluxo normal de sync.
pub fn doc_loc(tree: &str, rel: &str) -> FileLoc {
    docref_to_loc(&DocRef {
        tree: tree.to_string(),
        rel: rel.to_string(),
    })
}

fn join_rel(base: &str, rel: &str) -> String {
    if base.is_empty() {
        rel.to_string()
    } else {
        format!("{}/{}", base.trim_end_matches('/'), rel)
    }
}

#[async_trait]
impl<B: PluginBridge> LocalStorage for MobileStorage<B> {
    async fn scan(&self, root: &Path, bases: &[PathBuf]) -> AppResult<Vec<LocalFile>> {
        // `tree` é a árvore concedida; cada `base` é varrida à parte. `rel_path`
        // (para o diff) é relativo à base; o locador guarda `base+rel` relativo à
        // árvore, para o plugin localizar o documento. Dedup: a primeira base
        // vence em `rel_path` duplicado — mesmo critério da `DesktopStorage`.
        let tree = root.to_string_lossy().into_owned();
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for base in bases {
            let base_str = base.to_string_lossy().replace('\\', "/");
            let resp: ListResponse = self
                .call(
                    CMD_LIST,
                    ListRequest {
                        tree: tree.clone(),
                        base: base_str.clone(),
                    },
                )
                .await?;
            for e in resp.entries {
                if !seen.insert(e.rel.clone()) {
                    continue;
                }
                let doc_rel = join_rel(&base_str, &e.rel);
                out.push(LocalFile {
                    loc: docref_to_loc(&DocRef {
                        tree: tree.clone(),
                        rel: doc_rel,
                    }),
                    rel_path: e.rel,
                    mtime_ms: e.mtime_ms,
                    // SAF (`DocumentsContract`) só expõe `lastModified` em ms.
                    mtime_ns: 0,
                    size_bytes: e.size,
                    hash: None,
                });
            }
        }
        Ok(out)
    }

    fn join(&self, base: &FileLoc, rel_path: &str) -> FileLoc {
        // Path nativo (ex.: diretório de backup) — preserva o tipo para que
        // operações subsequentes usem tokio::fs e não o plugin SAF.
        if let Some(path) = base.as_native_path() {
            return FileLoc::from_path(path.join(rel_path));
        }
        let mut d = loc_to_docref(base);
        d.rel = join_rel(&d.rel, rel_path);
        docref_to_loc(&d)
    }

    fn root_loc(&self, root: &Path) -> FileLoc {
        // `root` guarda a URI SAF (`content://...`) como string — nunca um
        // caminho de filesystem real. `doc_loc` com `rel` vazio aponta para a
        // própria raiz da árvore concedida.
        doc_loc(&root.to_string_lossy(), "")
    }

    fn loc_to_stored(&self, loc: &FileLoc) -> String {
        match loc.as_doc() {
            Some(doc) => doc.to_string(),
            None => serde_json::to_string(&loc_to_docref(loc)).unwrap_or_default(),
        }
    }

    fn loc_from_stored(&self, stored: &str) -> FileLoc {
        FileLoc::doc(stored.to_owned())
    }

    async fn exists(&self, loc: &FileLoc) -> bool {
        let d = loc_to_docref(loc);
        matches!(
            self.call::<_, ExistsResponse>(CMD_EXISTS, DocRequest { tree: d.tree, rel: d.rel })
                .await,
            Ok(r) if r.exists
        )
    }

    async fn mtime_ms(&self, loc: &FileLoc) -> AppResult<i64> {
        let d = loc_to_docref(loc);
        let r: StatResponse = self
            .call(
                CMD_STAT,
                DocRequest {
                    tree: d.tree,
                    rel: d.rel,
                },
            )
            .await?;
        Ok(r.mtime_ms)
    }

    async fn read(&self, loc: &FileLoc) -> AppResult<Vec<u8>> {
        let d = loc_to_docref(loc);
        let r: ReadResponse = self
            .call(
                CMD_READ,
                DocRequest {
                    tree: d.tree,
                    rel: d.rel,
                },
            )
            .await?;
        B64.decode(r.data_base64)
            .map_err(|e| AppError::Other(format!("base64 inválido do plugin: {e}")))
    }

    async fn write_atomic(
        &self,
        dest: &FileLoc,
        bytes: &[u8],
        mtime_ms: Option<i64>,
    ) -> AppResult<()> {
        // Destino em armazenamento privado do app (ex.: resolução de conflito) —
        // usa tokio::fs diretamente, sem passar pelo plugin SAF.
        if let Some(path) = dest.as_native_path() {
            if let Some(parent) = path.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            tokio::fs::write(path, bytes).await?;
            return Ok(());
        }
        let d = loc_to_docref(dest);
        let req = WriteRequest {
            tree: d.tree,
            rel: d.rel,
            data_base64: B64.encode(bytes),
            mtime_ms,
        };
        let _: serde_json::Value = self.call(CMD_WRITE, req).await?;
        Ok(())
    }

    async fn copy_to(&self, src: &FileLoc, dest: &FileLoc) -> AppResult<()> {
        // Destino em path nativo (ex.: backup em armazenamento privado do app) —
        // lê do SAF via plugin e grava no filesystem diretamente.
        if let Some(dest_path) = dest.as_native_path() {
            let bytes = self.read(src).await?;
            if let Some(parent) = dest_path.parent() {
                tokio::fs::create_dir_all(parent).await?;
            }
            tokio::fs::write(dest_path, bytes).await?;
            return Ok(());
        }
        let s = loc_to_docref(src);
        let t = loc_to_docref(dest);
        let req = CopyRequest {
            src_tree: s.tree,
            src_rel: s.rel,
            dest_tree: t.tree,
            dest_rel: t.rel,
        };
        let _: serde_json::Value = self.call(CMD_COPY, req).await?;
        Ok(())
    }

    async fn is_valid_root(&self, loc: &FileLoc) -> bool {
        // A raiz mobile é a árvore concedida (URI SAF): existe = concedida e
        // acessível. O plugin resolve `DocumentFile` a partir da URI.
        self.exists(loc).await
    }

    async fn subdir_exists(&self, root: &FileLoc, rel: &str) -> bool {
        // Constrói `{tree, rel}` explicitamente para não achatar árvore+subpasta
        // num único path (o que `join` faria para locadores de caminho nativo).
        let base = loc_to_docref(root);
        let doc = DocRef {
            tree: base.tree,
            rel: join_rel(&base.rel, rel),
        };
        self.exists(&docref_to_loc(&doc)).await
    }
}

// --- Ligação ao runtime nativo (PluginHandle) ---

use tauri::plugin::{Builder, PluginHandle, TauriPlugin};
use tauri::{Manager, Runtime};

/// Ponte concreta gerenciada no estado do app; o `lib.rs` a usa para montar a
/// [`MobileStorage`].
struct BridgeState(Arc<dyn PluginBridge>);

/// Ponte que chama o plugin nativo via `run_mobile_plugin`.
struct TauriBridge<R: Runtime> {
    handle: PluginHandle<R>,
}

#[async_trait]
impl<R: Runtime> PluginBridge for TauriBridge<R> {
    async fn invoke(
        &self,
        command: &str,
        payload: serde_json::Value,
    ) -> AppResult<serde_json::Value> {
        self.handle
            .run_mobile_plugin::<serde_json::Value>(command, payload)
            .map_err(|e| AppError::Other(format!("plugin de storage falhou ({command}): {e}")))
    }
}

/// Plugin Tauri que registra o lado nativo (Kotlin/Swift) e guarda a ponte no
/// estado do app. Registrar no `Builder` (`.plugin(mobile_storage::init())`).
pub fn init<R: Runtime>() -> TauriPlugin<R> {
    Builder::new("slot2sync-storage")
        .setup(|app, _api| {
            #[cfg(target_os = "android")]
            let handle = _api.register_android_plugin("com.slot2sync.app", "StoragePlugin")?;
            // iOS: ligar ao Swift package via `register_ios_plugin` — implementar
            // e validar no macOS/Xcode.
            #[cfg(target_os = "ios")]
            let handle: PluginHandle<R> =
                todo!("registro do plugin de storage no iOS (macOS/Xcode)");

            let bridge: Arc<dyn PluginBridge> = Arc::new(TauriBridge { handle });
            app.manage(BridgeState(bridge));
            Ok(())
        })
        .build()
}

/// Abre o seletor de pasta nativo (SAF no Android) e retorna a URI da árvore
/// concedida. A permissão é persistida pelo plugin para reinícios do app.
pub async fn pick_folder<R: Runtime>(app: &tauri::AppHandle<R>) -> AppResult<String> {
    let state = app
        .try_state::<BridgeState>()
        .ok_or_else(|| AppError::Other("plugin de storage não inicializado".into()))?;
    let result = state.0.invoke("pickFolder", serde_json::json!({})).await?;
    result
        .get("tree")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .ok_or_else(|| AppError::Other("pickFolder não retornou 'tree'".into()))
}

/// Monta a [`MobileStorage`] a partir da ponte registrada por [`init`].
pub fn storage<R: Runtime>(app: &tauri::AppHandle<R>) -> AppResult<Arc<dyn LocalStorage>> {
    let state = app
        .try_state::<BridgeState>()
        .ok_or_else(|| AppError::Other("plugin de storage não inicializado".into()))?;
    Ok(Arc::new(MobileStorage::new(state.0.clone())))
}
