//! The engine (LIO-470): carries out each MCP tool call against an `App`,
//! on the thread that owns it. It is the same code for a headless session
//! (`runner`) and, later, the live window; what differs is the `Backend`.
//!
//! A command is checked before anything is sent (its precondition), sent as
//! the `Message`s a click would send, and then owns an **ownership window**
//! until the app's work has settled (`App::pending`): a second mutation
//! meanwhile is `busy`, and nothing of it is sent. Its call is answered
//! once the window closes, with the effect checked (`done`, `no_change`,
//! `failed`, or `cancelled`), or when its `wait_s` is up, with
//! `still_running` and the `command_id` that `wait` and `cancel` take.
//! Reads, `wait`, `cancel` and the machine's stops are answered meanwhile.
//!
//! A window's edges are where the work state is snapshotted (`state`): a
//! difference found when it opens is the user's, one found when it closes
//! the agent's, or mixed when external events came during it. Every result
//! carries the `revision`, what the command `changed`, and what the user
//! changed since the agent was last told (`user_changed`). A command may
//! not change the machinist's records unless the machinist did meanwhile.
use super::checkpoint;
use super::commands::{self, Ctx, Expect, Problem};
use super::fields;
use super::pick;
use super::reads::{self, Reads, Started};
use super::state::{self, Change, Entry, Records};
use crate::{App, Message};
use iced::Task;
use serde::Serialize;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};

/// What the engine needs of the side that owns the `App`.
pub trait Backend {
    fn app(&self) -> &App;
    fn app_mut(&mut self) -> &mut App;
    /// Sends `message` to the `App` as a click would, marked as the
    /// agent's (`App::update_by_agent`), and runs the work it starts, whose
    /// messages are the agent's too.
    fn send(&mut self, message: Message);
    /// Runs `task` as the `App`'s own tasks run, its messages sent to it
    /// as the agent's.
    fn run(&mut self, task: Task<Message>);
    /// The backend has nothing of its own still to come: no task running,
    /// no message waiting, and a `Tick` since the last message.
    fn quiet(&mut self) -> bool;
    /// The work still due: `App::pending`, and the backend's own.
    fn pending(&self) -> Vec<&'static str>;
    /// "headless", or "live" for the window.
    fn kind(&self) -> &'static str;
    /// The session's id and folder.
    fn session(&self) -> Value;
    /// The headless session's folder, the one place in the app's data
    /// folder a command may write; None in the window.
    fn session_folder(&self) -> Option<std::path::PathBuf>;
}

/// What a tool call gives back: its result object, or why the call could
/// not be carried out at all (an MCP tool error).
pub type Answer = Result<Value, String>;
/// Where a tool call's answer goes, called once on the `App`'s thread.
pub type Reply = Box<dyn FnOnce(Answer) + Send>;

/// How long a call waits for its work by default, and at most, s. Clients
/// time out on their own (Codex at 60 s), so the server answers first.
pub const DEFAULT_WAIT: f64 = 25.;
pub const MAX_WAIT: f64 = 50.;
/// A call's `wait_s`, within 0 to `MAX_WAIT`.
pub fn wait_of(args: &Value) -> Duration {
    let s = (args.get("wait_s").and_then(Value::as_f64)).unwrap_or(DEFAULT_WAIT);
    Duration::from_secs_f64(if s.is_finite() {
        s.clamp(0., MAX_WAIT)
    } else {
        DEFAULT_WAIT
    })
}

/// The refusal while a run, jog, probe or homing is under way
/// (`App::locked`).
const LOCKED: &str = "The machine is running; the job cannot change until it stops.";

/// The tools, as `tools/list` lists them.
pub fn tools() -> Value {
    let wait = json!({
        "type": "number",
        "minimum": 0,
        "maximum": MAX_WAIT,
        "description": "How long to wait for the work the call starts, s (default 25, at most 50).",
    });
    let id =
        json!({"type": "string", "description": "The command_id a still_running result gave."});
    let if_revision = json!({
        "type": "integer",
        "minimum": 0,
        "description": "Run only if the job is still at this revision (from the last result); otherwise the outcome is conflict, with what changed.",
    });
    let mut tools = json!([
        {
            "name": "inspect",
            "title": "Inspect the job",
            "description": "The state as the user would read it: the project (title, project file, part, material, whether it is saved), its setups and their operations by id, the selection with its page and tab, the status line, the work still pending, the backend and the session. With `what: \"steps\"`, a manual mill or lathe setup's steps instead: one step (`step`, from 0) with its text, DRO readouts and holes, the sections and where the machinist's own place is; it reads with a cursor of its own and never moves that place. A guide the app has not built for the job as it is is built first, which simulates the setup: refused while the machine runs.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "what": {"type": "string", "enum": ["steps"], "description": "Leave out for the whole state; steps for a hand setup's steps."},
                    "setup": {"type": ["string", "integer"], "description": "With steps: the setup's id or index from 0; default the active setup."},
                    "step": {"type": "integer", "minimum": 0, "description": "With steps: the step to read, from 0 (default 0)."},
                    "wait_s": wait,
                },
            },
            "annotations": {"readOnlyHint": true},
        },
        {
            "name": "list_fields",
            "title": "List the editable fields",
            "description": "Every field the user could type into or pick for a target, as the editor draws it now: key, label, tab (or step), value, choices, unit and a line of help. `edit_field` takes only these keys, and a choice only from its choices. An operation's fields are on every tab it has, and the user's open tab is not changed. A key with `command` is set by that command instead.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "target": {"type": "string", "enum": fields::TARGETS.map(|(n, _)| n)},
                    "setup": {"type": ["string", "integer"], "description": "For a setup (or an operation in it): its id or index; default the active setup."},
                    "operation": {"type": ["string", "integer"], "description": "For an operation: its id or index in the active setup; default the selected one."},
                },
                "required": ["target"],
            },
            "annotations": {"readOnlyHint": true},
        },
        {
            "name": "list_commands",
            "title": "List the commands",
            "description": "The commands `execute` runs: name, group, one-line help and whether the agent may run it. `filter` keeps those whose name or help holds it; `group` keeps one group.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "filter": {"type": "string"},
                    "group": {"type": "string", "enum": GROUPS},
                },
            },
            "annotations": {"readOnlyHint": true},
        },
        {
            "name": "describe_command",
            "title": "Describe a command",
            "description": "One command's full help, the JSON schema of its `args`, what must exist or be selected first, and the effect `execute` checks for.",
            "inputSchema": {
                "type": "object",
                "properties": {"name": {"type": "string"}},
                "required": ["name"],
            },
            "annotations": {"readOnlyHint": true},
        },
        {
            "name": "execute",
            "title": "Run a command",
            "description": "Runs one command as the user's click would, waits for its work, and says what happened: `outcome` done, no_change, refused, busy, failed, conflict or still_running (with a command_id for wait and cancel), with `reason`, the status line, the selection, the work still pending, the job's `revision`, what the command `changed`, and what the user changed meanwhile (`user_changed`).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "command": {"type": "string", "description": "A name from list_commands."},
                    "args": {"type": "object", "description": "The command's arguments (describe_command gives their schema)."},
                    "wait_s": wait,
                    "if_revision": if_revision,
                },
                "required": ["command"],
            },
        },
        {
            "name": "batch",
            "title": "Run commands in order",
            "description": "Runs commands one after another, each once the previous one's work has settled, and lists each one's outcome. It stops at the first outcome other than done or no_change (stop_on_error, true by default), and dispatches no new step once wait_s is up: those are not_run, and the step still working has its command_id. It is not all-or-nothing.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "steps": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {"command": {"type": "string"}, "args": {"type": "object"}},
                            "required": ["command"],
                        },
                    },
                    "stop_on_error": {"type": "boolean"},
                    "wait_s": wait,
                    "if_revision": if_revision,
                },
                "required": ["steps"],
            },
        },
        {
            "name": "wait",
            "title": "Wait for a command",
            "description": "Waits again for a command that answered still_running: its final result (done, no_change, failed or cancelled) once its work has settled, or still_running again when wait_s is up. Results are kept for the session.",
            "inputSchema": {
                "type": "object",
                "properties": {"command_id": id, "wait_s": wait},
                "required": ["command_id"],
            },
            "annotations": {"readOnlyHint": true},
        },
        {
            "name": "checkpoint",
            "title": "Take a checkpoint",
            "description": "Keeps the job as it is now under `name`, to go back to with restore_checkpoint: the project with its libraries, every editor's draft, the project file, the selection with its page and tab, and the part it was taken with. A checkpoint of the same name is replaced; at most 20 are kept, the oldest dropped. It belongs to this project: after another project is opened or started it cannot be restored.",
            "inputSchema": {
                "type": "object",
                "properties": {"name": {"type": "string"}},
                "required": ["name"],
            },
        },
        {
            "name": "restore_checkpoint",
            "title": "Restore a checkpoint",
            "description": "Puts the job back as checkpoint `name` held it, as one new revision (an if_revision from before is then stale). The machinist's step places, measurements and dial readings stay as they are now. Whether the project is saved is worked out against its file. Refused, with nothing changed: for a checkpoint of another project; when the part file is not the part the checkpoint was taken with; while the machine runs; and when the user changed the job since the checkpoint, or changes it while the part loads, unless `discard_user_changes: true`. Waits for what the app works out again.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": {"type": "string"},
                    "discard_user_changes": {"type": "boolean", "description": "Restore even over the user's changes since the checkpoint."},
                    "wait_s": wait,
                    "if_revision": if_revision,
                },
                "required": ["name"],
            },
        },
        {
            "name": "list_checkpoints",
            "title": "List the checkpoints",
            "description": "The checkpoints taken, oldest first: name, the revision taken at, when, the project's title, and whether it can be restored now (and why not).",
            "inputSchema": {"type": "object", "properties": {}},
            "annotations": {"readOnlyHint": true},
        },
        {
            "name": "cancel",
            "title": "Cancel a command",
            "description": "Cancels the generation a still_running command started, or a read still working (its worker finishes on its own); the final result is cancelled. Other work cannot be cancelled: wait for it.",
            "inputSchema": {
                "type": "object",
                "properties": {"command_id": id, "wait_s": wait},
                "required": ["command_id"],
            },
        },
    ]);
    if let Value::Array(list) = &mut tools {
        list.extend(reads::tools());
    }
    tools
}

