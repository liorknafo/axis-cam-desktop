//! Headless sessions on disk (LIO-470): what a session keeps beside its
//! workspace so a client's end or a crash loses nothing, the lock that keeps
//! it to one process, and the session tools (`list_sessions`,
//! `resume_session`, `delete_session`).
//!
//! A session's folder, `AXIS CAM Studio/mcp-sessions/<id>/`, holds:
//! - `workspace.json`: the `Document`, saved by the app (`persist`);
//! - `settings.json`: the app's settings for the session, its project file;
//! - `session.json` (`SessionFile`): the rest a resume needs, which the app
//!   does not save: the open drafts, the selection with its page and tab,
//!   the revision with its change log, the last snapshot and the revision
//!   last reported, the project file and whether it is saved, the paths of
//!   the last saves and exports, and the checkpoints' index. It is written
//!   (`storage::write_atomic`) with the `Document` before a result about
//!   anything that changed is sent (`engine::Backend::keep`);
//! - `checkpoints/`: the checkpoints (`checkpoint`);
//! - `lock`: the process that has it open (`Lock`).
//!
//! Sessions are never deleted on their own: one may hold the only copy of
//! a job. `delete_session` deletes one, never an open one, and one with work
//! not saved to a project file only when asked to discard it.
use super::checkpoint::{Drafts, Meta, Place};
use super::engine::Reply;
use super::runner::{Runner, Session};
use super::state::Log;
use crate::App;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::mpsc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

/// A save or export the agent made: the command, where it wrote, and when
/// (s since 1970). The last of each command is kept.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Output {
    pub what: String,
    pub path: String,
    pub at: u64,
}
/// Keeps `path` as the last place `what` wrote.
pub fn record_output(app: &mut App, what: &str, path: &Path) {
    let outputs = &mut app.mcp.outputs;
    outputs.retain(|o| o.what != what);
    outputs.push(Output {
        what: what.into(),
        path: path.display().to_string(),
        at: now_s(),
    });
}

fn now_s() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// `session.json`: what a resume needs that the workspace does not hold.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionFile {
    /// The project's title, for `list_sessions`.
    #[serde(default)]
    pub title: String,
    /// The open library drafts.
    #[serde(default)]
    pub drafts: Option<Drafts>,
    /// The selection, the page and the tab.
    #[serde(default)]
    pub place: Place,
    /// The revision, the change log, the last snapshot and the revision
    /// last reported to the agent.
    #[serde(default)]
    pub log: Log,
    #[serde(default)]
    pub project_path: Option<PathBuf>,
    #[serde(default)]
    pub project_dirty: bool,
    /// The last saves and exports.
    #[serde(default)]
    pub outputs: Vec<Output>,
    /// The checkpoints' index (`checkpoint::Kept`).
    #[serde(default)]
    pub checkpoints: Vec<Meta>,
    /// When it was written, s since 1970.
    #[serde(default)]
    pub written: u64,
}
/// The file's name in the session's folder.
pub const FILE: &str = "session.json";
impl SessionFile {
    pub fn of(app: &App) -> SessionFile {
        SessionFile {
            title: app.doc.title.clone(),
            drafts: Some(Drafts::of(app)),
            place: Place::of(app),
            log: app.mcp.log.clone(),
            project_path: app.project_path.clone(),
            project_dirty: app.project_dirty,
            outputs: app.mcp.outputs.clone(),
            checkpoints: app.mcp.checkpoints.list.clone(),
            written: 0,
        }
    }
    /// What changed of it since `key`: its hash, without when it was
    /// written.
    pub fn key(&self) -> u64 {
        super::state::hash_of(self)
    }
    pub fn read(folder: &Path) -> Option<Result<SessionFile, String>> {
        let path = folder.join(FILE);
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
            Err(e) => return Some(Err(format!("{}: {e}", path.display()))),
        };
        Some(serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display())))
    }
    pub fn write(mut self, folder: &Path) -> Result<(), String> {
        self.written = now_s();
        let data = serde_json::to_vec_pretty(&self).map_err(|e| e.to_string())?;
        crate::storage::write_atomic(&folder.join(FILE), &data)
    }
    /// Puts what it kept into `app`, loaded from the session's workspace:
    /// all but the selection, which `put_place` puts once the project is
    /// open again.
    pub fn put(&self, app: &mut App) {
        if let Some(drafts) = &self.drafts {
            drafts.clone().put(app);
        }
        app.mcp.log = self.log.clone();
        app.mcp.outputs = self.outputs.clone();
        app.mcp.checkpoints = super::checkpoint::Kept::from_list(self.checkpoints.clone());
        app.project_path = self.project_path.clone().filter(|p| p.is_file());
        app.project_dirty = self.project_dirty || app.project_path.is_none();
    }
    pub fn put_place(&self, app: &mut App) {
        self.place.put(app);
    }
    /// Whether the job is saved to a project file.
    pub fn saved(&self) -> bool {
        self.project_path.is_some() && !self.project_dirty
    }
}

