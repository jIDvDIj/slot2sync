//! Watcher de filesystem (gatilho `file-change`): reage a escritas nas pastas
//! de saves/savestates sem esperar o emulador fechar — útil em sessões longas.
//!
//! - **Eventos nativos** via a crate `notify` (`ReadDirectoryChangesW` no
//!   Windows, `inotify` no Linux, `FSEvents` no macOS);
//! - **Debounce agregador**: cada evento reinicia a janela do emulador; o sync
//!   só dispara `FS_WATCHER_DEBOUNCE_SECS` após o ÚLTIMO evento (agrupa
//!   rajadas de escrita em um único sync);
//! - **Anti-loop**: eventos em arquivos que o próprio sync acabou de baixar
//!   (`SyncEngine::is_recent_download`) e em temporários `.slot2sync-tmp` são
//!   ignorados;
//! - **Nunca com o jogo aberto**: o disparo é adiado enquanto qualquer
//!   emulador estiver rodando (o gatilho `emulator-stop` cobre o fechamento).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;

use super::RunningEmulators;
use crate::constants::{FS_WATCHER_DEBOUNCE_SECS, FS_WATCHER_RECONCILE_SECS, TRIGGER_FILE_CHANGE};
use crate::storage::db::Db;
use crate::storage::emulators;
use crate::sync::{is_temp_name, SyncDirection, SyncEngine};

/// Pastas observadas de um emulador (absolutas: raiz + bases de saves/states).
struct WatchedEmulator {
    name: String,
    dirs: Vec<PathBuf>,
}

/// Emuladores cuja janela de debounce venceu (sem eventos novos há pelo menos
/// `debounce`). Função pura para ser testável.
fn due_emulators(
    pending: &HashMap<String, Instant>,
    now: Instant,
    debounce: Duration,
) -> Vec<String> {
    pending
        .iter()
        .filter(|(_, last)| now.duration_since(**last) >= debounce)
        .map(|(name, _)| name.clone())
        .collect()
}

/// Só mudanças de conteúdo/estrutura interessam — eventos de acesso (leitura)
/// gerariam ruído constante com o emulador aberto.
fn is_relevant(kind: &notify::EventKind) -> bool {
    matches!(
        kind,
        notify::EventKind::Create(_) | notify::EventKind::Modify(_) | notify::EventKind::Remove(_)
    )
}

/// Emulador dono de `path`, se o caminho está sob alguma pasta observada.
fn owner_of<'a>(watched: &'a [WatchedEmulator], path: &Path) -> Option<&'a str> {
    watched
        .iter()
        .find(|w| w.dirs.iter().any(|dir| path.starts_with(dir)))
        .map(|w| w.name.as_str())
}

/// Temporários do próprio sync (`.slot2sync-tmp`) nunca disparam um novo sync.
fn is_tmp_path(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|n| is_temp_name(&n.to_string_lossy()))
}

/// Alguma pasta observada mudou desde a última reconciliação (emulador
/// adicionado/removido ou raiz/paths trocados) — função pura para ser
/// testável sem tocar o SQLite.
fn watch_list_changed(old: &[WatchedEmulator], fresh: &[WatchedEmulator]) -> bool {
    fresh.len() != old.len()
        || fresh
            .iter()
            .zip(old)
            .any(|(a, b)| a.name != b.name || a.dirs != b.dirs)
}

/// Algum emulador monitorado está rodando agora — enquanto isso, o disparo do
/// fs-watcher fica em espera (o gatilho `emulator-stop` cobre o fechamento).
/// Mutex "envenenado" (thread pânico com o lock preso) é tratado como "livre"
/// para não travar o watcher para sempre.
fn is_any_emulator_running(running: &RunningEmulators) -> bool {
    running.lock().map(|set| !set.is_empty()).unwrap_or(false)
}

