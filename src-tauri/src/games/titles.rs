//! Tradução `serial → nome` a partir de um asset gerado.
//!
//! O arquivo `assets/game-titles.tsv` é gerado por `scripts/build-game-titles.mjs`
//! a partir do libretro-database (CC BY-SA 4.0) e embutido no binário: a busca
//! precisa funcionar offline, inclusive no mobile.

use std::collections::HashMap;
use std::sync::OnceLock;

const TITLES_TSV: &str = include_str!("../../assets/game-titles.tsv");

fn table() -> &'static HashMap<&'static str, &'static str> {
    static TABLE: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    TABLE.get_or_init(|| {
        TITLES_TSV
            .lines()
            .filter(|line| !line.starts_with('#'))
            .filter_map(|line| line.split_once('\t'))
            .collect()
    })
}

/// `key` já normalizada: só alfanuméricos, em maiúsculas.
pub fn lookup(key: &str) -> Option<&'static str> {
    table().get(key).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tabela_cobre_seriais_de_sistemas_diferentes() {
        assert_eq!(lookup("ULUS10391"), Some("Monster Hunter Freedom Unite"));
        assert_eq!(lookup("SCUS97472"), Some("Shadow of the Colossus"));
        assert_eq!(lookup("ULUS99999"), None);
    }

    #[test]
    fn linhas_de_comentario_nao_entram_na_tabela() {
        assert!(table().keys().all(|key| !key.starts_with('#')));
        assert!(table().len() > 30_000);
    }
}
