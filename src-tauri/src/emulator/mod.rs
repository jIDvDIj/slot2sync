//! Perfis de emuladores e detecção automática.
//!
//! O catálogo de emuladores conhecidos é dirigido por dados: vive em
//! `profiles.toml` e é interpretado por `profiles.rs`. Cada perfil descreve onde
//! ficam saves, savestates e configurações relativos à pasta raiz.
//! `detect_emulator(root_path)` identifica o emulador a partir de marcadores
//! no filesystem (pastas características de cada um).

mod profiles;

use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Descrição de um emulador configurado. Cruza a boundary para o frontend. (→ ipc.ts)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmulatorProfile {
    /// Nome canônico ("PPSSPP", "PCSX2") — usado como nome da pasta no Drive.
    pub name: String,
    /// Pasta raiz selecionada pelo usuário.
    pub root_path: PathBuf,
    /// Pastas de saves, relativas a `root_path`.
    pub saves_paths: Vec<PathBuf>,
    /// Pastas de configuração, relativas a `root_path`.
    pub config_paths: Vec<PathBuf>,
    /// Pastas de savestates, relativas a `root_path`.
    pub state_paths: Vec<PathBuf>,
    /// Padrões glob (`*.tmp`, `cache/**`) de arquivos a IGNORAR no sync,
    /// casados contra o `rel_path` de cada arquivo. Defaults por emulador vêm
    /// do `profiles.toml`; o usuário edita nas configurações.
    #[serde(default)]
    pub exclude_patterns: Vec<String>,
}

/// Sugestão da descoberta automática. (→ ipc.ts)
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredEmulator {
    /// Nome canônico do catálogo.
    pub name: String,
    /// `Some` quando os saves foram encontrados (pode adicionar direto).
    /// `None` = só o registro confirmou instalação (sem pasta de dados ainda).
    pub profile: Option<EmulatorProfile>,
    /// De onde veio o reconhecimento.
    pub source: DiscoverySource,
}

/// Origem do reconhecimento na descoberta — serializa em camelCase
/// (`dataDir`/`registry`/`both`). (→ ipc.ts)
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DiscoverySource {
    DataDir,
    Registry,
    Both,
}

/// Identifica o emulador presente em `root_path` e monta o perfil com os
/// caminhos relevantes. `None` quando nenhum emulador suportado é reconhecido.
///
/// Faz I/O síncrono de disco — em contexto async, chamar via `spawn_blocking`.
pub fn detect_emulator(root_path: &Path) -> Option<EmulatorProfile> {
    profiles::detect(root_path)
}

/// Variante mobile de [`detect_emulator`]: identifica o emulador presente sob
/// a árvore SAF `root_display` delegando cada checagem de existência a
/// `exists` (chamada ao plugin nativo), já que não há filesystem direto.
#[cfg(mobile)]
pub async fn detect_emulator_async<F, Fut>(root_display: &str, exists: F) -> Option<EmulatorProfile>
where
    F: FnMut(String) -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    profiles::detect_async(root_display, exists).await
}

/// Nomes de processo do SO associados a um emulador, para o process watcher.
/// Vazio se o nome canônico não corresponder a um perfil do catálogo.
/// Só-desktop: no mobile não há process watcher.
#[cfg(desktop)]
pub fn process_names(emulator_name: &str) -> Vec<String> {
    profiles::process_names(emulator_name)
}

/// Varre o catálogo por emuladores instalados no sistema (pastas de dados
/// conhecidas + registro no Windows). Não persiste nada — a UI usa o resultado
/// para sugerir adições.
///
/// Faz I/O de disco e, no Windows, leitura de registro — em contexto async,
/// chamar via `spawn_blocking`.
pub fn discover_installed() -> Vec<DiscoveredEmulator> {
    profiles::discover_installed()
}

