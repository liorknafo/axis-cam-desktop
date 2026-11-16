//! The commands `execute` runs (LIO-470): one table, each entry with its
//! group, help, argument schema, precondition and builder. A builder turns
//! the arguments and the `App` into the `Message`s the user's clicks would
//! send; for a file command it does the dialog's work and sends the
//! dialog's answer. Every message a builder makes must be of a `Command`
//! class (`class`), which the engine checks before it sends any.
//!
//! Setups and operations are given by id or by index; an index is turned
//! into the id when the call arrives. A command that acts on the selected
//! setup or operation selects the one given first, in the same turn.
use super::{
    class::{self, Class},
    fields,
    pick::{self, Mode},
    state::{Completion, Kind},
};
use crate::{
    App, GuideAction, Message, Page, Target, control, model::Strategy, run, section, settings,
    stock_surface, ui, viewport,
};
use iced::Task;
use serde_json::{Value, json};
use std::{
    path::{Component, Path, PathBuf},
    sync::LazyLock,
};

/// What a command sees: the `App`, the arguments, the headless session's
/// folder (None in the window), and the setup and operation the
/// arguments address, already resolved.
pub struct Ctx<'a> {
    pub app: &'a App,
    pub args: &'a Value,
    pub session: Option<&'a Path>,
    /// The setup a setup command acts on, by index.
    pub setup: Option<usize>,
    /// The operation an operation command acts on: (setup, operation).
    pub operation: Option<(usize, usize)>,
    /// The fields `edit_field` may set, as the editors now draw them
    /// (`fields::of`), or why they could not be listed. Only `edit_field`
    /// has them.
    pub fields: Option<Result<Vec<ui::FieldInfo>, String>>,
}
impl Ctx<'_> {
    /// The fields `edit_field` checks its key against.
    fn listed(&self) -> Result<&[ui::FieldInfo], Problem> {
        match &self.fields {
            Some(Ok(fields)) => Ok(fields),
            Some(Err(why)) => refused(why.clone()),
            None => refused("The editor's fields were not listed."),
        }
    }
    fn op(&self) -> Option<&crate::model::Operation> {
        let (si, oi) = self.operation?;
        self.app.doc.setups.get(si)?.operations.get(oi)
    }
}

/// Why a command is not sent.
#[derive(Debug, Clone)]
pub enum Problem {
    /// Its arguments do not read: an error of the call.
    Args(String),
    /// Its precondition failed: outcome `refused`, and nothing was done.
    Refused(String),
}
fn refused<T>(why: impl Into<String>) -> Result<T, Problem> {
    Err(Problem::Refused(why.into()))
}
fn bad<T>(why: impl Into<String>) -> Result<T, Problem> {
    Err(Problem::Args(why.into()))
}

/// What the effect check found: `Ok(None)` it holds (done), `Ok(Some(why))`
/// it already held and nothing changed (no_change), `Err(what)` it does not
/// hold (failed).
pub type Verdict = Result<Option<String>, String>;
/// The effect a command must have, checked on the `App` once its work has
/// settled; made before the command is sent, from the state it starts on.
pub type Expect = Box<dyn Fn(&App) -> Verdict>;

/// What a command acts on, selected first when the arguments name it.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum On {
    Nothing,
    /// The active setup, or `setup`.
    Setup,
    /// The selected operation, or `operation` (an id in any setup, or an
    /// index in the active setup).
    Operation,
}

