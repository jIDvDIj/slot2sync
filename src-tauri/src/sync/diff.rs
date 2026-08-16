//! Scan do estado local e montagem do plano de sincronização:
//! união (local ∪ remoto ∪ manifest) → `conflict::decide` por arquivo →
//! filtro pela direção do sync.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::conflict::{decide, SyncAction};
use super::storage::FileLoc;
use super::SyncDirection;
use crate::constants::{TMP_PREFIX_UNIX, TMP_PREFIX_WINDOWS};
use crate::error::{AppError, AppResult};
use crate::remote::RemoteFile;
use crate::storage::manifest::ManifestEntry;

/// Acima deste tamanho, o nome-base do temporário vira um hash — nomes de
/// jogo/arquivo muito longos (comuns em coleções importadas de outros
/// sistemas) podem estourar o limite de caminho do filesystem quando somados
/// ao prefixo e ao caminho da pasta.
const TMP_NAME_HASH_THRESHOLD: usize = 200;

/// Nome do arquivo temporário de gravação atômica para `name` (nome final,
/// sem diretório). Prefixo por plataforma (ver `TMP_PREFIX_WINDOWS`/
/// `TMP_PREFIX_UNIX`); nomes muito longos viram um hash curto do nome
/// original em vez de `prefixo + nome`.
pub fn tmp_name(name: &str) -> String {
    let prefix = if cfg!(target_os = "windows") {
        TMP_PREFIX_WINDOWS
    } else {
        TMP_PREFIX_UNIX
    };
    if name.len() > TMP_NAME_HASH_THRESHOLD {
        format!("{prefix}{}", &super::sha256_hex(name.as_bytes())[..16])
    } else {
        format!("{prefix}{name}")
    }
}

/// `true` se `name` é um temporário de gravação atômica do Slot2Sync — checa
/// os dois prefixos (não só o da plataforma atual), já que um scan pode topar
/// com um resto de escrita feita por outra instalação/SO (ex.: dual-boot
/// apontando para a mesma pasta).
pub fn is_temp_name(name: &str) -> bool {
    name.starts_with(TMP_PREFIX_WINDOWS) || name.starts_with(TMP_PREFIX_UNIX)
}

#[derive(Debug, Clone)]
pub struct LocalFile {
    /// Relativo à pasta-base da categoria, sempre com separador `/`.
    pub rel_path: String,
    /// Locador opaco do arquivo no armazenamento local (ver [`FileLoc`]).
    pub loc: FileLoc,
    pub mtime_ms: i64,
    /// Remanescente sub-milissegundo do mtime (0..999_999 ns), quando o SO/
    /// filesystem oferece essa precisão. `0` em plataformas que não oferecem
    /// (mobile via SAF) ou quando o próprio mtime é `0`. Refina a detecção de
    /// "arquivo tocado" em [`crate::sync::engine::SyncEngine::hash_touched_files`]
    /// dentro da janela de tolerância de [`super::conflict::TIMESTAMP_TOLERANCE_MS`]
    /// — duas escritas reais quase nunca compartilham o mesmo remanescente,
    /// então ele distingue "conteúdo idêntico" de "escrita rápida sucessiva
    /// que a tolerância de 2s teria mascarado".
    pub mtime_ns: i64,
    #[allow(dead_code)]
    pub size_bytes: i64,
    /// SHA-256 (hex) do conteúdo atual. Calculado pelo engine SOMENTE quando o
    /// mtime diverge da âncora do manifest (pré-filtro) — `None` nos demais.
    pub hash: Option<String>,
}

/// Operação planejada (apenas Upload/Download; NoOps são contados à parte).
#[derive(Debug, Clone)]
pub struct PlannedOp {
    pub rel_path: String,
    pub action: SyncAction,
    pub local: Option<LocalFile>,
    pub remote: Option<RemoteFile>,
}

/// Varre as pastas-base de uma categoria (relativas a `root`). Em `rel_path`
/// duplicado entre bases, a primeira base vence. Ignora symlinks e arquivos
/// temporários do Slot2Sync. Pastas inexistentes são puladas sem erro.
/// Consumida pela `DesktopStorage`; no mobile o scan é feito pelo plugin nativo.
#[cfg_attr(not(desktop), allow(dead_code))]
pub fn scan_local_bases(root: &Path, bases: &[PathBuf]) -> AppResult<Vec<LocalFile>> {
    // Sem isto, uma raiz ausente (drive removível desconectado, pasta de rede
    // fora do ar) faz todo `base` parecer "pasta inexistente, pular" — o scan
    // volta vazio como se todo arquivo local tivesse sumido, e o diff tenta
    // baixar de volta a coleção inteira para dentro do que seria o ponto de
    // montagem (ver `AppError::FolderNotMounted`: erro dedicado e
    // não-retryable, tratado antes de entrar no plano por arquivo).
    if !root.is_dir() {
        return Err(AppError::FolderNotMounted(root.display().to_string()));
    }

    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for base in bases {
        let base_abs = root.join(base);
        if base_abs.is_dir() {
            walk(&base_abs, &base_abs, &mut seen, &mut out)?;
        }
    }
    Ok(out)
}

