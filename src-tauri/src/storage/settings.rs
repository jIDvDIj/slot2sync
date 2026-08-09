//! Configurações globais do usuário — tabela `app_settings` (chave→valor).
//!
//! Um único `Settings` agrega as configurações expostas ao frontend; cada
//! campo é persistido como uma linha chave→valor, com defaults aplicados na
//! leitura. Inclui nome do dispositivo, gatilhos de sync e nível de
//! notificação, entre outras configurações.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::constants::{
    BACKUP_RETENTION_DAYS_DEFAULT, MAX_BACKUP_VERSIONS_DEFAULT, SCAN_INTERVAL_MINUTES_DEFAULT,
    SETTING_BACKUP_RETENTION_DAYS, SETTING_DEVICE_NAME, SETTING_DISMISSED_NOTICES,
    SETTING_DOWNLOAD_KBPS, SETTING_MAX_BACKUP_VERSIONS, SETTING_NOTIFICATION_LEVEL,
    SETTING_SCAN_INTERVAL_MINUTES, SETTING_TRIGGER_EMULATOR_START, SETTING_TRIGGER_EMULATOR_STOP,
    SETTING_TRIGGER_STARTUP, SETTING_UPLOAD_KBPS,
};
// Consumido apenas pelas funções de autostart (só-desktop).
#[cfg(desktop)]
use crate::constants::SETTING_AUTOSTART_INITIALIZED;
use crate::error::AppResult;

/// Configurações globais. (→ ipc.ts)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// Nome amigável deste dispositivo (ex.: "PC Gamer"). `None` até o usuário
    /// defini-lo no login. Gravado também nos metadados de sync no Drive.
    pub device_name: Option<String>,
    /// Gatilhos de sync automático habilitados.
    pub triggers: TriggerSettings,
    /// Quais eventos geram notificação nativa do SO.
    pub notification_level: NotificationLevel,
    /// Início automático junto com o sistema operacional. NÃO é persistido no
    /// banco: o estado vive no SO (registro do Windows / LaunchAgent) e é
    /// preenchido pelo comando `get_settings` via o plugin de autostart.
    /// `load` sempre devolve `false`; o valor real é injetado na camada de
    /// comando.
    #[serde(default)]
    pub autostart: bool,
    /// Dias de retenção dos backups locais (0 = manter para sempre). A limpeza
    /// roda no startup do app.
    #[serde(default = "default_backup_retention_days")]
    pub backup_retention_days: u32,
    /// Intervalo do scan periódico em minutos (0 = desativado). O timer aplica
    /// jitter de ±25% e só dispara quando nenhum emulador está rodando.
    #[serde(default = "default_scan_interval_minutes")]
    pub scan_interval_minutes: u32,
    /// Máximo de versões arquivadas por arquivo antes de cada download
    /// sobrescrever o local (mínimo 1).
    #[serde(default = "default_max_backup_versions")]
    pub max_backup_versions: u32,
    /// Limite de upload em KB/s (0 = ilimitado).
    #[serde(default)]
    pub upload_kbps: u32,
    /// Limite de download em KB/s (0 = ilimitado).
    #[serde(default)]
    pub download_kbps: u32,
}

fn default_max_backup_versions() -> u32 {
    MAX_BACKUP_VERSIONS_DEFAULT
}

fn default_scan_interval_minutes() -> u32 {
    SCAN_INTERVAL_MINUTES_DEFAULT
}

fn default_backup_retention_days() -> u32 {
    BACKUP_RETENTION_DAYS_DEFAULT
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            device_name: None,
            triggers: TriggerSettings::default(),
            notification_level: NotificationLevel::default(),
            autostart: false,
            backup_retention_days: BACKUP_RETENTION_DAYS_DEFAULT,
            scan_interval_minutes: SCAN_INTERVAL_MINUTES_DEFAULT,
            max_backup_versions: MAX_BACKUP_VERSIONS_DEFAULT,
            upload_kbps: 0,
            download_kbps: 0,
        }
    }
}

/// Nível de notificações nativas. (→ ipc.ts)
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NotificationLevel {
    /// Sync concluído, erros e emulador detectado.
    #[default]
    All,
    /// Apenas erros de sync.
    ErrorsOnly,
    /// Nenhuma notificação.
    None,
}

