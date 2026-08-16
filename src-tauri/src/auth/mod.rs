//! Autenticação OAuth2 + PKCE com os provedores de storage (Google, Dropbox,
//! Microsoft).
//!
//! `AuthManager` é a única porta de entrada: fluxo interativo de conexão,
//! status, desconexão e `access_token()` com renovação automática (usado
//! pelos módulos `drive`/`dropbox`/`onedrive`). Cada instância é de UM
//! provedor (`ProviderKind::is_oauth() == true`) — a pasta local/rede não usa
//! `AuthManager`. Tokens nunca cruzam a boundary — o frontend só recebe
//! `AuthStatus`.

#![allow(dead_code)]

mod oauth;
mod token_store;

use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

use crate::constants::{
    KEYRING_DROPBOX_REFRESH_TOKEN_KEY, KEYRING_GOOGLE_REFRESH_TOKEN_KEY,
    KEYRING_ONEDRIVE_REFRESH_TOKEN_KEY,
};
use crate::error::{AppError, AppResult};
use crate::remote::ProviderKind;
use crate::secrets::SecretStore;
use oauth::OAuthConfig;
use token_store::{StoredAuth, TokenStore};

/// Obtenção de tokens + email via o fluxo interativo completo (navegador +
/// rede real). Isolado atrás de um trait para que `connect()` seja testável
/// com um dublê — a implementação real nunca é exercitada em teste.
#[async_trait]
trait AuthorizeFlow: Send + Sync {
    async fn run(
        &self,
        http: &reqwest::Client,
        config: &OAuthConfig,
    ) -> AppResult<(oauth::TokenResponse, Option<String>)>;
}

struct RealAuthorizeFlow;

#[async_trait]
impl AuthorizeFlow for RealAuthorizeFlow {
    async fn run(
        &self,
        http: &reqwest::Client,
        config: &OAuthConfig,
    ) -> AppResult<(oauth::TokenResponse, Option<String>)> {
        let tokens = oauth::authorize_interactive(http, config).await?;
        let email = oauth::fetch_user_email(config.userinfo, http, &tokens.access_token).await;
        Ok((tokens, email))
    }
}

/// Estado da conexão com o provedor de storage exposto ao frontend. (→ ipc.ts)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthStatus {
    pub connected: bool,
    pub email: Option<String>,
}

impl AuthStatus {
    pub(crate) fn disconnected() -> Self {
        Self {
            connected: false,
            email: None,
        }
    }
}

/// Renova o access token quando faltar menos que isso para expirar.
const TOKEN_EXPIRY_MARGIN: Duration = Duration::from_secs(60);

struct CachedToken {
    access_token: String,
    expires_at: Instant,
}

/// Chave do keyring para o refresh token deste provedor.
fn keyring_key(provider: ProviderKind) -> &'static str {
    match provider {
        ProviderKind::GoogleDrive => KEYRING_GOOGLE_REFRESH_TOKEN_KEY,
        ProviderKind::Dropbox => KEYRING_DROPBOX_REFRESH_TOKEN_KEY,
        ProviderKind::OneDrive => KEYRING_ONEDRIVE_REFRESH_TOKEN_KEY,
        ProviderKind::LocalFolder => {
            unreachable!("LocalFolder não usa AuthManager (ProviderKind::is_oauth() == false)")
        }
    }
}

pub struct AuthManager {
    http: reqwest::Client,
    config: Option<OAuthConfig>,
    keyring_key: &'static str,
    cached: RwLock<Option<CachedToken>>,
    secrets: Arc<dyn SecretStore>,
    authorize_flow: Box<dyn AuthorizeFlow>,
    /// Token "sempre renovável" para testes de retry: quando setado, uma
    /// invalidação (401) é seguida por uma renovação sem OAuth real — os
    /// testes de `send_with_retry` não precisam mockar o endpoint do provedor.
    #[cfg(test)]
    test_fixed_token: RwLock<Option<String>>,
}

