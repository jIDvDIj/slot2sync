//! Fluxo OAuth2 com PKCE e redirect loopback (RFC 8252) para apps nativos.
//!
//! Sequência: gera `code_verifier`/`code_challenge`, sobe um listener TCP em
//! porta efêmera de 127.0.0.1, abre o navegador do sistema na tela de
//! consentimento do Google e aguarda o redirect com o authorization code,
//! que é então trocado por tokens no token endpoint.

use std::time::Duration;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::RngCore;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::error::{AppError, AppResult};

/// Sufixo do redirect URI mobile: o Worker recebe o code do provedor e faz um
/// 302 para o deep link `com.slot2sync.app:/oauth2redirect`. O redirect URI
/// completo é `{token_proxy_url}/oauth/callback` e deve estar registrado no
/// console OAuth do provedor. Só o Google exige o Worker (ver `OAuthConfig`);
/// Dropbox e Microsoft aceitam PKCE puro sem client secret.
#[cfg(mobile)]
pub const MOBILE_REDIRECT_SUFFIX: &str = "/oauth/callback";

const GOOGLE_AUTH_ENDPOINT: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const GOOGLE_TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";
const GOOGLE_USERINFO_ENDPOINT: &str = "https://www.googleapis.com/oauth2/v3/userinfo";
/// `drive.file` (não-sensível): o app só enxerga arquivos criados por ele.
/// `openid email` permite exibir a conta conectada na UI.
const GOOGLE_SCOPE: &str = "openid email https://www.googleapis.com/auth/drive.file";

const DROPBOX_AUTH_ENDPOINT: &str = "https://www.dropbox.com/oauth2/authorize";
const DROPBOX_TOKEN_ENDPOINT: &str = "https://api.dropboxapi.com/oauth2/token";
/// Sem escopo explícito: o acesso já é restrito à App Folder pelo tipo de app
/// registrado no App Console do Dropbox (não há equivalente ao `drive.file`
/// como parâmetro de escopo).
const DROPBOX_SCOPE: &str = "account_info.read";

const MICROSOFT_AUTH_ENDPOINT: &str =
    "https://login.microsoftonline.com/common/oauth2/v2.0/authorize";
const MICROSOFT_TOKEN_ENDPOINT: &str = "https://login.microsoftonline.com/common/oauth2/v2.0/token";
/// `Files.ReadWrite.AppFolder`: mesmo espírito do `drive.file` — o app só
/// enxerga sua própria pasta especial (`/me/drive/special/approot`).
const MICROSOFT_SCOPE: &str = "offline_access Files.ReadWrite.AppFolder User.Read";

const LOOPBACK_HOST: &str = "127.0.0.1";
const AUTH_FLOW_TIMEOUT: Duration = Duration::from_secs(300);

const SUCCESS_PAGE: &str = "<!doctype html><html lang=\"pt-BR\"><meta charset=\"utf-8\">\
<title>Slot2Sync</title><body style=\"font-family:sans-serif;text-align:center;padding-top:4rem\">\
<h2>Slot2Sync autorizado ✔</h2><p>Pode fechar esta aba e voltar ao aplicativo.</p></body></html>";

const ERROR_PAGE: &str = "<!doctype html><html lang=\"pt-BR\"><meta charset=\"utf-8\">\
<title>Slot2Sync</title><body style=\"font-family:sans-serif;text-align:center;padding-top:4rem\">\
<h2>Autorização não concluída ✘</h2><p>Volte ao Slot2Sync e tente novamente.</p></body></html>";

/// Estratégia de obtenção do e-mail do usuário conectado — cada provedor tem
/// um endpoint e um shape de resposta próprios; todas convergem para o mesmo
/// `Option<String>` que `AuthStatus.email` expõe.
#[derive(Clone, Copy)]
pub enum UserinfoStrategy {
    /// OIDC padrão: `GET {endpoint}` com Bearer, campo `email` no corpo.
    GoogleOidc,
    /// `POST https://api.dropboxapi.com/2/users/get_current_account` com
    /// Bearer, campo `email` aninhado em `{ "email": "..." }` no corpo (a API
    /// do Dropbox devolve um objeto de conta, não um JWT/OIDC claims).
    DropboxAccount,
    /// Microsoft Graph `GET https://graph.microsoft.com/v1.0/me` com Bearer;
    /// `mail` costuma vir preenchido, com `userPrincipalName` como fallback
    /// (contas pessoais às vezes não têm `mail` setado).
    MicrosoftGraph,
}

