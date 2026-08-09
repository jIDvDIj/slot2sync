//! Emuladores configurados pelo usuário (perfil completo serializado) e suas
//! configurações de sync (quais categorias sincronizar).

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::emulator::EmulatorProfile;
use crate::error::AppResult;

/// Categorias de sync habilitadas para um emulador. (→ ipc.ts) Default:
/// saves/savestates ativas.
///
/// `config` (versionamento das pastas de configuração do emulador) está
/// permanentemente desativado — `get_categories` sempre devolve `false` e
/// `set_categories` ignora o valor recebido, independente do que o SQLite
/// tenha armazenado de sessões antigas. Não há opção na UI para reativar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncCategories {
    pub saves: bool,
    pub savestates: bool,
    pub config: bool,
}

impl Default for SyncCategories {
    fn default() -> Self {
        Self {
            saves: true,
            savestates: true,
            config: false,
        }
    }
}

/// Categorias habilitadas de um emulador; default (saves/savestates ativas)
/// se nunca foi configurado. `config` é sempre forçado a `false`.
pub fn get_categories(conn: &Connection, emulator: &str) -> AppResult<SyncCategories> {
    let cats = conn
        .query_row(
            "SELECT saves_enabled, savestates_enabled, config_enabled \
             FROM emulator_settings WHERE emulator = ?1",
            params![emulator],
            |row| {
                Ok(SyncCategories {
                    saves: row.get::<_, i64>(0)? != 0,
                    savestates: row.get::<_, i64>(1)? != 0,
                    config: row.get::<_, i64>(2)? != 0,
                })
            },
        )
        .optional()?;
    let mut cats = cats.unwrap_or_default();
    cats.config = false;
    Ok(cats)
}

/// Grava as categorias habilitadas. `cats.config` é ignorado — sempre
/// persistido como desativado, mesmo que o chamador envie `true`.
pub fn set_categories(conn: &Connection, emulator: &str, cats: &SyncCategories) -> AppResult<()> {
    conn.execute(
        "INSERT OR REPLACE INTO emulator_settings \
         (emulator, saves_enabled, savestates_enabled, config_enabled) VALUES (?1, ?2, ?3, ?4)",
        params![emulator, cats.saves as i64, cats.savestates as i64, 0],
    )?;
    Ok(())
}

pub fn remove_categories(conn: &Connection, emulator: &str) -> AppResult<()> {
    conn.execute(
        "DELETE FROM emulator_settings WHERE emulator = ?1",
        params![emulator],
    )?;
    Ok(())
}

pub fn upsert(conn: &Connection, profile: &EmulatorProfile) -> AppResult<()> {
    conn.execute(
        "INSERT OR REPLACE INTO emulators (name, root_path, profile_json, added_at_ms) \
         VALUES (?1, ?2, ?3, ?4)",
        params![
            profile.name,
            profile.root_path.to_string_lossy(),
            serde_json::to_string(profile)?,
            chrono::Utc::now().timestamp_millis(),
        ],
    )?;
    Ok(())
}

/// `upsert` ciente de troca de caminho. Quando um emulador já registrado é
/// regravado apontando para outro `root_path` (ex.: trocar a instalação portátil
/// no pendrive pela instalada no sistema), os mtimes ancorados no `sync_manifest`
/// passam a se referir aos arquivos de OUTRA instalação. Comparar o estado local
/// novo contra essas âncoras inverteria a direção do sync — o diff veria "o local
/// mudou" e subiria saves antigos por cima dos recém-sincronizados no Drive.
///
/// Por isso, ao detectar a troca de caminho, zera o estado de sync do emulador
/// (manifest, conflitos e fila offline) na mesma transação do upsert. Sem âncoras,
/// o próximo sync trata tudo como primeiro sync: o Drive vence com backup local
/// antes de sobrescrever (ver `conflict::decide` → `DownloadWithBackup`), então
/// nada é perdido. Retorna `true` se houve reset.
pub fn upsert_resetting_on_path_change(
    conn: &Connection,
    profile: &EmulatorProfile,
) -> AppResult<bool> {
    let previous_root: Option<String> = conn
        .query_row(
            "SELECT root_path FROM emulators WHERE name = ?1",
            params![profile.name],
            |row| row.get(0),
        )
        .optional()?;

    let new_root = profile.root_path.to_string_lossy().into_owned();
    let path_changed = previous_root.as_ref().is_some_and(|old| *old != new_root);

    upsert(conn, profile)?;

    if path_changed {
        crate::storage::manifest::remove_for_emulator(conn, &profile.name)?;
        crate::storage::conflicts::remove_for_emulator(conn, &profile.name)?;
        crate::storage::queue::remove_for_emulator(conn, &profile.name)?;
    }

    Ok(path_changed)
}

