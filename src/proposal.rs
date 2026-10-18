//! The AI planner's proposal: the format the model must return, and how a
//! proposal becomes a job. Only IDs, enumerated choices and numbers reach the
//! job; the model's free text is displayed or stored as notes.
use crate::{
    analysis::{self, OrientationReport, PartReport, PlaneReport},
    mesh::Mesh,
    model::{CLEARANCE, Document, Operation, Setup, Strategy, Tool, get, num},
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
const CYCLES: [&str; 4] = [
    "Drilling",
    "Chip breaking",
    "Deep drilling",
    "Counterboring",
];
const ENTRIES: [&str; 2] = ["Profile ramp", "Plunge"];
/// The stock of a chuck proposal: a round bar, or the user's own box.
const STOCKS: [&str; 2] = ["Round bar", "Box"];
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
    pub coolant: String,
    pub reason: String,
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
    schema_with(false)
}
/// `schema` for the machine of `report`: with a rotary A axis it also asks
/// for the chuck and the A angles of the setups.
pub fn schema_for(report: &PartReport) -> Value {
    schema_with(report.rotary.is_some())
}
fn schema_with(rotary: bool) -> Value {
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
        "contour_mode": choice(&["outside", "inside_floor"], true),
        "pocket_entry": choice(&ENTRIES, true),
        "ramp_angle": optional(),
        "surface_stepover": optional(),
        "tolerance": optional(),
        "drill_cycle": choice(&CYCLES, true),
        "peck": optional(),
        "breakthrough": optional(),
        "hole_diameter": optional(),
        "through": {"type": ["boolean", "null"]},
        "coolant": choice(&COOLANTS, false),
        "reason": text(),
    }));
    let mut setup = json!({
        "orientation": choice(&ups, false),
        "name": text(),
        "origin": choice(&ORIGINS, false),
        "workholding": text(),
        "holder": {"type": ["integer", "null"]},
        "operations": {"type": "array", "items": operation},
    });
    let mut plan = json!({
        "summary": text(),
        "unmachined": {"type": "array", "items": object(json!({"feature": text(), "reason": text()}))},
        "warnings": {"type": "array", "items": text()},
    });
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
    // Negative stock to leave or bottom offset would cut below the model.
    for (field, value) in [
        ("radial_allowance", p.radial_allowance),
        ("axial_allowance", p.axial_allowance),
        ("bottom_offset", p.bottom_offset),
    ] {
        if value.is_some_and(|v| v < 0. && v.is_finite()) {
            wrong.push(format!("{field} cannot be negative."));
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
        (
            &p.contour_mode,
            &["outside", "inside_floor"][..],
            "contour mode",
        ),
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
        Strategy::Drill => ["Drill", "Chamfer mill"].contains(&kind),
        Strategy::Parallel => ["Ball end mill", "Flat end mill"].contains(&kind),
        _ => kind == "Flat end mill",
    };
    if !suits {
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
        Strategy::Contour => {
            set("boundary", "Model silhouette".into());
            set("side", "Outside".into());
            set("containment", "Tool outside boundary".into());
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
    // The entries a proposal's `holder` counts over, as the report numbers
    // them, and the number of setup 1's own, which only setup 1 may name.
    let library = analysis::library(doc);
    let own = analysis::own_entry(doc, setup1);
    let mut setups: Vec<Setup> = vec![];
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
            .find(|o| !setups.iter().any(|s| get(&s.p, "offset") == o))
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
        if !angled
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
        if si > 0 && !angled && !ORIGINS.contains(&proposed.origin.as_str()) {
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
                let holding = if holder.kind() == "Vise" {
                    "Vise"
                } else {
                    "Clamps"
                };
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
        if si > 0 {
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
            match operation(
                op,
                si,
                orientation,
                sides,
                report,
                &doc.tools,
                &mesh.fingerprint,
            ) {
                Ok(op) => setup.operations.push(op),
                Err(messages) => messages.into_iter().for_each(|m| wrong(Some(oi + 1), m)),
            }
        }
        setups.push(setup);
    }
    if !problems.is_empty() {
        return Err(problems);
    }
    let mut built = doc.clone();
    built.active = setups[0].id.clone();
    built.setups = setups;
    if proposal.chuck.is_some() {
        built.settle_holding(0);
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
            coolant: "Flood".into(),
            reason: "test".into(),
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
        let schema = schema();
        let mut objects = 0;
        walk(&schema, &mut objects);
        assert_eq!(objects, 4);
        // Every field of the Rust types is in the schema, and nothing else.
        let (_, _, report) = block();
        let value = serde_json::to_value(good(&report)).unwrap();
        let fields = |v: &Value| v.as_object().unwrap().keys().cloned().collect::<Vec<_>>();
        let props = |v: &Value| {
            v["properties"]
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect::<Vec<_>>()
        };
        assert_eq!(fields(&value), props(&schema));
        let setup = &schema["properties"]["setups"]["items"];
        assert_eq!(fields(&value["setups"][0]), props(setup));
        assert_eq!(
            fields(&value["setups"][0]["operations"][0]),
            props(&setup["properties"]["operations"]["items"])
        );
        assert_eq!(
            setup["properties"]["orientation"]["enum"],
            json!(["+Z", "-Z", "+X", "-X", "+Y", "-Y"])
        );
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
    /// The rotary scenario's project, report and the plan that ends it,
    /// with `edit` applied to the plan.
    fn rotary(edit: impl FnOnce(&mut Proposal)) -> (Document, Mesh, PartReport, Proposal) {
        let (doc, mesh, report) = crate::fake_openai::rotary::report();
        let mut plan = crate::fake_openai::rotary::good();
        edit(&mut plan);
        (doc, mesh, report, plan)
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
        let schema = schema_for(&report);
        let mut objects = 0;
        walk(&schema, &mut objects);
        // The proposal, its chuck, a setup, an operation and an unmachined entry.
        assert_eq!(objects, 5);
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
