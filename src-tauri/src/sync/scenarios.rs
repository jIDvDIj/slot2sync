//! Cenários de integração do `SyncEngine`: engine real de ponta a
//! ponta — SQLite em memória, filesystem em `tempdir`, `AppHandle` do
//! `MockRuntime` — com o Drive substituído pelo [`MockDrive`] em memória.
//! Sem rede e sem credenciais; testes com credenciais reais ficam atrás da
//! feature `integration-tests` (hoje vazia).

use std::path::PathBuf;
use std::sync::Arc;

use tauri::test::MockRuntime;

use super::engine::ConflictResolution;
use super::{
    DesktopStorage, LastSyncStore, SyncCategory, SyncDirection, SyncEngine, SyncState, SyncSummary,
};
use crate::constants::{DRIVE_BATCH_MIN_OPS, DRIVE_MANIFEST_FILE};
use crate::drive::mock::MockDrive;
use crate::emulator::EmulatorProfile;
use crate::remote::RemoteProvider;
use crate::secrets::{MemSecrets, SecretStore};
use crate::storage::db::Db;
use crate::storage::settings::NotificationLevel;
use crate::storage::{conflicts, emulators, manifest, settings};
use crate::sync::FileLoc;

const EMU: &str = "PPSSPP";
const T: i64 = 1_700_000_000_000;
const S10: i64 = 10_000; // 10s — bem além da tolerância de ±2s do diff.

/// Fixture completa: engine pronto para sincronizar um emulador com uma
/// categoria de saves, contra um Drive falso.
struct Harness {
    _tmp: tempfile::TempDir,
    _app: tauri::App<MockRuntime>,
    db: Db,
    drive: Arc<MockDrive>,
    engine: SyncEngine<MockRuntime>,
    saves_dir: PathBuf,
    backups_dir: PathBuf,
    device_id: String,
}

impl Harness {
    async fn new() -> Self {
        Self::with_storage(Arc::new(DesktopStorage)).await
    }

    /// Fixture com um [`LocalStorage`] escolhido pelo teste — usado pelo
    /// cenário de FAT32, que precisa de um filesystem que arredonde o mtime.
    async fn with_storage(storage: Arc<dyn crate::sync::LocalStorage>) -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("emulador");
        let saves_dir = root.join("saves");
        std::fs::create_dir_all(&saves_dir).unwrap();
        let backups_dir = tmp.path().join("backups");

        let db = Db::open_in_memory().unwrap();
        let profile = EmulatorProfile {
            name: EMU.to_string(),
            root_path: root,
            saves_paths: vec![PathBuf::from("saves")],
            config_paths: vec![],
            state_paths: vec![],
            exclude_patterns: vec!["*.tmp".into()],
        };
        db.with(move |conn| emulators::upsert(conn, &profile))
            .await
            .unwrap();
        // Sem notificações nativas: o MockRuntime não registra o plugin.
        db.with(|conn| settings::set_notification_level(conn, NotificationLevel::None))
            .await
            .unwrap();

        let secrets: Arc<dyn SecretStore> = Arc::new(MemSecrets::default());
        let device_id = crate::device::get_or_create(&*secrets).unwrap();

        let drive = Arc::new(MockDrive::new());

        let app = tauri::test::mock_builder()
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();

        let remote_provider = drive.clone() as Arc<dyn RemoteProvider>;
        let engine = SyncEngine::new(
            db.clone(),
            Some(remote_provider),
            app.handle().clone(),
            LastSyncStore::default(),
            backups_dir.clone(),
            storage,
            secrets,
        );

        Self {
            _tmp: tmp,
            _app: app,
            db,
            drive,
            engine,
            saves_dir,
            backups_dir,
            device_id,
        }
    }

    /// Grava um save local com mtime controlado.
    fn write_local(&self, rel_path: &str, content: &[u8], mtime_ms: i64) {
        let path = self.saves_dir.join(rel_path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, content).unwrap();
        let ft = filetime::FileTime::from_unix_time(
            mtime_ms / 1000,
            ((mtime_ms % 1000) * 1_000_000) as u32,
        );
        filetime::set_file_mtime(&path, ft).unwrap();
    }

    fn read_local(&self, rel_path: &str) -> Vec<u8> {
        std::fs::read(self.saves_dir.join(rel_path)).unwrap()
    }

    fn seed_remote(&self, rel_path: &str, content: &[u8], mtime_ms: i64, device_id: Option<&str>) {
        self.drive.seed_category_file(
            EMU,
            SyncCategory::Saves,
            rel_path,
            content,
            mtime_ms,
            device_id,
        );
    }

    fn remote_content(&self, rel_path: &str) -> Option<Vec<u8>> {
        self.drive
            .file_by_path(EMU, SyncCategory::Saves, rel_path)
            .map(|f| f.content)
    }

    async fn sync(&self) -> SyncSummary {
        self.sync_dir(SyncDirection::Bidirectional).await
    }

    async fn sync_dir(&self, direction: SyncDirection) -> SyncSummary {
        self.engine.sync_all(direction, "teste").await.unwrap()
    }

    async fn pending_ops(&self) -> i64 {
        self.db.with(crate::storage::queue::count).await.unwrap()
    }

    async fn manifest_len(&self) -> usize {
        self.db.with(manifest::list_all).await.unwrap().len()
    }

    async fn has_conflict(&self) -> bool {
        self.db
            .with(|conn| conflicts::has_for_emulator(conn, EMU))
            .await
            .unwrap()
    }

    /// Procura `name` recursivamente na pasta de backups.
    fn backup_of(&self, name: &str) -> Option<Vec<u8>> {
        fn walk(dir: &std::path::Path, name: &str) -> Option<PathBuf> {
            for entry in std::fs::read_dir(dir).ok()? {
                let path = entry.ok()?.path();
                if path.is_dir() {
                    if let Some(found) = walk(&path, name) {
                        return Some(found);
                    }
                } else if path.file_name().is_some_and(|n| n == name) {
                    return Some(path);
                }
            }
            None
        }
        walk(&self.backups_dir, name).map(|p| std::fs::read(p).unwrap())
    }
}

