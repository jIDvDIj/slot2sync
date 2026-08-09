//! Fila de operações pendentes (resiliência offline).
//!
//! Quando uma transferência falha por rede ou arquivo em uso, a intenção é
//! registrada aqui e sobrevive a reinícios do app. O diff do próximo sync
//! re-detecta a diferença e refaz a operação; ao sincronizar o arquivo com
//! sucesso, `resolve` limpa as pendências dele.

use std::collections::HashSet;

use rusqlite::{params, Connection};
use serde::Serialize;

use crate::error::AppResult;
use crate::sync::SyncCategory;

/// Base do backoff exponencial (30 s) e teto (1 h):
/// `next_retry = agora + min(2^attempts × 30 s, 1 h)`.
const RETRY_BASE_MS: i64 = 30_000;
const RETRY_MAX_MS: i64 = 3_600_000;

/// Depois desta quantidade de tentativas a pendência vira "morta"
/// (`next_retry_at_ms = NULL`) e só volta pela ação "tentar novamente" da UI.
const MAX_ATTEMPTS: u32 = 10;

/// Delay de backoff para uma contagem de tentativas (saturado no teto).
fn backoff_ms(attempts: u32) -> i64 {
    let factor = 2i64.checked_pow(attempts.min(30)).unwrap_or(i64::MAX);
    factor.saturating_mul(RETRY_BASE_MS).min(RETRY_MAX_MS)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpDirection {
    Upload,
    Download,
}

impl OpDirection {
    pub fn as_str(self) -> &'static str {
        match self {
            OpDirection::Upload => "upload",
            OpDirection::Download => "download",
        }
    }
}

/// Pendência exposta à UI (fila offline visível). (→ ipc.ts)
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingOp {
    pub emulator: String,
    pub category: SyncCategory,
    pub rel_path: String,
    /// "upload" | "download" (mesmos valores de [`OpDirection::as_str`]).
    pub direction: String,
    pub enqueued_at_ms: i64,
    pub attempts: u32,
    pub last_error: Option<String>,
    /// A partir de quando a pendência pode ser retentada (backoff exponencial).
    /// `None` = morta após esgotar as tentativas; exige "tentar novamente".
    pub next_retry_at_ms: Option<i64>,
}

impl PendingOp {
    /// Esgotou as tentativas automáticas — só a ação da UI reativa.
    #[cfg(test)]
    pub fn is_dead(&self) -> bool {
        self.next_retry_at_ms.is_none()
    }
}

/// Todas as pendências, mais antigas primeiro — a UI agrupa por emulador.
pub fn list_all(conn: &Connection) -> AppResult<Vec<PendingOp>> {
    let mut stmt = conn.prepare(
        "SELECT emulator, category, rel_path, direction, enqueued_at_ms, attempts, last_error, \
         next_retry_at_ms FROM pending_ops ORDER BY enqueued_at_ms ASC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, i64>(4)?,
            row.get::<_, u32>(5)?,
            row.get::<_, Option<String>>(6)?,
            row.get::<_, Option<i64>>(7)?,
        ))
    })?;

    let mut out = Vec::new();
    for row in rows {
        let (
            emulator,
            category,
            rel_path,
            direction,
            enqueued_at_ms,
            attempts,
            last_error,
            next_retry_at_ms,
        ) = row?;
        // Linha com categoria desconhecida (schema futuro?) é ignorada em vez
        // de derrubar a listagem inteira.
        let Some(category) = SyncCategory::parse(&category) else {
            continue;
        };
        out.push(PendingOp {
            emulator,
            category,
            rel_path,
            direction,
            enqueued_at_ms,
            attempts,
            last_error,
            next_retry_at_ms,
        });
    }
    Ok(out)
}

/// Registra (ou reforça, somando tentativa) uma pendência, agendando a próxima
/// retentativa com backoff exponencial. Após [`MAX_ATTEMPTS`] tentativas a
/// pendência vira morta (`next_retry_at_ms = NULL`).
pub fn enqueue(
    conn: &Connection,
    emulator: &str,
    category: SyncCategory,
    rel_path: &str,
    direction: OpDirection,
    error: &str,
) -> AppResult<()> {
    let now = chrono::Utc::now().timestamp_millis();
    conn.execute(
        "INSERT INTO pending_ops (emulator, category, rel_path, direction, enqueued_at_ms, attempts, last_error) \
         VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6) \
         ON CONFLICT (emulator, category, rel_path, direction) \
         DO UPDATE SET attempts = attempts + 1, last_error = excluded.last_error",
        params![
            emulator,
            category.as_str(),
            rel_path,
            direction.as_str(),
            now,
            error,
        ],
    )?;

    let attempts: u32 = conn.query_row(
        "SELECT attempts FROM pending_ops \
         WHERE emulator = ?1 AND category = ?2 AND rel_path = ?3 AND direction = ?4",
        params![emulator, category.as_str(), rel_path, direction.as_str()],
        |row| row.get(0),
    )?;
    let next_retry: Option<i64> = if attempts >= MAX_ATTEMPTS {
        None
    } else {
        Some(now + backoff_ms(attempts))
    };
    conn.execute(
        "UPDATE pending_ops SET next_retry_at_ms = ?5 \
         WHERE emulator = ?1 AND category = ?2 AND rel_path = ?3 AND direction = ?4",
        params![
            emulator,
            category.as_str(),
            rel_path,
            direction.as_str(),
            next_retry,
        ],
    )?;
    Ok(())
}

