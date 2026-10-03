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
#[cfg(target_os = "macos")]
pub struct KeychainStore;
#[cfg(target_os = "macos")]
impl KeychainStore {
    const SERVICE: &str = "AXIS CAM Studio";
    const ACCOUNT: &str = "chatgpt-session";
    /// errSecItemNotFound
    const NOT_FOUND: i32 = -25300;
}
#[cfg(target_os = "macos")]
impl SessionStore for KeychainStore {
    fn load(&self) -> Result<Option<Session>, String> {
        match security_framework::passwords::get_generic_password(Self::SERVICE, Self::ACCOUNT) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|e| format!("The saved sign-in is unreadable: {e}")),
            Err(e) if e.code() == Self::NOT_FOUND => Ok(None),
            Err(e) => Err(format!("Keychain: {e}")),
        }
    }
    fn save(&self, session: &Session) -> Result<(), String> {
        let data = serde_json::to_vec(session).map_err(|e| e.to_string())?;
        security_framework::passwords::set_generic_password(Self::SERVICE, Self::ACCOUNT, &data)
            .map_err(|e| format!("Keychain: {e}"))
    }
    fn clear(&self) -> Result<(), String> {
        match security_framework::passwords::delete_generic_password(Self::SERVICE, Self::ACCOUNT) {
            Err(e) if e.code() != Self::NOT_FOUND => Err(format!("Keychain: {e}")),
            _ => Ok(()),
        }
    }
}
/// The Keychain for the real workspace on macOS; a file beside an isolated
/// (test or smoke) workspace and on other platforms.
pub fn default_store(data_dir: &Path, isolated: bool) -> Arc<dyn SessionStore> {
    #[cfg(target_os = "macos")]
    if !isolated {
        return Arc::new(KeychainStore);
    }
    let _ = isolated;
    Arc::new(FileStore(data_dir.join("chatgpt-session.json")))
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
        let store = default_store(dir.path(), true);
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
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "writes to the login Keychain; run with --ignored"]
    fn keychain_store_roundtrips() {
        let store = KeychainStore;
        let before = store.load().unwrap();
        let session = sample();
        store.save(&session).unwrap();
        assert_eq!(store.load().unwrap(), Some(session));
        match before {
            Some(saved) => store.save(&saved).unwrap(),
            None => store.clear().unwrap(),
        }
    }
}
