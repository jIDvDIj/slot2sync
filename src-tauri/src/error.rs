//! Tipo de erro unificado do backend. Comandos Tauri retornam `AppResult<T>`;
//! o erro cruza a boundary serializado como `{ code, message }` (ver
//! `AppErrorPayload` em `src/types/ipc.ts`).

use serde::ser::SerializeStruct;
use serde::{Serialize, Serializer};

pub type AppResult<T> = Result<T, AppError>;

#[derive(Debug, thiserror::Error)]
#[allow(dead_code)]
pub enum AppError {
    #[error("erro de IO: {0}")]
    Io(#[from] std::io::Error),

    #[error("erro de banco de dados: {0}")]
    Database(#[from] rusqlite::Error),

    #[error("erro de rede: {0}")]
    Network(#[from] reqwest::Error),

    // No mobile o keyring do SO não está disponível; os segredos ficam no
    // SQLite privado do app (ver `secrets::SqliteSecretStore`).
    #[cfg(desktop)]
    #[error("erro no cofre de credenciais: {0}")]
    Keyring(#[from] keyring::Error),

    #[error("erro de serialização: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("erro de autenticação: {0}")]
    Auth(String),

    #[error("emulador não reconhecido em: {0}")]
    EmulatorNotDetected(String),

    #[error("já existe um emulador com este nome: {0}")]
    EmulatorExists(String),

    #[error("arquivo em uso (modificado durante a leitura): {0}")]
    FileBusy(String),

    #[error("objeto não encontrado no provedor remoto: {0}")]
    RemoteObjectNotFound(String),

    #[error(
        "espaço em disco insuficiente: necessário {needed_mb} MB, disponível {available_mb} MB"
    )]
    InsufficientDiskSpace { needed_mb: u64, available_mb: u64 },

    #[error("falha de integridade na transferência: {0}")]
    Integrity(String),

    #[error("pasta não encontrada: {0} — dispositivo desconectado ou pasta removida?")]
    FolderNotMounted(String),

    /// Windows apenas: dois arquivos com nomes diferindo só em
    /// maiúsculas/minúsculas colidiriam no mesmo destino (NTFS é
    /// case-insensitive). Ver `SyncEngine::check_case_collision`.
    #[error(
        "colisão de maiúsculas/minúsculas: já existe \"{existing}\" (baixando \"{incoming}\")"
    )]
    CaseConflict { existing: String, incoming: String },

    #[error("{0}")]
    Other(String),
}

impl AppError {
    fn code(&self) -> &'static str {
        match self {
            AppError::Io(_) => "io",
            AppError::Database(_) => "database",
            AppError::Network(_) => "network",
            #[cfg(desktop)]
            AppError::Keyring(_) => "keyring",
            AppError::Serialization(_) => "serialization",
            AppError::Auth(_) => "auth",
            AppError::EmulatorNotDetected(_) => "emulator_not_detected",
            AppError::EmulatorExists(_) => "emulator_exists",
            AppError::FileBusy(_) => "file_busy",
            AppError::RemoteObjectNotFound(_) => "remote_not_found",
            AppError::InsufficientDiskSpace { .. } => "insufficient_disk_space",
            AppError::Integrity(_) => "integrity",
            AppError::FolderNotMounted(_) => "folder_not_mounted",
            AppError::CaseConflict { .. } => "case_conflict",
            AppError::Other(_) => "other",
        }
    }

    /// Detalhe técnico do erro (caminho, nome, mensagem da lib subjacente), sem
    /// o prefixo em português. O frontend localiza o prefixo pelo `code` e anexa
    /// este detalhe. `Other` não tem prefixo — todo o texto vem aqui.
    fn detail(&self) -> String {
        match self {
            AppError::Io(e) => e.to_string(),
            AppError::Database(e) => e.to_string(),
            AppError::Network(e) => e.to_string(),
            #[cfg(desktop)]
            AppError::Keyring(e) => e.to_string(),
            AppError::Serialization(e) => e.to_string(),
            AppError::InsufficientDiskSpace {
                needed_mb,
                available_mb,
            } => format!("necessário {needed_mb} MB, disponível {available_mb} MB"),
            AppError::CaseConflict { existing, incoming } => {
                format!("existente: {existing}, chegando: {incoming}")
            }
            AppError::Auth(s)
            | AppError::EmulatorNotDetected(s)
            | AppError::EmulatorExists(s)
            | AppError::FileBusy(s)
            | AppError::RemoteObjectNotFound(s)
            | AppError::Integrity(s)
            | AppError::FolderNotMounted(s)
            | AppError::Other(s) => s.clone(),
        }
    }
}