impl AuthManager {
    /// Instância para um provedor OAuth específico (Google/Dropbox/OneDrive).
    pub fn new_for(
        provider: ProviderKind,
        http: reqwest::Client,
        secrets: Arc<dyn SecretStore>,
    ) -> Self {
        let config = OAuthConfig::from_env(provider);
        if config.is_none() {
            tracing::warn!(
                provider = provider.as_str(),
                "client ID não configurado; conexão indisponível para este provedor"
            );
        }
        Self {
            http,
            config,
            keyring_key: keyring_key(provider),
            cached: RwLock::new(None),
            secrets,
            authorize_flow: Box::new(RealAuthorizeFlow),
            #[cfg(test)]
            test_fixed_token: RwLock::new(None),
        }
    }

    /// Atalho para o Google Drive — mantém os call sites/testes existentes
    /// simples quando o provedor já é conhecido em compile-time.
    pub fn new(http: reqwest::Client, secrets: Arc<dyn SecretStore>) -> Self {
        Self::new_for(ProviderKind::GoogleDrive, http, secrets)
    }

    fn config(&self) -> AppResult<&OAuthConfig> {
        self.config.as_ref().ok_or_else(|| {
            AppError::Auth(
                "credenciais OAuth não configuradas para este provedor (veja o README)".into(),
            )
        })
    }

    /// Fluxo interativo completo: navegador → consentimento → tokens.
    /// Persiste o refresh token no keyring e retorna o novo status.
    pub async fn connect(&self) -> AppResult<AuthStatus> {
        let config = self.config()?;
        let (tokens, email) = self.authorize_flow.run(&self.http, config).await?;

        let status = self.finish_connect(tokens, email).await?;
        tracing::info!(
            email = status.email.as_deref().unwrap_or("?"),
            "conectado ao provedor remoto"
        );
        Ok(status)
    }

    /// Parte final, comum a `connect`/`connect_mobile`, de completar a conexão
    /// já com os tokens em mãos: valida o refresh token, persiste e põe em
    /// cache. Isolada da obtenção dos tokens (rede real) para ser testável.
    async fn finish_connect(
        &self,
        tokens: oauth::TokenResponse,
        email: Option<String>,
    ) -> AppResult<AuthStatus> {
        let refresh_token = tokens.refresh_token.clone().ok_or_else(|| {
            AppError::Auth(
                "o provedor não retornou um refresh token; revogue o acesso do Slot2Sync nas \
                 configurações de segurança da sua conta e conecte novamente"
                    .into(),
            )
        })?;

        let stored = StoredAuth {
            refresh_token,
            email: email.clone(),
        };
        let (secrets, key) = (self.secrets.clone(), self.keyring_key);
        run_blocking(move || TokenStore::save(key, &stored, &*secrets)).await?;

        self.cache_token(&tokens).await;

        Ok(AuthStatus {
            connected: true,
            email,
        })
    }

