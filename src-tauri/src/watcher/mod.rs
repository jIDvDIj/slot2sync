//! Process watcher — detecção de abertura/fechamento dos emuladores.
//!
//! Duas tasks ligadas por um canal `tokio::sync::mpsc`:
//! - **produtor**: loop `tokio::time::interval` (`WATCHER_POLL_INTERVAL_SECS`)
//!   que consulta os processos do SO via `sysinfo` (em `spawn_blocking`) e
//!   publica transições, com debounce contra flapping;
//! - **consumidor**: para cada transição, dispara o sync direcionado e emite
//!   o status ao frontend.
//!
//! Gatilhos:
//! - emulador **abriu** → sync Drive → Local (saves frescos antes do jogo carregar);
//! - emulador **fechou** → sync Local → Drive (sobe os saves da sessão).

pub mod fs_watcher;
mod process_watcher;

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use sysinfo::System;
use tauri::{AppHandle, Emitter};
use tauri_plugin_notification::NotificationExt;
use tokio::sync::mpsc;

use crate::constants::{
    EMULATOR_STOP_SETTLE_MS, TRIGGER_EMULATOR_START, TRIGGER_EMULATOR_STOP,
    WATCHER_POLL_INTERVAL_SECS, WATCHER_STOP_DEBOUNCE_TICKS,
};
use crate::emulator;
use crate::events::EVT_EMULATOR_STATUS;
use crate::shutdown::ShutdownHandle;
use crate::storage::db::Db;
use crate::storage::emulators;
use crate::sync::{SyncDirection, SyncEngine};
use process_watcher::{poll_once, MonitoredEmulator, RunStateTracker};

/// Evento publicado no canal watcher → consumidor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WatcherEvent {
    /// Emulador (nome canônico do perfil) começou a rodar.
    EmulatorStarted(String),
    /// Emulador deixou de rodar.
    EmulatorStopped(String),
}

/// Payload do evento `emulator:status`. (→ ipc.ts)
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct EmulatorStatusEvent {
    emulator: String,
    running: bool,
}

/// Conjunto dos emuladores atualmente em execução, alimentado pelo watcher e
/// consultado por quem só deve agir com tudo parado (scan periódico, watcher
/// de filesystem). `std::sync::Mutex`: locks curtos, sem `await` no meio.
pub type RunningEmulators = Arc<std::sync::Mutex<HashSet<String>>>;

/// Sobe o produtor e o consumidor do watcher. Chamado uma vez no `setup`.
/// Ambas as tasks rodam sob o `tracker` do desligamento e param no
/// cancelamento do `token`.
pub fn start(
    db: Db,
    engine: Arc<SyncEngine>,
    app: AppHandle,
    running: RunningEmulators,
    shutdown: ShutdownHandle,
) {
    let (tx, rx) = mpsc::channel::<WatcherEvent>(32);
    spawn_poll_loop(db.clone(), tx, shutdown.clone());
    spawn_consumer(rx, engine, app, db, running, shutdown);
}

