//! The signed-in ChatGPT session. Tokens live in the macOS Keychain (an
//! owner-only file elsewhere) and never in projects, settings or logs.
use serde::{Deserialize, Serialize};
use std::{
    fmt,
    path::{Path, PathBuf},
    sync::Arc,
};

pub const PLAN_SCOPE: &str = "chatgpt.tokens.use.direct";

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub access_token: String,
    pub refresh_token: String,
    pub id_token: String,
    /// Unix seconds when the access token expires.
    pub expires_at: u64,
    pub scope: String,
    pub subject: String,
    pub email: String,
    pub name: String,
}
impl fmt::Debug for Session {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Session")
            .field("email", &self.email)
            .field("name", &self.name)
            .field("expires_at", &self.expires_at)
            .field("scope", &self.scope)
            .field("tokens", &"<redacted>")
            .finish()
    }
}
impl Session {
    pub fn plan_usage_granted(&self) -> bool {
        self.scope.split_whitespace().any(|s| s == PLAN_SCOPE)
    }
    pub fn expires_within(&self, seconds: u64, now: u64) -> bool {
        self.expires_at <= now + seconds
    }
}
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}
pub trait SessionStore: Send + Sync {
    fn load(&self) -> Result<Option<Session>, String>;
    fn save(&self, session: &Session) -> Result<(), String>;
    fn clear(&self) -> Result<(), String>;
}
/// Why a sign-in is not saved when the app has no data directory of its own.
pub const NO_HOME: &str = "Signing in is unavailable: this account has no home directory, so the sign-in cannot be kept privately. Set HOME or XDG_DATA_HOME and start the app again.";
/// Owner-only file, written atomically. Used off macOS and by isolated runs.
pub struct FileStore(pub PathBuf);
impl SessionStore for FileStore {
    fn load(&self) -> Result<Option<Session>, String> {
        match std::fs::read(&self.0) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|e| format!("The saved sign-in is unreadable: {e}")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }
    fn save(&self, session: &Session) -> Result<(), String> {
        let data = serde_json::to_vec(session).map_err(|e| e.to_string())?;
        let parent = self.0.parent().ok_or("Missing parent directory")?;
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        let tmp = parent.join(format!(".session-{}.tmp", uuid::Uuid::new_v4()));
        let result = (|| {
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
            let mut file = options.open(&tmp)?;
            std::io::Write::write_all(&mut file, &data)?;
            file.sync_all()?;
            std::fs::rename(&tmp, &self.0)
        })()
        .map_err(|e: std::io::Error| e.to_string());
        if result.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        result
    }
    fn clear(&self) -> Result<(), String> {
        match std::fs::remove_file(&self.0) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
            _ => Ok(()),
        }
    }
}
/// One Keychain item of the service "AXIS CAM Studio".
#[cfg(target_os = "macos")]
pub struct KeychainStore {
    /// The item's account name.
    pub account: &'static str,
}
#[cfg(target_os = "macos")]
impl KeychainStore {
    const SERVICE: &str = "AXIS CAM Studio";
    /// errSecItemNotFound
    const NOT_FOUND: i32 = -25300;
    /// The item that holds the user's sign-in.
    pub fn saved_sign_in() -> Self {
        Self {
            account: "chatgpt-session",
        }
    }
}
#[cfg(target_os = "macos")]
impl SessionStore for KeychainStore {
    fn load(&self) -> Result<Option<Session>, String> {
        match security_framework::passwords::get_generic_password(Self::SERVICE, self.account) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|e| format!("The saved sign-in is unreadable: {e}")),
            Err(e) if e.code() == Self::NOT_FOUND => Ok(None),
            Err(e) => Err(format!("Keychain: {e}")),
        }
    }
    fn save(&self, session: &Session) -> Result<(), String> {
        let data = serde_json::to_vec(session).map_err(|e| e.to_string())?;
        security_framework::passwords::set_generic_password(Self::SERVICE, self.account, &data)
            .map_err(|e| format!("Keychain: {e}"))
    }
    fn clear(&self) -> Result<(), String> {
        match security_framework::passwords::delete_generic_password(Self::SERVICE, self.account) {
            Err(e) if e.code() != Self::NOT_FOUND => Err(format!("Keychain: {e}")),
            _ => Ok(()),
        }
    }
}
/// The Keychain for the real workspace on macOS; a file beside an isolated
/// (test or smoke) workspace and on other platforms.
pub fn default_store(data_dir: &Path, isolated: bool, shared: bool) -> Arc<dyn SessionStore> {
    #[cfg(target_os = "macos")]
    if !isolated {
        return Arc::new(KeychainStore::saved_sign_in());
    }
    file_store(data_dir, shared && !isolated)
}

