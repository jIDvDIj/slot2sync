//! Leitura e streaming do log da aplicação para a UI de diagnóstico.
//!
//! Duas fontes alimentam a mesma [`LogEntry`]: o arquivo rotacionado pelo
//! `tracing-appender` (histórico, via [`tail`]) e um layer de `tracing` que
//! publica no barramento interno (tempo real, via [`LogBusLayer`]). O layer só
//! publica enquanto [`set_streaming`] estiver ligado — sem a janela de
//! diagnóstico aberta, todo evento de log viraria uma mensagem de broadcast
//! descartada.

use std::fmt::Write as _;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::Serialize;
use tracing::field::{Field, Visit};
use tracing_subscriber::layer::Context;
use tracing_subscriber::Layer;

use crate::error::AppResult;
use crate::events::bus::{AppEvent, EventBus};

/// Teto de linhas devolvidas por [`tail`], independente do que a UI pedir.
pub const MAX_TAIL_LINES: usize = 5_000;

/// Alvo dos avisos da ponte de eventos. O layer ignora este alvo: a ponte
/// avisa quando fica para trás no barramento, e publicar esse aviso no próprio
/// barramento a atrasaria ainda mais, realimentando o aviso.
pub const EVENT_BRIDGE_TARGET: &str = "slot2sync::event_bridge";

static STREAMING: AtomicBool = AtomicBool::new(false);

/// Liga/desliga a publicação de [`AppEvent::LogEntry`] no barramento.
pub fn set_streaming(enabled: bool) {
    STREAMING.store(enabled, Ordering::Relaxed);
}

/// Uma linha de log exposta à UI. (→ ipc.ts)
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogEntry {
    /// ISO-8601 como o `tracing` escreve; string opaca para a UI.
    pub timestamp: String,
    /// `TRACE` | `DEBUG` | `INFO` | `WARN` | `ERROR`.
    pub level: String,
    pub target: String,
    pub message: String,
}

/// Últimas `limit` linhas do log mais recente de `log_dir`, mais antigas
/// primeiro. Diretório vazio ou linhas fora do formato do `tracing` não são
/// erro: viram lista vazia e linhas puladas, respectivamente.
pub fn tail(log_dir: &Path, limit: usize) -> AppResult<Vec<LogEntry>> {
    let Some(newest) = newest_log_file(log_dir) else {
        return Ok(Vec::new());
    };
    let content = std::fs::read_to_string(newest)?;
    let limit = limit.min(MAX_TAIL_LINES);

    let mut entries: Vec<LogEntry> = content.lines().filter_map(parse_line).collect();
    if entries.len() > limit {
        entries.drain(..entries.len() - limit);
    }
    Ok(entries)
}

/// O `tracing-appender` roda diariamente com sufixo `.YYYY-MM-DD`, que ordena
/// lexicograficamente igual à data.
fn newest_log_file(log_dir: &Path) -> Option<std::path::PathBuf> {
    std::fs::read_dir(log_dir)
        .ok()?
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("slot2sync.log"))
        })
        .max()
}

/// Formato do `fmt::layer()` sem ANSI:
/// `2025-07-01T10:30:00.123456Z  INFO slot2sync::sync: mensagem campo=valor`.
/// Continuações de eventos multilinha não casam e são descartadas.
fn parse_line(line: &str) -> Option<LogEntry> {
    let (timestamp, rest) = line.split_once(' ')?;
    if !timestamp.ends_with('Z') {
        return None;
    }
    let rest = rest.trim_start();
    let (level, rest) = rest.split_once(' ')?;
    if !matches!(level, "TRACE" | "DEBUG" | "INFO" | "WARN" | "ERROR") {
        return None;
    }
    let (target, message) = rest.trim_start().split_once(": ")?;

    Some(LogEntry {
        timestamp: timestamp.to_string(),
        level: level.to_string(),
        target: target.to_string(),
        message: message.to_string(),
    })
}

/// Layer que espelha cada evento de `tracing` no barramento interno enquanto o
/// streaming estiver ligado.
pub struct LogBusLayer {
    bus: EventBus,
}

impl LogBusLayer {
    pub fn new(bus: EventBus) -> Self {
        Self { bus }
    }
}

