//! Abstração de acesso ao armazenamento local de saves.
//!
//! O `SyncEngine` nunca toca em `std::fs`/`tokio::fs`/`filetime` diretamente:
//! todo o I/O local passa por [`LocalStorage`]. No desktop, [`DesktopStorage`]
//! usa o filesystem nativo. No mobile (futuro), uma implementação sobre o SAF
//! (Android) / security-scoped bookmarks (iOS) plugará o **mesmo** trait — por
//! isso os arquivos são endereçados por [`FileLoc`] opaco, e não por `PathBuf`.

use std::path::{Path, PathBuf};

use async_trait::async_trait;

use super::diff::LocalFile;
use crate::error::AppResult;

#[cfg(desktop)]
use super::diff;
#[cfg(desktop)]
use crate::constants::TMP_SUFFIX;
#[cfg(desktop)]
use crate::error::AppError;

/// Locador opaco de um arquivo ou pasta no armazenamento local.
///
/// Desktop: um caminho nativo. Mobile (futuro): URI do SAF ou um
/// security-scoped bookmark. O conteúdo é privado de propósito — o engine só
/// obtém um `FileLoc` via [`LocalStorage`] (do `scan`/`join`) ou de uma string
/// persistida ([`LocalStorage::loc_from_stored`]), nunca manipulando o caminho.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileLoc(Loc);

#[derive(Debug, Clone, PartialEq, Eq)]
enum Loc {
    /// Caminho no filesystem nativo (desktop).
    Path(PathBuf),
    /// Documento no armazenamento concedido do mobile — encoding opaco
    /// interpretado por [`super::mobile_storage`] e pelo plugin nativo (SAF /
    /// bookmark).
    #[cfg(mobile)]
    Doc(String),
}

impl FileLoc {
    /// Locador de um caminho nativo (desktop).
    pub fn from_path(path: PathBuf) -> Self {
        Self(Loc::Path(path))
    }

    /// Locador de um documento no armazenamento mobile (SAF/bookmark).
    #[cfg(mobile)]
    pub fn doc(handle: impl Into<String>) -> Self {
        Self(Loc::Doc(handle.into()))
    }

    /// Caminho nativo subjacente, se for um locador de filesystem.
    /// No desktop todos os locadores são paths; no mobile apenas os de
    /// armazenamento privado do app (ex.: diretório de backup).
    pub(crate) fn as_native_path(&self) -> Option<&Path> {
        match &self.0 {
            Loc::Path(p) => Some(p),
            #[cfg(mobile)]
            Loc::Doc(_) => None,
        }
    }

    #[cfg(desktop)]
    pub(crate) fn as_path(&self) -> Option<&Path> {
        self.as_native_path()
    }

    /// Handle do documento mobile, se for um locador mobile.
    #[cfg(mobile)]
    pub(crate) fn as_doc(&self) -> Option<&str> {
        match &self.0 {
            Loc::Doc(s) => Some(s),
            Loc::Path(_) => None,
        }
    }
}

impl std::fmt::Display for FileLoc {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.0 {
            Loc::Path(p) => write!(f, "{}", p.display()),
            #[cfg(mobile)]
            Loc::Doc(s) => write!(f, "{s}"),
        }
    }
}

/// Acesso ao armazenamento local de saves, abstraído do filesystem nativo.
///
/// Métodos puros (`join`/`loc_*`) constroem locadores; os assíncronos fazem o
/// I/O. Implementações devem ser não-destrutivas — só leem, gravam e copiam.
#[async_trait]
pub trait LocalStorage: Send + Sync {
    /// Varre as pastas-base de uma categoria (relativas a `root`), devolvendo os
    /// arquivos com locador + mtime + tamanho. Pastas inexistentes são puladas.
    async fn scan(&self, root: &Path, bases: &[PathBuf]) -> AppResult<Vec<LocalFile>>;

    /// Resolve `base + rel_path` (`"jogo/save.bin"`) num locador de arquivo.
    fn join(&self, base: &FileLoc, rel_path: &str) -> FileLoc;