/// The lock file's name in a session's folder.
const LOCK: &str = "lock";
/// How often the owner refreshes its lock, and how old a lock is that no
/// owner refreshed (where liveness is told by it).
const BEAT: Duration = Duration::from_secs(5);
const STALE: Duration = Duration::from_secs(30);

/// Who holds a session's lock.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Owner {
    pid: u32,
    /// The process's start time as its OS gives it (`started`), which a
    /// process given the same pid later does not share.
    #[serde(default)]
    started: Option<String>,
    /// When the lock was taken, s since 1970.
    at: u64,
}

/// When process `pid` started, as its OS tells it; None when there is no
/// such process, or the OS is not asked (Windows).
///
/// Liveness of a lock's owner:
/// - **macOS:** `proc_pidinfo(PROC_PIDTBSDINFO)` gives the process's start
///   time; the owner is alive when a process with its pid exists and
///   started when it did (`libc`, already a macOS dependency).
/// - **Linux:** the same from `/proc/<pid>/stat` (its 22nd field, the start
///   time in clock ticks since boot), with std only.
/// - **Elsewhere (Windows):** std has no way to ask about another process
///   and no new crate is added for it, so the owner refreshes its lock
///   file's modification time every `BEAT` (5 s), and a lock not refreshed
///   for `STALE` (30 s) is taken as dead.
fn started(pid: u32) -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        let pid = i32::try_from(pid).ok()?;
        // SAFETY: proc_pidinfo writes at most `size` bytes into `info`, a
        // plain C struct for which all zeroes is a valid value.
        unsafe {
            let mut info: libc::proc_bsdinfo = std::mem::zeroed();
            let size = std::mem::size_of::<libc::proc_bsdinfo>() as i32;
            let got =
                libc::proc_pidinfo(pid, libc::PROC_PIDTBSDINFO, 0, (&raw mut info).cast(), size);
            (got == size).then(|| format!("{}.{:06}", info.pbi_start_tvsec, info.pbi_start_tvusec))
        }
    }
    #[cfg(target_os = "linux")]
    {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        // After the command's closing parenthesis: field 3 (state) on, so
        // the start time, field 22, is the 20th.
        let rest = &stat[stat.rfind(')')? + 1..];
        rest.split_whitespace().nth(19).map(String::from)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = pid;
        None
    }
}

/// Whether the owner of a lock whose file was last refreshed at `beat` is
/// alive (`started` says how it is told).
fn alive(owner: &Owner, beat: Option<SystemTime>) -> bool {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        let _ = beat;
        owner.started.is_some() && started(owner.pid) == owner.started
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = owner;
        fresh(beat)
    }
}
/// Whether a lock file last refreshed at `beat` was refreshed within
/// `STALE`.
fn fresh(beat: Option<SystemTime>) -> bool {
    beat.and_then(|t| t.elapsed().ok())
        .is_some_and(|age| age < STALE)
}