impl<S: tracing::Subscriber> Layer<S> for LogBusLayer {
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        if !STREAMING.load(Ordering::Relaxed) {
            return;
        }
        let metadata = event.metadata();
        if metadata.target() == EVENT_BRIDGE_TARGET {
            return;
        }
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);

        self.bus.publish(AppEvent::LogEntry(LogEntry {
            timestamp: chrono::Utc::now().to_rfc3339(),
            level: metadata.level().to_string(),
            target: metadata.target().to_string(),
            message: visitor.into_message(),
        }));
    }
}

/// Junta o campo `message` com os demais campos estruturados, na mesma ordem
/// em que o `fmt` os escreveria no arquivo.
#[derive(Default)]
struct MessageVisitor {
    message: String,
    fields: String,
}

impl MessageVisitor {
    fn into_message(self) -> String {
        match (self.message.is_empty(), self.fields.is_empty()) {
            (true, _) => self.fields,
            (false, true) => self.message,
            (false, false) => format!("{} {}", self.message, self.fields),
        }
    }
}

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            let _ = write!(self.message, "{value:?}");
            return;
        }
        if !self.fields.is_empty() {
            self.fields.push(' ');
        }
        let _ = write!(self.fields, "{}={:?}", field.name(), value);
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message.push_str(value);
            return;
        }
        if !self.fields.is_empty() {
            self.fields.push(' ');
        }
        let _ = write!(self.fields, "{}={}", field.name(), value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_line_extrai_carimbo_nivel_alvo_e_mensagem() {
        let entry = parse_line(
            "2025-07-01T10:30:00.123456Z  INFO slot2sync::sync: sync iniciado emuladores=2",
        )
        .unwrap();

        assert_eq!(entry.timestamp, "2025-07-01T10:30:00.123456Z");
        assert_eq!(entry.level, "INFO");
        assert_eq!(entry.target, "slot2sync::sync");
        assert_eq!(entry.message, "sync iniciado emuladores=2");
    }

    #[test]
    fn parse_line_descarta_linha_fora_do_formato() {
        assert!(parse_line("").is_none());
        assert!(parse_line("continuação de um evento multilinha").is_none());
        assert!(parse_line("2025-07-01T10:30:00Z  FATAL x: y").is_none());
        assert!(parse_line("sem-carimbo INFO slot2sync: mensagem").is_none());
    }

    #[test]
    fn tail_devolve_as_ultimas_linhas_do_arquivo_mais_recente() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("slot2sync.log.2025-07-01"),
            "2025-07-01T10:00:00Z  INFO a: antigo\n",
        )
        .unwrap();
        let recente: String = (1..=5)
            .map(|i| format!("2025-07-02T10:00:0{i}Z  INFO a: linha {i}\n"))
            .collect();
        std::fs::write(tmp.path().join("slot2sync.log.2025-07-02"), recente).unwrap();

        let entries = tail(tmp.path(), 2).unwrap();

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].message, "linha 4");
        assert_eq!(entries[1].message, "linha 5");
    }

    #[test]
    fn tail_sem_pasta_de_log_devolve_vazio() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(tail(&tmp.path().join("nao-existe"), 100)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn layer_ignora_o_alvo_da_ponte_de_eventos() {
        use tracing_subscriber::layer::SubscriberExt;

        let bus = EventBus::new();
        let mut rx = bus.subscribe();
        let subscriber = tracing_subscriber::registry().with(LogBusLayer::new(bus));

        set_streaming(true);
        tracing::subscriber::with_default(subscriber, || {
            tracing::warn!(target: EVENT_BRIDGE_TARGET, "ponte atrasada");
            tracing::info!(target: "slot2sync::sync", "sync iniciado");
        });
        set_streaming(false);

        let AppEvent::LogEntry(entry) = rx.try_recv().unwrap() else {
            panic!("esperava uma linha de log no barramento");
        };
        assert_eq!(entry.message, "sync iniciado");
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn into_message_junta_mensagem_e_campos_estruturados() {
        let com_ambos = MessageVisitor {
            message: "sync iniciado".into(),
            fields: "trigger=manual".into(),
        };
        assert_eq!(com_ambos.into_message(), "sync iniciado trigger=manual");

        let so_campos = MessageVisitor {
            message: String::new(),
            fields: "trigger=manual".into(),
        };
        assert_eq!(so_campos.into_message(), "trigger=manual");

        let so_mensagem = MessageVisitor {
            message: "sync iniciado".into(),
            fields: String::new(),
        };
        assert_eq!(so_mensagem.into_message(), "sync iniciado");
    }
}