/// Primeiro sync com arquivos dos dois lados: o só-local sobe, o só-remoto
/// desce e o que existe nos dois lados com mtimes divergentes é baixado COM
/// backup do local (Drive vence).
#[tokio::test]
async fn primeiro_sync_mescla_local_e_drive_com_backup() {
    let h = Harness::new().await;
    h.write_local("a.bin", b"local-a", T);
    h.seed_remote("b.bin", b"drive-b", T, None);
    h.write_local("c.bin", b"local-c", T);
    h.seed_remote("c.bin", b"drive-c", T + S10, None);

    let summary = h.sync().await;

    assert_eq!(summary.uploaded, 1, "a.bin sobe");
    assert_eq!(summary.downloaded, 2, "b.bin e c.bin descem");
    assert_eq!(summary.backed_up, 1, "c.bin gera backup antes de descer");
    assert_eq!(summary.conflicts, 0);
    assert_eq!(summary.failed, 0);

    assert_eq!(h.remote_content("a.bin").unwrap(), b"local-a");
    assert_eq!(h.read_local("b.bin"), b"drive-b");
    assert_eq!(
        h.read_local("c.bin"),
        b"drive-c",
        "Drive vence no primeiro sync"
    );
    assert_eq!(h.backup_of("c.bin").unwrap(), b"local-c");

    assert_eq!(h.manifest_len().await, 3);
    // Snapshot de auditoria publicado na raiz Slot2Sync/.
    assert!(h.drive.root_file(DRIVE_MANIFEST_FILE).is_some());
}

/// Primeiro sync de um arquivo divergente publicado por OUTRO dispositivo:
/// ninguém vence sozinho — vira conflito.
#[tokio::test]
async fn primeiro_sync_divergente_de_outro_dispositivo_vira_conflito() {
    let h = Harness::new().await;
    h.write_local("save.bin", b"deste-pc", T);
    h.seed_remote(
        "save.bin",
        b"do-outro-pc",
        T + S10,
        Some("outro-dispositivo"),
    );
    assert_ne!(h.device_id, "outro-dispositivo");

    let summary = h.sync().await;

    assert_eq!(summary.conflicts, 1);
    assert_eq!(summary.downloaded, 0, "nada é sobrescrito num conflito");
    assert_eq!(summary.uploaded, 0);
    assert!(h.has_conflict().await);
    assert_eq!(h.read_local("save.bin"), b"deste-pc");
    assert_eq!(h.remote_content("save.bin").unwrap(), b"do-outro-pc");
}

/// Mudança dos dois lados desde o último sync: conflito, emulador bloqueado até
/// a resolução; resolver mantendo o local envia a versão local e desbloqueia.
#[tokio::test]
async fn conflito_bloqueia_emulador_e_resolucao_desbloqueia() {
    let h = Harness::new().await;
    h.write_local("save.bin", b"v1", T);
    let first = h.sync().await;
    assert_eq!(first.uploaded, 1);

    // Cada lado muda de forma independente após o sync.
    h.write_local("save.bin", b"v2-local", T + S10);
    h.drive.overwrite_as_device(
        EMU,
        SyncCategory::Saves,
        "save.bin",
        b"v2-drive",
        T + 2 * S10,
        "outro-dispositivo",
    );

    let second = h.sync().await;
    assert_eq!(second.conflicts, 1);
    assert!(h.has_conflict().await);

    // Bloqueado: nova mudança local não sincroniza enquanto o conflito viver.
    h.write_local("save.bin", b"v3-local", T + 3 * S10);
    let blocked = h.sync().await;
    assert_eq!(blocked.uploaded + blocked.downloaded + blocked.conflicts, 0);
    assert_eq!(h.remote_content("save.bin").unwrap(), b"v2-drive");

    // Resolução mantendo o local: sobe a versão local vigente e desbloqueia.
    h.engine
        .resolve_conflict(
            EMU,
            SyncCategory::Saves,
            "save.bin",
            ConflictResolution::Local,
        )
        .await
        .unwrap();
    assert!(!h.has_conflict().await);
    assert_eq!(h.remote_content("save.bin").unwrap(), b"v3-local");

    // Desbloqueado e convergido: próximo sync não move nada.
    let after = h.sync().await;
    assert_eq!(after.uploaded + after.downloaded + after.conflicts, 0);
}

