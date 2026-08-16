/**
 * Wrappers tipados sobre `invoke()`. O restante do frontend nunca chama
 * `invoke` diretamente — só funções deste módulo.
 */

import { invoke } from "@tauri-apps/api/core";

import type {
  AuthStatus,
  BackupEntry,
  Conflict,
  ConflictResolution,
  DiscoveredEmulator,
  EmulatorProfile,
  EmulatorStats,
  ErrorEntry,
  FileVersion,
  HealthStatus,
  LastSync,
  NotificationLevel,
  PendingOp,
  Settings,
  SyncCategories,
  SyncedGame,
  SyncStateSnapshot,
  SyncSummary,
  TriggerSettings,
} from "../types/ipc";

export function healthCheck(): Promise<HealthStatus> {
  return invoke<HealthStatus>("health_check");
}

/** Abre o navegador para o consentimento OAuth2; resolve ao fim do fluxo. */
export function connectGoogleDrive(): Promise<AuthStatus> {
  return invoke<AuthStatus>("connect_google_drive");
}

/** Abre o navegador para o consentimento OAuth2 do Dropbox. */
export function connectDropbox(): Promise<AuthStatus> {
  return invoke<AuthStatus>("connect_dropbox");
}

/** Abre o navegador para o consentimento OAuth2 do OneDrive/Microsoft. */
export function connectOneDrive(): Promise<AuthStatus> {
  return invoke<AuthStatus>("connect_onedrive");
}

/**
 * Conecta a uma pasta local ou de rede como provedor de storage — sem OAuth.
 * Cria a pasta se ainda não existir; rejeita se não for gravável.
 */
export function connectLocalFolder(path: string): Promise<AuthStatus> {
  return invoke<AuthStatus>("connect_local_folder", { path });
}

/** Consulta o status do provedor ativo sem disparar fluxo interativo. */
export function getAuthStatus(): Promise<AuthStatus> {
  return invoke<AuthStatus>("get_auth_status");
}

/**
 * Desconecta do provedor ativo (qualquer que seja) e limpa a config
 * persistida — a UI volta a mostrar o seletor de provedor, sem reiniciar.
 */
export function disconnectProvider(): Promise<AuthStatus> {
  return invoke<AuthStatus>("disconnect_provider");
}

/** `null` = pasta válida, mas nenhum emulador suportado reconhecido nela. */
export function detectEmulator(path: string): Promise<EmulatorProfile | null> {
  return invoke<EmulatorProfile | null>("detect_emulator", { path });
}

/** Detecta e registra o emulador da pasta para sincronização. */
export function addEmulator(path: string): Promise<EmulatorProfile> {
  return invoke<EmulatorProfile>("add_emulator", { path });
}

/**
 * Registra um emulador com pastas informadas manualmente (fallback quando a
 * detecção falha). Caminhos relativos à raiz. Rejeita com `emulator_exists` se
 * já houver um emulador com o mesmo nome.
 */
export function addEmulatorManual(
  name: string,
  path: string,
  savesPaths: string[],
  statePaths: string[],
  configPaths: string[],
): Promise<EmulatorProfile> {
  return invoke<EmulatorProfile>("add_emulator_manual", {
    name,
    path,
    savesPaths,
    statePaths,
    configPaths,
  });
}

export function listEmulators(): Promise<EmulatorProfile[]> {
  return invoke<EmulatorProfile[]>("list_emulators");
}

/** Emuladores do catálogo detectados instalados no sistema. Não persiste nada. */
export function discoverEmulators(): Promise<DiscoveredEmulator[]> {
  return invoke<DiscoveredEmulator[]>("discover_emulators");
}

/** Jogos sincronizados (agregados do manifest), com nome legível quando conhecido. */
export function listSyncedGames(): Promise<SyncedGame[]> {
  return invoke<SyncedGame[]>("list_synced_games");
}