/// Whether `tool` is one of `tools()`.
pub fn is_tool(tool: &str) -> bool {
    reads::TOOLS.contains(&tool)
        || matches!(
            tool,
            "inspect"
                | "list_fields"
                | "list_commands"
                | "describe_command"
                | "execute"
                | "batch"
                | "wait"
                | "cancel"
                | "checkpoint"
                | "restore_checkpoint"
                | "list_checkpoints"
        )
}

/// What became of a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// Its expected effect holds.
    Done,
    /// It was already so; nothing changed.
    NoChange,
    /// Its precondition failed; nothing was sent.
    Refused,
    /// Another command's work is still settling; nothing was sent.
    Busy,
    /// It was sent, and its expected effect does not hold.
    Failed,
    /// It was sent, and its work had not settled within `wait_s`.
    StillRunning,
    /// Its generation was cancelled (`cancel`).
    Cancelled,
    /// The job is no longer at the call's `if_revision`; nothing was sent.
    Conflict,
}

/// The selection, its page and tab, as `inspect` and every result give it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Selection {
    pub setup: Option<String>,
    pub operation: Option<String>,
    pub page: String,
    pub tab: String,
}
impl Selection {
    pub(crate) fn of(app: &App) -> Selection {
        Selection {
            setup: app.doc.active_setup().map(|s| s.id.clone()),
            operation: app.operation().map(|o| o.id.clone()),
            page: format!("{:?}", app.page),
            tab: app.tab.clone(),
        }
    }
}

