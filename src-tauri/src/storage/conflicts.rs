//! Tabela `sync_conflicts`: arquivos em que ambos os lados (local e remoto)
//! mudaram desde o último sync. Enquanto houver conflito para um emulador, o
//! sync dele fica bloqueado até o usuário escolher qual versão manter.

use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::Serialize;

use crate::error::AppResult;
use crate::sync::SyncCategory;

/// Um conflito pendente, com os metadados dos dois lados para a UI decidir. (→ ipc.ts)
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Conflict {
    pub emulator: String,
    pub category: SyncCategory,
    /// Caminho relativo à pasta da categoria, sempre com separador `/`.
    pub rel_path: String,
    pub local_mtime_ms: i64,
    pub local_size: i64,
    /// Dispositivo de origem da versão local (este dispositivo, no momento da
    /// detecção). `None` se o nome ainda não foi definido.
    pub local_device: Option<String>,
    pub remote_mtime_ms: i64,
    pub remote_size: i64,
    /// Dispositivo que publicou a versão remota (via metadata do provedor).
    pub remote_device: Option<String>,
    pub remote_file_id: String,
    /// Caminho absoluto local — interno, usado pela resolução.
    pub local_abs_path: String,
    pub detected_at_ms: i64,
    /// Cópia padronizada do lado local
    /// (`<nome>.slot2sync-conflict-<carimbo>-<device><ext>`) para inspeção
    /// manual. `None` quando a cópia não pôde ser criada.
    pub backup_path: Option<String>,
}

const COLS: &str = "emulator, category, rel_path, local_mtime_ms, local_size, local_device, \
                    remote_mtime_ms, remote_size, remote_device, remote_file_id, local_abs_path, \
                    detected_at_ms, backup_path";

fn from_row(row: &Row) -> rusqlite::Result<Conflict> {
    let category_str: String = row.get(1)?;
    let category = SyncCategory::parse(&category_str).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            1,
            rusqlite::types::Type::Text,
            format!("categoria inválida em sync_conflicts: {category_str}").into(),
        )
    })?;
    Ok(Conflict {
        emulator: row.get(0)?,
        category,
        rel_path: row.get(2)?,
        local_mtime_ms: row.get(3)?,
        local_size: row.get(4)?,
        local_device: row.get(5)?,
        remote_mtime_ms: row.get(6)?,
        remote_size: row.get(7)?,
        remote_device: row.get(8)?,
        remote_file_id: row.get(9)?,
        local_abs_path: row.get(10)?,
        detected_at_ms: row.get(11)?,
        backup_path: row.get(12)?,
    })
}

pub fn upsert(conn: &Connection, c: &Conflict) -> AppResult<()> {
    conn.execute(
        "INSERT OR REPLACE INTO sync_conflicts \
         (emulator, category, rel_path, local_mtime_ms, local_size, local_device, \
          remote_mtime_ms, remote_size, remote_device, remote_file_id, local_abs_path, \
          detected_at_ms, backup_path) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        params![
            c.emulator,
            c.category.as_str(),
            c.rel_path,
            c.local_mtime_ms,
            c.local_size,
            c.local_device,
            c.remote_mtime_ms,
            c.remote_size,
            c.remote_device,
            c.remote_file_id,
            c.local_abs_path,
            c.detected_at_ms,
            c.backup_path,
        ],
    )?;
    Ok(())
}

pub fn get(
    conn: &Connection,
    emulator: &str,
    category: SyncCategory,
    rel_path: &str,
) -> AppResult<Option<Conflict>> {
    let conflict = conn
        .query_row(
            &format!(
                "SELECT {COLS} FROM sync_conflicts \
                 WHERE emulator = ?1 AND category = ?2 AND rel_path = ?3"
            ),
            params![emulator, category.as_str(), rel_path],
            from_row,
        )
        .optional()?;
    Ok(conflict)
}

pub fn list_all(conn: &Connection) -> AppResult<Vec<Conflict>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLS} FROM sync_conflicts ORDER BY emulator, category, rel_path"
    ))?;
    let conflicts = stmt
        .query_map([], from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(conflicts)
}

