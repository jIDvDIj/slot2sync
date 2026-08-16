//! Polling de processos via `sysinfo` e a máquina de estados de debounce.
//!
//! `RunStateTracker` é pura (sem `sysinfo`): recebe quais emuladores estão
//! presentes no tick e devolve as transições, aplicando o debounce de
//! encerramento. `poll_once` faz a ponte com o SO.

use std::collections::{HashMap, HashSet};

use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};

use super::WatcherEvent;

/// Um emulador monitorado: nome canônico + nomes de processo do SO.
pub(super) struct MonitoredEmulator {
    pub name: String,
    pub process_names: Vec<String>,
}

#[derive(Default, Clone, Copy)]
struct TrackedState {
    running: bool,
    missed_ticks: u32,
}

/// Acompanha o estado rodando/parado de cada emulador com debounce de parada.
pub(super) struct RunStateTracker {
    states: HashMap<String, TrackedState>,
    stop_debounce_ticks: u32,
}

impl RunStateTracker {
    pub fn new(stop_debounce_ticks: u32) -> Self {
        Self {
            states: HashMap::new(),
            stop_debounce_ticks,
        }
    }

    /// Reconcilia o estado conhecido com o tick atual. `monitored` são os
    /// emuladores configurados; `present` os que têm processo rodando agora.
    /// A abertura emite `EmulatorStarted` imediatamente; o encerramento só
    /// após `stop_debounce_ticks` ticks consecutivos ausente.
    pub fn reconcile(
        &mut self,
        monitored: &[String],
        present: &HashSet<String>,
    ) -> Vec<WatcherEvent> {
        let mut events = Vec::new();

        for name in monitored {
            let state = self.states.entry(name.clone()).or_default();
            if present.contains(name) {
                state.missed_ticks = 0;
                if !state.running {
                    state.running = true;
                    events.push(WatcherEvent::EmulatorStarted(name.clone()));
                }
            } else if state.running {
                state.missed_ticks += 1;
                if state.missed_ticks >= self.stop_debounce_ticks {
                    state.running = false;
                    state.missed_ticks = 0;
                    events.push(WatcherEvent::EmulatorStopped(name.clone()));
                }
            }
        }

        // Esquece emuladores que deixaram de ser monitorados (removidos pelo
        // usuário) sem emitir evento — não queremos disparar sync por isso.
        let monitored_set: HashSet<&String> = monitored.iter().collect();
        self.states.retain(|name, _| monitored_set.contains(name));

        events
    }
}

