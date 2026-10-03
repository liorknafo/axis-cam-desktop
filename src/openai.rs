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
/// Splits a server-sent-event byte stream into the JSON of each `data:` event.
#[derive(Default)]
pub struct SseParser(Vec<u8>);
impl SseParser {
    pub fn push(&mut self, chunk: &[u8]) -> Vec<Value> {
        self.0.extend(chunk.iter().filter(|b| **b != b'\r'));
        let mut events = vec![];
        while let Some(end) = self.0.windows(2).position(|w| w == b"\n\n") {
            let block: Vec<u8> = self.0.drain(..end + 2).collect();
            let data: Vec<&str> = std::str::from_utf8(&block)
                .unwrap_or("")
                .lines()
                .filter_map(|l| l.strip_prefix("data:"))
                .map(str::trim_start)
                .collect();
            if let Ok(value) = serde_json::from_str::<Value>(&data.join("\n")) {
                events.push(value);
            }
        }
        events
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
pub struct Client {
    http: reqwest::Client,
    endpoints: Endpoints,
    client_id: String,
    store: Arc<dyn SessionStore>,
    session: tokio::sync::Mutex<Option<Session>>,
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
            http: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(15))
                .timeout(Duration::from_secs(300))
                .build()
                .expect("HTTP client"),
            endpoints,
            client_id,
            store,
            session: tokio::sync::Mutex::new(Some(session)),
            backoff: Duration::from_secs(1),
        }
    }
    /// A usable access token. Refreshes when it is close to expiry or when
    /// `rejected` is the token the API just refused. One refresh runs at a
    /// time, and the rotated tokens are saved before they are used.
    async fn token(&self, rejected: Option<&str>) -> Result<String, ApiError> {
        let mut guard = self.session.lock().await;
        let current = guard
            .clone()
            .ok_or_else(|| ApiError::SignedOut("Sign in with ChatGPT to continue.".into()))?;
        let stale = rejected == Some(current.access_token.as_str());
        if !stale && !current.expires_within(300, now()) {
            return Ok(current.access_token);
        }
        for attempt in 1..=3 {
            match oauth::refresh(&self.http, &self.endpoints, &self.client_id, &current).await {
                Ok(new) => {
                    self.store.save(&new).map_err(|e| {
                        ApiError::Other(format!("Could not save the ChatGPT sign-in: {e}"))
                    })?;
                    let token = new.access_token.clone();
                    *guard = Some(new);
                    return Ok(token);
                }
                Err(AuthError::Fatal(message)) => {
                    let _ = self.store.clear();
                    *guard = None;
                    return Err(ApiError::SignedOut(message));
                }
                Err(AuthError::Retry(message)) if attempt == 3 => {
                    return Err(ApiError::Unavailable(message));
                }
                Err(AuthError::Retry(_)) => tokio::time::sleep(self.backoff * attempt).await,
            }
        }
        unreachable!("the loop returns on its third attempt")
    }
    /// Sends with the bearer token: one forced refresh after a 401, and up to
    /// three tries on network or server errors.
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
                Err(_) => "Could not reach OpenAI.",
            };
            tries += 1;
            if tries == 3 {
                return Err(ApiError::Unavailable(unavailable.into()));
            }
            tokio::time::sleep(self.backoff * tries).await;
        }
    }
    /// Maps an error response and forgets the session when it is no longer valid.
    async fn failure(&self, response: reqwest::Response) -> ApiError {
        let status = response.status().as_u16();
        let body = response.json::<Value>().await.unwrap_or(Value::Null);
        let error = api_error(status, &body);
        if matches!(error, ApiError::SignedOut(_)) {
            let _ = self.store.clear();
            *self.session.lock().await = None;
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
        }
        reply.finish().map(|text| Reply { text, structured })
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
    /// Revokes the refresh token and clears the local session. `Err` means
    /// the local session is cleared but OpenAI did not confirm the revocation.
    pub async fn sign_out(&self) -> Result<(), String> {
        let session = self.session.lock().await.take();
        self.store.clear()?;
        match session {
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
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn request() -> Request {
        Request {
            model: "gpt-6.1-sol".into(),
            messages: vec![(Role::System, "Plan.".into()), (Role::User, "Part.".into())],
            schema_name: "plan",
            schema: json!({"type": "object"}),
        }
    }
    struct Rig {
        client: Client,
        store: Arc<FileStore>,
        fake: Fake,
        _dir: tempfile::TempDir,
    }
    async fn rig(session: Session, handler: impl Fn(&Req) -> Resp + Send + Sync + 'static) -> Rig {
        let fake = Fake::start(handler).await;
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(FileStore(dir.path().join("session.json")));
        store.save(&session).unwrap();
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
        }
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
        let r = rig(sample(), |_| Resp::json(500, json!({}))).await;
        assert!(
            r.client
                .sign_out()
                .await
                .unwrap_err()
                .contains("not confirm")
        );
        assert_eq!(r.store.load().unwrap(), None);
    }
}
