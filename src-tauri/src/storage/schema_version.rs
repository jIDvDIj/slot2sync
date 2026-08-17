//! Versionamento lógico do *formato dos dados* por componente (`settings`,
//! `sync_manifest`), distinto do `PRAGMA user_version` físico em `storage::db`.
//!
//! Cada componente registra sua versão atual em `constants.rs`
//! (`SETTINGS_SCHEMA_VERSION`, `MANIFEST_SCHEMA_VERSION`). Ao abrir o banco,
//! [`ensure_current`] compara a versão gravada contra a versão atual do app:
//! - Gravada menor → aplica as migrações lógicas pendentes (nenhuma existe
//!   ainda; o mecanismo está pronto para quando o formato de `app_settings`
//!   ou `sync_manifest` precisar mudar sem alterar o schema físico).
//! - Gravada maior (banco aberto por uma versão mais nova do app, ex.: usuário
//!   voltou para uma build antiga) → loga aviso e não mexe nos dados, em vez
//!   de silenciosamente corrompê-los.
//! - Ausente (banco novo) → grava a versão atual, nada a migrar.

use rusqlite::{params, Connection};

use crate::error::AppResult;

fn get_version(conn: &Connection, component: &str) -> AppResult<Option<i64>> {
    let version = conn
        .prepare_cached("SELECT version FROM schema_version WHERE component = ?1")?
        .query_row(params![component], |row| row.get(0))
        .ok();
    Ok(version)
}

fn set_version(conn: &Connection, component: &str, version: i64) -> AppResult<()> {
    conn.prepare_cached(
        "INSERT INTO schema_version (component, version) VALUES (?1, ?2) \
         ON CONFLICT(component) DO UPDATE SET version = excluded.version",
    )?
    .execute(params![component, version])?;
    Ok(())
}

/// Garante que `component` está na versão `current`, aplicando migrações
/// lógicas pendentes (hoje nenhuma) ou apenas avisando se o banco pertence a
/// uma versão futura do app.
pub fn ensure_current(conn: &Connection, component: &str, current: i64) -> AppResult<()> {
    let stored = get_version(conn, component)?;
    match stored {
        None => set_version(conn, component, current)?,
        Some(v) if v < current => {
            // Migrações lógicas por componente entrariam aqui, uma por versão
            // (rename de chave, conversão de valor, preenchimento de default).
            set_version(conn, component, current)?;
        }
        Some(v) if v > current => {
            tracing::warn!(
                component,
                stored_version = v,
                current_version = current,
                "banco de dados foi aberto por uma versão mais nova do app; \
                 dados deste componente não serão tocados"
            );
        }
        Some(_) => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::db::Db;

    #[test]
    // Usa nomes de componente que `storage::db::migrate` não toca — "settings"
    // e "sync_manifest" já são inicializados por `Db::open_in_memory`.

    fn primeira_vez_grava_a_versao_atual() {
        let db = Db::open_in_memory().unwrap();
        db.with_sync(|conn| {
            assert_eq!(get_version(conn, "widget")?, None);
            ensure_current(conn, "widget", 3)?;
            assert_eq!(get_version(conn, "widget")?, Some(3));
            Ok(())
        });
    }

    #[test]
    fn versao_gravada_menor_e_atualizada_para_a_atual() {
        let db = Db::open_in_memory().unwrap();
        db.with_sync(|conn| {
            set_version(conn, "widget", 1)?;
            ensure_current(conn, "widget", 2)?;
            assert_eq!(get_version(conn, "widget")?, Some(2));
            Ok(())
        });
    }

    #[test]
    fn versao_gravada_maior_e_preservada_sem_erro() {
        let db = Db::open_in_memory().unwrap();
        db.with_sync(|conn| {
            set_version(conn, "widget", 5)?;
            ensure_current(conn, "widget", 2)?;
            assert_eq!(get_version(conn, "widget")?, Some(5));
            Ok(())
        });
    }

    #[test]
    fn componentes_sao_independentes() {
        let db = Db::open_in_memory().unwrap();
        db.with_sync(|conn| {
            ensure_current(conn, "widget-a", 1)?;
            ensure_current(conn, "widget-b", 2)?;
            assert_eq!(get_version(conn, "widget-a")?, Some(1));
            assert_eq!(get_version(conn, "widget-b")?, Some(2));
            Ok(())
        });
    }
}
