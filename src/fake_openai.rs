//! Test-only stand-in for OpenAI's sign-in and API servers on a loopback port.
use serde_json::{Value, json};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Test-only RSA key, generated on first use and never written to disk. It
/// signs ID tokens for the fake server and nothing else.
static KEY: OnceLock<(jsonwebtoken::EncodingKey, Value)> = OnceLock::new();
fn key() -> &'static (jsonwebtoken::EncodingKey, Value) {
    KEY.get_or_init(|| {
        use aws_lc_rs::{encoding::AsDer, rsa::KeyPair, rsa::KeySize};
        use base64::{Engine, engine::general_purpose::STANDARD};
        let der = KeyPair::generate(KeySize::Rsa2048)
            .unwrap()
            .as_der()
            .unwrap();
        let body = STANDARD.encode(der.as_ref());
        let pem = format!("-----BEGIN PRIVATE KEY-----\n{body}\n-----END PRIVATE KEY-----\n");
        let key = jsonwebtoken::EncodingKey::from_rsa_pem(pem.as_bytes()).unwrap();
        let jwk = jsonwebtoken::jwk::Jwk::from_encoding_key(&key, jsonwebtoken::Algorithm::RS256)
            .unwrap();
        let mut jwk = serde_json::to_value(jwk).unwrap();
        jwk["kid"] = "test-key".into();
        jwk["use"] = "sig".into();
        (key, json!({ "keys": [jwk] }))
    })
}
pub const ISSUER: &str = "https://auth.openai.com";