/** Estatísticas acumuladas de um emulador; `null` = nunca houve atividade. */
export function getEmulatorStats(name: string): Promise<EmulatorStats | null> {
  return invoke<EmulatorStats | null>("get_emulator_stats", { name });
}

/** Estatísticas acumuladas de todos os emuladores com atividade. */
export function listEmulatorStats(): Promise<EmulatorStats[]> {
  return invoke<EmulatorStats[]>("list_emulator_stats");
}

/** Remove da sincronização; nada é apagado no Drive nem no disco. */
export function removeEmulator(name: string): Promise<void> {
  return invoke<void>("remove_emulator", { name });
}

/** Sync manual bidirecional; resolve com o resumo ao terminar. */
export function syncNow(): Promise<SyncSummary> {
  return invoke<SyncSummary>("sync_now");
}

/** Último sync concluído nesta execução; `null` se ainda não houve nenhum. */
export function getLastSync(): Promise<LastSync | null> {
  return invoke<LastSync | null>("get_last_sync");
}

/** Estado corrente do sync (idle/scanning/syncing/conflict/error) — usado para
 * renderizar o estado certo ao montar a UI, sem depender de eventos perdidos
 * antes da conexão (ex.: reconectar no meio de um sync). */
export function getSyncState(): Promise<SyncStateSnapshot> {
  return invoke<SyncStateSnapshot>("get_sync_state");
}

/** Histórico de erros em memória desde o último reinício (mais antigo primeiro). */
export function getRecentErrors(): Promise<ErrorEntry[]> {
  return invoke<ErrorEntry[]>("get_recent_errors");
}

/** Limpa o histórico de erros em memória. */
export function clearErrors(): Promise<void> {
  return invoke<void>("clear_errors");
}

/** Gera o .zip de diagnóstico na pasta de Downloads; resolve com o caminho gerado. */
export function exportDiagnostics(): Promise<string> {
  return invoke<string>("export_diagnostics");
}

/** Configurações globais do usuário (nome do dispositivo, etc.). */
export function getSettings(): Promise<Settings> {
  return invoke<Settings>("get_settings");
}

/** Define o nome deste dispositivo (obrigatório no login). */
export function setDeviceName(name: string): Promise<void> {
  return invoke<void>("set_device_name", { name });
}

/** Liga/desliga os gatilhos de sync automático (sync manual não é afetado). */
export function setTriggers(triggers: TriggerSettings): Promise<void> {
  return invoke<void>("set_triggers", { triggers });
}

/** Define o nível de notificações nativas (all | errors_only | none). */
export function setNotificationLevel(level: NotificationLevel): Promise<void> {
  return invoke<void>("set_notification_level", { level });
}

/** Retenção dos backups locais em dias (0 = manter para sempre). */
export function setBackupRetentionDays(days: number): Promise<void> {
  return invoke<void>("set_backup_retention_days", { days });
}

/** Intervalo do scan periódico em minutos (0 = desativado). */
export function setScanIntervalMinutes(minutes: number): Promise<void> {
  return invoke<void>("set_scan_interval_minutes", { minutes });
}

/** Máximo de versões arquivadas por arquivo no histórico pré-download. */
export function setMaxBackupVersions(versions: number): Promise<void> {
  return invoke<void>("set_max_backup_versions", { versions });
}

/** Limites de banda em KB/s (0 = ilimitado). Aplicados imediatamente. */
export function setBandwidthLimits(uploadKbps: number, downloadKbps: number): Promise<void> {
  return invoke<void>("set_bandwidth_limits", { uploadKbps, downloadKbps });
}

/** Liga/desliga o início automático do Slot2Sync junto com o sistema. */
export function setAutostart(enabled: boolean): Promise<void> {
  return invoke<void>("set_autostart", { enabled });
}

/** Abre a pasta de backups locais no gerenciador de arquivos do SO. */
export function openBackupFolder(): Promise<void> {
  return invoke<void>("open_backup_folder");
}

/** Mostra um arquivo de backup no gerenciador de arquivos (abre a pasta dele). */
export function revealBackupPath(path: string): Promise<void> {
  return invoke<void>("reveal_backup_path", { path });
}

