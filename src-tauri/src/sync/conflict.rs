//! Resolução de conflito por timestamp: o lado mais recente vence; nunca há
//! deleção. Tolerância de ±2s absorve granularidade de
//! filesystem e pequenos desvios de relógio; o par de mtimes registrado no
//! manifest no último sync permite reconhecer "nada mudou" mesmo quando os
//! relógios local e remoto divergem além da tolerância.

/// Diferenças de timestamp até este valor são tratadas como "iguais".
pub const TIMESTAMP_TOLERANCE_MS: i64 = 2_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncAction {
    Upload,
    Download,
    /// Download em que o arquivo local existente é copiado para uma pasta de
    /// backup antes de ser sobrescrito. Usado no primeiro sync de um arquivo
    /// que existe nos dois lados (Drive vence).
    DownloadWithBackup,
    /// Ambos os lados mudaram desde o último sync: nenhum vence
    /// automaticamente. O sync do emulador pausa até o usuário escolher.
    Conflict,
    NoOp,
}

/// Decide a ação para um arquivo dado seu mtime local, o `modifiedTime` no
/// Drive, o par `(local, drive)` registrado no manifest no último sync e os IDs
/// estáveis do dispositivo que publicou a versão do Drive (`drive_device`) e
/// deste dispositivo (`this_device`).
///
/// Os IDs só influenciam o **primeiro sync** de um arquivo (sem manifest): se a
/// versão do Drive veio de outro dispositivo, divergir vira conflito em vez de
/// o Drive vencer automaticamente. No caminho com manifest, os timestamps já
/// decidem corretamente (avançar sobre uma versão inalterada é seguro, mesmo
/// que ela tenha sido publicada por outro dispositivo).
pub fn decide(
    local_mtime_ms: Option<i64>,
    drive_mtime_ms: Option<i64>,
    last_synced: Option<(i64, i64)>,
    drive_device: Option<&str>,
    this_device: Option<&str>,
) -> SyncAction {
    match (local_mtime_ms, drive_mtime_ms) {
        (None, None) => SyncAction::NoOp,
        (Some(_), None) => SyncAction::Upload,
        (None, Some(_)) => SyncAction::Download,
        (Some(local), Some(drive)) => match last_synced {
            // Já sincronizado antes: o que mudou desde o último sync decide.
            // Se ambos mudaram, é conflito real — ninguém vence sozinho.
            Some((last_local, last_drive)) => {
                let local_changed = !eq_within_tolerance(local, last_local);
                let drive_changed = !eq_within_tolerance(drive, last_drive);
                match (local_changed, drive_changed) {
                    (false, false) => SyncAction::NoOp,
                    (true, false) => SyncAction::Upload,
                    (false, true) => SyncAction::Download,
                    (true, true) => {
                        // Os dois mudaram; mtime idêntico ainda é NoOp.
                        if eq_within_tolerance(local, drive) {
                            SyncAction::NoOp
                        } else {
                            SyncAction::Conflict
                        }
                    }
                }
            }
            // Primeiro sync deste arquivo (sem manifest), presente nos dois
            // lados. mtime igual = nada a fazer. Divergindo: se a versão do
            // Drive veio de OUTRO dispositivo, são saves independentes — caso
            // ambíguo que vira conflito (o usuário decide), em vez de o Drive
            // vencer cegamente. Caso contrário (mesma origem, ex.: reinstalação;
            // ou origem desconhecida), o Drive vence com backup local antes de
            // sobrescrever.
            None => {
                if eq_within_tolerance(local, drive) {
                    SyncAction::NoOp
                } else if published_by_other_device(drive_device, this_device) {
                    SyncAction::Conflict
                } else {
                    SyncAction::DownloadWithBackup
                }
            }
        },
    }
}

/// A versão do Drive foi publicada por um dispositivo identificável e diferente
/// deste? Exige ambos os IDs conhecidos; na dúvida (algum ausente) devolve
/// `false`, mantendo o comportamento conservador de Drive-vence.
fn published_by_other_device(drive_device: Option<&str>, this_device: Option<&str>) -> bool {
    matches!((drive_device, this_device), (Some(drive), Some(this)) if drive != this)
}

fn eq_within_tolerance(a: i64, b: i64) -> bool {
    (a - b).abs() <= TIMESTAMP_TOLERANCE_MS
}

#[cfg(test)]
mod tests {
    use super::*;

    const T: i64 = 1_700_000_000_000;

    /// `decide` sem informação de dispositivo — cobre os casos que dependem só
    /// de timestamp/manifest (identidade de dispositivo é irrelevante).
    fn decide_t(
        local_mtime_ms: Option<i64>,
        drive_mtime_ms: Option<i64>,
        last_synced: Option<(i64, i64)>,
    ) -> SyncAction {
        decide(local_mtime_ms, drive_mtime_ms, last_synced, None, None)
    }

    #[test]
    fn arquivo_so_local_sobe() {
        assert_eq!(decide_t(Some(T), None, None), SyncAction::Upload);
    }

    #[test]
    fn arquivo_so_no_drive_desce() {
        assert_eq!(decide_t(None, Some(T), None), SyncAction::Download);
    }

    #[test]
    fn inexistente_dos_dois_lados_e_noop() {
        assert_eq!(decide_t(None, None, None), SyncAction::NoOp);
    }

    #[test]
    fn timestamps_iguais_sao_noop() {
        assert_eq!(decide_t(Some(T), Some(T), None), SyncAction::NoOp);
    }

