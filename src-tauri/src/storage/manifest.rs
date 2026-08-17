//! Tabela `sync_manifest`: estado conhecido de cada arquivo no último sync
//! bem-sucedido (mtime local, mtime remoto e ID remoto). É a referência
//! do diff — permite distinguir "mudou de um lado" de "mudou dos dois".

use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::Serialize;

use crate::error::AppResult;
use crate::sync::SyncCategory;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestEntry {
    pub emulator: String,
    pub category: SyncCategory,
    /// Caminho relativo à pasta da categoria, sempre com separador `/`.
    pub rel_path: String,
    pub remote_file_id: Option<String>,
    pub local_mtime_ms: Option<i64>,
    pub remote_mtime_ms: Option<i64>,
    pub size_bytes: Option<i64>,
    pub last_synced_at_ms: i64,
    /// SHA-256 (hex) do conteúdo no último sync. `None` em entradas gravadas
    /// antes da migração v7 — o hash passa a existir no próximo sync do arquivo.
    pub file_hash: Option<String>,
    /// Bitmask best-effort (`FLAG_*`) — índice secundário para consulta rápida,
    /// não a fonte de verdade (essa continua em `sync_conflicts`/`pending_ops`).
    /// Zerado a cada sync bem-sucedido do arquivo.
    pub flags: i64,
    /// Upload abortado por `PermissionDenied`/`WouldBlock` (emulador segura o
    /// arquivo aberto). Distingue trava transitória de erro permanente: o
    /// watcher de filesystem dispara um resync quando o arquivo for liberado,
    /// em vez de entrar no backoff exponencial de `pending_ops`. Zerado no
    /// próximo sync bem-sucedido do arquivo.
    pub inaccessible: bool,
    /// Remanescente sub-milissegundo (ns) de `local_mtime_ms`, quando
    /// disponível. Ver `sync::diff::LocalFile::mtime_ns`.
    pub mtime_ns: i64,
}

/// Conflito pendente para este arquivo (ver `storage::conflicts`).
pub const FLAG_CONFLICT: i64 = 1;
/// Pendência na fila offline para este arquivo (ver `storage::queue`).
pub const FLAG_PENDING: i64 = 2;
/// Reservada para uso futuro: arquivo fora do sync por padrão de exclusão.
#[allow(dead_code)]
pub const FLAG_IGNORED: i64 = 4;
/// Reservada para uso futuro: última verificação encontrou hash divergente
/// do esperado.
#[allow(dead_code)]
pub const FLAG_HASH_MISMATCH: i64 = 8;

const COLS: &str = "emulator, category, rel_path, remote_file_id, local_mtime_ms, \
                    remote_mtime_ms, size_bytes, last_synced_at_ms, file_hash, flags, \
                    inaccessible, mtime_ns";

fn from_row(row: &Row) -> rusqlite::Result<ManifestEntry> {
    let category_str: String = row.get(1)?;
    let category = SyncCategory::parse(&category_str).ok_or_else(|| {
        rusqlite::Error::FromSqlConversionFailure(
            1,
            rusqlite::types::Type::Text,
            format!("categoria inválida no manifest: {category_str}").into(),
        )
    })?;
    Ok(ManifestEntry {
        emulator: row.get(0)?,
        category,
        rel_path: row.get(2)?,
        remote_file_id: row.get(3)?,
        local_mtime_ms: row.get(4)?,
        remote_mtime_ms: row.get(5)?,
        size_bytes: row.get(6)?,
        last_synced_at_ms: row.get(7)?,
        file_hash: row.get(8)?,
        flags: row.get(9)?,
        inaccessible: row.get(10)?,
        mtime_ns: row.get(11)?,
    })
}

pub fn upsert(conn: &Connection, entry: &ManifestEntry) -> AppResult<()> {
    conn.prepare_cached(
        "INSERT OR REPLACE INTO sync_manifest (emulator, category, rel_path, remote_file_id, \
         local_mtime_ms, remote_mtime_ms, size_bytes, last_synced_at_ms, file_hash, flags, \
         inaccessible, mtime_ns) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
    )?
    .execute(params![
        entry.emulator,
        entry.category.as_str(),
        entry.rel_path,
        entry.remote_file_id,
        entry.local_mtime_ms,
        entry.remote_mtime_ms,
        entry.size_bytes,
        entry.last_synced_at_ms,
        entry.file_hash,
        entry.flags,
        entry.inaccessible,
        entry.mtime_ns,
    ])?;
    Ok(())
}

