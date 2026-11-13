//! The engine (LIO-470): carries out each MCP tool call against an `App`,
//! on the thread that owns it. It is the same code for a headless session
//! (`runner`) and, later, the live window; what differs is the `Backend`.
//!
//! A command is checked before anything is sent (its precondition), sent as
//! the `Message`s a click would send, waited for until the app's work has
//! settled (`App::pending`) or `wait_s` is up, and then checked for its
//! expected effect. The result says which of these happened (`Outcome`).
use crate::{App, Message, Page, model::Strategy};
use serde::Serialize;
use serde_json::{Value, json};
use std::time::{Duration, Instant};

/// What the engine needs of the side that owns the `App`.
pub trait Backend {
    fn app(&self) -> &App;
    /// Sends `message` to the `App` as a click would, and runs the work
    /// it starts.
    fn send(&mut self, message: Message);
    /// Lets the `App` work until what is due has settled (`pending` is
    /// empty) or `deadline` passes; whether it settled.
    fn settle(&mut self, deadline: Instant) -> bool;
    /// The work still due: `App::pending`, and the backend's own.
    fn pending(&self) -> Vec<&'static str>;
    /// "headless", or "live" for the window.
    fn kind(&self) -> &'static str;
    /// The session's id and folder.
    fn session(&self) -> Value;
}

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
            "description": "Runs one command as the user's click would, waits for its work, and says what happened: `outcome` done, no_change, refused, failed or still_running, with `reason`, the status line, the selection and the work still pending.",
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
    ])
}

/// Carries out tool `tool` with `args`, waiting for its work until
/// `deadline`. An `Err` is a call that could not be carried out at all:
/// an unknown tool or command, or arguments that do not read.
pub fn call(
    b: &mut impl Backend,
    tool: &str,
    args: &Value,
    deadline: Instant,
) -> Result<Value, String> {
    match tool {
        "inspect" => Ok(inspect(b)),
        "list_commands" => Ok(list_commands(args)),
        "describe_command" => describe_command(args),
        "execute" => execute(b, args, deadline),
        _ => Err(format!("There is no tool {tool}.")),
    }
}

/// Whether `tool` is one of `tools()`.
pub fn is_tool(tool: &str) -> bool {
    matches!(
        tool,
        "inspect" | "list_commands" | "describe_command" | "execute"
    )
}

