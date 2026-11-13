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
use super::class::{self, Class};
use crate::{
    App, GuideAction, Message, Page, Target, control, model::Strategy, run, section, settings,
    stock_surface, ui, viewport,
};
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
}
impl Ctx<'_> {
    fn op(&self) -> Option<&crate::model::Operation> {
        let (si, oi) = self.operation?;
        self.app.doc.setups.get(si)?.operations.get(oi)
    }
}

/// Why a command is not sent.
#[derive(Debug)]
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

/// The effect a command must have: `Err` says what did not happen.
pub type Expect = Box<dyn Fn(&App) -> Result<(), String>>;

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
    params: fn() -> Vec<(&'static str, Value, bool)>,
    examples: fn() -> Vec<Value>,
    precondition: fn(&Ctx) -> Result<(), Problem>,
    build: Build,
    /// Why it is already so, when it is: nothing is sent (`no_change`).
    pub unchanged: Option<fn(&Ctx) -> Option<String>>,
    /// The effect checked once the work has settled (Step 3 gives every
    /// command one).
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
            On::Setup => Some(("setup", addressed("setup"))),
            On::Operation => Some(("operation", addressed("operation"))),
            On::Nothing => None,
        };
        for (name, schema, needed) in selector
            .map(|(n, s)| (n, s, false))
            .into_iter()
            .chain((self.params)())
        {
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
    /// The messages, without the precondition: what the builder makes.
    #[cfg(test)]
    fn built(&self, ctx: &Ctx) -> Result<Vec<Message>, Problem> {
        let mut messages = select_first(self.on, ctx)?;
        match &self.build {
            Build::Sends(m) => messages.push(m()),
            Build::With(f) => messages.extend(f(ctx)?),
        }
        Ok(messages)
    }
    /// Checks the precondition, then makes the messages: the selection
    /// first, then the command's own.
    pub fn messages(&self, ctx: &Ctx) -> Result<Vec<Message>, Problem> {
        (self.precondition)(ctx)?;
        let mut messages = select_first(self.on, ctx)?;
        match &self.build {
            Build::Sends(m) => messages.push(m()),
            Build::With(f) => messages.extend(f(ctx)?),
        }
        Ok(messages)
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
            Ok(if app.doc.setups[si].id == app.doc.active {
                vec![]
            } else {
                vec![Message::SelectSetup(si)]
            })
        }
        On::Operation => {
            let Some((si, oi)) = ctx.operation else {
                return refused(
                    "No operation is selected: give `operation`, or select one (select_operation).",
                );
            };
            let mut m = vec![];
            if app.doc.setups[si].id != app.doc.active {
                m.push(Message::SelectSetup(si));
            }
            if app.doc.setups[si].id != app.doc.active || app.selected != Some(oi) {
                m.push(Message::SelectOp(oi));
            }
            Ok(m)
        }
    }
}

/// The setup a command acts on, and its operation: (setup, operation).
pub type Addressed = (Option<usize>, Option<(usize, usize)>);
/// Resolves what `on` addresses in `args`: the setup and operation the
/// command acts on, before its precondition runs.
pub fn resolve(
    on: On,
    app: &App,
    args: &Value,
) -> Result<Addressed, Problem> {
    let active = app.doc.setups.iter().position(|s| s.id == app.doc.active);
    match on {
        On::Nothing => Ok((None, None)),
        On::Setup => match args.get("setup") {
            Some(v) => Ok((Some(setup_of(app, v)?), None)),
            None => Ok((active, None)),
        },
        On::Operation => match args.get("operation") {
            Some(v) => {
                let (si, oi) = operation_of(app, None, v)?;
                Ok((Some(si), Some((si, oi))))
            }
            None => Ok((
                active,
                active
                    .zip(app.selected)
                    .filter(|&(si, oi)| oi < app.doc.setups[si].operations.len()),
            )),
        },
    }
}