/// Marca o arquivo como inacessível (trava do emulador durante upload), best-
/// effort: cria uma linha mínima no manifest se ainda não existir uma, em vez
/// de fazer no-op como `set_flag` — o primeiro upload de um arquivo pode
/// muito bem topar com o arquivo já travado.
pub fn mark_inaccessible(
    conn: &Connection,
    emulator: &str,
    category: SyncCategory,
    rel_path: &str,
) -> AppResult<()> {
    conn.prepare_cached(
        "INSERT INTO sync_manifest (emulator, category, rel_path, last_synced_at_ms, inaccessible) \
         VALUES (?1, ?2, ?3, 0, 1) \
         ON CONFLICT (emulator, category, rel_path) DO UPDATE SET inaccessible = 1",
    )?
    .execute(params![emulator, category.as_str(), rel_path])?;
    Ok(())
}

/// Liga os bits de `flags` na entrada, se ela existir. Best-effort: não-op
/// silencioso quando não há linha do manifest para o arquivo ainda (ex.:
/// conflito detectado num arquivo nunca sincronizado antes).
pub fn set_flag(
    conn: &Connection,
    emulator: &str,
    category: SyncCategory,
    rel_path: &str,
    flag: i64,
) -> AppResult<()> {
    conn.prepare_cached(
        "UPDATE sync_manifest SET flags = flags | ?4 \
         WHERE emulator = ?1 AND category = ?2 AND rel_path = ?3",
    )?
    .execute(params![emulator, category.as_str(), rel_path, flag])?;
    Ok(())
}

/// Desliga os bits de `flags` na entrada, se ela existir. Hoje as limpezas
/// acontecem via `upsert`/`upsert_batch` reescrevendo `flags: 0` numa
/// sincronização bem-sucedida; esta função fica disponível para limpar uma
/// flag isolada sem tocar no resto da entrada.
#[allow(dead_code)]
pub fn clear_flag(
    conn: &Connection,
    emulator: &str,
    category: SyncCategory,
    rel_path: &str,
    flag: i64,
) -> AppResult<()> {
    conn.prepare_cached(
        "UPDATE sync_manifest SET flags = flags & ~?4 \
         WHERE emulator = ?1 AND category = ?2 AND rel_path = ?3",
    )?
    .execute(params![emulator, category.as_str(), rel_path, flag])?;
    Ok(())
}

/// Grava várias entradas numa única transação — usado ao final de um
/// `sync_target` por categoria, no lugar de um `upsert` por arquivo transferido.
pub fn upsert_batch(conn: &Connection, entries: &[ManifestEntry]) -> AppResult<()> {
    if entries.is_empty() {
        return Ok(());
    }
    let tx = conn.unchecked_transaction()?;
    for entry in entries {
        upsert(&tx, entry)?;
    }
    tx.commit()?;
    Ok(())
}

#[allow(dead_code)]
pub fn get(
    conn: &Connection,
    emulator: &str,
    category: SyncCategory,
    rel_path: &str,
) -> AppResult<Option<ManifestEntry>> {
    let entry = conn
        .prepare_cached(&format!(
            "SELECT {COLS} FROM sync_manifest \
             WHERE emulator = ?1 AND category = ?2 AND rel_path = ?3"
        ))?
        .query_row(params![emulator, category.as_str(), rel_path], from_row)
        .optional()?;
    Ok(entry)
}

pub fn list_for_category(
    conn: &Connection,
    emulator: &str,
    category: SyncCategory,
) -> AppResult<Vec<ManifestEntry>> {
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT {COLS} FROM sync_manifest \
         WHERE emulator = ?1 AND category = ?2 ORDER BY rel_path"
    ))?;
    let entries = stmt
        .query_map(params![emulator, category.as_str()], from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(entries)
}

/// Todas as entradas — base do snapshot `sync_manifest.json` publicado no provedor remoto.
pub fn list_all(conn: &Connection) -> AppResult<Vec<ManifestEntry>> {
    let mut stmt = conn.prepare_cached(&format!(
        "SELECT {COLS} FROM sync_manifest ORDER BY emulator, category, rel_path"
    ))?;
    let entries = stmt
        .query_map([], from_row)?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(entries)
}

/// Remove a entrada de um único arquivo (usado na detecção de renomeação: a
/// âncora antiga sai, a nova entra com o nome novo).
pub fn remove_entry(
    conn: &Connection,
    emulator: &str,
    category: SyncCategory,
    rel_path: &str,
) -> AppResult<()> {
    conn.prepare_cached(
        "DELETE FROM sync_manifest WHERE emulator = ?1 AND category = ?2 AND rel_path = ?3",
    )?
    .execute(params![emulator, category.as_str(), rel_path])?;
    Ok(())
}