/// An `execute` result (the spec's *Outcomes*).
#[derive(Debug, Serialize)]
pub struct Report {
    pub outcome: Outcome,
    /// The command's id: every command that was sent has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command_id: Option<String>,
    /// Why, for anything but done.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub backend: &'static str,
    /// The job's revision (`state::Log`).
    pub revision: u64,
    /// What the command changed: the items of its window's entry.
    pub changed: Vec<Change>,
    /// For a conflict: the entries since the call's `if_revision`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub entries: Vec<Entry>,
    /// The status line after the command.
    pub status: String,
    /// The status line reports a problem the checks found.
    pub status_problem: bool,
    pub selection: Selection,
    pub pending: Vec<&'static str>,
    /// A pick command's picks afterwards, as the app lists them (`pick`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub picks: Option<Value>,
    /// What each of a pick command's ids did.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub clicks: Vec<Value>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}
impl Report {
    fn of(b: &impl Backend, outcome: Outcome, reason: Option<String>) -> Report {
        let app = b.app();
        Report {
            outcome,
            command_id: None,
            reason,
            backend: b.kind(),
            revision: app.mcp.log.revision,
            changed: vec![],
            entries: vec![],
            status: app.status.clone(),
            status_problem: app.status_problem,
            selection: Selection::of(app),
            pending: b.pending(),
            picks: None,
            clicks: vec![],
            notes: vec![],
        }
    }
    fn with_id(mut self, id: &str) -> Report {
        self.command_id = Some(id.into());
        self
    }
    fn json(&self) -> Value {
        serde_json::to_value(self).expect("a report serializes")
    }
}

fn inspect(b: &impl Backend) -> Value {
    let app = b.app();
    let doc = &app.doc;
    let tool = |id: &str| {
        (doc.tools.iter())
            .find(|t| !id.is_empty() && t.id == id)
            .map(|t| t.name().to_string())
    };
    let setups: Vec<Value> = (doc.setups.iter().enumerate())
        .map(|(si, s)| {
            let operations: Vec<Value> = (s.operations.iter().enumerate())
                .map(|(oi, o)| {
                    json!({
                        "index": oi,
                        "id": o.id,
                        "name": crate::threading::shown_name(o, &doc.tools),
                        "strategy": format!("{:?}", o.strategy),
                        "tool": tool(&o.tool),
                        "tool_id": o.tool,
                        "enabled": o.enabled,
                    })
                })
                .collect();
            json!({
                "index": si,
                "id": s.id,
                "name": s.name(),
                "machine": s.machine.name(),
                "active": s.id == doc.active,
                "operations": operations,
            })
        })
        .collect();
    let model = doc
        .file
        .as_ref()
        .map(|f| json!({"name": f.name, "path": f.path.display().to_string(), "format": f.format}));
    json!({
        "backend": b.kind(),
        "project": {
            "title": doc.title,
            "file": app.project_path.as_ref().map(|p| p.display().to_string()),
            "revision": app.mcp.log.revision,
            "saved": app.project_path.is_some() && !app.project_dirty,
            "dirty": app.project_dirty,
            "model": model,
            "sample": app.is_sample(),
            "material": doc.material,
        },
        "setups": setups,
        "selection": Selection::of(app),
        "status": app.status,
        "status_problem": app.status_problem,
        "pending": b.pending(),
        "session": b.session(),
    })
}

/// The command groups, in the order `list_commands` gives them.
pub const GROUPS: [&str; 11] = [
    "project",
    "part",
    "setup",
    "operation",
    "picks",
    "generate",
    "libraries",
    "view",
    "review",
    "export",
    "machine",
];

fn list_commands(args: &Value) -> Value {
    let filter = (args.get("filter").and_then(Value::as_str))
        .unwrap_or("")
        .to_lowercase();
    let group = args.get("group").and_then(Value::as_str);
    let matches =
        |name: &str, help: &str| name.contains(&filter) || help.to_lowercase().contains(&filter);
    let mut listed: Vec<(usize, &str, Value)> = vec![];
    for c in commands::all() {
        let (line, _) = c.help();
        if group.is_none_or(|g| c.group == g) && matches(c.name, &line) {
            let order = GROUPS
                .iter()
                .position(|g| *g == c.group)
                .unwrap_or(GROUPS.len());
            listed.push((
                order,
                c.name,
                json!({"name": c.name, "group": c.group, "help": line, "allowed": true}),
            ));
        }
    }
    for (name, help, reason) in commands::REFUSED {
        if group.is_none_or(|g| g == "machine") && matches(name, help) {
            listed.push((
                GROUPS.len(),
                name,
                json!({"name": name, "group": "machine", "help": help, "allowed": false, "reason": reason}),
            ));
        }
    }
    listed.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));
    let commands: Vec<Value> = listed.into_iter().map(|(_, _, v)| v).collect();
    json!({
        "commands": commands,
        "read_tools": reads::TOOLS,
        "note": "The read tools (checks, the part, pictures, the rules) are called by their own names, not with execute; describe_command describes them too.",
    })
}

fn describe_command(args: &Value) -> Result<Value, String> {
    let name = (args.get("name").and_then(Value::as_str))
        .ok_or("`name` is missing: give a command's name from list_commands.")?;
    if let Some((name, help, reason)) = commands::REFUSED.iter().find(|r| r.0 == name) {
        return Ok(json!({
            "name": name,
            "group": "machine",
            "help": help,
            "allowed": false,
            "reason": reason,
        }));
    }
    if reads::TOOLS.contains(&name) {
        let tool = (reads::tools().into_iter()).find(|t| t["name"] == name);
        let tool = tool.unwrap_or_default();
        return Ok(json!({
            "name": name,
            "tool": true,
            "help": tool["description"],
            "args": tool["inputSchema"],
            "while_machine_runs": reads::heaviness(name),
            "note": "This is a read tool: call it by its name, not with execute.",
            "allowed": true,
        }));
    }
    let c = commands::find(name).ok_or_else(|| no_command(name))?;
    let (line, full) = c.help();
    Ok(json!({
        "name": c.name,
        "group": c.group,
        "help": line,
        "details": full,
        "args": c.schema(),
        "example": c.example_args().first(),
        "requires": c.requires,
        "effect": c.effect,
        "while_machine_runs": c.while_locked,
        "waits": if c.clicks_checked() {
            "for picking to come on and the app to settle, then makes each click, checks what it did, and waits for the work they start to settle, up to wait_s"
        } else {
            "for the work it starts to settle (pending empty), up to wait_s"
        },
        "allowed": true,
    }))
}

/// A `refused` result.
fn refuse(b: &impl Backend, why: String) -> Answer {
    Ok(Report::of(b, Outcome::Refused, Some(why)).json())
}
/// A `conflict` result: the job moved on from `revision`.
fn conflict(b: &impl Backend, revision: u64) -> Answer {
    let log = &b.app().mcp.log;
    let why = format!(
        "Nothing was done: the job changed since revision {revision} and is at revision {} now. Read what changed (entries), then call again with if_revision {}.",
        log.revision, log.revision
    );
    let mut report = Report::of(b, Outcome::Conflict, Some(why));
    report.entries = log.since(revision).cloned().collect();
    Ok(report.json())
}
/// A call's `if_revision`: None when it has none.
fn if_revision(args: &Value) -> Result<Option<u64>, String> {
    match args.get("if_revision") {
        None | Some(Value::Null) => Ok(None),
        Some(v) => (v.as_u64().map(Some))
            .ok_or_else(|| "`if_revision` is a revision number from a result.".into()),
    }
}

/// An ownership window: the agent command whose work is settling. It
/// opens when the command's messages are sent and closes only when the
/// work has settled, whatever its call was answered meanwhile.
struct Window {
    id: String,
    name: &'static str,
    /// The status line before the command, to tell the app's own words.
    status: String,
    expect: Option<Expect>,
    cancelled: bool,
    /// The machinist's records when it opened: None for a command that
    /// replaces the project, which brings its own.
    records: Option<Records>,
    /// A pick command's clicks: due once picking is on and the app has
    /// settled, then what they came to.
    clicks: Option<Clicking>,
    /// The step of the revision the command made as it was sent (a
    /// checkpoint restored), before what settling it changed.
    stepped: Option<Entry>,
    notes: Vec<String>,
}
/// A checkpoint being restored while its part loads (`checkpoint`).
struct Restoring {
    id: String,
    checkpoint: Box<checkpoint::Checkpoint>,
    discard: bool,
    /// The work state's hash when the restore began.
    began: Option<u64>,
    candidate: checkpoint::Candidate,
}
/// A pick command's clicks in its window.
enum Clicking {
    /// To make, and the view to set back after them.
    Due(pick::Job, pick::View),
    Made(pick::Done),
}
/// A call answered when a command's window closes, or when its time is up.
struct Waiter {
    id: String,
    deadline: Instant,
    reply: Reply,
}
/// A call that came before the session's start had settled.
struct Held {
    tool: String,
    args: Value,
    deadline: Instant,
    reply: Reply,
}
/// A batch under way: its steps, what each did, and the step running.
struct Batch {
    steps: Vec<Value>,
    next: usize,
    done: Vec<Value>,
    stop_on_error: bool,
    deadline: Instant,
    running: Option<(String, String)>,
    reply: Reply,
}

/// What a command's start came to: a window opened for it, or its answer.
enum Start {
    Opened(String),
    Now(Answer),
}