    /// Locador da raiz de um emulador (`EmulatorProfile::root_path`). Ponto de
    /// partida para [`Self::join`] — no desktop é um caminho de filesystem; no
    /// mobile é a árvore SAF concedida (com `rel` vazio). Nunca usar
    /// `FileLoc::from_path` diretamente sobre `root_path` fora daqui: no
    /// mobile ele guarda uma URI (`content://...`), não um caminho real, e
    /// tratá-lo como nativo grava fora da árvore concedida (falha com "Read-only
    /// file system").
    fn root_loc(&self, root: &Path) -> FileLoc;

    /// Serializa um locador para persistência (coluna `local_abs_path` do
    /// conflito no SQLite, que também cruza a boundary IPC).
    fn loc_to_stored(&self, loc: &FileLoc) -> String;

    /// Reconstrói um locador a partir da forma persistida por [`Self::loc_to_stored`].
    fn loc_from_stored(&self, stored: &str) -> FileLoc;

    /// O arquivo existe? Usado antes de fazer backup ao resolver conflito.
    async fn exists(&self, loc: &FileLoc) -> bool;

    /// mtime do arquivo em ms desde a época. Erro se o arquivo não existir.
    async fn mtime_ms(&self, loc: &FileLoc) -> AppResult<i64>;

    /// Lê o conteúdo inteiro do arquivo.
    async fn read(&self, loc: &FileLoc) -> AppResult<Vec<u8>>;

    /// Gravação atômica (grava num temporário e renomeia). Cria as pastas-pai do
    /// destino e, se `mtime_ms` for `Some`, ajusta o mtime do arquivo final
    /// (para o diff convergir com o `modifiedTime` do Drive).
    async fn write_atomic(
        &self,
        dest: &FileLoc,
        bytes: &[u8],
        mtime_ms: Option<i64>,
    ) -> AppResult<()>;

    /// Copia `src` para `dest`, criando as pastas-pai do destino.
    async fn copy_to(&self, src: &FileLoc, dest: &FileLoc) -> AppResult<()>;

    /// O locador aponta para um diretório válido e acessível? Usado para validar
    /// a raiz de um emulador antes de registrá-lo, sem que o comando manipule
    /// caminhos diretamente. No desktop é `Path::is_dir`; no mobile a raiz é uma
    /// URI SAF (não um caminho de filesystem), então a checagem passa pelo plugin
    /// nativo.
    async fn is_valid_root(&self, loc: &FileLoc) -> bool;

    /// Existe a subpasta `rel` (separador `/`) sob `root`? Valida os caminhos de
    /// saves/savestates/config informados manualmente sem vazar `std::fs`/URIs
    /// SAF para fora desta abstração.
    async fn subdir_exists(&self, root: &FileLoc, rel: &str) -> bool;

    /// Bytes livres no volume que contém `loc`. `None` quando a plataforma não
    /// consegue medir (locador não-filesystem, volume não identificado) — nesse
    /// caso a checagem de espaço antes do download é simplesmente pulada.
    async fn available_space(&self, _loc: &FileLoc) -> Option<u64> {
        None
    }
}

/// `true` se `path` existe e é um diretório (não propaga erro — ausência,
/// permissão negada ou "não é pasta" viram `false`).
#[cfg(desktop)]
async fn path_is_dir(path: &Path) -> bool {
    tokio::fs::metadata(path)
        .await
        .map(|m| m.is_dir())
        .unwrap_or(false)
}

/// Implementação desktop: filesystem nativo via `tokio::fs`/`filetime`.
#[cfg(desktop)]
pub struct DesktopStorage;

/// Caminho nativo de um locador, ou erro se vier um locador não-filesystem
/// (não deve acontecer no desktop, onde só existem locadores de caminho).
#[cfg(desktop)]
fn require_path(loc: &FileLoc) -> AppResult<&Path> {
    loc.as_path()
        .ok_or_else(|| AppError::Other("DesktopStorage requer um locador de filesystem".into()))
}