/// Mesmo mtime com conteúdo diferente passa despercebido: o diff é por
/// timestamp — detectar isso exige hash, ainda não implementado.
/// Este teste DOCUMENTA a limitação atual; quando o hash entrar, ele deve
/// passar a falhar e ser invertido.
#[tokio::test]
async fn progresso_emite_retrato_final_com_completed_igual_a_total() {
    use std::sync::{Arc, Mutex};
    use tauri::Listener;

    let h = Harness::new().await;
    h.write_local("a.bin", b"1", T);
    h.write_local("b.bin", b"2", T);
    h.write_local("c.bin", b"3", T);

    let last_progress: Arc<Mutex<Option<serde_json::Value>>> = Arc::new(Mutex::new(None));
    let captured = last_progress.clone();
    h._app
        .handle()
        .listen(crate::events::EVT_SYNC_PROGRESS, move |event| {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(event.payload()) {
                *captured.lock().unwrap() = Some(v);
            }
        });

    h.sync().await;

    let last = last_progress
        .lock()
        .unwrap()
        .clone()
        .expect("deveria ter emitido ao menos um sync:progress (o retrato final garantido)");
    assert_eq!(last["completed"], last["total"]);
    assert_eq!(last["total"], 3);
}

#[tokio::test]
async fn erro_de_sync_entra_no_historico_e_clear_errors_esvazia() {
    let h = Harness::new().await;
    assert!(h.engine.recent_errors().is_empty());

    // Raiz do emulador desaparece (drive removível desconectado) — falha
    // dura, específica deste emulador, propagada por `sync_target`.
    let root = h.saves_dir.parent().unwrap();
    std::fs::remove_dir_all(root).unwrap();

    h.sync().await;

    let errors = h.engine.recent_errors();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].emulator.as_deref(), Some(EMU));

    h.engine.clear_errors();
    assert!(h.engine.recent_errors().is_empty());
}

#[tokio::test]
async fn sync_state_comeca_e_termina_ocioso() {
    let h = Harness::new().await;
    assert_eq!(h.engine.current_sync_state(), (SyncState::Idle, None));

    h.write_local("save.bin", b"conteudo", T);
    h.sync().await;

    assert_eq!(
        h.engine.current_sync_state(),
        (SyncState::Idle, None),
        "sync termina sempre voltando a Idle, mesmo com transferência real"
    );
}

#[tokio::test]
async fn sync_state_emite_transicao_para_conflict() {
    use std::sync::{Arc, Mutex};
    use tauri::Listener;

    let h = Harness::new().await;
    h.write_local("save.bin", b"v1", T);
    h.sync().await;

    // Ambos os lados mudam desde a âncora → conflito.
    h.write_local("save.bin", b"v2-local", T + S10);
    h.drive.overwrite_as_device(
        EMU,
        SyncCategory::Saves,
        "save.bin",
        b"v2-drive",
        T + 2 * S10,
        "dev-B",
    );

    let seen_conflict = Arc::new(Mutex::new(false));
    let flag = seen_conflict.clone();
    h._app
        .handle()
        .listen(crate::events::EVT_SYNC_STATE_CHANGED, move |event| {
            if event.payload().contains("\"to\":\"conflict\"") {
                *flag.lock().unwrap() = true;
            }
        });

    let summary = h.sync().await;

    assert_eq!(summary.conflicts, 1);
    assert!(
        *seen_conflict.lock().unwrap(),
        "deveria ter emitido sync:state-changed com to=conflict"
    );
    // O sync sempre volta a Idle ao final da leva, mesmo após um conflito.
    assert_eq!(h.engine.current_sync_state(), (SyncState::Idle, None));
}

#[tokio::test]
async fn mtime_igual_com_conteudo_diferente_passa_despercebido() {
    let h = Harness::new().await;
    h.write_local("save.bin", b"conteudo-A", T);
    h.seed_remote("save.bin", b"conteudo-B", T, None);

    let summary = h.sync().await;

    assert_eq!(summary.uploaded + summary.downloaded + summary.conflicts, 0);
    assert_eq!(
        summary.skipped, 1,
        "mtimes iguais → tratado como sem mudança"
    );
    assert_eq!(
        h.read_local("save.bin"),
        b"conteudo-A",
        "conteúdos seguem divergentes"
    );
    assert_eq!(h.remote_content("save.bin").unwrap(), b"conteudo-B");
}

