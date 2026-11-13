//! The engine (LIO-470): carries out each MCP tool call against an `App`,
//! on the thread that owns it. It is the same code for a headless session
//! (`runner`) and, later, the live window; what differs is the `Backend`.
//!
//! A command is checked before anything is sent (its precondition), sent as
//! the `Message`s a click would send, waited for until the app's work has
//! settled (`App::pending`) or `wait_s` is up, and then checked for its
//! expected effect. The result says which of these happened (`Outcome`).
use super::commands::{self, Ctx, Problem};
use crate::{App, Message};
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
    /// The headless session's folder, the one place in the app's data
    /// folder a command may write; None in the window.
    fn session_folder(&self) -> Option<std::path::PathBuf>;
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
fn refuse(b: &impl Backend, why: String) -> Result<Value, String> {
    Ok(Report::of(b, Outcome::Refused, Some(why)).json())
}

fn no_command(name: &str) -> String {
    match commands::LATER.iter().find(|(n, _)| *n == name) {
        Some((_, what)) => format!("{name} is not in this version yet ({what})."),
        None => format!("There is no command {name}; list_commands lists them."),
    }
}

fn execute(b: &mut impl Backend, args: &Value, deadline: Instant) -> Result<Value, String> {
    let name = (args.get("command").and_then(Value::as_str))
        .ok_or("`command` is missing: give a command's name from list_commands.")?;
    let Some(c) = commands::find(name) else {
        return match commands::REFUSED.iter().find(|r| r.0 == name) {
            Some((_, _, reason)) => refuse(b, reason.to_string()),
            None => Err(no_command(name)),
        };
    };
    let empty = json!({});
    let command_args = match args.get("args") {
        None | Some(Value::Null) => &empty,
        Some(a @ Value::Object(_)) => a,
        Some(_) => return Err("`args` must be an object.".into()),
    };
    if b.app().locked() && !c.while_locked {
        return refuse(b, LOCKED.into());
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
            if let Some(why) = c.unchanged.and_then(|f| f(&ctx)) {
                return Ok(Err(why));
            }
            let messages = c.messages(&ctx)?;
            Ok(Ok((messages, c.expect.map(|f| f(&ctx)))))
        })
    };
    let (messages, expect) = match prepared {
        Err(Problem::Args(e)) => return Err(format!("{name}: {e}")),
        Err(Problem::Refused(why)) => return refuse(b, why),
        Ok(Err(why)) => return Ok(Report::of(b, Outcome::NoChange, Some(why)).json()),
        Ok(Ok(prepared)) => prepared,
    };
    // Nothing of a command is sent unless all of it may be.
    if let Err(why) = commands::guard(&messages) {
        return refuse(b, why.into());
    }
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
    let Some(expect) = expect else {
        let mut report = Report::of(b, Outcome::Done, None);
        report.notes.push(
            "Sent and settled; this command's effect is not checked yet, so read the status line and inspect.".into(),
        );
        return Ok(report.json());
    };
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
