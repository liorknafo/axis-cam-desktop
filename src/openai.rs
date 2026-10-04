//! OpenAI Responses API client for requests billed to the user's ChatGPT plan.
use crate::{
    oauth::{self, AuthError, Endpoints},
    session::{Session, SessionStore, now},
};
use futures_util::StreamExt;
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};

pub const USAGE_URL: &str = "https://chatgpt.com/settings/usage";
/// The most one request may take in total: a planning reply can stream for
/// minutes. A connection that goes quiet is given up much sooner; see
/// [`oauth::http_client`].
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15 * 60);
const TIMED_OUT: &str = "OpenAI did not answer in time.";

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
    /// The sentence to show. Every text the variants carry can come from
    /// OpenAI's server or the model, so each is shown as one cleaned line of
    /// at most 300 characters.
    pub fn message(&self) -> String {
        let clip = |m: &str| crate::proposal::clip(m, 300);
        match self {
            Self::UsageLimit => {
                "Usage limit reached. Manage usage in your ChatGPT settings.".into()
            }
            Self::NotEligible(m) => {
                format!("Your ChatGPT plan cannot be used in this app. {}", clip(m))
            }
            Self::Refused(m) => format!("The model declined the request: {}", clip(m)),
            Self::SignedOut(m) | Self::Unavailable(m) | Self::Unsupported(m) | Self::Other(m) => {
                clip(m)
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
    Assistant,
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
                    Role::Assistant => "assistant",
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
/// The most reply text (output and refusal) one request may stream.
const MAX_REPLY: usize = 4 * 1024 * 1024;
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
        if self.text.len() + self.refusal.len() > MAX_REPLY {
            self.text = String::new();
            self.refusal = String::new();
            self.error
                .get_or_insert(ApiError::Other("The reply was too large.".into()));
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
async fn off_executor<T: Send + 'static>(
    store: &Arc<dyn SessionStore>,
    call: impl FnOnce(&dyn SessionStore) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let store = store.clone();
    tokio::task::spawn_blocking(move || call(store.as_ref()))
        .await
        .unwrap_or_else(|_| Err("the session store stopped unexpectedly".into()))
}
/// Saves a refresh's rotated tokens unless the client's sign-in has been
/// abandoned (`Client::start_abandoning`). The check and the save hold the
/// same lock as that delete, so a save can never land after it.
async fn store_save_unless_abandoned(
    store: &Arc<dyn SessionStore>,
    abandoned: &Arc<std::sync::Mutex<bool>>,
    session: &Session,
) -> Result<(), String> {
    let (abandoned, session) = (abandoned.clone(), session.clone());
    off_executor(store, move |store| {
        let abandoned = abandoned
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if *abandoned {
            Ok(())
        } else {
            store.save(&session)
        }
    })
    .await
}
async fn store_clear(store: &Arc<dyn SessionStore>) -> Result<(), String> {
    off_executor(store, |store| store.clear()).await
}
async fn store_load(store: &Arc<dyn SessionStore>) -> Result<Option<Session>, String> {
    off_executor(store, |store| store.load()).await
}
/// Whether a saved sign-in may replace this client's `current` one: the same
/// user, a different refresh token that this client has not spent, and newer
/// (it expires later). An older saved set, such as one a failed save left
/// behind, holds a refresh token OpenAI has already rotated.
fn newer_saved(saved: &Session, current: &Session, spent: &std::sync::Mutex<Vec<String>>) -> bool {
    saved.subject == current.subject
        && saved.refresh_token != current.refresh_token
        && saved.expires_at > current.expires_at
        && !spent.lock().unwrap().contains(&saved.refresh_token)
}
/// One refresh of the sign-in in flight, counted while it lives.
pub(crate) struct InFlight(Arc<std::sync::atomic::AtomicUsize>);
impl InFlight {
    fn new(count: &Arc<std::sync::atomic::AtomicUsize>) -> Self {
        count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Self(count.clone())
    }
}
impl Drop for InFlight {
    fn drop(&mut self) {
        self.0.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    }
}
pub struct Client {
    http: reqwest::Client,
    endpoints: Endpoints,
    client_id: String,
    store: Arc<dyn SessionStore>,
    session: Arc<tokio::sync::Mutex<Option<Session>>>,
    /// Refresh tokens this client has presented and OpenAI accepted or
    /// refused, kept for the client's lifetime: none is presented again.
    spent: Arc<std::sync::Mutex<Vec<String>>>,
    /// Refreshes of the sign-in in flight (see `refreshing`).
    refreshes: Arc<std::sync::atomic::AtomicUsize>,
    /// Set by `start_abandoning`: no refresh saves its tokens any more.
    abandoned: Arc<std::sync::Mutex<bool>>,
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
            http: oauth::http_client(REQUEST_TIMEOUT),
            endpoints,
            client_id,
            store,
            session: Arc::new(tokio::sync::Mutex::new(Some(session))),
            spent: Arc::default(),
            refreshes: Arc::default(),
            abandoned: Arc::default(),
            backoff: Duration::from_secs(1),
        }
    }
    /// Whether a refresh of the sign-in is in flight, from before its
    /// request until after its save. A close waits for it (at most
    /// `CLOSE_WAIT`): once OpenAI has accepted the request, the old refresh
    /// token is spent, and losing the new set would sign the user out.
    pub fn refreshing(&self) -> bool {
        self.refreshes.load(std::sync::atomic::Ordering::SeqCst) > 0
    }
    /// A refresh in flight for as long as the result lives.
    #[cfg(test)]
    pub(crate) fn hold_refresh(&self) -> InFlight {
        InFlight::new(&self.refreshes)
    }
    /// Starts deleting the saved sign-in on the blocking pool and returns at
    /// once, for a close that stopped waiting for this client's sign-out.
    /// Under the lock a refresh's save takes, it first marks the sign-in
    /// abandoned, so a refresh still running cannot put its tokens back
    /// afterwards. The runtime lets the delete finish when the app ends.
    /// Call it from within a Tokio runtime.
    pub fn start_abandoning(&self) {
        let (store, abandoned) = (self.store.clone(), self.abandoned.clone());
        drop(tokio::task::spawn_blocking(move || {
            let mut abandoned = abandoned
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            *abandoned = true;
            store.clear()
        }));
    }
    /// A usable access token. Refreshes when it is close to expiry or when
    /// `rejected` is the token the API just refused. One refresh runs at a
    /// time, and the rotated tokens are saved before they are used.
    ///
    /// The refresh runs on its own task that owns the session lock, so it
    /// finishes, saves and updates the session even if this caller is
    /// dropped after OpenAI has accepted (and spent) the old refresh token.
    ///
    /// Before refreshing, it reads the saved sign-in: when another running
    /// copy has saved newer tokens for the same user, those are used, and
    /// refreshed only if they are close to expiry themselves. A saved set is
    /// adopted only when it is newer than this client's own and holds no
    /// refresh token this client has spent (`newer_saved`): after both saves
    /// of a refresh failed, the store still holds the rotated-out set.
    ///
    /// When OpenAI rejects the refresh token for good, another running copy
    /// of the app may have spent it and saved its new tokens. Those are then
    /// used from here on (the next call refreshes them if they are close to
    /// expiry), but only when they belong to the same user and are newer:
    /// another user's sign-in is neither used nor deleted, and this client
    /// signs out. Only a saved sign-in with the rejected refresh token is
    /// deleted; one that cannot be read is left alone.
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
        let spent = self.spent.clone();
        let abandoned = self.abandoned.clone();
        let in_flight = InFlight::new(&self.refreshes);
        let rejected = rejected.map(str::to_string);
        let refresh = async move {
            // Counted until this ends, also when the runtime drops it.
            let _in_flight = in_flight;
            // Another running copy may have refreshed already and saved the
            // rotated tokens; presenting the refresh token it spent would
            // look like reuse to OpenAI. Its saved set is used instead, and
            // refreshed only if it is close to expiry itself (or refused).
            let mut current = current;
            if let Ok(Some(saved)) = store_load(&store).await
                && newer_saved(&saved, &current, &spent)
            {
                if rejected.as_deref() != Some(saved.access_token.as_str())
                    && !saved.expires_within(300, now())
                {
                    let token = saved.access_token.clone();
                    *session = Some(saved);
                    return Ok(token);
                }
                current = saved;
            }
            for attempt in 1..=3 {
                match oauth::refresh(&http, &endpoints, &client_id, &current).await {
                    Ok(new) => {
                        // The old refresh token is spent now. If the store
                        // refuses twice, keep the new set in memory anyway.
                        spent.lock().unwrap().push(current.refresh_token.clone());
                        if store_save_unless_abandoned(&store, &abandoned, &new)
                            .await
                            .is_err()
                        {
                            let _ = store_save_unless_abandoned(&store, &abandoned, &new).await;
                        }
                        let token = new.access_token.clone();
                        *session = Some(new);
                        return Ok(token);
                    }
                    Err(AuthError::Fatal(message)) => {
                        spent.lock().unwrap().push(current.refresh_token.clone());
                        match store_load(&store).await {
                            Ok(Some(saved)) if newer_saved(&saved, &current, &spent) => {
                                let token = saved.access_token.clone();
                                *session = Some(saved);
                                return Ok(token);
                            }
                            // Another user's sign-in: not ours to use or delete.
                            Ok(Some(saved)) if saved.refresh_token != current.refresh_token => {}
                            Ok(_) => {
                                let _ = store_clear(&store).await;
                            }
                            // Unreadable: it may be a newer sign-in.
                            Err(_) => {}
                        }
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
                Err(e) if e.is_timeout() => return Err(ApiError::Unavailable(TIMED_OUT.into())),
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
    /// the rejected session in use. A client that no longer holds a session
    /// (it signed out, or another call already forgot it) leaves the store
    /// alone: whatever is saved there now belongs to a newer sign-in.
    async fn forget(&self) {
        let mut session = self.session.lock().await;
        if session.take().is_some() {
            let _ = store_clear(&self.store).await;
        }
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
    /// `respond` without the structured attempt, for a caller that already
    /// knows the plan route refuses it.
    pub async fn respond_plain(&self, request: &Request) -> Result<Reply, ApiError> {
        self.attempt(request, false).await
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
            let chunk = match chunk {
                Ok(chunk) => chunk,
                Err(e) if e.is_timeout() => return Err(ApiError::Unavailable(TIMED_OUT.into())),
                Err(_) => {
                    return Err(ApiError::Unavailable(
                        "The connection to OpenAI was interrupted.".into(),
                    ));
                }
            };
            for event in parser.push(&chunk) {
                reply.apply(&event);
            }
            // The reply is complete, or too large to keep: do not wait for
            // the server to close the connection.
            if parser.too_large || reply.done || reply.error.is_some() {
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
    /// Both are always attempted; `Err` is the sentence to show the user. A
    /// client without a session has nothing to clear or revoke, and does not
    /// touch the store, which may now hold a newer sign-in.
    pub async fn sign_out(&self) -> Result<(), String> {
        let (session, cleared) = {
            let mut session = self.session.lock().await;
            let Some(taken) = session.take() else {
                return Ok(());
            };
            (taken, store_clear(&self.store).await)
        };
        let revoked = oauth::revoke(
            &self.http,
            &self.endpoints,
            &self.client_id,
            &session.refresh_token,
        )
        .await;
        match (cleared, revoked) {
            (Ok(()), Ok(())) => Ok(()),
            (Ok(()), Err(revoke)) => Err(format!("Signed out on this computer. {revoke}")),
            (Err(clear), revoked) => {
                let mut message = format!(
                    "The saved sign-in could not be deleted ({clear}). On macOS, delete the \"AXIS CAM Studio\" item in Keychain Access."
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
    #[test]
    fn every_message_from_openai_is_one_clean_bounded_line() {
        let hostile = format!("Bad\nnews \u{202e}\u{7}{}", "x".repeat(5000));
        for (error, prefix) in [
            (ApiError::SignedOut(hostile.clone()), ""),
            (
                ApiError::NotEligible(hostile.clone()),
                "Your ChatGPT plan cannot be used in this app. ",
            ),
            (ApiError::Unavailable(hostile.clone()), ""),
            (
                ApiError::Refused(hostile.clone()),
                "The model declined the request: ",
            ),
            (ApiError::Unsupported(hostile.clone()), ""),
            (ApiError::Other(hostile.clone()), ""),
        ] {
            let message = error.message();
            assert!(
                message.starts_with(&format!("{prefix}Bad news x")),
                "{message}"
            );
            assert!(!message.contains(['\n', '\u{202e}', '\u{7}']), "{error:?}");
            assert_eq!(message.chars().count(), prefix.chars().count() + 300);
        }
        // The server's own text, from a response body, is bounded the same way.
        let body = json!({"error": {"code": "other", "message": hostile}});
        assert_eq!(api_error(400, &body).message().chars().count(), 300);
        // Short messages are unchanged.
        let short = ApiError::Other("OpenAI returned HTTP 418.".into());
        assert_eq!(short.message(), "OpenAI returned HTTP 418.");
    }
    /// A file store that can be told to fail, like a Keychain that refuses.
    struct TestStore {
        file: FileStore,
        /// How many of the next saves fail.
        failing_saves: AtomicUsize,
        saves: AtomicUsize,
        failing_clear: AtomicBool,
        failing_load: AtomicBool,
    }
    impl SessionStore for TestStore {
        fn load(&self) -> Result<Option<Session>, String> {
            if self.failing_load.load(Ordering::SeqCst) {
                return Err("Keychain: read denied".into());
            }
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
            failing_load: AtomicBool::new(false),
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
    /// A session that needs a refresh, and an OpenAI that rejects its
    /// refresh token as already used.
    async fn rejected_refresh() -> Rig {
        let soon = Session {
            expires_at: now() + 60,
            ..sample()
        };
        rig(soon, |req| match req.path.as_str() {
            "/token" => Resp::json(400, json!({"error": "refresh_token_reused"})),
            "/v1/models" => Resp::json(200, json!({"data": []})),
            _ => Resp::answer("{}"),
        })
        .await
    }
    #[tokio::test]
    async fn a_sign_in_refreshed_by_another_copy_is_adopted_without_a_refresh() {
        // Copies A and B of the app restored the same sign-in. B refreshes
        // first and saves the rotated tokens.
        let soon = Session {
            expires_at: now() + 60,
            ..sample()
        };
        let r = rig(soon.clone(), |req| match req.path.as_str() {
            "/token" => rotated(),
            "/v1/models" => Resp::json(200, json!({"data": []})),
            _ => Resp::answer("{}"),
        })
        .await;
        let mut b = Client::new(
            Endpoints::local(&r.fake.base),
            "oaiapp_1".into(),
            r.store.clone(),
            soon,
        );
        b.backoff = Duration::ZERO;
        b.respond(&request()).await.unwrap();
        // A then needs a token: it adopts B's saved set instead of
        // presenting the refresh token B has already spent.
        r.client.respond(&request()).await.unwrap();
        r.client.models().await.unwrap();
        let refreshes = r.fake.seen("/token");
        assert_eq!(refreshes.len(), 1);
        assert_eq!(refreshes[0].field("refresh_token"), "refresh-1");
        let auths: Vec<String> = r
            .fake
            .seen("/v1/responses")
            .into_iter()
            .chain(r.fake.seen("/v1/models"))
            .map(|q| q.auth)
            .collect();
        assert_eq!(auths, ["Bearer access-2"; 3]);
        assert_eq!(r.store.load().unwrap().unwrap().refresh_token, "refresh-2");
    }
    #[tokio::test]
    async fn a_saved_sign_in_that_is_itself_near_expiry_is_refreshed_with_its_own_token() {
        let soon = Session {
            expires_at: now() + 60,
            ..sample()
        };
        let r = rig(soon, |req| match req.path.as_str() {
            "/token" => Resp::json(
                200,
                json!({"access_token": "access-3", "refresh_token": "refresh-3", "expires_in": 3600}),
            ),
            _ => Resp::answer("{}"),
        })
        .await;
        // Newer than this client's set, and near expiry itself.
        let newer = Session {
            access_token: "access-2".into(),
            refresh_token: "refresh-2".into(),
            expires_at: now() + 120,
            ..sample()
        };
        r.store.file.save(&newer).unwrap();
        r.client.respond(&request()).await.unwrap();
        let refreshes = r.fake.seen("/token");
        assert_eq!(refreshes.len(), 1);
        assert_eq!(refreshes[0].field("refresh_token"), "refresh-2");
        assert_eq!(r.fake.seen("/v1/responses")[0].auth, "Bearer access-3");
        assert_eq!(r.store.load().unwrap().unwrap().refresh_token, "refresh-3");
    }
    #[tokio::test]
    async fn a_fatal_refresh_uses_a_newer_saved_sign_in_instead_of_deleting_it() {
        // Another running copy of the app refreshes and saves its tokens
        // while this copy's refresh is on its way.
        let newer = Session {
            access_token: "access-2".into(),
            refresh_token: "refresh-2".into(),
            ..sample()
        };
        let soon = Session {
            expires_at: now() + 60,
            ..sample()
        };
        let path = Arc::new(std::sync::Mutex::new(None::<std::path::PathBuf>));
        let (shared, saved) = (path.clone(), newer.clone());
        let r = rig(soon, move |req| match req.path.as_str() {
            "/token" => {
                if let Some(p) = shared.lock().unwrap().clone() {
                    FileStore(p).save(&saved).unwrap();
                }
                Resp::json(400, json!({"error": "refresh_token_reused"}))
            }
            "/v1/models" => Resp::json(200, json!({"data": []})),
            _ => Resp::answer("{}"),
        })
        .await;
        *path.lock().unwrap() = Some(r.store.file.0.clone());
        r.client.respond(&request()).await.unwrap();
        assert_eq!(r.fake.seen("/v1/responses")[0].auth, "Bearer access-2");
        assert_eq!(r.store.load().unwrap(), Some(newer));
        // The client goes on with the saved session; nothing is refreshed again.
        r.client.models().await.unwrap();
        assert_eq!(r.fake.seen("/v1/models")[0].auth, "Bearer access-2");
        let refreshes = r.fake.seen("/token");
        assert_eq!(refreshes.len(), 1);
        assert_eq!(refreshes[0].field("refresh_token"), "refresh-1");
    }
    #[tokio::test]
    async fn a_fatal_refresh_does_not_adopt_another_users_saved_sign_in() {
        let r = rejected_refresh().await;
        // Another running copy of the app signed in as someone else.
        let other = Session {
            access_token: "access-2".into(),
            refresh_token: "refresh-2".into(),
            subject: "user-2".into(),
            ..sample()
        };
        r.store.file.save(&other).unwrap();
        assert!(matches!(
            r.client.respond(&request()).await,
            Err(ApiError::SignedOut(_))
        ));
        // That sign-in is someone else's: it stays saved, and is not used.
        assert_eq!(r.store.load().unwrap(), Some(other));
        assert!(matches!(
            r.client.models().await,
            Err(ApiError::SignedOut(_))
        ));
        assert!(r.fake.seen("/v1/models").is_empty() && r.fake.seen("/v1/responses").is_empty());
    }
    #[tokio::test]
    async fn a_fatal_refresh_signs_out_when_no_newer_sign_in_can_be_read() {
        // Nothing is saved at all.
        let r = rejected_refresh().await;
        r.store.file.clear().unwrap();
        assert!(matches!(
            r.client.respond(&request()).await,
            Err(ApiError::SignedOut(_))
        ));
        assert_eq!(r.store.load().unwrap(), None);
        // The saved sign-in cannot be read: it may be a newer one, so it is
        // not deleted, but this client's session is gone.
        let r = rejected_refresh().await;
        r.store.failing_load.store(true, Ordering::SeqCst);
        assert!(matches!(
            r.client.respond(&request()).await,
            Err(ApiError::SignedOut(_))
        ));
        r.store.failing_load.store(false, Ordering::SeqCst);
        assert!(r.store.load().unwrap().is_some());
        assert!(matches!(
            r.client.models().await,
            Err(ApiError::SignedOut(_))
        ));
        assert!(r.fake.seen("/v1/models").is_empty() && r.fake.seen("/v1/responses").is_empty());
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
        // The connection drops right after `response.completed`: the reply
        // stays.
        let dropped_after = rig(sample(), |_| Resp {
            cut_after_ms: Some(0),
            ..Resp::answer("{\"ok\":true}")
        })
        .await;
        assert_eq!(
            dropped_after.client.respond(&request()).await.unwrap().text,
            "{\"ok\":true}"
        );
        // A reply that was cut off before it completed is an error.
        let cut_off = rig(sample(), |_| Resp {
            cut_after_ms: Some(0),
            ..Resp::sse(&[json!({"type": "response.output_text.delta", "delta": "{"})])
        })
        .await;
        assert_eq!(
            cut_off.client.respond(&request()).await.unwrap_err(),
            ApiError::Unavailable("The connection to OpenAI was interrupted.".into())
        );
    }
    #[test]
    fn a_refusal_is_shown_as_one_bounded_line() {
        let hostile = format!("No\nway \u{202e}\u{7}{}", "x".repeat(5000));
        let message = ApiError::Refused(hostile).message();
        assert!(message.starts_with("The model declined the request: No way "));
        assert!(!message.contains(['\n', '\u{202e}', '\u{7}']));
        assert!(message.chars().count() <= "The model declined the request: ".len() + 300);
    }
    #[tokio::test]
    async fn a_reply_larger_than_4_mb_is_refused() {
        let r = rig(sample(), |_| {
            let chunk = "x".repeat(1 << 20);
            let mut events = vec![json!({"type": "response.created"})];
            for _ in 0..5 {
                events.push(json!({"type": "response.output_text.delta", "delta": chunk}));
            }
            events.push(json!({"type": "response.completed"}));
            Resp::sse(&events)
        })
        .await;
        assert_eq!(
            r.client.respond(&request()).await.unwrap_err(),
            ApiError::Other("The reply was too large.".into())
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
    /// An HTTP client that waits only a second and a half for data, far
    /// below its total limit, in place of the two minutes the app allows.
    /// The fakes stall for ten seconds, so a slow parallel test run cannot
    /// reach the idle limit before the headers arrive.
    fn impatient() -> reqwest::Client {
        oauth::http_client_with(Duration::from_millis(1_500), Duration::from_secs(30))
    }
    #[tokio::test]
    async fn a_completed_reply_is_returned_without_waiting_for_the_connection_to_close() {
        // The whole reply arrives, then the server holds the connection open.
        let r = rig(sample(), |_| Resp {
            cut_after_ms: Some(10_000),
            ..Resp::answer("{\"ok\":true}")
        })
        .await;
        let started = std::time::Instant::now();
        let reply = r.client.respond(&request()).await.unwrap();
        assert_eq!(reply.text, "{\"ok\":true}");
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "waited {:?} for a connection that was held open",
            started.elapsed()
        );
    }
    #[tokio::test]
    async fn a_stalled_request_or_reply_says_openai_did_not_answer_in_time() {
        let in_time = ApiError::Unavailable("OpenAI did not answer in time.".into());
        // The reply starts and then goes quiet.
        let mut r = rig(sample(), |_| Resp {
            cut_after_ms: Some(10_000),
            ..Resp::sse(&[json!({"type": "response.output_text.delta", "delta": "{"})])
        })
        .await;
        r.client.http = impatient();
        assert_eq!(r.client.respond(&request()).await.unwrap_err(), in_time);
        // No answer at all. The request reached OpenAI, so it is not repeated.
        let mut r = rig(sample(), |_| Resp {
            delay_ms: 10_000,
            ..Resp::answer("{}")
        })
        .await;
        r.client.http = impatient();
        assert_eq!(r.client.respond(&request()).await.unwrap_err(), in_time);
        assert_eq!(r.fake.seen("/v1/responses").len(), 1);
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
            "The saved sign-in could not be deleted (Keychain: delete denied). On macOS, delete the \"AXIS CAM Studio\" item in Keychain Access."
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
            "The saved sign-in could not be deleted (Keychain: delete denied). On macOS, delete the \"AXIS CAM Studio\" item in Keychain Access. OpenAI did not confirm the sign-out either."
        );
        assert_eq!(r.fake.seen("/revoke").len(), 1);
    }
    #[tokio::test]
    async fn a_retired_client_leaves_a_newer_saved_sign_in_alone() {
        let r = rig(sample(), |req| match req.path.as_str() {
            "/v1/models" => Resp {
                delay_ms: 500,
                ..Resp::error(403, "subscription_sharing_invalid_user")
            },
            _ => Resp::json(200, json!({})),
        })
        .await;
        let newer = Session {
            refresh_token: "refresh-new".into(),
            ..sample()
        };
        // The request is still in flight when the user signs out and signs in
        // again; OpenAI then rejects the old session.
        let late = r.client.models();
        let meanwhile = async {
            assert!(until(|| !r.fake.seen("/v1/models").is_empty()).await);
            r.client.sign_out().await.unwrap();
            r.store.save(&newer).unwrap();
        };
        let (result, ()) = tokio::join!(late, meanwhile);
        assert!(matches!(result, Err(ApiError::SignedOut(_))));
        assert_eq!(r.store.load().unwrap(), Some(newer.clone()));
        // A client without a session has nothing to delete or revoke.
        r.client.sign_out().await.unwrap();
        assert_eq!(r.store.load().unwrap(), Some(newer));
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
        let r = rig(soon.clone(), handler).await;
        r.store.failing_saves.store(usize::MAX, Ordering::SeqCst);
        r.client.respond(&request()).await.unwrap();
        r.client.respond(&request()).await.unwrap();
        assert_eq!(r.store.saves.load(Ordering::SeqCst), 2);
        let sent = r.fake.seen("/v1/responses");
        assert_eq!(sent.len(), 2);
        assert!(sent.iter().all(|s| s.auth == "Bearer access-2"));
        assert_eq!(r.fake.seen("/token").len(), 1);
        assert_eq!(r.store.load().unwrap().unwrap().refresh_token, "refresh-1");
        // Both saves of the first refresh fail, so the store keeps the spent
        // refresh-1 set. When the rotated set nears expiry, the next refresh
        // sends its own refresh-2: the older saved set is not adopted.
        let used = std::sync::Mutex::new(Vec::<String>::new());
        let r = rig(soon, move |req| match req.path.as_str() {
            "/token" => {
                // Like OpenAI, each refresh token is accepted once.
                let token = req.field("refresh_token");
                let mut used = used.lock().unwrap();
                if used.contains(&token) {
                    return Resp::json(400, json!({"error": "refresh_token_reused"}));
                }
                used.push(token.clone());
                let next = match token.as_str() {
                    "refresh-1" => json!({"access_token": "access-2", "refresh_token": "refresh-2", "expires_in": 60}),
                    _ => json!({"access_token": "access-3", "refresh_token": "refresh-3", "expires_in": 3600}),
                };
                Resp::json(200, next)
            }
            _ => Resp::answer("{}"),
        })
        .await;
        r.store.failing_saves.store(2, Ordering::SeqCst);
        r.client.respond(&request()).await.unwrap();
        r.client.respond(&request()).await.unwrap();
        let refreshes: Vec<String> = r
            .fake
            .seen("/token")
            .iter()
            .map(|q| q.field("refresh_token"))
            .collect();
        assert_eq!(refreshes, ["refresh-1", "refresh-2"]);
        let auths: Vec<String> = r
            .fake
            .seen("/v1/responses")
            .into_iter()
            .map(|q| q.auth)
            .collect();
        assert_eq!(auths, ["Bearer access-2", "Bearer access-3"]);
        assert_eq!(r.store.load().unwrap().unwrap().refresh_token, "refresh-3");
    }
    /// Like OpenAI, accepts each refresh token once: refresh-n rotates to
    /// refresh-(n+1), with `expires_in` from `lifetimes` (by n, then 3600 s).
    fn rotating(lifetimes: &'static [u64]) -> impl Fn(&Req) -> Resp + Send + Sync + 'static {
        let used = std::sync::Mutex::new(Vec::<String>::new());
        move |req: &Req| match req.path.as_str() {
            "/token" => {
                let token = req.field("refresh_token");
                let mut used = used.lock().unwrap();
                if used.contains(&token) {
                    return Resp::json(400, json!({"error": "refresh_token_reused"}));
                }
                used.push(token.clone());
                let n: usize = token.trim_start_matches("refresh-").parse().unwrap();
                Resp::json(
                    200,
                    json!({
                        "access_token": format!("access-{}", n + 1),
                        "refresh_token": format!("refresh-{}", n + 1),
                        "expires_in": lifetimes.get(n - 1).copied().unwrap_or(3600),
                    }),
                )
            }
            _ => Resp::answer("{}"),
        }
    }
    #[tokio::test]
    async fn a_refresh_token_this_client_spent_is_never_presented_again() {
        let soon = Session {
            expires_at: now() + 60,
            ..sample()
        };
        // refresh-2's set lives 60 s and refresh-3's only 30 s.
        let r = rig(soon, rotating(&[60, 30])).await;
        r.client.respond(&request()).await.unwrap();
        // The second refresh's saves fail: the store keeps refresh-2's set,
        // which expires after this client's refresh-3 set although OpenAI
        // has already spent refresh-2.
        r.store.failing_saves.store(2, Ordering::SeqCst);
        r.client.respond(&request()).await.unwrap();
        assert_eq!(r.store.load().unwrap().unwrap().refresh_token, "refresh-2");
        r.client.respond(&request()).await.unwrap();
        let refreshes: Vec<String> = r
            .fake
            .seen("/token")
            .iter()
            .map(|q| q.field("refresh_token"))
            .collect();
        assert_eq!(refreshes, ["refresh-1", "refresh-2", "refresh-3"]);
        let auths: Vec<String> = r
            .fake
            .seen("/v1/responses")
            .into_iter()
            .map(|q| q.auth)
            .collect();
        assert_eq!(
            auths,
            ["Bearer access-2", "Bearer access-3", "Bearer access-4"]
        );
        assert_eq!(r.store.load().unwrap().unwrap().refresh_token, "refresh-4");
    }
    #[tokio::test]
    async fn a_fatal_refresh_does_not_adopt_an_older_saved_sign_in() {
        let r = rejected_refresh().await;
        // The same user's set from before this client's: it expires sooner.
        let older = Session {
            access_token: "access-0".into(),
            refresh_token: "refresh-0".into(),
            expires_at: now() + 30,
            ..sample()
        };
        r.store.file.save(&older).unwrap();
        assert!(matches!(
            r.client.respond(&request()).await,
            Err(ApiError::SignedOut(_))
        ));
        assert!(r.fake.seen("/v1/responses").is_empty());
        let refreshes = r.fake.seen("/token");
        assert_eq!(refreshes.len(), 1);
        assert_eq!(refreshes[0].field("refresh_token"), "refresh-1");
        // It is not the rejected one, so it is left as it is.
        assert_eq!(r.store.load().unwrap(), Some(older));
    }
    /// A file store whose `save` blocks its thread until the test releases it.
    struct GatedSave {
        file: FileStore,
        started: AtomicBool,
        go: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
    }
    impl SessionStore for GatedSave {
        fn load(&self) -> Result<Option<Session>, String> {
            self.file.load()
        }
        fn save(&self, session: &Session) -> Result<(), String> {
            self.started.store(true, Ordering::SeqCst);
            let _ = self.go.lock().unwrap().recv_timeout(Duration::from_secs(5));
            self.file.save(session)
        }
        fn clear(&self) -> Result<(), String> {
            self.file.clear()
        }
    }
    #[tokio::test]
    async fn a_refresh_is_in_flight_from_its_request_until_its_save_ends() {
        let fake = Fake::start(|req: &Req| match req.path.as_str() {
            "/token" => Resp {
                delay_ms: 200,
                ..rotated()
            },
            _ => Resp::answer("{}"),
        })
        .await;
        let dir = tempfile::tempdir().unwrap();
        let (go, wait) = std::sync::mpsc::channel();
        let soon = Session {
            expires_at: now() + 60,
            ..sample()
        };
        let store = Arc::new(GatedSave {
            file: FileStore(dir.path().join("session.json")),
            started: AtomicBool::new(false),
            go: std::sync::Mutex::new(wait),
        });
        store.file.save(&soon).unwrap();
        let client = Client::new(
            Endpoints::local(&fake.base),
            "oaiapp_1".into(),
            store.clone(),
            soon,
        );
        assert!(!client.refreshing());
        let release = async {
            // OpenAI has the request and has not answered yet.
            assert!(
                until(|| fake
                    .requests
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|r| r.path == "/token"))
                .await
            );
            let requesting = client.refreshing();
            assert!(until(|| store.started.load(Ordering::SeqCst)).await);
            let saving = client.refreshing();
            go.send(()).unwrap();
            (requesting, saving)
        };
        let request = request();
        let (result, (requesting, saving)) = tokio::join!(client.respond(&request), release);
        result.unwrap();
        assert!(requesting && saving);
        assert!(!client.refreshing());
        assert_eq!(store.load().unwrap().unwrap().refresh_token, "refresh-2");
    }
    #[tokio::test]
    async fn a_refresh_save_after_an_abandoning_delete_leaves_the_store_empty() {
        let fake = Fake::start(|req: &Req| match req.path.as_str() {
            "/token" => Resp {
                delay_ms: 300,
                ..rotated()
            },
            _ => Resp::answer("{}"),
        })
        .await;
        let dir = tempfile::tempdir().unwrap();
        let soon = Session {
            expires_at: now() + 60,
            ..sample()
        };
        let store = Arc::new(FileStore(dir.path().join("session.json")));
        store.save(&soon).unwrap();
        let client = Client::new(
            Endpoints::local(&fake.base),
            "oaiapp_1".into(),
            store.clone(),
            soon,
        );
        // A close that stopped waiting for a sign-out deletes the saved
        // sign-in while the refresh's request is still out.
        let abandon = async {
            assert!(
                until(|| fake
                    .requests
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|r| r.path == "/token"))
                .await
            );
            assert!(client.refreshing());
            client.start_abandoning();
            assert!(until(|| store.load().unwrap().is_none()).await);
        };
        let request = request();
        let (result, ()) = tokio::join!(client.respond(&request), abandon);
        // The refresh still finished and its tokens were used, but its late
        // save did not put the sign-in back.
        result.unwrap();
        assert!(!client.refreshing());
        assert_eq!(store.load().unwrap(), None);
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
