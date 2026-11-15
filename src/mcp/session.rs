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
//!   (`storage::write_atomic`) after the `Document`, and only once the
//!   `Document` is on disk, before a result about anything that changed is
//!   sent (`engine::Backend::keep`); it holds the `fingerprint` of the
//!   workspace it goes with. A write that fails is reported in the result
//!   (`persisted: false`, a change `failed`);
//! - `checkpoints/`: the checkpoints (`checkpoint`);
//! - `lock`: held with an OS lock by the process that has it open
//!   (`Lock`).
//!
//! Sessions are never deleted on their own: one may hold the only copy of
//! a job. `delete_session` deletes one, never an open one, and one with work
//! not saved to a project file only when asked to discard it. Saved means
//! the project file is there now and holds the session's job by value
//! (`saved_now`), not what the session last knew of it.
use super::checkpoint::{Drafts, Meta, Place};
use super::engine::Reply;
use super::runner::{Runner, Session};
use super::state::Log;
use crate::App;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
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
    /// The `fingerprint` of the `workspace.json` it was written with: the
    /// two are one pair only while they match (`runner`).
    #[serde(default)]
    pub workspace: Option<u64>,
}
/// A stable fingerprint of a file's bytes (FNV-1a, 64 bits): the same in
/// every build, so a resume by a newer app still pairs the files.
pub fn fingerprint(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
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
            workspace: None,
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
        // Saved only if the file holds this job now: it may have been
        // removed or changed since.
        app.project_dirty =
            !super::checkpoint::saved_by_value(&app.doc, app.project_path.as_deref());
    }
    pub fn put_place(&self, app: &mut App) {
        self.place.put(app);
    }
    /// Puts into `app` what it kept that does not depend on the job, when
    /// the workspace on disk is not the one it was written with (`runner`):
    /// that workspace may hold another project than the one it names. The
    /// revision and change log, and the last saves and exports, go on. The
    /// project file is not restored (the project has none, and is not
    /// saved), nor the drafts and the selection; its checkpoints stay
    /// listed, but as another project's (`checkpoint::other_project`), so
    /// none is restored over this job. A note says so in the next result.
    pub fn put_unpaired(&self, app: &mut App) {
        app.mcp.log = self.log.clone();
        app.mcp.log.projects += 1;
        app.mcp.outputs = self.outputs.clone();
        app.mcp.checkpoints = super::checkpoint::Kept::from_list(self.checkpoints.clone());
        app.project_path = None;
        app.project_dirty = true;
        app.mcp.notes.push(OLDER_RECORD.into());
    }
}

/// The note of a session resumed whose `session.json` is older than its
/// workspace (`SessionFile::put_unpaired`).
pub const OLDER_RECORD: &str = "The session record was older than its job; the project file was not restored — save with a path.";

/// Whether the job of the session in `folder`, which kept `kept`, is saved
/// to a project file now: its project file is there, loads, and holds the
/// session's `Document` (its `workspace.json`) by value
/// (`checkpoint::saved_by_value`). A file removed, unreadable or changed
/// since, or a session never saved, is not.
pub fn saved_now(folder: &Path, kept: Option<&SessionFile>) -> bool {
    let Some(project) = kept.and_then(|k| k.project_path.as_deref()) else {
        return false;
    };
    let Ok(doc) = crate::storage::load(&folder.join("workspace.json")) else {
        return false;
    };
    super::checkpoint::saved_by_value(&doc, Some(project))
}

/// The lock file's name in a session's folder.
pub const LOCK: &str = "lock";

/// Who holds a session's lock, as its holder writes it into the lock file
/// once it holds it: for the refusal's message only, never to tell whether
/// the lock is held (the OS tells that).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Owner {
    pid: u32,
    /// The process's start time as its OS gives it (`instance::started`).
    #[serde(default)]
    started: Option<String>,
    /// When the lock was taken, s since 1970.
    at: u64,
}

