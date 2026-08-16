//! Boundary frontend ↔ backend: todos os `#[tauri::command]` vivem aqui.
//! Toda struct que cruza esta boundary deriva `Serialize`/`Deserialize` e tem
//! interface TypeScript espelhada em `src/types/ipc.ts`.

#[cfg(desktop)]
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use serde::Serialize;
#[cfg(mobile)]
use tauri::Listener;
#[cfg(all(test, desktop, not(windows)))]
use tauri::Manager;
use tauri::{AppHandle, Emitter, Runtime, State};
#[cfg(desktop)]
use tauri_plugin_autostart::ManagerExt;

use crate::auth::{AuthManager, AuthStatus};
use crate::constants::TRIGGER_MANUAL;
use crate::emulator::{self, EmulatorProfile};
use crate::error::{AppError, AppResult};
use crate::events::EVT_AUTH_STATUS;
use crate::games::{self, SyncedGame};
use crate::remote::{ProviderKind, RemoteProvider};
use crate::state::AppState;
use crate::storage::conflicts::{self, Conflict};
use crate::storage::db::Db;
use crate::storage::emulators::SyncCategories;
use crate::storage::settings::{NotificationLevel, Settings, TriggerSettings};
use crate::storage::{emulators, manifest, queue, settings};
use crate::sync::{
    ConflictResolution, FileLoc, LastSync, SyncCategory, SyncDirection, SyncSummary,
};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HealthStatus {
    pub version: String,
    pub ready: bool,
    /// `true` quando compilado para Android ou iOS; `false` no desktop.
    pub is_mobile: bool,
    /// Tamanho do arquivo SQLite (via `dbstat`). Crescimento anormal pode
    /// indicar corrupção ou vazamento de dados de conflito.
    pub db_size_bytes: u64,
    /// Pendências acumuladas na fila offline. Valor alto por muito tempo
    /// sugere que o dispositivo está sem conseguir sincronizar.
    pub pending_ops_count: u32,
}

/// Verificação mínima de que a boundary frontend ↔ Rust está funcional, mais
/// um retrato rápido da saúde do SQLite local.
#[tauri::command]
pub async fn health_check(state: State<'_, AppState>) -> AppResult<HealthStatus> {
    health_check_impl(&state).await
}

async fn health_check_impl<R: Runtime>(state: &State<'_, AppState<R>>) -> AppResult<HealthStatus> {
    let db_size_bytes = state.db.with(crate::storage::db::size_bytes).await?;
    let pending_ops_count = state.db.with(queue::count).await? as u32;
    Ok(HealthStatus {
        version: env!("CARGO_PKG_VERSION").to_string(),
        ready: true,
        is_mobile: cfg!(mobile),
        db_size_bytes,
        pending_ops_count,
    })
}

/// Abre o seletor nativo de pasta do SO (SAF no Android) e retorna a URI da
/// árvore concedida. No desktop retorna erro — use o seletor de ficheiros.
#[cfg(mobile)]
#[tauri::command]
pub async fn pick_emulator_folder(app: AppHandle) -> AppResult<String> {
    crate::sync::mobile_storage::pick_folder(&app).await
}

#[cfg(desktop)]
#[tauri::command]
pub async fn pick_emulator_folder(_app: AppHandle) -> AppResult<String> {
    Err(AppError::Other(
        "pick_emulator_folder não disponível no desktop".into(),
    ))
}

/// Tenta reconhecer automaticamente o emulador na árvore SAF `tree` (URI
/// concedida por [`pick_emulator_folder`]), testando o mesmo catálogo do
/// `profiles.toml` usado no desktop — a diferença é que cada checagem de
/// pasta vira uma chamada `exists` ao plugin nativo em vez de `is_dir()`.
/// `None` quando nenhum emulador do catálogo é reconhecido (cai no formulário
/// manual, como hoje).
#[cfg(mobile)]
#[tauri::command]
pub async fn detect_emulator_mobile(
    tree: String,
    state: State<'_, AppState>,
) -> AppResult<Option<EmulatorProfile>> {
    let storage = state.engine.storage().clone();
    let profile = emulator::detect_emulator_async(&tree, |rel| {
        let storage = storage.clone();
        let loc = crate::sync::mobile_storage::doc_loc(&tree, &rel);
        async move { storage.exists(&loc).await }
    })
    .await;
    Ok(profile)
}

#[cfg(desktop)]
#[tauri::command]
pub async fn detect_emulator_mobile(_tree: String) -> AppResult<Option<EmulatorProfile>> {
    Err(AppError::Other(
        "detect_emulator_mobile não disponível no desktop".into(),
    ))
}

/// Monta o cliente remoto concreto de `kind` a partir de um `AuthManager` já
/// conectado — o único lugar que sabe qual implementação de
/// `RemoteProvider` corresponde a cada provedor OAuth. `LocalFolder` não usa
/// OAuth e nunca chega aqui (ver `connect_local_folder`).
fn build_oauth_remote(
    kind: ProviderKind,
    http: reqwest::Client,
    auth: Arc<AuthManager>,
    db: Db,
) -> Arc<dyn RemoteProvider> {
    match kind {
        ProviderKind::GoogleDrive => Arc::new(crate::drive::DriveClient::new(http, auth, db)),
        ProviderKind::Dropbox => Arc::new(crate::dropbox::DropboxClient::new(http, auth)),
        ProviderKind::OneDrive => Arc::new(crate::onedrive::OneDriveClient::new(http, auth)),
        ProviderKind::LocalFolder => {
            unreachable!("LocalFolder não usa AuthManager/build_oauth_remote")
        }
    }
}

/// Troca o `AuthManager`/provedor remoto ativos no `AppState`/`SyncEngine` —
/// efetivo imediatamente, sem reiniciar o app — e persiste qual provedor
/// ficou ativo. Chamado depois que a conexão (OAuth ou validação de pasta)
/// já deu certo.
async fn activate_provider<R: Runtime>(
    state: &State<'_, AppState<R>>,
    kind: ProviderKind,
    auth: Option<Arc<AuthManager>>,
    remote: Arc<dyn RemoteProvider>,
) -> AppResult<()> {
    {
        let mut guard = state
            .auth
            .write()
            .map_err(|_| AppError::Other("lock de autenticação envenenado".into()))?;
        *guard = auth;
    }
    state.engine.set_remote_provider(Some(remote));
    state
        .db
        .with(move |conn| settings::set_storage_provider(conn, kind))
        .await
}

