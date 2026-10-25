//! The AI planner's proposal: the format the model must return, and how a
//! proposal becomes a job. Only IDs, enumerated choices and numbers reach the
//! job; the model's free text is displayed or stored as notes.
use crate::{
    analysis::{self, OrientationReport, PartReport, PlaneReport},
    mesh::Mesh,
    model::{CLEARANCE, Document, Operation, Setup, Strategy, Tool, get, num, pocket_setting},
    surface::Selection,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub use crate::post::ORIGINS;
const STRATEGIES: [(&str, Strategy); 6] = [
    ("Face", Strategy::Face),
    ("Pocket", Strategy::Pocket),
    ("Contour", Strategy::Contour),
    ("Drill", Strategy::Drill),
    ("Parallel", Strategy::Parallel),
    ("Waterline", Strategy::Waterline),
];
const COOLANTS: [&str; 4] = ["Flood", "Mist", "Air", "Off"];
const CYCLES: [&str; 7] = crate::model::DRILL_CYCLES;
const ENTRIES: [&str; 2] = ["Profile ramp", "Plunge"];
/// A Contour's modes: round the outline, a floor's walls, or inside every
/// hole through the outline (a flat part's cut-outs).
const CONTOUR_MODES: [&str; 3] = ["outside", "inside_floor", "inside_holes"];
/// The stock of a chuck proposal: a round bar, or the user's own box.
const STOCKS: [&str; 2] = ["Round bar", "Box"];
/// The work zeros a proposal may name: the stock box's and the part's
/// (`ORIGINS`), and those on a round bar lying along X
/// (`round_hold::ORIGINS`), which only such a bar takes.
/// The setup's `holding` choice that takes a library entry of `kind`:
/// "Clamps" for a "Clamp", the kind's own name otherwise.
fn holding_of(kind: &str) -> &str {
    if kind == "Clamp" { "Clamps" } else { kind }
}
fn work_zeros() -> Vec<&'static str> {
    ORIGINS
        .iter()
        .chain(&crate::round_hold::ORIGINS)
        .copied()
        .collect()
}
/// The stock left beyond the part at the free end of a chuck proposal's
/// stock when it names none, mm.
pub const FREE_END: f32 = 2.;
const MAX_SETUPS: usize = 6;
pub const MAX_SETUP_OPERATIONS: usize = 12;
pub const MAX_OPERATIONS: usize = 40;
const MAX_SURFACES: usize = 500;
/// The largest `clearance` when the machine's Z travel is not known (0).
const UNKNOWN_TRAVEL_CLEARANCE: f32 = 300.;
/// The largest `clearance` an operation may ask for: the machine's Z travel
/// when the report has one (never below the default), else
/// [`UNKNOWN_TRAVEL_CLEARANCE`].
pub fn clearance_cap(report: &PartReport) -> f32 {
    match report.machine.travel[2] {
        z if z > 0. => z.max(CLEARANCE),
        _ => UNKNOWN_TRAVEL_CLEARANCE,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Proposal {
    pub summary: String,
    /// How setup 1's stock is held in the chuck on the rotary A axis; None
    /// keeps the user's own workholding. Only on a machine with the axis.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chuck: Option<ProposedChuck>,
    pub setups: Vec<ProposedSetup>,
    pub unmachined: Vec<Unmachined>,
    pub warnings: Vec<String>,
}
/// Setup 1's stock in a chuck, as the setup's Stock and Modelled workholding
/// fields have it: a bar along X centred on the A axis, the part placed on
/// that axis in the stick-out.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProposedChuck {
    /// "Round bar" (`diameter`) or "Box" (`width` across Y and `height` in Z
    /// in setup 1's axes), `length` long along X in both.
    pub stock: String,
    pub length: f32,
    pub diameter: Option<f32>,
    pub width: Option<f32>,
    pub height: Option<f32>,
    /// `chuck::JAWS`.
    pub jaws: String,
    /// How far the stock reaches out of the jaws, mm.
    pub stickout: f32,
    /// `chuck::TAILSTOCK`.
    pub tailstock: String,
    /// The stock left beyond the part at the free (+X) end, mm; null is
    /// `FREE_END`.
    pub free_end: Option<f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProposedSetup {
    pub orientation: String,
    pub name: String,
    pub origin: String,
    pub workholding: String,
    /// The `number` of the `workholding_library` entry that holds this setup,
    /// or None: setup 1's own, which for a later setup is setup 1's.
    pub holder: Option<usize>,
    /// The A angles, degrees, this setup indexes to in setup 1's chuck
    /// instead of re-clamping the stock (every operation runs at each); None
    /// for a setup that is clamped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub a_angles: Option<Vec<f32>>,
    pub operations: Vec<ProposedOp>,
    /// When the planner changes a plan: the number of the current setup this
    /// one keeps (see [`describe`]), None for a new setup.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<usize>,
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProposedOp {
    pub strategy: String,
    pub name: String,
    pub tool: u32,
    pub surfaces: Vec<usize>,
    pub rpm: f32,
    pub feed: f32,
    pub stepdown: Option<f32>,
    pub stepover_percent: Option<f32>,
    pub radial_allowance: Option<f32>,
    pub axial_allowance: Option<f32>,
    pub bottom_offset: Option<f32>,
    pub clearance: Option<f32>,
    pub finish_passes: Option<u32>,
    pub facing_area: Option<String>,
    pub contour_mode: Option<String>,
    pub pocket_entry: Option<String>,
    pub ramp_angle: Option<f32>,
    pub surface_stepover: Option<f32>,
    pub tolerance: Option<f32>,
    pub drill_cycle: Option<String>,
    pub peck: Option<f32>,
    pub breakthrough: Option<f32>,
    pub hole_diameter: Option<f32>,
    pub through: Option<bool>,
    /// An outside Contour's holding tabs; None for none. A proposal written
    /// before tabs has none.
    #[serde(default)]
    pub tabs: Option<ProposedTabs>,
    pub coolant: String,
    pub reason: String,
    /// When the planner changes a plan: the number of the current operation
    /// this one keeps (see [`describe`]), None for a new operation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<usize>,
}
/// An outside Contour's holding tabs (`cam`'s, placed on the outline's
/// straight sides and gentle curves): how many on each outline, each one's
/// width and its height above where the cut leaves the stock, mm.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProposedTabs {
    pub count: u32,
    pub width: f32,
    pub height: f32,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Unmachined {
    pub feature: String,
    pub reason: String,
}
/// One thing wrong with a proposal. `setup` and `operation` count from 1;
/// `setup` 0 means the whole proposal.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Problem {
    pub setup: usize,
    pub operation: Option<usize>,
    pub stage: &'static str,
    pub message: String,
}
impl Problem {
    /// The problem as the page shows it, "Setup 1, operation 2: message" with
    /// the message one line of at most 300 characters.
    pub(crate) fn line(&self) -> String {
        let place = match (self.setup, self.operation) {
            (0, _) => "Plan".to_string(),
            (s, None) => format!("Setup {s}"),
            (s, Some(o)) => format!("Setup {s}, operation {o}"),
        };
        format!("{place}: {}", clip(&self.message, 300))
    }
}
fn object(properties: Value) -> Value {
    let required: Vec<&String> = properties.as_object().unwrap().keys().collect();
    json!({"type": "object", "properties": properties, "required": required, "additionalProperties": false})
}
fn choice(options: &[&str], nullable: bool) -> Value {
    if nullable {
        let mut all: Vec<Value> = options.iter().map(|o| json!(o)).collect();
        all.push(Value::Null);
        json!({"type": ["string", "null"], "enum": all})
    } else {
        json!({"type": "string", "enum": options})
    }
}
/// Strict JSON Schema for a proposal on a machine without a rotary axis:
/// every field required, nothing extra.
pub fn schema() -> Value {
    schema_with(false, false)
}
/// [`schema`], with `edit` (the planner changes the current plan) a `from`
/// on every setup and operation: the number of the current one it keeps, or
/// null.
#[cfg(test)]
pub fn schema_for(edit: bool) -> Value {
    schema_with(edit, false)
}
/// `schema` for the machine of `report`: with a rotary A axis it also asks
/// for the chuck and the A angles of the setups.
#[cfg(test)]
pub fn schema_on(report: &PartReport) -> Value {
    schema_with(false, report.rotary.is_some())
}
/// The schema with `from` when `edit` (`schema_for`), and with the chuck
/// and the A angles when `rotary` (`schema_on`).
pub fn schema_with(edit: bool, rotary: bool) -> Value {
    let text = || json!({"type": "string"});
    let number = || json!({"type": "number"});
    let optional = || json!({"type": ["number", "null"]});
    let ups: Vec<&str> = analysis::ORIENTATIONS.iter().map(|(up, _)| *up).collect();
    let strategies: Vec<&str> = STRATEGIES.iter().map(|(name, _)| *name).collect();
    let operation = object(json!({
        "strategy": choice(&strategies, false),
        "name": text(),
        "tool": {"type": "integer"},
        "surfaces": {"type": "array", "items": {"type": "integer"}},
        "rpm": number(),
        "feed": number(),
        "stepdown": optional(),
        "stepover_percent": optional(),
        "radial_allowance": optional(),
        "axial_allowance": optional(),
        "bottom_offset": optional(),
        "clearance": optional(),
        "finish_passes": {"type": ["integer", "null"]},
        "facing_area": choice(&["stock", "model"], true),
        "contour_mode": choice(&CONTOUR_MODES, true),
        "pocket_entry": choice(&ENTRIES, true),
        "ramp_angle": optional(),
        "surface_stepover": optional(),
        "tolerance": optional(),
        "drill_cycle": choice(&CYCLES, true),
        "peck": optional(),
        "breakthrough": optional(),
        "hole_diameter": optional(),
        "through": {"type": ["boolean", "null"]},
        "tabs": {"anyOf": [object(json!({
            "count": {"type": "integer"},
            "width": number(),
            "height": number(),
        })), {"type": "null"}]},
        "coolant": choice(&COOLANTS, false),
        "reason": text(),
    }));
    let from = || json!({"type": ["integer", "null"]});
    let mut operation = operation;
    if edit {
        operation["properties"]["from"] = from();
        operation["required"]
            .as_array_mut()
            .unwrap()
            .push(json!("from"));
    }
    let mut setup = json!({
        "orientation": choice(&ups, false),
        "name": text(),
        "origin": choice(&work_zeros(), false),
        "workholding": text(),
        "holder": {"type": ["integer", "null"]},
        "operations": {"type": "array", "items": operation},
    });
    let mut plan = json!({
        "summary": text(),
        "unmachined": {"type": "array", "items": object(json!({"feature": text(), "reason": text()}))},
        "warnings": {"type": "array", "items": text()},
    });
    if edit {
        setup["from"] = from();
    }
    if rotary {
        setup["a_angles"] = json!({"type": ["array", "null"], "items": number()});
        plan["chuck"] = json!({"anyOf": [object(json!({
            "stock": choice(&STOCKS, false),
            "length": number(),
            "diameter": optional(),
            "width": optional(),
            "height": optional(),
            "jaws": choice(&crate::chuck::JAWS, false),
            "stickout": number(),
            "tailstock": choice(&crate::chuck::TAILSTOCK, false),
            "free_end": optional(),
        })), {"type": "null"}]});
    }
    plan["setups"] = json!({"type": "array", "items": object(setup)});
    object(plan)
}
pub fn parse(text: &str) -> Result<Proposal, String> {
    serde_json::from_str(text.trim())
        .map_err(|e| format!("The proposal is not valid JSON for the schema: {e}"))
}
/// One operation of a reply still arriving; a field not yet written is `None`.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct DraftOp {
    pub strategy: Option<String>,
    pub name: Option<String>,
    pub tool: Option<u64>,
    pub rpm: Option<f64>,
    pub feed: Option<f64>,
}
/// One setup of a reply still arriving.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct DraftSetup {
    pub name: Option<String>,
    pub orientation: Option<String>,
    pub operations: Vec<DraftOp>,
}
/// What a reply says so far.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Draft {
    pub summary: Option<String>,
    pub setups: Vec<DraftSetup>,
    /// The text is not a whole JSON object yet.
    pub writing: bool,
}
/// The most of a reply the timeline reads; plans are far shorter.
const PARTIAL_LIMIT: usize = 256 * 1024;
/// Nesting deeper than a proposal's is not read.
const PARTIAL_DEPTH: usize = 8;
/// The proposal as far as `text` has come: text before the first `{` is
/// skipped, a string cut short is closed, and a key, number or literal cut
/// short is left out with its member.
pub fn partial(text: &str) -> Draft {
    let Some(start) = text.find('{') else {
        return Draft::default();
    };
    let mut body = &text[start..];
    if let Some(Ok(value)) = serde_json::Deserializer::from_str(body)
        .into_iter::<Value>()
        .next()
    {
        return Draft::of(&value, false);
    }
    if body.len() > PARTIAL_LIMIT {
        let mut end = PARTIAL_LIMIT;
        while !body.is_char_boundary(end) {
            end -= 1;
        }
        body = &body[..end];
    }
    let (mut end, mut delimited) = (body.len(), false);
    for _ in 0..64 {
        let (candidate, boundary) = closed(&body[..end], delimited);
        if let Some(candidate) = candidate
            && let Ok(value) = serde_json::from_str::<Value>(&candidate)
        {
            return Draft::of(&value, true);
        }
        match boundary {
            // A cut at a comma or bracket leaves a text that ends in a delimiter.
            Some(b) if b < end => (end, delimited) = (b, true),
            _ => break,
        }
    }
    Draft {
        writing: true,
        ..Draft::default()
    }
}
/// `prefix` closed into JSON (None when it ends inside a number, literal or
/// key), and where to cut it back to: before its last `,`, or just after its
/// last `{` or `[`, outside strings. A `,` or whitespace after the last value
/// ends it, and so does `delimited`, which says the caller already cut
/// `prefix` at a comma or bracket; only a value with nothing after it may still
/// be growing. A trailing `,` is dropped.
fn closed(prefix: &str, delimited: bool) -> (Option<String>, Option<usize>) {
    let mut stack = vec![];
    let (mut in_string, mut escaped) = (false, false);
    // The last two cut points outside strings, the latest last.
    let (mut before, mut boundary) = (None, None);
    for (i, c) in prefix.char_indices() {
        if in_string {
            match (escaped, c) {
                (true, _) => escaped = false,
                (false, '\\') => escaped = true,
                (false, '"') => in_string = false,
                _ => {}
            }
            continue;
        }
        match c {
            '"' => in_string = true,
            '{' | '[' => {
                stack.push(if c == '{' { '}' } else { ']' });
                (before, boundary) = (boundary, Some(i + 1));
                if stack.len() > PARTIAL_DEPTH {
                    return (None, None);
                }
            }
            '}' | ']' => {
                stack.pop();
            }
            ',' => (before, boundary) = (boundary, Some(i)),
            _ => {}
        }
    }
    let mut out = prefix.to_string();
    if in_string {
        // Drop an escape cut short: a lone backslash or a \u without 4 digits.
        if let Some(at) = out.rfind('\\') {
            let tail = &out[at..];
            if escaped || (tail.starts_with("\\u") && tail.len() < 6) {
                out.truncate(at);
            }
        }
        out.push('"');
    } else {
        let kept = out.trim_end().len();
        let mut finished = delimited || kept < out.len();
        out.truncate(kept);
        if out.ends_with(',') {
            out.pop();
            let kept = out.trim_end().len();
            out.truncate(kept);
            (boundary, finished) = (before, true);
        }
        if !finished
            && out.ends_with(|c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':'))
        {
            return (None, boundary);
        }
    }
    out.extend(stack.iter().rev());
    (Some(out), boundary)
}
/// The objects among the elements of the array `value`: only those are setups
/// or operations, so a reply of `[1,1,1,…` makes none.
fn objects(value: &Value) -> impl Iterator<Item = &Value> {
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter(|item| item.is_object())
}
impl Draft {
    fn of(value: &Value, writing: bool) -> Self {
        let text = |v: &Value| v.as_str().map(String::from);
        Draft {
            summary: text(&value["summary"]),
            setups: objects(&value["setups"])
                .map(|s| DraftSetup {
                    name: text(&s["name"]),
                    orientation: text(&s["orientation"]),
                    operations: objects(&s["operations"])
                        .map(|o| DraftOp {
                            strategy: text(&o["strategy"]),
                            name: text(&o["name"]),
                            tool: o["tool"].as_u64(),
                            rpm: o["rpm"].as_f64(),
                            feed: o["feed"].as_f64(),
                        })
                        .collect(),
                })
                .collect(),
            writing,
        }
    }
}
/// Invisible characters that can reorder or hide text (Unicode category Cf).
fn format_character(c: char) -> bool {
    matches!(c, '\u{ad}' | '\u{600}'..='\u{605}' | '\u{61c}' | '\u{6dd}' | '\u{70f}' | '\u{180e}'
        | '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{2064}'
        | '\u{2066}'..='\u{206f}' | '\u{feff}' | '\u{fff9}'..='\u{fffb}' | '\u{e0001}'
        | '\u{e0020}'..='\u{e007f}')
}
/// One line of plain text: whitespace turned into spaces, other control and
/// format characters removed, whitespace collapsed, at most `limit` characters.
/// Reading stops once `limit` characters are kept, so a huge text costs no
/// more than a short one.
pub(crate) fn clip(text: &str, limit: usize) -> String {
    let mut line = String::new();
    let (mut kept, mut space) = (0, false);
    for c in text.chars() {
        if kept == limit {
            break;
        }
        if c.is_whitespace() {
            // A space is kept only between two words.
            space = kept > 0;
        } else if !c.is_control() && !format_character(c) {
            if space {
                line.push(' ');
                kept += 1;
                space = false;
                if kept == limit {
                    break;
                }
            }
            line.push(c);
            kept += 1;
        }
    }
    line
}
/// The plane of `orientation` that contains every listed surface.
fn plane_of<'a>(orientation: &'a OrientationReport, surfaces: &[usize]) -> Option<&'a PlaneReport> {
    orientation
        .planes
        .iter()
        .find(|p| !surfaces.is_empty() && surfaces.iter().all(|id| p.surfaces.contains(id)))
}
/// The floor planes of `orientation` that hold `surfaces`, when every one
/// of them is on a floor plane.
fn floors_of<'a>(
    orientation: &'a OrientationReport,
    surfaces: &[usize],
) -> Option<Vec<&'a PlaneReport>> {
    let mut planes: Vec<&PlaneReport> = vec![];
    for id in surfaces {
        let plane = orientation
            .planes
            .iter()
            .find(|p| p.kind == "floor" && p.surfaces.contains(id))?;
        if !planes.iter().any(|p| std::ptr::eq(*p, plane)) {
            planes.push(plane);
        }
    }
    (!planes.is_empty()).then_some(planes)
}
/// Checks an operation of setup `si` (from 0) and builds it. Returns what is
/// wrong instead when the tool, surfaces or choices do not fit the strategy.
/// `sides` is set for a Face of an indexed setup's several angles, whose
/// surfaces `faces_each_angle` has checked: the tool's centre goes out to
/// each face's edges, as the UI's facing of a square's sides does.
fn operation(
    p: &ProposedOp,
    si: usize,
    orientation: &OrientationReport,
    sides: bool,
    report: &PartReport,
    tools: &[Tool],
    fingerprint: &str,
) -> Result<Operation, Vec<String>> {
    let mut wrong = vec![];
    if p.surfaces.len() > MAX_SURFACES {
        wrong.push(format!(
            "The surfaces list has {} IDs; at most {MAX_SURFACES} are allowed.",
            p.surfaces.len()
        ));
    }
    let mut ids = p.surfaces.clone();
    ids.sort_unstable();
    ids.dedup();
    let strategy = STRATEGIES
        .iter()
        .find(|(name, _)| *name == p.strategy)
        .map(|(_, s)| *s);
    if strategy.is_none() {
        wrong.push(format!(
            "\"{}\" is not one of the six available strategies.",
            p.strategy
        ));
    }
    let tool = tools.iter().find(|t| t.number() == Some(p.tool));
    if tool.is_none() {
        wrong.push(format!("T{} is not in the tool library.", p.tool));
    }
    for (field, value) in [
        ("rpm", Some(p.rpm)),
        ("feed", Some(p.feed)),
        ("stepdown", p.stepdown),
        ("stepover_percent", p.stepover_percent),
        ("radial_allowance", p.radial_allowance),
        ("axial_allowance", p.axial_allowance),
        ("bottom_offset", p.bottom_offset),
        ("clearance", p.clearance),
        ("ramp_angle", p.ramp_angle),
        ("surface_stepover", p.surface_stepover),
        ("tolerance", p.tolerance),
        ("peck", p.peck),
        ("breakthrough", p.breakthrough),
        ("hole_diameter", p.hole_diameter),
    ] {
        if value.is_some_and(|v| !v.is_finite()) {
            wrong.push(format!("{field} is not a finite number."));
        }
    }
    // Negative stock to leave or bottom offset would cut below the model;
    // in setup 1 on a sheet a bottom offset may go into the spoilboard, by
    // its allowance at most.
    let board = report
        .sheet
        .as_ref()
        .filter(|_| si == 0)
        .map_or(0., |sheet| sheet.spoilboard_allowance);
    for (field, value, least) in [
        ("radial_allowance", p.radial_allowance, 0.),
        ("axial_allowance", p.axial_allowance, 0.),
        ("bottom_offset", p.bottom_offset, -board),
    ] {
        if value.is_some_and(|v| v < least - 0.0001 && v.is_finite()) {
            wrong.push(if least < 0. {
                format!(
                    "{field} cannot be below -{board}: a cut through the sheet goes at most {board} mm into the spoilboard."
                )
            } else {
                format!("{field} cannot be negative.")
            });
        }
    }
    if let Some(tabs) = &p.tabs {
        if p.strategy != "Contour" || p.contour_mode.as_deref().is_some_and(|m| m != "outside") {
            wrong
                .push("tabs are for a Contour with contour_mode outside; set tabs to null.".into());
        }
        if !(1..=50).contains(&tabs.count)
            || !(tabs.width.is_finite() && tabs.width > 0.)
            || !(tabs.height.is_finite() && tabs.height > 0.)
        {
            wrong.push("tabs need a count of 1-50 and a positive width and height.".into());
        }
    }
    // Every planned operation travels at the default height or above it: a
    // plan never takes away the margin the default leaves over what the app
    // does not check (bolts, clamps it does not model).
    if p.clearance.is_some_and(|v| v.is_finite() && v < CLEARANCE) {
        wrong.push(format!(
            "clearance can only raise the default of {CLEARANCE} mm."
        ));
    }
    // ... and not above what the machine can reach, or a few hundred mm when
    // its Z travel is not known: a sky-high plane makes rapids to Z+500 m.
    let cap = clearance_cap(report);
    if p.clearance.is_some_and(|v| v.is_finite() && v > cap) {
        wrong.push(format!(
            "clearance {} is above the largest this machine allows, {cap} mm.",
            p.clearance.unwrap_or_default()
        ));
    }
    if p.rpm.is_finite() && !(p.rpm >= 100. && p.rpm <= report.machine.max_rpm) {
        wrong.push(format!(
            "rpm {} is outside 100-{} for this machine.",
            p.rpm, report.machine.max_rpm
        ));
    }
    if p.feed.is_finite() && !(p.feed >= 1. && p.feed <= report.machine.max_feed) {
        wrong.push(format!(
            "feed {} is outside 1-{} mm/min for this machine.",
            p.feed, report.machine.max_feed
        ));
    }
    if !COOLANTS.contains(&p.coolant.as_str()) {
        wrong.push(format!("\"{}\" is not a coolant choice.", p.coolant));
    }
    let inside = p.contour_mode.as_deref() == Some("inside_floor");
    for (value, allowed, what) in [
        (&p.facing_area, &["stock", "model"][..], "facing area"),
        (&p.contour_mode, &CONTOUR_MODES[..], "contour mode"),
        (&p.pocket_entry, &ENTRIES[..], "pocket entry"),
        (&p.drill_cycle, &CYCLES[..], "drill cycle"),
    ] {
        if value.as_deref().is_some_and(|v| !allowed.contains(&v)) {
            wrong.push(format!(
                "\"{}\" is not a {what} choice.",
                value.as_deref().unwrap()
            ));
        }
    }
    let (Some(strategy), Some(tool)) = (strategy, tool) else {
        return Err(wrong);
    };
    let kind = get(&tool.p, "type");
    let suits = match strategy {
        Strategy::Face => ["Flat end mill", "Face mill"].contains(&kind),
        Strategy::Drill if suits_helical(p) => kind == "Flat end mill",
        Strategy::Drill if suits_cycle(p, crate::model::THREAD_MILLING) => kind == "Thread mill",
        Strategy::Drill if suits_cycle(p, crate::model::TAPPING) => kind == "Tap",
        Strategy::Drill => ["Drill", "Chamfer mill"].contains(&kind),
        Strategy::Parallel => ["Ball end mill", "Flat end mill"].contains(&kind),
        _ => kind == "Flat end mill",
    };
    if !suits && kind != "Flat end mill" && suits_helical(p) {
        wrong.push(format!(
            "T{} is a {kind}; Drill with Helical milling takes a flat end mill.",
            p.tool
        ));
    } else if !suits && suits_cycle(p, crate::model::THREAD_MILLING) {
        wrong.push(format!(
            "T{} is a {kind}; Drill with Thread milling takes a thread mill.",
            p.tool
        ));
    } else if !suits && suits_cycle(p, crate::model::TAPPING) {
        wrong.push(format!(
            "T{} is a {kind}; Drill with Tapping takes a tap.",
            p.tool
        ));
    } else if !suits {
        wrong.push(format!(
            "T{} is a {kind}, which {} cannot use.",
            p.tool, p.strategy
        ));
    }
    let plane = plane_of(orientation, &ids);
    let floor = strategy == Strategy::Pocket || (strategy == Strategy::Contour && inside);
    match strategy {
        Strategy::Face if sides => {}
        Strategy::Face if !plane.is_some_and(|pl| pl.kind == "top") => wrong.push(format!(
            "Facing needs surfaces of the top plane in orientation {}.",
            orientation.up
        )),
        Strategy::Drill if !plane.is_some_and(|pl| !pl.holes.is_empty()) => wrong.push(format!(
            "Drilling needs the surfaces of one plane with holes in orientation {}.",
            orientation.up
        )),
        // A Pocket clears each of the floor planes it names to its own depth.
        Strategy::Pocket => match floors_of(orientation, &ids) {
            None => wrong.push(format!(
                "Pocket needs the surfaces of one or more floor planes in orientation {}.",
                orientation.up
            )),
            Some(planes) => {
                let diameter = num(&tool.p, "diameter").unwrap_or(0.);
                let fit = planes
                    .iter()
                    .map(|pl| pl.max_cutter_diameter.unwrap_or(f32::INFINITY))
                    .fold(0., f32::max);
                if diameter > fit {
                    wrong.push(format!(
                        "T{} (diameter {diameter} mm) does not fit {}; the largest cutter that fits is {fit} mm.",
                        p.tool,
                        if planes.len() == 1 { "this floor" } else { "any of these floors" }
                    ));
                }
                if p.through == Some(true) && planes.iter().all(|pl| pl.openings.is_empty()) {
                    wrong.push("through is true, but none of these floors has openings that go through the part. Set through to false.".into());
                }
            }
        },
        _ if floor => match plane.filter(|pl| pl.kind == "floor") {
            None => wrong.push(format!(
                "{} needs the surfaces of one floor plane in orientation {}.",
                p.strategy, orientation.up
            )),
            Some(pl) => {
                let diameter = num(&tool.p, "diameter").unwrap_or(0.);
                if pl.max_cutter_diameter.is_some_and(|fit| diameter > fit) {
                    wrong.push(format!(
                        "T{} (diameter {diameter} mm) does not fit this floor; the largest cutter that fits is {} mm.",
                        p.tool,
                        pl.max_cutter_diameter.unwrap()
                    ));
                }
            }
        },
        Strategy::Face | Strategy::Drill => {}
        _ if !ids.is_empty() => wrong.push(format!(
            "{} works on the whole model; its surfaces list must be empty.",
            p.strategy
        )),
        _ => {}
    }
    if strategy == Strategy::Drill && p.through == Some(true) && si > 0 {
        // Later setups may not go below the part's underside (plan check).
        wrong.push(format!(
            "through is true, but in setup {} the part rests on the face where the drill would exit, so it cannot break through. Drill through holes in setup 1, or set through to false and list the unfinished exit in unmachined.",
            si + 1
        ));
    } else if strategy == Strategy::Drill && p.through == Some(true) {
        // The holes the generator will drill: the plane's, narrowed by diameter.
        let filter = p.hole_diameter.filter(|d| *d > 0.);
        if let Some(blind) = plane
            .iter()
            .flat_map(|pl| &pl.holes)
            .filter(|h| filter.is_none_or(|d| (h.diameter - d).abs() <= 0.11))
            .find(|h| !h.through)
        {
            wrong.push(format!(
                "through is true, but the selection includes a blind hole (diameter {} mm, depth {} mm). Set through to false, or use hole_diameter to select only through holes.",
                blind.diameter, blind.depth
            ));
        }
        let cap = num(&tool.p, "diameter").unwrap_or(0.) * 0.5 + 1.;
        let breakthrough = p.breakthrough.unwrap_or(1.);
        if breakthrough.is_finite() && !(breakthrough >= 0. && breakthrough <= cap + 0.0001) {
            wrong.push(format!(
                "breakthrough {breakthrough} mm is outside 0-{} mm for this drill (half its diameter plus 1 mm).",
                (cap * 100.).round() / 100.
            ));
        }
    }
    let selection = (!ids.is_empty()).then(|| Selection {
        mesh_fingerprint: fingerprint.into(),
        ids,
    });
    if selection.as_ref().is_some_and(|s| !s.valid_structure()) {
        wrong.push("The surfaces list is not valid for this model.".into());
    }
    if !wrong.is_empty() {
        return Err(wrong);
    }
    let mut op = Operation::new(strategy, tool.id.clone());
    op.geometry.clear();
    op.surfaces = selection;
    let mut set = |key: &str, value: String| {
        op.p.insert(key.into(), value);
    };
    let name = clip(&p.name, 60);
    set(
        "name",
        if name.is_empty() {
            strategy.name().into()
        } else {
            name
        },
    );
    set("rpm", p.rpm.to_string());
    set("feed", p.feed.to_string());
    set("coolant", p.coolant.clone());
    // One operation machines to size unless the proposal leaves stock.
    set("allowance", p.radial_allowance.unwrap_or(0.).to_string());
    set(
        "axial_allowance",
        p.axial_allowance.unwrap_or(0.).to_string(),
    );
    set("bottom", p.bottom_offset.unwrap_or(0.).to_string());
    for (key, value) in [
        ("stepdown", p.stepdown),
        ("stepover", p.stepover_percent),
        ("clearance", p.clearance),
        ("ramp_angle", p.ramp_angle),
        ("surface_stepover", p.surface_stepover),
        ("tolerance", p.tolerance),
        ("peck", p.peck),
        ("breakthrough", p.breakthrough),
        ("hole_diameter", p.hole_diameter),
        ("finish_passes", p.finish_passes.map(|n| n as f32)),
    ] {
        if let Some(value) = value {
            set(key, value.to_string());
        }
    }
    // Combinations the generators require are fixed here, not by the model.
    match strategy {
        Strategy::Face if sides => {
            set("boundary", "Selected geometry".into());
            set("containment", "Tool center on boundary".into());
            set("allowance", "0".into());
        }
        Strategy::Face => {
            let model = p.facing_area.as_deref() == Some("model");
            set(
                "boundary",
                if model {
                    "Model silhouette"
                } else {
                    "Stock silhouette"
                }
                .into(),
            );
            set("containment", "Tool outside boundary".into());
            set("allowance", "0".into());
            // Past the outline far enough for the cutter's corners.
            if let Some(d) = num(&tool.p, "diameter").filter(|d| *d > 0.) {
                set("silhouette_extend", crate::model::outline_extend_for(d));
            }
        }
        Strategy::Pocket => {
            set("boundary", "Selected geometry".into());
            set("containment", "Tool inside boundary".into());
            set(
                "openings",
                crate::model::POCKET_OPENINGS[usize::from(p.through == Some(true))].into(),
            );
        }
        Strategy::Contour if inside => {
            set("boundary", "Selected geometry".into());
            set("side", "Inside".into());
            set("containment", "Tool inside boundary".into());
        }
        // Inside every hole through the outline at least `hole_diameter`
        // wide: a flat part's cut-outs.
        Strategy::Contour if p.contour_mode.as_deref() == Some("inside_holes") => {
            set("boundary", "Model silhouette".into());
            set("side", crate::model::CONTOUR_HOLES.into());
            set("containment", "Tool inside boundary".into());
            if let Some(d) = p.hole_diameter.filter(|d| *d > 0.) {
                set("hole_min", d.to_string());
            }
        }
        Strategy::Contour => {
            set("boundary", "Model silhouette".into());
            set("side", "Outside".into());
            set("containment", "Tool outside boundary".into());
            let tabs = p.tabs.as_ref();
            set(
                "tabs",
                crate::model::CONTOUR_TABS[usize::from(tabs.is_some())].into(),
            );
            if let Some(tabs) = tabs {
                for (key, value) in [
                    ("tab_count", tabs.count as f32),
                    ("tab_width", tabs.width),
                    ("tab_height", tabs.height),
                ] {
                    set(key, value.to_string());
                }
            }
        }
        Strategy::Drill => {
            set(
                "cycle",
                p.drill_cycle.clone().unwrap_or_else(|| "Drilling".into()),
            );
            set(
                "through",
                if p.through == Some(true) { "Yes" } else { "No" }.into(),
            );
        }
        _ => set("boundary", "Model silhouette".into()),
    }
    if let Some(entry) = &p.pocket_entry {
        set("pocket_entry", entry.clone());
    }
    Ok(op)
}
/// Whether `p` is a Drill operation with the Helical milling cycle, which
/// cuts with a flat end mill.
fn suits_helical(p: &ProposedOp) -> bool {
    suits_cycle(p, crate::model::HELICAL_MILLING)
}
/// Whether `p` is a Drill operation with the cycle `cycle`.
fn suits_cycle(p: &ProposedOp, cycle: &str) -> bool {
    p.strategy == "Drill" && p.drill_cycle.as_deref() == Some(cycle)
}
/// An operation of the current plan the planner cannot see or change: its
/// strategy is not one of the six, its tool has no number, or it machines a
/// selection that is not surfaces. It stays in its setup, at its place.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct KeptOp {
    pub setup: usize,
    pub from: usize,
    /// Its place in the setup, from 1.
    pub position: usize,
    pub strategy: &'static str,
    pub name: String,
}
/// The current plan as the planner is shown it when it is asked to change
/// it: every setup and operation in the proposal's own format, each with
/// its `from` (setups numbered from 1, operations from 1 across the whole
/// job), and the operations it cannot change.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Current {
    pub current_plan: Proposal,
    pub kept_operations: Vec<KeptOp>,
}
/// `op` as a proposal writes it, or None when a proposal cannot (see
/// [`KeptOp`]). Fields its strategy does not use are null, and `reason` is
/// empty; a coolant that is not a choice reads `Off`.
fn described(op: &Operation, tools: &[Tool]) -> Option<ProposedOp> {
    let strategy = STRATEGIES.iter().find(|(_, s)| *s == op.strategy)?.0;
    let tool = tools.iter().find(|t| t.id == op.tool)?.number()?;
    if op.surfaces.is_none() && !op.geometry.is_empty() {
        return None;
    }
    let mut surfaces = op
        .surfaces
        .as_ref()
        .map(|s| s.ids.clone())
        .unwrap_or_default();
    surfaces.sort_unstable();
    surfaces.dedup();
    let value = |key: &str| num(&op.p, key).filter(|v| v.is_finite());
    let coolant = get(&op.p, "coolant");
    let mut p = ProposedOp {
        strategy: strategy.into(),
        name: crate::planner::name(get(&op.p, "name")),
        tool,
        surfaces,
        rpm: value("rpm").unwrap_or(0.),
        feed: value("feed").unwrap_or(0.),
        coolant: if COOLANTS.contains(&coolant) {
            coolant
        } else {
            "Off"
        }
        .into(),
        ..ProposedOp::default()
    };
    let clearance = value("clearance").filter(|c| *c != CLEARANCE);
    let passes = value("finish_passes").map(|n| n.max(0.).round() as u32);
    let entry = Some(get(&op.p, "pocket_entry"))
        .filter(|e| ENTRIES.contains(e))
        .map(String::from);
    p.clearance = clearance;
    match op.strategy {
        Strategy::Face => {
            p.stepdown = value("stepdown");
            p.stepover_percent = value("stepover");
            p.bottom_offset = value("bottom");
            let model = get(&op.p, "boundary") == "Model silhouette";
            p.facing_area = Some(if model { "model" } else { "stock" }.into());
        }
        Strategy::Pocket => {
            p.stepdown = value("stepdown");
            p.stepover_percent = value("stepover");
            p.radial_allowance = value("allowance");
            p.axial_allowance = value("axial_allowance");
            p.finish_passes = passes;
            p.pocket_entry = entry;
            p.ramp_angle = value("ramp_angle");
            let through = get(&op.p, "openings") == crate::model::POCKET_OPENINGS[1];
            p.through = Some(through);
            p.breakthrough = value("breakthrough").filter(|_| through);
        }
        Strategy::Contour => {
            let inside = get(&op.p, "side") == "Inside";
            let holes = get(&op.p, "side") == crate::model::CONTOUR_HOLES;
            p.contour_mode = Some(
                if inside {
                    "inside_floor"
                } else if holes {
                    "inside_holes"
                } else {
                    "outside"
                }
                .into(),
            );
            if holes {
                p.hole_diameter = value("hole_min").filter(|d| *d > 0.);
            } else if !inside && pocket_setting(&op.p, "tabs") == crate::model::CONTOUR_TABS[1] {
                p.tabs = Some(ProposedTabs {
                    count: value("tab_count").unwrap_or(4.).max(0.).round() as u32,
                    width: value("tab_width").unwrap_or(6.),
                    height: value("tab_height").unwrap_or(1.5),
                });
            }
            p.stepdown = value("stepdown");
            p.radial_allowance = value("allowance");
            p.axial_allowance = value("axial_allowance");
            p.finish_passes = passes;
            if inside {
                p.pocket_entry = entry;
                p.ramp_angle = value("ramp_angle");
            } else {
                p.bottom_offset = value("bottom");
            }
        }
        Strategy::Drill => {
            let cycle = get(&op.p, "cycle");
            p.drill_cycle = Some(
                if CYCLES.contains(&cycle) {
                    cycle
                } else {
                    CYCLES[0]
                }
                .into(),
            );
            p.peck = value("peck");
            // Helical milling's pitch and rings.
            if op.helical() {
                p.stepdown = value("stepdown");
                p.stepover_percent = value("stepover");
            }
            let through = get(&op.p, "through") == "Yes";
            p.through = Some(through);
            p.breakthrough = value("breakthrough").filter(|_| through);
            p.hole_diameter = value("hole_diameter").filter(|d| *d > 0.);
        }
        Strategy::Parallel => {
            p.surface_stepover = value("surface_stepover");
            p.tolerance = value("tolerance");
            p.radial_allowance = value("allowance");
            p.axial_allowance = value("axial_allowance");
        }
        _ => {
            p.stepdown = value("stepdown");
            p.bottom_offset = value("bottom");
            p.radial_allowance = value("allowance");
            p.axial_allowance = value("axial_allowance");
        }
    }
    Some(p)
}
/// Whether two names are one: `proposed` is the name as the planner cleans
/// it, `current` the operation's or setup's own.
fn same_name(proposed: &str, current: &str) -> bool {
    proposed == current || proposed == crate::planner::name(current)
}
/// Whether `p` asks for nothing other than `d`, the operation it keeps as
/// [`described`] writes it: its reason does not count, nor a name the
/// planner only cleaned, nor the order of its surfaces.
fn unchanged(p: &ProposedOp, d: &ProposedOp, current_name: &str) -> bool {
    let mut p = p.clone();
    p.reason.clone_from(&d.reason);
    p.from = d.from;
    p.surfaces.sort_unstable();
    p.surfaces.dedup();
    if same_name(&p.name, current_name) {
        p.name.clone_from(&d.name);
    }
    p == *d
}
/// `base`, an operation the planner keeps, with what `p` changed of `d` (its
/// description) written from `fresh` (`p` built from scratch): every other
/// setting stays as the user left it, including those a proposal does not
/// carry.
fn merged(base: &Operation, fresh: &Operation, p: &ProposedOp, d: &ProposedOp) -> Operation {
    let mut op = base.clone();
    let mut copy = |keys: &[&str]| {
        for key in keys {
            match fresh.p.get(*key) {
                Some(value) => op.p.insert((*key).into(), value.clone()),
                None => op.p.remove(*key),
            };
        }
    };
    let mut surfaces = p.surfaces.clone();
    surfaces.sort_unstable();
    surfaces.dedup();
    if !same_name(&p.name, get(&base.p, "name")) {
        copy(&["name"]);
    }
    if p.rpm != d.rpm {
        copy(&["rpm"]);
    }
    if p.feed != d.feed {
        copy(&["feed"]);
    }
    if p.coolant != d.coolant {
        copy(&["coolant"]);
    }
    for (changed, keys) in [
        (p.stepdown != d.stepdown, &["stepdown"][..]),
        (p.stepover_percent != d.stepover_percent, &["stepover"]),
        (p.radial_allowance != d.radial_allowance, &["allowance"]),
        (p.axial_allowance != d.axial_allowance, &["axial_allowance"]),
        (p.bottom_offset != d.bottom_offset, &["bottom"]),
        (p.clearance != d.clearance, &["clearance"]),
        (p.finish_passes != d.finish_passes, &["finish_passes"]),
        // How far past the outline a Facing clears is sized to its cutter.
        (
            p.facing_area != d.facing_area,
            &["boundary", "silhouette_extend"],
        ),
        (p.pocket_entry != d.pocket_entry, &["pocket_entry"]),
        (p.ramp_angle != d.ramp_angle, &["ramp_angle"]),
        (
            p.surface_stepover != d.surface_stepover,
            &["surface_stepover"],
        ),
        (p.tolerance != d.tolerance, &["tolerance"]),
        (p.drill_cycle != d.drill_cycle, &["cycle"]),
        (p.peck != d.peck, &["peck"]),
        (p.breakthrough != d.breakthrough, &["breakthrough"]),
        (
            p.hole_diameter != d.hole_diameter,
            &["hole_diameter", "hole_min"],
        ),
        (p.through != d.through, &["through", "openings"]),
        (
            p.tabs != d.tabs,
            &["tabs", "tab_count", "tab_width", "tab_height"],
        ),
    ] {
        if changed {
            copy(keys);
        }
    }
    if p.tool != d.tool {
        op.tool.clone_from(&fresh.tool);
        if base.strategy == Strategy::Face {
            copy(&["silhouette_extend"]);
        }
    }
    if surfaces != d.surfaces {
        op.surfaces.clone_from(&fresh.surfaces);
        op.geometry.clone_from(&fresh.geometry);
    }
    op
}
/// What a proposal writes of setup `si` of `doc`, the planner's last note
/// for setup 1 as its `workholding`: its name, work zero, workholding note
/// and holder.
fn setup_fields(doc: &Document, si: usize) -> (String, String, String, Option<usize>) {
    let setup = &doc.setups[si];
    let setup1 = &doc.setups[0];
    let origin = get(&setup.p, "origin");
    let origin = if work_zeros().contains(&origin) {
        origin
    } else {
        "Model origin"
    };
    let (notes, holder) = if si == 0 {
        let note = get(&setup.p, crate::planner::PLANNER_NOTE_KEY);
        (note, analysis::own_entry(doc, setup1))
    } else {
        let same = ["holding", "holder"]
            .iter()
            .all(|k| get(&setup.p, k) == get(&setup1.p, k));
        let holder = (!same && crate::holding::modelled(setup))
            .then(|| {
                analysis::library(doc)
                    .iter()
                    .position(|h| h.id == get(&setup.p, "holder"))
            })
            .flatten()
            .map(|i| i + 1);
        (get(&setup.p, "notes"), holder)
    };
    (
        crate::planner::name(setup.name()),
        origin.into(),
        clip(notes, 500),
        holder,
    )
}
/// The current plan of `doc` as the planner is shown it when it is asked to
/// change it. Fails when a setup after the first is not turned square to the
/// part: no orientation names it.
pub fn describe(doc: &Document, report: &PartReport) -> Result<Current, String> {
    if doc.setups.is_empty() {
        return Err("The project has no setup 1.".into());
    }
    let (mut setups, mut kept, mut number) = (vec![], vec![], 0);
    for (si, setup) in doc.setups.iter().enumerate() {
        // A plan's chuck and A angles are made afresh (`build`), not kept.
        if crate::chuck::holds(setup) || crate::indexed::is_indexed(setup) {
            return Err(format!(
                "Setup {} {}, so the planner cannot change this plan. Ask it to plan the job again with an empty request, or change the setups by hand.",
                si + 1,
                if crate::indexed::is_indexed(setup) {
                    "indexes A"
                } else {
                    "holds the stock in the chuck on A"
                }
            ));
        }
        let up = if si == 0 {
            report.orientations.first().map(|o| o.up)
        } else {
            analysis::up_axis(setup)
        };
        let Some(up) = up else {
            return Err(format!(
                "Setup {} is not turned square to the part, so the planner cannot change this plan.",
                si + 1
            ));
        };
        let mut operations = vec![];
        for (oi, op) in setup.operations.iter().enumerate() {
            number += 1;
            match described(op, &doc.tools) {
                Some(p) => operations.push(ProposedOp {
                    from: Some(number),
                    ..p
                }),
                None => kept.push(KeptOp {
                    setup: si + 1,
                    from: number,
                    position: oi + 1,
                    strategy: op.strategy.name(),
                    name: crate::planner::name(get(&op.p, "name")),
                }),
            }
        }
        let (name, origin, workholding, holder) = setup_fields(doc, si);
        setups.push(ProposedSetup {
            orientation: up.into(),
            name,
            origin,
            workholding,
            holder,
            a_angles: None,
            operations,
            from: Some(si + 1),
        });
    }
    Ok(Current {
        current_plan: Proposal {
            summary: String::new(),
            setups,
            chuck: None,
            unmachined: vec![],
            warnings: vec![],
        },
        kept_operations: kept,
    })
}
/// Why a setup cannot index A without the stock in the chuck.
const INDEX_NEEDS_CHUCK: &str = "a_angles index the stock in the chuck: give `chuck` (or have setup 1 held in the chuck) and put this setup after it, or give a_angles null.";
/// Why the first setup cannot index A.
const FIRST_INDEXED: &str = "The first setup cannot index A: it is the clamping later setups turn. Give its a_angles null and index the setups after it.";
/// Why an operation with surfaces cannot run at several A angles.
const SURFACES_AT_ANGLES: &str = "This setup runs every operation at each of its A angles, but its surfaces face up in one orientation only. At several angles use Contour outside, Parallel or Waterline (surfaces empty), a Face whose surfaces are the top plane's IDs of the orientation each angle brings up (all the sides to face), or give each angle its own setup.";
/// Whether `p` is a Facing of the sides an indexed setup brings up, one top
/// plane of the orientations `at` (one per angle) after the other: every
/// surface lies in the top plane of one of them and every angle has a face
/// of its own to face (LIO-214). The Facing then faces, at each angle, the
/// faces that are on top there (`indexed::on_top`).
fn faces_each_angle(p: &ProposedOp, at: &[Option<&OrientationReport>]) -> Result<(), String> {
    fn top(o: &OrientationReport) -> Option<&PlaneReport> {
        o.planes.iter().find(|pl| pl.kind == "top")
    }
    if p.strategy != "Face" {
        return Err(SURFACES_AT_ANGLES.into());
    }
    let Some(at) = at.iter().copied().collect::<Option<Vec<_>>>() else {
        return Err(format!(
            "An A angle of this setup brings no face of the part up, so a Face has none to face there. {SURFACES_AT_ANGLES}"
        ));
    };
    if let Some(id) = p.surfaces.iter().find(|id| {
        !at.iter()
            .any(|o| top(o).is_some_and(|pl| pl.surfaces.contains(id)))
    }) {
        return Err(format!(
            "Surface {id} is not the top plane of any orientation this setup's A angles bring up. {SURFACES_AT_ANGLES}"
        ));
    }
    if let Some(o) = at
        .iter()
        .find(|o| !top(o).is_some_and(|pl| pl.surfaces.iter().any(|id| p.surfaces.contains(id))))
    {
        return Err(format!(
            "None of the surfaces is the top plane of {}, which an A angle of this setup brings up. {SURFACES_AT_ANGLES}",
            o.up
        ));
    }
    Ok(())
}
/// Seats setup 1's stock in the chuck the way the setup page does: the
/// stock fields, `Chuck` as its workholding with the proposal's jaws,
/// stick-out and tailstock, the A axis as its work zero, and the part turned
/// to `up` and placed on the bar's axis in the stick-out, `free_end` mm short
/// of the free end. The chuck's defaults and the clearance gap are filled in
/// afterwards by `Document::settle_holding`.
fn in_chuck(setup: &mut Setup, chuck: &ProposedChuck, up: &str, mesh: &Mesh) -> Result<(), String> {
    let positive = |v: f32| v.is_finite() && v > 0.;
    let round = match chuck.stock.as_str() {
        "Round bar" => true,
        "Box" => false,
        other => return Err(format!("\"{other}\" is not a chuck stock.")),
    };
    let need = |name: &str, v: Option<f32>| {
        v.filter(|v| positive(*v)).ok_or_else(|| {
            format!(
                "chuck.{name} must be a positive number for {}.",
                chuck.stock
            )
        })
    };
    let length = need("length", Some(chuck.length))?;
    let stickout = need("stickout", Some(chuck.stickout))?;
    let (diameter, width, height) = if round {
        let d = need("diameter", chuck.diameter)?;
        (d, d, d)
    } else {
        (
            0.,
            need("width", chuck.width)?,
            need("height", chuck.height)?,
        )
    };
    let free_end = chuck.free_end.unwrap_or(FREE_END);
    if !(free_end.is_finite() && free_end >= 0.) {
        return Err("chuck.free_end cannot be negative.".into());
    }
    for (value, choices, what) in [
        (&chuck.jaws, &crate::chuck::JAWS[..], "jaws"),
        (&chuck.tailstock, &crate::chuck::TAILSTOCK[..], "tailstock"),
    ] {
        if !choices.contains(&value.as_str()) {
            return Err(format!("\"{value}\" is not a chuck {what} choice."));
        }
    }
    let rotation = analysis::rotation_for(setup, up).ok_or("Setup 1 has no such orientation.")?;
    for (key, value) in ["rx", "ry", "rz", "tx", "ty", "tz"]
        .iter()
        .zip(rotation.into_iter().chain([0.; 3]))
    {
        setup.p.insert((*key).into(), value.to_string());
    }
    // The part in the bar's frame: X along it, the stock centred on Y and
    // standing on Z0 (`stock::resolve`).
    let scale = mesh.display_scale();
    let placed = crate::stock::Placement::unturned(mesh, Some(setup)).bounds;
    let size = placed.dimensions().map(|v| v / scale);
    let across = if round { size[1].hypot(size[2]) } else { 0. };
    if round && across > diameter {
        return Err(format!(
            "In orientation {up} the part is {:.2} × {:.2} mm across the bar, which needs a bar of at least Ø{:.2} mm, not Ø{diameter} mm.",
            size[1],
            size[2],
            (across * 100.).ceil() / 100.
        ));
    }
    if !round && (size[1] > width || size[2] > height) {
        return Err(format!(
            "In orientation {up} the part is {:.2} × {:.2} mm across the bar, which does not fit a box of {width} × {height} mm.",
            size[1], size[2]
        ));
    }
    if size[0] + free_end > stickout {
        return Err(format!(
            "In orientation {up} the part is {:.2} mm along X: with {free_end} mm of stock beyond it, it needs a stick-out of at least {:.2} mm, not {stickout} mm.",
            size[0],
            ((size[0] + free_end) * 100.).ceil() / 100.
        ));
    }
    let [tx, ty, tz] = [
        length / 2. - free_end - placed.max[0] / scale,
        -(placed.min[1] + placed.max[1]) / 2. / scale,
        height / 2. - 4. / scale - size[2] / 2.,
    ];
    for (key, value) in [("tx", tx), ("ty", ty), ("tz", tz)] {
        setup.p.insert(key.into(), value.to_string());
    }
    let p = &mut setup.p;
    if round {
        p.insert("stock_type".into(), "Round bar".into());
        p.insert("stock_diameter".into(), diameter.to_string());
        p.insert("stock_length".into(), length.to_string());
    } else {
        p.insert("stock_type".into(), "Fixed size box".into());
        for (key, value) in [("stock_x", length), ("stock_y", width), ("stock_z", height)] {
            p.insert(key.into(), value.to_string());
        }
    }
    p.insert("holding".into(), "Chuck".into());
    p.insert("chuck_jaws".into(), chuck.jaws.clone());
    p.insert("chuck_stickout".into(), stickout.to_string());
    p.insert("chuck_tailstock".into(), chuck.tailstock.clone());
    if !crate::indexed::ORIGINS.contains(&get(p, "origin")) {
        p.insert("origin".into(), crate::indexed::ORIGINS[0].into());
    }
    Ok(())
}
/// Builds the job a proposal describes. Setup 1 keeps the user's machine,
/// stock, work zero and fixture; its operations and all later setups are
/// replaced. Returns every reference problem instead when there is any.
///
/// When the planner changes the current plan, a setup or operation whose
/// `from` names one of `doc` (numbered as [`describe`] numbers them) keeps
/// it: an operation it does not change stays exactly as it is, a changed
/// one has only the changed settings written, and a kept setup keeps its id,
/// work offset, notes and workholding placement but for what the proposal
/// changes. The operations a proposal cannot carry ([`KeptOp`]) stay in
/// their kept setups, at their place.
pub fn build(
    doc: &Document,
    mesh: &Mesh,
    report: &PartReport,
    proposal: &Proposal,
) -> Result<Document, Vec<Problem>> {
    let mut problems = vec![];
    let mut whole = |message: String| {
        problems.push(Problem {
            setup: 0,
            operation: None,
            stage: "structure",
            message,
        })
    };
    let Some(setup1) = doc.setups.first() else {
        return Err(vec![Problem {
            setup: 0,
            operation: None,
            stage: "structure",
            message: "The project has no setup 1.".into(),
        }]);
    };
    if proposal.setups.is_empty() {
        whole("The proposal has no setups.".into());
    }
    if proposal.setups.len() > MAX_SETUPS {
        whole(format!(
            "The proposal has {} setups; at most {MAX_SETUPS} work offsets are available.",
            proposal.setups.len()
        ));
    }
    let total: usize = proposal.setups.iter().map(|s| s.operations.len()).sum();
    if total > MAX_OPERATIONS {
        whole(format!(
            "The proposal has {total} operations; at most {MAX_OPERATIONS} are allowed."
        ));
    }
    let first = report.orientations.first().map(|o| o.up);
    let rotary = report.rotary.is_some();
    if !rotary && proposal.chuck.is_some() {
        whole("`chuck` must be null: this machine has no rotary A axis.".into());
    }
    if !rotary && proposal.setups.iter().any(|s| s.a_angles.is_some()) {
        whole("`a_angles` must be null: this machine has no rotary A axis.".into());
    }
    // A stock in the chuck is turned to the orientation the proposal gives
    // setup 1; otherwise setup 1 keeps the user's.
    if proposal.chuck.is_none()
        && proposal
            .setups
            .first()
            .is_some_and(|s| Some(s.orientation.as_str()) != first)
    {
        whole(format!(
            "The first setup must use orientation {}, where the stock is defined.",
            first.unwrap_or("?")
        ));
    }
    let editing = proposal
        .setups
        .iter()
        .any(|s| s.from.is_some() || s.operations.iter().any(|o| o.from.is_some()));
    // The current plan's operations by their number, from 1: (setup, operation).
    let numbered: Vec<(usize, usize)> = doc
        .setups
        .iter()
        .enumerate()
        .flat_map(|(si, s)| (0..s.operations.len()).map(move |oi| (si, oi)))
        .collect();
    // The work offsets of the current setups a later setup may keep, which
    // a new setup does not take.
    let reserved: Vec<&str> = proposal
        .setups
        .iter()
        .skip(1)
        .filter_map(|s| s.from.and_then(|n| n.checked_sub(1)))
        .filter(|&b| b > 0)
        .filter_map(|b| doc.setups.get(b))
        .map(|s| get(&s.p, "offset"))
        .collect();
    // The entries a proposal's `holder` counts over, as the report numbers
    // them, and the number of setup 1's own, which only setup 1 may name.
    let library = analysis::library(doc);
    let own = analysis::own_entry(doc, setup1);
    let mut setups: Vec<Setup> = vec![];
    // The current setups and operations kept so far, by index and number.
    let (mut kept_setups, mut kept_ops) = (vec![], vec![]);
    for (si, proposed) in proposal.setups.iter().take(MAX_SETUPS).enumerate() {
        let mut wrong = |operation: Option<usize>, message: String| {
            problems.push(Problem {
                setup: si + 1,
                operation,
                stage: "structure",
                message,
            })
        };
        // Setup 1 is held by the user's own entry, so for it `holder` only
        // says which that is; a later setup is held by the entry it names.
        let holder = proposed.holder.and_then(|number| {
            let Some(holder) = number.checked_sub(1).and_then(|i| library.get(i)) else {
                wrong(
                    None,
                    format!("workholding {number} is not in workholding_library."),
                );
                return None;
            };
            if si == 0 && own != Some(number) {
                let or = own.map(|n| format!(" or {n}")).unwrap_or_default();
                wrong(
                    None,
                    format!("holder must be null{or}: setup 1's workholding is the user's."),
                );
                return None;
            }
            (si > 0).then_some(*holder)
        });
        let angled = proposed.a_angles.is_some();
        let offset = (54..=59)
            .map(|n| format!("G{n}"))
            .find(|o| {
                !setups.iter().any(|s| get(&s.p, "offset") == o) && !reserved.contains(&o.as_str())
            })
            .unwrap_or_default();
        // An indexed setup is made as the Reposition choice makes it, and
        // shows the part as the setup it indexes does, turned by its first
        // A angle.
        let mut indexed = None;
        let mut at_angles: Option<Vec<Option<&OrientationReport>>> = None;
        let mut up = proposed.orientation.as_str();
        if let Some(list) = &proposed.a_angles {
            if si == 0 {
                wrong(None, FIRST_INDEXED.into());
                continue;
            }
            if proposed.holder.is_some() {
                wrong(
                    None,
                    "holder must be null: an indexed setup is held by the chuck of the setup it indexes.".into(),
                );
            }
            let base = setups.iter().rev().find(|s| !crate::indexed::is_indexed(s));
            let mut made = Setup::new(setup1.machine.clone(), &offset);
            crate::indexed::set_reposition(&mut made.p, crate::indexed::ACTION);
            made.p.insert(
                crate::indexed::ANGLES.into(),
                crate::indexed::angles_list(list),
            );
            let mut issues = crate::indexed::issues(&made);
            // A chuck the proposal gave but could not seat is said once, by
            // its own problem.
            if proposal.chuck.is_none() && !base.is_some_and(crate::chuck::holds) {
                issues.push(INDEX_NEEDS_CHUCK.into());
            } else if let (Some(base), Ok(listed)) = (
                base.filter(|b| crate::chuck::holds(b)),
                crate::indexed::angles(&made),
            ) {
                let turn = crate::indexed::sign(&made.machine) * listed[0];
                // The orientation each angle brings up, for a Face of the
                // sides.
                at_angles = Some(
                    (listed.iter())
                        .map(|a| {
                            let turn = crate::indexed::sign(&made.machine) * a;
                            let up = analysis::orientation_at(base, turn)?;
                            report.orientations.iter().find(|o| o.up == up)
                        })
                        .collect(),
                );
                match analysis::orientation_at(base, turn) {
                    Some(shown) if shown == proposed.orientation => up = shown,
                    Some(shown) => issues.push(format!(
                        "At A {}° the part shows {shown} up, not {}: an indexed setup's orientation is the one its first A angle brings up (positive A turns +Y towards +Z).",
                        crate::indexed::degrees(listed[0]),
                        proposed.orientation
                    )),
                    None => issues.push(format!(
                        "At A {}° no face of the part is up: index to angles that turn a face of setup 1's orientation up, such as multiples of 90°.",
                        crate::indexed::degrees(listed[0])
                    )),
                }
            }
            if !issues.is_empty() {
                issues.into_iter().for_each(|m| wrong(None, m));
                continue;
            }
            indexed = Some(made);
        }
        let Some(orientation) = report.orientations.iter().find(|o| o.up == up) else {
            wrong(
                None,
                format!("\"{}\" is not an orientation.", proposed.orientation),
            );
            continue;
        };
        // The current setup this one keeps: the one its `from` names, when
        // that is still turned the same way (else this is a new setup). An
        // indexed setup is made afresh.
        let base = match proposed.from {
            None => None,
            Some(_) if angled => {
                wrong(
                    None,
                    "An indexed setup (a_angles) is made afresh: give it from null.".into(),
                );
                None
            }
            Some(n) if n == 0 || n > doc.setups.len() => {
                wrong(
                    None,
                    format!("from {n} is not a setup of the current plan."),
                );
                None
            }
            Some(n) if si == 0 && n != 1 => {
                wrong(
                    None,
                    "The first setup is the current setup 1: its from must be 1 or null.".into(),
                );
                None
            }
            Some(1) if si > 0 => {
                wrong(
                    None,
                    "Setup 1 stays first: no later setup can have from 1.".into(),
                );
                None
            }
            Some(n) if kept_setups.contains(&(n - 1)) => {
                wrong(None, format!("from {n} is used by an earlier setup."));
                None
            }
            Some(n) => {
                let up = if n == 1 {
                    first
                } else {
                    analysis::up_axis(&doc.setups[n - 1])
                };
                (up == Some(proposed.orientation.as_str())).then_some(n - 1)
            }
        };
        if let Some(b) = base {
            kept_setups.push(b);
        }
        if base.is_none()
            && !angled
            && proposal.setups[..si]
                .iter()
                .any(|s| s.a_angles.is_none() && s.orientation == proposed.orientation)
        {
            wrong(
                None,
                format!(
                    "Orientation {} is used by an earlier setup.",
                    proposed.orientation
                ),
            );
        }
        // What a kept setup was, as a proposal writes it; a field the
        // proposal changes is written, the rest stay.
        let current = base.map(|b| setup_fields(doc, b));
        let changed = |i: usize| {
            current
                .as_ref()
                .is_none_or(|(name, origin, notes, holder)| match i {
                    0 => !same_name(&proposed.name, name),
                    1 => proposed.origin != *origin,
                    2 => clip(&proposed.workholding, 500) != *notes,
                    _ => proposed.holder != *holder,
                })
        };
        if si > 0 && !angled && changed(1) && !work_zeros().contains(&proposed.origin.as_str()) {
            wrong(
                None,
                format!("\"{}\" is not a work zero choice.", proposed.origin),
            );
        }
        if proposed.operations.is_empty() {
            wrong(None, "The setup has no operations.".into());
        }
        if proposed.operations.len() > MAX_SETUP_OPERATIONS {
            wrong(
                None,
                format!(
                    "The setup has {} operations; at most {MAX_SETUP_OPERATIONS} are allowed.",
                    proposed.operations.len()
                ),
            );
        }
        let mut setup = if si == 0 {
            let mut setup = setup1.clone();
            if let Some(chuck) = &proposal.chuck
                && let Err(message) = in_chuck(&mut setup, chuck, orientation.up, mesh)
            {
                wrong(None, message);
            }
            setup
        } else if let Some(mut setup) = indexed {
            setup
                .p
                .insert("notes".into(), clip(&proposed.workholding, 500));
            setup
        } else if let Some(b) = base {
            let mut setup = doc.setups[b].clone();
            if changed(1) {
                setup.p.insert("origin".into(), proposed.origin.clone());
            }
            if changed(2) {
                setup
                    .p
                    .insert("notes".into(), clip(&proposed.workholding, 500));
            }
            if changed(3) {
                // Another entry, or setup 1's: laid out afresh by the check.
                for key in ["holding", "holder"] {
                    match setup1.p.get(key) {
                        Some(value) => setup.p.insert(key.into(), value.clone()),
                        None => setup.p.remove(key),
                    };
                }
                if let Some(holder) = holder {
                    let holding = holding_of(holder.kind());
                    setup.p.insert("holding".into(), holding.into());
                    setup.p.insert("holder".into(), holder.id.clone());
                }
                for key in ["vise_axis", "vise_offset", "vise_grip", "clamps"] {
                    setup.p.remove(key);
                }
            }
            setup
        } else {
            let mut setup = Setup::new(setup1.machine.clone(), &offset);
            let rotation = analysis::rotation_for(setup1, orientation.up).unwrap_or_default();
            for (key, value) in ["rx", "ry", "rz"].iter().zip(rotation) {
                setup.p.insert((*key).into(), value.to_string());
            }
            setup.p.insert("origin".into(), proposed.origin.clone());
            // The workholding entry the proposal chose, or setup 1's own;
            // the planner's advice for it is in the notes. Either is set
            // without a position, which a new setup does not have: the plan
            // check lays it out for this setup's toolpaths. The clearance gap
            // is the user's, from setup 1.
            setup
                .p
                .insert("fixture".into(), get(&setup1.p, "fixture").into());
            for key in ["holding", "holder", "holding_gap"] {
                if let Some(value) = setup1.p.get(key) {
                    setup.p.insert(key.into(), value.clone());
                }
            }
            if let Some(holder) = holder {
                let holding = holding_of(holder.kind());
                setup.p.insert("holding".into(), holding.into());
                setup.p.insert("holder".into(), holder.id.clone());
            }
            setup
                .p
                .insert("reposition".into(), "Reclamp and probe origin".into());
            setup
                .p
                .insert("notes".into(), clip(&proposed.workholding, 500));
            setup
        };
        if si > 0 && changed(0) {
            let name = clip(&proposed.name, 60);
            setup.p.insert(
                "name".into(),
                if name.is_empty() {
                    format!("Setup {}", si + 1)
                } else {
                    name
                },
            );
        }
        setup.operations.clear();
        for (oi, op) in proposed
            .operations
            .iter()
            .take(MAX_SETUP_OPERATIONS)
            .enumerate()
        {
            // Every operation of an indexed setup runs at each A angle, but a
            // surface faces up in one orientation only: a Face may name the
            // sides that come up at the angles, each faced where it is up.
            let sides =
                proposed.a_angles.as_ref().is_some_and(|a| a.len() > 1) && !op.surfaces.is_empty();
            // (Without the chuck's orientations the setup has said why.)
            if sides && let Some(at) = &at_angles {
                if let Err(message) = faces_each_angle(op, at) {
                    wrong(Some(oi + 1), message);
                    continue;
                }
            } else if sides {
                continue;
            }
            // The current operation this one keeps, and what it was.
            let kept = match op.from {
                None => None,
                Some(n) => match numbered.get(n.wrapping_sub(1)) {
                    None => {
                        wrong(
                            Some(oi + 1),
                            format!("from {n} is not an operation of the current plan."),
                        );
                        continue;
                    }
                    Some(&(bs, _)) if Some(bs) != base => {
                        wrong(
                            Some(oi + 1),
                            format!(
                                "from {n} is an operation of setup {} of the current plan, which this setup does not keep; an operation keeps its from only in its own setup. Set from to null to add it here as a new operation.",
                                bs + 1
                            ),
                        );
                        continue;
                    }
                    Some(_) if kept_ops.contains(&n) => {
                        wrong(
                            Some(oi + 1),
                            format!("from {n} is used by an earlier operation."),
                        );
                        continue;
                    }
                    Some(&(bs, bo)) => {
                        let current = &doc.setups[bs].operations[bo];
                        let Some(d) = described(current, &doc.tools) else {
                            wrong(
                                Some(oi + 1),
                                format!(
                                    "from {n} is one of the kept_operations, which cannot be changed and stays as it is: leave it out."
                                ),
                            );
                            continue;
                        };
                        kept_ops.push(n);
                        Some((current, d))
                    }
                },
            };
            let fresh = || {
                operation(
                    op,
                    si,
                    orientation,
                    sides,
                    report,
                    &doc.tools,
                    &mesh.fingerprint,
                )
            };
            let built = match kept {
                Some((current, d)) if unchanged(op, &d, get(&current.p, "name")) => {
                    Ok(current.clone())
                }
                Some((current, d))
                    if op.strategy == d.strategy && op.contour_mode == d.contour_mode =>
                {
                    fresh().map(|fresh| merged(current, &fresh, op, &d))
                }
                _ => fresh(),
            };
            match built {
                Ok(op) => setup.operations.push(op),
                Err(messages) => messages.into_iter().for_each(|m| wrong(Some(oi + 1), m)),
            }
        }
        // The operations a proposal cannot carry stay at their place.
        if let Some(b) = base {
            for (oi, op) in doc.setups[b].operations.iter().enumerate() {
                if described(op, &doc.tools).is_none() {
                    let at = oi.min(setup.operations.len());
                    setup.operations.insert(at, op.clone());
                }
            }
        }
        setups.push(setup);
    }
    if editing {
        for (b, setup) in doc.setups.iter().enumerate() {
            let locked: Vec<String> = setup
                .operations
                .iter()
                .filter(|op| described(op, &doc.tools).is_none())
                .map(|op| format!("\"{}\"", clip(get(&op.p, "name"), 60)))
                .collect();
            if !locked.is_empty() && !kept_setups.contains(&b) {
                let up = if b == 0 {
                    first
                } else {
                    analysis::up_axis(setup)
                };
                problems.push(Problem {
                    setup: 0,
                    operation: None,
                    stage: "structure",
                    message: format!(
                        "Setup {} of the current plan has kept_operations ({}), which cannot be removed: keep that setup with from {} and orientation {}.",
                        b + 1,
                        locked.join(", "),
                        b + 1,
                        up.unwrap_or("?")
                    ),
                });
            }
        }
    }
    if !problems.is_empty() {
        return Err(problems);
    }
    let mut built = doc.clone();
    // A change keeps the setup the user was on when it is still there.
    built.active = if editing && setups.iter().any(|s| s.id == doc.active) {
        doc.active.clone()
    } else {
        setups[0].id.clone()
    };
    built.setups = setups;
    if proposal.chuck.is_some() {
        built.settle_holding(0);
    }
    // A later setup in a V-block, soft jaws or a collet block takes the
    // entry's grip as the setup page fills it in.
    for si in 1..built.setups.len() {
        if crate::round_hold::Kind::of(get(&built.setups[si].p, "holding")).is_some() {
            built.settle_holding(si);
        }
    }
    built.sync_stock_sources();
    if !rotary {
        return Ok(built);
    }
    // The checks the chuck and the A angles have of their own, as the setups
    // page shows them, before any operation is generated.
    built.sync_turns(Some(mesh));
    let mut refused = vec![];
    if proposal.chuck.is_some() {
        for message in built.holding_issues(0, Some(mesh)) {
            refused.push((1, "workholding", format!("The chuck: {message}")));
        }
    }
    for si in 1..built.setups.len() {
        for message in built.index_issues(si) {
            refused.push((si + 1, "structure", message));
        }
    }
    if refused.is_empty() {
        return Ok(built);
    }
    Err(refused
        .into_iter()
        .map(|(setup, stage, message)| Problem {
            setup,
            operation: None,
            stage,
            message,
        })
        .collect())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::post::fixture;
    /// A proposed operation with speeds inside Aluminium 6061-T6's cutting
    /// data for the fixture's 3-flute tools: Ø10 and Ø6 at 8000 rpm and
    /// 1000 mm/min (151–251 m/min, 0.042 mm per tooth); the Ø4 drill at the
    /// machine's 12000 rpm and 1200 mm/min (151 m/min, 0.033 mm per tooth).
    pub fn op(strategy: &str, tool: u32, surfaces: Vec<usize>) -> ProposedOp {
        ProposedOp {
            strategy: strategy.into(),
            name: format!("{strategy} by planner"),
            tool,
            surfaces,
            rpm: if strategy == "Drill" { 12000. } else { 8000. },
            feed: if strategy == "Drill" { 1200. } else { 1000. },
            stepdown: None,
            stepover_percent: None,
            radial_allowance: None,
            axial_allowance: None,
            bottom_offset: None,
            clearance: None,
            finish_passes: None,
            facing_area: None,
            contour_mode: None,
            pocket_entry: None,
            ramp_angle: None,
            surface_stepover: None,
            tolerance: None,
            drill_cycle: None,
            peck: None,
            breakthrough: None,
            hole_diameter: None,
            through: None,
            tabs: None,
            coolant: "Flood".into(),
            reason: "test".into(),
            from: None,
        }
    }
    fn ids(report: &PartReport, up: &str, depth: f32) -> Vec<usize> {
        report
            .orientations
            .iter()
            .find(|o| o.up == up)
            .unwrap()
            .planes
            .iter()
            .find(|p| (p.depth_below_top - depth).abs() < 0.01)
            .unwrap()
            .surfaces
            .clone()
    }
    /// `doc` with the library's starter "Aluminium 6061-T6" selected, as
    /// planning needs a material with cutting data.
    pub fn aluminium(doc: &mut Document) {
        doc.adopt_materials();
        let starter = doc
            .materials
            .iter()
            .find(|m| m.name() == "Aluminium 6061-T6")
            .unwrap()
            .id
            .clone();
        assert!(doc.select_material(&starter));
    }
    /// Setup 1 held by the starter vise or clamp (`kind` "Vise" or "Clamp"), with
    /// `pairs` set on the setup.
    pub fn hold(doc: &mut Document, kind: &str, pairs: &[(&str, &str)]) {
        doc.adopt_holders();
        let id = doc
            .workholding
            .iter()
            .find(|h| h.kind() == kind)
            .unwrap()
            .id
            .clone();
        let p = &mut doc.setups[0].p;
        p.insert("holder".into(), id);
        p.insert(
            "holding".into(),
            if kind == "Vise" { "Vise" } else { "Clamps" }.into(),
        );
        for (k, v) in pairs {
            p.insert((*k).into(), (*v).into());
        }
    }
    /// The fixture job with its part report: tools T1 flat Ø10, T2 ball Ø6, T3 drill Ø4.
    pub fn block() -> (Document, Mesh, PartReport) {
        let (mut doc, mesh) = fixture::job();
        aluminium(&mut doc);
        let report = analysis::report(&doc, &mesh).unwrap();
        (doc, mesh, report)
    }
    /// A two-setup plan for the example block that passes every check.
    pub fn good(report: &PartReport) -> Proposal {
        let mut drill = op("Drill", 3, ids(report, "+Z", 0.));
        drill.drill_cycle = Some("Deep drilling".into());
        drill.peck = Some(3.);
        Proposal {
            summary: "Top side, then the underside.".into(),
            chuck: None,
            setups: vec![
                ProposedSetup {
                    orientation: "+Z".into(),
                    name: "Top".into(),
                    origin: "Stock top center".into(),
                    workholding: "Vise on the long sides.".into(),
                    holder: None,
                    a_angles: None,
                    operations: vec![
                        op("Face", 1, ids(report, "+Z", 0.)),
                        op("Pocket", 1, ids(report, "+Z", 12.)),
                        op("Contour", 1, vec![]),
                        drill,
                    ],
                    from: None,
                },
                ProposedSetup {
                    orientation: "-Z".into(),
                    name: "Underside".into(),
                    origin: "Model origin".into(),
                    workholding: "Flip about X; clamp on the machined walls.".into(),
                    holder: None,
                    a_angles: None,
                    operations: vec![
                        op("Face", 1, ids(report, "-Z", 0.)),
                        op("Pocket", 1, ids(report, "-Z", 6.)),
                    ],
                    from: None,
                },
            ],
            unmachined: vec![Unmachined {
                feature: "Counterbores".into(),
                reason: "No tool fits the 2.2 mm ring.".into(),
            }],
            warnings: vec![],
        }
    }
    /// `good` with setup 2 turned so that `up` faces up, after setup 1's
    /// Contour has removed the stock's side margins, and one Face on that side.
    pub fn side_up(report: &PartReport, up: &str) -> Proposal {
        let mut proposal = good(report);
        proposal.setups[1].orientation = up.into();
        proposal.setups[1].operations = vec![op("Face", 1, ids(report, up, 0.))];
        proposal
    }
    fn messages(result: Result<Document, Vec<Problem>>) -> Vec<String> {
        result
            .unwrap_err()
            .into_iter()
            .map(|p| format!("{}/{:?}: {}", p.setup, p.operation, p.message))
            .collect()
    }
    #[test]
    fn the_schema_is_strict_at_every_level_and_matches_the_types() {
        fn walk(value: &Value, objects: &mut usize) {
            if value["type"] == "object" {
                *objects += 1;
                assert_eq!(value["additionalProperties"], false);
                let keys: Vec<&String> = value["properties"].as_object().unwrap().keys().collect();
                let required: Vec<&str> = value["required"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_str().unwrap())
                    .collect();
                assert_eq!(keys.len(), required.len());
                assert!(keys.iter().all(|k| required.contains(&k.as_str())));
            }
            match value {
                Value::Object(map) => map.values().for_each(|v| walk(v, objects)),
                Value::Array(items) => items.iter().for_each(|v| walk(v, objects)),
                _ => {}
            }
        }
        // The plain schema and the one for changing a plan (with `from`).
        for edit in [false, true] {
            let schema = schema_for(edit);
            let mut objects = 0;
            walk(&schema, &mut objects);
            // The proposal, its setups, operations, tabs and unmachined items.
            assert_eq!(objects, 5);
            // Every field of the Rust types is in the schema, and nothing else.
            let (_, _, report) = block();
            let mut plan = good(&report);
            if edit {
                for (n, setup) in plan.setups.iter_mut().enumerate() {
                    setup.from = Some(n + 1);
                    setup.operations.iter_mut().for_each(|o| o.from = Some(1));
                }
            }
            let value = serde_json::to_value(plan).unwrap();
            let fields = |v: &Value| {
                let mut keys = v.as_object().unwrap().keys().cloned().collect::<Vec<_>>();
                keys.sort();
                keys
            };
            let props = |v: &Value| {
                let mut keys = v["properties"]
                    .as_object()
                    .unwrap()
                    .keys()
                    .cloned()
                    .collect::<Vec<_>>();
                keys.sort();
                keys
            };
            assert_eq!(fields(&value), props(&schema));
            let setup = &schema["properties"]["setups"]["items"];
            assert_eq!(fields(&value["setups"][0]), props(setup), "edit {edit}");
            assert_eq!(
                fields(&value["setups"][0]["operations"][0]),
                props(&setup["properties"]["operations"]["items"]),
                "edit {edit}"
            );
            assert_eq!(
                setup["properties"]["orientation"]["enum"],
                json!(["+Z", "-Z", "+X", "-X", "+Y", "-Y"])
            );
        }
    }
    #[test]
    fn a_plan_may_list_surfaces_of_coplanar_faces_a_few_microns_apart() {
        let (doc, _, _) = block();
        for step in 0..20 {
            let mesh = analysis::tests::blocks_with_tops_a_hair_apart(step);
            let report = analysis::report(&doc, &mesh).unwrap();
            let top = &report.orientations[0];
            let ids: Vec<usize> = top.planes.iter().flat_map(|p| p.surfaces.clone()).collect();
            assert_eq!(ids.len(), 2);
            let plane = plane_of(top, &ids)
                .unwrap_or_else(|| panic!("step {step}: the tops are in different planes"));
            assert_eq!(plane.kind, "top");
        }
    }
    #[test]
    fn parsing_accepts_a_valid_proposal_and_rejects_extras_and_missing_fields() {
        let (_, _, report) = block();
        let proposal = good(&report);
        let text = serde_json::to_string(&proposal).unwrap();
        assert_eq!(parse(&format!("  {text}\n")).unwrap(), proposal);
        let mut extra = serde_json::to_value(&proposal).unwrap();
        extra["setups"][0]["operations"][0]["gcode"] = json!("G0 Z-50");
        assert!(parse(&extra.to_string()).unwrap_err().contains("gcode"));
        let mut missing = serde_json::to_value(&proposal).unwrap();
        missing["setups"][0]["operations"][0]
            .as_object_mut()
            .unwrap()
            .remove("tool");
        assert!(parse(&missing.to_string()).is_err());
        assert!(parse("Here is the plan: {}").is_err());
    }
    #[test]
    fn building_keeps_setup_one_and_creates_later_setups() {
        let (mut doc, mesh, report) = block();
        doc.setups[0]
            .p
            .insert("origin".into(), "Stock top front-left".into());
        doc.setups[0].p.insert("fixture".into(), "Vise".into());
        let built = build(&doc, &mesh, &report, &good(&report)).unwrap();
        assert_eq!(built.setups.len(), 2);
        let (first, second) = (&built.setups[0], &built.setups[1]);
        assert_eq!(first.id, doc.setups[0].id);
        assert_eq!(get(&first.p, "origin"), "Stock top front-left");
        assert_eq!(get(&first.p, "fixture"), "Vise");
        assert_eq!(
            get(&first.p, "stock_top"),
            get(&doc.setups[0].p, "stock_top")
        );
        assert_eq!(get(&first.p, "name"), "Top");
        assert_eq!(first.operations.len(), 4);
        assert_eq!(get(&second.p, "offset"), "G55");
        assert_eq!(
            (
                get(&second.p, "rx"),
                get(&second.p, "ry"),
                get(&second.p, "rz")
            ),
            ("180", "0", "0")
        );
        assert_eq!(get(&second.p, "origin"), "Model origin");
        assert_eq!(
            get(&second.p, "notes"),
            "Flip about X; clamp on the machined walls."
        );
        assert_eq!(get(&second.p, "stock_source"), first.id);
        assert_eq!(second.machine.id, first.machine.id);
        assert_eq!(built.active, first.id);
        assert!(built.validate_structure().is_ok());
    }
    #[test]
    fn later_setups_copy_setup_1s_workholding_without_a_placement() {
        let (mut doc, mesh, report) = block();
        // Setup 1 without workholding: the later setup carries none either.
        let built = build(&doc, &mesh, &report, &good(&report)).unwrap();
        for key in ["holding", "holder", "holding_gap"] {
            assert!(!built.setups[1].p.contains_key(key), "{key}");
        }
        hold(
            &mut doc,
            "Clamp",
            &[("clamps", "-X:0;+X:10"), ("holding_gap", "3.5")],
        );
        let built = build(&doc, &mesh, &report, &good(&report)).unwrap();
        let (first, second) = (&built.setups[0], &built.setups[1]);
        assert_eq!(get(&first.p, "clamps"), "-X:0;+X:10");
        for key in ["holding", "holder", "holding_gap"] {
            assert!(!get(&first.p, key).is_empty());
            assert_eq!(get(&second.p, key), get(&first.p, key), "{key}");
        }
        assert_eq!(get(&second.p, "holding"), "Clamps");
        assert_eq!(get(&second.p, "holding_gap"), "3.5");
        // No position: the app lays it out once the toolpaths are known.
        for key in ["clamps", "vise_axis", "vise_offset", "vise_grip"] {
            assert!(!second.p.contains_key(key), "{key}");
        }
        hold(&mut doc, "Vise", &[("vise_axis", "Y"), ("vise_grip", "8")]);
        let built = build(&doc, &mesh, &report, &good(&report)).unwrap();
        assert_eq!(get(&built.setups[1].p, "holding"), "Vise");
        assert!(!built.setups[1].p.contains_key("vise_grip"));
    }
    /// The block held in setup 1 by the starter vise (library entry 1; the
    /// clamp is entry 2), the report that lists them, and the two-setup plan.
    fn with_library(pairs: &[(&str, &str)]) -> (Document, Mesh, PartReport, Proposal) {
        let (mut doc, mesh, _) = block();
        hold(&mut doc, "Vise", pairs);
        let report = analysis::report(&doc, &mesh).unwrap();
        let proposal = good(&report);
        (doc, mesh, report, proposal)
    }
    #[test]
    fn the_schema_asks_every_setup_for_its_holder_as_an_integer_or_null() {
        let schema = schema();
        let setup = &schema["properties"]["setups"]["items"];
        assert_eq!(
            setup["properties"]["holder"],
            json!({"type": ["integer", "null"]})
        );
        assert!(
            setup["required"]
                .as_array()
                .unwrap()
                .contains(&json!("holder"))
        );
        // A reply with a number reads back as one; a missing key reads as null.
        let (_, _, report) = block();
        let mut value = serde_json::to_value(good(&report)).unwrap();
        assert_eq!(value["setups"][1]["holder"], Value::Null);
        value["setups"][1]["holder"] = json!(2);
        let parsed = parse(&value.to_string()).unwrap();
        assert_eq!(parsed.setups[1].holder, Some(2));
        assert_eq!(parsed.setups[0].holder, None);
        value["setups"][1]["holder"] = json!(-1);
        assert!(parse(&value.to_string()).is_err());
    }
    #[test]
    fn a_later_setup_builds_with_the_library_entry_it_names() {
        let (doc, mesh, report, mut proposal) = with_library(&[
            ("vise_axis", "Y"),
            ("vise_grip", "8"),
            ("holding_gap", "3.5"),
        ]);
        let (vise, clamp) = (doc.workholding[0].id.clone(), doc.workholding[1].id.clone());
        assert_eq!(get(&doc.setups[0].p, "holder"), vise);
        // Entry 2 is a clamp: setup 2 is held by Clamps, with that entry,
        // the user's clearance gap and no position yet.
        proposal.setups[1].holder = Some(2);
        let built = build(&doc, &mesh, &report, &proposal).unwrap();
        let (first, second) = (&built.setups[0], &built.setups[1]);
        assert_eq!(
            (get(&first.p, "holding"), get(&first.p, "holder")),
            ("Vise", vise.as_str())
        );
        assert_eq!(
            (get(&second.p, "holding"), get(&second.p, "holder")),
            ("Clamps", clamp.as_str())
        );
        assert_eq!(get(&second.p, "holding_gap"), "3.5");
        for key in ["clamps", "vise_axis", "vise_offset", "vise_grip"] {
            assert!(!second.p.contains_key(key), "{key}");
        }
        assert!(!crate::holding::placed(second));
        // Null keeps setup 1's, as before; naming setup 1's own entry is the same.
        for holder in [None, Some(1)] {
            proposal.setups[1].holder = holder;
            let built = build(&doc, &mesh, &report, &proposal).unwrap();
            let second = &built.setups[1];
            assert_eq!(
                (get(&second.p, "holding"), get(&second.p, "holder")),
                ("Vise", vise.as_str()),
                "{holder:?}"
            );
            assert!(!second.p.contains_key("vise_axis"));
        }
    }
    #[test]
    fn a_later_setup_can_be_held_when_setup_1_is_not_and_can_switch_kind() {
        // Setup 1 without workholding; the planner holds setup 2 with a vise.
        let (mut doc, mesh, _) = block();
        doc.adopt_holders();
        let report = analysis::report(&doc, &mesh).unwrap();
        let mut proposal = good(&report);
        proposal.setups[1].holder = Some(1);
        let built = build(&doc, &mesh, &report, &proposal).unwrap();
        assert!(!built.setups[0].p.contains_key("holding"));
        assert_eq!(get(&built.setups[1].p, "holding"), "Vise");
        assert_eq!(get(&built.setups[1].p, "holder"), doc.workholding[0].id);
        assert!(!built.setups[1].p.contains_key("holding_gap"));
        // Setup 1 clamped, setup 2 in a vise.
        let (mut doc, mesh, _) = block();
        crate::proposal::tests::hold(&mut doc, "Clamp", &[("clamps", "-X:0;+X:10")]);
        let report = analysis::report(&doc, &mesh).unwrap();
        let mut proposal = good(&report);
        proposal.setups[1].holder = Some(1);
        let built = build(&doc, &mesh, &report, &proposal).unwrap();
        assert_eq!(get(&built.setups[0].p, "holding"), "Clamps");
        assert_eq!(get(&built.setups[1].p, "holding"), "Vise");
        assert_eq!(get(&built.setups[1].p, "holder"), doc.workholding[0].id);
    }
    #[test]
    fn a_holder_that_is_not_in_the_library_or_not_setup_1s_is_refused() {
        let (doc, mesh, report, mut proposal) =
            with_library(&[("vise_axis", "Y"), ("vise_grip", "8")]);
        let problems = |proposal: &Proposal| {
            build(&doc, &mesh, &report, proposal)
                .unwrap_err()
                .into_iter()
                .map(|p| (p.setup, p.operation, p.stage, p.line()))
                .collect::<Vec<_>>()
        };
        // Past the end of the library, and 0: the library counts from 1.
        for number in [9, 0, 3] {
            proposal.setups[1].holder = Some(number);
            assert_eq!(
                problems(&proposal),
                [(
                    2,
                    None,
                    "structure",
                    format!("Setup 2: workholding {number} is not in workholding_library.")
                )]
            );
        }
        proposal.setups[1].holder = None;
        // Setup 1 is the user's: null or its own entry, nothing else.
        for ok in [None, Some(1)] {
            proposal.setups[0].holder = ok;
            assert!(build(&doc, &mesh, &report, &proposal).is_ok(), "{ok:?}");
        }
        proposal.setups[0].holder = Some(2);
        assert_eq!(
            problems(&proposal),
            [(
                1,
                None,
                "structure",
                "Setup 1: holder must be null or 1: setup 1's workholding is the user's.".into()
            )]
        );
        proposal.setups[0].holder = Some(9);
        assert_eq!(
            problems(&proposal),
            [(
                1,
                None,
                "structure",
                "Setup 1: workholding 9 is not in workholding_library.".into()
            )]
        );
        // Setup 1 with no workholding of its own has no entry to name.
        let (mut doc, mesh, _) = block();
        doc.adopt_holders();
        let report = analysis::report(&doc, &mesh).unwrap();
        let mut proposal = good(&report);
        proposal.setups[0].holder = Some(1);
        let found: Vec<_> = build(&doc, &mesh, &report, &proposal)
            .unwrap_err()
            .iter()
            .map(Problem::line)
            .collect();
        assert_eq!(
            found,
            ["Setup 1: holder must be null: setup 1's workholding is the user's."]
        );
    }
    #[test]
    fn clearance_is_capped_by_the_machines_z_travel_or_300_mm() {
        let (doc, mesh, mut report) = block();
        let mut proposal = good(&report);
        let ask = |proposal: &mut Proposal, report: &PartReport, clearance: f32| {
            proposal.setups[0].operations[0].clearance = Some(clearance);
            build(&doc, &mesh, report, proposal).map_or_else(|p| messages(Err(p)), |_| vec![])
        };
        // Z travel unknown: 300 mm is accepted, a hair above it is not.
        report.machine.travel[2] = 0.;
        assert_eq!(clearance_cap(&report), 300.);
        assert!(ask(&mut proposal, &report, 300.).is_empty());
        assert_eq!(
            ask(&mut proposal, &report, 500_000.),
            ["1/Some(1): clearance 500000 is above the largest this machine allows, 300 mm."]
        );
        // Known: the travel is the cap, even above 300.
        report.machine.travel[2] = 450.;
        assert_eq!(clearance_cap(&report), 450.);
        assert!(ask(&mut proposal, &report, 450.).is_empty());
        assert_eq!(
            ask(&mut proposal, &report, 450.5),
            ["1/Some(1): clearance 450.5 is above the largest this machine allows, 450 mm."]
        );
        // A travel under the default still leaves the default usable.
        report.machine.travel[2] = 10.;
        assert_eq!(clearance_cap(&report), CLEARANCE);
    }
    #[test]
    fn clearance_is_the_proposals_to_raise_and_never_below_the_default() {
        let (doc, mesh, report) = block();
        // The default the proposal's null stands for is the app's own.
        let fresh = Operation::new(Strategy::Face, String::new());
        assert_eq!(num(&fresh.p, "clearance"), Some(CLEARANCE));
        let mut proposal = good(&report);
        proposal.setups[0].operations[1].clearance = Some(30.);
        let built = build(&doc, &mesh, &report, &proposal).unwrap();
        let clearance = |si: usize, oi: usize| get(&built.setups[si].operations[oi].p, "clearance");
        assert_eq!(
            (clearance(0, 1), clearance(0, 0), clearance(1, 0)),
            ("30", "15", "15")
        );
        // The default itself is accepted; anything under it is not, down to
        // the retract height and below.
        proposal.setups[0].operations[0].clearance = Some(15.);
        assert!(build(&doc, &mesh, &report, &proposal).is_ok());
        for low in [14.9, 5., 0.] {
            proposal.setups[0].operations[0].clearance = Some(low);
            assert_eq!(
                messages(build(&doc, &mesh, &report, &proposal)),
                ["1/Some(1): clearance can only raise the default of 15 mm."],
                "{low}"
            );
        }
        proposal.setups[0].operations[0].clearance = Some(14.9);
        proposal.setups[1].operations[0].clearance = Some(f32::NAN);
        assert_eq!(
            messages(build(&doc, &mesh, &report, &proposal)),
            [
                "1/Some(1): clearance can only raise the default of 15 mm.",
                "2/Some(1): clearance is not a finite number."
            ]
        );
    }
    #[test]
    fn a_proposal_without_the_clearance_key_still_parses_with_none() {
        let (_, _, report) = block();
        let text = serde_json::to_string(&good(&report)).unwrap();
        assert!(text.contains("\"clearance\":null"), "{text}");
        // What an earlier version of the schema produced: no such key.
        let old = text.replace("\"clearance\":null,", "");
        assert!(!old.contains("clearance"), "{old}");
        let parsed = parse(&old).unwrap();
        assert!(
            parsed
                .setups
                .iter()
                .flat_map(|s| &s.operations)
                .all(|o| o.clearance.is_none())
        );
        assert_eq!(parsed, good(&report));
        // A key that is not in the schema is still refused.
        assert!(parse(&text.replace("\"clearance\":null", "\"clearence\":null")).is_err());
    }
    #[test]
    fn a_facing_of_the_models_outline_clears_past_it_far_enough_for_its_cutter() {
        // A Ø25 face mill reaches the block's square corners 2.9 mm past
        // them: the plan sets 4 mm, and the Face generates.
        let (mut doc, mesh, report) = block();
        doc.tools[0].p.insert("diameter".into(), "25".into());
        let mut proposal = good(&report);
        proposal.setups.truncate(1);
        proposal.setups[0].operations.truncate(1);
        proposal.setups[0].operations[0].facing_area = Some("model".into());
        let built = build(&doc, &mesh, &report, &proposal).unwrap();
        let setup = &built.setups[0];
        let face = &setup.operations[0];
        assert!(face.faces_outline());
        assert_eq!(get(&face.p, "silhouette_extend"), "4.0");
        let tool = built.tools.iter().find(|t| t.id == face.tool).unwrap();
        crate::simulation::generate(setup, face, tool, &mesh).unwrap();
    }
    #[test]
    fn a_change_of_cutter_resizes_how_far_past_the_outline_a_facing_clears() {
        // Applied on the Ø10 T1, then changed to a Ø25 T4.
        let (mut doc, mesh, report) = block();
        let mut big = Tool::new("Face mill", "Flat end mill", "25");
        big.p.insert("number".into(), "4".into());
        doc.tools.push(big);
        let mut plan = good(&report);
        plan.setups[0].operations[0].facing_area = Some("model".into());
        let doc = build(&doc, &mesh, &report, &plan).unwrap();
        assert_eq!(
            get(&doc.setups[0].operations[0].p, "silhouette_extend"),
            "2.0"
        );
        let mut change = shown(&doc, &report);
        change.setups[0].operations[0].tool = 4;
        let rebuilt = build(&doc, &mesh, &report, &change).unwrap();
        let (setup, face) = (&rebuilt.setups[0], &rebuilt.setups[0].operations[0]);
        assert_eq!(face.id, doc.setups[0].operations[0].id);
        assert_eq!(get(&face.p, "silhouette_extend"), "4.0");
        let tool = rebuilt.tools.iter().find(|t| t.id == face.tool).unwrap();
        assert_eq!(get(&tool.p, "diameter"), "25");
        crate::simulation::generate(setup, face, tool, &mesh).unwrap();
    }
    #[test]
    fn operations_get_defaults_overrides_and_fixed_combinations() {
        let (doc, mesh, report) = block();
        let mut proposal = good(&report);
        {
            let ops = &mut proposal.setups[0].operations;
            ops[0].facing_area = Some("model".into());
            ops[0].radial_allowance = Some(5.);
            ops[1].radial_allowance = Some(0.3);
            ops[1].stepover_percent = Some(35.);
            ops[1].pocket_entry = Some("Plunge".into());
            ops[1].finish_passes = Some(2);
            ops[1].name = "  A very long name ".repeat(10);
            ops[3].through = Some(true);
            ops[3].breakthrough = Some(1.5);
            ops[3].hole_diameter = Some(6.6);
            let mut walls = op("Contour", 1, ids(&report, "+Z", 12.));
            walls.contour_mode = Some("inside_floor".into());
            ops.push(walls);
            ops.push(op("Parallel", 2, vec![]));
            ops.push(op("Waterline", 1, vec![]));
        }
        let built = build(&doc, &mesh, &report, &proposal).unwrap();
        let ops = &built.setups[0].operations;
        let p = |i: usize, key: &str| get(&ops[i].p, key).to_string();
        assert_eq!(
            (p(0, "boundary"), p(0, "containment"), p(0, "allowance")),
            (
                "Model silhouette".into(),
                "Tool outside boundary".into(),
                "0".into()
            )
        );
        assert_eq!(
            get(&built.setups[1].operations[0].p, "boundary"),
            "Stock silhouette"
        );
        assert_eq!(
            (p(1, "boundary"), p(1, "containment")),
            ("Selected geometry".into(), "Tool inside boundary".into())
        );
        assert_eq!(
            (
                p(1, "allowance"),
                p(1, "axial_allowance"),
                p(1, "stepover"),
                p(1, "pocket_entry"),
                p(1, "finish_passes")
            ),
            (
                "0.3".into(),
                "0".into(),
                "35".into(),
                "Plunge".into(),
                "2".into()
            )
        );
        assert_eq!(p(1, "name").chars().count(), 60);
        assert_eq!(p(1, "stepdown"), "2", "unset values keep the app default");
        assert_eq!(
            (p(2, "boundary"), p(2, "side"), p(2, "allowance")),
            ("Model silhouette".into(), "Outside".into(), "0".into())
        );
        assert_eq!(
            (
                p(3, "cycle"),
                p(3, "peck"),
                p(3, "through"),
                p(3, "breakthrough"),
                p(3, "hole_diameter")
            ),
            (
                "Deep drilling".into(),
                "3".into(),
                "Yes".into(),
                "1.5".into(),
                "6.6".into()
            )
        );
        assert_eq!(
            (p(4, "boundary"), p(4, "side"), p(4, "containment")),
            (
                "Selected geometry".into(),
                "Inside".into(),
                "Tool inside boundary".into()
            )
        );
        assert_eq!(
            (p(5, "boundary"), p(6, "boundary")),
            ("Model silhouette".into(), "Model silhouette".into())
        );
        assert!(ops[2].surfaces.is_none() && ops[5].surfaces.is_none());
        assert_eq!(
            ops[1].surfaces.as_ref().unwrap().mesh_fingerprint,
            mesh.fingerprint
        );
        assert_eq!(
            (p(0, "rpm"), p(0, "feed"), p(0, "coolant")),
            ("8000".into(), "1000".into(), "Flood".into())
        );
        assert_eq!(ops[3].tool, doc.tools[2].id);
        assert!(ops.iter().all(|o| o.enabled
            && o.geometry.is_empty()
            && get(&o.p, "spindle_rotation") == "Clockwise (CW)"));
    }
    #[test]
    fn every_reference_problem_is_reported_with_its_place() {
        let (doc, mesh, report) = block();
        let refused = |edit: &dyn Fn(&mut Proposal), needle: &str| {
            let mut proposal = good(&report);
            edit(&mut proposal);
            let found = messages(build(&doc, &mesh, &report, &proposal));
            assert!(
                found.iter().any(|m| m.contains(needle)),
                "{needle}: {found:?}"
            );
        };
        refused(&|p| p.setups.clear(), "no setups");
        refused(
            &|p| p.setups.swap(0, 1),
            "0/None: The first setup must use orientation +Z",
        );
        refused(
            &|p| p.setups[1].orientation = "+Z".into(),
            "2/None: Orientation +Z is used",
        );
        refused(
            &|p| p.setups[1].orientation = "up".into(),
            "2/None: \"up\" is not an orientation",
        );
        refused(
            &|p| p.setups[1].origin = "Selected point (planned)".into(),
            "not a work zero",
        );
        refused(
            &|p| p.setups[1].operations.clear(),
            "2/None: The setup has no operations",
        );
        refused(
            &|p| p.setups[0].operations[1].strategy = "Adaptive".into(),
            "1/Some(2): \"Adaptive\" is not one of the six",
        );
        refused(
            &|p| p.setups[0].operations[0].tool = 9,
            "1/Some(1): T9 is not in the tool library",
        );
        refused(
            &|p| p.setups[0].operations[0].tool = 3,
            "T3 is a Drill, which Face cannot use",
        );
        refused(
            &|p| p.setups[0].operations[3].tool = 1,
            "T1 is a Flat end mill, which Drill cannot use",
        );
        refused(
            &|p| p.setups[0].operations[1].tool = 2,
            "T2 is a Ball end mill, which Pocket cannot use",
        );
        refused(
            &|p| {
                p.setups[0].operations[3].drill_cycle = Some(crate::model::HELICAL_MILLING.into());
            },
            "T3 is a Drill; Drill with Helical milling takes a flat end mill",
        );
        refused(
            &|p| p.setups[0].operations[0].rpm = 20000.,
            "rpm 20000 is outside 100-12000",
        );
        refused(
            &|p| p.setups[0].operations[0].feed = 0.,
            "feed 0 is outside",
        );
        refused(
            &|p| p.setups[0].operations[0].coolant = "Oil".into(),
            "not a coolant",
        );
        refused(
            &|p| p.setups[0].operations[1].pocket_entry = Some("Helix".into()),
            "not a pocket entry",
        );
        refused(
            &|p| p.setups[0].operations[0].surfaces = vec![],
            "Facing needs surfaces of the top plane",
        );
        refused(
            &|p| p.setups[0].operations[0].surfaces = p.setups[0].operations[1].surfaces.clone(),
            "Facing needs surfaces of the top plane",
        );
        refused(
            &|p| p.setups[0].operations[1].surfaces = p.setups[0].operations[0].surfaces.clone(),
            "Pocket needs the surfaces of one or more floor planes",
        );
        refused(
            &|p| p.setups[0].operations[1].surfaces = p.setups[1].operations[1].surfaces.clone(),
            "1/Some(2): Pocket needs the surfaces of one or more floor planes in orientation +Z",
        );
        // The Ø10 cutter fits the Ø11 counterbores, not the 8 mm deep floor.
        refused(
            &|p| p.setups[0].operations[1].surfaces = ids(&report, "+Z", 8.),
            "does not fit this floor; the largest cutter that fits is",
        );
        refused(
            &|p| p.setups[0].operations[3].surfaces = ids(&report, "+Z", 12.),
            "Drilling needs the surfaces of one plane with holes",
        );
        refused(
            &|p| p.setups[0].operations[2].surfaces = vec![1],
            "its surfaces list must be empty",
        );
        refused(
            &|p| p.setups[0].operations[1].surfaces.push(999_999),
            "Pocket needs the surfaces of one or more floor planes",
        );
        let mut many = good(&report);
        many.setups = vec![many.setups[0].clone(); 7];
        assert!(
            messages(build(&doc, &mesh, &report, &many))
                .iter()
                .any(|m| m.contains("at most 6"))
        );
        let mut none = doc.clone();
        none.setups.clear();
        assert!(build(&none, &mesh, &report, &good(&report)).is_err());
    }
    #[test]
    fn duplicate_surface_ids_are_merged_and_the_job_saves_and_reopens() {
        let (doc, mesh, report) = block();
        let mut proposal = good(&report);
        let face = &mut proposal.setups[0].operations[0];
        face.surfaces.reverse();
        face.surfaces.push(face.surfaces[0]);
        let built = build(&doc, &mesh, &report, &proposal).unwrap();
        let mut ids = ids(&report, "+Z", 0.);
        ids.sort();
        assert_eq!(
            built.setups[0].operations[0].surfaces.as_ref().unwrap().ids,
            ids
        );
        assert!(built.validate_structure().is_ok());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("applied.axis.json");
        crate::storage::save(&path, &built).unwrap();
        let reopened = crate::storage::load(&path).unwrap();
        assert_eq!(reopened.setups.len(), 2);
        assert_eq!(
            reopened.setups[0].operations[0]
                .surfaces
                .as_ref()
                .unwrap()
                .ids,
            ids
        );
    }
    #[test]
    fn surface_lists_the_selection_rules_reject_are_refused() {
        let (doc, mesh, mut report) = block();
        let mut proposal = good(&report);
        proposal.setups[0].operations[0].surfaces = (0..501).collect();
        assert!(
            messages(build(&doc, &mesh, &report, &proposal))
                .iter()
                .any(|m| m.starts_with("1/Some(1): ") && m.contains("501 IDs; at most 500")),
        );
        // An ID the saved project's selection rules reject, even if a report lists it.
        let mut proposal = good(&report);
        proposal.setups[0].operations[0].surfaces.push(250_000);
        report.orientations[0]
            .planes
            .iter_mut()
            .find(|p| p.kind == "top")
            .unwrap()
            .surfaces
            .push(250_000);
        assert_eq!(
            messages(build(&doc, &mesh, &report, &proposal)),
            vec!["1/Some(1): The surfaces list is not valid for this model."]
        );
    }
    #[test]
    fn through_drilling_needs_through_holes_and_a_bounded_breakthrough() {
        let (doc, mesh, report) = block();
        let drill = |edit: &dyn Fn(&mut ProposedOp)| {
            let mut proposal = good(&report);
            edit(&mut proposal.setups[0].operations[3]);
            build(&doc, &mesh, &report, &proposal)
        };
        // The top plane has a blind Ø6 × 8 hole among four through holes.
        assert_eq!(
            messages(drill(&|d| d.through = Some(true))),
            vec![
                "1/Some(4): through is true, but the selection includes a blind hole (diameter 6 mm, depth 8 mm). Set through to false, or use hole_diameter to select only through holes."
            ]
        );
        assert!(
            messages(drill(&|d| {
                d.through = Some(true);
                d.hole_diameter = Some(6.);
            }))[0]
                .contains("blind hole")
        );
        let built = drill(&|d| {
            d.through = Some(true);
            d.hole_diameter = Some(6.6);
        })
        .unwrap();
        assert_eq!(get(&built.setups[0].operations[3].p, "through"), "Yes");
        // T3 is Ø4: breakthrough up to 3 mm; null means the default of 1.
        for (breakthrough, ok) in [
            (None, true),
            (Some(3.), true),
            (Some(3.5), false),
            (Some(-0.5), false),
        ] {
            let result = drill(&|d| {
                d.through = Some(true);
                d.hole_diameter = Some(6.6);
                d.breakthrough = breakthrough;
            });
            assert_eq!(result.is_ok(), ok, "{breakthrough:?}");
            if !ok {
                assert_eq!(
                    messages(result),
                    vec![format!(
                        "1/Some(4): breakthrough {} mm is outside 0-3 mm for this drill (half its diameter plus 1 mm).",
                        breakthrough.unwrap()
                    )]
                );
            }
        }
        // Without through, breakthrough is not used.
        assert!(drill(&|d| d.breakthrough = Some(30.)).is_ok());
    }
    #[test]
    fn helical_milling_builds_a_drill_operation_with_a_flat_end_mill() {
        let (doc, mesh, report) = block();
        let mut proposal = good(&report);
        let drill = &mut proposal.setups[0].operations[3];
        (
            drill.tool,
            drill.peck,
            drill.stepdown,
            drill.stepover_percent,
        ) = (1, None, Some(1.5), Some(30.));
        drill.drill_cycle = Some(crate::model::HELICAL_MILLING.into());
        let job = build(&doc, &mesh, &report, &proposal).unwrap();
        let op = &job.setups[0].operations[3];
        assert!(op.helical());
        assert_eq!(
            get(
                &job.tools.iter().find(|t| t.id == op.tool).unwrap().p,
                "type"
            ),
            "Flat end mill"
        );
        assert_eq!(
            (get(&op.p, "stepdown"), get(&op.p, "stepover")),
            ("1.5", "30")
        );
    }
    #[test]
    fn speeds_and_feeds_have_floors() {
        let (doc, mesh, report) = block();
        let mut proposal = good(&report);
        proposal.setups[0].operations[0].rpm = 50.;
        proposal.setups[0].operations[1].feed = 0.5;
        assert_eq!(
            messages(build(&doc, &mesh, &report, &proposal)),
            vec![
                "1/Some(1): rpm 50 is outside 100-12000 for this machine.",
                "1/Some(2): feed 0.5 is outside 1-5000 mm/min for this machine."
            ]
        );
        proposal.setups[0].operations[0].rpm = 100.;
        proposal.setups[0].operations[1].feed = 1.;
        assert!(build(&doc, &mesh, &report, &proposal).is_ok());
    }
    #[test]
    fn operation_counts_are_limited_per_setup_and_in_total() {
        let (doc, mesh, report) = block();
        let mut proposal = good(&report);
        let face = proposal.setups[0].operations[0].clone();
        proposal.setups[0]
            .operations
            .resize(MAX_SETUP_OPERATIONS + 1, face.clone());
        let found = messages(build(&doc, &mesh, &report, &proposal));
        assert_eq!(
            found,
            vec!["1/None: The setup has 13 operations; at most 12 are allowed."]
        );
        let mut proposal = good(&report);
        for up in ["+X", "-X"] {
            let mut setup = proposal.setups[1].clone();
            setup.orientation = up.into();
            proposal.setups.push(setup);
        }
        for (setup, n) in proposal.setups.iter_mut().zip([12, 12, 12, 5]) {
            setup.operations.resize(n, face.clone());
        }
        let found = messages(build(&doc, &mesh, &report, &proposal));
        assert!(
            found.contains(
                &"0/None: The proposal has 41 operations; at most 40 are allowed.".to_string()
            ),
            "{found:?}"
        );
        assert!(found.iter().all(|m| !m.contains("operations; at most 12")));
    }
    #[test]
    fn names_and_notes_lose_control_and_format_characters() {
        assert_eq!(
            clip(
                "a\u{0}b\u{1b}[2J\u{202e}c \u{200b}d\u{feff}  e\u{e0041}",
                60
            ),
            "ab[2Jc d e"
        );
        assert_eq!(
            clip(
                "\u{ad}\u{600}\u{61c}\u{6dd}\u{70f}\u{180e}\u{2060}\u{2066}\u{206f}\u{fff9}\u{e0001}x",
                60
            ),
            "x"
        );
        let (doc, mesh, report) = block();
        let mut proposal = good(&report);
        proposal.setups[1].name = "Under)\u{0}\u{1b}[2J\u{202e}side (G0 Z-50".into();
        proposal.setups[1].workholding = "Vise\u{7}  on\u{2028}the walls\u{200f}.".into();
        proposal.setups[0].operations[0].name = "\u{202d}Face\u{9d} top".into();
        let built = build(&doc, &mesh, &report, &proposal).unwrap();
        assert_eq!(get(&built.setups[1].p, "name"), "Under)[2Jside (G0 Z-50");
        assert_eq!(get(&built.setups[1].p, "notes"), "Vise on the walls.");
        assert_eq!(get(&built.setups[0].operations[0].p, "name"), "Face top");
    }
    #[test]
    fn setup_one_keeps_the_users_name() {
        let (doc, mesh, report) = block();
        let mut proposal = good(&report);
        proposal.setups[0].name = "Planner top".into();
        proposal.setups[1].name = String::new();
        let built = build(&doc, &mesh, &report, &proposal).unwrap();
        assert_eq!(
            get(&built.setups[0].p, "name"),
            get(&doc.setups[0].p, "name")
        );
        assert_eq!(get(&built.setups[1].p, "name"), "Setup 2");
    }
    #[test]
    fn numbers_must_be_finite() {
        let (doc, mesh, report) = block();
        // JSON numbers beyond f32 parse to infinity.
        let mut value = serde_json::to_value(good(&report)).unwrap();
        value["setups"][0]["operations"][0]["peck"] = json!(1e39);
        value["setups"][0]["operations"][3]["hole_diameter"] = json!(-1e39);
        let proposal = parse(&value.to_string()).unwrap();
        assert_eq!(proposal.setups[0].operations[0].peck, Some(f32::INFINITY));
        assert_eq!(
            messages(build(&doc, &mesh, &report, &proposal)),
            vec![
                "1/Some(1): peck is not a finite number.",
                "1/Some(4): hole_diameter is not a finite number."
            ]
        );
        let mut proposal = good(&report);
        proposal.setups[0].operations[0].rpm = f32::NAN;
        proposal.setups[0].operations[1].feed = f32::INFINITY;
        proposal.setups[0].operations[1].stepdown = Some(f32::NEG_INFINITY);
        assert_eq!(
            messages(build(&doc, &mesh, &report, &proposal)),
            vec![
                "1/Some(1): rpm is not a finite number.",
                "1/Some(2): feed is not a finite number.",
                "1/Some(2): stepdown is not a finite number."
            ]
        );
    }
    #[test]
    fn allowances_and_the_bottom_offset_cannot_be_negative() {
        let (doc, mesh, report) = block();
        let mut proposal = good(&report);
        proposal.setups[0].operations[1].radial_allowance = Some(-0.1);
        proposal.setups[0].operations[1].axial_allowance = Some(-0.1);
        proposal.setups[0].operations[2].bottom_offset = Some(-3.9);
        assert_eq!(
            messages(build(&doc, &mesh, &report, &proposal)),
            vec![
                "1/Some(2): radial_allowance cannot be negative.",
                "1/Some(2): axial_allowance cannot be negative.",
                "1/Some(3): bottom_offset cannot be negative."
            ]
        );
        for op in &mut proposal.setups[0].operations[1..3] {
            op.radial_allowance = Some(0.);
            op.axial_allowance = Some(0.);
            op.bottom_offset = Some(0.);
        }
        assert!(build(&doc, &mesh, &report, &proposal).is_ok());
    }
    #[test]
    fn whitespace_in_model_text_becomes_a_space() {
        assert_eq!(clip("a\nb", 60), "a b");
        assert_eq!(clip("a\tb\r\nc\u{85}d\u{2028}e", 60), "a b c d e");
    }
    #[test]
    fn a_reply_cut_anywhere_reads_as_far_as_it_has_come() {
        let (_, _, report) = block();
        let full = serde_json::to_string(&good(&report)).unwrap();
        let summary = good(&report).summary;
        let summary_end = full.find("\",\"setups\"").unwrap() + 1;
        let rpm_at = full.find("\"rpm\":").unwrap();
        let rpm_end = rpm_at + full[rpm_at..].find(',').unwrap() + 1;
        // What each operation has shown so far, by setup and operation.
        let mut seen: std::collections::HashMap<(usize, usize), DraftOp> = Default::default();
        let (mut last_setups, mut last_ops) = (0, 0);
        for end in (0..=full.len()).filter(|i| full.is_char_boundary(*i)) {
            let draft = partial(&full[..end]);
            let ops: usize = draft.setups.iter().map(|s| s.operations.len()).sum();
            assert!(draft.setups.len() >= last_setups, "at {end}: setups left");
            assert!(ops >= last_ops, "at {end}: {ops} after {last_ops}");
            (last_setups, last_ops) = (draft.setups.len(), ops);
            if let Some(shown) = &draft.summary {
                assert!(summary.starts_with(shown), "at {end}: {shown:?}");
            }
            if end >= summary_end {
                assert_eq!(draft.summary.as_deref(), Some(summary.as_str()), "at {end}");
            }
            if end >= rpm_end {
                assert_eq!(draft.setups[0].operations[0].rpm, Some(8000.), "at {end}");
            }
            for (s, setup) in draft.setups.iter().enumerate() {
                for (o, op) in setup.operations.iter().enumerate() {
                    let before = seen.entry((s, o)).or_default();
                    for (was, now, what) in [
                        (
                            before.tool.map(|v| v as f64),
                            op.tool.map(|v| v as f64),
                            "tool",
                        ),
                        (before.rpm, op.rpm, "rpm"),
                        (before.feed, op.feed, "feed"),
                    ] {
                        assert!(
                            was.is_none() || was == now,
                            "at {end}: {what} {was:?} -> {now:?}"
                        );
                    }
                    *before = op.clone();
                }
            }
        }
        let done = partial(&full);
        assert!(!done.writing);
        assert_eq!(
            done.summary.as_deref(),
            Some("Top side, then the underside.")
        );
        assert_eq!(done.setups.len(), 2);
        let cut = partial(&full[..full.find("then").unwrap()]);
        assert_eq!(cut.summary.as_deref(), Some("Top side, "));
        assert!(cut.writing && cut.setups.is_empty());
        // A number being written is not shown until it ends.
        let at = full.find("\"rpm\":").unwrap() + 7;
        let draft = partial(&full[..at]);
        assert_eq!(draft.setups[0].operations[0].rpm, None);
    }
    #[test]
    fn text_before_the_json_and_a_fence_are_skipped() {
        let draft = partial(
            "Here is the plan:\n```json\n{\"summary\": \"Face it\", \"setups\": [{\"name\": \"Top\"",
        );
        assert_eq!(draft.summary.as_deref(), Some("Face it"));
        assert_eq!(draft.setups[0].name.as_deref(), Some("Top"));
        assert_eq!(partial("thinking…"), Draft::default());
        let done = partial("```json\n{\"summary\": \"x\", \"setups\": []}\n```");
        assert!(!done.writing);
    }
    #[test]
    fn only_objects_are_setups_and_operations() {
        let draft =
            partial(r#"{"setups":[1,2,{"name":"A","operations":[3,{"strategy":"Face"},"x",null]}"#);
        assert_eq!(draft.setups.len(), 1);
        assert_eq!(draft.setups[0].name.as_deref(), Some("A"));
        assert_eq!(draft.setups[0].operations.len(), 1);
        let hostile = format!("{{\"setups\":[{}", "1,".repeat(5000));
        assert!(partial(&hostile).setups.is_empty());
        let hostile = format!("{{\"setups\":[{{\"operations\":[{}", "[],".repeat(5000));
        assert!(partial(&hostile).setups[0].operations.is_empty());
    }
    #[test]
    fn partial_is_bounded_and_never_panics() {
        let deep = "{\"setups\":".to_string() + &"[".repeat(100_000);
        let _ = partial(&deep);
        let escape = "{\"summary\":\"a\\";
        assert_eq!(partial(escape).summary.as_deref(), Some("a"));
        let unicode = "{\"summary\":\"a\\u00";
        assert_eq!(partial(unicode).summary.as_deref(), Some("a"));
        // A reply past the limit is read up to it, not whole.
        let huge = format!("{{\"summary\":\"{}", "x".repeat(4 << 20));
        let summary = partial(&huge).summary.expect("the summary so far");
        assert!(
            (PARTIAL_LIMIT / 2..=PARTIAL_LIMIT).contains(&summary.len()),
            "{}",
            summary.len()
        );
    }
    #[test]
    fn a_long_array_of_numbers_keeps_everything_before_it() {
        let (_, _, report) = block();
        let mut proposal = good(&report);
        proposal.setups[0].operations[1].surfaces = (1000..1500).collect();
        let full = serde_json::to_string(&proposal).unwrap();
        let start = full.find("[1000,1001,").unwrap();
        let end = start + full[start..].find(']').unwrap();
        for at in start..=end {
            let draft = partial(&full[..at]);
            let place = format!("at {at}: {}", &full[at - 6..at]);
            assert_eq!(
                draft.summary.as_deref(),
                Some("Top side, then the underside."),
                "{place}"
            );
            assert_eq!(draft.setups.len(), 1, "{place}");
            let ops = &draft.setups[0].operations;
            assert_eq!(ops.len(), 2, "{place}");
            assert_eq!(ops[0].rpm, Some(8000.), "{place}");
            assert_eq!(ops[1].tool, Some(1), "{place}");
            assert!(draft.writing, "{place}");
        }
        // Cut just after a digit, and just after a comma.
        for at in [start + 5, start + 6] {
            assert_eq!(partial(&full[..at]).setups[0].operations.len(), 2);
        }
    }
    #[test]
    fn a_finished_number_shows_at_its_comma_and_one_being_written_hides_only_itself() {
        let head = r#"{"summary":"s","setups":[{"name":"Top","operations":[{"strategy":"Face","tool":2,"surfaces":[3],"rpm":8000,"#;
        let op = |text: String| partial(&text).setups[0].operations[0].clone();
        let after_comma = op(head.to_string());
        assert_eq!((after_comma.tool, after_comma.rpm), (Some(2), Some(8000.)));
        let writing = op(format!("{head}\"feed\":5"));
        assert_eq!((writing.tool, writing.rpm), (Some(2), Some(8000.)));
        assert_eq!(writing.feed, None);
        assert_eq!(op(format!("{head}\"feed\":5,")).feed, Some(5.));
        // Whitespace after a number ends it as well.
        assert_eq!(op(format!("{head} \"feed\": 7\n")).feed, Some(7.));
        assert_eq!(op(format!("{head} \"feed\": ")).feed, None);
        // Spaces inside a string are kept.
        assert_eq!(
            partial("{\"summary\": \"a  ").summary.as_deref(),
            Some("a  ")
        );
    }
    /// The block with the good plan built into it: the job a change starts from.
    fn planned() -> (Document, Mesh, PartReport) {
        let (doc, mesh, report) = block();
        let job = build(&doc, &mesh, &report, &good(&report)).unwrap();
        (job, mesh, report)
    }
    /// The block with three setups built into it: +Z, -Z and -Y.
    fn planned_three() -> (Document, Mesh, PartReport) {
        let (doc, mesh, report) = block();
        let mut plan = good(&report);
        let mut third = side_up(&report, "-Y").setups.remove(1);
        third.name = "Side".into();
        plan.setups.push(third);
        let job = build(&doc, &mesh, &report, &plan).unwrap();
        (job, mesh, report)
    }
    /// What the planner is shown of `doc`, as the proposal it would answer
    /// with when it changes nothing.
    fn shown(doc: &Document, report: &PartReport) -> Proposal {
        let mut plan = describe(doc, report).unwrap().current_plan;
        plan.summary = "No change.".into();
        plan
    }
    /// Whether two operations are one: same id, tool, selection and settings.
    fn same_operation(a: &Operation, b: &Operation) -> bool {
        a.id == b.id
            && a.tool == b.tool
            && a.enabled == b.enabled
            && a.strategy == b.strategy
            && a.geometry == b.geometry
            && a.surfaces.as_ref().map(|s| &s.ids) == b.surfaces.as_ref().map(|s| &s.ids)
            && a.p == b.p
    }
    /// The keys whose values differ between two sets of settings.
    fn differing(a: &crate::model::Fields, b: &crate::model::Fields) -> Vec<String> {
        let mut keys: Vec<String> = a.keys().chain(b.keys()).cloned().collect();
        keys.sort();
        keys.dedup();
        keys.retain(|k| a.get(k) != b.get(k));
        keys
    }
    /// The same job with its `index`th operation of setup `si` turned into
    /// one the proposal format cannot carry.
    fn with_adaptive(mut doc: Document, si: usize, index: usize) -> Document {
        doc.setups[si].operations[index].strategy = Strategy::Adaptive;
        doc
    }
    #[test]
    fn describe_shows_the_current_plan_in_the_proposal_format_with_its_numbers() {
        let (doc, _, report) = planned();
        let current = describe(&doc, &report).unwrap();
        assert!(current.kept_operations.is_empty());
        let plan = &current.current_plan;
        assert!(plan.summary.is_empty() && plan.unmachined.is_empty() && plan.warnings.is_empty());
        assert_eq!(plan.setups.len(), 2);
        // Setups count from 1, operations from 1 across the whole job.
        let setup_numbers: Vec<_> = plan.setups.iter().map(|s| s.from).collect();
        assert_eq!(setup_numbers, [Some(1), Some(2)]);
        let op_numbers: Vec<Vec<_>> = plan
            .setups
            .iter()
            .map(|s| s.operations.iter().map(|o| o.from).collect())
            .collect();
        assert_eq!(
            op_numbers,
            [
                vec![Some(1), Some(2), Some(3), Some(4)],
                vec![Some(5), Some(6)]
            ]
        );
        let (first, second) = (&plan.setups[0], &plan.setups[1]);
        assert_eq!(
            (first.orientation.as_str(), first.name.as_str()),
            ("+Z", "Top")
        );
        assert_eq!(first.origin, "Stock top center");
        assert_eq!(first.holder, None);
        assert_eq!(
            (second.orientation.as_str(), second.name.as_str()),
            ("-Z", "Underside")
        );
        assert_eq!(second.origin, "Model origin");
        assert_eq!(
            second.workholding,
            "Flip about X; clamp on the machined walls."
        );
        let strategies: Vec<&str> = first
            .operations
            .iter()
            .map(|o| o.strategy.as_str())
            .collect();
        assert_eq!(strategies, ["Face", "Pocket", "Contour", "Drill"]);
        let face = &first.operations[0];
        assert_eq!(face.name, "Face by planner");
        let mut surfaces = ids(&report, "+Z", 0.);
        surfaces.sort_unstable();
        assert_eq!(face.surfaces, surfaces);
        assert_eq!((face.tool, face.rpm, face.feed), (1, 8000., 1000.));
        assert_eq!(face.coolant, "Flood");
        assert_eq!(face.facing_area.as_deref(), Some("stock"));
        assert!(face.reason.is_empty() && face.contour_mode.is_none());
        assert_eq!(first.operations[2].contour_mode.as_deref(), Some("outside"));
        let drill = &first.operations[3];
        assert_eq!((drill.tool, drill.rpm), (3, 12000.));
        assert_eq!(drill.drill_cycle.as_deref(), Some("Deep drilling"));
        assert_eq!((drill.peck, drill.through), (Some(3.), Some(false)));
        // A value the format has no word for reads as its neutral choice.
        let mut odd = doc.clone();
        odd.setups[0].operations[0]
            .p
            .insert("coolant".into(), "Oil".into());
        let shown = describe(&odd, &report).unwrap();
        assert_eq!(shown.current_plan.setups[0].operations[0].coolant, "Off");
        // The JSON the planner is sent names both parts.
        let json = serde_json::to_value(&current).unwrap();
        assert_eq!(json["current_plan"]["setups"][1]["from"], 2);
        assert_eq!(
            json["current_plan"]["setups"][1]["operations"][1]["from"],
            6
        );
        assert_eq!(json["kept_operations"], json!([]));
    }
    #[test]
    fn describe_gives_setup_1_its_planner_note_as_the_workholding() {
        let (mut doc, _, report) = planned();
        doc.setups[0].p.insert(
            crate::planner::PLANNER_NOTE_KEY.into(),
            "Vise on the long sides.".into(),
        );
        let plan = describe(&doc, &report).unwrap().current_plan;
        assert_eq!(plan.setups[0].workholding, "Vise on the long sides.");
    }
    #[test]
    fn an_operation_the_format_cannot_carry_is_listed_and_still_counted() {
        let (doc, _, report) = planned();
        // A strategy that is not one of the six.
        let adaptive = with_adaptive(doc.clone(), 0, 1);
        let current = describe(&adaptive, &report).unwrap();
        assert_eq!(
            current.kept_operations,
            [KeptOp {
                setup: 1,
                from: 2,
                position: 2,
                strategy: "Adaptive clearing",
                name: get(&adaptive.setups[0].operations[1].p, "name").to_string(),
            }]
        );
        let first = &current.current_plan.setups[0];
        let numbers: Vec<_> = first.operations.iter().map(|o| o.from).collect();
        assert_eq!(numbers, [Some(1), Some(3), Some(4)], "its number is spent");
        assert_eq!(current.current_plan.setups[1].operations[0].from, Some(5));
        // A tool without a number.
        let mut bare = doc.clone();
        bare.tools[2].p.insert("number".into(), String::new());
        let current = describe(&bare, &report).unwrap();
        let kept: Vec<_> = current
            .kept_operations
            .iter()
            .map(|k| (k.setup, k.from, k.position, k.strategy))
            .collect();
        assert_eq!(kept, [(1, 4, 4, "Drilling")]);
        // A contour that machines a selection that is not surfaces.
        let mut geometry = doc.clone();
        geometry.setups[1].operations[1].strategy = Strategy::Contour;
        geometry.setups[1].operations[1].surfaces = None;
        geometry.setups[1].operations[1].geometry = vec![3];
        let current = describe(&geometry, &report).unwrap();
        let kept: Vec<_> = current
            .kept_operations
            .iter()
            .map(|k| (k.setup, k.from, k.position, k.strategy))
            .collect();
        assert_eq!(kept, [(2, 6, 2, "Contour")]);
        assert_eq!(current.current_plan.setups[1].operations.len(), 1);
        // A contour with no selection at all is carried.
        assert!(
            describe(&doc, &report).unwrap().current_plan.setups[0]
                .operations
                .iter()
                .any(|o| o.strategy == "Contour")
        );
    }
    #[test]
    fn a_later_setup_that_is_not_turned_square_to_the_part_cannot_be_described() {
        let (mut doc, _, report) = planned();
        doc.setups[1].p.insert("rx".into(), "45".into());
        let error = describe(&doc, &report).unwrap_err();
        assert_eq!(
            error,
            "Setup 2 is not turned square to the part, so the planner cannot change this plan."
        );
        let mut none = doc.clone();
        none.setups.clear();
        assert_eq!(
            describe(&none, &report).unwrap_err(),
            "The project has no setup 1."
        );
    }
    #[test]
    fn building_what_describe_shows_keeps_every_setup_and_operation_identical() {
        let (mut doc, mesh, report) = planned();
        // What the user did to the plan since: settings the format does not
        // carry, a renamed setup with its own notes, a renamed operation.
        doc.setups[0].operations[0]
            .p
            .insert("retract".into(), "9".into());
        doc.setups[0].operations[0]
            .p
            .insert("facing_pattern".into(), "One way".into());
        doc.setups[0].operations[1]
            .p
            .insert("name".into(), "  Rough \"deep\" pocket  ".into());
        doc.setups[1].p.insert("name".into(), "Flip over".into());
        doc.setups[1]
            .p
            .insert("notes".into(), "Soft jaws, tap the part down.".into());
        doc.setups[1].operations[1].enabled = false;
        let rebuilt = build(&doc, &mesh, &report, &shown(&doc, &report)).unwrap();
        assert_eq!(rebuilt.setups.len(), doc.setups.len());
        for (kept, user) in rebuilt.setups.iter().zip(&doc.setups) {
            assert_eq!(kept.id, user.id);
            assert_eq!(kept.p, user.p, "setup {}", user.id);
            assert_eq!(kept.operations.len(), user.operations.len());
            for (kept, user) in kept.operations.iter().zip(&user.operations) {
                assert!(same_operation(kept, user), "{}", user.id);
            }
        }
        assert_eq!(
            get(&rebuilt.setups[0].operations[0].p, "retract"),
            "9",
            "a key the format does not carry stays"
        );
        assert_eq!(rebuilt.active, doc.active);
        assert!(rebuilt.validate_structure().is_ok());
    }
    #[test]
    fn a_reason_a_cleaned_name_and_the_order_of_surfaces_do_not_change_a_kept_operation() {
        let (mut doc, mesh, report) = planned();
        doc.setups[0].operations[2]
            .p
            .insert("name".into(), "Outline \u{202e}\"fast\"".into());
        let mut plan = shown(&doc, &report);
        for setup in &mut plan.setups {
            for op in &mut setup.operations {
                op.reason = "I looked at it again.".into();
                op.surfaces.reverse();
            }
        }
        let rebuilt = build(&doc, &mesh, &report, &plan).unwrap();
        for (kept, user) in rebuilt.setups.iter().zip(&doc.setups) {
            assert_eq!(kept.p, user.p);
            for (kept, user) in kept.operations.iter().zip(&user.operations) {
                assert!(same_operation(kept, user), "{}", user.id);
            }
        }
    }
    #[test]
    fn a_change_keeps_the_active_setup_when_it_is_still_there() {
        let (mut doc, mesh, report) = planned();
        doc.active = doc.setups[1].id.clone();
        let rebuilt = build(&doc, &mesh, &report, &shown(&doc, &report)).unwrap();
        assert_eq!(rebuilt.active, doc.setups[1].id);
        // The setup the user was on is gone: setup 1 is active.
        let mut plan = shown(&doc, &report);
        plan.setups.truncate(1);
        let rebuilt = build(&doc, &mesh, &report, &plan).unwrap();
        assert_eq!(rebuilt.active, doc.setups[0].id);
        // Planning from zero (no `from`) goes back to setup 1 as before.
        let fresh = build(&doc, &mesh, &report, &good(&report)).unwrap();
        assert_eq!(fresh.active, fresh.setups[0].id);
    }
    #[test]
    fn changing_one_field_of_a_kept_operation_writes_only_that_key() {
        let (mut doc, mesh, report) = planned();
        doc.setups[0].operations[1]
            .p
            .insert("retract".into(), "9".into());
        let change = |edit: &dyn Fn(&mut ProposedOp)| {
            let mut plan = shown(&doc, &report);
            edit(&mut plan.setups[0].operations[1]);
            let rebuilt = build(&doc, &mesh, &report, &plan).unwrap();
            let (before, after) = (
                &doc.setups[0].operations[1],
                &rebuilt.setups[0].operations[1],
            );
            assert_eq!(after.id, before.id);
            assert_eq!(after.tool, before.tool);
            assert_eq!(
                after.surfaces.as_ref().map(|s| &s.ids),
                before.surfaces.as_ref().map(|s| &s.ids)
            );
            assert_eq!(get(&after.p, "retract"), "9");
            // Nothing else of the job moved.
            for si in 0..2 {
                for oi in 0..doc.setups[si].operations.len() {
                    if (si, oi) != (0, 1) {
                        assert!(same_operation(
                            &rebuilt.setups[si].operations[oi],
                            &doc.setups[si].operations[oi]
                        ));
                    }
                }
            }
            (differing(&before.p, &after.p), after.p.clone())
        };
        let (keys, p) = change(&|o| o.rpm = 9000.);
        assert_eq!((keys, get(&p, "rpm")), (vec!["rpm".to_string()], "9000"));
        let (keys, p) = change(&|o| o.feed = 750.);
        assert_eq!((keys, get(&p, "feed")), (vec!["feed".to_string()], "750"));
        let (keys, p) = change(&|o| o.coolant = "Mist".into());
        assert_eq!(
            (keys, get(&p, "coolant")),
            (vec!["coolant".to_string()], "Mist")
        );
        let (keys, p) = change(&|o| o.stepdown = Some(1.5));
        assert_eq!(
            (keys, get(&p, "stepdown")),
            (vec!["stepdown".to_string()], "1.5")
        );
        let (keys, p) = change(&|o| o.name = "Rough pocket".into());
        assert_eq!(
            (keys, get(&p, "name")),
            (vec!["name".to_string()], "Rough pocket")
        );
        let (keys, p) = change(&|o| o.finish_passes = Some(2));
        assert_eq!(
            (keys, get(&p, "finish_passes")),
            (vec!["finish_passes".to_string()], "2")
        );
        // Two changes, two keys.
        let (keys, _) = change(&|o| {
            o.rpm = 9000.;
            o.stepover_percent = Some(30.);
        });
        assert_eq!(keys, ["rpm", "stepover"]);
    }
    #[test]
    fn leaving_an_operation_out_removes_it_and_a_null_from_adds_a_new_one() {
        let (doc, mesh, report) = planned();
        let originals: Vec<String> = doc.setups[0]
            .operations
            .iter()
            .map(|o| o.id.clone())
            .collect();
        // Out: the Contour (operation 3).
        let mut plan = shown(&doc, &report);
        plan.setups[0].operations.remove(2);
        let rebuilt = build(&doc, &mesh, &report, &plan).unwrap();
        let ids_now: Vec<&str> = rebuilt.setups[0]
            .operations
            .iter()
            .map(|o| o.id.as_str())
            .collect();
        assert_eq!(
            ids_now,
            [&originals[0], &originals[1], &originals[3]],
            "the others keep their ids, in order"
        );
        // In: a new operation with from null, between two kept ones.
        let mut plan = shown(&doc, &report);
        let mut added = op("Contour", 1, vec![]);
        added.name = "Skim cut".into();
        plan.setups[0].operations.insert(1, added);
        let rebuilt = build(&doc, &mesh, &report, &plan).unwrap();
        let operations = &rebuilt.setups[0].operations;
        assert_eq!(operations.len(), 5);
        assert_eq!(get(&operations[1].p, "name"), "Skim cut");
        assert!(
            !doc.setups
                .iter()
                .flat_map(|s| &s.operations)
                .any(|o| o.id == operations[1].id),
            "a new operation has a new id"
        );
        for (now, was) in [(0, 0), (2, 1), (3, 2), (4, 3)] {
            assert!(same_operation(
                &operations[now],
                &doc.setups[0].operations[was]
            ));
        }
        // Reordered: the kept operations follow the proposal's order.
        let mut plan = shown(&doc, &report);
        plan.setups[0].operations.swap(1, 2);
        let rebuilt = build(&doc, &mesh, &report, &plan).unwrap();
        let ids_now: Vec<&str> = rebuilt.setups[0]
            .operations
            .iter()
            .map(|o| o.id.as_str())
            .collect();
        assert_eq!(
            ids_now,
            [&originals[0], &originals[2], &originals[1], &originals[3]]
        );
    }
    #[test]
    fn an_operation_of_another_strategy_or_contour_mode_is_built_fresh() {
        let (mut doc, mesh, report) = planned();
        doc.setups[0].operations[1]
            .p
            .insert("retract".into(), "9".into());
        doc.setups[0].operations[2]
            .p
            .insert("retract".into(), "9".into());
        // The Pocket becomes a Contour: nothing of the old settings stays.
        let mut plan = shown(&doc, &report);
        let mut contour = op("Contour", 1, vec![]);
        contour.from = Some(2);
        plan.setups[0].operations[1] = contour;
        let rebuilt = build(&doc, &mesh, &report, &plan).unwrap();
        let now = &rebuilt.setups[0].operations[1];
        assert_eq!(now.strategy, Strategy::Contour);
        assert_ne!(now.id, doc.setups[0].operations[1].id);
        assert_eq!(get(&now.p, "retract"), "5");
        // The outside Contour becomes an inside one.
        let mut plan = shown(&doc, &report);
        let mut inside = op("Contour", 1, ids(&report, "+Z", 12.));
        inside.contour_mode = Some("inside_floor".into());
        inside.from = Some(3);
        plan.setups[0].operations[2] = inside;
        let rebuilt = build(&doc, &mesh, &report, &plan).unwrap();
        let now = &rebuilt.setups[0].operations[2];
        assert_eq!(get(&now.p, "side"), "Inside");
        assert_ne!(now.id, doc.setups[0].operations[2].id);
        assert_eq!(get(&now.p, "retract"), "5");
    }
    /// What the problems of building `edit` of the planned job say.
    fn from_problems(
        doc: &Document,
        mesh: &Mesh,
        report: &PartReport,
        edit: &dyn Fn(&mut Proposal),
    ) -> Vec<String> {
        let mut plan = shown(doc, report);
        edit(&mut plan);
        messages(build(doc, mesh, report, &plan))
    }
    #[test]
    fn a_from_that_is_not_in_the_plan_is_a_problem() {
        let (doc, mesh, report) = planned();
        let problems = |edit: &dyn Fn(&mut Proposal)| from_problems(&doc, &mesh, &report, edit);
        for number in [0, 3, 9] {
            // Its operations, which name setup 2's, are refused with it.
            let found = problems(&|p| p.setups[1].from = Some(number));
            assert_eq!(
                found[0],
                format!("2/None: from {number} is not a setup of the current plan.")
            );
            assert_eq!(found.len(), 3, "{found:?}");
        }
        for number in [0, 7, 99] {
            assert_eq!(
                problems(&|p| p.setups[0].operations[0].from = Some(number)),
                [format!(
                    "1/Some(1): from {number} is not an operation of the current plan."
                )]
            );
        }
    }
    #[test]
    fn an_operation_keeps_its_from_only_in_its_own_setup() {
        let (doc, mesh, report) = planned();
        let problems = |edit: &dyn Fn(&mut Proposal)| from_problems(&doc, &mesh, &report, edit);
        // Operation 5 belongs to setup 2.
        let found = problems(&|p| p.setups[0].operations[0].from = Some(5));
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(
            found[0].starts_with(
                "1/Some(1): from 5 is an operation of setup 2 of the current plan, which this setup does not keep"
            ),
            "{found:?}"
        );
        // Moving an operation to the other setup keeps it from nowhere.
        let found = problems(&|p| {
            let moved = p.setups[0].operations.remove(0);
            p.setups[1].operations.push(moved);
        });
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(
            found[0].starts_with("2/Some(3): from 1 is an operation of setup 1"),
            "{found:?}"
        );
        // A setup that is not kept cannot keep operations.
        let found = problems(&|p| p.setups[1].from = None);
        assert_eq!(found.len(), 2, "{found:?}");
        assert!(found.iter().all(|m| m.starts_with("2/Some(")), "{found:?}");
        assert!(found.iter().all(|m| m.contains("of setup 2")), "{found:?}");
        // Same when it is turned another way: it is a new setup.
        let found = problems(&|p| p.setups[1].orientation = "-Y".into());
        assert_eq!(found.len(), 2, "{found:?}");
    }
    #[test]
    fn a_from_used_twice_is_a_problem() {
        let (doc, mesh, report) = planned_three();
        let problems = |edit: &dyn Fn(&mut Proposal)| from_problems(&doc, &mesh, &report, edit);
        assert_eq!(
            problems(&|p| p.setups[0].operations[1].from = Some(1)),
            ["1/Some(2): from 1 is used by an earlier operation."]
        );
        let found = problems(&|p| p.setups[2].from = Some(2));
        assert_eq!(found[0], "3/None: from 2 is used by an earlier setup.");
    }
    #[test]
    fn setup_1_stays_first_and_keeps_from_1() {
        let (doc, mesh, report) = planned();
        let problems = |edit: &dyn Fn(&mut Proposal)| from_problems(&doc, &mesh, &report, edit);
        let found = problems(&|p| p.setups[0].from = Some(2));
        assert_eq!(
            found[0],
            "1/None: The first setup is the current setup 1: its from must be 1 or null."
        );
        let found = problems(&|p| p.setups[1].from = Some(1));
        assert_eq!(
            found[0],
            "2/None: Setup 1 stays first: no later setup can have from 1."
        );
        // Null for setup 1 is fine: its operations are then new ones.
        let mut plan = shown(&doc, &report);
        plan.setups[0].from = None;
        plan.setups[0]
            .operations
            .iter_mut()
            .for_each(|o| o.from = None);
        let rebuilt = build(&doc, &mesh, &report, &plan).unwrap();
        assert_eq!(rebuilt.setups[0].id, doc.setups[0].id);
        assert_ne!(
            rebuilt.setups[0].operations[0].id,
            doc.setups[0].operations[0].id
        );
    }
    #[test]
    fn an_operation_the_planner_cannot_see_cannot_be_named_in_a_proposal() {
        let (doc, mesh, report) = planned();
        let doc = with_adaptive(doc, 0, 1);
        let found = from_problems(&doc, &mesh, &report, &|p| {
            p.setups[0]
                .operations
                .insert(1, op("Pocket", 1, ids(&report, "+Z", 12.)));
            p.setups[0].operations[1].from = Some(2);
        });
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(
            found[0].starts_with("1/Some(2): from 2 is one of the kept_operations"),
            "{found:?}"
        );
    }
    #[test]
    fn a_kept_operation_returns_to_its_place_in_its_setup() {
        let (doc, mesh, report) = planned();
        let doc = with_adaptive(doc, 0, 1);
        let originals: Vec<String> = doc.setups[0]
            .operations
            .iter()
            .map(|o| o.id.clone())
            .collect();
        let current = describe(&doc, &report).unwrap();
        assert_eq!(current.current_plan.setups[0].operations.len(), 3);
        // Unchanged: the whole setup is as it was, the Adaptive in place.
        let rebuilt = build(&doc, &mesh, &report, &shown(&doc, &report)).unwrap();
        for (now, was) in rebuilt.setups[0]
            .operations
            .iter()
            .zip(&doc.setups[0].operations)
        {
            assert!(same_operation(now, was), "{}", was.id);
        }
        assert_eq!(rebuilt.setups[0].operations.len(), 4);
        // The Face before it is left out: it returns at its own index.
        let mut plan = shown(&doc, &report);
        plan.setups[0].operations.remove(0);
        let rebuilt = build(&doc, &mesh, &report, &plan).unwrap();
        let ids_now: Vec<&str> = rebuilt.setups[0]
            .operations
            .iter()
            .map(|o| o.id.as_str())
            .collect();
        assert_eq!(ids_now, [&originals[2], &originals[1], &originals[3]]);
        // Only the last one kept: it stays at its index, past the end of what is there.
        let mut plan = shown(&doc, &report);
        plan.setups[0].operations.drain(..2);
        let rebuilt = build(&doc, &mesh, &report, &plan).unwrap();
        let ids_now: Vec<&str> = rebuilt.setups[0]
            .operations
            .iter()
            .map(|o| o.id.as_str())
            .collect();
        assert_eq!(ids_now, [&originals[3], &originals[1]]);
    }
    #[test]
    fn a_setup_with_kept_operations_must_be_kept() {
        let (doc, mesh, report) = planned();
        let doc = with_adaptive(doc, 1, 1);
        let problems = |edit: &dyn Fn(&mut Proposal)| from_problems(&doc, &mesh, &report, edit);
        let needle = "Setup 2 of the current plan has kept_operations";
        assert!(build(&doc, &mesh, &report, &shown(&doc, &report)).is_ok());
        // Dropped.
        let found = problems(&|p| p.setups.truncate(1));
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(
            found[0].starts_with(&format!("0/None: {needle}")),
            "{found:?}"
        );
        assert!(
            found[0].contains("keep that setup with from 2 and orientation -Z"),
            "{found:?}"
        );
        // Replaced by a new setup of the same orientation, or turned another way.
        let found = problems(&|p| {
            p.setups[1].from = None;
            p.setups[1]
                .operations
                .iter_mut()
                .for_each(|o| o.from = None);
        });
        assert!(found.iter().any(|m| m.contains(needle)), "{found:?}");
        let found = problems(&|p| {
            p.setups[1].orientation = "-Y".into();
            p.setups[1]
                .operations
                .iter_mut()
                .for_each(|o| o.from = None);
        });
        assert!(found.iter().any(|m| m.contains(needle)), "{found:?}");
        // Planning from zero (no from anywhere) replaces the plan as it always did.
        let rebuilt = build(&doc, &mesh, &report, &good(&report)).unwrap();
        assert!(
            rebuilt
                .setups
                .iter()
                .flat_map(|s| &s.operations)
                .all(|o| o.strategy != Strategy::Adaptive)
        );
    }
    /// The planned job with a vise that holds setup 1 and the placement of
    /// setup 2 the user laid out by hand; its report lists the library.
    fn planned_and_held() -> (Document, Mesh, PartReport) {
        let (mut doc, mesh, _) = block();
        hold(
            &mut doc,
            "Vise",
            &[
                ("vise_axis", "Y"),
                ("vise_grip", "8"),
                ("holding_gap", "3.5"),
            ],
        );
        let report = analysis::report(&doc, &mesh).unwrap();
        let mut job = build(&doc, &mesh, &report, &good(&report)).unwrap();
        let p = &mut job.setups[1].p;
        p.insert("name".into(), "Flip over".into());
        p.insert("notes".into(), "Soft jaws.".into());
        p.insert("offset".into(), "G57".into());
        p.insert("vise_axis".into(), "Y".into());
        p.insert("vise_offset".into(), "4".into());
        p.insert("vise_grip".into(), "7".into());
        p.insert("clamps".into(), "-X:0;+X:10".into());
        (job, mesh, report)
    }
    #[test]
    fn a_kept_later_setup_keeps_its_id_offset_notes_and_placement() {
        let (doc, mesh, report) = planned_and_held();
        let rebuilt = build(&doc, &mesh, &report, &shown(&doc, &report)).unwrap();
        assert_eq!(rebuilt.setups[1].id, doc.setups[1].id);
        assert_eq!(rebuilt.setups[1].p, doc.setups[1].p);
        // Setup 1 held, setup 2 in a clamp of its own: that stays too.
        let mut plan = shown(&doc, &report);
        plan.setups[1].holder = Some(2);
        let held = build(&doc, &mesh, &report, &plan).unwrap();
        assert_eq!(get(&held.setups[1].p, "holding"), "Clamps");
        let second = shown(&held, &report);
        assert_eq!(second.setups[1].holder, Some(2));
        let again = build(&held, &mesh, &report, &second).unwrap();
        assert_eq!(again.setups[1].p, held.setups[1].p);
    }
    #[test]
    fn changing_a_kept_setups_name_origin_or_notes_writes_only_that() {
        let (doc, mesh, report) = planned_and_held();
        let change = |edit: &dyn Fn(&mut ProposedSetup)| {
            let mut plan = shown(&doc, &report);
            edit(&mut plan.setups[1]);
            let rebuilt = build(&doc, &mesh, &report, &plan).unwrap();
            assert_eq!(rebuilt.setups[1].id, doc.setups[1].id);
            (
                differing(&doc.setups[1].p, &rebuilt.setups[1].p),
                rebuilt.setups[1].p.clone(),
            )
        };
        let (keys, p) = change(&|s| s.name = "Underside, second".into());
        assert_eq!(keys, ["name"]);
        assert_eq!(get(&p, "name"), "Underside, second");
        let (keys, p) = change(&|s| s.origin = "Stock top center".into());
        assert_eq!(keys, ["origin"]);
        assert_eq!(get(&p, "origin"), "Stock top center");
        let (keys, p) = change(&|s| s.workholding = "Hold in the soft jaws.".into());
        assert_eq!(keys, ["notes"]);
        assert_eq!(get(&p, "notes"), "Hold in the soft jaws.");
        // An origin that is not a choice is still refused.
        let mut plan = shown(&doc, &report);
        plan.setups[1].origin = "Somewhere".into();
        assert!(
            messages(build(&doc, &mesh, &report, &plan))
                .iter()
                .any(|m| m.contains("not a work zero choice"))
        );
    }
    #[test]
    fn a_holder_change_of_a_kept_setup_clears_its_placement() {
        let (doc, mesh, report) = planned_and_held();
        let (vise, clamp) = (doc.workholding[0].id.clone(), doc.workholding[1].id.clone());
        let mut plan = shown(&doc, &report);
        plan.setups[1].holder = Some(2);
        let rebuilt = build(&doc, &mesh, &report, &plan).unwrap();
        let second = &rebuilt.setups[1];
        assert_eq!(second.id, doc.setups[1].id);
        assert_eq!(
            (get(&second.p, "holding"), get(&second.p, "holder")),
            ("Clamps", clamp.as_str())
        );
        for key in ["vise_axis", "vise_offset", "vise_grip", "clamps"] {
            assert!(!second.p.contains_key(key), "{key}");
        }
        // The rest of the setup is as the user left it.
        assert_eq!(
            differing(&doc.setups[1].p, &second.p),
            [
                "clamps",
                "holder",
                "holding",
                "vise_axis",
                "vise_grip",
                "vise_offset"
            ]
        );
        assert_eq!(get(&second.p, "offset"), "G57");
        assert_eq!(get(&second.p, "notes"), "Soft jaws.");
        assert_eq!(get(&second.p, "name"), "Flip over");
        // Back to setup 1's holder (null): the placement goes again.
        let (mut doc, mesh, report) = (rebuilt, mesh, report);
        doc.setups[1].p.insert("clamps".into(), "-X:0".into());
        let mut plan = shown(&doc, &report);
        assert_eq!(plan.setups[1].holder, Some(2));
        plan.setups[1].holder = None;
        let rebuilt = build(&doc, &mesh, &report, &plan).unwrap();
        let second = &rebuilt.setups[1];
        assert_eq!(
            (get(&second.p, "holding"), get(&second.p, "holder")),
            ("Vise", vise.as_str())
        );
        assert!(!second.p.contains_key("clamps"));
    }
    #[test]
    fn a_new_setup_does_not_take_a_kept_setups_offset() {
        let (doc, mesh, report) = planned_three();
        let offsets: Vec<&str> = doc.setups.iter().map(|s| get(&s.p, "offset")).collect();
        assert_eq!(offsets, ["G54", "G55", "G56"]);
        // A new setup comes before the kept setup 2: it must not take G55.
        let mut plan = shown(&doc, &report);
        let kept = plan.setups.remove(1);
        let mut new = plan.setups.pop().unwrap();
        new.from = None;
        new.name = "New side".into();
        new.operations.iter_mut().for_each(|o| o.from = None);
        plan.setups.push(new);
        plan.setups.push(kept);
        let rebuilt = build(&doc, &mesh, &report, &plan).unwrap();
        assert_eq!(rebuilt.setups.len(), 3);
        assert_eq!(rebuilt.setups[2].id, doc.setups[1].id);
        assert_eq!(get(&rebuilt.setups[2].p, "offset"), "G55");
        assert_ne!(rebuilt.setups[1].id, doc.setups[2].id);
        assert_eq!(get(&rebuilt.setups[1].p, "offset"), "G56");
        let mut now: Vec<&str> = rebuilt.setups.iter().map(|s| get(&s.p, "offset")).collect();
        now.sort_unstable();
        now.dedup();
        assert_eq!(now.len(), 3, "every setup has an offset of its own");
        assert!(rebuilt.validate_structure().is_ok());
    }
    #[test]
    fn a_setup_turned_another_way_is_a_new_setup() {
        let (doc, mesh, report) = planned();
        let mut plan = shown(&doc, &report);
        let side = side_up(&report, "-Y").setups.remove(1);
        plan.setups[1] = ProposedSetup {
            from: Some(2),
            name: "Side".into(),
            ..side
        };
        let rebuilt = build(&doc, &mesh, &report, &plan).unwrap();
        let second = &rebuilt.setups[1];
        assert_ne!(second.id, doc.setups[1].id);
        assert_eq!(analysis::up_axis(second), Some("-Y"));
        assert_ne!(
            (get(&second.p, "rx"), get(&second.p, "ry")),
            (get(&doc.setups[1].p, "rx"), get(&doc.setups[1].p, "ry"))
        );
        assert_eq!(get(&second.p, "name"), "Side");
        assert_ne!(
            get(&second.p, "offset"),
            get(&doc.setups[1].p, "offset"),
            "the kept setup's offset is not reused by a new one"
        );
        // Turned the same way again, it is kept.
        plan.setups[1].orientation = "-Z".into();
        plan.setups[1].operations = shown(&doc, &report).setups[1].operations.clone();
        let rebuilt = build(&doc, &mesh, &report, &plan).unwrap();
        assert_eq!(rebuilt.setups[1].id, doc.setups[1].id);
    }
    #[test]
    fn the_edit_schema_asks_every_setup_and_operation_for_a_from() {
        assert_eq!(schema(), schema_for(false));
        assert_ne!(schema(), schema_for(true));
        let nullable = json!({"type": ["integer", "null"]});
        let plain = schema();
        let edit = schema_for(true);
        let setup = |s: &Value| s["properties"]["setups"]["items"].clone();
        let operation = |s: &Value| setup(s)["properties"]["operations"]["items"].clone();
        for part in [setup(&plain), operation(&plain)] {
            assert!(part["properties"].get("from").is_none());
            assert!(
                !part["required"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("from"))
            );
        }
        for part in [setup(&edit), operation(&edit)] {
            assert_eq!(part["properties"]["from"], nullable);
            assert!(
                part["required"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("from"))
            );
        }
        // Nothing else differs: the plain schema is the edit one less `from`.
        let mut stripped = edit.clone();
        for path in [
            "/properties/setups/items",
            "/properties/setups/items/properties/operations/items",
        ] {
            let part = stripped.pointer_mut(path).unwrap();
            part["properties"].as_object_mut().unwrap().remove("from");
            part["required"]
                .as_array_mut()
                .unwrap()
                .retain(|k| k != "from");
        }
        assert_eq!(stripped, plain);
    }
    #[test]
    fn a_proposal_without_from_parses_and_none_is_not_written() {
        let (_, _, report) = block();
        let proposal = good(&report);
        let text = serde_json::to_string(&proposal).unwrap();
        assert!(!text.contains("\"from\""), "{text}");
        assert_eq!(parse(&text).unwrap(), proposal);
        // With numbers they are written, and read back.
        let mut with = proposal.clone();
        with.setups[0].from = Some(1);
        with.setups[0].operations[1].from = Some(2);
        let text = serde_json::to_string(&with).unwrap();
        assert_eq!(text.matches("\"from\"").count(), 2, "{text}");
        assert_eq!(parse(&text).unwrap(), with);
        // What the planner answers in edit mode: null for new, or a number.
        let mut value = serde_json::to_value(&proposal).unwrap();
        value["setups"][0]["from"] = json!(1);
        value["setups"][1]["from"] = Value::Null;
        value["setups"][0]["operations"][0]["from"] = json!(3);
        value["setups"][0]["operations"][1]["from"] = Value::Null;
        let parsed = parse(&value.to_string()).unwrap();
        assert_eq!(parsed.setups[0].from, Some(1));
        assert_eq!(parsed.setups[1].from, None);
        assert_eq!(parsed.setups[0].operations[0].from, Some(3));
        assert_eq!(parsed.setups[0].operations[1].from, None);
        // Not a count.
        for bad in [json!(-1), json!(1.5), json!("1")] {
            value["setups"][0]["from"] = bad;
            assert!(parse(&value.to_string()).is_err());
        }
    }
    /// The rotary scenario's project, report and the plan that ends it,
    /// with `edit` applied to the plan.
    fn rotary(edit: impl FnOnce(&mut Proposal)) -> (Document, Mesh, PartReport, Proposal) {
        let (doc, mesh, report) = crate::fake_openai::rotary::report();
        let mut plan = crate::fake_openai::rotary::good();
        edit(&mut plan);
        (doc, mesh, report, plan)
    }
    /// A change of a plan on a rotary machine asks for `from` as well as
    /// the chuck and the angles, and the schema stays strict.
    #[test]
    fn the_schema_of_a_change_on_a_rotary_machine_has_from_and_the_chuck() {
        let both = schema_with(true, true);
        let setup = &both["properties"]["setups"]["items"];
        let op = &setup["properties"]["operations"]["items"];
        for (object, key) in [
            (setup, "from"),
            (setup, "a_angles"),
            (op, "from"),
            (&both, "chuck"),
        ] {
            assert!(object["properties"].get(key).is_some(), "{key}");
            let required = object["required"].as_array().unwrap();
            assert!(required.contains(&json!(key)), "{key}");
            assert_eq!(
                required.len(),
                object["properties"].as_object().unwrap().len()
            );
            assert_eq!(object["additionalProperties"], false);
        }
        let (_, _, report, _) = rotary(|_| {});
        assert_eq!(schema_with(true, false), schema_for(true));
        assert_eq!(schema_with(false, true), schema_on(&report));
        assert_eq!(schema_with(false, false), schema());
    }
    /// The planner cannot change a plan that holds the stock in the chuck
    /// on A or indexes A (it makes those afresh): it says so before
    /// anything is sent.
    #[test]
    fn a_plan_on_the_chuck_or_indexing_a_is_not_described_for_a_change() {
        let (doc, mesh, report, plan) = rotary(|_| {});
        let job = build(&doc, &mesh, &report, &plan).unwrap();
        assert!(crate::chuck::holds(&job.setups[0]));
        let e = describe(&job, &report).unwrap_err();
        assert!(
            e.starts_with(
                "Setup 1 holds the stock in the chuck on A, so the planner cannot change this plan."
            ),
            "{e}"
        );
        let mut loose = job.clone();
        loose.setups[0].p.insert("holding".into(), "None".into());
        assert!(crate::indexed::is_indexed(&loose.setups[1]));
        let e = describe(&loose, &report).unwrap_err();
        assert!(e.starts_with("Setup 2 indexes A"), "{e}");
    }
    #[test]
    fn the_rotary_schema_asks_for_the_chuck_and_the_angles_and_stays_strict() {
        fn walk(value: &Value, objects: &mut usize) {
            if value["type"] == "object" {
                *objects += 1;
                let keys: Vec<&String> = value["properties"].as_object().unwrap().keys().collect();
                assert_eq!(value["required"].as_array().unwrap().len(), keys.len());
                assert_eq!(value["additionalProperties"], false);
            }
            match value {
                Value::Object(map) => map.values().for_each(|v| walk(v, objects)),
                Value::Array(items) => items.iter().for_each(|v| walk(v, objects)),
                _ => {}
            }
        }
        let (_, _, report, plan) = rotary(|_| {});
        let schema = schema_on(&report);
        let mut objects = 0;
        walk(&schema, &mut objects);
        // The proposal, its chuck, a setup, an operation, its tabs and an
        // unmachined entry.
        assert_eq!(objects, 6);
        let keys = |v: &Value| v.as_object().unwrap().keys().cloned().collect::<Vec<_>>();
        let props = |v: &Value| keys(&v["properties"]);
        let value = serde_json::to_value(&plan).unwrap();
        assert_eq!(keys(&value), props(&schema));
        assert_eq!(
            keys(&value["chuck"]),
            props(&schema["properties"]["chuck"]["anyOf"][0])
        );
        let setup = &schema["properties"]["setups"]["items"];
        assert_eq!(keys(&value["setups"][1]), props(setup));
        // The angles are numbers or null; the chuck an object or null.
        assert_eq!(
            setup["properties"]["a_angles"],
            json!({"type": ["array", "null"], "items": {"type": "number"}})
        );
        assert_eq!(
            schema["properties"]["chuck"]["anyOf"][1],
            json!({"type": "null"})
        );
        // What the model answers reads back, null or not.
        assert_eq!(parse(&value.to_string()).unwrap(), plan);
        let mut nulls = value;
        nulls["chuck"] = Value::Null;
        nulls["setups"][1]["a_angles"] = Value::Null;
        let parsed = parse(&nulls.to_string()).unwrap();
        assert!(parsed.chuck.is_none() && parsed.setups[1].a_angles.is_none());
    }
    #[test]
    fn a_machine_without_a_rotary_axis_refuses_a_chuck_and_angles() {
        let (doc, mesh, report) = block();
        assert!(report.rotary.is_none());
        let mut plan = good(&report);
        plan.chuck = rotary(|_| {}).3.chuck;
        plan.setups[1].a_angles = Some(vec![90.]);
        let said = messages(build(&doc, &mesh, &report, &plan));
        assert!(
            said.iter()
                .any(|m| m.contains("`chuck` must be null: this machine has no rotary A axis"))
                && said.iter().any(|m| m.contains("`a_angles` must be null")),
            "{said:?}"
        );
    }
    #[test]
    fn a_chuck_seats_setup_1s_stock_and_an_indexed_setup_is_made_as_the_setups_page_makes_it() {
        let (doc, mesh, report, plan) = rotary(|_| {});
        let built = build(&doc, &mesh, &report, &plan).unwrap();
        let (first, second) = (&built.setups[0], &built.setups[1]);
        // Setup 1: a 24 mm box of the proposal's length, in a 4-jaw chuck with
        // the chuck's defaults and the A axis as its work zero.
        assert_eq!(
            [
                get(&first.p, "stock_type"),
                get(&first.p, "stock_x"),
                get(&first.p, "stock_y")
            ],
            ["Fixed size box", "130", "24"]
        );
        assert_eq!(
            [
                get(&first.p, "holding"),
                get(&first.p, "chuck_jaws"),
                get(&first.p, "chuck_stickout")
            ],
            ["Chuck", "4-jaw", "90"]
        );
        assert_eq!(get(&first.p, "chuck_tailstock"), "Off the table");
        assert_eq!(get(&first.p, "chuck_jaw_angle"), "0");
        assert_eq!(get(&first.p, "holding_gap"), "2");
        assert_eq!(get(&first.p, "origin"), crate::indexed::ORIGINS[0]);
        assert_eq!(first.id, doc.setups[0].id);
        // The part sits in the stick-out, 2 mm short of the free end, on the axis.
        let stock = built.setup_stock(0, Some(&mesh)).unwrap();
        let placed = crate::stock::Placement::new(&mesh, Some(first)).bounds;
        let scale = mesh.display_scale();
        assert!((placed.max[0] / scale - (stock.max[0] - crate::proposal::FREE_END)).abs() < 0.01);
        let mid = |b: crate::mesh::Bounds, k: usize| (b.min[k] + b.max[k]) / 2.;
        assert!((mid(placed, 1) / scale - mid(stock, 1)).abs() < 0.01);
        assert!((mid(placed, 2) / scale - mid(stock, 2)).abs() < 0.01);
        // Setup 2 is the Reposition choice with its angles, and the rest is setup 1's.
        assert_eq!(get(&second.p, "reposition"), crate::indexed::ACTION);
        assert_eq!(get(&second.p, crate::indexed::ANGLES), "0, 90, 180, 270");
        assert!(crate::indexed::is_indexed(second));
        assert_eq!(built.index_base(1), Some(0));
        for key in [
            "offset",
            "holding",
            "chuck_jaws",
            "origin",
            "stock_type",
            "tx",
            "tz",
        ] {
            assert_eq!(get(&second.p, key), get(&first.p, key), "{key}");
        }
        assert_eq!(second.machine.id, first.machine.id);
        assert_eq!(get(&second.p, "notes"), "The bar in the chuck.");
        assert_eq!(second.turn.map(|t| (t.base, t.a)), Some((0, 0.)));
        assert!(built.validate_structure().is_ok());
    }
    #[test]
    fn a_round_bar_is_seated_on_its_axis_and_a_part_that_does_not_fit_is_refused() {
        let round = |diameter, stickout| {
            move |plan: &mut Proposal| {
                let chuck = plan.chuck.as_mut().unwrap();
                chuck.stock = "Round bar".into();
                (chuck.diameter, chuck.width, chuck.height) = (Some(diameter), None, None);
                chuck.jaws = "3-jaw".into();
                chuck.stickout = stickout;
            }
        };
        let (doc, mesh, report, plan) = rotary(round(30., 60.));
        let built = build(&doc, &mesh, &report, &plan).unwrap();
        let first = &built.setups[0];
        assert_eq!(
            [
                get(&first.p, "stock_type"),
                get(&first.p, "stock_diameter"),
                get(&first.p, "stock_length")
            ],
            ["Round bar", "30", "130"]
        );
        // The 20 × 20 mm part is 28.28 mm across its corners.
        let (doc, mesh, report, plan) = rotary(round(28., 60.));
        let said = messages(build(&doc, &mesh, &report, &plan));
        assert_eq!(said.len(), 1, "{said:?}");
        assert!(
            said[0].contains("needs a bar of at least Ø28.29 mm, not Ø28 mm"),
            "{said:?}"
        );
        // 30 mm of part and 2 mm of stock beyond it do not fit a 31 mm stick-out.
        let (doc, mesh, report, plan) = rotary(round(30., 31.));
        let said = messages(build(&doc, &mesh, &report, &plan));
        assert!(
            said[0].contains("needs a stick-out of at least 32.00 mm, not 31 mm"),
            "{said:?}"
        );
    }
    #[test]
    fn a_stock_the_chuck_cannot_hold_is_refused_with_the_chucks_own_message() {
        // A 3-jaw chuck cannot hold a box.
        let (doc, mesh, report, plan) = rotary(|p| p.chuck.as_mut().unwrap().jaws = "3-jaw".into());
        let said = build(&doc, &mesh, &report, &plan).unwrap_err();
        assert_eq!((said[0].setup, said[0].stage), (1, "workholding"));
        assert!(
            said[0]
                .message
                .contains("A 3-jaw chuck centres round stock"),
            "{said:?}"
        );
        // Wider across its corners than the rotary unit takes.
        let (doc, mesh, report, plan) = rotary(|p| {
            let chuck = p.chuck.as_mut().unwrap();
            (chuck.width, chuck.height) = (Some(60.), Some(60.));
        });
        let said = build(&doc, &mesh, &report, &plan).unwrap_err();
        assert!(
            said.iter()
                .any(|p| p.message.contains("the jaws grip from Ø5 to Ø80 mm")),
            "{said:?}"
        );
        // Nothing but the two choices of the schema is a jaw or a tailstock choice.
        let (doc, mesh, report, plan) = rotary(|p| p.chuck.as_mut().unwrap().jaws = "5-jaw".into());
        assert!(
            messages(build(&doc, &mesh, &report, &plan))[0]
                .contains("\"5-jaw\" is not a chuck jaws choice")
        );
    }
    #[test]
    fn the_orientation_of_an_indexed_setup_is_the_one_its_first_angle_brings_up() {
        // With +Z up in setup 1, positive A brings +Y up at 90, -Z at 180, -Y at 270.
        for (angle, up) in [(0., "+Z"), (90., "+Y"), (180., "-Z"), (270., "-Y")] {
            let (doc, mesh, report, plan) = rotary(|p| {
                p.setups[1].a_angles = Some(vec![angle]);
                p.setups[1].orientation = up.into();
                p.setups[1].operations = vec![op("Contour", 1, vec![])];
            });
            assert!(build(&doc, &mesh, &report, &plan).is_ok(), "A {angle} {up}");
        }
        let (doc, mesh, report, plan) = rotary(|p| {
            p.setups[1].a_angles = Some(vec![90., 180.]);
            p.setups[1].orientation = "+Z".into();
        });
        let said = messages(build(&doc, &mesh, &report, &plan));
        assert_eq!(said.len(), 1, "{said:?}");
        assert!(
            said[0].contains("At A 90° the part shows +Y up, not +Z"),
            "{said:?}"
        );
        // An angle that turns no face up has none to name.
        let (doc, mesh, report, plan) = rotary(|p| p.setups[1].a_angles = Some(vec![45.]));
        let said = messages(build(&doc, &mesh, &report, &plan));
        assert!(
            said[0].contains("At A 45° no face of the part is up"),
            "{said:?}"
        );
    }
    #[test]
    fn an_indexed_setup_is_refused_for_what_the_setups_page_refuses() {
        // An angle outside the machine's A range (-360° to 360°), a repeat, none.
        for (angles, words) in [
            (
                vec![400.],
                "A 400° is outside the machine's A range, -360° to 360°.",
            ),
            (vec![90., 90.], "A 90° is listed twice."),
            (vec![], "List the A angles to index to"),
        ] {
            let (doc, mesh, report, plan) = rotary(|p| p.setups[1].a_angles = Some(angles));
            let said = messages(build(&doc, &mesh, &report, &plan));
            assert!(said.iter().any(|m| m.contains(words)), "{words}: {said:?}");
        }
        // Several angles run each operation at every one of them, so no surfaces ...
        let (doc, mesh, report, plan) = rotary(|p| {
            let top = report_top(&crate::fake_openai::rotary::report().2);
            p.setups[1].operations = vec![op("Contour", 1, top)];
        });
        let said = messages(build(&doc, &mesh, &report, &plan));
        assert_eq!(said.len(), 1, "{said:?}");
        assert!(
            said[0]
                .starts_with("2/Some(1): This setup runs every operation at each of its A angles")
                && said[0].contains("Contour outside, Parallel or Waterline"),
            "{said:?}"
        );
        // ... but a Face of the sides each angle brings up (LIO-214): one top
        // plane alone leaves the angles after it nothing to face, a surface
        // that is on top at none of them is refused.
        let (doc, mesh, report, plan) = rotary(|p| {
            let top = report_top(&crate::fake_openai::rotary::report().2);
            p.setups[1].operations = vec![op("Face", 1, top)];
        });
        let said = messages(build(&doc, &mesh, &report, &plan));
        assert_eq!(said.len(), 1, "{said:?}");
        assert!(
            said[0].starts_with("2/Some(1): None of the surfaces is the top plane of +Y"),
            "{said:?}"
        );
        let (doc, mesh, report, plan) = rotary(|p| {
            let mut ids = p.setups[1].operations[0].surfaces.clone();
            ids.push(usize::MAX);
            p.setups[1].operations[0].surfaces = ids;
        });
        let said = messages(build(&doc, &mesh, &report, &plan));
        assert!(
            said.len() == 1
                && said[0].contains(&format!("Surface {} is not the top plane", usize::MAX)),
            "{said:?}"
        );
        // The four sides build as one Face, its tool centre out to the edges.
        let (doc, mesh, report, plan) = rotary(|_| {});
        let built = build(&doc, &mesh, &report, &plan).unwrap();
        let face = &built.setups[1].operations[0];
        assert_eq!(face.strategy, Strategy::Face);
        assert_eq!(face.surfaces.as_ref().unwrap().ids.len(), 4);
        assert_eq!(
            (get(&face.p, "boundary"), get(&face.p, "containment")),
            ("Selected geometry", "Tool center on boundary")
        );
        // One angle may use them: its orientation is told.
        let (doc, mesh, report, plan) = rotary(|p| {
            let top = report_top(&crate::fake_openai::rotary::report().2);
            p.setups[1].a_angles = Some(vec![0.]);
            p.setups[1].operations = vec![op("Face", 1, top)];
        });
        assert!(build(&doc, &mesh, &report, &plan).is_ok());
        // The holder is the chuck's.
        let (mut doc, mesh, report, plan) = rotary(|p| p.setups[1].holder = Some(1));
        doc.adopt_holders();
        let said = messages(build(&doc, &mesh, &report, &plan));
        assert!(
            said.iter()
                .any(|m| m.contains("holder must be null: an indexed setup is held by the chuck")),
            "{said:?}"
        );
        // The stock must be in the chuck: without `chuck` setup 1 keeps the user's.
        let (doc, mesh, report, plan) = rotary(|p| p.chuck = None);
        let said = messages(build(&doc, &mesh, &report, &plan));
        assert!(
            said.iter()
                .any(|m| m.contains("a_angles index the stock in the chuck")),
            "{said:?}"
        );
        // ... and an indexed setup cannot come first.
        let (doc, mesh, report, plan) = rotary(|p| {
            p.setups.remove(0);
            p.setups[0].a_angles = Some(vec![90.]);
            p.setups[0].orientation = "+Y".into();
        });
        let said = messages(build(&doc, &mesh, &report, &plan));
        assert!(
            said.iter()
                .any(|m| m.contains("The first setup cannot index A")),
            "{said:?}"
        );
    }
    /// The top plane's surface IDs of the first orientation.
    fn report_top(report: &PartReport) -> Vec<usize> {
        report.orientations[0]
            .planes
            .iter()
            .find(|p| p.kind == "top")
            .unwrap()
            .surfaces
            .clone()
    }
    #[test]
    fn a_user_chuck_in_setup_1_takes_indexed_setups_without_a_chuck_proposal() {
        let (doc, mesh, report, plan) = rotary(|p| p.chuck = None);
        // Build the chuck into the project as the user would, then plan on it.
        let seated = build(&doc, &mesh, &report, &crate::fake_openai::rotary::good()).unwrap();
        let report = analysis::report(&seated, &mesh).unwrap();
        assert_eq!(report.workholding.as_ref().unwrap().kind, "Chuck");
        let built = build(&seated, &mesh, &report, &plan).unwrap();
        assert_eq!(get(&built.setups[0].p, "holding"), "Chuck");
        assert_eq!(built.index_base(1), Some(0));
        assert_eq!(get(&built.setups[0].p, "stock_x"), "130");
    }
}
