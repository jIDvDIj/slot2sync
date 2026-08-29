//! Inicialização exclusiva do ambiente desktop: bandeja do sistema, watcher de
//! processos, janela inicial e autostart padrão.
//!
//! Tudo aqui é guardado por `#[cfg(desktop)]` no módulo pai — nunca é
//! compilado em builds mobile (Android / iOS).

use std::sync::Arc;

use tauri::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{App, AppHandle, Manager, WindowEvent};
use tauri_plugin_autostart::ManagerExt;

use crate::constants;
use crate::events::bus::EventBus;
use crate::shutdown::ShutdownHandle;
use crate::storage::db::Db;
use crate::sync::SyncEngine;

/// Ponto de entrada: configura tudo que é desktop-only após o setup comum.
pub fn setup(
    app: &mut App,
    db: Db,
    engine: Arc<SyncEngine>,
    shutdown: ShutdownHandle,
    bus: EventBus,
) -> Result<(), Box<dyn std::error::Error>> {
    setup_tray(app.handle())?;
    maybe_show_window(app.handle());
    let running = crate::watcher::RunningEmulators::default();
    start_watcher(
        db.clone(),
        engine.clone(),
        bus,
        running.clone(),
        shutdown.clone(),
    );
    start_scheduled_scan(
        db.clone(),
        engine.clone(),
        running.clone(),
        shutdown.clone(),
    );
    crate::watcher::fs_watcher::start(db.clone(), engine, running, shutdown);
    setup_default_autostart(app.handle().clone(), db);
    Ok(())
}

/// Handler de `CloseRequested` que esconde a janela em vez de encerrá-la —
/// o app segue vivo na bandeja. Registrado via `.on_window_event()` no Builder.
pub fn on_close_requested(window: &tauri::Window, event: &WindowEvent) {
    if let WindowEvent::CloseRequested { api, .. } = event {
        if window.label() == constants::MAIN_WINDOW_LABEL {
            api.prevent_close();
            let _ = window.hide();
        }
    }
}

fn setup_tray(app: &AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    let open = MenuItem::with_id(app, constants::TRAY_MENU_OPEN, "Open", true, None::<&str>)?;
    let sync = MenuItem::with_id(
        app,
        constants::TRAY_MENU_SYNC,
        "Sync now",
        true,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, constants::TRAY_MENU_QUIT, "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[&open, &sync, &PredefinedMenuItem::separator(app)?, &quit],
    )?;

    let icon = app
        .default_window_icon()
        .cloned()
        .ok_or("ícone padrão da janela ausente")?;

    TrayIconBuilder::with_id("slot2sync-tray")
        .icon(icon)
        .tooltip("Slot2Sync")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(on_tray_menu_event)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        })
        .build(app)?;

    Ok(())
}

fn on_tray_menu_event(app: &AppHandle, event: MenuEvent) {
    let id = event.id.as_ref();
    if id == constants::TRAY_MENU_OPEN {
        show_main_window(app);
    } else if id == constants::TRAY_MENU_SYNC {
        spawn_sync(app.clone(), constants::TRIGGER_MANUAL, false);
    } else if id == constants::TRAY_MENU_QUIT {
        spawn_sync(app.clone(), constants::TRIGGER_SHUTDOWN, true);
    }
}