/// Abre o navegador para o consentimento OAuth2 e aguarda a autorização.
/// Desktop: TCP loopback (RFC 8252). Mobile: deep link `slot2sync://oauth`
/// (ver `connect_oauth_mobile`, compartilhado pelos três provedores OAuth).
#[cfg(desktop)]
#[tauri::command]
pub async fn connect_google_drive(
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<AuthStatus> {
    connect_oauth_desktop(ProviderKind::GoogleDrive, &app, &state).await
}

#[cfg(desktop)]
#[tauri::command]
pub async fn connect_dropbox(app: AppHandle, state: State<'_, AppState>) -> AppResult<AuthStatus> {
    connect_oauth_desktop(ProviderKind::Dropbox, &app, &state).await
}

#[cfg(desktop)]
#[tauri::command]
pub async fn connect_onedrive(app: AppHandle, state: State<'_, AppState>) -> AppResult<AuthStatus> {
    connect_oauth_desktop(ProviderKind::OneDrive, &app, &state).await
}

#[cfg(desktop)]
async fn connect_oauth_desktop<R: Runtime>(
    kind: ProviderKind,
    app: &AppHandle<R>,
    state: &State<'_, AppState<R>>,
) -> AppResult<AuthStatus> {
    let auth = Arc::new(AuthManager::new_for(
        kind,
        state.http.clone(),
        state.secrets.clone(),
    ));
    let status = auth.connect().await?;
    let remote = build_oauth_remote(kind, state.http.clone(), auth.clone(), state.db.clone());
    activate_provider(state, kind, Some(auth), remote).await?;
    let _ = app.emit(EVT_AUTH_STATUS, &status);
    Ok(status)
}

/// Variante mobile, compartilhada pelos três provedores OAuth: registra o
/// listener de deep link antes de abrir o browser, para não perder o
/// redirect caso o app já esteja rodando em background.
#[cfg(mobile)]
#[tauri::command]
pub async fn connect_google_drive(
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<AuthStatus> {
    connect_oauth_mobile(ProviderKind::GoogleDrive, app, state).await
}

#[cfg(mobile)]
#[tauri::command]
pub async fn connect_dropbox(app: AppHandle, state: State<'_, AppState>) -> AppResult<AuthStatus> {
    connect_oauth_mobile(ProviderKind::Dropbox, app, state).await
}

#[cfg(mobile)]
#[tauri::command]
pub async fn connect_onedrive(app: AppHandle, state: State<'_, AppState>) -> AppResult<AuthStatus> {
    connect_oauth_mobile(ProviderKind::OneDrive, app, state).await
}

#[cfg(mobile)]
async fn connect_oauth_mobile<R: Runtime>(
    kind: ProviderKind,
    app: AppHandle<R>,
    state: State<'_, AppState<R>>,
) -> AppResult<AuthStatus> {
    use std::sync::Mutex;
    use tokio::sync::oneshot;

    let auth = Arc::new(AuthManager::new_for(
        kind,
        state.http.clone(),
        state.secrets.clone(),
    ));

    let (tx, rx) = oneshot::channel::<String>();
    let tx = Arc::new(Mutex::new(Some(tx)));

    // O payload do evento `deep-link://new-url` é um array JSON de URLs.
    let listener_id = {
        let tx = tx.clone();
        app.once("deep-link://new-url", move |event| {
            let urls: Vec<String> = serde_json::from_str(event.payload()).unwrap_or_default();
            if let Some(url) = urls
                .into_iter()
                .find(|u| u.starts_with("com.slot2sync.app:/oauth2redirect"))
            {
                if let Some(sender) = tx.lock().unwrap().take() {
                    let _ = sender.send(url);
                }
            }
        })
    };

    let result = auth.connect_mobile(&app, rx).await;
    if result.is_err() {
        app.unlisten(listener_id);
        return result;
    }
    let status = result?;
    let remote = build_oauth_remote(kind, state.http.clone(), auth.clone(), state.db.clone());
    activate_provider(&state, kind, Some(auth), remote).await?;
    let _ = app.emit(EVT_AUTH_STATUS, &status);
    Ok(status)
}

/// Conecta a uma pasta local ou de rede como provedor de storage — sem
/// OAuth: só valida que o caminho existe e é gravável. Cria a pasta se ainda
/// não existir (comportamento útil para um caminho de rede recém-mapeado).
#[tauri::command]
pub async fn connect_local_folder(
    state: State<'_, AppState>,
    path: String,
) -> AppResult<AuthStatus> {
    connect_local_folder_impl(&state, path).await
}

async fn connect_local_folder_impl<R: Runtime>(
    state: &State<'_, AppState<R>>,
    path: String,
) -> AppResult<AuthStatus> {
    let root = PathBuf::from(&path);
    tokio::fs::create_dir_all(&root).await.map_err(|e| {
        AppError::Other(format!(
            "não foi possível criar/acessar a pasta \"{path}\": {e}"
        ))
    })?;
    let probe = root.join(".slot2sync-write-test");
    tokio::fs::write(&probe, b"ok")
        .await
        .map_err(|e| AppError::Other(format!("pasta não é gravável \"{path}\": {e}")))?;
    let _ = tokio::fs::remove_file(&probe).await;

    let remote: Arc<dyn RemoteProvider> = Arc::new(crate::folder::FolderProvider::new(root));
    activate_provider(state, ProviderKind::LocalFolder, None, remote).await?;
    let path_to_store = path.clone();
    state
        .db
        .with(move |conn| settings::set_folder_provider_path(conn, &path_to_store))
        .await?;

    let status = AuthStatus {
        connected: true,
        email: None,
    };
    Ok(status)
}

/// Status atual do provedor configurado, sem disparar fluxo interativo.
/// Provedores OAuth: consulta só o keyring (não exige rede). `LocalFolder`:
/// confere se o caminho salvo ainda existe. Nenhum provedor escolhido ainda
/// (primeiro uso): desconectado — a UI mostra o seletor de provedor.
#[tauri::command]
pub async fn get_auth_status(state: State<'_, AppState>) -> AppResult<AuthStatus> {
    get_auth_status_impl(&state).await
}

async fn get_auth_status_impl<R: Runtime>(state: &State<'_, AppState<R>>) -> AppResult<AuthStatus> {
    let stored = state.db.with(settings::storage_provider).await?;
    match stored {
        None => Ok(AuthStatus::disconnected()),
        Some(ProviderKind::LocalFolder) => {
            let path = state.db.with(settings::folder_provider_path).await?;
            let connected = path.as_deref().is_some_and(|p| PathBuf::from(p).is_dir());
            Ok(AuthStatus {
                connected,
                email: None,
            })
        }
        Some(_oauth_provider) => {
            let auth = {
                state
                    .auth
                    .read()
                    .map_err(|_| AppError::Other("lock de autenticação envenenado".into()))?
                    .clone()
            };
            match auth {
                Some(auth) => auth.status().await,
                None => Ok(AuthStatus::disconnected()),
            }
        }
    }
}

/// Desconecta do provedor ativo (qualquer que seja) e limpa a config
/// persistida — a UI volta a mostrar o seletor de provedor, sem reiniciar o
/// app. Para provedores OAuth, também remove o refresh token do keyring.
#[tauri::command]
pub async fn disconnect_provider(
    app: AppHandle,
    state: State<'_, AppState>,
) -> AppResult<AuthStatus> {
    disconnect_provider_impl(&app, &state).await
}

async fn disconnect_provider_impl<R: Runtime>(
    app: &AppHandle<R>,
    state: &State<'_, AppState<R>>,
) -> AppResult<AuthStatus> {
    let auth = {
        state
            .auth
            .read()
            .map_err(|_| AppError::Other("lock de autenticação envenenado".into()))?
            .clone()
    };
    if let Some(auth) = auth {
        auth.disconnect().await?;
    }
    {
        let mut guard = state
            .auth
            .write()
            .map_err(|_| AppError::Other("lock de autenticação envenenado".into()))?;
        *guard = None;
    }
    // Os IDs/paths de pasta cacheados são por provedor — zera para não
    // reaproveitá-los ao conectar com outro provedor/conta.
    state.engine.clear_folder_cache().await;
    state.engine.set_remote_provider(None);
    state.db.with(settings::clear_storage_provider).await?;

    let status = AuthStatus::disconnected();
    let _ = app.emit(EVT_AUTH_STATUS, &status);
    Ok(status)
}

/// Valida via `LocalStorage` que `path` aponta para uma pasta acessível. No
/// desktop é `Path::is_dir`; no mobile a raiz é uma URI SAF conferida pelo
/// plugin nativo. Centraliza a checagem que antes vazava `std::fs` e era pulada
/// no mobile.
async fn ensure_valid_root(state: &State<'_, AppState>, path: &str) -> AppResult<()> {
    let loc = FileLoc::from_path(PathBuf::from(path));
    if state.storage.is_valid_root(&loc).await {
        Ok(())
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("pasta não encontrada: {path}"),
        )
        .into())
    }
}