/// Monta um `EmulatorProfile` a partir de pastas informadas manualmente pelo
/// usuário (fallback quando a detecção automática falha). Os caminhos chegam
/// relativos à raiz; cada um é validado quanto à *segurança* e normalizado.
///
/// Falha (com mensagem para o usuário) se: o nome for vazio; algum caminho for
/// absoluto ou contiver `..`/prefixo de raiz; ou se nenhuma categoria tiver
/// pasta (perfil vazio).
///
/// **Não** verifica se as pastas existem — essa checagem (que no mobile depende
/// do SAF, não de `std::fs`) é feita pelo chamador via
/// [`crate::sync::LocalStorage::subdir_exists`]. Função pura, sem I/O.
pub fn build_manual_profile(
    root: &Path,
    name: String,
    saves: Vec<String>,
    states: Vec<String>,
    config: Vec<String>,
) -> Result<EmulatorProfile, String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("nome do emulador é obrigatório".into());
    }

    let saves_paths = validate_rel_dirs(saves)?;
    let state_paths = validate_rel_dirs(states)?;
    let config_paths = validate_rel_dirs(config)?;

    if saves_paths.is_empty() && state_paths.is_empty() && config_paths.is_empty() {
        return Err("informe ao menos uma pasta (saves, savestates ou config)".into());
    }

    Ok(EmulatorProfile {
        name,
        root_path: root.to_path_buf(),
        saves_paths,
        config_paths,
        state_paths,
        exclude_patterns: Vec::new(),
    })
}

