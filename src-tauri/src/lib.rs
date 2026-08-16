mod auth;
mod backups;
mod commands;
mod constants;
mod device;
mod drive;
mod dropbox;
mod emulator;
mod error;
mod events;
mod folder;
mod games;
mod locations;
mod onedrive;
mod platform;
mod remote;
mod secrets;
mod state;
mod storage;
mod sync;
mod versioning;
// O process watcher depende de inspecionar processos do SO (`sysinfo`), o que
// não existe/aplica no mobile — gatilhos automáticos são exclusivos do desktop.
#[cfg(desktop)]
mod watcher;

use std::sync::Arc;

use tauri::Manager;
use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

use state::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // `mut` é usado apenas no bloco `#[cfg(desktop)]` abaixo; no mobile o
    // builder não é reatribuído.
    #[allow(unused_mut)]
    let mut builder = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init());

    // Recursos só-desktop registrados no builder: autostart ("subir com o
    // sistema") e o fechar-esconde da janela (o app segue vivo na bandeja). No
    // mobile não há bandeja nem ciclo de janela equivalente.
    // `single-instance` precisa ser o primeiro plugin registrado (exigência da
    // própria crate). Uma segunda tentativa de abrir o app só foca a janela
    // existente — evita duas instâncias concorrentes disputando o keyring e o
    // listener loopback do OAuth.
    #[cfg(desktop)]
    {
        builder = builder
            .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
                platform::desktop::show_main_window(app);
            }))
            .plugin(tauri_plugin_autostart::init(
                tauri_plugin_autostart::MacosLauncher::LaunchAgent,
                Some(vec![constants::STARTUP_MINIMIZED_FLAG]),
            ))
            .on_window_event(platform::desktop::on_close_requested);
    }

    // Plugins exclusivos do mobile:
    // - deep-link: captura `slot2sync://oauth?code=...` de volta ao app (OAuth).
    // - opener: abre o browser nativo (o crate `open` não funciona no sandbox Android).
    #[cfg(mobile)]
    {
        builder = builder
            .plugin(sync::mobile_storage::init())
            .plugin(tauri_plugin_deep_link::init())
            .plugin(tauri_plugin_opener::init());
    }

    builder
        .setup(|app| {
            init_logging(app.handle())?;
            tracing::info!(version = env!("CARGO_PKG_VERSION"), "Slot2Sync iniciado");
            #[cfg(windows)]
            lower_process_priority();

            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let db_path = locations::AppPath::Database.resolve(app.handle())?;
            let db = storage::db::Db::open(&db_path)?;

            let last_sync: sync::LastSyncStore = Arc::new(std::sync::Mutex::new(None));
            let http = reqwest::Client::new();

            // Inicializa o store de segredos adequado à plataforma.
            #[cfg(desktop)]
            let secret_store: Arc<dyn secrets::SecretStore> = Arc::new(secrets::KeyringStore);
            #[cfg(mobile)]
            let secret_store: Arc<dyn secrets::SecretStore> =
                Arc::new(secrets::SqliteSecretStore(db.clone()));

            // Garante a identidade estável deste dispositivo (UUID no keyring /
            // SQLite, gerado na primeira execução; consumido na detecção de conflito).
            match device::get_or_create(&*secret_store) {
                Ok(id) => tracing::info!(device_id = %id, "device_id resolvido"),
                Err(err) => tracing::warn!(
                    error = %err,
                    "device_id indisponível; seguindo sem identidade estável"
                ),
            }

            // Constrói o provedor (e o AuthManager, se OAuth) a partir da config
            // persistida. Nenhum provedor escolhido ainda (primeiro uso) = os
            // dois ficam vazios; a UI mostra o seletor e `connect_*`/
            // `connect_local_folder` os preenchem em tempo de execução (ver
            // `commands.rs`) — sem precisar reiniciar o app.
            let stored_provider = db
                .with_conn_blocking(storage::settings::storage_provider)
                .ok()
                .flatten();
            let (auth, remote_provider): (
                Option<Arc<auth::AuthManager>>,
                Option<Arc<dyn remote::RemoteProvider>>,
            ) = match stored_provider {
                Some(kind @ remote::ProviderKind::GoogleDrive) => {
                    let auth = Arc::new(auth::AuthManager::new_for(
                        kind,
                        http.clone(),
                        secret_store.clone(),
                    ));
                    let client = Arc::new(drive::DriveClient::new(
                        http.clone(),
                        auth.clone(),
                        db.clone(),
                    ));
                    (Some(auth), Some(client))
                }
                Some(kind @ remote::ProviderKind::Dropbox) => {
                    let auth = Arc::new(auth::AuthManager::new_for(
                        kind,
                        http.clone(),
                        secret_store.clone(),
                    ));
                    let client = Arc::new(dropbox::DropboxClient::new(http.clone(), auth.clone()));
                    (Some(auth), Some(client))
                }
                Some(kind @ remote::ProviderKind::OneDrive) => {
                    let auth = Arc::new(auth::AuthManager::new_for(
                        kind,
                        http.clone(),
                        secret_store.clone(),
                    ));
                    let client =
                        Arc::new(onedrive::OneDriveClient::new(http.clone(), auth.clone()));
                    (Some(auth), Some(client))
                }
                Some(remote::ProviderKind::LocalFolder) => {
                    let path = db
                        .with_conn_blocking(storage::settings::folder_provider_path)
                        .ok()
                        .flatten();
                    match path {
                        Some(p) => (
                            None,
                            Some(
                                Arc::new(folder::FolderProvider::new(std::path::PathBuf::from(p)))
                                    as Arc<dyn remote::RemoteProvider>,
                            ),
                        ),
                        // Config inconsistente (settings corrompidas) — trata como
                        // não configurado; a UI volta a mostrar o seletor.
                        None => (None, None),
                    }
                }
                None => (None, None),
            };

            // Storage local: filesystem no desktop; plugin nativo (SAF/bookmarks)
            // no mobile, montado a partir da ponte registrada pelo plugin acima.
            #[cfg(desktop)]
            let storage: Arc<dyn sync::LocalStorage> = Arc::new(sync::DesktopStorage);
            #[cfg(mobile)]
            let storage: Arc<dyn sync::LocalStorage> = sync::mobile_storage::storage(app.handle())?;

            let engine = Arc::new(sync::SyncEngine::new(
                db.clone(),
                remote_provider,
                app.handle().clone(),
                last_sync.clone(),
                locations::AppPath::BackupDir.resolve(app.handle())?,
                storage.clone(),
                secret_store.clone(),
            ));

            app.manage(AppState {
                auth: std::sync::RwLock::new(auth),
                db: db.clone(),
                engine: engine.clone(),
                last_sync,
                storage,
                http,
                secrets: secret_store,
            });

            // Bandeja, janela escondível, autostart e process watcher são
            // exclusivos do desktop. No mobile o webview único já é exibido pelo
            // sistema e os gatilhos automáticos por processo não existem.
            #[cfg(desktop)]
            platform::desktop::setup(app, db.clone(), engine.clone())?;
            #[cfg(mobile)]
            platform::mobile::setup(app)?;

            // Gatilhos mobile: foreground (app volta à tela) e background (app some).
            // Substituem o process watcher e o sync de despedida do desktop.
            #[cfg(mobile)]
            {
                use tauri::Listener;
                let eng = engine.clone();
                app.listen("tauri://resume", move |_| {
                    let e = eng.clone();
                    tauri::async_runtime::spawn(async move {
                        if let Err(err) = e
                            .sync_all(
                                sync::SyncDirection::Bidirectional,
                                constants::TRIGGER_FOREGROUND,
                            )
                            .await
                        {
                            tracing::warn!(error = %err, "sync de foreground falhou");
                        }
                    });
                });
                let eng = engine.clone();
                app.listen("tauri://pause", move |_| {
                    let e = eng.clone();
                    tauri::async_runtime::spawn(async move {
                        if let Err(err) = e
                            .sync_all(
                                sync::SyncDirection::LocalToDrive,
                                constants::TRIGGER_BACKGROUND,
                            )
                            .await
                        {
                            tracing::warn!(error = %err, "sync de background falhou");
                        }
                    });
                });
            }

            // Retenção de backups: remove no startup as execuções de backup mais
            // antigas que o limite configurado (default 30 dias; 0 desativa).
            let retention_db = db.clone();
            let backups_root = locations::AppPath::BackupDir.resolve(app.handle())?;
            tauri::async_runtime::spawn(async move {
                let days = retention_db
                    .with(storage::settings::backup_retention_days)
                    .await
                    .unwrap_or(constants::BACKUP_RETENTION_DAYS_DEFAULT);
                let result =
                    tokio::task::spawn_blocking(move || backups::prune_old(&backups_root, days))
                        .await;
                match result {
                    Ok(Ok(0)) => {}
                    Ok(Ok(removed)) => {
                        tracing::info!(
                            removidas = removed,
                            dias = days,
                            "retenção de backups aplicada"
                        )
                    }
                    Ok(Err(err)) => tracing::warn!(error = %err, "falha na retenção de backups"),
                    Err(err) => tracing::warn!(error = %err, "tarefa de retenção abortada"),
                }
            });

            // Gatilho "ao iniciar o Slot2Sync": sync bidirecional em background,
            // se o usuário não tiver desativado o gatilho `startup`. Vale para
            // desktop e mobile (no mobile é o sync ao abrir o app).
            let startup_db = db.clone();
            let startup_engine = engine;
            tauri::async_runtime::spawn(async move {
                let enabled = startup_db
                    .with(storage::settings::triggers)
                    .await
                    .map(|t| t.startup)
                    .unwrap_or(true);
                if !enabled {
                    tracing::info!("gatilho startup desativado; sync de inicialização ignorado");
                    return;
                }
                match startup_engine
                    .sync_all(
                        sync::SyncDirection::Bidirectional,
                        constants::TRIGGER_STARTUP,
                    )
                    .await
                {
                    Ok(summary) => tracing::info!(?summary, "sync de inicialização concluído"),
                    Err(err) => tracing::warn!(error = %err, "sync de inicialização não executado"),
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::health_check,
            commands::connect_google_drive,
            commands::connect_dropbox,
            commands::connect_onedrive,
            commands::connect_local_folder,
            commands::get_auth_status,
            commands::disconnect_provider,
            commands::detect_emulator,
            commands::add_emulator,
            commands::add_emulator_manual,
            commands::discover_emulators,
            commands::list_emulators,
            commands::list_synced_games,
            commands::get_emulator_stats,
            commands::list_emulator_stats,
            commands::remove_emulator,
            commands::sync_now,
            commands::get_last_sync,
            commands::get_settings,
            commands::set_device_name,
            commands::set_triggers,
            commands::set_notification_level,
            commands::set_backup_retention_days,
            commands::set_scan_interval_minutes,
            commands::set_max_backup_versions,
            commands::set_bandwidth_limits,
            commands::get_emulator_categories,
            commands::set_emulator_categories,
            commands::set_exclude_patterns,
            commands::list_conflicts,
            commands::resolve_conflict,
            commands::list_pending_ops,
            commands::retry_pending_op,
            commands::list_dismissed_notices,
            commands::dismiss_notice,
            commands::list_backups,
            commands::list_file_versions,
            commands::restore_version,
            commands::pick_emulator_folder,
            commands::detect_emulator_mobile,
            #[cfg(desktop)]
            commands::set_autostart,
            #[cfg(desktop)]
            commands::open_backup_folder,
            #[cfg(desktop)]
            commands::reveal_backup_path,
        ])
        .run(tauri::generate_context!())
        .expect("erro ao iniciar o Slot2Sync");
}

/// Logs em stdout (dev) e em arquivo diário no diretório de logs do app
/// (`%LOCALAPPDATA%/com.slot2sync.app/logs` no Windows).
/// Dias de retenção dos arquivos de log rotacionados (`prune_old_logs`).
/// Fixo por ora — mesmo racional de outras constantes de retenção do app que
/// não viraram configuração de usuário (evita expandir a boundary IPC por uma
/// issue de manutenção interna).
const LOG_RETENTION_DAYS: u32 = 7;

/// Baixa a prioridade do processo (`BELOW_NORMAL_PRIORITY_CLASS`) no
/// startup — a sincronização faz I/O em background e não deve competir com um
/// emulador aberto pelos ciclos de CPU/prioridade de I/O do Windows.
/// Best-effort: falha aqui não impede o app de iniciar.
#[cfg(windows)]
fn lower_process_priority() {
    use windows::Win32::System::Threading::{
        GetCurrentProcess, SetPriorityClass, BELOW_NORMAL_PRIORITY_CLASS,
    };
    // SAFETY: `GetCurrentProcess` devolve um pseudo-handle válido para a
    // duração do processo, sem precisar ser fechado; `SetPriorityClass` só
    // lê esse handle e a flag de prioridade, não há estado inválido possível.
    let result = unsafe { SetPriorityClass(GetCurrentProcess(), BELOW_NORMAL_PRIORITY_CLASS) };
    if let Err(err) = result {
        tracing::warn!(error = %err, "falha ao baixar a prioridade do processo");
    }
}

fn init_logging(app: &tauri::AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let log_dir = locations::AppPath::LogDir.resolve(app)?;
    std::fs::create_dir_all(&log_dir)?;
    prune_old_logs(&log_dir, LOG_RETENTION_DAYS);
    let file_appender = tracing_appender::rolling::daily(&log_dir, "slot2sync.log");

    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(fmt::layer())
        .with(fmt::layer().with_ansi(false).with_writer(file_appender))
        .init();

    Ok(())
}

/// Remove de `log_dir` os arquivos de log (rotação diária do
/// `tracing-appender`) cujo mtime é mais antigo que `retention_days` — sem
/// isso, o diretório de logs cresce indefinidamente ao longo do uso do app.
/// Roda antes do subscriber ser inicializado, então erros aqui só vão para
/// stderr (não há `tracing` disponível ainda).
fn prune_old_logs(log_dir: &std::path::Path, retention_days: u32) {
    let cutoff = std::time::SystemTime::now()
        .checked_sub(std::time::Duration::from_secs(
            u64::from(retention_days) * 24 * 60 * 60,
        ))
        .unwrap_or(std::time::SystemTime::UNIX_EPOCH);

    let Ok(entries) = std::fs::read_dir(log_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let is_old_log = entry.file_type().is_ok_and(|t| t.is_file())
            && entry
                .metadata()
                .and_then(|m| m.modified())
                .is_ok_and(|modified| modified < cutoff);
        if is_old_log {
            if let Err(err) = std::fs::remove_file(entry.path()) {
                eprintln!(
                    "falha ao remover log antigo {}: {err}",
                    entry.path().display()
                );
            }
        }
    }
}

#[cfg(test)]
mod prune_old_logs_tests {
    use super::prune_old_logs;

    fn touch_with_age(path: &std::path::Path, days_old: u64) {
        std::fs::write(path, b"log").unwrap();
        let old_time = filetime::FileTime::from_system_time(
            std::time::SystemTime::now() - std::time::Duration::from_secs(days_old * 24 * 60 * 60),
        );
        filetime::set_file_mtime(path, old_time).unwrap();
    }

    #[test]
    fn remove_apenas_logs_mais_antigos_que_a_retencao() {
        let tmp = tempfile::tempdir().unwrap();
        let old_log = tmp.path().join("slot2sync.log.2020-01-01");
        let recent_log = tmp.path().join("slot2sync.log.2026-01-01");
        touch_with_age(&old_log, 30);
        touch_with_age(&recent_log, 1);

        prune_old_logs(tmp.path(), 7);

        assert!(!old_log.exists());
        assert!(recent_log.exists());
    }

    #[test]
    fn diretorio_ausente_nao_causa_panico() {
        prune_old_logs(std::path::Path::new("/nao/existe/mesmo"), 7);
    }
}