/// Identifica o emulador presente na pasta selecionada pelo usuário.
/// `Ok(None)` = pasta válida, mas nenhum emulador suportado reconhecido.
#[tauri::command]
pub async fn detect_emulator(
    state: State<'_, AppState>,
    path: String,
) -> AppResult<Option<EmulatorProfile>> {
    ensure_valid_root(&state, &path).await?;
    let root = PathBuf::from(&path);

    tokio::task::spawn_blocking(move || Ok(emulator::detect_emulator(&root)))
        .await
        .map_err(|e| AppError::Other(format!("tarefa bloqueante abortada: {e}")))?
}

/// Detecta o emulador na pasta e o registra para sincronização.
#[cfg(not(mobile))]
#[tauri::command]
pub async fn add_emulator(state: State<'_, AppState>, path: String) -> AppResult<EmulatorProfile> {
    ensure_valid_root(&state, &path).await?;
    let root = PathBuf::from(&path);

    let profile = tokio::task::spawn_blocking(move || emulator::detect_emulator(&root))
        .await
        .map_err(|e| AppError::Other(format!("tarefa bloqueante abortada: {e}")))?
        .ok_or(AppError::EmulatorNotDetected(path))?;

    persist_detected(&state, profile).await
}

/// Variante mobile: `path` é a URI SAF concedida, não um caminho de
/// filesystem — a detecção passa pelo mesmo mecanismo assíncrono usado por
/// `detect_emulator_mobile`, em vez de `emulator::detect_emulator` (que
/// depende de `is_dir()` e sempre falharia aqui).
#[cfg(mobile)]
#[tauri::command]
pub async fn add_emulator(state: State<'_, AppState>, path: String) -> AppResult<EmulatorProfile> {
    let storage = state.engine.storage().clone();
    let tree = path.clone();
    let profile = emulator::detect_emulator_async(&tree, |rel| {
        let storage = storage.clone();
        let loc = crate::sync::mobile_storage::doc_loc(&tree, &rel);
        async move { storage.exists(&loc).await }
    })
    .await
    .ok_or(AppError::EmulatorNotDetected(path))?;

    persist_detected(&state, profile).await
}

/// Grava o perfil detectado (upsert por raiz, com reset de estado de sync se o
/// caminho mudou) — compartilhado pelas duas variantes de `add_emulator`.
async fn persist_detected(
    state: &State<'_, AppState>,
    profile: EmulatorProfile,
) -> AppResult<EmulatorProfile> {
    // Marcador inerte na raiz (ver `constants::LOCAL_ROOT_MARKER`) — metadado
    // para uma futura heurística de detecção de desconexão, não checado hoje.
    #[cfg(desktop)]
    {
        let marker = profile.root_path.join(crate::constants::LOCAL_ROOT_MARKER);
        let _ = tokio::fs::write(&marker, "").await;
    }

    let to_store = profile.clone();
    let path_reset = state
        .db
        .with(move |conn| emulators::upsert_resetting_on_path_change(conn, &to_store))
        .await?;
    if path_reset {
        tracing::info!(
            emulador = %profile.name,
            raiz = %profile.root_path.display(),
            "caminho do emulador alterado; estado de sync reiniciado (manifest, conflitos e fila zerados)"
        );
    } else {
        tracing::info!(emulador = %profile.name, raiz = %profile.root_path.display(), "emulador adicionado");
    }
    Ok(profile)
}

