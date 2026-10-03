//! Test-only stand-in for OpenAI's sign-in and API servers on a loopback port.
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Test-only RSA key. It signs ID tokens for the fake server and nothing else.
const KEY: &str = include_str!("fake_openai_key.pem");
const JWKS: &str = include_str!("fake_openai_jwks.json");
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
}
impl Resp {
    pub fn json(status: u16, value: Value) -> Self {
        Self {
            status,
            content_type: "application/json",
            body: value.to_string(),
        }
    }
    pub fn sse(events: &[Value]) -> Self {
        Self {
            status: 200,
            content_type: "text/event-stream",
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
}
impl Fake {
    pub async fn start(handler: impl Fn(&Req) -> Resp + Send + Sync + 'static) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(Mutex::new(vec![]));
        let log = requests.clone();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let Some(request) = read(&mut socket).await else {
                    continue;
                };
                let response = handler(&request);
                log.lock().unwrap().push(request);
                let head = format!(
                    "HTTP/1.1 {} X\r\ncontent-type: {}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                    response.status,
                    response.content_type,
                    response.body.len()
                );
                let _ = socket.write_all(head.as_bytes()).await;
                let _ = socket.write_all(response.body.as_bytes()).await;
            }
        });
        Self { base, requests }
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
    serde_json::from_str(JWKS).unwrap()
}
pub fn id_token(claims: Value) -> String {
    let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
    header.kid = Some("test-key".into());
    let key = jsonwebtoken::EncodingKey::from_rsa_pem(KEY.as_bytes()).unwrap();
    jsonwebtoken::encode(&header, &claims, &key).unwrap()
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
