# AI Planner M1 — Sign in with ChatGPT Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The user can sign in with their ChatGPT plan from the AI planner page, see the account and its models, run a connection check, and sign out.

**Architecture:** `oauth.rs` runs the authorization-code + PKCE flow with a loopback redirect. `session.rs` stores tokens in the macOS Keychain (an owner-only file for isolated runs and other platforms), and `settings.rs` keeps non-secret settings. `openai.rs` is a Responses API client with single-flight token refresh and SSE streaming. The app holds an `Account` state driven by messages; the planner page shows an account panel.

**Tech Stack:** Rust 2024, Iced 0.14, tokio, reqwest 0.13 (rustls), jsonwebtoken 11 (aws-lc-rs), security-framework 3 (macOS).

**Spec:** `docs/superpowers/specs/2026-10-03-ai-planner-design.md` (section "M1")

## Global Constraints

- Source folder: `/Users/liorknafo/Documents/Codex/2026-09-21/i-x20/outputs/axis-cam-desktop`, branch `ai-planner`. Before every `cargo` command: `export CARGO_TARGET_DIR='/Users/liorknafo/Documents/Codex/2026-09-21/i-x20/work/native-target'`.
- Never read, write or delete anything under `~/Library/Application Support/AXIS CAM Studio`, `~/.codex`, or the login Keychain. Tests use temp folders. The one Keychain test is `#[ignore]` and must not be run.
- Never start a real sign-in, open a browser, or contact `auth.openai.com` / `api.openai.com`. All network tests use the loopback fake.
- Tokens never appear in project files, the workspace file, `settings.json`, status text, logs or `Debug` output.
- Endpoints, scope and parameters exactly as in the spec: authorize `https://auth.openai.com/api/accounts/authorize`, token `https://auth.openai.com/api/accounts/oauth/token`, revoke `https://auth.openai.com/api/accounts/oauth/revoke`, JWKS `https://auth.openai.com/.well-known/jwks.json`, issuer `https://auth.openai.com`, API `https://api.openai.com/v1`, scope `openid profile email offline_access resource.invoke chatgpt.tokens.use.direct`, first client `dynamic_agent_client`, app name `AXIS CAM Studio`.
- Required UI strings (OpenAI guidelines): `Continue with ChatGPT`, `You're using your ChatGPT plan`, `Got it`, `Using ChatGPT plan`, `Usage limit reached`, `Manage usage`. Usage settings URL: `https://chatgpt.com/settings/usage`.
- The sign-in button is text-only. OpenAI's devkit license grants no rights to its logos, so no ChatGPT logo is bundled or drawn.
- `Cargo.lock` changes only in Task 1. After Task 1 use `cargo test --locked`.
- Commit messages end with a blank line and `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>`. Stage only the files each task names.
- Style: match the surrounding code (compact Rust, few comments, plain user-facing sentences).

## Reference implementation

`docs/superpowers/plans/2026-10-03-ai-planner-m1/` holds source files that were prototyped in a scratch crate, where their 27 tests pass (1 Keychain test ignored). Tasks 1–3 move them into `src/` in test-first order. They are the exact code to use. After Task 3, `src/` is authoritative; Task 6 deletes the folder.

| Attachment | Goes to |
| --- | --- |
| `settings.rs`, `session.rs`, `oauth.rs`, `openai.rs`, `fake_openai.rs` | `src/` |
| `fake_openai_key.pem`, `fake_openai_jwks.json` (test-only signing key and its public JWKS) | `src/` |

---

### Task 1: Dependencies, settings and the session store

**Files:**
- Modify: `Cargo.toml`, `Cargo.lock`, `src/main.rs` (module list)
- Create: `src/settings.rs`, `src/session.rs`

**Interfaces:**
- Produces: `settings::{Settings { host_id, client_id, model, plan_notice_shown }, path(&Path) -> PathBuf, load(&Path) -> Settings, save(&Path, &Settings) -> Result<(), String>}`; `session::{Session, PLAN_SCOPE, now() -> u64, SessionStore (load/save/clear), FileStore(PathBuf), KeychainStore (macOS), default_store(&Path, isolated: bool) -> Arc<dyn SessionStore>, sample() (test only)}`.

- [ ] **Step 1: Add the dependencies**

In `Cargo.toml` replace the `tokio` line and add the rest under `[dependencies]`; add the target section after `[dependencies]`' last entry:

```toml
tokio = { version = "1", features = ["rt", "net", "time", "io-util", "sync", "macros"] }
reqwest = { version = "0.13.5", default-features = false, features = ["rustls", "json", "stream", "form"] }
jsonwebtoken = { version = "11.1.0", features = ["aws_lc_rs"] }
base64 = "0.23.1"
getrandom = "0.4.3"
futures-util = "0.3.34"

[target.'cfg(target_os = "macos")'.dependencies]
security-framework = "3.7.0"
```

