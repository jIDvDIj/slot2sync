//! Constantes globais do Slot2Sync — nomes de pastas do Drive, chaves do
//! keyring e parâmetros de runtime. Nenhum magic string fora daqui.

#![allow(dead_code)]

/// Pasta raiz criada no Google Drive do usuário.
pub const DRIVE_ROOT_FOLDER: &str = "Slot2Sync";

/// Subpastas criadas dentro de `Slot2Sync/<Emulador>/`.
pub const DRIVE_SAVES_FOLDER: &str = "saves";
pub const DRIVE_STATES_FOLDER: &str = "savestates";
pub const DRIVE_CONFIG_FOLDER: &str = "config";

/// Snapshot do manifest publicado no Drive a cada sync (a fonte de verdade
/// operacional é a tabela SQLite local).
pub const DRIVE_MANIFEST_FILE: &str = "sync_manifest.json";

/// Chave de `appProperties` (privada ao app) que marca, em cada arquivo do
/// Drive, o NOME amigável do dispositivo que publicou aquela versão — usada na
/// UI de conflito para mostrar a origem da versão remota.
pub const DRIVE_APP_PROP_DEVICE: &str = "device";

/// Chave de `appProperties` com o ID estável (UUID do keyring) do dispositivo
/// que publicou a versão. Diferente do nome, não muda quando o usuário renomeia
/// o dispositivo — é o que a detecção de conflito entre dispositivos compara.
pub const DRIVE_APP_PROP_DEVICE_ID: &str = "deviceId";

/// Arquivo SQLite local (criado no diretório de dados do app).
pub const LOCAL_DB_FILE: &str = "slot2sync.db";

/// Pasta de backups locais (criada no diretório de dados do app). Recebe o
/// arquivo local antes de ser sobrescrito no primeiro sync.
pub const LOCAL_BACKUP_DIR: &str = "backups";

/// Identificação das credenciais no keychain do SO.
pub const KEYRING_SERVICE: &str = "com.slot2sync.app";
/// Chave do refresh token por provedor OAuth (a pasta local não usa keyring).
pub const KEYRING_GOOGLE_REFRESH_TOKEN_KEY: &str = "google_drive_refresh_token";
pub const KEYRING_DROPBOX_REFRESH_TOKEN_KEY: &str = "dropbox_refresh_token";
pub const KEYRING_ONEDRIVE_REFRESH_TOKEN_KEY: &str = "onedrive_refresh_token";

/// Chave do keyring para o identificador estável deste dispositivo (UUID v4).
/// Vive fora do SQLite de propósito: sobrevive à desinstalação do app e à
/// limpeza do banco, ao contrário do nome amigável (`SETTING_DEVICE_NAME`).
/// Prefixada com `slot2sync_` para não colidir com entradas de outros apps.
pub const KEYRING_DEVICE_ID_KEY: &str = "slot2sync_device_id";

/// Intervalo de polling do process watcher.
pub const WATCHER_POLL_INTERVAL_SECS: u64 = 2;

/// Ticks consecutivos sem o processo antes de declarar o emulador encerrado.
/// Debounce contra flapping; a abertura é detectada sem atraso. Com 2 ticks
/// de 2s, são ~4s de ausência confirmada antes do sync Local → Drive.
pub const WATCHER_STOP_DEBOUNCE_TICKS: u32 = 2;

/// Espera após confirmar o encerramento do emulador antes de disparar o sync
/// Local → Drive. Dá tempo ao SO de terminar o flush dos buffers de escrita —
/// sem isso o scan pode capturar um save parcialmente gravado.
pub const EMULATOR_STOP_SETTLE_MS: u64 = 3_000;

/// Máximo de tentativas (com backoff exponencial) por chamada à API do Drive.
pub const DRIVE_MAX_RETRIES: u32 = 3;

/// Máximo de transferências simultâneas com o Drive. Elevado de 3 → 6 para
/// encurtar o tempo total em coleções com muitos arquivos pequenos; o
/// `send_with_retry` absorve eventuais 429/rateLimit com backoff.
pub const DRIVE_MAX_CONCURRENT_TRANSFERS: usize = 6;

/// Teto de bytes em trânsito simultaneamente numa categoria, além do limite
/// de contagem acima — um savestate de 500 MB não deve ocupar a mesma "vaga"
/// que um save de 1 KB e deixar memória/banda livres para os demais.
pub const MAX_BYTES_IN_FLIGHT: u32 = 64 * 1024 * 1024;