    /// Variante mobile do fluxo interativo: usa deep link em vez de TCP loopback.
    /// O chamador (comando Tauri) configura o listener e passa o receptor do canal.
    #[cfg(mobile)]
    pub async fn connect_mobile<R: tauri::Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        redirect_rx: tokio::sync::oneshot::Receiver<String>,
    ) -> AppResult<AuthStatus> {
        let config = self.config()?;
        let tokens =
            oauth::authorize_interactive_mobile(&self.http, config, app, redirect_rx).await?;
        let email =
            oauth::fetch_user_email(config.userinfo, &self.http, &tokens.access_token).await;

        let status = self.finish_connect(tokens, email).await?;
        tracing::info!(
            email = status.email.as_deref().unwrap_or("?"),
            "conectado ao provedor remoto (mobile)"
        );
        Ok(status)
    }

    /// Conectado = existe refresh token no keyring (não exige rede).
    pub async fn status(&self) -> AppResult<AuthStatus> {
        let (secrets, key) = (self.secrets.clone(), self.keyring_key);
        let stored = run_blocking(move || TokenStore::load(key, &*secrets)).await?;
        Ok(match stored {
            Some(auth) => AuthStatus {
                connected: true,
                email: auth.email,
            },
            None => AuthStatus::disconnected(),
        })
    }

    pub async fn disconnect(&self) -> AppResult<AuthStatus> {
        let (secrets, key) = (self.secrets.clone(), self.keyring_key);
        run_blocking(move || TokenStore::clear(key, &*secrets)).await?;
        *self.cached.write().await = None;
        tracing::info!("desconectado do provedor remoto");
        Ok(AuthStatus::disconnected())
    }

    /// Access token válido, renovando automaticamente quando necessário.
    /// API interna para o módulo `drive` — nunca exposta ao frontend.
    pub async fn access_token(&self) -> AppResult<String> {
        if let Some(cached) = self.cached.read().await.as_ref() {
            if cached.expires_at > Instant::now() + TOKEN_EXPIRY_MARGIN {
                return Ok(cached.access_token.clone());
            }
        }

        #[cfg(test)]
        if let Some(token) = self.test_fixed_token.read().await.clone() {
            *self.cached.write().await = Some(CachedToken {
                access_token: token.clone(),
                expires_at: Instant::now() + Duration::from_secs(3600),
            });
            return Ok(token);
        }

        let config = self.config()?;
        let (secrets, key) = (self.secrets.clone(), self.keyring_key);
        let stored = run_blocking(move || TokenStore::load(key, &*secrets))
            .await?
            .ok_or_else(|| AppError::Auth("não conectado ao provedor remoto".into()))?;

        let tokens = oauth::refresh_access_token(&self.http, config, &stored.refresh_token).await?;
        self.cache_token(&tokens).await;
        tracing::debug!("access token renovado");
        Ok(tokens.access_token)
    }

    /// Popula o access token em cache diretamente, sem OAuth — evita que os
    /// testes do `DriveClient` precisem mockar também o endpoint de refresh.
    #[cfg(test)]
    pub(crate) async fn set_test_access_token(&self, token: &str) {
        *self.test_fixed_token.write().await = Some(token.to_string());
        *self.cached.write().await = Some(CachedToken {
            access_token: token.to_string(),
            expires_at: Instant::now() + Duration::from_secs(3600),
        });
    }

    /// Descarta o access token em cache (ex.: após um 401 do Drive),
    /// forçando renovação via refresh token na próxima chamada.
    pub async fn invalidate_cached_token(&self) {
        *self.cached.write().await = None;
    }

    async fn cache_token(&self, tokens: &oauth::TokenResponse) {
        *self.cached.write().await = Some(CachedToken {
            access_token: tokens.access_token.clone(),
            expires_at: Instant::now() + Duration::from_secs(tokens.expires_in),
        });
    }
}