/// What became of a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// Its expected effect holds.
    Done,
    /// It was already so; nothing was sent.
    NoChange,
    /// Its precondition failed; nothing was sent.
    Refused,
    /// It was sent, and its expected effect does not hold.
    Failed,
    /// It was sent, and its work had not settled within `wait_s`.
    StillRunning,
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
            reason,
            backend: b.kind(),
            status: app.status.clone(),
            status_problem: app.status_problem,
            selection: Selection::of(app),
            pending: b.pending(),
            notes: vec![],
        }
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
const GROUPS: [&str; 11] = [
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

/// Why a command was not sent.
enum Problem {
    /// Its arguments do not read: an error of the call.
    Args(String),
    /// Its precondition failed: outcome `refused`.
    Refused(String),
}
/// The effect a command must have: `Err` says what did not happen.
type Expect = Box<dyn Fn(&App) -> Result<(), String>>;
/// What a command's precondition decided.
enum Prepared {
    /// Send these messages, then check the effect.
    Send(Vec<Message>, Expect),
    /// It is already so: why.
    Unchanged(String),
}

/// A command `execute` runs.
struct Command {
    name: &'static str,
    group: &'static str,
    /// One line.
    help: &'static str,
    /// What must exist or be selected.
    requires: &'static str,
    /// What `execute` checks for after it.
    effect: &'static str,
    /// The JSON schema of its `args`.
    schema: fn() -> Value,
    /// Its fuller help.
    details: fn() -> String,
    /// Its precondition, and the messages and effect when it holds.
    prepare: fn(&App, &Value) -> Result<Prepared, Problem>,
}

const COMMANDS: &[Command] = &[
    Command {
        name: "set_project_title",
        group: "project",
        help: "Names the project.",
        requires: "Nothing.",
        effect: "The project's title is `title`.",
        schema: || {
            json!({
                "type": "object",
                "properties": {"title": {"type": "string"}},
                "required": ["title"],
            })
        },
        details: || {
            "The title names the project in the window's title bar, its setup sheets and its programs' headers.".into()
        },
        prepare: |app, args| {
            let title = string(args, "title")?;
            if app.doc.title == title {
                return Ok(Prepared::Unchanged(format!(
                    "The project is already titled \"{title}\"."
                )));
            }
            let expect = title.clone();
            Ok(Prepared::Send(
                vec![Message::Title(title)],
                Box::new(move |app| {
                    (app.doc.title == expect)
                        .then_some(())
                        .ok_or_else(|| "The title was not changed.".into())
                }),
            ))
        },
    },
    Command {
        name: "select_setup",
        group: "setup",
        help: "Makes a setup active, with no operation selected, on the Work page.",
        requires: "The setup exists.",
        effect: "The setup is the active one.",
        schema: || {
            json!({
                "type": "object",
                "properties": {"setup": addressed("setup")},
                "required": ["setup"],
            })
        },
        details: || help_text(&Message::SelectSetup(0)),
        prepare: |app, args| {
            let si = setup_of(app, arg(args, "setup")?)?;
            let id = app.doc.setups[si].id.clone();
            if app.doc.active == id && app.selected.is_none() && app.page == Page::Work {
                return Ok(Prepared::Unchanged(format!(
                    "{} is already the active setup.",
                    app.doc.setups[si].name()
                )));
            }
            Ok(Prepared::Send(
                vec![Message::SelectSetup(si)],
                Box::new(move |app| {
                    (app.doc.active == id)
                        .then_some(())
                        .ok_or_else(|| "The setup was not made active.".into())
                }),
            ))
        },
    },
    Command {
        name: "select_operation",
        group: "operation",
        help: "Selects an operation and opens its editor on the Work page.",
        requires: "The operation exists. An index counts in `setup`, else in the active setup; an id may be in any setup, which is made active.",
        effect: "The operation is the selected one.",
        schema: || {
            json!({
                "type": "object",
                "properties": {
                    "operation": addressed("operation"),
                    "setup": addressed("setup"),
                },
                "required": ["operation"],
            })
        },
        details: || help_text(&Message::SelectOp(0)),
        prepare: |app, args| {
            let operation = arg(args, "operation")?;
            let setup = match args.get("setup") {
                Some(s) => Some(setup_of(app, s)?),
                None => None,
            };
            let (si, oi) = operation_of(app, setup, operation)?;
            let s = &app.doc.setups[si];
            let id = s.operations[oi].id.clone();
            if app.doc.active == s.id && app.selected == Some(oi) && app.page == Page::Work {
                return Ok(Prepared::Unchanged(format!(
                    "{} is already selected.",
                    s.operations[oi].name()
                )));
            }
            let mut messages = vec![];
            if app.doc.active != s.id {
                messages.push(Message::SelectSetup(si));
            }
            messages.push(Message::SelectOp(oi));
            Ok(Prepared::Send(
                messages,
                Box::new(move |app| {
                    (app.operation().is_some_and(|o| o.id == id))
                        .then_some(())
                        .ok_or_else(|| "The operation was not selected.".into())
                }),
            ))
        },
    },
    Command {
        name: "add_operation",
        group: "operation",
        help: "Adds an operation of a strategy to the active setup, with the library's first tool that fits.",
        requires: "An active setup; on an imported part, a strategy available for it. With no fitting tool in the library the app opens the Tool library to make one, and the operation is not added.",
        effect: "The active setup has a new operation (and its centre drilling, for a drill on the manual mill), and the new operation is selected.",
        schema: || {
            let names: Vec<String> = Strategy::ALL.iter().map(|s| format!("{s:?}")).collect();
            json!({
                "type": "object",
                "properties": {"strategy": {"type": "string", "enum": names}},
                "required": ["strategy"],
            })
        },
        details: || {
            let strategies: Vec<String> = (Strategy::ALL.iter())
                .map(|s| format!("- {s:?} ({}): {}", s.name(), s.availability()))
                .collect();
            format!(
                "The new operation opens on its Tool tab, or on Geometry, picking, when it needs a place on the part. The strategies:\n{}",
                strategies.join("\n")
            )
        },
        prepare: |app, args| {
            let strategy = strategy_of(&string(args, "strategy")?)?;
            let Some(setup) = app.doc.active_setup() else {
                return Err(Problem::Refused(
                    "There is no setup to add the operation to: add a setup first.".into(),
                ));
            };
            if !app.is_sample() && !strategy.imported_preview() {
                return Err(Problem::Refused(format!(
                    "{} cannot be added to an imported part: {}",
                    strategy.name(),
                    strategy.availability()
                )));
            }
            let (id, before) = (setup.id.clone(), setup.operations.clone());
            Ok(Prepared::Send(
                vec![Message::AddOp(strategy)],
                Box::new(move |app| {
                    let setup = app.doc.setups.iter().find(|s| s.id == id);
                    let added = setup.is_some_and(|s| s.operations.len() > before.len());
                    let selected = app.operation().is_some_and(|o| {
                        o.strategy == strategy && !before.iter().any(|b| b.id == o.id)
                    });
                    (added && selected && app.doc.active == id)
                        .then_some(())
                        .ok_or_else(|| "No operation was added.".into())
                }),
            ))
        },
    },
];

fn command(name: &str) -> Option<&'static Command> {
    COMMANDS.iter().find(|c| c.name == name)
}