/// Monta a lista de pastas observadas a partir dos perfis configurados
/// (saves + savestates; config fica de fora — muda o tempo todo com o app
/// aberto e já sincroniza nos gatilhos de processo).
async fn watch_list(db: &Db) -> Vec<WatchedEmulator> {
    let profiles = match db.with(emulators::list).await {
        Ok(profiles) => profiles,
        Err(err) => {
            tracing::warn!(error = %err, "fs-watcher: falha ao listar emuladores");
            return Vec::new();
        }
    };
    profiles
        .into_iter()
        .map(|p| WatchedEmulator {
            dirs: p
                .saves_paths
                .iter()
                .chain(&p.state_paths)
                .map(|rel| p.root_path.join(rel))
                .filter(|abs| abs.is_dir())
                .collect(),
            name: p.name,
        })
        .filter(|w| !w.dirs.is_empty())
        .collect()
}

/// (Re)cria o watcher nativo observando as pastas de `watched`. Devolve `None`
/// (com warning) se o backend nativo não puder ser iniciado.
fn build_watcher(
    watched: &[WatchedEmulator],
    tx: mpsc::Sender<PathBuf>,
) -> Option<RecommendedWatcher> {
    let mut watcher =
        match notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            let Ok(event) = res else { return };
            if !is_relevant(&event.kind) {
                return;
            }
            for path in event.paths {
                // `blocking_send` roda na thread do backend nativo, fora do runtime.
                let _ = tx.blocking_send(path);
            }
        }) {
            Ok(watcher) => watcher,
            Err(err) => {
                tracing::warn!(error = %err, "fs-watcher: backend nativo indisponível");
                return None;
            }
        };

    for w in watched {
        for dir in &w.dirs {
            if let Err(err) = watcher.watch(dir, RecursiveMode::Recursive) {
                tracing::warn!(pasta = %dir.display(), error = %err, "fs-watcher: falha ao observar pasta");
            }
        }
    }
    Some(watcher)
}

