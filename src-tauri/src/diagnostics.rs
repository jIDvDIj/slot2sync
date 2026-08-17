//! Export de diagnóstico: empacota o estado local relevante (configurações,
//! manifest, conflitos, fila offline, informações do app e o final do log
//! ativo) num único `.zip` para o usuário anexar a um relato de bug.
//!
//! Todo o trabalho pesado (montar o zip) é síncrono — roda em
//! `spawn_blocking`, chamado a partir de `commands::export_diagnostics`.

use std::io::Write;
use std::path::Path;

use zip::write::SimpleFileOptions;
use zip::ZipWriter;

use crate::error::AppResult;
use crate::storage::conflicts::Conflict;
use crate::storage::manifest::ManifestEntry;
use crate::storage::queue::PendingOp;
use crate::storage::settings::Settings;

/// Quantidade de linhas finais do log ativo incluídas no export.
const LOG_TAIL_LINES: usize = 1000;

/// Chaves cujo valor é substituído por `"REDACTED"` no `settings.json`, caso
/// alguma configuração futura acabe carregando algo sensível — hoje nenhum
/// campo de `Settings` guarda token/segredo (isso mora no keyring do SO,
/// nunca em `app_settings`), mas a rede de segurança é barata de manter.
const REDACT_KEY_PATTERNS: &[&str] = &["token", "secret", "password"];

fn redact_secrets(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, v) in map.iter_mut() {
                let key_lower = key.to_lowercase();
                if REDACT_KEY_PATTERNS.iter().any(|p| key_lower.contains(p)) {
                    *v = serde_json::Value::String("REDACTED".into());
                } else {
                    redact_secrets(v);
                }
            }
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(redact_secrets),
        _ => {}
    }
}

/// Últimas `LOG_TAIL_LINES` linhas do arquivo de log de hoje
/// (`tracing_appender::rolling::daily`), ou uma mensagem explicando a
/// ausência — nunca falha o export inteiro por causa do log.
fn read_log_tail(log_dir: &Path) -> String {
    let today = chrono::Local::now().format("%Y-%m-%d");
    let log_path = log_dir.join(format!("slot2sync.log.{today}"));
    match std::fs::read_to_string(&log_path) {
        Ok(content) => {
            let lines: Vec<&str> = content.lines().collect();
            let start = lines.len().saturating_sub(LOG_TAIL_LINES);
            lines[start..].join("\n")
        }
        Err(err) => format!(
            "(log de hoje indisponível em {}: {err})",
            log_path.display()
        ),
    }
}

fn write_json_entry<T: serde::Serialize>(
    zip: &mut ZipWriter<std::fs::File>,
    name: &str,
    value: &T,
    options: SimpleFileOptions,
) -> AppResult<()> {
    let json = serde_json::to_vec_pretty(value)?;
    zip.start_file(name, options)?;
    zip.write_all(&json)?;
    Ok(())
}

/// Monta o `.zip` em `dest`. Síncrono — chamar a partir de `spawn_blocking`.
#[allow(clippy::too_many_arguments)]
pub fn write_zip(
    dest: &Path,
    settings: &Settings,
    manifest: &[ManifestEntry],
    conflicts: &[Conflict],
    pending_ops: &[PendingOp],
    version: &str,
    log_dir: &Path,
) -> AppResult<()> {
    let file = std::fs::File::create(dest)?;
    let mut zip = ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    let mut settings_json = serde_json::to_value(settings)?;
    redact_secrets(&mut settings_json);
    zip.start_file("settings.json", options)?;
    zip.write_all(serde_json::to_string_pretty(&settings_json)?.as_bytes())?;

    write_json_entry(&mut zip, "sync_manifest.json", &manifest, options)?;
    write_json_entry(&mut zip, "conflicts.json", &conflicts, options)?;
    write_json_entry(&mut zip, "pending_ops.json", &pending_ops, options)?;

    let app_info = serde_json::json!({
        "version": version,
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
    });
    write_json_entry(&mut zip, "app_info.json", &app_info, options)?;

    zip.start_file("log_tail.txt", options)?;
    zip.write_all(read_log_tail(log_dir).as_bytes())?;

    zip.finish()?;
    Ok(())
}

/// Nome de arquivo do export, com carimbo de data/hora (evita sobrescrever
/// um export anterior no mesmo dia).
pub fn file_name() -> String {
    let stamp = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S");
    format!("slot2sync-diagnostics-{stamp}.zip")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redact_secrets_substitui_chaves_sensiveis_recursivamente() {
        let mut value = serde_json::json!({
            "deviceName": "PC Gamer",
            "auth": { "accessToken": "abc123", "refreshToken": "def456" },
            "clientSecret": "shh",
            "nested": [{ "password": "1234", "keep": "isto fica" }],
        });

        redact_secrets(&mut value);

        assert_eq!(value["deviceName"], "PC Gamer");
        assert_eq!(value["auth"]["accessToken"], "REDACTED");
        assert_eq!(value["auth"]["refreshToken"], "REDACTED");
        assert_eq!(value["clientSecret"], "REDACTED");
        assert_eq!(value["nested"][0]["password"], "REDACTED");
        assert_eq!(value["nested"][0]["keep"], "isto fica");
    }

    #[test]
    fn read_log_tail_sem_arquivo_de_hoje_nao_falha() {
        let tmp = tempfile::tempdir().unwrap();
        let tail = read_log_tail(tmp.path());
        assert!(tail.contains("indisponível"));
    }

    #[test]
    fn read_log_tail_corta_para_as_ultimas_linhas() {
        let tmp = tempfile::tempdir().unwrap();
        let today = chrono::Local::now().format("%Y-%m-%d");
        let log_path = tmp.path().join(format!("slot2sync.log.{today}"));
        let content: String = (0..LOG_TAIL_LINES + 50)
            .map(|i| format!("linha {i}\n"))
            .collect();
        std::fs::write(&log_path, content).unwrap();

        let tail = read_log_tail(tmp.path());

        let lines: Vec<&str> = tail.lines().collect();
        assert_eq!(lines.len(), LOG_TAIL_LINES);
        assert_eq!(lines[0], "linha 50");
        assert_eq!(
            lines[LOG_TAIL_LINES - 1],
            format!("linha {}", LOG_TAIL_LINES + 49)
        );
    }

    #[test]
    fn write_zip_produz_arquivo_com_todas_as_entradas() {
        let tmp = tempfile::tempdir().unwrap();
        let dest = tmp.path().join("out.zip");

        write_zip(
            &dest,
            &Settings::default(),
            &[],
            &[],
            &[],
            "1.2.3",
            tmp.path(),
        )
        .unwrap();

        let file = std::fs::File::open(&dest).unwrap();
        let mut archive = zip::ZipArchive::new(file).unwrap();
        let mut names: Vec<String> = (0..archive.len())
            .map(|i| archive.by_index(i).unwrap().name().to_string())
            .collect();
        names.sort();
        assert_eq!(
            names,
            vec![
                "app_info.json",
                "conflicts.json",
                "log_tail.txt",
                "pending_ops.json",
                "settings.json",
                "sync_manifest.json",
            ]
        );
    }
}