/// Where a command's help comes from.
enum Help {
    /// The app's own help for the control that sends this message, with
    /// the control's label.
    Of(fn() -> Message, &'static str),
    Text(&'static str),
}

/// How a command makes its messages.
enum Build {
    Sends(fn() -> Message),
    With(fn(&Ctx) -> Result<Vec<Message>, Problem>),
    /// Messages, and work off the `App`'s thread whose answer is a message:
    /// what a file dialog's command does after the dialog (a load, a
    /// write), so the event loop keeps taking events meanwhile.
    Work(fn(&Ctx) -> Worked),
    /// Clicks on the part, each checked for what it did (`pick`).
    Clicks(fn(&Ctx) -> Result<pick::Job, Problem>),
}

/// A `Build::Work` builder's result: its messages and its work.
type Worked = Result<(Vec<Message>, Task<Message>), Problem>;
/// What a command sends: its messages in order, then its work, or its
/// clicks.
pub struct Dispatch {
    pub messages: Vec<Message>,
    pub task: Option<Task<Message>>,
    pub clicks: Option<pick::Job>,
}

/// A command `execute` runs.
pub struct Command {
    pub name: &'static str,
    pub group: &'static str,
    help: Help,
    /// What must exist or be selected, in a sentence.
    pub requires: &'static str,
    /// What the command does, as `describe_command` says it.
    pub effect: &'static str,
    /// It may run while the machine runs (`App::locked`): the stops and the
    /// view, which leave the job alone.
    pub while_locked: bool,
    pub on: On,
    /// Its `setup` argument is a value of its own, not where its operation is.
    own_setup: bool,
    params: fn() -> Vec<(&'static str, Value, bool)>,
    examples: fn() -> Vec<Value>,
    precondition: fn(&Ctx) -> Result<(), Problem>,
    build: Build,
    /// Why it is already so, when it is: nothing is sent (`no_change`).
    pub unchanged: Option<fn(&Ctx) -> Option<String>>,
    /// The effect checked once the work has settled (`effects`).
    pub expect: Option<fn(&Ctx) -> Expect>,
}

fn cmd(name: &'static str, group: &'static str) -> Command {
    Command {
        name,
        group,
        help: Help::Text(""),
        requires: "Nothing.",
        effect: "",
        while_locked: false,
        on: On::Nothing,
        own_setup: false,
        params: Vec::new,
        examples: || vec![json!({})],
        precondition: |_| Ok(()),
        build: Build::With(|_| Ok(vec![])),
        unchanged: None,
        expect: None,
    }
}
impl Command {
    /// Sends `m`, and takes the help of its control.
    fn sends(mut self, m: fn() -> Message) -> Self {
        self.build = Build::Sends(m);
        if matches!(self.help, Help::Text("")) {
            self.help = Help::Of(m, "");
        }
        self
    }
    fn with(mut self, f: fn(&Ctx) -> Result<Vec<Message>, Problem>) -> Self {
        self.build = Build::With(f);
        self
    }
    fn work(mut self, f: fn(&Ctx) -> Worked) -> Self {
        self.build = Build::Work(f);
        self
    }
    fn clicks(mut self, f: fn(&Ctx) -> Result<pick::Job, Problem>) -> Self {
        self.build = Build::Clicks(f);
        self
    }
    /// Whether it clicks on the part, each click checked for what it did,
    /// instead of an effect checked once its work has settled.
    pub fn clicks_checked(&self) -> bool {
        matches!(self.build, Build::Clicks(_))
    }
    fn help_of(mut self, m: fn() -> Message, label: &'static str) -> Self {
        self.help = Help::Of(m, label);
        self
    }
    fn text(mut self, help: &'static str) -> Self {
        self.help = Help::Text(help);
        self
    }
    fn requires(mut self, s: &'static str) -> Self {
        self.requires = s;
        self
    }
    fn effect(mut self, s: &'static str) -> Self {
        self.effect = s;
        self
    }
    fn locked_ok(mut self) -> Self {
        self.while_locked = true;
        self
    }
    fn on_setup(mut self) -> Self {
        self.on = On::Setup;
        if self.requires == "Nothing." {
            self.requires = "A setup: the active one, or `setup`.";
        }
        self
    }
    fn on_operation(mut self) -> Self {
        self.on = On::Operation;
        if self.requires == "Nothing." {
            self.requires = "An operation: the selected one, or `operation`.";
        }
        self
    }
    /// Its `setup` argument is not an address (`set_pin_setup`'s pin setup).
    fn own_setup(mut self) -> Self {
        self.own_setup = true;
        self
    }
    fn params(mut self, p: fn() -> Vec<(&'static str, Value, bool)>) -> Self {
        self.params = p;
        self
    }
    fn examples(mut self, e: fn() -> Vec<Value>) -> Self {
        self.examples = e;
        self
    }
    fn when(mut self, f: fn(&Ctx) -> Result<(), Problem>) -> Self {
        self.precondition = f;
        self
    }
    fn unchanged(mut self, f: fn(&Ctx) -> Option<String>) -> Self {
        self.unchanged = Some(f);
        self
    }
    fn expect(mut self, f: fn(&Ctx) -> Expect) -> Self {
        self.expect = Some(f);
        self
    }

    /// The JSON schema of its `args`.
    pub fn schema(&self) -> Value {
        let mut properties = serde_json::Map::new();
        let mut required = vec![];
        let selector = match self.on {
            On::Setup => vec![("setup", addressed("setup"), false)],
            On::Operation => operation_selectors(),
            On::Nothing => vec![],
        };
        for (name, schema, needed) in selector.into_iter().chain((self.params)()) {
            properties.insert(name.into(), schema);
            if needed {
                required.push(name);
            }
        }
        json!({"type": "object", "properties": properties, "required": required})
    }
    /// Its help: one line, and the whole text.
    pub fn help(&self) -> (String, String) {
        let full = match &self.help {
            Help::Of(m, label) => {
                let h = crate::help::command(&m(), label.to_string());
                if h.more.is_empty() {
                    h.body
                } else {
                    format!("{}\n\n{}", h.body, h.more)
                }
            }
            Help::Text(t) => t.to_string(),
        };
        let line = match full.find(". ") {
            Some(end) => full[..=end].to_string(),
            None => full.lines().next().unwrap_or_default().to_string(),
        };
        (line, full)
    }
    /// Example arguments, for the tests and `describe_command`.
    pub fn example_args(&self) -> Vec<Value> {
        (self.examples)()
    }
    /// What the builder makes, without the precondition.
    fn built(&self, ctx: &Ctx) -> Result<Dispatch, Problem> {
        let mut messages = select_first(self.on, ctx)?;
        let (mut task, mut clicks) = (None, None);
        match &self.build {
            Build::Sends(m) => messages.push(m()),
            Build::With(f) => messages.extend(f(ctx)?),
            Build::Work(f) => {
                let (more, work) = f(ctx)?;
                messages.extend(more);
                task = Some(work);
            }
            Build::Clicks(f) => clicks = Some(f(ctx)?),
        }
        Ok(Dispatch {
            messages,
            task,
            clicks,
        })
    }
    /// The setup and operation its arguments address (`address`).
    pub fn resolve(&self, app: &App, args: &Value) -> Result<Addressed, Problem> {
        if self.own_setup {
            let mut own = args.clone();
            if let Some(o) = own.as_object_mut() {
                o.remove("setup");
            }
            return resolve(self.on, app, &own);
        }
        resolve(self.on, app, args)
    }
    /// Its precondition: what must hold before anything is sent.
    pub fn check(&self, ctx: &Ctx) -> Result<(), Problem> {
        (self.precondition)(ctx)
    }
    /// What the command sends, once its precondition holds: the selection
    /// first, then the command's own.
    pub fn dispatch(&self, ctx: &Ctx) -> Result<Dispatch, Problem> {
        self.built(ctx)
    }
}

/// The messages that select what a command acts on, when it is not
/// selected yet.
fn select_first(on: On, ctx: &Ctx) -> Result<Vec<Message>, Problem> {
    let app = ctx.app;
    match on {
        On::Nothing => Ok(vec![]),
        On::Setup => {
            let Some(si) = ctx.setup else {
                return refused("There is no setup: add one first (add_setup).");
            };
            Ok(selecting(app, si, None))
        }
        On::Operation => {
            let Some((si, oi)) = ctx.operation else {
                return refused(
                    "No operation is selected: give `operation`, or select one (select_operation).",
                );
            };
            Ok(selecting(app, si, Some(oi)))
        }
    }
}

/// The setup a command acts on, and its operation: (setup, operation).
pub type Addressed = (Option<usize>, Option<(usize, usize)>);

/// The one place `setup` and `operation` arguments become indices, for every
/// command and read: an operation by index counts in the given `setup`, else
/// in the active one; an operation by id names its own setup (refused when
/// `setup` is also given and the id is not in it). Without `operation`, the
/// selected one, when its setup is the one addressed. Returns (setup,
/// operation); the setup is the operation's, else `setup`, else the active
/// one. `with_op` false leaves `operation` unread.
pub fn address(app: &App, args: &Value, with_op: bool) -> Result<Addressed, Problem> {
    let given = match args.get("setup") {
        Some(v) => Some(setup_of(app, v)?),
        None => None,
    };
    if let (true, Some(v)) = (with_op, args.get("operation")) {
        let (si, oi) = operation_of(app, given, v)?;
        return Ok((Some(si), Some((si, oi))));
    }
    let active = app.doc.setups.iter().position(|s| s.id == app.doc.active);
    let si = given.or(active);
    let op = si.zip(app.selected).filter(|&(si, oi)| {
        with_op
            && app.doc.setups[si].id == app.doc.active
            && oi < app.doc.setups[si].operations.len()
    });
    Ok((si, op))
}
/// Resolves what `on` addresses in `args` (`address`): the setup and
/// operation the command acts on, before its precondition runs.
pub fn resolve(on: On, app: &App, args: &Value) -> Result<Addressed, Problem> {
    match on {
        On::Nothing => Ok((None, None)),
        On::Setup => {
            let (si, _) = address(app, args, args.get("operation").is_some())?;
            Ok((si, None))
        }
        On::Operation => address(app, args, true),
    }
}
/// The messages that make setup `si` active and operation `oi` selected,
/// when they are not, for the app's handler to act on in the same turn.
fn selecting(app: &App, si: usize, oi: Option<usize>) -> Vec<Message> {
    let mut m = vec![];
    let other = app.doc.setups[si].id != app.doc.active;
    if other {
        m.push(Message::SelectSetup(si));
    }
    if let Some(oi) = oi
        && (other || app.selected != Some(oi))
    {
        m.push(Message::SelectOp(oi));
    }
    m
}

/// The `operation` and `setup` arguments of every operation-addressed
/// command, `list_fields` and `edit_field` (the one place they are written).
pub fn operation_selectors() -> Vec<(&'static str, Value, bool)> {
    vec![
        ("operation", addressed("operation"), false),
        ("setup", addressed("setup_of_operation"), false),
    ]
}

/// The schema of an argument that names a setup or an operation.
fn addressed(what: &str) -> Value {
    let description = match what {
        "setup_of_operation" => "The setup's id or index from 0: the setup the operation index counts in; default the active setup. An operation id names its own setup.".to_string(),
        _ => format!("The {what}'s id, or its index from 0 as inspect lists it."),
    };
    json!({"type": ["string", "integer"], "description": description})
}
fn text_param(description: &str) -> Value {
    json!({"type": "string", "description": description})
}
fn choice_param(values: &[&str]) -> Value {
    json!({"type": "string", "enum": values})
}
fn number_param(description: &str) -> Value {
    json!({"type": "number", "description": description})
}
fn integer_param(description: &str) -> Value {
    json!({"type": "integer", "description": description})
}
fn bool_param(description: &str) -> Value {
    json!({"type": "boolean", "description": description})
}

fn arg<'a>(ctx: &'a Ctx, key: &str) -> Result<&'a Value, Problem> {
    match ctx.args.get(key) {
        Some(v) => Ok(v),
        None => bad(format!("`{key}` is missing.")),
    }
}
fn string(ctx: &Ctx, key: &str) -> Result<String, Problem> {
    match arg(ctx, key)?.as_str() {
        Some(s) => Ok(s.to_string()),
        None => bad(format!("`{key}` must be a string.")),
    }
}
fn number(ctx: &Ctx, key: &str) -> Result<f64, Problem> {
    match arg(ctx, key)?.as_f64().filter(|v| v.is_finite()) {
        Some(n) => Ok(n),
        None => bad(format!("`{key}` must be a number.")),
    }
}
fn integer(ctx: &Ctx, key: &str) -> Result<i64, Problem> {
    match arg(ctx, key)?.as_i64() {
        Some(n) => Ok(n),
        None => bad(format!("`{key}` must be an integer.")),
    }
}
/// A move by `delta` places, as the clicks of the Up or Down button that
/// make it: one place each, for the app swaps an item with the one `delta`
/// away (a move of two would swap past the one between).
fn moves(ctx: &Ctx) -> Result<impl Iterator<Item = isize>, Problem> {
    let delta = integer(ctx, "delta")?;
    Ok(std::iter::repeat_n(
        delta.signum() as isize,
        delta.unsigned_abs() as usize,
    ))
}
/// Refused when a move by `delta` of item `from` of `n` would leave the
/// list (the app would do nothing). With nothing to move, the selection's
/// refusal says so.
fn moves_within(ctx: &Ctx, from: Option<usize>, n: usize, what: &str) -> Result<(), Problem> {
    let Some(from) = from else {
        return Ok(());
    };
    let delta = integer(ctx, "delta")?;
    let to = from as i64 + delta;
    if to < 0 || to >= n as i64 {
        return refused(format!(
            "The {what} is number {} of {n}: it cannot move {delta} places.",
            from + 1
        ));
    }
    Ok(())
}
fn index(ctx: &Ctx, key: &str) -> Result<usize, Problem> {
    match arg(ctx, key)?.as_u64() {
        Some(n) => Ok(n as usize),
        None => bad(format!("`{key}` must be an index from 0.")),
    }
}
fn flag(ctx: &Ctx, key: &str) -> bool {
    ctx.args.get(key).and_then(Value::as_bool).unwrap_or(false)
}
/// `key`'s value, one of `values` (any case), as the app spells it.
fn one_of(ctx: &Ctx, key: &str, values: &[&'static str]) -> Result<&'static str, Problem> {
    let given = string(ctx, key)?;
    match values.iter().find(|v| v.eq_ignore_ascii_case(given.trim())) {
        Some(v) => Ok(v),
        None => bad(format!("`{key}` must be one of: {}.", values.join(", "))),
    }
}

/// The setup `v` names: an id, or an index.
pub fn setup_of(app: &App, v: &Value) -> Result<usize, Problem> {
    let setups = &app.doc.setups;
    let found = match v {
        Value::String(id) => setups.iter().position(|s| s.id == *id),
        Value::Number(n) => n.as_u64().map(|i| i as usize).filter(|i| *i < setups.len()),
        _ => return bad("A setup is given by its id or its index."),
    };
    match found {
        Some(si) => Ok(si),
        None => refused(format!(
            "There is no setup {v}; the job has {} (inspect lists them).",
            setups.len()
        )),
    }
}

/// The operation `v` names, as (setup, operation) indices: an id in any
/// setup (or in `setup`), or an index in `setup` or the active setup.
pub fn operation_of(app: &App, setup: Option<usize>, v: &Value) -> Result<(usize, usize), Problem> {
    let setups = &app.doc.setups;
    let found = match v {
        Value::String(id) => (setups.iter().enumerate())
            .filter(|(si, _)| setup.is_none_or(|s| s == *si))
            .find_map(|(si, s)| Some((si, s.operations.iter().position(|o| o.id == *id)?))),
        Value::Number(n) => {
            let Some(si) = setup.or_else(|| setups.iter().position(|s| s.id == app.doc.active))
            else {
                return refused("There is no active setup: select a setup first.");
            };
            (n.as_u64().map(|i| i as usize))
                .filter(|i| *i < setups[si].operations.len())
                .map(|oi| (si, oi))
        }
        _ => return bad("An operation is given by its id or its index."),
    };
    match (found, v, setup) {
        (Some(found), ..) => Ok(found),
        (None, Value::String(id), Some(si))
            if setups
                .iter()
                .any(|s| s.operations.iter().any(|o| o.id == *id)) =>
        {
            refused(format!("Operation {id} is not in setup {}.", setups[si].id))
        }
        _ => refused(format!(
            "There is no operation {v} there (inspect lists them)."
        )),
    }
}

/// The strategy `name` names: as `inspect` writes it ("FiveAxis"), its
/// tile's word ("5-axis") or its full name ("Adaptive clearing").
pub fn strategy_of(name: &str) -> Result<Strategy, Problem> {
    let name = name.trim();
    let found = (Strategy::ALL.iter().copied()).find(|s| {
        [format!("{s:?}"), s.short().into(), s.name().into()]
            .iter()
            .any(|n| n.eq_ignore_ascii_case(name))
    });
    match found {
        Some(s) => Ok(s),
        None => bad(format!(
            "There is no strategy \"{name}\"; the strategies are {}.",
            strategy_names().join(", ")
        )),
    }
}
fn strategy_names() -> Vec<String> {
    Strategy::ALL.iter().map(|s| format!("{s:?}")).collect()
}

/// A library entry's id that must exist, from `key`.
fn library_id(ctx: &Ctx, key: &str, what: &str, ids: Vec<&str>) -> Result<String, Problem> {
    let id = string(ctx, key)?;
    if ids.contains(&id.as_str()) {
        Ok(id)
    } else {
        refused(format!(
            "There is no {what} {id} in the library (inspect {{what: \"libraries\"}} lists them)."
        ))
    }
}
fn machine_id(ctx: &Ctx, key: &str) -> Result<String, Problem> {
    let ids = ctx.app.doc.machines.iter().map(|m| m.id.as_str()).collect();
    library_id(ctx, key, "machine", ids)
}
fn tool_id(ctx: &Ctx, key: &str) -> Result<String, Problem> {
    let ids = ctx.app.doc.tools.iter().map(|t| t.id.as_str()).collect();
    library_id(ctx, key, "tool", ids)
}
fn material_id(ctx: &Ctx, key: &str) -> Result<String, Problem> {
    let ids = ctx
        .app
        .doc
        .materials
        .iter()
        .map(|m| m.id.as_str())
        .collect();
    library_id(ctx, key, "material", ids)
}
fn holder_id(ctx: &Ctx, key: &str) -> Result<String, Problem> {
    let ids = ctx
        .app
        .doc
        .workholding
        .iter()
        .map(|h| h.id.as_str())
        .collect();
    library_id(ctx, key, "workholding entry", ids)
}

/// What a project file may hold, for a save over it.
#[derive(PartialEq)]
enum FileHolds {
    /// Not there, or this project (its `project_id`, or none: saved before
    /// projects had ids).
    ThisOrNothing,
    /// A project with another id.
    Another,
    /// There, but cannot be read or parsed (or is too large to be one): it
    /// may hold another project.
    Unknown,
}
fn file_holds(path: &Path, doc: &crate::model::Document) -> FileHolds {
    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return FileHolds::ThisOrNothing,
        Err(_) => return FileHolds::Unknown,
    };
    if !meta.is_file() || meta.len() > 20_000_000 {
        return FileHolds::Unknown;
    }
    let Some(value) = std::fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .filter(Value::is_object)
    else {
        return FileHolds::Unknown;
    };
    let id = value
        .get("project_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    if !id.is_empty() && id != doc.project_id {
        FileHolds::Another
    } else {
        FileHolds::ThisOrNothing
    }
}

/// Whether the job has work not saved to a project file: changes since its
/// file was saved (the file not there, not readable, or not holding the
/// job by value: `App::saved_to_file`), or setups never saved to one.
fn unsaved(app: &App) -> bool {
    if app.project_path.is_none() {
        !app.doc.setups.is_empty()
    } else {
        !app.saved_to_file()
    }
}
/// The rule of the commands that replace the project: refused over unsaved
/// work unless `discard_unsaved` is true, so the agent cannot drop the
/// records the machinist made and never saved.
fn replaces_project(ctx: &Ctx) -> Result<(), Problem> {
    files_idle(ctx)?;
    if unsaved(ctx.app) && !flag(ctx, "discard_unsaved") {
        return refused(
            "The project has work not saved to a project file. Save it (save_project), or call again with discard_unsaved: true; the current project is kept in Project history either way.",
        );
    }
    Ok(())
}
fn discard_param() -> Vec<(&'static str, Value, bool)> {
    vec![(
        "discard_unsaved",
        bool_param("Replace the project although it has work not saved to a project file."),
        false,
    )]
}
/// No file action is under way, as the app's own buttons require.
fn files_idle(ctx: &Ctx) -> Result<(), Problem> {
    if ctx.app.import_busy || ctx.app.save_busy {
        refused("A file action is still under way (a load or a save): wait for it.")
    } else {
        Ok(())
    }
}

/// A path argument: absolute, `~` for the home folder, no `..`.
fn path_arg(ctx: &Ctx, key: &str) -> Result<PathBuf, Problem> {
    let text = string(ctx, key)?;
    let t = text.trim();
    let home = || {
        dirs::home_dir().ok_or(Problem::Refused(
            "The home folder is unknown: give the whole path.".into(),
        ))
    };
    let path = if t == "~" {
        home()?
    } else if let Some(rest) = t.strip_prefix("~/") {
        home()?.join(rest)
    } else {
        PathBuf::from(t)
    };
    if !path.is_absolute() {
        return refused(format!(
            "`{key}` must be an absolute path (or start with ~/): \"{t}\" is not."
        ));
    }
    if path.components().any(|c| c == Component::ParentDir) {
        return refused(format!(
            "`{key}` must not go up with \"..\": give the path itself."
        ));
    }
    Ok(path)
}
/// `path` with the part of it that exists resolved (links, `/var` and
/// `/private/var`), so two spellings of one place compare equal.
fn resolved(path: &Path) -> PathBuf {
    let mut existing = path.to_path_buf();
    let mut rest = vec![];
    while !existing.exists() {
        match (existing.file_name(), existing.parent()) {
            (Some(name), Some(parent)) => {
                rest.push(name.to_os_string());
                existing = parent.to_path_buf();
            }
            _ => return path.to_path_buf(),
        }
    }
    let mut p = existing.canonicalize().unwrap_or(existing);
    for name in rest.iter().rev() {
        p.push(name);
    }
    p
}
/// The app's data folder (`AXIS CAM Studio`): the workspace, its history,
/// settings, sessions and instance files.
fn data_folder() -> PathBuf {
    crate::storage::state_path()
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default()
}
/// Whether `target` (resolved) is inside the app's data folder or the
/// headless session's own folder: the files only the app and the server
/// write (the workspace, its history, settings, the instance files, every
/// session's workspace, `session.json`, lock and checkpoints).
fn in_app_files(ctx: &Ctx, target: &Path) -> bool {
    target.starts_with(resolved(&data_folder()))
        || ctx.session.is_some_and(|s| target.starts_with(resolved(s)))
}
/// Whether the command may write `path`: nowhere inside the app's data
/// folder, the headless session's own folder included (`in_app_files`),
/// not onto a library machine's UC100 profile, not onto a folder, and not
/// over a file that is there unless `overwrite` is true or it is the
/// project's own file (`own`). Every write a command makes goes through it,
/// the project's own file on a save without `path` too.
fn may_write(ctx: &Ctx, path: &Path, own: Option<&Path>) -> Result<(), Problem> {
    let target = resolved(path);
    if in_app_files(ctx, &target) {
        return refused(format!(
            "{} is inside the app's data folder, whose files only the app itself writes: choose a place outside it.",
            path.display()
        ));
    }
    let app = ctx.app;
    let machines = (app.doc.machines.iter())
        .chain(app.doc.setups.iter().map(|s| &s.machine))
        .chain([&app.machine_draft]);
    for m in machines {
        let profile = crate::model::get(&m.p, "uc100_profile").trim();
        if !profile.is_empty() && resolved(Path::new(profile)) == target {
            return refused(format!(
                "{} is the UC100 profile of the machine {}. {}",
                path.display(),
                m.name(),
                class::MACHINE
            ));
        }
    }
    if target.is_dir() {
        return refused(format!(
            "{} is a folder: give a file's path.",
            path.display()
        ));
    }
    let own = own.is_some_and(|o| resolved(o) == target);
    if target.exists() && !own && !flag(ctx, "overwrite") {
        return refused(format!(
            "{} is there already: call again with overwrite: true to replace it.",
            path.display()
        ));
    }
    Ok(())
}
fn overwrite_param() -> (&'static str, Value, bool) {
    (
        "overwrite",
        bool_param("Replace a file that is there already."),
        false,
    )
}

/// The tabs an operation editor has, over every strategy.
const TABS: [&str; 14] = [
    "Tool",
    "Geometry",
    "Heights",
    "Orientation",
    "Cycle",
    "Passes",
    "Linking",
    "Turn",
    "Drill",
    "Thread",
    "Box",
    "Slot",
    "Corners",
    "Groove",
];
/// The 3D view's named views.
const CAMERAS: [&str; 5] = ["ISO", "TOP", "FRONT", "RIGHT", "FIT"];
/// The pages the agent may open: not the Control page, whose entry moves
/// the manual steps' place, and not the planner's.
const PAGES: [(&str, Page); 9] = [
    ("Home", Page::Home),
    ("Work", Page::Work),
    ("Machines", Page::Machines),
    ("Tools", Page::Tools),
    ("Materials", Page::Materials),
    ("Workholding", Page::Workholding),
    ("Wizard", Page::Wizard),
    ("Post", Page::Post),
    ("Audit", Page::Audit),
];
const CONTROL_TABS: [(&str, ui::ControlTab); 8] = [
    ("job", ui::ControlTab::Job),
    ("tool_change", ui::ControlTab::ToolChange),
    ("start", ui::ControlTab::Start),
    ("work_zero", ui::ControlTab::WorkZero),
    ("probe", ui::ControlTab::Probe),
    ("rotary", ui::ControlTab::Rotary),
    ("mdi", ui::ControlTab::Mdi),
    ("machine", ui::ControlTab::Machine),
];
const EDIT_TARGETS: [(&str, Target); 6] = [
    ("operation", Target::Operation),
    ("setup", Target::Setup),
    ("machine", Target::Machine),
    ("tool", Target::Tool),
    ("material", Target::Material),
    ("holder", Target::Holder),
];
/// Every work zero the setup editor offers.
fn origins() -> Vec<&'static str> {
    let mut all: Vec<&'static str> = crate::post::ORIGINS.to_vec();
    all.extend(crate::indexed::ORIGINS);
    all.extend(crate::round_hold::ORIGINS);
    all.dedup();
    all
}
fn detail_names() -> Vec<String> {
    stock_surface::Detail::ALL
        .iter()
        .map(|d| d.to_string())
        .collect()
}
fn names<T>(table: &[(&'static str, T)]) -> Vec<&'static str> {
    table.iter().map(|(n, _)| *n).collect()
}

/// The commands that come with later steps of M1, which some messages are
/// classified to already: `class` names them, and their tests wait.
pub const LATER: [(&str, &str); 0] = [];

/// The user's actions an agent may not run, by name, so `list_commands`
/// shows them and says why.
pub const REFUSED: [(&str, &str, &str); 28] = [
    ("connect", "Connects to the machine.", class::MACHINE),
    (
        "disconnect",
        "Disconnects from the machine.",
        class::MACHINE,
    ),
    (
        "machine_reset",
        "Resets the controller out of E-stop.",
        class::MACHINE,
    ),
    (
        "switch_machine",
        "Chooses the machine the Control page connects to.",
        class::MACHINE,
    ),
    (
        "machine_connection",
        "Chooses USB, simulated or the Pi for the machine.",
        class::MACHINE,
    ),
    (
        "pi_pairing",
        "Pairs with, forgets or takes control of the machine's Pi.",
        class::MACHINE,
    ),
    (
        "add_rotary_machine",
        "Adds the CNC4030 with its rotary unit, writing a UC100 profile.",
        class::MACHINE,
    ),
    (
        "edit_uc100_profile",
        "Edits, saves or calibrates the machine's UC100 profile.",
        class::MACHINE,
    ),
    ("home", "Homes the machine.", class::MACHINE),
    (
        "machine_zero",
        "Declares machine zero or keeps the position.",
        class::MACHINE,
    ),
    ("jog", "Jogs an axis.", class::MACHINE),
    (
        "move_to",
        "Moves the machine to a position.",
        class::MACHINE,
    ),
    (
        "work_zero",
        "Sets work zero, a work offset or the work system.",
        class::MACHINE,
    ),
    (
        "spindle",
        "Turns the spindle on or off or sets its speed.",
        class::MACHINE,
    ),
    ("coolant", "Turns flood or mist on or off.", class::MACHINE),
    (
        "feed_override",
        "Changes the feed override.",
        class::MACHINE,
    ),
    (
        "override_limits",
        "Ignores the limit switches.",
        class::MACHINE,
    ),
    ("mdi", "Sends an MDI line.", class::MACHINE),
    (
        "test_output",
        "Forces an output pin for a wiring test.",
        class::MACHINE,
    ),
    (
        "probe",
        "Probes, touches off, calibrates or records the pins or the centreline.",
        class::MACHINE,
    ),
    (
        "run_on_machine",
        "Picks a setup to run on the machine.",
        class::MACHINE,
    ),
    ("cycle_start", "Starts or resumes the run.", class::MACHINE),
    (
        "run_progress",
        "Picks the next tool run, a start step or skips.",
        class::MACHINE,
    ),
    (
        "goto_control_page",
        "Opens the Control page, which enters the manual setups' steps.",
        class::MACHINE,
    ),
    (
        "steps_progress",
        "Moves the step guide's place (Next, Back, Go, Resume, Reached, Open).",
        class::SHOP_FLOOR,
    ),
    (
        "steps_measure",
        "Records a measurement or a dial reading in the step guide.",
        class::SHOP_FLOOR,
    ),
    (
        "planner",
        "The AI planner's plan, chat and sign-in.",
        class::PLANNER,
    ),
    (
        "pair_tablet",
        "Pairs a shop tablet with this computer, or lets tablets through Windows Firewall.",
        class::DEVICE,
    ),
];

/// The command named `name`.
pub fn find(name: &str) -> Option<&'static Command> {
    REGISTRY.iter().find(|c| c.name == name)
}
/// Every command, in the table's order.
pub fn all() -> &'static [Command] {
    &REGISTRY
}

static REGISTRY: LazyLock<Vec<Command>> = LazyLock::new(|| {
    let mut commands = table();
    for c in &mut commands {
        c.expect = c.expect.or(effect(c.name));
        c.unchanged = c.unchanged.or(already(c.name));
    }
    commands
});

#[allow(clippy::too_many_lines)]
fn table() -> Vec<Command> {
    vec![
        // Project.
        cmd("new_project", "project")
            .sends(|| Message::NewProject)
            .params(discard_param)
            .when(replaces_project)
            .requires("No unsaved work, or discard_unsaved: true.")
            .effect("A blank project with the libraries kept; the old one is in Project history."),
        cmd("open_sample", "project")
            .sends(|| Message::OpenSample)
            .params(discard_param)
            .when(replaces_project)
            .requires("No unsaved work, or discard_unsaved: true.")
            .effect("The example project; the old one is in Project history."),
        cmd("resume_project", "project")
            .sends(|| Message::ResumeProject)
            .when(files_idle)
            .effect("The Work page on the workspace's project, its part loaded again."),
        cmd("recover_previous_project", "project")
            .sends(|| Message::RecoverPrevious)
            .params(discard_param)
            .when(|ctx| {
                replaces_project(ctx)?;
                match ctx.app.last_archived {
                    Some(_) => Ok(()),
                    None => refused("No project was archived in this session to go back to."),
                }
            })
            .requires("A project archived in this session; no unsaved work, or discard_unsaved: true.")
            .effect("The project archived before the last switch."),
        cmd("open_project", "project")
            .help_of(|| Message::OpenProject, "")
            .params(|| {
                let mut p = vec![("path", text_param("The .axis.json project file."), true)];
                p.extend(discard_param());
                p
            })
            .examples(|| vec![json!({"path": "/tmp/job.axis.json"})])
            .when(replaces_project)
            .work(|ctx| {
                let path = path_arg(ctx, "path")?;
                if !path.is_file() {
                    return refused(format!("There is no project file at {}.", path.display()));
                }
                // The app's own files are not projects to open: an archive
                // in Project history may be, and saving it again then needs
                // a path outside (`save_project`).
                let target = resolved(&path);
                let history = [Some(data_folder()), ctx.session.map(Path::to_path_buf)]
                    .into_iter()
                    .flatten()
                    .any(|f| target.starts_with(resolved(&f.join("history"))));
                if in_app_files(ctx, &target) && !history {
                    return refused(format!(
                        "{} is one of the app's own files (its data folder), not a project an agent may open. Of that folder, only the archives in Project history (history/) open; save one elsewhere with save_project {{path}}.",
                        path.display()
                    ));
                }
                Ok((vec![], off_thread(move || crate::storage::load_reporting(&path).map(|(d, n)| (d, path, n)), |opened| Message::ProjectOpened(Some(opened)))))
            })
            .requires("An absolute path to a project file, not one of the app's own files (in its data folder only Project history's archives open); no unsaved work, or discard_unsaved: true.")
            .effect("The project opened, its part loaded; the old one is in Project history."),
        cmd("save_project", "project")
            .help_of(|| Message::SaveProject, "")
            .locked_ok()
            .params(|| {
                vec![
                    ("path", text_param("Where to save; without it, the project's own file."), false),
                    overwrite_param(),
                ]
            })
            .examples(|| vec![json!({"path": "/tmp/job.axis.json"}), json!({})])
            .when(files_idle)
            .with(|ctx| {
                let own = ctx.app.project_path.as_deref();
                if ctx.args.get("path").is_none() {
                    let Some(own) = own else {
                        return refused("The project has no file yet: give `path`.");
                    };
                    // Never over another project: the file may have been
                    // replaced since, or the project file named may not be
                    // this project's (a stale record).
                    match file_holds(own, &ctx.app.doc) {
                        FileHolds::ThisOrNothing => {}
                        FileHolds::Another => {
                            return refused(format!(
                                "The project file {} holds another project; save with a path.",
                                own.display()
                            ));
                        }
                        FileHolds::Unknown if !flag(ctx, "overwrite") => {
                            return refused(format!(
                                "The project file {} cannot be read, so it may hold another project; save with a path, or with overwrite: true.",
                                own.display()
                            ));
                        }
                        FileHolds::Unknown => {}
                    }
                    // The project's own file follows the same rules as a
                    // path given: one opened from the app's data folder
                    // (Project history) is saved elsewhere.
                    return match may_write(ctx, own, Some(own)) {
                        Ok(()) => Ok(vec![Message::SaveProject]),
                        Err(Problem::Refused(why)) => refused(format!(
                            "The project's own file cannot be saved over: {why} Give `path` to save the project elsewhere."
                        )),
                        Err(e) => Err(e),
                    };
                }
                let path = path_arg(ctx, "path")?;
                may_write(ctx, &path, own)?;
                // The project's own file, given by its path, needs no
                // overwrite: unless it holds another project now.
                if own.is_some_and(|o| resolved(o) == resolved(&path))
                    && !flag(ctx, "overwrite")
                    && file_holds(&path, &ctx.app.doc) != FileHolds::ThisOrNothing
                {
                    return refused(format!(
                        "The project file {} holds another project, or cannot be read and may: call again with overwrite: true to replace it.",
                        path.display()
                    ));
                }
                Ok(vec![Message::Saved(Some(path))])
            })
            .requires("A path, or a project that has its file; the path's rules, for the project's own file too (absolute, outside the app's data folder and the session's folder, not a UC100 profile, overwrite to replace).")
            .effect("The file is written and becomes the project's file; the project is saved."),
        cmd("set_project_title", "project")
            .text("Names the project: the window's title, its setup sheets and its programs' headers.")
            .params(|| vec![("title", text_param("The new title."), true)])
            .examples(|| vec![json!({"title": "Bracket"})])
            .with(|ctx| Ok(vec![Message::Title(string(ctx, "title")?)]))
            .unchanged(|ctx| {
                let title = ctx.args.get("title")?.as_str()?;
                (ctx.app.doc.title == title)
                    .then(|| format!("The project is already titled \"{title}\"."))
            })
            .expect(|ctx| {
                let title = ctx.args["title"].as_str().unwrap_or_default().to_string();
                Box::new(move |app| {
                    (app.doc.title == title)
                        .then_some(None)
                        .ok_or_else(|| "The title was not changed.".into())
                })
            })
            .effect("The project's title is `title`."),
        cmd("set_project_material", "project")
            .help_of(|| Message::PickMaterial(String::new()), "")
            .params(|| vec![("id", text_param("A library material's id."), true)])
            .examples(|| vec![json!({"id": "<material>"})])
            .with(|ctx| Ok(vec![Message::PickMaterial(material_id(ctx, "id")?)]))
            .requires("The material is in the library.")
            .effect("The project's material; each operation's speed and feed follow it."),
        cmd("goto_page", "project")
            .text("Opens a page: Home, Work, Machines, Tools, Materials, Workholding, Wizard, Post (NC review) or Audit. The Control page is the user's: its entry moves the manual steps' place.")
            .params(|| vec![("page", choice_param(&names(&PAGES)), true)])
            .examples(|| vec![json!({"page": "Post"})])
            .with(|ctx| {
                let given = string(ctx, "page")?;
                match PAGES.iter().find(|(n, _)| n.eq_ignore_ascii_case(given.trim())) {
                    Some((_, p)) => Ok(vec![Message::Page(*p)]),
                    None if given.eq_ignore_ascii_case("control") => Ok(vec![Message::Page(Page::Control)]),
                    None => bad(format!("`page` must be one of: {}.", names(&PAGES).join(", "))),
                }
            })
            .effect("The page shows."),
        // Part.
        cmd("import_model", "part")
            .help_of(|| Message::Import, "")
            .params(|| vec![("path", text_param("A STEP/STP, 3MF, STL, OBJ or DXF file."), true)])
            .examples(|| vec![json!({"path": "/tmp/part.step"})])
            .when(files_idle)
            .with(|ctx| {
                let path = path_arg(ctx, "path")?;
                if !path.is_file() {
                    return refused(format!("There is no file at {}.", path.display()));
                }
                Ok(vec![
                    Message::ImportPath(path.display().to_string()),
                    Message::ImportFromPath,
                ])
            })
            .requires("An absolute path to a model file.")
            .effect("The part is loaded into the project."),
        cmd("reimport_model", "part")
            .sends(|| Message::Reimport)
            .when(|ctx| {
                files_idle(ctx)?;
                match ctx.app.doc.file {
                    Some(_) => Ok(()),
                    None => refused("The project has no model file to load again: import one."),
                }
            })
            .requires("A project with a model file.")
            .effect("The part loaded again, with the setups, operations and picks kept."),
        cmd("dismiss_pick_note", "part")
            .sends(|| Message::PicksChecked)
            .on_operation()
            .when(|ctx| match ctx.op() {
                Some(o) if ctx.app.doc.pick_checks.contains_key(&o.id) => Ok(()),
                Some(_) => refused("This operation has no note about its picks."),
                None => Ok(()),
            })
            .effect("The note about the operation's picks after a model change goes."),
        // Setup.
        cmd("select_setup", "setup")
            .help_of(|| Message::SelectSetup(0), "")
            .params(|| vec![("setup", addressed("setup"), true)])
            .examples(|| vec![json!({"setup": 0})])
            .with(|ctx| Ok(vec![Message::SelectSetup(setup_of(ctx.app, arg(ctx, "setup")?)?)]))
            .unchanged(|ctx| {
                let si = setup_of(ctx.app, ctx.args.get("setup")?).ok()?;
                let s = &ctx.app.doc.setups[si];
                (ctx.app.doc.active == s.id && ctx.app.selected.is_none() && ctx.app.page == Page::Work)
                    .then(|| format!("{} is already the active setup.", s.name()))
            })
            .expect(|ctx| {
                let si = setup_of(ctx.app, &ctx.args["setup"]).ok();
                let id = si.map(|si| ctx.app.doc.setups[si].id.clone()).unwrap_or_default();
                Box::new(move |app| {
                    (app.doc.active == id)
                        .then_some(None)
                        .ok_or_else(|| "The setup was not made active.".into())
                })
            })
            .requires("The setup exists.")
            .effect("The setup is the active one, with no operation selected, on the Work page."),
        cmd("add_setup", "setup")
            .help_of(|| Message::AddSetup, "")
            .params(|| vec![("machine", text_param("A library machine's id; without it, the first CNC."), false)])
            .examples(|| vec![json!({}), json!({"machine": "<machine>"})])
            .with(|ctx| {
                Ok(vec![match ctx.args.get("machine") {
                    Some(_) => Message::AddSetupOn(machine_id(ctx, "machine")?),
                    None => Message::AddSetup,
                }])
            })
            .effect("A new setup, active."),
        cmd("add_manual_setup", "setup").sends(|| Message::AddManualSetup).effect("A manual mill setup before the CNC setups."),
        cmd("add_lathe_setup", "setup").sends(|| Message::AddLatheSetup).effect("A lathe setup before the CNC setups."),
        cmd("duplicate_setup", "setup").sends(|| Message::DuplicateSetup).on_setup().effect("A copy of the setup and its operations."),
        cmd("remove_setup", "setup")
            .help_of(|| Message::RemoveSetup, "")
            .on_setup()
            .with(|_| Ok(vec![Message::RemoveSetup, Message::ConfirmDeleteSetup(true)]))
            .effect("The setup and its operations are removed (the editor's question is answered yes)."),
        cmd("move_setup", "setup")
            .help_of(|| Message::MoveSetup(1), "")
            .on_setup()
            .params(|| vec![("delta", integer_param("Places to move: negative earlier, positive later."), true)])
            .examples(|| vec![json!({"delta": 1})])
            .when(|ctx| moves_within(ctx, ctx.setup, ctx.app.doc.setups.len(), "setup"))
            .with(|ctx| Ok(moves(ctx)?.map(Message::MoveSetup).collect()))
            .effect("The setup moves in the job's order."),
        cmd("flip_setup", "setup")
            .help_of(|| Message::Flip("rx"), "")
            .on_setup()
            .params(|| vec![("axis", choice_param(&["rx", "ry"]), true)])
            .examples(|| vec![json!({"axis": "rx"})])
            .with(|ctx| Ok(vec![Message::Flip(one_of(ctx, "axis", &["rx", "ry"])?)]))
            .effect("rx turns the part 180° about X; ry 90° about Y."),
        cmd("turn_stock", "setup")
            .help_of(|| Message::TurnStock(1, 0), "")
            .on_setup()
            .params(|| {
                let faces: Vec<&str> = crate::volume::FACES.iter().map(|(n, _)| *n).collect();
                vec![
                    ("face", json!({"type": "integer", "minimum": 0, "maximum": 5, "description": format!("The stock's face up, by index: {}.", faces.join(", "))}), true),
                    ("table_turn", json!({"type": "integer", "minimum": 0, "maximum": 3, "description": "Quarter turns on the table: 0°, 90°, 180°, 270°."}), true),
                ]
            })
            .examples(|| vec![json!({"face": 1, "table_turn": 0})])
            .with(|ctx| {
                let (face, turn) = (index(ctx, "face")?, index(ctx, "table_turn")?);
                if face >= crate::volume::FACES.len() || turn >= crate::volume::TABLE_TURNS.len() {
                    return bad("`face` is 0 to 5 and `table_turn` 0 to 3.");
                }
                Ok(vec![Message::TurnStock(face, turn)])
            })
            .effect("The setup holds the stock with that face up and turn, square to setup 1."),
        cmd("square_to_stock", "setup").sends(|| Message::SquareToStock).on_setup().effect("The setup's rotation snaps to the stock's nearest quarter turn."),
        cmd("assign_machine", "setup")
            .help_of(|| Message::AssignMachine(String::new()), "")
            .on_setup()
            .params(|| vec![("id", text_param("A library machine's id."), true)])
            .examples(|| vec![json!({"id": "<machine>"})])
            .with(|ctx| Ok(vec![Message::AssignMachine(machine_id(ctx, "id")?)]))
            .effect("The setup runs on a copy of that machine."),
        cmd("set_origin", "setup")
            .help_of(|| Message::Edit(Target::Setup, "origin", crate::turn::ZERO.into()), "")
            .on_setup()
            .params(|| vec![("origin", choice_param(&origins()), true)])
            .examples(|| vec![json!({"origin": "Stock top center"})])
            .with(|ctx| {
                let origin = one_of(ctx, "origin", &origins())?;
                Ok(vec![Message::Edit(Target::Setup, "origin", origin.into())])
            })
            .effect("The setup's work zero."),
        cmd("set_setup_step", "setup")
            .help_of(|| Message::SetupStep("Machine"), "")
            .on_setup()
            .params(|| vec![("step", choice_param(&ui::SETUP_STEPS), true)])
            .examples(|| vec![json!({"step": "Stock"})])
            .with(|ctx| Ok(vec![Message::SetupStep(one_of(ctx, "step", &ui::SETUP_STEPS)?)]))
            .effect("The setup editor opens on that step."),
        cmd("set_clamps", "setup")
            .text("Sets the active setup's clamp positions field, as typed in the setup editor's Holding step.")
            .on_setup()
            .params(|| vec![("text", text_param("The clamp positions, as the field takes them."), true)])
            .examples(|| vec![json!({"text": ""})])
            .with(|ctx| Ok(vec![Message::SetClamps(string(ctx, "text")?)]))
            .effect("The clamp positions are the setup's when they read; otherwise the status says why."),
        cmd("suggest_layout", "setup").sends(|| Message::SuggestLayout).on_setup().effect("The vise jaws or clamps are placed clear of the toolpaths, or the status says why not."),
        cmd("show_stock_after_setup", "setup")
            .text("Shows the stock the active setup's operations leave, in the 3D view.")
            .sends(|| Message::SetupEnd)
            .on_setup()
            .effect("The 3D view shows the stock after the setup."),
        cmd("open_setup_editor", "setup").sends(|| Message::SetupEditor).on_setup().effect("The setup editor opens."),
        // Operation.
        cmd("select_operation", "operation")
            .help_of(|| Message::SelectOp(0), "")
            .params(|| {
                vec![
                    ("operation", addressed("operation"), true),
                    ("setup", addressed("setup"), false),
                ]
            })
            .examples(|| vec![json!({"operation": 0})])
            .with(|ctx| {
                arg(ctx, "operation")?;
                let (si, oi) = selected_operation_arg(ctx).ok_or_else(|| Problem::Refused("There is no such operation.".into()))?;
                let mut m = selecting(ctx.app, si, Some(oi));
                if m.last().is_none_or(|m| !matches!(m, Message::SelectOp(_))) {
                    m.push(Message::SelectOp(oi));
                }
                Ok(m)
            })
            .unchanged(|ctx| {
                let (si, oi) = selected_operation_arg(ctx)?;
                let s = &ctx.app.doc.setups[si];
                (ctx.app.doc.active == s.id && ctx.app.selected == Some(oi) && ctx.app.page == Page::Work)
                    .then(|| format!("{} is already selected.", s.operations[oi].name()))
            })
            .expect(|ctx| {
                let id = selected_operation_arg(ctx)
                    .map(|(si, oi)| ctx.app.doc.setups[si].operations[oi].id.clone())
                    .unwrap_or_default();
                Box::new(move |app| {
                    (app.operation().is_some_and(|o| o.id == id))
                        .then_some(None)
                        .ok_or_else(|| "The operation was not selected.".into())
                })
            })
            .requires("The operation exists. An index counts in `setup`, else in the active setup; an id may be in any setup, which is made active.")
            .effect("The operation is selected and its editor open on the Work page."),
        cmd("add_operation", "operation")
            .text("Adds an operation of a strategy to the active setup, with the library's first tool that fits. With no fitting tool the app opens the Tool library to make one, and the operation is not added.")
            .on_setup()
            .params(|| vec![("strategy", json!({"type": "string", "enum": strategy_names()}), true)])
            .examples(|| vec![json!({"strategy": "Pocket"})])
            .when(|ctx| {
                let strategy = strategy_of(&string(ctx, "strategy")?)?;
                if !ctx.app.is_sample() && !strategy.imported_preview() {
                    return refused(format!(
                        "{} cannot be added to an imported part: {}",
                        strategy.name(),
                        strategy.availability()
                    ));
                }
                Ok(())
            })
            .with(|ctx| Ok(vec![Message::AddOp(strategy_of(&string(ctx, "strategy")?)?)]))
            .expect(|ctx| {
                let strategy = strategy_of(ctx.args["strategy"].as_str().unwrap_or_default()).ok();
                let setup = ctx.setup.map(|si| &ctx.app.doc.setups[si]);
                let id = setup.map(|s| s.id.clone()).unwrap_or_default();
                let before: Vec<String> = (setup.iter().flat_map(|s| &s.operations))
                    .map(|o| o.id.clone())
                    .collect();
                Box::new(move |app| {
                    let setup = app.doc.setups.iter().find(|s| s.id == id);
                    let added = setup.is_some_and(|s| s.operations.len() > before.len());
                    let selected = app.operation().is_some_and(|o| {
                        Some(o.strategy) == strategy && !before.contains(&o.id)
                    });
                    (added && selected && app.doc.active == id)
                        .then_some(None)
                        .ok_or_else(|| "No operation was added.".into())
                })
            })
            .requires("An active setup; on an imported part, a strategy available for it.")
            .effect("The setup has a new operation (and its centre drilling, for a drill on the manual mill), selected."),
        cmd("add_turn", "operation")
            .help_of(|| Message::AddTurn("Outside"), "")
            .on_setup()
            .params(|| vec![("kind", choice_param(&crate::lathe::TURN_KINDS), true)])
            .examples(|| vec![json!({"kind": "Outside"})])
            .with(|ctx| Ok(vec![Message::AddTurn(one_of(ctx, "kind", &crate::lathe::TURN_KINDS)?)]))
            .requires("A lathe setup: the active one, or `setup`.")
            .effect("The lathe setup has a new Turn of that kind, selected."),
        cmd("duplicate_operation", "operation").sends(|| Message::DuplicateOp).on_operation().effect("A copy of the operation after it."),
        cmd("remove_operation", "operation")
            .sends(|| Message::RemoveOp)
            .on_operation()
            .effect("The operation is removed, with the centre drilling and what a Thread brought."),
        cmd("move_operation", "operation")
            .help_of(|| Message::MoveOp(1), "")
            .on_operation()
            .params(|| vec![("delta", integer_param("Places to move: negative earlier, positive later."), true)])
            .examples(|| vec![json!({"delta": 1})])
            .when(|ctx| {
                let n = (ctx.operation)
                    .and_then(|(si, _)| ctx.app.doc.setups.get(si))
                    .map_or(0, |s| s.operations.len());
                moves_within(ctx, ctx.operation.map(|(_, oi)| oi), n, "operation")
            })
            .with(|ctx| Ok(moves(ctx)?.map(Message::MoveOp).collect()))
            .effect("The operation moves in its setup's order."),
        cmd("toggle_operation", "operation").sends(|| Message::ToggleOp).on_operation().effect("The operation is excluded from, or included in, generation and review."),
        cmd("set_operation_tool", "operation")
            .help_of(|| Message::SetOpTool(String::new()), "")
            .on_operation()
            .params(|| vec![("id", text_param("A library tool's id."), true)])
            .examples(|| vec![json!({"id": "<tool>"})])
            .with(|ctx| Ok(vec![Message::SetOpTool(tool_id(ctx, "id")?)]))
            .effect("The operation cuts with that tool."),
        cmd("recommend_speeds", "operation").sends(|| Message::RecommendSpeeds).on_operation().when(|ctx| match ctx.app.doc.selected_material() {
            Some(_) => Ok(()),
            None => refused("The project has no material: choose one first (set_project_material)."),
        }).effect("The operation's speed and feed from the project material's cutting data."),
        cmd("set_rest_source", "operation")
            .help_of(|| Message::RestSource(String::new()), "")
            .on_operation()
            .params(|| vec![("source", text_param("The id of the earlier operation whose leftover guides it, or \"\" for none."), true)])
            .examples(|| vec![json!({"source": ""})])
            .with(|ctx| {
                let source = string(ctx, "source")?;
                let known = source.is_empty()
                    || (ctx.app.doc.setups.iter().flat_map(|s| &s.operations)).any(|o| o.id == source);
                if !known {
                    return refused(format!("There is no operation {source}."));
                }
                Ok(vec![Message::RestSource(source)])
            })
            .effect("The operation's rest machining source."),
        cmd("set_pin_setup", "operation")
            .help_of(|| Message::Edit(Target::Operation, "pin_setup", String::new()), "")
            .on_operation()
            .own_setup()
            .params(|| vec![("setup", addressed("setup"), true)])
            .examples(|| vec![json!({"setup": 0})])
            .with(|ctx| {
                let si = setup_of(ctx.app, arg(ctx, "setup")?)?;
                Ok(vec![Message::Edit(Target::Operation, "pin_setup", ctx.app.doc.setups[si].id.clone())])
            })
            .effect("The operation makes the holes for that setup's fixture pins."),
        cmd("set_tab", "operation")
            .text("Opens a tab of the selected operation's editor: Tool, Geometry, Heights, Passes, Linking and the strategy's own.")
            .on_operation()
            .params(|| vec![("tab", choice_param(&TABS), true)])
            .examples(|| vec![json!({"tab": "Heights"})])
            .when(|ctx| {
                let tab = one_of(ctx, "tab", &TABS)?;
                let machine = ctx.operation.map(|(si, _)| &ctx.app.doc.setups[si].machine);
                match (ctx.op(), machine) {
                    (Some(o), Some(m)) if !o.tabs(m).contains(&tab) => refused(format!(
                        "This operation has no {tab} tab; its tabs are {}.",
                        o.tabs(m).join(", ")
                    )),
                    _ => Ok(()),
                }
            })
            .with(|ctx| Ok(vec![Message::Tab(one_of(ctx, "tab", &TABS)?.into())]))
            .effect("The tab is open."),
        cmd("add_threading", "operation").sends(|| Message::AddThreading).on_setup().effect("A Thread operation for each modelled thread the setup reaches, with what it needs before it."),
        cmd("add_needed_tool", "operation").sends(|| Message::AddNeededTool).on_operation().effect("The tool the threading operation needs, in the library and given to it."),
        cmd("add_hole_stages", "operation").sends(|| Message::AddHoleStages).on_operation().effect("The drilling (and roughing) before the Reaming or Boring."),
        cmd("add_centring", "operation")
            .sends(|| Message::AddCentring)
            .on_operation()
            // Offered as the app offers it: on a manual-mill twist drill
            // drilling from solid, not yet centred.
            .when(|ctx| {
                let (Some((si, oi)), Some(o)) = (ctx.operation, ctx.op()) else {
                    return Ok(());
                };
                let setup = &ctx.app.doc.setups[si];
                if !crate::centre::wants_centre(o, &ctx.app.doc.tools, &setup.machine) {
                    return refused("A centre drilling goes before a Drill on the manual mill, with a twist drill drilling from solid.");
                }
                if crate::centre::centred(setup, oi) {
                    return refused("This drill is centre drilled already.");
                }
                Ok(())
            })
            .requires("A manual-mill Drill with a twist drill, not yet centre drilled.")
            .effect("A centre drilling before the manual-mill drill."),
        cmd("add_pin_drilling", "operation").sends(|| Message::AddPinDrilling).on_setup().effect("A Drill of the pin holes in the earlier setup."),
        cmd("thread_from_part", "operation").sends(|| Message::ThreadFromPart).on_operation().effect("The Turn thread's side, diameter, pitch and end from the part."),
        cmd("box_from_face", "operation").sends(|| Message::BoxFromFace).on_operation().effect("The Box's six fields from its picked face."),
        cmd("pick_all_corners", "operation").sends(|| Message::AllCorners).on_operation().effect("Every rounded outside corner picked for Round corners."),
        cmd("edit_field", "operation")
            .text("Sets one field of an operation, a setup, or the machine, tool, material or holder draft, as typing in the editor does. The key must be one `list_fields` gives for the target as it is now, and a pick list's value one of its choices. The machine's controller fields (uc100_profile, uc100_connection, pi_*) and a lathe's cross-slide dial reading (cross_reads) are the user's.")
            .params(|| {
                vec![
                    ("target", choice_param(&names(&EDIT_TARGETS)), true),
                    ("key", text_param("The field's key, one `list_fields` gives for the target."), true),
                    ("value", json!({"type": ["string", "number", "boolean"], "description": "The value, as typed."}), true),
                ]
                .into_iter()
                .chain(operation_selectors())
                .collect()
            })
            .examples(|| vec![json!({"target": "operation", "key": "rpm", "value": "8000"})])
            // The machine's controller fields and the dial reading are the
            // user's; any other key must be one the editor draws now, and a
            // choice one of its choices (`list_fields`, `fields::check`).
            .when(|ctx| {
                let key = string(ctx, "key")?;
                if class::machine_key(&key) {
                    return refused(format!("The field {key}: {}", class::MACHINE));
                }
                // The lathe's cross-slide dial reading is the machinist's
                // record (`state::Records`).
                if key == "cross_reads" {
                    return refused(format!("The field {key}: {}", class::SHOP_FLOOR));
                }
                let name = one_of(ctx, "target", &names(&EDIT_TARGETS))?;
                let target = (EDIT_TARGETS.iter())
                    .find(|(n, _)| *n == name)
                    .map_or(Target::Operation, |(_, t)| *t);
                let value = match arg(ctx, "value")? {
                    Value::String(s) => s.clone(),
                    Value::Number(n) => n.to_string(),
                    Value::Bool(b) => b.to_string(),
                    _ => return bad("`value` is a string, a number or true/false."),
                };
                fields::check(ctx.listed()?, target, &key, &value).or_else(refused)
            })
            .with(|ctx| {
                let name = one_of(ctx, "target", &names(&EDIT_TARGETS))?;
                let target = (EDIT_TARGETS.iter())
                    .find(|(n, _)| *n == name)
                    .map_or(Target::Operation, |(_, t)| *t);
                let key = string(ctx, "key")?;
                let Some(field) = ctx.listed()?.iter().find(|f| f.key == key) else {
                    return refused(format!("There is no field {key}."));
                };
                let key = field.key;
                let value = match arg(ctx, "value")? {
                    Value::String(s) => s.clone(),
                    Value::Number(n) => n.to_string(),
                    Value::Bool(b) => b.to_string(),
                    _ => return bad("`value` is a string, a number or true/false."),
                };
                let mut m = match target {
                    Target::Operation => {
                        let (_, Some((si, oi))) = address(ctx.app, ctx.args, true)? else {
                            return refused("No operation is selected: give `operation`.");
                        };
                        selecting(ctx.app, si, Some(oi))
                    }
                    Target::Setup => {
                        let (Some(si), _) = address(ctx.app, ctx.args, false)? else {
                            return refused("There is no setup: add one first.");
                        };
                        selecting(ctx.app, si, None)
                    }
                    Target::Machine | Target::Tool | Target::Material | Target::Holder => vec![],
                };
                m.push(Message::Edit(target, key, value));
                Ok(m)
            })
            .requires("The operation or setup (selected, or given); for a library target, its draft (new_* or edit_* opens it).")
            .effect("The field holds the value."),
        cmd("undo_last_edit", "operation")
            .text("Undoes the last field edit, while nothing else changed since.")
            .sends(|| Message::PutBackEdit)
            .when(|ctx| {
                if ctx.app.can_put_back() {
                    Ok(())
                } else {
                    refused("There is no edit to undo, or the job changed since it.")
                }
            })
            .effect("The last edit is undone."),
        // Picks.
        cmd("clear_picks", "picks").sends(|| Message::ClearSurfaces).on_operation().effect("The operation's picked surfaces are cleared."),
        cmd("drop_hole", "picks")
            .help_of(|| Message::DropHole(0), "")
            .on_operation()
            .params(|| vec![("index", integer_param("The hole's index in the operation's Geometry list."), true)])
            .examples(|| vec![json!({"index": 0})])
            .with(|ctx| Ok(vec![Message::DropHole(index(ctx, "index")?)]))
            .effect("The hole is dropped from the Pocket or Contour."),
        cmd("pick_surfaces", "picks")
            .text("Picks the part's surfaces for an operation as clicks on the part do: one click per id, each checked for what it did. A click takes a whole group, as the operation groups faces: a hole's walls (Pocket, Adaptive clearing, Contour, Drill), a rounded corner (Round corners), a whole smooth face (Parallel, Waterline), or the one face. mode add (the default) skips an id already picked, and undoes in full a click that would drop a group already picked (two ids of one corner pick it once); remove drops each picked id's group; replace clears the picks, then adds. A Slot, a Box or a lathe cut takes one face, which fills its fields: send one id with mode replace (clear_picks clears it). Picking is turned on for the clicks and the tab put back after. The result's `picks` are the operation's picks afterwards (ids, the holes, corners or faces they make, the part's fingerprint) and `clicks` what each id did.")
            .on_operation()
            .params(|| vec![("ids", ids_param("The part's surface ids to click, in order."), true), ("mode", choice_param(&Mode::NAMES), false)])
            .examples(|| vec![json!({"ids": [0], "mode": "add"})])
            .when(|ctx| {
                let (ids, mode) = (ids_arg(ctx)?, mode_arg(ctx)?);
                let app = ctx.app;
                let Some(o) = ctx.op() else {
                    return refused("No operation is selected: give `operation`, or select one (select_operation).");
                };
                if app.is_sample() {
                    return refused("The example project's part has features, not surfaces: pick them with pick_features.");
                }
                let Some(mesh) = app.imported_mesh.as_ref().filter(|_| app.doc.file.is_some()) else {
                    return refused("No part is imported: import one first (import_model).");
                };
                if let Some(n) = ids.iter().find(|&&n| n >= mesh.surfaces.len()) {
                    return refused(format!("There is no surface {n}: the part's surfaces are 0 to {}.", mesh.surfaces.len().saturating_sub(1)));
                }
                if !app.can_pick_op(o) {
                    let name = crate::threading::shown_name(o, &app.doc.tools);
                    let bounded = matches!(o.strategy, Strategy::Contour | Strategy::Parallel | Strategy::Waterline);
                    return refused(if bounded {
                        format!("{name} cuts the whole part: set its boundary to Selected geometry (edit_field) to pick faces.")
                    } else {
                        format!("{name} takes nothing picked on the part.")
                    });
                }
                if app.pick_kind(o) == crate::PickKind::Fill && (mode != Mode::Replace || ids.len() != 1) {
                    return refused(format!("{:?} takes one face, which fills it; send one id with mode replace, or clear_picks.", o.strategy));
                }
                Ok(())
            })
            .clicks(|ctx| {
                Ok(pick::Job::Surfaces { op: op_id(ctx), ids: ids_arg(ctx)?, mode: mode_arg(ctx)? })
            })
            .requires("An operation (selected, or `operation`) that takes picks on an imported part.")
            .effect("Each id clicked and checked; the operation's picks afterwards, and what each id did."),
        cmd("pick_features", "picks")
            .text("Picks the example project's features (model::FEATURES: 0 Top rim face, 1 Pocket floor, 2 Pocket walls, 3 Outer profile, 4 Center boss, 5 to 8 the Ø8 holes) for an operation, as clicks on the example part do, each checked. mode add (the default) skips a feature picked already, remove drops it, replace drops the others first. Only the features the operation's strategy takes are accepted. An imported part's surfaces are picked with pick_surfaces.")
            .on_operation()
            .params(|| vec![("ids", ids_param("The features to click, by number."), true), ("mode", choice_param(&Mode::NAMES), false)])
            .examples(|| vec![json!({"ids": [1], "mode": "add"})])
            .when(|ctx| {
                let ids = ids_arg(ctx)?;
                mode_arg(ctx)?;
                let Some(o) = ctx.op() else {
                    return refused("No operation is selected: give `operation`, or select one (select_operation).");
                };
                if !ctx.app.is_sample() {
                    return refused("Features are the example project's; an imported part's surfaces are picked with pick_surfaces.");
                }
                let takes = o.strategy.features();
                if let Some(n) = ids.iter().find(|n| !takes.contains(n)) {
                    let named: Vec<String> = (takes.iter())
                        .map(|&f| format!("{f} ({})", crate::model::FEATURES[f]))
                        .collect();
                    return refused(if named.is_empty() {
                        format!("{:?} takes no features.", o.strategy)
                    } else {
                        format!("{:?} does not take feature {n}; it takes {}.", o.strategy, named.join(", "))
                    });
                }
                Ok(())
            })
            .clicks(|ctx| {
                Ok(pick::Job::Features { op: op_id(ctx), ids: ids_arg(ctx)?, mode: mode_arg(ctx)? })
            })
            .requires("An operation of the example project (selected, or `operation`).")
            .effect("Each feature clicked and checked; the operation's features afterwards."),
        cmd("pick_pin_holes", "picks")
            .text("Chooses the part's holes that drop over the fixture pins, for a setup that sits on the pins by the part's own holes (its Work zero step): one click per id, a surface inside the hole (its wall or its end), as Pick holes on the part does in the 3D view. mode add (the default) leaves a hole chosen already as it is, remove takes each id's hole off, replace clears the holes first. The result lists the chosen holes (in the part's own frame) and what each id did.")
            .on_setup()
            .params(|| vec![("ids", ids_param("The part's surface ids, each inside a hole that opens on the bottom in this setup."), true), ("mode", choice_param(&Mode::NAMES), false)])
            .examples(|| vec![json!({"ids": [0], "mode": "add"})])
            .when(|ctx| {
                let ids = ids_arg(ctx)?;
                mode_arg(ctx)?;
                pins_setup(ctx, false)?;
                let mesh = ctx.app.imported_mesh.as_ref().map_or(0, |m| m.surfaces.len());
                if let Some(n) = ids.iter().find(|&&n| n >= mesh) {
                    return refused(format!("There is no surface {n}: the part's surfaces are 0 to {}.", mesh.saturating_sub(1)));
                }
                Ok(())
            })
            .clicks(|ctx| {
                Ok(pick::Job::PinHoles { setup: setup_id(ctx), ids: ids_arg(ctx)?, mode: mode_arg(ctx)? })
            })
            .requires("A setup that sits on the pins by the part's holes, on an imported part.")
            .effect("Each id clicked and checked; the holes chosen for the pins afterwards."),
        cmd("clear_pin_holes", "picks")
            .text("Clears the holes chosen for the fixture pins (the Work zero step's Clear the pin holes).")
            .on_setup()
            .when(|ctx| pins_setup(ctx, false).map(|_| ()))
            .with(|_| Ok(vec![Message::Edit(Target::Setup, "pin_holes", String::new())]))
            .requires("A setup that sits on the pins by the part's holes.")
            .effect("No hole is chosen for the pins."),
        cmd("place_pins", "picks")
            .text("Places the fixture pins in the stock's leftover, for a setup that sits on pins in its leftover (its Work zero step): pin 1 where the click is, at x, y in the setup's plan (mm, as the part is placed), the plate putting the others, as Place the pins on the leftover and a click in the 3D view do. Where they do not fit they stay where they were, and the reason is given.")
            .on_setup()
            .params(|| vec![("x", number_param("Pin 1's X in the setup's plan, mm."), true), ("y", number_param("Pin 1's Y in the setup's plan, mm."), true)])
            .examples(|| vec![json!({"x": 0, "y": 0})])
            .when(|ctx| {
                number(ctx, "x")?;
                number(ctx, "y")?;
                let si = pins_setup(ctx, true)?;
                if ctx.app.known_stock(si).0.is_none() {
                    return refused("The stock this setup starts from is not known yet: set its stock first.");
                }
                Ok(())
            })
            .clicks(|ctx| {
                let at = [number(ctx, "x")? as f32, number(ctx, "y")? as f32];
                Ok(pick::Job::Place { setup: setup_id(ctx), at })
            })
            .requires("A setup that sits on pins in the stock's leftover, on an imported part whose stock is known.")
            .effect("Pin 1 stands there, or the reason it does not fit."),
        cmd("clear_pin_place", "picks")
            .text("Clears where the fixture pins stand in the stock's leftover (the Work zero step's Clear the pins' place).")
            .on_setup()
            .when(|ctx| pins_setup(ctx, true).map(|_| ()))
            .with(|_| Ok(vec![Message::Edit(Target::Setup, "pin_place", String::new())]))
            .requires("A setup that sits on pins in the stock's leftover.")
            .effect("The pins have no place."),
        // Generate.
        cmd("generate", "generate").sends(|| Message::Generate).on_operation().effect("The operation's simulation, or the reason it could not be generated."),
        cmd("generate_all", "generate").sends(|| Message::GenerateAll).when(|ctx| match ctx.app.doc.active_setup() {
            Some(_) => Ok(()),
            None => refused("There is no setup to generate."),
        }).effect("Every enabled operation generated and checked; the audit lists them."),
        cmd("cancel_generation", "generate")
            .text("Cancels the generation under way.")
            .sends(|| Message::CancelGenerate)
            .when(|ctx| match ctx.app.generating {
                Some(_) => Ok(()),
                None => refused("No generation is under way."),
            })
            .effect("The generation stops."),
        // Libraries: machines.
        cmd("new_machine", "libraries").sends(|| Message::NewMachine).effect("The machine wizard on a new draft."),
        cmd("edit_machine", "libraries")
            .help_of(|| Message::EditMachine(String::new()), "")
            .params(|| vec![("id", text_param("A library machine's id."), true)])
            .examples(|| vec![json!({"id": "<machine>"})])
            .with(|ctx| Ok(vec![Message::EditMachine(machine_id(ctx, "id")?)]))
            .effect("The machine wizard on that machine."),
        cmd("machine_wizard_step", "libraries")
            .help_of(|| Message::WizardStep(0), "")
            .params(|| vec![("index", json!({"type": "integer", "minimum": 0, "maximum": 4}), true)])
            .examples(|| vec![json!({"index": 1})])
            .with(|ctx| {
                let i = index(ctx, "index")?;
                if i > 4 {
                    return bad("`index` is 0 to 4.");
                }
                Ok(vec![Message::WizardStep(i)])
            })
            .effect("The wizard shows that step."),
        cmd("save_machine", "libraries")
            .sends(|| Message::SaveMachine)
            .when(|ctx| match &ctx.app.control {
                Some(c) if c.machine_id == ctx.app.machine_draft.id => refused(format!(
                    "This machine is connected, and saving it changes what its controller holds. {}",
                    class::MACHINE
                )),
                _ => saved_machine(ctx).map(|_| ()),
            })
            .requires("A machine draft that is not the connected machine's.")
            .effect("The draft is saved to the library."),
        cmd("delete_machine", "libraries")
            .help_of(|| Message::DeleteMachine(String::new()), "")
            .params(|| vec![("id", text_param("A library machine's id."), true)])
            .examples(|| vec![json!({"id": "<machine>"})])
            .with(|ctx| {
                let id = machine_id(ctx, "id")?;
                if ctx.app.control.as_ref().is_some_and(|c| c.machine_id == id) {
                    return refused(format!("This machine is connected. {}", class::MACHINE));
                }
                Ok(vec![Message::DeleteMachine(id)])
            })
            .requires("The machine is in the library and not connected.")
            .effect("The machine leaves the library."),
        // Libraries: tools.
        cmd("new_tool", "libraries").sends(|| Message::NewTool).effect("The tool editor on a new draft."),
        cmd("edit_tool", "libraries")
            .help_of(|| Message::EditTool(String::new()), "")
            .params(|| vec![("id", text_param("A library tool's id."), true)])
            .examples(|| vec![json!({"id": "<tool>"})])
            .with(|ctx| Ok(vec![Message::EditTool(tool_id(ctx, "id")?)]))
            .effect("The tool editor on that tool."),
        cmd("save_tool", "libraries").sends(|| Message::SaveTool).when(|ctx| saved_tool(ctx).map(|_| ())).requires("A tool draft with no issues.").effect("The tool draft is saved to the library."),
        cmd("delete_tool", "libraries")
            .help_of(|| Message::DeleteTool(String::new()), "")
            .params(|| vec![("id", text_param("A library tool's id."), true)])
            .examples(|| vec![json!({"id": "<tool>"})])
            .with(|ctx| Ok(vec![Message::DeleteTool(tool_id(ctx, "id")?)]))
            .effect("The tool leaves the library, if no operation uses it."),
        cmd("tool_machine", "libraries")
            .help_of(|| Message::ToolMachine(String::new(), true), "")
            .params(|| vec![("id", text_param("A library machine's id."), true), ("checked", bool_param("Whether the tool fits it."), true)])
            .examples(|| vec![json!({"id": "<machine>", "checked": true})])
            .with(|ctx| {
                let checked = arg(ctx, "checked")?.as_bool().ok_or(Problem::Args("`checked` is true or false.".into()))?;
                Ok(vec![Message::ToolMachine(machine_id(ctx, "id")?, checked)])
            })
            .effect("The tool draft fits that machine, or not."),
        // Libraries: materials.
        cmd("new_material", "libraries").sends(|| Message::NewMaterial).effect("The material editor on a new draft."),
        cmd("edit_material", "libraries")
            .help_of(|| Message::EditMaterial(String::new()), "")
            .params(|| vec![("id", text_param("A library material's id."), true)])
            .examples(|| vec![json!({"id": "<material>"})])
            .with(|ctx| Ok(vec![Message::EditMaterial(material_id(ctx, "id")?)]))
            .effect("The material editor on that material."),
        cmd("save_material", "libraries").sends(|| Message::SaveMaterial).when(|ctx| saved_material(ctx).map(|_| ())).requires("A material draft with no issues.").effect("The material draft is saved to the library."),
        cmd("delete_material", "libraries")
            .help_of(|| Message::DeleteMaterial(String::new()), "")
            .params(|| vec![("id", text_param("A library material's id."), true)])
            .examples(|| vec![json!({"id": "<material>"})])
            .with(|ctx| Ok(vec![Message::DeleteMaterial(material_id(ctx, "id")?)]))
            .effect("The material leaves the library, unless it is the project's."),
        // Libraries: workholding.
        cmd("new_holder", "libraries")
            .help_of(|| Message::NewHolder("Vise"), "")
            .params(|| vec![("kind", choice_param(&crate::holding::KINDS), true)])
            .examples(|| vec![json!({"kind": "Vise"})])
            .with(|ctx| Ok(vec![Message::NewHolder(one_of(ctx, "kind", &crate::holding::KINDS)?)]))
            .effect("The workholding editor on a new draft of that kind."),
        cmd("edit_holder", "libraries")
            .help_of(|| Message::EditHolder(String::new()), "")
            .params(|| vec![("id", text_param("A workholding entry's id."), true)])
            .examples(|| vec![json!({"id": "<holder>"})])
            .with(|ctx| Ok(vec![Message::EditHolder(holder_id(ctx, "id")?)]))
            .effect("The workholding editor on that entry."),
        cmd("save_holder", "libraries").sends(|| Message::SaveHolder).when(|ctx| saved_holder(ctx).map(|_| ())).requires("A workholding draft with no issues.").effect("The workholding draft is saved to the library."),
        cmd("delete_holder", "libraries")
            .help_of(|| Message::DeleteHolder(String::new()), "")
            .params(|| vec![("id", text_param("A workholding entry's id."), true)])
            .examples(|| vec![json!({"id": "<holder>"})])
            .with(|ctx| Ok(vec![Message::DeleteHolder(holder_id(ctx, "id")?)]))
            .effect("The entry leaves the library, unless a setup uses it."),
        cmd("holder_machine", "libraries")
            .help_of(|| Message::HolderMachine(String::new(), true), "")
            .params(|| vec![("id", text_param("A library machine's id."), true), ("checked", bool_param("Whether the entry fits it."), true)])
            .examples(|| vec![json!({"id": "<machine>", "checked": true})])
            .with(|ctx| {
                let checked = arg(ctx, "checked")?.as_bool().ok_or(Problem::Args("`checked` is true or false.".into()))?;
                Ok(vec![Message::HolderMachine(machine_id(ctx, "id")?, checked)])
            })
            .effect("The workholding draft fits that machine, or not."),
        // View.
        cmd("set_camera", "view")
            .help_of(|| Message::Camera("ISO"), "")
            .locked_ok()
            .params(|| vec![("view", choice_param(&CAMERAS), true)])
            .examples(|| vec![json!({"view": "TOP"})])
            .with(|ctx| Ok(vec![Message::Camera(one_of(ctx, "view", &CAMERAS)?)]))
            .effect("The 3D view looks from there; FIT fits the scene again."),
        cmd("orbit_camera", "view")
            .text("Turns the 3D view about the scene.")
            .locked_ok()
            .params(|| vec![("yaw", number_param("Radians about the vertical."), true), ("pitch", number_param("Radians up or down."), true)])
            .examples(|| vec![json!({"yaw": 0.3, "pitch": 0.1})])
            .with(|ctx| Ok(vec![Message::Viewport(viewport::Event::Orbit(number(ctx, "yaw")? as f32, number(ctx, "pitch")? as f32, [0., 0.]))]))
            .effect("The view turns."),
        cmd("pan_camera", "view")
            .text("Moves the 3D view sideways and up or down.")
            .locked_ok()
            .params(|| vec![("dx", number_param("Pixels right."), true), ("dy", number_param("Pixels down."), true)])
            .examples(|| vec![json!({"dx": 20, "dy": 0})])
            .with(|ctx| Ok(vec![Message::Viewport(viewport::Event::Pan(number(ctx, "dx")? as f32, number(ctx, "dy")? as f32))]))
            .effect("The view moves."),
        cmd("zoom_camera", "view")
            .text("Zooms the 3D view about its centre.")
            .locked_ok()
            .params(|| vec![("factor", json!({"type": "number", "minimum": 0.5, "maximum": 1.5, "description": "Above 1 zooms in, below 1 out."}), true)])
            .examples(|| vec![json!({"factor": 1.2})])
            .with(|ctx| {
                let factor = number(ctx, "factor")?.clamp(0.5, 1.5) as f32;
                Ok(vec![Message::Viewport(viewport::Event::Zoom(factor - 1., [0., 0.]))])
            })
            .effect("The view zooms."),
        cmd("section", "view")
            .help_of(|| Message::Section(section::Edit::Toggle), "")
            .locked_ok()
            .params(|| {
                vec![
                    ("action", choice_param(&["toggle", "axis", "at", "flip", "pick_face"]), true),
                    ("axis", choice_param(&["x", "y", "z"]), false),
                    ("at", number_param("For at: where the plane is, mm."), false),
                    ("surface", integer_param("For pick_face: the part's surface id the plane goes through."), false),
                ]
            })
            .examples(|| {
                vec![
                    json!({"action": "toggle"}),
                    json!({"action": "axis", "axis": "x"}),
                    json!({"action": "at", "at": 5}),
                    json!({"action": "flip"}),
                    json!({"action": "pick_face", "surface": 0}),
                ]
            })
            .with(|ctx| {
                let action = one_of(ctx, "action", &["toggle", "axis", "at", "flip", "pick_face"])?;
                Ok(match action {
                    "toggle" => vec![Message::Section(section::Edit::Toggle)],
                    "axis" => {
                        let axis = one_of(ctx, "axis", &["x", "y", "z"])?;
                        let k = ["x", "y", "z"].iter().position(|a| *a == axis).unwrap_or(2);
                        vec![Message::Section(section::Edit::Axis(k))]
                    }
                    "at" => vec![Message::Section(section::Edit::At(number(ctx, "at")? as f32))],
                    "flip" => vec![Message::Section(section::Edit::Flip)],
                    _ => {
                        let surface = index(ctx, "surface")?;
                        let mut m = vec![];
                        if !ctx.app.picking_section {
                            m.push(Message::Section(section::Edit::PickFace));
                        }
                        m.push(Message::Viewport(viewport::Event::SectionSurface(surface)));
                        m
                    }
                })
            })
            .effect("The 3D view's section changes; the job does not."),
        cmd("toggle_stock", "view").sends(|| Message::ToggleStock).locked_ok().effect("The stock is shown or hidden."),
        cmd("toggle_uncut", "view").sends(|| Message::ToggleUncut).locked_ok().effect("The uncut faces are shown or hidden."),
        cmd("toggle_paths", "view").sends(|| Message::TogglePaths).locked_ok().effect("The cutter path is shown or hidden."),
        cmd("stock_detail", "view")
            .help_of(|| Message::StockDetail(stock_surface::Detail::Standard), "")
            .params(|| vec![("level", json!({"type": "string", "enum": detail_names()}), true)])
            .examples(|| vec![json!({"level": "Fine"})])
            .with(|ctx| {
                let given = string(ctx, "level")?;
                match stock_surface::Detail::ALL.iter().find(|d| d.to_string().eq_ignore_ascii_case(given.trim())) {
                    Some(d) => Ok(vec![Message::StockDetail(*d)]),
                    None => bad(format!("`level` must be one of: {}.", detail_names().join(", "))),
                }
            })
            .effect("The simulated stock's surface detail."),
        cmd("play", "view").sends(|| Message::Play).effect("Playback starts or pauses, generating first when needed."),
        cmd("scrub", "view")
            .help_of(|| Message::Scrub(0.), "")
            .params(|| vec![("at", json!({"type": "number", "minimum": 0, "maximum": 1, "description": "Where in the preview, 0 to 1."}), true)])
            .examples(|| vec![json!({"at": 0.5})])
            .when(|ctx| {
                if ctx.app.playback_ready() {
                    Ok(())
                } else {
                    refused("There is no preview to scrub: generate the operation first.")
                }
            })
            .with(|ctx| Ok(vec![Message::Scrub(number(ctx, "at")? as f32), Message::ScrubDone]))
            .effect("The preview is at that point, the stock caught up."),
        cmd("playback_speed", "view")
            .text("Sets how fast the preview plays, 0.5 to 120 times.")
            .locked_ok()
            .params(|| vec![("x", json!({"type": "number", "minimum": 0.5, "maximum": 120}), true)])
            .examples(|| vec![json!({"x": 4})])
            .with(|ctx| Ok(vec![Message::Speed(number(ctx, "x")? as f32)]))
            .effect("The playback speed."),
        cmd("toggle_sidebar", "view")
            .help_of(|| Message::ToggleSidebar(settings::Side::Browser), "")
            .locked_ok()
            .params(|| vec![("side", choice_param(&["browser", "inspector"]), true)])
            .examples(|| vec![json!({"side": "browser"})])
            .with(|ctx| Ok(vec![Message::ToggleSidebar(side(ctx)?)]))
            .effect("The sidebar opens or closes."),
        cmd("sidebar_width", "view")
            .text("Sets a sidebar's width, as dragging its edge does.")
            .locked_ok()
            .params(|| vec![("side", choice_param(&["browser", "inspector"]), true), ("px", number_param("Its width, logical pixels."), true)])
            .examples(|| vec![json!({"side": "inspector", "px": 360})])
            .with(|ctx| Ok(vec![Message::SidebarWidth(side(ctx)?, number(ctx, "px")? as f32, true)]))
            .effect("The sidebar's width, within what the window leaves."),
        cmd("toggle_sim_details", "view").sends(|| Message::ToggleSimDetails).locked_ok().effect("The simulation strip's details show or fold."),
        cmd("toggle_warning_details", "view").sends(|| Message::ToggleWarningDetails).locked_ok().effect("The warnings show in full or at their first sentence."),
        cmd("toggle_about", "view").sends(|| Message::ToggleStrategyHelp).locked_ok().effect("The operation inspector's About opens or closes."),
        cmd("control_tab", "view")
            .text("Chooses the tab the Control page's centre shows, for when the user opens it. Nothing on the machine changes.")
            .locked_ok()
            .params(|| vec![("tab", choice_param(&names(&CONTROL_TABS)), true)])
            .examples(|| vec![json!({"tab": "job"})])
            .with(|ctx| {
                let tab = one_of(ctx, "tab", &names(&CONTROL_TABS))?;
                let t = CONTROL_TABS.iter().find(|(n, _)| *n == tab).map(|(_, t)| *t).unwrap_or(ui::ControlTab::Job);
                Ok(vec![Message::ControlTab(t)])
            })
            .effect("The Control page's tab."),
        cmd("job_view", "view")
            .help_of(|| Message::ControlView(ui::JobView::Stock), "")
            .locked_ok()
            .params(|| vec![("view", choice_param(&["stock", "top"]), true)])
            .examples(|| vec![json!({"view": "top"})])
            .with(|ctx| {
                Ok(vec![Message::ControlView(match one_of(ctx, "view", &["stock", "top"])? {
                    "top" => ui::JobView::Top,
                    _ => ui::JobView::Stock,
                })])
            })
            .effect("How the Control page's Job tab shows the stock."),
        // Review.
        cmd("open_audit_row", "review")
            .text("Opens the operation of a row of the operation audit (generate_all), at the tab its line names.")
            .params(|| vec![("index", integer_param("The row's index from 0."), true)])
            .examples(|| vec![json!({"index": 0})])
            .with(|ctx| {
                let i = index(ctx, "index")?;
                if i >= ctx.app.audit_report.len() {
                    return refused(format!("The audit has {} rows: run generate_all first.", ctx.app.audit_report.len()));
                }
                Ok(vec![Message::AuditRow(i)])
            })
            .effect("The row's operation is open at its tab."),
        cmd("open_issue", "review")
            .help_of(|| Message::OpenIssue(0, 0, String::new()), "")
            .params(|| vec![("setup", addressed("setup"), true), ("operation", addressed("operation"), true), ("message", text_param("The problem's message, as the NC review gives it."), true)])
            .examples(|| vec![json!({"setup": 0, "operation": 0, "message": ""})])
            .with(|ctx| {
                arg(ctx, "setup")?;
                arg(ctx, "operation")?;
                let Some((si, oi)) = address(ctx.app, ctx.args, true)?.1 else {
                    return refused("There is no such operation.");
                };
                Ok(vec![Message::OpenIssue(si, oi, string(ctx, "message")?)])
            })
            .effect("The operation is open at the tab the problem is about."),
        cmd("steps_fix", "review")
            .text("From a manual setup's steps, opens the operation (or, with none, the setup editor) at the problem the steps report.")
            .on_setup()
            .params(|| vec![("operation", addressed("operation"), false), ("problem", text_param("The problem, as the steps say it."), true)])
            .examples(|| vec![json!({"operation": 0, "problem": ""}), json!({"problem": ""})])
            .with(|ctx| {
                let op = match ctx.args.get("operation") {
                    Some(v) => Some(operation_of(ctx.app, ctx.setup, v)?.1),
                    None => None,
                };
                Ok(vec![Message::Guide(GuideAction::Fix(op, string(ctx, "problem")?))])
            })
            .effect("The editor is open at the problem; the steps' place does not move."),
        cmd("run_details", "review")
            .text("Folds the Run panel's rows open or shut: an operation's skips (more) and its steps (steps).")
            .locked_ok()
            .params(|| vec![("more", json!({"type": ["integer", "null"], "description": "The operation whose skips open, by index; null closes them."}), false), ("steps", json!({"type": ["integer", "null"], "description": "The operation whose steps unfold, by index; null folds them."}), false)])
            .examples(|| vec![json!({"more": 0}), json!({"steps": null})])
            .when(|ctx| match ctx.app.control.as_ref().and_then(|c| c.run.as_ref()) {
                Some(_) => Ok(()),
                None => refused("No run is open in the Run panel."),
            })
            .with(|ctx| {
                let row = |key: &str| -> Result<Option<Option<usize>>, Problem> {
                    match ctx.args.get(key) {
                        None => Ok(None),
                        Some(Value::Null) => Ok(Some(None)),
                        Some(v) => match v.as_u64() {
                            Some(i) => Ok(Some(Some(i as usize))),
                            None => bad(format!("`{key}` is an index or null.")),
                        },
                    }
                };
                let mut m = vec![];
                if let Some(more) = row("more")? {
                    m.push(Message::Run(run::Action::More(more)));
                }
                if let Some(steps) = row("steps")? {
                    m.push(Message::Run(run::Action::Steps(steps)));
                }
                if m.is_empty() {
                    return bad("Give `more`, `steps` or both.");
                }
                Ok(m)
            })
            .effect("The rows are folded as asked."),
        // Export.
        cmd("export_gcode", "export")
            .help_of(|| Message::ExportGcode, "")
            .params(|| vec![("folder", text_param("The folder the programs go in."), true), overwrite_param()])
            .examples(|| vec![json!({"folder": "/tmp"})])
            .work(|ctx| {
                let folder = path_arg(ctx, "folder")?;
                if !folder.is_dir() {
                    return refused(format!("There is no folder {}.", folder.display()));
                }
                let Some(mesh) = ctx.app.part_mesh().cloned() else {
                    return refused("Load the imported part before exporting G-code.");
                };
                // The programs are named after the job and each setup: a
                // setup's own name, or its tool runs' names after it.
                let names: Vec<String> = std::fs::read_dir(&folder)
                    .map(|d| d.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect())
                    .unwrap_or_default();
                for si in 0..ctx.app.doc.setups.len() {
                    let whole = crate::post::file_name(&ctx.app.doc, si, None);
                    may_write(ctx, &folder.join(&whole), None)?;
                    let base = whole.trim_end_matches(".ngc");
                    let part = |n: &&String| n.starts_with(&format!("{base} - ")) && n.ends_with(".ngc");
                    if let Some(n) = names.iter().find(part).filter(|_| !flag(ctx, "overwrite")) {
                        return refused(format!(
                            "{} is there already: call again with overwrite: true to replace it.",
                            folder.join(n).display()
                        ));
                    }
                }
                let doc = ctx.app.doc.clone();
                Ok((vec![], off_thread(
                    move || crate::post::write_job(&crate::post::post_job(&doc, &mesh), &folder),
                    |written| Message::GcodeExported(Some(written)),
                )))
            })
            .requires("An imported part; an absolute folder that exists, outside the app's data folder; overwrite: true to replace programs there.")
            .effect("One LinuxCNC program per setup, written after every check passes; nothing is written if one fails."),
        cmd("export_setup_sheet", "export")
            .help_of(|| Message::Export("sheet"), "")
            .params(|| vec![("path", text_param("The Markdown file to write."), true), overwrite_param()])
            .examples(|| vec![json!({"path": "/tmp/sheet.md"})])
            .work(|ctx| {
                let path = path_arg(ctx, "path")?;
                may_write(ctx, &path, None)?;
                let (doc, mesh) = (ctx.app.doc.clone(), ctx.app.part_mesh().cloned());
                Ok((vec![], off_thread(
                    move || {
                        let data = doc.setup_sheet(mesh.as_ref());
                        crate::storage::write_atomic(&path, data.as_bytes()).map(|()| path)
                    },
                    |written| match written {
                        Ok(path) => Message::Exported(path),
                        Err(e) => Message::ExportError(e),
                    },
                )))
            })
            .requires("An absolute path outside the app's data folder; overwrite: true to replace a file.")
            .effect("The setup sheet is written."),
        cmd("save_for_tablet", "export")
            .text("Saves the project with its part as one file the shop tablet opens.")
            .params(|| vec![("path", text_param("The tablet file to write."), true), overwrite_param()])
            .examples(|| vec![json!({"path": "/tmp/job.axis"})])
            .when(|ctx| {
                files_idle(ctx)?;
                match &ctx.app.doc.file {
                    None => refused("Import the part first: the tablet file carries it."),
                    Some(f) if !f.path.exists() => refused(format!(
                        "The part {} is not where the project says. Reimport it, then save for the tablet.",
                        f.path.display()
                    )),
                    Some(_) => Ok(()),
                }
            })
            .work(|ctx| {
                let path = path_arg(ctx, "path")?;
                may_write(ctx, &path, None)?;
                let mut doc = ctx.app.doc.clone();
                if let Some(mesh) = &ctx.app.imported_mesh {
                    crate::picks::record(&mut doc, mesh);
                }
                Ok((vec![], off_thread(
                    move || crate::bundle::write(&path, &doc).map(|()| path),
                    |written| Message::TabletSaved(Some(written)),
                )))
            })
            .requires("An imported part whose file is there; an absolute path outside the app's data folder; overwrite: true to replace a file.")
            .effect("The tablet file is written."),
        // Machine: the stops the agent may send.
        cmd("hold", "machine")
            .help_of(|| Message::Run(run::Action::Hold), "")
            .locked_ok()
            .sends(|| Message::Run(run::Action::Hold))
            .when(connected)
            .requires("A connected machine.")
            .effect("The run holds."),
        cmd("stop", "machine")
            .help_of(|| Message::Run(run::Action::Stop), "")
            .locked_ok()
            .sends(|| Message::Run(run::Action::Stop))
            .when(connected)
            .requires("A connected machine.")
            .effect("The run stops."),
        cmd("estop", "machine")
            .text("E-stops the machine: every axis and the spindle stop at once.")
            .locked_ok()
            .sends(|| Message::MachineEStop)
            .when(connected)
            .requires("A connected machine.")
            .effect("The machine is in E-stop."),
        cmd("jog_release", "machine")
            .help_of(|| Message::Machine(control::Action::JogRelease), "")
            .locked_ok()
            .sends(|| Message::Machine(control::Action::JogRelease))
            .when(connected)
            .requires("A connected machine.")
            .effect("A jog the user holds ends."),
    ]
}

/// The operation `select_operation`'s arguments name.
fn selected_operation_arg(ctx: &Ctx) -> Option<(usize, usize)> {
    ctx.args.get("operation")?;
    address(ctx.app, ctx.args, true).ok()?.1
}
/// The schema of a pick command's ids.
fn ids_param(description: &str) -> Value {
    json!({
        "type": "array",
        "items": {"type": "integer", "minimum": 0},
        "minItems": 1,
        "description": description,
    })
}
/// A pick command's ids, each once, in the order given.
fn ids_arg(ctx: &Ctx) -> Result<Vec<usize>, Problem> {
    let Some(given) = arg(ctx, "ids")?.as_array().filter(|a| !a.is_empty()) else {
        return bad("`ids` must be a list of ids (integers from 0).");
    };
    let mut ids: Vec<usize> = vec![];
    for v in given {
        let Some(n) = v.as_u64() else {
            return bad("`ids` must be a list of ids (integers from 0).");
        };
        if !ids.contains(&(n as usize)) {
            ids.push(n as usize);
        }
    }
    Ok(ids)
}
/// A pick command's mode: add when not given.
fn mode_arg(ctx: &Ctx) -> Result<Mode, Problem> {
    match ctx.args.get("mode") {
        None | Some(Value::Null) => Ok(Mode::Add),
        Some(_) => Ok(Mode::of(one_of(ctx, "mode", &Mode::NAMES)?).unwrap_or(Mode::Add)),
    }
}
/// The setup a pin command acts on, by index: one on an imported part that
/// sits on the pins, by holes in the stock's leftover (`leftover`) or by
/// the part's own.
fn pins_setup(ctx: &Ctx, leftover: bool) -> Result<usize, Problem> {
    let app = ctx.app;
    let Some(si) = ctx.setup else {
        return refused("There is no setup: add one first (add_setup).");
    };
    if app.is_sample() || app.doc.file.is_none() || app.imported_mesh.is_none() {
        return refused(
            "The pins stand in a part's holes or its stock's leftover: import a part first (import_model).",
        );
    }
    let s = &app.doc.setups[si];
    if !crate::post::sits_on_pins(s) {
        return refused(
            "This setup does not sit on the fixture pins: set on_pins to Yes in its Work zero step first (edit_field).",
        );
    }
    match (leftover, crate::post::pins_in_leftover(s)) {
        (true, false) => refused(
            "This setup drops over the pins by the part's own holes: choose them with pick_pin_holes.",
        ),
        (false, true) => {
            refused("This setup's pins stand in the stock's leftover: place them with place_pins.")
        }
        _ => Ok(si),
    }
}
/// `work` off the `App`'s thread, as the app runs its file work, and its
/// result as `done`'s message.
fn off_thread<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
    done: impl FnOnce(Result<T, String>) -> Message + Send + 'static,
) -> Task<Message> {
    Task::perform(
        async move {
            tokio::task::spawn_blocking(work)
                .await
                .unwrap_or_else(|e| Err(format!("The worker failed: {e}")))
        },
        done,
    )
}
fn connected(ctx: &Ctx) -> Result<(), Problem> {
    match ctx.app.control {
        Some(_) => Ok(()),
        None => refused("No machine is connected."),
    }
}
fn side(ctx: &Ctx) -> Result<settings::Side, Problem> {
    Ok(match one_of(ctx, "side", &["browser", "inspector"])? {
        "inspector" => settings::Side::Inspector,
        _ => settings::Side::Browser,
    })
}

/// The library entry the machine draft saves as, or why it is refused.
fn saved_machine(ctx: &Ctx) -> Result<crate::model::Machine, Problem> {
    (ctx.app.doc.saved_machine(&ctx.app.machine_draft))
        .map_err(|e| Problem::Refused(format!("The machine is not saved: {}", e.join(" "))))
}
fn saved_tool(ctx: &Ctx) -> Result<crate::model::Tool, Problem> {
    (ctx.app.doc.saved_tool(&ctx.app.tool_draft))
        .map_err(|e| Problem::Refused(format!("The tool is not saved: {}", e.join(" "))))
}
fn saved_material(ctx: &Ctx) -> Result<crate::material::Material, Problem> {
    (ctx.app.doc.saved_material(&ctx.app.material_draft))
        .map_err(|e| Problem::Refused(format!("The material is not saved: {}", e.join(" "))))
}
fn saved_holder(ctx: &Ctx) -> Result<crate::holding::Holder, Problem> {
    (ctx.app.doc.saved_holder(&ctx.app.holder_draft))
        .map_err(|e| Problem::Refused(format!("The entry is not saved: {}", e.join(" "))))
}
/// Two entries as a project saves them.
fn same<T: serde::Serialize>(a: &T, b: &T) -> bool {
    serde_json::to_value(a).ok() == serde_json::to_value(b).ok()
}
/// The machine `m` with `version`: machines compare without it, so an
/// unchanged save does not raise it.
fn versioned(m: &crate::model::Machine, version: u32) -> crate::model::Machine {
    crate::model::Machine {
        version,
        ..m.clone()
    }
}

/// Whether setup `si` (as `resolve` gave it) is the active one.
fn setup_is_open(app: &App, si: Option<usize>) -> bool {
    si.and_then(|si| app.doc.setups.get(si))
        .is_some_and(|s| s.id == app.doc.active)
}
/// Whether the addressed operation is the selected one, in the active setup.
fn operation_is_open(app: &App, target: Option<(usize, usize)>) -> bool {
    target.is_some_and(|(si, oi)| setup_is_open(app, Some(si)) && app.selected == Some(oi))
}

/// An effect that holds when `f` does.
fn holds(f: impl Fn(&App) -> bool + 'static, fail: impl Into<String>) -> Expect {
    let fail = fail.into();
    Box::new(move |app| if f(app) { Ok(None) } else { Err(fail.clone()) })
}
/// An effect for a command whose work completes through the app's result
/// message (`Kind`): the messages that arrived in its window decide first,
/// an error among them (or none of `primary`) failing it, and `check` then
/// verifies the state the success must have left.
fn judged(
    command: &'static str,
    primary: Kind,
    also: &'static [Kind],
    check: impl Fn(&App) -> bool + 'static,
    fail: impl Into<String>,
) -> Expect {
    let fail = fail.into();
    Box::new(move |app| {
        let mine = |k: Kind| k == primary || also.contains(&k);
        let done = &app.mcp.completions;
        if let Some(Completion { result: Err(e), .. }) =
            done.iter().find(|c| mine(c.kind) && c.result.is_err())
        {
            return Err(e.clone());
        }
        if !done.iter().any(|c| c.kind == primary) {
            return Err(format!("The app did not finish {command}."));
        }
        if check(app) {
            Ok(None)
        } else {
            Err(fail.clone())
        }
    })
}
/// An effect that holds when what `get` reads changed. Unchanged, it is
/// `no_change` (`already` says why) unless the status line said something
/// new, which is then the failure's reason: the app refused.
fn alters<T: PartialEq + 'static>(
    ctx: &Ctx,
    get: impl Fn(&App) -> T + 'static,
    already: &'static str,
) -> Expect {
    let (before, status) = (get(ctx.app), ctx.app.status.clone());
    Box::new(move |app| {
        if get(app) != before {
            Ok(None)
        } else if app.status != status && !app.status.is_empty() {
            Err(app.status.clone())
        } else {
            Ok(Some(already.into()))
        }
    })
}
/// An effect that holds when the flag `get` reads turned over.
fn flips(ctx: &Ctx, get: fn(&App) -> bool) -> Expect {
    let before = get(ctx.app);
    holds(move |a| get(a) != before, "It did not change.")
}
fn op_in<'a>(app: &'a App, id: &str) -> Option<&'a crate::model::Operation> {
    (app.doc.setups.iter().flat_map(|s| &s.operations)).find(|o| o.id == id)
}
fn setup_in<'a>(app: &'a App, id: &str) -> Option<&'a crate::model::Setup> {
    app.doc.setups.iter().find(|s| s.id == id)
}
fn op_ids(app: &App) -> Vec<String> {
    (app.doc.setups.iter().flat_map(|s| &s.operations))
        .map(|o| o.id.clone())
        .collect()
}
/// The targeted operation's id, and the targeted setup's.
fn op_id(ctx: &Ctx) -> String {
    ctx.op().map(|o| o.id.clone()).unwrap_or_default()
}
fn setup_id(ctx: &Ctx) -> String {
    (ctx.setup.and_then(|si| ctx.app.doc.setups.get(si)))
        .map(|s| s.id.clone())
        .unwrap_or_default()
}
fn op_field(app: &App, id: &str, key: &str) -> Option<String> {
    op_in(app, id).map(|o| crate::model::get(&o.p, key).to_string())
}
fn setup_field(app: &App, id: &str, key: &str) -> Option<String> {
    setup_in(app, id).map(|s| crate::model::get(&s.p, key).to_string())
}
fn arg_str(ctx: &Ctx, key: &str) -> String {
    ctx.args
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}
fn arg_path(ctx: &Ctx, key: &str) -> PathBuf {
    path_arg(ctx, key).unwrap_or_default()
}
/// A library draft's field (`edit_field` on a library target).
fn draft_field(app: &App, target: Target, key: &str) -> String {
    let p = match target {
        Target::Machine => &app.machine_draft.p,
        Target::Tool => &app.tool_draft.p,
        Target::Material => &app.material_draft.p,
        Target::Holder => &app.holder_draft.p,
        Target::Operation | Target::Setup => return String::new(),
    };
    crate::model::get(p, key).to_string()
}
/// What `edit_field` edits: the target, its operation or setup id, the key.
fn edited(ctx: &Ctx) -> Option<(Target, String, String)> {
    let name = ctx.args.get("target")?.as_str()?;
    let (_, target) = EDIT_TARGETS
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(name))?;
    let app = ctx.app;
    let id = match target {
        Target::Operation => {
            let (si, oi) = address(app, ctx.args, true).ok()?.1?;
            app.doc.setups[si].operations[oi].id.clone()
        }
        Target::Setup => {
            let si = address(app, ctx.args, false).ok()?.0?;
            app.doc.setups[si].id.clone()
        }
        _ => String::new(),
    };
    Some((*target, id, ctx.args.get("key")?.as_str()?.to_string()))
}
fn edited_value(app: &App, (target, id, key): &(Target, String, String)) -> Option<String> {
    match target {
        Target::Operation => op_field(app, id, key),
        Target::Setup => setup_field(app, id, key),
        _ => Some(draft_field(app, *target, key)),
    }
}
fn edit_value(ctx: &Ctx) -> String {
    match ctx.args.get("value") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Bool(b)) => b.to_string(),
        _ => String::new(),
    }
}
/// The page `goto_page`'s argument names.
fn page_arg(ctx: &Ctx) -> Option<Page> {
    let given = arg_str(ctx, "page");
    PAGES
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(given.trim()))
        .map(|(_, p)| *p)
}
/// Whether `list` (machine ids joined by commas) holds `id`.
fn lists(list: &str, id: &str) -> bool {
    list.split(',').any(|m| m.trim() == id)
}
/// A file `path` written since `since` (a little earlier, for the file
/// system's clock).
fn written_since(path: &Path, since: std::time::SystemTime) -> bool {
    let since = since - std::time::Duration::from_secs(2);
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .is_ok_and(|t| t >= since)
}
fn recommended(ctx: &Ctx) -> Option<((String, String), String)> {
    let (si, oi) = ctx.operation?;
    let mut doc = ctx.app.doc.clone();
    let said = doc.fit_op_speeds(si, oi);
    let o = &doc.setups[si].operations[oi];
    let fields = |o: &crate::model::Operation| {
        (
            crate::model::get(&o.p, "rpm").to_string(),
            crate::model::get(&o.p, "feed").to_string(),
        )
    };
    Some((fields(o), said))
}