The `net` and `time` features are what let reqwest and timers run inside Iced's tokio runtime.

Run: `cargo build 2>&1 | tail -3`
Expected: `Finished` (the first build compiles aws-lc and takes a few minutes). `Cargo.lock` now lists the new crates.

- [ ] **Step 2: Write the failing tests**

Create `src/settings.rs` and `src/session.rs` containing only the `#[cfg(test)]` parts of the attachments (in `session.rs` that is `fn sample()` and `mod tests`; in `settings.rs`, `mod tests`). Add to `src/main.rs`, in alphabetical position in the `mod` list:

```rust
mod session;
mod settings;
```

Run: `cargo test --quiet settings:: 2>&1 | tail -5`
Expected: compile errors (`load`, `Session`, `default_store` not found).

- [ ] **Step 3: Add the implementation**

Replace both files with the full attachments (`docs/superpowers/plans/2026-10-03-ai-planner-m1/settings.rs` and `session.rs`), unchanged.

- [ ] **Step 4: Run the tests**

Run: `cargo test --quiet 2>&1 | tail -3`
Expected: `162 passed; 0 failed; 1 ignored` (158 before, +1 settings, +3 session; the Keychain test is ignored). Dead-code warnings for the new modules are expected until Task 4.

- [ ] **Step 5: Commit**

```bash
cargo fmt && git add Cargo.toml Cargo.lock src/main.rs src/settings.rs src/session.rs && git commit -q -m "Settings file and session store for ChatGPT sign-in

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 2: OAuth flow

**Files:**
- Create: `src/oauth.rs`, `src/fake_openai.rs`, `src/fake_openai_key.pem`, `src/fake_openai_jwks.json`
- Modify: `src/main.rs` (module list)

**Interfaces:**
- Consumes: `settings::Settings`, `session::{Session, now}`.
- Produces: `oauth::{SCOPE, Endpoints { authorize, token, revoke, jwks, issuer, api } with openai() and (test) local(base), AuthError { Fatal, Retry }, Attempt, authorize_url, Callback, parse_callback, Listener, exchange, refresh, revoke, open_browser, sign_in(http, endpoints, settings, id_token_hint, limit, open) -> Result<(Session, String), String>}`; test-only `fake_openai::{Fake, Req, Resp, jwks, id_token, claims, tokens, ISSUER}`.

- [ ] **Step 1: Write the failing tests**

Copy `fake_openai.rs`, `fake_openai_key.pem` and `fake_openai_jwks.json` from the attachments into `src/`. Create `src/oauth.rs` containing only the attachment's `#[cfg(test)] mod tests`. Add to `src/main.rs`:

```rust
#[cfg(test)]
mod fake_openai;
mod oauth;
```

Run: `cargo test --locked --quiet oauth:: 2>&1 | tail -5`
Expected: compile errors (`sign_in`, `Endpoints`, `Attempt` not found).

- [ ] **Step 2: Add the implementation**

Replace `src/oauth.rs` with the full attachment, unchanged.

- [ ] **Step 3: Run the tests**

Run: `cargo test --locked --quiet 2>&1 | tail -3`
Expected: `172 passed; 0 failed; 1 ignored` (+10 OAuth tests).

- [ ] **Step 4: Commit**

```bash
cargo fmt && git add src/main.rs src/oauth.rs src/fake_openai.rs src/fake_openai_key.pem src/fake_openai_jwks.json && git commit -q -m "Sign in with ChatGPT: PKCE flow, loopback callback, ID token validation

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 3: OpenAI client

**Files:**
- Create: `src/openai.rs`
- Modify: `src/main.rs` (module list)

**Interfaces:**
- Consumes: `oauth::{Endpoints, AuthError, refresh, revoke}`, `session::{Session, SessionStore, now}`.
- Produces: `openai::{USAGE_URL, ApiError { SignedOut, UsageLimit, NotEligible, Unavailable, Refused, Unsupported, Other } with message(), Role { System, User }, Request { model, messages, schema_name, schema }, Reply { text, structured }, SseParser, Client::new(endpoints, client_id, store, session), Client::{models, respond, check, sign_out}}`.

- [ ] **Step 1: Write the failing tests**

Create `src/openai.rs` containing only the attachment's `#[cfg(test)] mod tests`. Add `mod openai;` to `src/main.rs`.

Run: `cargo test --locked --quiet openai:: 2>&1 | tail -5`
Expected: compile errors (`Client`, `ApiError` not found).

- [ ] **Step 2: Add the implementation**

Replace `src/openai.rs` with the full attachment, unchanged.

- [ ] **Step 3: Run the tests**

Run: `cargo test --locked --quiet 2>&1 | tail -3`
Expected: `185 passed; 0 failed; 1 ignored` (+13 client tests).

- [ ] **Step 4: Commit**

