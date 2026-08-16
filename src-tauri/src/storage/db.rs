//! Conexão SQLite e migrações de schema.
//!
//! `rusqlite` é síncrono: a conexão única vive atrás de `Arc<Mutex>` e todo
//! acesso passa por `Db::with`, que executa em `spawn_blocking` para não
//! bloquear o runtime async.

use std::path::Path;
use std::sync::{Arc, Mutex};

use rusqlite::Connection;

use crate::error::{AppError, AppResult};

const SCHEMA_V1: &str = "
CREATE TABLE IF NOT EXISTS sync_manifest (
    emulator          TEXT NOT NULL,
    category          TEXT NOT NULL,
    rel_path          TEXT NOT NULL,
    drive_file_id     TEXT,
    local_mtime_ms    INTEGER,
    drive_mtime_ms    INTEGER,
    size_bytes        INTEGER,
    last_synced_at_ms INTEGER NOT NULL,
    PRIMARY KEY (emulator, category, rel_path)
);

CREATE TABLE IF NOT EXISTS pending_ops (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    emulator       TEXT NOT NULL,
    category       TEXT NOT NULL,
    rel_path       TEXT NOT NULL,
    direction      TEXT NOT NULL,
    enqueued_at_ms INTEGER NOT NULL,
    attempts       INTEGER NOT NULL DEFAULT 0,
    last_error     TEXT,
    UNIQUE (emulator, category, rel_path, direction)
);

CREATE TABLE IF NOT EXISTS emulators (
    name         TEXT PRIMARY KEY,
    root_path    TEXT NOT NULL,
    profile_json TEXT NOT NULL,
    added_at_ms  INTEGER NOT NULL
);
";

/// v2 — configurações globais do usuário (chave→valor). Ver `storage::settings`.
const SCHEMA_V2: &str = "
CREATE TABLE IF NOT EXISTS app_settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
";

/// v3 — categorias de sync habilitadas por emulador (default: todas ativas).
const SCHEMA_V3: &str = "
CREATE TABLE IF NOT EXISTS emulator_settings (
    emulator           TEXT PRIMARY KEY,
    saves_enabled      INTEGER NOT NULL DEFAULT 1,
    savestates_enabled INTEGER NOT NULL DEFAULT 1,
    config_enabled     INTEGER NOT NULL DEFAULT 1
);
";

