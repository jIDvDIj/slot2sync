//! Transporte HTTP compartilhado pelos provedores OAuth (`drive`, `dropbox`,
//! `onedrive`): política de retry com backoff exponencial + jitter, renovação
//! de token em 401, e o limitador de banda usado pelo throttle de
//! upload/download de cada cliente.

use std::time::Duration;

use rand::Rng;

use crate::auth::AuthManager;
use crate::error::{AppError, AppResult};

/// Limitador global de banda: cada transferência reserva uma janela de tempo
/// proporcional ao tamanho e à taxa configurada; as seguintes esperam a
/// janela anterior vencer. Como os corpos são transferidos inteiros, o
/// limite vale como média entre operações — suficiente para não saturar
/// conexões lentas durante um sync grande.
#[derive(Default)]
pub struct RateLimiter {
    /// Instante até o qual a banda já está comprometida por transferências
    /// anteriores. `None` = ocioso.
    committed_until: tokio::sync::Mutex<Option<tokio::time::Instant>>,
}

impl RateLimiter {
    /// Reserva a janela para `bytes` a `kbps` e dorme até a vez desta
    /// transferência. `kbps == 0` = ilimitado (retorna na hora).
    pub async fn throttle(&self, bytes: usize, kbps: u32) {
        if kbps == 0 || bytes == 0 {
            return;
        }
        let window = Duration::from_secs_f64(bytes as f64 / (f64::from(kbps) * 1024.0));
        let start = {
            let mut guard = self.committed_until.lock().await;
            let now = tokio::time::Instant::now();
            let start = guard.map_or(now, |busy_until| busy_until.max(now));
            *guard = Some(start + window);
            start
        };
        tokio::time::sleep_until(start).await;
    }
}

/// Envia a requisição construída por `build` (que recebe o access token),
/// aplicando a política de retry comum aos três provedores OAuth: renova o
/// token em 401, tenta de novo com backoff em 429/5xx (ou quando
/// `extra_retryable` reconhecer um sinal de rate-limit específico do
/// provedor, ex.: o corpo 403 do Drive), mapeia 404 para
/// `RemoteObjectNotFound` e desiste após `max_retries` tentativas. `build` é
/// chamada de novo a cada tentativa para reconstruir o request do zero.
pub async fn send_with_retry<F>(
    auth: &AuthManager,
    op_name: &str,
    max_retries: u32,
    extra_retryable: impl Fn(reqwest::StatusCode, &str) -> bool,
    build: F,
) -> AppResult<reqwest::Response>
where
    F: Fn(&str) -> reqwest::RequestBuilder,
{
    let mut attempt: u32 = 0;
    loop {
        attempt += 1;
        let token = auth.access_token().await?;

        match build(&token).send().await {
            Ok(response) => {
                let status = response.status();
                if status.is_success() {
                    return Ok(response);
                }

                if status == reqwest::StatusCode::UNAUTHORIZED && attempt < max_retries {
                    tracing::debug!(op_name, "401 do provedor remoto; renovando access token");
                    auth.invalidate_cached_token().await;
                    continue;
                }

                let body = response.text().await.unwrap_or_default();
                let rate_limited = status == reqwest::StatusCode::TOO_MANY_REQUESTS
                    || extra_retryable(status, &body);

                if (rate_limited || status.is_server_error()) && attempt < max_retries {
                    let delay = backoff_delay(attempt);
                    tracing::warn!(op_name, %status, attempt, ?delay, "provedor remoto instável; aguardando retry");
                    tokio::time::sleep(delay).await;
                    continue;
                }

                // 404: o objeto (arquivo/pasta) não existe mais. Erro tipado
                // para o engine invalidar o cache de pastas e re-resolver
                // quando um ID/path cacheado ficou obsoleto.
                if status == reqwest::StatusCode::NOT_FOUND {
                    return Err(AppError::RemoteObjectNotFound(format!("{op_name}: {body}")));
                }

                return Err(AppError::Other(format!(
                    "{op_name} falhou ({status}): {body}"
                )));
            }
            Err(err) => {
                if attempt < max_retries {
                    let delay = backoff_delay(attempt);
                    tracing::warn!(op_name, error = %err, attempt, ?delay, "falha de rede; aguardando retry");
                    tokio::time::sleep(delay).await;
                    continue;
                }
                return Err(err.into());
            }
        }
    }
}

/// 500ms, 1s, 2s... + jitter de até 250ms.
pub fn backoff_delay(attempt: u32) -> Duration {
    let base = 500u64.saturating_mul(2u64.saturating_pow(attempt.saturating_sub(1)));
    let jitter = rand::rng().random_range(0..250);
    Duration::from_millis(base + jitter)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_cresce_exponencialmente_com_jitter_limitado() {
        for (attempt, base) in [(1u32, 500u64), (2, 1000), (3, 2000), (4, 4000)] {
            let d = backoff_delay(attempt).as_millis() as u64;
            assert!(
                (base..base + 250).contains(&d),
                "tentativa {attempt}: {d}ms fora de [{base}, {})",
                base + 250
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn throttle_espaca_transferencias_pela_taxa() {
        let limiter = RateLimiter::default();
        let start = tokio::time::Instant::now();

        limiter.throttle(64 * 1024, 64).await;
        assert_eq!(start.elapsed(), Duration::ZERO);

        limiter.throttle(64 * 1024, 64).await;
        assert!(start.elapsed() >= Duration::from_secs(1));
    }

    #[tokio::test(start_paused = true)]
    async fn throttle_zero_e_ilimitado() {
        let limiter = RateLimiter::default();
        let start = tokio::time::Instant::now();
        limiter.throttle(10 * 1024 * 1024, 0).await;
        limiter.throttle(10 * 1024 * 1024, 0).await;
        assert_eq!(start.elapsed(), Duration::ZERO);
    }
}