impl NotificationLevel {
    fn as_str(self) -> &'static str {
        match self {
            NotificationLevel::All => "all",
            NotificationLevel::ErrorsOnly => "errors_only",
            NotificationLevel::None => "none",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "all" => Some(NotificationLevel::All),
            "errors_only" => Some(NotificationLevel::ErrorsOnly),
            "none" => Some(NotificationLevel::None),
            _ => None,
        }
    }

    /// Erros de sync devem notificar?
    pub fn notifies_errors(self) -> bool {
        !matches!(self, NotificationLevel::None)
    }

    /// Eventos informativos (sync concluído, emulador detectado) devem notificar?
    pub fn notifies_info(self) -> bool {
        matches!(self, NotificationLevel::All)
    }
}

/// Gatilhos de sync automático. (→ ipc.ts) Default: todos ligados. O sync
/// manual nunca é afetado por estes flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TriggerSettings {
    /// Sync ao abrir o Slot2Sync.
    pub startup: bool,
    /// Download antes de o emulador abrir.
    pub emulator_start: bool,
    /// Upload ao fechar o emulador.
    pub emulator_stop: bool,
}

impl Default for TriggerSettings {
    fn default() -> Self {
        Self {
            startup: true,
            emulator_start: true,
            emulator_stop: true,
        }
    }
}

fn get(conn: &Connection, key: &str) -> AppResult<Option<String>> {
    let value = conn
        .query_row(
            "SELECT value FROM app_settings WHERE key = ?1",
            params![key],
            |row| row.get(0),
        )
        .optional()?;
    Ok(value)
}

fn set(conn: &Connection, key: &str, value: &str) -> AppResult<()> {
    conn.execute(
        "INSERT OR REPLACE INTO app_settings (key, value) VALUES (?1, ?2)",
        params![key, value],
    )?;
    Ok(())
}

fn get_bool(conn: &Connection, key: &str, default: bool) -> AppResult<bool> {
    Ok(get(conn, key)?.map(|v| v == "true").unwrap_or(default))
}

fn set_bool(conn: &Connection, key: &str, value: bool) -> AppResult<()> {
    set(conn, key, if value { "true" } else { "false" })
}

/// Lê todas as configurações, aplicando defaults para chaves ausentes.
pub fn load(conn: &Connection) -> AppResult<Settings> {
    Ok(Settings {
        device_name: get(conn, SETTING_DEVICE_NAME)?,
        triggers: triggers(conn)?,
        notification_level: notification_level(conn)?,
        // Estado do SO, não do banco: o comando `get_settings` injeta o valor
        // real lido pelo plugin de autostart.
        autostart: false,
        backup_retention_days: backup_retention_days(conn)?,
        scan_interval_minutes: scan_interval_minutes(conn)?,
        max_backup_versions: max_backup_versions(conn)?,
        upload_kbps: upload_kbps(conn)?,
        download_kbps: download_kbps(conn)?,
    })
}

/// Limite de upload em KB/s (default: 0 = ilimitado).
pub fn upload_kbps(conn: &Connection) -> AppResult<u32> {
    Ok(get(conn, SETTING_UPLOAD_KBPS)?
        .and_then(|v| v.parse().ok())
        .unwrap_or(0))
}

/// Limite de download em KB/s (default: 0 = ilimitado).
pub fn download_kbps(conn: &Connection) -> AppResult<u32> {
    Ok(get(conn, SETTING_DOWNLOAD_KBPS)?
        .and_then(|v| v.parse().ok())
        .unwrap_or(0))
}

pub fn set_bandwidth_limits(conn: &Connection, upload: u32, download: u32) -> AppResult<()> {
    set(conn, SETTING_UPLOAD_KBPS, &upload.to_string())?;
    set(conn, SETTING_DOWNLOAD_KBPS, &download.to_string())
}

/// Máximo de versões arquivadas por arquivo (default: 5; mínimo efetivo 1).
pub fn max_backup_versions(conn: &Connection) -> AppResult<u32> {
    Ok(get(conn, SETTING_MAX_BACKUP_VERSIONS)?
        .and_then(|v| v.parse().ok())
        .unwrap_or(MAX_BACKUP_VERSIONS_DEFAULT))
}