impl Serialize for AppError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut s = serializer.serialize_struct("AppError", 3)?;
        s.serialize_field("code", self.code())?;
        s.serialize_field("message", &self.to_string())?;
        s.serialize_field("detail", &self.detail())?;
        s.end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload(err: AppError) -> serde_json::Value {
        serde_json::to_value(err).unwrap()
    }

    #[test]
    fn erro_serializa_como_code_message_detail() {
        let v = payload(AppError::FileBusy("save.bin".into()));
        assert_eq!(v["code"], "file_busy");
        assert_eq!(v["detail"], "save.bin");
        // message = prefixo localizável + detalhe.
        assert!(v["message"].as_str().unwrap().contains("arquivo em uso"));
        assert!(v["message"].as_str().unwrap().contains("save.bin"));
    }

    #[test]
    fn codes_sao_estaveis_para_o_frontend() {
        // O union AppErrorPayload["code"] em src/types/ipc.ts depende destes valores.
        assert_eq!(payload(AppError::Auth("x".into()))["code"], "auth");
        assert_eq!(
            payload(AppError::RemoteObjectNotFound("x".into()))["code"],
            "remote_not_found"
        );
        assert_eq!(
            payload(AppError::EmulatorNotDetected("x".into()))["code"],
            "emulator_not_detected"
        );
        assert_eq!(
            payload(AppError::EmulatorExists("x".into()))["code"],
            "emulator_exists"
        );
        assert_eq!(payload(AppError::Other("x".into()))["code"], "other");
        assert_eq!(
            payload(AppError::InsufficientDiskSpace {
                needed_mb: 12,
                available_mb: 3
            })["code"],
            "insufficient_disk_space"
        );
    }

    #[test]
    fn other_usa_o_texto_inteiro_como_message_e_detail() {
        let v = payload(AppError::Other("falha específica".into()));
        assert_eq!(v["message"], "falha específica");
        assert_eq!(v["detail"], "falha específica");
    }

    #[test]
    fn integrity_serializa_code_e_detail() {
        let v = payload(AppError::Integrity("checksum divergente".into()));
        assert_eq!(v["code"], "integrity");
        assert_eq!(v["detail"], "checksum divergente");
    }

    #[test]
    fn folder_not_mounted_serializa_code_e_detail() {
        let v = payload(AppError::FolderNotMounted("/media/usb/PPSSPP".into()));
        assert_eq!(v["code"], "folder_not_mounted");
        assert_eq!(v["detail"], "/media/usb/PPSSPP");
    }

    #[test]
    fn case_conflict_serializa_code_e_detail() {
        let v = payload(AppError::CaseConflict {
            existing: "Save.bin".into(),
            incoming: "save.bin".into(),
        });
        assert_eq!(v["code"], "case_conflict");
        assert_eq!(v["detail"], "existente: Save.bin, chegando: save.bin");
    }

    #[test]
    fn io_serializa_code_e_detalhe_da_lib_subjacente() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "arquivo sumiu");
        let v = payload(AppError::from(io_err));
        assert_eq!(v["code"], "io");
        assert!(v["detail"].as_str().unwrap().contains("arquivo sumiu"));
    }

    #[test]
    fn database_serializa_code_e_detalhe_da_lib_subjacente() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        let db_err = conn
            .execute("SELECT * FROM tabela_inexistente", [])
            .unwrap_err();
        let v = payload(AppError::from(db_err));
        assert_eq!(v["code"], "database");
        assert!(!v["detail"].as_str().unwrap().is_empty());
    }

    #[test]
    fn serialization_serializa_code_e_detalhe_da_lib_subjacente() {
        let json_err = serde_json::from_str::<serde_json::Value>("não é json").unwrap_err();
        let v = payload(AppError::from(json_err));
        assert_eq!(v["code"], "serialization");
        assert!(!v["detail"].as_str().unwrap().is_empty());
    }

    #[tokio::test]
    async fn network_serializa_code_e_detalhe_da_lib_subjacente() {
        // Porta 1 em loopback: conexão recusada de forma rápida e determinística.
        let net_err = reqwest::Client::new()
            .get("http://127.0.0.1:1")
            .send()
            .await
            .unwrap_err();
        let v = payload(AppError::from(net_err));
        assert_eq!(v["code"], "network");
        assert!(!v["detail"].as_str().unwrap().is_empty());
    }
}