#[cfg_attr(not(desktop), allow(dead_code))]
fn walk(
    base: &Path,
    dir: &Path,
    seen: &mut HashSet<String>,
    out: &mut Vec<LocalFile>,
) -> AppResult<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            continue;
        }
        let path = entry.path();
        if file_type.is_dir() {
            walk(base, &path, seen, out)?;
            continue;
        }

        let name = entry.file_name().to_string_lossy().into_owned();
        if is_temp_name(&name) {
            continue;
        }

        let rel = path
            .strip_prefix(base)
            .map_err(|e| AppError::Other(format!("caminho fora da base no scan: {e}")))?;
        let rel_path = rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/");
        if !seen.insert(rel_path.clone()) {
            continue;
        }

        let metadata = entry.metadata()?;
        let modified = metadata.modified()?;
        out.push(LocalFile {
            rel_path,
            loc: FileLoc::from_path(path),
            mtime_ms: system_time_ms(modified),
            mtime_ns: system_time_subsec_ns_remainder(modified),
            size_bytes: metadata.len() as i64,
            hash: None,
        });
    }
    Ok(())
}

#[cfg_attr(not(desktop), allow(dead_code))]
pub fn system_time_ms(time: SystemTime) -> i64 {
    time.duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Nanossegundos restantes dentro do milissegundo atual de `time` (0..999_999).
/// Complementa [`system_time_ms`] com a precisão sub-milissegundo que o SO
/// eventualmente oferece (ext4, NTFS) e que o `as_millis()` trunca.
#[cfg_attr(not(desktop), allow(dead_code))]
pub fn system_time_subsec_ns_remainder(time: SystemTime) -> i64 {
    time.duration_since(UNIX_EPOCH)
        .map(|d| (d.subsec_nanos() % 1_000_000) as i64)
        .unwrap_or(0)
}

/// Plano montado para uma categoria.
pub struct CategoryPlan {
    pub ops: Vec<PlannedOp>,
    /// Arquivos sem mudança (inclui os descartados pela direção do sync).
    pub skipped: u32,
    /// Entradas cujo arquivo local só teve o mtime tocado (hash idêntico ao do
    /// último sync): nada a transferir, mas o manifest precisa reancorar o
    /// mtime — senão o pré-filtro dispara de novo em todo sync seguinte.
    pub mtime_refreshes: Vec<ManifestEntry>,
}

/// Monta o plano da categoria. Retorna as operações ativas, a contagem de
/// arquivos sem mudança e as reancoragens de mtime (hash igual).
pub fn build_plan(
    local: Vec<LocalFile>,
    remote: Vec<RemoteFile>,
    manifest: Vec<ManifestEntry>,
    direction: SyncDirection,
    this_device_id: Option<&str>,
) -> CategoryPlan {
    let local_map: HashMap<String, LocalFile> =
        local.into_iter().map(|f| (f.rel_path.clone(), f)).collect();
    let remote_map: HashMap<String, RemoteFile> = remote
        .into_iter()
        .map(|f| (f.rel_path.clone(), f))
        .collect();
    let manifest_map: HashMap<String, ManifestEntry> = manifest
        .into_iter()
        .map(|e| (e.rel_path.clone(), e))
        .collect();

    let all_paths: BTreeSet<String> = local_map.keys().chain(remote_map.keys()).cloned().collect();

    let mut ops = Vec::new();
    let mut skipped: u32 = 0;
    let mut mtime_refreshes = Vec::new();

    for rel_path in all_paths {
        let local_file = local_map.get(&rel_path);
        let remote_file = remote_map.get(&rel_path);
        let last_synced = manifest_map
            .get(&rel_path)
            .and_then(|e| e.local_mtime_ms.zip(e.remote_mtime_ms));

        let action = decide(
            local_file.map(|f| f.mtime_ms),
            remote_file.and_then(|f| f.modified_ms),
            last_synced,
            remote_file.and_then(|f| f.device_id.as_deref()),
            this_device_id,
        );

        // Pré-filtro de hash: o mtime local divergiu, mas o conteúdo é idêntico
        // ao do último sync — o emulador só tocou o timestamp. Nada a enviar;
        // reancora o mtime no manifest para não recalcular o hash sempre.
        if action == SyncAction::Upload {
            if let (Some(file), Some(entry)) = (local_file, manifest_map.get(&rel_path)) {
                if file.hash.is_some() && file.hash == entry.file_hash {
                    let mut refreshed = entry.clone();
                    refreshed.local_mtime_ms = Some(file.mtime_ms);
                    refreshed.mtime_ns = file.mtime_ns;
                    mtime_refreshes.push(refreshed);
                    skipped += 1;
                    continue;
                }
            }
        }

        let allowed = match action {
            SyncAction::NoOp => false,
            SyncAction::Upload => direction != SyncDirection::DriveToLocal,
            SyncAction::Download | SyncAction::DownloadWithBackup => {
                direction != SyncDirection::LocalToDrive
            }
            // Conflito é registrado em qualquer direção — nunca queremos
            // sobrescrever silenciosamente, mesmo num sync de mão única.
            SyncAction::Conflict => true,
        };

        if allowed {
            ops.push(PlannedOp {
                rel_path,
                action,
                local: local_file.cloned(),
                remote: remote_file.cloned(),
            });
        } else {
            skipped += 1;
        }
    }

    CategoryPlan {
        ops,
        skipped,
        mtime_refreshes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::SyncCategory;

    const T: i64 = 1_700_000_000_000;

    #[test]
    fn system_time_subsec_ns_remainder_extrai_so_o_resto_sub_ms() {
        let t = UNIX_EPOCH + std::time::Duration::new(1_700_000_000, 123_456_789);
        assert_eq!(system_time_ms(t), 1_700_000_000_123);
        // 123_456_789 ns % 1_000_000 = 456_789 (resto dentro do milissegundo).
        assert_eq!(system_time_subsec_ns_remainder(t), 456_789);
    }

    #[test]
    fn system_time_subsec_ns_remainder_e_zero_em_fronteira_de_ms() {
        let t = UNIX_EPOCH + std::time::Duration::new(1_700_000_000, 5_000_000);
        assert_eq!(system_time_subsec_ns_remainder(t), 0);
    }

    #[test]
    fn tmp_name_usa_o_prefixo_da_plataforma_atual() {
        let name = tmp_name("save.bin");
        let prefix = if cfg!(target_os = "windows") {
            TMP_PREFIX_WINDOWS
        } else {
            TMP_PREFIX_UNIX
        };
        assert_eq!(name, format!("{prefix}save.bin"));
    }

    #[test]
    fn tmp_name_troca_por_hash_quando_o_nome_e_muito_longo() {
        let long_name = "a".repeat(250);
        let name = tmp_name(&long_name);
        assert!(name.len() < long_name.len());
        assert!(is_temp_name(&name));
    }

    #[test]
    fn is_temp_name_reconhece_os_dois_prefixos_independente_do_so_atual() {
        assert!(is_temp_name(&format!("{TMP_PREFIX_WINDOWS}save.bin")));
        assert!(is_temp_name(&format!("{TMP_PREFIX_UNIX}save.bin")));
        assert!(!is_temp_name("save.bin"));
    }

    fn local_file(rel: &str, mtime: i64) -> LocalFile {
        LocalFile {
            rel_path: rel.to_string(),
            loc: FileLoc::from_path(PathBuf::from("/tmp").join(rel)),
            mtime_ms: mtime,
            mtime_ns: 0,
            size_bytes: 100,
            hash: None,
        }
    }

    fn remote_file(rel: &str, mtime: i64) -> RemoteFile {
        RemoteFile {
            id: format!("id-{rel}"),
            rel_path: rel.to_string(),
            modified_ms: Some(mtime),
            size_bytes: Some(100),
            hash: None,
            device_name: None,
            device_id: None,
        }
    }

    /// Como [`remote_file`], mas marcando o ID do dispositivo que publicou a versão.
    fn remote_file_from(rel: &str, mtime: i64, device_id: &str) -> RemoteFile {
        let mut rf = remote_file(rel, mtime);
        rf.device_id = Some(device_id.to_string());
        rf
    }

    fn manifest_entry(rel: &str, local: i64, remote: i64) -> ManifestEntry {
        ManifestEntry {
            emulator: "PPSSPP".into(),
            category: SyncCategory::Saves,
            rel_path: rel.to_string(),
            remote_file_id: Some(format!("id-{rel}")),
            local_mtime_ms: Some(local),
            remote_mtime_ms: Some(remote),
            size_bytes: Some(100),
            last_synced_at_ms: T,
            file_hash: None,
            flags: 0,
            inaccessible: false,
            mtime_ns: 0,
        }
    }

    #[test]
    fn arquivo_novo_local_vira_upload() {
        let CategoryPlan { ops, skipped, .. } = build_plan(
            vec![local_file("novo.bin", T)],
            vec![],
            vec![],
            SyncDirection::Bidirectional,
            None,
        );
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].action, SyncAction::Upload);
        assert_eq!(skipped, 0);
    }

    #[test]
    fn arquivo_novo_no_drive_vira_download() {
        let CategoryPlan { ops, .. } = build_plan(
            vec![],
            vec![remote_file("remoto.bin", T)],
            vec![],
            SyncDirection::Bidirectional,
            None,
        );
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].action, SyncAction::Download);
        assert!(ops[0].remote.is_some());
    }

    #[test]
    fn mtime_tocado_com_hash_igual_vira_refresh_sem_upload() {
        // O emulador reescreveu o arquivo sem mudar o conteúdo: mtime diverge,
        // hash bate com o manifest → nada a transferir, só reancorar o mtime.
        let mut file = local_file("save.bin", T + 60_000);
        file.hash = Some("hash-igual".into());
        let mut entry = manifest_entry("save.bin", T, T);
        entry.file_hash = Some("hash-igual".into());

        let CategoryPlan {
            ops,
            skipped,
            mtime_refreshes,
        } = build_plan(
            vec![file],
            vec![remote_file("save.bin", T)],
            vec![entry],
            SyncDirection::Bidirectional,
            None,
        );

        assert!(ops.is_empty());
        assert_eq!(skipped, 1);
        assert_eq!(mtime_refreshes.len(), 1);
        assert_eq!(mtime_refreshes[0].local_mtime_ms, Some(T + 60_000));
    }

    #[test]
    fn mtime_tocado_com_hash_diferente_segue_como_upload() {
        let mut file = local_file("save.bin", T + 60_000);
        file.hash = Some("hash-novo".into());
        let mut entry = manifest_entry("save.bin", T, T);
        entry.file_hash = Some("hash-antigo".into());

        let CategoryPlan {
            ops,
            mtime_refreshes,
            ..
        } = build_plan(
            vec![file],
            vec![remote_file("save.bin", T)],
            vec![entry],
            SyncDirection::Bidirectional,
            None,
        );

        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].action, SyncAction::Upload);
        assert!(mtime_refreshes.is_empty());
    }

    #[test]
    fn sem_hash_local_o_upload_nao_e_filtrado() {
        // Hash ausente (arquivo não foi lido no pré-passo) não pode suprimir o
        // upload — na dúvida, transfere.
        let mut entry = manifest_entry("save.bin", T, T);
        entry.file_hash = Some("hash".into());

        let CategoryPlan { ops, .. } = build_plan(
            vec![local_file("save.bin", T + 60_000)],
            vec![remote_file("save.bin", T)],
            vec![entry],
            SyncDirection::Bidirectional,
            None,
        );

        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].action, SyncAction::Upload);
    }

    #[test]
    fn arquivo_sem_mudanca_e_pulado() {
        let CategoryPlan { ops, skipped, .. } = build_plan(
            vec![local_file("igual.bin", T)],
            vec![remote_file("igual.bin", T)],
            vec![manifest_entry("igual.bin", T, T)],
            SyncDirection::Bidirectional,
            None,
        );
        assert!(ops.is_empty());
        assert_eq!(skipped, 1);
    }

    #[test]
    fn local_mais_recente_vira_upload_com_remote_id() {
        let CategoryPlan { ops, .. } = build_plan(
            vec![local_file("save.bin", T + 60_000)],
            vec![remote_file("save.bin", T)],
            vec![manifest_entry("save.bin", T, T)],
            SyncDirection::Bidirectional,
            None,
        );
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].action, SyncAction::Upload);
        assert_eq!(ops[0].remote.as_ref().unwrap().id, "id-save.bin");
    }

    #[test]
    fn primeiro_sync_com_arquivo_nos_dois_lados_baixa_com_backup() {
        // Sem manifest e arquivo presente local e no Drive → DownloadWithBackup.
        let CategoryPlan { ops, skipped, .. } = build_plan(
            vec![local_file("save.bin", T + 60_000)],
            vec![remote_file("save.bin", T)],
            vec![],
            SyncDirection::Bidirectional,
            None,
        );
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].action, SyncAction::DownloadWithBackup);
        assert!(ops[0].local.is_some());
        assert!(ops[0].remote.is_some());
        assert_eq!(skipped, 0);
    }

    #[test]
    fn primeiro_sync_de_outro_dispositivo_vira_conflito() {
        // Sem manifest, ambos existem e divergem, e o Drive foi publicado por
        // outro dispositivo (dev-A) enquanto este é dev-C → Conflict.
        let CategoryPlan { ops, skipped, .. } = build_plan(
            vec![local_file("save.bin", T + 60_000)],
            vec![remote_file_from("save.bin", T, "dev-A")],
            vec![],
            SyncDirection::Bidirectional,
            Some("dev-C"),
        );
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].action, SyncAction::Conflict);
        assert!(ops[0].local.is_some());
        assert!(ops[0].remote.is_some());
        assert_eq!(skipped, 0);
    }

    #[test]
    fn primeiro_sync_do_mesmo_dispositivo_baixa_com_backup() {
        // Mesmo dispositivo publicou o Drive (dev-C) e é este (dev-C): não há
        // conflito entre dispositivos — segue o Drive-vence com backup.
        let CategoryPlan { ops, .. } = build_plan(
            vec![local_file("save.bin", T + 60_000)],
            vec![remote_file_from("save.bin", T, "dev-C")],
            vec![],
            SyncDirection::Bidirectional,
            Some("dev-C"),
        );
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].action, SyncAction::DownloadWithBackup);
    }

    #[test]
    fn ambos_mudaram_desde_o_ultimo_sync_vira_conflito() {
        // local e drive divergem de (T, T) registrado → Conflict, com os dois
        // lados disponíveis para a UI.
        let CategoryPlan { ops, .. } = build_plan(
            vec![local_file("save.bin", T + 300_000)],
            vec![remote_file("save.bin", T + 60_000)],
            vec![manifest_entry("save.bin", T, T)],
            SyncDirection::Bidirectional,
            None,
        );
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].action, SyncAction::Conflict);
        assert!(ops[0].local.is_some());
        assert!(ops[0].remote.is_some());
    }

    #[test]
    fn direcao_drive_to_local_descarta_uploads() {
        let CategoryPlan { ops, skipped, .. } = build_plan(
            vec![local_file("novo.bin", T)],
            vec![remote_file("remoto.bin", T)],
            vec![],
            SyncDirection::DriveToLocal,
            None,
        );
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].action, SyncAction::Download);
        assert_eq!(skipped, 1);
    }

    #[test]
    fn direcao_local_to_drive_descarta_downloads() {
        let CategoryPlan { ops, skipped, .. } = build_plan(
            vec![local_file("novo.bin", T)],
            vec![remote_file("remoto.bin", T)],
            vec![],
            SyncDirection::LocalToDrive,
            None,
        );
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].action, SyncAction::Upload);
        assert_eq!(skipped, 1);
    }

    #[test]
    fn scan_ignora_temporarios_e_entra_em_subpastas() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path().join("SAVEDATA");
        std::fs::create_dir_all(base.join("GAME01")).unwrap();
        std::fs::write(base.join("GAME01/SAVE.bin"), b"abc").unwrap();
        std::fs::write(base.join("topo.txt"), b"x").unwrap();
        std::fs::write(base.join(tmp_name("baixando")), b"parcial").unwrap();

        let files = scan_local_bases(tmp.path(), &[PathBuf::from("SAVEDATA")]).unwrap();

        let mut rels: Vec<_> = files.iter().map(|f| f.rel_path.as_str()).collect();
        rels.sort();
        assert_eq!(rels, vec!["GAME01/SAVE.bin", "topo.txt"]);
    }

    #[test]
    fn scan_de_base_inexistente_retorna_vazio() {
        let tmp = tempfile::tempdir().unwrap();
        let files = scan_local_bases(tmp.path(), &[PathBuf::from("NAO_EXISTE")]).unwrap();
        assert!(files.is_empty());
    }

    #[test]
    fn scan_com_raiz_inexistente_retorna_folder_not_mounted() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("pendrive-desconectado");

        let err = scan_local_bases(&root, &[PathBuf::from("SAVEDATA")]).unwrap_err();

        assert!(matches!(err, AppError::FolderNotMounted(_)));
    }
}
