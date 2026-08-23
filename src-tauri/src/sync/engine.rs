//! `SyncEngine` — orquestração da sincronização bidirecional.
//!
//! Agnóstico a emuladores: opera sobre `SyncTarget` (rótulo + listas de
//! caminhos). Por categoria: garante as pastas no Drive, lista a árvore
//! remota, varre o estado local, monta o plano via `diff`/`conflict` e
//! executa as transferências com concorrência limitada, emitindo progresso
//! ao frontend. Falhas de rede/arquivo em uso vão para a fila offline.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::stream::{self, StreamExt};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Runtime, Wry};
use tauri_plugin_notification::NotificationExt;
use tokio::sync::{Mutex, Semaphore};
use tokio_util::sync::CancellationToken;

use super::conflict::{SyncAction, TIMESTAMP_TOLERANCE_MS};
use super::diff::{self, CategoryPlan, LocalFile, PlannedOp};
use super::storage::{FileLoc, LocalStorage};
use super::{SyncCategory, SyncDirection, SyncProgress, SyncTarget};
use crate::constants::{
    DRIVE_BATCH_MAX_OPS, DRIVE_BATCH_MIN_OPS, DRIVE_MANIFEST_FILE, DRIVE_MAX_CONCURRENT_TRANSFERS,
    DRIVE_SIMPLE_UPLOAD_MAX_BYTES, MAX_BYTES_IN_FLIGHT, MAX_DISK_WRITES, MAX_NETWORK_OPS,
};
use crate::error::{AppError, AppResult};
use crate::events::{
    EVT_SYNC_CANCELLED, EVT_SYNC_COMPLETED, EVT_SYNC_CONFLICT, EVT_SYNC_ERROR, EVT_SYNC_PROGRESS,
    EVT_SYNC_STARTED, EVT_SYNC_STATE_CHANGED,
};
use crate::remote::{BatchUploadOp, DeviceTag, RemoteProvider};
use crate::storage::conflicts::{self, Conflict};
use crate::storage::db::Db;
use crate::storage::manifest::{self, ManifestEntry, FLAG_CONFLICT, FLAG_PENDING};
use crate::storage::mtime_overrides;
use crate::storage::settings::{self, NotificationLevel};
use crate::storage::{emulators, queue, stats};
use crate::versioning::Versioner;

/// Resultado agregado de um sync. (→ ipc.ts)
#[derive(Debug, Default, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncSummary {
    pub uploaded: u32,
    pub downloaded: u32,
    pub skipped: u32,
    pub failed: u32,
    pub queued: u32,
    /// Arquivos locais copiados para backup antes de serem sobrescritos no
    /// primeiro sync. `> 0` sinaliza à UI que há backups a oferecer.
    pub backed_up: u32,
    /// Conflitos detectados neste sync (ambos os lados mudaram).
    pub conflicts: u32,
    /// Renomeações detectadas por hash e aplicadas no Drive sem retransferir.
    pub renamed: u32,
    /// Operações que o desligamento do app cancelou antes de começarem. `> 0`
    /// significa que este sync ficou incompleto de propósito.
    pub cancelled: u32,
    pub duration_ms: u64,
}

impl SyncSummary {
    fn merge(&mut self, other: &SyncSummary) {
        self.uploaded += other.uploaded;
        self.downloaded += other.downloaded;
        self.skipped += other.skipped;
        self.failed += other.failed;
        self.queued += other.queued;
        self.backed_up += other.backed_up;
        self.conflicts += other.conflicts;
        self.renamed += other.renamed;
        self.cancelled += other.cancelled;
    }
}

/// Estado corrente do `SyncEngine`, visto de fora (tray, badge do app). O
/// frontend renderiza a partir disto e de [`EVT_SYNC_STATE_CHANGED`] em vez
/// de acumular os eventos discretos (`sync:started`/`progress`/`completed`/
/// `conflict`/`error`) — reconectar no meio de um sync já chega com o estado
/// certo, sem precisar ter visto os eventos anteriores.
///
/// Não é um estado travado por emulador: reflete a execução de `sync_all`
/// como um todo. `Conflict`/`Error` são transições momentâneas (emitidas
/// quando acontecem) — o sync continua para os demais emuladores da mesma
/// leva, então o estado segue para `Scanning`/`Syncing` do próximo alvo, ou
/// `Idle` ao final da leva.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncState {
    Idle,
    Scanning,
    Syncing,
    Conflict,
    Error(String),
}

impl SyncState {
    pub fn as_str(&self) -> &'static str {
        match self {
            SyncState::Idle => "idle",
            SyncState::Scanning => "scanning",
            SyncState::Syncing => "syncing",
            SyncState::Conflict => "conflict",
            SyncState::Error(_) => "error",
        }
    }
}

/// Payload do evento `sync:state-changed`. (→ ipc.ts)
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStateChanged {
    pub from: &'static str,
    pub to: &'static str,
    pub emulator: Option<String>,
    /// Só preenchido quando `to == "error"`.
    pub error_message: Option<String>,
}

/// Payload do evento `sync:started`. (→ ipc.ts)
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStarted {
    pub trigger: String,
    pub direction: SyncDirection,
}

/// Payload do evento `sync:error`. (→ ipc.ts)
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncError {
    pub emulator: Option<String>,
    pub message: String,
}

/// Entrada do histórico de erros em memória (`SyncEngine::recent_errors`),
/// exposto via `get_recent_errors`. (→ ipc.ts)
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorEntry {
    pub at_ms: i64,
    pub emulator: Option<String>,
    pub message: String,
}

/// Resumo do último sync concluído, exposto à UI via `get_last_sync` (e
/// atualizado ao vivo pelo evento `sync:completed`). (→ ipc.ts)
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LastSync {
    pub at_ms: i64,
    pub trigger: String,
    pub summary: SyncSummary,
}

/// Célula compartilhada entre o `SyncEngine` (escreve) e o `AppState`
/// (lê via comando). `std::sync::Mutex` basta: o lock é curto e sem `await`.
pub type LastSyncStore = Arc<std::sync::Mutex<Option<LastSync>>>;

enum OpOutcome {
    /// A entrada do manifest vem junto para ser gravada em lote depois do
    /// `buffer_unordered` da categoria, em vez de um `upsert` por arquivo.
    Uploaded(Option<ManifestEntry>),
    Downloaded(Option<ManifestEntry>),
    /// Download que também gerou um backup local (primeiro sync).
    DownloadedWithBackup(Option<ManifestEntry>),
    /// Conflito registrado; nenhuma transferência feita.
    Conflicted,
    Queued,
    Failed,
    /// Op abandonada porque o app está encerrando.
    Cancelled,
}

/// Escolha do usuário ao resolver um conflito. (→ ipc.ts)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConflictResolution {
    /// Manter a versão local e enviá-la ao provedor remoto.
    Local,
    /// Manter a versão remota e baixá-la (com backup do local).
    Remote,
}

struct CategoryCtx {
    emulator: String,
    category: SyncCategory,
    direction: SyncDirection,
    /// Pasta da categoria no Drive e sua chave de cache.
    folder_id: String,
    folder_key: String,
    /// Destino de downloads de arquivos que ainda não existem localmente
    /// (primeira pasta-base da categoria).
    download_base: FileLoc,
    /// Pasta onde gravar backups locais desta categoria neste sync
    /// (`<backup_dir>/<emulador>/<timestamp>/<categoria>`).
    backup_base: FileLoc,
    /// Nome amigável deste dispositivo (marca a origem nos uploads e exibido
    /// nos conflitos).
    device: Option<String>,
    /// ID estável deste dispositivo (estampado nos uploads; alimenta a detecção
    /// de conflito entre dispositivos no primeiro sync).
    device_id: Option<String>,
    /// Nível de notificação vigente (gating da notificação de conflito).
    notif: NotificationLevel,
    /// Máximo de versões arquivadas por arquivo no histórico pré-download.
    max_versions: usize,
    total: u32,
    completed: AtomicU32,
    /// Total de bytes do plano e bytes já concluídos — para a UI mostrar
    /// progresso em bytes, velocidade e ETA (não só contagem de arquivos).
    bytes_total: u64,
    bytes_done: AtomicU64,
    /// Nome do último arquivo concluído — lido pelo ticker de progresso
    /// (`emit_progress_snapshot`), já que múltiplas transferências rodam
    /// concorrentemente e não há um "arquivo atual" único a qualquer momento.
    last_file: std::sync::Mutex<String>,
}

/// Genérico sobre o runtime do Tauri para ser testável: em produção é o `Wry`
/// (default); nos testes de cenário (`sync::scenarios`), o `MockRuntime` do
/// `tauri::test`. O storage remoto entra pelo trait [`RemoteProvider`] —
/// `DriveClient`/`DropboxClient`/`OneDriveClient`/`FolderProvider` reais ou
/// `MockDrive` em memória. Trocável em tempo de execução (troca de provedor
/// sem reiniciar o app): `None` antes da primeira conexão.
pub struct SyncEngine<R: Runtime = Wry> {
    db: Db,
    remote_provider: std::sync::RwLock<Option<Arc<dyn RemoteProvider>>>,
    app: AppHandle<R>,
    last_sync: LastSyncStore,
    /// Raiz dos backups locais (`<app_data>/backups`).
    backup_dir: PathBuf,
    /// Histórico de versões pré-download (`<backups>/<emulador>/history/`).
    versioner: Arc<crate::versioning::FsVersioner>,
    /// Acesso ao armazenamento local de saves (filesystem no desktop; SAF /
    /// bookmarks no mobile, futuramente). Todo o I/O local passa por aqui.
    storage: Arc<dyn LocalStorage>,
    /// Leitura do device_id estável para auditoria de conflitos.
    secrets: Arc<dyn crate::secrets::SecretStore>,
    /// Serializa execuções: um sync por vez, os demais aguardam.
    running: Mutex<()>,
    /// Arquivos gravados por downloads recentes (caminho nativo → instante).
    /// O watcher de filesystem consulta para não reagir às próprias escritas
    /// do sync (anti-loop). Entradas expiram após `RECENT_DOWNLOAD_TTL_SECS`.
    recent_downloads: std::sync::Mutex<std::collections::HashMap<PathBuf, Instant>>,
    /// Limita chamadas de rede simultâneas (upload/download com o provedor
    /// remoto), separado do limite de I/O de disco — são recursos diferentes
    /// e não faz sentido que uma escrita local lenta seja bloqueada por uma
    /// chamada de API em andamento, nem o contrário.
    network_ops: Semaphore,
    /// Limita I/O de disco local simultâneo (leitura em `do_upload`, escrita
    /// em `do_download`). Em HDD, escritas sequenciais são mais rápidas que
    /// paralelas — um teto baixo evita thrashing de cabeça de leitura/escrita.
    disk_io: Semaphore,
    /// Estado corrente exposto ao frontend (ver [`SyncState`]).
    current_state: std::sync::Mutex<(SyncState, Option<String>)>,
    /// Histórico de erros em memória (mais recente por último), exposto via
    /// `get_recent_errors`/`clear_errors`. Perdido a cada reinício do app —
    /// não é persistido, é só um retrato rápido pra diagnóstico.
    recent_errors: std::sync::Mutex<std::collections::VecDeque<ErrorEntry>>,
    /// Sinaliza que o app está encerrando. Consultado antes de cada operação
    /// do plano: cancelado, o restante do plano é abandonado em vez de ser
    /// interrompido no meio de uma transferência. Compartilhado com o watcher
    /// e as demais tasks longas via `state::AppState::shutdown`.
    cancel: CancellationToken,
}

