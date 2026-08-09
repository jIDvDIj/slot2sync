mod auth;
mod backups;
mod commands;
mod constants;
mod device;
mod drive;
mod emulator;
mod error;
mod events;
mod games;
mod platform;
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

            let data_dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let db = storage::db::Db::open(&data_dir.join(constants::LOCAL_DB_FILE))?;

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

            let auth = Arc::new(auth::AuthManager::new(http.clone(), secret_store.clone()));
            let drive = Arc::new(drive::DriveClient::new(http, auth.clone(), db.clone()));

            // Storage local: filesystem no desktop; plugin nativo (SAF/bookmarks)
            // no mobile, montado a partir da ponte registrada pelo plugin acima.
            #[cfg(desktop)]
            let storage: Arc<dyn sync::LocalStorage> = Arc::new(sync::DesktopStorage);
            #[cfg(mobile)]
            let storage: Arc<dyn sync::LocalStorage> = sync::mobile_storage::storage(app.handle())?;

            let engine = Arc::new(sync::SyncEngine::new(
                db.clone(),
                drive,
                auth.clone(),
                app.handle().clone(),
                last_sync.clone(),
                data_dir.join(constants::LOCAL_BACKUP_DIR),
                storage.clone(),
                secret_store,
            ));

            app.manage(AppState {
                auth,
                db: db.clone(),
                engine: engine.clone(),
                last_sync,
                storage,
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
            let backups_root = data_dir.join(constants::LOCAL_BACKUP_DIR);
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
            commands::get_auth_status,
            commands::disconnect_google_drive,
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
fn init_logging(app: &tauri::AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let log_dir = app.path().app_log_dir()?;
    std::fs::create_dir_all(&log_dir)?;
    let file_appender = tracing_appender::rolling::daily(&log_dir, "slot2sync.log");

    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(fmt::layer())
        .with(fmt::layer().with_ansi(false).with_writer(file_appender))
        .init();

    Ok(())
}