#[cfg(desktop)]
#[async_trait]
impl LocalStorage for DesktopStorage {
    async fn scan(&self, root: &Path, bases: &[PathBuf]) -> AppResult<Vec<LocalFile>> {
        // Scan é I/O de disco síncrono e potencialmente pesado: fora do executor.
        let (root, bases) = (root.to_path_buf(), bases.to_vec());
        tokio::task::spawn_blocking(move || diff::scan_local_bases(&root, &bases))
            .await
            .map_err(|e| AppError::Other(format!("tarefa bloqueante abortada: {e}")))?
    }

    fn join(&self, base: &FileLoc, rel_path: &str) -> FileLoc {
        let mut path = base
            .as_path()
            .expect("DesktopStorage usa locador de filesystem")
            .to_path_buf();
        // `rel_path` usa sempre `/`; reconstrói com o separador nativo.
        for part in rel_path.split('/') {
            path.push(part);
        }
        FileLoc::from_path(path)
    }

    fn root_loc(&self, root: &Path) -> FileLoc {
        FileLoc::from_path(root.to_path_buf())
    }

    fn loc_to_stored(&self, loc: &FileLoc) -> String {
        loc.to_string()
    }

    fn loc_from_stored(&self, stored: &str) -> FileLoc {
        FileLoc::from_path(PathBuf::from(stored))
    }

    async fn exists(&self, loc: &FileLoc) -> bool {
        match loc.as_path() {
            Some(p) => tokio::fs::try_exists(p).await.unwrap_or(false),
            None => false,
        }
    }

    async fn mtime_ms(&self, loc: &FileLoc) -> AppResult<i64> {
        let metadata = tokio::fs::metadata(require_path(loc)?).await?;
        Ok(diff::system_time_ms(metadata.modified()?))
    }

    async fn read(&self, loc: &FileLoc) -> AppResult<Vec<u8>> {
        Ok(tokio::fs::read(require_path(loc)?).await?)
    }

    async fn write_atomic(
        &self,
        dest: &FileLoc,
        bytes: &[u8],
        mtime_ms: Option<i64>,
    ) -> AppResult<()> {
        let dest = require_path(dest)?;
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        // Gravação atômica: temp + rename evita save corrompido se cair no meio.
        let tmp = dest.with_file_name(format!(
            "{}{TMP_SUFFIX}",
            dest.file_name().unwrap_or_default().to_string_lossy()
        ));
        tokio::fs::write(&tmp, bytes).await?;
        tokio::fs::rename(&tmp, dest).await?;

        if let Some(ms) = mtime_ms {
            let ft =
                filetime::FileTime::from_unix_time(ms / 1000, ((ms % 1000) * 1_000_000) as u32);
            filetime::set_file_mtime(dest, ft)?;
        }
        Ok(())
    }

    async fn copy_to(&self, src: &FileLoc, dest: &FileLoc) -> AppResult<()> {
        let (src, dest) = (require_path(src)?, require_path(dest)?);
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::copy(src, dest).await?;
        Ok(())
    }

    async fn is_valid_root(&self, loc: &FileLoc) -> bool {
        match loc.as_path() {
            Some(p) => path_is_dir(p).await,
            None => false,
        }
    }

    async fn subdir_exists(&self, root: &FileLoc, rel: &str) -> bool {
        let Some(base) = root.as_path() else {
            return false;
        };
        let mut path = base.to_path_buf();
        for part in rel.split('/').filter(|s| !s.is_empty()) {
            path.push(part);
        }
        path_is_dir(&path).await
    }

    async fn available_space(&self, loc: &FileLoc) -> Option<u64> {
        let path = loc.as_path()?.to_path_buf();
        tokio::task::spawn_blocking(move || available_space_for(&path))
            .await
            .ok()
            .flatten()
    }
}

