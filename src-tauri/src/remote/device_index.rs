//! Índice de dispositivo compartilhado por provedores sem um equivalente
//! nativo a `appProperties` arbitrárias por arquivo (Dropbox, OneDrive, pasta
//! local/rede). O Google Drive não usa isto — ele já tem `appProperties`.
//!
//! Cada pasta de categoria (`<raiz>/<emulador>/<saves|savestates|config>`)
//! ganha um único arquivo `INDEX_FILE_NAME` com um mapa `rel_path →
//! DeviceEntry`, atualizado a cada upload/rename. Cada implementação de
//! `RemoteProvider` é dona de COMO ler/escrever esse arquivo (via download/
//! upload da própria API, no caso do Dropbox/OneDrive; via `std::fs` direto,
//! no caso da pasta local) — este módulo só cuida do formato dos dados.
//!
//! `list_tree` de cada provedor que usa este índice deve excluir
//! `INDEX_FILE_NAME` do resultado: é bookkeeping interno, não um save do
//! usuário.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

pub const INDEX_FILE_NAME: &str = ".slot2sync-index.json";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DeviceEntry {
    pub device_name: Option<String>,
    pub device_id: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DeviceIndex(HashMap<String, DeviceEntry>);

impl DeviceIndex {
    /// JSON malformado ou ausente vira índice vazio — degrada para "sem
    /// atribuição de dispositivo conhecida", nunca falha o sync.
    pub fn parse(bytes: &[u8]) -> Self {
        serde_json::from_slice(bytes).unwrap_or_default()
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        serde_json::to_vec_pretty(self).unwrap_or_default()
    }

    pub fn get(&self, rel_path: &str) -> Option<&DeviceEntry> {
        self.0.get(rel_path)
    }

    pub fn set(&mut self, rel_path: &str, entry: DeviceEntry) {
        self.0.insert(rel_path.to_string(), entry);
    }

    /// Move a entrada de `old_rel_path` para `new_rel_path` (usado por
    /// `rename_file`). Sem efeito se `old_rel_path` não tinha entrada.
    pub fn rename(&mut self, old_rel_path: &str, new_rel_path: &str) {
        if let Some(entry) = self.0.remove(old_rel_path) {
            self.0.insert(new_rel_path.to_string(), entry);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_bytes_invalidos_vira_indice_vazio() {
        let index = DeviceIndex::parse(b"nao e json");
        assert!(index.get("save.bin").is_none());
    }

    #[test]
    fn set_e_get_fazem_roundtrip_via_bytes() {
        let mut index = DeviceIndex::default();
        index.set(
            "save.bin",
            DeviceEntry {
                device_name: Some("PC Gamer".into()),
                device_id: Some("dev-1".into()),
            },
        );
        let bytes = index.to_bytes();
        let reparsed = DeviceIndex::parse(&bytes);
        assert_eq!(
            reparsed.get("save.bin").unwrap().device_name.as_deref(),
            Some("PC Gamer")
        );
    }

    #[test]
    fn rename_move_a_entrada_existente() {
        let mut index = DeviceIndex::default();
        index.set(
            "antigo.bin",
            DeviceEntry {
                device_name: Some("Notebook".into()),
                device_id: None,
            },
        );
        index.rename("antigo.bin", "novo.bin");
        assert!(index.get("antigo.bin").is_none());
        assert_eq!(
            index.get("novo.bin").unwrap().device_name.as_deref(),
            Some("Notebook")
        );
    }

    #[test]
    fn rename_sem_entrada_existente_nao_faz_nada() {
        let mut index = DeviceIndex::default();
        index.rename("nao-existe.bin", "novo.bin");
        assert!(index.get("novo.bin").is_none());
    }
}