/// v5 — segredos do app (refresh token, device_id) no mobile, onde o keyring
/// do SO não está disponível. No desktop esta tabela existe mas fica vazia.
const SCHEMA_V5: &str = "
CREATE TABLE IF NOT EXISTS secrets (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
";

/// v4 — conflitos pendentes (ambos os lados mudaram desde o último sync).
/// Enquanto houver linha para um emulador, o sync dele fica bloqueado.
const SCHEMA_V4: &str = "
CREATE TABLE IF NOT EXISTS sync_conflicts (
    emulator       TEXT NOT NULL,
    category       TEXT NOT NULL,
    rel_path       TEXT NOT NULL,
    local_mtime_ms INTEGER NOT NULL,
    local_size     INTEGER NOT NULL,
    local_device   TEXT,
    drive_mtime_ms INTEGER NOT NULL,
    drive_size     INTEGER NOT NULL,
    drive_device   TEXT,
    drive_file_id  TEXT NOT NULL,
    local_abs_path TEXT NOT NULL,
    detected_at_ms INTEGER NOT NULL,
    PRIMARY KEY (emulator, category, rel_path)
);
";

/// v6 — cache persistente de IDs de pasta do Drive por caminho lógico
/// (ex.: "Slot2Sync/PPSSPP/saves" → fileId). Sobrevive a reinícios para que o
/// sync de startup não re-resolva toda a cadeia de pastas a cada boot.
/// Invalidada reativamente em `notFound` e zerada no logout.
const SCHEMA_V6: &str = "
CREATE TABLE IF NOT EXISTS drive_folders (
    cache_key TEXT PRIMARY KEY,
    folder_id TEXT NOT NULL
);
";

/// v7 — hash SHA-256 do conteúdo no último sync. Pré-filtro de mtime: só é
/// recalculado quando o mtime diverge; hash igual = conteúdo intacto (emulador
/// tocou o mtime sem alterar o save) e o upload é dispensado.
const SCHEMA_V7: &str = "
ALTER TABLE sync_manifest ADD COLUMN file_hash TEXT;
";

/// v8 — backoff exponencial na fila offline. `next_retry_at_ms` diz a partir de
/// quando a pendência pode ser retentada (0 = imediatamente); `NULL` marca a
/// pendência como morta após esgotar as tentativas — só o usuário reativa.
const SCHEMA_V8: &str = "
ALTER TABLE pending_ops ADD COLUMN next_retry_at_ms INTEGER DEFAULT 0;
";

/// v9 — caminho da cópia padronizada do lado local do conflito
/// (`<nome>.slot2sync-conflict-<carimbo>-<device>.<ext>`), para o usuário
/// inspecionar os dois lados antes de decidir. `NULL` quando a cópia falhou.
const SCHEMA_V9: &str = "
ALTER TABLE sync_conflicts ADD COLUMN backup_path TEXT;
";

/// v10 — estatísticas acumuladas por emulador (contadores desde a instalação
/// e carimbos do último sync/scan). Ver `storage::stats`.
const SCHEMA_V10: &str = "
CREATE TABLE IF NOT EXISTS emulator_stats (
    emulator         TEXT PRIMARY KEY,
    total_uploads    INTEGER NOT NULL DEFAULT 0,
    total_downloads  INTEGER NOT NULL DEFAULT 0,
    total_bytes_up   INTEGER NOT NULL DEFAULT 0,
    total_bytes_down INTEGER NOT NULL DEFAULT 0,
    total_conflicts  INTEGER NOT NULL DEFAULT 0,
    last_sync_at_ms  INTEGER,
    last_file        TEXT,
    last_scan_at_ms  INTEGER
);
";

/// v11 — generaliza as colunas antes específicas do Drive para o suporte a
/// múltiplos provedores de storage remoto (ver `remote::RemoteProvider`).
const SCHEMA_V11: &str = "
ALTER TABLE sync_manifest RENAME COLUMN drive_file_id TO remote_file_id;
ALTER TABLE sync_manifest RENAME COLUMN drive_mtime_ms TO remote_mtime_ms;
ALTER TABLE sync_conflicts RENAME COLUMN drive_mtime_ms TO remote_mtime_ms;
ALTER TABLE sync_conflicts RENAME COLUMN drive_size TO remote_size;
ALTER TABLE sync_conflicts RENAME COLUMN drive_device TO remote_device;
ALTER TABLE sync_conflicts RENAME COLUMN drive_file_id TO remote_file_id;
";

/// v12 — tabela chave→valor genérica para metadados internos do app (carimbos
/// de manutenção, versionamento lógico de schema). Separada de `app_settings`,
/// que guarda apenas preferências visíveis/editáveis pelo usuário. Ver
/// `storage::kv`.
const SCHEMA_V12: &str = "
CREATE TABLE IF NOT EXISTS internal_kv (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
";

#[derive(Clone)]
pub struct Db {
    conn: Arc<Mutex<Connection>>,
}

impl Db {
    pub fn open(path: &Path) -> AppResult<Self> {
        Self::from_connection(Connection::open(path)?)
    }

    #[cfg(test)]
    pub fn open_in_memory() -> AppResult<Self> {
        Self::from_connection(Connection::open_in_memory()?)
    }

    /// Acesso síncrono direto (sem spawn_blocking). Usado pelo `SqliteSecretStore`
    /// no mobile; em desktop a função existe mas não é chamada.
    #[cfg_attr(not(mobile), allow(dead_code))]
    pub fn with_conn_blocking<T>(
        &self,
        f: impl FnOnce(&Connection) -> AppResult<T>,
    ) -> AppResult<T> {
        let guard = self
            .conn
            .lock()
            .map_err(|_| AppError::Other("lock do SQLite envenenado".into()))?;
        f(&guard)
    }

    /// Acesso síncrono direto para testes (sem runtime async).
    #[cfg(test)]
    pub fn with_sync<T>(&self, f: impl FnOnce(&Connection) -> AppResult<T>) -> T {
        let guard = self.conn.lock().unwrap();
        f(&guard).expect("operação de teste no SQLite falhou")
    }

    fn from_connection(conn: Connection) -> AppResult<Self> {
        // journal_mode=WAL: leituras não bloqueiam a escrita em andamento.
        // foreign_keys: nenhuma FK é declarada hoje, mas protege migrações futuras.
        // synchronous=NORMAL: seguro em WAL (só fsync no checkpoint), evita o
        // custo de FULL a cada commit sem abrir mão de durabilidade após crash.
        // auto_vacuum=INCREMENTAL: só tem efeito num banco novo (SQLite exige
        // VACUUM para aplicar retroativamente); para bancos existentes é um
        // no-op inofensivo — não vale forçar um VACUUM de uma tabela grande
        // só por causa disso.
        let _: String = conn.query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))?;
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA synchronous = NORMAL;
             PRAGMA auto_vacuum = INCREMENTAL;",
        )?;
        migrate(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    /// Executa `f` com a conexão num thread bloqueante do Tokio.
    pub async fn with<T, F>(&self, f: F) -> AppResult<T>
    where
        F: FnOnce(&Connection) -> AppResult<T> + Send + 'static,
        T: Send + 'static,
    {
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || {
            let guard = conn
                .lock()
                .map_err(|_| AppError::Other("lock do SQLite envenenado".into()))?;
            f(&guard)
        })
        .await
        .map_err(|e| AppError::Other(format!("tarefa bloqueante abortada: {e}")))?
    }
}

fn migrate(conn: &Connection) -> AppResult<()> {
    // Migrações incrementais: cada bloco eleva o `user_version` em 1. Adicionar
    // uma migração nova = mais um `if version < N` com seu `SCHEMA_VN`.
    let mut version: i64 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if version < 1 {
        conn.execute_batch(SCHEMA_V1)?;
        version = 1;
    }
    if version < 2 {
        conn.execute_batch(SCHEMA_V2)?;
        version = 2;
    }
    if version < 3 {
        conn.execute_batch(SCHEMA_V3)?;
        version = 3;
    }
    if version < 4 {
        conn.execute_batch(SCHEMA_V4)?;
        version = 4;
    }
    if version < 5 {
        conn.execute_batch(SCHEMA_V5)?;
        version = 5;
    }
    if version < 6 {
        conn.execute_batch(SCHEMA_V6)?;
        version = 6;
    }
    if version < 7 {
        conn.execute_batch(SCHEMA_V7)?;
        version = 7;
    }
    if version < 8 {
        conn.execute_batch(SCHEMA_V8)?;
        version = 8;
    }
    if version < 9 {
        conn.execute_batch(SCHEMA_V9)?;
        version = 9;
    }
    if version < 10 {
        conn.execute_batch(SCHEMA_V10)?;
        version = 10;
    }
    if version < 11 {
        conn.execute_batch(SCHEMA_V11)?;
        version = 11;
    }
    if version < 12 {
        conn.execute_batch(SCHEMA_V12)?;
        version = 12;
    }
    conn.pragma_update(None, "user_version", version)?;
    Ok(())
}