/// Coleção grande de arquivos novos sobe em UM batch; o caminho
/// per-file fica só para o snapshot do manifest.
#[tokio::test]
async fn batch_upload_agrupa_arquivos_novos() {
    let h = Harness::new().await;
    for i in 0..DRIVE_BATCH_MIN_OPS {
        h.write_local(
            &format!("jogo/save-{i:02}.bin"),
            format!("dado-{i}").as_bytes(),
            T,
        );
    }

    let summary = h.sync().await;

    assert_eq!(summary.uploaded as usize, DRIVE_BATCH_MIN_OPS);
    assert_eq!(summary.failed, 0);
    assert_eq!(
        h.drive
            .batch_calls
            .load(std::sync::atomic::Ordering::SeqCst),
        1
    );
    // Único upload per-file é o sync_manifest.json de auditoria.
    assert_eq!(
        h.drive
            .upload_new_calls
            .load(std::sync::atomic::Ordering::SeqCst),
        1
    );
    assert_eq!(h.manifest_len().await, DRIVE_BATCH_MIN_OPS);
    assert_eq!(h.remote_content("jogo/save-00.bin").unwrap(), b"dado-0");
}

/// Batch falhou (rede/parse): os mesmos arquivos caem para o caminho per-file
/// e o sync ainda conclui todos os uploads.
#[tokio::test]
async fn fallback_per_file_quando_batch_falha() {
    let h = Harness::new().await;
    h.drive.set_fail_next_batch();
    for i in 0..DRIVE_BATCH_MIN_OPS {
        h.write_local(
            &format!("jogo/save-{i:02}.bin"),
            format!("dado-{i}").as_bytes(),
            T,
        );
    }

    let summary = h.sync().await;

    assert_eq!(summary.uploaded as usize, DRIVE_BATCH_MIN_OPS);
    assert_eq!(summary.failed, 0);
    assert_eq!(
        h.drive
            .batch_calls
            .load(std::sync::atomic::Ordering::SeqCst),
        1
    );
    // Todos per-file + o snapshot do manifest.
    assert_eq!(
        h.drive
            .upload_new_calls
            .load(std::sync::atomic::Ordering::SeqCst) as usize,
        DRIVE_BATCH_MIN_OPS + 1
    );
}

/// Resolver conflito mantendo o Drive: faz backup do local vigente antes de
/// sobrescrever, baixa a versão remota e desbloqueia o emulador.
#[tokio::test]
async fn resolucao_mantendo_drive_baixa_com_backup() {
    let h = Harness::new().await;
    h.write_local("save.bin", b"v1", T);
    h.sync().await;

    h.write_local("save.bin", b"v2-local", T + S10);
    h.drive.overwrite_as_device(
        EMU,
        SyncCategory::Saves,
        "save.bin",
        b"v2-drive",
        T + 2 * S10,
        "outro-dispositivo",
    );
    let conflicted = h.sync().await;
    assert_eq!(conflicted.conflicts, 1);

    h.engine
        .resolve_conflict(
            EMU,
            SyncCategory::Saves,
            "save.bin",
            ConflictResolution::Remote,
        )
        .await
        .unwrap();

    assert!(!h.has_conflict().await);
    assert_eq!(
        h.read_local("save.bin"),
        b"v2-drive",
        "Drive escolhido vence"
    );
    assert_eq!(
        h.backup_of("save.bin").unwrap(),
        b"v2-local",
        "local vigente é preservado em backup antes de sobrescrever"
    );

    // Convergido: próximo sync não move nada.
    let after = h.sync().await;
    assert_eq!(after.uploaded + after.downloaded + after.conflicts, 0);
}

/// Falha retryable de download vira pendência na fila offline (`pending_ops`),
/// não erro fatal; o sync seguinte refaz a operação e limpa a pendência.
#[tokio::test]
async fn falha_de_download_vira_pendencia_e_proximo_sync_recupera() {
    let h = Harness::new().await;
    h.seed_remote("save.bin", b"drive-v1", T, None);

    h.drive.set_fail_downloads(true);
    let failed = h.sync().await;
    assert_eq!(failed.queued, 1, "falha retryable vira pendência");
    assert_eq!(failed.downloaded, 0);
    assert_eq!(failed.failed, 0, "não é erro fatal");
    assert_eq!(h.pending_ops().await, 1);

    h.drive.set_fail_downloads(false);

    // Backoff: com a janela de retentativa ainda no futuro, o sync pula o
    // arquivo em vez de retentar imediatamente.
    let deferred = h.sync().await;
    assert_eq!(deferred.downloaded, 0, "backoff adia a retentativa");
    assert_eq!(h.pending_ops().await, 1);

    // Janela vencida (simulada zerando o backoff): o sync seguinte recupera.
    h.db.with(|conn| crate::storage::queue::retry_now(conn, EMU, SyncCategory::Saves, "save.bin"))
        .await
        .unwrap();
    let recovered = h.sync().await;
    assert_eq!(recovered.downloaded, 1);
    assert_eq!(h.read_local("save.bin"), b"drive-v1");
    assert_eq!(h.pending_ops().await, 0, "sucesso limpa a pendência");
}