/**
 * Abre o seletor de pasta nativo do SO (SAF no Android) e retorna a URI da
 * árvore concedida. No desktop lança erro — use o seletor de ficheiros nativo.
 */
export function pickEmulatorFolder(): Promise<string> {
  return invoke<string>("pick_emulator_folder");
}

/**
 * Tenta reconhecer automaticamente o emulador na árvore SAF `tree` (retornada
 * por {@link pickEmulatorFolder}), testando o mesmo catálogo do desktop via
 * chamadas ao plugin nativo. `null` quando nenhum emulador é reconhecido —
 * cai no formulário manual.
 */
export function detectEmulatorMobile(tree: string): Promise<EmulatorProfile | null> {
  return invoke<EmulatorProfile | null>("detect_emulator_mobile", { tree });
}

/** Conflitos pendentes (ambos os lados mudaram desde o último sync). */
export function listConflicts(): Promise<Conflict[]> {
  return invoke<Conflict[]>("list_conflicts");
}

/** Resolve um conflito mantendo a versão `local` ou `remote`. */
export function resolveConflict(
  emulator: string,
  category: Conflict["category"],
  relPath: string,
  keep: ConflictResolution,
): Promise<void> {
  return invoke<void>("resolve_conflict", { emulator, category, relPath, keep });
}

/** Fila offline: arquivos que falharam (rede/arquivo em uso) e serão retentados. */
export function listPendingOps(): Promise<PendingOp[]> {
  return invoke<PendingOp[]>("list_pending_ops");
}

/** Zera tentativas/backoff de uma pendência (inclusive mortas) para retentar já. */
export function retryPendingOp(
  emulator: string,
  category: PendingOp["category"],
  relPath: string,
): Promise<void> {
  return invoke<void>("retry_pending_op", { emulator, category, relPath });
}

/** IDs de banners informativos já dispensados pelo usuário. */
export function listDismissedNotices(): Promise<string[]> {
  return invoke<string[]>("list_dismissed_notices");
}

/** Dispensa um banner de forma persistente — ele não reaparece. */
export function dismissNotice(id: string): Promise<void> {
  return invoke<void>("dismiss_notice", { id });
}

/** Histórico de backups locais (primeiro sync e resoluções de conflito). */
export function listBackups(): Promise<BackupEntry[]> {
  return invoke<BackupEntry[]>("list_backups");
}

/** Versões arquivadas de um arquivo no histórico pré-download, recentes primeiro. */
export function listFileVersions(
  emulator: string,
  category: PendingOp["category"],
  relPath: string,
): Promise<FileVersion[]> {
  return invoke<FileVersion[]>("list_file_versions", { emulator, category, relPath });
}

/**
 * Restaura uma versão arquivada por cima do arquivo atual do emulador.
 * O estado atual é arquivado antes; o restaurado sobe no próximo sync.
 * `versionedRelPath` é o caminho listado no histórico (nome com carimbo).
 */
export function restoreVersion(
  emulator: string,
  category: PendingOp["category"],
  versionedRelPath: string,
): Promise<void> {
  return invoke<void>("restore_version", { emulator, category, versionedRelPath });
}

/** Categorias de sync habilitadas para um emulador (default: todas ativas). */
export function getEmulatorCategories(name: string): Promise<SyncCategories> {
  return invoke<SyncCategories>("get_emulator_categories", { name });
}

/** Define quais categorias sincronizar para um emulador. */
export function setEmulatorCategories(name: string, categories: SyncCategories): Promise<void> {
  return invoke<void>("set_emulator_categories", { name, categories });
}

/**
 * Define os padrões glob de exclusão de um emulador (arquivos que casam ficam
 * fora do sync nas duas direções). Rejeita padrões glob inválidos.
 */
export function setExcludePatterns(name: string, patterns: string[]): Promise<void> {
  return invoke<void>("set_exclude_patterns", { name, patterns });
}
