//! Identificação legível dos jogos sincronizados.
//!
//! O serial do jogo (`ULUS12345`, `SLUS-12345`) já vive no `rel_path` de cada
//! entrada do `sync_manifest` — é o primeiro componente do caminho nos saves do
//! PPSSPP e o prefixo do nome nos savestates. Este módulo:
//!
//! 1. extrai o serial do `rel_path` ([`serial_from_rel_path`]);
//! 2. agrega as entradas do manifest por `(emulador, serial)` ([`aggregate`]);
//! 3. traduz o serial para um nome legível quando conhecido ([`resolve_name`]).
//!
//! A tradução usa uma tabela embutida pequena (semente verificada). A cobertura
//! ampla e offline virá do empacotamento do OpenVGDB (asset SQLite). Sem
//! correspondência, a UI exibe o próprio serial.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::storage::manifest::ManifestEntry;
use crate::sync::SyncCategory;

/// Um jogo cujos arquivos foram sincronizados, agregado a partir do manifest. (→ ipc.ts)
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncedGame {
    /// Identificador técnico extraído do caminho (`ULUS12345`, `SLUS-12345`, ou
    /// o nome de arquivo quando não há serial — ex.: memory card do PCSX2).
    pub serial: String,
    /// Nome legível, quando o serial é conhecido; `None` cai no serial na UI.
    pub name: Option<String>,
    pub emulator: String,
    /// Categorias em que o jogo tem arquivos sincronizados.
    pub categories: Vec<SyncCategory>,
    /// Sync mais recente entre os arquivos do jogo.
    pub last_synced_at_ms: i64,
    /// Soma dos tamanhos dos arquivos sincronizados do jogo.
    pub size_bytes: i64,
}

/// Agrega as entradas do manifest em jogos por `(emulador, serial)`, somando
/// tamanho, unindo categorias e mantendo o sync mais recente. A ordem é estável
/// (emulador, depois serial) para uma UI determinística.
pub fn aggregate(entries: Vec<ManifestEntry>) -> Vec<SyncedGame> {
    #[derive(Default)]
    struct Acc {
        categories: Vec<SyncCategory>,
        size_bytes: i64,
        last_synced_at_ms: i64,
    }

    let mut games: BTreeMap<(String, String), Acc> = BTreeMap::new();
    for entry in entries {
        let Some(serial) = serial_from_rel_path(&entry.rel_path) else {
            continue;
        };
        let acc = games.entry((entry.emulator, serial)).or_default();
        if !acc.categories.contains(&entry.category) {
            acc.categories.push(entry.category);
        }
        acc.size_bytes += entry.size_bytes.unwrap_or(0);
        acc.last_synced_at_ms = acc.last_synced_at_ms.max(entry.last_synced_at_ms);
    }

    games
        .into_iter()
        .map(|((emulator, serial), acc)| SyncedGame {
            name: resolve_name(&serial).map(str::to_string),
            serial,
            emulator,
            categories: acc.categories,
            last_synced_at_ms: acc.last_synced_at_ms,
            size_bytes: acc.size_bytes,
        })
        .collect()
}

/// Extrai o identificador de jogo de um `rel_path` de manifest.
///
/// - Com subpasta por jogo (`ULUS12345/DATA.BIN`): o primeiro componente.
/// - Arquivo direto na categoria: o prefixo se parecer um serial
///   (`SLUS-12345.00.p2s` → `SLUS-12345`, `ULUS12345_1.00_0.ppst` → `ULUS12345`);
///   senão, o próprio nome do arquivo (ex.: `Mcd001.ps2`, sem granularidade por
///   jogo — comportamento documentado para memory cards do PCSX2).
pub fn serial_from_rel_path(rel_path: &str) -> Option<String> {
    let rel_path = rel_path.trim();
    if rel_path.is_empty() {
        return None;
    }
    if let Some((first, _)) = rel_path.split_once('/') {
        let first = first.trim();
        return (!first.is_empty()).then(|| first.to_string());
    }
    let token = rel_path
        .split(['.', '_', ' '])
        .next()
        .unwrap_or(rel_path)
        .trim();
    if looks_like_serial(token) {
        Some(token.to_string())
    } else {
        Some(rel_path.to_string())
    }
}

/// `true` se `s` tem a forma de um serial de console: 4 letras, hífen opcional e
/// ≥ 3 dígitos (`ULUS12345`, `SLUS-12345`, `UCUS98653`, `NPUG80086`).
fn looks_like_serial(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() < 7 || !b[..4].iter().all(u8::is_ascii_alphabetic) {
        return false;
    }
    let digits = if b.get(4) == Some(&b'-') {
        &b[5..]
    } else {
        &b[4..]
    };
    digits.len() >= 3 && digits.iter().all(u8::is_ascii_digit)
}

/// Nome legível de um serial, se conhecido. Normaliza (só alfanumérico,
/// maiúsculas) antes de consultar a tabela embutida.
pub fn resolve_name(serial: &str) -> Option<&'static str> {
    let key = normalize(serial);
    NAMES.iter().find(|(k, _)| *k == key).map(|(_, name)| *name)
}