/// Download comum (arquivo já sincronizado, Drive mudou) arquiva a versão
/// local vigente em `history/` antes de sobrescrever.
#[tokio::test]
async fn download_arquiva_versao_anterior_no_historico() {
    let h = Harness::new().await;
    h.write_local("save.bin", b"v1-local", T);
    h.sync().await; // sobe v1 e ancora o manifest

    // Outro lado publica v2 no Drive; o local ainda tem v1.
    h.drive.overwrite_as_device(
        EMU,
        SyncCategory::Saves,
        "save.bin",
        b"v2-drive",
        T + S10,
        "dev-B",
    );

    let summary = h.sync().await;

    assert_eq!(summary.downloaded, 1);
    assert_eq!(h.read_local("save.bin"), b"v2-drive");

    // A v1 local foi arquivada em <backups>/<emu>/history/... com carimbo.
    let history = h.backups_dir.join(EMU).join("history");
    assert!(history.is_dir(), "pasta history criada");
    let archived = h.backup_of("save.bin");
    assert!(
        archived.is_none(),
        "nome arquivado carrega o carimbo (não é o nome original)"
    );
    fn find_versioned(dir: &std::path::Path) -> Option<Vec<u8>> {
        for entry in std::fs::read_dir(dir).ok()? {
            let path = entry.ok()?.path();
            if path.is_dir() {
                if let Some(found) = find_versioned(&path) {
                    return Some(found);
                }
            } else if path
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("save~"))
            {
                return Some(std::fs::read(path).unwrap());
            }
        }
        None
    }
    assert_eq!(find_versioned(&history).unwrap(), b"v1-local");
}

/// Arquivos que casam com os padrões de exclusão do emulador ficam fora do
/// sync nas duas direções. O Harness configura `*.tmp` no perfil.
#[tokio::test]
async fn padroes_de_exclusao_ignoram_arquivos_nas_duas_direcoes() {
    let h = Harness::new().await;
    h.write_local("save.bin", b"sobe", T);
    h.write_local("lixo.tmp", b"nao-sobe", T);
    h.seed_remote("outro.tmp", b"nao-desce", T, None);

    let summary = h.sync().await;

    assert_eq!(summary.uploaded, 1, "só o save.bin sobe");
    assert_eq!(summary.downloaded, 0, "o .tmp remoto não desce");
    assert!(h.remote_content("lixo.tmp").is_none());
    assert!(!h.saves_dir.join("outro.tmp").exists());
}

/// Renomear um arquivo local vira um rename no Drive: sem novo
/// upload, sem zumbi do nome antigo, manifest reancorado no nome novo.
#[tokio::test]
async fn renomeacao_local_vira_rename_no_drive_sem_retransferir() {
    let h = Harness::new().await;
    h.write_local("antigo.bin", b"conteudo-unico", T);
    h.sync().await; // sobe e ancora
    let uploads_before = h
        .drive
        .upload_new_calls
        .load(std::sync::atomic::Ordering::SeqCst);

    // Usuário renomeia localmente (mesmo conteúdo, nome novo).
    std::fs::rename(h.saves_dir.join("antigo.bin"), h.saves_dir.join("novo.bin")).unwrap();

    let summary = h.sync().await;

    assert_eq!(summary.renamed, 1, "rename detectado por hash");
    assert_eq!(summary.uploaded, 0, "nada retransferido");
    assert_eq!(summary.downloaded, 0, "órfão não é re-baixado");
    assert_eq!(
        h.drive
            .upload_new_calls
            .load(std::sync::atomic::Ordering::SeqCst),
        uploads_before,
        "nenhum upload novo"
    );
    assert!(
        h.remote_content("novo.bin").is_some(),
        "Drive tem o nome novo"
    );
    assert!(
        h.remote_content("antigo.bin").is_none(),
        "sem zumbi do nome antigo"
    );

    // Convergido: sync seguinte não move nada.
    let after = h.sync().await;
    assert_eq!(after.uploaded + after.downloaded + after.renamed, 0);
}