pub(crate) fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(constants::MAIN_WINDOW_LABEL) {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// Dispara um sync bidirecional em background. Se `then_exit`, encerra o app
/// ao terminar — é o sync de despedida do menu "Sair".
fn spawn_sync(app: AppHandle, trigger: &'static str, then_exit: bool) {
    use std::time::Duration;

    use crate::state::AppState;
    use crate::sync::SyncDirection;
    tauri::async_runtime::spawn(async move {
        let engine = app.state::<AppState>().engine.clone();
        if let Err(err) = engine.sync_all(SyncDirection::Bidirectional, trigger).await {
            tracing::warn!(trigger, error = %err, "sync acionado pela bandeja falhou");
        }
        if then_exit {
            let db = app.state::<AppState>().db.clone();
            if let Err(err) = db.run_maintenance_if_due().await {
                tracing::warn!(error = %err, "manutenção do SQLite no shutdown falhou");
            }
            // Sinaliza o cancelamento e espera as tasks longas drenarem antes
            // de derrubar o processo — sem isso, `exit` interrompia o watcher
            // e qualquer sync ainda em curso no meio de uma transferência.
            let shutdown = app.state::<AppState>().shutdown.clone();
            let grace = Duration::from_secs(constants::SHUTDOWN_GRACE_SECS);
            if !shutdown.shutdown(grace).await {
                tracing::warn!(
                    segundos = constants::SHUTDOWN_GRACE_SECS,
                    "tasks não terminaram no prazo; encerrando assim mesmo"
                );
            }
            app.exit(0);
        }
    });
}

/// A janela nasce oculta (`visible: false` no tauri.conf.json). Em abertura
/// normal ela é exibida; quando o SO lança o app com `--minimized`
/// (autostart junto com o sistema), fica só na bandeja.
fn maybe_show_window(app: &AppHandle) {
    let launched_minimized = std::env::args().any(|a| a == constants::STARTUP_MINIMIZED_FLAG);
    if !launched_minimized {
        if let Some(window) = app.get_webview_window(constants::MAIN_WINDOW_LABEL) {
            let _ = window.show();
        }
    }
}

fn start_watcher(
    db: Db,
    engine: Arc<SyncEngine>,
    bus: EventBus,
    running: crate::watcher::RunningEmulators,
    shutdown: ShutdownHandle,
) {
    crate::watcher::start(db, engine, bus, running, shutdown);
}

/// Scan periódico em background: a cada `scan_interval_minutes` (com jitter de
/// ±25%, para não sincronizar em hora cheia nem alinhar com outros
/// dispositivos), dispara um sync completo — mas só quando nenhum emulador
/// está rodando. Captura divergências que os gatilhos discretos perdem
/// (cópia manual de saves, hibernação, evento de watcher perdido).
fn start_scheduled_scan(
    db: Db,
    engine: Arc<SyncEngine>,
    running: crate::watcher::RunningEmulators,
    shutdown: ShutdownHandle,
) {
    use rand::Rng;
    let tracker = shutdown.tracker.clone();
    tauri::async_runtime::spawn(tracker.track_future(async move {
        loop {
            if shutdown.token.is_cancelled() {
                return;
            }
            let minutes = db
                .with(crate::storage::settings::scan_interval_minutes)
                .await
                .unwrap_or(constants::SCAN_INTERVAL_MINUTES_DEFAULT);
            if minutes == 0 {
                // Desativado: reconfere a cada minuto se o usuário religou.
                tokio::select! {
                    _ = tokio::time::sleep(std::time::Duration::from_secs(60)) => continue,
                    _ = shutdown.token.cancelled() => return,
                }
            }

            let jitter = rand::rng().random_range(0.75..1.25);
            let delay = std::time::Duration::from_secs_f64(f64::from(minutes) * 60.0 * jitter);
            // Esperas longas (dezenas de minutos) não podem segurar a saída do
            // app: o cancelamento acorda o sleep imediatamente.
            tokio::select! {
                _ = tokio::time::sleep(delay) => {}
                _ = shutdown.token.cancelled() => return,
            }

            let busy = running.lock().map(|set| !set.is_empty()).unwrap_or(false);
            if busy {
                tracing::info!("scan periódico adiado: emulador em execução");
                continue;
            }

            if let Err(err) = engine
                .sync_all(
                    crate::sync::SyncDirection::Bidirectional,
                    constants::TRIGGER_SCHEDULED,
                )
                .await
            {
                tracing::warn!(error = %err, "scan periódico falhou");
            }
        }
    }));
}

/// Na primeiríssima execução registra o autostart para o app subir com o
/// sistema. Depois disso, a escolha do usuário prevalece.
fn setup_default_autostart(app: AppHandle, db: Db) {
    tauri::async_runtime::spawn(async move {
        let already = db
            .with(crate::storage::settings::autostart_initialized)
            .await
            .unwrap_or(true);
        if already {
            return;
        }
        let enabled = app.autolaunch().enable();
        match enabled {
            Ok(()) => {
                let _ = db
                    .with(crate::storage::settings::mark_autostart_initialized)
                    .await;
                tracing::info!("autostart ativado por padrão (primeira execução)");
            }
            Err(err) => {
                tracing::warn!(error = %err, "autostart padrão não pôde ser ativado");
            }
        }
    });
}
