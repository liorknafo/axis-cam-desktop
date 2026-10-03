//! OpenAI Responses API client for requests billed to the user's ChatGPT plan.
use crate::{
    oauth::{self, AuthError, Endpoints},
    session::{Session, SessionStore, now},
};
use futures_util::StreamExt;
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};

pub const USAGE_URL: &str = "https://chatgpt.com/settings/usage";

#[derive(Debug, Clone, PartialEq)]
pub enum ApiError {
    /// The session is gone or was rejected; the user must sign in again.
    SignedOut(String),
    UsageLimit,
    NotEligible(String),
    /// Network trouble or a server error, after retries.
    Unavailable(String),
    Refused(String),
    /// The plan route does not support a requested feature.
    Unsupported(String),
    Other(String),
}
impl ApiError {
    pub fn message(&self) -> String {
        match self {
            Self::UsageLimit => {
                "Usage limit reached. Manage usage in your ChatGPT settings.".into()
            }
            Self::NotEligible(m) => format!("Your ChatGPT plan cannot be used in this app. {m}"),
            Self::Refused(m) => format!("The model declined the request: {m}"),
            Self::SignedOut(m) | Self::Unavailable(m) | Self::Unsupported(m) | Self::Other(m) => {
                m.clone()
            }
        }
    }
}
/// `status` is 0 for an `error` or `response.failed` event in a stream.
fn api_error(status: u16, body: &Value) -> ApiError {
    let error = &body["error"];
    let code = error["code"]
        .as_str()
        .or(error["type"].as_str())
        .unwrap_or("");
    let message = error["message"].as_str().unwrap_or("").to_string();
    match code {
        "subscription_sharing_usage_limit_exceeded" => ApiError::UsageLimit,
        "subscription_sharing_user_not_eligible" => ApiError::NotEligible(message),
        "subscription_sharing_usage_unavailable" => {
            ApiError::Unavailable("OpenAI is unavailable; try again.".into())
        }
        "subscription_sharing_unsupported_capability" => ApiError::Unsupported(message),
        "subscription_sharing_invalid_user" => {
            ApiError::SignedOut("Your ChatGPT sign-in was disconnected. Sign in again.".into())
        }
        _ if status == 401 => {
            ApiError::SignedOut("Your ChatGPT sign-in was rejected. Sign in again.".into())
        }
        _ if message.is_empty() && status == 0 => {
            ApiError::Other("OpenAI reported an error.".into())
        }
        _ if message.is_empty() => ApiError::Other(format!("OpenAI returned HTTP {status}.")),
        _ => ApiError::Other(message),
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    System,
    User,
}
#[derive(Debug, Clone)]
pub struct Request {
    pub model: String,
    pub messages: Vec<(Role, String)>,
    pub schema_name: &'static str,
    pub schema: Value,
}
impl Request {
    /// `structured` asks the API to enforce the schema. Without it the schema
    /// is restated in the instructions; replies are validated locally either way.
    fn body(&self, structured: bool) -> Value {
        let mut input: Vec<Value> = self
            .messages
            .iter()
            .map(|(role, text)| {
                let role = match role {
                    Role::System => "system",
                    Role::User => "user",
                };
                json!({"role": role, "content": text})
            })
            .collect();
        let mut body = json!({"model": self.model, "store": false, "stream": true});
        if structured {
            body["text"] = json!({"format": {
                "type": "json_schema",
                "name": self.schema_name,
                "schema": self.schema,
                "strict": true,
            }});
        } else {
            input.push(json!({"role": "system", "content": format!(
                "Reply with only one JSON object that matches this JSON Schema, and no other text:\n{}",
                self.schema
            )}));
        }
        body["input"] = Value::Array(input);
        body
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct Reply {
    pub text: String,
    /// Whether the API enforced the schema.
    pub structured: bool,
}
/// The most the parser buffers while waiting for an event to end.
const MAX_SSE_BUFFER: usize = 16 * 1024 * 1024;
/// The JSON of one event's `data:` lines, if any.
fn sse_event(block: &[u8]) -> Option<Value> {
    let data: Vec<&str> = std::str::from_utf8(block)
        .unwrap_or("")
        .lines()
        .filter_map(|l| l.strip_prefix("data:"))
        .map(str::trim_start)
        .collect();
    serde_json::from_str(&data.join("\n")).ok()
}
/// Splits a server-sent-event byte stream into the JSON of each `data:` event.
#[derive(Default)]
pub struct SseParser {
    buffer: Vec<u8>,
    /// How much of `buffer` has already been searched for an event end.
    scanned: usize,
    too_large: bool,
}
impl SseParser {
    /// The events completed by `chunk`. Call [`SseParser::finish`] when the
    /// stream ends: it reports a reply that was too large and returns a final
    /// event that had no trailing blank line.
    pub fn push(&mut self, chunk: &[u8]) -> Vec<Value> {
        if self.too_large {
            return vec![];
        }
        self.buffer.extend(chunk.iter().filter(|b| **b != b'\r'));
        if self.buffer.len() > MAX_SSE_BUFFER {
            self.too_large = true;
            self.buffer = Vec::new();
            self.scanned = 0;
            return vec![];
        }
        let mut events = vec![];
        let mut start = 0;
        // Only new bytes are searched; the last scanned byte is searched again
        // because a blank line can straddle two chunks.
        let mut from = self.scanned.saturating_sub(1);
        while let Some(i) = self.buffer[from..].windows(2).position(|w| w == b"\n\n") {
            let end = from + i + 2;
            events.extend(sse_event(&self.buffer[start..end]));
            start = end;
            from = end;
        }
        self.buffer.drain(..start);
        self.scanned = self.buffer.len();
        events
    }
    /// The last event when the stream ended without its blank line. `Err`
    /// once the reply has outgrown the buffer.
    pub fn finish(&mut self) -> Result<Option<Value>, ApiError> {
        if self.too_large {
            return Err(ApiError::Other("The reply was too large.".into()));
        }
        self.scanned = 0;
        Ok(sse_event(&std::mem::take(&mut self.buffer)))
    }
}
/// Folds response events into the reply text or the first error.
#[derive(Default)]
struct Accumulator {
    text: String,
    refusal: String,
    done: bool,
    error: Option<ApiError>,
}
impl Accumulator {
    fn apply(&mut self, event: &Value) {
        match event["type"].as_str().unwrap_or("") {
            "response.output_text.delta" => self.text += event["delta"].as_str().unwrap_or(""),
            "response.refusal.delta" => self.refusal += event["delta"].as_str().unwrap_or(""),
            "response.completed" => {
                if self.text.is_empty() {
                    let parts = event["response"]["output"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .flat_map(|item| item["content"].as_array().into_iter().flatten());
                    for part in parts {
                        if part["type"] == "output_text" {
                            self.text += part["text"].as_str().unwrap_or("");
                        }
                    }
                }
                self.done = true;
            }
            "response.failed" => {
                self.error
                    .get_or_insert(api_error(0, &json!({"error": event["response"]["error"]})));
            }
            "response.incomplete" => {
                let reason = event["response"]["incomplete_details"]["reason"]
                    .as_str()
                    .unwrap_or("unknown reason");
                self.error.get_or_insert(ApiError::Other(format!(
                    "The reply was cut off ({reason})."
                )));
            }
            "error" => {
                let error = if event["error"].is_object() {
                    &event["error"]
                } else {
                    event
                };
                self.error
                    .get_or_insert(api_error(0, &json!({"error": error})));
            }
            _ => {}
        }
    }
    fn finish(self) -> Result<String, ApiError> {
        if let Some(error) = self.error {
            return Err(error);
        }
        if !self.refusal.is_empty() {
            return Err(ApiError::Refused(self.refusal));
        }
        if !self.done {
            return Err(ApiError::Other(
                "The reply ended before it was complete.".into(),
            ));
        }
        Ok(self.text)
    }
}
/// Runs a store call on the blocking pool: the Keychain can wait on a prompt.
async fn off_executor(
    store: &Arc<dyn SessionStore>,
    call: impl FnOnce(&dyn SessionStore) -> Result<(), String> + Send + 'static,
) -> Result<(), String> {
    let store = store.clone();
    tokio::task::spawn_blocking(move || call(store.as_ref()))
        .await
        .unwrap_or_else(|_| Err("the session store stopped unexpectedly".into()))
}
async fn store_save(store: &Arc<dyn SessionStore>, session: &Session) -> Result<(), String> {
    let session = session.clone();
    off_executor(store, move |store| store.save(&session)).await
}
async fn store_clear(store: &Arc<dyn SessionStore>) -> Result<(), String> {
    off_executor(store, |store| store.clear()).await
}
pub struct Client {
    http: reqwest::Client,
    endpoints: Endpoints,
    client_id: String,
    store: Arc<dyn SessionStore>,
    session: Arc<tokio::sync::Mutex<Option<Session>>>,
    /// Base delay between retries.
    backoff: Duration,
}
impl Client {
    pub fn new(
        endpoints: Endpoints,
        client_id: String,
        store: Arc<dyn SessionStore>,
        session: Session,
    ) -> Self {
        Self {
            http: oauth::http_client(Duration::from_secs(300)),
            endpoints,
            client_id,
            store,
            session: Arc::new(tokio::sync::Mutex::new(Some(session))),
            backoff: Duration::from_secs(1),
        }
    }
    /// A usable access token. Refreshes when it is close to expiry or when
    /// `rejected` is the token the API just refused. One refresh runs at a
    /// time, and the rotated tokens are saved before they are used.
    ///
    /// The refresh runs on its own task that owns the session lock, so it
    /// finishes, saves and updates the session even if this caller is
    /// dropped after OpenAI has accepted (and spent) the old refresh token.
    async fn token(&self, rejected: Option<&str>) -> Result<String, ApiError> {
        let mut session = self.session.clone().lock_owned().await;
        let current = (*session)
            .clone()
            .ok_or_else(|| ApiError::SignedOut("Sign in with ChatGPT to continue.".into()))?;
        let stale = rejected == Some(current.access_token.as_str());
        if !stale && !current.expires_within(300, now()) {
            return Ok(current.access_token);
        }
        let http = self.http.clone();
        let endpoints = self.endpoints.clone();
        let client_id = self.client_id.clone();
        let store = self.store.clone();
        let backoff = self.backoff;
        let refresh = async move {
            for attempt in 1..=3 {
                match oauth::refresh(&http, &endpoints, &client_id, &current).await {
                    Ok(new) => {
                        // The old refresh token is spent now. If the store
                        // refuses twice, keep the new set in memory anyway.
                        if store_save(&store, &new).await.is_err() {
                            let _ = store_save(&store, &new).await;
                        }
                        let token = new.access_token.clone();
                        *session = Some(new);
                        return Ok(token);
                    }
                    Err(AuthError::Fatal(message)) => {
                        let _ = store_clear(&store).await;
                        *session = None;
                        return Err(ApiError::SignedOut(message));
                    }
                    Err(AuthError::Retry(message)) if attempt == 3 => {
                        return Err(ApiError::Unavailable(message));
                    }
                    Err(AuthError::Retry(_)) => tokio::time::sleep(backoff * attempt).await,
                }
            }
            unreachable!("the loop returns on its third attempt")
        };
        tokio::spawn(refresh).await.unwrap_or_else(|_| {
            Err(ApiError::Other(
                "The sign-in refresh was interrupted.".into(),
            ))
        })
    }
    /// Sends with the bearer token: one forced refresh after a 401, and up to
    /// three tries when OpenAI could not be reached or answered with a server
    /// error. A request that may have reached OpenAI without an answer is not
    /// repeated, so it is never billed twice.
    async fn send(
        &self,
        build: impl Fn(&str) -> reqwest::RequestBuilder,
    ) -> Result<reqwest::Response, ApiError> {
        let mut token = self.token(None).await?;
        let mut refreshed = false;
        let mut tries = 0;
        loop {
            let unavailable = match build(&token).send().await {
                Ok(r) if r.status().as_u16() == 401 && !refreshed => {
                    refreshed = true;
                    token = self.token(Some(&token)).await?;
                    continue;
                }
                Ok(r) if r.status().is_server_error() => "OpenAI is unavailable; try again.",
                Ok(r) => return Ok(r),
                Err(e) if e.is_connect() => "Could not reach OpenAI.",
                Err(_) => {
                    return Err(ApiError::Unavailable(
                        "OpenAI did not answer. The request was not repeated.".into(),
                    ));
                }
            };
            tries += 1;
            if tries == 3 {
                return Err(ApiError::Unavailable(unavailable.into()));
            }
            tokio::time::sleep(self.backoff * tries).await;
        }
    }
    /// Forgets a session OpenAI no longer accepts. The lock is taken first,
    /// so a refresh in flight cannot save its tokens after the clear, and the
    /// memory is cleared before the store, so a cancelled caller cannot leave
    /// the rejected session in use.
    async fn forget(&self) {
        let mut session = self.session.lock().await;
        *session = None;
        let _ = store_clear(&self.store).await;
    }
    /// Maps an error response and forgets the session when it is no longer valid.
    async fn failure(&self, response: reqwest::Response) -> ApiError {
        let status = response.status().as_u16();
        let body = response.json::<Value>().await.unwrap_or(Value::Null);
        let error = api_error(status, &body);
        if matches!(error, ApiError::SignedOut(_)) {
            self.forget().await;
        }
        error
    }
    /// Models the signed-in plan can use.
    pub async fn models(&self) -> Result<Vec<String>, ApiError> {
        let url = format!("{}/models", self.endpoints.api);
        let response = self
            .send(|token| self.http.get(&url).bearer_auth(token))
            .await?;
        if !response.status().is_success() {
            return Err(self.failure(response).await);
        }
        let body = response
            .json::<Value>()
            .await
            .map_err(|_| ApiError::Other("OpenAI returned an unreadable model list.".into()))?;
        Ok(body["data"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|m| m["visibility"].is_null() || m["visibility"] == "list")
            .filter_map(|m| m["id"].as_str().map(str::to_string))
            .collect())
    }
    pub async fn respond(&self, request: &Request) -> Result<Reply, ApiError> {
        match self.attempt(request, true).await {
            Err(ApiError::Unsupported(_)) => self.attempt(request, false).await,
            other => other,
        }
    }
    async fn attempt(&self, request: &Request, structured: bool) -> Result<Reply, ApiError> {
        let url = format!("{}/responses", self.endpoints.api);
        let body = request.body(structured);
        let response = self
            .send(|token| self.http.post(&url).bearer_auth(token).json(&body))
            .await?;
        if !response.status().is_success() {
            return Err(self.failure(response).await);
        }
        let mut parser = SseParser::default();
        let mut reply = Accumulator::default();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| {
                ApiError::Unavailable("The connection to OpenAI was interrupted.".into())
            })?;
            for event in parser.push(&chunk) {
                reply.apply(&event);
            }
            if parser.too_large {
                break;
            }
        }
        if let Some(event) = parser.finish()? {
            reply.apply(&event);
        }
        let text = reply.finish();
        if matches!(text, Err(ApiError::SignedOut(_))) {
            self.forget().await;
        }
        text.map(|text| Reply { text, structured })
    }
    /// One tiny request that proves plan tokens, streaming and replies work.
    pub async fn check(&self, model: &str) -> Result<String, ApiError> {
        let reply = self
            .respond(&Request {
                model: model.into(),
                messages: vec![(Role::User, "Reply with ok set to true.".into())],
                schema_name: "connection_check",
                schema: json!({
                    "type": "object",
                    "properties": {"ok": {"type": "boolean"}},
                    "required": ["ok"],
                    "additionalProperties": false,
                }),
            })
            .await?;
        match serde_json::from_str::<Value>(&reply.text) {
            Ok(value) if value["ok"] == true => Ok(format!(
                "Connection works: {model} answered{}.",
                if reply.structured {
                    " with structured output"
                } else {
                    ""
                }
            )),
            _ => Err(ApiError::Other(format!(
                "{model} answered, but not in the requested format."
            ))),
        }
    }
    /// Clears the saved and in-memory session and revokes the refresh token.
    /// Both are always attempted; `Err` is the sentence to show the user.
    pub async fn sign_out(&self) -> Result<(), String> {
        let (session, cleared) = {
            let mut session = self.session.lock().await;
            (session.take(), store_clear(&self.store).await)
        };
        let revoked = match &session {
            Some(s) => {
                oauth::revoke(
                    &self.http,
                    &self.endpoints,
                    &self.client_id,
                    &s.refresh_token,
                )
                .await
            }
            None => Ok(()),
        };
        match (cleared, revoked) {
            (Ok(()), Ok(())) => Ok(()),
            (Ok(()), Err(revoke)) => Err(format!("Signed out on this computer. {revoke}")),
            (Err(clear), revoked) => {
                let mut message = format!(
                    "The saved sign-in could not be deleted ({clear}). Delete the \"AXIS CAM Studio\" item in Keychain Access."
                );
                if revoked.is_err() {
                    message.push_str(" OpenAI did not confirm the sign-out either.");
                }
                Err(message)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        fake_openai::{Fake, Req, Resp},
        session::{FileStore, sample},
    };
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    fn request() -> Request {
        Request {
            model: "gpt-6.1-sol".into(),
            messages: vec![(Role::System, "Plan.".into()), (Role::User, "Part.".into())],
            schema_name: "plan",
            schema: json!({"type": "object"}),
        }
    }
    /// A file store that can be told to fail, like a Keychain that refuses.
    struct TestStore {
        file: FileStore,
        /// How many of the next saves fail.
        failing_saves: AtomicUsize,
        saves: AtomicUsize,
        failing_clear: AtomicBool,
    }
    impl SessionStore for TestStore {
        fn load(&self) -> Result<Option<Session>, String> {
            self.file.load()
        }
        fn save(&self, session: &Session) -> Result<(), String> {
            self.saves.fetch_add(1, Ordering::SeqCst);
            let failing = self.failing_saves.load(Ordering::SeqCst);
            if failing > 0 {
                self.failing_saves.store(failing - 1, Ordering::SeqCst);
                return Err("Keychain: save denied".into());
            }
            self.file.save(session)
        }
        fn clear(&self) -> Result<(), String> {
            if self.failing_clear.load(Ordering::SeqCst) {
                return Err("Keychain: delete denied".into());
            }
            self.file.clear()
        }
    }
    struct Rig {
        client: Client,
        store: Arc<TestStore>,
        fake: Fake,
        _dir: tempfile::TempDir,
    }
    async fn rig(session: Session, handler: impl Fn(&Req) -> Resp + Send + Sync + 'static) -> Rig {
        let fake = Fake::start(handler).await;
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(TestStore {
            file: FileStore(dir.path().join("session.json")),
            failing_saves: AtomicUsize::new(0),
            saves: AtomicUsize::new(0),
            failing_clear: AtomicBool::new(false),
        });
        store.file.save(&session).unwrap();
        let mut client = Client::new(
            Endpoints::local(&fake.base),
            "oaiapp_1".into(),
            store.clone(),
            session,
        );
        client.backoff = Duration::ZERO;
        Rig {
            client,
            store,
            fake,
            _dir: dir,
        }
    }
    fn rotated() -> Resp {
        Resp::json(
            200,
            json!({"access_token": "access-2", "refresh_token": "refresh-2", "expires_in": 3600}),
        )
    }
    #[tokio::test]
    async fn replies_stream_with_the_bearer_token_and_required_flags() {
        let r = rig(sample(), |_| Resp::answer("{\"ok\":true}")).await;
        let reply = r.client.respond(&request()).await.unwrap();
        assert_eq!(
            reply,
            Reply {
                text: "{\"ok\":true}".into(),
                structured: true
            }
        );
        let sent = &r.fake.seen("/v1/responses")[0];
        assert_eq!(sent.auth, "Bearer access-1");
        let body = sent.json();
        assert_eq!(
            (body["store"].clone(), body["stream"].clone()),
            (json!(false), json!(true))
        );
        assert_eq!(body["model"], "gpt-6.1-sol");
        assert_eq!(
            body["input"],
            json!([{"role": "system", "content": "Plan."}, {"role": "user", "content": "Part."}])
        );
        assert_eq!(
            body["text"]["format"],
            json!({"type": "json_schema", "name": "plan", "schema": {"type": "object"}, "strict": true})
        );
        assert!(r.fake.seen("/token").is_empty());
    }
    #[tokio::test]
    async fn unsupported_structured_output_falls_back_to_plain_json_once() {
        let r = rig(sample(), |req| {
            if req.json()["text"].is_object() {
                Resp::error(400, "subscription_sharing_unsupported_capability")
            } else {
                Resp::answer("{}")
            }
        })
        .await;
        let reply = r.client.respond(&request()).await.unwrap();
        assert_eq!(
            reply,
            Reply {
                text: "{}".into(),
                structured: false
            }
        );
        let sent = r.fake.seen("/v1/responses");
        assert_eq!(sent.len(), 2);
        let plain = sent[1].json();
        assert!(plain["text"].is_null());
        assert!(
            plain["input"][2]["content"]
                .as_str()
                .unwrap()
                .contains("{\"type\":\"object\"}")
        );
    }
    #[tokio::test]
    async fn tokens_refresh_before_expiry_and_are_saved_before_use() {
        let soon = Session {
            expires_at: now() + 60,
            ..sample()
        };
        let r = rig(soon, |req| match req.path.as_str() {
            "/token" => rotated(),
            _ => Resp::answer("{}"),
        })
        .await;
        r.client.respond(&request()).await.unwrap();
        assert_eq!(r.fake.seen("/v1/responses")[0].auth, "Bearer access-2");
        assert_eq!(r.store.load().unwrap().unwrap().refresh_token, "refresh-2");
    }
    #[tokio::test]
    async fn concurrent_requests_share_one_refresh() {
        let soon = Session {
            expires_at: now() + 60,
            ..sample()
        };
        let r = rig(soon, |req| match req.path.as_str() {
            "/token" => rotated(),
            "/v1/models" => Resp::json(200, json!({"data": []})),
            _ => Resp::answer("{}"),
        })
        .await;
        let request = request();
        let (a, b, c) = tokio::join!(
            r.client.respond(&request),
            r.client.respond(&request),
            r.client.models()
        );
        assert!(a.is_ok() && b.is_ok() && c.is_ok());
        assert_eq!(r.fake.seen("/token").len(), 1);
    }
    #[tokio::test]
    async fn a_401_forces_one_refresh_then_signs_out() {
        let r = rig(sample(), |req| {
            match (req.path.as_str(), req.auth.as_str()) {
                ("/token", _) => rotated(),
                (_, "Bearer access-2") => Resp::answer("{}"),
                _ => Resp::json(401, json!({})),
            }
        })
        .await;
        assert!(r.client.respond(&request()).await.is_ok());
        assert_eq!(r.fake.seen("/token").len(), 1);
        let r = rig(sample(), |req| match req.path.as_str() {
            "/token" => rotated(),
            _ => Resp::json(401, json!({})),
        })
        .await;
        assert!(matches!(
            r.client.respond(&request()).await,
            Err(ApiError::SignedOut(_))
        ));
        assert_eq!(r.store.load().unwrap(), None);
        assert!(matches!(
            r.client.models().await,
            Err(ApiError::SignedOut(_))
        ));
    }
    #[tokio::test]
    async fn fatal_refresh_errors_clear_the_session() {
        let soon = Session {
            expires_at: now() + 60,
            ..sample()
        };
        let r = rig(soon, |_| {
            Resp::json(400, json!({"error": "refresh_token_reused"}))
        })
        .await;
        let error = r.client.respond(&request()).await.unwrap_err();
        assert!(matches!(&error, ApiError::SignedOut(m) if m.contains("refresh_token_reused")));
        assert_eq!(r.store.load().unwrap(), None);
        assert!(r.fake.seen("/v1/responses").is_empty());
    }
    #[tokio::test]
    async fn server_errors_are_retried_then_reported() {
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        let r = rig(sample(), move |_| {
            if seen.fetch_add(1, Ordering::SeqCst) < 2 {
                Resp::json(503, json!({}))
            } else {
                Resp::answer("{}")
            }
        })
        .await;
        assert!(r.client.respond(&request()).await.is_ok());
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        let r = rig(sample(), |_| {
            Resp::error(503, "subscription_sharing_usage_unavailable")
        })
        .await;
        assert!(matches!(
            r.client.respond(&request()).await,
            Err(ApiError::Unavailable(_))
        ));
        assert_eq!(r.fake.seen("/v1/responses").len(), 3);
        let soon = Session {
            expires_at: now() + 60,
            ..sample()
        };
        let r = rig(soon, |_| Resp::json(500, json!({}))).await;
        assert!(matches!(
            r.client.models().await,
            Err(ApiError::Unavailable(_))
        ));
        assert!(r.store.load().unwrap().is_some());
    }
    #[tokio::test]
    async fn api_errors_map_to_user_facing_states() {
        for (status, code, expected) in [
            (
                429,
                "subscription_sharing_usage_limit_exceeded",
                ApiError::UsageLimit,
            ),
            (
                403,
                "subscription_sharing_user_not_eligible",
                ApiError::NotEligible("subscription_sharing_user_not_eligible message".into()),
            ),
            (
                403,
                "subscription_sharing_route_not_supported",
                ApiError::Other("subscription_sharing_route_not_supported message".into()),
            ),
            (
                401,
                "subscription_sharing_invalid_user",
                ApiError::SignedOut("Your ChatGPT sign-in was disconnected. Sign in again.".into()),
            ),
        ] {
            let r = rig(sample(), move |req| match req.path.as_str() {
                "/token" => rotated(),
                _ => Resp::error(status, code),
            })
            .await;
            assert_eq!(
                r.client.respond(&request()).await.unwrap_err(),
                expected,
                "{code}"
            );
        }
        assert_eq!(
            api_error(418, &Value::Null),
            ApiError::Other("OpenAI returned HTTP 418.".into())
        );
        assert!(
            ApiError::UsageLimit
                .message()
                .contains("Usage limit reached")
        );
    }
    #[tokio::test]
    async fn stream_failures_refusals_and_early_ends_are_errors() {
        let failed = rig(sample(), |_| Resp::sse(&[
            json!({"type": "response.output_text.delta", "delta": "{"}),
            json!({"type": "response.failed", "response": {"error": {"code": "subscription_sharing_usage_limit_exceeded", "message": "cap"}}}),
        ]))
        .await;
        assert_eq!(
            failed.client.respond(&request()).await.unwrap_err(),
            ApiError::UsageLimit
        );
        let refused = rig(sample(), |_| {
            Resp::sse(&[
                json!({"type": "response.refusal.delta", "delta": "No."}),
                json!({"type": "response.completed"}),
            ])
        })
        .await;
        assert_eq!(
            refused.client.respond(&request()).await.unwrap_err(),
            ApiError::Refused("No.".into())
        );
        let early = rig(sample(), |_| {
            Resp::sse(&[json!({"type": "response.output_text.delta", "delta": "{"})])
        })
        .await;
        assert!(
            matches!(early.client.respond(&request()).await, Err(ApiError::Other(m)) if m.contains("before it was complete"))
        );
        let cut = rig(sample(), |_| Resp::sse(&[json!({"type": "response.incomplete", "response": {"incomplete_details": {"reason": "max_output_tokens"}}})])).await;
        assert!(
            matches!(cut.client.respond(&request()).await, Err(ApiError::Other(m)) if m.contains("max_output_tokens"))
        );
        let errored = rig(sample(), |_| {
            Resp::sse(&[json!({"type": "error", "code": "server_error", "message": "boom"})])
        })
        .await;
        assert_eq!(
            errored.client.respond(&request()).await.unwrap_err(),
            ApiError::Other("boom".into())
        );
        let final_only = rig(sample(), |_| Resp::sse(&[json!({"type": "response.completed", "response": {"output": [{"content": [{"type": "output_text", "text": "{\"a\":1}"}]}]}})])).await;
        assert_eq!(
            final_only.client.respond(&request()).await.unwrap().text,
            "{\"a\":1}"
        );
    }
    #[tokio::test]
    async fn a_final_event_without_a_blank_line_completes_the_reply() {
        let r = rig(sample(), |_| Resp {
            body: "data: {\"type\":\"response.output_text.delta\",\"delta\":\"{}\"}\n\ndata: {\"type\":\"response.completed\"}".into(),
            ..Resp::sse(&[])
        })
        .await;
        assert_eq!(r.client.respond(&request()).await.unwrap().text, "{}");
    }
    #[test]
    fn sse_events_survive_any_chunking_and_crlf() {
        let stream = "event: a\r\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"é1\"}\r\n\r\n: comment\n\ndata: [DONE]\n\ndata: {\"type\":\"response.completed\"}\n\n";
        for size in [1, 2, 3, 7, 1000] {
            let mut parser = SseParser::default();
            let events: Vec<Value> = stream
                .as_bytes()
                .chunks(size)
                .flat_map(|c| parser.push(c))
                .collect();
            assert_eq!(events.len(), 2, "chunk size {size}");
            assert_eq!(events[0]["delta"], "é1");
            assert_eq!(events[1]["type"], "response.completed");
            assert_eq!(parser.finish(), Ok(None), "chunk size {size}");
        }
        // The last event may end without the blank line.
        for ending in ["", "\n", "\r\n"] {
            let stream = format!(
                "data: {{\"type\":\"response.created\"}}\r\n\r\ndata: {{\"type\":\"response.completed\"}}{ending}"
            );
            for size in [1, 2, 3, 7, 1000] {
                let mut parser = SseParser::default();
                let mut events: Vec<Value> = stream
                    .as_bytes()
                    .chunks(size)
                    .flat_map(|c| parser.push(c))
                    .collect();
                assert_eq!(events.len(), 1, "chunk size {size}, ending {ending:?}");
                events.extend(parser.finish().unwrap());
                assert_eq!(events.len(), 2, "chunk size {size}, ending {ending:?}");
                assert_eq!(events[1]["type"], "response.completed");
                assert_eq!(parser.finish(), Ok(None));
            }
        }
    }
    #[test]
    fn sse_parser_refuses_to_buffer_more_than_16_mb() {
        let mut parser = SseParser::default();
        assert_eq!(
            parser
                .push(b"data: {\"type\":\"response.created\"}\n\n")
                .len(),
            1
        );
        // A huge event: 16 MiB is still buffered; more is refused, even when
        // the extra bytes carry the blank line that would end it.
        let chunk = vec![b'x'; 64 * 1024];
        for _ in 0..256 {
            assert!(parser.push(&chunk).is_empty());
        }
        assert!(parser.push(b"x\n\n").is_empty());
        assert_eq!(
            parser.finish(),
            Err(ApiError::Other("The reply was too large.".into()))
        );
        assert!(parser.push(b"data: {}\n\n").is_empty());
    }
    #[tokio::test]
    async fn models_lists_only_selectable_ids() {
        let r = rig(sample(), |_| {
            Resp::json(
                200,
                json!({"data": [
                    {"id": "gpt-6.1-sol", "visibility": "list"},
                    {"id": "internal", "visibility": "hidden"},
                    {"id": "gpt-6-luna"},
                ]}),
            )
        })
        .await;
        assert_eq!(
            r.client.models().await.unwrap(),
            vec!["gpt-6.1-sol", "gpt-6-luna"]
        );
        assert_eq!(r.fake.seen("/v1/models")[0].auth, "Bearer access-1");
    }
    #[tokio::test]
    async fn check_reports_success_fallback_and_wrong_format() {
        let ok = rig(sample(), |_| Resp::answer("{\"ok\":true}")).await;
        assert_eq!(
            ok.client.check("gpt-6.1-sol").await.unwrap(),
            "Connection works: gpt-6.1-sol answered with structured output."
        );
        let wrong = rig(sample(), |_| Resp::answer("Sure!")).await;
        assert!(
            wrong
                .client
                .check("gpt-6.1-sol")
                .await
                .unwrap_err()
                .message()
                .contains("requested format")
        );
    }
    #[tokio::test]
    async fn sign_out_revokes_and_clears_even_when_revocation_fails() {
        let r = rig(sample(), |_| Resp::json(200, json!({}))).await;
        r.client.sign_out().await.unwrap();
        assert_eq!(r.fake.seen("/revoke")[0].field("token"), "refresh-1");
        assert_eq!(r.store.load().unwrap(), None);
        assert!(matches!(
            r.client.respond(&request()).await,
            Err(ApiError::SignedOut(_))
        ));
        // Signing out again has nothing to revoke.
        r.client.sign_out().await.unwrap();
        assert_eq!(r.fake.seen("/revoke").len(), 1);
        let r = rig(sample(), |_| Resp::json(500, json!({}))).await;
        assert_eq!(
            r.client.sign_out().await.unwrap_err(),
            "Signed out on this computer. OpenAI did not confirm the sign-out (HTTP 500)."
        );
        assert_eq!(r.store.load().unwrap(), None);
    }
    #[tokio::test]
    async fn sign_out_still_revokes_when_the_saved_sign_in_cannot_be_deleted() {
        let r = rig(sample(), |_| Resp::json(200, json!({}))).await;
        r.store.failing_clear.store(true, Ordering::SeqCst);
        assert_eq!(
            r.client.sign_out().await.unwrap_err(),
            "The saved sign-in could not be deleted (Keychain: delete denied). Delete the \"AXIS CAM Studio\" item in Keychain Access."
        );
        let revoked = r.fake.seen("/revoke");
        assert_eq!(revoked.len(), 1);
        assert_eq!(revoked[0].field("token"), "refresh-1");
        assert!(matches!(
            r.client.respond(&request()).await,
            Err(ApiError::SignedOut(_))
        ));
        let r = rig(sample(), |_| Resp::json(500, json!({}))).await;
        r.store.failing_clear.store(true, Ordering::SeqCst);
        assert_eq!(
            r.client.sign_out().await.unwrap_err(),
            "The saved sign-in could not be deleted (Keychain: delete denied). Delete the \"AXIS CAM Studio\" item in Keychain Access. OpenAI did not confirm the sign-out either."
        );
        assert_eq!(r.fake.seen("/revoke").len(), 1);
    }
    /// Polls `done` every 10 ms; false if it still fails after two seconds.
    async fn until(done: impl Fn() -> bool) -> bool {
        for _ in 0..200 {
            if done() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        done()
    }
    #[tokio::test]
    async fn a_cancelled_request_still_keeps_the_rotated_tokens() {
        let soon = Session {
            expires_at: now() + 60,
            ..sample()
        };
        // Like OpenAI, the fake accepts a refresh token only once.
        let used = AtomicBool::new(false);
        let r = rig(soon, move |req| match req.path.as_str() {
            "/token"
                if req.field("refresh_token") == "refresh-1"
                    && !used.swap(true, Ordering::SeqCst) =>
            {
                Resp {
                    delay_ms: 400,
                    ..rotated()
                }
            }
            "/token" => Resp::json(400, json!({"error": "refresh_token_reused"})),
            _ => Resp::answer("{}"),
        })
        .await;
        let request = request();
        let mut first = Box::pin(r.client.respond(&request));
        tokio::select! {
            _ = &mut first => panic!("the request finished before the refresh was answered"),
            received = until(|| !r.fake.seen("/token").is_empty()) => assert!(received),
        }
        // OpenAI has the refresh request but has not answered: cancel.
        drop(first);
        assert!(
            until(|| {
                r.store
                    .load()
                    .unwrap()
                    .is_some_and(|s| s.refresh_token == "refresh-2")
            })
            .await,
            "the rotated refresh token was not saved"
        );
        r.client.respond(&request).await.unwrap();
        let sent = r.fake.seen("/v1/responses");
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0].auth, "Bearer access-2");
        assert_eq!(r.fake.seen("/token").len(), 1);
    }
    #[tokio::test]
    async fn rotated_tokens_stay_in_memory_when_they_cannot_be_saved() {
        let soon = Session {
            expires_at: now() + 60,
            ..sample()
        };
        let handler = |req: &Req| match req.path.as_str() {
            "/token" => rotated(),
            _ => Resp::answer("{}"),
        };
        // The first save fails and the retry saves them.
        let r = rig(soon.clone(), handler).await;
        r.store.failing_saves.store(1, Ordering::SeqCst);
        r.client.respond(&request()).await.unwrap();
        assert_eq!(r.store.saves.load(Ordering::SeqCst), 2);
        assert_eq!(r.store.load().unwrap().unwrap().refresh_token, "refresh-2");
        // Every save fails: the new tokens are still used, from memory.
        let r = rig(soon, handler).await;
        r.store.failing_saves.store(usize::MAX, Ordering::SeqCst);
        r.client.respond(&request()).await.unwrap();
        r.client.respond(&request()).await.unwrap();
        assert_eq!(r.store.saves.load(Ordering::SeqCst), 2);
        let sent = r.fake.seen("/v1/responses");
        assert_eq!(sent.len(), 2);
        assert!(sent.iter().all(|s| s.auth == "Bearer access-2"));
        assert_eq!(r.fake.seen("/token").len(), 1);
        assert_eq!(r.store.load().unwrap().unwrap().refresh_token, "refresh-1");
    }
    /// A store whose `clear` blocks its thread until the test releases it.
    struct GatedStore {
        started: AtomicBool,
        go: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
    }
    impl SessionStore for GatedStore {
        fn load(&self) -> Result<Option<Session>, String> {
            Ok(None)
        }
        fn save(&self, _: &Session) -> Result<(), String> {
            Ok(())
        }
        fn clear(&self) -> Result<(), String> {
            self.started.store(true, Ordering::SeqCst);
            self.go
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(1))
                .map_err(|_| "the async executor was blocked".to_string())
        }
    }
    #[tokio::test]
    async fn store_calls_run_off_the_async_executor() {
        // A single-threaded runtime: a store call made on the executor thread
        // would keep `release` from ever running.
        let fake = Fake::start(|_| Resp::json(200, json!({}))).await;
        let (go, wait) = std::sync::mpsc::channel();
        let store = Arc::new(GatedStore {
            started: AtomicBool::new(false),
            go: std::sync::Mutex::new(wait),
        });
        let client = Client::new(
            Endpoints::local(&fake.base),
            "oaiapp_1".into(),
            store.clone(),
            sample(),
        );
        let release = async {
            while !store.started.load(Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
            go.send(()).unwrap();
        };
        let (result, ()) = tokio::join!(client.sign_out(), release);
        assert_eq!(result, Ok(()));
    }
    #[tokio::test]
    async fn a_cancelled_request_still_forgets_a_rejected_session() {
        let fake = Fake::start(|req: &Req| match req.path.as_str() {
            "/token" => rotated(),
            _ => Resp::error(401, "subscription_sharing_invalid_user"),
        })
        .await;
        let (go, wait) = std::sync::mpsc::channel();
        let store = Arc::new(GatedStore {
            started: AtomicBool::new(false),
            go: std::sync::Mutex::new(wait),
        });
        let client = Client::new(
            Endpoints::local(&fake.base),
            "oaiapp_1".into(),
            store.clone(),
            sample(),
        );
        let request = request();
        let mut call = Box::pin(client.respond(&request));
        tokio::select! {
            _ = &mut call => panic!("the request finished while the store was still clearing"),
            started = until(|| store.started.load(Ordering::SeqCst)) => assert!(started),
        }
        // OpenAI rejected the session and its clear is under way: cancel.
        drop(call);
        go.send(()).unwrap();
        assert!(matches!(client.models().await, Err(ApiError::SignedOut(_))));
        assert!(fake.seen("/v1/models").is_empty());
    }
    #[tokio::test]
    async fn a_streamed_invalid_user_error_clears_the_session() {
        for event in [
            json!({"type": "response.failed", "response": {"error": {"code": "subscription_sharing_invalid_user", "message": "gone"}}}),
            json!({"type": "error", "code": "subscription_sharing_invalid_user", "message": "gone"}),
        ] {
            let r = rig(sample(), move |_| Resp::sse(std::slice::from_ref(&event))).await;
            assert!(matches!(
                r.client.respond(&request()).await,
                Err(ApiError::SignedOut(_))
            ));
            assert_eq!(r.store.load().unwrap(), None);
            assert!(matches!(
                r.client.models().await,
                Err(ApiError::SignedOut(_))
            ));
        }
    }
    #[tokio::test]
    async fn stream_errors_without_details_say_openai_reported_an_error() {
        for event in [
            json!({"type": "error"}),
            json!({"type": "error", "error": {}}),
            json!({"type": "response.failed", "response": {}}),
        ] {
            let r = rig(sample(), move |_| Resp::sse(std::slice::from_ref(&event))).await;
            assert_eq!(
                r.client.respond(&request()).await.unwrap_err(),
                ApiError::Other("OpenAI reported an error.".into())
            );
        }
    }
    #[tokio::test]
    async fn requests_are_not_repeated_after_an_unanswered_send() {
        let r = rig(sample(), |_| Resp::hangup()).await;
        assert_eq!(
            r.client.respond(&request()).await.unwrap_err(),
            ApiError::Unavailable("OpenAI did not answer. The request was not repeated.".into())
        );
        assert_eq!(r.fake.seen("/v1/responses").len(), 1);
        // Nothing listens on port 9: connect errors are retried, then reported.
        let mut client = Client::new(
            Endpoints::local("http://127.0.0.1:9"),
            "oaiapp_1".into(),
            r.store.clone(),
            sample(),
        );
        client.backoff = Duration::ZERO;
        assert_eq!(
            client.models().await.unwrap_err(),
            ApiError::Unavailable("Could not reach OpenAI.".into())
        );
    }
}