```bash
cargo fmt && git add src/main.rs src/openai.rs && git commit -q -m "OpenAI client on the ChatGPT plan: single-flight refresh and streamed replies

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 4: Account state in the app

**Files:**
- Modify: `src/main.rs` (`Account`, `App` fields, `from_workspace`, `Message`, `update`, helpers)
- Test: `src/smoke.rs` (`mod tests`)

**Interfaces:**
- Consumes: Tasks 1–3.
- Produces: `pub enum Account { SignedOut, SigningIn, SignedIn { name, email, granted } }`; `App { settings, account, account_note, account_busy, usage_limit, models, endpoints (pub(crate)), .. }`; messages `AccountRestored`, `SignIn`, `SignInCancel`, `SignedIn`, `SignOut`, `SignedOut`, `ModelsLoaded`, `PickModel`, `CheckConnection`, `ConnectionChecked`, `PlanNoticeDismissed`, `OpenUrl(&'static str)`.

- [ ] **Step 1: Write the failing tests** (append inside `mod tests` in `src/smoke.rs`)

```rust
    fn signed_in_app(dir: &tempfile::TempDir) -> App {
        let mut a = App::from_workspace(dir.path().join("workspace.json"), None);
        let session = crate::session::sample();
        let _ = a.update(Message::SignedIn(Ok((session, "oaiapp_1".into()))));
        a
    }
    #[test]
    fn signing_in_saves_the_client_and_session_but_never_tokens_in_settings() {
        let dir = tempfile::tempdir().unwrap();
        let a = signed_in_app(&dir);
        assert_eq!(
            a.account,
            crate::Account::SignedIn {
                name: "Lior".into(),
                email: "lior@example.com".into(),
                granted: true
            }
        );
        let settings = std::fs::read_to_string(dir.path().join("settings.json")).unwrap();
        assert!(settings.contains("oaiapp_1"));
        assert!(!settings.contains("access-1") && !settings.contains("refresh-1"));
        let saved = std::fs::read_to_string(dir.path().join("chatgpt-session.json")).unwrap();
        assert!(saved.contains("refresh-1"));
        let reopened = App::from_workspace(dir.path().join("workspace.json"), None);
        assert_eq!(reopened.settings.client_id, "oaiapp_1");
        assert_eq!(reopened.account, crate::Account::SignedOut);
    }
    #[test]
    fn account_messages_drive_the_panel_state() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = App::from_workspace(dir.path().join("workspace.json"), None);
        let _ = a.update(Message::SignIn);
        assert_eq!(a.account, crate::Account::SigningIn);
        let _ = a.update(Message::SignInCancel);
        assert_eq!(a.account, crate::Account::SignedOut);
        assert_eq!(a.account_note, "Sign-in cancelled.");
        let _ = a.update(Message::SignedIn(Err("Sign-in timed out. Try again.".into())));
        assert_eq!(a.account_note, "Sign-in timed out. Try again.");
        let mut a = signed_in_app(&dir);
        let listed = vec!["gpt-6-luna".to_string(), "gpt-6.1-sol".to_string()];
        let _ = a.update(Message::ModelsLoaded(Ok(listed)));
        assert_eq!(a.settings.model, "gpt-6.1-sol");
        let _ = a.update(Message::PickModel("gpt-6-luna".into()));
        let _ = a.update(Message::PickModel("not-listed".into()));
        assert_eq!(a.settings.model, "gpt-6-luna");
        let _ = a.update(Message::ModelsLoaded(Ok(vec!["gpt-6-luna".into()])));
        assert_eq!(a.settings.model, "gpt-6-luna");
        let _ = a.update(Message::CheckConnection);
        assert!(a.account_busy);
        let limit = crate::openai::ApiError::UsageLimit;
        let _ = a.update(Message::ConnectionChecked(Err(limit)));
        assert!(a.usage_limit && !a.account_busy);
        assert!(a.account_note.contains("Usage limit reached"));
        let _ = a.update(Message::ConnectionChecked(Ok("Connection works.".into())));
        assert!(!a.usage_limit);
        assert_eq!(a.account_note, "Connection works.");
        assert!(!a.settings.plan_notice_shown);
        let _ = a.update(Message::PlanNoticeDismissed);
        assert!(crate::settings::load(&dir.path().join("settings.json")).plan_notice_shown);
        let gone = crate::openai::ApiError::SignedOut("Sign in again.".into());
        let _ = a.update(Message::ModelsLoaded(Err(gone)));
        assert_eq!(a.account, crate::Account::SignedOut);
        assert!(a.models.is_empty());
        assert_eq!(a.account_note, "Sign in again.");
    }
    #[test]
    fn a_session_without_plan_usage_is_signed_in_but_not_granted() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = App::from_workspace(dir.path().join("workspace.json"), None);
        let session = crate::session::Session {
            scope: "openid profile email".into(),
            ..crate::session::sample()
        };
        let _ = a.update(Message::SignedIn(Ok((session, "oaiapp_1".into()))));
        assert!(matches!(a.account, crate::Account::SignedIn { granted: false, .. }));
        assert!(a.account_note.contains("not granted"));
        assert!(a.models.is_empty());
        let _ = a.update(Message::SignOut);
        assert_eq!(a.account, crate::Account::SignedOut);
        assert_eq!(a.account_note, "Signed out.");
        // Signing in again waits until the old sign-in has been deleted.
        assert!(a.account_busy);
        let _ = a.update(Message::SignIn);
        assert_eq!(a.account, crate::Account::SignedOut);
        let failed = "Signed out on this computer. Could not reach OpenAI.".to_string();
        let _ = a.update(Message::SignedOut(Err(failed.clone())));
        assert!(!a.account_busy);
        assert_eq!(a.account_note, failed);
        let _ = a.update(Message::SignIn);
        assert_eq!(a.account, crate::Account::SigningIn);
    }
    #[test]
    fn the_saved_session_is_adopted_when_the_planner_page_restores_it() {
        let dir = tempfile::tempdir().unwrap();
        let _ = signed_in_app(&dir);
        let mut a = App::from_workspace(dir.path().join("workspace.json"), None);
        let _ = a.update(Message::Page(Page::Ai));
        assert_eq!(a.account, crate::Account::SignedOut);
        // Unit tests do not run tasks; deliver the restore result directly.
        let saved = crate::session::default_store(dir.path(), true).load();
        let _ = a.update(Message::AccountRestored(saved));
        assert!(matches!(a.account, crate::Account::SignedIn { granted: true, .. }));
        let _ = a.update(Message::AccountRestored(Err("Keychain: denied".into())));
        assert_eq!(a.account_note, "Keychain: denied");
    }
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test --locked --quiet account 2>&1 | tail -5`
Expected: compile errors (`Account`, `Message::SignedIn` not found).

- [ ] **Step 3: Implement in `src/main.rs`**

After the `Target` enum add:

```rust
#[derive(Debug, Clone, PartialEq)]
pub enum Account {
    SignedOut,
    SigningIn,
    SignedIn {
        name: String,
        email: String,
        /// Whether the user allowed requests to use their ChatGPT plan.
        granted: bool,
    },
}
```

Add to `enum Message` after `GcodeExported(..)`:

```rust
    AccountRestored(Result<Option<session::Session>, String>),
    SignIn,
    SignInCancel,
    SignedIn(Result<(session::Session, String), String>),
    SignOut,
    SignedOut(Result<(), String>),
    ModelsLoaded(Result<Vec<String>, openai::ApiError>),
    PickModel(String),
    CheckConnection,
    ConnectionChecked(Result<String, openai::ApiError>),
    PlanNoticeDismissed,
    OpenUrl(&'static str),
```

Add to `struct App` after `post_epoch: u64,`:

```rust
    pub settings: settings::Settings,
    settings_path: PathBuf,
    pub account: Account,
    /// Latest sign-in or connection message for the account panel.
    pub account_note: String,
    pub account_busy: bool,
    pub usage_limit: bool,
    pub models: Vec<String>,
    pub(crate) endpoints: oauth::Endpoints,
    store: std::sync::Arc<dyn session::SessionStore>,
    client: Option<std::sync::Arc<openai::Client>>,
    sign_in: Option<iced::task::Handle>,
    account_restored: bool,
```

At the very start of `from_workspace` (before the `let (mut doc, status) = …` statement, because a recovery workspace renames `state_path`):

```rust
        // Tests and smoke runs keep sign-in data beside their own workspace,
        // never in the real settings file or the Keychain.
        let isolated = smoke_dir.is_some() || state_path != storage::state_path();
        let settings_path = settings::path(&state_path);
        let store = session::default_store(
            state_path.parent().unwrap_or(std::path::Path::new(".")),
            isolated,
        );
```

In the `Self { .. }` literal after `post_epoch: 0,`:

```rust
            settings: settings::load(&settings_path),
            settings_path,
            account: Account::SignedOut,
            account_note: String::new(),
            account_busy: false,
            usage_limit: false,
            models: vec![],
            endpoints: oauth::Endpoints::openai(),
            store,
            client: None,
            sign_in: None,
            account_restored: false,
```

Add methods to `impl App` next to `refresh_post`:

```rust
    fn save_settings(&mut self) {
        if let Err(e) = settings::save(&self.settings_path, &self.settings) {
            self.account_note = format!("Could not save settings: {e}");
        }
    }
    /// Use this session for requests and load the models its plan allows.
    fn adopt_session(&mut self, session: session::Session) -> Task<Message> {
        let granted = session.plan_usage_granted();
        self.account = Account::SignedIn {
            name: session.name.clone(),
            email: session.email.clone(),
            granted,
        };
        self.usage_limit = false;
        let client = std::sync::Arc::new(openai::Client::new(
            self.endpoints.clone(),
            self.settings.client_id.clone(),
            self.store.clone(),
            session,
        ));
        self.client = Some(client.clone());
        if !granted {
            self.account_note = "ChatGPT plan usage was not granted. Sign out, then sign in again and allow plan usage.".into();
            return Task::none();
        }
        self.account_note.clear();
        Task::perform(async move { client.models().await }, Message::ModelsLoaded)
    }
    fn account_error(&mut self, error: openai::ApiError) {
        self.usage_limit = error == openai::ApiError::UsageLimit;
        if matches!(error, openai::ApiError::SignedOut(_)) {
            self.account = Account::SignedOut;
            self.client = None;
            self.models.clear();
        }
        self.account_note = error.message();
    }
    /// Read the saved session the first time the planner page opens. The
    /// Keychain may ask for permission, so this never runs at launch.
    fn restore_account(&mut self) -> Task<Message> {
        if self.account_restored || self.settings.client_id.is_empty() {
            return Task::none();
        }
        self.account_restored = true;
        let store = self.store.clone();
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || store.load())
                    .await
                    .unwrap_or_else(|e| Err(format!("Could not read the saved sign-in: {e}")))
            },
            Message::AccountRestored,
        )
    }
```

Replace the `Message::Page(p)` arm's `return self.refresh_post();` with:

```rust
                let restore = if p == Page::Ai {
                    self.restore_account()
                } else {
                    Task::none()
                };
                return Task::batch([self.refresh_post(), restore]);
```

Add these arms to `update` next to `Message::GcodeExported(..)`:

```rust
            Message::AccountRestored(Ok(Some(session))) => return self.adopt_session(session),
            Message::AccountRestored(Ok(None)) => {}
            Message::AccountRestored(Err(e)) => self.account_note = e,
            Message::SignIn => {
                if self.account == Account::SigningIn || self.account_busy {
                    return Task::none();
                }
                self.account = Account::SigningIn;
                self.account_note.clear();
                let (endpoints, settings) = (self.endpoints.clone(), self.settings.clone());
                let (task, handle) = Task::perform(
                    async move {
                        let http = oauth::http_client(Duration::from_secs(60));
                        let limit = Duration::from_secs(300);
                        oauth::sign_in(&http, &endpoints, &settings, None, limit, oauth::open_browser)
                            .await
                    },
                    Message::SignedIn,
                )
                .abortable();
                self.sign_in = Some(handle);
                return task;
            }
            Message::SignInCancel => {
                if let Some(handle) = self.sign_in.take() {
                    handle.abort();
                }
                if self.account == Account::SigningIn {
                    self.account = Account::SignedOut;
                    self.account_note = "Sign-in cancelled.".into();
                }
            }
            Message::SignedIn(Ok((session, client_id))) => {
                self.sign_in = None;
                self.settings.client_id = client_id;
                self.save_settings();
                if let Err(e) = self.store.save(&session) {
                    self.account = Account::SignedOut;
                    self.account_note = format!("Could not save the ChatGPT sign-in: {e}");
                    return Task::none();
                }
                return self.adopt_session(session);
            }
            Message::SignedIn(Err(e)) => {
                self.sign_in = None;
                self.account = Account::SignedOut;
                self.account_note = e;
            }
            Message::ModelsLoaded(Ok(models)) => {
                self.models = models;
                if !self.models.contains(&self.settings.model) {
                    self.settings.model = self
                        .models
                        .iter()
                        .find(|m| *m == "gpt-6.1-sol")
                        .or(self.models.first())
                        .cloned()
                        .unwrap_or_default();
                    self.save_settings();
                }
            }
            Message::ModelsLoaded(Err(e)) | Message::ConnectionChecked(Err(e)) => {
                self.account_busy = false;
                self.account_error(e);
            }
            Message::PickModel(model) => {
                if self.models.contains(&model) {
                    self.settings.model = model;
                    self.save_settings();
                }
            }
            Message::CheckConnection => {
                let Some(client) = self.client.clone() else {
                    return Task::none();
                };
                if self.account_busy || self.settings.model.is_empty() {
                    return Task::none();
                }
                self.account_busy = true;
                self.account_note = "Checking the connection…".into();
                let model = self.settings.model.clone();
                return Task::perform(
                    async move { client.check(&model).await },
                    Message::ConnectionChecked,
                );
            }
            Message::ConnectionChecked(Ok(message)) => {
                self.account_busy = false;
                self.usage_limit = false;
                self.account_note = message;
            }
            Message::SignOut => {
                let client = self.client.take();
                self.account = Account::SignedOut;
                // Sign-in stays disabled until the old sign-in is deleted, so
                // a late delete cannot remove a newer one.
                self.account_busy = client.is_some();
                self.usage_limit = false;
                self.models.clear();
                self.account_note = "Signed out.".into();
                if let Some(client) = client {
                    return Task::perform(
                        async move { client.sign_out().await },
                        Message::SignedOut,
                    );
                }
            }
            Message::SignedOut(Ok(())) => self.account_busy = false,
            Message::SignedOut(Err(e)) => {
                self.account_busy = false;
                self.account_note = e;
            }
            Message::PlanNoticeDismissed => {
                self.settings.plan_notice_shown = true;
                self.save_settings();
            }
            Message::OpenUrl(url) => {
                if let Err(e) = oauth::open_browser(url) {
                    self.account_note = e;
                }
            }
```

- [ ] **Step 4: Run the tests**

Run: `cargo test --locked --quiet 2>&1 | tail -3`
Expected: 4 more tests than before this task pass (the Tasks 1–3 review added tests, so use the current total: run the suite before starting and add 4); `0 failed; 1 ignored`.

- [ ] **Step 5: Commit**

```bash
cargo fmt && git add src/main.rs src/smoke.rs && git commit -q -m "Account state: sign in, restore, models, connection check, sign out

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 5: Account panel, help and smoke screens

**Files:**
- Modify: `src/ui.rs` (`ai` page, new `account_panel`), `src/help.rs` (`command`), `src/smoke.rs` (names 101–102, steps 101–102)

**Interfaces:**
- Consumes: `App { account, settings, models, account_note, account_busy, sign_out_pending, usage_limit }`, the Task 4 messages (including `LoadModels`, added in the Task 4 review), `openai::USAGE_URL`.

- [ ] **Step 1: Write the failing test** (append inside `mod tests` in `src/help.rs`; if the file has no test module, add `#[cfg(test)] mod tests { use super::*; … }` at the end)

```rust
    #[test]
    fn account_commands_have_specific_help() {
        let generic = command(&Message::Noop, "x".into()).body;
        for message in [
            Message::SignIn,
            Message::SignInCancel,
            Message::SignOut,
            Message::CheckConnection,
            Message::LoadModels,
            Message::PlanNoticeDismissed,
            Message::OpenUrl(crate::openai::USAGE_URL),
        ] {
            let help = command(&message, "x".into());
            assert_ne!(help.body, generic, "{message:?}");
        }
        assert!(command(&Message::SignIn, "x".into()).body.contains("name, email and profile picture"));
        assert!(command(&Message::CheckConnection, "x".into()).body.contains("ChatGPT plan"));
    }
```

Run: `cargo test --locked --quiet account_commands 2>&1 | tail -5`
Expected: FAIL (the generic "Change this setting…" text).

- [ ] **Step 2: Help text** (`src/help.rs`, in `command`'s `let description = match m { … }`, before the final `_ =>` arm)

```rust
        SignIn => {
            "Open your browser to sign in with ChatGPT. AXIS CAM Studio receives your name, email and profile picture. Planning requests then count against your ChatGPT plan."
        }
        SignInCancel => "Stop waiting for the browser sign-in. Nothing is saved.",
        SignOut => {
            "Sign out of ChatGPT on this computer and ask OpenAI to revoke this app's access. The saved sign-in is deleted."
        }
        CheckConnection => {
            "Send one tiny request to the selected model to confirm that your ChatGPT plan works with this app. It uses a very small amount of your plan's usage."
        }
        LoadModels => {
            "Ask OpenAI again which models your ChatGPT plan can use in this app."
        }
        PlanNoticeDismissed => "Close this notice. It is shown once.",
        OpenUrl(_) => {
            "Open your ChatGPT usage settings in the browser, where you can see and limit how much of your plan this app may use."
        }
```

- [ ] **Step 3: Account panel** (`src/ui.rs`, add above `fn ai`)

```rust
fn account_panel(app: &App) -> Element<'_, Message> {
    use crate::Account;
    let manage = || Message::OpenUrl(crate::openai::USAGE_URL);
    let mut c = column![bold("ChatGPT account", 18)].spacing(12);
    match &app.account {
        Account::SignedOut if app.sign_out_pending => c = c.push(muted("Signing out…")),
        Account::SignedOut => {
            c = c
                .push(muted("Sign in to plan with your ChatGPT plan. AXIS CAM Studio receives your name, email and profile picture. Planning requests count against your ChatGPT plan."))
                .push(primary("Continue with ChatGPT", Message::SignIn).width(Fill));
        }
        Account::SigningIn => {
            c = c
                .push(muted("Waiting for sign-in in your browser…"))
                .push(btn("Cancel", Message::SignInCancel, false));
        }
        Account::SignedIn { .. } if !app.settings.plan_notice_shown => {
            c = c.push(
                container(
                    column![
                        bold("You're using your ChatGPT plan", 15),
                        text("Eligible AI requests in this app use your ChatGPT plan. Manage usage in your ChatGPT settings.").size(12),
                        primary("Got it", Message::PlanNoticeDismissed)
                    ]
                    .spacing(10),
                )
                .padding(14)
                .width(Fill)
                .style(panel_style),
            );
        }
        Account::SignedIn {
            name,
            email,
            granted,
        } => {
            let title = if name.is_empty() { email } else { name };
            c = c
                .push(column![bold(title.as_str(), 14), muted(email.clone())].spacing(3))
                .push(
                    row![
                        muted("Using ChatGPT plan"),
                        btn("Manage usage", manage(), false)
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center),
                );
            if *granted && app.models.is_empty() {
                c = c.push(btn("Load models", Message::LoadModels, false).width(Fill));
            }
            if *granted && !app.models.is_empty() {
                let check = if app.account_busy {
                    "Checking…"
                } else {
                    "Check connection"
                };
                c = c
                    .push(
                        column![
                            muted("Model"),
                            pick_list(
                                app.models.clone(),
                                Some(app.settings.model.clone()),
                                Message::PickModel
                            )
                            .width(Fill)
                            .padding([7, 9])
                            .text_size(13)
                        ]
                        .spacing(5),
                    )
                    .push(btn(check, Message::CheckConnection, false).width(Fill));
            }
            c = c.push(btn("Sign out", Message::SignOut, false).width(Fill));
        }
    }
    if app.usage_limit {
        c = c
            .push(bold("Usage limit reached", 13))
            .push(primary("Manage usage", manage()).width(Fill));
    } else if !app.account_note.is_empty() {
        c = c.push(note(app.account_note.clone()));
    }
    c.into()
}
```

In `fn ai`, replace `container(scrollable(controls))` with:

```rust
                container(scrollable(
                    column![account_panel(app), rule::horizontal(1), controls].spacing(18)
                ))
```

and replace the page subtitle `"A reviewable planning workflow, ready for a future OpenAI integration."` with `"Sign in with ChatGPT. The planner below is still the local example for the sample housing."`.

Confirm the required strings against `https://developers.openai.com/siwc/ui-ux-guidelines.md` (read-only fetch). If the guideline's first-sign-in body text differs from the sentence above, use the guideline's exact text and say so in the report.

- [ ] **Step 4: Smoke screens** (`src/smoke.rs`)

In the name table add after `100 => "77-back-to-machining".into(),`:

```rust
                101 => "78-chatgpt-notice".into(),
                102 => "79-chatgpt-signed-in".into(),
```

In the actions `match`, after the `100 => { … }` arm:

```rust
            101 => {
                self.account = crate::Account::SignedIn {
                    name: "Lior".into(),
                    email: "lior@example.com".into(),
                    granted: true,
                };
                self.models = vec!["gpt-6.1-sol".into(), "gpt-6-luna".into()];
                self.settings.model = "gpt-6.1-sol".into();
                self.settings.plan_notice_shown = false;
                vec![Message::Page(Page::Ai)]
            }
            102 => vec![Message::PlanNoticeDismissed],
```

- [ ] **Step 5: Run tests and the renderer smoke**

```bash
cargo test --locked --quiet 2>&1 | tail -3
rm -rf '/Users/liorknafo/Documents/Codex/2026-09-21/i-x20/work/account-smoke'
cargo run --release --locked -- --smoke-test '/Users/liorknafo/Documents/Codex/2026-09-21/i-x20/work/account-smoke' 2>&1 | tail -3
ls '/Users/liorknafo/Documents/Codex/2026-09-21/i-x20/work/account-smoke' | grep -c png
```

Expected: one more test than before this task; `0 failed; 1 ignored`; the smoke exits 0 with `103` PNGs. Open `08-ai.png` (signed out: the Continue with ChatGPT button), `78-chatgpt-notice.png` and `79-chatgpt-signed-in.png` with the Read tool and confirm the text is readable and nothing overlaps. Describe each in the report. The smoke run uses its own folder and never signs in.

- [ ] **Step 6: Commit**

```bash
cargo fmt && git add src/ui.rs src/help.rs src/smoke.rs && git commit -q -m "ChatGPT account panel on the planner page

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 6: Documentation, licenses and full verification

**Files:**
- Modify: `README.md`, `docs/VALIDATION.md`, `THIRD_PARTY.md`
- Create: `licenses/aws-lc-LICENSE.txt`
- Delete: `docs/superpowers/plans/2026-10-03-ai-planner-m1/`

- [ ] **Step 1: README**

Add before `## Persistence`:

```markdown
## Sign in with ChatGPT

The AI planner page has a **ChatGPT account** panel. **Continue with ChatGPT** opens your browser; after you approve, the app can send planning requests on your ChatGPT plan. No API key is used.

- OpenAI shares your name, email and profile picture with the app. The app cannot read your ChatGPT conversations.
- Requests count against your ChatGPT plan, up to a weekly per-app cap you set at [chatgpt.com/settings/usage](https://chatgpt.com/settings/usage). The panel's **Manage usage** opens that page.
- The sign-in is stored in the macOS Keychain (item `AXIS CAM Studio`). It is never written to project files. Non-secret settings (the chosen model, this installation's ID) are in `settings.json` beside the workspace file. On other platforms the sign-in is an owner-only file in the same folder; those platforms are unverified.
- **Check connection** sends one tiny request to the selected model. **Sign out** deletes the saved sign-in and asks OpenAI to revoke it.
- After rebuilding the app from source, macOS may ask once to allow the new build to read its Keychain item.

In this version the account panel is the only part that talks to OpenAI. The planner below it is still the local example.
```

In `## Run`, add one sentence after the Open CASCADE prerequisite: `The HTTPS client builds aws-lc, which needs a C compiler and CMake.`

In `## Code map`, add lines in the same style as the existing entries for: `src/oauth.rs` (Sign in with ChatGPT: PKCE, loopback redirect, ID token validation, refresh, revoke), `src/session.rs` (session and its Keychain or file store), `src/settings.rs` (non-secret app settings), `src/openai.rs` (Responses API client on the ChatGPT plan), `src/fake_openai.rs` (test-only loopback stand-in for OpenAI).

- [ ] **Step 2: Third-party notices**

```bash
cp ~/.cargo/registry/src/*/aws-lc-sys-0.45.0/aws-lc/LICENSE licenses/aws-lc-LICENSE.txt
```

If the locked `aws-lc-sys` version differs, use the version in `Cargo.lock`. Append to `THIRD_PARTY.md`:

```markdown
HTTPS and token validation use rustls, reqwest and jsonwebtoken with the AWS-LC cryptographic library (aws-lc-rs, aws-lc-sys), statically linked. AWS-LC is distributed under the ISC, Apache-2.0 and OpenSSL licenses; its license text is in `licenses/aws-lc-LICENSE.txt` and is copied to the app's `Contents/Resources/Licenses` directory. Keychain access uses security-framework (MIT or Apache-2.0).

"ChatGPT" and "OpenAI" are trademarks of OpenAI. This app signs in with ChatGPT through OpenAI's documented flow for locally run apps; it is not affiliated with or endorsed by OpenAI and bundles no OpenAI logos.
```

- [ ] **Step 3: Remove the reference attachments**

```bash
git rm -r -q docs/superpowers/plans/2026-10-03-ai-planner-m1
```

- [ ] **Step 4: Full verification**

```bash
cargo fmt --check && cargo test --locked --quiet 2>&1 | tail -2 && cargo build --release --locked 2>&1 | grep -E "^warning: unused|never used|never read|never constructed|Finished"
```

Expected: no fmt output; the same total as after Task 5 with `0 failed; 1 ignored`; `Finished` with no dead-code warnings (the only warning allowed is the existing `block v0.1.6` future-compatibility note). If a new item is unused, report it; do not add `#[allow]`.

```bash
grep -rn "access_token\|refresh_token" src/ui.rs src/help.rs src/smoke.rs | grep -v "^src/smoke.rs.*sample" | head
```

Expected: no matches (UI code never touches tokens).

- [ ] **Step 5: VALIDATION**

Update the test count in `docs/VALIDATION.md` to the Step 4 result and the smoke count to 79 static screens plus 24 motion samples (also in README's smoke sentence: `seventy-nine screens`). Add:

```markdown
## Sign in with ChatGPT

- Verified against a loopback stand-in for OpenAI's servers; no test contacts OpenAI or the Keychain.
- First sign-in registers the app, exchanges the code with PKCE and validates the ID token (RS256 signature, issuer, audience, expiry, nonce). Wrong state, denial, timeout and each invalid token are refused.
- Tokens refresh before expiry and once after a 401, one refresh at a time, saved before use; fatal refresh errors and sign-out clear the saved sign-in.
- Streamed replies require completion; failures, refusals, early ends and each plan-usage error map to a clear state. Unsupported structured output falls back to plain JSON once.
- Tokens never appear in settings, Debug output or UI code.
- Not verified here: a live sign-in. That needs the user's own ChatGPT account.
```

- [ ] **Step 6: Commit**

```bash
git add README.md docs/VALIDATION.md THIRD_PARTY.md licenses/aws-lc-LICENSE.txt && git commit -q -m "Document ChatGPT sign-in; third-party notices; remove plan attachments

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

## Live check (the user, after Task 6)

Only the user can sign in. With the old installed app closed, run the new build:

```bash
'/Users/liorknafo/Documents/Codex/2026-09-21/i-x20/work/native-target/release/axis-cam'
```

Open **AI planner** → **Continue with ChatGPT** → approve in the browser → the panel shows the account, **Using ChatGPT plan** and the model list → **Check connection** reports success → **Sign out**.