/// Conflito gera a cópia padronizada do lado local em `conflicts/`
/// (`nome.slot2sync-conflict-<carimbo>-<device>.ext`) e grava o caminho no
/// registro do conflito.
#[tokio::test]
async fn conflito_gera_copia_padronizada_do_lado_local() {
    let h = Harness::new().await;
    h.write_local("save.bin", b"v1", T);
    h.sync().await;

    // Ambos os lados mudam desde a âncora → conflito.
    h.write_local("save.bin", b"v2-local", T + S10);
    h.drive.overwrite_as_device(
        EMU,
        SyncCategory::Saves,
        "save.bin",
        b"v2-drive",
        T + 2 * S10,
        "dev-B",
    );

    let summary = h.sync().await;
    assert_eq!(summary.conflicts, 1);

    let conflicts = h.db.with(conflicts::list_all).await.unwrap();
    let backup_path = conflicts[0]
        .backup_path
        .clone()
        .expect("cópia de conflito registrada");
    assert!(backup_path.contains(".slot2sync-conflict-"));
    assert!(backup_path.contains(&h.device_id), "device id no nome");
    assert_eq!(
        std::fs::read(&backup_path).unwrap(),
        b"v2-local",
        "cópia preserva o lado local"
    );
}

/// Renomeação que também muda de subpasta move o arquivo no Drive
/// (addParents/removeParents), sem retransferir.
#[tokio::test]
async fn renomeacao_entre_subpastas_move_no_drive() {
    let h = Harness::new().await;
    h.write_local("GAME01/save.bin", b"conteudo-movido", T);
    h.sync().await;

    std::fs::create_dir_all(h.saves_dir.join("GAME02")).unwrap();
    std::fs::rename(
        h.saves_dir.join("GAME01/save.bin"),
        h.saves_dir.join("GAME02/save.bin"),
    )
    .unwrap();
    std::fs::remove_dir(h.saves_dir.join("GAME01")).unwrap();

    let summary = h.sync().await;

    assert_eq!(summary.renamed, 1);
    assert_eq!(summary.uploaded, 0);
    assert!(h.remote_content("GAME02/save.bin").is_some());
    assert!(h.remote_content("GAME01/save.bin").is_none());
}

/// O rastro de downloads recentes (anti-loop do watcher de filesystem) marca
/// os arquivos gravados pelo próprio sync e expira apenas por TTL.
#[tokio::test]
async fn download_marca_arquivo_no_rastro_anti_loop() {
    let h = Harness::new().await;
    h.seed_remote("save.bin", b"drive-v1", T, None);

    h.sync().await;

    let written = h.saves_dir.join("save.bin");
    assert!(h.engine.is_recent_download(&written));
    assert!(!h.engine.is_recent_download(&h.saves_dir.join("outro.bin")));
}

/// Sync só-upload (`LocalToDrive`, gatilho emulator-stop) não baixa nada;
/// o arquivo remoto pendente fica para o sync bidirecional seguinte.
#[tokio::test]
async fn direcao_local_to_drive_sobe_sem_baixar() {
    let h = Harness::new().await;
    h.write_local("local.bin", b"sobe", T);
    h.seed_remote("remoto.bin", b"nao-desce", T, None);

    let summary = h.sync_dir(SyncDirection::LocalToDrive).await;

    assert_eq!(summary.uploaded, 1);
    assert_eq!(summary.downloaded, 0);
    assert_eq!(summary.skipped, 1, "download reprimido conta como skipped");
    assert!(!h.saves_dir.join("remoto.bin").exists());
}

/// Sync só-download (`DriveToLocal`, gatilho emulator-start) não sobe nada;
/// a mudança local pendente fica para o sync bidirecional seguinte.
#[tokio::test]
async fn direcao_drive_to_local_baixa_sem_subir() {
    let h = Harness::new().await;
    h.write_local("local.bin", b"nao-sobe", T);
    h.seed_remote("remoto.bin", b"desce", T, None);

    let summary = h.sync_dir(SyncDirection::DriveToLocal).await;

    assert_eq!(summary.downloaded, 1);
    assert_eq!(summary.uploaded, 0);
    assert_eq!(summary.skipped, 1, "upload reprimido conta como skipped");
    assert!(
        h.remote_content("local.bin").is_none(),
        "nada foi enviado ao Drive"
    );
    assert_eq!(h.read_local("remoto.bin"), b"desce");
}

/// Sem provedor configurado (`set_remote_provider(None)`), `sync_all` falha
/// de forma limpa com um erro tipado — os gatilhos automáticos (startup/
/// watcher) sabem tratar isso sem crashar, só logam/pulam.
#[tokio::test]
async fn sync_sem_provedor_configurado_falha_limpo() {
    let h = Harness::new().await;
    h.engine.set_remote_provider(None);

    let err = h
        .engine
        .sync_all(SyncDirection::Bidirectional, "teste")
        .await
        .unwrap_err();
    let msg = err.to_string();
    assert!(
        matches!(err, crate::error::AppError::Auth(_)) && msg.contains("nenhum provedor"),
        "erro inesperado: {msg}"
    );
}

