//! Fila do sync em andamento: o que ainda não começou e o que está em voo.
//!
//! Substitui o `buffer_unordered` do engine por uma `VecDeque` explícita
//! drenada por N workers. A diferença que importa é ser inspecionável e
//! reordenável de fora: a UI lista o que falta e pode pedir que um arquivo
//! específico passe à frente ([`SyncQueue::bring_to_front`]).
//!
//! Escopo: uma categoria de um emulador por vez, esvaziada ao fim de cada
//! rodada. Não sobrevive entre syncs nem substitui a fila offline
//! (`storage::queue`), que persiste as transferências que falharam.

use std::collections::VecDeque;
use std::sync::Mutex;

use serde::Serialize;

use super::conflict::SyncAction;
use super::diff::PlannedOp;
use super::SyncCategory;

/// Uma operação da fila, como a UI a enxerga. (→ ipc.ts)
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QueuedOp {
    pub emulator: String,
    pub category: SyncCategory,
    pub rel_path: String,
    /// `upload` | `download` | `download-with-backup` | `conflict` | `noop`.
    pub action: String,
    pub size_bytes: u64,
}

/// Retrato da fila para o frontend. (→ ipc.ts)
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncQueueSnapshot {
    pub in_progress: Vec<QueuedOp>,
    pub queued: Vec<QueuedOp>,
}

pub fn action_str(action: SyncAction) -> &'static str {
    match action {
        SyncAction::Upload => "upload",
        SyncAction::Download => "download",
        SyncAction::DownloadWithBackup => "download-with-backup",
        SyncAction::Conflict => "conflict",
        SyncAction::NoOp => "noop",
    }
}

#[derive(Default)]
struct State {
    emulator: String,
    category: Option<SyncCategory>,
    queued: VecDeque<PlannedOp>,
    in_progress: Vec<PlannedOp>,
}

#[derive(Default)]
pub struct SyncQueue {
    state: Mutex<State>,
}

impl SyncQueue {
    /// Carrega a fila para uma rodada. Descarta o que restou de uma anterior:
    /// só existe uma categoria em voo por vez.
    pub fn load(&self, emulator: &str, category: SyncCategory, ops: Vec<PlannedOp>) {
        let mut state = self.lock();
        state.emulator = emulator.to_string();
        state.category = Some(category);
        state.queued = ops.into();
        state.in_progress.clear();
    }

    /// Próxima operação, já contabilizada como em voo. `None` = fila vazia.
    pub fn take_next(&self) -> Option<PlannedOp> {
        let mut state = self.lock();
        let op = state.queued.pop_front()?;
        state.in_progress.push(op.clone());
        Some(op)
    }

    pub fn finish(&self, rel_path: &str) {
        let mut state = self.lock();
        if let Some(idx) = state
            .in_progress
            .iter()
            .position(|op| op.rel_path == rel_path)
        {
            state.in_progress.remove(idx);
        }
    }

    pub fn clear(&self) {
        let mut state = self.lock();
        *state = State::default();
    }

    /// Move um arquivo para a frente da fila. `false` = não está mais na fila
    /// (já transferido, ou já em voo — nesse caso não há o que antecipar).
    pub fn bring_to_front(&self, emulator: &str, rel_path: &str) -> bool {
        let mut state = self.lock();
        if state.emulator != emulator {
            return false;
        }
        let Some(idx) = state.queued.iter().position(|op| op.rel_path == rel_path) else {
            return false;
        };
        let Some(op) = state.queued.remove(idx) else {
            return false;
        };
        state.queued.push_front(op);
        true
    }

    pub fn snapshot(&self) -> SyncQueueSnapshot {
        let state = self.lock();
        let Some(category) = state.category else {
            return SyncQueueSnapshot::default();
        };
        let to_queued = |op: &PlannedOp| QueuedOp {
            emulator: state.emulator.clone(),
            category,
            rel_path: op.rel_path.clone(),
            action: action_str(op.action).to_string(),
            size_bytes: super::engine::op_bytes(op),
        };
        SyncQueueSnapshot {
            in_progress: state.in_progress.iter().map(to_queued).collect(),
            queued: state.queued.iter().map(to_queued).collect(),
        }
    }

