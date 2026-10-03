//! Sign in with ChatGPT: OAuth authorization code flow with PKCE and a
//! loopback redirect, for locally run apps. No client secret is involved.
use crate::{
    session::{Session, now},
    settings::Settings,
};
use base64::Engine;
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fmt,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub const SCOPE: &str =
    "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct";
const FIRST_CLIENT: &str = "dynamic_agent_client";
const APP_NAME: &str = "AXIS CAM Studio";
/// Token-endpoint error codes after which the refresh token can never work.
const FATAL_REFRESH_CODES: [&str; 5] = [
    "invalid_grant",
    "token_expired",
    "refresh_token_expired",
    "refresh_token_reused",
    "invalid_client",
];

#[derive(Debug, Clone, PartialEq)]
pub struct Endpoints {
    pub authorize: String,
    pub token: String,
    pub revoke: String,
    pub jwks: String,
    pub issuer: String,
    /// API base URL, also the OAuth `resource`.
    pub api: String,
}
impl Endpoints {
    pub fn openai() -> Self {
        Self {
            authorize: "https://auth.openai.com/api/accounts/authorize".into(),
            token: "https://auth.openai.com/api/accounts/oauth/token".into(),
            revoke: "https://auth.openai.com/api/accounts/oauth/revoke".into(),
            jwks: "https://auth.openai.com/.well-known/jwks.json".into(),
            issuer: "https://auth.openai.com".into(),
            api: "https://api.openai.com/v1".into(),
        }
    }
    #[cfg(test)]
    pub fn local(base: &str) -> Self {
        Self {
            authorize: format!("{base}/authorize"),
            token: format!("{base}/token"),
            revoke: format!("{base}/revoke"),
            jwks: format!("{base}/jwks"),
            issuer: crate::fake_openai::ISSUER.into(),
            api: format!("{base}/v1"),
        }
    }
}
/// Refresh failures: `Fatal` means the tokens are unusable and the user must
/// sign in again; `Retry` is a network or server problem.
#[derive(Debug, Clone, PartialEq)]
pub enum AuthError {
    Fatal(String),
    Retry(String),
}
/// The HTTP client for sign-in and API calls: it never follows redirects (so
/// tokens and codes only go where they were sent) and gives up after
/// `timeout` in total.
pub fn http_client(timeout: Duration) -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(15))
        .timeout(timeout)
        .build()
        .expect("HTTP client")
}
fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}
/// 32 random bytes, base64url.
pub fn random_token() -> String {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).expect("system random source");
    b64(&bytes)
}
pub fn challenge(verifier: &str) -> String {
    b64(&Sha256::digest(verifier.as_bytes()))
}
/// Fresh per-attempt values. The verifier and nonce never leave this process
/// except as their challenge and inside the validated ID token, so there is
/// deliberately no `Debug`.
#[derive(Clone)]
pub struct Attempt {
    pub state: String,
    pub nonce: String,
    pub verifier: String,
    pub redirect_uri: String,
}
impl Attempt {
    pub fn new(redirect_uri: String) -> Self {
        Self {
            state: random_token(),
            nonce: random_token(),
            verifier: random_token(),
            redirect_uri,
        }
    }
}
pub fn authorize_url(
    e: &Endpoints,
    settings: &Settings,
    attempt: &Attempt,
    id_token_hint: Option<&str>,
) -> String {
    let mut url = reqwest::Url::parse(&e.authorize).expect("authorize endpoint");
    {
        let mut q = url.query_pairs_mut();
        if settings.client_id.is_empty() {
            q.append_pair("client_id", FIRST_CLIENT)
                .append_pair("agent_name_hint", APP_NAME);
        } else {
            q.append_pair("client_id", &settings.client_id);
        }
        q.append_pair("ext_agent_host_id", &settings.host_id)
            .append_pair("response_type", "code")
            .append_pair("redirect_uri", &attempt.redirect_uri)
            .append_pair("scope", SCOPE)
            .append_pair("resource", &e.api)
            .append_pair("state", &attempt.state)
            .append_pair("nonce", &attempt.nonce)
            .append_pair("code_challenge_method", "S256")
            .append_pair("code_challenge", &challenge(&attempt.verifier));
        if let Some(hint) = id_token_hint.filter(|_| !settings.client_id.is_empty()) {
            q.append_pair("id_token_hint", hint);
        }
    }
    url.into()
}
#[derive(Clone, PartialEq)]
pub struct Callback {
    pub code: String,
    /// Issued at first registration.
    pub client_id: Option<String>,
}
impl fmt::Debug for Callback {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Callback")
            .field("code", &"<redacted>")
            .field("client_id", &self.client_id)
            .finish()
    }
}
/// `None` when the request is not this attempt's callback (another path or a
/// different `state`); such requests are answered 404 and ignored.
pub fn parse_callback(target: &str, state: &str) -> Option<Result<Callback, String>> {
    let url = reqwest::Url::parse(&format!("http://127.0.0.1{target}")).ok()?;
    if url.path() != "/callback" {
        return None;
    }
    let value = |key: &str| {
        url.query_pairs()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.into_owned())
            .filter(|v| !v.is_empty())
    };
    if value("state").as_deref() != Some(state) {
        return None;
    }
    if let Some(error) = value("error") {
        return Some(Err(if error == "access_denied" {
            "Sign-in was cancelled in the browser.".into()
        } else {
            format!("OpenAI could not complete the sign-in ({error}).")
        }));
    }
    Some(match value("code") {
        Some(code) => Ok(Callback {
            code,
            client_id: value("client_id"),
        }),
        None => Err("The sign-in response had no authorization code.".into()),
    })
}
/// Where the first matching callback delivers its result.
type Winner = Arc<Mutex<Option<tokio::sync::oneshot::Sender<Result<Callback, String>>>>>;
/// One-shot loopback listener for the browser redirect.
pub struct Listener {
    listener: tokio::net::TcpListener,
    port: u16,
}
impl Listener {
    pub async fn bind() -> Result<Self, String> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|e| format!("Could not open a local port for sign-in: {e}"))?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        Ok(Self { listener, port })
    }
    pub fn redirect_uri(&self) -> String {
        format!("http://127.0.0.1:{}/callback", self.port)
    }
    /// Waits for this attempt's callback. Each connection is answered on its
    /// own task, so idle or slow connections cannot hold up the real one.
    pub async fn wait(self, state: &str, limit: Duration) -> Result<Callback, String> {
        let (sender, mut answer) = tokio::sync::oneshot::channel();
        let winner: Winner = Arc::new(Mutex::new(Some(sender)));
        let state: Arc<str> = state.into();
        let serve = async {
            loop {
                tokio::select! {
                    accepted = self.listener.accept() => {
                        let (socket, _) = accepted.map_err(|e| e.to_string())?;
                        tokio::spawn(answer_connection(socket, state.clone(), winner.clone()));
                    }
                    result = &mut answer => {
                        return result
                            .unwrap_or_else(|_| Err("Sign-in was interrupted. Try again.".into()));
                    }
                }
            }
        };
        tokio::time::timeout(limit, serve)
            .await
            .unwrap_or_else(|_| Err("Sign-in timed out. Try again.".into()))
    }
}
/// Answers one loopback connection. The first matching callback wins; every
/// other request gets 404.
async fn answer_connection(mut socket: tokio::net::TcpStream, state: Arc<str>, winner: Winner) {
    let Some(target) = request_target(&mut socket).await else {
        return;
    };
    let result = parse_callback(&target, &state);
    let sender = match result {
        Some(_) => winner.lock().ok().and_then(|mut slot| slot.take()),
        None => None,
    };
    match (result, sender) {
        (Some(result), Some(sender)) => {
            let text = if result.is_ok() {
                "You can return to AXIS CAM Studio to finish signing in."
            } else {
                "Sign-in was not completed. You can return to AXIS CAM Studio."
            };
            page(&mut socket, "200 OK", text).await;
            let _ = sender.send(result);
        }
        _ => page(&mut socket, "404 Not Found", "Not found.").await,
    }
}
async fn request_target(socket: &mut tokio::net::TcpStream) -> Option<String> {
    let mut data = vec![];
    let mut buf = [0u8; 2048];
    let read = async {
        while !data.windows(4).any(|w| w == b"\r\n\r\n") && data.len() < 16_384 {
            let n = socket.read(&mut buf).await.ok()?;
            if n == 0 {
                break;
            }
            data.extend_from_slice(&buf[..n]);
        }
        Some(())
    };
    tokio::time::timeout(Duration::from_secs(5), read)
        .await
        .ok()??;
    let head = String::from_utf8_lossy(&data);
    let mut first = head.lines().next()?.split(' ');
    (first.next()? == "GET").then(|| first.next().map(str::to_string))?
}
async fn page(socket: &mut tokio::net::TcpStream, status: &str, text: &str) {
    let body = format!(
        "<!doctype html><meta charset=\"utf-8\"><title>AXIS CAM Studio</title><p style=\"font:16px sans-serif;margin:3em\">{text}</p>"
    );
    let response = format!(
        "HTTP/1.1 {status}\r\ncontent-type: text/html; charset=utf-8\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = socket.write_all(response.as_bytes()).await;
    let _ = socket.shutdown().await;
}
#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    id_token: Option<String>,
    /// Seconds: an integer, a float or missing.
    expires_in: Option<f64>,
    scope: Option<String>,
}
impl TokenResponse {
    fn expires_at(&self) -> u64 {
        // `as` saturates: a negative lifetime becomes 0, a huge one u64::MAX.
        now().saturating_add(self.expires_in.map_or(3600, |seconds| seconds as u64))
    }
}
#[derive(Debug, Deserialize)]
struct IdClaims {
    sub: String,
    nonce: Option<String>,
    email: Option<String>,
    name: Option<String>,
}
/// The OAuth error code of a token-endpoint error body, in either common shape.
fn error_code(body: &Value) -> String {
    let error = &body["error"];
    error
        .as_str()
        .or(error["code"].as_str())
        .or(error["type"].as_str())
        .unwrap_or("unknown_error")
        .to_string()
}
/// Granted scopes: the token response's `scope`, else the access token's claim.
fn granted_scope(tokens: &TokenResponse) -> String {
    if let Some(scope) = &tokens.scope {
        return scope.clone();
    }
    let claim = tokens
        .access_token
        .split('.')
        .nth(1)
        .and_then(|p| {
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(p)
                .ok()
        })
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .map(|claims| claims["scope"].clone());
    match claim {
        Some(Value::String(scope)) => scope,
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(" "),
        _ => String::new(),
    }
}
async fn post_form(
    http: &reqwest::Client,
    url: &str,
    form: &[(&str, &str)],
) -> Result<(u16, Value), String> {
    let response = http
        .post(url)
        .form(form)
        .send()
        .await
        .map_err(|_| "Could not reach OpenAI.".to_string())?;
    let status = response.status().as_u16();
    let text = response.text().await.unwrap_or_default();
    Ok((status, serde_json::from_str(&text).unwrap_or(Value::Null)))
}
struct Identity {
    subject: String,
    email: String,
    name: String,
}
async fn validate_id_token(
    http: &reqwest::Client,
    e: &Endpoints,
    id_token: &str,
    client_id: &str,
    nonce: &str,
) -> Result<Identity, String> {
    use jsonwebtoken::{Algorithm, DecodingKey, Validation, jwk::JwkSet};
    let header = jsonwebtoken::decode_header(id_token).map_err(|_| "The ID token is malformed.")?;
    let jwks: JwkSet = http
        .get(&e.jwks)
        .send()
        .await
        .map_err(|_| "Could not reach OpenAI.".to_string())?
        .json()
        .await
        .map_err(|_| "OpenAI's signing keys could not be read.".to_string())?;
    let jwk = match &header.kid {
        Some(kid) => jwks.find(kid),
        None => jwks.keys.first(),
    }
    .ok_or("The ID token's signing key is unknown.")?;
    let key = DecodingKey::from_jwk(jwk).map_err(|_| "OpenAI's signing key is unusable.")?;
    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_issuer(&[&e.issuer]);
    validation.set_audience(&[client_id]);
    validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
    let claims = jsonwebtoken::decode::<IdClaims>(id_token, &key, &validation)
        .map_err(|e| format!("The ID token is not valid: {e}."))?
        .claims;
    if claims.nonce.as_deref() != Some(nonce) {
        return Err("The ID token does not belong to this sign-in attempt.".into());
    }
    Ok(Identity {
        subject: claims.sub,
        email: claims.email.unwrap_or_default(),
        name: claims.name.unwrap_or_default(),
    })
}
pub async fn exchange(
    http: &reqwest::Client,
    e: &Endpoints,
    client_id: &str,
    code: &str,
    attempt: &Attempt,
) -> Result<Session, String> {
    let (status, body) = post_form(
        http,
        &e.token,
        &[
            ("grant_type", "authorization_code"),
            ("client_id", client_id),
            ("code", code),
            ("code_verifier", &attempt.verifier),
            ("redirect_uri", &attempt.redirect_uri),
            ("resource", &e.api),
        ],
    )
    .await?;
    if status != 200 {
        return Err(format!(
            "OpenAI rejected the sign-in ({}).",
            error_code(&body)
        ));
    }
    let tokens: TokenResponse = serde_json::from_value(body)
        .map_err(|_| "OpenAI returned an unreadable sign-in response.".to_string())?;
    let id_token = tokens
        .id_token
        .clone()
        .ok_or("OpenAI returned no ID token.")?;
    let refresh_token = tokens
        .refresh_token
        .clone()
        .ok_or("OpenAI returned no refresh token.")?;
    let identity = validate_id_token(http, e, &id_token, client_id, &attempt.nonce).await?;
    Ok(Session {
        scope: granted_scope(&tokens),
        expires_at: tokens.expires_at(),
        access_token: tokens.access_token,
        refresh_token,
        id_token,
        subject: identity.subject,
        email: identity.email,
        name: identity.name,
    })
}
/// Rotate the tokens. The caller persists the result before using it.
pub async fn refresh(
    http: &reqwest::Client,
    e: &Endpoints,
    client_id: &str,
    session: &Session,
) -> Result<Session, AuthError> {
    let (status, body) = post_form(
        http,
        &e.token,
        &[
            ("grant_type", "refresh_token"),
            ("client_id", client_id),
            ("refresh_token", &session.refresh_token),
            ("resource", &e.api),
        ],
    )
    .await
    .map_err(AuthError::Retry)?;
    if status != 200 {
        let code = error_code(&body);
        if FATAL_REFRESH_CODES.contains(&code.as_str()) {
            return Err(AuthError::Fatal(format!(
                "Your ChatGPT sign-in is no longer valid ({code}). Sign in again."
            )));
        }
        return Err(AuthError::Retry("OpenAI is unavailable; try again.".into()));
    }
    // OpenAI has already rotated the refresh token, so trying again would
    // only reuse the old one.
    let tokens: TokenResponse = serde_json::from_value(body).map_err(|_| {
        AuthError::Fatal("OpenAI returned an unreadable sign-in response. Sign in again.".into())
    })?;
    let expires_at = tokens.expires_at();
    Ok(Session {
        scope: match granted_scope(&tokens) {
            scope if scope.is_empty() => session.scope.clone(),
            scope => scope,
        },
        access_token: tokens.access_token,
        refresh_token: tokens
            .refresh_token
            .unwrap_or_else(|| session.refresh_token.clone()),
        id_token: tokens.id_token.unwrap_or_else(|| session.id_token.clone()),
        expires_at,
        ..session.clone()
    })
}
pub async fn revoke(
    http: &reqwest::Client,
    e: &Endpoints,
    client_id: &str,
    refresh_token: &str,
) -> Result<(), String> {
    let (status, _) = post_form(
        http,
        &e.revoke,
        &[
            ("token", refresh_token),
            ("token_type_hint", "refresh_token"),
            ("client_id", client_id),
        ],
    )
    .await?;
    if status == 200 {
        Ok(())
    } else {
        Err(format!(
            "OpenAI did not confirm the sign-out (HTTP {status})."
        ))
    }
}
pub fn open_browser(url: &str) -> Result<(), String> {
    let mut command = if cfg!(target_os = "macos") {
        std::process::Command::new("open")
    } else if cfg!(target_os = "windows") {
        let mut c = std::process::Command::new("rundll32");
        c.arg("url.dll,FileProtocolHandler");
        c
    } else {
        std::process::Command::new("xdg-open")
    };
    command.arg(url);
    launch(command).map(|_| ())
}
/// Starts `command` and waits for it on a short-lived thread, so the child is
/// reaped instead of lingering as a zombie.
fn launch(
    mut command: std::process::Command,
) -> Result<std::thread::JoinHandle<Option<std::process::ExitStatus>>, String> {
    let mut child = command
        .spawn()
        .map_err(|e| format!("Could not open the browser: {e}"))?;
    Ok(std::thread::spawn(move || child.wait().ok()))
}
/// The whole sign-in: listen, send the user to the browser, wait for the
/// redirect, exchange the code and validate the ID token. Returns the session
/// and the client ID to remember.
pub async fn sign_in(
    http: &reqwest::Client,
    e: &Endpoints,
    settings: &Settings,
    id_token_hint: Option<&str>,
    limit: Duration,
    open: impl FnOnce(&str) -> Result<(), String>,
) -> Result<(Session, String), String> {
    let listener = Listener::bind().await?;
    let attempt = Attempt::new(listener.redirect_uri());
    open(&authorize_url(e, settings, &attempt, id_token_hint))?;
    let callback = listener.wait(&attempt.state, limit).await?;
    let client_id = callback
        .client_id
        .or_else(|| Some(settings.client_id.clone()).filter(|id| !id.is_empty()))
        .ok_or("OpenAI did not register this app. Try again.")?;
    let session = exchange(http, e, &client_id, &callback.code, &attempt).await?;
    Ok((session, client_id))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake_openai::{Fake, Req, Resp, claims, id_token, id_token_with, jwks, tokens};
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    fn settings(client_id: &str) -> Settings {
        Settings {
            host_id: "host-1".into(),
            client_id: client_id.into(),
            ..Default::default()
        }
    }
    fn query(url: &str, key: &str) -> String {
        reqwest::Url::parse(url)
            .unwrap()
            .query_pairs()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.into_owned())
            .unwrap_or_default()
    }
    /// A server that issues tokens whose ID token carries `claims`.
    async fn server(id_claims: Arc<Mutex<Value>>) -> Fake {
        Fake::start(move |r: &Req| match r.path.as_str() {
            "/jwks" => Resp::json(200, jwks()),
            "/token" => Resp::json(
                200,
                tokens(
                    "access-1",
                    "refresh-1",
                    &id_token(id_claims.lock().unwrap().clone()),
                    SCOPE,
                ),
            ),
            _ => Resp::json(404, json!({})),
        })
        .await
    }
    /// Plays the browser: follows the authorize URL straight to the callback.
    fn browser(
        extra: &'static str,
        nonce_out: Arc<Mutex<Value>>,
        client_id: &'static str,
    ) -> impl FnOnce(&str) -> Result<(), String> {
        move |url: &str| {
            *nonce_out.lock().unwrap() = claims(client_id, &query(url, "nonce"));
            let callback = format!(
                "{}?code=code-1&state={}{extra}",
                query(url, "redirect_uri"),
                query(url, "state")
            );
            tokio::spawn(async move { reqwest::get(callback).await });
            Ok(())
        }
    }
    #[test]
    fn authorize_url_registers_first_then_reuses_the_issued_client() {
        let e = Endpoints::openai();
        let attempt = Attempt::new("http://127.0.0.1:5555/callback".into());
        let first = authorize_url(&e, &settings(""), &attempt, Some("old-id-token"));
        assert!(first.starts_with("https://auth.openai.com/api/accounts/authorize?"));
        assert_eq!(query(&first, "client_id"), "dynamic_agent_client");
        assert_eq!(query(&first, "agent_name_hint"), "AXIS CAM Studio");
        assert_eq!(query(&first, "id_token_hint"), "");
        assert_eq!(query(&first, "ext_agent_host_id"), "host-1");
        assert_eq!(query(&first, "response_type"), "code");
        assert_eq!(
            query(&first, "redirect_uri"),
            "http://127.0.0.1:5555/callback"
        );
        assert_eq!(query(&first, "scope"), SCOPE);
        assert_eq!(query(&first, "resource"), "https://api.openai.com/v1");
        assert_eq!(query(&first, "code_challenge_method"), "S256");
        assert_eq!(
            query(&first, "code_challenge"),
            challenge(&attempt.verifier)
        );
        assert_eq!(query(&first, "state"), attempt.state);
        assert_eq!(query(&first, "nonce"), attempt.nonce);
        let again = authorize_url(&e, &settings("oaiapp_1"), &attempt, Some("old-id-token"));
        assert_eq!(query(&again, "client_id"), "oaiapp_1");
        assert_eq!(query(&again, "agent_name_hint"), "");
        assert_eq!(query(&again, "id_token_hint"), "old-id-token");
    }
    #[test]
    fn pkce_values_are_fresh_and_match_the_rfc_example() {
        let a = Attempt::new(String::new());
        let b = Attempt::new(String::new());
        assert!(a.state != b.state && a.nonce != b.nonce && a.verifier != b.verifier);
        assert_eq!(a.verifier.len(), 43);
        // RFC 7636 appendix B
        assert_eq!(
            challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }
    #[test]
    fn callbacks_for_other_paths_or_states_are_ignored() {
        assert_eq!(parse_callback("/favicon.ico", "s"), None);
        assert_eq!(parse_callback("/callback?code=c&state=other", "s"), None);
        assert_eq!(parse_callback("/callback?code=c", "s"), None);
        assert_eq!(
            parse_callback("/callback?code=c&state=s&client_id=oaiapp_9", "s"),
            Some(Ok(Callback {
                code: "c".into(),
                client_id: Some("oaiapp_9".into())
            }))
        );
        assert_eq!(
            parse_callback("/callback?code=c&state=s", "s"),
            Some(Ok(Callback {
                code: "c".into(),
                client_id: None
            }))
        );
        assert!(
            parse_callback("/callback?error=access_denied&state=s", "s")
                .unwrap()
                .unwrap_err()
                .contains("cancelled")
        );
        assert!(parse_callback("/callback?state=s", "s").unwrap().is_err());
    }
    #[tokio::test]
    async fn first_sign_in_registers_exchanges_and_validates() {
        let id_claims = Arc::new(Mutex::new(json!({})));
        let fake = server(id_claims.clone()).await;
        let e = Endpoints::local(&fake.base);
        let http = reqwest::Client::new();
        let (session, client_id) = sign_in(
            &http,
            &e,
            &settings(""),
            None,
            Duration::from_secs(5),
            browser("&client_id=oaiapp_new", id_claims, "oaiapp_new"),
        )
        .await
        .unwrap();
        assert_eq!(client_id, "oaiapp_new");
        assert_eq!(
            (session.email.as_str(), session.name.as_str()),
            ("lior@example.com", "Lior")
        );
        assert_eq!(
            (
                session.access_token.as_str(),
                session.refresh_token.as_str()
            ),
            ("access-1", "refresh-1")
        );
        assert!(session.plan_usage_granted());
        assert!(session.expires_at > now() + 3500);
        let token = &fake.seen("/token")[0];
        assert_eq!(token.field("grant_type"), "authorization_code");
        assert_eq!(token.field("client_id"), "oaiapp_new");
        assert_eq!(token.field("code"), "code-1");
        assert_eq!(token.field("resource"), e.api);
        assert!(token.field("redirect_uri").ends_with("/callback"));
        assert_eq!(token.field("code_verifier").len(), 43);
        assert_eq!(token.field("client_secret"), "");
    }
    #[tokio::test]
    async fn later_sign_ins_reuse_the_saved_client() {
        let id_claims = Arc::new(Mutex::new(json!({})));
        let fake = server(id_claims.clone()).await;
        let (_, client_id) = sign_in(
            &reqwest::Client::new(),
            &Endpoints::local(&fake.base),
            &settings("oaiapp_saved"),
            None,
            Duration::from_secs(5),
            browser("", id_claims, "oaiapp_saved"),
        )
        .await
        .unwrap();
        assert_eq!(client_id, "oaiapp_saved");
    }
    #[tokio::test]
    async fn sign_in_fails_on_denial_timeout_and_unopened_browser() {
        let id_claims = Arc::new(Mutex::new(json!({})));
        let fake = server(id_claims.clone()).await;
        let e = Endpoints::local(&fake.base);
        let http = reqwest::Client::new();
        let denied = sign_in(
            &http,
            &e,
            &settings("oaiapp_1"),
            None,
            Duration::from_secs(5),
            |url: &str| {
                let callback = format!(
                    "{}?error=access_denied&state={}",
                    query(url, "redirect_uri"),
                    query(url, "state")
                );
                tokio::spawn(async move { reqwest::get(callback).await });
                Ok(())
            },
        )
        .await;
        assert!(denied.unwrap_err().contains("cancelled"));
        let ignored = sign_in(
            &http,
            &e,
            &settings("oaiapp_1"),
            None,
            Duration::from_millis(300),
            |url: &str| {
                let callback = format!("{}?code=stolen&state=wrong", query(url, "redirect_uri"));
                tokio::spawn(async move { reqwest::get(callback).await });
                Ok(())
            },
        )
        .await;
        assert!(ignored.unwrap_err().contains("timed out"));
        let unopened = sign_in(
            &http,
            &e,
            &settings("oaiapp_1"),
            None,
            Duration::from_secs(5),
            |_: &str| Err("Could not open the browser: none".into()),
        )
        .await;
        assert!(unopened.unwrap_err().contains("browser"));
        assert!(fake.seen("/token").is_empty());
    }
    #[tokio::test]
    async fn id_tokens_must_match_key_audience_nonce_issuer_and_time() {
        let cases: Vec<(&str, Box<dyn Fn(Value) -> Value + Send + Sync>)> = vec![
            (
                "audience",
                Box::new(|mut c| {
                    c["aud"] = json!("someone-else");
                    c
                }),
            ),
            (
                "nonce",
                Box::new(|mut c| {
                    c["nonce"] = json!("replayed");
                    c
                }),
            ),
            (
                "issuer",
                Box::new(|mut c| {
                    c["iss"] = json!("https://evil.example");
                    c
                }),
            ),
            (
                "expired",
                Box::new(|mut c| {
                    c["exp"] = json!(now() - 600);
                    c
                }),
            ),
        ];
        for (name, edit) in cases {
            let id_claims = Arc::new(Mutex::new(json!({})));
            let fake = server(id_claims.clone()).await;
            let seen = id_claims.clone();
            let result = sign_in(
                &reqwest::Client::new(),
                &Endpoints::local(&fake.base),
                &settings("oaiapp_1"),
                None,
                Duration::from_secs(5),
                move |url: &str| {
                    *seen.lock().unwrap() = edit(claims("oaiapp_1", &query(url, "nonce")));
                    let callback = format!(
                        "{}?code=c&state={}",
                        query(url, "redirect_uri"),
                        query(url, "state")
                    );
                    tokio::spawn(async move { reqwest::get(callback).await });
                    Ok(())
                },
            )
            .await;
            assert!(result.is_err(), "{name}");
        }
        // A token whose signature does not verify.
        let fake = Fake::start(|r: &Req| match r.path.as_str() {
            "/jwks" => Resp::json(200, jwks()),
            _ => {
                let mut forged = id_token(claims("oaiapp_1", "n"));
                forged.push_str("AAAA");
                Resp::json(200, tokens("a", "r", &forged, SCOPE))
            }
        })
        .await;
        let attempt = Attempt {
            state: "s".into(),
            nonce: "n".into(),
            verifier: "v".into(),
            redirect_uri: "http://127.0.0.1:1/callback".into(),
        };
        let forged = exchange(
            &reqwest::Client::new(),
            &Endpoints::local(&fake.base),
            "oaiapp_1",
            "c",
            &attempt,
        )
        .await;
        assert!(forged.unwrap_err().contains("not valid"));
    }
    #[tokio::test]
    async fn scope_without_plan_usage_is_kept_but_not_granted() {
        let fake = Fake::start(|r: &Req| match r.path.as_str() {
            "/jwks" => Resp::json(200, jwks()),
            _ => Resp::json(
                200,
                tokens(
                    "a",
                    "r",
                    &id_token(claims("oaiapp_1", "n")),
                    "openid profile email offline_access",
                ),
            ),
        })
        .await;
        let attempt = Attempt {
            state: "s".into(),
            nonce: "n".into(),
            verifier: "v".into(),
            redirect_uri: "http://127.0.0.1:1/callback".into(),
        };
        let session = exchange(
            &reqwest::Client::new(),
            &Endpoints::local(&fake.base),
            "oaiapp_1",
            "c",
            &attempt,
        )
        .await
        .unwrap();
        assert!(!session.plan_usage_granted());
    }
    #[tokio::test]
    async fn refresh_rotates_tokens_and_classifies_failures() {
        let fake = Fake::start(|r: &Req| match r.field("refresh_token").as_str() {
            "refresh-1" => Resp::json(200, json!({"access_token": "access-2", "refresh_token": "refresh-2", "expires_in": 3600})),
            "down" => Resp::json(503, json!({})),
            code => Resp::json(400, json!({"error": code})),
        })
        .await;
        let e = Endpoints::local(&fake.base);
        let http = reqwest::Client::new();
        let old = crate::session::sample();
        let new = refresh(&http, &e, "oaiapp_1", &old).await.unwrap();
        assert_eq!(
            (new.access_token.as_str(), new.refresh_token.as_str()),
            ("access-2", "refresh-2")
        );
        assert_eq!(
            (
                new.id_token.as_str(),
                new.email.as_str(),
                new.scope.as_str()
            ),
            (
                old.id_token.as_str(),
                old.email.as_str(),
                old.scope.as_str()
            )
        );
        let sent = &fake.seen("/token")[0];
        assert_eq!(sent.field("grant_type"), "refresh_token");
        assert_eq!(sent.field("client_id"), "oaiapp_1");
        assert_eq!(sent.field("resource"), e.api);
        for code in [
            "invalid_grant",
            "token_expired",
            "refresh_token_expired",
            "refresh_token_reused",
            "invalid_client",
        ] {
            let bad = Session {
                refresh_token: code.into(),
                ..old.clone()
            };
            assert!(
                matches!(refresh(&http, &e, "oaiapp_1", &bad).await, Err(AuthError::Fatal(m)) if m.contains(code)),
                "{code}"
            );
        }
        let down = Session {
            refresh_token: "down".into(),
            ..old.clone()
        };
        assert!(matches!(
            refresh(&http, &e, "oaiapp_1", &down).await,
            Err(AuthError::Retry(_))
        ));
        let unreachable = Endpoints::local("http://127.0.0.1:9");
        assert!(matches!(
            refresh(&http, &unreachable, "oaiapp_1", &old).await,
            Err(AuthError::Retry(_))
        ));
    }
    #[tokio::test]
    async fn refresh_retries_other_failures_and_is_fatal_on_an_unreadable_success() {
        let fake = Fake::start(|r: &Req| match r.field("refresh_token").as_str() {
            "busy" => Resp::json(429, json!({"error": {"code": "rate_limit_exceeded"}})),
            "forbidden" => Resp::json(403, json!({})),
            "float" => Resp::json(200, json!({"access_token": "a", "expires_in": 1799.5})),
            "missing" => Resp::json(200, json!({"access_token": "a"})),
            _ => Resp::json(200, json!({"refresh_token": "rotated-but-unreadable"})),
        })
        .await;
        let e = Endpoints::local(&fake.base);
        let http = reqwest::Client::new();
        let with = |token: &str| Session {
            refresh_token: token.into(),
            ..crate::session::sample()
        };
        for retry in ["busy", "forbidden"] {
            assert!(
                matches!(
                    refresh(&http, &e, "oaiapp_1", &with(retry)).await,
                    Err(AuthError::Retry(_))
                ),
                "{retry}"
            );
        }
        let before = now();
        let float = refresh(&http, &e, "oaiapp_1", &with("float"))
            .await
            .unwrap();
        let missing = refresh(&http, &e, "oaiapp_1", &with("missing"))
            .await
            .unwrap();
        let after = now();
        assert!((before + 1799..=after + 1799).contains(&float.expires_at));
        assert!((before + 3600..=after + 3600).contains(&missing.expires_at));
        assert_eq!(missing.refresh_token, "missing");
        assert_eq!(
            refresh(&http, &e, "oaiapp_1", &with("unreadable")).await,
            Err(AuthError::Fatal(
                "OpenAI returned an unreadable sign-in response. Sign in again.".into()
            ))
        );
    }
    #[tokio::test]
    async fn idle_connections_do_not_hold_up_the_callback() {
        let listener = Listener::bind().await.unwrap();
        let callback = listener.redirect_uri();
        let address = callback
            .trim_start_matches("http://")
            .trim_end_matches("/callback")
            .to_string();
        let mut idle = vec![];
        for _ in 0..3 {
            idle.push(tokio::net::TcpStream::connect(&address).await.unwrap());
        }
        let started = std::time::Instant::now();
        let browser = tokio::spawn(reqwest::get(format!("{callback}?code=c&state=s")));
        let result = listener.wait("s", Duration::from_secs(3)).await;
        assert_eq!(result.map(|c| c.code), Ok("c".to_string()));
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "{:?}",
            started.elapsed()
        );
        assert_eq!(browser.await.unwrap().unwrap().status(), 200);
        drop(idle);
    }
    #[tokio::test]
    async fn the_browser_page_does_not_claim_the_sign_in_is_finished() {
        let listener = Listener::bind().await.unwrap();
        let browser = tokio::spawn(reqwest::get(format!(
            "{}?code=c&state=s",
            listener.redirect_uri()
        )));
        listener.wait("s", Duration::from_secs(5)).await.unwrap();
        let page = browser.await.unwrap().unwrap().text().await.unwrap();
        assert!(
            page.contains("You can return to AXIS CAM Studio to finish signing in."),
            "{page}"
        );
        let listener = Listener::bind().await.unwrap();
        let browser = tokio::spawn(reqwest::get(format!(
            "{}?error=access_denied&state=s",
            listener.redirect_uri()
        )));
        assert!(listener.wait("s", Duration::from_secs(5)).await.is_err());
        let page = browser.await.unwrap().unwrap().text().await.unwrap();
        assert!(
            page.contains("Sign-in was not completed. You can return to AXIS CAM Studio."),
            "{page}"
        );
    }
    #[test]
    fn debug_output_hides_the_authorization_code_and_pkce_values() {
        let callback = Callback {
            code: "secret-code".into(),
            client_id: Some("oaiapp_9".into()),
        };
        let text = format!("{callback:?}");
        assert!(!text.contains("secret-code"), "{text}");
        assert!(
            text.contains("code: \"<redacted>\"") && text.contains("oaiapp_9"),
            "{text}"
        );
        // `Attempt` holds the PKCE verifier and nonce, so it has no Debug at all.
        struct Probe<T>(std::marker::PhantomData<T>);
        trait NoDebug {
            const DEBUG: bool = false;
        }
        impl<T> NoDebug for Probe<T> {}
        impl<T: std::fmt::Debug> Probe<T> {
            const DEBUG: bool = true;
        }
        assert!(Probe::<Callback>::DEBUG, "the probe detects Debug");
        assert!(!Probe::<Attempt>::DEBUG, "Attempt must not implement Debug");
    }
    #[tokio::test]
    async fn id_tokens_need_rs256_and_may_omit_kid_with_a_single_key() {
        let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::HS256);
        header.kid = Some("test-key".into());
        let shared_secret = jsonwebtoken::encode(
            &header,
            &claims("oaiapp_1", "n"),
            &jsonwebtoken::EncodingKey::from_secret(b"any shared secret"),
        )
        .unwrap();
        let without_kid = id_token_with(
            jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256),
            claims("oaiapp_1", "n"),
        );
        assert!(
            jsonwebtoken::decode_header(&without_kid)
                .unwrap()
                .kid
                .is_none()
        );
        let fake = Fake::start(
            move |r: &Req| match (r.path.as_str(), r.field("code").as_str()) {
                ("/jwks", _) => Resp::json(200, jwks()),
                (_, "hs256") => Resp::json(200, tokens("a", "r", &shared_secret, SCOPE)),
                _ => Resp::json(200, tokens("a", "r", &without_kid, SCOPE)),
            },
        )
        .await;
        assert_eq!(jwks()["keys"].as_array().unwrap().len(), 1);
        let attempt = Attempt {
            state: "s".into(),
            nonce: "n".into(),
            verifier: "v".into(),
            redirect_uri: "http://127.0.0.1:1/callback".into(),
        };
        let http = reqwest::Client::new();
        let e = Endpoints::local(&fake.base);
        let rejected = exchange(&http, &e, "oaiapp_1", "hs256", &attempt).await;
        assert!(rejected.unwrap_err().contains("not valid"));
        let session = exchange(&http, &e, "oaiapp_1", "unkeyed", &attempt)
            .await
            .unwrap();
        assert_eq!(session.subject, "user-1");
    }
    #[tokio::test]
    async fn the_http_client_never_follows_redirects_and_times_out() {
        let fake = Fake::start(|r: &Req| match r.path.as_str() {
            "/start" => Resp::redirect("/elsewhere"),
            "/slow" => Resp {
                delay_ms: 1000,
                ..Resp::json(200, json!({}))
            },
            _ => Resp::json(200, json!({})),
        })
        .await;
        let http = http_client(Duration::from_millis(200));
        let response = http
            .get(format!("{}/start", fake.base))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status().as_u16(), 302);
        assert!(fake.seen("/elsewhere").is_empty());
        let slow = http.get(format!("{}/slow", fake.base)).send().await;
        assert!(slow.unwrap_err().is_timeout());
    }
    #[cfg(unix)]
    #[test]
    fn launched_commands_are_reaped() {
        // `true` stands in for the browser opener; no browser is opened.
        let reaper = launch(std::process::Command::new("true")).unwrap();
        assert!(
            reaper
                .join()
                .unwrap()
                .is_some_and(|status| status.success())
        );
        let missing = launch(std::process::Command::new("/nonexistent/axis-cam-opener"));
        assert!(missing.unwrap_err().contains("Could not open the browser"));
    }
    #[tokio::test]
    async fn revoke_sends_the_refresh_token_and_reports_failure() {
        let fake = Fake::start(|r: &Req| {
            Resp::json(
                if r.field("token") == "refresh-1" {
                    200
                } else {
                    500
                },
                json!({}),
            )
        })
        .await;
        let e = Endpoints::local(&fake.base);
        let http = reqwest::Client::new();
        revoke(&http, &e, "oaiapp_1", "refresh-1").await.unwrap();
        let sent = &fake.seen("/revoke")[0];
        assert_eq!(
            (sent.field("token_type_hint"), sent.field("client_id")),
            ("refresh_token".into(), "oaiapp_1".into())
        );
        assert!(
            revoke(&http, &e, "oaiapp_1", "other")
                .await
                .unwrap_err()
                .contains("not confirm")
        );
    }
}