/// Registra um emulador cujas pastas o usuário informou manualmente — fallback
/// quando a detecção automática falha (instalação portátil ou fora do catálogo).
/// Os caminhos chegam relativos à raiz. Não sobrescreve um emulador já existente.
#[tauri::command]
pub async fn add_emulator_manual(
    state: State<'_, AppState>,
    name: String,
    path: String,
    saves_paths: Vec<String>,
    state_paths: Vec<String>,
    config_paths: Vec<String>,
) -> AppResult<EmulatorProfile> {
    let root = PathBuf::from(&path);
    let root_loc = FileLoc::from_path(root.clone());
    // No mobile o path é uma URI SAF (content://...); a checagem de existência
    // passa pelo plugin nativo via LocalStorage, não por std::fs.
    ensure_valid_root(&state, &path).await?;

    let profile =
        emulator::build_manual_profile(&root, name, saves_paths, state_paths, config_paths)
            .map_err(AppError::Other)?;

    // Cada pasta informada precisa existir sob a raiz. A checagem sai do
    // `build_manual_profile` (puro) e passa pelo `LocalStorage`, que sabe tratar
    // tanto caminhos nativos quanto URIs SAF.
    for rel in profile
        .saves_paths
        .iter()
        .chain(&profile.state_paths)
        .chain(&profile.config_paths)
    {
        let rel_str = rel.to_string_lossy().replace('\\', "/");
        if !state.storage.subdir_exists(&root_loc, &rel_str).await {
            return Err(AppError::Other(format!(
                "pasta não encontrada sob a raiz: {rel_str}"
            )));
        }
    }

    let name_check = profile.name.clone();
    if state
        .db
        .with(move |conn| emulators::exists(conn, &name_check))
        .await?
    {
        return Err(AppError::EmulatorExists(profile.name));
    }

    let to_store = profile.clone();
    state
        .db
        .with(move |conn| emulators::upsert(conn, &to_store))
        .await?;
    tracing::info!(emulador = %profile.name, raiz = %profile.root_path.display(), "emulador manual adicionado");
    Ok(profile)
}

/// Varre locais conhecidos e o registro do Windows por emuladores do catálogo
/// instalados no sistema. Não persiste nada — a UI usa o resultado para sugerir
/// adições em um clique.
#[tauri::command]
pub async fn discover_emulators() -> AppResult<Vec<emulator::DiscoveredEmulator>> {
    tokio::task::spawn_blocking(emulator::discover_installed)
        .await
        .map_err(|e| AppError::Other(format!("tarefa bloqueante abortada: {e}")))
}

#[tauri::command]
pub async fn list_emulators(state: State<'_, AppState>) -> AppResult<Vec<EmulatorProfile>> {
    state.db.with(emulators::list).await
}

/// Jogos cujos arquivos foram sincronizados, agregados a partir do manifest e
/// com o serial traduzido para nome legível quando conhecido. A
/// UI lista por emulador; sem nome, exibe o próprio serial.
#[tauri::command]
pub async fn list_synced_games(state: State<'_, AppState>) -> AppResult<Vec<SyncedGame>> {
    let entries = state.db.with(manifest::list_all).await?;
    Ok(games::aggregate(entries))
}

/// Remove o emulador da sincronização (manifest e pendências inclusos).
/// Nada é apagado no Drive nem no disco local.
#[tauri::command]
pub async fn remove_emulator(state: State<'_, AppState>, name: String) -> AppResult<()> {
    state
        .db
        .with(move |conn| {
            emulators::remove(conn, &name)?;
            emulators::remove_categories(conn, &name)?;
            conflicts::remove_for_emulator(conn, &name)?;
            manifest::remove_for_emulator(conn, &name)?;
            crate::storage::stats::remove_for_emulator(conn, &name)?;
            queue::remove_for_emulator(conn, &name)
        })
        .await
}

/// Estatísticas acumuladas de um emulador (uploads, downloads, bytes,
/// conflitos, últimos sync/scan). `None` = nunca houve atividade.
#[tauri::command]
pub async fn get_emulator_stats(
    state: State<'_, AppState>,
    name: String,
) -> AppResult<Option<crate::storage::stats::EmulatorStats>> {
    state
        .db
        .with(move |conn| crate::storage::stats::get(conn, &name))
        .await
}

/// Estatísticas acumuladas de todos os emuladores com atividade — a UI carrega
/// uma vez e distribui pelos cards.
#[tauri::command]
pub async fn list_emulator_stats(
    state: State<'_, AppState>,
) -> AppResult<Vec<crate::storage::stats::EmulatorStats>> {
    state.db.with(crate::storage::stats::list_all).await
}

/// Conflitos pendentes (ambos os lados mudaram). A UI exibe o botão de resolver
/// no card do emulador afetado.
#[tauri::command]
pub async fn list_conflicts(state: State<'_, AppState>) -> AppResult<Vec<Conflict>> {
    state.db.with(conflicts::list_all).await
}

/// Resolve um conflito mantendo a versão escolhida (`local` ou `remote`) e
/// desbloqueia o sync do emulador.
#[tauri::command]
pub async fn resolve_conflict(
    state: State<'_, AppState>,
    emulator: String,
    category: SyncCategory,
    rel_path: String,
    keep: ConflictResolution,
) -> AppResult<()> {
    state
        .engine
        .resolve_conflict(&emulator, category, &rel_path, keep)
        .await
}

/// Categorias de sync habilitadas para um emulador (default: todas ativas).
#[tauri::command]
pub async fn get_emulator_categories(
    state: State<'_, AppState>,
    name: String,
) -> AppResult<SyncCategories> {
    state
        .db
        .with(move |conn| emulators::get_categories(conn, &name))
        .await
}

/// Define quais categorias (saves/savestates/config) sincronizar para um
/// emulador. Desativar `config`, p.ex., evita compartilhar resolução/controles
/// entre dispositivos diferentes.
#[tauri::command]
pub async fn set_emulator_categories(
    state: State<'_, AppState>,
    name: String,
    categories: SyncCategories,
) -> AppResult<()> {
    state
        .db
        .with(move |conn| emulators::set_categories(conn, &name, &categories))
        .await
}

/// Define os padrões glob de exclusão de um emulador (arquivos que casam ficam
/// fora do sync nas duas direções). Valida cada padrão antes de gravar.
#[tauri::command]
pub async fn set_exclude_patterns(
    state: State<'_, AppState>,
    name: String,
    patterns: Vec<String>,
) -> AppResult<()> {
    let patterns: Vec<String> = patterns
        .into_iter()
        .map(|p| p.trim().to_string())
        .filter(|p| !p.is_empty())
        .collect();
    for pattern in &patterns {
        globset::Glob::new(pattern)
            .map_err(|e| AppError::Other(format!("padrão inválido \"{pattern}\": {e}")))?;
    }
    state
        .db
        .with(move |conn| emulators::set_exclude_patterns(conn, &name, &patterns))
        .await
}