/// The sign-in file in `data_dir`. When `data_dir` is the shared temp
/// directory, which the app falls back to with no home directory, another
/// local user may own it, so nothing is saved there and sign-in fails with
/// `NO_HOME`.
pub fn file_store(data_dir: &Path, shared: bool) -> Arc<dyn SessionStore> {
    if shared {
        return Arc::new(NoHomeStore);
    }
    Arc::new(FileStore(data_dir.join("chatgpt-session.json")))
}
/// Holds nothing and saves nothing: there is no private place for a sign-in.
struct NoHomeStore;
impl SessionStore for NoHomeStore {
    fn load(&self) -> Result<Option<Session>, String> {
        Ok(None)
    }
    fn save(&self, _: &Session) -> Result<(), String> {
        Err(NO_HOME.into())
    }
    fn clear(&self) -> Result<(), String> {
        Ok(())
    }
}

/// Starts deleting the saved sign-in on Tokio's blocking pool and returns at
/// once. The runtime lets blocking work finish when it shuts down, so the
/// delete completes even when the app ends right after. Call it from within
/// a Tokio runtime.
pub fn start_clear(store: Arc<dyn SessionStore>) {
    drop(tokio::task::spawn_blocking(move || store.clear()));
}
#[cfg(test)]
pub(crate) fn sample() -> Session {
    Session {
        access_token: "access-1".into(),
        refresh_token: "refresh-1".into(),
        id_token: "id-1".into(),
        expires_at: now() + 3600,
        scope: format!("openid profile email offline_access resource.invoke {PLAN_SCOPE}"),
        subject: "user-1".into(),
        email: "lior@example.com".into(),
        name: "Lior".into(),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    /// A store whose delete takes a while, like a Keychain that is slow.
    struct SlowClear(FileStore);
    impl SessionStore for SlowClear {
        fn load(&self) -> Result<Option<Session>, String> {
            self.0.load()
        }
        fn save(&self, session: &Session) -> Result<(), String> {
            self.0.save(session)
        }
        fn clear(&self) -> Result<(), String> {
            std::thread::sleep(std::time::Duration::from_millis(200));
            self.0.clear()
        }
    }
    #[test]
    fn a_sign_in_is_refused_under_the_shared_temp_directory() {
        let dir = tempfile::tempdir().unwrap();
        let store = file_store(dir.path(), true);
        let error = store.save(&sample()).unwrap_err();
        assert_eq!(error, NO_HOME);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
        assert_eq!(store.load().unwrap(), None);
        store.clear().unwrap();
        // The same directory is fine when it is the user's own.
        let own = file_store(dir.path(), false);
        own.save(&sample()).unwrap();
        assert_eq!(own.load().unwrap(), Some(sample()));
    }
    #[test]
    fn a_clear_started_just_before_the_runtime_ends_still_finishes() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(SlowClear(FileStore(dir.path().join("session.json"))));
        store.save(&sample()).unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let started = store.clone();
        runtime.block_on(async move { start_clear(started) });
        // Ending the runtime, as closing the app does, waits for the delete.
        drop(runtime);
        assert_eq!(store.load().unwrap(), None);
    }
    #[test]
    fn debug_output_never_contains_tokens() {
        let text = format!("{:?}", sample());
        assert!(text.contains("lior@example.com") && text.contains("<redacted>"));
        for secret in ["access-1", "refresh-1", "id-1"] {
            assert!(!text.contains(secret), "{text}");
        }
    }
    #[test]
    fn plan_scope_and_expiry() {
        let mut s = sample();
        assert!(s.plan_usage_granted());
        assert!(!s.expires_within(300, now()));
        assert!(s.expires_within(3700, now()));
        s.scope = "openid profile email".into();
        assert!(!s.plan_usage_granted());
    }
    #[test]
    fn file_store_roundtrips_with_owner_only_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let store = default_store(dir.path(), true, false);
        assert_eq!(store.load().unwrap(), None);
        store.save(&sample()).unwrap();
        assert_eq!(store.load().unwrap().unwrap().refresh_token, "refresh-1");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.path().join("chatgpt-session.json"))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        store.clear().unwrap();
        store.clear().unwrap();
        assert_eq!(store.load().unwrap(), None);
        std::fs::write(dir.path().join("chatgpt-session.json"), b"{").unwrap();
        assert!(store.load().is_err());
    }
    /// The item of the ignored test below, never the user's saved sign-in.
    #[cfg(target_os = "macos")]
    const TEST_ITEM: KeychainStore = KeychainStore {
        account: "chatgpt-session-test",
    };
    #[cfg(target_os = "macos")]
    #[test]
    fn the_keychain_test_item_is_not_the_saved_sign_in() {
        // Nothing here touches the Keychain: only the item names are compared.
        assert_eq!(KeychainStore::saved_sign_in().account, "chatgpt-session");
        assert_ne!(TEST_ITEM.account, KeychainStore::saved_sign_in().account);
    }
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "writes to the login Keychain; run with --ignored"]
    fn keychain_store_roundtrips() {
        let store = TEST_ITEM;
        let session = sample();
        let saved = store.save(&session);
        let loaded = store.load();
        // The test item is deleted whatever the checks below find.
        let cleared = store.clear();
        saved.unwrap();
        assert_eq!(loaded.unwrap(), Some(session));
        cleared.unwrap();
        assert_eq!(store.load().unwrap(), None);
    }
}
