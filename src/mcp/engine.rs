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
use super::commands::{self, Ctx, Expect, Problem};
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
    /// Sends `message` to the `App` as a click would, and runs the work
    /// it starts.
    fn send(&mut self, message: Message);
    /// Runs `task` as the `App`'s own tasks run, its messages sent to it.
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
    json!([
        {
            "name": "inspect",
            "title": "Inspect the job",
            "description": "The state as the user would read it: the project (title, project file, part, material, whether it is saved), its setups and their operations by id, the selection with its page and tab, the status line, the work still pending, the backend and the session.",
            "inputSchema": {"type": "object", "properties": {"wait_s": wait}},
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
            "description": "Runs one command as the user's click would, waits for its work, and says what happened: `outcome` done, no_change, refused, busy, failed or still_running (with a command_id for wait and cancel), with `reason`, the status line, the selection and the work still pending.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "command": {"type": "string", "description": "A name from list_commands."},
                    "args": {"type": "object", "description": "The command's arguments (describe_command gives their schema)."},
                    "wait_s": wait,
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
            "name": "cancel",
            "title": "Cancel a command",
            "description": "Cancels the generation a still_running command started; its final result is cancelled. Other work cannot be cancelled: wait for it.",
            "inputSchema": {
                "type": "object",
                "properties": {"command_id": id, "wait_s": wait},
                "required": ["command_id"],
            },
        },
    ])
}

/// Whether `tool` is one of `tools()`.
pub fn is_tool(tool: &str) -> bool {
    matches!(
        tool,
        "inspect" | "list_commands" | "describe_command" | "execute" | "batch" | "wait" | "cancel"
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
    fn of(app: &App) -> Selection {
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
    /// The status line after the command.
    pub status: String,
    /// The status line reports a problem the checks found.
    pub status_problem: bool,
    pub selection: Selection,
    pub pending: Vec<&'static str>,
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
            status: app.status.clone(),
            status_problem: app.status_problem,
            selection: Selection::of(app),
            pending: b.pending(),
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
    json!({"commands": commands})
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
        "waits": "for the work it starts to settle (pending empty), up to wait_s",
        "allowed": true,
    }))
}

/// A `refused` result.
fn refuse(b: &impl Backend, why: String) -> Answer {
    Ok(Report::of(b, Outcome::Refused, Some(why)).json())
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
}
impl Agent {
    pub fn new() -> Agent {
        Agent::default()
    }
    /// Whether a call waits for the app's work: the event loop then ticks
    /// fast, as the window's does for a pending MCP request.
    pub fn waiting(&self) -> bool {
        self.window.is_some() || !self.held.is_empty() || !self.started
    }
    /// Whether nothing is open or unanswered.
    pub fn idle(&self) -> bool {
        self.window.is_none()
            && self.waiters.is_empty()
            && self.batch.is_none()
            && self.held.is_empty()
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
    }
    fn answer(
        &mut self,
        b: &mut impl Backend,
        tool: &str,
        args: Value,
        deadline: Instant,
        reply: Reply,
    ) {
        match tool {
            "inspect" => reply(Ok(inspect(b))),
            "list_commands" => reply(Ok(list_commands(&args))),
            "describe_command" => reply(describe_command(&args)),
            "execute" => self.execute(b, &args, deadline, reply),
            "batch" => self.batch(b, &args, deadline, reply),
            "wait" => self.wait(&args, deadline, reply),
            "cancel" => self.cancel(b, &args, deadline, reply),
            _ => reply(Err(format!("There is no tool {tool}."))),
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
            for h in std::mem::take(&mut self.held) {
                if self.started || h.deadline <= now {
                    self.answer(b, &h.tool, h.args, h.deadline, h.reply);
                } else {
                    self.held.push(h);
                }
            }
        }
        if settled && self.window.is_some() {
            self.close(b);
        }
        let now = Instant::now();
        let (due, waiting): (Vec<_>, Vec<_>) = std::mem::take(&mut self.waiters)
            .into_iter()
            .partition(|w| w.deadline <= now);
        self.waiters = waiting;
        for w in due {
            (w.reply)(Ok(still_running(b, &w.id)));
        }
        if self
            .batch
            .as_ref()
            .is_some_and(|batch| batch.deadline <= now)
        {
            self.finish_batch(b);
        }
    }