/// Atualiza a lista de processos e reconcilia o estado dos emuladores.
pub(super) fn poll_once(
    system: &mut System,
    tracker: &mut RunStateTracker,
    monitored: &[MonitoredEmulator],
) -> Vec<WatcherEvent> {
    // `nothing()` mantém o refresh barato: o nome do processo vem mesmo sem
    // coletar memória/CPU/disco.
    system.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());

    let running_proc_names: HashSet<String> = system
        .processes()
        .values()
        .map(|p| p.name().to_string_lossy().to_lowercase())
        .collect();

    let present: HashSet<String> = monitored
        .iter()
        .filter(|m| {
            m.process_names
                .iter()
                .any(|pn| running_proc_names.contains(&pn.to_lowercase()))
        })
        .map(|m| m.name.clone())
        .collect();

    let names: Vec<String> = monitored.iter().map(|m| m.name.clone()).collect();
    tracker.reconcile(&names, &present)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(items: &[&str]) -> HashSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn names(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn abertura_emite_started_imediatamente() {
        let mut t = RunStateTracker::new(2);
        let evs = t.reconcile(&names(&["PPSSPP"]), &set(&["PPSSPP"]));
        assert_eq!(evs, vec![WatcherEvent::EmulatorStarted("PPSSPP".into())]);
    }

    #[test]
    fn presenca_continua_nao_reemite_started() {
        let mut t = RunStateTracker::new(2);
        t.reconcile(&names(&["PPSSPP"]), &set(&["PPSSPP"]));
        let evs = t.reconcile(&names(&["PPSSPP"]), &set(&["PPSSPP"]));
        assert!(evs.is_empty());
    }

    #[test]
    fn encerramento_emite_stopped_somente_apos_debounce() {
        let mut t = RunStateTracker::new(2);
        t.reconcile(&names(&["PPSSPP"]), &set(&["PPSSPP"]));

        // 1º tick ausente: ainda sem evento (debounce).
        let evs = t.reconcile(&names(&["PPSSPP"]), &set(&[]));
        assert!(evs.is_empty());

        // 2º tick ausente: declara encerrado.
        let evs = t.reconcile(&names(&["PPSSPP"]), &set(&[]));
        assert_eq!(evs, vec![WatcherEvent::EmulatorStopped("PPSSPP".into())]);
    }

    #[test]
    fn flap_curto_nao_emite_stopped() {
        let mut t = RunStateTracker::new(2);
        t.reconcile(&names(&["PPSSPP"]), &set(&["PPSSPP"]));
        // Some por 1 tick (abaixo do debounce)...
        assert!(t.reconcile(&names(&["PPSSPP"]), &set(&[])).is_empty());
        // ...e reaparece: nenhum Stopped, nenhum novo Started.
        assert!(t
            .reconcile(&names(&["PPSSPP"]), &set(&["PPSSPP"]))
            .is_empty());
    }

    #[test]
    fn emulador_nunca_presente_nao_emite_nada() {
        let mut t = RunStateTracker::new(2);
        let evs = t.reconcile(&names(&["PPSSPP", "PCSX2"]), &set(&[]));
        assert!(evs.is_empty());
    }

    #[test]
    fn ciclo_completo_start_stop_start() {
        let mut t = RunStateTracker::new(1);
        assert_eq!(
            t.reconcile(&names(&["PCSX2"]), &set(&["PCSX2"])),
            vec![WatcherEvent::EmulatorStarted("PCSX2".into())]
        );
        assert_eq!(
            t.reconcile(&names(&["PCSX2"]), &set(&[])),
            vec![WatcherEvent::EmulatorStopped("PCSX2".into())]
        );
        assert_eq!(
            t.reconcile(&names(&["PCSX2"]), &set(&["PCSX2"])),
            vec![WatcherEvent::EmulatorStarted("PCSX2".into())]
        );
    }

    #[test]
    fn remocao_do_monitoramento_nao_emite_stopped() {
        let mut t = RunStateTracker::new(2);
        t.reconcile(&names(&["PPSSPP"]), &set(&["PPSSPP"]));
        // Emulador sai da lista de monitorados (usuário removeu): silencioso.
        let evs = t.reconcile(&names(&[]), &set(&[]));
        assert!(evs.is_empty());
        // E o estado foi esquecido: readicionar e reabrir emite Started de novo.
        let evs = t.reconcile(&names(&["PPSSPP"]), &set(&["PPSSPP"]));
        assert_eq!(evs, vec![WatcherEvent::EmulatorStarted("PPSSPP".into())]);
    }

    /// `poll_once` faz a ponte real com o SO via `sysinfo`: usa o próprio
    /// processo de teste (garantidamente rodando) como "emulador monitorado".
    #[test]
    fn poll_once_detecta_o_proprio_processo_como_presente() {
        let mut system = System::new_all();
        system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing(),
        );
        let pid = sysinfo::get_current_pid().expect("PID do processo atual");
        let own_name = system
            .process(pid)
            .expect("processo atual visível no sysinfo")
            .name()
            .to_string_lossy()
            .to_string();

        let monitored = vec![MonitoredEmulator {
            name: "self".into(),
            process_names: vec![own_name],
        }];
        let mut tracker = RunStateTracker::new(2);

        let evs = poll_once(&mut system, &mut tracker, &monitored);
        assert_eq!(evs, vec![WatcherEvent::EmulatorStarted("self".into())]);

        // Continua presente: segunda chamada não reemite Started.
        let evs = poll_once(&mut system, &mut tracker, &monitored);
        assert!(evs.is_empty());
    }

    #[test]
    fn poll_once_processo_desconhecido_fica_ausente() {
        let mut system = System::new_all();
        let monitored = vec![MonitoredEmulator {
            name: "fantasma".into(),
            process_names: vec!["processo-que-nao-existe-de-verdade-xyz".into()],
        }];
        let mut tracker = RunStateTracker::new(2);

        let evs = poll_once(&mut system, &mut tracker, &monitored);
        assert!(evs.is_empty());
    }

    #[test]
    fn dois_emuladores_sao_rastreados_independentemente() {
        let mut t = RunStateTracker::new(1);
        let evs = t.reconcile(&names(&["PPSSPP", "PCSX2"]), &set(&["PPSSPP"]));
        assert_eq!(evs, vec![WatcherEvent::EmulatorStarted("PPSSPP".into())]);

        let evs = t.reconcile(&names(&["PPSSPP", "PCSX2"]), &set(&["PCSX2"]));
        assert_eq!(
            evs,
            vec![
                WatcherEvent::EmulatorStopped("PPSSPP".into()),
                WatcherEvent::EmulatorStarted("PCSX2".into()),
            ]
        );
    }
}