pub fn remove_for_emulator(conn: &Connection, emulator: &str) -> AppResult<()> {
    conn.prepare_cached("DELETE FROM sync_manifest WHERE emulator = ?1")?
        .execute(params![emulator])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::db::Db;

    fn sample_entry() -> ManifestEntry {
        ManifestEntry {
            emulator: "PPSSPP".into(),
            category: SyncCategory::Saves,
            rel_path: "GAME123/SAVE.bin".into(),
            remote_file_id: Some("drive-id-1".into()),
            local_mtime_ms: Some(1_700_000_000_000),
            remote_mtime_ms: Some(1_700_000_000_500),
            size_bytes: Some(4096),
            last_synced_at_ms: 1_700_000_001_000,
            file_hash: Some("ab".repeat(32)),
            flags: 0,
            inaccessible: false,
            mtime_ns: 0,
        }
    }

    #[test]
    fn upsert_e_get_fazem_roundtrip_completo() {
        let db = Db::open_in_memory().unwrap();
        db.with_sync(|conn| {
            let entry = sample_entry();
            upsert(conn, &entry)?;

            let loaded = get(conn, "PPSSPP", SyncCategory::Saves, "GAME123/SAVE.bin")?
                .expect("entrada deveria existir");
            assert_eq!(loaded, entry);
            Ok(())
        });
    }

    #[test]
    fn mtime_ns_sobrevive_ao_roundtrip() {
        let db = Db::open_in_memory().unwrap();
        db.with_sync(|conn| {
            let mut entry = sample_entry();
            entry.mtime_ns = 456_789;
            upsert(conn, &entry)?;

            let loaded = get(conn, "PPSSPP", SyncCategory::Saves, "GAME123/SAVE.bin")?.unwrap();
            assert_eq!(loaded.mtime_ns, 456_789);
            Ok(())
        });
    }

    #[test]
    fn get_retorna_none_para_entrada_inexistente() {
        let db = Db::open_in_memory().unwrap();
        db.with_sync(|conn| {
            assert_eq!(get(conn, "PPSSPP", SyncCategory::Saves, "nada.bin")?, None);
            Ok(())
        });
    }

    #[test]
    fn upsert_substitui_entrada_existente() {
        let db = Db::open_in_memory().unwrap();
        db.with_sync(|conn| {
            let mut entry = sample_entry();
            upsert(conn, &entry)?;

            entry.local_mtime_ms = Some(1_800_000_000_000);
            entry.remote_file_id = None;
            upsert(conn, &entry)?;

            let loaded = get(conn, "PPSSPP", SyncCategory::Saves, "GAME123/SAVE.bin")?.unwrap();
            assert_eq!(loaded.local_mtime_ms, Some(1_800_000_000_000));
            assert_eq!(loaded.remote_file_id, None);
            assert_eq!(list_all(conn)?.len(), 1);
            Ok(())
        });
    }

    #[test]
    fn list_for_category_filtra_emulador_e_categoria() {
        let db = Db::open_in_memory().unwrap();
        db.with_sync(|conn| {
            let mut a = sample_entry();
            upsert(conn, &a)?;
            a.category = SyncCategory::Config;
            a.rel_path = "ppsspp.ini".into();
            upsert(conn, &a)?;
            a.emulator = "PCSX2".into();
            upsert(conn, &a)?;

            let saves = list_for_category(conn, "PPSSPP", SyncCategory::Saves)?;
            assert_eq!(saves.len(), 1);
            assert_eq!(saves[0].rel_path, "GAME123/SAVE.bin");
            assert_eq!(list_all(conn)?.len(), 3);
            Ok(())
        });
    }

    #[test]
    fn remove_for_emulator_apaga_somente_o_emulador() {
        let db = Db::open_in_memory().unwrap();
        db.with_sync(|conn| {
            let mut entry = sample_entry();
            upsert(conn, &entry)?;
            entry.emulator = "PCSX2".into();
            upsert(conn, &entry)?;

            remove_for_emulator(conn, "PPSSPP")?;

            let rest = list_all(conn)?;
            assert_eq!(rest.len(), 1);
            assert_eq!(rest[0].emulator, "PCSX2");
            Ok(())
        });
    }

    #[test]
    fn upsert_batch_grava_todas_as_entradas_numa_transacao() {
        let db = Db::open_in_memory().unwrap();
        db.with_sync(|conn| {
            let mut a = sample_entry();
            a.rel_path = "a.bin".into();
            let mut b = sample_entry();
            b.rel_path = "b.bin".into();

            upsert_batch(conn, &[a, b])?;

            assert_eq!(list_all(conn)?.len(), 2);
            Ok(())
        });
    }

    #[test]
    fn upsert_batch_vazio_e_no_op() {
        let db = Db::open_in_memory().unwrap();
        db.with_sync(|conn| {
            upsert_batch(conn, &[])?;
            assert!(list_all(conn)?.is_empty());
            Ok(())
        });
    }

    #[test]
    fn entry_serializa_em_camel_case_para_o_snapshot() {
        let json = serde_json::to_value(sample_entry()).unwrap();
        assert_eq!(json["relPath"], "GAME123/SAVE.bin");
        assert_eq!(json["category"], "saves");
        assert_eq!(json["remoteFileId"], "drive-id-1");
        assert_eq!(json["lastSyncedAtMs"], 1_700_000_001_000i64);
    }

    #[test]
    fn set_flag_e_clear_flag_ligam_e_desligam_bits_sem_afetar_outros() {
        let db = Db::open_in_memory().unwrap();
        db.with_sync(|conn| {
            let entry = sample_entry();
            upsert(conn, &entry)?;

            set_flag(
                conn,
                "PPSSPP",
                SyncCategory::Saves,
                "GAME123/SAVE.bin",
                FLAG_CONFLICT,
            )?;
            set_flag(
                conn,
                "PPSSPP",
                SyncCategory::Saves,
                "GAME123/SAVE.bin",
                FLAG_PENDING,
            )?;
            let loaded = get(conn, "PPSSPP", SyncCategory::Saves, "GAME123/SAVE.bin")?.unwrap();
            assert_eq!(loaded.flags, FLAG_CONFLICT | FLAG_PENDING);

            clear_flag(
                conn,
                "PPSSPP",
                SyncCategory::Saves,
                "GAME123/SAVE.bin",
                FLAG_CONFLICT,
            )?;
            let loaded = get(conn, "PPSSPP", SyncCategory::Saves, "GAME123/SAVE.bin")?.unwrap();
            assert_eq!(loaded.flags, FLAG_PENDING);
            Ok(())
        });
    }

    #[test]
    fn set_flag_sem_linha_correspondente_e_no_op_silencioso() {
        let db = Db::open_in_memory().unwrap();
        db.with_sync(|conn| {
            set_flag(
                conn,
                "PPSSPP",
                SyncCategory::Saves,
                "nada.bin",
                FLAG_CONFLICT,
            )?;
            assert_eq!(get(conn, "PPSSPP", SyncCategory::Saves, "nada.bin")?, None);
            Ok(())
        });
    }

    #[test]
    fn mark_inaccessible_cria_linha_minima_quando_nao_existe() {
        let db = Db::open_in_memory().unwrap();
        db.with_sync(|conn| {
            mark_inaccessible(conn, "PPSSPP", SyncCategory::Saves, "locked.bin")?;
            let loaded = get(conn, "PPSSPP", SyncCategory::Saves, "locked.bin")?.unwrap();
            assert!(loaded.inaccessible);
            assert_eq!(loaded.remote_file_id, None);
            Ok(())
        });
    }

    #[test]
    fn mark_inaccessible_em_linha_existente_preserva_o_resto_da_entrada() {
        let db = Db::open_in_memory().unwrap();
        db.with_sync(|conn| {
            upsert(conn, &sample_entry())?;

            mark_inaccessible(conn, "PPSSPP", SyncCategory::Saves, "GAME123/SAVE.bin")?;

            let loaded = get(conn, "PPSSPP", SyncCategory::Saves, "GAME123/SAVE.bin")?.unwrap();
            assert!(loaded.inaccessible);
            assert_eq!(loaded.remote_file_id, Some("drive-id-1".into()));
            Ok(())
        });
    }

    #[test]
    fn upsert_zera_inaccessible_ao_sincronizar_com_sucesso() {
        let db = Db::open_in_memory().unwrap();
        db.with_sync(|conn| {
            mark_inaccessible(conn, "PPSSPP", SyncCategory::Saves, "GAME123/SAVE.bin")?;

            upsert(conn, &sample_entry())?;

            let loaded = get(conn, "PPSSPP", SyncCategory::Saves, "GAME123/SAVE.bin")?.unwrap();
            assert!(!loaded.inaccessible);
            Ok(())
        });
    }
}
