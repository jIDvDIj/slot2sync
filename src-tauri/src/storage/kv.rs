//! Tabela chave→valor genérica para metadados internos do app.
//!
//! Distinta de `app_settings` (preferências do usuário, visíveis/editáveis na
//! UI): esta tabela guarda estado operacional interno — carimbos de
//! manutenção, versionamento lógico de schema (`storage::schema_version`) — que
//! nunca aparece na tela de configurações.

use rusqlite::{params, Connection};

use crate::error::AppResult;

pub fn get(conn: &Connection, key: &str) -> AppResult<Option<String>> {
    let value = conn
        .query_row(
            "SELECT value FROM internal_kv WHERE key = ?1",
            params![key],
            |row| row.get::<_, String>(0),
        )
        .ok();
    Ok(value)
}

pub fn set(conn: &Connection, key: &str, value: &str) -> AppResult<()> {
    conn.execute(
        "INSERT INTO internal_kv (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::db::Db;

    #[test]
    fn get_sem_chave_retorna_none() {
        let db = Db::open_in_memory().unwrap();
        db.with_sync(|conn| {
            assert_eq!(get(conn, "missing")?, None);
            Ok(())
        });
    }

    #[test]
    fn set_depois_get_faz_roundtrip_e_sobrescreve() {
        let db = Db::open_in_memory().unwrap();
        db.with_sync(|conn| {
            set(conn, "last_maintenance_at_ms", "1000")?;
            assert_eq!(
                get(conn, "last_maintenance_at_ms")?,
                Some("1000".to_string())
            );

            set(conn, "last_maintenance_at_ms", "2000")?;
            assert_eq!(
                get(conn, "last_maintenance_at_ms")?,
                Some("2000".to_string())
            );
            Ok(())
        });
    }
}