/// The schema of an argument that names a setup or an operation.
fn addressed(what: &str) -> Value {
    json!({
        "type": ["string", "integer"],
        "description": format!("The {what}'s id, or its index from 0 as inspect lists it."),
    })
}

/// The app's own help for the control that sends `message`.
fn help_text(message: &Message) -> String {
    let help = crate::help::command(message, String::new());
    if help.more.is_empty() {
        help.body
    } else {
        format!("{}\n\n{}", help.body, help.more)
    }
}

fn arg<'a>(args: &'a Value, key: &str) -> Result<&'a Value, Problem> {
    args.get(key)
        .ok_or_else(|| Problem::Args(format!("`{key}` is missing.")))
}
fn string(args: &Value, key: &str) -> Result<String, Problem> {
    (arg(args, key)?.as_str())
        .map(str::to_string)
        .ok_or_else(|| Problem::Args(format!("`{key}` must be a string.")))
}

/// The setup `v` names: an id, or an index.
fn setup_of(app: &App, v: &Value) -> Result<usize, Problem> {
    let setups = &app.doc.setups;
    let found = match v {
        Value::String(id) => setups.iter().position(|s| s.id == *id),
        Value::Number(n) => n.as_u64().map(|i| i as usize).filter(|i| *i < setups.len()),
        _ => return Err(Problem::Args("`setup` must be an id or an index.".into())),
    };
    found.ok_or_else(|| {
        Problem::Refused(format!(
            "There is no setup {v}; the job has {} (inspect lists them).",
            setups.len()
        ))
    })
}

/// The operation `v` names, as (setup, operation) indices: an id in any
/// setup (or in `setup`), or an index in `setup` or the active setup.
fn operation_of(app: &App, setup: Option<usize>, v: &Value) -> Result<(usize, usize), Problem> {
    let setups = &app.doc.setups;
    let found = match v {
        Value::String(id) => (setups.iter().enumerate())
            .filter(|(si, _)| setup.is_none_or(|s| s == *si))
            .find_map(|(si, s)| Some((si, s.operations.iter().position(|o| o.id == *id)?))),
        Value::Number(n) => {
            let si = match setup.or_else(|| setups.iter().position(|s| s.id == app.doc.active)) {
                Some(si) => si,
                None => {
                    return Err(Problem::Refused(
                        "There is no active setup: select a setup first.".into(),
                    ));
                }
            };
            (n.as_u64().map(|i| i as usize))
                .filter(|i| *i < setups[si].operations.len())
                .map(|oi| (si, oi))
        }
        _ => {
            return Err(Problem::Args(
                "`operation` must be an id or an index.".into(),
            ));
        }
    };
    found.ok_or_else(|| {
        Problem::Refused(format!(
            "There is no operation {v} there (inspect lists them)."
        ))
    })
}

/// The strategy `name` names: as `inspect` writes it ("FiveAxis"), its
/// tile's word ("5-axis") or its full name ("Adaptive clearing").
fn strategy_of(name: &str) -> Result<Strategy, Problem> {
    let name = name.trim();
    (Strategy::ALL.iter().copied())
        .find(|s| {
            [format!("{s:?}"), s.short().into(), s.name().into()]
                .iter()
                .any(|n| n.eq_ignore_ascii_case(name))
        })
        .ok_or_else(|| {
            let names: Vec<String> = Strategy::ALL.iter().map(|s| format!("{s:?}")).collect();
            Problem::Args(format!(
                "There is no strategy \"{name}\"; the strategies are {}.",
                names.join(", ")
            ))
        })
}

fn list_commands(args: &Value) -> Value {
    let filter = (args.get("filter").and_then(Value::as_str))
        .unwrap_or("")
        .to_lowercase();
    let group = args.get("group").and_then(Value::as_str);
    let mut listed: Vec<&Command> = (COMMANDS.iter())
        .filter(|c| group.is_none_or(|g| c.group == g))
        .filter(|c| c.name.contains(&filter) || c.help.to_lowercase().contains(&filter))
        .collect();
    listed.sort_by_key(|c| (GROUPS.iter().position(|g| *g == c.group), c.name));
    let commands: Vec<Value> = (listed.iter())
        .map(|c| json!({"name": c.name, "group": c.group, "help": c.help, "allowed": true}))
        .collect();
    json!({"commands": commands})
}