#[derive(Debug, Clone)]
pub struct Req {
    pub path: String,
    pub query: String,
    pub auth: String,
    pub body: String,
}
impl Req {
    /// A form-encoded body or query value.
    pub fn field(&self, key: &str) -> String {
        let all = format!("http://x/?{}&{}", self.query, self.body);
        reqwest::Url::parse(&all)
            .unwrap()
            .query_pairs()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.into_owned())
            .unwrap_or_default()
    }
    pub fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or(Value::Null)
    }
}
pub struct Resp {
    pub status: u16,
    pub content_type: &'static str,
    pub body: String,
    /// Wait this long after recording the request before answering.
    pub delay_ms: u64,
    /// Close the connection without answering.
    pub hangup: bool,
    /// A `location` header, for redirects.
    pub location: Option<String>,
    /// Promise more body than is sent, keep the connection open this long
    /// and then drop it: a reply that stalls or, with 0, is cut off.
    pub cut_after_ms: Option<u64>,
}
impl Resp {
    pub fn json(status: u16, value: Value) -> Self {
        Self {
            status,
            content_type: "application/json",
            body: value.to_string(),
            delay_ms: 0,
            hangup: false,
            location: None,
            cut_after_ms: None,
        }
    }
    /// Reads the request, then hangs up without sending a response.
    pub fn hangup() -> Self {
        Self {
            hangup: true,
            ..Self::json(200, json!({}))
        }
    }
    pub fn redirect(location: &str) -> Self {
        Self {
            location: Some(location.into()),
            ..Self::json(302, json!({}))
        }
    }
    pub fn sse(events: &[Value]) -> Self {
        Self {
            status: 200,
            content_type: "text/event-stream",
            delay_ms: 0,
            hangup: false,
            location: None,
            cut_after_ms: None,
            body: events
                .iter()
                .map(|e| {
                    format!(
                        "event: {}\ndata: {e}\n\n",
                        e["type"].as_str().unwrap_or("message")
                    )
                })
                .collect(),
        }
    }
    /// A completed response whose text arrives in two deltas.
    pub fn answer(text: &str) -> Self {
        let (a, b) = text.split_at(text.len() / 2);
        Self::sse(&[
            json!({"type": "response.created"}),
            json!({"type": "response.output_text.delta", "delta": a}),
            json!({"type": "response.output_text.delta", "delta": b}),
            json!({"type": "response.completed", "response": {"status": "completed"}}),
        ])
    }
    pub fn error(status: u16, code: &str) -> Self {
        Self::json(
            status,
            json!({"error": {"code": code, "message": format!("{code} message")}}),
        )
    }
}
pub struct Fake {
    pub base: String,
    pub requests: Arc<Mutex<Vec<Req>>>,
    answered: Arc<AtomicUsize>,
    closed: Arc<AtomicUsize>,
}
/// Counts a connection closed when its task ends, however it ends.
struct Closing(Arc<AtomicUsize>);
impl Drop for Closing {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
impl Fake {
    pub async fn start(handler: impl Fn(&Req) -> Resp + Send + Sync + 'static) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(vec![]));
        let (answered, closed) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
        let log = requests.clone();
        let (answers, closes) = (answered.clone(), closed.clone());
        let handler = Arc::new(handler);
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let handler = handler.clone();
                let log = log.clone();
                let answers = answers.clone();
                let closing = Closing(closes.clone());
                // Each connection on its own task, so a delayed answer does
                // not hold up other requests.
                tokio::spawn(async move {
                    let _closing = closing;
                    let Some(request) = read(&mut socket).await else {
                        return;
                    };
                    let response = handler(&request);
                    log.lock().unwrap().push(request);
                    if response.delay_ms > 0 {
                        tokio::time::sleep(std::time::Duration::from_millis(response.delay_ms))
                            .await;
                    }
                    if response.hangup {
                        return;
                    }
                    let location = response
                        .location
                        .as_ref()
                        .map(|l| format!("location: {l}\r\n"))
                        .unwrap_or_default();
                    let head = format!(
                        "HTTP/1.1 {} X\r\ncontent-type: {}\r\ncontent-length: {}\r\n{location}connection: close\r\n\r\n",
                        response.status,
                        response.content_type,
                        response.body.len() + usize::from(response.cut_after_ms.is_some())
                    );
                    let _ = socket.write_all(head.as_bytes()).await;
                    let _ = socket.write_all(response.body.as_bytes()).await;
                    answers.fetch_add(1, Ordering::SeqCst);
                    if let Some(ms) = response.cut_after_ms {
                        tokio::time::sleep(std::time::Duration::from_millis(ms)).await;
                    }
                });
            }
        });
        Self {
            base,
            requests,
            answered,
            closed,
        }
    }
    /// Requests the fake has answered (head and body written) so far: a
    /// reply the client gave before that is its own, such as a limit's.
    pub fn answered(&self) -> usize {
        self.answered.load(Ordering::SeqCst)
    }
    /// Connections the fake has finished with so far, a held one once its
    /// hold is over: a reply the client gave before that did not wait for
    /// the close.
    pub fn closed(&self) -> usize {
        self.closed.load(Ordering::SeqCst)
    }
    pub fn seen(&self, path: &str) -> Vec<Req> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|r| r.path == path)
            .cloned()
            .collect()
    }
}
async fn read(socket: &mut tokio::net::TcpStream) -> Option<Req> {
    let mut data = vec![];
    let mut buf = [0u8; 4096];
    let end = loop {
        let n = socket.read(&mut buf).await.ok()?;
        if n == 0 {
            return None;
        }
        data.extend_from_slice(&buf[..n]);
        if let Some(end) = data.windows(4).position(|w| w == b"\r\n\r\n") {
            break end + 4;
        }
    };
    let head = String::from_utf8_lossy(&data[..end]).into_owned();
    let header = |name: &str| {
        head.lines()
            .find_map(|l| {
                l.split_once(':')
                    .filter(|(k, _)| k.eq_ignore_ascii_case(name))
            })
            .map(|(_, v)| v.trim().to_string())
            .unwrap_or_default()
    };
    let length: usize = header("content-length").parse().unwrap_or(0);
    while data.len() < end + length {
        let n = socket.read(&mut buf).await.ok()?;
        if n == 0 {
            break;
        }
        data.extend_from_slice(&buf[..n]);
    }
    let target = head.lines().next()?.split(' ').nth(1)?;
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    Some(Req {
        path: path.into(),
        query: query.into(),
        auth: header("authorization"),
        body: String::from_utf8_lossy(&data[end..]).into_owned(),
    })
}
pub fn jwks() -> Value {
    key().1.clone()
}
pub fn id_token(claims: Value) -> String {
    let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
    header.kid = Some("test-key".into());
    id_token_with(header, claims)
}
/// An ID token signed with the test key under any header (for example one
/// without `kid`).
pub fn id_token_with(header: jsonwebtoken::Header, claims: Value) -> String {
    jsonwebtoken::encode(&header, &claims, &key().0).unwrap()
}
/// Standard ID token claims for `client_id` and `nonce`, valid for ten minutes.
pub fn claims(client_id: &str, nonce: &str) -> Value {
    json!({
        "iss": ISSUER,
        "aud": client_id,
        "sub": "user-1",
        "exp": crate::session::now() + 600,
        "nonce": nonce,
        "email": "lior@example.com",
        "name": "Lior",
    })
}
pub fn tokens(access: &str, refresh: &str, id_token: &str, scope: &str) -> Value {
    json!({
        "access_token": access,
        "refresh_token": refresh,
        "id_token": id_token,
        "token_type": "Bearer",
        "expires_in": 3600,
        "scope": scope,
    })
}