/// The agent's side of a session: the window open, the results kept, and
/// the calls still to answer.
#[derive(Default)]
pub struct Agent {
    count: u64,
    window: Option<Window>,
    results: HashMap<String, Value>,
    waiters: Vec<Waiter>,
    batch: Option<Batch>,
    held: Vec<Held>,
    started: bool,
    /// The reads working on a snapshot of the job (`reads`).
    reads: Reads,
    /// A checkpoint restore waiting for its part.
    restoring: Option<Restoring>,
    /// The answers given, each stamped with the revision and what the user
    /// changed (`flush`) as it goes; a read's or a list's is not.
    outbox: Vec<(Reply, Answer, bool)>,
}
impl Agent {
    pub fn new() -> Agent {
        Agent::default()
    }
    /// Whether a call waits for the app's work: the event loop then ticks
    /// fast, as the window's does for a pending MCP request.
    pub fn waiting(&self) -> bool {
        self.window.is_some()
            || !self.held.is_empty()
            || !self.started
            || self.reads.any()
            || self.restoring.is_some()
    }
    /// Whether nothing is open or unanswered.
    pub fn idle(&self) -> bool {
        self.window.is_none()
            && self.waiters.is_empty()
            && self.batch.is_none()
            && self.held.is_empty()
            && self.restoring.is_none()
    }
    /// The soonest a call must be answered.
    pub fn deadline(&self) -> Option<Instant> {
        (self.waiters.iter().map(|w| w.deadline))
            .chain(self.held.iter().map(|h| h.deadline))
            .chain(self.batch.iter().map(|b| b.deadline))
            .min()
    }

    /// Takes a tool call: answered now, or by `poll` later. The first calls
    /// wait for the session's start (its saved part restored) to settle.
    pub fn call(&mut self, b: &mut impl Backend, tool: &str, args: Value, reply: Reply) {
        let deadline = Instant::now() + wait_of(&args);
        if !self.started {
            let tool = tool.to_string();
            self.held.push(Held {
                tool,
                args,
                deadline,
                reply,
            });
            return;
        }
        self.answer(b, tool, args, deadline, reply);
        self.flush(b);
    }
    fn answer(
        &mut self,
        b: &mut impl Backend,
        tool: &str,
        args: Value,
        deadline: Instant,
        reply: Reply,
    ) {
        if reads::is_read(tool, &args) {
            return self.read(b, tool, &args, deadline, reply);
        }
        // A read takes a snapshot only while no window is open: a wait
        // cannot take the user's work for the agent's.
        if self.window.is_none()
            && self.restoring.is_none()
            && matches!(
                tool,
                "inspect" | "list_fields" | "wait" | "cancel" | "list_checkpoints"
            )
        {
            b.app_mut().mcp_outside();
        }
        match tool {
            "inspect" => self.give(reply, Ok(inspect(b))),
            "list_fields" => {
                let fields = fields::list(b.app_mut(), &args);
                self.give(reply, fields)
            }
            "list_commands" => self.give_plain(reply, Ok(list_commands(&args))),
            "describe_command" => self.give_plain(reply, describe_command(&args)),
            "execute" => self.execute(b, &args, deadline, reply),
            "batch" => self.batch(b, &args, deadline, reply),
            "wait" => self.wait(&args, deadline, reply),
            "cancel" => self.cancel(b, &args, deadline, reply),
            "checkpoint" => self.checkpoint(b, &args, reply),
            "restore_checkpoint" => self.restore(b, &args, deadline, reply),
            "list_checkpoints" => self.give(reply, Ok(list_checkpoints(b))),
            _ => self.give_plain(reply, Err(format!("There is no tool {tool}."))),
        }
    }
    /// Starts a read (`reads`): answered now, or by its worker within the
    /// call's time, else `still_running` with its id. Its snapshot of the
    /// work state is taken as a read's is: the user's changes are found
    /// first while no window is open.
    fn read(
        &mut self,
        b: &mut impl Backend,
        tool: &str,
        args: &Value,
        deadline: Instant,
        reply: Reply,
    ) {
        let outside = self.window.is_none();
        if outside {
            b.app_mut().mcp_outside();
        }
        self.count += 1;
        let id = format!("{tool}-{}", self.count);
        let app = b.app();
        let before = || match app.mcp.log.last.as_ref().filter(|_| outside) {
            Some(last) => last.clone(),
            None => state::snapshot(app),
        };
        match self.reads.start(app, &id, tool, args, before) {
            Started::Now(answer) => self.give(reply, answer),
            Started::Working => self.waiters.push(Waiter {
                id,
                deadline,
                reply,
            }),
        }
    }
    /// Gives each finished read's result to the calls waiting for it, and
    /// keeps it for `wait`.
    fn finish_reads(&mut self, b: &impl Backend) {
        for (id, result) in self.reads.finished(b.app()) {
            self.results.insert(id.clone(), result.clone());
            let (mine, others): (Vec<_>, Vec<_>) = std::mem::take(&mut self.waiters)
                .into_iter()
                .partition(|w| w.id == id);
            self.waiters = others;
            for w in mine {
                self.give(w.reply, Ok(result.clone()));
            }
        }
    }
    /// Gives `answer` to `reply`, stamped with the revision and what the
    /// user changed when it goes (`flush`).
    fn give(&mut self, reply: Reply, answer: Answer) {
        self.outbox.push((reply, answer, true));
    }
    /// Gives `answer` to `reply` as it is: it is not about the job.
    fn give_plain(&mut self, reply: Reply, answer: Answer) {
        self.outbox.push((reply, answer, false));
    }
    /// Sends the answers given. Each result about the job gets the
    /// revision and the `user` and `mixed` entries since the agent was
    /// last told, which it is now (`state::Log::report`).
    fn flush(&mut self, b: &mut impl Backend) {
        for (reply, mut answer, stamp) in std::mem::take(&mut self.outbox) {
            if stamp && let Ok(Value::Object(result)) = &mut answer {
                let log = &mut b.app_mut().mcp.log;
                result.insert("revision".into(), json!(log.revision));
                result.insert("user_changed".into(), json!(log.report()));
            }
            reply(answer);
        }
    }

    /// After the backend has taken its events: starts the held calls once
    /// the session's start has settled, closes a window whose work has
    /// settled, and answers the calls whose time is up.
    pub fn poll(&mut self, b: &mut impl Backend) {
        let settled = b.quiet() && b.app().pending().is_empty();
        let now = Instant::now();
        if !self.started {
            self.started = settled;
            if self.started {
                // Where the change log starts, or what changed since the
                // last agent worked on this `App`.
                b.app_mut().mcp_outside();
            }
            for h in std::mem::take(&mut self.held) {
                if self.started || h.deadline <= now {
                    self.answer(b, &h.tool, h.args, h.deadline, h.reply);
                } else {
                    self.held.push(h);
                }
            }
        }
        // A checkpoint's part loaded: the restore is checked again and
        // committed, in this turn.
        if let Some(loaded) = self.restoring.as_ref().and_then(|r| r.candidate.take())
            && let Some(r) = self.restoring.take()
        {
            self.commit_restore(b, r, Some(loaded));
        }
        // A pick command's clicks are made once picking is on and the app
        // has settled, as a user clicks; its window closes after them.
        let due =
            (self.window.as_ref()).is_some_and(|w| matches!(w.clicks, Some(Clicking::Due(..))));
        if due {
            if b.quiet() && b.app().pending().is_empty() {
                self.click(b);
            }
        } else if settled && self.window.is_some() {
            self.close(b);
        }
        self.finish_reads(b);
        let now = Instant::now();
        let (due, waiting): (Vec<_>, Vec<_>) = std::mem::take(&mut self.waiters)
            .into_iter()
            .partition(|w| w.deadline <= now);
        self.waiters = waiting;
        for w in due {
            let answer = if self.reads.working(&w.id) {
                Ok(self.reads.still_running(&w.id))
            } else if self.restoring.as_ref().is_some_and(|r| r.id == w.id) {
                let why = "The checkpoint's part is still loading; nothing is restored yet. Call wait with its command_id for the final result.";
                Ok(Report::of(b, Outcome::StillRunning, Some(why.into()))
                    .with_id(&w.id)
                    .json())
            } else {
                Ok(still_running(b, &w.id))
            };
            self.give(w.reply, answer);
        }
        if self
            .batch
            .as_ref()
            .is_some_and(|batch| batch.deadline <= now)
        {
            self.finish_batch(b);
        }
        self.flush(b);
    }

