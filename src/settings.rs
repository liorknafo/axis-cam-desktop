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
    /// The AI planner may not show the model pictures of the part.
    pub plan_pictures_off: bool,
    /// The Work page's layout as the user left it.
    pub layout: Layout,
}
/// The browser or the inspector: the sidebars beside the Work page's 3D view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Browser,
    Inspector,
}
/// The Work page's layout as the user left it: each sidebar's width and
/// whether it is open, and whether the operation inspector's About
/// disclosure is open. The widths are what was dragged; `ui::fitted` gives
/// what the window has room for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Layout {
    pub browser_width: f32,
    pub inspector_width: f32,
    pub browser_open: bool,
    pub inspector_open: bool,
    pub strategy_help_open: bool,
}
impl Default for Layout {
    fn default() -> Self {
        Self {
            browser_width: crate::ui::BROWSER.default,
            inspector_width: crate::ui::INSPECTOR.default,
            browser_open: true,
            inspector_open: true,
            strategy_help_open: false,
        }
    }
}
impl Layout {
    pub fn width(&self, side: Side) -> f32 {
        match side {
            Side::Browser => self.browser_width,
            Side::Inspector => self.inspector_width,
        }
    }
    pub fn width_mut(&mut self, side: Side) -> &mut f32 {
        match side {
            Side::Browser => &mut self.browser_width,
            Side::Inspector => &mut self.inspector_width,
        }
    }
    pub fn open(&self, side: Side) -> bool {
        match side {
            Side::Browser => self.browser_open,
            Side::Inspector => self.inspector_open,
        }
    }
    pub fn open_mut(&mut self, side: Side) -> &mut bool {
        match side {
            Side::Browser => &mut self.browser_open,
            Side::Inspector => &mut self.inspector_open,
        }
    }
}
pub fn path(state_path: &Path) -> PathBuf {
    state_path.with_file_name("settings.json")
}
/// Missing settings start fresh. The host ID is created once. A saved ID that
/// is not valid is repaired and saved: a bare version-4 UUID from an earlier
/// build keeps its UUID, anything else gets a new ID.
#[cfg(test)]
pub fn load(path: &Path) -> Settings {
    load_reporting(path).0
}
/// `load`, and a sentence for the user when the file was there but could not
/// be read. Such a file is never written over: it is renamed to
/// `settings-unreadable-<id>.json` beside it, and only the two IDs are taken
/// from it, so the saved sign-in can still be found and used.
pub fn load_reporting(path: &Path) -> (Settings, Option<String>) {
    let mut note = None;
    // `changed`: the file's place is to get this run's settings. `writable`:
    // it may, because nothing there is left to lose.
    let (mut changed, mut writable) = (false, true);
    let mut settings = match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|_| {
            let (aside, moved) = set_aside(path);
            (note, changed, writable) = (Some(aside), true, moved);
            salvaged_ids(&bytes)
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Settings::default(),
        // A file that cannot be read now may be fine later: leave it alone.
        Err(e) => {
            note = Some(format!(
                "The settings file could not be read ({e}), so the app started with fresh settings that are not saved."
            ));
            writable = false;
            Settings::default()
        }
    };
    if !valid_host_id(&settings.host_id) {
        let prefixed = format!("urn:uuid:{}", settings.host_id);
        settings.host_id = if valid_host_id(&prefixed) {
            prefixed
        } else {
            new_host_id()
        };
        changed = true;
    }
    if changed && writable {
        let _ = save(path, &settings);
    }
    (settings, note)
}
/// Renames the unreadable file aside. Returns what to tell the user and
/// whether the file was moved; one that could not be moved is left alone.
fn set_aside(path: &Path) -> (String, bool) {
    let name = format!("settings-unreadable-{}.json", uuid::Uuid::new_v4());
    match std::fs::rename(path, path.with_file_name(&name)) {
        Ok(()) => (
            format!(
                "The settings file could not be read, so the app started with fresh settings. The old file is kept beside it as {name}."
            ),
            true,
        ),
        Err(e) => (
            format!(
                "The settings file could not be read, and could not be set aside ({e}), so the app started with fresh settings that are not saved."
            ),
            false,
        ),
    }
}
/// The installation's IDs, if the file is JSON and holds them as text.
fn salvaged_ids(bytes: &[u8]) -> Settings {
    let value: serde_json::Value = serde_json::from_slice(bytes).unwrap_or_default();
    let text = |key: &str| value.get(key).and_then(|v| v.as_str()).map(str::to_owned);
    Settings {
        host_id: text("host_id").unwrap_or_default(),
        client_id: text("client_id")
            .filter(|id| crate::oauth::valid_client_id(id))
            .unwrap_or_default(),
        ..Settings::default()
    }
}
/// A host ID is `urn:uuid:` and a lowercase version-4 UUID, the form OpenAI's
/// authorize endpoint accepts as `ext_agent_host_id`.
pub fn valid_host_id(id: &str) -> bool {
    let Some(uuid) = id.strip_prefix("urn:uuid:") else {
        return false;
    };
    uuid.len() == 36
        && uuid.bytes().enumerate().all(|(i, b)| match i {
            8 | 13 | 18 | 23 => b == b'-',
            14 => b == b'4',
            19 => matches!(b, b'8' | b'9' | b'a' | b'b'),
            _ => matches!(b, b'0'..=b'9' | b'a'..=b'f'),
        })
}
fn new_host_id() -> String {
    format!("urn:uuid:{}", uuid::Uuid::new_v4())
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
        assert!(valid_host_id(&first.host_id), "{}", first.host_id);
        assert_eq!(load(&file), first);
        let mut changed = first.clone();
        changed.client_id = "oaiapp_123".into();
        changed.model = "gpt-6.1-sol".into();
        changed.plan_notice_shown = true;
        save(&file, &changed).unwrap();
        assert_eq!(load(&file), changed);
        std::fs::write(&file, b"not json").unwrap();
        let recovered = load(&file);
        assert!(recovered.client_id.is_empty() && valid_host_id(&recovered.host_id));
    }
    #[test]
    fn an_unreadable_file_is_renamed_aside_and_its_ids_are_kept() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("settings.json");
        let host = format!("urn:uuid:{V4}");
        let broken = serde_json::json!({ "host_id": host, "client_id": "oaiapp_1", "model": 5 });
        std::fs::write(&file, broken.to_string()).unwrap();
        let (loaded, note) = load_reporting(&file);
        assert_eq!(
            (loaded.host_id.as_str(), loaded.client_id.as_str()),
            (host.as_str(), "oaiapp_1")
        );
        assert!(loaded.model.is_empty());
        let note = note.expect("the user is told");
        let aside: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .filter(|n| n.starts_with("settings-unreadable-"))
            .collect();
        assert_eq!(aside.len(), 1);
        assert!(note.contains(&aside[0]), "{note}");
        assert_eq!(
            std::fs::read(dir.path().join(&aside[0])).unwrap(),
            broken.to_string().into_bytes()
        );
        // Fresh settings are saved in its place; a readable file has no note.
        assert_eq!(load_reporting(&file), (loaded, None));
    }
    #[test]
    fn pictures_are_on_unless_turned_off_and_the_choice_is_saved() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        assert!(!load(&path).plan_pictures_off);
        let mut s = load(&path);
        s.plan_pictures_off = true;
        save(&path, &s).unwrap();
        assert!(load(&path).plan_pictures_off);
        // Settings saved before the field existed read as pictures on.
        std::fs::write(
            &path,
            br#"{"host_id":"urn:uuid:3b241101-e2bb-4255-8caf-4136c566a962","model":"gpt-6.1-sol"}"#,
        )
        .unwrap();
        assert!(!load(&path).plan_pictures_off);
    }
    /// The Work page's layout is saved with the settings, and a file saved
    /// before it existed, or with some of its fields, reads as the defaults
    /// for what it lacks (#76, #77).
    #[test]
    fn the_work_page_layout_is_saved_and_a_file_without_it_reads_as_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let fresh = load(&path).layout;
        assert_eq!(fresh, Layout::default());
        assert!(fresh.browser_open && fresh.inspector_open && !fresh.strategy_help_open);
        assert_eq!(
            (fresh.browser_width, fresh.inspector_width),
            (crate::ui::BROWSER.default, crate::ui::INSPECTOR.default)
        );
        let mut s = load(&path);
        s.layout = Layout {
            browser_width: 300.,
            inspector_width: 400.,
            browser_open: false,
            inspector_open: true,
            strategy_help_open: true,
        };
        save(&path, &s).unwrap();
        assert_eq!(load(&path), s);
        for (side, width, open) in [(Side::Browser, 300., false), (Side::Inspector, 400., true)] {
            assert_eq!(s.layout.width(side), width);
            assert_eq!(s.layout.open(side), open);
        }
        std::fs::write(
            &path,
            br#"{"host_id":"urn:uuid:3b241101-e2bb-4255-8caf-4136c566a962","layout":{"inspector_width":500}}"#,
        )
        .unwrap();
        let partial = load(&path).layout;
        assert_eq!(
            partial,
            Layout {
                inspector_width: 500.,
                ..Layout::default()
            }
        );
        std::fs::write(
            &path,
            br#"{"host_id":"urn:uuid:3b241101-e2bb-4255-8caf-4136c566a962","model":"gpt-6.1-sol"}"#,
        )
        .unwrap();
        assert_eq!(load(&path).layout, Layout::default());
    }
    const V4: &str = "3f2b8c1e-9d4a-4e6f-a1b2-0c9d8e7f6a5b";
    fn saved_with(host_id: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("settings.json");
        std::fs::write(&file, serde_json::json!({ "host_id": host_id }).to_string()).unwrap();
        (dir, file)
    }
    #[test]
    fn a_new_installation_gets_a_urn_uuid_host_id_that_is_saved() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("settings.json");
        let created = load(&file);
        assert!(
            created.host_id.starts_with("urn:uuid:"),
            "{}",
            created.host_id
        );
        assert!(valid_host_id(&created.host_id), "{}", created.host_id);
        let on_disk: Settings = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        assert_eq!(on_disk.host_id, created.host_id);
    }
    #[test]
    fn a_bare_v4_uuid_from_an_earlier_build_keeps_its_value_with_the_prefix() {
        let (_dir, file) = saved_with(V4);
        let migrated = load(&file);
        assert_eq!(migrated.host_id, format!("urn:uuid:{V4}"));
        let on_disk: Settings = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        assert_eq!(on_disk.host_id, migrated.host_id);
    }
    #[test]
    fn host_ids_that_are_not_lowercase_v4_are_replaced_with_a_new_valid_one() {
        let upper = V4.to_uppercase();
        let bad = [
            "garbage".to_string(),
            String::new(),
            upper.clone(),
            format!("urn:uuid:{upper}"),
            // version 1
            "3f2b8c1e-9d4a-1e6f-a1b2-0c9d8e7f6a5b".to_string(),
            // version 4 with a variant outside 8 9 a b
            "3f2b8c1e-9d4a-4e6f-c1b2-0c9d8e7f6a5b".to_string(),
            "urn:uuid:3f2b8c1e-9d4a-1e6f-a1b2-0c9d8e7f6a5b".to_string(),
        ];
        for saved in bad {
            let (_dir, file) = saved_with(&saved);
            let replaced = load(&file);
            assert!(valid_host_id(&replaced.host_id), "{saved:?}");
            assert_ne!(replaced.host_id, saved, "{saved:?}");
            assert_ne!(replaced.host_id, format!("urn:uuid:{saved}"), "{saved:?}");
            let on_disk: Settings = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
            assert_eq!(on_disk.host_id, replaced.host_id, "{saved:?}");
        }
    }
    #[test]
    fn a_valid_host_id_is_kept_and_the_file_is_not_rewritten() {
        let saved = format!("urn:uuid:{V4}");
        let (_dir, file) = saved_with(&saved);
        let before = std::fs::read(&file).unwrap();
        assert_eq!(load(&file).host_id, saved);
        assert_eq!(std::fs::read(&file).unwrap(), before);
    }
    #[test]
    fn valid_host_id_accepts_and_rejects_the_right_cases() {
        for ok in [
            format!("urn:uuid:{V4}"),
            "urn:uuid:00000000-0000-4000-8000-000000000000".to_string(),
            "urn:uuid:ffffffff-ffff-4fff-bfff-ffffffffffff".to_string(),
            "urn:uuid:12345678-9abc-4def-9012-3456789abcde".to_string(),
        ] {
            assert!(valid_host_id(&ok), "{ok}");
        }
        for bad in [
            "",
            "urn:uuid:",
            V4,
            "URN:UUID:3f2b8c1e-9d4a-4e6f-a1b2-0c9d8e7f6a5b",
            "urn:uuid:3F2B8C1E-9D4A-4E6F-A1B2-0C9D8E7F6A5B",
            "urn:uuid:3f2b8c1e-9d4a-4e6f-a1b2-0c9d8e7f6a5",
            "urn:uuid:3f2b8c1e-9d4a-4e6f-a1b2-0c9d8e7f6a5bb",
            " urn:uuid:3f2b8c1e-9d4a-4e6f-a1b2-0c9d8e7f6a5b",
            "urn:uuid:3f2b8c1e-9d4a-4e6f-a1b2-0c9d8e7f6a5b ",
            "urn:uuid:3f2b8c1e-9d4a-1e6f-a1b2-0c9d8e7f6a5b",
            "urn:uuid:3f2b8c1e-9d4a-5e6f-a1b2-0c9d8e7f6a5b",
            "urn:uuid:3f2b8c1e-9d4a-4e6f-71b2-0c9d8e7f6a5b",
            "urn:uuid:3f2b8c1e-9d4a-4e6f-c1b2-0c9d8e7f6a5b",
            "urn:uuid:3f2b8c1e9d4a4e6fa1b20c9d8e7f6a5b",
            "urn:uuid:3f2b8c1e-9d4a-4e6f-a1b2-0c9d8e7f6a5g",
            "urn:uuid:3f2b8c1e_9d4a-4e6f-a1b2-0c9d8e7f6a5b",
            "urn:uuid:3f2b8c1e-9d4a-4e6f-a1b2-0c9d8e7f6a5é",
        ] {
            assert!(!valid_host_id(bad), "{bad:?}");
        }
    }
}