/// Sync manual (botão da UI / menu da tray). Bidirecional.
#[tauri::command]
pub async fn sync_now(state: State<'_, AppState>) -> AppResult<SyncSummary> {
    state
        .engine
        .sync_all(SyncDirection::Bidirectional, TRIGGER_MANUAL)
        .await
}

/// Configurações globais do usuário (nome do dispositivo, etc.). O flag de
/// autostart não vive no banco — é lido do SO via plugin e injetado aqui
/// (apenas em desktop; em mobile permanece `false`).
#[tauri::command]
pub async fn get_settings(app: AppHandle, state: State<'_, AppState>) -> AppResult<Settings> {
    let mut settings = state.db.with(settings::load).await?;
    #[cfg(desktop)]
    {
        settings.autostart = autostart_enabled(&app)?;
    }
    #[cfg(not(desktop))]
    {
        let _ = &app;
    }
    Ok(settings)
}

/// Liga/desliga o início automático do Slot2Sync junto com o sistema. O estado
/// é persistido pelo SO (registro do Windows / LaunchAgent), não no banco local.
/// Ao subir pelo SO, o app é lançado com `--minimized` e fica só na bandeja.
#[cfg(desktop)]
#[tauri::command]
pub async fn set_autostart(app: AppHandle, enabled: bool) -> AppResult<()> {
    let manager = app.autolaunch();
    let result = if enabled {
        manager.enable()
    } else {
        manager.disable()
    };
    result.map_err(|e| AppError::Other(format!("falha ao configurar o autostart: {e}")))
}

/// No mobile não existe "subir com o sistema". O comando segue exposto (no-op)
/// para manter a boundary IPC idêntica entre plataformas.
#[cfg(mobile)]
#[tauri::command]
pub async fn set_autostart(app: AppHandle, enabled: bool) -> AppResult<()> {
    let _ = (&app, enabled);
    Ok(())
}

/// Lê do SO se o Slot2Sync está registrado para iniciar com o sistema.
#[cfg(desktop)]
fn autostart_enabled(app: &AppHandle) -> AppResult<bool> {
    app.autolaunch()
        .is_enabled()
        .map_err(|e| AppError::Other(format!("autostart indisponível: {e}")))
}

/// No mobile o conceito não existe — sempre `false`.
#[cfg(mobile)]
fn autostart_enabled(_app: &AppHandle) -> AppResult<bool> {
    Ok(false)
}

/// Abre a pasta de backups locais no gerenciador de arquivos do SO. A pasta é
/// criada se ainda não existir (recebe os backups do primeiro sync).
#[cfg(desktop)]
#[tauri::command]
pub async fn open_backup_folder(app: AppHandle) -> AppResult<()> {
    let dir = crate::locations::AppPath::BackupDir.resolve(&app)?;
    tokio::fs::create_dir_all(&dir).await?;
    tokio::task::spawn_blocking(move || open::that(&dir))
        .await
        .map_err(|e| AppError::Other(format!("tarefa bloqueante abortada: {e}")))??;
    Ok(())
}

/// Mostra um arquivo de backup no gerenciador de arquivos do SO (abre a pasta
/// que o contém). Restrito à árvore de backups do app — recusa qualquer
/// caminho fora dela.
#[cfg(desktop)]
#[tauri::command]
pub async fn reveal_backup_path(app: AppHandle, path: String) -> AppResult<()> {
    let backups_root = crate::locations::AppPath::BackupDir.resolve(&app)?;
    let target = PathBuf::from(&path);
    let canonical = tokio::fs::canonicalize(&target).await?;
    let root_canonical = tokio::fs::canonicalize(&backups_root).await?;
    if !canonical.starts_with(&root_canonical) {
        return Err(AppError::Other(
            "caminho fora da pasta de backups do Slot2Sync".into(),
        ));
    }
    let dir = canonical
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or(canonical);
    tokio::task::spawn_blocking(move || open::that(&dir))
        .await
        .map_err(|e| AppError::Other(format!("tarefa bloqueante abortada: {e}")))??;
    Ok(())
}

/// Liga/desliga os gatilhos de sync automático. O sync manual (botão/tray) não
/// é afetado por estes flags.
#[tauri::command]
pub async fn set_triggers(state: State<'_, AppState>, triggers: TriggerSettings) -> AppResult<()> {
    state
        .db
        .with(move |conn| settings::set_triggers(conn, &triggers))
        .await
}

/// Define a retenção dos backups locais em dias (0 = manter para sempre).
/// A limpeza roda no próximo startup do app.
#[tauri::command]
pub async fn set_backup_retention_days(state: State<'_, AppState>, days: u32) -> AppResult<()> {
    state
        .db
        .with(move |conn| settings::set_backup_retention_days(conn, days))
        .await
}

/// Define o máximo de versões arquivadas por arquivo no histórico
/// pré-download (mínimo 1).
#[tauri::command]
pub async fn set_max_backup_versions(state: State<'_, AppState>, versions: u32) -> AppResult<()> {
    state
        .db
        .with(move |conn| settings::set_max_backup_versions(conn, versions))
        .await
}

/// Define os limites de banda das transferências em KB/s (0 = ilimitado).
/// Aplicados imediatamente — o cliente relê os valores a cada operação.
#[tauri::command]
pub async fn set_bandwidth_limits(
    state: State<'_, AppState>,
    upload_kbps: u32,
    download_kbps: u32,
) -> AppResult<()> {
    state
        .db
        .with(move |conn| settings::set_bandwidth_limits(conn, upload_kbps, download_kbps))
        .await
}

/// Define o intervalo do scan periódico em minutos (0 = desativado). O timer
/// relê o valor a cada ciclo — não precisa reiniciar o app.
#[tauri::command]
pub async fn set_scan_interval_minutes(state: State<'_, AppState>, minutes: u32) -> AppResult<()> {
    state
        .db
        .with(move |conn| settings::set_scan_interval_minutes(conn, minutes))
        .await
}