    fn busy(&self, b: &impl Backend) -> Option<Answer> {
        let (name, id) = match (&self.window, &self.batch, &self.restoring) {
            (Some(w), ..) => (w.name.to_string(), w.id.clone()),
            (None, Some(_), _) => ("batch".to_string(), "the batch".to_string()),
            (None, None, Some(r)) => ("restore_checkpoint".to_string(), r.id.clone()),
            (None, None, None) => return None,
        };
        let why = format!(
            "Nothing was done; the previous command ({name}, {id}) is still running. Call wait, then retry."
        );
        Some(Ok(Report::of(b, Outcome::Busy, Some(why)).json()))
    }

    fn execute(&mut self, b: &mut impl Backend, args: &Value, deadline: Instant, reply: Reply) {
        // The machine's stops go ahead whatever is under way.
        let stop = (args.get("command").and_then(Value::as_str))
            .and_then(commands::find)
            .is_some_and(|c| c.group == "machine");
        if !stop && let Some(busy) = self.busy(b) {
            return self.give(reply, busy);
        }
        match self.start(b, args) {
            Start::Now(answer) => self.give(reply, answer),
            Start::Opened(id) => self.waiters.push(Waiter {
                id,
                deadline,
                reply,
            }),
        }
    }

    /// Checks a command and sends it: an answer at once when nothing was
    /// sent (refused, no_change, conflict, an error) or for the stops, else
    /// a window. The window opens with a snapshot, in the turn that sends.
    fn start(&mut self, b: &mut impl Backend, args: &Value) -> Start {
        let Some(name) = args.get("command").and_then(Value::as_str) else {
            return Start::Now(Err(
                "`command` is missing: give a command's name from list_commands.".into(),
            ));
        };
        let Some(c) = commands::find(name) else {
            return Start::Now(match commands::REFUSED.iter().find(|r| r.0 == name) {
                Some((_, _, reason)) => refuse(b, reason.to_string()),
                None => Err(no_command(name)),
            });
        };
        let empty = json!({});
        let command_args = match args.get("args") {
            None | Some(Value::Null) => &empty,
            Some(a @ Value::Object(_)) => a,
            Some(_) => return Start::Now(Err("`args` must be an object.".into())),
        };
        let expected = match if_revision(args) {
            Ok(r) => r,
            Err(e) => return Start::Now(Err(e)),
        };
        // The machine's stops open no window: they must go at once, and
        // leave the job alone.
        let windowed = c.group != "machine";
        if windowed {
            // What changed since the last window closed is the user's,
            // and an `if_revision` counts it.
            b.app_mut().mcp_outside();
            if let Some(r) = expected
                && r != b.app().mcp.log.revision
            {
                return Start::Now(conflict(b, r));
            }
        }
        if b.app().locked() && !c.while_locked {
            return Start::Now(refuse(b, LOCKED.into()));
        }
        let folder = b.session_folder();
        // `edit_field` checks its key against the fields the editor draws,
        // which building it for another setup or operation needs the `App`
        // changed for (`fields::of`).
        let fields = (c.name == "edit_field").then(|| fields::of(b.app_mut(), command_args));
        let prepared = {
            let app = b.app();
            commands::resolve(c.on, app, command_args).and_then(|(setup, operation)| {
                let ctx = Ctx {
                    app,
                    args: command_args,
                    session: folder.as_deref(),
                    setup,
                    operation,
                    fields,
                };
                // Refused first: a command that may not run is never
                // reported as already done.
                c.check(&ctx)?;
                if let Some(why) = c.unchanged.and_then(|f| f(&ctx)) {
                    return Ok(Err(why));
                }
                let dispatch = c.dispatch(&ctx)?;
                Ok(Ok((dispatch, c.expect.map(|f| f(&ctx)))))
            })
        };
        let (dispatch, expect) = match prepared {
            Err(Problem::Args(e)) => return Start::Now(Err(format!("{name}: {e}"))),
            Err(Problem::Refused(why)) => return Start::Now(refuse(b, why)),
            Ok(Err(why)) => {
                return Start::Now(Ok(Report::of(b, Outcome::NoChange, Some(why)).json()));
            }
            Ok(Ok(prepared)) => prepared,
        };
        // Nothing of a command is sent unless all of it may be: its clicks
        // too.
        let clicked = dispatch.clicks.iter().flat_map(pick::Job::messages);
        let all: Vec<Message> = dispatch.messages.iter().cloned().chain(clicked).collect();
        if let Err(why) = commands::guard(&all) {
            return Start::Now(refuse(b, why.into()));
        }
        self.count += 1;
        let id = format!("{}-{}", c.name, self.count);
        let status = b.app().status.clone();
        let records = (windowed && !state::REPLACES_PROJECT.contains(&c.name))
            .then(|| Records::of(&b.app().doc));
        for message in dispatch.messages {
            b.send(message);
        }
        if let Some(task) = dispatch.task {
            b.run(task);
        }
        // Picking on now; the clicks once the app has settled (`poll`).
        let clicks = (dispatch.clicks).map(|job| {
            let view = job.begin(b);
            Clicking::Due(job, view)
        });
        if !windowed {
            let report = Report::of(b, Outcome::Done, None).with_id(&id).json();
            self.results.insert(id, report.clone());
            return Start::Now(Ok(report));
        }
        self.window = Some(Window {
            id: id.clone(),
            name: c.name,
            status,
            expect,
            cancelled: false,
            records,
            clicks,
            stepped: None,
            notes: vec![],
        });
        Start::Opened(id)
    }

    /// Makes the open window's pick clicks (`pick::Job::run`), each checked
    /// and undone in full when it did not do what was asked; the window
    /// closes once the work they started has settled.
    fn click(&mut self, b: &mut impl Backend) {
        let Some(w) = &mut self.window else {
            return;
        };
        if let Some(Clicking::Due(job, view)) = w.clicks.take() {
            w.clicks = Some(Clicking::Made(job.run(b, &view)));
        }
    }