/// Why a pick command's ids change nothing: in `add` all of them picked,
/// in `remove` none; `replace` is clicked.
fn unpicked_already(ctx: &Ctx, picked: &[usize]) -> Option<String> {
    let ids = ids_arg(ctx).ok()?;
    match mode_arg(ctx).ok()? {
        Mode::Add => {
            (ids.iter().all(|i| picked.contains(i))).then(|| "Every id is picked already.".into())
        }
        Mode::Remove => {
            (ids.iter().all(|i| !picked.contains(i))).then(|| "None of the ids is picked.".into())
        }
        Mode::Replace => None,
    }
}

/// Why a command is already so, by name: then nothing is sent.
fn already(name: &str) -> Option<fn(&Ctx) -> Option<String>> {
    let f: fn(&Ctx) -> Option<String> = match name {
        "set_project_material" => |ctx| {
            (ctx.app.doc.material_id == arg_str(ctx, "id"))
                .then(|| "That is the project's material already.".into())
        },
        "goto_page" => {
            |ctx| (page_arg(ctx) == Some(ctx.app.page)).then(|| "That page is open already.".into())
        }
        "set_origin" => |ctx| {
            (setup_field(ctx.app, &setup_id(ctx), "origin") == Some(arg_str(ctx, "origin")))
                .then(|| "The setup's work zero is that already.".into())
        },
        "set_setup_step" => |ctx| {
            (setup_is_open(ctx.app, ctx.setup)
                && ctx.app.setup_step == arg_str(ctx, "step")
                && ctx.app.selected.is_none()
                && ctx.app.page == Page::Work)
                .then(|| "The setup editor shows that step already.".into())
        },
        "set_operation_tool" => |ctx| {
            (ctx.op().is_some_and(|o| o.tool == arg_str(ctx, "id")))
                .then(|| "The operation cuts with that tool already.".into())
        },
        "set_rest_source" => |ctx| {
            (op_field(ctx.app, &op_id(ctx), "rest_source") == Some(arg_str(ctx, "source")))
                .then(|| "That is the operation's rest source already.".into())
        },
        "set_pin_setup" => |ctx| {
            let si = setup_of(ctx.app, ctx.args.get("setup")?).ok()?;
            let want = ctx.app.doc.setups[si].id.clone();
            (op_field(ctx.app, &op_id(ctx), "pin_setup") == Some(want))
                .then(|| "The operation makes that setup's pin holes already.".into())
        },
        "set_tab" => |ctx| {
            (operation_is_open(ctx.app, ctx.operation)
                && ctx.app.tab.eq_ignore_ascii_case(&arg_str(ctx, "tab"))
                && ctx.app.page == Page::Work)
                .then(|| "That tab is open already.".into())
        },
        "edit_field" => |ctx| {
            let target = edited(ctx)?;
            (edited_value(ctx.app, &target) == Some(edit_value(ctx)))
                .then(|| "The field holds that value already.".into())
        },
        "clear_picks" => |ctx| {
            ctx.op()
                .is_some_and(|o| o.surfaces.as_ref().is_none_or(|s| s.ids.is_empty()))
                .then(|| "The operation has no picks to clear.".into())
        },
        // A toggle's ids all picked already, or none picked to remove:
        // nothing is clicked. A fill's face is clicked to see.
        "pick_surfaces" => |ctx| {
            let o = ctx.op()?;
            if ctx.app.pick_kind(o) == crate::PickKind::Fill {
                return None;
            }
            let mesh = ctx.app.imported_mesh.as_ref()?;
            let picked: Vec<usize> = (o.surfaces.as_ref())
                .filter(|s| s.matches(mesh))
                .map_or(vec![], |s| s.ids.clone());
            unpicked_already(ctx, &picked)
        },
        "pick_features" => |ctx| {
            let picked = ctx.op()?.geometry.clone();
            unpicked_already(ctx, &picked)
        },
        "clear_pin_holes" => |ctx| {
            let s = ctx.app.doc.setups.get(ctx.setup?)?;
            (crate::post::chosen_pin_holes(s).is_empty())
                .then(|| "No hole is chosen for the pins.".into())
        },
        "clear_pin_place" => |ctx| {
            let s = ctx.app.doc.setups.get(ctx.setup?)?;
            (crate::model::get(&s.p, "pin_place").is_empty())
                .then(|| "The pins have no place.".into())
        },
        "recommend_speeds" => |ctx| {
            let (fields, said) = recommended(ctx)?;
            let o = ctx.op()?;
            let now = (
                crate::model::get(&o.p, "rpm").to_string(),
                crate::model::get(&o.p, "feed").to_string(),
            );
            (now == fields).then(|| {
                if said.is_empty() {
                    "The speed and feed are the recommended ones already.".into()
                } else {
                    said
                }
            })
        },
        "stock_detail" => |ctx| {
            (ctx.app
                .stock_detail
                .to_string()
                .eq_ignore_ascii_case(&arg_str(ctx, "level")))
            .then(|| "The stock is shown at that detail already.".into())
        },
        "playback_speed" => |ctx| {
            let x = ctx.args.get("x")?.as_f64()? as f32;
            (ctx.app.speed == x.clamp(0.5, 120.))
                .then(|| "Playback runs at that speed already.".into())
        },
        "control_tab" => |ctx| {
            let tab = CONTROL_TABS
                .iter()
                .find(|(n, _)| n.eq_ignore_ascii_case(&arg_str(ctx, "tab")))?
                .1;
            (ctx.app.control_tab == tab).then(|| "That tab is chosen already.".into())
        },
        "job_view" => |ctx| {
            let top = arg_str(ctx, "view").eq_ignore_ascii_case("top");
            (ctx.app.control_view
                == if top {
                    ui::JobView::Top
                } else {
                    ui::JobView::Stock
                })
            .then(|| "The Job tab shows the stock so already.".into())
        },
        "machine_wizard_step" => |ctx| {
            (ctx.app.page == Page::Wizard
                && ctx.args.get("index")?.as_u64()? as usize == ctx.app.wizard_step)
                .then(|| "The wizard shows that step already.".into())
        },
        "save_machine" => |ctx| {
            let saved = saved_machine(ctx).ok()?;
            let there = ctx.app.doc.machines.iter().find(|m| m.id == saved.id)?;
            same(&versioned(&saved, there.version), there)
                .then(|| "The library holds this machine as the draft has it.".into())
        },
        "save_tool" => |ctx| {
            let saved = saved_tool(ctx).ok()?;
            let there = ctx.app.doc.tools.iter().find(|t| t.id == saved.id)?;
            same(&saved, there).then(|| "The library holds this tool as the draft has it.".into())
        },
        "save_material" => |ctx| {
            let saved = saved_material(ctx).ok()?;
            let there = ctx.app.doc.materials.iter().find(|m| m.id == saved.id)?;
            (saved == *there).then(|| "The library holds this material as the draft has it.".into())
        },
        "save_holder" => |ctx| {
            let saved = saved_holder(ctx).ok()?;
            let there = ctx.app.doc.workholding.iter().find(|h| h.id == saved.id)?;
            (saved == *there).then(|| "The library holds this entry as the draft has it.".into())
        },
        _ => return None,
    };
    Some(f)
}