fn normalize(serial: &str) -> String {
    serial
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

/// Semente pequena e verificada de `serial → nome` (chaves já normalizadas, sem
/// hífen). Substituível/ampliável pelo OpenVGDB no futuro.
static NAMES: &[(&str, &str)] = &[
    // PSP
    ("ULUS10041", "Grand Theft Auto: Liberty City Stories"),
    ("ULUS10160", "Grand Theft Auto: Vice City Stories"),
    ("UCUS98653", "God of War: Chains of Olympus"),
    ("UCUS98737", "God of War: Ghost of Sparta"),
    ("ULUS10336", "Crisis Core: Final Fantasy VII"),
    ("ULUS10391", "Monster Hunter Freedom Unite"),
    // PS2
    ("SCUS97472", "Shadow of the Colossus"),
    ("SCUS97399", "God of War"),
    ("SLUS20370", "Kingdom Hearts"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serial_de_subpasta_usa_primeiro_componente() {
        assert_eq!(
            serial_from_rel_path("ULUS12345/DATA.BIN").as_deref(),
            Some("ULUS12345")
        );
        assert_eq!(
            serial_from_rel_path("ULUS12345/sub/pasta/x.bin").as_deref(),
            Some("ULUS12345")
        );
    }

    #[test]
    fn serial_de_savestate_extrai_prefixo() {
        // PPSSPP: <SERIAL>_<slot>.ppst
        assert_eq!(
            serial_from_rel_path("ULUS12345_1.00_0.ppst").as_deref(),
            Some("ULUS12345")
        );
        // PCSX2: <SERIAL>.<slot>.p2s
        assert_eq!(
            serial_from_rel_path("SLUS-12345.00.p2s").as_deref(),
            Some("SLUS-12345")
        );
    }

    #[test]
    fn arquivo_sem_serial_cai_no_nome_do_arquivo() {
        // Memory card monolítico do PCSX2 — sem granularidade por jogo.
        assert_eq!(
            serial_from_rel_path("Mcd001.ps2").as_deref(),
            Some("Mcd001.ps2")
        );
        assert_eq!(serial_from_rel_path("").as_deref(), None);
    }

    #[test]
    fn looks_like_serial_reconhece_padroes() {
        assert!(looks_like_serial("ULUS12345"));
        assert!(looks_like_serial("SLUS-12345"));
        assert!(looks_like_serial("NPUG80086"));
        assert!(!looks_like_serial("Mcd001"));
        assert!(!looks_like_serial("SAVE"));
        assert!(!looks_like_serial("abc"));
    }

    #[test]
    fn resolve_name_normaliza_hifen_e_caixa() {
        assert_eq!(resolve_name("SCUS-97472"), Some("Shadow of the Colossus"));
        assert_eq!(resolve_name("scus97472"), Some("Shadow of the Colossus"));
        assert_eq!(resolve_name("ULUS99999"), None);
    }

    fn entry(
        emulator: &str,
        category: SyncCategory,
        rel: &str,
        size: i64,
        ts: i64,
    ) -> ManifestEntry {
        ManifestEntry {
            emulator: emulator.into(),
            category,
            rel_path: rel.into(),
            drive_file_id: Some("id".into()),
            local_mtime_ms: Some(ts),
            drive_mtime_ms: Some(ts),
            size_bytes: Some(size),
            last_synced_at_ms: ts,
            file_hash: None,
        }
    }

    #[test]
    fn aggregate_agrupa_por_jogo_soma_tamanho_e_une_categorias() {
        let entries = vec![
            entry("PPSSPP", SyncCategory::Saves, "ULUS12345/DATA.BIN", 100, 10),
            entry("PPSSPP", SyncCategory::Saves, "ULUS12345/ICON.PNG", 50, 20),
            entry(
                "PPSSPP",
                SyncCategory::Savestates,
                "ULUS12345_1.00_0.ppst",
                200,
                30,
            ),
            entry("PPSSPP", SyncCategory::Saves, "UCUS98653/DATA.BIN", 10, 5),
        ];
        let games = aggregate(entries);

        assert_eq!(games.len(), 2);
        // Ordem estável: UCUS98653 antes de ULUS12345.
        assert_eq!(games[0].serial, "UCUS98653");
        assert_eq!(
            games[0].name.as_deref(),
            Some("God of War: Chains of Olympus")
        );

        let g = &games[1];
        assert_eq!(g.serial, "ULUS12345");
        assert_eq!(g.name, None);
        assert_eq!(g.size_bytes, 350);
        assert_eq!(g.last_synced_at_ms, 30);
        assert!(g.categories.contains(&SyncCategory::Saves));
        assert!(g.categories.contains(&SyncCategory::Savestates));
    }

    #[test]
    fn synced_game_serializa_em_camel_case() {
        let entries = vec![entry(
            "PPSSPP",
            SyncCategory::Saves,
            "ULUS12345/DATA.BIN",
            100,
            10,
        )];
        let game = &aggregate(entries)[0];
        let json = serde_json::to_value(game).unwrap();
        assert_eq!(json["serial"], "ULUS12345");
        assert_eq!(json["lastSyncedAtMs"], 10);
        assert_eq!(json["sizeBytes"], 100);
        assert_eq!(json["categories"][0], "saves");
    }
}
