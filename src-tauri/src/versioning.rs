//! Histórico de versões dos saves: antes de qualquer download sobrescrever um
//! arquivo local, a versão vigente é arquivada em
//! `<backup_dir>/<emulador>/history/<categoria>/<rel_dir>/<nome>~<carimbo><ext>`.
//! Cada arquivo mantém no máximo N versões (as mais antigas são apagadas).
//!
//! O diretório `history/` participa da árvore que `backups::list` varre, então
//! as versões também aparecem no histórico de backups da UI.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::AppResult;

/// Nome da "execução" fixa que agrupa o histórico de versões na árvore de
/// backups (`<emulador>/history/<categoria>/...`).
pub const HISTORY_DIR_NAME: &str = "history";

/// Separador entre o nome original e o carimbo de versão no nome arquivado.
const VERSION_SEP: char = '~';

/// Formato do carimbo de versão (`20250717-103000`).
const STAMP_FORMAT: &str = "%Y%m%d-%H%M%S";

/// Uma versão arquivada de um arquivo. (→ ipc.ts)
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileVersion {
    /// Carimbo `YYYYMMDD-HHMMSS` extraído do nome arquivado.
    pub stamp: String,
    pub size_bytes: i64,
    pub modified_at_ms: i64,
    pub abs_path: String,
}

/// Estratégia de versionamento de arquivos locais antes de sobrescrever.
pub trait Versioner: Send + Sync {
    /// Arquiva `src` como a versão vigente de `rel_path`, e poda o histórico
    /// para no máximo `max_versions`. Retorna o caminho arquivado.
    fn archive(
        &self,
        emulator: &str,
        category: &str,
        rel_path: &str,
        src: &Path,
        max_versions: usize,
    ) -> AppResult<PathBuf>;

    /// Versões arquivadas de um arquivo, mais recentes primeiro.
    fn versions(
        &self,
        emulator: &str,
        category: &str,
        rel_path: &str,
    ) -> AppResult<Vec<FileVersion>>;

    /// Remove as versões mais antigas além de `max_versions`.
    fn clean(
        &self,
        emulator: &str,
        category: &str,
        rel_path: &str,
        max_versions: usize,
    ) -> AppResult<()>;
}

/// Implementação sobre o filesystem nativo (o diretório de backups é sempre um
/// caminho nativo, mesmo no mobile — é área privada do app).
pub struct FsVersioner {
    /// Raiz dos backups (`<app_data>/backups`).
    root: PathBuf,
}

impl FsVersioner {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// Pasta que guarda as versões de `rel_path` (a pasta-mãe do arquivo dentro
    /// de `history/<categoria>/`).
    fn dir_for(&self, emulator: &str, category: &str, rel_path: &str) -> PathBuf {
        let mut dir = self
            .root
            .join(emulator)
            .join(HISTORY_DIR_NAME)
            .join(category);
        if let Some((parent, _)) = rel_path.rsplit_once('/') {
            for part in parent.split('/') {
                dir.push(part);
            }
        }
        dir
    }
}

/// `("SAVE.bin", "20250717-103000")` → `"SAVE~20250717-103000.bin"`.
fn versioned_name(file_name: &str, stamp: &str) -> String {
    match file_name.rsplit_once('.') {
        Some((stem, ext)) => format!("{stem}{VERSION_SEP}{stamp}.{ext}"),
        None => format!("{file_name}{VERSION_SEP}{stamp}"),
    }
}

/// Carimbo de um nome arquivado, se o nome pertence às versões de `file_name`.
fn stamp_of(archived: &str, file_name: &str) -> Option<String> {
    let (stem, ext) = match file_name.rsplit_once('.') {
        Some((stem, ext)) => (stem, Some(ext)),
        None => (file_name, None),
    };
    let rest = archived.strip_prefix(stem)?.strip_prefix(VERSION_SEP)?;
    let stamp = match ext {
        Some(ext) => rest.strip_suffix(ext)?.strip_suffix('.')?,
        None => rest,
    };
    // Carimbo esperado: `YYYYMMDD-HHMMSS` (15 chars, um hífen).
    (stamp.len() == 15 && stamp.chars().all(|c| c.is_ascii_digit() || c == '-'))
        .then(|| stamp.to_string())
}