#[derive(Clone)]
pub struct OAuthConfig {
    pub client_id: String,
    pub auth_endpoint: String,
    pub token_endpoint: String,
    pub scope: String,
    pub userinfo: UserinfoStrategy,
    /// URL do proxy Cloudflare Worker que guarda o client_secret (produção).
    /// Quando presente, `exchange_code` e `refresh_access_token` chamam o
    /// Worker em vez do token endpoint do provedor diretamente. Só o Google
    /// precisa disso — Dropbox e Microsoft suportam PKCE puro (client
    /// público, sem secret) para apps nativos.
    pub token_proxy_url: Option<String>,
    /// Shared secret enviado no header `X-Proxy-Secret` para impedir que
    /// terceiros esgotem a quota do Worker.
    pub proxy_secret: Option<String>,
    /// Fallback para desenvolvimento local sem Worker configurado (Google).
    pub client_secret: Option<String>,
    /// Parâmetros extra da URL de autorização que pedem um refresh token —
    /// cada provedor tem sua própria convenção (Google: `access_type=offline`
    /// e `prompt=consent`; Dropbox: `token_access_type=offline`; Microsoft já
    /// cobre isso via o escopo `offline_access`, sem parâmetro extra).
    pub extra_auth_params: &'static [(&'static str, &'static str)],
}

