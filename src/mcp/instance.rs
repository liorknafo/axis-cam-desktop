//! Instance discovery (LIO-470): the files by which an open AXIS CAM
//! window tells an MCP client it is there, and how the client chooses its
//! backend from them.
//!
//! Each desktop app writes `AXIS CAM Studio/mcp/<pid>.json` (`Instance`),
//! beside `workspace.json`, with `Instance::write`: mode `0600` on Unix, and
//! on Windows in `%LOCALAPPDATA%`, whose inherited ACL keeps it to the user.
//! With agent access off it still writes it, `state: "off"` and no port.
//! The app deletes only its own file on close; a reader deletes the files
//! whose process is not alive (`session::process_alive`: the pid and its
//! start time on macOS and Linux; elsewhere the file's modification time,
//! which the app refreshes every few seconds as a session lock's owner does).
//!
//! This version writes no instance file, and has no bridge to a window
//! (the next milestone, M2, adds both, writing this same `Instance`). The
//! client side is here: a server finds the windows at each call until its
//! headless session is committed (it has dispatched a change,
//! `session::Sessions`), and never goes headless while a window is open
//! or while the folder of these files cannot be read.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Whether the window lets agents in (its Agents toggle).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Access {
    On,
    Off,
}

/// An instance file: one open AXIS CAM window. The window's side, which
/// writes it and reads its port and token, comes in M2.
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Instance {
    pub pid: u32,
    /// The process's start time as its OS gives it (`session::started`),
    /// which a process given the same pid later does not share.
    #[serde(default)]
    pub started: Option<String>,
    /// When the app started, s since 1970.
    #[serde(default)]
    pub start_time: u64,
    /// The loopback port its bridge listens on; none while access is off.
    #[serde(default)]
    pub port: Option<u16>,
    /// The token a client gives the bridge: 256 random bits, as hex.
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub project_title: String,
    #[serde(default)]
    pub project_file: Option<PathBuf>,
    pub state: Access,
}

/// The folder of the instance files, beside the user's `workspace`.
pub fn folder(workspace: &Path) -> PathBuf {
    workspace.with_file_name("mcp")
}

#[cfg_attr(not(test), allow(dead_code))]
impl Instance {
    /// This process as a window would describe itself, access `state`.
    pub fn of_this_process(state: Access) -> Instance {
        let pid = std::process::id();
        Instance {
            pid,
            started: super::session::started(pid),
            start_time: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs()),
            port: None,
            token: None,
            project_title: String::new(),
            project_file: None,
            state,
        }
    }
    /// Its file in `folder`: `<pid>.json`.
    pub fn path(&self, folder: &Path) -> PathBuf {
        folder.join(format!("{}.json", self.pid))
    }
    /// Writes its file in `folder` atomically, readable by the user alone
    /// on Unix (`0600` from its creation).
    pub fn write(&self, folder: &Path) -> Result<PathBuf, String> {
        use std::io::Write;
        std::fs::create_dir_all(folder).map_err(|e| e.to_string())?;
        let path = self.path(folder);
        let data = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        let tmp = folder.join(format!(".{}-{}.tmp", self.pid, uuid::Uuid::new_v4()));
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let result = options
            .open(&tmp)
            .and_then(|mut f| f.write_all(&data).and_then(|()| f.sync_all()))
            .and_then(|()| std::fs::rename(&tmp, &path));
        if let Err(e) = result {
            let _ = std::fs::remove_file(&tmp);
            return Err(format!("{}: {e}", path.display()));
        }
        Ok(path)
    }
    /// How it is named in an error: pid, project and start.
    fn describe(&self) -> String {
        let project = match (&self.project_title, &self.project_file) {
            (t, Some(f)) if !t.is_empty() => format!("\"{t}\", {}", f.display()),
            (t, None) if !t.is_empty() => format!("\"{t}\""),
            (_, Some(f)) => f.display().to_string(),
            _ => "no project named".into(),
        };
        let since = super::checkpoint::utc(self.start_time);
        format!("pid {} ({project}; started {since})", self.pid)
    }
}

/// What the instance files show: the windows open now.
#[derive(Debug, Clone, PartialEq)]
pub enum Found {
    /// No window: headless.
    None,
    /// The windows open, by pid.
    Windows(Vec<Instance>),
    /// A live window's file that cannot be read: its pid and why.
    Unreadable(u32, String),
    /// The instance folder, or an entry of it, cannot be read (not merely
    /// missing): whether a window is open is not known. Why.
    Unknown(String),
}

