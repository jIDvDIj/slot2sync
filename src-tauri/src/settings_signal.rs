//! Sinal de "as configurações mudaram", para quem espera por muito tempo.
//!
//! O SQLite continua sendo a fonte de verdade: o canal carrega só um contador
//! de revisão, e cada consumidor relê do banco o que lhe interessa. Sem ele, um
//! ajuste de configuração só valeria no fim da espera corrente — até uma hora,
//! no caso do scan periódico.

use tokio::sync::watch;

/// Ponta de escrita, guardada no `AppState`; os comandos que persistem
/// configuração chamam [`SettingsSignal::bump`].
#[derive(Clone)]
pub struct SettingsSignal(watch::Sender<u64>);

/// Ponta de leitura, entregue às tasks longas no `setup`.
pub type SettingsWatch = watch::Receiver<u64>;

impl SettingsSignal {
    pub fn new() -> Self {
        Self(watch::channel(0).0)
    }

    pub fn bump(&self) {
        self.0.send_modify(|rev| *rev += 1);
    }

    pub fn subscribe(&self) -> SettingsWatch {
        self.0.subscribe()
    }
}

impl Default for SettingsSignal {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn bump_acorda_quem_espera() {
        let signal = SettingsSignal::new();
        let mut watch = signal.subscribe();

        signal.bump();

        watch.changed().await.unwrap();
        assert_eq!(*watch.borrow(), 1);
    }

    #[tokio::test]
    async fn sem_bump_nao_ha_mudanca_pendente() {
        let signal = SettingsSignal::new();
        let mut watch = signal.subscribe();

        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), watch.changed())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn assinantes_novos_so_veem_mudancas_posteriores() {
        let signal = SettingsSignal::new();
        signal.bump();
        let mut watch = signal.subscribe();

        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(20), watch.changed())
                .await
                .is_err()
        );

        signal.bump();
        watch.changed().await.unwrap();
        assert_eq!(*watch.borrow(), 2);
    }
}