/// The effect each command is checked for once its work has settled
/// (the spec's *Outcomes*), made from the state it starts on.
fn effect(name: &str) -> Option<fn(&Ctx) -> Expect> {
    let f: fn(&Ctx) -> Expect = match name {
        // Project.
        "new_project" => |_| {
            holds(
                |a| a.doc.setups.is_empty() && a.doc.file.is_none() && !a.doc.sample_model,
                "No blank project was started.",
            )
        },
        "open_sample" => |_| holds(|a| a.is_sample(), "The example project did not open."),
        "resume_project" => |_| {
            holds(
                |a| a.page == Page::Work && (a.doc.file.is_none() || a.imported_mesh.is_some()),
                "The project's part could not be loaded.",
            )
        },
        "recover_previous_project" => |ctx| {
            alters(
                ctx,
                |a| serde_json::to_string(&a.doc).unwrap_or_default(),
                "The project archived last is the one open.",
            )
        },
        "open_project" => |ctx| {
            let path = arg_path(ctx, "path");
            judged(
                "open_project",
                Kind::Open,
                &[Kind::Restore],
                move |a| {
                    let history = a.state_path.with_file_name("history");
                    let opened = a.project_path.as_deref() == Some(path.as_path())
                        || path.starts_with(&history);
                    opened && (a.doc.file.is_none() || a.imported_mesh.is_some())
                },
                "The project did not open.",
            )
        },
        "save_project" => |ctx| {
            let path = match ctx.args.get("path") {
                Some(_) => arg_path(ctx, "path"),
                None => ctx.app.project_path.clone().unwrap_or_default(),
            };
            Box::new(move |a| {
                match &a.mcp.last_save {
                    Some(Ok(done)) if *done == path => {}
                    Some(Ok(_)) => return Err("The project was saved somewhere else.".into()),
                    Some(Err(e)) => return Err(format!("Save failed: {e}")),
                    None => return Err("The app did not finish save_project.".into()),
                }
                // The file holds the project, by value.
                if a.project_path.as_deref() == Some(path.as_path())
                    && !a.project_dirty
                    && a.saved_to_file()
                {
                    Ok(None)
                } else {
                    Err("The project was not saved.".into())
                }
            })
        },
        "set_project_material" => |ctx| {
            let id = arg_str(ctx, "id");
            holds(
                move |a| a.doc.material_id == id,
                "The project's material did not change.",
            )
        },
        "goto_page" => |ctx| {
            let page = page_arg(ctx);
            holds(move |a| Some(a.page) == page, "The page did not open.")
        },
        // Part.
        "import_model" => |ctx| {
            let asked = arg_path(ctx, "path");
            judged(
                "import_model",
                Kind::Import,
                &[Kind::Reimport],
                move |a| {
                    // The requested file, by any spelling of its path.
                    let same = |d: &Path| {
                        d == asked
                            || std::fs::canonicalize(d)
                                .is_ok_and(|d| std::fs::canonicalize(&asked).is_ok_and(|p| p == d))
                    };
                    a.doc.file.as_ref().is_some_and(|f| same(&f.path)) && a.imported_mesh.is_some()
                },
                "The model was not imported.",
            )
        },
        "reimport_model" => |_| {
            judged(
                "reimport_model",
                Kind::Reimport,
                &[],
                |a| a.imported_mesh.is_some(),
                "The model was not loaded again.",
            )
        },
        "dismiss_pick_note" => |ctx| {
            let id = op_id(ctx);
            holds(
                move |a| !a.doc.pick_checks.contains_key(&id),
                "The note stayed.",
            )
        },
        // Setup.
        "add_setup" | "add_manual_setup" | "add_lathe_setup" | "duplicate_setup" => |ctx| {
            let before: Vec<String> = ctx.app.doc.setups.iter().map(|s| s.id.clone()).collect();
            holds(
                move |a| a.doc.setups.len() > before.len() && !before.contains(&a.doc.active),
                "No setup was added.",
            )
        },
        "remove_setup" => |ctx| {
            let id = setup_id(ctx);
            holds(
                move |a| setup_in(a, &id).is_none(),
                "The setup was not removed.",
            )
        },
        "move_setup" => |ctx| {
            let (id, from) = (setup_id(ctx), ctx.setup.unwrap_or(0) as i64);
            let to = from + ctx.args.get("delta").and_then(Value::as_i64).unwrap_or(0);
            holds(
                move |a| {
                    a.doc
                        .setups
                        .iter()
                        .position(|s| s.id == id)
                        .map(|i| i as i64)
                        == Some(to)
                },
                "The setup did not move there.",
            )
        },
        "flip_setup" | "turn_stock" | "square_to_stock" => |ctx| {
            let id = setup_id(ctx);
            alters(
                ctx,
                move |a| {
                    setup_in(a, &id)
                        .map(|s| ["rx", "ry", "rz"].map(|k| crate::model::get(&s.p, k).to_string()))
                },
                "The setup is turned so already.",
            )
        },
        "assign_machine" => |ctx| {
            let (id, machine) = (setup_id(ctx), arg_str(ctx, "id"));
            holds(
                move |a| setup_in(a, &id).is_some_and(|s| s.machine.id == machine),
                "The setup's machine did not change.",
            )
        },
        "set_origin" => |ctx| {
            let (id, origin) = (setup_id(ctx), arg_str(ctx, "origin"));
            holds(
                move |a| {
                    setup_field(a, &id, "origin").is_some_and(|o| o.eq_ignore_ascii_case(&origin))
                },
                "The work zero did not change.",
            )
        },
        "set_setup_step" => |ctx| {
            let (step, id) = (arg_str(ctx, "step"), setup_id(ctx));
            holds(
                move |a| {
                    a.setup_step.eq_ignore_ascii_case(&step)
                        && a.selected.is_none()
                        && a.doc.active == id
                },
                "The setup editor did not open on that step.",
            )
        },
        "set_clamps" => |ctx| {
            let (id, text) = (setup_id(ctx), arg_str(ctx, "text"));
            holds(
                move |a| {
                    setup_field(a, &id, "clamps").as_deref() == Some(text.as_str())
                        && a.clamps_draft.is_none()
                },
                "The clamp positions were not taken.",
            )
        },
        "suggest_layout" => |ctx| {
            let id = setup_id(ctx);
            let placed = move |a: &App| setup_in(a, &id).map(|s| s.p.clone());
            let (before, status) = (placed(ctx.app), ctx.app.status.clone());
            // A layout suggested ("Suggested: ...") that is the one saved
            // already changes nothing: no_change, in the app's words, which
            // say what the toolpaths come to beside it.
            Box::new(move |a| {
                if placed(a) != before {
                    Ok(None)
                } else if a.status.starts_with("Suggested: ") {
                    Ok(Some(format!(
                        "The workholding stands where Suggest layout puts it already. {}",
                        a.status
                    )))
                } else if a.status != status && !a.status.is_empty() {
                    Err(a.status.clone())
                } else {
                    Ok(Some(
                        "The workholding stands where Suggest layout puts it already.".into(),
                    ))
                }
            })
        },
        "show_stock_after_setup" => {
            |_| holds(|a| a.setup_end, "The stock after the setup is not shown.")
        }
        "open_setup_editor" => |_| {
            holds(
                |a| a.selected.is_none() && a.page == Page::Work,
                "The setup editor did not open.",
            )
        },
        // Operation.
        "add_turn" => |ctx| {
            let before = op_ids(ctx.app);
            let kind = arg_str(ctx, "kind");
            holds(
                move |a| {
                    a.operation().is_some_and(|o| {
                        !before.contains(&o.id)
                            && o.strategy == Strategy::Turn
                            && crate::model::get(&o.p, "turn_kind").eq_ignore_ascii_case(&kind)
                    })
                },
                "No Turn was added.",
            )
        },
        "duplicate_operation" => |ctx| {
            let before = op_ids(ctx.app);
            holds(
                move |a| a.operation().is_some_and(|o| !before.contains(&o.id)),
                "No copy was added.",
            )
        },
        "remove_operation" => |ctx| {
            let id = op_id(ctx);
            holds(
                move |a| op_in(a, &id).is_none(),
                "The operation was not removed.",
            )
        },
        "move_operation" => |ctx| {
            let id = op_id(ctx);
            let from = ctx.operation.map_or(0, |(_, oi)| oi) as i64;
            let to = from + ctx.args.get("delta").and_then(Value::as_i64).unwrap_or(0);
            holds(
                move |a| {
                    (a.doc.setups.iter())
                        .find_map(|s| s.operations.iter().position(|o| o.id == id))
                        .map(|i| i as i64)
                        == Some(to)
                },
                "The operation did not move there.",
            )
        },
        "toggle_operation" => |ctx| {
            let id = op_id(ctx);
            let before = ctx.op().is_some_and(|o| o.enabled);
            holds(
                move |a| op_in(a, &id).is_some_and(|o| o.enabled != before),
                "The operation was not switched.",
            )
        },
        "set_operation_tool" => |ctx| {
            let (id, tool) = (op_id(ctx), arg_str(ctx, "id"));
            holds(
                move |a| op_in(a, &id).is_some_and(|o| o.tool == tool),
                "The tool did not change.",
            )
        },
        "recommend_speeds" => |ctx| {
            let id = op_id(ctx);
            let (want, said) = recommended(ctx).unwrap_or_default();
            holds(
                move |a| {
                    (op_field(a, &id, "rpm"), op_field(a, &id, "feed"))
                        == (Some(want.0.clone()), Some(want.1.clone()))
                },
                if said.is_empty() {
                    "The speed and feed did not change.".to_string()
                } else {
                    said
                },
            )
        },
        "set_rest_source" => |ctx| {
            let (id, source) = (op_id(ctx), arg_str(ctx, "source"));
            holds(
                move |a| op_field(a, &id, "rest_source").as_deref() == Some(source.as_str()),
                "The rest source did not change.",
            )
        },
        "set_pin_setup" => |ctx| {
            let id = op_id(ctx);
            let want = (ctx.args.get("setup"))
                .and_then(|v| setup_of(ctx.app, v).ok())
                .map(|si| ctx.app.doc.setups[si].id.clone())
                .unwrap_or_default();
            holds(
                move |a| op_field(a, &id, "pin_setup").as_deref() == Some(want.as_str()),
                "The pins' setup did not change.",
            )
        },
        "set_tab" => |ctx| {
            let (tab, id) = (arg_str(ctx, "tab"), op_id(ctx));
            holds(
                move |a| {
                    a.tab.eq_ignore_ascii_case(&tab)
                        && a.selected.is_some_and(|oi| {
                            (a.doc.setups.iter().find(|s| s.id == a.doc.active))
                                .and_then(|s| s.operations.get(oi))
                                .is_some_and(|o| o.id == id)
                        })
                },
                "The tab did not open.",
            )
        },
        "add_threading" | "add_hole_stages" | "add_centring" | "add_pin_drilling" => |ctx| {
            let before = op_ids(ctx.app).len();
            holds(move |a| op_ids(a).len() > before, "No operation was added.")
        },
        "add_needed_tool" => |ctx| {
            let id = op_id(ctx);
            let before: Vec<String> = ctx.app.doc.tools.iter().map(|t| t.id.clone()).collect();
            holds(
                move |a| {
                    op_in(a, &id).is_some_and(|o| {
                        !before.contains(&o.tool) && a.doc.tools.iter().any(|t| t.id == o.tool)
                    })
                },
                "No tool was added for the operation.",
            )
        },
        "thread_from_part" | "box_from_face" => |ctx| {
            let id = op_id(ctx);
            alters(
                ctx,
                move |a| op_in(a, &id).map(|o| o.p.clone()),
                "The fields are what the part gives already.",
            )
        },
        "pick_all_corners" => |ctx| {
            let id = op_id(ctx);
            alters(
                ctx,
                move |a| op_in(a, &id).and_then(|o| o.surfaces.as_ref().map(|s| s.ids.clone())),
                "Every rounded corner is picked already.",
            )
        },
        "edit_field" => |ctx| {
            let target = edited(ctx);
            let want = edit_value(ctx);
            let before = target.as_ref().and_then(|t| edited_value(ctx.app, t));
            Box::new(move |a| {
                let now = target.as_ref().and_then(|t| edited_value(a, t));
                if now.as_deref() == Some(want.as_str()) || now != before {
                    Ok(None)
                } else {
                    Err("The field kept its value.".into())
                }
            })
        },
        "undo_last_edit" => |_| holds(|a| a.typed_last.is_none(), "The edit was not undone."),
        // Picks.
        "clear_picks" => |ctx| {
            let id = op_id(ctx);
            holds(
                move |a| {
                    op_in(a, &id)
                        .is_some_and(|o| o.surfaces.as_ref().is_none_or(|s| s.ids.is_empty()))
                },
                "The picks were not cleared.",
            )
        },
        "drop_hole" => |ctx| {
            let id = op_id(ctx);
            alters(
                ctx,
                move |a| op_in(a, &id).and_then(|o| o.surfaces.as_ref().map(|s| s.ids.clone())),
                "No picked hole has that index.",
            )
        },
        "clear_pin_holes" => |ctx| {
            let id = setup_id(ctx);
            holds(
                move |a| {
                    setup_in(a, &id).is_some_and(|s| crate::post::chosen_pin_holes(s).is_empty())
                },
                "The pin holes were not cleared.",
            )
        },
        "clear_pin_place" => |ctx| {
            let id = setup_id(ctx);
            holds(
                move |a| setup_field(a, &id, "pin_place").is_some_and(|p| p.is_empty()),
                "The pins' place was not cleared.",
            )
        },
        // Generate.
        "generate" => |ctx| {
            let id = op_id(ctx);
            holds(
                move |a| {
                    let made = a.operation().is_some_and(|o| o.id == id)
                        && (a.simulation.is_some()
                            || (a.is_sample() && op_in(a, &id).is_some_and(|o| o.preview)));
                    made && a.generating.is_none() && !a.status_problem
                },
                "No toolpath was generated.",
            )
        },
        "generate_all" => |_| {
            holds(
                |a| {
                    a.generating.is_none()
                        && !a.status_problem
                        && (a.is_sample() || !a.audit_report.is_empty())
                },
                "The operations were not generated.",
            )
        },
        "cancel_generation" => |_| holds(|a| a.generating.is_none(), "The generation goes on."),
        // Libraries.
        "new_machine" => |ctx| {
            let before = ctx.app.machine_draft.id.clone();
            holds(
                move |a| a.machine_draft.id != before && a.page == Page::Wizard,
                "No machine draft was opened.",
            )
        },
        "edit_machine" => |ctx| {
            let id = arg_str(ctx, "id");
            holds(
                move |a| a.machine_draft.id == id,
                "The machine is not open in the wizard.",
            )
        },
        "machine_wizard_step" => |ctx| {
            let i = ctx.args.get("index").and_then(Value::as_u64).unwrap_or(0) as usize;
            holds(
                move |a| a.wizard_step == i,
                "The wizard did not go to that step.",
            )
        },
        "save_machine" => |ctx| {
            let want = saved_machine(ctx).ok();
            holds(
                move |a| {
                    want.as_ref()
                        .is_some_and(|w| a.doc.machines.iter().any(|m| m.id == w.id && same(m, w)))
                },
                "The machine was not saved.",
            )
        },
        "save_tool" => |ctx| {
            let want = saved_tool(ctx).ok();
            holds(
                move |a| {
                    want.as_ref()
                        .is_some_and(|w| a.doc.tools.iter().any(|t| t.id == w.id && same(t, w)))
                },
                "The tool was not saved.",
            )
        },
        "save_material" => |ctx| {
            let want = saved_material(ctx).ok();
            holds(
                move |a| want.as_ref().is_some_and(|w| a.doc.materials.contains(w)),
                "The material was not saved.",
            )
        },
        "save_holder" => |ctx| {
            let want = saved_holder(ctx).ok();
            holds(
                move |a| want.as_ref().is_some_and(|w| a.doc.workholding.contains(w)),
                "The entry was not saved.",
            )
        },
        "delete_machine" => |ctx| {
            let id = arg_str(ctx, "id");
            holds(
                move |a| a.doc.machines.iter().all(|m| m.id != id),
                "The machine was not deleted.",
            )
        },
        "delete_tool" => |ctx| {
            let id = arg_str(ctx, "id");
            holds(
                move |a| a.doc.tools.iter().all(|t| t.id != id),
                "The tool was not deleted.",
            )
        },
        "delete_material" => |ctx| {
            let id = arg_str(ctx, "id");
            holds(
                move |a| a.doc.materials.iter().all(|m| m.id != id),
                "The material was not deleted.",
            )
        },
        "delete_holder" => |ctx| {
            let id = arg_str(ctx, "id");
            holds(
                move |a| a.doc.workholding.iter().all(|h| h.id != id),
                "The entry was not deleted.",
            )
        },
        "new_tool" => |ctx| {
            let before = ctx.app.tool_draft.id.clone();
            holds(
                move |a| a.tool_draft.id != before && a.page == Page::Tools,
                "No tool draft was opened.",
            )
        },
        "edit_tool" => |ctx| {
            let id = arg_str(ctx, "id");
            holds(
                move |a| a.tool_draft.id == id,
                "The tool is not open in its editor.",
            )
        },
        "new_material" => |ctx| {
            let before = ctx.app.material_draft.id.clone();
            holds(
                move |a| a.material_draft.id != before && a.page == Page::Materials,
                "No material draft was opened.",
            )
        },
        "edit_material" => |ctx| {
            let id = arg_str(ctx, "id");
            holds(
                move |a| a.material_draft.id == id,
                "The material is not open in its editor.",
            )
        },
        "new_holder" => |ctx| {
            let (before, kind) = (ctx.app.holder_draft.id.clone(), arg_str(ctx, "kind"));
            holds(
                move |a| {
                    a.holder_draft.id != before && a.holder_draft.kind().eq_ignore_ascii_case(&kind)
                },
                "No workholding draft was opened.",
            )
        },
        "edit_holder" => |ctx| {
            let id = arg_str(ctx, "id");
            holds(
                move |a| a.holder_draft.id == id,
                "The entry is not open in its editor.",
            )
        },
        "tool_machine" => |ctx| {
            let (id, on) = (arg_str(ctx, "id"), flag(ctx, "checked"));
            holds(
                move |a| lists(crate::model::get(&a.tool_draft.p, "machines"), &id) == on,
                "The tool's machines did not change.",
            )
        },
        "holder_machine" => |ctx| {
            let (id, on) = (arg_str(ctx, "id"), flag(ctx, "checked"));
            holds(
                move |a| lists(crate::model::get(&a.holder_draft.p, "machines"), &id) == on,
                "The entry's machines did not change.",
            )
        },
        // View.
        "set_camera" | "orbit_camera" | "pan_camera" | "zoom_camera" => |ctx| {
            alters(
                ctx,
                |a| format!("{:?}", a.camera),
                "The view looks so already.",
            )
        },
        "section" => |ctx| {
            alters(
                ctx,
                |a| (a.section_on, a.picking_section, format!("{:?}", a.section)),
                "The section is so already.",
            )
        },
        "toggle_stock" => |ctx| flips(ctx, |a| a.stock),
        "toggle_uncut" => |ctx| flips(ctx, |a| a.show_uncut),
        "toggle_paths" => |ctx| flips(ctx, |a| a.paths),
        "toggle_sim_details" => |ctx| flips(ctx, |a| a.sim_details),
        "toggle_warning_details" => |ctx| flips(ctx, |a| a.warning_details),
        "toggle_about" => |ctx| flips(ctx, |a| a.settings.layout.strategy_help_open),
        "toggle_sidebar" | "sidebar_width" => |ctx| {
            alters(
                ctx,
                |a| a.settings.layout.clone(),
                "The sidebar is so already.",
            )
        },
        "stock_detail" => |ctx| {
            let level = arg_str(ctx, "level");
            holds(
                move |a| a.stock_detail.to_string().eq_ignore_ascii_case(&level),
                "The detail did not change.",
            )
        },
        "play" => |ctx| {
            alters(
                ctx,
                |a| (a.playing, a.simulation.is_some()),
                "Nothing plays.",
            )
        },
        "scrub" => |ctx| {
            let at = ctx
                .args
                .get("at")
                .and_then(Value::as_f64)
                .unwrap_or(0.)
                .clamp(0., 1.) as f32;
            holds(
                move |a| (a.progress - at).abs() < 1e-3,
                "The preview did not move there.",
            )
        },
        "playback_speed" => |ctx| {
            let x = ctx.args.get("x").and_then(Value::as_f64).unwrap_or(1.) as f32;
            holds(
                move |a| a.speed == x.clamp(0.5, 120.),
                "The speed did not change.",
            )
        },
        "control_tab" => |ctx| {
            let tab = CONTROL_TABS
                .iter()
                .find(|(n, _)| n.eq_ignore_ascii_case(&arg_str(ctx, "tab")))
                .map(|(_, t)| *t);
            holds(
                move |a| Some(a.control_tab) == tab,
                "The tab did not change.",
            )
        },
        "job_view" => |ctx| {
            let top = arg_str(ctx, "view").eq_ignore_ascii_case("top");
            holds(
                move |a| {
                    a.control_view
                        == if top {
                            ui::JobView::Top
                        } else {
                            ui::JobView::Stock
                        }
                },
                "The view did not change.",
            )
        },
        // Review.
        "open_audit_row" => |ctx| {
            let row = (ctx.args.get("index").and_then(Value::as_u64))
                .and_then(|i| ctx.app.audit_report.get(i as usize))
                .map(|r| (r.setup, r.operation));
            holds(
                move |a| row == Some((a.doc.index(), a.selected.unwrap_or(usize::MAX))),
                "The row's operation did not open.",
            )
        },
        "open_issue" => |ctx| {
            let at = address(ctx.app, ctx.args, true).ok().and_then(|(_, at)| at);
            holds(
                move |a| at == Some((a.doc.index(), a.selected.unwrap_or(usize::MAX))),
                "The operation did not open.",
            )
        },
        "steps_fix" => |ctx| {
            let op = (ctx.args.get("operation"))
                .and_then(|v| operation_of(ctx.app, ctx.setup, v).ok())
                .map(|(_, oi)| oi);
            holds(
                move |a| a.page == Page::Work && a.selected == op,
                "The editor did not open at the problem.",
            )
        },
        // The Run panel's fold-outs are its view on the Control page: sent
        // is all there is to it.
        "run_details" => |_| holds(|_| true, ""),
        // Export.
        "export_gcode" => |ctx| {
            let (folder, since) = (arg_path(ctx, "folder"), std::time::SystemTime::now());
            judged(
                "export_gcode",
                Kind::Gcode,
                &[],
                move |_| {
                    std::fs::read_dir(&folder).is_ok_and(|d| {
                        d.flatten().any(|e| {
                            e.path().extension().is_some_and(|x| x == "ngc")
                                && written_since(&e.path(), since)
                        })
                    })
                },
                "No program was written.",
            )
        },
        "export_setup_sheet" => |ctx| {
            let (path, since) = (arg_path(ctx, "path"), std::time::SystemTime::now());
            judged(
                "export_setup_sheet",
                Kind::Sheet,
                &[],
                move |_| written_since(&path, since),
                "The setup sheet was not written.",
            )
        },
        "save_for_tablet" => |ctx| {
            let (path, since) = (arg_path(ctx, "path"), std::time::SystemTime::now());
            judged(
                "save_for_tablet",
                Kind::Tablet,
                &[],
                move |_| written_since(&path, since),
                "The tablet file was not written.",
            )
        },
        // The stops are sent at once; what the machine did is M3's
        // machine_status.
        "hold" | "stop" | "estop" | "jog_release" => |_| holds(|_| true, ""),
        _ => return None,
    };
    Some(f)
}