/// `rel_path`s da categoria que NÃO devem ser retentados neste ciclo: ou o
/// backoff ainda não venceu, ou a pendência está morta. O diff pula esses
/// arquivos e eles contam como `skipped`.
pub fn deferred_rel_paths(
    conn: &Connection,
    emulator: &str,
    category: SyncCategory,
    now_ms: i64,
) -> AppResult<HashSet<String>> {
    let mut stmt = conn.prepare(
        "SELECT rel_path FROM pending_ops \
         WHERE emulator = ?1 AND category = ?2 \
         AND (next_retry_at_ms IS NULL OR next_retry_at_ms > ?3)",
    )?;
    let rows = stmt.query_map(params![emulator, category.as_str(), now_ms], |row| {
        row.get::<_, String>(0)
    })?;
    let mut out = HashSet::new();
    for row in rows {
        out.insert(row?);
    }
    Ok(out)
}

/// Ação "tentar novamente" da UI: zera as tentativas e libera a retentativa
/// imediata (inclusive de pendências mortas).
pub fn retry_now(
    conn: &Connection,
    emulator: &str,
    category: SyncCategory,
    rel_path: &str,
) -> AppResult<()> {
    conn.execute(
        "UPDATE pending_ops SET attempts = 0, next_retry_at_ms = 0 \
         WHERE emulator = ?1 AND category = ?2 AND rel_path = ?3",
        params![emulator, category.as_str(), rel_path],
    )?;
    Ok(())
}

/// Remove as pendências de um arquivo após sync bem-sucedido.
pub fn resolve(
    conn: &Connection,
    emulator: &str,
    category: SyncCategory,
    rel_path: &str,
) -> AppResult<()> {
    conn.execute(
        "DELETE FROM pending_ops WHERE emulator = ?1 AND category = ?2 AND rel_path = ?3",
        params![emulator, category.as_str(), rel_path],
    )?;
    Ok(())
}

pub fn remove_for_emulator(conn: &Connection, emulator: &str) -> AppResult<()> {
    conn.execute(
        "DELETE FROM pending_ops WHERE emulator = ?1",
        params![emulator],
    )?;
    Ok(())
}