impl<R: Runtime> SyncEngine<R> {
    // Construtor de injeção: recebe o wiring completo do app montado no setup.
    // `remote_provider` entra vazio quando nenhum provedor foi configurado
    // ainda (primeira execução) — preenchido depois via `set_remote_provider`.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        db: Db,
        remote_provider: Option<Arc<dyn RemoteProvider>>,
        app: AppHandle<R>,
        last_sync: LastSyncStore,
        backup_dir: PathBuf,
        storage: Arc<dyn LocalStorage>,
        secrets: Arc<dyn crate::secrets::SecretStore>,
    ) -> Self {
        Self {
            db,
            remote_provider: std::sync::RwLock::new(remote_provider),
            app,
            last_sync,
            versioner: Arc::new(crate::versioning::FsVersioner::new(backup_dir.clone())),
            backup_dir,
            storage,
            secrets,
            running: Mutex::new(()),
            recent_downloads: std::sync::Mutex::new(std::collections::HashMap::new()),
            network_ops: Semaphore::new(MAX_NETWORK_OPS),
            disk_io: Semaphore::new(MAX_DISK_WRITES),
            current_state: std::sync::Mutex::new((SyncState::Idle, None)),
            recent_errors: std::sync::Mutex::new(std::collections::VecDeque::new()),
            cancel: CancellationToken::new(),
        }
    }

    /// Clone do token de cancelamento do engine. O `setup` usa este mesmo
    /// token para montar o `ShutdownHandle`, de modo que cancelar o
    /// desligamento também interrompa o sync em andamento.
    pub fn cancel_token(&self) -> CancellationToken {
        self.cancel.clone()
    }

    /// Retrato do histórico de erros em memória, mais antigo primeiro.
    pub fn recent_errors(&self) -> Vec<ErrorEntry> {
        self.recent_errors.lock().unwrap().iter().cloned().collect()
    }

    /// Esvazia o histórico de erros (ação "limpar" da UI de diagnóstico).
    pub fn clear_errors(&self) {
        self.recent_errors.lock().unwrap().clear();
    }

    /// Registra uma entrada no histórico de erros, descartando a mais antiga
    /// se já estiver no teto (`MAX_RECENT_ERRORS`).
    fn record_error(&self, emulator: Option<&str>, message: String) {
        let mut buf = self.recent_errors.lock().unwrap();
        if buf.len() >= crate::constants::MAX_RECENT_ERRORS {
            buf.pop_front();
        }
        buf.push_back(ErrorEntry {
            at_ms: chrono::Utc::now().timestamp_millis(),
            emulator: emulator.map(str::to_string),
            message,
        });
    }

    /// Estado corrente do engine e, se houver, o emulador associado — usado
    /// pelo comando `get_sync_state` para o frontend renderizar o estado
    /// certo ao reconectar no meio de um sync, sem depender de ter recebido
    /// os eventos anteriores.
    pub fn current_sync_state(&self) -> (SyncState, Option<String>) {
        self.current_state.lock().unwrap().clone()
    }

    /// Muda o estado corrente e emite `sync:state-changed`. Sem efeito
    /// (não emite) se `to` for igual ao estado atual E o emulador não mudar —
    /// evita ruído de eventos idênticos repetidos.
    fn transition(&self, to: SyncState, emulator: Option<&str>) {
        let mut guard = self.current_state.lock().unwrap();
        let (from, from_emulator) = &*guard;
        if *from == to && from_emulator.as_deref() == emulator {
            return;
        }
        let from_str = from.as_str();
        let to_str = to.as_str();
        let error_message = match &to {
            SyncState::Error(msg) => Some(msg.clone()),
            _ => None,
        };
        *guard = (to, emulator.map(str::to_string));
        drop(guard);

        let _ = self.app.emit(
            EVT_SYNC_STATE_CHANGED,
            &SyncStateChanged {
                from: from_str,
                to: to_str,
                emulator: emulator.map(str::to_string),
                error_message,
            },
        );
    }

    /// Troca o provedor de storage ativo (conectar pela primeira vez ou mudar
    /// de provedor) — efetivo imediatamente, sem reiniciar o app. `None`
    /// desconecta (o próximo sync automático falha graciosamente até um novo
    /// provedor ser configurado).
    pub fn set_remote_provider(&self, provider: Option<Arc<dyn RemoteProvider>>) {
        *self.remote_provider.write().unwrap() = provider;
    }

    /// Provedor remoto ativo, ou erro se nenhum foi configurado ainda —
    /// gatilhos automáticos (startup/watcher) caem aqui de forma graciosa
    /// antes do primeiro login, sem crashar.
    fn remote(&self) -> AppResult<Arc<dyn RemoteProvider>> {
        self.remote_provider.read().unwrap().clone().ok_or_else(|| {
            AppError::Auth("nenhum provedor de storage conectado — sync ignorado".into())
        })
    }

    /// Registra que `loc` acabou de ser gravado por um download (anti-loop do
    /// watcher de filesystem). Também expira entradas antigas de passagem.
    fn mark_recent_download(&self, loc: &FileLoc) {
        let Some(path) = loc.as_native_path() else {
            return;
        };
        let ttl = std::time::Duration::from_secs(crate::constants::RECENT_DOWNLOAD_TTL_SECS);
        if let Ok(mut map) = self.recent_downloads.lock() {
            let now = Instant::now();
            map.retain(|_, at| now.duration_since(*at) < ttl);
            map.insert(path.to_path_buf(), now);
        }
    }

    /// O caminho foi gravado por um download nos últimos
    /// `RECENT_DOWNLOAD_TTL_SECS`? Consultado pelo watcher de filesystem.
    #[cfg(desktop)]
    pub fn is_recent_download(&self, path: &Path) -> bool {
        let ttl = std::time::Duration::from_secs(crate::constants::RECENT_DOWNLOAD_TTL_SECS);
        self.recent_downloads
            .lock()
            .map(|map| {
                map.get(path)
                    .is_some_and(|at| Instant::now().duration_since(*at) < ttl)
            })
            .unwrap_or(false)
    }

    /// Acesso ao armazenamento local — usado pela detecção automática mobile
    /// (`commands::detect_emulator_mobile`), que precisa checar existência de
    /// pastas via SAF fora do fluxo normal de sync.
    #[cfg(mobile)]
    pub fn storage(&self) -> &Arc<dyn LocalStorage> {
        &self.storage
    }

    /// Sincroniza todos os emuladores configurados.
    pub async fn sync_all(
        &self,
        direction: SyncDirection,
        trigger: &str,
    ) -> AppResult<SyncSummary> {
        self.sync_filtered(None, direction, trigger).await
    }

    /// Zera o cache de pastas do provedor remoto ativo, se houver (memória +
    /// SQLite). Chamado no logout para não reaproveitar IDs de outra conta.
    pub async fn clear_folder_cache(&self) {
        let remote = self.remote_provider.read().unwrap().clone();
        if let Some(remote) = remote {
            remote.clear_folder_cache().await;
        }
    }

    /// Sincroniza um único emulador (gatilhos do process watcher).
    /// Só-desktop: no mobile não há watcher para acionar sync por emulador.
    #[cfg(desktop)]
    pub async fn sync_emulator(
        &self,
        name: &str,
        direction: SyncDirection,
        trigger: &str,
    ) -> AppResult<SyncSummary> {
        self.sync_filtered(Some(name), direction, trigger).await
    }

    async fn sync_filtered(
        &self,
        only: Option<&str>,
        direction: SyncDirection,
        trigger: &str,
    ) -> AppResult<SyncSummary> {
        let _guard = self.running.lock().await;

        // Garante cedo (antes de qualquer I/O) que há um provedor conectado —
        // mesmo erro tipado que as demais operações usam quando chamadas sem
        // provedor, então os gatilhos automáticos (startup/watcher) já sabem
        // tratar isso de forma graciosa.
        self.remote()?;

        let notif = self
            .db
            .with(settings::notification_level)
            .await
            .unwrap_or_default();
        let device = self
            .db
            .with(settings::device_name)
            .await
            .unwrap_or_default();
        // ID estável deste dispositivo (keyring), lido uma vez por sync. `None`
        // se o keyring estiver indisponível — desliga só a detecção de conflito
        // entre dispositivos nesta execução.
        let device_id = crate::device::current(self.secrets.clone()).await;
        // Máximo de versões arquivadas por arquivo (histórico pré-download).
        let max_versions =
            self.db
                .with(settings::max_backup_versions)
                .await
                .unwrap_or(crate::constants::MAX_BACKUP_VERSIONS_DEFAULT) as usize;

        let profiles = self.db.with(emulators::list).await?;
        // Por emulador: monta o target e remove as categorias que o usuário
        // desativou nas configurações (default: todas ativas).
        let mut targets: Vec<SyncTarget> = Vec::new();
        for profile in profiles
            .iter()
            .filter(|p| only.is_none_or(|name| p.name == name))
        {
            let name = profile.name.clone();
            let cats = self
                .db
                .with(move |conn| emulators::get_categories(conn, &name))
                .await?;
            let mut target = SyncTarget::from_profile(profile);
            target.categories.retain(|(category, _)| match category {
                SyncCategory::Saves => cats.saves,
                SyncCategory::Savestates => cats.savestates,
                SyncCategory::Config => cats.config,
            });
            targets.push(target);
        }
        if targets.is_empty() {
            tracing::info!(trigger, "nenhum emulador configurado; nada a sincronizar");
            return Ok(SyncSummary::default());
        }

        let started_at = Instant::now();
        tracing::info!(
            trigger,
            ?direction,
            emuladores = targets.len(),
            "sync iniciado"
        );
        let _ = self.app.emit(
            EVT_SYNC_STARTED,
            &SyncStarted {
                trigger: trigger.to_string(),
                direction,
            },
        );
        self.transition(SyncState::Scanning, None);

        // Rótulo desta execução, usado para agrupar os backups locais do
        // primeiro sync numa pasta por sync.
        let run_stamp = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string();

        let mut summary = SyncSummary::default();
        for target in &targets {
            // Emulador com conflito pendente fica bloqueado até o usuário
            // resolver — nem manual nem automático sincroniza.
            let name = target.label.clone();
            let blocked = self
                .db
                .with(move |conn| conflicts::has_for_emulator(conn, &name))
                .await
                .unwrap_or(false);
            if blocked {
                tracing::info!(emulador = %target.label, "conflito pendente; sync do emulador bloqueado");
                continue;
            }

            // Cancelado no meio da lista: não vale escanear o próximo
            // emulador, todas as ops do plano seriam descartadas de qualquer
            // forma. Os já processados continuam contabilizados no summary.
            if self.cancel.is_cancelled() {
                tracing::info!(
                    emulador = %target.label,
                    "desligamento em curso; emuladores restantes não serão sincronizados"
                );
                break;
            }

            self.transition(SyncState::Scanning, Some(&target.label));

            match self
                .sync_target(
                    target,
                    direction,
                    &run_stamp,
                    device.as_deref(),
                    device_id.as_deref(),
                    notif,
                    max_versions,
                )
                .await
            {
                Ok(partial) => {
                    summary.merge(&partial);
                    let (emulator, at) =
                        (target.label.clone(), chrono::Utc::now().timestamp_millis());
                    let _ = self
                        .db
                        .with(move |conn| stats::touch_last_sync(conn, &emulator, at))
                        .await;
                }
                Err(err) => {
                    summary.failed += 1;
                    tracing::error!(emulador = %target.label, error = %err, "sync do emulador falhou");
                    self.transition(SyncState::Error(err.to_string()), Some(&target.label));
                    self.record_error(Some(&target.label), err.to_string());
                    let _ = self.app.emit(
                        EVT_SYNC_ERROR,
                        &SyncError {
                            emulator: Some(target.label.clone()),
                            message: err.to_string(),
                        },
                    );
                    if notif.notifies_errors() {
                        self.notify_error(&target.label, &err.to_string());
                    }
                }
            }
        }

        self.transition(SyncState::Idle, None);

        if let Err(err) = self.publish_manifest_snapshot().await {
            tracing::warn!(error = %err, "falha ao publicar sync_manifest.json no Drive");
        }

        summary.duration_ms = started_at.elapsed().as_millis() as u64;
        tracing::info!(?summary, trigger, "sync concluído");

        let last = LastSync {
            at_ms: chrono::Utc::now().timestamp_millis(),
            trigger: trigger.to_string(),
            summary: summary.clone(),
        };
        if let Ok(mut guard) = self.last_sync.lock() {
            *guard = Some(last);
        }

        // Notifica a conclusão só quando houve transferência — evita "sync
        // concluído" repetido em syncs automáticos que nada fizeram.
        if notif.notifies_info() && (summary.uploaded + summary.downloaded > 0) {
            self.notify_completed(&summary);
        }

        // Cancelado: o front precisa distinguir "terminou" de "foi
        // interrompido pela saída do app" — os dois eventos são emitidos, o
        // `sync:cancelled` primeiro para chegar antes do `completed`.
        if self.cancel.is_cancelled() {
            let _ = self.app.emit(EVT_SYNC_CANCELLED, &summary);
        }
        let _ = self.app.emit(EVT_SYNC_COMPLETED, &summary);
        Ok(summary)
    }

    /// Notificação nativa do SO de sync concluído (nível `all`).
    fn notify_completed(&self, summary: &SyncSummary) {
        if let Err(err) = self
            .app
            .notification()
            .builder()
            .title("Slot2Sync — sincronização concluída")
            .body(format!(
                "↑ {} enviados · ↓ {} baixados",
                summary.uploaded, summary.downloaded
            ))
            .show()
        {
            tracing::debug!(error = %err, "não foi possível exibir notificação nativa");
        }
    }

    /// Notificação nativa do SO de conflito (gated pelo nível de notificação).
    fn notify_conflict(&self, emulator: &str, rel_path: &str) {
        if let Err(err) = self
            .app
            .notification()
            .builder()
            .title("Slot2Sync — conflito de sincronização")
            .body(format!(
                "{emulator}: \"{rel_path}\" mudou nos dois lados. Resolva no app."
            ))
            .show()
        {
            tracing::debug!(error = %err, "não foi possível exibir notificação nativa");
        }
    }

    /// Notificação nativa do SO para erro crítico de sync. Útil quando o
    /// gatilho é automático (startup/watcher/shutdown) e a janela está oculta.
    fn notify_error(&self, emulator: &str, message: &str) {
        if let Err(err) = self
            .app
            .notification()
            .builder()
            .title("Slot2Sync — falha na sincronização")
            .body(format!("{emulator}: {message}"))
            .show()
        {
            tracing::debug!(error = %err, "não foi possível exibir notificação nativa");
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn sync_target(
        &self,
        target: &SyncTarget,
        direction: SyncDirection,
        run_stamp: &str,
        device: Option<&str>,
        device_id: Option<&str>,
        notif: NotificationLevel,
        max_versions: usize,
    ) -> AppResult<SyncSummary> {
        let mut summary = SyncSummary::default();

        // Carimbo do início do scan deste emulador (estatísticas).
        let (emulator, now_ms) = (target.label.clone(), chrono::Utc::now().timestamp_millis());
        let _ = self
            .db
            .with(move |conn| stats::touch_last_scan(conn, &emulator, now_ms))
            .await;

        // Restos de um download interrompido por queda entre a escrita e o
        // rename atômico (ver `TMP_SUFFIX`) — best-effort, uma vez por
        // emulador, antes de qualquer scan.
        let all_bases: Vec<PathBuf> = target
            .categories
            .iter()
            .flat_map(|(_, bases)| bases.iter().cloned())
            .collect();
        self.storage
            .cleanup_orphaned_temp_files(&target.root, &all_bases)
            .await;

        // Padrões de exclusão do emulador, compilados uma vez por sync.
        let exclude = super::build_exclude_set(&target.exclude_patterns);

        for (category, bases) in &target.categories {
            if bases.is_empty() {
                continue;
            }

            let remote_provider = self.remote()?;
            let mut folder_id = remote_provider
                .ensure_category_folder(&target.label, *category)
                .await?;
            let folder_key = format!(
                "{}/{}/{}",
                crate::constants::DRIVE_ROOT_FOLDER,
                target.label,
                category.as_str()
            );

            let remote = match remote_provider.list_tree(&folder_id).await {
                Ok(remote) => remote,
                Err(AppError::RemoteObjectNotFound(detail)) => {
                    // ID/path de pasta cacheado ficou obsoleto (pasta movida/
                    // apagada no provedor remoto). Invalida a subárvore e
                    // re-resolve — reencontra a existente ou recria.
                    tracing::warn!(
                        emulador = %target.label,
                        categoria = category.as_str(),
                        %detail,
                        "pasta da categoria não encontrada no provedor remoto; invalidando cache e re-resolvendo"
                    );
                    remote_provider.invalidate_folder_path(&folder_key).await;
                    folder_id = remote_provider
                        .ensure_category_folder(&target.label, *category)
                        .await?;
                    remote_provider.list_tree(&folder_id).await?
                }
                Err(err) => return Err(err),
            };

            let mut local = self.storage.scan(&target.root, bases).await?;

            // Camada de mtime virtual: em FAT32 o mtime que o download gravou
            // não é o que o disco guardou. Onde há override válido, o diff
            // passa a ver o valor lógico e não conclui "mudou" por causa do
            // arredondamento do filesystem.
            self.apply_mtime_overrides(&target.label, *category, &mut local)
                .await;

            // Exclusões: arquivos que casam com os padrões do emulador ficam
            // fora do sync nas duas direções (nem sobem nem descem).
            let mut remote = remote;
            if let Some(set) = &exclude {
                let before = local.len() + remote.len();
                local.retain(|f| !set.is_match(&f.rel_path));
                remote.retain(|f| !set.is_match(&f.rel_path));
                let excluded = before - (local.len() + remote.len());
                if excluded > 0 {
                    tracing::debug!(
                        emulador = %target.label,
                        categoria = category.as_str(),
                        arquivos = excluded,
                        "arquivos ignorados pelos padrões de exclusão"
                    );
                }
            }

            let (emulator, cat) = (target.label.clone(), *category);
            let manifest_entries = self
                .db
                .with(move |conn| manifest::list_for_category(conn, &emulator, cat))
                .await?;

            // Pré-filtro de mtime: calcula o SHA-256 apenas dos arquivos cujo
            // mtime divergiu da âncora do manifest — nunca sem essa divergência.
            let local = self.hash_touched_files(local, &manifest_entries).await;

            let CategoryPlan {
                ops: mut plan,
                skipped,
                mtime_refreshes,
            } = diff::build_plan(local, remote, manifest_entries, direction, device_id);
            summary.skipped += skipped;

            // Arquivos com mtime tocado mas conteúdo intacto: reancora o mtime
            // no manifest para o pré-filtro não redisparar a cada sync. Uma
            // transação para a categoria inteira, não uma por arquivo.
            if !mtime_refreshes.is_empty() {
                let _ = self
                    .db
                    .with(move |conn| manifest::upsert_batch(conn, &mtime_refreshes))
                    .await;
            }

            // Backoff da fila offline: pendências cuja janela de retentativa
            // ainda não venceu (ou mortas) são puladas neste ciclo.
            let (emulator, cat) = (target.label.clone(), *category);
            let now_ms = chrono::Utc::now().timestamp_millis();
            let deferred = self
                .db
                .with(move |conn| queue::deferred_rel_paths(conn, &emulator, cat, now_ms))
                .await
                .unwrap_or_default();
            if !deferred.is_empty() {
                let before = plan.len();
                plan.retain(|op| !deferred.contains(&op.rel_path));
                let skipped_by_backoff = (before - plan.len()) as u32;
                if skipped_by_backoff > 0 {
                    tracing::info!(
                        emulador = %target.label,
                        categoria = category.as_str(),
                        arquivos = skipped_by_backoff,
                        "pendências em backoff puladas neste ciclo"
                    );
                    summary.skipped += skipped_by_backoff;
                }
            }

            // Detecção de renomeação por hash: arquivo novo local com o mesmo
            // conteúdo de um órfão remoto vira um `files.update` (rename) em
            // vez de Upload + zumbi do nome antigo.
            let (renamed_plan, renamed) = self
                .detect_renames(&target.label, *category, &folder_id, &folder_key, plan)
                .await;
            let plan = renamed_plan;
            summary.renamed += renamed;

            if plan.is_empty() {
                continue;
            }

            // Pré-cria as subpastas necessárias de forma sequencial, populando
            // o cache de IDs, ANTES dos uploads concorrentes. Sem isto, várias
            // tarefas paralelas do mesmo jogo passam juntas pelo "miss" do cache
            // e criam pastas duplicadas no Drive (uma por arquivo concorrente).
            for dir in upload_dirs(&plan) {
                remote_provider
                    .ensure_subpath(&folder_id, &folder_key, dir)
                    .await?;
            }

            self.transition(SyncState::Syncing, Some(&target.label));

            let ctx = CategoryCtx {
                emulator: target.label.clone(),
                category: *category,
                direction,
                folder_id,
                folder_key,
                download_base: self.storage.join(
                    &self.storage.root_loc(&target.root),
                    &bases[0].to_string_lossy().replace('\\', "/"),
                ),
                backup_base: FileLoc::from_path(
                    self.backup_dir
                        .join(&target.label)
                        .join(run_stamp)
                        .join(category.as_str()),
                ),
                device: device.map(str::to_string),
                device_id: device_id.map(str::to_string),
                notif,
                max_versions,
                total: plan.len() as u32,
                completed: AtomicU32::new(0),
                bytes_total: plan.iter().map(op_bytes).sum(),
                bytes_done: AtomicU64::new(0),
                last_file: std::sync::Mutex::new(String::new()),
            };

            // Uploads de arquivos NOVOS e pequenos vão em lote (Batch API),
            // cortando ~100× as chamadas HTTP no primeiro sync de coleções
            // grandes. Os demais (downloads, updates, conflitos, arquivos grandes)
            // e o que o batch não conseguir seguem pelo caminho per-file abaixo.
            let plan = self.batch_new_uploads(&ctx, plan, &mut summary).await;

            // Além do teto de contagem do `buffer_unordered` abaixo, um
            // semáforo ponderado por bytes evita que poucos arquivos grandes
            // (savestates) monopolizem as vagas de um jeito que um monte de
            // saves pequenos jamais faria — cada op só roda depois de
            // reservar seu peso em bytes (até o teto do semáforo inteiro).
            let bytes_semaphore = Semaphore::new(MAX_BYTES_IN_FLIGHT as usize);
            let transfers = stream::iter(plan.into_iter().map(|op| {
                let (bytes_semaphore, ctx) = (&bytes_semaphore, &ctx);
                async move {
                    let weight = op_bytes(&op).max(1).min(MAX_BYTES_IN_FLIGHT as u64) as u32;
                    let _permit = bytes_semaphore
                        .acquire_many(weight)
                        .await
                        .expect("semáforo de bytes em trânsito nunca é fechado");
                    self.execute_op(ctx, op).await
                }
            }))
            .buffer_unordered(DRIVE_MAX_CONCURRENT_TRANSFERS)
            .collect::<Vec<_>>();

            // Retrato consolidado a cada 500ms em vez de um evento por
            // arquivo — um sync de 200 arquivos não devia inundar o frontend
            // com 200 eventos. O ticker roda até o `select!` resolver pelo
            // outro lado (transferências concluídas), quando é cancelado.
            let mut last_emitted: u32 = ctx.completed.load(Ordering::Relaxed);
            let ticker = async {
                let mut interval = tokio::time::interval(Duration::from_millis(500));
                interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
                loop {
                    interval.tick().await;
                    let completed = ctx.completed.load(Ordering::Relaxed);
                    if completed != last_emitted {
                        last_emitted = completed;
                        self.emit_progress_snapshot(&ctx);
                    }
                }
            };
            let outcomes = tokio::select! {
                outcomes = transfers => outcomes,
                _ = ticker => unreachable!("o ticker nunca termina sozinho"),
            };
            // Retrato final garantido (completed == total) mesmo que o
            // último arquivo tenha terminado entre dois ticks do timer.
            self.emit_progress_snapshot(&ctx);

            // Uma transação por categoria para todas as entradas sincronizadas
            // com sucesso, em vez de um `upsert` por arquivo transferido.
            let mut synced_entries = Vec::new();
            for outcome in outcomes {
                match outcome {
                    OpOutcome::Uploaded(entry) => {
                        summary.uploaded += 1;
                        synced_entries.extend(entry);
                    }
                    OpOutcome::Downloaded(entry) => {
                        summary.downloaded += 1;
                        synced_entries.extend(entry);
                    }
                    OpOutcome::DownloadedWithBackup(entry) => {
                        summary.downloaded += 1;
                        summary.backed_up += 1;
                        synced_entries.extend(entry);
                    }
                    OpOutcome::Conflicted => summary.conflicts += 1,
                    OpOutcome::Queued => summary.queued += 1,
                    OpOutcome::Failed => summary.failed += 1,
                    OpOutcome::Cancelled => summary.cancelled += 1,
                }
            }
            if !synced_entries.is_empty() {
                let _ = self
                    .db
                    .with(move |conn| manifest::upsert_batch(conn, &synced_entries))
                    .await;
            }
        }

        Ok(summary)
    }

    /// Substitui, nos arquivos varridos, o mtime arredondado pelo filesystem
    /// pelo mtime lógico registrado em `mtime_overrides` (ver o módulo para o
    /// porquê). Um override só vale enquanto o mtime no disco continuar
    /// exatamente igual ao `ondisk_ms` que foi anotado: quando muda, o arquivo
    /// foi realmente editado e a linha é descartada.
    ///
    /// Best-effort: falha ao ler a tabela deixa os mtimes como vieram do disco
    /// — o pré-filtro de hash ainda evita o upload inútil, só mais caro.
    async fn apply_mtime_overrides(
        &self,
        emulator: &str,
        category: SyncCategory,
        local: &mut [LocalFile],
    ) {
        let (emu, cat) = (emulator.to_string(), category);
        let Ok(overrides) = self
            .db
            .with(move |conn| mtime_overrides::list_for_category(conn, &emu, cat))
            .await
        else {
            return;
        };
        if overrides.is_empty() {
            return;
        }

        let mut stale = Vec::new();
        for file in local.iter_mut() {
            let Some(entry) = overrides.get(&file.rel_path) else {
                continue;
            };
            if file.mtime_ms == entry.ondisk_ms {
                file.mtime_ms = entry.virtual_ms;
                // O remanescente sub-ms é do carimbo arredondado, não do mtime
                // lógico que acabou de substituí-lo — mantê-lo faria
                // `hash_touched_files` comparar um ns com o outro.
                file.mtime_ns = 0;
            } else {
                stale.push(file.rel_path.clone());
            }
        }

        // Overrides de arquivos que sumiram da varredura também são lixo, mas
        // ficam para a remoção do emulador: um arquivo pode estar
        // temporariamente fora (drive desmontado) sem ter sido apagado.
        if !stale.is_empty() {
            let (emu, cat) = (emulator.to_string(), category);
            let _ = self
                .db
                .with(move |conn| mtime_overrides::remove_batch(conn, &emu, cat, &stale))
                .await;
        }
    }

    /// Pré-passo do diff: calcula o SHA-256 dos arquivos locais cujo mtime
    /// divergiu da âncora do manifest (e que têm hash conhecido para comparar).
    /// Falha de leitura deixa o hash `None` — o arquivo segue o fluxo normal.
    async fn hash_touched_files(
        &self,
        mut local: Vec<LocalFile>,
        manifest: &[ManifestEntry],
    ) -> Vec<LocalFile> {
        use std::collections::HashMap;
        let anchors: HashMap<&str, (&Option<String>, Option<i64>)> = manifest
            .iter()
            .map(|e| (e.rel_path.as_str(), (&e.file_hash, e.local_mtime_ms)))
            .collect();
        // Remanescente sub-ms da âncora, à parte para não mexer no padrão de
        // desreferência acima. `0` = sem precisão sub-ms conhecida.
        let ns_anchors: HashMap<&str, i64> = manifest
            .iter()
            .map(|e| (e.rel_path.as_str(), e.mtime_ns))
            .collect();

        // 1ª passada: decide quais arquivos precisam de hash (I/O assíncrono,
        // sequencial) e junta o conteúdo lido — o hash em si (CPU-bound) é
        // calculado à parte, em paralelo, depois desta passada.
        let mut to_hash: Vec<(usize, Vec<u8>)> = Vec::new();
        for (idx, file) in local.iter().enumerate() {
            let Some((known_hash, Some(anchor))) = anchors.get(file.rel_path.as_str()) else {
                continue;
            };
            if known_hash.is_none() {
                continue;
            }
            let ms_within_tolerance = (file.mtime_ms - anchor).abs() <= TIMESTAMP_TOLERANCE_MS;
            // Dentro da tolerância em ms, mas o remanescente sub-ms diverge de
            // um valor conhecido dos dois lados: escrita real diferente que a
            // tolerância de 2s teria mascarado (ex.: duas gravações do
            // emulador a menos de 2s uma da outra).
            let anchor_ns = ns_anchors.get(file.rel_path.as_str()).copied().unwrap_or(0);
            let ns_disagrees = anchor_ns != 0 && file.mtime_ns != 0 && anchor_ns != file.mtime_ns;
            if ms_within_tolerance && !ns_disagrees {
                continue;
            }
            if let Ok(content) = self.storage.read(&file.loc).await {
                to_hash.push((idx, content));
            }
        }

        if to_hash.is_empty() {
            return local;
        }

        // 2ª passada: SHA-256 de cada arquivo tocado em paralelo via rayon —
        // savestates grandes tocados na mesma categoria não esperam uns pelos
        // outros. `spawn_blocking` tira o cálculo do executor async: o join
        // do rayon dentro dele bloqueia a thread, mas é uma thread dedicada a
        // isso, não uma worker do tokio.
        let hashed = tokio::task::spawn_blocking(move || {
            use rayon::prelude::*;
            to_hash
                .into_par_iter()
                .map(|(idx, content)| (idx, super::sha256_hex(&content)))
                .collect::<Vec<_>>()
        })
        .await
        .unwrap_or_default();

        for (idx, hash) in hashed {
            local[idx].hash = Some(hash);
        }
        local
    }

    /// Detecção de renomeação por hash: um Upload novo cujo MD5
    /// bate com o `md5Checksum` de um Download órfão (arquivo remoto que sumiu
    /// localmente) é a mesma coisa renomeada — aplica `files.update` no Drive,
    /// reancora o manifest e remove os dois lados do plano. MD5 ambíguo (dois
    /// órfãos com o mesmo conteúdo) é ignorado por segurança.
    async fn detect_renames(
        &self,
        emulator: &str,
        category: SyncCategory,
        folder_id: &str,
        folder_key: &str,
        mut plan: Vec<PlannedOp>,
    ) -> (Vec<PlannedOp>, u32) {
        use std::collections::{HashMap, HashSet};

        // Já verificado pelo gate no início de `sync_filtered`.
        let remote_provider = self
            .remote()
            .expect("provedor remoto verificado no início do sync");

        // Órfãos remotos: Download de arquivo sem contraparte local.
        let mut orphans: HashMap<String, usize> = HashMap::new();
        let mut ambiguous: HashSet<String> = HashSet::new();
        for (i, op) in plan.iter().enumerate() {
            if op.action == SyncAction::Download && op.local.is_none() {
                if let Some(md5) = op.remote.as_ref().and_then(|r| r.hash.clone()) {
                    if orphans.insert(md5.clone(), i).is_some() {
                        ambiguous.insert(md5);
                    }
                }
            }
        }
        orphans.retain(|md5, _| !ambiguous.contains(md5));
        let has_new_upload = plan
            .iter()
            .any(|op| op.action == SyncAction::Upload && op.remote.is_none());
        if orphans.is_empty() || !has_new_upload {
            return (plan, 0);
        }

        let mut consumed: Vec<usize> = Vec::new();
        let mut renamed = 0u32;
        for i in 0..plan.len() {
            let op = &plan[i];
            if op.action != SyncAction::Upload || op.remote.is_some() {
                continue;
            }
            let Some(local) = op.local.as_ref() else {
                continue;
            };
            let Ok(content) = self.storage.read(&local.loc).await else {
                continue;
            };
            let Some(&orphan_idx) = orphans.get(&super::md5_hex(&content)) else {
                continue;
            };
            if consumed.contains(&orphan_idx) {
                continue;
            }
            let orphan_rel = plan[orphan_idx].rel_path.clone();
            let remote = plan[orphan_idx].remote.as_ref().expect("órfão tem remoto");

            // Subpasta mudou? Então o rename também move de parent.
            let (new_dir, new_name) = split_rel_path(&op.rel_path);
            let (old_dir, _) = split_rel_path(&orphan_rel);
            let parents = if new_dir == old_dir {
                Some((None, None))
            } else {
                let new_parent = match new_dir {
                    Some(dir) => remote_provider
                        .ensure_subpath(folder_id, folder_key, dir)
                        .await
                        .ok(),
                    None => Some(folder_id.to_string()),
                };
                let old_parent = match old_dir {
                    Some(dir) => remote_provider
                        .ensure_subpath(folder_id, folder_key, dir)
                        .await
                        .ok(),
                    None => Some(folder_id.to_string()),
                };
                match (new_parent, old_parent) {
                    (Some(new_parent), Some(old_parent)) => {
                        Some((Some(new_parent), Some(old_parent)))
                    }
                    _ => None,
                }
            };
            let Some((add_parent, remove_parent)) = parents else {
                continue;
            };

            match remote_provider
                .rename_file(
                    &remote.id,
                    new_name,
                    add_parent.as_deref(),
                    remove_parent.as_deref(),
                )
                .await
            {
                Ok(updated) => {
                    tracing::info!(
                        emulador = %emulator,
                        de = %orphan_rel,
                        para = %op.rel_path,
                        "renomeação detectada por hash; aplicada no provedor remoto sem retransferir"
                    );
                    let entry = ManifestEntry {
                        emulator: emulator.to_string(),
                        category,
                        rel_path: op.rel_path.clone(),
                        remote_file_id: Some(updated.id.clone()),
                        local_mtime_ms: Some(local.mtime_ms),
                        remote_mtime_ms: updated.modified_ms,
                        size_bytes: Some(content.len() as i64),
                        last_synced_at_ms: chrono::Utc::now().timestamp_millis(),
                        file_hash: Some(super::sha256_hex(&content)),
                        flags: 0,
                        inaccessible: false,
                        mtime_ns: local.mtime_ns,
                    };
                    let (emu, old_rel) = (emulator.to_string(), orphan_rel);
                    let _ = self
                        .db
                        .with(move |conn| {
                            manifest::remove_entry(conn, &emu, category, &old_rel)?;
                            manifest::upsert(conn, &entry)
                        })
                        .await;
                    consumed.push(i);
                    consumed.push(orphan_idx);
                    renamed += 1;
                }
                Err(err) => {
                    tracing::warn!(
                        arquivo = %op.rel_path,
                        error = %err,
                        "rename no Drive falhou; seguindo com upload normal"
                    );
                }
            }
        }

        consumed.sort_unstable();
        consumed.dedup();
        for idx in consumed.into_iter().rev() {
            plan.remove(idx);
        }
        (plan, renamed)
    }

    async fn execute_op(&self, ctx: &CategoryCtx, op: PlannedOp) -> OpOutcome {
        // Checagem entre operações: o plano já está montado e as ops correm
        // concorrentemente, então o ponto seguro para desistir é antes de
        // começar mais uma — nunca no meio de uma transferência.
        if self.cancel.is_cancelled() {
            return OpOutcome::Cancelled;
        }

        let rel_path = op.rel_path.clone();
        let bytes = op_bytes(&op);
        let result: AppResult<Option<ManifestEntry>> = match op.action {
            SyncAction::Upload => self.do_upload(ctx, &op).await.map(Some),
            SyncAction::Download => self.do_download(ctx, &op).await.map(Some),
            SyncAction::DownloadWithBackup => {
                self.do_download_with_backup(ctx, &op).await.map(Some)
            }
            SyncAction::Conflict => self.record_conflict(ctx, &op).await.map(|()| None),
            SyncAction::NoOp => Ok(None),
        };

        self.record_progress(ctx, &rel_path, bytes);

        match result {
            Ok(entry) => {
                // Conflito não é transferência: não limpa a pendência (o
                // emulador fica bloqueado até a resolução).
                if matches!(op.action, SyncAction::Conflict) {
                    let emulator = ctx.emulator.clone();
                    let _ = self
                        .db
                        .with(move |conn| stats::record_conflict(conn, &emulator))
                        .await;
                    return OpOutcome::Conflicted;
                }
                let (emulator, category, rel) =
                    (ctx.emulator.clone(), ctx.category, rel_path.clone());
                let _ = self
                    .db
                    .with(move |conn| queue::resolve(conn, &emulator, category, &rel))
                    .await;

                // Estatísticas acumuladas do emulador (best-effort).
                let (emulator, rel, bytes_i64, is_upload) = (
                    ctx.emulator.clone(),
                    rel_path,
                    bytes as i64,
                    matches!(op.action, SyncAction::Upload),
                );
                let _ = self
                    .db
                    .with(move |conn| {
                        if is_upload {
                            stats::record_upload(conn, &emulator, bytes_i64, &rel)
                        } else {
                            stats::record_download(conn, &emulator, bytes_i64, &rel)
                        }
                    })
                    .await;

                match op.action {
                    SyncAction::Upload => OpOutcome::Uploaded(entry),
                    SyncAction::DownloadWithBackup => OpOutcome::DownloadedWithBackup(entry),
                    _ => OpOutcome::Downloaded(entry),
                }
            }
            Err(err) => {
                // Upload travado pelo emulador (arquivo aberto/exclusivo): não é um
                // erro permanente nem uma falha de rede a reagendar por backoff — o
                // watcher de filesystem já dispara um resync quando o emulador
                // libera o arquivo. Marca inacessível em vez de enfileirar.
                let locked = op.action == SyncAction::Upload
                    && matches!(
                        &err,
                        AppError::Io(io)
                            if matches!(
                                io.kind(),
                                std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::WouldBlock
                            )
                    );
                if locked {
                    tracing::info!(
                        emulador = %ctx.emulator,
                        arquivo = %rel_path,
                        "upload abortado: arquivo travado pelo emulador; marcado inacessível"
                    );
                    let (emulator, category, rel) = (ctx.emulator.clone(), ctx.category, rel_path);
                    let _ = self
                        .db
                        .with(move |conn| {
                            manifest::mark_inaccessible(conn, &emulator, category, &rel)
                        })
                        .await;
                    return OpOutcome::Failed;
                }

                let retryable = matches!(
                    err,
                    AppError::Network(_) | AppError::FileBusy(_) | AppError::Integrity(_)
                );
                tracing::warn!(
                    emulador = %ctx.emulator,
                    arquivo = %rel_path,
                    error = %err,
                    retryable,
                    "operação de sync falhou"
                );
                if retryable {
                    let (emulator, category, rel) =
                        (ctx.emulator.clone(), ctx.category, rel_path.clone());
                    let direction = match op.action {
                        SyncAction::Upload => queue::OpDirection::Upload,
                        _ => queue::OpDirection::Download,
                    };
                    let message = err.to_string();
                    let _ = self
                        .db
                        .with(move |conn| {
                            queue::enqueue(conn, &emulator, category, &rel, direction, &message)
                        })
                        .await;
                    // Índice best-effort em sync_manifest.flags — não-op se a
                    // linha ainda não existir.
                    let (emulator, category) = (ctx.emulator.clone(), ctx.category);
                    let _ = self
                        .db
                        .with(move |conn| {
                            manifest::set_flag(conn, &emulator, category, &rel_path, FLAG_PENDING)
                        })
                        .await;
                    OpOutcome::Queued
                } else {
                    OpOutcome::Failed
                }
            }
        }
    }

    /// Avança os contadores (arquivos e bytes) de concluídos da categoria e
    /// registra o nome do arquivo — sem emitir evento. A emissão é
    /// responsabilidade do ticker periódico em `sync_target`
    /// ([`Self::emit_progress_snapshot`]), não de cada arquivo individual.
    fn record_progress(&self, ctx: &CategoryCtx, rel_path: &str, bytes: u64) {
        ctx.completed.fetch_add(1, Ordering::Relaxed);
        ctx.bytes_done.fetch_add(bytes, Ordering::Relaxed);
        if let Ok(mut last) = ctx.last_file.lock() {
            *last = rel_path.to_string();
        }
    }

    /// Emite um retrato consolidado do progresso da categoria a partir dos
    /// contadores atômicos. Chamado periodicamente (não por arquivo) por um
    /// `tokio::time::interval` em `sync_target`, para não inundar o frontend
    /// num sync de muitos arquivos.
    fn emit_progress_snapshot(&self, ctx: &CategoryCtx) {
        let completed = ctx.completed.load(Ordering::Relaxed);
        let bytes_done = ctx.bytes_done.load(Ordering::Relaxed);
        let current_file = ctx.last_file.lock().map(|s| s.clone()).unwrap_or_default();
        let _ = self.app.emit(
            EVT_SYNC_PROGRESS,
            &SyncProgress {
                emulator: ctx.emulator.clone(),
                current_file,
                completed,
                total: ctx.total,
                bytes_done,
                bytes_total: ctx.bytes_total,
                direction: ctx.direction,
            },
        );
    }

    /// Pré-passo de batch: envia em lote os uploads de arquivos
    /// novos e pequenos, atualizando manifest/summary/progresso, e devolve o
    /// plano restante para o caminho per-file. Ops inelegíveis ou que não puderam
    /// ser preparadas (arquivo em uso, parent irresolvível) voltam ao restante,
    /// preservando o tratamento de fila/erro individual do `execute_op`.
    async fn batch_new_uploads(
        &self,
        ctx: &CategoryCtx,
        plan: Vec<PlannedOp>,
        summary: &mut SyncSummary,
    ) -> Vec<PlannedOp> {
        let (eligible, mut rest): (Vec<PlannedOp>, Vec<PlannedOp>) =
            plan.into_iter().partition(is_batchable);

        // Poucos elegíveis: o overhead de montar o batch não compensa — deixa o
        // caminho per-file concorrente resolver.
        if eligible.len() < DRIVE_BATCH_MIN_OPS {
            rest.extend(eligible);
            return rest;
        }

        // Prepara cada op (lê conteúdo, confere mtime estável, resolve parent).
        let mut prepared: Vec<PreparedBatchOp> = Vec::with_capacity(eligible.len());
        for op in eligible {
            match self.prepare_batch_op(ctx, op).await {
                Ok(item) => prepared.push(item),
                Err(op) => rest.push(op),
            }
        }

        tracing::info!(
            emulador = %ctx.emulator,
            categoria = ctx.category.as_str(),
            arquivos = prepared.len(),
            "batch upload de arquivos novos"
        );

        let remote_provider = self
            .remote()
            .expect("provedor remoto verificado no início do sync");
        // Entradas do manifest de todos os chunks, gravadas numa única
        // transação ao final — não uma por arquivo do batch.
        let mut synced = Vec::new();
        for chunk in prepared.chunks(DRIVE_BATCH_MAX_OPS) {
            let ops: Vec<BatchUploadOp> = chunk.iter().map(|p| p.batch.clone()).collect();
            match remote_provider.upload_batch(ops).await {
                Ok(files) if files.len() == chunk.len() => {
                    for (p, uploaded) in chunk.iter().zip(files) {
                        let drive_mtime = uploaded.modified_ms;
                        let local_mtime_ns = p.op.local.as_ref().map(|l| l.mtime_ns).unwrap_or(0);
                        synced.push(self.record_synced(
                            ctx,
                            &p.rel_path,
                            uploaded.id,
                            p.mtime_ms,
                            local_mtime_ns,
                            drive_mtime,
                            p.size_bytes,
                            Some(p.content_hash.clone()),
                        ));

                        let (emulator, category, rel) =
                            (ctx.emulator.clone(), ctx.category, p.rel_path.clone());
                        let _ = self
                            .db
                            .with(move |conn| queue::resolve(conn, &emulator, category, &rel))
                            .await;
                        let (emulator, rel, bytes) =
                            (ctx.emulator.clone(), p.rel_path.clone(), p.size_bytes);
                        let _ = self
                            .db
                            .with(move |conn| stats::record_upload(conn, &emulator, bytes, &rel))
                            .await;
                        summary.uploaded += 1;

                        self.record_progress(ctx, &p.rel_path, p.size_bytes.max(0) as u64);
                    }
                }
                result => {
                    // Falha do batch (rede/parse/sub-request) ou contagem
                    // inesperada: devolve o chunk ao per-file, que aplica
                    // retry/fila por arquivo.
                    if let Err(err) = result {
                        tracing::warn!(
                            emulador = %ctx.emulator,
                            error = %err,
                            arquivos = chunk.len(),
                            "batch falhou; caindo para upload per-file"
                        );
                    }
                    for p in chunk {
                        rest.push(p.op.clone());
                    }
                }
            }
        }

        if !synced.is_empty() {
            let _ = self
                .db
                .with(move |conn| manifest::upsert_batch(conn, &synced))
                .await;
        }

        rest
    }

    /// Prepara uma op elegível para o batch: lê o conteúdo com a mesma proteção
    /// de mtime estável do `do_upload` e resolve o `parent_id`. `Err(op)` devolve
    /// a op original para o caminho per-file quando não pôde ser preparada.
    ///
    /// O `Err` não é um caminho de erro: é o fallback normal, consumido no
    /// único chamador por um `match` que empurra a op de volta para a fila
    /// per-file. Nada é propagado com `?`, então o tamanho do `Result` não
    /// atravessa a pilha. Boxar o `Err` também não encolheria nada — o `Ok`
    /// (`PreparedBatchOp`) carrega o mesmo `PlannedOp` mais o conteúdo lido,
    /// e é ele quem determina o tamanho do `Result`.
    #[allow(clippy::result_large_err)]
    async fn prepare_batch_op(
        &self,
        ctx: &CategoryCtx,
        op: PlannedOp,
    ) -> Result<PreparedBatchOp, PlannedOp> {
        // Clona o locador para não manter `op` emprestado até o move final.
        let loc = match op.local.as_ref() {
            Some(local) => local.loc.clone(),
            None => return Err(op),
        };

        // Mesma proteção do do_upload: conteúdo estável entre duas leituras de mtime.
        let read = async {
            let before = self.storage.mtime_ms(&loc).await?;
            let content = self.storage.read(&loc).await?;
            let after = self.storage.mtime_ms(&loc).await?;
            if before != after {
                return Err(AppError::FileBusy(op.rel_path.clone()));
            }
            Ok::<_, AppError>((content, after))
        }
        .await;
        let (content, mtime) = match read {
            Ok(v) => v,
            Err(_) => return Err(op),
        };

        let remote_provider = self
            .remote()
            .expect("provedor remoto verificado no início do sync");
        let (dir_part, file_name) = split_rel_path(&op.rel_path);
        let parent_id = match dir_part {
            Some(dir) => {
                match remote_provider
                    .ensure_subpath(&ctx.folder_id, &ctx.folder_key, dir)
                    .await
                {
                    Ok(id) => id,
                    Err(_) => return Err(op),
                }
            }
            None => ctx.folder_id.clone(),
        };

        let size_bytes = content.len() as i64;
        let content_hash = super::sha256_hex(&content);
        let batch = BatchUploadOp {
            parent_id,
            name: file_name.to_string(),
            content,
            mtime_ms: mtime,
            device_name: ctx.device.clone(),
            device_id: ctx.device_id.clone(),
        };
        Ok(PreparedBatchOp {
            rel_path: op.rel_path.clone(),
            mtime_ms: mtime,
            size_bytes,
            content_hash,
            op,
            batch,
        })
    }

    async fn do_upload(&self, ctx: &CategoryCtx, op: &PlannedOp) -> AppResult<ManifestEntry> {
        let remote_provider = self.remote()?;
        let local = op
            .local
            .as_ref()
            .ok_or_else(|| AppError::Other("upload planejado sem arquivo local".into()))?;

        let (content, mtime_after) = {
            // I/O de disco local: teto separado do de rede abaixo (ver
            // `disk_io`/`MAX_DISK_WRITES`) — em HDD, ler vários arquivos ao
            // mesmo tempo é mais lento que ler em sequência.
            let _permit = self.disk_io.acquire().await.expect("disk_io não fecha");
            let mtime_before = self.storage.mtime_ms(&local.loc).await?;
            let content = self.storage.read(&local.loc).await?;
            let mtime_after = self.storage.mtime_ms(&local.loc).await?;
            if mtime_before != mtime_after {
                return Err(AppError::FileBusy(local.rel_path.clone()));
            }
            (content, mtime_after)
        };

        let size_bytes = content.len() as i64;
        let content_hash = super::sha256_hex(&content);
        let uploaded = {
            // Chamadas de rede: teto separado do de disco acima (ver
            // `network_ops`/`MAX_NETWORK_OPS`).
            let _permit = self
                .network_ops
                .acquire()
                .await
                .expect("network_ops não fecha");
            let (dir_part, file_name) = split_rel_path(&op.rel_path);
            let parent_id = match dir_part {
                Some(dir) => {
                    remote_provider
                        .ensure_subpath(&ctx.folder_id, &ctx.folder_key, dir)
                        .await?
                }
                None => ctx.folder_id.clone(),
            };
            let tag = DeviceTag {
                name: ctx.device.as_deref(),
                id: ctx.device_id.as_deref(),
            };
            match op.remote.as_ref() {
                Some(existing) => {
                    remote_provider
                        .upload_existing(&existing.id, content, mtime_after, tag)
                        .await?
                }
                None => {
                    remote_provider
                        .upload_new(&parent_id, file_name, content, mtime_after, tag)
                        .await?
                }
            }
        };

        let drive_mtime = uploaded.modified_ms;
        // Remanescente sub-ms do scan original — a estabilidade do arquivo já
        // foi confirmada acima (mtime_before == mtime_after em ms).
        let local_mtime_ns = local.mtime_ns;
        Ok(self.record_synced(
            ctx,
            &op.rel_path,
            uploaded.id,
            mtime_after,
            local_mtime_ns,
            drive_mtime,
            size_bytes,
            Some(content_hash),
        ))
    }

    /// Primeiro sync de um arquivo que existe nos dois lados: copia o local
    /// para a pasta de backup e só então baixa o do Drive (que vence). O backup
    /// roda ANTES do download — se falhar, o download não acontece, evitando
    /// perder a versão local sem uma cópia de segurança.
    async fn do_download_with_backup(
        &self,
        ctx: &CategoryCtx,
        op: &PlannedOp,
    ) -> AppResult<ManifestEntry> {
        if let Some(local) = op.local.as_ref() {
            let backup_dest = self.storage.join(&ctx.backup_base, &op.rel_path);
            self.storage.copy_to(&local.loc, &backup_dest).await?;
            tracing::info!(
                emulador = %ctx.emulator,
                arquivo = %op.rel_path,
                backup = %backup_dest,
                "backup local antes do primeiro sync (Drive vence)"
            );
        }
        self.do_download(ctx, op).await
    }

    /// Arquiva a versão local vigente de `op` no histórico
    /// (`<backups>/<emulador>/history/...`), mantendo no máximo
    /// `ctx.max_versions` por arquivo. No-op se o arquivo local não existe ou
    /// não tem caminho nativo (armazenamento SAF do mobile).
    async fn archive_previous_version(&self, ctx: &CategoryCtx, op: &PlannedOp) {
        let Some(src) = op
            .local
            .as_ref()
            .and_then(|l| l.loc.as_native_path())
            .map(Path::to_path_buf)
        else {
            return;
        };

        let versioner = self.versioner.clone();
        let (emulator, category, rel_path) = (
            ctx.emulator.clone(),
            ctx.category.as_str().to_string(),
            op.rel_path.clone(),
        );
        let max = ctx.max_versions;
        let archived = tokio::task::spawn_blocking(move || {
            versioner.archive(&emulator, &category, &rel_path, &src, max)
        })
        .await;
        match archived {
            Ok(Ok(dest)) => {
                tracing::debug!(arquivo = %op.rel_path, destino = %dest.display(), "versão anterior arquivada");
            }
            Ok(Err(err)) => {
                tracing::warn!(arquivo = %op.rel_path, error = %err, "falha ao arquivar versão anterior");
            }
            Err(err) => {
                tracing::warn!(arquivo = %op.rel_path, error = %err, "tarefa de arquivamento abortada");
            }
        }
    }

    /// Windows apenas: `dest` colide (case-insensitive) com outro nome já
    /// presente na mesma pasta? Compara só o componente final do caminho —
    /// escaneia a pasta-pai, não a árvore inteira.
    #[cfg(target_os = "windows")]
    async fn check_case_collision(&self, dest: &FileLoc) -> AppResult<()> {
        let Some(path) = dest.as_native_path() else {
            return Ok(());
        };
        let (Some(parent), Some(incoming)) = (path.parent(), path.file_name()) else {
            return Ok(());
        };
        let incoming = incoming.to_string_lossy().into_owned();
        let incoming_lower = incoming.to_lowercase();

        let Ok(mut entries) = tokio::fs::read_dir(parent).await else {
            return Ok(());
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            let existing = entry.file_name().to_string_lossy().into_owned();
            if existing != incoming && existing.to_lowercase() == incoming_lower {
                return Err(AppError::CaseConflict { existing, incoming });
            }
        }
        Ok(())
    }

    async fn do_download(&self, ctx: &CategoryCtx, op: &PlannedOp) -> AppResult<ManifestEntry> {
        let remote = op
            .remote
            .as_ref()
            .ok_or_else(|| AppError::Other("download planejado sem arquivo remoto".into()))?;

        let dest = match op.local.as_ref() {
            Some(local) => local.loc.clone(),
            None => self.storage.join(&ctx.download_base, &op.rel_path),
        };

        // NTFS é case-preserving mas case-insensitive: "Save.bin" e "save.bin"
        // são o MESMO arquivo pro Windows, mas o motor de sync (rel_path exato)
        // os trata como dois arquivos distintos. Sem essa checagem, baixar o
        // segundo sobrescreveria o primeiro em silêncio — cada lado acha que
        // sincronizou o seu, e um deles some do disco sem aviso.
        #[cfg(target_os = "windows")]
        self.check_case_collision(&dest).await?;

        // Checa o espaço livre no volume de destino ANTES de baixar (margem de
        // 10%). Sem medição disponível (mobile/volume desconhecido), segue.
        let expected_size: u64 = remote.size_bytes.map(|s| s.max(0) as u64).unwrap_or(0);
        if expected_size > 0 {
            if let Some(available) = self.storage.available_space(&dest).await {
                let needed = expected_size + expected_size / 10;
                if available < needed {
                    return Err(AppError::InsufficientDiskSpace {
                        needed_mb: needed / (1024 * 1024),
                        available_mb: available / (1024 * 1024),
                    });
                }
            }
        }

        let content = {
            let _permit = self
                .network_ops
                .acquire()
                .await
                .expect("network_ops não fecha");
            self.remote()?.download(&remote.id).await?
        };

        // Verificação de integridade: o tamanho do que chegou precisa bater com
        // o que a listagem reportou. Divergência = transferência corrompida/
        // truncada → falha retryable (vai para a fila offline). Checagem por
        // tamanho (não por hash) porque cada provedor usa um algoritmo próprio
        // (MD5 no Drive, `content_hash` no Dropbox, `quickXorHash` no OneDrive)
        // — não comparável entre si nem contra o SHA-256 que o app calcula.
        if let Some(expected) = remote.size_bytes {
            let got = content.len() as i64;
            if got != expected {
                return Err(AppError::Integrity(format!(
                    "{}: tamanho divergente após download (esperado {expected} bytes, obtido {got})",
                    op.rel_path
                )));
            }
        }

        // mtime local = mtime remoto, para o diff convergir.
        let drive_mtime = remote.modified_ms;
        let size_bytes = content.len() as i64;
        let content_hash = super::sha256_hex(&content);
        {
            // I/O de disco local: teto separado do de rede acima (ver
            // `disk_io`/`MAX_DISK_WRITES`) — em HDD, escritas paralelas
            // demais viram thrashing de cabeça de leitura/escrita.
            let _permit = self.disk_io.acquire().await.expect("disk_io não fecha");
            // Versionamento: arquiva a versão local vigente ANTES de
            // sobrescrever (só em downloads comuns — o primeiro sync já tem
            // seu próprio backup dedicado). Best-effort: falha de
            // arquivamento não bloqueia o sync, pois a versão anterior já
            // esteve no provedor remoto em algum momento.
            if op.action == SyncAction::Download {
                self.archive_previous_version(ctx, op).await;
            }
            self.storage
                .write_atomic(&dest, &content, drive_mtime)
                .await?;
        }
        self.mark_recent_download(&dest);
        self.record_mtime_override(ctx, &op.rel_path, &dest, drive_mtime)
            .await;

        Ok(self.record_synced(
            ctx,
            &op.rel_path,
            remote.id.clone(),
            drive_mtime.unwrap_or(0),
            // Sem precisão sub-ms real: mtime local vem do modifiedTime remoto.
            0,
            drive_mtime,
            size_bytes,
            Some(content_hash),
        ))
    }

    /// Depois de gravar o mtime remoto no arquivo baixado, confere o que o
    /// filesystem de fato registrou. Divergiu (FAT32 arredonda para múltiplos
    /// de 2s), guarda o par `(ondisk, virtual)` para o próximo scan enxergar o
    /// valor lógico no lugar do arredondado — sem isso o arquivo pareceria
    /// modificado e subiria de novo sem ter mudado.
    ///
    /// Best-effort em todas as pontas: sem mtime remoto, ou se o `stat` falhar,
    /// simplesmente não há override (o pré-filtro de hash segue como rede de
    /// segurança).
    async fn record_mtime_override(
        &self,
        ctx: &CategoryCtx,
        rel_path: &str,
        dest: &FileLoc,
        drive_mtime: Option<i64>,
    ) {
        let Some(virtual_ms) = drive_mtime else {
            return;
        };
        let Ok(ondisk_ms) = self.storage.mtime_ms(dest).await else {
            return;
        };
        if ondisk_ms == virtual_ms {
            // Filesystem com granularidade suficiente (NTFS, ext4, APFS): o
            // valor pedido é o valor gravado, não há nada a compensar.
            return;
        }

        let (emulator, category, rel_path) =
            (ctx.emulator.clone(), ctx.category, rel_path.to_string());
        let value = mtime_overrides::MtimeOverride {
            ondisk_ms,
            virtual_ms,
        };
        tracing::debug!(
            emulador = %emulator,
            arquivo = %rel_path,
            ondisk_ms,
            virtual_ms,
            "filesystem arredondou o mtime; override registrado"
        );
        let _ = self
            .db
            .with(move |conn| mtime_overrides::upsert(conn, &emulator, category, &rel_path, value))
            .await;
    }

    /// Registra um conflito (ambos os lados mudaram desde o último sync). Não
    /// transfere nada; emite evento e notifica. O emulador fica bloqueado até a
    /// resolução pelo usuário.
    async fn record_conflict(&self, ctx: &CategoryCtx, op: &PlannedOp) -> AppResult<()> {
        let local = op
            .local
            .as_ref()
            .ok_or_else(|| AppError::Other("conflito planejado sem arquivo local".into()))?;
        let remote = op
            .remote
            .as_ref()
            .ok_or_else(|| AppError::Other("conflito planejado sem arquivo remoto".into()))?;

        // Cópia padronizada do lado local em <backups>/<emu>/conflicts/, com
        // carimbo e device no nome — o usuário inspeciona os dois lados antes
        // de decidir. Best-effort: a falha não impede o registro do conflito.
        let backup_path = self.copy_conflict_side(ctx, op, &local.loc).await;

        let conflict = Conflict {
            emulator: ctx.emulator.clone(),
            category: ctx.category,
            rel_path: op.rel_path.clone(),
            local_mtime_ms: local.mtime_ms,
            local_size: local.size_bytes,
            local_device: ctx.device.clone(),
            remote_mtime_ms: remote.modified_ms.unwrap_or(0),
            remote_size: remote.size_bytes.unwrap_or(0),
            remote_device: remote.device_name.clone(),
            remote_file_id: remote.id.clone(),
            local_abs_path: self.storage.loc_to_stored(&local.loc),
            detected_at_ms: chrono::Utc::now().timestamp_millis(),
            backup_path,
        };

        let stored = conflict.clone();
        self.db
            .with(move |conn| conflicts::upsert(conn, &stored))
            .await?;
        // Índice best-effort em sync_manifest.flags — não-op se a linha ainda
        // não existir (arquivo nunca sincronizado antes deste conflito).
        let (emulator, category, rel) = (ctx.emulator.clone(), ctx.category, op.rel_path.clone());
        let _ = self
            .db
            .with(move |conn| manifest::set_flag(conn, &emulator, category, &rel, FLAG_CONFLICT))
            .await;

        tracing::warn!(emulador = %ctx.emulator, arquivo = %op.rel_path, "conflito detectado: ambos os lados mudaram");
        self.transition(SyncState::Conflict, Some(&ctx.emulator));
        let _ = self.app.emit(EVT_SYNC_CONFLICT, &conflict);
        if ctx.notif.notifies_errors() {
            self.notify_conflict(&ctx.emulator, &op.rel_path);
        }
        Ok(())
    }

    /// Copia o lado local do conflito para
    /// `<backups>/<emu>/conflicts/<cat>/<rel_dir>/<nome>.slot2sync-conflict-<carimbo>-<device><ext>`
    /// e poda cópias antigas do mesmo arquivo (máx. [`MAX_CONFLICT_COPIES`]).
    /// Retorna o caminho persistível da cópia, ou `None` em falha.
    async fn copy_conflict_side(
        &self,
        ctx: &CategoryCtx,
        op: &PlannedOp,
        src: &FileLoc,
    ) -> Option<String> {
        use crate::constants::{CONFLICT_COPIES_DIR, MAX_CONFLICT_COPIES};

        let (dir_part, file_name) = split_rel_path(&op.rel_path);
        let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
        let device = ctx.device_id.as_deref().unwrap_or("unknown");
        let copy_name = crate::versioning::conflict_copy_name(file_name, &stamp, device);
        let copy_rel = match dir_part {
            Some(dir) => format!("{dir}/{copy_name}"),
            None => copy_name,
        };

        let base = FileLoc::from_path(
            self.backup_dir
                .join(&ctx.emulator)
                .join(CONFLICT_COPIES_DIR)
                .join(ctx.category.as_str()),
        );
        let dest = self.storage.join(&base, &copy_rel);

        if let Err(err) = self.storage.copy_to(src, &dest).await {
            tracing::warn!(arquivo = %op.rel_path, error = %err, "falha ao copiar o lado local do conflito");
            return None;
        }

        // Poda best-effort das cópias antigas deste arquivo.
        if let Some(dir) = dest.as_native_path().and_then(|p| p.parent()) {
            let (dir, name) = (dir.to_path_buf(), file_name.to_string());
            let _ = tokio::task::spawn_blocking(move || {
                crate::versioning::prune_conflict_copies(&dir, &name, MAX_CONFLICT_COPIES)
            })
            .await;
        }

        Some(self.storage.loc_to_stored(&dest))
    }

    #[allow(clippy::too_many_arguments)]
    /// Monta a entrada do manifest para um upload/download bem-sucedido. Não
    /// grava no SQLite — quem chama coleta as entradas da categoria e grava
    /// em lote (ver `sync_target`), em vez de um `upsert` por arquivo.
    /// `local_mtime_ns`: remanescente sub-ms do mtime local, quando a
    /// chamada tiver um `LocalFile` fresco à mão (upload); `0` para download,
    /// já que o mtime local nesse caso é derivado do `modifiedTime` remoto
    /// (sem precisão sub-ms real).
    fn record_synced(
        &self,
        ctx: &CategoryCtx,
        rel_path: &str,
        remote_file_id: String,
        local_mtime_ms: i64,
        local_mtime_ns: i64,
        remote_mtime_ms: Option<i64>,
        size_bytes: i64,
        file_hash: Option<String>,
    ) -> ManifestEntry {
        ManifestEntry {
            emulator: ctx.emulator.clone(),
            category: ctx.category,
            rel_path: rel_path.to_string(),
            remote_file_id: Some(remote_file_id),
            local_mtime_ms: Some(local_mtime_ms),
            remote_mtime_ms,
            size_bytes: Some(size_bytes),
            last_synced_at_ms: chrono::Utc::now().timestamp_millis(),
            file_hash,
            // Sync bem-sucedido: qualquer flag/trava anterior não se aplica
            // mais a esta versão do arquivo.
            flags: 0,
            inaccessible: false,
            mtime_ns: local_mtime_ns,
        }
    }

    /// Snapshot do manifest publicado na raiz `Slot2Sync/` (best-effort).
    /// É só registro/auditoria: grava quem (`device`) e quando (`generatedAt`)
    /// publicou a última versão, além de um dump das entradas. O app nunca lê
    /// este arquivo de volta — a fonte de verdade operacional é a tabela
    /// `sync_manifest` no SQLite local.
    async fn publish_manifest_snapshot(&self) -> AppResult<()> {
        let remote_provider = self.remote()?;
        let entries = self.db.with(manifest::list_all).await?;
        let device = self.db.with(crate::storage::settings::device_name).await?;
        let device_id = crate::device::current(self.secrets.clone()).await;
        let now_ms = chrono::Utc::now().timestamp_millis();
        let doc = serde_json::json!({
            "generatedAt": chrono::DateTime::<chrono::Utc>::from_timestamp_millis(now_ms)
                .unwrap_or_else(chrono::Utc::now)
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            "device": device,
            "deviceId": device_id,
            "entries": entries,
        });
        let bytes = serde_json::to_vec_pretty(&doc)?;
        let tag = DeviceTag {
            name: device.as_deref(),
            id: device_id.as_deref(),
        };

        let root_id = remote_provider.ensure_root().await?;
        match remote_provider
            .find_child(&root_id, DRIVE_MANIFEST_FILE)
            .await?
        {
            Some(existing) => {
                remote_provider
                    .upload_existing(&existing.id, bytes, now_ms, tag)
                    .await?;
            }
            None => {
                remote_provider
                    .upload_new(&root_id, DRIVE_MANIFEST_FILE, bytes, now_ms, tag)
                    .await?;
            }
        }
        Ok(())
    }

    /// Resolve um conflito mantendo a versão escolhida e desbloqueia o emulador.
    pub async fn resolve_conflict(
        &self,
        emulator: &str,
        category: SyncCategory,
        rel_path: &str,
        keep: ConflictResolution,
    ) -> AppResult<()> {
        let (emu, rel) = (emulator.to_string(), rel_path.to_string());
        let conflict = self
            .db
            .with(move |conn| conflicts::get(conn, &emu, category, &rel))
            .await?
            .ok_or_else(|| AppError::Other("conflito não encontrado".into()))?;

        match keep {
            ConflictResolution::Remote => self.resolve_keep_remote(&conflict).await?,
            ConflictResolution::Local => self.resolve_keep_local(&conflict).await?,
        }

        let (emu, rel) = (emulator.to_string(), rel_path.to_string());
        self.db
            .with(move |conn| conflicts::remove(conn, &emu, category, &rel))
            .await?;
        tracing::info!(emulador = %emulator, arquivo = %rel_path, ?keep, "conflito resolvido");
        Ok(())
    }

    /// Mantém a versão remota: faz backup do local e baixa por cima.
    async fn resolve_keep_remote(&self, c: &Conflict) -> AppResult<()> {
        let remote_provider = self.remote()?;
        let dest = self.storage.loc_from_stored(&c.local_abs_path);
        if self.storage.exists(&dest).await {
            let stamp = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S").to_string();
            let backup_base = FileLoc::from_path(
                self.backup_dir
                    .join(&c.emulator)
                    .join(format!("conflito-{stamp}"))
                    .join(c.category.as_str()),
            );
            let backup_dest = self.storage.join(&backup_base, &c.rel_path);
            self.storage.copy_to(&dest, &backup_dest).await?;
            tracing::info!(arquivo = %c.rel_path, backup = %backup_dest, "backup local antes de resolver conflito (manter versão remota)");
        }

        let content = remote_provider.download(&c.remote_file_id).await?;
        let size_bytes = content.len() as i64;
        let content_hash = super::sha256_hex(&content);
        let remote_mtime = c.remote_mtime_ms;
        self.storage
            .write_atomic(&dest, &content, Some(remote_mtime))
            .await?;
        self.mark_recent_download(&dest);

        self.upsert_resolved_manifest(
            c,
            remote_mtime,
            Some(remote_mtime),
            size_bytes,
            &c.remote_file_id,
            Some(content_hash),
        )
        .await
    }

    /// Mantém o local: envia a versão local por cima da remota.
    async fn resolve_keep_local(&self, c: &Conflict) -> AppResult<()> {
        let remote_provider = self.remote()?;
        let src = self.storage.loc_from_stored(&c.local_abs_path);
        let content = self.storage.read(&src).await?;
        let size_bytes = content.len() as i64;
        let content_hash = super::sha256_hex(&content);
        let local_mtime = self.storage.mtime_ms(&src).await?;
        let device = self
            .db
            .with(settings::device_name)
            .await
            .unwrap_or_default();
        let device_id = crate::device::current(self.secrets.clone()).await;
        let tag = DeviceTag {
            name: device.as_deref(),
            id: device_id.as_deref(),
        };

        let uploaded = remote_provider
            .upload_existing(&c.remote_file_id, content, local_mtime, tag)
            .await?;
        let remote_mtime = uploaded.modified_ms;

        self.upsert_resolved_manifest(
            c,
            local_mtime,
            remote_mtime,
            size_bytes,
            &uploaded.id,
            Some(content_hash),
        )
        .await
    }

    async fn upsert_resolved_manifest(
        &self,
        c: &Conflict,
        local_mtime_ms: i64,
        remote_mtime_ms: Option<i64>,
        size_bytes: i64,
        remote_file_id: &str,
        file_hash: Option<String>,
    ) -> AppResult<()> {
        let entry = ManifestEntry {
            emulator: c.emulator.clone(),
            category: c.category,
            rel_path: c.rel_path.clone(),
            remote_file_id: Some(remote_file_id.to_string()),
            local_mtime_ms: Some(local_mtime_ms),
            remote_mtime_ms,
            size_bytes: Some(size_bytes),
            last_synced_at_ms: chrono::Utc::now().timestamp_millis(),
            file_hash,
            // Conflito resolvido: a versão escolhida não está mais em conflito nem travada.
            flags: 0,
            inaccessible: false,
            // Sem `LocalFile` fresco nesta trilha (leitura direta via `storage.read`).
            mtime_ns: 0,
        };
        self.db
            .with(move |conn| manifest::upsert(conn, &entry))
            .await
    }
}

/// Op de upload já preparada para o batch: os dados prontos (`batch`) mais o que
/// o engine precisa para registrar o manifest, e a op original (`op`) para o
/// fallback per-file caso o batch falhe.
struct PreparedBatchOp {
    op: PlannedOp,
    batch: BatchUploadOp,
    rel_path: String,
    mtime_ms: i64,
    size_bytes: i64,
    /// SHA-256 do conteúdo enviado — gravado no manifest (`file_hash`).
    content_hash: String,
}

/// Bytes que a op vai transferir — tamanho local para uploads, tamanho
/// remoto para downloads; conflitos/no-ops não transferem nada.
fn op_bytes(op: &PlannedOp) -> u64 {
    match op.action {
        SyncAction::Upload => op
            .local
            .as_ref()
            .map(|l| l.size_bytes.max(0) as u64)
            .unwrap_or(0),
        SyncAction::Download | SyncAction::DownloadWithBackup => op
            .remote
            .as_ref()
            .and_then(|r| r.size_bytes)
            .map(|s| s.max(0) as u64)
            .unwrap_or(0),
        SyncAction::Conflict | SyncAction::NoOp => 0,
    }
}

/// Elegível ao batch: upload de arquivo que ainda não existe no Drive e é
/// pequeno o suficiente para `multipart` (o batch não suporta resumable).
fn is_batchable(op: &PlannedOp) -> bool {
    op.action == SyncAction::Upload
        && op.remote.is_none()
        && op
            .local
            .as_ref()
            .is_some_and(|l| l.size_bytes <= DRIVE_SIMPLE_UPLOAD_MAX_BYTES as i64)
}

/// `"a/b/c.bin"` → `(Some("a/b"), "c.bin")`; `"c.bin"` → `(None, "c.bin")`.
fn split_rel_path(rel_path: &str) -> (Option<&str>, &str) {
    match rel_path.rsplit_once('/') {
        Some((dir, name)) => (Some(dir), name),
        None => (None, rel_path),
    }
}

/// Diretórios (relativos à categoria) que precisam existir no Drive para os
/// uploads do plano — únicos e ordenados, para que pastas-pai sejam criadas
/// antes das filhas e cada uma só uma vez.
fn upload_dirs(plan: &[PlannedOp]) -> Vec<&str> {
    let mut dirs: Vec<&str> = plan
        .iter()
        .filter(|op| op.action == SyncAction::Upload)
        .filter_map(|op| op.rel_path.rsplit_once('/').map(|(dir, _)| dir))
        .collect();
    dirs.sort_unstable();
    dirs.dedup();
    dirs
}

#[cfg(test)]
mod tests {
    use super::upload_dirs;
    use crate::sync::conflict::SyncAction;
    use crate::sync::diff::PlannedOp;

    fn op(rel_path: &str, action: SyncAction) -> PlannedOp {
        PlannedOp {
            rel_path: rel_path.to_string(),
            action,
            local: None,
            remote: None,
        }
    }

    #[test]
    fn upload_dirs_dedup_e_ordena_apenas_uploads() {
        // Vários arquivos de uma mesma subpasta produzem o diretório uma só vez.
        let plan = vec![
            op("game-b/file1.bin", SyncAction::Upload),
            op("game-a/icon.png", SyncAction::Upload),
            op("game-a/param.sfo", SyncAction::Upload),
            op("game-a/data.bin", SyncAction::Upload),
            // download não cria pasta no Drive
            op("game-c/save.bin", SyncAction::Download),
        ];
        assert_eq!(upload_dirs(&plan), vec!["game-a", "game-b"]);
    }

    #[test]
    fn upload_dirs_ignora_arquivos_na_raiz_da_categoria() {
        // Arquivos sem subpasta (ex.: savestates soltos) não geram diretório.
        let plan = vec![
            op("state-0.bin", SyncAction::Upload),
            op("state-0.jpg", SyncAction::Upload),
        ];
        assert!(upload_dirs(&plan).is_empty());
    }
}