/// How often a lock another process holds is asked for again before the
/// refusal, and the pause between: `list_sessions` holds a lock shared
/// for a moment to tell whether its session is open (`holder`).
const TRIES: u32 = 10;
const PAUSE: Duration = Duration::from_millis(10);

/// A session's lock: its folder's `lock` file, opened (made when it is
/// not there) and held with an exclusive OS lock (`try_lock`: `flock`
/// on Unix, `LockFileEx` on Windows) for as long as this process has the
/// session open. The OS lets go when the file is closed, by drop or by the
/// process's end however it ends: there is no stale lock to tell, take over
/// or delete, and the file stays with its session (`delete` removes it
/// last). Who holds it is written into it once it is held, for the
/// refusal's message.
#[derive(Debug)]
pub struct Lock {
    file: std::fs::File,
}
impl Lock {
    /// Takes session `id`'s lock in `folder`, which must be there (the
    /// folder is never made here): refused while another process, or
    /// another open of this one, holds it.
    pub fn take(folder: &Path, id: &str) -> Result<Lock, String> {
        let path = folder.join(LOCK);
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(|e| format!("Could not lock session {id}: {e}"))?;
        let mut tries = 1;
        loop {
            match try_lock(&file) {
                Ok(()) => break,
                Err(std::fs::TryLockError::WouldBlock) if tries < TRIES => {
                    tries += 1;
                    std::thread::sleep(PAUSE);
                }
                Err(std::fs::TryLockError::WouldBlock) => {
                    let pid = owner(&mut &file).map(|o| o.pid);
                    return Err(open_elsewhere(id, pid));
                }
                Err(std::fs::TryLockError::Error(e)) => {
                    return Err(format!("Could not lock session {id}: {e}"));
                }
            }
        }
        let lock = Lock { file };
        lock.sign();
        Ok(lock)
    }
    /// Writes this process into the lock file it holds. A write that fails
    /// leaves the lock held, only the message without its pid.
    fn sign(&self) {
        use std::io::{Seek, Write};
        let pid = std::process::id();
        let me = Owner {
            pid,
            started: super::instance::started(pid),
            at: now_s(),
        };
        let Ok(bytes) = serde_json::to_vec(&me) else {
            return;
        };
        let mut file = &self.file;
        let _ = file
            .seek(std::io::SeekFrom::Start(0))
            .and_then(|_| file.write_all(&bytes))
            .and_then(|()| file.set_len(bytes.len() as u64));
    }
}

/// Holds `file`'s exclusive lock. On Unix `flock`, which leaves the file
/// readable. Windows' locks bar every other handle from reading the bytes
/// they cover, the whole file with `File::try_lock`, so there only the one
/// byte at the end of the range is locked: the owner written at the start
/// stays readable, and a shared lock of the whole file (`holder`) still
/// meets it.
#[cfg(not(windows))]
fn try_lock(file: &std::fs::File) -> Result<(), std::fs::TryLockError> {
    file.try_lock()
}
#[cfg(windows)]
fn try_lock(file: &std::fs::File) -> Result<(), std::fs::TryLockError> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::{Foundation, Storage::FileSystem, System::IO};
    let locked = unsafe {
        let mut at: IO::OVERLAPPED = std::mem::zeroed();
        at.Anonymous.Anonymous.Offset = u32::MAX - 1;
        at.Anonymous.Anonymous.OffsetHigh = u32::MAX;
        FileSystem::LockFileEx(
            file.as_raw_handle(),
            FileSystem::LOCKFILE_EXCLUSIVE_LOCK | FileSystem::LOCKFILE_FAIL_IMMEDIATELY,
            0,
            1,
            0,
            &mut at,
        )
    };
    if locked != 0 {
        return Ok(());
    }
    let e = std::io::Error::last_os_error();
    match e.raw_os_error().map(|c| c as u32) {
        Some(Foundation::ERROR_LOCK_VIOLATION | Foundation::ERROR_IO_PENDING) => {
            Err(std::fs::TryLockError::WouldBlock)
        }
        _ => Err(std::fs::TryLockError::Error(e)),
    }
}