fn describe_command(args: &Value) -> Result<Value, String> {
    let name = (args.get("name").and_then(Value::as_str))
        .ok_or("`name` is missing: give a command's name from list_commands.")?;
    let c = command(name).ok_or_else(|| no_command(name))?;
    Ok(json!({
        "name": c.name,
        "group": c.group,
        "help": c.help,
        "details": (c.details)(),
        "args": (c.schema)(),
        "requires": c.requires,
        "effect": c.effect,
        "allowed": true,
    }))
}

fn no_command(name: &str) -> String {
    format!("There is no command {name}; list_commands lists them.")
}

fn execute(b: &mut impl Backend, args: &Value, deadline: Instant) -> Result<Value, String> {
    let name = (args.get("command").and_then(Value::as_str))
        .ok_or("`command` is missing: give a command's name from list_commands.")?;
    let c = command(name).ok_or_else(|| no_command(name))?;
    let empty = json!({});
    let command_args = match args.get("args") {
        None | Some(Value::Null) => &empty,
        Some(a @ Value::Object(_)) => a,
        Some(_) => return Err("`args` must be an object.".into()),
    };
    if b.app().locked() {
        return Ok(Report::of(b, Outcome::Refused, Some(LOCKED.into())).json());
    }
    let (messages, expect) = match (c.prepare)(b.app(), command_args) {
        Err(Problem::Args(e)) => return Err(format!("{name}: {e}")),
        Err(Problem::Refused(reason)) => {
            return Ok(Report::of(b, Outcome::Refused, Some(reason)).json());
        }
        Ok(Prepared::Unchanged(reason)) => {
            return Ok(Report::of(b, Outcome::NoChange, Some(reason)).json());
        }
        Ok(Prepared::Send(messages, expect)) => (messages, expect),
    };
    let status = b.app().status.clone();
    for message in messages {
        b.send(message);
    }
    if !b.settle(deadline) {
        let reason = format!(
            "The command was carried out and its work is still running ({}). Call inspect to see when it is done.",
            b.pending().join(", ")
        );
        return Ok(Report::of(b, Outcome::StillRunning, Some(reason)).json());
    }
    let report = match expect(b.app()) {
        Ok(()) => Report::of(b, Outcome::Done, None),
        Err(sentence) => {
            // The app's own words, when the command changed the status line.
            let said = &b.app().status;
            let reason = if *said != status && !said.is_empty() {
                said.clone()
            } else {
                sentence
            };
            Report::of(b, Outcome::Failed, Some(reason))
        }
    };
    Ok(report.json())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_command_has_help_a_schema_and_a_group() {
        for c in COMMANDS {
            assert!(
                !c.help.is_empty() && !(c.details)().is_empty(),
                "{}",
                c.name
            );
            assert_eq!((c.schema)()["type"], "object", "{}", c.name);
            assert!(GROUPS.contains(&c.group), "{}", c.name);
        }
        for t in tools().as_array().unwrap() {
            assert!(is_tool(t["name"].as_str().unwrap()));
            assert_eq!(t["inputSchema"]["type"], "object");
        }
    }

    #[test]
    fn strategies_read_by_any_of_their_names() {
        assert!(matches!(strategy_of("FiveAxis"), Ok(Strategy::FiveAxis)));
        assert!(matches!(strategy_of("5-axis"), Ok(Strategy::FiveAxis)));
        assert!(matches!(
            strategy_of("adaptive clearing"),
            Ok(Strategy::Adaptive)
        ));
        assert!(matches!(strategy_of(" pocket "), Ok(Strategy::Pocket)));
        assert!(matches!(strategy_of("Laser"), Err(Problem::Args(_))));
    }

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
    fn list_commands_filters_by_word_and_group() {
        let names = |v: Value| -> Vec<String> {
            (v["commands"].as_array().unwrap().iter())
                .map(|c| c["name"].as_str().unwrap().to_string())
                .collect()
        };
        assert_eq!(
            names(list_commands(&json!({"group": "setup"}))),
            ["select_setup"]
        );
        assert_eq!(
            names(list_commands(&json!({"filter": "SELECT"}))),
            ["select_setup", "select_operation"]
        );
        assert_eq!(
            list_commands(&json!({}))["commands"]
                .as_array()
                .unwrap()
                .len(),
            COMMANDS.len()
        );
        assert!(
            describe_command(&json!({"name": "add_operation"})).unwrap()["details"]
                .as_str()
                .unwrap()
                .contains("Pocket")
        );
        assert!(describe_command(&json!({"name": "fly"})).is_err());
    }
}