async fn run_blocking<T: Send + 'static>(
    f: impl FnOnce() -> AppResult<T> + Send + 'static,
) -> AppResult<T> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| AppError::Other(format!("tarefa bloqueante abortada: {e}")))?
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use async_trait::async_trait;

    use super::oauth::OAuthConfig;
    use super::token_store::{StoredAuth, TokenStore};
    use super::{AuthManager, AuthorizeFlow, CachedToken};
    use crate::constants::KEYRING_GOOGLE_REFRESH_TOKEN_KEY;
    use crate::error::{AppError, AppResult};
    use crate::secrets::{MemSecrets, SecretStore};

    fn manager(secrets: &Arc<MemSecrets>) -> AuthManager {
        AuthManager::new(reqwest::Client::new(), secrets.clone())
    }

    /// Dublê de `AuthorizeFlow`: nunca toca rede/browser — devolve um
    /// resultado fixo, combinado na hora de montar o `AuthManager` de teste.
    struct FakeAuthorizeFlow(AppResult<(super::oauth::TokenResponse, Option<String>)>);

    impl Clone for FakeAuthorizeFlow {
        fn clone(&self) -> Self {
            match &self.0 {
                Ok((tokens, email)) => FakeAuthorizeFlow(Ok((
                    super::oauth::TokenResponse {
                        access_token: tokens.access_token.clone(),
                        expires_in: tokens.expires_in,
                        refresh_token: tokens.refresh_token.clone(),
                    },
                    email.clone(),
                ))),
                Err(AppError::Auth(msg)) => FakeAuthorizeFlow(Err(AppError::Auth(msg.clone()))),
                Err(_) => FakeAuthorizeFlow(Err(AppError::Auth("falha simulada".into()))),
            }
        }
    }

    #[async_trait]
    impl AuthorizeFlow for FakeAuthorizeFlow {
        async fn run(
            &self,
            _http: &reqwest::Client,
            _config: &OAuthConfig,
        ) -> AppResult<(super::oauth::TokenResponse, Option<String>)> {
            self.clone().0
        }
    }

    /// Constrói um `AuthManager` com uma `OAuthConfig` explícita (em vez da
    /// lida de env por `new`), para exercitar os fluxos de refresh contra um
    /// `wiremock::MockServer` local via `token_proxy_url`. O `authorize_flow`
    /// nunca é chamado por esses testes — usa um dublê que sempre falha, para
    /// que qualquer chamada acidental a `connect()` não bata na rede real.
    fn manager_with_config(secrets: &Arc<MemSecrets>, config: Option<OAuthConfig>) -> AuthManager {
        manager_with_flow(
            secrets,
            config,
            FakeAuthorizeFlow(Err(AppError::Auth("não usado neste teste".into()))),
        )
    }

    fn manager_with_flow(
        secrets: &Arc<MemSecrets>,
        config: Option<OAuthConfig>,
        flow: FakeAuthorizeFlow,
    ) -> AuthManager {
        AuthManager {
            http: reqwest::Client::new(),
            config,
            keyring_key: KEYRING_GOOGLE_REFRESH_TOKEN_KEY,
            cached: tokio::sync::RwLock::new(None),
            secrets: secrets.clone(),
            authorize_flow: Box::new(flow),
            test_fixed_token: tokio::sync::RwLock::new(None),
        }
    }

    fn proxy_config(proxy_url: &str) -> OAuthConfig {
        OAuthConfig {
            client_id: "client-de-teste".into(),
            auth_endpoint: "https://example.invalid/auth".into(),
            token_endpoint: "https://example.invalid/token".into(),
            scope: "scope-de-teste".into(),
            userinfo: super::oauth::UserinfoStrategy::GoogleOidc,
            token_proxy_url: Some(proxy_url.into()),
            proxy_secret: None,
            client_secret: None,
            extra_auth_params: &[],
        }
    }

    #[tokio::test]
    async fn status_desconectado_sem_token_salvo() {
        let secrets = Arc::new(MemSecrets::default());
        let status = manager(&secrets).status().await.unwrap();
        assert!(!status.connected);
        assert!(status.email.is_none());
    }

    #[tokio::test]
    async fn status_conectado_le_email_do_token_salvo() {
        let secrets = Arc::new(MemSecrets::default());
        TokenStore::save(
            KEYRING_GOOGLE_REFRESH_TOKEN_KEY,
            &StoredAuth {
                refresh_token: "tok".into(),
                email: Some("dev@slot2sync".into()),
            },
            &*secrets,
        )
        .unwrap();

        let status = manager(&secrets).status().await.unwrap();
        assert!(status.connected);
        assert_eq!(status.email.as_deref(), Some("dev@slot2sync"));
    }

    #[tokio::test]
    async fn token_ilegivel_degrada_para_desconectado() {
        let secrets = Arc::new(MemSecrets::default());
        secrets
            .set(KEYRING_GOOGLE_REFRESH_TOKEN_KEY, "não é json")
            .unwrap();

        let status = manager(&secrets).status().await.unwrap();
        assert!(!status.connected, "token corrompido não pode conectar");
    }

    #[tokio::test]
    async fn disconnect_apaga_o_token_persistido() {
        let secrets = Arc::new(MemSecrets::default());
        TokenStore::save(
            KEYRING_GOOGLE_REFRESH_TOKEN_KEY,
            &StoredAuth {
                refresh_token: "tok".into(),
                email: None,
            },
            &*secrets,
        )
        .unwrap();

        let m = manager(&secrets);
        assert!(m.status().await.unwrap().connected);

        let after = m.disconnect().await.unwrap();
        assert!(!after.connected);
        assert!(secrets
            .get(KEYRING_GOOGLE_REFRESH_TOKEN_KEY)
            .unwrap()
            .is_none());
    }

    #[test]
    fn token_store_roundtrip_persiste_e_limpa() {
        let secrets = MemSecrets::default();
        let auth = StoredAuth {
            refresh_token: "abc".into(),
            email: Some("x@y".into()),
        };

        TokenStore::save(KEYRING_GOOGLE_REFRESH_TOKEN_KEY, &auth, &secrets).unwrap();
        let loaded = TokenStore::load(KEYRING_GOOGLE_REFRESH_TOKEN_KEY, &secrets)
            .unwrap()
            .unwrap();
        assert_eq!(loaded.refresh_token, "abc");
        assert_eq!(loaded.email.as_deref(), Some("x@y"));

        TokenStore::clear(KEYRING_GOOGLE_REFRESH_TOKEN_KEY, &secrets).unwrap();
        assert!(TokenStore::load(KEYRING_GOOGLE_REFRESH_TOKEN_KEY, &secrets)
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn access_token_sem_config_retorna_erro_de_client_id() {
        let secrets = Arc::new(MemSecrets::default());
        let m = manager_with_config(&secrets, None);

        let err = m.access_token().await.unwrap_err();
        assert!(matches!(err, AppError::Auth(_)));
    }

    #[tokio::test]
    async fn access_token_sem_refresh_token_salvo_retorna_erro() {
        let secrets = Arc::new(MemSecrets::default());
        let m = manager_with_config(&secrets, Some(proxy_config("http://127.0.0.1:1")));

        let err = m.access_token().await.unwrap_err();
        assert!(matches!(err, AppError::Auth(_)));
    }

    #[tokio::test]
    async fn access_token_renova_via_proxy_quando_cache_vazio() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/refresh"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "novo-token",
                "expires_in": 3600,
            })))
            .mount(&server)
            .await;

        let secrets = Arc::new(MemSecrets::default());
        TokenStore::save(
            KEYRING_GOOGLE_REFRESH_TOKEN_KEY,
            &StoredAuth {
                refresh_token: "refresh-antigo".into(),
                email: None,
            },
            &*secrets,
        )
        .unwrap();

        let m = manager_with_config(&secrets, Some(proxy_config(&server.uri())));

        let token = m.access_token().await.unwrap();
        assert_eq!(token, "novo-token");
        assert_eq!(
            m.cached.read().await.as_ref().unwrap().access_token,
            "novo-token"
        );
    }

    #[tokio::test]
    async fn access_token_propaga_erro_quando_proxy_falha() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/refresh"))
            .respond_with(ResponseTemplate::new(400).set_body_string("refresh_token inválido"))
            .mount(&server)
            .await;

        let secrets = Arc::new(MemSecrets::default());
        TokenStore::save(
            KEYRING_GOOGLE_REFRESH_TOKEN_KEY,
            &StoredAuth {
                refresh_token: "refresh-invalido".into(),
                email: None,
            },
            &*secrets,
        )
        .unwrap();

        let m = manager_with_config(&secrets, Some(proxy_config(&server.uri())));

        let err = m.access_token().await.unwrap_err();
        assert!(matches!(err, AppError::Auth(_)));
    }

    #[tokio::test]
    async fn access_token_renova_quando_cache_esta_no_prazo_de_expirar() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/refresh"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "renovado",
                "expires_in": 3600,
            })))
            .mount(&server)
            .await;

        let secrets = Arc::new(MemSecrets::default());
        TokenStore::save(
            KEYRING_GOOGLE_REFRESH_TOKEN_KEY,
            &StoredAuth {
                refresh_token: "refresh".into(),
                email: None,
            },
            &*secrets,
        )
        .unwrap();

        let m = manager_with_config(&secrets, Some(proxy_config(&server.uri())));
        // Cache válido só por mais 10s: dentro da margem de 60s, deve renovar.
        *m.cached.write().await = Some(CachedToken {
            access_token: "quase-vencido".into(),
            expires_at: Instant::now() + Duration::from_secs(10),
        });

        let token = m.access_token().await.unwrap();
        assert_eq!(token, "renovado");
    }

    #[tokio::test]
    async fn access_token_usa_cache_quando_ainda_valido_sem_chamar_rede() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/refresh"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "nao-deveria-ser-usado",
                "expires_in": 3600,
            })))
            .expect(0)
            .mount(&server)
            .await;

        let secrets = Arc::new(MemSecrets::default());
        let m = manager_with_config(&secrets, Some(proxy_config(&server.uri())));
        *m.cached.write().await = Some(CachedToken {
            access_token: "cache-valido".into(),
            expires_at: Instant::now() + Duration::from_secs(3600),
        });

        let token = m.access_token().await.unwrap();
        assert_eq!(token, "cache-valido");
    }

    #[tokio::test]
    async fn set_test_access_token_curto_circuita_refresh_sem_config() {
        let secrets = Arc::new(MemSecrets::default());
        let m = manager_with_config(&secrets, None);

        m.set_test_access_token("fixo").await;
        let token = m.access_token().await.unwrap();
        assert_eq!(token, "fixo");
    }

    #[tokio::test]
    async fn invalidate_cached_token_nao_remove_test_fixed_token() {
        let secrets = Arc::new(MemSecrets::default());
        let m = manager_with_config(&secrets, None);

        m.set_test_access_token("fixo").await;
        m.invalidate_cached_token().await;

        assert!(m.cached.read().await.is_none());
        let token = m.access_token().await.unwrap();
        assert_eq!(token, "fixo");
    }

    #[tokio::test]
    async fn disconnect_limpa_cache_em_memoria() {
        let secrets = Arc::new(MemSecrets::default());
        TokenStore::save(
            KEYRING_GOOGLE_REFRESH_TOKEN_KEY,
            &StoredAuth {
                refresh_token: "r".into(),
                email: None,
            },
            &*secrets,
        )
        .unwrap();

        let m = manager_with_config(&secrets, None);
        *m.cached.write().await = Some(CachedToken {
            access_token: "tok".into(),
            expires_at: Instant::now() + Duration::from_secs(3600),
        });

        m.disconnect().await.unwrap();
        assert!(m.cached.read().await.is_none());
    }

    #[tokio::test]
    async fn connect_falha_sem_client_id_configurado() {
        let secrets = Arc::new(MemSecrets::default());
        let m = manager_with_config(&secrets, None);

        let err = m.connect().await.unwrap_err();
        assert!(matches!(err, AppError::Auth(_)));
    }

    #[tokio::test]
    async fn connect_persiste_cacheia_e_retorna_status_conectado() {
        let secrets = Arc::new(MemSecrets::default());
        let flow = FakeAuthorizeFlow(Ok((
            super::oauth::TokenResponse {
                access_token: "acesso".into(),
                expires_in: 3600,
                refresh_token: Some("refresh-novo".into()),
            },
            Some("dev@slot2sync".into()),
        )));
        let m = manager_with_flow(&secrets, Some(proxy_config("http://127.0.0.1:1")), flow);

        let status = m.connect().await.unwrap();

        assert!(status.connected);
        assert_eq!(status.email.as_deref(), Some("dev@slot2sync"));
        let stored = TokenStore::load(
            KEYRING_GOOGLE_REFRESH_TOKEN_KEY,
            &*secrets as &dyn SecretStore,
        )
        .unwrap()
        .unwrap();
        assert_eq!(stored.refresh_token, "refresh-novo");
        assert_eq!(
            m.cached.read().await.as_ref().unwrap().access_token,
            "acesso"
        );
    }

    #[tokio::test]
    async fn connect_propaga_erro_do_fluxo_de_autorizacao() {
        let secrets = Arc::new(MemSecrets::default());
        let flow = FakeAuthorizeFlow(Err(AppError::Auth("autorização negada".into())));
        let m = manager_with_flow(&secrets, Some(proxy_config("http://127.0.0.1:1")), flow);

        let err = m.connect().await.unwrap_err();
        assert!(matches!(err, AppError::Auth(msg) if msg.contains("negada")));
    }

    #[tokio::test]
    async fn connect_falha_sem_refresh_token_nao_persiste_nada() {
        let secrets = Arc::new(MemSecrets::default());
        let flow = FakeAuthorizeFlow(Ok((
            super::oauth::TokenResponse {
                access_token: "acesso".into(),
                expires_in: 3600,
                refresh_token: None,
            },
            None,
        )));
        let m = manager_with_flow(&secrets, Some(proxy_config("http://127.0.0.1:1")), flow);

        let err = m.connect().await.unwrap_err();
        assert!(matches!(err, AppError::Auth(msg) if msg.contains("refresh token")));
        assert!(TokenStore::load(
            KEYRING_GOOGLE_REFRESH_TOKEN_KEY,
            &*secrets as &dyn SecretStore
        )
        .unwrap()
        .is_none());
    }

    fn tokens(refresh_token: Option<&str>) -> super::oauth::TokenResponse {
        super::oauth::TokenResponse {
            access_token: "acesso".into(),
            expires_in: 3600,
            refresh_token: refresh_token.map(str::to_string),
        }
    }

    #[tokio::test]
    async fn finish_connect_falha_sem_refresh_token() {
        let secrets = Arc::new(MemSecrets::default());
        let m = manager_with_config(&secrets, None);

        let err = m
            .finish_connect(tokens(None), Some("dev@slot2sync".into()))
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Auth(msg) if msg.contains("refresh token")));
        assert!(TokenStore::load(
            KEYRING_GOOGLE_REFRESH_TOKEN_KEY,
            &*secrets as &dyn SecretStore
        )
        .unwrap()
        .is_none());
    }

    #[tokio::test]
    async fn finish_connect_persiste_cacheia_e_retorna_status_conectado() {
        let secrets = Arc::new(MemSecrets::default());
        let m = manager_with_config(&secrets, None);

        let status = m
            .finish_connect(tokens(Some("refresh-novo")), Some("dev@slot2sync".into()))
            .await
            .unwrap();

        assert!(status.connected);
        assert_eq!(status.email.as_deref(), Some("dev@slot2sync"));

        let stored = TokenStore::load(
            KEYRING_GOOGLE_REFRESH_TOKEN_KEY,
            &*secrets as &dyn SecretStore,
        )
        .unwrap()
        .unwrap();
        assert_eq!(stored.refresh_token, "refresh-novo");
        assert_eq!(
            m.cached.read().await.as_ref().unwrap().access_token,
            "acesso"
        );
    }

    #[tokio::test]
    async fn finish_connect_aceita_email_ausente() {
        let secrets = Arc::new(MemSecrets::default());
        let m = manager_with_config(&secrets, None);

        let status = m
            .finish_connect(tokens(Some("refresh")), None)
            .await
            .unwrap();

        assert!(status.connected);
        assert!(status.email.is_none());
    }
}