/// The refusal for session `id`, whose lock process `pid` holds.
fn open_elsewhere(id: &str, pid: Option<u32>) -> String {
    match pid {
        Some(pid) => format!("Session {id} is open in another MCP server (pid {pid})."),
        // Being written.
        None => format!("Session {id} is open in another MCP server."),
    }
}

/// Who a lock file says holds it, when it says.
fn owner(file: &mut impl std::io::Read) -> Option<Owner> {
    let mut bytes = vec![];
    file.read_to_end(&mut bytes).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Whether a server has the session in `folder` open (its lock is held),
/// with its pid when the lock file says it: told by holding the lock shared
/// for a moment, which a holder's exclusive lock refuses. None when it is
/// not open (no lock file: never opened).
fn holder(folder: &Path) -> Option<Option<u32>> {
    let mut file = std::fs::File::open(folder.join(LOCK)).ok()?;
    match file.try_lock_shared() {
        // Not held; let go as the file closes.
        Ok(()) => None,
        Err(std::fs::TryLockError::WouldBlock) => Some(owner(&mut file).map(|o| o.pid)),
        Err(std::fs::TryLockError::Error(_)) => None,
    }
}

/// Whether the session in `folder` holds a job to resume: its workspace or
/// its record. `delete` removes both first, so a folder with neither is
/// being deleted or gone.
pub fn has_job(folder: &Path) -> bool {
    folder.join("workspace.json").is_file() || folder.join(FILE).is_file()
}

/// What `DELETE_STEP` holds: called with each step's name.
#[cfg(test)]
pub type DeleteHook = Box<dyn FnMut(&str)>;
#[cfg(test)]
thread_local! {
    /// Called by `delete` on this thread after each of its steps, with the
    /// step's name (tests: a race made at a step).
    pub static DELETE_STEP: std::cell::RefCell<Option<DeleteHook>> =
        const { std::cell::RefCell::new(None) };
}
/// `delete` has done `step`.
fn deleted(step: &str) {
    #[cfg(test)]
    DELETE_STEP.with(|hook| {
        if let Some(hook) = hook.borrow_mut().as_mut() {
            hook(step);
        }
    });
    #[cfg(not(test))]
    let _ = step;
}

/// Deletes the session folder `folder` while its lock, `lock`, is held,
/// so no server opens it meanwhile: its workspace and record first (a
/// folder without them is never resumed, `has_job`), then everything else
/// but the lock file, then the lock file and the folder, the lock held to
/// the end. On Windows a file held open cannot be deleted: the lock is let
/// go just before its file. A resume that opens the lock file after it is
/// removed makes a new one, finds no job and refuses; that file goes with
/// the folder.
fn remove(folder: &Path, lock: Lock) -> Result<(), String> {
    let err = |e: std::io::Error| format!("Could not delete {}: {e}", folder.display());
    let gone = |r: std::io::Result<()>| match r {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        r => r,
    };
    let clear = |keep_lock: bool| -> std::io::Result<()> {
        for entry in std::fs::read_dir(folder)? {
            let entry = entry?;
            if keep_lock && entry.file_name() == LOCK {
                continue;
            }
            let path = entry.path();
            gone(if entry.file_type()?.is_dir() {
                std::fs::remove_dir_all(&path)
            } else {
                std::fs::remove_file(&path)
            })?;
        }
        Ok(())
    };
    for name in [FILE, "workspace.json"] {
        gone(std::fs::remove_file(folder.join(name))).map_err(err)?;
        deleted(name);
    }
    clear(true).map_err(err)?;
    deleted("entries");
    let mut held = Some(lock);
    if cfg!(windows) {
        held = None;
    }
    gone(std::fs::remove_file(folder.join(LOCK))).map_err(err)?;
    deleted(LOCK);
    let mut tries = 0;
    loop {
        match gone(std::fs::remove_dir(folder)) {
            Ok(()) => break,
            Err(_) if tries < 50 => {
                tries += 1;
                let _ = clear(false);
                std::thread::sleep(PAUSE);
            }
            Err(e) => return Err(err(e)),
        }
    }
    deleted("folder");
    drop(held);
    Ok(())
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
    let owner = holder(&folder);
    Some(json!({
        "id": id,
        "title": title,
        "last_change": last.map(super::checkpoint::utc),
        "saved_to_project_file": saved_now(&folder, kept.as_ref()),
        "project_file": kept.as_ref().and_then(|k| k.project_path.as_ref()).map(|p| p.display().to_string()),
        "revision": kept.as_ref().map_or(0, |k| k.log.revision),
        "checkpoints": kept.as_ref().map_or(0, |k| k.checkpoints.len()),
        "size_bytes": size_of(&folder),
        "open": owner.is_some(),
        "pid": owner.flatten(),
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
/// held, by this process too), and while it has work not saved to a project
/// file unless `discard_unsaved`. Its lock is taken before anything is read
/// and held through the deletion (`remove`).
pub fn delete(root: &Path, id: &str, discard_unsaved: bool) -> Result<Value, String> {
    if !valid_id(id) {
        return Err(format!("\"{id}\" is not a session id."));
    }
    let folder = root.join(id);
    let missing = || format!("There is no MCP session {id} in {}.", root.display());
    if !folder.is_dir() {
        return Err(missing());
    }
    let lock = match Lock::take(&folder, id) {
        Ok(lock) => lock,
        Err(_) if !folder.is_dir() => return Err(missing()),
        Err(why) => {
            return Ok(refused(format!(
                "{why} It cannot be deleted while it is open."
            )));
        }
    };
    let kept = SessionFile::read(&folder).and_then(Result::ok);
    if !discard_unsaved && !saved_now(&folder, kept.as_ref()) {
        let title = describe(root, id)
            .and_then(|d| d["title"].as_str().map(String::from))
            .unwrap_or_default();
        return Ok(refused(format!(
            "Session {id} (\"{title}\") holds work not saved to a project file; deleting it would lose it. Resume it and save the project, or call again with discard_unsaved: true."
        )));
    }
    remove(&folder, lock)?;
    Ok(json!({"outcome": "done", "deleted": id}))
}

/// The session tools, and the headless session the other tools go to:
/// opened at the start (`--session <id>`), resumed (`resume_session`), or
/// made new at the first tool that needs one.
///
/// Instance discovery (`instance`): until the headless session is committed
/// (`Runner::committed`: it has dispatched work that may change its job, or
/// has changes from before a resume), each call first looks for an open
/// AXIS CAM window, and while one is open, or whether one is cannot be
/// told, the call is refused (its access off, several open, or driving a
/// window, which comes in the next version); a committed session stays
/// headless. `wait` and `cancel` for a `command_id` the open session gave
/// out, and the machine's stops sent to it, are never refused for a window.
pub struct Sessions {
    root: PathBuf,
    workspace: PathBuf,
    runner: Option<(String, Runner)>,
    /// The window the agent chose with `select_window`.
    chosen: Option<u32>,
}
impl Sessions {
    pub fn new(root: PathBuf, workspace: PathBuf, opened: Option<Session>) -> Sessions {
        Sessions {
            root,
            workspace,
            runner: opened.map(|s| (s.id.clone(), Runner::start(s))),
            chosen: None,
        }
    }
    /// Whether `tool` is one of the session tools.
    pub fn is_tool(tool: &str) -> bool {
        matches!(
            tool,
            "list_sessions" | "resume_session" | "delete_session" | "select_window"
        )
    }
    /// Why the call cannot go to a headless session: a window is open, or
    /// whether one is cannot be told (`instance::refusal`). None when there
    /// is no window, or when the session open now is committed to staying
    /// headless.
    fn window(&self) -> Option<String> {
        if self.runner.as_ref().is_some_and(|(_, r)| r.committed()) {
            return None;
        }
        self.windows()
    }
    /// Why a window keeps the calls from a headless session, whatever the
    /// session (`instance::refusal`).
    fn windows(&self) -> Option<String> {
        let found = super::instance::discover(&super::instance::folder(&self.workspace));
        super::instance::refusal(&found, self.chosen)
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
                "name": "select_window",
                "title": "Choose an AXIS CAM window",
                "description": "When several AXIS CAM windows are open, chooses the one (by its pid, as the error listing them gives) the calls go to. In this version an open window cannot be driven yet: close the app to work in a headless session.",
                "inputSchema": {
                    "type": "object",
                    "properties": {"pid": {"type": "integer", "minimum": 0}},
                    "required": ["pid"],
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
    /// Whether `tool` with `args` goes to the session open now whatever
    /// window is open: `wait` or `cancel` for a `command_id` it gave out
    /// (its work was dispatched there, and only it can answer), and the
    /// machine's stops. With no session open, none does.
    fn bound(&self, tool: &str, args: &Value) -> bool {
        let Some((_, runner)) = &self.runner else {
            return false;
        };
        match tool {
            "wait" | "cancel" => {
                (args.get("command_id").and_then(Value::as_str)).is_some_and(|id| runner.knows(id))
            }
            "execute" => (args.get("command").and_then(Value::as_str))
                .and_then(super::commands::find)
                .is_some_and(|c| c.group == "machine"),
            _ => false,
        }
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
            "select_window" => {
                let Some(pid) = args.get("pid").and_then(Value::as_u64) else {
                    return reply(Err(
                        "`pid` is missing: give a window's pid from the list of windows.".into(),
                    ));
                };
                let pid = u32::try_from(pid).unwrap_or(u32::MAX);
                let found = super::instance::discover(&super::instance::folder(&self.workspace));
                if let super::instance::Found::Windows(all) = &found
                    && all.iter().any(|i| i.pid == pid)
                {
                    self.chosen = Some(pid);
                }
                reply(Err(super::instance::select(&found, pid)));
            }
            "resume_session" => {
                let id = match id() {
                    Ok(id) => id,
                    Err(e) => return reply(Err(e)),
                };
                let open_now = self.runner.as_ref().is_some_and(|(open, _)| *open == id);
                let window = if open_now {
                    self.window()
                } else {
                    self.windows()
                };
                // Locked before anything of it is read (whether it is
                // committed to headless, which a window cannot then
                // refuse); let go again when the call is refused.
                let session = if open_now {
                    None
                } else {
                    match Session::open(&self.root, Some(&id), &self.workspace) {
                        Ok(session) => Some(session),
                        Err(why) => {
                            return reply(match window {
                                Some(window) => Err(window),
                                None => Ok(refused(why)),
                            });
                        }
                    }
                };
                let committed = session.as_ref().is_some_and(|s| {
                    SessionFile::read(&s.folder)
                        .and_then(Result::ok)
                        .is_some_and(|k| k.log.revision > 0)
                });
                if let Some(why) = window.filter(|_| !committed) {
                    return reply(Err(why));
                }
                if let Some(session) = session {
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
            _ => {
                if !self.bound(tool, &args)
                    && let Some(why) = self.window()
                {
                    return reply(Err(why));
                }
                match self.runner() {
                    Ok(runner) => runner.send(tool.into(), args, reply),
                    Err(e) => reply(Err(e)),
                }
            }
        }
    }
    /// Answers the calls already sent, saves the session and ends it.
    pub fn stop(mut self) {
        if let Some((_, runner)) = self.runner.take() {
            runner.stop();
        }
    }
}
