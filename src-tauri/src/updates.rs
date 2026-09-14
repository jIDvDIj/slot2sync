//! Atualização automática pelo `tauri-plugin-updater`.
//!
//! O manifesto vem do asset `latest.json` da release mais recente no GitHub e
//! cada artefato é assinado em Ed25519 no build; a chave pública embutida no
//! binário é o que impede um manifesto forjado de instalar qualquer coisa.
//!
//! Enquanto a chave não estiver configurada em `tauri.conf.json`, a checagem
//! falha e o app segue normalmente — atualizar é conveniência, não requisito
//! de funcionamento.

use serde::Serialize;

/// Versão nova encontrada. (→ ipc.ts)
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub version: String,
    pub notes: Option<String>,
    /// Data de publicação declarada no manifesto, como veio.
    pub date: Option<String>,
}
