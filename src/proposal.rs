//! The AI planner's proposal: the format the model must return, and how a
//! proposal becomes a job. Only IDs, enumerated choices and numbers reach the
//! job; the model's free text is displayed or stored as notes.
use crate::{
    analysis::{self, OrientationReport, PartReport, PlaneReport},
    mesh::Mesh,
    model::{Document, Operation, Setup, Strategy, Tool, get, num},
    surface::Selection,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const ORIGINS: [&str; 3] = ["Stock top center", "Stock top front-left", "Model origin"];
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
const MAX_SETUPS: usize = 6;
pub const MAX_SETUP_OPERATIONS: usize = 12;
pub const MAX_OPERATIONS: usize = 40;
const MAX_SURFACES: usize = 500;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Proposal {
    pub summary: String,
    pub setups: Vec<ProposedSetup>,
    pub unmachined: Vec<Unmachined>,
    pub warnings: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProposedSetup {
    pub orientation: String,
    pub name: String,
    pub origin: String,
    pub workholding: String,
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
/// Strict JSON Schema for a proposal: every field required, nothing extra.
pub fn schema() -> Value {
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
    let setup = object(json!({
        "orientation": choice(&ups, false),
        "name": text(),
        "origin": choice(&ORIGINS, false),
        "workholding": text(),
        "operations": {"type": "array", "items": operation},
    }));
    object(json!({
        "summary": text(),
        "setups": {"type": "array", "items": setup},
        "unmachined": {"type": "array", "items": object(json!({"feature": text(), "reason": text()}))},
        "warnings": {"type": "array", "items": text()},
    }))
}
pub fn parse(text: &str) -> Result<Proposal, String> {
    serde_json::from_str(text.trim())
        .map_err(|e| format!("The proposal is not valid JSON for the schema: {e}"))
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
pub(crate) fn clip(text: &str, limit: usize) -> String {
    let plain: String = text
        .chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .filter(|c| !c.is_control() && !format_character(*c))
        .collect();
    plain
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(limit)
        .collect()
}
/// The plane of `orientation` that contains every listed surface.
fn plane_of<'a>(orientation: &'a OrientationReport, surfaces: &[usize]) -> Option<&'a PlaneReport> {
    orientation
        .planes
        .iter()
        .find(|p| !surfaces.is_empty() && surfaces.iter().all(|id| p.surfaces.contains(id)))
}
/// Checks an operation of setup `si` (from 0) and builds it. Returns what is
/// wrong instead when the tool, surfaces or choices do not fit the strategy.
fn operation(
    p: &ProposedOp,
    si: usize,
    orientation: &OrientationReport,
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
        Strategy::Drill => kind == "Drill",
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
        Strategy::Face if !plane.is_some_and(|pl| pl.kind == "top") => wrong.push(format!(
            "Facing needs surfaces of the top plane in orientation {}.",
            orientation.up
        )),
        Strategy::Drill if !plane.is_some_and(|pl| !pl.holes.is_empty()) => wrong.push(format!(
            "Drilling needs the surfaces of one plane with holes in orientation {}.",
            orientation.up
        )),
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
    if proposal
        .setups
        .first()
        .is_some_and(|s| Some(s.orientation.as_str()) != first)
    {
        whole(format!(
            "The first setup must use orientation {}, where the stock is defined.",
            first.unwrap_or("?")
        ));
    }
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
        let Some(orientation) = report
            .orientations
            .iter()
            .find(|o| o.up == proposed.orientation)
        else {
            wrong(
                None,
                format!("\"{}\" is not an orientation.", proposed.orientation),
            );
            continue;
        };
        if proposal.setups[..si]
            .iter()
            .any(|s| s.orientation == proposed.orientation)
        {
            wrong(
                None,
                format!(
                    "Orientation {} is used by an earlier setup.",
                    proposed.orientation
                ),
            );
        }
        if si > 0 && !ORIGINS.contains(&proposed.origin.as_str()) {
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
            setup1.clone()
        } else {
            let offset = (54..=59)
                .map(|n| format!("G{n}"))
                .find(|o| !setups.iter().any(|s| get(&s.p, "offset") == o))
                .unwrap_or_default();
            let mut setup = Setup::new(setup1.machine.clone(), &offset);
            let rotation = analysis::rotation_for(setup1, orientation.up).unwrap_or_default();
            for (key, value) in ["rx", "ry", "rz"].iter().zip(rotation) {
                setup.p.insert((*key).into(), value.to_string());
            }
            setup.p.insert("origin".into(), proposed.origin.clone());
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
            match operation(op, si, orientation, report, &doc.tools, &mesh.fingerprint) {
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
    built.sync_stock_sources();
    Ok(built)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::post::fixture;
    pub fn op(strategy: &str, tool: u32, surfaces: Vec<usize>) -> ProposedOp {
        ProposedOp {
            strategy: strategy.into(),
            name: format!("{strategy} by planner"),
            tool,
            surfaces,
            rpm: if strategy == "Drill" { 2400. } else { 8000. },
            feed: if strategy == "Drill" { 280. } else { 1000. },
            stepdown: None,
            stepover_percent: None,
            radial_allowance: None,
            axial_allowance: None,
            bottom_offset: None,
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
    /// The fixture job with its part report: tools T1 flat Ø10, T2 ball Ø6, T3 drill Ø4.
    pub fn block() -> (Document, Mesh, PartReport) {
        let (doc, mesh) = fixture::job();
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
            setups: vec![
                ProposedSetup {
                    orientation: "+Z".into(),
                    name: "Top".into(),
                    origin: "Stock top center".into(),
                    workholding: "Vise on the long sides.".into(),
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
            "Pocket needs the surfaces of one floor plane",
        );
        refused(
            &|p| p.setups[0].operations[1].surfaces = p.setups[1].operations[1].surfaces.clone(),
            "1/Some(2): Pocket needs the surfaces of one floor plane in orientation +Z",
        );
        refused(
            &|p| p.setups[0].operations[1].surfaces = ids(&report, "+Z", 6.),
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
            "Pocket needs the surfaces of one floor plane",
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
}