/// Valida e normaliza caminhos relativos à raiz. Entradas vazias são ignoradas
/// (campo não preenchido na UI); rejeita absolutos, `..` e prefixos/raiz. A
/// existência de cada pasta é conferida depois, pelo `LocalStorage` do chamador.
fn validate_rel_dirs(dirs: Vec<String>) -> Result<Vec<PathBuf>, String> {
    let mut out = Vec::with_capacity(dirs.len());
    for d in dirs {
        if d.trim().is_empty() {
            continue;
        }
        let rel = PathBuf::from(&d);
        let escapes = rel.components().any(|c| {
            matches!(
                c,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        });
        if rel.is_absolute() || escapes {
            return Err(format!("o caminho deve ser relativo à raiz: {d}"));
        }
        out.push(rel);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn mkdirs(root: &Path, dirs: &[&str]) {
        for dir in dirs {
            fs::create_dir_all(root.join(dir)).unwrap();
        }
    }

    #[test]
    fn detecta_ppsspp_em_pasta_de_dados() {
        let tmp = tempfile::tempdir().unwrap();
        mkdirs(tmp.path(), &["PSP/SAVEDATA", "PSP/SYSTEM"]);

        let profile = detect_emulator(tmp.path()).expect("deveria detectar PPSSPP");

        assert_eq!(profile.name, "PPSSPP");
        assert_eq!(profile.root_path, tmp.path());
        assert_eq!(profile.saves_paths, vec![Path::new("PSP").join("SAVEDATA")]);
        assert_eq!(profile.config_paths, vec![Path::new("PSP").join("SYSTEM")]);
        assert_eq!(
            profile.state_paths,
            vec![Path::new("PSP").join("PPSSPP_STATE")]
        );
    }

    #[test]
    fn detecta_ppsspp_em_instalacao_portatil() {
        let tmp = tempfile::tempdir().unwrap();
        mkdirs(tmp.path(), &["memstick/PSP/SAVEDATA"]);

        let profile = detect_emulator(tmp.path()).expect("deveria detectar PPSSPP portátil");

        assert_eq!(profile.name, "PPSSPP");
        assert_eq!(
            profile.saves_paths,
            vec![Path::new("memstick").join("PSP").join("SAVEDATA")]
        );
    }

    #[test]
    fn nao_detecta_ppsspp_com_psp_sem_marcadores() {
        let tmp = tempfile::tempdir().unwrap();
        mkdirs(tmp.path(), &["PSP"]);

        assert_eq!(detect_emulator(tmp.path()), None);
    }

    #[test]
    fn detecta_pcsx2_em_pasta_de_dados() {
        let tmp = tempfile::tempdir().unwrap();
        mkdirs(tmp.path(), &["inis", "memcards", "sstates"]);

        let profile = detect_emulator(tmp.path()).expect("deveria detectar PCSX2");

        assert_eq!(profile.name, "PCSX2");
        assert_eq!(profile.root_path, tmp.path());
        assert_eq!(profile.saves_paths, vec![PathBuf::from("memcards")]);
        assert_eq!(profile.config_paths, vec![PathBuf::from("inis")]);
        assert_eq!(profile.state_paths, vec![PathBuf::from("sstates")]);
    }

    #[test]
    fn detecta_pcsx2_somente_com_inis_e_bios() {
        let tmp = tempfile::tempdir().unwrap();
        mkdirs(tmp.path(), &["inis", "bios"]);

        let profile = detect_emulator(tmp.path()).expect("deveria detectar PCSX2");
        assert_eq!(profile.name, "PCSX2");
    }

    #[test]
    fn nao_detecta_pcsx2_somente_com_inis() {
        let tmp = tempfile::tempdir().unwrap();
        mkdirs(tmp.path(), &["inis"]);

        assert_eq!(detect_emulator(tmp.path()), None);
    }

    #[test]
    fn nao_detecta_em_pasta_vazia() {
        let tmp = tempfile::tempdir().unwrap();

        assert_eq!(detect_emulator(tmp.path()), None);
    }

    #[test]
    fn perfil_serializa_em_camel_case() {
        let tmp = tempfile::tempdir().unwrap();
        mkdirs(tmp.path(), &["inis", "memcards"]);

        let profile = detect_emulator(tmp.path()).unwrap();
        let json = serde_json::to_value(&profile).unwrap();

        assert_eq!(json["name"], "PCSX2");
        assert!(json["rootPath"].is_string());
        assert!(json["savesPaths"].is_array());
        assert!(json["configPaths"].is_array());
        assert!(json["statePaths"].is_array());
    }

    #[test]
    fn manual_aceita_perfil_valido_e_normaliza_relativos() {
        let tmp = tempfile::tempdir().unwrap();
        mkdirs(tmp.path(), &["saves", "states", "cfg"]);

        let profile = build_manual_profile(
            tmp.path(),
            "  MeuEmu  ".into(),
            vec!["saves".into()],
            vec!["states".into()],
            vec!["cfg".into(), "".into()], // entrada vazia é ignorada
        )
        .expect("perfil válido");

        assert_eq!(profile.name, "MeuEmu"); // trim aplicado
        assert_eq!(profile.root_path, tmp.path());
        assert_eq!(profile.saves_paths, vec![PathBuf::from("saves")]);
        assert_eq!(profile.state_paths, vec![PathBuf::from("states")]);
        assert_eq!(profile.config_paths, vec![PathBuf::from("cfg")]);
    }

    #[test]
    fn manual_rejeita_nome_vazio() {
        let tmp = tempfile::tempdir().unwrap();
        mkdirs(tmp.path(), &["saves"]);
        let err = build_manual_profile(
            tmp.path(),
            "   ".into(),
            vec!["saves".into()],
            vec![],
            vec![],
        )
        .unwrap_err();
        assert!(err.contains("nome"));
    }

    #[test]
    fn manual_rejeita_perfil_sem_nenhuma_pasta() {
        let tmp = tempfile::tempdir().unwrap();
        let err = build_manual_profile(tmp.path(), "Emu".into(), vec![], vec![], vec!["".into()])
            .unwrap_err();
        assert!(err.contains("ao menos uma pasta"));
    }

    // A existência de cada pasta passou a ser conferida no comando via
    // `LocalStorage::subdir_exists` — coberta pelos testes de
    // `subdir_exists` em `sync::storage`. Aqui só validamos segurança de caminho.

    #[test]
    fn manual_rejeita_path_traversal() {
        let tmp = tempfile::tempdir().unwrap();
        mkdirs(tmp.path(), &["saves"]);
        let err = build_manual_profile(
            tmp.path(),
            "Emu".into(),
            vec!["../saves".into()],
            vec![],
            vec![],
        )
        .unwrap_err();
        assert!(err.contains("relativo à raiz"));
    }

    #[test]
    fn manual_rejeita_caminho_absoluto() {
        let tmp = tempfile::tempdir().unwrap();
        let abs = tmp.path().join("saves");
        fs::create_dir_all(&abs).unwrap();
        let err = build_manual_profile(
            tmp.path(),
            "Emu".into(),
            vec![abs.to_string_lossy().into_owned()],
            vec![],
            vec![],
        )
        .unwrap_err();
        assert!(err.contains("relativo à raiz"));
    }
}