/// Trocar o provedor em runtime via `set_remote_provider` é efetivo
/// imediatamente — o sync seguinte já opera contra o novo provedor, sem
/// reiniciar o engine/app.
#[tokio::test]
async fn set_remote_provider_troca_o_provedor_ativo_sem_restart() {
    let h = Harness::new().await;
    h.seed_remote("a.bin", b"do-provedor-antigo", T, None);
    h.sync().await;
    assert_eq!(h.read_local("a.bin"), b"do-provedor-antigo");

    let novo_drive = Arc::new(MockDrive::new());
    novo_drive.seed_category_file(
        EMU,
        SyncCategory::Saves,
        "b.bin",
        b"do-provedor-novo",
        T,
        None,
    );
    h.engine
        .set_remote_provider(Some(novo_drive.clone() as Arc<dyn RemoteProvider>));

    let summary = h.sync().await;
    assert_eq!(summary.downloaded, 1, "baixa b.bin do provedor novo");
    assert_eq!(h.read_local("b.bin"), b"do-provedor-novo");
}

/// Cenário FAT32: o filesystem arredonda o mtime que o download carimbou, e o
/// scan seguinte vê um timestamp que não bate com a âncora do manifest.
///
/// Os dois testes abaixo isolam o efeito com uma entrada de manifest sem hash
/// (como as gravadas antes da migração v7), justamente o caso em que o
/// pré-filtro de hash do diff não entra para segurar o upload inútil.
async fn arredondamento_de_fat32(com_override: bool) -> SyncSummary {
    let h = Harness::new().await;
    h.seed_remote("save.bin", b"conteudo", T, None);
    h.sync().await;

    // O disco "arredondou": mesmo conteúdo, mtime deslocado para fora da
    // tolerância de ±2s do diff.
    let ondisk = T - S10;
    h.write_local("save.bin", b"conteudo", ondisk);
    h.db.with(|conn| {
        conn.execute("UPDATE sync_manifest SET file_hash = NULL", [])
            .map(|_| ())
            .map_err(Into::into)
    })
    .await
    .unwrap();

    if com_override {
        h.db.with(move |conn| {
            crate::storage::mtime_overrides::upsert(
                conn,
                EMU,
                SyncCategory::Saves,
                "save.bin",
                crate::storage::mtime_overrides::MtimeOverride {
                    ondisk_ms: ondisk,
                    virtual_ms: T,
                },
            )
        })
        .await
        .unwrap();
    }

    h.sync().await
}

/// Sem a camada de mtime virtual, o arredondamento do filesystem faz o arquivo
/// subir de novo sem ter mudado — é o desperdício que a tabela existe para
/// evitar.
#[tokio::test]
async fn sem_override_o_arredondamento_causa_upload_inutil() {
    let summary = arredondamento_de_fat32(false).await;
    assert_eq!(summary.uploaded, 1);
}

/// Com o override registrado, o diff enxerga o mtime lógico e conclui
/// corretamente que nada mudou.
#[tokio::test]
async fn override_de_mtime_evita_o_upload_causado_pelo_arredondamento() {
    let summary = arredondamento_de_fat32(true).await;
    assert_eq!(summary.uploaded, 0);
}

/// O override é uma âncora para UM estado do disco: quando o arquivo é
/// realmente editado, o mtime deixa de bater com `ondisk_ms`, o override é
/// descartado e a mudança sobe normalmente.
#[tokio::test]
async fn edicao_real_invalida_o_override_e_sobe() {
    let h = Harness::new().await;
    h.seed_remote("save.bin", b"conteudo", T, None);
    h.sync().await;

    let ondisk = T - S10;
    h.db.with(move |conn| {
        crate::storage::mtime_overrides::upsert(
            conn,
            EMU,
            SyncCategory::Saves,
            "save.bin",
            crate::storage::mtime_overrides::MtimeOverride {
                ondisk_ms: ondisk,
                virtual_ms: T,
            },
        )
    })
    .await
    .unwrap();

    // O emulador gravou de verdade: conteúdo novo e mtime que não é o
    // `ondisk_ms` anotado.
    h.write_local("save.bin", b"conteudo-novo", T + S10);
    let summary = h.sync().await;

    assert_eq!(summary.uploaded, 1);
    assert_eq!(h.remote_content("save.bin").unwrap(), b"conteudo-novo");

    let restantes =
        h.db.with(|conn| {
            crate::storage::mtime_overrides::list_for_category(conn, EMU, SyncCategory::Saves)
        })
        .await
        .unwrap();
    assert!(
        restantes.is_empty(),
        "override obsoleto deveria ter sido descartado"
    );
}

/// `LocalStorage` que imita a granularidade do FAT32: toda escrita com mtime
/// definido é arredondada para baixo, para o múltiplo de 2 segundos mais
/// próximo. Delega o resto ao [`DesktopStorage`].
///
/// Existe porque o `tempdir` dos testes fica num filesystem de granularidade
/// fina, onde o mtime pedido é exatamente o mtime gravado — e é justamente a
/// divergência entre os dois que faz o engine registrar um override.
struct RoundingStorage {
    inner: DesktopStorage,
}