/// Whether every message is one a command may send, else why not.
pub fn guard(messages: &[Message]) -> Result<(), &'static str> {
    for m in messages {
        if !matches!(class::class(m), Class::Command(_)) {
            return Err(class::refusal(m).unwrap_or(class::MACHINE));
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::mcp::engine::{self, Agent, Backend};
    use std::mem::discriminant;

    /// An `App` driven straight: each message's tasks run to their end as
    /// it is sent, so a command's work has settled when its call returns.
    /// The registry's messages, effects, refusals and file rules, without a
    /// session's thread.
    pub(crate) struct Direct {
        pub(crate) app: App,
        /// Where the tests write their files.
        pub(crate) folder: PathBuf,
        /// The session's own folder, which holds its workspace, in `folder`.
        pub(crate) session: PathBuf,
        pub(crate) agent: Option<Agent>,
        runtime: tokio::runtime::Runtime,
        _dir: tempfile::TempDir,
    }
    impl Direct {
        /// The messages `task` produces, run to its end.
        pub(crate) fn outputs(&self, task: Task<Message>) -> Vec<Message> {
            use futures_util::StreamExt;
            let Some(stream) = iced_runtime::task::into_stream(task) else {
                return vec![];
            };
            self.runtime.block_on(
                stream
                    .filter_map(|action| async move {
                        match action {
                            iced_runtime::Action::Output(m) => Some(m),
                            _ => None,
                        }
                    })
                    .collect(),
            )
        }
    }
    impl Backend for Direct {
        fn app(&self) -> &App {
            &self.app
        }
        fn app_mut(&mut self) -> &mut App {
            &mut self.app
        }
        fn send(&mut self, message: Message) {
            let task = self.app.update_by_agent(message);
            self.run(task);
        }
        fn run(&mut self, task: Task<Message>) {
            for m in self.outputs(task) {
                self.send(m);
            }
        }
        /// The edit pause taken as over, and the `Tick` that then saves the
        /// workspace and starts the checks, run to its end.
        fn quiet(&mut self) -> bool {
            let past = std::time::Instant::now().checked_sub(std::time::Duration::from_secs(1));
            if let Some(past) = past {
                self.app.last_edit = past;
            }
            self.send(Message::Tick);
            true
        }
        fn pending(&self) -> Vec<&'static str> {
            self.app.pending()
        }
        fn kind(&self) -> &'static str {
            "headless"
        }
        fn session(&self) -> Value {
            json!({"id": "test", "folder": self.session.display().to_string()})
        }
        fn session_folder(&self) -> Option<PathBuf> {
            Some(self.session.clone())
        }
    }
    /// The sample job, its first operation selected, in a folder of its own.
    pub(crate) fn direct() -> Direct {
        let dir = tempfile::tempdir().unwrap();
        let mut app = App::from_workspace(dir.path().join("session").join("workspace.json"), None);
        let mut doc = crate::model::Document::sample();
        doc.adopt_materials();
        doc.adopt_holders();
        app.replace_document(doc, None);
        app.selected = Some(0);
        app.audit_report.push(crate::AuditRow {
            setup: 0,
            operation: 0,
            name: "row".into(),
            ok: true,
            detail: String::new(),
        });
        // Edits settle at once here: the workspace is saved as it changes.
        app.dirty = false;
        let folder = dir.path().to_path_buf();
        let session = folder.join("session");
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        Direct {
            app,
            folder,
            session,
            agent: Some(Agent::new()),
            runtime,
            _dir: dir,
        }
    }
    /// A tool call, answered once its work is done.
    pub(crate) fn call(d: &mut Direct, tool: &str, args: Value) -> Result<Value, String> {
        let mut agent = d.agent.take().unwrap();
        let answer = engine::call_now(&mut agent, d, tool, args);
        d.agent = Some(agent);
        answer
    }
    pub(crate) fn run(d: &mut Direct, command: &str, args: Value) -> Result<Value, String> {
        call(d, "execute", json!({"command": command, "args": args}))
    }
    /// The example's placeholders filled from `d`'s libraries and folder.
    fn filled(d: &Direct, example: &Value) -> Value {
        let doc = &d.app.doc;
        // Escaped for the JSON text: a Windows folder has backslashes.
        let folder = Value::from(d.folder.display().to_string()).to_string();
        let text = example
            .to_string()
            .replace("<tool>", &doc.tools[0].id)
            .replace("<machine>", &doc.machines[0].id)
            .replace("<material>", &doc.materials[0].id)
            .replace("<holder>", &doc.workholding[0].id)
            .replace("/tmp", folder.trim_matches('"'));
        serde_json::from_str(&text).unwrap()
    }
    fn ctx<'a>(
        d: &'a Direct,
        c: &Command,
        args: &'a Value,
        fields: Option<Result<Vec<ui::FieldInfo>, String>>,
    ) -> Ctx<'a> {
        let (setup, operation) = c.resolve(&d.app, args).ok().unwrap_or_default();
        Ctx {
            app: &d.app,
            args,
            session: Some(&d.session),
            setup,
            operation,
            fields,
        }
    }
    /// What tells two messages of one command apart: the variant, and the
    /// variant of the action or event it carries, or the field it edits.
    fn shape(m: &Message) -> String {
        let inner = match m {
            Message::Section(e) => format!("{:?}", discriminant(e)),
            Message::Run(a) => format!("{:?}", discriminant(a)),
            Message::Machine(a) => format!("{:?}", discriminant(a)),
            Message::Guide(a) => format!("{:?}", discriminant(a)),
            Message::Viewport(e) => format!("{:?}", discriminant(e)),
            Message::Edit(t, k, _) => format!("{t:?}.{k}"),
            _ => String::new(),
        };
        format!("{:?}/{inner}", discriminant(m))
    }
    /// The messages of every command's examples, built on the sample.
    fn built(d: &mut Direct) -> Vec<(&'static str, Message)> {
        std::fs::write(d.folder.join("part.stl"), "solid x\nendsolid x\n").unwrap();
        crate::storage::save(&d.folder.join("job.axis.json"), &d.app.doc).unwrap();
        let mut all = vec![];
        for c in all_commands() {
            for example in c.example_args() {
                let mut args = filled(d, &example);
                if c.name == "import_model" {
                    args = json!({"path": d.folder.join("part.stl").display().to_string()});
                }
                let fields = (c.name == "edit_field").then(|| fields::of(&mut d.app, &args));
                let ctx = ctx(d, c, &args, fields);
                match c.built(&ctx) {
                    Ok(dispatch) => {
                        let outputs = dispatch.task.map(|t| d.outputs(t)).unwrap_or_default();
                        let clicks = dispatch.clicks.iter().flat_map(pick::Job::messages);
                        let messages = dispatch.messages.into_iter().chain(outputs).chain(clicks);
                        all.extend(messages.map(|m| (c.name, m)));
                    }
                    Err(Problem::Args(e) | Problem::Refused(e)) => {
                        panic!("{} {args} does not build: {e}", c.name)
                    }
                }
            }
        }
        all
    }
    fn all_commands() -> &'static [Command] {
        all()
    }

    /// One message of each variant a user sends, and of each action and
    /// event inside one (help.rs `every_card` lists them the same way).
    fn user_messages() -> Vec<Message> {
        use crate::{control::Action as Ctl, run::Action as Run};
        let mut m = vec![
            Message::NewProject,
            Message::ResumeProject,
            Message::OpenSample,
            Message::RecoverPrevious,
            Message::Page(Page::Work),
            Message::Page(Page::Control),
            Message::Page(Page::Ai),
            Message::SelectSetup(0),
            Message::SelectOp(0),
            Message::SetupEditor,
            Message::SetupEnd,
            Message::OperationsStep,
            Message::PreviewStep,
            Message::SetupStep("Stock"),
            Message::Tab("Heights".into()),
            Message::Edit(Target::Operation, "rpm", String::new()),
            Message::Edit(Target::Setup, "origin", String::new()),
            Message::Edit(Target::Operation, "pin_setup", String::new()),
            Message::Edit(Target::Setup, "pin_holes", String::new()),
            Message::Edit(Target::Setup, "pin_place", String::new()),
            Message::Edit(Target::Machine, "uc100_profile", String::new()),
            Message::Edit(Target::Machine, "pi_address", String::new()),
            Message::PutBackEdit,
            Message::Title(String::new()),
            Message::AddOp(Strategy::Pocket),
            Message::AddTurn("Outside"),
            Message::DuplicateOp,
            Message::RemoveOp,
            Message::AskRemoveOp,
            Message::ConfirmDeleteOp(true),
            Message::MoveOp(1),
            Message::ToggleOp,
            Message::Feature(0),
            Message::Surface(0),
            Message::DropHole(0),
            Message::ClearSurfaces,
            Message::BoxFromFace,
            Message::AllCorners,
            Message::PickMode,
            Message::PickPinHoles,
            Message::PinHoleSurface(0),
            Message::PlacePins,
            Message::AddPinDrilling,
            Message::AddThreading,
            Message::AddNeededTool,
            Message::AddHoleStages,
            Message::AddCentring,
            Message::ThreadFromPart,
            Message::Generate,
            Message::GenerateAll,
            Message::CancelGenerate,
            Message::AddSetup,
            Message::AddManualSetup,
            Message::AddLatheSetup,
            Message::AddSetupOn(String::new()),
            Message::DuplicateSetup,
            Message::RemoveSetup,
            Message::ConfirmDeleteSetup(true),
            Message::MoveSetup(1),
            Message::Flip("rx"),
            Message::TurnStock(1, 0),
            Message::SquareToStock,
            Message::AssignMachine(String::new()),
            Message::NewMachine,
            Message::AddRotaryMachine,
            Message::EditMachine(String::new()),
            Message::WizardStep(0),
            Message::SaveMachine,
            Message::DeleteMachine(String::new()),
            Message::ToolMachine(String::new(), true),
            Message::HolderMachine(String::new(), true),
            Message::BrowseUc100Profile,
            Message::ControlMachine(String::new()),
            Message::SwitchMachine(true),
            Message::ShowGuide,
            Message::MachineConnection("Simulated"),
            Message::PiAddress(String::new()),
            Message::PiCode(String::new()),
            Message::PiPair,
            Message::PiForget,
            Message::PiTakeControl,
            Message::Connect,
            Message::Disconnect,
            Message::MachineReset,
            Message::MachineEStop,
            Message::ControlSetup(true),
            Message::ControlTab(ui::ControlTab::Probe),
            Message::ControlView(ui::JobView::Top),
            Message::Setup(crate::setup::Msg::Save),
            Message::Setup(crate::setup::Msg::Revert),
            Message::Setup(crate::setup::Msg::UseCalibration),
            Message::Setup(crate::setup::Msg::Edit(crate::setup::Edit::Flag(
                String::new(),
                true,
            ))),
            Message::JogKey(0, false, false),
            Message::JogKeyUp,
            Message::HoldKey,
            Message::AuditRow(0),
            Message::OpenIssue(0, 0, String::new()),
            Message::RunOnMachine(0),
            Message::EditTool(String::new()),
            Message::NewTool,
            Message::SaveTool,
            Message::DeleteTool(String::new()),
            Message::EditMaterial(String::new()),
            Message::NewMaterial,
            Message::SaveMaterial,
            Message::DeleteMaterial(String::new()),
            Message::PickMaterial(String::new()),
            Message::NewHolder("Vise"),
            Message::EditHolder(String::new()),
            Message::SaveHolder,
            Message::DeleteHolder(String::new()),
            Message::SetClamps(String::new()),
            Message::SuggestLayout,
            Message::SetOpTool(String::new()),
            Message::RecommendSpeeds,
            Message::RestSource(String::new()),
            Message::SaveProject,
            Message::SaveProjectAs,
            Message::Saved(None),
            Message::SaveForTablet,
            Message::TabletSaved(None),
            Message::OpenProject,
            Message::OpenHistory,
            Message::ProjectOpened(None),
            Message::Import,
            Message::ImportPathOpen(true),
            Message::ImportPath(String::new()),
            Message::ImportFromPath,
            Message::Reimport,
            Message::PicksChecked,
            Message::Export("sheet"),
            Message::Exported(PathBuf::new()),
            Message::ExportError(String::new()),
            Message::ToggleStock,
            Message::ToggleUncut,
            Message::ToggleSimDetails,
            Message::ToggleWarningDetails,
            Message::TogglePaths,
            Message::Camera("TOP"),
            Message::ToggleSidebar(settings::Side::Browser),
            Message::SidebarWidth(settings::Side::Browser, 300., true),
            Message::ToggleStrategyHelp,
            Message::Play,
            Message::Scrub(0.5),
            Message::ScrubDone,
            Message::Speed(2.),
            Message::StockDetail(stock_surface::Detail::Fine),
            Message::ExportGcode,
            Message::GcodeExported(None),
        ];
        for e in [
            section::Edit::Toggle,
            section::Edit::Axis(0),
            section::Edit::PickFace,
            section::Edit::At(1.),
            section::Edit::Flip,
        ] {
            m.push(Message::Section(e));
        }
        for e in [
            viewport::Event::Orbit(0., 0., [0., 0.]),
            viewport::Event::Pan(0., 0.),
            viewport::Event::Zoom(0., [0., 0.]),
            viewport::Event::Pick(0),
            viewport::Event::PickSurface(0),
            viewport::Event::SectionSurface(0),
            viewport::Event::PlacePins([0., 0.]),
        ] {
            m.push(Message::Viewport(e));
        }
        for a in [
            Ctl::Home,
            Ctl::MachineZero(None),
            Ctl::KeepPosition,
            Ctl::JogPress(0, false),
            Ctl::JogRelease,
            Ctl::JogStep(None),
            Ctl::JogSpeed(50.),
            Ctl::WorkZero(None),
            Ctl::WorkHalf(0),
            Ctl::WorkSet(0),
            Ctl::GoTo(control::Go::All),
            Ctl::Wcs(0),
            Ctl::Spindle(uc100::gcode::Spindle::Cw),
            Ctl::Flood(true),
            Ctl::Mist(true),
            Ctl::FeedOverride(100.),
            Ctl::OverrideLimits(true),
            Ctl::MdiSend,
            Ctl::TestOutput(0, Some(true)),
            Ctl::CalMove,
            Ctl::ProbeZ,
            Ctl::ProbeEdges,
            Ctl::TouchOff,
            Ctl::ToSimZero,
            Ctl::RecordPins,
            Ctl::ReferencePins,
            Ctl::PinZero,
            Ctl::Calibrate(control::Calibrate::Centreline),
            Ctl::SaveCentreline,
            Ctl::Remounted,
        ] {
            m.push(Message::Machine(a));
        }
        for a in [
            Run::Setup(0),
            Run::Check,
            Run::Next(0),
            Run::From(0, 0),
            Run::KeepZero,
            Run::CycleStart,
            Run::Hold,
            Run::Resume,
            Run::Stop,
            Run::ProbeReference,
            Run::CoolantOff,
            Run::Skip(0, None),
            Run::More(Some(0)),
            Run::Steps(None),
            Run::PickStep(0, 0),
            Run::ClearSteps,
            Run::SkipSteps(true),
            Run::StartStep,
            Run::KeepToolZ,
            Run::StopAfterStep,
        ] {
            m.push(Message::Run(a));
        }
        for a in [
            GuideAction::Next,
            GuideAction::Back,
            GuideAction::Go(0),
            GuideAction::Section(0),
            GuideAction::Reached(0),
            GuideAction::Resume,
            GuideAction::Open(0),
            GuideAction::Fix(Some(0), String::new()),
            GuideAction::Typed(String::new(), String::new()),
            GuideAction::Measured(String::new(), 1.),
        ] {
            m.push(Message::Guide(a));
        }
        m
    }
    /// The user's messages a command stands for without sending them
    /// itself: confirmations it answers, dialogs whose work it does, and the
    /// workflow footer's steps, which select an operation.
    fn collapsed(m: &Message) -> bool {
        matches!(
            m,
            Message::AskRemoveOp
                | Message::ConfirmDeleteOp(_)
                | Message::OperationsStep
                | Message::PreviewStep
                | Message::SaveProjectAs
                | Message::SaveForTablet
                | Message::OpenProject
                | Message::OpenHistory
                | Message::Import
                | Message::ImportPathOpen(_)
                | Message::Export(_)
                | Message::ExportError(_)
                | Message::ExportGcode
                | Message::ScrubDone
                // The 3D view's click, which the pick commands send as the
                // message it becomes (`Feature`, `Surface`, `PinHoleSurface`).
                | Message::Viewport(viewport::Event::Pick(_) | viewport::Event::PickSurface(_))
        )
    }

    #[test]
    fn every_command_class_has_a_command_that_sends_it() {
        let mut d = direct();
        // A project with its file, which Save writes without a path.
        d.app.project_path = Some(d.folder.join("job.axis.json"));
        let built = built(&mut d);
        for m in user_messages() {
            let Class::Command(name) = class::class(&m) else {
                continue;
            };
            if LATER.iter().any(|(n, _)| *n == name) {
                continue;
            }
            assert!(find(name).is_some(), "{m:?} is {name}, which is no command");
            if collapsed(&m) {
                continue;
            }
            assert!(
                built
                    .iter()
                    .any(|(c, b)| *c == name && shape(b) == shape(&m)),
                "{name} never sends {m:?}"
            );
        }
        // And each command sends a message classified as itself, and only
        // messages a command may send.
        for c in all_commands() {
            assert!(
                built
                    .iter()
                    .any(|(n, b)| *n == c.name && class::class(b) == Class::Command(c.name)),
                "{} sends nothing of its own",
                c.name
            );
        }
        for (name, m) in &built {
            assert!(
                matches!(class::class(m), Class::Command(_)),
                "{name} sends {m:?}"
            );
        }
    }

    /// Every allowed command, with each of its examples, on a job whose
    /// setup holds the machinist's records: none changes them (the guard
    /// in `engine` would fail it), and those that replace the project are
    /// let through.
    #[test]
    fn no_allowed_command_changes_the_machinists_records() {
        for c in all_commands() {
            for example in c.example_args() {
                let mut d = direct();
                let doc = &mut d.app.doc;
                let id = doc.setups[0].id.clone();
                doc.guide.insert(id.clone(), 1);
                doc.measured
                    .insert(crate::guide::measure_key(&id, "zero"), 40.);
                doc.setups[0]
                    .machine
                    .p
                    .insert("cross_reads".into(), "Diameter".into());
                for m in &mut doc.machines {
                    m.p.insert("cross_reads".into(), "Diameter".into());
                }
                d.app
                    .machine_draft
                    .p
                    .insert("cross_reads".into(), "Diameter".into());
                std::fs::write(d.folder.join("part.stl"), "solid x\nendsolid x\n").unwrap();
                crate::storage::save(&d.folder.join("job.axis.json"), &d.app.doc).unwrap();
                let mut args = filled(&d, &example);
                if c.name == "import_model" {
                    args = json!({"path": d.folder.join("part.stl").display().to_string()});
                }
                let r = match run(&mut d, c.name, args.clone()) {
                    Ok(r) => r,
                    Err(_) => continue,
                };
                let reason = r["reason"].as_str().unwrap_or_default();
                assert!(
                    !reason.starts_with(crate::mcp::state::PROGRESS),
                    "{} {args}: {r}",
                    c.name
                );
            }
        }
    }

    #[test]
    fn every_command_has_help_and_a_schema() {
        let fallback = crate::help::command(&Message::Noop, String::new()).body;
        let mut names = std::collections::BTreeSet::new();
        for c in all_commands() {
            assert!(names.insert(c.name), "{} twice", c.name);
            let (line, full) = c.help();
            assert!(!line.is_empty() && full.len() >= line.len(), "{}", c.name);
            assert_ne!(full, fallback, "{} has only the generic help", c.name);
            assert!(!c.effect.is_empty() && !c.requires.is_empty(), "{}", c.name);
            assert!(engine::GROUPS.contains(&c.group), "{}", c.name);
            let schema = c.schema();
            assert_eq!(schema["type"], "object", "{}", c.name);
            let properties = schema["properties"].as_object().unwrap();
            // `set_pin_setup`'s `setup` is a value, not an address.
            if (c.on == On::Operation || c.name == "edit_field") && !c.own_setup {
                let setup = &properties.get("setup");
                let said = setup.and_then(|s| s["description"].as_str());
                assert!(
                    said.is_some_and(|d| d.contains("operation index counts in")),
                    "{}: no `setup` saying where an operation index counts",
                    c.name
                );
            }
            for p in properties.values() {
                assert!(
                    p.get("type").is_some() || p.get("enum").is_some(),
                    "{}: {p}",
                    c.name
                );
            }
            for r in schema["required"].as_array().unwrap() {
                assert!(
                    properties.contains_key(r.as_str().unwrap()),
                    "{}: {r}",
                    c.name
                );
            }
            for example in c.example_args() {
                for key in example.as_object().unwrap().keys() {
                    assert!(properties.contains_key(key), "{}: example's {key}", c.name);
                }
            }
        }
        for (name, ..) in REFUSED {
            assert!(find(name).is_none(), "{name} is both allowed and refused");
        }
    }

    #[test]
    fn every_machine_message_is_refused_with_the_boundary_sentence() {
        for m in user_messages() {
            match class::class(&m) {
                Class::Machine => {
                    let why = guard(std::slice::from_ref(&m)).unwrap_err();
                    let expected = if matches!(m, Message::Guide(_)) {
                        class::SHOP_FLOOR
                    } else {
                        class::MACHINE
                    };
                    assert_eq!(why, expected, "{m:?}");
                }
                Class::Planner => assert_eq!(guard(&[m]).unwrap_err(), class::PLANNER),
                Class::Device => assert_eq!(guard(&[m]).unwrap_err(), class::DEVICE),
                Class::Command(_) | Class::Internal => {}
            }
        }
        let mut d = direct();
        for (name, _, reason) in REFUSED {
            let r = run(&mut d, name, json!({})).unwrap();
            assert_eq!(r["outcome"], "refused", "{name}");
            assert_eq!(r["reason"], reason, "{name}");
        }
        let control = run(&mut d, "goto_page", json!({"page": "Control"})).unwrap();
        assert_eq!(control["reason"], class::MACHINE);
        assert_ne!(d.app.page, Page::Control);
        for key in ["uc100_profile", "uc100_connection", "pi_address"] {
            let args = json!({"target": "machine", "key": key, "value": "x"});
            let r = run(&mut d, "edit_field", args).unwrap();
            assert_eq!(r["outcome"], "refused", "{key}");
            assert!(
                r["reason"].as_str().unwrap().ends_with(class::MACHINE),
                "{key}"
            );
        }
        let dial = json!({"target": "machine", "key": "cross_reads", "value": "Radius"});
        let r = run(&mut d, "edit_field", dial).unwrap();
        assert!(
            r["reason"].as_str().unwrap().ends_with(class::SHOP_FLOOR),
            "{r}"
        );
        // With no machine connected the stops have nothing to stop.
        assert_eq!(
            run(&mut d, "estop", json!({})).unwrap()["outcome"],
            "refused"
        );
    }

    #[test]
    fn on_a_connected_simulated_uc100_the_boundary_holds() {
        let mut d = direct();
        let profile = d.folder.join("uc100.toml");
        std::fs::write(&profile, uc100::config::TEMPLATE).unwrap();
        let mut m = crate::model::Machine::new(3);
        m.p.insert("name".into(), "JY5800".into());
        m.p.insert("controller".into(), "UC100".into());
        m.p.insert("uc100_profile".into(), profile.display().to_string());
        m.p.insert("uc100_connection".into(), "Simulated".into());
        let id = m.id.clone();
        d.app.doc.machines.push(m.clone());
        let _ = d.app.update(Message::Connect);
        assert!(d.app.control.is_some(), "{}", d.app.status);
        for (name, _, reason) in REFUSED {
            let r = run(&mut d, name, json!({})).unwrap();
            assert_eq!(
                (r["outcome"].as_str(), r["reason"].as_str()),
                (Some("refused"), Some(reason))
            );
        }
        d.app.machine_draft = m;
        let save = run(&mut d, "save_machine", json!({})).unwrap();
        assert_eq!(save["outcome"], "refused");
        assert!(save["reason"].as_str().unwrap().contains("connected"));
        let delete = run(&mut d, "delete_machine", json!({"id": id})).unwrap();
        assert_eq!(delete["outcome"], "refused");
        assert!(d.app.doc.machines.iter().any(|m| m.id == id));
        // The profile is not a file the agent may write.
        let onto = json!({"path": profile.display().to_string(), "overwrite": true});
        let r = run(&mut d, "export_setup_sheet", onto).unwrap();
        assert_eq!(r["outcome"], "refused");
        assert!(r["reason"].as_str().unwrap().contains("UC100 profile"));
        assert_eq!(
            std::fs::read_to_string(&profile).unwrap(),
            uc100::config::TEMPLATE
        );
        // The stops go through: the machine is connected.
        let stop = run(&mut d, "jog_release", json!({})).unwrap();
        assert_ne!(stop["outcome"], "refused", "{stop}");
    }

    /// The literal `Message::Edit(Target::X, "key"` pairs of a source.
    fn edit_literals(source: &str) -> Vec<(String, String)> {
        let mut found = vec![];
        for (at, _) in source.match_indices("Message::Edit(") {
            let rest = source[at + "Message::Edit(".len()..].trim_start();
            let rest = rest.strip_prefix("crate::").unwrap_or(rest);
            let Some(rest) = rest.strip_prefix("Target::") else {
                continue;
            };
            let target: String = rest.chars().take_while(|c| c.is_alphanumeric()).collect();
            let rest = rest[target.len()..].trim_start();
            let Some(rest) = rest.strip_prefix(',').map(str::trim_start) else {
                continue;
            };
            let Some(rest) = rest.strip_prefix('"') else {
                continue;
            };
            let key: String = rest.chars().take_while(|c| *c != '"').collect();
            found.push((target, key));
        }
        found
    }
    /// Where a field was listed: its target, the setup and the operation (by
    /// index, which two runs of a scene share), and the field.
    type Listed = (Target, usize, usize, ui::FieldInfo);
    /// The arguments that address a target's fields at (`si`, `oi`).
    fn address(d: &Direct, target: Target, si: usize, oi: usize) -> Value {
        let name = (EDIT_TARGETS.iter().find(|(_, t)| *t == target)).unwrap().0;
        match target {
            Target::Setup => json!({"target": name, "setup": si}),
            Target::Operation => {
                let id = d.app.doc.setups[si].operations[oi].id.clone();
                json!({"target": name, "operation": id})
            }
            _ => json!({"target": name}),
        }
    }
    /// The fields of every target, as `list_fields` gives them.
    fn all_fields(d: &mut Direct) -> Vec<Listed> {
        let mut found = vec![];
        let mut add = |d: &mut Direct, target: Target, si: usize, oi: usize, args: Value| {
            for f in fields::of(&mut d.app, &args).unwrap_or_default() {
                found.push((target, si, oi, f));
            }
        };
        for si in 0..d.app.doc.setups.len() {
            add(
                d,
                Target::Setup,
                si,
                0,
                json!({"target": "setup", "setup": si}),
            );
            for oi in 0..d.app.doc.setups[si].operations.len() {
                let args = address(d, Target::Operation, si, oi);
                add(d, Target::Operation, si, oi, args);
            }
        }
        for (name, target) in &EDIT_TARGETS[2..] {
            add(d, *target, 0, 0, json!({"target": name}));
        }
        found
    }
    /// Every key the fields of `scene` list, and while each of its pick
    /// lists is set to each choice in turn (a choice brings its own fields: a
    /// tool's type, a stock's shape, a thread's kind), put back after.
    fn reached(scene: &dyn Fn(&mut Direct), keys: &mut std::collections::BTreeSet<String>) {
        let mut d = direct();
        scene(&mut d);
        let first = all_fields(&mut d);
        keys.extend(first.iter().map(|(.., f)| f.key.to_string()));
        for (target, si, oi, f) in first {
            for choice in f.choices.iter().flatten() {
                let edits = |d: &mut Direct, value: &str| {
                    d.send(Message::SelectSetup(si));
                    if target == Target::Operation {
                        d.send(Message::SelectOp(oi));
                    }
                    d.send(Message::Edit(target, f.key, value.to_string()));
                };
                edits(&mut d, choice);
                let args = address(&d, target, si, oi);
                let after = fields::of(&mut d.app, &args).unwrap_or_default();
                keys.extend(after.iter().map(|f| f.key.to_string()));
                edits(&mut d, &f.value);
            }
        }
    }
    /// A part loaded from `format`, not the sample block.
    fn loaded(d: &mut Direct, format: &str) {
        d.app.doc.sample_model = false;
        d.app.doc.file = Some(crate::model::ModelFile {
            name: format!("part.{format}"),
            path: PathBuf::from(format!("part.{format}")),
            format: format.into(),
            bytes: 1,
            thickness: None,
        });
    }
    /// Operations of every strategy, of each Drill cycle and of each Turn kind
    /// in the active setup.
    fn every_strategy(d: &mut Direct) {
        use crate::model::{Operation, Strategy};
        let tool = d.app.doc.tools[0].id.clone();
        let mut ops: Vec<Operation> = Strategy::ALL
            .into_iter()
            .map(|s| Operation::new(s, tool.clone()))
            .collect();
        for cycle in [
            crate::model::THREAD_MILLING,
            crate::model::TAPPING,
            crate::model::REAMING,
            crate::model::BORING,
        ] {
            let mut o = Operation::new(Strategy::Drill, tool.clone());
            o.p.insert("cycle".into(), cycle.into());
            ops.push(o);
        }
        for kind in crate::lathe::TURN_KINDS {
            let mut o = Operation::new(Strategy::Turn, tool.clone());
            o.set_turn_kind(kind);
            ops.push(o);
        }
        // A Chamfer along a line, sized by its tip's depth, and a Drill of
        // the pin holes.
        let mut line = Operation::new(Strategy::Chamfer, tool.clone());
        line.p
            .insert("chamfer_source".into(), crate::chamfer::SOURCES[1].into());
        line.p
            .insert("chamfer_size".into(), crate::chamfer::SIZES[1].into());
        let mut pins = Operation::new(Strategy::Drill, tool);
        pins.p
            .insert("holes".into(), crate::model::DRILL_HOLES[1].into());
        ops.extend([line, pins]);
        d.app.doc.setup_mut().operations.extend(ops);
    }
    /// A fixture: a project made from the sample.
    type Scene = Box<dyn Fn(&mut Direct)>;
    /// The scenes the editors' fields are listed on: the sample, and fixtures
    /// with every kind of setup, an operation of every strategy and a
    /// draft of every kind, each also as a part loaded from a file.
    fn scenes() -> Vec<Scene> {
        let mut all: Vec<Scene> = vec![
            Box::new(|_| {}),
            Box::new(every_strategy),
            Box::new(|d| {
                for machine in 1..3 {
                    let id = d.app.doc.machines[machine].id.clone();
                    run(d, "add_setup", json!({"machine": id})).unwrap();
                    every_strategy(d);
                }
            }),
            Box::new(|d| {
                run(d, "add_manual_setup", json!({})).unwrap();
                every_strategy(d);
            }),
            Box::new(|d| {
                run(d, "add_lathe_setup", json!({})).unwrap();
                every_strategy(d);
            }),
            Box::new(|d| {
                run(d, "new_machine", json!({})).unwrap();
                run(d, "new_tool", json!({})).unwrap();
                run(d, "new_material", json!({})).unwrap();
            }),
            Box::new(|d| {
                let s = d.app.doc.setup_mut();
                s.p.insert("stock_type".into(), crate::sheet::SHEET.into());
                s.p.insert("holddown".into(), "Screws".into());
            }),
            // A part on the fixture pins, by its own holes and by holes in
            // its stock's leftover: the Work zero step's pin_holes and
            // pin_place.
            Box::new(|d| pinned(d, crate::post::fixture::pinned_flip("rx", true))),
            Box::new(|d| pinned(d, crate::post::fixture::leftover_flip("rx", None))),
        ];
        for kind in crate::holding::KINDS {
            all.push(Box::new(move |d| {
                run(d, "new_holder", json!({"kind": kind})).unwrap();
            }));
        }
        all
    }
    /// Field keys that no fixture lists, and why.
    const KEYS_UNREACHED: [(&str, &str); 10] = [
        (
            "cal_distance",
            "controller setup: the machine's own (class::MACHINE)",
        ),
        (
            "cal_measured",
            "controller setup: the machine's own (class::MACHINE)",
        ),
        (
            "measured",
            "the Control page's rotary calibration: the machine's own",
        ),
        (
            "control_rpm",
            "the Control page's spindle speed: the machine's own",
        ),
        ("mdi_line", "the Control page's MDI line: the machine's own"),
        (
            "move_target",
            "the Control page's Move there: the machine's own",
        ),
        (
            "work_set",
            "the Control page's Work zero: the machine's own",
        ),
        (
            "rotary_bar",
            "the Control page's rotary probing: the machine's own",
        ),
        (
            "rotary_spacing",
            "the Control page's rotary probing: the machine's own",
        ),
        (
            "turn_kind",
            "chosen when the Turn is added (`add_turn`); no editor field sends it",
        ),
    ];
    /// Keys whose edit is a message of its own, and the command that sends it.
    const KEYS_COVERED: [(&str, &str); 2] =
        [("clamps", "set_clamps"), ("machines", "tool_machine")];

    #[test]
    fn every_field_key_is_listed_or_covered_by_a_command() {
        let mut listed = std::collections::BTreeSet::new();
        for scene in scenes() {
            reached(scene.as_ref(), &mut listed);
            // The same as a part loaded from a file, which has more fields.
            let loaded_part = |d: &mut Direct| {
                scene(d);
                loaded(d, "stl");
            };
            reached(&loaded_part, &mut listed);
        }
        // A DXF part is a flat profile with a thickness of its own.
        reached(&|d| loaded(d, "dxf"), &mut listed);
        let mut literals = edit_literals(include_str!("../ui.rs"));
        literals.extend(edit_literals(include_str!("../main.rs")));
        assert!(literals.len() >= 10, "{literals:?}");
        let mut wanted: Vec<(String, String)> = crate::help::tests::field_keys()
            .into_iter()
            .map(|k| ("help".into(), k))
            .collect();
        wanted.extend(literals);
        let mut missing = vec![];
        for (from, key) in wanted {
            // The controller's fields and the dial reading are the user's:
            // refused, not listed.
            let user = class::machine_key(&key) || key == "cross_reads";
            let own = !matches!(
                class::edit(Target::Setup, &key),
                Class::Command("edit_field") | Class::Machine
            );
            let covered = KEYS_COVERED.iter().any(|(k, _)| *k == key);
            if user
                || listed.contains(&key)
                || own
                || covered
                || KEYS_UNREACHED.iter().any(|(k, _)| *k == key)
            {
                continue;
            }
            missing.push(format!("{from}: {key}"));
        }
        missing.sort();
        missing.dedup();
        assert!(missing.is_empty(), "not listed anywhere: {missing:#?}");
        for (key, _) in KEYS_UNREACHED {
            assert!(!listed.contains(key), "{key} is listed after all");
        }
        for (key, command) in KEYS_COVERED {
            assert!(find(command).is_some(), "{key}: no command {command}");
            assert!(!listed.contains(key), "{key} is listed after all");
        }
        // Keys with a command of their own are listed, with the command.
        for key in ["origin", "pin_setup", "pin_holes", "pin_place"] {
            assert!(listed.contains(key), "{key}");
        }
    }

    #[test]
    fn paths_follow_the_rules() {
        let mut d = direct();
        let refused = |r: Value| -> String {
            assert_eq!(r["outcome"], "refused", "{r}");
            r["reason"].as_str().unwrap().to_string()
        };
        let sheet = |path: &str| json!({"path": path});
        assert!(
            refused(run(&mut d, "export_setup_sheet", sheet("sheet.md")).unwrap())
                .contains("absolute")
        );
        let up = d.folder.join("x").join("..").join("sheet.md");
        assert!(
            refused(
                run(
                    &mut d,
                    "export_setup_sheet",
                    sheet(&up.display().to_string())
                )
                .unwrap()
            )
            .contains("..")
        );
        let inside = data_folder().join("mcp-sheet-test.md");
        let r = run(
            &mut d,
            "export_setup_sheet",
            sheet(&inside.display().to_string()),
        )
        .unwrap();
        assert!(refused(r).contains("data folder"));
        assert!(!inside.exists());
        let there = d.folder.join("there.md");
        std::fs::write(&there, "kept").unwrap();
        let r = run(
            &mut d,
            "export_setup_sheet",
            sheet(&there.display().to_string()),
        )
        .unwrap();
        assert!(refused(r).contains("overwrite"));
        assert_eq!(std::fs::read_to_string(&there).unwrap(), "kept");
        let r = run(
            &mut d,
            "export_setup_sheet",
            json!({"path": there.display().to_string(), "overwrite": true}),
        )
        .unwrap();
        assert_eq!(r["outcome"], "done", "{r}");
        assert_ne!(std::fs::read_to_string(&there).unwrap(), "kept");
        // A machine's UC100 profile is never written.
        let profile = d.folder.join("profile.toml");
        std::fs::write(&profile, "kept").unwrap();
        d.app.doc.machines[0]
            .p
            .insert("uc100_profile".into(), profile.display().to_string());
        let r = run(
            &mut d,
            "save_project",
            json!({"path": profile.display().to_string(), "overwrite": true}),
        )
        .unwrap();
        assert!(refused(r).contains("UC100 profile"));
        assert_eq!(std::fs::read_to_string(&profile).unwrap(), "kept");
        // Nothing in the data folder is written by a path, a headless
        // session's own folder included.
        let session = data_folder().join("mcp-sessions").join("s");
        let args = json!({});
        let c = Ctx {
            app: &d.app,
            args: &args,
            session: Some(&session),
            setup: None,
            operation: None,
            fields: None,
        };
        assert!(may_write(&c, &session.join("job.axis.json"), None).is_err());
        assert!(may_write(&c, &data_folder().join("workspace.json"), None).is_err());
        assert!(may_write(&c, &d.folder.join("job.axis.json"), None).is_ok());
    }

    /// The reviewer's probe (LIO-470): an export onto the session's own
    /// `workspace.json` with overwrite replaced the job with Markdown. No
    /// path argument writes into the session's folder: its workspace,
    /// `session.json`, lock, checkpoints or anything new there.
    #[test]
    fn writes_into_the_sessions_own_folder_are_refused() {
        let mut d = direct();
        std::fs::create_dir_all(d.session.join("checkpoints")).unwrap();
        crate::storage::save(&d.session.join("workspace.json"), &d.app.doc).unwrap();
        std::fs::write(d.session.join("session.json"), "{}").unwrap();
        std::fs::write(d.session.join("lock"), "{}").unwrap();
        let before = |name: &str| std::fs::read(d.session.join(name)).ok();
        let kept: Vec<_> = ["workspace.json", "session.json", "lock"]
            .map(before)
            .into();
        for name in [
            "workspace.json",
            "session.json",
            "lock",
            "settings.json",
            "checkpoints/c.json",
            "sheet.md",
        ] {
            let path = d.session.join(name).display().to_string();
            for (command, args) in [
                (
                    "export_setup_sheet",
                    json!({"path": path, "overwrite": true}),
                ),
                ("save_project", json!({"path": path, "overwrite": true})),
            ] {
                let r = run(&mut d, command, args).unwrap();
                assert_eq!(r["outcome"], "refused", "{command} {name}: {r}");
                assert!(r["reason"].as_str().unwrap().contains("data folder"), "{r}");
            }
        }
        let folder = d.session.display().to_string();
        let r = run(
            &mut d,
            "export_gcode",
            json!({"folder": folder, "overwrite": true}),
        )
        .unwrap();
        assert_eq!(r["outcome"], "refused", "{r}");
        let now: Vec<_> = ["workspace.json", "session.json", "lock"]
            .map(|n| std::fs::read(d.session.join(n)).ok())
            .into();
        assert_eq!(now, kept, "nothing of the session's was written");
        assert!(!d.session.join("sheet.md").exists());
        assert!(!d.session.join("checkpoints").join("c.json").exists());
        // An ordinary place outside is written as before.
        let sheet = d.folder.join("sheet.md").display().to_string();
        let r = run(&mut d, "export_setup_sheet", json!({"path": sheet})).unwrap();
        assert_eq!(r["outcome"], "done", "{r}");
    }

    /// The reviewer's probe (LIO-470): the user's workspace opened as a
    /// project, retitled and saved with no path overwrote it, though a save
    /// to the same path was refused. Opening the app's own files is refused;
    /// an archive in Project history opens, and a save without a path is
    /// then refused (it needs one outside), leaving the archive as it was.
    #[test]
    fn the_apps_own_files_are_never_saved_over_by_a_save_without_a_path() {
        let mut d = direct();
        // The session's own workspace, as the probe opened the user's.
        let workspace = d.session.join("workspace.json");
        crate::storage::save(&workspace, &d.app.doc).unwrap();
        let open = |p: &Path| json!({"path": p.display().to_string(), "discard_unsaved": true});
        let r = run(&mut d, "open_project", open(&workspace)).unwrap();
        assert_eq!(r["outcome"], "refused", "{r}");
        assert!(
            r["reason"].as_str().unwrap().contains("app's own files"),
            "{r}"
        );
        // An archive in Project history opens, read-only.
        let mut archived = d.app.doc.clone();
        archived.title = "Archived job".into();
        let archive = d.session.join("history").join("Archived-1.axis.json");
        crate::storage::save(&archive, &archived).unwrap();
        let bytes = std::fs::read(&archive).unwrap();
        let r = run(&mut d, "open_project", open(&archive)).unwrap();
        assert_eq!(r["outcome"], "done", "{r}");
        assert_eq!(d.app.doc.title, "Archived job");
        let title = json!({"title": "OVERWROTE USER WORKSPACE"});
        assert_eq!(
            run(&mut d, "set_project_title", title).unwrap()["outcome"],
            "done"
        );
        // The app takes an archive of its own history for a recovery, with
        // no file of its own; one of the user's history (the data folder's)
        // is the project's file, and is saved over by no save without a
        // path either: as here, the archive the project's file.
        let r = run(&mut d, "save_project", json!({})).unwrap();
        assert_eq!(r["outcome"], "refused", "{r}");
        d.app.project_path = Some(archive.clone());
        let r = run(&mut d, "save_project", json!({})).unwrap();
        assert_eq!(r["outcome"], "refused", "{r}");
        let why = r["reason"].as_str().unwrap();
        assert!(why.contains("data folder") && why.contains("`path`"), "{r}");
        let r = run(
            &mut d,
            "save_project",
            json!({"path": archive.display().to_string(), "overwrite": true}),
        )
        .unwrap();
        assert_eq!(r["outcome"], "refused", "{r}");
        assert_eq!(
            std::fs::read(&archive).unwrap(),
            bytes,
            "the archive is as it was"
        );
        // Saved elsewhere, it is the project's file, and saves again there.
        let elsewhere = d.folder.join("job.axis.json");
        let r = run(
            &mut d,
            "save_project",
            json!({"path": elsewhere.display().to_string()}),
        )
        .unwrap();
        assert_eq!(r["outcome"], "done", "{r}");
        let r = run(&mut d, "save_project", json!({})).unwrap();
        assert_eq!(r["outcome"], "done", "{r}");
        assert_eq!(
            crate::storage::load(&elsewhere).unwrap().title,
            "OVERWROTE USER WORKSPACE"
        );
        // A project whose own file became a machine's UC100 profile is
        // not saved over it either.
        d.app.doc.machines[0]
            .p
            .insert("uc100_profile".into(), elsewhere.display().to_string());
        let before = std::fs::read(&elsewhere).unwrap();
        let title = json!({"title": "Changed again"});
        assert_eq!(
            run(&mut d, "set_project_title", title).unwrap()["outcome"],
            "done"
        );
        let r = run(&mut d, "save_project", json!({})).unwrap();
        assert_eq!(r["outcome"], "refused", "{r}");
        assert!(
            r["reason"].as_str().unwrap().contains("UC100 profile"),
            "{r}"
        );
        assert_eq!(std::fs::read(&elsewhere).unwrap(), before);
    }

    /// `save_project` never writes over a file that holds another project
    /// now (LIO-470, round 2): not without a path, and not by the project's
    /// own path without overwrite.
    #[test]
    fn save_project_never_writes_over_another_project() {
        let mut d = direct();
        let file = d.folder.join("job.axis.json");
        let path = file.display().to_string();
        let r = run(&mut d, "save_project", json!({"path": path})).unwrap();
        assert_eq!(r["outcome"], "done", "{r}");
        let mut other: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        other["project_id"] = json!("another-project");
        other["title"] = json!("Another");
        let bytes = serde_json::to_vec(&other).unwrap();
        std::fs::write(&file, &bytes).unwrap();
        let r = run(&mut d, "save_project", json!({})).unwrap();
        assert_eq!(r["outcome"], "refused", "{r}");
        assert_eq!(
            r["reason"],
            format!(
                "The project file {} holds another project; save with a path.",
                file.display()
            )
        );
        let r = run(&mut d, "save_project", json!({"path": path})).unwrap();
        assert_eq!(r["outcome"], "refused", "{r}");
        assert!(
            r["reason"].as_str().unwrap().contains("overwrite: true"),
            "{r}"
        );
        assert_eq!(std::fs::read(&file).unwrap(), bytes, "as it was");
        let r = run(
            &mut d,
            "save_project",
            json!({"path": path, "overwrite": true}),
        )
        .unwrap();
        assert_eq!(r["outcome"], "done", "{r}");
        let saved = crate::storage::load(&file).unwrap();
        assert_eq!(saved.project_id, d.app.doc.project_id);
    }

    /// The reviewer's probe (LIO-470, round 3): the project file replaced
    /// by another project and made unreadable; both save forms said done
    /// and destroyed it. An unreadable file may hold another project.
    #[cfg(unix)]
    #[test]
    fn save_project_never_writes_over_an_unreadable_file() {
        use std::os::unix::fs::PermissionsExt;
        let mut d = direct();
        let file = d.folder.join("job.axis.json");
        let path = file.display().to_string();
        let r = run(&mut d, "save_project", json!({"path": path})).unwrap();
        assert_eq!(r["outcome"], "done", "{r}");
        let mut other: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        other["project_id"] = json!("another-project");
        let bytes = serde_json::to_vec(&other).unwrap();
        std::fs::write(&file, &bytes).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o000)).unwrap();
        // Root reads it anyway: nothing to probe then.
        if std::fs::read(&file).is_ok() {
            return;
        }
        let r = run(&mut d, "save_project", json!({})).unwrap();
        assert_eq!(r["outcome"], "refused", "{r}");
        assert_eq!(
            r["reason"],
            format!(
                "The project file {} cannot be read, so it may hold another project; save with a path, or with overwrite: true.",
                file.display()
            )
        );
        let r = run(&mut d, "save_project", json!({"path": path})).unwrap();
        assert_eq!(r["outcome"], "refused", "{r}");
        assert!(
            r["reason"].as_str().unwrap().contains("overwrite: true"),
            "{r}"
        );
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), bytes, "as it was");
    }

    /// The reviewer's probe (LIO-470, round 3): `new_project` went by a
    /// cached dirty flag, so a saved file changed or removed outside
    /// dropped the job unasked. It goes by the file as it is now.
    #[test]
    fn replacing_the_project_goes_by_the_file_as_it_is_now() {
        let mut d = direct();
        let file = d.folder.join("job.axis.json");
        let path = file.display().to_string();
        let r = run(&mut d, "save_project", json!({"path": path})).unwrap();
        assert_eq!(r["outcome"], "done", "{r}");
        let original = std::fs::read(&file).unwrap();
        let setups = d.app.doc.setups.len();
        assert!(setups > 0);
        let intact = |d: &Direct| {
            assert_eq!(d.app.doc.setups.len(), setups);
            assert_eq!(d.app.project_path.as_deref(), Some(file.as_path()));
        };
        // Changed outside.
        let mut changed = crate::storage::load(&file).unwrap();
        changed.title = "Changed elsewhere".into();
        crate::storage::save(&file, &changed).unwrap();
        let r = run(&mut d, "new_project", json!({})).unwrap();
        assert_eq!(r["outcome"], "refused", "{r}");
        intact(&d);
        for tool in ["open_sample", "recover_previous_project"] {
            let r = run(&mut d, tool, json!({})).unwrap();
            assert_eq!(r["outcome"], "refused", "{tool}: {r}");
            intact(&d);
        }
        // Removed.
        std::fs::remove_file(&file).unwrap();
        let r = run(&mut d, "new_project", json!({})).unwrap();
        assert_eq!(r["outcome"], "refused", "{r}");
        intact(&d);
        // Put back as it was: saved, so replacing is allowed.
        std::fs::write(&file, &original).unwrap();
        std::fs::remove_file(&file).unwrap();
        let r = run(&mut d, "new_project", json!({"discard_unsaved": true})).unwrap();
        assert_eq!(r["outcome"], "done", "{r}");
        assert!(d.app.doc.setups.is_empty());
    }

    #[test]
    fn a_saved_project_opens_again() {
        let mut d = direct();
        let file = d.folder.join("job.axis.json");
        let path = file.display().to_string();
        assert_eq!(
            run(&mut d, "set_project_title", json!({"title": "Saved title"})).unwrap()["outcome"],
            "done"
        );
        let saved = run(&mut d, "save_project", json!({"path": path})).unwrap();
        assert_eq!(saved["outcome"], "done", "{saved}");
        assert_eq!(d.app.project_path.as_deref(), Some(file.as_path()));
        assert!(!d.app.project_dirty);
        // Saving again needs no path, and no overwrite for its own file.
        assert_eq!(
            run(&mut d, "save_project", json!({})).unwrap()["outcome"],
            "done"
        );
        assert_eq!(
            run(&mut d, "save_project", json!({"path": path})).unwrap()["outcome"],
            "done"
        );
        run(&mut d, "set_project_title", json!({"title": "Changed"})).unwrap();
        // The change is not saved: opening refuses to drop it unless told.
        let open = json!({"path": path});
        assert_eq!(
            run(&mut d, "open_project", open).unwrap()["outcome"],
            "refused"
        );
        assert_eq!(d.app.doc.title, "Changed");
        let open = json!({"path": path, "discard_unsaved": true});
        let opened = run(&mut d, "open_project", open).unwrap();
        assert_eq!(opened["outcome"], "done", "{opened}");
        assert_eq!(d.app.doc.title, "Saved title");
        assert_eq!(d.app.project_path.as_deref(), Some(file.as_path()));
    }

    #[test]
    fn every_command_has_an_effect_to_check() {
        for c in all_commands() {
            // A pick command checks each click as it makes it (`pick`).
            assert!(
                c.expect.is_some() != c.clicks_checked(),
                "{} has no expected effect",
                c.name
            );
        }
    }

    /// A move of more places is the Up or Down clicks that make it, one
    /// place each: the app swaps an item with the one `delta` away, which
    /// for two places would swap past the one between. A move out of the
    /// list is refused.
    #[test]
    fn a_move_goes_one_place_a_click() {
        let mut d = direct();
        let order = |d: &Direct| -> Vec<String> {
            (d.app.doc.setup().operations.iter())
                .map(|o| format!("{:?}", o.strategy))
                .collect()
        };
        assert_eq!(order(&d), ["Face", "Adaptive", "Contour", "Drill"]);
        let moved = run(
            &mut d,
            "move_operation",
            json!({"operation": 3, "delta": -2}),
        )
        .unwrap();
        assert_eq!(moved["outcome"], "done", "{moved}");
        assert_eq!(order(&d), ["Face", "Drill", "Adaptive", "Contour"]);
        assert_eq!(d.app.selected, Some(1));
        let moved = run(
            &mut d,
            "move_operation",
            json!({"operation": 1, "delta": 2}),
        )
        .unwrap();
        assert_eq!(moved["outcome"], "done", "{moved}");
        assert_eq!(order(&d), ["Face", "Adaptive", "Contour", "Drill"]);
        for delta in [-1, 4] {
            let out = run(
                &mut d,
                "move_operation",
                json!({"operation": 0, "delta": delta}),
            )
            .unwrap();
            assert_eq!(out["outcome"], "refused", "{out}");
        }
        assert_eq!(order(&d), ["Face", "Adaptive", "Contour", "Drill"]);
        let out = run(&mut d, "move_setup", json!({"delta": 1})).unwrap();
        assert_eq!(out["outcome"], "refused", "{out}");
    }

    /// A centre drilling is offered as the app offers it: before a manual
    /// mill's drill only. The sample's CNC drill and its Face are refused.
    #[test]
    fn add_centring_is_for_a_manual_mill_drill() {
        let mut d = direct();
        for op in [3, 0] {
            let out = run(&mut d, "add_centring", json!({"operation": op})).unwrap();
            assert_eq!(out["outcome"], "refused", "{out}");
        }
        assert_eq!(d.app.doc.setup().operations.len(), 4);
    }

    /// The fixture block imported, its job's material chosen.
    pub(crate) fn imported() -> Direct {
        let mut d = direct();
        let (mut doc, mesh) = crate::post::fixture::job();
        // Its Face alone: the checks after each edit stay quick.
        doc.setups[0].operations.truncate(1);
        doc.adopt_materials();
        doc.adopt_holders();
        let aluminium = (doc.materials.iter())
            .find(|m| m.name() == "Aluminium 6061-T6")
            .map(|m| m.id.clone())
            .unwrap();
        assert!(doc.select_material(&aluminium));
        d.app.replace_document(doc, None);
        d.app.imported_mesh = Some(mesh);
        d.app.selected = Some(0);
        d.app.dirty = false;
        d
    }
    /// `job` (a `post::fixture` on the fixture pins) in `d`, its second
    /// setup, the one on the pins, active.
    pub(crate) fn pinned(
        d: &mut Direct,
        (mut doc, mesh): (crate::model::Document, crate::mesh::Mesh),
    ) {
        doc.adopt_materials();
        doc.adopt_holders();
        doc.active = doc.setups[1].id.clone();
        d.app.replace_document(doc, None);
        d.app.imported_mesh = Some(mesh);
        d.app.page = Page::Work;
        d.app.selected = None;
        d.app.dirty = false;
    }
    pub(crate) fn outcome(r: &Value) -> &str {
        r["outcome"].as_str().unwrap_or_default()
    }

    /// A hand Drill on the manual mill over `tris`, with a Ø`diameter`
    /// drill, its one operation selected and nothing picked (LIO-553).
    fn hand_drilling(tris: Vec<[[f32; 3]; 3]>, diameter: &str) -> Direct {
        use crate::model::{ModelFile, Operation, Strategy, Tool};
        let mut d = direct();
        let mesh = crate::mesh::normalize(tris).unwrap();
        let machine = crate::manual::defaults();
        let mut drill = Tool::new(&format!("Drill {diameter}"), "Drill", diameter);
        drill.set_type("Drill");
        for (k, v) in [("length", "40"), ("stickout", "60")] {
            drill.p.insert(k.into(), v.into());
        }
        drill.p.insert("machines".into(), machine.id.clone());
        let mut doc = d.app.doc.clone();
        doc.machines.push(machine.clone());
        doc.tools.push(drill.clone());
        doc.add_manual_setup().unwrap();
        doc.setups[0].operations = vec![Operation::for_machine(
            Strategy::Drill,
            drill.id.clone(),
            &machine,
        )];
        doc.active = doc.setups[0].id.clone();
        doc.file = Some(ModelFile {
            name: "part.stl".into(),
            path: "part.stl".into(),
            format: "stl".into(),
            bytes: 1,
            thickness: None,
        });
        d.app.replace_document(doc, None);
        d.app.imported_mesh = Some(mesh);
        d.app.selected = Some(0);
        d.app.page = Page::Work;
        d
    }
    /// A wall of the round hole of radius `r` round `x`, `y` from the
    /// part's middle, as the setup holds it.
    fn hole_wall(d: &Direct, x: f32, y: f32, r: f32) -> usize {
        let mesh = d.app.imported_mesh.as_ref().unwrap();
        let setup = d.app.doc.setup();
        let placement = crate::stock::Placement::new(mesh, Some(setup));
        let scale = mesh.display_scale();
        let b = placement.bounds;
        let mid = [0, 1].map(|k| (b.min[k] + b.max[k]) / 2. / scale);
        (0..mesh.surfaces.len())
            .find(|&id| {
                let c = placement
                    .transform(mesh.surfaces[id].center)
                    .map(|v| v / scale);
                crate::pocket::is_wall(setup, mesh, id)
                    && (c[0] - mid[0] - x).hypot(c[1] - mid[1] - y) <= r + 0.01
            })
            .unwrap()
    }
    /// LIO-553 review: the built-in example's Ø8 holes are drawn on its
    /// flange's top, not modelled (`mesh::sample`), and its drill goes down
    /// to Z8 in the flange (`viewport::paths`): nothing says they go through,
    /// so a Drill picking one from the feature list keeps No, as it does a
    /// blind hole; picked by a Feature click as by a wall, the check is the
    /// same, and finds no hole there.
    #[test]
    fn a_drill_picking_the_examples_drawn_hole_from_its_features_keeps_its_through() {
        use crate::model::{Operation, Strategy, Tool};
        let mut d = direct();
        assert!(d.app.is_sample());
        let mut drill = Tool::new("Drill 8", "Drill", "8");
        drill.set_type("Drill");
        for (k, v) in [("length", "40"), ("stickout", "60")] {
            drill.p.insert(k.into(), v.into());
        }
        let mut op = Operation::new(Strategy::Drill, drill.id.clone());
        op.geometry.clear();
        d.app.doc.tools.push(drill);
        d.app.doc.setup_mut().operations = vec![op];
        d.app.selected = Some(0);
        let get = |d: &Direct, k: &str| {
            crate::model::get(&d.app.doc.setup().operations[0].p, k).to_string()
        };
        for through in ["No", "Yes"] {
            let o = &mut d.app.doc.setup_mut().operations[0];
            o.geometry.clear();
            o.p.insert("through".into(), through.into());
            let _ = d.app.update(Message::Feature(5));
            assert_eq!(d.app.doc.setup().operations[0].geometry, vec![5]);
            assert_eq!(get(&d, "through"), through, "{}", d.app.status);
        }
    }
    /// LIO-553 review: a Drill of one Hole diameter judges by the holes it
    /// drills. A Ø8 drill through the ring, then a blind Ø6 picked beside
    /// it in a cup it does not drill (its filter is Ø8): it still goes
    /// through, by the same breakthrough.
    #[test]
    fn a_drill_judges_through_by_the_holes_its_diameter_filter_drills() {
        let mut tris = crate::cam::boring::tests::ring_triangles(8., 40., 10.);
        // A Ø20 cup 10 high at X40, a Ø6 hole 6 deep in its top.
        for part in [
            &[(10., 10.), (3., 10.), (3., 4.), (0., 4.)][..],
            &[(0., 0.), (10., 0.), (10., 10.)][..],
        ] {
            tris.extend(
                crate::cam::tests::revolved(part)
                    .into_iter()
                    .map(|t| t.map(|p| [p[0] + 40., p[1], p[2]])),
            );
        }
        let mut d = hand_drilling(tris, "8");
        let args = json!({"target": "operation", "key": "hole_diameter", "value": "8"});
        assert_eq!(outcome(&run(&mut d, "edit_field", args).unwrap()), "done");
        let op = |d: &Direct, k: &str| {
            crate::model::get(&d.app.doc.setup().operations[0].p, k).to_string()
        };
        // The part's middle is at X15: the ring's bore at −15, the cup's
        // hole at +25.
        let _ = d.app.update(Message::Surface(hole_wall(&d, -15., 0., 4.)));
        assert_eq!(op(&d, "through"), "Yes", "{}", d.app.status);
        assert_eq!(op(&d, "breakthrough"), "3.5");
        let _ = d.app.update(Message::Surface(hole_wall(&d, 25., 0., 3.)));
        let picked = d.app.doc.setup().operations[0].surfaces.clone().unwrap();
        assert!(picked.ids.len() > 1, "{picked:?}");
        assert_eq!(op(&d, "through"), "Yes", "{}", d.app.status);
        assert_eq!(op(&d, "breakthrough"), "3.5");
        assert!(!d.app.status.contains("blind"), "{}", d.app.status);
    }
    /// LIO-553: a hand Drill says whether its tip goes out under a hole that
    /// goes through the part, and how far, in its editor and to `edit_field`;
    /// a pick of holes that all go through makes it go through, past by the
    /// drill's point and a millimetre, and a blind one picked keeps it from it.
    #[test]
    fn a_hand_drill_goes_through_the_holes_that_go_through() {
        let mut d = hand_drilling(crate::cam::boring::tests::ring_triangles(8., 40., 10.), "8");
        assert!(crate::manual::is_hand(&d.app.doc.setup().machine));
        let op = |d: &Direct, k: &str| {
            crate::model::get(&d.app.doc.setup().operations[0].p, k).to_string()
        };
        assert_eq!(op(&d, "through"), "No");
        let listed = |d: &mut Direct| -> Vec<String> {
            let found = fields::of(&mut d.app, &json!({"target": "operation"})).unwrap();
            found.iter().map(|f| f.key.to_string()).collect()
        };
        let keys = listed(&mut d);
        assert!(keys.contains(&"through".into()), "{keys:?}");
        assert!(!keys.contains(&"breakthrough".into()), "{keys:?}");
        for (key, value) in [("through", "Yes"), ("breakthrough", "2.5")] {
            let args = json!({"target": "operation", "key": key, "value": value});
            let r = run(&mut d, "edit_field", args).unwrap();
            assert_eq!(outcome(&r), "done", "{key}: {r}");
            assert_eq!(op(&d, key), value);
        }
        assert!(listed(&mut d).contains(&"breakthrough".into()));
        let args = json!({"target": "operation", "key": "through", "value": "No"});
        assert_eq!(outcome(&run(&mut d, "edit_field", args).unwrap()), "done");
        // Picked, the Ø8 hole through the ring: through, 2.4 mm of point
        // and 1 mm past the exit, to the tenth up.
        let wall = hole_wall(&d, 0., 0., 4.);
        let _ = d.app.update(Message::Surface(wall));
        assert_eq!(op(&d, "through"), "Yes", "{}", d.app.status);
        assert_eq!(op(&d, "breakthrough"), "3.5");
        assert!(
            d.app.status.contains("Breakthrough 3.5 mm"),
            "{}",
            d.app.status
        );
        // Its plan takes the drill's tip 3.5 mm under the part's underside.
        let _ = d.app.update(Message::Generate);
        let sim = d.app.active_simulation().expect(&d.app.status);
        let mesh = d.app.imported_mesh.as_ref().unwrap();
        let placement = crate::stock::Placement::new(mesh, Some(d.app.doc.setup()));
        let under = placement.bounds.min[2] / mesh.display_scale();
        let low = (sim.plan.moves.iter())
            .map(|m| m.to[2])
            .fold(f32::INFINITY, f32::min);
        assert!((low - (under - 3.5)).abs() < 0.01, "{low} {under}");
        // A blind hole picked: it stops at the bottom, where through is refused.
        let mut d = hand_drilling(crate::pocket::hole_plate(false), "6");
        let args = json!({"target": "operation", "key": "through", "value": "Yes"});
        assert_eq!(outcome(&run(&mut d, "edit_field", args).unwrap()), "done");
        let wall = hole_wall(&d, -10., 0., 3.);
        let _ = d.app.update(Message::Surface(wall));
        assert_eq!(op(&d, "through"), "No", "{}", d.app.status);
        assert!(d.app.status.contains("blind"), "{}", d.app.status);
    }

    #[test]
    fn outcomes_say_what_happened() {
        let mut d = direct();
        // Already so: nothing is sent.
        let title = d.app.doc.title.clone();
        let same = run(&mut d, "set_project_title", json!({"title": title})).unwrap();
        assert_eq!(outcome(&same), "no_change", "{same}");
        // A missing selection is refused, and nothing changes.
        d.app.selected = None;
        let r = run(&mut d, "toggle_operation", json!({})).unwrap();
        assert_eq!(outcome(&r), "refused");
        assert!(
            r["reason"]
                .as_str()
                .unwrap()
                .contains("No operation is selected")
        );
        // A save the file system refuses fails, in the app's words.
        // (Under a file: absolute on every system, and never writable.)
        let file = d.folder.join("a-file");
        std::fs::write(&file, "x").unwrap();
        let nowhere = json!({"path": file.join("job.axis.json")});
        let r = run(&mut d, "save_project", nowhere).unwrap();
        assert_eq!(outcome(&r), "failed", "{r}");
        assert!(
            r["reason"].as_str().unwrap().starts_with("Save failed"),
            "{r}"
        );
        assert!(r["command_id"].is_string());
        assert!(d.app.project_path.is_none());
        // A generation the checks stop fails with the check's sentence.
        d.app.selected = Some(0);
        d.app.doc.setup_mut().operations[0].tool = String::new();
        let r = run(&mut d, "generate", json!({})).unwrap();
        assert_eq!(outcome(&r), "failed", "{r}");
        assert_eq!(r["reason"], d.app.status.as_str());
        // A tool still in use is not deleted: the app's reason.
        let used = d.app.doc.setup().operations[1].tool.clone();
        let r = run(&mut d, "delete_tool", json!({"id": used})).unwrap();
        assert_eq!(outcome(&r), "failed", "{r}");
        assert!(
            r["reason"]
                .as_str()
                .unwrap()
                .starts_with("This tool is not deleted")
        );
    }

    #[test]
    fn a_strategy_the_imported_part_has_no_preview_for_is_refused() {
        let mut d = imported();
        let before = d.app.doc.setup().operations.len();
        let r = run(&mut d, "add_operation", json!({"strategy": "Scallop"})).unwrap();
        assert_eq!(outcome(&r), "refused", "{r}");
        assert_eq!(d.app.doc.setup().operations.len(), before);
        let r = run(&mut d, "add_operation", json!({"strategy": "Pocket"})).unwrap();
        assert_eq!(outcome(&r), "done", "{r}");
        assert_eq!(d.app.doc.setup().operations.len(), before + 1);
    }

    #[test]
    fn library_saves_check_the_entry_they_make() {
        let mut d = direct();
        // A tool: a bad draft is refused with its issues; a good one saved,
        // and saved again it is no change.
        assert_eq!(
            outcome(&run(&mut d, "new_tool", json!({})).unwrap()),
            "done"
        );
        let bad = json!({"target": "tool", "key": "diameter", "value": "-1"});
        run(&mut d, "edit_field", bad).unwrap();
        let r = run(&mut d, "save_tool", json!({})).unwrap();
        assert_eq!(outcome(&r), "refused", "{r}");
        assert!(
            r["reason"]
                .as_str()
                .unwrap()
                .starts_with("The tool is not saved:"),
            "{r}"
        );
        let id = d.app.tool_draft.id.clone();
        assert!(d.app.doc.tools.iter().all(|t| t.id != id));
        run(
            &mut d,
            "edit_field",
            json!({"target": "tool", "key": "diameter", "value": "8"}),
        )
        .unwrap();
        assert_eq!(
            outcome(&run(&mut d, "save_tool", json!({})).unwrap()),
            "done"
        );
        assert!(d.app.doc.tools.iter().any(|t| t.id == id));
        assert_eq!(
            outcome(&run(&mut d, "save_tool", json!({})).unwrap()),
            "no_change"
        );
        // A material and a holder: their names trimmed on the way in.
        run(&mut d, "new_material", json!({})).unwrap();
        run(
            &mut d,
            "edit_field",
            json!({"target": "material", "key": "name", "value": "  Test alloy "}),
        )
        .unwrap();
        let r = run(&mut d, "save_material", json!({})).unwrap();
        assert_eq!(outcome(&r), "done", "{r}");
        assert!(d.app.doc.materials.iter().any(|m| m.name() == "Test alloy"));
        let holder = d.app.doc.workholding[0].id.clone();
        run(&mut d, "edit_holder", json!({"id": holder})).unwrap();
        run(
            &mut d,
            "edit_field",
            json!({"target": "holder", "key": "name", "value": " Test clamp"}),
        )
        .unwrap();
        let r = run(&mut d, "save_holder", json!({})).unwrap();
        assert_eq!(outcome(&r), "done", "{r}");
        assert!(
            d.app
                .doc
                .workholding
                .iter()
                .any(|h| h.name() == "Test clamp")
        );
        // A machine: unchanged it is no change, and its version stays.
        let machine = d.app.doc.machines[1].id.clone();
        run(&mut d, "edit_machine", json!({"id": machine})).unwrap();
        let version = d.app.doc.machines[1].version;
        assert_eq!(
            outcome(&run(&mut d, "save_machine", json!({})).unwrap()),
            "no_change"
        );
        assert_eq!(d.app.doc.machines[1].version, version);
        // A typed centreline is no longer the measured one: its marks go.
        let mark = |m: &crate::model::Machine| {
            m.p.contains_key(crate::control::CENTRE_MARK)
                || m.p.contains_key(crate::model::CENTRE_REF)
        };
        for m in [&mut d.app.doc.machines[1], &mut d.app.machine_draft] {
            m.p.insert("a_centre_y".into(), "5".into());
            m.p.insert(crate::control::CENTRE_MARK.into(), "1".into());
            m.p.insert(crate::model::CENTRE_REF.into(), "1".into());
        }
        run(
            &mut d,
            "edit_field",
            json!({"target": "machine", "key": "a_centre_y", "value": "7"}),
        )
        .unwrap();
        let r = run(&mut d, "save_machine", json!({})).unwrap();
        assert_eq!(outcome(&r), "done", "{r}");
        let saved = &d.app.doc.machines[1];
        assert_eq!(crate::model::get(&saved.p, "a_centre_y"), "7");
        assert!(!mark(saved));
        assert_eq!(saved.version, version + 1);
    }

    fn fields_of(d: &mut Direct, args: Value) -> Vec<Value> {
        let r = call(d, "list_fields", args).unwrap();
        r["fields"].as_array().unwrap().clone()
    }
    fn keys_of(fields: &[Value]) -> Vec<String> {
        fields
            .iter()
            .map(|f| f["key"].as_str().unwrap().into())
            .collect()
    }

    #[test]
    fn a_key_the_editor_does_not_draw_is_refused_with_the_keys_that_it_does() {
        let mut d = direct();
        let before = d.app.doc.clone();
        let bad = json!({"target": "operation", "key": "no_such_field", "value": "1"});
        let r = run(&mut d, "edit_field", bad).unwrap();
        assert_eq!(outcome(&r), "refused", "{r}");
        let why = r["reason"].as_str().unwrap();
        assert!(why.contains("no_such_field"), "{why}");
        assert!(why.contains("feed") && why.contains("rpm"), "{why}");
        // The same on every target, and a key of another target is no key.
        for (target, key) in [
            ("setup", "feed"),
            ("machine", "rpm_x"),
            ("tool", "origin"),
            ("material", "diameter"),
            ("holder", "feed"),
        ] {
            let args = json!({"target": target, "key": key, "value": "1"});
            let r = run(&mut d, "edit_field", args).unwrap();
            assert_eq!(outcome(&r), "refused", "{target}.{key}: {r}");
        }
        assert_eq!(
            serde_json::to_value(&d.app.doc).unwrap(),
            serde_json::to_value(&before).unwrap()
        );
    }

    #[test]
    fn a_value_off_the_choice_list_is_refused_and_one_on_it_is_set() {
        let mut d = direct();
        let fields = fields_of(&mut d, json!({"target": "operation"}));
        let picked = fields
            .iter()
            .find(|f| f["choices"].is_array())
            .expect("an operation has a pick list");
        let key = picked["key"].as_str().unwrap();
        let choices: Vec<&str> = (picked["choices"].as_array().unwrap().iter())
            .map(|c| c.as_str().unwrap())
            .collect();
        let off = json!({"target": "operation", "key": key, "value": "not a choice"});
        let r = run(&mut d, "edit_field", off).unwrap();
        assert_eq!(outcome(&r), "refused", "{r}");
        assert!(r["reason"].as_str().unwrap().contains(choices[0]), "{r}");
        let other = choices
            .iter()
            .find(|c| **c != picked["value"].as_str().unwrap())
            .expect("a second choice");
        let on = json!({"target": "operation", "key": key, "value": other});
        let r = run(&mut d, "edit_field", on).unwrap();
        assert_eq!(outcome(&r), "done", "{r}");
        let now = fields_of(&mut d, json!({"target": "operation"}));
        let now = now.iter().find(|f| f["key"] == key).unwrap();
        assert_eq!(now["value"], *other);
    }

    #[test]
    fn a_listed_key_is_edited_on_any_setup_and_operation() {
        let mut d = direct();
        // The text field of an operation that is not the selected one, in
        // the setup that is not the active one.
        run(&mut d, "add_setup", json!({})).unwrap();
        run(&mut d, "select_setup", json!({"setup": 0})).unwrap();
        let args = json!({"target": "operation", "operation": 1, "key": "rpm", "value": "7777"});
        let r = run(&mut d, "edit_field", args).unwrap();
        assert_eq!(outcome(&r), "done", "{r}");
        assert_eq!(
            crate::model::get(&d.app.doc.setups[0].operations[1].p, "rpm"),
            "7777"
        );
        let args = json!({"target": "setup", "setup": 1, "key": "name", "value": "Second"});
        let r = run(&mut d, "edit_field", args).unwrap();
        assert_eq!(outcome(&r), "done", "{r}");
        assert_eq!(d.app.doc.setups[1].name(), "Second");
        // A key with a command of its own is refused with its name.
        let args = json!({"target": "setup", "key": "origin", "value": "G54"});
        let r = run(&mut d, "edit_field", args).unwrap();
        assert_eq!(outcome(&r), "refused", "{r}");
        assert!(r["reason"].as_str().unwrap().contains("set_origin"), "{r}");
    }

    #[test]
    fn tabs_and_steps_follow_the_target_named() {
        let mut d = direct();
        let (_a, b) = two_setups(&mut d);
        // Setup 0 (active after duplicate? make it so) with its operation
        // selected on the Tool tab; then name the other setup's operation.
        run(&mut d, "select_setup", json!({"setup": 0})).unwrap();
        run(&mut d, "select_operation", json!({"operation": 0})).unwrap();
        let r = run(&mut d, "set_tab", json!({"tab": "Tool"})).unwrap();
        assert!(["done", "no_change"].contains(&outcome(&r)), "{r}");
        let r = run(
            &mut d,
            "set_tab",
            json!({"tab": "Tool", "setup": 1, "operation": 0}),
        )
        .unwrap();
        assert_eq!(outcome(&r), "done", "{r}");
        assert_eq!(d.app.doc.setups[1].operations[0].id, b);
        assert_eq!(d.app.doc.active, d.app.doc.setups[1].id);
        assert_eq!(d.app.selected, Some(0));
        assert_eq!(d.app.tab, "Tool");
        // The same tab, same target: nothing to do.
        let r = run(
            &mut d,
            "set_tab",
            json!({"tab": "Tool", "setup": 1, "operation": 0}),
        )
        .unwrap();
        assert_eq!(outcome(&r), "no_change", "{r}");
        // A setup step on the other setup while this one's is shown.
        run(&mut d, "select_setup", json!({"setup": 0})).unwrap();
        let r = run(&mut d, "set_setup_step", json!({"step": "Stock"})).unwrap();
        assert!(["done", "no_change"].contains(&outcome(&r)), "{r}");
        let r = run(
            &mut d,
            "set_setup_step",
            json!({"step": "Stock", "setup": 1}),
        )
        .unwrap();
        assert_eq!(outcome(&r), "done", "{r}");
        assert_eq!(d.app.doc.active, d.app.doc.setups[1].id);
        assert_eq!(d.app.setup_step, "Stock");
        assert!(d.app.selected.is_none());
        let r = run(
            &mut d,
            "set_setup_step",
            json!({"step": "Stock", "setup": 1}),
        )
        .unwrap();
        assert_eq!(outcome(&r), "no_change", "{r}");
    }

    /// Two setups with an operation each, setup 1 active; each operation's
    /// feed set apart (1000 and 2000). Returns the two operation ids.
    fn two_setups(d: &mut Direct) -> (String, String) {
        run(d, "duplicate_setup", json!({})).unwrap();
        let ids = |d: &Direct| {
            let ops = |si: usize| d.app.doc.setups[si].operations[0].id.clone();
            (ops(0), ops(1))
        };
        let (a, b) = ids(d);
        for (id, feed) in [(&a, "1111"), (&b, "2222")] {
            let args =
                json!({"target": "operation", "operation": id, "key": "feed", "value": feed});
            let r = run(d, "edit_field", args).unwrap();
            assert_eq!(outcome(&r), "done", "{r}");
        }
        run(d, "select_setup", json!({"setup": 0})).unwrap();
        (a, b)
    }
    fn feed(d: &Direct, id: &str) -> String {
        op_field(&d.app, id, "feed").unwrap()
    }

    #[test]
    fn an_operation_by_index_is_one_of_the_setup_given_not_the_active_one() {
        let mut d = direct();
        let (a, b) = two_setups(&mut d);
        let second = d.app.doc.setups[1].id.clone();
        let args = json!({"target": "operation", "setup": second, "operation": 0, "key": "feed", "value": "733"});
        let r = run(&mut d, "edit_field", args).unwrap();
        assert_eq!(outcome(&r), "done", "{r}");
        assert_eq!(feed(&d, &a), "1111");
        assert_eq!(feed(&d, &b), "733");
        // list_fields reads the same operation.
        let fields = fields_of(
            &mut d,
            json!({"target": "operation", "setup": 1, "operation": 0}),
        );
        let shown = fields.iter().find(|f| f["key"] == "feed").unwrap();
        assert_eq!(shown["value"], "733", "{shown}");
        // Without `setup`, an index counts in the active setup.
        run(&mut d, "select_setup", json!({"setup": 0})).unwrap();
        let fields = fields_of(&mut d, json!({"target": "operation", "operation": 0}));
        let shown = fields.iter().find(|f| f["key"] == "feed").unwrap();
        assert_eq!(shown["value"], "1111", "{shown}");
    }

    #[test]
    fn an_operation_id_with_a_setup_that_does_not_hold_it_is_refused() {
        let mut d = direct();
        let (a, b) = two_setups(&mut d);
        let first = d.app.doc.setups[0].id.clone();
        let before = d.app.doc.clone();
        let args = json!({"target": "operation", "setup": first, "operation": b, "key": "feed", "value": "733"});
        let r = run(&mut d, "edit_field", args).unwrap();
        assert_eq!(outcome(&r), "refused", "{r}");
        assert!(
            r["reason"].as_str().unwrap().contains("is not in setup"),
            "{r}"
        );
        assert_eq!(feed(&d, &a), "1111");
        assert_eq!(feed(&d, &b), "2222");
        assert_eq!(
            serde_json::to_string(&d.app.doc).unwrap(),
            serde_json::to_string(&before).unwrap()
        );
        let args = json!({"target": "operation", "setup": first, "operation": b});
        let e = call(&mut d, "list_fields", args).unwrap_err();
        assert!(e.contains("is not in setup"), "{e}");
        // An id alone names its own setup.
        let args = json!({"target": "operation", "operation": b, "key": "feed", "value": "733"});
        let r = run(&mut d, "edit_field", args).unwrap();
        assert_eq!(outcome(&r), "done", "{r}");
        assert_eq!(feed(&d, &b), "733");
    }

    #[test]
    fn another_operation_command_given_a_setup_acts_on_that_setups_operation() {
        let mut d = direct();
        let (a, b) = two_setups(&mut d);
        let second = d.app.doc.setups[1].id.clone();
        let off = |d: &Direct, id: &str| {
            let o = d
                .app
                .doc
                .setups
                .iter()
                .flat_map(|s| &s.operations)
                .find(|o| o.id == id);
            o.unwrap().enabled
        };
        let (was_a, was_b) = (off(&d, &a), off(&d, &b));
        let r = run(
            &mut d,
            "toggle_operation",
            json!({"setup": second, "operation": 0}),
        )
        .unwrap();
        assert_eq!(outcome(&r), "done", "{r}");
        assert_eq!(off(&d, &a), was_a);
        assert_eq!(off(&d, &b), !was_b);
    }

    #[test]
    fn list_fields_gives_each_tab_and_changes_nothing_the_user_sees() {
        let mut d = direct();
        run(&mut d, "add_setup", json!({})).unwrap();
        run(&mut d, "select_setup", json!({"setup": 0})).unwrap();
        d.app.tab = "Passes".into();
        d.app.setup_step = "Stock";
        d.app.wizard_step = 2;
        let shown = |d: &Direct| {
            (
                d.app.tab.clone(),
                d.app.setup_step,
                d.app.wizard_step,
                d.app.page,
                d.app.selected,
                d.app.doc.active.clone(),
                serde_json::to_string(&d.app.doc).unwrap(),
            )
        };
        let before = shown(&d);
        let tabs = crate::model::Strategy::Contour.tabs();
        let ops = fields_of(&mut d, json!({"target": "operation", "operation": 2}));
        for tab in tabs {
            assert!(ops.iter().any(|f| f["tab"] == tab), "{tab}: {ops:?}");
        }
        let feed = ops.iter().find(|f| f["key"] == "feed").unwrap();
        assert_eq!(feed["unit"], "mm/min");
        assert!(!feed["help"].as_str().unwrap().is_empty());
        // Another setup's operations and steps, the libraries and the wizard.
        let setup = fields_of(&mut d, json!({"target": "setup", "setup": 1}));
        for step in crate::ui::SETUP_STEPS {
            assert!(setup.iter().any(|f| f["tab"] == step), "{step}");
        }
        for target in ["machine", "tool", "material", "holder"] {
            let list = fields_of(&mut d, json!({"target": target}));
            assert!(!keys_of(&list).is_empty(), "{target}");
        }
        assert_eq!(shown(&d), before);
        // The user's own window shows what it did: the editor of the open tab.
        let r = call(
            &mut d,
            "list_fields",
            json!({"target": "operation", "operation": 99}),
        );
        assert!(r.is_err() || r.unwrap().get("fields").is_none());
        assert_eq!(shown(&d), before);
    }

    #[test]
    fn the_collector_is_off_after_a_listing_and_the_editor_is_the_same_with_it_off() {
        let mut d = direct();
        let window = iced::Size::new(1440., 900.);
        let _ = d.app.update(Message::Resized(window));
        let page = |d: &Direct| ui::probe::texts(d.app.view(), window);
        let once = page(&d);
        assert!(once.iter().any(|t| t.contains("Feed")), "{once:?}");
        assert!(!ui::recording());
        fields_of(&mut d, json!({"target": "operation"}));
        fields_of(&mut d, json!({"target": "setup"}));
        assert!(!ui::recording());
        assert_eq!(page(&d), once);
        // A listing found while a view was built is not kept.
        assert_eq!(ui::collecting("x", || {}), vec![]);
    }

    /// A valid ASCII STL part.
    fn stl_text() -> String {
        let mut text = String::from("solid hole\n");
        for t in crate::pocket::chamfered_hole() {
            text += "facet normal 0 0 0\nouter loop\n";
            for p in t {
                text += &format!("vertex {} {} {}\n", p[0], p[1], p[2]);
            }
            text += "endloop\nendfacet\n";
        }
        text + "endsolid hole\n"
    }
    fn import_part(d: &mut Direct, path: &Path) -> Value {
        run(
            d,
            "import_model",
            json!({"path": path.display().to_string()}),
        )
        .unwrap()
    }

    /// The reviewer's probes (LIO-470, round 4): a file command is judged
    /// by the app's own completion result, not by state that may already
    /// match. An invalid import after a valid one fails, whether it has
    /// the same basename in another folder or is the same path.
    #[test]
    fn a_failed_import_is_failed_though_a_part_of_that_name_is_loaded() {
        let mut d = direct();
        let (one, two) = (d.folder.join("one"), d.folder.join("two"));
        std::fs::create_dir_all(&one).unwrap();
        std::fs::create_dir_all(&two).unwrap();
        let (p, q) = (one.join("part.stl"), two.join("part.stl"));
        std::fs::write(&p, stl_text()).unwrap();
        std::fs::write(&q, "not a mesh").unwrap();
        let r = import_part(&mut d, &p);
        assert_eq!(r["outcome"], "done", "{r}");
        // Another folder, the same name.
        let r = import_part(&mut d, &q);
        assert_eq!(r["outcome"], "failed", "{r}");
        assert_eq!(d.app.doc.file.as_ref().unwrap().path, p);
        assert!(d.app.imported_mesh.is_some());
        // The same path, damaged since.
        std::fs::write(&p, "not a mesh").unwrap();
        let r = import_part(&mut d, &p);
        assert_eq!(r["outcome"], "failed", "{r}");
        assert!(d.app.imported_mesh.is_some(), "the previous part is kept");
        // A valid one still imports.
        std::fs::write(&q, stl_text()).unwrap();
        let r = import_part(&mut d, &q);
        assert_eq!(r["outcome"], "done", "{r}");
        assert_eq!(d.app.doc.file.as_ref().unwrap().path, q);
    }

    /// A save that could not write is failed: the project was clean, so
    /// the old dirty flag said it was saved.
    #[cfg(unix)]
    #[test]
    fn a_save_that_could_not_write_is_failed() {
        use std::os::unix::fs::PermissionsExt;
        let mut d = direct();
        let folder = d.folder.join("work");
        std::fs::create_dir_all(&folder).unwrap();
        let file = folder.join("job.axis.json");
        let r = run(
            &mut d,
            "save_project",
            json!({"path": file.display().to_string()}),
        )
        .unwrap();
        assert_eq!(r["outcome"], "done", "{r}");
        // Another project's title in the file, with this one's id.
        let mut text: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        text["title"] = json!("External job B");
        let external = serde_json::to_vec(&text).unwrap();
        std::fs::write(&file, &external).unwrap();
        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o555)).unwrap();
        let binds = std::fs::write(folder.join("probe"), "x").is_err();
        let r = run(&mut d, "save_project", json!({})).unwrap();
        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o755)).unwrap();
        // (Running as a user the mode does not bind, nothing to see.)
        if !binds {
            return;
        }
        assert_eq!(r["outcome"], "failed", "{r}");
        assert!(r["reason"].as_str().unwrap().contains("Save failed"), "{r}");
        assert_eq!(
            std::fs::read(&file).unwrap(),
            external,
            "the file is as it was"
        );
        assert!(!d.app.saved_to_file());
        // Writable again, it saves.
        let r = run(&mut d, "save_project", json!({"overwrite": true})).unwrap();
        assert_eq!(r["outcome"], "done", "{r}");
        assert!(d.app.saved_to_file());
    }

    /// Reopening the current path after its contents became invalid fails
    /// and keeps the job; a valid one opens.
    #[test]
    fn reopening_a_now_invalid_current_file_is_failed() {
        let mut d = direct();
        let file = d.folder.join("job.axis.json");
        let path = file.display().to_string();
        let r = run(&mut d, "save_project", json!({"path": path})).unwrap();
        assert_eq!(r["outcome"], "done", "{r}");
        let title = d.app.doc.title.clone();
        let good = std::fs::read(&file).unwrap();
        std::fs::write(&file, "{broken").unwrap();
        let open = json!({"path": path, "discard_unsaved": true});
        let r = run(&mut d, "open_project", open.clone()).unwrap();
        assert_eq!(r["outcome"], "failed", "{r}");
        assert_eq!(d.app.doc.title, title, "the previous job is kept");
        assert_eq!(d.app.project_path.as_deref(), Some(file.as_path()));
        std::fs::write(&file, good).unwrap();
        let r = run(&mut d, "open_project", open).unwrap();
        assert_eq!(r["outcome"], "done", "{r}");
    }
}