/// Atualiza somente os padrões de exclusão do perfil persistido. Não mexe no
/// estado de sync — excluir um arquivo não invalida as âncoras dos demais.
pub fn set_exclude_patterns(conn: &Connection, name: &str, patterns: &[String]) -> AppResult<()> {
    let json: Option<String> = conn
        .query_row(
            "SELECT profile_json FROM emulators WHERE name = ?1",
            params![name],
            |row| row.get(0),
        )
        .optional()?;
    let Some(json) = json else {
        return Err(crate::error::AppError::Other(format!(
            "emulador não encontrado: {name}"
        )));
    };
    let mut profile: EmulatorProfile = serde_json::from_str(&json)?;
    profile.exclude_patterns = patterns.to_vec();
    upsert(conn, &profile)
}

/// `true` se já existe um emulador registrado com este nome (auto ou manual).
pub fn exists(conn: &Connection, name: &str) -> AppResult<bool> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM emulators WHERE name = ?1",
        params![name],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

pub fn list(conn: &Connection) -> AppResult<Vec<EmulatorProfile>> {
    let mut stmt = conn.prepare("SELECT profile_json FROM emulators ORDER BY name")?;
    let raw = stmt
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut profiles = Vec::with_capacity(raw.len());
    for json in raw {
        profiles.push(serde_json::from_str(&json)?);
    }
    Ok(profiles)
}