/// The live owner of the lock in `folder`, if any: the lock file read, and
/// whether its owner is alive. An unreadable lock counts as held while it is
/// fresh (it may be being written), else as dead.
fn holder(folder: &Path) -> Option<(Owner, bool, Vec<u8>)> {
    let path = folder.join(LOCK);
    let bytes = std::fs::read(&path).ok()?;
    let beat = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
    match serde_json::from_slice::<Owner>(&bytes) {
        Ok(owner) => {
            let live = alive(&owner, beat);
            Some((owner, live, bytes))
        }
        Err(_) => {
            let fresh = fresh(beat);
            let owner = Owner {
                pid: 0,
                started: None,
                at: 0,
            };
            Some((owner, fresh, bytes))
        }
    }
}

/// A session's lock, held while this process has it open: a file created
/// exclusively (`create_new`) with the owner's pid and start time, its
/// modification time refreshed every `BEAT`. Dropped, it is removed.
#[derive(Debug)]
pub struct Lock {
    path: PathBuf,
    written: Vec<u8>,
    stop: Option<mpsc::Sender<()>>,
    beat: Option<std::thread::JoinHandle<()>>,
}
impl Lock {
    /// Takes session `id`'s lock in `folder`: refused while another live
    /// process holds it; a lock whose owner is dead is taken over.
    pub fn take(folder: &Path, id: &str) -> Result<Lock, String> {
        let path = folder.join(LOCK);
        let pid = std::process::id();
        let me = Owner {
            pid,
            started: started(pid),
            at: now_s(),
        };
        let written = serde_json::to_vec(&me).map_err(|e| e.to_string())?;
        for _ in 0..4 {
            use std::io::Write;
            let created = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path);
            match created {
                Ok(mut file) => {
                    file.write_all(&written)
                        .and_then(|()| file.sync_all())
                        .map_err(|e| format!("Could not lock session {id}: {e}"))?;
                    return Ok(Lock::beating(path, written));
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    match holder(folder) {
                        Some((owner, true, _)) => {
                            return Err(match owner.pid {
                                0 => format!(
                                    "Session {id} is open in another MCP server (its lock is being written)."
                                ),
                                pid => format!(
                                    "Session {id} is open in another MCP server (pid {pid})."
                                ),
                            });
                        }
                        // Dead: taken over, if it is still that lock.
                        Some((_, false, bytes))
                            if std::fs::read(&path).ok().as_ref() == Some(&bytes) =>
                        {
                            let _ = std::fs::remove_file(&path);
                        }
                        // Gone or changed meanwhile: try again.
                        _ => {}
                    }
                }
                Err(e) => return Err(format!("Could not lock session {id}: {e}")),
            }
        }
        Err(format!(
            "Session {id} could not be locked: another process keeps taking its lock."
        ))
    }
    fn beating(path: PathBuf, written: Vec<u8>) -> Lock {
        let (stop, stopped) = mpsc::channel::<()>();
        let beat_path = path.clone();
        let beat = std::thread::Builder::new()
            .name("mcp-session-lock".into())
            .spawn(move || {
                while let Err(mpsc::RecvTimeoutError::Timeout) = stopped.recv_timeout(BEAT) {
                    if let Ok(file) = std::fs::File::options().write(true).open(&beat_path) {
                        let _ = file.set_modified(SystemTime::now());
                    }
                }
            })
            .ok();
        Lock {
            path,
            written,
            stop: Some(stop),
            beat,
        }
    }
}
impl Drop for Lock {
    fn drop(&mut self) {
        drop(self.stop.take());
        if let Some(beat) = self.beat.take() {
            let _ = beat.join();
        }
        // Only its own: a lock taken over meanwhile is another's.
        if std::fs::read(&self.path).ok().as_ref() == Some(&self.written) {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// Whether `id` names a session folder: letters, digits, `-` and `_`.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// The bytes the files under `path` hold.
fn size_of(path: &Path) -> u64 {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return 0;
    };
    if !meta.is_dir() {
        return meta.len();
    }
    (std::fs::read_dir(path).into_iter().flatten().flatten())
        .map(|e| size_of(&e.path()))
        .sum()
}

/// What `list_sessions` says of one session.
fn describe(root: &Path, id: &str) -> Option<Value> {
    let folder = root.join(id);
    if !folder.is_dir() || !valid_id(id) {
        return None;
    }
    let kept = SessionFile::read(&folder).and_then(Result::ok);
    let modified = |name: &str| {
        std::fs::metadata(folder.join(name))
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs())
    };
    let last = [modified(FILE), modified("workspace.json")]
        .into_iter()
        .flatten()
        .max();
    let title = match &kept {
        Some(k) => k.title.clone(),
        // Never worked in: the copy of the user's workspace.
        None => std::fs::read(folder.join("workspace.json"))
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .and_then(|v| v.get("title")?.as_str().map(String::from))
            .unwrap_or_default(),
    };
    let owner = holder(&folder).filter(|(_, live, _)| *live);
    Some(json!({
        "id": id,
        "title": title,
        "last_change": last.map(super::checkpoint::utc),
        "saved_to_project_file": kept.as_ref().is_some_and(SessionFile::saved),
        "project_file": kept.as_ref().and_then(|k| k.project_path.as_ref()).map(|p| p.display().to_string()),
        "revision": kept.as_ref().map_or(0, |k| k.log.revision),
        "checkpoints": kept.as_ref().map_or(0, |k| k.checkpoints.len()),
        "size_bytes": size_of(&folder),
        "open": owner.is_some(),
        "pid": owner.map(|(o, ..)| o.pid),
        "folder": folder.display().to_string(),
    }))
}

/// The sessions in `root`, the last changed first.
pub fn list(root: &Path) -> Vec<Value> {
    let mut sessions: Vec<Value> = (std::fs::read_dir(root).into_iter().flatten().flatten())
        .filter_map(|e| describe(root, &e.file_name().to_string_lossy()))
        .collect();
    sessions.sort_by(|a, b| b["last_change"].as_str().cmp(&a["last_change"].as_str()));
    sessions
}

/// A refused session tool's result.
fn refused(why: String) -> Value {
    json!({"outcome": "refused", "reason": why})
}

/// Deletes session `id` in `root`: refused while it is open (its lock is
/// held by a live process, this one's included), and while it has work not
/// saved to a project file unless `discard_unsaved`.
pub fn delete(root: &Path, id: &str, discard_unsaved: bool) -> Result<Value, String> {
    if !valid_id(id) {
        return Err(format!("\"{id}\" is not a session id."));
    }
    let folder = root.join(id);
    if !folder.is_dir() {
        return Err(format!(
            "There is no MCP session {id} in {}.",
            root.display()
        ));
    }
    // Held while it is deleted: no server opens it meanwhile.
    let lock = match Lock::take(&folder, id) {
        Ok(lock) => lock,
        Err(why) => {
            return Ok(refused(format!(
                "{why} It cannot be deleted while it is open."
            )));
        }
    };
    let kept = SessionFile::read(&folder).and_then(Result::ok);
    if !discard_unsaved && !kept.as_ref().is_some_and(SessionFile::saved) {
        let title = describe(root, id)
            .and_then(|d| d["title"].as_str().map(String::from))
            .unwrap_or_default();
        return Ok(refused(format!(
            "Session {id} (\"{title}\") holds work not saved to a project file; deleting it would lose it. Resume it and save the project, or call again with discard_unsaved: true."
        )));
    }
    drop(lock);
    std::fs::remove_dir_all(&folder)
        .map_err(|e| format!("Could not delete {}: {e}", folder.display()))?;
    Ok(json!({"outcome": "done", "deleted": id}))
}

/// The session tools, and the headless session the other tools go to:
/// opened at the start (`--session <id>`), resumed (`resume_session`), or
/// made new at the first tool that needs one.
pub struct Sessions {
    root: PathBuf,
    workspace: PathBuf,
    runner: Option<(String, Runner)>,
}
impl Sessions {
    pub fn new(root: PathBuf, workspace: PathBuf, opened: Option<Session>) -> Sessions {
        Sessions {
            root,
            workspace,
            runner: opened.map(|s| (s.id.clone(), Runner::start(s))),
        }
    }
    /// Whether `tool` is one of the session tools.
    pub fn is_tool(tool: &str) -> bool {
        matches!(tool, "list_sessions" | "resume_session" | "delete_session")
    }
    /// The session tools, as `tools/list` lists them.
    pub fn tools() -> Vec<Value> {
        vec![
            json!({
                "name": "list_sessions",
                "title": "List the headless sessions",
                "description": "The headless sessions kept on disk, the last changed first: id, title, last change, whether the job is saved to a project file, size, and whether a server has it open. Sessions are never deleted on their own.",
                "inputSchema": {"type": "object", "properties": {}},
                "annotations": {"readOnlyHint": true},
            }),
            json!({
                "name": "resume_session",
                "title": "Resume a headless session",
                "description": "Continues headless session `id` where it was left: its job, drafts, selection, revision, change log and checkpoints, its part loaded again. The session open now is left on disk. Refused while another MCP server has it open.",
                "inputSchema": {
                    "type": "object",
                    "properties": {"id": {"type": "string"}},
                    "required": ["id"],
                },
            }),
            json!({
                "name": "delete_session",
                "title": "Delete a headless session",
                "description": "Deletes headless session `id` and everything in its folder. Refused for an open session (this one included), and for one whose job is not saved to a project file unless `discard_unsaved: true`.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "id": {"type": "string"},
                        "discard_unsaved": {"type": "boolean"},
                    },
                    "required": ["id"],
                },
                "annotations": {"destructiveHint": true},
            }),
        ]
    }
    /// The session open now, or a new one.
    fn runner(&mut self) -> Result<&Runner, String> {
        if self.runner.is_none() {
            let session = Session::open(&self.root, None, &self.workspace)?;
            self.runner = Some((session.id.clone(), Runner::start(session)));
        }
        Ok(&self.runner.as_ref().expect("a session").1)
    }
    /// Takes a tool call: a session tool here, any other in the session.
    pub fn call(&mut self, tool: &str, args: Value, reply: Reply) {
        let id = || {
            args.get("id")
                .and_then(Value::as_str)
                .map(String::from)
                .ok_or_else(|| {
                    "`id` is missing: give a session's id from list_sessions.".to_string()
                })
        };
        match tool {
            "list_sessions" => {
                let current = self.runner.as_ref().map(|(id, _)| id.clone());
                reply(Ok(json!({
                    "sessions": list(&self.root),
                    "current": current,
                    "folder": self.root.display().to_string(),
                })));
            }
            "delete_session" => {
                let discard = args.get("discard_unsaved").and_then(Value::as_bool) == Some(true);
                reply(id().and_then(|id| delete(&self.root, &id, discard)));
            }
            "resume_session" => {
                let id = match id() {
                    Ok(id) => id,
                    Err(e) => return reply(Err(e)),
                };
                if self.runner.as_ref().is_none_or(|(open, _)| *open != id) {
                    let session = match Session::open(&self.root, Some(&id), &self.workspace) {
                        Ok(session) => session,
                        Err(why) => return reply(Ok(refused(why))),
                    };
                    // The session open now is saved and left on disk.
                    if let Some((_, old)) = self.runner.take() {
                        old.stop();
                    }
                    self.runner = Some((id.clone(), Runner::start(session)));
                }
                let runner = &self.runner.as_ref().expect("a session").1;
                // Answered once the session's part is loaded again.
                runner.send(
                    "inspect".into(),
                    json!({}),
                    Box::new(move |answer| {
                        reply(answer.map(|mut v| {
                            if let Some(o) = v.as_object_mut() {
                                o.insert("resumed".into(), json!(id));
                            }
                            v
                        }))
                    }),
                );
            }
            _ => match self.runner() {
                Ok(runner) => runner.send(tool.into(), args, reply),
                Err(e) => reply(Err(e)),
            },
        }
    }
    /// Answers the calls already sent, saves the session and ends it.
    pub fn stop(mut self) {
        if let Some((_, runner)) = self.runner.take() {
            runner.stop();
        }
    }
}