    /// Closes the window: the machinist's records guarded, the closing
    /// snapshot taken, its effect checked, its result kept and given to the
    /// calls waiting for it, and a batch's next step started.
    fn close(&mut self, b: &mut impl Backend) {
        let Some(mut w) = self.window.take() else {
            return;
        };
        // A pick command's verdict is its clicks', each checked as made.
        let made = match w.clicks.take() {
            Some(Clicking::Made(done)) => Some(done),
            Some(Clicking::Due(..)) | None => None,
        };
        // The machinist's records: put back when the agent's path changed
        // them, theirs when they worked meanwhile.
        let (mut restored, mut theirs) = (vec![], vec![]);
        if let Some(before) = &w.records {
            let app = b.app_mut();
            let changed = before.changed(&Records::of(&app.doc));
            if !changed.is_empty() {
                let names: Vec<String> = (changed.iter())
                    .map(|r| state::record_name(&app.doc, r))
                    .collect();
                if app.mcp_events().is_empty() {
                    before.put_back(&mut app.doc, &changed);
                    app.dirty = true;
                    app.last_edit = Instant::now();
                    restored = names;
                } else {
                    theirs = names;
                }
            }
        }
        let entry = b.app_mut().mcp_close();
        let (outcome, reason) = if !restored.is_empty() {
            (
                Outcome::Failed,
                Some(format!(
                    "{}: {}. It was put back.",
                    state::PROGRESS,
                    restored.join(", ")
                )),
            )
        } else if w.cancelled {
            (
                Outcome::Cancelled,
                Some("Cancelled at the agent's request.".to_string()),
            )
        } else if let Some(done) = &made {
            match &done.verdict {
                Ok(None) => (Outcome::Done, None),
                Ok(Some(why)) => (Outcome::NoChange, Some(why.clone())),
                Err(why) => (Outcome::Failed, Some(why.clone())),
            }
        } else {
            match w.expect.as_ref().map_or(Ok(None), |e| e(b.app())) {
                Ok(None) => (Outcome::Done, None),
                Ok(Some(why)) => (Outcome::NoChange, Some(why)),
                Err(sentence) => {
                    // The app's own words, when the command changed the
                    // status line.
                    let said = &b.app().status;
                    let reason = if *said != w.status && !said.is_empty() {
                        said.clone()
                    } else {
                        sentence
                    };
                    (Outcome::Failed, Some(reason))
                }
            }
        };
        let mut report = Report::of(b, outcome, reason).with_id(&w.id);
        if let Some(done) = made {
            report.picks = Some(done.picks);
            report.clicks = done.clicks;
        }
        report.notes = std::mem::take(&mut w.notes);
        if let Some(stepped) = w.stepped.take() {
            report.changed = stepped.changed;
        }
        if let Some(entry) = entry {
            if !entry.events.is_empty() {
                report.notes.push(format!(
                    "Changed by others while this command worked: {}.",
                    entry.events.join("; ")
                ));
            }
            report.changed.extend(entry.changed);
        }
        if !theirs.is_empty() {
            report.notes.push(format!(
                "The machinist changed {} meanwhile; kept as theirs.",
                theirs.join(", ")
            ));
        }
        let report = report.json();
        self.results.insert(w.id.clone(), report.clone());
        let (mine, others): (Vec<_>, Vec<_>) = std::mem::take(&mut self.waiters)
            .into_iter()
            .partition(|x| x.id == w.id);
        self.waiters = others;
        for x in mine {
            self.give(x.reply, Ok(report.clone()));
        }
        if let Some(batch) = &mut self.batch
            && batch.running.as_ref().is_some_and(|(_, id)| *id == w.id)
        {
            let (name, _) = batch.running.take().unwrap_or_default();
            batch.done.push(step(&name, Ok(report)));
            self.advance(b);
        }
    }

    fn wait(&mut self, args: &Value, deadline: Instant, reply: Reply) {
        let Some(id) = args.get("command_id").and_then(Value::as_str) else {
            return self.give_plain(reply, Err("`command_id` is missing.".into()));
        };
        if let Some(result) = self.results.get(id) {
            let result = result.clone();
            return self.give(reply, Ok(result));
        }
        if self.window.as_ref().is_some_and(|w| w.id == id)
            || self.reads.working(id)
            || self.restoring.as_ref().is_some_and(|r| r.id == id)
        {
            let id = id.to_string();
            return self.waiters.push(Waiter {
                id,
                deadline,
                reply,
            });
        }
        self.give_plain(
            reply,
            Err(format!("There is no command {id} in this session.")),
        )
    }

    fn cancel(&mut self, b: &mut impl Backend, args: &Value, deadline: Instant, reply: Reply) {
        let Some(id) = args.get("command_id").and_then(Value::as_str) else {
            return self.give_plain(reply, Err("`command_id` is missing.".into()));
        };
        if let Some(result) = self.results.get(id) {
            let answer = json!({
                "cancelled": false,
                "command_id": id,
                "reason": "It had finished; this is its result.",
                "result": result,
            });
            return self.give(reply, Ok(answer));
        }
        if let Some(result) = self.reads.cancel(id) {
            self.results.insert(id.to_string(), result.clone());
            let (mine, others): (Vec<_>, Vec<_>) = std::mem::take(&mut self.waiters)
                .into_iter()
                .partition(|w| w.id == id);
            self.waiters = others;
            for w in mine {
                self.give(w.reply, Ok(result.clone()));
            }
            return self.give(reply, Ok(result));
        }
        if self.restoring.as_ref().is_some_and(|r| r.id == id) {
            let answer = json!({
                "cancelled": false,
                "command_id": id,
                "reason": "A checkpoint restore cannot be cancelled while its part loads: wait for it. Nothing is restored until it is loaded and checked again.",
            });
            return self.give(reply, Ok(answer));
        }
        if !self.window.as_ref().is_some_and(|w| w.id == id) {
            return self.give_plain(
                reply,
                Err(format!("There is no command {id} in this session.")),
            );
        }
        if b.app().generating.is_none() {
            let answer = json!({
                "cancelled": false,
                "command_id": id,
                "reason": "Only a generation can be cancelled, and this command's work is not one: wait for it.",
            });
            return self.give(reply, Ok(answer));
        }
        b.send(Message::CancelGenerate);
        if let Some(w) = &mut self.window {
            w.cancelled = true;
        }
        let id = id.to_string();
        self.waiters.push(Waiter {
            id,
            deadline,
            reply,
        });
    }

    /// `checkpoint {name}`: the job kept as it is (`checkpoint::take`),
    /// once no command's work is settling, with the user's changes found
    /// first.
    fn checkpoint(&mut self, b: &mut impl Backend, args: &Value, reply: Reply) {
        let Some(name) = args.get("name").and_then(Value::as_str) else {
            return self.give_plain(reply, Err("`name` is missing: name the checkpoint.".into()));
        };
        if let Some(busy) = self.busy(b) {
            return self.give(reply, busy);
        }
        b.app_mut().mcp_outside();
        let folder = b.session_folder();
        let answer = match checkpoint::take(b.app_mut(), name, folder.as_deref()) {
            Ok(meta) => Ok(json!({
                "outcome": "done",
                "backend": b.kind(),
                "checkpoint": {"name": meta.name, "revision": meta.revision, "taken": checkpoint::utc(meta.taken)},
                "checkpoints": list_checkpoints(b)["checkpoints"],
            })),
            Err(why) => refuse(b, why),
        };
        self.give(reply, answer)
    }