pub fn remove(conn: &Connection, name: &str) -> AppResult<()> {
    conn.execute("DELETE FROM emulators WHERE name = ?1", params![name])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::storage::db::Db;

    fn sample_profile() -> EmulatorProfile {
        EmulatorProfile {
            name: "PPSSPP".into(),
            root_path: PathBuf::from("/tmp/ppsspp"),
            saves_paths: vec![PathBuf::from("PSP/SAVEDATA")],
            config_paths: vec![PathBuf::from("PSP/SYSTEM")],
            state_paths: vec![PathBuf::from("PSP/PPSSPP_STATE")],
            exclude_patterns: vec![],
        }
    }

    #[test]
    fn upsert_e_list_fazem_roundtrip_do_perfil() {
        let db = Db::open_in_memory().unwrap();
        db.with_sync(|conn| {
            upsert(conn, &sample_profile())?;

            let profiles = list(conn)?;
            assert_eq!(profiles, vec![sample_profile()]);
            Ok(())
        });
    }

    #[test]
    fn upsert_substitui_perfil_com_mesmo_nome() {
        let db = Db::open_in_memory().unwrap();
        db.with_sync(|conn| {
            upsert(conn, &sample_profile())?;
            let mut updated = sample_profile();
            updated.root_path = PathBuf::from("/outro/lugar");
            upsert(conn, &updated)?;

            let profiles = list(conn)?;
            assert_eq!(profiles.len(), 1);
            assert_eq!(profiles[0].root_path, PathBuf::from("/outro/lugar"));
            Ok(())
        });
    }

    fn seed_manifest(conn: &Connection) -> AppResult<()> {
        use crate::storage::manifest::{self, ManifestEntry};
        use crate::sync::SyncCategory;
        manifest::upsert(
            conn,
            &ManifestEntry {
                emulator: "PPSSPP".into(),
                category: SyncCategory::Saves,
                rel_path: "GAME123/SAVE.bin".into(),
                drive_file_id: Some("drive-id".into()),
                local_mtime_ms: Some(1_700_000_000_000),
                drive_mtime_ms: Some(1_700_000_000_000),
                size_bytes: Some(4096),
                last_synced_at_ms: 1_700_000_000_000,
                file_hash: None,
            },
        )
    }

    #[test]
    fn upsert_com_caminho_novo_reseta_estado_de_sync() {
        // Trocar o root_path de um emulador já registrado (portátil → instalado)
        // zera o manifest — as âncoras de mtime apontavam para a outra instalação.
        let db = Db::open_in_memory().unwrap();
        db.with_sync(|conn| {
            upsert(conn, &sample_profile())?;
            seed_manifest(conn)?;

            let mut moved = sample_profile();
            moved.root_path = PathBuf::from("/pendrive/ppsspp");
            let reset = upsert_resetting_on_path_change(conn, &moved)?;

            assert!(reset);
            assert!(crate::storage::manifest::list_all(conn)?.is_empty());
            assert_eq!(list(conn)?[0].root_path, PathBuf::from("/pendrive/ppsspp"));
            Ok(())
        });
    }

    #[test]
    fn upsert_com_mesmo_caminho_preserva_o_manifest() {
        // Re-detectar a mesma pasta não pode nukear o estado de sync.
        let db = Db::open_in_memory().unwrap();
        db.with_sync(|conn| {
            upsert(conn, &sample_profile())?;
            seed_manifest(conn)?;

            let reset = upsert_resetting_on_path_change(conn, &sample_profile())?;

            assert!(!reset);
            assert_eq!(crate::storage::manifest::list_all(conn)?.len(), 1);
            Ok(())
        });
    }

    #[test]
    fn upsert_de_emulador_novo_nao_reseta() {
        let db = Db::open_in_memory().unwrap();
        db.with_sync(|conn| {
            let reset = upsert_resetting_on_path_change(conn, &sample_profile())?;
            assert!(!reset);
            assert_eq!(list(conn)?.len(), 1);
            Ok(())
        });
    }

    #[test]
    fn remove_apaga_o_perfil() {
        let db = Db::open_in_memory().unwrap();
        db.with_sync(|conn| {
            upsert(conn, &sample_profile())?;
            remove(conn, "PPSSPP")?;
            assert!(list(conn)?.is_empty());
            Ok(())
        });
    }

    #[test]
    fn exists_reflete_presenca_do_perfil() {
        let db = Db::open_in_memory().unwrap();
        db.with_sync(|conn| {
            assert!(!exists(conn, "PPSSPP")?);
            upsert(conn, &sample_profile())?;
            assert!(exists(conn, "PPSSPP")?);
            assert!(!exists(conn, "PCSX2")?);
            Ok(())
        });
    }

    #[test]
    fn set_exclude_patterns_atualiza_o_perfil_persistido() {
        let db = Db::open_in_memory().unwrap();
        db.with_sync(|conn| {
            upsert(conn, &sample_profile())?;

            set_exclude_patterns(conn, "PPSSPP", &["*.tmp".into(), "cache/**".into()])?;

            let profiles = list(conn)?;
            assert_eq!(
                profiles[0].exclude_patterns,
                vec!["*.tmp".to_string(), "cache/**".to_string()]
            );
            Ok(())
        });
    }

    #[test]
    fn set_exclude_patterns_em_emulador_inexistente_e_erro() {
        let db = Db::open_in_memory().unwrap();
        db.with_sync(|conn| {
            assert!(set_exclude_patterns(conn, "Nada", &[]).is_err());
            Ok(())
        });
    }

    #[test]
    fn categorias_default_sao_saves_e_savestates_config_sempre_off() {
        let db = Db::open_in_memory().unwrap();
        db.with_sync(|conn| {
            assert_eq!(get_categories(conn, "PPSSPP")?, SyncCategories::default());
            assert_eq!(
                get_categories(conn, "PPSSPP")?,
                SyncCategories {
                    saves: true,
                    savestates: true,
                    config: false,
                }
            );
            Ok(())
        });
    }

    #[test]
    fn set_e_get_categorias_fazem_roundtrip_config_sempre_off() {
        let db = Db::open_in_memory().unwrap();
        db.with_sync(|conn| {
            let cats = SyncCategories {
                saves: true,
                savestates: false,
                config: true, // ignorado: config nunca fica ativo
            };
            set_categories(conn, "PPSSPP", &cats)?;
            assert_eq!(
                get_categories(conn, "PPSSPP")?,
                SyncCategories {
                    saves: true,
                    savestates: false,
                    config: false,
                }
            );
            Ok(())
        });
    }

    #[test]
    fn remove_categorias_volta_ao_default() {
        let db = Db::open_in_memory().unwrap();
        db.with_sync(|conn| {
            set_categories(
                conn,
                "PPSSPP",
                &SyncCategories {
                    saves: false,
                    savestates: false,
                    config: false,
                },
            )?;
            remove_categories(conn, "PPSSPP")?;
            assert_eq!(get_categories(conn, "PPSSPP")?, SyncCategories::default());
            Ok(())
        });
    }

    #[test]
    fn categorias_serializam_em_camel_case() {
        let json = serde_json::to_value(SyncCategories::default()).unwrap();
        assert_eq!(json["saves"], true);
        assert_eq!(json["savestates"], true);
        assert_eq!(json["config"], false);
    }
}
