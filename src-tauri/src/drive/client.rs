//! Camada de transporte: toda chamada à API do Drive passa por
//! `send_with_retry` — backoff exponencial com jitter, no máximo
//! `DRIVE_MAX_RETRIES` tentativas, renovação de token em 401 e tratamento
//! de rate limit (429/403 *RateLimitExceeded*/5xx).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use rand::Rng;
use tokio::sync::RwLock;

use crate::auth::AuthManager;
use crate::constants::DRIVE_MAX_RETRIES;
use crate::error::{AppError, AppResult};
use crate::storage::db::Db;
use crate::storage::drive_folders;

/// Limitador global de banda: cada transferência reserva
/// uma janela de tempo proporcional ao tamanho e à taxa configurada; as
/// seguintes esperam a janela anterior vencer. Como os corpos são transferidos
/// inteiros, o limite vale como média entre operações — suficiente para não
/// saturar conexões lentas durante um sync grande.
#[derive(Default)]
pub(crate) struct RateLimiter {
    /// Instante até o qual a banda já está comprometida por transferências
    /// anteriores. `None` = ocioso.
    committed_until: tokio::sync::Mutex<Option<tokio::time::Instant>>,
}

impl RateLimiter {
    /// Reserva a janela para `bytes` a `kbps` e dorme até a vez desta
    /// transferência. `kbps == 0` = ilimitado (retorna na hora).
    pub(crate) async fn throttle(&self, bytes: usize, kbps: u32) {
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

pub struct DriveClient {
    pub(crate) http: reqwest::Client,
    pub(crate) auth: Arc<AuthManager>,
    /// Banco local — espelha o `folder_cache` na tabela `drive_folders` para que
    /// os IDs sobrevivam a reinícios.
    pub(crate) db: Db,
    /// Cache de IDs de pastas por caminho lógico (ex.: "Slot2Sync/PPSSPP/saves").
    /// Semente carregada do SQLite no boot; escrito a cada ID novo resolvido.
    pub(crate) folder_cache: RwLock<HashMap<String, String>>,
    /// Bases da API — sempre o Google real em produção; sobrescritas por
    /// `with_base_url` nos testes para apontar a um servidor HTTP fake.
    pub(crate) api_base: String,
    pub(crate) upload_base: String,
    pub(crate) batch_base: String,
    /// Throttle global de banda (limites lidos das settings a cada operação).
    pub(crate) limiter: RateLimiter,
}

impl DriveClient {
    pub fn new(http: reqwest::Client, auth: Arc<AuthManager>, db: Db) -> Self {
        // Popula o cache com os IDs persistidos: o primeiro sync após o boot pula
        // a re-resolução das pastas já conhecidas.
        let seed = db
            .with_conn_blocking(drive_folders::load_all)
            .unwrap_or_else(|err| {
                tracing::warn!(error = %err, "cache de pastas do Drive indisponível; seguindo vazio");
                HashMap::new()
            });
        if !seed.is_empty() {
            tracing::debug!(
                pastas = seed.len(),
                "cache de IDs de pasta do Drive restaurado do SQLite"
            );
        }
        Self {
            http,
            auth,
            db,
            folder_cache: RwLock::new(seed),
            api_base: super::DRIVE_API_BASE.to_string(),
            upload_base: super::DRIVE_UPLOAD_BASE.to_string(),
            batch_base: super::DRIVE_BATCH_BASE.to_string(),
            limiter: RateLimiter::default(),
        }
    }

    /// Aplica o limite de upload configurado antes de enviar `bytes`.
    pub(crate) async fn throttle_upload(&self, bytes: usize) {
        let kbps = self
            .db
            .with(crate::storage::settings::upload_kbps)
            .await
            .unwrap_or(0);
        self.limiter.throttle(bytes, kbps).await;
    }

    /// Aplica o limite de download configurado após receber `bytes` — compromete
    /// a janela para que os próximos downloads aguardem, mantendo a média.
    pub(crate) async fn throttle_download(&self, bytes: usize) {
        let kbps = self
            .db
            .with(crate::storage::settings::download_kbps)
            .await
            .unwrap_or(0);
        self.limiter.throttle(bytes, kbps).await;
    }

    /// Redireciona as três bases da API para `base` (URI de um servidor de
    /// teste), preservando os prefixos de path reais (`/drive/v3`,
    /// `/upload/drive/v3`, `/batch/drive/v3`) para que os mocks casem com o
    /// mesmo shape de request que o `DriveClient` monta em produção.
    #[cfg(test)]
    pub(crate) fn with_base_url(mut self, base: &str) -> Self {
        self.api_base = format!("{base}/drive/v3");
        self.upload_base = format!("{base}/upload/drive/v3");
        self.batch_base = format!("{base}/batch/drive/v3");
        self
    }

    /// Invalida um caminho lógico de pasta e sua subárvore no cache (memória +
    /// SQLite). Chamado quando uma operação encontra `notFound` num ID cacheado
    /// (pasta movida/apagada no Drive); a próxima resolução reencontra ou recria.
    pub async fn invalidate_folder_path(&self, cache_key: &str) {
        let prefix = format!("{cache_key}/");
        self.folder_cache
            .write()
            .await
            .retain(|k, _| k != cache_key && !k.starts_with(&prefix));
        let key = cache_key.to_string();
        if let Err(err) = self
            .db
            .with(move |conn| drive_folders::remove_subtree(conn, &key))
            .await
        {
            tracing::warn!(error = %err, cache_key, "falha ao invalidar cache de pasta no SQLite");
        }
    }

    /// Zera todo o cache de pastas (logout/troca de conta — os IDs são por conta
    /// Google e ficam inválidos ao autenticar com outra).
    pub async fn clear_folder_cache(&self) {
        self.folder_cache.write().await.clear();
        if let Err(err) = self.db.with(drive_folders::clear).await {
            tracing::warn!(error = %err, "falha ao limpar cache de pastas no SQLite");
        }
    }

    /// Envia a requisição construída por `build` (que recebe o access token),
    /// aplicando a política de retry. `build` é chamada de novo a cada
    /// tentativa para reconstruir o request do zero.
    pub(crate) async fn send_with_retry<F>(
        &self,
        op_name: &str,
        build: F,
    ) -> AppResult<reqwest::Response>
    where
        F: Fn(&str) -> reqwest::RequestBuilder,
    {
        let mut attempt: u32 = 0;
        loop {
            attempt += 1;
            let token = self.auth.access_token().await?;

            match build(&token).send().await {
                Ok(response) => {
                    let status = response.status();
                    if status.is_success() {
                        return Ok(response);
                    }

                    if status == reqwest::StatusCode::UNAUTHORIZED && attempt < DRIVE_MAX_RETRIES {
                        tracing::debug!(op_name, "401 do Drive; renovando access token");
                        self.auth.invalidate_cached_token().await;
                        continue;
                    }

                    let body = response.text().await.unwrap_or_default();
                    let rate_limited = status == reqwest::StatusCode::TOO_MANY_REQUESTS
                        || (status == reqwest::StatusCode::FORBIDDEN
                            && body.contains("ateLimitExceeded"));

                    if (rate_limited || status.is_server_error()) && attempt < DRIVE_MAX_RETRIES {
                        let delay = backoff_delay(attempt);
                        tracing::warn!(op_name, %status, attempt, ?delay, "Drive instável; aguardando retry");
                        tokio::time::sleep(delay).await;
                        continue;
                    }

                    // 404: o objeto (arquivo/pasta) não existe mais. Erro tipado
                    // para o engine invalidar o cache de pastas e re-resolver
                    // quando um ID cacheado ficou obsoleto.
                    if status == reqwest::StatusCode::NOT_FOUND {
                        return Err(AppError::DriveObjectNotFound(format!("{op_name}: {body}")));
                    }

                    return Err(AppError::Other(format!(
                        "Drive {op_name} falhou ({status}): {body}"
                    )));
                }
                Err(err) => {
                    if attempt < DRIVE_MAX_RETRIES {
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
}

/// 500ms, 1s, 2s... + jitter de até 250ms.
fn backoff_delay(attempt: u32) -> Duration {
    let base = 500u64.saturating_mul(2u64.saturating_pow(attempt.saturating_sub(1)));
    let jitter = rand::rng().random_range(0..250);
    Duration::from_millis(base + jitter)
}

#[cfg(test)]
mod tests {
    use super::backoff_delay;

    #[test]
    fn backoff_cresce_exponencialmente_com_jitter_limitado() {
        // base 500ms·2^(n-1) + jitter [0, 250).
        for (attempt, base) in [(1u32, 500u64), (2, 1000), (3, 2000), (4, 4000)] {
            let d = backoff_delay(attempt).as_millis() as u64;
            assert!(
                (base..base + 250).contains(&d),
                "tentativa {attempt}: {d}ms fora de [{base}, {})",
                base + 250
            );
        }
    }
}

#[cfg(test)]
mod limiter_tests {
    use super::*;

    /// Duas transferências de 64 KB a 64 KB/s: a segunda só começa após a
    /// janela de 1s da primeira (tempo virtual do tokio, sem espera real).
    #[tokio::test(start_paused = true)]
    async fn throttle_espaca_transferencias_pela_taxa() {
        let limiter = RateLimiter::default();
        let start = tokio::time::Instant::now();

        limiter.throttle(64 * 1024, 64).await; // 1ª: janela imediata
        assert_eq!(start.elapsed(), Duration::ZERO);

        limiter.throttle(64 * 1024, 64).await; // 2ª: espera a janela de 1s
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

/// Testes da política de retry (`send_with_retry`) contra um servidor fake:
/// o primeiro mock (prioridade mais alta) responde uma vez com a falha em
/// questão e expira; o segundo (fallback) responde 200 — provando que o
/// `DriveClient` se recupera na tentativa seguinte. Backoff real (jitter
/// incluso) roda de verdade, então estes testes levam ~0,5–1,5s cada.
#[cfg(test)]
mod retry_tests {
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use crate::drive::test_support::client_against;
    use crate::error::AppError;

    #[tokio::test]
    async fn renova_token_e_recupera_apos_401() {
        let server = MockServer::start().await;
        let client = client_against(&server).await;

        Mock::given(method("GET"))
            .and(path("/drive/v3/files/f1"))
            .respond_with(ResponseTemplate::new(401))
            .up_to_n_times(1)
            .with_priority(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/drive/v3/files/f1"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"ok".to_vec()))
            .with_priority(2)
            .mount(&server)
            .await;

        assert_eq!(client.download("f1").await.unwrap(), b"ok");
    }

    #[tokio::test]
    async fn recupera_apos_429_com_backoff() {
        let server = MockServer::start().await;
        let client = client_against(&server).await;

        Mock::given(method("GET"))
            .and(path("/drive/v3/files/f1"))
            .respond_with(ResponseTemplate::new(429))
            .up_to_n_times(1)
            .with_priority(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/drive/v3/files/f1"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"ok".to_vec()))
            .with_priority(2)
            .mount(&server)
            .await;

        assert_eq!(client.download("f1").await.unwrap(), b"ok");
    }

    #[tokio::test]
    async fn recupera_apos_403_com_corpo_de_rate_limit() {
        let server = MockServer::start().await;
        let client = client_against(&server).await;

        Mock::given(method("GET"))
            .and(path("/drive/v3/files/f1"))
            .respond_with(
                ResponseTemplate::new(403).set_body_string(
                    r#"{"error":{"errors":[{"reason":"userRateLimitExceeded"}]}}"#,
                ),
            )
            .up_to_n_times(1)
            .with_priority(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/drive/v3/files/f1"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"ok".to_vec()))
            .with_priority(2)
            .mount(&server)
            .await;

        assert_eq!(client.download("f1").await.unwrap(), b"ok");
    }

    #[tokio::test]
    async fn recupera_apos_5xx_com_backoff() {
        let server = MockServer::start().await;
        let client = client_against(&server).await;

        Mock::given(method("GET"))
            .and(path("/drive/v3/files/f1"))
            .respond_with(ResponseTemplate::new(503))
            .up_to_n_times(1)
            .with_priority(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/drive/v3/files/f1"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"ok".to_vec()))
            .with_priority(2)
            .mount(&server)
            .await;

        assert_eq!(client.download("f1").await.unwrap(), b"ok");
    }

    #[tokio::test]
    async fn esgota_tentativas_e_falha_com_erro_tipado() {
        let server = MockServer::start().await;
        let client = client_against(&server).await;

        // Sempre 503: as DRIVE_MAX_RETRIES tentativas se esgotam.
        Mock::given(method("GET"))
            .and(path("/drive/v3/files/f1"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;

        let err = client.download("f1").await.unwrap_err();
        assert!(matches!(err, AppError::Other(_)));
    }

    #[tokio::test]
    async fn quatrocentos_e_quatro_vira_erro_tipado_sem_retry() {
        let server = MockServer::start().await;
        let client = client_against(&server).await;

        Mock::given(method("GET"))
            .and(path("/drive/v3/files/f1"))
            .respond_with(ResponseTemplate::new(404))
            .expect(1) // não deve haver retry: exatamente 1 chamada.
            .mount(&server)
            .await;

        let err = client.download("f1").await.unwrap_err();
        assert!(matches!(err, AppError::DriveObjectNotFound(_)));
    }
}