/// Define o nível de notificações nativas (all | errors_only | none).
#[tauri::command]
pub async fn set_notification_level(
    state: State<'_, AppState>,
    level: NotificationLevel,
) -> AppResult<()> {
    state
        .db
        .with(move |conn| settings::set_notification_level(conn, level))
        .await
}

/// Define o nome amigável deste dispositivo. Obrigatório no login; pode ser
/// alterado nas configurações sem refazer a autenticação.
#[tauri::command]
pub async fn set_device_name(state: State<'_, AppState>, name: String) -> AppResult<()> {
    let trimmed = name.trim().to_string();
    if trimmed.is_empty() {
        return Err(AppError::Other(
            "o nome do dispositivo não pode ser vazio".into(),
        ));
    }
    state
        .db
        .with(move |conn| settings::set_device_name(conn, &trimmed))
        .await
}

/// Último sync concluído (para a UI exibir ao montar). `None` se ainda não
/// houve nenhum nesta execução.
#[tauri::command]
pub fn get_last_sync(state: State<'_, AppState>) -> AppResult<Option<LastSync>> {
    let guard = state
        .last_sync
        .lock()
        .map_err(|_| AppError::Other("lock do último sync envenenado".into()))?;
    Ok(guard.clone())
}

/// Fila offline visível: arquivos cuja transferência falhou (rede/arquivo em
/// uso) e será refeita no próximo sync. A UI exibe o badge "N pendentes" no
/// card do emulador e a lista com o último erro de cada arquivo.
#[tauri::command]
pub async fn list_pending_ops(state: State<'_, AppState>) -> AppResult<Vec<queue::PendingOp>> {
    state.db.with(queue::list_all).await
}

/// Retrato do `SyncState` corrente do engine. (→ ipc.ts)
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStateSnapshot {
    pub state: &'static str,
    pub emulator: Option<String>,
    /// Só preenchido quando `state == "error"`.
    pub error_message: Option<String>,
}

/// Estado corrente do sync (`idle`/`scanning`/`syncing`/`conflict`/`error`) —
/// permite ao frontend renderizar o estado certo ao reconectar no meio de um
/// sync, sem depender de ter recebido os eventos `sync:*` anteriores.
#[tauri::command]
pub fn get_sync_state(state: State<'_, AppState>) -> SyncStateSnapshot {
    let (sync_state, emulator) = state.engine.current_sync_state();
    let error_message = match &sync_state {
        crate::sync::SyncState::Error(msg) => Some(msg.clone()),
        _ => None,
    };
    SyncStateSnapshot {
        state: sync_state.as_str(),
        emulator,
        error_message,
    }
}

/// Ação "tentar novamente" da fila offline: zera as tentativas e o backoff de
/// um arquivo (inclusive pendências mortas), liberando a retentativa no próximo
/// sync.
#[tauri::command]
pub async fn retry_pending_op(
    state: State<'_, AppState>,
    emulator: String,
    category: SyncCategory,
    rel_path: String,
) -> AppResult<()> {
    state
        .db
        .with(move |conn| queue::retry_now(conn, &emulator, category, &rel_path))
        .await
}

/// IDs de banners informativos que o usuário dispensou (não reaparecem).
#[tauri::command]
pub async fn list_dismissed_notices(state: State<'_, AppState>) -> AppResult<Vec<String>> {
    state.db.with(settings::dismissed_notices).await
}

/// Dispensa um banner informativo de forma persistente (idempotente).
#[tauri::command]
pub async fn dismiss_notice(state: State<'_, AppState>, id: String) -> AppResult<()> {
    state
        .db
        .with(move |conn| settings::dismiss_notice(conn, &id))
        .await
}

/// Versões arquivadas de um arquivo no histórico pré-download
/// (`<backups>/<emulador>/history/<categoria>/…`), mais recentes primeiro.
#[tauri::command]
pub async fn list_file_versions(
    app: AppHandle,
    emulator: String,
    category: SyncCategory,
    rel_path: String,
) -> AppResult<Vec<crate::versioning::FileVersion>> {
    let dir = crate::locations::AppPath::BackupDir.resolve(&app)?;
    tokio::task::spawn_blocking(move || {
        use crate::versioning::Versioner;
        crate::versioning::FsVersioner::new(dir).versions(&emulator, category.as_str(), &rel_path)
    })
    .await
    .map_err(|e| AppError::Other(format!("tarefa bloqueante abortada: {e}")))?
}

/// Restaura uma versão arquivada por cima do arquivo atual do emulador.
/// `versioned_rel_path` é o caminho relativo dentro de
/// `history/<categoria>/` como listado no histórico (nome com carimbo).
/// O estado atual é arquivado ANTES da restauração — nada se perde. O arquivo
/// restaurado recebe mtime atual, então o próximo sync o envia ao Drive.
#[tauri::command]
pub async fn restore_version(
    app: AppHandle,
    state: State<'_, AppState>,
    emulator: String,
    category: SyncCategory,
    versioned_rel_path: String,
) -> AppResult<()> {
    let backups_dir = crate::locations::AppPath::BackupDir.resolve(&app)?;

    // Valida o caminho versionado e deriva origem + rel_path original
    // (lógica pura e testada em `versioning::resolve_restore`).
    let (src_abs, original_rel) = {
        let (dir, emu, cat, rel) = (
            backups_dir.clone(),
            emulator.clone(),
            category.as_str().to_string(),
            versioned_rel_path.clone(),
        );
        tokio::task::spawn_blocking(move || {
            crate::versioning::resolve_restore(&dir, &emu, &cat, &rel)
        })
        .await
        .map_err(|e| AppError::Other(format!("tarefa bloqueante abortada: {e}")))??
    };

    // Perfil e primeira pasta-base da categoria → destino da restauração.
    let name = emulator.clone();
    let profile = state
        .db
        .with(emulators::list)
        .await?
        .into_iter()
        .find(|p| p.name == name)
        .ok_or_else(|| AppError::Other(format!("emulador não encontrado: {emulator}")))?;
    let bases = match category {
        SyncCategory::Saves => &profile.saves_paths,
        SyncCategory::Savestates => &profile.state_paths,
        SyncCategory::Config => &profile.config_paths,
    };
    let base = bases
        .first()
        .ok_or_else(|| AppError::Other(format!("categoria sem pasta configurada: {emulator}")))?;
    let root_loc = state.storage.root_loc(&profile.root_path);
    let base_loc = state
        .storage
        .join(&root_loc, &base.to_string_lossy().replace('\\', "/"));
    let dest = state.storage.join(&base_loc, &original_rel);

    // Arquiva o estado ATUAL antes de sobrescrever (best-effort; sem estado
    // atual — arquivo já apagado — só restaura).
    if state.storage.exists(&dest).await {
        let max_versions = state
            .db
            .with(settings::max_backup_versions)
            .await
            .unwrap_or(crate::constants::MAX_BACKUP_VERSIONS_DEFAULT)
            as usize;
        let current = state.storage.loc_to_stored(&dest);
        let (emu, cat, rel) = (
            emulator.clone(),
            category.as_str().to_string(),
            original_rel.clone(),
        );
        let dir = backups_dir.clone();
        let archived = tokio::task::spawn_blocking(move || {
            use crate::versioning::Versioner;
            crate::versioning::FsVersioner::new(dir).archive(
                &emu,
                &cat,
                &rel,
                std::path::Path::new(&current),
                max_versions,
            )
        })
        .await
        .map_err(|e| AppError::Other(format!("tarefa bloqueante abortada: {e}")))?;
        if let Err(err) = archived {
            tracing::warn!(error = %err, "falha ao arquivar o estado atual antes de restaurar");
        }
    }

    // Restaura com mtime atual: o próximo sync envia a versão restaurada.
    let content = tokio::fs::read(&src_abs).await?;
    state.storage.write_atomic(&dest, &content, None).await?;
    tracing::info!(
        emulador = %emulator,
        arquivo = %original_rel,
        versao = %versioned_rel_path,
        "versão restaurada do histórico"
    );
    Ok(())
}