pub fn set_max_backup_versions(conn: &Connection, versions: u32) -> AppResult<()> {
    set(
        conn,
        SETTING_MAX_BACKUP_VERSIONS,
        &versions.max(1).to_string(),
    )
}

/// Intervalo do scan periódico em minutos (default: 60; 0 = desativado).
pub fn scan_interval_minutes(conn: &Connection) -> AppResult<u32> {
    Ok(get(conn, SETTING_SCAN_INTERVAL_MINUTES)?
        .and_then(|v| v.parse().ok())
        .unwrap_or(SCAN_INTERVAL_MINUTES_DEFAULT))
}

pub fn set_scan_interval_minutes(conn: &Connection, minutes: u32) -> AppResult<()> {
    set(conn, SETTING_SCAN_INTERVAL_MINUTES, &minutes.to_string())
}

/// Dias de retenção dos backups locais (default: 30; 0 = manter para sempre).
pub fn backup_retention_days(conn: &Connection) -> AppResult<u32> {
    Ok(get(conn, SETTING_BACKUP_RETENTION_DAYS)?
        .and_then(|v| v.parse().ok())
        .unwrap_or(BACKUP_RETENTION_DAYS_DEFAULT))
}

pub fn set_backup_retention_days(conn: &Connection, days: u32) -> AppResult<()> {
    set(conn, SETTING_BACKUP_RETENTION_DAYS, &days.to_string())
}

/// Nível de notificações (default: `All`).
pub fn notification_level(conn: &Connection) -> AppResult<NotificationLevel> {
    Ok(get(conn, SETTING_NOTIFICATION_LEVEL)?
        .as_deref()
        .and_then(NotificationLevel::parse)
        .unwrap_or_default())
}

pub fn set_notification_level(conn: &Connection, level: NotificationLevel) -> AppResult<()> {
    set(conn, SETTING_NOTIFICATION_LEVEL, level.as_str())
}

/// Gatilhos automáticos habilitados (default: todos ligados).
pub fn triggers(conn: &Connection) -> AppResult<TriggerSettings> {
    Ok(TriggerSettings {
        startup: get_bool(conn, SETTING_TRIGGER_STARTUP, true)?,
        emulator_start: get_bool(conn, SETTING_TRIGGER_EMULATOR_START, true)?,
        emulator_stop: get_bool(conn, SETTING_TRIGGER_EMULATOR_STOP, true)?,
    })
}

pub fn set_triggers(conn: &Connection, triggers: &TriggerSettings) -> AppResult<()> {
    set_bool(conn, SETTING_TRIGGER_STARTUP, triggers.startup)?;
    set_bool(
        conn,
        SETTING_TRIGGER_EMULATOR_START,
        triggers.emulator_start,
    )?;
    set_bool(conn, SETTING_TRIGGER_EMULATOR_STOP, triggers.emulator_stop)?;
    Ok(())
}

/// O default de fábrica do autostart (ligado) já foi aplicado? `false` na
/// primeiríssima execução. Ver [`mark_autostart_initialized`] e o setup em
/// `lib.rs`. Só-desktop: não há autostart no mobile.
#[cfg(desktop)]
pub fn autostart_initialized(conn: &Connection) -> AppResult<bool> {
    get_bool(conn, SETTING_AUTOSTART_INITIALIZED, false)
}

/// Marca o default de fábrica do autostart como já aplicado. Só-desktop.
#[cfg(desktop)]
pub fn mark_autostart_initialized(conn: &Connection) -> AppResult<()> {
    set_bool(conn, SETTING_AUTOSTART_INITIALIZED, true)
}

/// IDs de banners informativos dispensados pelo usuário (array JSON na chave
/// `dismissed_notices`). Um valor ilegível conta como "nada dispensado".
pub fn dismissed_notices(conn: &Connection) -> AppResult<Vec<String>> {
    Ok(get(conn, SETTING_DISMISSED_NOTICES)?
        .and_then(|value| serde_json::from_str(&value).ok())
        .unwrap_or_default())
}