    /// `restore_checkpoint {name, discard_user_changes?}`: checked first,
    /// changing nothing (another project, the user's changes since, a
    /// running machine); the checkpoint's part loaded as a candidate when
    /// it is not the one loaded; then committed (`commit_restore`).
    fn restore(&mut self, b: &mut impl Backend, args: &Value, deadline: Instant, reply: Reply) {
        let Some(name) = args.get("name").and_then(Value::as_str) else {
            return self.give_plain(
                reply,
                Err("`name` is missing: give a checkpoint's name from list_checkpoints.".into()),
            );
        };
        let discard = args.get("discard_user_changes").and_then(Value::as_bool) == Some(true);
        let expected = match if_revision(args) {
            Ok(r) => r,
            Err(e) => return self.give_plain(reply, Err(e)),
        };
        if let Some(busy) = self.busy(b) {
            return self.give(reply, busy);
        }
        // What changed since the last window closed is the user's.
        b.app_mut().mcp_outside();
        if let Some(r) = expected
            && r != b.app().mcp.log.revision
        {
            let answer = conflict(b, r);
            return self.give(reply, answer);
        }
        let folder = b.session_folder();
        let found = checkpoint::load(b.app(), name, folder.as_deref());
        let found = found.and_then(|cp| {
            match checkpoint::other_project(b.app(), cp.projects, &cp.doc.project_id) {
                Some(why) => Err(why.to_string()),
                None => Ok(cp),
            }
        });
        let cp = match found {
            Ok(cp) => cp,
            Err(why) => {
                let answer = refuse(b, why);
                return self.give(reply, answer);
            }
        };
        let log = &b.app().mcp.log;
        let theirs: Vec<Entry> = (log.since(cp.revision))
            .filter(|e| e.source != state::Source::Agent)
            .cloned()
            .collect();
        if !theirs.is_empty() && !discard {
            let why = format!(
                "Nothing was done: the user changed the job since checkpoint \"{name}\" (revision {}), and restoring it would undo their work (entries). Call again with discard_user_changes: true to restore it anyway.",
                cp.revision
            );
            let mut report = Report::of(b, Outcome::Refused, Some(why));
            report.entries = theirs;
            return self.give(reply, Ok(report.json()));
        }
        if b.app().locked() {
            let answer = refuse(b, LOCKED.into());
            return self.give(reply, answer);
        }
        self.count += 1;
        let id = format!("restore_checkpoint-{}", self.count);
        self.waiters.push(Waiter {
            id: id.clone(),
            deadline,
            reply,
        });
        let began = b.app().mcp.log.last.as_ref().map(|s| s.hash);
        let needs_part = checkpoint::needs_part(b.app(), &cp);
        #[cfg(test)]
        let slow = b.app().mcp.slow_candidate;
        #[cfg(not(test))]
        let slow = None;
        let candidate = match needs_part {
            true => checkpoint::Candidate::load(&cp, slow),
            false => checkpoint::Candidate::default(),
        };
        let r = Restoring {
            id,
            checkpoint: Box::new(cp),
            discard,
            began,
            candidate,
        };
        match needs_part {
            true => self.restoring = Some(r),
            false => self.commit_restore(b, r, None),
        }
    }

    /// The turn that commits a restore: refused if its part is not the
    /// checkpoint's, if the user changed the job since the restore began
    /// (unless discarding), or while the machine runs; otherwise the
    /// checkpoint put back (`App::restore`) as one step of the revision,
    /// and a window open until what the app works out again has settled.
    fn commit_restore(
        &mut self,
        b: &mut impl Backend,
        r: Restoring,
        loaded: Option<Result<crate::mesh::Mesh, String>>,
    ) {
        let mesh = match loaded {
            None => None,
            Some(Ok(mesh)) => Some(mesh),
            Some(Err(why)) => {
                let answer = Report::of(b, Outcome::Refused, Some(why))
                    .with_id(&r.id)
                    .json();
                return self.finish(&r.id, answer);
            }
        };
        let app = b.app_mut();
        let now = state::snapshot(app);
        let theirs = app.mcp_events().to_vec();
        if Some(now.hash) != r.began && !theirs.is_empty() && !r.discard {
            // Theirs, and kept.
            app.mcp_outside();
            let why = format!(
                "Nothing was restored: the job changed while the checkpoint's part loaded ({}). Their change is kept. Call again with discard_user_changes: true to restore it anyway.",
                theirs.join("; ")
            );
            let answer = Report::of(b, Outcome::Refused, Some(why))
                .with_id(&r.id)
                .json();
            return self.finish(&r.id, answer);
        }
        if app.locked() {
            let answer = Report::of(b, Outcome::Refused, Some(LOCKED.into()))
                .with_id(&r.id)
                .json();
            return self.finish(&r.id, answer);
        }
        // What changed meanwhile, discarded, is still logged as theirs.
        app.mcp_outside();
        let records = Records::of(&app.doc);
        let status = app.status.clone();
        app.restore(&r.checkpoint, mesh);
        let stepped = app.mcp_restored(&r.checkpoint.name);
        let saved = !app.project_dirty;
        let note = format!(
            "Checkpoint \"{}\" (revision {}) restored as revision {}. {}",
            r.checkpoint.name,
            r.checkpoint.revision,
            stepped.revision,
            match (saved, &app.project_path) {
                (true, Some(p)) => format!("The project is as its file {} holds it.", p.display()),
                (false, Some(p)) => format!(
                    "The project differs from its file {}: it is not saved.",
                    p.display()
                ),
                (_, None) => "The project has no file: it is not saved.".to_string(),
            }
        );
        self.window = Some(Window {
            id: r.id,
            name: "restore_checkpoint",
            status,
            expect: None,
            cancelled: false,
            records: Some(records),
            clicks: None,
            stepped: Some(stepped),
            notes: vec![note],
        });
    }

    /// A command's final `answer` kept for `wait` and given to the calls
    /// waiting for it.
    fn finish(&mut self, id: &str, answer: Value) {
        self.results.insert(id.to_string(), answer.clone());
        let (mine, others): (Vec<_>, Vec<_>) = std::mem::take(&mut self.waiters)
            .into_iter()
            .partition(|w| w.id == id);
        self.waiters = others;
        for w in mine {
            self.give(w.reply, Ok(answer.clone()));
        }
    }

    fn batch(&mut self, b: &mut impl Backend, args: &Value, deadline: Instant, reply: Reply) {
        let Some(steps) = args
            .get("steps")
            .and_then(Value::as_array)
            .filter(|s| !s.is_empty())
        else {
            return self.give_plain(
                reply,
                Err("`steps` must be a list of {command, args}.".into()),
            );
        };
        if let Some(busy) = self.busy(b) {
            return self.give(reply, busy);
        }
        match if_revision(args) {
            Err(e) => return self.give_plain(reply, Err(e)),
            Ok(Some(r)) => {
                b.app_mut().mcp_outside();
                if r != b.app().mcp.log.revision {
                    let answer = conflict(b, r);
                    return self.give(reply, answer);
                }
            }
            Ok(None) => {}
        }
        self.batch = Some(Batch {
            steps: steps.clone(),
            next: 0,
            done: vec![],
            stop_on_error: args
                .get("stop_on_error")
                .and_then(Value::as_bool)
                .unwrap_or(true),
            deadline,
            running: None,
            reply,
        });
        self.advance(b);
    }

