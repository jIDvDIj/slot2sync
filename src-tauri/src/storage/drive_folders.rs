//! Cache persistente de IDs de pasta do Drive por caminho lógico
//! (ex.: `"Slot2Sync/PPSSPP/saves"` → `fileId`).
//!
//! O `DriveClient` mantém um `HashMap` em memória do mesmo mapa; esta tabela é o
//! espelho durável. Sem ela, o cache zera a cada reinício e o sync de startup
//! re-resolve toda a cadeia `Slot2Sync/<Emu>/<categoria>/...` via `files.list`
//! (uma chamada de latência pura por segmento).
//!
//! Invalidação: uma entrada cujo ID retornar `notFound` numa operação é
//! descartada (memória + disco) e re-resolvida; no logout/troca de conta o cache
//! inteiro é zerado (IDs são por conta Google).

use std::collections::HashMap;

use rusqlite::{params, Connection};

use crate::error::AppResult;

/// Carrega todo o mapa `cache_key → folder_id` para popular o cache em memória
/// ao construir o `DriveClient`.
pub fn load_all(conn: &Connection) -> AppResult<HashMap<String, String>> {
    let mut stmt = conn.prepare("SELECT cache_key, folder_id FROM drive_folders")?;
    let map = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<HashMap<_, _>>>()?;
    Ok(map)
}

/// Grava (ou substitui) o ID resolvido de um caminho lógico.
pub fn upsert(conn: &Connection, cache_key: &str, folder_id: &str) -> AppResult<()> {
    conn.execute(
        "INSERT OR REPLACE INTO drive_folders (cache_key, folder_id) VALUES (?1, ?2)",
        params![cache_key, folder_id],
    )?;
    Ok(())
}

/// Remove a entrada exata do caminho e todas as suas subpastas (invalidação
/// reativa após `notFound`: a pasta e sua subárvore precisam ser re-resolvidas).
pub fn remove_subtree(conn: &Connection, cache_key: &str) -> AppResult<()> {
    let subpaths = format!("{cache_key}/%");
    conn.execute(
        "DELETE FROM drive_folders WHERE cache_key = ?1 OR cache_key LIKE ?2",
        params![cache_key, subpaths],
    )?;
    Ok(())
}

/// Zera o cache inteiro (logout/troca de conta Google — os IDs são por conta).
pub fn clear(conn: &Connection) -> AppResult<()> {
    conn.execute("DELETE FROM drive_folders", [])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::db::Db;

    #[test]
    fn upsert_load_e_clear_fazem_roundtrip() {
        let db = Db::open_in_memory().unwrap();
        db.with_sync(|conn| {
            upsert(conn, "Slot2Sync", "id-root")?;
            upsert(conn, "Slot2Sync/PPSSPP", "id-emu")?;
            upsert(conn, "Slot2Sync/PPSSPP/saves", "id-cat")?;

            let map = load_all(conn)?;
            assert_eq!(map.len(), 3);
            assert_eq!(map.get("Slot2Sync/PPSSPP/saves").unwrap(), "id-cat");

            clear(conn)?;
            assert!(load_all(conn)?.is_empty());
            Ok(())
        });
    }

    #[test]
    fn remove_subtree_apaga_a_pasta_e_suas_subpastas() {
        let db = Db::open_in_memory().unwrap();
        db.with_sync(|conn| {
            upsert(conn, "Slot2Sync", "id-root")?;
            upsert(conn, "Slot2Sync/PPSSPP", "id-emu")?;
            upsert(conn, "Slot2Sync/PPSSPP/saves", "id-cat")?;
            upsert(conn, "Slot2Sync/PPSSPP/saves/GAME1", "id-sub")?;

            // Invalida a categoria: remove ela e a subpasta, preserva raiz e emulador.
            remove_subtree(conn, "Slot2Sync/PPSSPP/saves")?;

            let map = load_all(conn)?;
            assert_eq!(map.len(), 2);
            assert!(map.contains_key("Slot2Sync"));
            assert!(map.contains_key("Slot2Sync/PPSSPP"));
            assert!(!map.contains_key("Slot2Sync/PPSSPP/saves"));
            assert!(!map.contains_key("Slot2Sync/PPSSPP/saves/GAME1"));
            Ok(())
        });
    }
}