fn spawn_poll_loop(db: Db, tx: mpsc::Sender<WatcherEvent>, shutdown: ShutdownHandle) {
    shutdown.tracker.spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(WATCHER_POLL_INTERVAL_SECS));
        // System e tracker persistem entre ticks; viajam para dentro do
        // `spawn_blocking` a cada poll e voltam com os eventos.
        let mut sys_state: Option<(System, RunStateTracker)> = None;

        loop {
            // O tick é o ponto de parada natural: cancelar aqui evita começar
            // mais um poll do SO durante o desligamento.
            tokio::select! {
                _ = interval.tick() => {}
                _ = shutdown.token.cancelled() => {
                    tracing::debug!("watcher: desligamento sinalizado; polling encerrado");
                    return;
                }
            }

            let profiles = match db.with(emulators::list).await {
                Ok(profiles) => profiles,
                Err(err) => {
                    tracing::warn!(error = %err, "watcher: falha ao listar emuladores configurados");
                    continue;
                }
            };

            let monitored: Vec<MonitoredEmulator> = profiles
                .into_iter()
                .filter_map(|p| {
                    let process_names = emulator::process_names(&p.name);
                    (!process_names.is_empty()).then_some(MonitoredEmulator {
                        name: p.name,
                        process_names,
                    })
                })
                .collect();
            if monitored.is_empty() {
                continue;
            }

            let (mut system, mut tracker) = sys_state.take().unwrap_or_else(|| {
                (
                    System::new(),
                    RunStateTracker::new(WATCHER_STOP_DEBOUNCE_TICKS),
                )
            });

            let joined = tokio::task::spawn_blocking(move || {
                let events = poll_once(&mut system, &mut tracker, &monitored);
                (system, tracker, events)
            })
            .await;

            let (system, tracker, events) = match joined {
                Ok(out) => out,
                Err(err) => {
                    tracing::warn!(error = %err, "watcher: tarefa de polling abortada");
                    continue;
                }
            };
            sys_state = Some((system, tracker));

            for event in events {
                if tx.send(event).await.is_err() {
                    tracing::debug!("watcher: consumidor encerrado; parando o polling");
                    return;
                }
            }
        }
    });
}

fn spawn_consumer(
    mut rx: mpsc::Receiver<WatcherEvent>,
    engine: Arc<SyncEngine>,
    app: AppHandle,
    db: Db,
    running_set: RunningEmulators,
    shutdown: ShutdownHandle,
) {
    shutdown.tracker.spawn(async move {
        loop {
            let event = tokio::select! {
                event = rx.recv() => match event {
                    Some(event) => event,
                    None => return,
                },
                _ = shutdown.token.cancelled() => {
                    tracing::debug!("watcher: desligamento sinalizado; consumidor encerrado");
                    return;
                }
            };
            let (name, running, direction, trigger) = match event {
                WatcherEvent::EmulatorStarted(name) => (
                    name,
                    true,
                    SyncDirection::DriveToLocal,
                    TRIGGER_EMULATOR_START,
                ),
                WatcherEvent::EmulatorStopped(name) => (
                    name,
                    false,
                    SyncDirection::LocalToDrive,
                    TRIGGER_EMULATOR_STOP,
                ),
            };

            // Mantém o conjunto compartilhado em dia — consultado pelo scan
            // periódico para não sincronizar com um jogo aberto.
            if let Ok(mut set) = running_set.lock() {
                if running {
                    set.insert(name.clone());
                } else {
                    set.remove(&name);
                }
            }

            // O status sempre é emitido (a UI mostra "em execução"); só o sync
            // automático respeita o gatilho desativado pelo usuário.
            let _ = app.emit(
                EVT_EMULATOR_STATUS,
                &EmulatorStatusEvent {
                    emulator: name.clone(),
                    running,
                },
            );
            tracing::info!(emulador = %name, running, trigger, "transição de emulador detectada");

            let settings = db
                .with(crate::storage::settings::load)
                .await
                .unwrap_or_default();

            // Notificação "emulador detectado" (só na abertura, nível `all`).
            if running && settings.notification_level.notifies_info() {
                if let Err(err) = app
                    .notification()
                    .builder()
                    .title("Slot2Sync")
                    .body(format!("Emulador detectado: {name}"))
                    .show()
                {
                    tracing::debug!(error = %err, "não foi possível exibir notificação nativa");
                }
            }

            let enabled = if running {
                settings.triggers.emulator_start
            } else {
                settings.triggers.emulator_stop
            };
            if !enabled {
                tracing::info!(emulador = %name, trigger, "gatilho desativado; sync automático ignorado");
                continue;
            }

            // Settle delay: o processo já saiu, mas o SO pode ainda estar
            // liberando os arquivos da sessão — espera antes de escanear.
            if !running {
                tokio::time::sleep(Duration::from_millis(EMULATOR_STOP_SETTLE_MS)).await;
            }

            if let Err(err) = engine.sync_emulator(&name, direction, trigger).await {
                tracing::warn!(emulador = %name, error = %err, "sync disparado pelo watcher falhou");
            }
        }
    });
}
