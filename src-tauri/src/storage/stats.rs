//! Tabela `emulator_stats`: contadores acumulados por emulador desde a
//! instalação (uploads, downloads, bytes, conflitos) e carimbos do último
//! sync/scan. Alimentada pelo `SyncEngine` a cada operação concluída;
//! exibida no card do emulador.

use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::Serialize;

use crate::error::AppResult;

/// Estatísticas acumuladas de um emulador. (→ ipc.ts)
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EmulatorStats {
    pub emulator: String,
    pub total_uploads: i64,
    pub total_downloads: i64,
    pub total_bytes_up: i64,
    pub total_bytes_down: i64,
    pub total_conflicts: i64,
    /// Fim do último sync que tocou este emulador. `None` = nunca sincronizou.
    pub last_sync_at_ms: Option<i64>,
    /// Último arquivo transferido (rel_path).
    pub last_file: Option<String>,
    /// Início do último scan (varredura local) deste emulador.
    pub last_scan_at_ms: Option<i64>,
}

const COLS: &str = "emulator, total_uploads, total_downloads, total_bytes_up, \
                    total_bytes_down, total_conflicts, last_sync_at_ms, last_file, \
                    last_scan_at_ms";

fn from_row(row: &Row) -> rusqlite::Result<EmulatorStats> {
    Ok(EmulatorStats {
        emulator: row.get(0)?,
        total_uploads: row.get(1)?,
        total_downloads: row.get(2)?,
        total_bytes_up: row.get(3)?,
        total_bytes_down: row.get(4)?,
        total_conflicts: row.get(5)?,
        last_sync_at_ms: row.get(6)?,
        last_file: row.get(7)?,
        last_scan_at_ms: row.get(8)?,
    })
}

/// Garante a linha do emulador (contadores zerados).
fn ensure_row(conn: &Connection, emulator: &str) -> AppResult<()> {
    conn.execute(
        "INSERT OR IGNORE INTO emulator_stats (emulator) VALUES (?1)",
        params![emulator],
    )?;
    Ok(())
}

pub fn record_upload(conn: &Connection, emulator: &str, bytes: i64, file: &str) -> AppResult<()> {
    ensure_row(conn, emulator)?;
    conn.execute(
        "UPDATE emulator_stats SET total_uploads = total_uploads + 1, \
         total_bytes_up = total_bytes_up + ?2, last_file = ?3 WHERE emulator = ?1",
        params![emulator, bytes.max(0), file],
    )?;
    Ok(())
}

pub fn record_download(conn: &Connection, emulator: &str, bytes: i64, file: &str) -> AppResult<()> {
    ensure_row(conn, emulator)?;
    conn.execute(
        "UPDATE emulator_stats SET total_downloads = total_downloads + 1, \
         total_bytes_down = total_bytes_down + ?2, last_file = ?3 WHERE emulator = ?1",
        params![emulator, bytes.max(0), file],
    )?;
    Ok(())
}

pub fn record_conflict(conn: &Connection, emulator: &str) -> AppResult<()> {
    ensure_row(conn, emulator)?;
    conn.execute(
        "UPDATE emulator_stats SET total_conflicts = total_conflicts + 1 WHERE emulator = ?1",
        params![emulator],
    )?;
    Ok(())
}

/// Marca o fim de um sync que tocou este emulador.
pub fn touch_last_sync(conn: &Connection, emulator: &str, at_ms: i64) -> AppResult<()> {
    ensure_row(conn, emulator)?;
    conn.execute(
        "UPDATE emulator_stats SET last_sync_at_ms = ?2 WHERE emulator = ?1",
        params![emulator, at_ms],
    )?;
    Ok(())
}

/// Marca o início do scan (varredura local) deste emulador.
pub fn touch_last_scan(conn: &Connection, emulator: &str, at_ms: i64) -> AppResult<()> {
    ensure_row(conn, emulator)?;
    conn.execute(
        "UPDATE emulator_stats SET last_scan_at_ms = ?2 WHERE emulator = ?1",
        params![emulator, at_ms],
    )?;
    Ok(())
}

/// Estatísticas de um emulador; `None` se nunca houve atividade.
pub fn get(conn: &Connection, emulator: &str) -> AppResult<Option<EmulatorStats>> {
    let stats = conn
        .query_row(
            &format!("SELECT {COLS} FROM emulator_stats WHERE emulator = ?1"),
            params![emulator],
            from_row,
        )
        .optional()?;
    Ok(stats)
}

/// Estatísticas de todos os emuladores com atividade registrada.
pub fn list_all(conn: &Connection) -> AppResult<Vec<EmulatorStats>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLS} FROM emulator_stats ORDER BY emulator"
    ))?;
    let stats = stmt
        .query_map([], from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(stats)
}

/// Remove as estatísticas de um emulador (ao removê-lo do app).
pub fn remove_for_emulator(conn: &Connection, emulator: &str) -> AppResult<()> {
    conn.execute(
        "DELETE FROM emulator_stats WHERE emulator = ?1",
        params![emulator],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::db::Db;

    #[test]
    fn contadores_acumulam_por_emulador() {
        let db = Db::open_in_memory().unwrap();
        db.with_sync(|conn| {
            record_upload(conn, "PPSSPP", 1024, "a.bin")?;
            record_upload(conn, "PPSSPP", 2048, "b.bin")?;
            record_download(conn, "PPSSPP", 512, "c.bin")?;
            record_conflict(conn, "PPSSPP")?;
            record_upload(conn, "PCSX2", 10, "x.ps2")?;

            let s = get(conn, "PPSSPP")?.expect("linha criada");
            assert_eq!(s.total_uploads, 2);
            assert_eq!(s.total_downloads, 1);
            assert_eq!(s.total_bytes_up, 3072);
            assert_eq!(s.total_bytes_down, 512);
            assert_eq!(s.total_conflicts, 1);
            assert_eq!(s.last_file.as_deref(), Some("c.bin"));
            assert_eq!(list_all(conn)?.len(), 2);
            Ok(())
        });
    }

    #[test]
    fn carimbos_de_sync_e_scan_sao_gravados() {
        let db = Db::open_in_memory().unwrap();
        db.with_sync(|conn| {
            touch_last_scan(conn, "PPSSPP", 1_700_000_000_000)?;
            touch_last_sync(conn, "PPSSPP", 1_700_000_001_000)?;
            let s = get(conn, "PPSSPP")?.unwrap();
            assert_eq!(s.last_scan_at_ms, Some(1_700_000_000_000));
            assert_eq!(s.last_sync_at_ms, Some(1_700_000_001_000));
            Ok(())
        });
    }

    #[test]
    fn get_sem_atividade_retorna_none_e_remove_limpa() {
        let db = Db::open_in_memory().unwrap();
        db.with_sync(|conn| {
            assert_eq!(get(conn, "PPSSPP")?, None);
            record_upload(conn, "PPSSPP", 1, "a")?;
            remove_for_emulator(conn, "PPSSPP")?;
            assert_eq!(get(conn, "PPSSPP")?, None);
            Ok(())
        });
    }

    #[test]
    fn stats_serializam_em_camel_case() {
        let db = Db::open_in_memory().unwrap();
        db.with_sync(|conn| {
            record_upload(conn, "PPSSPP", 7, "a.bin")?;
            let json = serde_json::to_value(get(conn, "PPSSPP")?.unwrap()).unwrap();
            assert_eq!(json["totalUploads"], 1);
            assert_eq!(json["totalBytesUp"], 7);
            assert_eq!(json["lastFile"], "a.bin");
            assert!(json["lastSyncAtMs"].is_null());
            Ok(())
        });
    }
}
