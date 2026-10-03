//! Non-secret app settings, kept beside the workspace file and never in projects.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Stable opaque ID for this installation, sent as `ext_agent_host_id`.
    pub host_id: String,
    /// OAuth client ID issued by OpenAI at the first sign-in.
    pub client_id: String,
    pub model: String,
    pub plan_notice_shown: bool,
}
pub fn path(state_path: &Path) -> PathBuf {
    state_path.with_file_name("settings.json")
}
/// Missing or unreadable settings start fresh. The host ID is created once.
pub fn load(path: &Path) -> Settings {
    let mut settings: Settings = std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    if settings.host_id.is_empty() {
        settings.host_id = uuid::Uuid::new_v4().to_string();
        let _ = save(path, &settings);
    }
    settings
}
pub fn save(path: &Path, settings: &Settings) -> Result<(), String> {
    let data = serde_json::to_vec_pretty(settings).map_err(|e| e.to_string())?;
    crate::storage::write_atomic(path, &data)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn host_id_is_created_once_and_settings_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let file = path(&dir.path().join("workspace.json"));
        assert_eq!(file, dir.path().join("settings.json"));
        let first = load(&file);
        assert_eq!(first.host_id.len(), 36);
        assert_eq!(load(&file), first);
        let mut changed = first.clone();
        changed.client_id = "oaiapp_123".into();
        changed.model = "gpt-6.1-sol".into();
        changed.plan_notice_shown = true;
        save(&file, &changed).unwrap();
        assert_eq!(load(&file), changed);
        std::fs::write(&file, b"not json").unwrap();
        let recovered = load(&file);
        assert!(recovered.client_id.is_empty() && recovered.host_id.len() == 36);
    }
}
