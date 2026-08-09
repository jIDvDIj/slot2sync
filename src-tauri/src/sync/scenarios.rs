//! Cenários de integração do `SyncEngine`: engine real de ponta a
//! ponta — SQLite em memória, filesystem em `tempdir`, `AppHandle` do
//! `MockRuntime` — com o Drive substituído pelo [`MockDrive`] em memória.
//! Sem rede e sem credenciais; testes com credenciais reais ficam atrás da
//! feature `integration-tests` (hoje vazia).

use std::path::PathBuf;
use std::sync::Arc;

use tauri::test::MockRuntime;

use super::engine::ConflictResolution;
use super::{DesktopStorage, LastSyncStore, SyncCategory, SyncDirection, SyncEngine, SyncSummary};
use crate::auth::AuthManager;
use crate::constants::{DRIVE_BATCH_MIN_OPS, DRIVE_MANIFEST_FILE, KEYRING_REFRESH_TOKEN_KEY};
use crate::drive::mock::MockDrive;
use crate::emulator::EmulatorProfile;
use crate::secrets::{MemSecrets, SecretStore};
use crate::storage::db::Db;
use crate::storage::settings::NotificationLevel;
use crate::storage::{conflicts, emulators, manifest, settings};

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

        // "Conectado": o status() só exige um refresh token no SecretStore.
        let secrets: Arc<dyn SecretStore> = Arc::new(MemSecrets::default());
        secrets
            .set(
                KEYRING_REFRESH_TOKEN_KEY,
                r#"{"refresh_token":"tok-teste","email":"teste@slot2sync"}"#,
            )
            .unwrap();
        let device_id = crate::device::get_or_create(&*secrets).unwrap();

        let auth = Arc::new(AuthManager::new(reqwest::Client::new(), secrets.clone()));
        let drive = Arc::new(MockDrive::new());

        let app = tauri::test::mock_builder()
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();

        let engine = SyncEngine::new(
            db.clone(),
            drive.clone(),
            auth,
            app.handle().clone(),
            LastSyncStore::default(),
            backups_dir.clone(),
            Arc::new(DesktopStorage),
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
            ConflictResolution::Drive,
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
