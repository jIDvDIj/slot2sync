//! Persistência local em SQLite via `rusqlite`.
//!
//! - `db`: conexão única + migrações, acesso async via `spawn_blocking`;
//! - `manifest`: tabela `sync_manifest` — estado de cada arquivo no último sync;
//! - `queue`: fila de operações pendentes (resiliência offline);
//! - `emulators`: perfis configurados pelo usuário;
//! - `settings`: configurações globais (nome do dispositivo, gatilhos, etc.);
//! - `conflicts`: conflitos pendentes que bloqueiam o sync de um emulador;
//! - `drive_folders`: cache persistente de IDs de pasta do Drive;
//! - `stats`: contadores acumulados por emulador (uploads, downloads, bytes).

pub mod conflicts;
pub mod db;
pub mod drive_folders;
pub mod emulators;
pub mod manifest;
pub mod queue;
pub mod settings;
pub mod stats;