/// Máximo de entradas no histórico de erros em memória
/// (`SyncEngine::recent_errors`) — as mais antigas caem conforme novas
/// chegam.
pub const MAX_RECENT_ERRORS: usize = 100;

/// Prazo máximo, no menu "Sair", para as tasks de longa duração drenarem
/// depois do cancelamento. Estourou o prazo, o app encerra assim mesmo — não
/// vale prender o usuário numa saída que não termina.
pub const SHUTDOWN_GRACE_SECS: u64 = 10;

/// Chamadas de rede (upload/download) simultâneas com o provedor remoto —
/// separado do limite de I/O de disco (`MAX_DISK_WRITES`): são recursos
/// diferentes, um não deveria esperar o outro.
pub const MAX_NETWORK_OPS: usize = 4;
/// Leituras/escritas de disco local simultâneas. Menor que `MAX_NETWORK_OPS`
/// de propósito — em HDD, I/O paralelo demais vira thrashing de cabeça de
/// leitura/escrita; sequencial (ou quase) é mais rápido.
pub const MAX_DISK_WRITES: usize = 2;

/// Acima deste tamanho o upload usa sessão resumable; abaixo, multipart — e o
/// arquivo é elegível ao batch (a Batch API não suporta resumable).
pub const DRIVE_SIMPLE_UPLOAD_MAX_BYTES: usize = 5 * 1024 * 1024;

/// Máximo de operações agrupadas num único request de batch (limite do Google).
pub const DRIVE_BATCH_MAX_OPS: usize = 100;

/// Mínimo de uploads novos elegíveis para valer a pena montar um batch. Abaixo
/// disso, o caminho per-file concorrente já resolve sem o overhead do batch —
/// o ganho do batch aparece no primeiro sync de coleções grandes.
pub const DRIVE_BATCH_MIN_OPS: usize = 12;

/// Prefixo de arquivo temporário de gravação atômica (temp + rename) no
/// Windows: convenção comum de apps que fazem escrita segura ali (Office,
/// editores), reconhecível como "arquivo temporário de alguma coisa" mesmo
/// fora do Slot2Sync.
pub const TMP_PREFIX_WINDOWS: &str = "~slot2sync~";
/// Prefixo equivalente em Unix (Linux/macOS): ponto inicial segue a convenção
/// local de arquivo oculto.
pub const TMP_PREFIX_UNIX: &str = ".slot2sync.";

/// Arquivo-marcador gravado na raiz de um emulador ao ser adicionado
/// (`add_emulator`). Não é lido/checado hoje — `scan_local_bases` detecta
/// desconexão pela ausência da própria pasta raiz (`AppError::FolderNotMounted`),
/// que já cobre o caso comum (drive removível desaparece por completo). Um
/// marcador por si só não distingue de forma confiável "nunca foi montado
/// nesta instalação" de "estava montado e caiu, revelando um ponto de
/// montagem local vazio" sem estado adicional além do filesystem — fica
/// gravado como metadado para uma heurística futura mais completa.
pub const LOCAL_ROOT_MARKER: &str = ".slot2sync-root";

/// Identificação dos gatilhos de sync (logs e evento `sync:started`).
pub const TRIGGER_STARTUP: &str = "startup";
pub const TRIGGER_SHUTDOWN: &str = "shutdown";
pub const TRIGGER_MANUAL: &str = "manual";
pub const TRIGGER_EMULATOR_START: &str = "emulator-start";
pub const TRIGGER_EMULATOR_STOP: &str = "emulator-stop";
/// Gatilhos exclusivos do mobile (substituem watcher e startup/shutdown).
pub const TRIGGER_FOREGROUND: &str = "foreground";
pub const TRIGGER_BACKGROUND: &str = "background";
/// Scan periódico em background (timer com jitter; só-desktop).
pub const TRIGGER_SCHEDULED: &str = "scheduled";
/// Mudança de arquivo detectada pelo watcher de filesystem (só-desktop).
pub const TRIGGER_FILE_CHANGE: &str = "file-change";

/// Debounce do watcher de filesystem: o sync só dispara após este tempo sem
/// novos eventos nas pastas do emulador (agrupa rajadas de escrita).
pub const FS_WATCHER_DEBOUNCE_SECS: u64 = 8;
/// Intervalo de reconciliação das pastas observadas com a lista de emuladores.
pub const FS_WATCHER_RECONCILE_SECS: u64 = 60;
/// Janela em que um arquivo recém-baixado pelo próprio sync é ignorado pelo
/// watcher de filesystem (anti-loop: sync → grava → evento → sync…).
pub const RECENT_DOWNLOAD_TTL_SECS: u64 = 30;

