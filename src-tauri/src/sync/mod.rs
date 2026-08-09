//! Sincronização bidirecional com o Google Drive.
//!
//! O `SyncEngine` é agnóstico a emuladores: recebe `SyncTarget`s (rótulo +
//! listas de caminhos), nunca conhece PPSSPP ou PCSX2. Conflitos são
//! resolvidos por timestamp (mais recente vence; nunca deleta) e o progresso
//! é emitido ao frontend via eventos Tauri (`events::EVT_SYNC_*`).

mod conflict;
mod diff;
mod engine;
#[cfg(mobile)]
pub mod mobile_storage;
// `not(windows)`: o MockRuntime do tauri quebra o exe de teste no Windows
// (STATUS_ENTRYPOINT_NOT_FOUND); os cenários rodam no Linux/macOS, onde a
// cobertura também é medida.
#[cfg(all(test, desktop, not(windows)))]
mod scenarios;
mod storage;

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub use engine::{ConflictResolution, LastSync, LastSyncStore, SyncEngine, SyncSummary};
#[cfg(desktop)]
pub use storage::DesktopStorage;
pub use storage::{FileLoc, LocalStorage};

use crate::constants::{DRIVE_CONFIG_FOLDER, DRIVE_SAVES_FOLDER, DRIVE_STATES_FOLDER};
use crate::emulator::EmulatorProfile;

fn bytes_to_hex(bytes: impl AsRef<[u8]>) -> String {
    use std::fmt::Write;
    bytes.as_ref().iter().fold(String::new(), |mut hex, byte| {
        let _ = write!(hex, "{byte:02x}");
        hex
    })
}

/// SHA-256 (hex) de um conteúdo em memória — identidade de conteúdo usada no
/// pré-filtro de mtime do diff (coluna `file_hash` do manifest).
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    bytes_to_hex(Sha256::digest(bytes))
}

/// MD5 (hex) de um conteúdo em memória — comparável ao `md5Checksum` que a API
/// do Drive devolve (verificação de integridade pós-transferência).
pub(crate) fn md5_hex(bytes: &[u8]) -> String {
    use md5::{Digest, Md5};
    bytes_to_hex(Md5::digest(bytes))
}

/// Direção de uma operação de sync. (→ ipc.ts)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SyncDirection {
    DriveToLocal,
    LocalToDrive,
    Bidirectional,
}

/// Categoria de arquivos sincronizados; o valor textual é também o nome da
/// subpasta no Drive e a chave na coluna `category` do SQLite.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SyncCategory {
    Saves,
    Savestates,
    Config,
}

impl SyncCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            SyncCategory::Saves => DRIVE_SAVES_FOLDER,
            SyncCategory::Savestates => DRIVE_STATES_FOLDER,
            SyncCategory::Config => DRIVE_CONFIG_FOLDER,
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            DRIVE_SAVES_FOLDER => Some(SyncCategory::Saves),
            DRIVE_STATES_FOLDER => Some(SyncCategory::Savestates),
            DRIVE_CONFIG_FOLDER => Some(SyncCategory::Config),
            _ => None,
        }
    }
}

/// Payload do evento `sync:progress`. (→ ipc.ts)
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncProgress {
    pub emulator: String,
    pub current_file: String,
    pub completed: u32,
    pub total: u32,
    /// Bytes já transferidos / totais do plano da categoria em andamento —
    /// alimentam a barra de progresso, a velocidade e o ETA na UI.
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub direction: SyncDirection,
}

/// Alvo de sincronização agnóstico: o engine só enxerga rótulo + caminhos.
#[derive(Debug, Clone)]
pub struct SyncTarget {
    /// Nome da pasta do emulador no Drive (ex.: "PPSSPP").
    pub label: String,
    pub root: PathBuf,
    pub categories: Vec<(SyncCategory, Vec<PathBuf>)>,
    /// Padrões glob de arquivos a ignorar no sync (herdados do perfil).
    pub exclude_patterns: Vec<String>,
}

impl SyncTarget {
    pub fn from_profile(profile: &EmulatorProfile) -> Self {
        Self {
            label: profile.name.clone(),
            root: profile.root_path.clone(),
            categories: vec![
                (SyncCategory::Saves, profile.saves_paths.clone()),
                (SyncCategory::Savestates, profile.state_paths.clone()),
                (SyncCategory::Config, profile.config_paths.clone()),
            ],
            exclude_patterns: profile.exclude_patterns.clone(),
        }
    }
}

/// Compila os padrões de exclusão do emulador num `GlobSet` (uma vez por sync,
/// não por arquivo). Padrões inválidos são ignorados com warning — um glob
/// quebrado nunca derruba o sync. `None` = nada a excluir.
pub(crate) fn build_exclude_set(patterns: &[String]) -> Option<globset::GlobSet> {
    if patterns.is_empty() {
        return None;
    }
    let mut builder = globset::GlobSetBuilder::new();
    let mut any = false;
    for pattern in patterns {
        match globset::Glob::new(pattern) {
            Ok(glob) => {
                builder.add(glob);
                any = true;
            }
            Err(err) => {
                tracing::warn!(padrao = %pattern, error = %err, "padrão de exclusão inválido; ignorado");
            }
        }
    }
    if !any {
        return None;
    }
    builder.build().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn categoria_roundtrip_as_str_parse() {
        for cat in [
            SyncCategory::Saves,
            SyncCategory::Savestates,
            SyncCategory::Config,
        ] {
            assert_eq!(SyncCategory::parse(cat.as_str()), Some(cat));
        }
        assert_eq!(SyncCategory::parse("outra-coisa"), None);
    }

    #[test]
    fn from_profile_mapeia_as_tres_categorias() {
        let profile = EmulatorProfile {
            name: "PPSSPP".into(),
            root_path: PathBuf::from("/raiz"),
            saves_paths: vec![PathBuf::from("saves")],
            config_paths: vec![PathBuf::from("cfg"), PathBuf::from("cfg2")],
            state_paths: vec![],
            exclude_patterns: vec!["*.tmp".into()],
        };

        let target = SyncTarget::from_profile(&profile);

        assert_eq!(target.label, "PPSSPP");
        assert_eq!(target.root, PathBuf::from("/raiz"));
        assert_eq!(target.categories.len(), 3);
        let by_cat = |c: SyncCategory| {
            target
                .categories
                .iter()
                .find(|(cat, _)| *cat == c)
                .map(|(_, paths)| paths.len())
                .unwrap()
        };
        assert_eq!(by_cat(SyncCategory::Saves), 1);
        assert_eq!(by_cat(SyncCategory::Config), 2);
        assert_eq!(by_cat(SyncCategory::Savestates), 0);
    }
}
