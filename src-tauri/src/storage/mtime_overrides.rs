//! Tabela `mtime_overrides`: camada de mtime virtual para filesystems de baixa
//! granularidade temporal (FAT32 e derivados, comuns em cartão SD de handheld
//! e pendrive).
//!
//! O FAT32 arredonda o mtime para múltiplos de 2 segundos. Depois de um
//! download, `set_file_mtime` grava o `modifiedTime` do provedor, mas o valor
//! que volta do disco é outro — o arredondado. Como o manifest ancora o valor
//! lógico, o scan seguinte vê divergência, conclui "o arquivo mudou" e o
//! reenvia sem nenhuma mudança de conteúdo.
//!
//! Cada linha guarda o par `(ondisk_ms, virtual_ms)`: o que o disco de fato
//! registrou e o que o arquivo logicamente representa. Enquanto o mtime lido
//! for exatamente `ondisk_ms`, o diff enxerga `virtual_ms` no lugar. Quando
//! deixar de ser, o arquivo mudou de verdade e a linha é descartada.

use std::collections::HashMap;

use rusqlite::{params, Connection};

use crate::error::AppResult;
use crate::sync::SyncCategory;

/// Par de mtimes de um arquivo: o gravado no disco e o lógico correspondente.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MtimeOverride {
    /// mtime que o filesystem realmente registrou (já arredondado por ele).
    pub ondisk_ms: i64,
    /// mtime lógico do arquivo — o `modifiedTime` que o provedor remoto tem.
    pub virtual_ms: i64,
}

/// Registra (ou substitui) o override de um arquivo. Chamado após um download
/// em que o mtime efetivo no disco divergiu do que foi pedido.
pub fn upsert(
    conn: &Connection,
    emulator: &str,
    category: SyncCategory,
    rel_path: &str,
    value: MtimeOverride,
) -> AppResult<()> {
    conn.prepare_cached(
        "INSERT OR REPLACE INTO mtime_overrides \
         (emulator, category, rel_path, ondisk_ms, virtual_ms) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
    )?
    .execute(params![
        emulator,
        category.as_str(),
        rel_path,
        value.ondisk_ms,
        value.virtual_ms,
    ])?;
    Ok(())
}

/// Todos os overrides de uma categoria, indexados por `rel_path`. Carregado
/// uma vez por categoria no início do sync, não um SELECT por arquivo.
pub fn list_for_category(
    conn: &Connection,
    emulator: &str,
    category: SyncCategory,
) -> AppResult<HashMap<String, MtimeOverride>> {
    let mut stmt = conn.prepare_cached(
        "SELECT rel_path, ondisk_ms, virtual_ms FROM mtime_overrides \
         WHERE emulator = ?1 AND category = ?2",
    )?;
    let rows = stmt.query_map(params![emulator, category.as_str()], |row| {
        Ok((
            row.get::<_, String>(0)?,
            MtimeOverride {
                ondisk_ms: row.get(1)?,
                virtual_ms: row.get(2)?,
            },
        ))
    })?;
    let mut out = HashMap::new();
    for row in rows {
        let (rel_path, value) = row?;
        out.insert(rel_path, value);
    }
    Ok(out)
}

/// Descarta os overrides de `rel_paths` numa única transação. Usado quando o
/// mtime no disco deixou de bater com `ondisk_ms` (o arquivo mudou de
/// verdade) ou quando o arquivo sumiu da varredura.
pub fn remove_batch(
    conn: &Connection,
    emulator: &str,
    category: SyncCategory,
    rel_paths: &[String],
) -> AppResult<()> {
    if rel_paths.is_empty() {
        return Ok(());
    }
    let tx = conn.unchecked_transaction()?;
    {
        let mut stmt = tx.prepare_cached(
            "DELETE FROM mtime_overrides \
             WHERE emulator = ?1 AND category = ?2 AND rel_path = ?3",
        )?;
        for rel_path in rel_paths {
            stmt.execute(params![emulator, category.as_str(), rel_path])?;
        }
    }
    tx.commit()?;
    Ok(())
}