impl Versioner for FsVersioner {
    fn archive(
        &self,
        emulator: &str,
        category: &str,
        rel_path: &str,
        src: &Path,
        max_versions: usize,
    ) -> AppResult<PathBuf> {
        let file_name = rel_path.rsplit('/').next().unwrap_or(rel_path);
        let stamp = chrono::Local::now().format(STAMP_FORMAT).to_string();
        let dir = self.dir_for(emulator, category, rel_path);
        std::fs::create_dir_all(&dir)?;
        let dest = dir.join(versioned_name(file_name, &stamp));
        std::fs::copy(src, &dest)?;
        self.clean(emulator, category, rel_path, max_versions)?;
        Ok(dest)
    }

    fn versions(
        &self,
        emulator: &str,
        category: &str,
        rel_path: &str,
    ) -> AppResult<Vec<FileVersion>> {
        let file_name = rel_path.rsplit('/').next().unwrap_or(rel_path);
        let dir = self.dir_for(emulator, category, rel_path);
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return Ok(Vec::new());
        };

        let mut out = Vec::new();
        for entry in entries.filter_map(|e| e.ok()) {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(stamp) = stamp_of(&name, file_name) else {
                continue;
            };
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if !metadata.is_file() {
                continue;
            }
            let modified_at_ms = metadata
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);
            out.push(FileVersion {
                stamp,
                size_bytes: metadata.len() as i64,
                modified_at_ms,
                abs_path: entry.path().to_string_lossy().into_owned(),
            });
        }
        // Carimbo é lexicograficamente ordenável (YYYYMMDD-HHMMSS).
        out.sort_by(|a, b| b.stamp.cmp(&a.stamp));
        Ok(out)
    }

    fn clean(
        &self,
        emulator: &str,
        category: &str,
        rel_path: &str,
        max_versions: usize,
    ) -> AppResult<()> {
        let versions = self.versions(emulator, category, rel_path)?;
        for old in versions.iter().skip(max_versions.max(1)) {
            let _ = std::fs::remove_file(&old.abs_path);
        }
        Ok(())
    }
}

/// Inverte [`versioned_name`]: recupera o nome original de um arquivo
/// arquivado (`"SAVE~20250717-103000.bin"` → `"SAVE.bin"`). `None` quando o
/// nome não segue o formato de versão.
pub fn original_name(archived: &str) -> Option<String> {
    let (stem_and_stamp, ext) = match archived.rsplit_once('.') {
        Some((rest, ext)) => (rest, Some(ext)),
        None => (archived, None),
    };
    let (stem, stamp) = stem_and_stamp.rsplit_once(VERSION_SEP)?;
    if stamp.len() != 15 || !stamp.chars().all(|c| c.is_ascii_digit() || c == '-') {
        return None;
    }
    Some(match ext {
        Some(ext) => format!("{stem}.{ext}"),
        None => stem.to_string(),
    })
}

/// Resolve uma restauração do histórico: valida `versioned_rel_path`
/// (relativo, sem `..`/segmentos vazios), monta o caminho absoluto da versão
/// sob `<backups_root>/<emulador>/history/<categoria>/` e deriva o `rel_path`
/// original (sem o carimbo). Erro se o caminho é inseguro, o nome não tem
/// formato de versão ou o arquivo não existe.
pub fn resolve_restore(
    backups_root: &Path,
    emulator: &str,
    category: &str,
    versioned_rel_path: &str,
) -> AppResult<(PathBuf, String)> {
    use crate::error::AppError;

    if versioned_rel_path.is_empty()
        || versioned_rel_path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(AppError::Other(format!(
            "caminho de versão inválido: {versioned_rel_path}"
        )));
    }

    let mut src_abs = backups_root
        .join(emulator)
        .join(HISTORY_DIR_NAME)
        .join(category);
    for part in versioned_rel_path.split('/') {
        src_abs.push(part);
    }
    if !src_abs.is_file() {
        return Err(AppError::Other(format!(
            "versão não encontrada: {versioned_rel_path}"
        )));
    }

    let (dir_part, archived_name) = match versioned_rel_path.rsplit_once('/') {
        Some((dir, name)) => (Some(dir), name),
        None => (None, versioned_rel_path),
    };
    let original = original_name(archived_name)
        .ok_or_else(|| AppError::Other(format!("nome de versão inválido: {archived_name}")))?;
    let original_rel = match dir_part {
        Some(dir) => format!("{dir}/{original}"),
        None => original,
    };

    Ok((src_abs, original_rel))
}