    fn busy(&self, b: &impl Backend) -> Option<Answer> {
        let (name, id) = match (&self.window, &self.batch) {
            (Some(w), _) => (w.name.to_string(), w.id.clone()),
            (None, Some(_)) => ("batch".to_string(), "the batch".to_string()),
            (None, None) => return None,
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
            return reply(busy);
        }
        match self.start(b, args) {
            Start::Now(answer) => reply(answer),
            Start::Opened(id) => self.waiters.push(Waiter {
                id,
                deadline,
                reply,
            }),
        }
    }

    /// Checks a command and sends it: an answer at once when nothing was
    /// sent (refused, no_change, an error) or for the stops, else a window.
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
        if b.app().locked() && !c.while_locked {
            return Start::Now(refuse(b, LOCKED.into()));
        }
        let folder = b.session_folder();
        let prepared = {
            let app = b.app();
            commands::resolve(c.on, app, command_args).and_then(|(setup, operation)| {
                let ctx = Ctx {
                    app,
                    args: command_args,
                    session: folder.as_deref(),
                    setup,
                    operation,
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
        // Nothing of a command is sent unless all of it may be.
        if let Err(why) = commands::guard(&dispatch.messages) {
            return Start::Now(refuse(b, why.into()));
        }
        self.count += 1;
        let id = format!("{}-{}", c.name, self.count);
        let status = b.app().status.clone();
        for message in dispatch.messages {
            b.send(message);
        }
        if let Some(task) = dispatch.task {
            b.run(task);
        }
        if c.group == "machine" {
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
        });
        Start::Opened(id)
    }

    /// Closes the window: its effect checked, its result kept and given to
    /// the calls waiting for it, and a batch's next step started.
    fn close(&mut self, b: &mut impl Backend) {
        let Some(w) = self.window.take() else {
            return;
        };
        let (outcome, reason) = if w.cancelled {
            (
                Outcome::Cancelled,
                Some("Cancelled at the agent's request.".to_string()),
            )
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
        let report = Report::of(b, outcome, reason).with_id(&w.id).json();
        self.results.insert(w.id.clone(), report.clone());
        let (theirs, others): (Vec<_>, Vec<_>) = std::mem::take(&mut self.waiters)
            .into_iter()
            .partition(|x| x.id == w.id);
        self.waiters = others;
        for x in theirs {
            (x.reply)(Ok(report.clone()));
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
            return reply(Err("`command_id` is missing.".into()));
        };
        if let Some(result) = self.results.get(id) {
            return reply(Ok(result.clone()));
        }
        if self.window.as_ref().is_some_and(|w| w.id == id) {
            let id = id.to_string();
            return self.waiters.push(Waiter {
                id,
                deadline,
                reply,
            });
        }
        reply(Err(format!("There is no command {id} in this session.")))
    }

    fn cancel(&mut self, b: &mut impl Backend, args: &Value, deadline: Instant, reply: Reply) {
        let Some(id) = args.get("command_id").and_then(Value::as_str) else {
            return reply(Err("`command_id` is missing.".into()));
        };
        if let Some(result) = self.results.get(id) {
            return reply(Ok(json!({
                "cancelled": false,
                "command_id": id,
                "reason": "It had finished; this is its result.",
                "result": result,
            })));
        }
        if !self.window.as_ref().is_some_and(|w| w.id == id) {
            return reply(Err(format!("There is no command {id} in this session.")));
        }
        if b.app().generating.is_none() {
            return reply(Ok(json!({
                "cancelled": false,
                "command_id": id,
                "reason": "Only a generation can be cancelled, and this command's work is not one: wait for it.",
            })));
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

    fn batch(&mut self, b: &mut impl Backend, args: &Value, deadline: Instant, reply: Reply) {
        let Some(steps) = args
            .get("steps")
            .and_then(Value::as_array)
            .filter(|s| !s.is_empty())
        else {
            return reply(Err("`steps` must be a list of {command, args}.".into()));
        };
        if let Some(busy) = self.busy(b) {
            return reply(busy);
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
        (batch.reply)(Ok(
            json!({"outcome": outcome, "backend": b.kind(), "steps": steps}),
        ));
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