/// Histórico dos backups locais que o Slot2Sync criou antes de sobrescrever
/// arquivos (primeiro sync e resolução de conflito). Só leitura — restauração
/// continua manual, pela pasta.
#[tauri::command]
pub async fn list_backups(app: AppHandle) -> AppResult<Vec<crate::backups::BackupEntry>> {
    let dir = crate::locations::AppPath::BackupDir.resolve(&app)?;
    tokio::task::spawn_blocking(move || crate::backups::list(&dir))
        .await
        .map_err(|e| AppError::Other(format!("tarefa bloqueante abortada: {e}")))?
}

/// Cobre a lógica de troca/consulta de provedor extraída dos comandos acima
/// (`activate_provider`, `*_impl`, `build_oauth_remote`) — a parte
/// unitariamente testável sem uma janela real. O fluxo interativo completo
/// (`connect_oauth_desktop`/`connect_oauth_mobile`: abrir navegador, esperar
/// o redirect) fica de fora — mesmo racional do `ignore:` do `codecov.yml`
/// para bootstrap/orquestração, validado manualmente.
#[cfg(all(test, desktop, not(windows)))]
mod tests {
    use tauri::test::MockRuntime;

    use super::*;
    use crate::drive::mock::MockDrive;
    use crate::secrets::{MemSecrets, SecretStore};
    use crate::sync::{DesktopStorage, LastSyncStore, LocalStorage, SyncEngine};

    /// Monta um `App<MockRuntime>` com um `AppState<MockRuntime>` gerenciado —
    /// SQLite em memória, `MockDrive` como provedor remoto (não usado
    /// diretamente por estes testes, só precisa existir para o engine), sem
    /// provedor/keyring configurado. Devolvido junto com o `TempDir` dos
    /// backups (precisa sobreviver ao teste).
    async fn build_app() -> (tauri::App<MockRuntime>, tempfile::TempDir) {
        let tmp = tempfile::tempdir().unwrap();
        let db = Db::open_in_memory().unwrap();
        let secrets: Arc<dyn SecretStore> = Arc::new(MemSecrets::default());
        let drive = Arc::new(MockDrive::new());
        let remote: Arc<dyn RemoteProvider> = drive;
        let last_sync = LastSyncStore::default();

        let app = tauri::test::mock_builder()
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();

        let engine = Arc::new(SyncEngine::new(
            db.clone(),
            Some(remote),
            app.handle().clone(),
            last_sync.clone(),
            tmp.path().join("backups"),
            Arc::new(DesktopStorage) as Arc<dyn LocalStorage>,
            secrets.clone(),
        ));

        app.manage(AppState::<MockRuntime> {
            auth: std::sync::RwLock::new(None),
            db,
            engine,
            last_sync,
            storage: Arc::new(DesktopStorage) as Arc<dyn LocalStorage>,
            http: reqwest::Client::new(),
            secrets,
        });

        (app, tmp)
    }

    #[tokio::test]
    async fn health_check_reporta_versao_pronto_e_metricas_do_banco() {
        let (app, _tmp) = build_app().await;
        let state = app.state::<AppState<MockRuntime>>();

        let status = health_check_impl(&state).await.unwrap();
        assert!(status.ready);
        assert!(!status.version.is_empty());
        assert_eq!(status.pending_ops_count, 0);
        // Banco novo não é vazio (migrações já criaram tabelas).
        assert!(status.db_size_bytes > 0);
    }

    #[tokio::test]
    async fn discover_emulators_nao_falha_sem_instalacao_nenhuma() {
        // Não afirma quantidade (depende da máquina) — só que a tarefa
        // bloqueante completa e devolve uma lista (possivelmente vazia).
        discover_emulators().await.unwrap();
    }

    #[tokio::test]
    async fn build_oauth_remote_constroi_o_cliente_certo_por_provedor() {
        let db = Db::open_in_memory().unwrap();
        let secrets: Arc<dyn SecretStore> = Arc::new(MemSecrets::default());
        let http = reqwest::Client::new();

        for kind in [
            ProviderKind::GoogleDrive,
            ProviderKind::Dropbox,
            ProviderKind::OneDrive,
        ] {
            let auth = Arc::new(AuthManager::new_for(kind, http.clone(), secrets.clone()));
            let _remote = build_oauth_remote(kind, http.clone(), auth, db.clone());
        }
    }

    #[tokio::test]
    #[should_panic(expected = "LocalFolder não usa AuthManager/build_oauth_remote")]
    async fn build_oauth_remote_e_inalcancavel_para_local_folder() {
        let db = Db::open_in_memory().unwrap();
        let secrets: Arc<dyn SecretStore> = Arc::new(MemSecrets::default());
        let http = reqwest::Client::new();
        let auth = Arc::new(AuthManager::new_for(
            ProviderKind::GoogleDrive,
            http.clone(),
            secrets,
        ));
        let _ = build_oauth_remote(ProviderKind::LocalFolder, http, auth, db);
    }