/// Granularidade do FAT32 para mtime.
const FAT32_GRANULARITY_MS: i64 = 2_000;

fn round_down_to_fat32(mtime_ms: i64) -> i64 {
    mtime_ms - mtime_ms.rem_euclid(FAT32_GRANULARITY_MS)
}

#[async_trait::async_trait]
impl crate::sync::LocalStorage for RoundingStorage {
    async fn scan(
        &self,
        root: &std::path::Path,
        bases: &[PathBuf],
    ) -> crate::error::AppResult<Vec<crate::sync::diff::LocalFile>> {
        self.inner.scan(root, bases).await
    }

    fn join(&self, base: &FileLoc, rel_path: &str) -> FileLoc {
        self.inner.join(base, rel_path)
    }

    fn root_loc(&self, root: &std::path::Path) -> FileLoc {
        self.inner.root_loc(root)
    }

    fn loc_to_stored(&self, loc: &FileLoc) -> String {
        self.inner.loc_to_stored(loc)
    }

    fn loc_from_stored(&self, stored: &str) -> FileLoc {
        self.inner.loc_from_stored(stored)
    }

    async fn exists(&self, loc: &FileLoc) -> bool {
        self.inner.exists(loc).await
    }

    async fn mtime_ms(&self, loc: &FileLoc) -> crate::error::AppResult<i64> {
        self.inner.mtime_ms(loc).await
    }

    async fn read(&self, loc: &FileLoc) -> crate::error::AppResult<Vec<u8>> {
        self.inner.read(loc).await
    }

    async fn write_atomic(
        &self,
        dest: &FileLoc,
        bytes: &[u8],
        mtime_ms: Option<i64>,
    ) -> crate::error::AppResult<()> {
        self.inner
            .write_atomic(dest, bytes, mtime_ms.map(round_down_to_fat32))
            .await
    }

    async fn copy_to(&self, src: &FileLoc, dest: &FileLoc) -> crate::error::AppResult<()> {
        self.inner.copy_to(src, dest).await
    }

    async fn is_valid_root(&self, loc: &FileLoc) -> bool {
        self.inner.is_valid_root(loc).await
    }

    async fn subdir_exists(&self, root: &FileLoc, rel: &str) -> bool {
        self.inner.subdir_exists(root, rel).await
    }
}

/// Num filesystem que arredonda, o download registra o override sozinho: o
/// mtime pedido (vindo do provedor) e o gravado no disco divergem, e é esse
/// par que fica anotado.
#[tokio::test]
async fn download_em_filesystem_que_arredonda_registra_o_override() {
    let h = Harness::with_storage(Arc::new(RoundingStorage {
        inner: DesktopStorage,
    }))
    .await;
    // Mtime que NÃO cai numa fronteira de 2s, para o arredondamento morder.
    let remoto_ms = T + 1_234;
    h.seed_remote("save.bin", b"conteudo", remoto_ms, None);

    h.sync().await;

    let overrides =
        h.db.with(|conn| {
            crate::storage::mtime_overrides::list_for_category(conn, EMU, SyncCategory::Saves)
        })
        .await
        .unwrap();

    let anotado = overrides
        .get("save.bin")
        .expect("o download deveria ter registrado o override");
    assert_eq!(
        anotado.virtual_ms, remoto_ms,
        "o mtime lógico é o do provedor"
    );
    assert_eq!(
        anotado.ondisk_ms,
        round_down_to_fat32(remoto_ms),
        "o mtime anotado é o que o filesystem de fato gravou"
    );
}

/// Fecha o ciclo: gravado o override pelo download, o sync seguinte não sobe
/// nada — sem ele, o mtime arredondado faria o arquivo parecer modificado.
#[tokio::test]
async fn segundo_sync_apos_download_arredondado_nao_sobe_nada() {
    let h = Harness::with_storage(Arc::new(RoundingStorage {
        inner: DesktopStorage,
    }))
    .await;
    h.seed_remote("save.bin", b"conteudo", T + 1_234, None);
    h.sync().await;

    let segundo = h.sync().await;

    assert_eq!(segundo.uploaded, 0, "nada mudou; nada deveria subir");
    assert_eq!(segundo.downloaded, 0);
}

/// Num filesystem de granularidade fina o mtime pedido é o gravado, então não
/// há nada a compensar e nenhuma linha é criada.
#[tokio::test]
async fn download_em_filesystem_preciso_nao_registra_override() {
    let h = Harness::new().await;
    h.seed_remote("save.bin", b"conteudo", T + 1_234, None);

    h.sync().await;

    let overrides =
        h.db.with(|conn| {
            crate::storage::mtime_overrides::list_for_category(conn, EMU, SyncCategory::Saves)
        })
        .await
        .unwrap();
    assert!(
        overrides.is_empty(),
        "sem divergência de mtime, não há override a registrar"
    );
}