/// Remove todos os overrides de um emulador — chamado quando ele sai da
/// configuração, junto da limpeza do manifest.
pub fn remove_for_emulator(conn: &Connection, emulator: &str) -> AppResult<()> {
    conn.prepare_cached("DELETE FROM mtime_overrides WHERE emulator = ?1")?
        .execute(params![emulator])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::db::Db;

    const T: i64 = 1_700_000_000_000;

    fn value(ondisk_ms: i64, virtual_ms: i64) -> MtimeOverride {
        MtimeOverride {
            ondisk_ms,
            virtual_ms,
        }
    }

    #[test]
    fn upsert_e_list_fazem_roundtrip() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn_blocking(|conn| {
            upsert(
                conn,
                "PPSSPP",
                SyncCategory::Saves,
                "GAME/SAVE.bin",
                value(T, T + 1_500),
            )?;
            let all = list_for_category(conn, "PPSSPP", SyncCategory::Saves)?;
            assert_eq!(all.len(), 1);
            assert_eq!(all["GAME/SAVE.bin"], value(T, T + 1_500));
            Ok(())
        })
        .unwrap();
    }

    /// O mesmo `rel_path` em emuladores diferentes são linhas independentes —
    /// por isso a chave primária inclui emulador e categoria, e não só o
    /// caminho relativo.
    #[test]
    fn mesma_rel_path_em_emuladores_distintos_nao_colide() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn_blocking(|conn| {
            upsert(
                conn,
                "PPSSPP",
                SyncCategory::Saves,
                "SAVE.bin",
                value(T, T + 1),
            )?;
            upsert(
                conn,
                "PCSX2",
                SyncCategory::Saves,
                "SAVE.bin",
                value(T, T + 2),
            )?;
            upsert(
                conn,
                "PPSSPP",
                SyncCategory::Savestates,
                "SAVE.bin",
                value(T, T + 3),
            )?;

            assert_eq!(
                list_for_category(conn, "PPSSPP", SyncCategory::Saves)?["SAVE.bin"].virtual_ms,
                T + 1
            );
            assert_eq!(
                list_for_category(conn, "PCSX2", SyncCategory::Saves)?["SAVE.bin"].virtual_ms,
                T + 2
            );
            assert_eq!(
                list_for_category(conn, "PPSSPP", SyncCategory::Savestates)?["SAVE.bin"].virtual_ms,
                T + 3
            );
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn upsert_no_mesmo_arquivo_substitui_o_par_anterior() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn_blocking(|conn| {
            upsert(
                conn,
                "PPSSPP",
                SyncCategory::Saves,
                "SAVE.bin",
                value(T, T + 1),
            )?;
            upsert(
                conn,
                "PPSSPP",
                SyncCategory::Saves,
                "SAVE.bin",
                value(T + 10, T + 11),
            )?;
            let all = list_for_category(conn, "PPSSPP", SyncCategory::Saves)?;
            assert_eq!(all.len(), 1);
            assert_eq!(all["SAVE.bin"], value(T + 10, T + 11));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn remove_batch_apaga_so_os_caminhos_informados() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn_blocking(|conn| {
            upsert(
                conn,
                "PPSSPP",
                SyncCategory::Saves,
                "a.bin",
                value(T, T + 1),
            )?;
            upsert(
                conn,
                "PPSSPP",
                SyncCategory::Saves,
                "b.bin",
                value(T, T + 1),
            )?;

            remove_batch(conn, "PPSSPP", SyncCategory::Saves, &["a.bin".to_string()])?;

            let all = list_for_category(conn, "PPSSPP", SyncCategory::Saves)?;
            assert_eq!(all.len(), 1);
            assert!(all.contains_key("b.bin"));
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn remove_batch_vazio_e_no_op() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn_blocking(|conn| {
            upsert(
                conn,
                "PPSSPP",
                SyncCategory::Saves,
                "a.bin",
                value(T, T + 1),
            )?;
            remove_batch(conn, "PPSSPP", SyncCategory::Saves, &[])?;
            assert_eq!(
                list_for_category(conn, "PPSSPP", SyncCategory::Saves)?.len(),
                1
            );
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn remove_for_emulator_limpa_todas_as_categorias_dele() {
        let db = Db::open_in_memory().unwrap();
        db.with_conn_blocking(|conn| {
            upsert(
                conn,
                "PPSSPP",
                SyncCategory::Saves,
                "a.bin",
                value(T, T + 1),
            )?;
            upsert(
                conn,
                "PPSSPP",
                SyncCategory::Savestates,
                "b.bin",
                value(T, T + 1),
            )?;
            upsert(conn, "PCSX2", SyncCategory::Saves, "c.bin", value(T, T + 1))?;

            remove_for_emulator(conn, "PPSSPP")?;

            assert!(list_for_category(conn, "PPSSPP", SyncCategory::Saves)?.is_empty());
            assert!(list_for_category(conn, "PPSSPP", SyncCategory::Savestates)?.is_empty());
            assert_eq!(
                list_for_category(conn, "PCSX2", SyncCategory::Saves)?.len(),
                1
            );
            Ok(())
        })
        .unwrap();
    }
}