#[allow(dead_code)]
pub fn count(conn: &Connection) -> AppResult<i64> {
    let count = conn.query_row("SELECT COUNT(*) FROM pending_ops", [], |row| row.get(0))?;
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::db::Db;

    #[test]
    fn enqueue_deduplica_e_acumula_tentativas() {
        let db = Db::open_in_memory().unwrap();
        db.with_sync(|conn| {
            enqueue(
                conn,
                "PPSSPP",
                SyncCategory::Saves,
                "a.bin",
                OpDirection::Upload,
                "rede",
            )?;
            enqueue(
                conn,
                "PPSSPP",
                SyncCategory::Saves,
                "a.bin",
                OpDirection::Upload,
                "rede 2",
            )?;
            assert_eq!(count(conn)?, 1);

            let attempts: i64 =
                conn.query_row("SELECT attempts FROM pending_ops", [], |r| r.get(0))?;
            assert_eq!(attempts, 2);
            Ok(())
        });
    }

    #[test]
    fn resolve_limpa_pendencias_do_arquivo() {
        let db = Db::open_in_memory().unwrap();
        db.with_sync(|conn| {
            enqueue(
                conn,
                "PPSSPP",
                SyncCategory::Saves,
                "a.bin",
                OpDirection::Upload,
                "x",
            )?;
            enqueue(
                conn,
                "PPSSPP",
                SyncCategory::Saves,
                "b.bin",
                OpDirection::Download,
                "x",
            )?;

            resolve(conn, "PPSSPP", SyncCategory::Saves, "a.bin")?;

            assert_eq!(count(conn)?, 1);
            Ok(())
        });
    }

    #[test]
    fn list_all_expoe_direcao_tentativas_e_erro() {
        let db = Db::open_in_memory().unwrap();
        db.with_sync(|conn| {
            enqueue(
                conn,
                "PPSSPP",
                SyncCategory::Saves,
                "a.bin",
                OpDirection::Upload,
                "rede caiu",
            )?;
            enqueue(
                conn,
                "PPSSPP",
                SyncCategory::Saves,
                "a.bin",
                OpDirection::Upload,
                "arquivo em uso",
            )?;
            enqueue(
                conn,
                "PCSX2",
                SyncCategory::Config,
                "b.ini",
                OpDirection::Download,
                "x",
            )?;

            let ops = list_all(conn)?;
            assert_eq!(ops.len(), 2);
            let a = ops.iter().find(|o| o.rel_path == "a.bin").unwrap();
            assert_eq!(a.emulator, "PPSSPP");
            assert_eq!(a.category, SyncCategory::Saves);
            assert_eq!(a.direction, "upload");
            assert_eq!(a.attempts, 2);
            assert_eq!(a.last_error.as_deref(), Some("arquivo em uso"));
            Ok(())
        });
    }

    #[test]
    fn pending_op_serializa_em_camel_case() {
        let op = PendingOp {
            emulator: "PPSSPP".into(),
            category: SyncCategory::Savestates,
            rel_path: "GAME01/state0.bin".into(),
            direction: "download".into(),
            enqueued_at_ms: 1_700_000_000_000,
            attempts: 3,
            last_error: Some("rede".into()),
            next_retry_at_ms: Some(1_700_000_060_000),
        };
        let json = serde_json::to_value(&op).unwrap();
        assert_eq!(json["emulator"], "PPSSPP");
        assert_eq!(json["category"], "savestates");
        assert_eq!(json["relPath"], "GAME01/state0.bin");
        assert_eq!(json["direction"], "download");
        assert_eq!(json["enqueuedAtMs"], 1_700_000_000_000i64);
        assert_eq!(json["attempts"], 3);
        assert_eq!(json["lastError"], "rede");
        assert_eq!(json["nextRetryAtMs"], 1_700_000_060_000i64);
    }

    #[test]
    fn backoff_dobra_por_tentativa_e_satura_no_teto() {
        assert_eq!(backoff_ms(1), 60_000);
        assert_eq!(backoff_ms(2), 120_000);
        assert_eq!(backoff_ms(6), 1_920_000);
        // 2^7 × 30 s = 3 840 s > 1 h → teto.
        assert_eq!(backoff_ms(7), RETRY_MAX_MS);
        assert_eq!(backoff_ms(30), RETRY_MAX_MS);
    }

    #[test]
    fn enqueue_agenda_retentativa_no_futuro() {
        let db = Db::open_in_memory().unwrap();
        db.with_sync(|conn| {
            let before = chrono::Utc::now().timestamp_millis();
            enqueue(
                conn,
                "PPSSPP",
                SyncCategory::Saves,
                "a.bin",
                OpDirection::Upload,
                "rede",
            )?;
            let op = &list_all(conn)?[0];
            let next = op.next_retry_at_ms.expect("primeira falha não é morta");
            assert!(next >= before + backoff_ms(1));
            assert!(!op.is_dead());
            Ok(())
        });
    }

    #[test]
    fn pendencia_morre_apos_dez_tentativas_e_retry_now_ressuscita() {
        let db = Db::open_in_memory().unwrap();
        db.with_sync(|conn| {
            for _ in 0..MAX_ATTEMPTS {
                enqueue(
                    conn,
                    "PPSSPP",
                    SyncCategory::Saves,
                    "a.bin",
                    OpDirection::Upload,
                    "rede",
                )?;
            }
            let op = &list_all(conn)?[0];
            assert_eq!(op.attempts, MAX_ATTEMPTS);
            assert!(op.is_dead());

            retry_now(conn, "PPSSPP", SyncCategory::Saves, "a.bin")?;
            let op = &list_all(conn)?[0];
            assert_eq!(op.attempts, 0);
            assert_eq!(op.next_retry_at_ms, Some(0));
            Ok(())
        });
    }

    #[test]
    fn deferred_rel_paths_inclui_backoff_pendente_e_mortas() {
        let db = Db::open_in_memory().unwrap();
        db.with_sync(|conn| {
            // "a.bin": backoff no futuro. "b.bin": morta. "c.bin": liberada.
            enqueue(
                conn,
                "PPSSPP",
                SyncCategory::Saves,
                "a.bin",
                OpDirection::Upload,
                "x",
            )?;
            for _ in 0..MAX_ATTEMPTS {
                enqueue(
                    conn,
                    "PPSSPP",
                    SyncCategory::Saves,
                    "b.bin",
                    OpDirection::Upload,
                    "x",
                )?;
            }
            enqueue(
                conn,
                "PPSSPP",
                SyncCategory::Saves,
                "c.bin",
                OpDirection::Download,
                "x",
            )?;
            retry_now(conn, "PPSSPP", SyncCategory::Saves, "c.bin")?;

            let now = chrono::Utc::now().timestamp_millis();
            let deferred = deferred_rel_paths(conn, "PPSSPP", SyncCategory::Saves, now)?;
            assert!(deferred.contains("a.bin"));
            assert!(deferred.contains("b.bin"));
            assert!(!deferred.contains("c.bin"));
            Ok(())
        });
    }

    #[test]
    fn remove_for_emulator_limpa_somente_o_emulador() {
        let db = Db::open_in_memory().unwrap();
        db.with_sync(|conn| {
            enqueue(
                conn,
                "PPSSPP",
                SyncCategory::Saves,
                "a.bin",
                OpDirection::Upload,
                "x",
            )?;
            enqueue(
                conn,
                "PCSX2",
                SyncCategory::Config,
                "b.ini",
                OpDirection::Upload,
                "x",
            )?;

            remove_for_emulator(conn, "PPSSPP")?;

            assert_eq!(count(conn)?, 1);
            Ok(())
        });
    }
}