/// Chaves da tabela `app_settings` (configurações globais do usuário).
/// Nome amigável deste dispositivo (ex.: "PC Gamer"), definido no login.
pub const SETTING_DEVICE_NAME: &str = "device_name";

/// Gatilhos de sync automático ligáveis/desligáveis (default: todos ligados).
pub const SETTING_TRIGGER_STARTUP: &str = "trigger_startup";
pub const SETTING_TRIGGER_EMULATOR_START: &str = "trigger_emulator_start";
pub const SETTING_TRIGGER_EMULATOR_STOP: &str = "trigger_emulator_stop";

/// Nível de notificações nativas: all | errors_only | none (default: all).
pub const SETTING_NOTIFICATION_LEVEL: &str = "notification_level";

/// Dias de retenção dos backups locais (0 = manter para sempre).
pub const SETTING_BACKUP_RETENTION_DAYS: &str = "backup_retention_days";
/// Default de fábrica da retenção de backups.
pub const BACKUP_RETENTION_DAYS_DEFAULT: u32 = 30;

/// Intervalo do scan periódico em minutos (0 = desativado).
pub const SETTING_SCAN_INTERVAL_MINUTES: &str = "scan_interval_minutes";
/// Default de fábrica do scan periódico.
pub const SCAN_INTERVAL_MINUTES_DEFAULT: u32 = 60;

/// Máximo de versões arquivadas por arquivo no histórico (`history/`).
pub const SETTING_MAX_BACKUP_VERSIONS: &str = "max_backup_versions";
/// Default de fábrica do máximo de versões por arquivo.
pub const MAX_BACKUP_VERSIONS_DEFAULT: u32 = 5;

/// Limites de banda das transferências com o Drive, em KB/s (0 = ilimitado).
pub const SETTING_UPLOAD_KBPS: &str = "upload_kbps";
pub const SETTING_DOWNLOAD_KBPS: &str = "download_kbps";

/// Subpasta (por emulador) das cópias padronizadas de conflito.
pub const CONFLICT_COPIES_DIR: &str = "conflicts";
/// Máximo de cópias de conflito mantidas por arquivo (as mais antigas caem).
pub const MAX_CONFLICT_COPIES: usize = 3;

/// IDs de banners informativos que o usuário dispensou (array JSON). Um banner
/// dispensado não reaparece
pub const SETTING_DISMISSED_NOTICES: &str = "dismissed_notices";

/// Provedor de storage remoto ativo (`ProviderKind::as_str()`). Ausente =
/// nenhum escolhido ainda (primeiro uso) — a UI mostra o seletor de provedor.
pub const SETTING_STORAGE_PROVIDER: &str = "storage_provider";
/// Caminho absoluto da pasta local/de rede, quando o provedor é `LocalFolder`.
pub const SETTING_FOLDER_PROVIDER_PATH: &str = "folder_provider_path";

/// Marca que o default de fábrica do autostart (ligado) já foi aplicado na
/// primeira execução. Impede religar o autostart a cada inicialização — depois
/// disso a escolha do usuário prevalece, inclusive se ele desativar.
pub const SETTING_AUTOSTART_INITIALIZED: &str = "autostart_initialized";

/// Versionamento lógico do *formato dos dados* guardados em `app_settings` e
/// `sync_manifest` (chaves, encoding de valores) — distinto do `PRAGMA
/// user_version` em `storage::db`, que versiona o schema físico (tabelas/
/// colunas). Sobe quando uma migração muda como os dados são interpretados,
/// não quando uma coluna nasce. Ver `storage::schema_version`.
pub const SCHEMA_COMPONENT_SETTINGS: &str = "settings";
pub const SCHEMA_COMPONENT_MANIFEST: &str = "sync_manifest";
pub const SETTINGS_SCHEMA_VERSION: i64 = 1;
pub const MANIFEST_SCHEMA_VERSION: i64 = 1;

/// Label da janela principal (definida pelo Tauri quando não há `label`).
pub const MAIN_WINDOW_LABEL: &str = "main";

/// Argumento que o lançador do SO injeta quando o app sobe junto com o sistema
/// (registrado pelo plugin de autostart). Com ele o app inicia direto na
/// bandeja, sem abrir a janela principal.
pub const STARTUP_MINIMIZED_FLAG: &str = "--minimized";

/// IDs dos itens do menu da bandeja do sistema.
pub const TRAY_MENU_OPEN: &str = "tray-open";
pub const TRAY_MENU_SYNC: &str = "tray-sync";
pub const TRAY_MENU_QUIT: &str = "tray-quit";