    #[tokio::test]
    async fn activate_provider_troca_auth_engine_e_persiste_settings() {
        let (app, _tmp) = build_app().await;
        let state = app.state::<AppState<MockRuntime>>();

        let auth = Arc::new(AuthManager::new_for(
            ProviderKind::Dropbox,
            state.http.clone(),
            state.secrets.clone(),
        ));
        let remote: Arc<dyn RemoteProvider> = Arc::new(MockDrive::new());
        activate_provider(&state, ProviderKind::Dropbox, Some(auth), remote)
            .await
            .unwrap();

        assert!(state.auth.read().unwrap().is_some());
        let stored = state.db.with(settings::storage_provider).await.unwrap();
        assert_eq!(stored, Some(ProviderKind::Dropbox));
    }

    #[tokio::test]
    async fn activate_provider_local_folder_sem_auth() {
        let (app, _tmp) = build_app().await;
        let state = app.state::<AppState<MockRuntime>>();

        let remote: Arc<dyn RemoteProvider> = Arc::new(MockDrive::new());
        activate_provider(&state, ProviderKind::LocalFolder, None, remote)
            .await
            .unwrap();

        assert!(state.auth.read().unwrap().is_none());
        let stored = state.db.with(settings::storage_provider).await.unwrap();
        assert_eq!(stored, Some(ProviderKind::LocalFolder));
    }

    #[tokio::test]
    async fn get_auth_status_sem_provedor_escolhido_e_desconectado() {
        let (app, _tmp) = build_app().await;
        let state = app.state::<AppState<MockRuntime>>();

        let status = get_auth_status_impl(&state).await.unwrap();
        assert!(!status.connected);
        assert!(status.email.is_none());
    }

    #[tokio::test]
    async fn get_auth_status_local_folder_reflete_existencia_do_caminho() {
        let (app, _tmp) = build_app().await;
        let state = app.state::<AppState<MockRuntime>>();

        let existing = tempfile::tempdir().unwrap();
        state
            .db
            .with({
                let path = existing.path().display().to_string();
                move |conn| {
                    settings::set_storage_provider(conn, ProviderKind::LocalFolder)?;
                    settings::set_folder_provider_path(conn, &path)
                }
            })
            .await
            .unwrap();
        let status = get_auth_status_impl(&state).await.unwrap();
        assert!(status.connected);

        state
            .db
            .with(|conn| {
                settings::set_folder_provider_path(conn, "/caminho/que/nao/existe/slot2sync-x")
            })
            .await
            .unwrap();
        let status = get_auth_status_impl(&state).await.unwrap();
        assert!(!status.connected);
    }

    #[tokio::test]
    async fn get_auth_status_oauth_sem_auth_manager_carregado_e_desconectado() {
        let (app, _tmp) = build_app().await;
        let state = app.state::<AppState<MockRuntime>>();

        state
            .db
            .with(|conn| settings::set_storage_provider(conn, ProviderKind::GoogleDrive))
            .await
            .unwrap();
        // Provedor persistido, mas nenhum AuthManager carregado nesta sessão
        // (ex.: app reiniciou e ainda não tentou reconectar) — desconectado.
        let status = get_auth_status_impl(&state).await.unwrap();
        assert!(!status.connected);
    }

    #[tokio::test]
    async fn get_auth_status_oauth_com_auth_manager_delega_ao_keyring() {
        let (app, _tmp) = build_app().await;
        let state = app.state::<AppState<MockRuntime>>();

        state
            .db
            .with(|conn| settings::set_storage_provider(conn, ProviderKind::GoogleDrive))
            .await
            .unwrap();
        let auth = Arc::new(AuthManager::new_for(
            ProviderKind::GoogleDrive,
            state.http.clone(),
            state.secrets.clone(),
        ));
        *state.auth.write().unwrap() = Some(auth);

        // Sem refresh token salvo no keyring em memória: desconectado (não
        // lança erro nem tenta rede).
        let status = get_auth_status_impl(&state).await.unwrap();
        assert!(!status.connected);
    }

    #[tokio::test]
    async fn disconnect_provider_zera_estado_e_limpa_settings() {
        let (app, _tmp) = build_app().await;
        let state = app.state::<AppState<MockRuntime>>();

        let auth = Arc::new(AuthManager::new_for(
            ProviderKind::Dropbox,
            state.http.clone(),
            state.secrets.clone(),
        ));
        let remote: Arc<dyn RemoteProvider> = Arc::new(MockDrive::new());
        activate_provider(&state, ProviderKind::Dropbox, Some(auth), remote)
            .await
            .unwrap();

        let status = disconnect_provider_impl(app.handle(), &state)
            .await
            .unwrap();
        assert!(!status.connected);
        assert!(state.auth.read().unwrap().is_none());
        let stored = state.db.with(settings::storage_provider).await.unwrap();
        assert_eq!(stored, None);
    }

    #[tokio::test]
    async fn connect_local_folder_impl_cria_pasta_gravavel_e_persiste_caminho() {
        let (app, _tmp) = build_app().await;
        let state = app.state::<AppState<MockRuntime>>();

        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("nested").join("provider-root");
        let status = connect_local_folder_impl(&state, target.display().to_string())
            .await
            .unwrap();

        assert!(status.connected);
        assert!(target.is_dir());
        let stored = state.db.with(settings::storage_provider).await.unwrap();
        assert_eq!(stored, Some(ProviderKind::LocalFolder));
        let saved_path = state.db.with(settings::folder_provider_path).await.unwrap();
        assert_eq!(
            saved_path.as_deref(),
            Some(target.display().to_string().as_str())
        );
    }

    #[tokio::test]
    async fn connect_local_folder_impl_rejeita_caminho_nao_gravavel() {
        let (app, _tmp) = build_app().await;
        let state = app.state::<AppState<MockRuntime>>();

        // Aponta para dentro de um arquivo comum — `create_dir_all` falha
        // porque um componente do caminho já existe e não é diretório.
        let blocker = tempfile::NamedTempFile::new().unwrap();
        let impossible = blocker.path().join("subpasta");
        let err = connect_local_folder_impl(&state, impossible.display().to_string())
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Other(_)));
    }
}