/// Há conflito pendente para este emulador? (gate de bloqueio do sync)
pub fn has_for_emulator(conn: &Connection, emulator: &str) -> AppResult<bool> {
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sync_conflicts WHERE emulator = ?1",
        params![emulator],
        |row| row.get(0),
    )?;
    Ok(count > 0)
}

pub fn remove(
    conn: &Connection,
    emulator: &str,
    category: SyncCategory,
    rel_path: &str,
) -> AppResult<()> {
    conn.execute(
        "DELETE FROM sync_conflicts WHERE emulator = ?1 AND category = ?2 AND rel_path = ?3",
        params![emulator, category.as_str(), rel_path],
    )?;
    Ok(())
}

pub fn remove_for_emulator(conn: &Connection, emulator: &str) -> AppResult<()> {
    conn.execute(
        "DELETE FROM sync_conflicts WHERE emulator = ?1",
        params![emulator],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::db::Db;

    fn sample() -> Conflict {
        Conflict {
            emulator: "PPSSPP".into(),
            category: SyncCategory::Saves,
            rel_path: "GAME01/SAVE.bin".into(),
            local_mtime_ms: 1_700_000_100_000,
            local_size: 2048,
            local_device: Some("PC Gamer".into()),
            remote_mtime_ms: 1_700_000_200_000,
            remote_size: 4096,
            remote_device: Some("Notebook".into()),
            remote_file_id: "drive-id-1".into(),
            local_abs_path: "/tmp/ppsspp/SAVEDATA/GAME01/SAVE.bin".into(),
            detected_at_ms: 1_700_000_300_000,
            backup_path: Some(
                "/backups/PPSSPP/conflicts/saves/GAME01/SAVE.slot2sync-conflict-x.bin".into(),
            ),
        }
    }

    #[test]
    fn upsert_get_e_remove_roundtrip() {
        let db = Db::open_in_memory().unwrap();
        db.with_sync(|conn| {
            let c = sample();
            upsert(conn, &c)?;
            assert_eq!(
                get(conn, "PPSSPP", SyncCategory::Saves, "GAME01/SAVE.bin")?,
                Some(c)
            );
            assert!(has_for_emulator(conn, "PPSSPP")?);

            remove(conn, "PPSSPP", SyncCategory::Saves, "GAME01/SAVE.bin")?;
            assert!(!has_for_emulator(conn, "PPSSPP")?);
            Ok(())
        });
    }

    #[test]
    fn has_for_emulator_isola_por_emulador() {
        let db = Db::open_in_memory().unwrap();
        db.with_sync(|conn| {
            upsert(conn, &sample())?;
            assert!(has_for_emulator(conn, "PPSSPP")?);
            assert!(!has_for_emulator(conn, "PCSX2")?);
            Ok(())
        });
    }

    #[test]
    fn remove_for_emulator_limpa_tudo_do_emulador() {
        let db = Db::open_in_memory().unwrap();
        db.with_sync(|conn| {
            let mut a = sample();
            upsert(conn, &a)?;
            a.rel_path = "GAME02/SAVE.bin".into();
            upsert(conn, &a)?;
            assert_eq!(list_all(conn)?.len(), 2);

            remove_for_emulator(conn, "PPSSPP")?;
            assert!(list_all(conn)?.is_empty());
            Ok(())
        });
    }

    #[test]
    fn conflito_serializa_em_camel_case() {
        let json = serde_json::to_value(sample()).unwrap();
        assert_eq!(json["relPath"], "GAME01/SAVE.bin");
        assert_eq!(json["category"], "saves");
        assert_eq!(json["localDevice"], "PC Gamer");
        assert_eq!(json["remoteDevice"], "Notebook");
        assert_eq!(json["remoteFileId"], "drive-id-1");
        assert!(json["backupPath"]
            .as_str()
            .unwrap()
            .contains("slot2sync-conflict"));
    }
}