/// Marcador no nome das cópias padronizadas de conflito.
const CONFLICT_MARKER: &str = ".slot2sync-conflict-";

/// `("SAVE.bin", "20250717-103000", "dev-a")` →
/// `"SAVE.slot2sync-conflict-20250717-103000-dev-a.bin"`.
pub fn conflict_copy_name(file_name: &str, stamp: &str, device_id: &str) -> String {
    match file_name.rsplit_once('.') {
        Some((stem, ext)) => format!("{stem}{CONFLICT_MARKER}{stamp}-{device_id}.{ext}"),
        None => format!("{file_name}{CONFLICT_MARKER}{stamp}-{device_id}"),
    }
}

/// Mantém no máximo `max` cópias de conflito de `file_name` em `dir`, apagando
/// as mais antigas (o carimbo no nome é lexicograficamente ordenável).
pub fn prune_conflict_copies(dir: &Path, file_name: &str, max: usize) -> AppResult<()> {
    let stem = file_name.rsplit_once('.').map_or(file_name, |(s, _)| s);
    let prefix = format!("{stem}{CONFLICT_MARKER}");
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(());
    };
    let mut copies: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with(&prefix))
        .collect();
    // Mais recentes primeiro (carimbo no nome).
    copies.sort_by(|a, b| b.cmp(a));
    for old in copies.iter().skip(max.max(1)) {
        let _ = std::fs::remove_file(dir.join(old));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup() -> (tempfile::TempDir, FsVersioner, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let versioner = FsVersioner::new(tmp.path().join("backups"));
        let src = tmp.path().join("SAVE.bin");
        std::fs::write(&src, b"v1").unwrap();
        (tmp, versioner, src)
    }

    #[test]
    fn resolve_restore_deriva_origem_e_rel_original() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp
            .path()
            .join("PPSSPP")
            .join(HISTORY_DIR_NAME)
            .join("saves")
            .join("GAME01");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("SAVE~20250717-103000.bin"), b"v1").unwrap();

        let (src, original_rel) = resolve_restore(
            tmp.path(),
            "PPSSPP",
            "saves",
            "GAME01/SAVE~20250717-103000.bin",
        )
        .unwrap();

        assert_eq!(src, dir.join("SAVE~20250717-103000.bin"));
        assert_eq!(original_rel, "GAME01/SAVE.bin");
    }

    #[test]
    fn resolve_restore_rejeita_traversal_e_nomes_invalidos() {
        let tmp = tempfile::tempdir().unwrap();
        // Traversal e segmentos vazios.
        assert!(resolve_restore(tmp.path(), "E", "saves", "../fora.bin").is_err());
        assert!(resolve_restore(tmp.path(), "E", "saves", "a//b.bin").is_err());
        assert!(resolve_restore(tmp.path(), "E", "saves", "").is_err());
        // Arquivo inexistente.
        assert!(resolve_restore(tmp.path(), "E", "saves", "SAVE~20250717-103000.bin").is_err());
        // Existe mas sem formato de versão.
        let dir = tmp.path().join("E").join(HISTORY_DIR_NAME).join("saves");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("SAVE.bin"), b"x").unwrap();
        assert!(resolve_restore(tmp.path(), "E", "saves", "SAVE.bin").is_err());
    }

    #[test]
    fn original_name_inverte_o_nome_arquivado() {
        assert_eq!(
            original_name("SAVE~20250717-103000.bin").as_deref(),
            Some("SAVE.bin")
        );
        assert_eq!(
            original_name("SAVE~20250717-103000").as_deref(),
            Some("SAVE")
        );
        assert_eq!(original_name("SAVE.bin"), None);
        assert_eq!(original_name("SAVE~qualquer.bin"), None);
    }

    #[test]
    fn conflict_copy_name_embute_carimbo_e_dispositivo() {
        assert_eq!(
            conflict_copy_name("SAVE.bin", "20250717-103000", "dev-a"),
            "SAVE.slot2sync-conflict-20250717-103000-dev-a.bin"
        );
        assert_eq!(
            conflict_copy_name("SAVE", "20250717-103000", "dev-a"),
            "SAVE.slot2sync-conflict-20250717-103000-dev-a"
        );
    }

    #[test]
    fn prune_conflict_copies_mantem_as_mais_recentes() {
        let tmp = tempfile::tempdir().unwrap();
        for stamp in ["20250101-100000", "20250102-100000", "20250103-100000"] {
            std::fs::write(
                tmp.path()
                    .join(conflict_copy_name("SAVE.bin", stamp, "dev")),
                b"x",
            )
            .unwrap();
        }
        // Arquivo de outro save não é tocado.
        std::fs::write(tmp.path().join("OUTRO.bin"), b"x").unwrap();

        prune_conflict_copies(tmp.path(), "SAVE.bin", 2).unwrap();

        let mut names: Vec<String> = std::fs::read_dir(tmp.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names.len(), 3);
        assert!(names.iter().any(|n| n.contains("20250103")));
        assert!(names.iter().any(|n| n.contains("20250102")));
        assert!(!names.iter().any(|n| n.contains("20250101")));
    }

    #[test]
    fn versioned_name_preserva_extensao() {
        assert_eq!(
            versioned_name("SAVE.bin", "20250717-103000"),
            "SAVE~20250717-103000.bin"
        );
        assert_eq!(
            versioned_name("SAVE", "20250717-103000"),
            "SAVE~20250717-103000"
        );
    }

    #[test]
    fn stamp_of_reconhece_somente_versoes_do_arquivo() {
        assert_eq!(
            stamp_of("SAVE~20250717-103000.bin", "SAVE.bin").as_deref(),
            Some("20250717-103000")
        );
        // Outro arquivo com prefixo parecido não conta.
        assert_eq!(stamp_of("SAVEX~20250717-103000.bin", "SAVE.bin"), None);
        // Sem carimbo válido não conta.
        assert_eq!(stamp_of("SAVE~qualquer.bin", "SAVE.bin"), None);
    }

    #[test]
    fn archive_grava_em_history_e_lista_versoes() {
        let (_tmp, versioner, src) = setup();

        let dest = versioner
            .archive("PPSSPP", "saves", "GAME01/SAVE.bin", &src, 5)
            .unwrap();

        assert!(dest.to_string_lossy().contains("history"));
        assert_eq!(std::fs::read(&dest).unwrap(), b"v1");

        let versions = versioner
            .versions("PPSSPP", "saves", "GAME01/SAVE.bin")
            .unwrap();
        assert_eq!(versions.len(), 1);
        assert_eq!(versions[0].size_bytes, 2);
    }

    #[test]
    fn clean_mantem_apenas_as_n_mais_recentes() {
        let (_tmp, versioner, src) = setup();
        // Simula 4 versões com carimbos distintos (nomes forjados à mão para
        // não depender do relógio).
        let dir = versioner.dir_for("PPSSPP", "saves", "SAVE.bin");
        std::fs::create_dir_all(&dir).unwrap();
        for stamp in [
            "20250101-100000",
            "20250102-100000",
            "20250103-100000",
            "20250104-100000",
        ] {
            std::fs::copy(&src, dir.join(versioned_name("SAVE.bin", stamp))).unwrap();
        }

        versioner.clean("PPSSPP", "saves", "SAVE.bin", 2).unwrap();

        let versions = versioner.versions("PPSSPP", "saves", "SAVE.bin").unwrap();
        let stamps: Vec<_> = versions.iter().map(|v| v.stamp.as_str()).collect();
        assert_eq!(stamps, vec!["20250104-100000", "20250103-100000"]);
    }
}