/// Sobe o watcher de filesystem. Reconciliação periódica reabsorve mudanças na
/// lista de emuladores (adicionados/removidos/raiz trocada).
pub fn start(
    db: Db,
    engine: Arc<SyncEngine>,
    running: RunningEmulators,
    shutdown: crate::shutdown::ShutdownHandle,
) {
    shutdown.tracker.clone().spawn(async move {
        let (tx, mut rx) = mpsc::channel::<PathBuf>(256);
        let mut watched = watch_list(&db).await;
        // O watcher precisa permanecer vivo — dropar cancela as observações.
        let mut _watcher = build_watcher(&watched, tx.clone());

        // Última atividade por emulador (janela de debounce em aberto).
        let mut pending: HashMap<String, Instant> = HashMap::new();
        let debounce = Duration::from_secs(FS_WATCHER_DEBOUNCE_SECS);
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        let mut reconcile = tokio::time::interval(Duration::from_secs(FS_WATCHER_RECONCILE_SECS));
        reconcile.reset(); // o primeiro tick de `interval` é imediato

        loop {
            tokio::select! {
                Some(path) = rx.recv() => {
                    if is_tmp_path(&path) {
                        continue;
                    }
                    // Anti-loop: escrita feita pelo próprio sync há pouco.
                    if engine.is_recent_download(&path) {
                        continue;
                    }
                    if let Some(name) = owner_of(&watched, &path) {
                        pending.insert(name.to_string(), Instant::now());
                    }
                }
                _ = tick.tick() => {
                    let due = due_emulators(&pending, Instant::now(), debounce);
                    if due.is_empty() {
                        continue;
                    }
                    // Jogo aberto: mantém a janela pendente — o sync sai quando
                    // o emulador fechar (ou no próximo tick sem processo).
                    if is_any_emulator_running(&running) {
                        continue;
                    }
                    for name in due {
                        pending.remove(&name);
                        tracing::info!(emulador = %name, "mudança de arquivo detectada; sync Local → Drive");
                        if let Err(err) = engine
                            .sync_emulator(&name, SyncDirection::LocalToDrive, TRIGGER_FILE_CHANGE)
                            .await
                        {
                            tracing::warn!(emulador = %name, error = %err, "sync do fs-watcher falhou");
                        }
                    }
                }
                _ = shutdown.token.cancelled() => {
                    tracing::debug!("fs-watcher: desligamento sinalizado; observação encerrada");
                    return;
                }
                _ = reconcile.tick() => {
                    let fresh = watch_list(&db).await;
                    if watch_list_changed(&watched, &fresh) {
                        watched = fresh;
                        _watcher = build_watcher(&watched, tx.clone());
                        tracing::info!(emuladores = watched.len(), "fs-watcher: pastas observadas reconciliadas");
                    }
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn due_emulators_respeita_a_janela_de_debounce() {
        let now = Instant::now();
        let debounce = Duration::from_secs(8);
        let mut pending = HashMap::new();
        pending.insert("PPSSPP".to_string(), now - Duration::from_secs(10));
        pending.insert("PCSX2".to_string(), now - Duration::from_secs(2));

        let due = due_emulators(&pending, now, debounce);

        assert_eq!(due, vec!["PPSSPP".to_string()]);
    }

    #[test]
    fn owner_of_casa_caminho_com_a_pasta_observada() {
        let watched = vec![WatchedEmulator {
            name: "PPSSPP".into(),
            dirs: vec![PathBuf::from("/emu/PSP/SAVEDATA")],
        }];
        assert_eq!(
            owner_of(&watched, Path::new("/emu/PSP/SAVEDATA/GAME01/SAVE.bin")),
            Some("PPSSPP")
        );
        assert_eq!(owner_of(&watched, Path::new("/outro/lugar.bin")), None);
    }

    #[test]
    fn is_relevant_filtra_eventos_de_acesso() {
        use notify::event::{AccessKind, CreateKind, ModifyKind, RemoveKind};
        assert!(is_relevant(&notify::EventKind::Create(CreateKind::File)));
        assert!(is_relevant(&notify::EventKind::Modify(ModifyKind::Any)));
        assert!(is_relevant(&notify::EventKind::Remove(RemoveKind::File)));
        assert!(!is_relevant(&notify::EventKind::Access(AccessKind::Read)));
        assert!(!is_relevant(&notify::EventKind::Any));
    }

    #[test]
    fn due_emulators_vazio_sem_pendencias() {
        let pending: HashMap<String, Instant> = HashMap::new();
        assert!(due_emulators(&pending, Instant::now(), Duration::from_secs(8)).is_empty());
    }

    /// `watch_list` monta as pastas absolutas de saves+states dos perfis,
    /// ignorando pastas inexistentes e emuladores sem nenhuma pasta válida.
    #[tokio::test]
    async fn watch_list_resolve_pastas_existentes_dos_perfis() {
        use crate::emulator::EmulatorProfile;
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("emu");
        std::fs::create_dir_all(root.join("saves")).unwrap();
        // "states" NÃO existe — deve ser filtrada.

        let db = crate::storage::db::Db::open_in_memory().unwrap();
        let profile = EmulatorProfile {
            name: "PPSSPP".into(),
            root_path: root.clone(),
            saves_paths: vec![PathBuf::from("saves")],
            state_paths: vec![PathBuf::from("states")],
            config_paths: vec![],
            exclude_patterns: vec![],
        };
        let sem_pastas = EmulatorProfile {
            name: "Fantasma".into(),
            root_path: tmp.path().join("nao-existe"),
            saves_paths: vec![PathBuf::from("saves")],
            state_paths: vec![],
            config_paths: vec![],
            exclude_patterns: vec![],
        };
        db.with(move |conn| {
            emulators::upsert(conn, &profile)?;
            emulators::upsert(conn, &sem_pastas)
        })
        .await
        .unwrap();

        let watched = watch_list(&db).await;

        assert_eq!(watched.len(), 1, "emulador sem pasta válida fica de fora");
        assert_eq!(watched[0].name, "PPSSPP");
        assert_eq!(watched[0].dirs, vec![root.join("saves")]);
    }

    /// O watcher nativo entrega eventos de escrita nas pastas observadas.
    #[tokio::test]
    async fn build_watcher_observa_escritas_na_pasta() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("saves");
        std::fs::create_dir_all(&dir).unwrap();

        let (tx, mut rx) = mpsc::channel::<PathBuf>(16);
        let watched = vec![WatchedEmulator {
            name: "PPSSPP".into(),
            dirs: vec![dir.clone()],
        }];
        let _watcher = build_watcher(&watched, tx).expect("backend nativo disponível");

        std::fs::write(dir.join("save.bin"), b"conteudo").unwrap();

        let event = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("evento dentro do prazo")
            .expect("canal aberto");
        assert!(event.starts_with(&dir));
    }

    #[test]
    fn is_tmp_path_reconhece_nome_temporario() {
        let tmp_file = crate::sync::tmp_name("save.bin");
        assert!(is_tmp_path(Path::new(&format!(
            "/emu/PSP/SAVEDATA/GAME01/{tmp_file}"
        ))));
    }

    #[test]
    fn is_tmp_path_ignora_arquivo_normal() {
        assert!(!is_tmp_path(Path::new("/emu/PSP/SAVEDATA/GAME01/save.bin")));
    }

    #[test]
    fn is_tmp_path_sem_nome_de_arquivo_nao_e_temporario() {
        // Caminho terminando em "..", sem componente de nome de arquivo.
        assert!(!is_tmp_path(Path::new("/")));
    }

    #[test]
    fn watch_list_changed_falso_quando_identico() {
        let watched = vec![WatchedEmulator {
            name: "PPSSPP".into(),
            dirs: vec![PathBuf::from("/emu/saves")],
        }];
        let fresh = vec![WatchedEmulator {
            name: "PPSSPP".into(),
            dirs: vec![PathBuf::from("/emu/saves")],
        }];
        assert!(!watch_list_changed(&watched, &fresh));
    }

    #[test]
    fn watch_list_changed_true_quando_tamanho_difere() {
        let watched = vec![WatchedEmulator {
            name: "PPSSPP".into(),
            dirs: vec![PathBuf::from("/emu/saves")],
        }];
        let fresh = vec![
            WatchedEmulator {
                name: "PPSSPP".into(),
                dirs: vec![PathBuf::from("/emu/saves")],
            },
            WatchedEmulator {
                name: "PCSX2".into(),
                dirs: vec![PathBuf::from("/emu2/saves")],
            },
        ];
        assert!(watch_list_changed(&watched, &fresh));
    }

    #[test]
    fn watch_list_changed_true_quando_nome_difere() {
        let watched = vec![WatchedEmulator {
            name: "PPSSPP".into(),
            dirs: vec![PathBuf::from("/emu/saves")],
        }];
        let fresh = vec![WatchedEmulator {
            name: "Outro".into(),
            dirs: vec![PathBuf::from("/emu/saves")],
        }];
        assert!(watch_list_changed(&watched, &fresh));
    }

    #[test]
    fn watch_list_changed_true_quando_pastas_diferem() {
        let watched = vec![WatchedEmulator {
            name: "PPSSPP".into(),
            dirs: vec![PathBuf::from("/emu/saves")],
        }];
        let fresh = vec![WatchedEmulator {
            name: "PPSSPP".into(),
            dirs: vec![PathBuf::from("/emu/saves-novo")],
        }];
        assert!(watch_list_changed(&watched, &fresh));
    }

    #[test]
    fn is_any_emulator_running_falso_quando_conjunto_vazio() {
        let running: RunningEmulators = Arc::new(std::sync::Mutex::new(HashSet::new()));
        assert!(!is_any_emulator_running(&running));
    }

    #[test]
    fn is_any_emulator_running_verdadeiro_com_emulador_no_conjunto() {
        let running: RunningEmulators =
            Arc::new(std::sync::Mutex::new(HashSet::from(["PPSSPP".to_string()])));
        assert!(is_any_emulator_running(&running));
    }

    #[test]
    fn is_any_emulator_running_trata_mutex_envenenado_como_livre() {
        let running: RunningEmulators = Arc::new(std::sync::Mutex::new(HashSet::new()));
        let clone = running.clone();
        // Provoca um pânico com o lock preso para envenenar o Mutex.
        let _ = std::thread::spawn(move || {
            let _guard = clone.lock().unwrap();
            panic!("envenenando o mutex de propósito");
        })
        .join();

        assert!(!is_any_emulator_running(&running));
    }

    #[test]
    fn owner_of_escolhe_o_emulador_correto_entre_varios() {
        let watched = vec![
            WatchedEmulator {
                name: "PPSSPP".into(),
                dirs: vec![PathBuf::from("/emu/PSP/SAVEDATA")],
            },
            WatchedEmulator {
                name: "PCSX2".into(),
                dirs: vec![PathBuf::from("/emu/PS2/saves")],
            },
        ];
        assert_eq!(
            owner_of(&watched, Path::new("/emu/PS2/saves/slot1.ps2")),
            Some("PCSX2")
        );
        assert_eq!(
            owner_of(&watched, Path::new("/emu/PSP/SAVEDATA/save.bin")),
            Some("PPSSPP")
        );
    }

    #[test]
    fn due_emulators_retorna_todos_vencidos_simultaneamente() {
        let now = Instant::now();
        let debounce = Duration::from_secs(8);
        let mut pending = HashMap::new();
        pending.insert("PPSSPP".to_string(), now - Duration::from_secs(10));
        pending.insert("PCSX2".to_string(), now - Duration::from_secs(9));

        let mut due = due_emulators(&pending, now, debounce);
        due.sort();

        assert_eq!(due, vec!["PCSX2".to_string(), "PPSSPP".to_string()]);
    }

    #[test]
    fn due_emulators_na_borda_exata_do_debounce_conta_como_vencido() {
        let now = Instant::now();
        let debounce = Duration::from_secs(8);
        let mut pending = HashMap::new();
        pending.insert("PPSSPP".to_string(), now - debounce);

        let due = due_emulators(&pending, now, debounce);

        assert_eq!(due, vec!["PPSSPP".to_string()]);
    }

    /// Lista vazia de perfis (banco sem nenhum emulador cadastrado) resulta em
    /// nenhuma pasta observada, sem erro.
    #[tokio::test]
    async fn watch_list_vazia_sem_perfis_cadastrados() {
        let db = crate::storage::db::Db::open_in_memory().unwrap();
        let watched = watch_list(&db).await;
        assert!(watched.is_empty());
    }

    /// `build_watcher` não falha ao tentar observar uma pasta inexistente —
    /// apenas registra um warning e segue observando as demais pastas válidas.
    #[tokio::test]
    async fn build_watcher_ignora_pasta_inexistente_sem_falhar() {
        let tmp = tempfile::tempdir().unwrap();
        let existe = tmp.path().join("saves");
        std::fs::create_dir_all(&existe).unwrap();
        let nao_existe = tmp.path().join("nao-existe");

        let (tx, mut rx) = mpsc::channel::<PathBuf>(16);
        let watched = vec![WatchedEmulator {
            name: "PPSSPP".into(),
            dirs: vec![existe.clone(), nao_existe],
        }];
        let _watcher = build_watcher(&watched, tx).expect("backend nativo disponível");

        std::fs::write(existe.join("save.bin"), b"conteudo").unwrap();

        let event = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("evento dentro do prazo")
            .expect("canal aberto");
        assert!(event.starts_with(&existe));
    }
}