impl OAuthConfig {
    /// Monta a config do provedor a partir das variáveis `SLOT2SYNC_*`
    /// embutidas em build-time (ver `build.rs`) — `option_env!` só enxerga
    /// literais, então cada provedor precisa do próprio `option_env!` aqui.
    pub fn from_env(provider: crate::remote::ProviderKind) -> Option<Self> {
        use crate::remote::ProviderKind;
        match provider {
            ProviderKind::GoogleDrive => {
                let client_id = option_env!("SLOT2SYNC_GOOGLE_CLIENT_ID")
                    .map(str::to_owned)
                    .or_else(|| std::env::var("SLOT2SYNC_GOOGLE_CLIENT_ID").ok())?;
                let token_proxy_url = option_env!("SLOT2SYNC_TOKEN_PROXY_URL")
                    .map(str::to_owned)
                    .or_else(|| std::env::var("SLOT2SYNC_TOKEN_PROXY_URL").ok());
                let proxy_secret = option_env!("SLOT2SYNC_PROXY_SECRET")
                    .map(str::to_owned)
                    .or_else(|| std::env::var("SLOT2SYNC_PROXY_SECRET").ok());
                let client_secret = option_env!("SLOT2SYNC_GOOGLE_CLIENT_SECRET")
                    .map(str::to_owned)
                    .or_else(|| std::env::var("SLOT2SYNC_GOOGLE_CLIENT_SECRET").ok());
                Some(Self {
                    client_id,
                    auth_endpoint: GOOGLE_AUTH_ENDPOINT.to_string(),
                    token_endpoint: GOOGLE_TOKEN_ENDPOINT.to_string(),
                    scope: GOOGLE_SCOPE.to_string(),
                    userinfo: UserinfoStrategy::GoogleOidc,
                    token_proxy_url,
                    proxy_secret,
                    client_secret,
                    extra_auth_params: &[("access_type", "offline"), ("prompt", "consent")],
                })
            }
            ProviderKind::Dropbox => {
                let client_id = option_env!("SLOT2SYNC_DROPBOX_CLIENT_ID")
                    .map(str::to_owned)
                    .or_else(|| std::env::var("SLOT2SYNC_DROPBOX_CLIENT_ID").ok())?;
                Some(Self {
                    client_id,
                    auth_endpoint: DROPBOX_AUTH_ENDPOINT.to_string(),
                    token_endpoint: DROPBOX_TOKEN_ENDPOINT.to_string(),
                    scope: DROPBOX_SCOPE.to_string(),
                    userinfo: UserinfoStrategy::DropboxAccount,
                    token_proxy_url: option_env!("SLOT2SYNC_DROPBOX_TOKEN_PROXY_URL")
                        .map(str::to_owned)
                        .or_else(|| std::env::var("SLOT2SYNC_DROPBOX_TOKEN_PROXY_URL").ok()),
                    proxy_secret: None,
                    client_secret: None,
                    extra_auth_params: &[("token_access_type", "offline")],
                })
            }
            ProviderKind::OneDrive => {
                let client_id = option_env!("SLOT2SYNC_ONEDRIVE_CLIENT_ID")
                    .map(str::to_owned)
                    .or_else(|| std::env::var("SLOT2SYNC_ONEDRIVE_CLIENT_ID").ok())?;
                Some(Self {
                    client_id,
                    auth_endpoint: MICROSOFT_AUTH_ENDPOINT.to_string(),
                    token_endpoint: MICROSOFT_TOKEN_ENDPOINT.to_string(),
                    scope: MICROSOFT_SCOPE.to_string(),
                    userinfo: UserinfoStrategy::MicrosoftGraph,
                    token_proxy_url: option_env!("SLOT2SYNC_ONEDRIVE_TOKEN_PROXY_URL")
                        .map(str::to_owned)
                        .or_else(|| std::env::var("SLOT2SYNC_ONEDRIVE_TOKEN_PROXY_URL").ok()),
                    proxy_secret: None,
                    client_secret: None,
                    extra_auth_params: &[],
                })
            }
            // Pasta local/rede não usa OAuth — nunca deveria chegar aqui.
            ProviderKind::LocalFolder => None,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub expires_in: u64,
    #[serde(default)]
    pub refresh_token: Option<String>,
}

struct Pkce {
    verifier: String,
    challenge: String,
}

fn generate_pkce() -> Pkce {
    let mut bytes = [0u8; 64];
    rand::rng().fill_bytes(&mut bytes);
    let verifier = URL_SAFE_NO_PAD.encode(bytes);
    let challenge = challenge_for(&verifier);
    Pkce {
        verifier,
        challenge,
    }
}

fn challenge_for(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

fn random_state() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Executa o fluxo interativo completo e retorna os tokens.
pub async fn authorize_interactive(
    http: &reqwest::Client,
    config: &OAuthConfig,
) -> AppResult<TokenResponse> {
    let pkce = generate_pkce();
    let state = random_state();

    let listener = TcpListener::bind((LOOPBACK_HOST, 0)).await?;
    let port = listener.local_addr()?.port();
    let redirect_uri = format!("http://{LOOPBACK_HOST}:{port}");

    let mut auth_url = url::Url::parse(&config.auth_endpoint)
        .map_err(|e| AppError::Auth(format!("URL de autorização inválida: {e}")))?;
    {
        let mut query = auth_url.query_pairs_mut();
        query
            .append_pair("client_id", &config.client_id)
            .append_pair("redirect_uri", &redirect_uri)
            .append_pair("response_type", "code")
            .append_pair("scope", &config.scope)
            .append_pair("code_challenge", &pkce.challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("state", &state);
        for (key, value) in config.extra_auth_params {
            query.append_pair(key, value);
        }
    }

    open::that_detached(auth_url.as_str())
        .map_err(|e| AppError::Auth(format!("não foi possível abrir o navegador: {e}")))?;
    tracing::info!(port, "aguardando autorização no navegador");

    let code = tokio::time::timeout(AUTH_FLOW_TIMEOUT, wait_for_code(&listener, &state))
        .await
        .map_err(|_| {
            AppError::Auth("tempo esgotado aguardando a autorização no navegador".into())
        })??;

    exchange_code(http, config, &code, &pkce.verifier, &redirect_uri).await
}

/// Fluxo OAuth mobile: abre o browser com o redirect URI do Worker como destino.
/// O Worker recebe o code do Google, faz um 302 para o deep link do app e este
/// captura via `deep-link://new-url`. O chamador configura o listener e passa o
/// Receiver pelo `redirect_rx`.
#[cfg(mobile)]
pub async fn authorize_interactive_mobile<R: tauri::Runtime>(
    http: &reqwest::Client,
    config: &OAuthConfig,
    app: &tauri::AppHandle<R>,
    redirect_rx: tokio::sync::oneshot::Receiver<String>,
) -> AppResult<TokenResponse> {
    use tauri_plugin_opener::OpenerExt;

    // O redirect URI é o Worker + sufixo; deve estar registrado no console
    // OAuth do provedor.
    let redirect_uri = config
        .token_proxy_url
        .as_deref()
        .map(|base| format!("{base}{MOBILE_REDIRECT_SUFFIX}"))
        .ok_or_else(|| {
            AppError::Auth(
                "token proxy não configurado para este provedor — necessário para OAuth mobile"
                    .into(),
            )
        })?;

    let pkce = generate_pkce();
    let state = random_state();

    let mut auth_url = url::Url::parse(&config.auth_endpoint)
        .map_err(|e| AppError::Auth(format!("URL de autorização inválida: {e}")))?;
    {
        let mut query = auth_url.query_pairs_mut();
        query
            .append_pair("client_id", &config.client_id)
            .append_pair("redirect_uri", &redirect_uri)
            .append_pair("response_type", "code")
            .append_pair("scope", &config.scope)
            .append_pair("code_challenge", &pkce.challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("state", &state);
        for (key, value) in config.extra_auth_params {
            query.append_pair(key, value);
        }
    }

    app.opener()
        .open_url(auth_url.as_str(), None::<&str>)
        .map_err(|e| AppError::Auth(format!("não foi possível abrir o navegador: {e}")))?;
    tracing::info!("aguardando autorização via deep link (redirect: {redirect_uri})");

    let redirect_url = tokio::time::timeout(AUTH_FLOW_TIMEOUT, async {
        redirect_rx
            .await
            .map_err(|_| AppError::Auth("canal de deep link fechado antes do redirect".into()))
    })
    .await
    .map_err(|_| AppError::Auth("tempo esgotado aguardando o deep link OAuth".into()))??;

    let parsed = url::Url::parse(&redirect_url)
        .map_err(|e| AppError::Auth(format!("deep link inválido: {e}")))?;

    let mut code: Option<String> = None;
    let mut recv_state: Option<String> = None;
    let mut error: Option<String> = None;
    for (k, v) in parsed.query_pairs() {
        match k.as_ref() {
            "code" => code = Some(v.into_owned()),
            "state" => recv_state = Some(v.into_owned()),
            "error" => error = Some(v.into_owned()),
            _ => {}
        }
    }

    if let Some(err) = error {
        return Err(AppError::Auth(format!(
            "autorização negada pelo Google: {err}"
        )));
    }
    if recv_state.as_deref() != Some(&state) {
        return Err(AppError::Auth(
            "state do deep link não confere (possível CSRF)".into(),
        ));
    }
    let code = code.ok_or_else(|| AppError::Auth("deep link sem authorization code".into()))?;

    exchange_code(http, config, &code, &pkce.verifier, &redirect_uri).await
}

/// Aceita conexões no listener até receber o redirect do OAuth (ignorando
/// requisições alheias, ex.: favicon), valida o `state` e devolve o code.
async fn wait_for_code(listener: &TcpListener, expected_state: &str) -> AppResult<String> {
    loop {
        let (mut stream, _) = listener.accept().await?;
        let target = match read_request_target(&mut stream).await {
            Ok(target) => target,
            Err(_) => continue,
        };

        let Some(params) = parse_redirect_target(&target) else {
            respond(&mut stream, "404 Not Found", "").await;
            continue;
        };

        if let Some(error) = params.error {
            respond(&mut stream, "200 OK", ERROR_PAGE).await;
            return Err(AppError::Auth(format!(
                "autorização negada pelo Google: {error}"
            )));
        }
        if params.state.as_deref() != Some(expected_state) {
            respond(&mut stream, "400 Bad Request", ERROR_PAGE).await;
            return Err(AppError::Auth(
                "state do redirect não confere (possível CSRF)".into(),
            ));
        }
        match params.code {
            Some(code) => {
                respond(&mut stream, "200 OK", SUCCESS_PAGE).await;
                return Ok(code);
            }
            None => {
                respond(&mut stream, "400 Bad Request", ERROR_PAGE).await;
                return Err(AppError::Auth(
                    "redirect recebido sem authorization code".into(),
                ));
            }
        }
    }
}

async fn read_request_target(stream: &mut TcpStream) -> AppResult<String> {
    let mut buf = vec![0u8; 8192];
    let n = stream.read(&mut buf).await?;
    let request = String::from_utf8_lossy(&buf[..n]);
    let first_line = request.lines().next().unwrap_or_default();
    Ok(first_line
        .split_whitespace()
        .nth(1)
        .unwrap_or("/")
        .to_string())
}

#[derive(Debug, Default, PartialEq)]
struct RedirectParams {
    code: Option<String>,
    error: Option<String>,
    state: Option<String>,
}

/// `None` quando a requisição não é o redirect do OAuth (sem `code`/`error`).
fn parse_redirect_target(target: &str) -> Option<RedirectParams> {
    let url = url::Url::parse(&format!("http://localhost{target}")).ok()?;
    let mut params = RedirectParams::default();
    for (key, value) in url.query_pairs() {
        match key.as_ref() {
            "code" => params.code = Some(value.into_owned()),
            "error" => params.error = Some(value.into_owned()),
            "state" => params.state = Some(value.into_owned()),
            _ => {}
        }
    }
    if params.code.is_some() || params.error.is_some() {
        Some(params)
    } else {
        None
    }
}

async fn respond(stream: &mut TcpStream, status: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(response.as_bytes()).await;
    let _ = stream.shutdown().await;
}

async fn exchange_code(
    http: &reqwest::Client,
    config: &OAuthConfig,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
) -> AppResult<TokenResponse> {
    if let Some(proxy) = &config.token_proxy_url {
        let url = format!("{proxy}/token");
        let body = serde_json::json!({
            "code": code,
            "code_verifier": verifier,
            "redirect_uri": redirect_uri,
        });
        return post_token_proxy(http, &url, &body, config.proxy_secret.as_deref()).await;
    }
    let mut form = vec![
        ("grant_type", "authorization_code"),
        ("code", code),
        ("code_verifier", verifier),
        ("client_id", config.client_id.as_str()),
        ("redirect_uri", redirect_uri),
    ];
    if let Some(secret) = config.client_secret.as_deref() {
        form.push(("client_secret", secret));
    }
    post_token_at(http, &config.token_endpoint, &form).await
}

pub async fn refresh_access_token(
    http: &reqwest::Client,
    config: &OAuthConfig,
    refresh_token: &str,
) -> AppResult<TokenResponse> {
    if let Some(proxy) = &config.token_proxy_url {
        let url = format!("{proxy}/refresh");
        let body = serde_json::json!({ "refresh_token": refresh_token });
        return post_token_proxy(http, &url, &body, config.proxy_secret.as_deref()).await;
    }
    let mut form = vec![
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token),
        ("client_id", config.client_id.as_str()),
    ];
    if let Some(secret) = config.client_secret.as_deref() {
        form.push(("client_secret", secret));
    }
    post_token_at(http, &config.token_endpoint, &form).await
}

async fn post_token_at(
    http: &reqwest::Client,
    endpoint: &str,
    form: &[(&str, &str)],
) -> AppResult<TokenResponse> {
    let response = http.post(endpoint).form(form).send().await?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(AppError::Auth(format!(
            "token endpoint retornou {status}: {body}"
        )));
    }
    Ok(response.json::<TokenResponse>().await?)
}

async fn post_token_proxy(
    http: &reqwest::Client,
    url: &str,
    body: &serde_json::Value,
    proxy_secret: Option<&str>,
) -> AppResult<TokenResponse> {
    let mut request = http.post(url).json(body);
    if let Some(secret) = proxy_secret {
        request = request.header("X-Proxy-Secret", secret);
    }
    let response = request.send().await?;
    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        return Err(AppError::Auth(format!(
            "proxy de token retornou {status}: {text}"
        )));
    }
    Ok(response.json::<TokenResponse>().await?)
}

const DROPBOX_ACCOUNT_ENDPOINT: &str = "https://api.dropboxapi.com/2/users/get_current_account";
const MICROSOFT_ME_ENDPOINT: &str = "https://graph.microsoft.com/v1.0/me";

#[derive(Debug, Deserialize)]
struct UserInfo {
    email: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GraphUser {
    mail: Option<String>,
    #[serde(rename = "userPrincipalName")]
    user_principal_name: Option<String>,
}

/// Best-effort: falha em obter o e-mail não impede a conexão. Endpoint
/// injetável para os chamadores poderem testar contra um servidor fake.
/// Usado por `GoogleOidc` (GET simples, campo `email` no corpo).
pub(super) async fn fetch_user_email_at(
    http: &reqwest::Client,
    endpoint: &str,
    access_token: &str,
) -> AppResult<Option<String>> {
    let response = http.get(endpoint).bearer_auth(access_token).send().await?;
    if !response.status().is_success() {
        return Ok(None);
    }
    Ok(response
        .json::<UserInfo>()
        .await
        .map(|u| u.email)
        .unwrap_or(None))
}

/// `POST` com corpo vazio — a API do Dropbox devolve o mesmo shape `{email}`
/// dentre outros campos, então reaproveita `UserInfo`.
async fn fetch_dropbox_email_at(
    http: &reqwest::Client,
    endpoint: &str,
    access_token: &str,
) -> AppResult<Option<String>> {
    let response = http
        .post(endpoint)
        .bearer_auth(access_token)
        .json(&serde_json::json!({}))
        .send()
        .await?;
    if !response.status().is_success() {
        return Ok(None);
    }
    Ok(response
        .json::<UserInfo>()
        .await
        .map(|u| u.email)
        .unwrap_or(None))
}

/// `mail` costuma vir preenchido; contas pessoais às vezes só têm
/// `userPrincipalName` (que também é um e-mail válido nesse caso).
async fn fetch_microsoft_email_at(
    http: &reqwest::Client,
    endpoint: &str,
    access_token: &str,
) -> AppResult<Option<String>> {
    let response = http.get(endpoint).bearer_auth(access_token).send().await?;
    if !response.status().is_success() {
        return Ok(None);
    }
    Ok(response
        .json::<GraphUser>()
        .await
        .ok()
        .and_then(|u| u.mail.or(u.user_principal_name)))
}

/// Despacha para o endpoint/shape certo conforme a estratégia do provedor.
pub(super) async fn fetch_user_email(
    strategy: UserinfoStrategy,
    http: &reqwest::Client,
    access_token: &str,
) -> Option<String> {
    let result = match strategy {
        UserinfoStrategy::GoogleOidc => {
            fetch_user_email_at(http, GOOGLE_USERINFO_ENDPOINT, access_token).await
        }
        UserinfoStrategy::DropboxAccount => {
            fetch_dropbox_email_at(http, DROPBOX_ACCOUNT_ENDPOINT, access_token).await
        }
        UserinfoStrategy::MicrosoftGraph => {
            fetch_microsoft_email_at(http, MICROSOFT_ME_ENDPOINT, access_token).await
        }
    };
    result.unwrap_or(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_challenge_segue_rfc_7636() {
        // Vetor de teste do apêndice B da RFC 7636.
        assert_eq!(
            challenge_for("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn verifier_tem_tamanho_valido() {
        let pkce = generate_pkce();
        // RFC 7636 exige entre 43 e 128 caracteres.
        assert!((43..=128).contains(&pkce.verifier.len()));
        assert_eq!(challenge_for(&pkce.verifier), pkce.challenge);
    }

    #[test]
    fn parse_extrai_code_e_state_do_redirect() {
        let params = parse_redirect_target("/?state=xyz&code=4%2F0Abc-_123").unwrap();
        assert_eq!(params.code.as_deref(), Some("4/0Abc-_123"));
        assert_eq!(params.state.as_deref(), Some("xyz"));
        assert_eq!(params.error, None);
    }

    #[test]
    fn parse_extrai_erro_de_acesso_negado() {
        let params = parse_redirect_target("/?error=access_denied&state=xyz").unwrap();
        assert_eq!(params.error.as_deref(), Some("access_denied"));
        assert_eq!(params.code, None);
    }

    #[test]
    fn parse_ignora_requisicoes_alheias() {
        assert_eq!(parse_redirect_target("/favicon.ico"), None);
        assert_eq!(parse_redirect_target("/"), None);
        assert_eq!(parse_redirect_target("/?foo=bar"), None);
    }

    #[test]
    fn random_state_gera_valores_unicos_com_tamanho_esperado() {
        let a = random_state();
        let b = random_state();
        assert_ne!(a, b);
        // 32 bytes em base64url sem padding = 43 caracteres.
        assert_eq!(a.len(), 43);
    }

    #[tokio::test]
    async fn read_request_target_extrai_linha_de_requisicao() {
        let listener = TcpListener::bind((LOOPBACK_HOST, 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();

        let client = tokio::spawn(async move {
            let mut stream = TcpStream::connect(addr).await.unwrap();
            stream
                .write_all(b"GET /callback?code=abc HTTP/1.1\r\nHost: x\r\n\r\n")
                .await
                .unwrap();
        });

        let (mut stream, _) = listener.accept().await.unwrap();
        let target = read_request_target(&mut stream).await.unwrap();
        assert_eq!(target, "/callback?code=abc");
        client.await.unwrap();
    }

    #[tokio::test]
    async fn respond_escreve_status_e_corpo_http() {
        let listener = TcpListener::bind((LOOPBACK_HOST, 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();

        let reader = tokio::spawn(async move {
            let mut stream = TcpStream::connect(addr).await.unwrap();
            let mut buf = Vec::new();
            stream.read_to_end(&mut buf).await.unwrap();
            String::from_utf8(buf).unwrap()
        });

        let (mut stream, _) = listener.accept().await.unwrap();
        respond(&mut stream, "200 OK", SUCCESS_PAGE).await;

        let response = reader.await.unwrap();
        assert!(response.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(response.contains(&format!("Content-Length: {}", SUCCESS_PAGE.len())));
        assert!(response.ends_with(SUCCESS_PAGE));
    }

    /// Simula um cliente HTTP simples que faz uma requisição GET de redirect
    /// contra o listener loopback do `wait_for_code`.
    async fn send_redirect(addr: std::net::SocketAddr, query: &str) -> String {
        let mut stream = TcpStream::connect(addr).await.unwrap();
        stream
            .write_all(format!("GET /?{query} HTTP/1.1\r\nHost: x\r\n\r\n").as_bytes())
            .await
            .unwrap();
        let mut buf = Vec::new();
        stream.read_to_end(&mut buf).await.unwrap();
        String::from_utf8(buf).unwrap()
    }

    #[tokio::test]
    async fn wait_for_code_ignora_requisicoes_alheias_e_aceita_o_redirect_correto() {
        let listener = TcpListener::bind((LOOPBACK_HOST, 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();

        let waiter = tokio::spawn(async move { wait_for_code(&listener, "state-ok").await });

        // Requisição irrelevante (ex.: favicon) deve ser ignorada, não encerrar o loop.
        let favicon_resp = send_redirect(addr, "").await;
        assert!(favicon_resp.starts_with("HTTP/1.1 404 Not Found"));

        // Agora o redirect de verdade.
        let ok_resp = send_redirect(addr, "state=state-ok&code=abc123").await;
        assert!(ok_resp.starts_with("HTTP/1.1 200 OK"));

        let code = waiter.await.unwrap().unwrap();
        assert_eq!(code, "abc123");
    }

    #[tokio::test]
    async fn wait_for_code_rejeita_state_divergente() {
        let listener = TcpListener::bind((LOOPBACK_HOST, 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();

        let waiter = tokio::spawn(async move { wait_for_code(&listener, "state-esperado").await });

        let resp = send_redirect(addr, "state=state-errado&code=abc123").await;
        assert!(resp.starts_with("HTTP/1.1 400 Bad Request"));

        let err = waiter.await.unwrap().unwrap_err();
        assert!(matches!(err, AppError::Auth(msg) if msg.contains("CSRF")));
    }

    #[tokio::test]
    async fn wait_for_code_propaga_erro_de_autorizacao_negada() {
        let listener = TcpListener::bind((LOOPBACK_HOST, 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();

        let waiter = tokio::spawn(async move { wait_for_code(&listener, "state-ok").await });

        let resp = send_redirect(addr, "state=state-ok&error=access_denied").await;
        assert!(resp.starts_with("HTTP/1.1 200 OK"));

        let err = waiter.await.unwrap().unwrap_err();
        assert!(matches!(err, AppError::Auth(msg) if msg.contains("access_denied")));
    }

    // Não há teste para o branch `params.code == None` de `wait_for_code`: como
    // `parse_redirect_target` só devolve `Some` quando `code` ou `error` estão
    // presentes, e `error` já é tratado antes, esse branch é inalcançável por
    // qualquer requisição HTTP real.

    mod http_tests {
        use wiremock::matchers::{body_string_contains, header, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        use super::super::*;

        fn proxy_config(proxy_url: &str, proxy_secret: Option<&str>) -> OAuthConfig {
            OAuthConfig {
                client_id: "client-teste".into(),
                auth_endpoint: GOOGLE_AUTH_ENDPOINT.to_string(),
                token_endpoint: GOOGLE_TOKEN_ENDPOINT.to_string(),
                scope: GOOGLE_SCOPE.to_string(),
                userinfo: UserinfoStrategy::GoogleOidc,
                token_proxy_url: Some(proxy_url.to_string()),
                proxy_secret: proxy_secret.map(str::to_string),
                client_secret: None,
                extra_auth_params: &[],
            }
        }

        #[tokio::test]
        async fn exchange_code_via_proxy_retorna_tokens() {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/token"))
                .and(body_string_contains("\"code\":\"auth-code\""))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "access_token": "tok-abc",
                    "expires_in": 3600,
                    "refresh_token": "refresh-abc",
                })))
                .mount(&server)
                .await;

            let config = proxy_config(&server.uri(), None);
            let http = reqwest::Client::new();
            let tokens = exchange_code(&http, &config, "auth-code", "verifier", "http://redirect")
                .await
                .unwrap();

            assert_eq!(tokens.access_token, "tok-abc");
            assert_eq!(tokens.expires_in, 3600);
            assert_eq!(tokens.refresh_token.as_deref(), Some("refresh-abc"));
        }

        #[tokio::test]
        async fn exchange_code_via_proxy_envia_header_de_secret() {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/token"))
                .and(header("X-Proxy-Secret", "s3gredo"))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "access_token": "tok-abc",
                    "expires_in": 3600,
                })))
                .mount(&server)
                .await;

            let config = proxy_config(&server.uri(), Some("s3gredo"));
            let http = reqwest::Client::new();
            exchange_code(&http, &config, "auth-code", "verifier", "http://redirect")
                .await
                .unwrap();
        }

        #[tokio::test]
        async fn exchange_code_via_proxy_propaga_erro_http() {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/token"))
                .respond_with(ResponseTemplate::new(400).set_body_string("code inválido"))
                .mount(&server)
                .await;

            let config = proxy_config(&server.uri(), None);
            let http = reqwest::Client::new();
            let err = exchange_code(&http, &config, "auth-code", "verifier", "http://redirect")
                .await
                .unwrap_err();

            assert!(
                matches!(err, AppError::Auth(msg) if msg.contains("400") && msg.contains("code inválido"))
            );
        }

        #[tokio::test]
        async fn refresh_access_token_via_proxy_retorna_tokens() {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/refresh"))
                .and(body_string_contains("\"refresh_token\":\"refresh-xyz\""))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "access_token": "tok-novo",
                    "expires_in": 1800,
                })))
                .mount(&server)
                .await;

            let config = proxy_config(&server.uri(), None);
            let http = reqwest::Client::new();
            let tokens = refresh_access_token(&http, &config, "refresh-xyz")
                .await
                .unwrap();

            assert_eq!(tokens.access_token, "tok-novo");
            assert_eq!(tokens.expires_in, 1800);
            assert_eq!(tokens.refresh_token, None);
        }

        #[tokio::test]
        async fn refresh_access_token_via_proxy_propaga_erro_http() {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/refresh"))
                .respond_with(ResponseTemplate::new(401))
                .mount(&server)
                .await;

            let config = proxy_config(&server.uri(), None);
            let http = reqwest::Client::new();
            let err = refresh_access_token(&http, &config, "refresh-xyz")
                .await
                .unwrap_err();

            assert!(matches!(err, AppError::Auth(msg) if msg.contains("401")));
        }

        #[tokio::test]
        async fn post_token_at_retorna_tokens_no_fluxo_direto() {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/token-direto"))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "access_token": "tok-direto",
                    "expires_in": 3600,
                })))
                .mount(&server)
                .await;

            let http = reqwest::Client::new();
            let endpoint = format!("{}/token-direto", server.uri());
            let form = [("grant_type", "authorization_code"), ("code", "abc")];
            let tokens = post_token_at(&http, &endpoint, &form).await.unwrap();

            assert_eq!(tokens.access_token, "tok-direto");
        }

        #[tokio::test]
        async fn post_token_at_propaga_erro_http_com_corpo() {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/token-direto"))
                .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
                .mount(&server)
                .await;

            let http = reqwest::Client::new();
            let endpoint = format!("{}/token-direto", server.uri());
            let err = post_token_at(&http, &endpoint, &[]).await.unwrap_err();

            assert!(
                matches!(err, AppError::Auth(msg) if msg.contains("500") && msg.contains("boom"))
            );
        }

        #[tokio::test]
        async fn fetch_user_email_at_retorna_email_quando_sucesso() {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/userinfo"))
                .and(header("Authorization", "Bearer tok-abc"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(serde_json::json!({ "email": "user@example.com" })),
                )
                .mount(&server)
                .await;

            let http = reqwest::Client::new();
            let endpoint = format!("{}/userinfo", server.uri());
            let email = fetch_user_email_at(&http, &endpoint, "tok-abc")
                .await
                .unwrap();

            assert_eq!(email.as_deref(), Some("user@example.com"));
        }

        #[tokio::test]
        async fn fetch_user_email_at_retorna_none_quando_http_falha() {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/userinfo"))
                .respond_with(ResponseTemplate::new(401))
                .mount(&server)
                .await;

            let http = reqwest::Client::new();
            let endpoint = format!("{}/userinfo", server.uri());
            let email = fetch_user_email_at(&http, &endpoint, "tok-invalido")
                .await
                .unwrap();

            assert_eq!(email, None);
        }

        #[tokio::test]
        async fn fetch_user_email_at_retorna_none_quando_campo_email_ausente() {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/userinfo"))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
                .mount(&server)
                .await;

            let http = reqwest::Client::new();
            let endpoint = format!("{}/userinfo", server.uri());
            let email = fetch_user_email_at(&http, &endpoint, "tok-abc")
                .await
                .unwrap();

            assert_eq!(email, None);
        }
    }
}