    /// O lock só protege manipulações curtas de memória; nada faz `await`
    /// segurando ele. Envenenado, seguir com o conteúdo é melhor que derrubar
    /// o sync.
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::diff::LocalFile;
    use crate::sync::FileLoc;

    fn op(rel_path: &str, size: i64) -> PlannedOp {
        PlannedOp {
            rel_path: rel_path.to_string(),
            action: SyncAction::Upload,
            local: Some(LocalFile {
                rel_path: rel_path.to_string(),
                loc: FileLoc::from_path(std::path::PathBuf::from(rel_path)),
                mtime_ms: 0,
                mtime_ns: 0,
                size_bytes: size,
                hash: None,
            }),
            remote: None,
        }
    }

    fn loaded(paths: &[&str]) -> SyncQueue {
        let queue = SyncQueue::default();
        queue.load(
            "PPSSPP",
            SyncCategory::Saves,
            paths.iter().map(|p| op(p, 8)).collect(),
        );
        queue
    }

    #[test]
    fn take_next_drena_em_ordem_e_marca_em_voo() {
        let queue = loaded(&["a.sav", "b.sav"]);

        let first = queue.take_next().unwrap();

        assert_eq!(first.rel_path, "a.sav");
        let snapshot = queue.snapshot();
        assert_eq!(snapshot.in_progress.len(), 1);
        assert_eq!(snapshot.in_progress[0].rel_path, "a.sav");
        assert_eq!(snapshot.queued.len(), 1);
        assert_eq!(snapshot.queued[0].rel_path, "b.sav");
    }

    #[test]
    fn finish_tira_de_em_voo() {
        let queue = loaded(&["a.sav"]);
        queue.take_next().unwrap();

        queue.finish("a.sav");

        assert!(queue.snapshot().in_progress.is_empty());
        assert!(queue.take_next().is_none());
    }

    #[test]
    fn bring_to_front_antecipa_o_arquivo_pedido() {
        let queue = loaded(&["a.sav", "b.sav", "c.sav"]);

        assert!(queue.bring_to_front("PPSSPP", "c.sav"));

        assert_eq!(queue.take_next().unwrap().rel_path, "c.sav");
        assert_eq!(queue.take_next().unwrap().rel_path, "a.sav");
    }

    #[test]
    fn bring_to_front_ignora_emulador_e_arquivo_desconhecidos() {
        let queue = loaded(&["a.sav"]);

        assert!(!queue.bring_to_front("PCSX2", "a.sav"));
        assert!(!queue.bring_to_front("PPSSPP", "inexistente.sav"));
    }

    #[test]
    fn bring_to_front_nao_traz_de_volta_o_que_ja_esta_em_voo() {
        let queue = loaded(&["a.sav", "b.sav"]);
        queue.take_next().unwrap();

        assert!(!queue.bring_to_front("PPSSPP", "a.sav"));
    }

    #[test]
    fn fila_vazia_devolve_retrato_vazio() {
        let queue = SyncQueue::default();

        assert_eq!(queue.snapshot(), SyncQueueSnapshot::default());

        let queue = loaded(&["a.sav"]);
        queue.clear();
        assert_eq!(queue.snapshot(), SyncQueueSnapshot::default());
    }

    #[test]
    fn load_descarta_a_rodada_anterior() {
        let queue = loaded(&["a.sav", "b.sav"]);
        queue.take_next().unwrap();

        queue.load("PCSX2", SyncCategory::Savestates, vec![op("novo.sav", 4)]);

        let snapshot = queue.snapshot();
        assert!(snapshot.in_progress.is_empty());
        assert_eq!(snapshot.queued.len(), 1);
        assert_eq!(snapshot.queued[0].emulator, "PCSX2");
        assert_eq!(snapshot.queued[0].category, SyncCategory::Savestates);
    }
}