    #[test]
    fn diferenca_dentro_da_tolerancia_e_noop() {
        assert_eq!(
            decide_t(Some(T + TIMESTAMP_TOLERANCE_MS), Some(T), None),
            SyncAction::NoOp
        );
        assert_eq!(
            decide_t(Some(T), Some(T + TIMESTAMP_TOLERANCE_MS), None),
            SyncAction::NoOp
        );
    }

    #[test]
    fn primeiro_sync_drive_vence_mesmo_com_local_mais_recente() {
        // Sem manifest e ambos existem, origem desconhecida: Drive vence (com
        // backup), mesmo que o mtime local seja mais novo.
        assert_eq!(
            decide_t(Some(T + 60_000), Some(T), None),
            SyncAction::DownloadWithBackup
        );
    }

    #[test]
    fn primeiro_sync_drive_vence_com_drive_mais_recente() {
        assert_eq!(
            decide_t(Some(T), Some(T + 60_000), None),
            SyncAction::DownloadWithBackup
        );
    }

    #[test]
    fn primeiro_sync_de_outro_dispositivo_vira_conflito() {
        // Bug dos 3 dispositivos: sem manifest, ambos existem e divergem, e a
        // versão do Drive foi publicada por OUTRO dispositivo → conflito (o
        // usuário decide), em vez de o Drive vencer cegamente.
        assert_eq!(
            decide(
                Some(T + 60_000),
                Some(T),
                None,
                Some("dev-A"),
                Some("dev-C")
            ),
            SyncAction::Conflict
        );
        // Vale também com o Drive mais recente: divergência + origem distinta.
        assert_eq!(
            decide(
                Some(T),
                Some(T + 60_000),
                None,
                Some("dev-A"),
                Some("dev-C")
            ),
            SyncAction::Conflict
        );
    }

    #[test]
    fn primeiro_sync_do_mesmo_dispositivo_mantem_drive_vence() {
        // Mesmo ID dos dois lados (ex.: reinstalação que perdeu o manifest):
        // não é conflito entre dispositivos — Drive vence com backup.
        assert_eq!(
            decide(
                Some(T + 60_000),
                Some(T),
                None,
                Some("dev-C"),
                Some("dev-C")
            ),
            SyncAction::DownloadWithBackup
        );
    }

    #[test]
    fn primeiro_sync_com_origem_desconhecida_mantem_drive_vence() {
        // Arquivo do Drive sem ID (app antigo) ou keyring local indisponível:
        // na dúvida, comportamento conservador de Drive-vence.
        assert_eq!(
            decide(Some(T + 60_000), Some(T), None, None, Some("dev-C")),
            SyncAction::DownloadWithBackup
        );
        assert_eq!(
            decide(Some(T + 60_000), Some(T), None, Some("dev-A"), None),
            SyncAction::DownloadWithBackup
        );
    }

    #[test]
    fn primeiro_sync_de_outro_dispositivo_mas_mtime_igual_e_noop() {
        // Mesmo conteúdo (mtime dentro da tolerância): nada a fazer, ainda que
        // a origem do Drive seja outra.
        assert_eq!(
            decide(Some(T), Some(T), None, Some("dev-A"), Some("dev-C")),
            SyncAction::NoOp
        );
    }

    #[test]
    fn com_manifest_origem_diferente_nao_vira_conflito() {
        // Caminho com manifest: o Drive não mudou desde o último sync e só o
        // local mudou → Upload (avanço linear seguro), MESMO que a versão do
        // Drive tenha sido publicada por outro dispositivo.
        let drive = T;
        assert_eq!(
            decide(
                Some(T + 120_000),
                Some(drive),
                Some((T, drive)),
                Some("dev-A"),
                Some("dev-C"),
            ),
            SyncAction::Upload
        );
    }

    #[test]
    fn sem_mudanca_desde_o_ultimo_sync_e_noop_mesmo_com_relogio_divergente() {
        // Local e Drive diferem em 1 min (skew), mas ambos estão idênticos ao
        // que o manifest registrou — nada a fazer.
        let local = T;
        let drive = T + 60_000;
        assert_eq!(
            decide_t(Some(local), Some(drive), Some((local, drive))),
            SyncAction::NoOp
        );
    }

    #[test]
    fn mudanca_local_desde_o_ultimo_sync_sobe() {
        let drive = T;
        let novo_local = T + 120_000;
        assert_eq!(
            decide_t(Some(novo_local), Some(drive), Some((T, drive))),
            SyncAction::Upload
        );
    }

    #[test]
    fn mudanca_no_drive_desde_o_ultimo_sync_desce() {
        let local = T;
        let novo_drive = T + 120_000;
        assert_eq!(
            decide_t(Some(local), Some(novo_drive), Some((local, T))),
            SyncAction::Download
        );
    }

    #[test]
    fn conflito_real_ambos_mudaram_vira_conflito() {
        // Mudou dos dois lados desde o último sync: ninguém vence — Conflict.
        let last = (T, T);
        assert_eq!(
            decide_t(Some(T + 300_000), Some(T + 60_000), Some(last)),
            SyncAction::Conflict
        );
        assert_eq!(
            decide_t(Some(T + 60_000), Some(T + 300_000), Some(last)),
            SyncAction::Conflict
        );
    }

    #[test]
    fn ambos_mudaram_mas_com_mesmo_mtime_e_noop() {
        let last = (T, T);
        assert_eq!(
            decide_t(Some(T + 300_000), Some(T + 300_000), Some(last)),
            SyncAction::NoOp
        );
    }
}