/// Reads the instance files in `folder`, deleting those whose process is
/// not alive. A folder that is not there means no window; one that cannot
/// be listed, or an entry whose metadata cannot be read, is `Unknown`,
/// never taken as no window. A file gone between the listing and its read
/// (a window closing) is skipped.
pub fn discover(folder: &Path) -> Found {
    use std::io::ErrorKind::NotFound;
    let mut windows = vec![];
    let entries = match std::fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(e) if e.kind() == NotFound => return Found::None,
        Err(e) => return Found::Unknown(format!("{}: {e}", folder.display())),
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(e) => return Found::Unknown(format!("{}: {e}", folder.display())),
        };
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(pid) = name
            .strip_suffix(".json")
            .and_then(|p| p.parse::<u32>().ok())
        else {
            continue;
        };
        let beat = match std::fs::metadata(&path) {
            Ok(meta) => meta.modified().ok(),
            Err(e) if e.kind() == NotFound => continue,
            Err(e) => return Found::Unknown(format!("{}: {e}", path.display())),
        };
        let read = match std::fs::read(&path) {
            Err(e) if e.kind() == NotFound => continue,
            read => read
                .map_err(|e| e.to_string())
                .and_then(|b| serde_json::from_slice::<Instance>(&b).map_err(|e| e.to_string())),
        };
        match read {
            Ok(i) if super::session::process_alive(i.pid, i.started.as_deref(), beat) => {
                windows.push(i)
            }
            Ok(_) => {
                let _ = std::fs::remove_file(&path);
            }
            // Unreadable: a window being written, or a broken file. Its
            // pid alone tells whether a process has it (any start time).
            Err(why) => {
                if pid_exists(pid, beat) {
                    return Found::Unreadable(pid, why);
                }
                let _ = std::fs::remove_file(&path);
            }
        }
    }
    windows.sort_by_key(|i| i.pid);
    if windows.is_empty() {
        Found::None
    } else {
        Found::Windows(windows)
    }
}
/// Whether some process has `pid`, whenever it started; on Windows, by
/// the file's time alone.
fn pid_exists(pid: u32, beat: Option<std::time::SystemTime>) -> bool {
    if cfg!(any(target_os = "macos", target_os = "linux")) {
        super::session::started(pid).is_some()
    } else {
        super::session::process_alive(pid, None, beat)
    }
}

/// The error while access is off in the one window open.
pub const ACCESS_OFF: &str = "Agent access is off in the open AXIS CAM window. Turn it on in Agents, or close the app to work without a window.";

/// Why a call cannot go to a headless session, the windows being what
/// `found` shows; None when there is none. `chosen` is a pid the agent
/// chose with `select_window`.
pub fn refusal(found: &Found, chosen: Option<u32>) -> Option<String> {
    match found {
        Found::None => None,
        Found::Unreadable(pid, why) => Some(format!(
            "An AXIS CAM window (pid {pid}) is open, but its instance file cannot be read ({why}). Close the app to work without a window."
        )),
        Found::Unknown(why) => Some(format!(
            "Whether an AXIS CAM window is open cannot be told: its instance folder cannot be read ({why}). Make the folder readable, or remove it if no AXIS CAM window is open."
        )),
        Found::Windows(all) => {
            let one = match (all.as_slice(), chosen) {
                ([one], _) => one,
                (_, Some(pid)) => match all.iter().find(|i| i.pid == pid) {
                    Some(one) => one,
                    None => return Some(several(all)),
                },
                _ => return Some(several(all)),
            };
            Some(match one.state {
                Access::Off => format!("{ACCESS_OFF} (pid {})", one.pid),
                Access::On => window_open(one),
            })
        }
    }
}
/// The error for a window with access on: driving it comes in M2.
fn window_open(one: &Instance) -> String {
    format!(
        "An AXIS CAM window is open ({}). Driving the open window comes in the next version; close the app to work without a window.",
        one.describe()
    )
}
fn several(all: &[Instance]) -> String {
    let list: Vec<String> = all
        .iter()
        .map(|i| {
            let access = match i.state {
                Access::On => "agent access on",
                Access::Off => "agent access off",
            };
            format!("{}, {access}", i.describe())
        })
        .collect();
    format!(
        "Several AXIS CAM windows are open: {}. Call select_window {{pid}} to choose one.",
        list.join("; ")
    )
}

/// `select_window {pid}`: the window of `pid` among those `found` shows,
/// or why there is none. In this version every window chosen is refused
/// (`refusal`): driving one comes next.
pub fn select(found: &Found, pid: u32) -> String {
    match found {
        Found::Windows(all) if all.iter().any(|i| i.pid == pid) => {
            refusal(found, Some(pid)).unwrap_or_default()
        }
        Found::Unreadable(..) | Found::Unknown(..) => refusal(found, None).unwrap_or_default(),
        _ => format!("No AXIS CAM window with pid {pid} is open."),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A file whose process is gone: a pid no process has, a start no
    /// process had, and not refreshed for an hour (Windows).
    pub(crate) fn dead(folder: &Path) -> PathBuf {
        let mut i = Instance::of_this_process(Access::On);
        i.pid = 999_999;
        i.started = Some("1.000001".into());
        let path = i.write(folder).unwrap();
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
        (std::fs::File::options().write(true).open(&path))
            .unwrap()
            .set_modified(old)
            .unwrap();
        path
    }

    #[test]
    fn dead_files_are_deleted_and_live_ones_found() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(discover(dir.path()), Found::None, "no folder");
        let gone = dead(dir.path());
        assert_eq!(discover(dir.path()), Found::None);
        assert!(!gone.exists());
        let mut me = Instance::of_this_process(Access::Off);
        me.project_title = "Bracket".into();
        let path = me.write(dir.path()).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "readable by the user alone");
        }
        let found = discover(dir.path());
        assert_eq!(found, Found::Windows(vec![me.clone()]));
        assert_eq!(
            refusal(&found, None).unwrap(),
            format!("{ACCESS_OFF} (pid {})", me.pid)
        );
        // A broken file of a live pid is never taken as no window.
        std::fs::write(&path, "{").unwrap();
        assert!(matches!(discover(dir.path()), Found::Unreadable(pid, _) if pid == me.pid));
        assert!(path.exists());
    }
}