/// Marca um banner como dispensado — ele não volta a ser exibido. Idempotente.
pub fn dismiss_notice(conn: &Connection, id: &str) -> AppResult<()> {
    let mut ids = dismissed_notices(conn)?;
    if !ids.iter().any(|existing| existing == id) {
        ids.push(id.to_string());
        set(
            conn,
            SETTING_DISMISSED_NOTICES,
            &serde_json::to_string(&ids)?,
        )?;
    }
    Ok(())
}

/// Nome do dispositivo isolado (usado pelo engine ao publicar metadados).
pub fn device_name(conn: &Connection) -> AppResult<Option<String>> {
    get(conn, SETTING_DEVICE_NAME)
}

pub fn set_device_name(conn: &Connection, name: &str) -> AppResult<()> {
    set(conn, SETTING_DEVICE_NAME, name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::db::Db;

    #[test]
    fn load_retorna_defaults_quando_vazio() {
        let db = Db::open_in_memory().unwrap();
        db.with_sync(|conn| {
            assert_eq!(load(conn)?, Settings::default());
            // Default = todos os gatilhos ligados, notificações em `All`.
            assert_eq!(
                triggers(conn)?,
                TriggerSettings {
                    startup: true,
                    emulator_start: true,
                    emulator_stop: true,
                }
            );
            assert_eq!(notification_level(conn)?, NotificationLevel::All);
            Ok(())
        });
    }

    #[test]
    fn set_e_get_triggers_fazem_roundtrip() {
        let db = Db::open_in_memory().unwrap();
        db.with_sync(|conn| {
            let t = TriggerSettings {
                startup: false,
                emulator_start: true,
                emulator_stop: false,
            };
            set_triggers(conn, &t)?;
            assert_eq!(triggers(conn)?, t);
            assert_eq!(load(conn)?.triggers, t);
            Ok(())
        });
    }

    #[test]
    fn autostart_initialized_roundtrip() {
        let db = Db::open_in_memory().unwrap();
        db.with_sync(|conn| {
            // Default: ainda não aplicado na primeira execução.
            assert!(!autostart_initialized(conn)?);
            mark_autostart_initialized(conn)?;
            assert!(autostart_initialized(conn)?);
            Ok(())
        });
    }

    #[test]
    fn set_device_name_persiste_e_e_lido() {
        let db = Db::open_in_memory().unwrap();
        db.with_sync(|conn| {
            set_device_name(conn, "PC Gamer")?;
            assert_eq!(device_name(conn)?, Some("PC Gamer".to_string()));
            assert_eq!(load(conn)?.device_name, Some("PC Gamer".to_string()));
            Ok(())
        });
    }

    #[test]
    fn set_device_name_substitui_valor_anterior() {
        let db = Db::open_in_memory().unwrap();
        db.with_sync(|conn| {
            set_device_name(conn, "Notebook")?;
            set_device_name(conn, "PC Gamer")?;
            assert_eq!(device_name(conn)?, Some("PC Gamer".to_string()));
            Ok(())
        });
    }

    #[test]
    fn set_e_get_notification_level_fazem_roundtrip() {
        let db = Db::open_in_memory().unwrap();
        db.with_sync(|conn| {
            set_notification_level(conn, NotificationLevel::ErrorsOnly)?;
            assert_eq!(notification_level(conn)?, NotificationLevel::ErrorsOnly);
            set_notification_level(conn, NotificationLevel::None)?;
            assert_eq!(load(conn)?.notification_level, NotificationLevel::None);
            Ok(())
        });
    }

    #[test]
    fn notification_level_gating() {
        assert!(NotificationLevel::All.notifies_errors());
        assert!(NotificationLevel::All.notifies_info());
        assert!(NotificationLevel::ErrorsOnly.notifies_errors());
        assert!(!NotificationLevel::ErrorsOnly.notifies_info());
        assert!(!NotificationLevel::None.notifies_errors());
        assert!(!NotificationLevel::None.notifies_info());
    }

    #[test]
    fn backup_retention_days_default_e_roundtrip() {
        let db = Db::open_in_memory().unwrap();
        db.with_sync(|conn| {
            assert_eq!(backup_retention_days(conn)?, BACKUP_RETENTION_DAYS_DEFAULT);
            set_backup_retention_days(conn, 7)?;
            assert_eq!(backup_retention_days(conn)?, 7);
            assert_eq!(load(conn)?.backup_retention_days, 7);
            // 0 = manter para sempre (limpeza desativada).
            set_backup_retention_days(conn, 0)?;
            assert_eq!(backup_retention_days(conn)?, 0);
            Ok(())
        });
    }

    #[test]
    fn scan_interval_minutes_default_e_roundtrip() {
        let db = Db::open_in_memory().unwrap();
        db.with_sync(|conn| {
            assert_eq!(scan_interval_minutes(conn)?, SCAN_INTERVAL_MINUTES_DEFAULT);
            set_scan_interval_minutes(conn, 15)?;
            assert_eq!(scan_interval_minutes(conn)?, 15);
            assert_eq!(load(conn)?.scan_interval_minutes, 15);
            // 0 = scan periódico desativado.
            set_scan_interval_minutes(conn, 0)?;
            assert_eq!(scan_interval_minutes(conn)?, 0);
            Ok(())
        });
    }

    #[test]
    fn max_backup_versions_default_roundtrip_e_minimo() {
        let db = Db::open_in_memory().unwrap();
        db.with_sync(|conn| {
            assert_eq!(max_backup_versions(conn)?, MAX_BACKUP_VERSIONS_DEFAULT);
            set_max_backup_versions(conn, 8)?;
            assert_eq!(max_backup_versions(conn)?, 8);
            // 0 é rejeitado — mínimo efetivo é 1 versão.
            set_max_backup_versions(conn, 0)?;
            assert_eq!(max_backup_versions(conn)?, 1);
            Ok(())
        });
    }

    #[test]
    fn limites_de_banda_default_e_roundtrip() {
        let db = Db::open_in_memory().unwrap();
        db.with_sync(|conn| {
            assert_eq!(upload_kbps(conn)?, 0);
            assert_eq!(download_kbps(conn)?, 0);
            set_bandwidth_limits(conn, 512, 1024)?;
            assert_eq!(upload_kbps(conn)?, 512);
            assert_eq!(download_kbps(conn)?, 1024);
            assert_eq!(load(conn)?.upload_kbps, 512);
            assert_eq!(load(conn)?.download_kbps, 1024);
            Ok(())
        });
    }

    #[test]
    fn dismiss_notice_persiste_e_e_idempotente() {
        let db = Db::open_in_memory().unwrap();
        db.with_sync(|conn| {
            assert!(dismissed_notices(conn)?.is_empty());

            dismiss_notice(conn, "backup-primeiro-sync")?;
            dismiss_notice(conn, "backup-primeiro-sync")?;
            dismiss_notice(conn, "pendencias")?;

            assert_eq!(
                dismissed_notices(conn)?,
                vec!["backup-primeiro-sync".to_string(), "pendencias".to_string()]
            );
            Ok(())
        });
    }

    #[test]
    fn settings_serializa_em_camel_case() {
        let json = serde_json::to_value(Settings {
            device_name: Some("PC Gamer".into()),
            triggers: TriggerSettings::default(),
            notification_level: NotificationLevel::ErrorsOnly,
            autostart: false,
            backup_retention_days: 15,
            scan_interval_minutes: 45,
            max_backup_versions: 3,
            upload_kbps: 256,
            download_kbps: 0,
        })
        .unwrap();
        assert_eq!(json["deviceName"], "PC Gamer");
        assert_eq!(json["backupRetentionDays"], 15);
        assert_eq!(json["scanIntervalMinutes"], 45);
        assert_eq!(json["maxBackupVersions"], 3);
        assert_eq!(json["uploadKbps"], 256);
        assert_eq!(json["downloadKbps"], 0);
        assert_eq!(json["triggers"]["startup"], true);
        assert_eq!(json["triggers"]["emulatorStart"], true);
        assert_eq!(json["triggers"]["emulatorStop"], true);
        assert_eq!(json["notificationLevel"], "errors_only");
        assert_eq!(json["autostart"], false);
    }
}