/// Bytes livres do disco cujo ponto de montagem é o prefixo mais longo de
/// `path`. O caminho não precisa existir ainda (destino de download novo).
#[cfg(desktop)]
fn available_space_for(path: &Path) -> Option<u64> {
    let disks = sysinfo::Disks::new_with_refreshed_list();
    disks
        .iter()
        .filter(|d| path.starts_with(d.mount_point()))
        .max_by_key(|d| d.mount_point().as_os_str().len())
        .map(|d| d.available_space())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_usa_separador_nativo_e_preserva_subpastas() {
        let s = DesktopStorage;
        let base = FileLoc::from_path(PathBuf::from("/tmp/raiz"));
        let joined = s.join(&base, "jogo/save.bin");
        assert_eq!(
            joined,
            FileLoc::from_path(PathBuf::from("/tmp/raiz").join("jogo").join("save.bin"))
        );
    }

    #[test]
    fn loc_roundtrip_preserva_o_caminho() {
        let s = DesktopStorage;
        let loc = FileLoc::from_path(PathBuf::from("/tmp/raiz/save.bin"));
        let stored = s.loc_to_stored(&loc);
        assert_eq!(s.loc_from_stored(&stored), loc);
    }

    #[tokio::test]
    async fn write_atomic_grava_cria_pastas_e_ajusta_mtime() {
        let tmp = tempfile::tempdir().unwrap();
        let s = DesktopStorage;
        let dest = FileLoc::from_path(tmp.path().join("sub/dir/save.bin"));

        s.write_atomic(&dest, b"conteudo", Some(1_700_000_000_000))
            .await
            .unwrap();

        assert!(s.exists(&dest).await);
        assert_eq!(s.read(&dest).await.unwrap(), b"conteudo");
        assert_eq!(s.mtime_ms(&dest).await.unwrap(), 1_700_000_000_000);
        // Não deixa o temporário para trás.
        assert!(!tmp.path().join("sub/dir/save.bin.slot2sync-tmp").exists());
    }

    #[tokio::test]
    async fn copy_to_copia_criando_pastas_do_destino() {
        let tmp = tempfile::tempdir().unwrap();
        let s = DesktopStorage;
        let src = FileLoc::from_path(tmp.path().join("origem.bin"));
        s.write_atomic(&src, b"abc", None).await.unwrap();

        let dest = FileLoc::from_path(tmp.path().join("backup/origem.bin"));
        s.copy_to(&src, &dest).await.unwrap();

        assert_eq!(s.read(&dest).await.unwrap(), b"abc");
    }

    #[tokio::test]
    async fn mtime_de_arquivo_inexistente_e_erro() {
        let tmp = tempfile::tempdir().unwrap();
        let s = DesktopStorage;
        let loc = FileLoc::from_path(tmp.path().join("nao_existe.bin"));
        assert!(s.mtime_ms(&loc).await.is_err());
        assert!(!s.exists(&loc).await);
    }

    #[tokio::test]
    async fn is_valid_root_distingue_pasta_de_arquivo_e_ausencia() {
        let tmp = tempfile::tempdir().unwrap();
        let s = DesktopStorage;

        // Pasta existente → válida.
        assert!(
            s.is_valid_root(&FileLoc::from_path(tmp.path().to_path_buf()))
                .await
        );
        // Arquivo (não é pasta) → inválido.
        let file = FileLoc::from_path(tmp.path().join("arquivo.bin"));
        s.write_atomic(&file, b"x", None).await.unwrap();
        assert!(!s.is_valid_root(&file).await);
        // Inexistente → inválido.
        assert!(
            !s.is_valid_root(&FileLoc::from_path(tmp.path().join("nao_existe")))
                .await
        );
    }

    #[tokio::test]
    async fn subdir_exists_confere_subpasta_sob_a_raiz() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("PSP/SAVEDATA")).unwrap();
        let s = DesktopStorage;
        let root = FileLoc::from_path(tmp.path().to_path_buf());

        assert!(s.subdir_exists(&root, "PSP/SAVEDATA").await);
        assert!(s.subdir_exists(&root, "PSP").await);
        assert!(!s.subdir_exists(&root, "PSP/NAOEXISTE").await);
    }
}