    /// Starts the batch's next steps until one opens a window, the batch
    /// stops at a step that did not go, or its time is up.
    fn advance(&mut self, b: &mut impl Backend) {
        loop {
            let Some(batch) = &mut self.batch else {
                return;
            };
            let stopped = batch.stop_on_error && batch.done.last().is_some_and(|s| !went(s));
            if stopped || batch.next >= batch.steps.len() || Instant::now() >= batch.deadline {
                return self.finish_batch(b);
            }
            let args = batch.steps[batch.next].clone();
            batch.next += 1;
            let name = args
                .get("command")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            match self.start(b, &args) {
                Start::Opened(id) => {
                    if let Some(batch) = &mut self.batch {
                        batch.running = Some((name, id));
                    }
                    return;
                }
                Start::Now(answer) => {
                    if let Some(batch) = &mut self.batch {
                        batch.done.push(step(&name, answer));
                    }
                }
            }
        }
    }

    /// Answers the batch: each step's outcome, the one still working with
    /// its command_id, and the steps after it not run.
    fn finish_batch(&mut self, b: &mut impl Backend) {
        let Some(batch) = self.batch.take() else {
            return;
        };
        let mut steps = batch.done;
        let running = batch.running.is_some();
        if let Some((name, id)) = &batch.running {
            steps.push(step(name, Ok(still_running(b, id))));
        }
        let failed = steps
            .iter()
            .any(|s| !went(s) && s["outcome"] != "still_running");
        for args in &batch.steps[batch.next..] {
            let name = args
                .get("command")
                .and_then(Value::as_str)
                .unwrap_or_default();
            steps.push(json!({"command": name, "outcome": "not_run"}));
        }
        let unfinished = steps.iter().any(|s| s["outcome"] == "not_run");
        let outcome = if running {
            "still_running"
        } else if failed {
            "stopped"
        } else if unfinished {
            "timed_out"
        } else {
            "done"
        };
        let answer = json!({"outcome": outcome, "backend": b.kind(), "steps": steps});
        self.give(batch.reply, Ok(answer));
    }
}

/// A batch step's entry: the command's name with its result.
fn step(name: &str, answer: Answer) -> Value {
    match answer {
        Ok(Value::Object(mut report)) => {
            report.insert("command".into(), json!(name));
            Value::Object(report)
        }
        Ok(other) => json!({"command": name, "outcome": "done", "result": other}),
        Err(e) => json!({"command": name, "outcome": "error", "reason": e}),
    }
}
/// Whether a step went: done or no_change.
fn went(step: &Value) -> bool {
    matches!(step["outcome"].as_str(), Some("done" | "no_change"))
}
/// The result of a command whose work is still running.
fn still_running(b: &impl Backend, id: &str) -> Value {
    let reason = format!(
        "The command was carried out and its work is still running ({}). Call wait with its command_id for the final result.",
        b.pending().join(", ")
    );
    Report::of(b, Outcome::StillRunning, Some(reason))
        .with_id(id)
        .json()
}

/// A call answered at once, for a backend whose work is done as it is
/// sent (the tests' `App` driven straight): the call, then `poll`.
#[cfg(test)]
pub fn call_now(agent: &mut Agent, b: &mut impl Backend, tool: &str, args: Value) -> Answer {
    use std::sync::{Arc, Mutex};
    let answer: Arc<Mutex<Option<Answer>>> = Arc::default();
    let into = answer.clone();
    agent.call(
        b,
        tool,
        args,
        Box::new(move |a| *into.lock().unwrap() = Some(a)),
    );
    for _ in 0..4 {
        if let Some(a) = answer.lock().unwrap().take() {
            return a;
        }
        agent.poll(b);
    }
    let taken = answer.lock().unwrap().take();
    taken.unwrap_or_else(|| Err("no answer".into()))
}

/// `list_checkpoints`: each with whether it can be restored now.
fn list_checkpoints(b: &impl Backend) -> Value {
    let app = b.app();
    let log = &app.mcp.log;
    let list: Vec<Value> = (app.mcp.checkpoints.list.iter())
        .map(|m| {
            let why = checkpoint::other_project(app, m.projects, &m.project_id)
                .map(String::from)
                .or_else(|| {
                    (log.since(m.revision)).any(|e| e.source != state::Source::Agent).then(|| {
                        "The user changed the job since: restoring it needs discard_user_changes: true."
                            .to_string()
                    })
                });
            json!({
                "name": m.name,
                "revision": m.revision,
                "taken": checkpoint::utc(m.taken),
                "title": m.title,
                "restorable": why.is_none(),
                "why_not": why,
            })
        })
        .collect();
    json!({"backend": b.kind(), "checkpoints": list, "most": checkpoint::MOST})
}

fn no_command(name: &str) -> String {
    match commands::LATER.iter().find(|(n, _)| *n == name) {
        Some((_, what)) => format!("{name} is not in this version yet ({what})."),
        None => format!("There is no command {name}; list_commands lists them."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wait_s_is_kept_within_its_bounds() {
        assert_eq!(wait_of(&json!({})), Duration::from_secs(25));
        assert_eq!(wait_of(&json!({"wait_s": 400})), Duration::from_secs(50));
        assert_eq!(wait_of(&json!({"wait_s": -1})), Duration::ZERO);
        assert_eq!(
            wait_of(&json!({"wait_s": 2.5})),
            Duration::from_millis(2500)
        );
    }

    #[test]
    fn list_commands_filters_and_shows_the_refused_ones() {
        let listed = |args: Value| -> Vec<(String, bool)> {
            (list_commands(&args)["commands"].as_array().unwrap().iter())
                .map(|c| (c["name"].as_str().unwrap().into(), c["allowed"] == true))
                .collect()
        };
        let setup = listed(json!({"group": "setup"}));
        assert!(setup.iter().all(|(_, allowed)| *allowed));
        assert!(setup.iter().any(|(n, _)| n == "select_setup"));
        let machine = listed(json!({"group": "machine"}));
        for (name, allowed) in [
            ("estop", true),
            ("hold", true),
            ("cycle_start", false),
            ("jog", false),
        ] {
            assert!(
                machine.contains(&(name.into(), allowed)),
                "{name}: {machine:?}"
            );
        }
        let all = listed(json!({}));
        assert_eq!(all.len(), commands::all().len() + commands::REFUSED.len());
        assert!(listed(json!({"filter": "SPINDLE"})).contains(&("spindle".into(), false)));
        let pocket = describe_command(&json!({"name": "add_operation"})).unwrap();
        assert_eq!(
            pocket["args"]["properties"]["strategy"]["enum"][2],
            "Pocket"
        );
        let refused = describe_command(&json!({"name": "home"})).unwrap();
        assert_eq!(refused["allowed"], false);
        assert!(describe_command(&json!({"name": "fly"})).is_err());
    }
}