/// The schema of an argument that names a setup or an operation.
fn addressed(what: &str) -> Value {
    json!({
        "type": ["string", "integer"],
        "description": format!("The {what}'s id, or its index from 0 as inspect lists it."),
    })
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
    match found {
        Some(found) => Ok(found),
        None => refused(format!(
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

/// Whether the job has work not saved to a project file: changes since its
/// file was saved, or setups never saved to one.
fn unsaved(app: &App) -> bool {
    app.project_dirty || (app.project_path.is_none() && !app.doc.setups.is_empty())
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
/// Whether the command may write `path`: not inside the app's data folder
/// (a headless session's own folder aside), not onto a library machine's
/// UC100 profile, not onto a folder, and not over a file that is there
/// unless `overwrite` is true or it is the project's own file (`own`).
fn may_write(ctx: &Ctx, path: &Path, own: Option<&Path>) -> Result<(), Problem> {
    let target = resolved(path);
    let session = ctx.session.map(resolved);
    if target.starts_with(resolved(&data_folder()))
        && !session.is_some_and(|s| target.starts_with(s))
    {
        return refused(format!(
            "{} is inside the app's data folder, which only the app writes: choose another place.",
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

/// A field key typed by the agent, as the `&'static str` an `Edit` carries.
/// Each key is kept once, a few thousand at most. The key is checked against
/// the editor's own fields in Step 5 (`list_fields`), which then gives the
/// field's key instead.
fn field_key(key: &str) -> Result<&'static str, Problem> {
    use std::sync::Mutex;
    static KEYS: Mutex<std::collections::BTreeSet<&'static str>> =
        Mutex::new(std::collections::BTreeSet::new());
    let named = !key.is_empty()
        && key.len() <= 64
        && key
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
    if !named {
        return bad(format!(
            "\"{key}\" is not a field key: keys are lower case letters, digits and _."
        ));
    }
    let mut keys = KEYS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(k) = keys.get(key) {
        return Ok(k);
    }
    if keys.len() >= 4096 {
        return refused("Too many different field keys were given in this session.");
    }
    let k: &'static str = Box::leak(key.to_string().into_boxed_str());
    keys.insert(k);
    Ok(k)
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
pub const LATER: [(&str, &str); 4] = [
    ("pick_surfaces", "picking, Step 6"),
    ("pick_features", "picking, Step 6"),
    ("pick_pin_holes", "picking, Step 6"),
    ("place_pins", "picking, Step 6"),
];

/// The user's actions an agent may not run, by name, so `list_commands`
/// shows them and says why.
pub const REFUSED: [(&str, &str, &str); 27] = [
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
];

/// The command named `name`.
pub fn find(name: &str) -> Option<&'static Command> {
    REGISTRY.iter().find(|c| c.name == name)
}
/// Every command, in the table's order.
pub fn all() -> &'static [Command] {
    &REGISTRY
}

static REGISTRY: LazyLock<Vec<Command>> = LazyLock::new(table);

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
            .with(|ctx| {
                let path = path_arg(ctx, "path")?;
                let opened = crate::storage::load_reporting(&path).map(|(d, n)| (d, path, n));
                Ok(vec![Message::ProjectOpened(Some(opened))])
            })
            .requires("An absolute path; no unsaved work, or discard_unsaved: true.")
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
                    return match own {
                        Some(_) => Ok(vec![Message::SaveProject]),
                        None => refused("The project has no file yet: give `path`."),
                    };
                }
                let path = path_arg(ctx, "path")?;
                may_write(ctx, &path, own)?;
                Ok(vec![Message::Saved(Some(path))])
            })
            .requires("A path, or a project that has its file; the path's rules (absolute, outside the app's data folder, overwrite to replace).")
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
                        .then_some(())
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
                        .then_some(())
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
            .with(|ctx| Ok(vec![Message::MoveSetup(integer(ctx, "delta")? as isize)]))
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
                let setup = match ctx.args.get("setup") {
                    Some(s) => Some(setup_of(ctx.app, s)?),
                    None => None,
                };
                let (si, oi) = operation_of(ctx.app, setup, arg(ctx, "operation")?)?;
                let mut m = vec![];
                if ctx.app.doc.setups[si].id != ctx.app.doc.active {
                    m.push(Message::SelectSetup(si));
                }
                m.push(Message::SelectOp(oi));
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
                        .then_some(())
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
                        .then_some(())
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
            .with(|ctx| Ok(vec![Message::MoveOp(integer(ctx, "delta")? as isize)]))
            .effect("The operation moves in its setup's order."),
        cmd("toggle_operation", "operation").sends(|| Message::ToggleOp).on_operation().effect("The operation is excluded from, or included in, generation and review."),
        cmd("set_operation_tool", "operation")
            .help_of(|| Message::SetOpTool(String::new()), "")
            .on_operation()
            .params(|| vec![("id", text_param("A library tool's id."), true)])
            .examples(|| vec![json!({"id": "<tool>"})])
            .with(|ctx| Ok(vec![Message::SetOpTool(tool_id(ctx, "id")?)]))
            .effect("The operation cuts with that tool."),
        cmd("recommend_speeds", "operation").sends(|| Message::RecommendSpeeds).on_operation().effect("The operation's speed and feed from the project material's cutting data."),
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
        cmd("add_centring", "operation").sends(|| Message::AddCentring).on_operation().effect("A centre drilling before the manual-mill drill."),
        cmd("add_pin_drilling", "operation").sends(|| Message::AddPinDrilling).on_setup().effect("A Drill of the pin holes in the earlier setup."),
        cmd("thread_from_part", "operation").sends(|| Message::ThreadFromPart).on_operation().effect("The Turn thread's side, diameter, pitch and end from the part."),
        cmd("box_from_face", "operation").sends(|| Message::BoxFromFace).on_operation().effect("The Box's six fields from its picked face."),
        cmd("pick_all_corners", "operation").sends(|| Message::AllCorners).on_operation().effect("Every rounded outside corner picked for Round corners."),
        cmd("edit_field", "operation")
            .text("Sets one field of an operation, a setup, or the machine, tool, material or holder draft, as typing in the editor does. The machine's controller fields (uc100_profile, uc100_connection, pi_*) are the user's.")
            .params(|| {
                vec![
                    ("target", choice_param(&names(&EDIT_TARGETS)), true),
                    ("key", text_param("The field's key."), true),
                    ("value", json!({"type": ["string", "number", "boolean"], "description": "The value, as typed."}), true),
                    ("setup", addressed("setup"), false),
                    ("operation", addressed("operation"), false),
                ]
            })
            .examples(|| vec![json!({"target": "operation", "key": "rpm", "value": "8000"})])
            // The key and the choices are checked against the editor's own
            // fields in Step 5 (`list_fields`); here only the machine's
            // controller fields are refused.
            .when(|ctx| {
                let key = string(ctx, "key")?;
                if class::machine_key(&key) {
                    return refused(format!("The field {key}: {}", class::MACHINE));
                }
                Ok(())
            })
            .with(|ctx| {
                let name = one_of(ctx, "target", &names(&EDIT_TARGETS))?;
                let target = (EDIT_TARGETS.iter())
                    .find(|(n, _)| *n == name)
                    .map_or(Target::Operation, |(_, t)| *t);
                let key = field_key(&string(ctx, "key")?)?;
                let value = match arg(ctx, "value")? {
                    Value::String(s) => s.clone(),
                    Value::Number(n) => n.to_string(),
                    Value::Bool(b) => b.to_string(),
                    _ => return bad("`value` is a string, a number or true/false."),
                };
                let mut m = vec![];
                match target {
                    Target::Operation => {
                        let (si, oi) = match ctx.args.get("operation") {
                            Some(v) => operation_of(ctx.app, None, v)?,
                            None => match ctx.app.doc.setups.iter().position(|s| s.id == ctx.app.doc.active).zip(ctx.app.selected) {
                                Some(found) => found,
                                None => return refused("No operation is selected: give `operation`."),
                            },
                        };
                        if ctx.app.doc.setups[si].id != ctx.app.doc.active {
                            m.push(Message::SelectSetup(si));
                        }
                        if ctx.app.doc.setups[si].id != ctx.app.doc.active || ctx.app.selected != Some(oi) {
                            m.push(Message::SelectOp(oi));
                        }
                    }
                    Target::Setup => {
                        let si = match ctx.args.get("setup") {
                            Some(v) => setup_of(ctx.app, v)?,
                            None => match ctx.app.doc.setups.iter().position(|s| s.id == ctx.app.doc.active) {
                                Some(si) => si,
                                None => return refused("There is no setup: add one first."),
                            },
                        };
                        if ctx.app.doc.setups[si].id != ctx.app.doc.active {
                            m.push(Message::SelectSetup(si));
                        }
                    }
                    Target::Machine | Target::Tool | Target::Material | Target::Holder => {}
                }
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
                _ => Ok(()),
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
        cmd("save_tool", "libraries").sends(|| Message::SaveTool).effect("The tool draft is saved to the library."),
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
        cmd("save_material", "libraries").sends(|| Message::SaveMaterial).effect("The material draft is saved to the library."),
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
        cmd("save_holder", "libraries").sends(|| Message::SaveHolder).effect("The workholding draft is saved to the library."),
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
                let si = setup_of(ctx.app, arg(ctx, "setup")?)?;
                let (si, oi) = operation_of(ctx.app, Some(si), arg(ctx, "operation")?)?;
                Ok(vec![Message::OpenIssue(si, oi, string(ctx, "message")?)])
            })
            .effect("The operation is open at the tab the problem is about."),
        cmd("steps_fix", "review")
            .text("From a manual setup's steps, opens the operation (or, with none, the setup editor) at the problem the steps report.")
            .on_setup()
            .params(|| vec![("operation", addressed("operation"), false), ("problem", text_param("The problem, as the steps say it."), true)])
            .examples(|| vec![json!({"operation": 0, "problem": ""}), json!({"problem": ""})])
            .with(|ctx| {
                let si = ctx.setup.unwrap_or(0);
                let op = match ctx.args.get("operation") {
                    Some(v) => Some(operation_of(ctx.app, Some(si), v)?.1),
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
            .with(|ctx| {
                let folder = path_arg(ctx, "folder")?;
                if !folder.is_dir() {
                    return refused(format!("There is no folder {}.", folder.display()));
                }
                let Some(mesh) = ctx.app.part_mesh() else {
                    return refused("Load the imported part before exporting G-code.");
                };
                let job = crate::post::post_job(&ctx.app.doc, mesh);
                for program in job.setups.iter().flat_map(|s| s.programs()) {
                    may_write(ctx, &folder.join(&program.file_name), None)?;
                }
                Ok(vec![Message::GcodeExported(Some(crate::post::write_job(&job, &folder)))])
            })
            .requires("An imported part; an absolute folder that exists, outside the app's data folder; overwrite: true to replace programs there.")
            .effect("One LinuxCNC program per setup, written after every check passes; nothing is written if one fails."),
        cmd("export_setup_sheet", "export")
            .help_of(|| Message::Export("sheet"), "")
            .params(|| vec![("path", text_param("The Markdown file to write."), true), overwrite_param()])
            .examples(|| vec![json!({"path": "/tmp/sheet.md"})])
            .with(|ctx| {
                let path = path_arg(ctx, "path")?;
                may_write(ctx, &path, None)?;
                let data = ctx.app.doc.setup_sheet(ctx.app.part_mesh());
                Ok(vec![match crate::storage::write_atomic(&path, data.as_bytes()) {
                    Ok(()) => Message::Exported(path),
                    Err(e) => Message::ExportError(e),
                }])
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
            .with(|ctx| {
                let path = path_arg(ctx, "path")?;
                may_write(ctx, &path, None)?;
                let mut doc = ctx.app.doc.clone();
                if let Some(mesh) = &ctx.app.imported_mesh {
                    crate::picks::record(&mut doc, mesh);
                }
                let written = crate::bundle::write(&path, &doc).map(|()| path);
                Ok(vec![Message::TabletSaved(Some(written))])
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
    let setup = match ctx.args.get("setup") {
        Some(s) => Some(setup_of(ctx.app, s).ok()?),
        None => None,
    };
    operation_of(ctx.app, setup, ctx.args.get("operation")?).ok()
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
mod tests {
    use super::*;
    use crate::mcp::engine::{self, Backend};
    use std::{mem::discriminant, time::Instant};

    /// An `App` driven straight, its tasks dropped: the registry's
    /// messages, refusals and file rules, without a session's thread.
    struct Direct {
        app: App,
        folder: PathBuf,
        _dir: tempfile::TempDir,
    }
    impl Backend for Direct {
        fn app(&self) -> &App {
            &self.app
        }
        fn send(&mut self, message: Message) {
            let _ = self.app.update(message);
        }
        fn settle(&mut self, _: Instant) -> bool {
            true
        }
        fn pending(&self) -> Vec<&'static str> {
            vec![]
        }
        fn kind(&self) -> &'static str {
            "headless"
        }
        fn session(&self) -> Value {
            json!({"id": "test", "folder": self.folder.display().to_string()})
        }
        fn session_folder(&self) -> Option<PathBuf> {
            Some(self.folder.clone())
        }
    }
    /// The sample job, its first operation selected, in a folder of its own.
    fn direct() -> Direct {
        let dir = tempfile::tempdir().unwrap();
        let mut app = App::from_workspace(dir.path().join("workspace.json"), None);
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
        let folder = dir.path().to_path_buf();
        Direct {
            app,
            folder,
            _dir: dir,
        }
    }
    fn run(d: &mut Direct, command: &str, args: Value) -> Result<Value, String> {
        let call = json!({"command": command, "args": args});
        engine::call(d, "execute", &call, Instant::now())
    }
    /// The example's placeholders filled from `d`'s libraries and folder.
    fn filled(d: &Direct, example: &Value) -> Value {
        let doc = &d.app.doc;
        let text = example
            .to_string()
            .replace("<tool>", &doc.tools[0].id)
            .replace("<machine>", &doc.machines[0].id)
            .replace("<material>", &doc.materials[0].id)
            .replace("<holder>", &doc.workholding[0].id)
            .replace("/tmp", &d.folder.display().to_string());
        serde_json::from_str(&text).unwrap()
    }
    fn ctx<'a>(d: &'a Direct, c: &Command, args: &'a Value) -> Ctx<'a> {
        let (setup, operation) = resolve(c.on, &d.app, args).ok().unwrap_or_default();
        Ctx {
            app: &d.app,
            args,
            session: Some(&d.folder),
            setup,
            operation,
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
    fn built(d: &Direct) -> Vec<(&'static str, Message)> {
        std::fs::write(d.folder.join("part.stl"), "solid x\nendsolid x\n").unwrap();
        let mut all = vec![];
        for c in all_commands() {
            for example in c.example_args() {
                let mut args = filled(d, &example);
                if c.name == "import_model" {
                    args = json!({"path": d.folder.join("part.stl").display().to_string()});
                }
                let ctx = ctx(d, c, &args);
                match c.built(&ctx) {
                    Ok(messages) => all.extend(messages.into_iter().map(|m| (c.name, m))),
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
        )
    }

    #[test]
    fn every_command_class_has_a_command_that_sends_it() {
        let mut d = direct();
        // A project with its file, which Save writes without a path.
        d.app.project_path = Some(d.folder.join("job.axis.json"));
        let built = built(&d);
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
    /// Field keys an `Edit` literal sets that no command sets in this step,
    /// and why.
    const KEYS_REFUSED: [(&str, &str); 1] = [(
        "uc100_profile",
        "the machine's controller: refused (class::machine_key)",
    )];

    #[test]
    fn every_edit_key_in_the_ui_is_reachable() {
        let mut literals = edit_literals(include_str!("../ui.rs"));
        literals.extend(edit_literals(include_str!("../main.rs")));
        assert!(literals.len() >= 10, "{literals:?}");
        let mut d = direct();
        for (target, key) in literals {
            let t = EDIT_TARGETS
                .iter()
                .find(|(n, _)| n.eq_ignore_ascii_case(&target))
                .map(|(_, t)| *t)
                .unwrap_or_else(|| panic!("target {target}"));
            let m = Message::Edit(t, field_key(&key).unwrap(), String::new());
            match class::class(&m) {
                Class::Command("edit_field") => {
                    let args = json!({"target": target.to_lowercase(), "key": key, "value": ""});
                    let r = run(&mut d, "edit_field", args).unwrap();
                    assert_ne!(r["outcome"], "refused", "{target}.{key}: {r}");
                }
                Class::Command(name) => assert!(
                    find(name).is_some() || LATER.iter().any(|(n, _)| *n == name),
                    "{target}.{key} is {name}"
                ),
                Class::Machine => assert!(
                    KEYS_REFUSED.iter().any(|(k, _)| *k == key),
                    "{target}.{key} is refused without a reason listed"
                ),
                c => panic!("{target}.{key} is {c:?}"),
            }
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
        // A headless session's own folder is the one place in the data
        // folder a command may write.
        let session = data_folder().join("mcp-sessions").join("s");
        let args = json!({});
        let c = Ctx {
            app: &d.app,
            args: &args,
            session: Some(&session),
            setup: None,
            operation: None,
        };
        assert!(may_write(&c, &session.join("job.axis.json"), None).is_ok());
        assert!(may_write(&c, &data_folder().join("workspace.json"), None).is_err());
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
}
