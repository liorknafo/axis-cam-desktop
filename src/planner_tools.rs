//! The planner's tools: the functions the model may call within a planner
//! request, one registry for what is offered (`offered`) and how a call is
//! answered (`answer`). Every answer is JSON text for the model and one line
//! for the timeline; a call that cannot be answered gets one sentence for
//! both. The tools read the part report and the part; they change nothing.
use crate::{
    analysis::{self, PartReport, PlaneReport},
    cam::Geometry,
    look, material,
    openai::Tool,
    planner::{MAX_PICTURES_PER_REQUEST, MAX_PICTURES_PER_RUN, MAX_TOOL_ROUNDS},
    proposal::clip,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::json;
use std::sync::Arc;

/// `cutting_data` in requests.
pub const CUTTING_DATA: &str = "cutting_data";
/// `surface_info` in requests.
pub const SURFACE_INFO: &str = "surface_info";
/// The least spindle speed a plan may ask for (see RULES).
const MIN_RPM: f64 = 100.;

/// What a checked draft leaves for the tools that read it. Empty until the
/// tools that check a draft arrive.
#[allow(dead_code)]
pub struct Checked {}

/// What the tools remember within a run.
#[derive(Clone)]
pub struct ToolState {
    /// What the pictures and the part tools read; its `job` is the latest
    /// checked proposal's.
    pub scene: look::Scene,
    /// The latest checked draft.
    #[allow(dead_code)]
    pub checked: Option<Arc<Checked>>,
    /// Drafts checked in this planner request and in the run.
    #[allow(dead_code)]
    pub checks_here: usize,
    #[allow(dead_code)]
    pub checks_run: usize,
    /// Views asked for, drawn or not, in this planner request and in the run.
    pub pictures_here: usize,
    pub pictures_run: usize,
}
impl ToolState {
    pub fn new(scene: look::Scene) -> Self {
        Self {
            scene,
            checked: None,
            checks_here: 0,
            checks_run: 0,
            pictures_here: 0,
            pictures_run: 0,
        }
    }
    /// A planner request starts: its own limits start from zero, the run's go on.
    pub fn next_request(&mut self) {
        self.checks_here = 0;
        self.pictures_here = 0;
    }
}

/// How a call was answered.
pub struct Answer {
    /// What the model reads: JSON text, or one sentence.
    pub output: String,
    /// The timeline's line: what was asked and found, or the same sentence.
    pub line: String,
    /// The picture `view_model` drew; it goes to the model as an image.
    pub picture: Option<look::Picture>,
}
impl Answer {
    fn sentence(text: impl Into<String>) -> Self {
        let text = text.into();
        Self {
            output: text.clone(),
            line: text,
            picture: None,
        }
    }
}
/// The answer to a call whose work was lost: the thread that ran it failed.
pub fn crashed(name: &str) -> Answer {
    Answer::sentence(if name == look::NAME {
        "The picture could not be drawn."
    } else {
        "The tool could not be run."
    })
}

/// The tools the model is offered: `view_model` only when the run shows
/// pictures.
pub fn offered(pictures: bool) -> Vec<Tool> {
    let mut tools = vec![];
    if pictures {
        tools.push(look::tool());
    }
    tools.push(Tool {
        name: CUTTING_DATA,
        description: "The spindle speed and feed a tool can run at in the project's material: the rpm range from the material's surface speed and the machine's maximum, a recommended rpm (the middle of the range, rounded to 10), and at that rpm the feed range from the chip-load band of the tool's diameter and its flutes, clipped to the machine's maximum feed, with a recommended feed and the band used. Feeds are in mm/min.".into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "tool": {"type": "integer", "description": "A tool number from the part report's `tools`."}
            },
            "required": ["tool"],
            "additionalProperties": false
        }),
    });
    tools.push(Tool {
        name: SURFACE_INFO,
        description: "What the part report and the part say of one surface: every orientation whose planes list it (plane kind, depth below top, area, bbox, `max_cutter_diameter`, holes), the surface's own area in mm² and its mean normal in setup 1's frame.".into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "surface": {"type": "integer", "description": "A surface ID from the part report's `planes`."}
            },
            "required": ["surface"],
            "additionalProperties": false
        }),
    });
    tools
}

/// The TOOLS section of the instructions, with its blank line after it: each
/// tool offered with its limits, `view_model` only with `pictures`.
pub fn instructions(pictures: bool) -> String {
    let view = if pictures {
        format!(
            "- `view_model`: a picture of the part, the stock and the holding (see PICTURES). At most {MAX_PICTURES_PER_REQUEST} pictures per reply and {MAX_PICTURES_PER_RUN} in the run.\n"
        )
    } else {
        String::new()
    };
    format!("TOOLS
- Before you answer you may call these functions. A reply of yours with calls is one round, and a reply allows at most {MAX_TOOL_ROUNDS} rounds; after that, reply with the proposal. An answer is JSON text, or one sentence when the call cannot be answered.
{view}- `cutting_data` (`tool`, a number from `tools`): the rpm range from the material's surface speed and the machine's maximum rpm, a recommended rpm, and at that rpm the feed range (mm/min) from the chip-load band of the tool's diameter and its flutes, with a recommended feed and the band. No limit.
- `surface_info` (`surface`, an ID from `planes`): every orientation whose planes list the surface, with the plane's kind, depth, area, bbox, `max_cutter_diameter` and holes, and the surface's own area (`surface_area`, mm²) and `mean_normal` in setup 1's frame. No limit.

")
}

/// Answers the call of tool `name` with the JSON text `arguments`. May take
/// long (a picture is drawn): call it off the async threads.
pub fn answer(name: &str, arguments: &str, state: &mut ToolState) -> Answer {
    match name {
        look::NAME => view(arguments, state),
        CUTTING_DATA => cutting_data(arguments, &state.scene.report),
        SURFACE_INFO => surface_info(arguments, &state.scene),
        other => Answer::sentence(format!("There is no tool named {}.", clip(other, 40))),
    }
}

/// The arguments of a call, or the sentence that says they cannot be read.
fn parse<T: DeserializeOwned>(tool: &str, arguments: &str) -> Result<T, Answer> {
    serde_json::from_str(arguments).map_err(|e| {
        Answer::sentence(clip(
            &format!("The {tool} arguments could not be read: {e}"),
            300,
        ))
    })
}

fn view(arguments: &str, state: &mut ToolState) -> Answer {
    if state.pictures_here >= MAX_PICTURES_PER_REQUEST {
        return Answer::sentence("No more pictures for this request; reply with the proposal.");
    }
    if state.pictures_run >= MAX_PICTURES_PER_RUN {
        return Answer::sentence("No more pictures in this run; reply with the proposal.");
    }
    // A view that cannot be drawn counts against the limits too.
    state.pictures_here += 1;
    state.pictures_run += 1;
    match look::draw(&state.scene, arguments) {
        Ok(picture) => Answer {
            output: picture.description.clone(),
            line: picture.description.clone(),
            picture: Some(picture),
        },
        Err(message) => Answer::sentence(message),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ToolArgs {
    tool: u32,
}
/// A range with the value the tool recommends in it.
#[derive(Serialize)]
struct Range {
    min: u32,
    max: u32,
    recommended: u32,
}
#[derive(Serialize)]
struct Feed {
    /// The spindle speed the feed range is for: the recommended rpm.
    at_rpm: u32,
    min: u32,
    max: u32,
    recommended: u32,
}
/// The chip-load band of the tool's diameter, in mm per tooth.
#[derive(Serialize)]
struct Band {
    /// The largest diameter the band holds; null for every larger one.
    up_to_mm: Option<f32>,
    min: f32,
    max: f32,
}
#[derive(Serialize)]
struct CuttingData {
    tool: u32,
    diameter: f32,
    flutes: u32,
    rpm: Range,
    feed_mm_per_min: Feed,
    chip_load_band: Band,
}
/// `value` rounded to the nearest 10.
fn tens(value: f64) -> f64 {
    (value / 10.).round() * 10.
}
/// 7960 as "7 960".
fn grouped(n: u32) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, digit) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(' ');
        }
        out.push(digit);
    }
    out
}

fn cutting_data(arguments: &str, report: &PartReport) -> Answer {
    let args: ToolArgs = match parse(CUTTING_DATA, arguments) {
        Ok(args) => args,
        Err(answer) => return answer,
    };
    let Some(tool) = report.tools.iter().find(|t| t.number == args.tool) else {
        return Answer::sentence(format!("The part report has no tool number {}.", args.tool));
    };
    let (material, machine) = (&report.material, &report.machine);
    let band = report
        .material
        .chip_load_mm
        .get(material::band(tool.diameter));
    let Some(band) = band.filter(|band| {
        material.vc_min_m_per_min > 0.
            && material.vc_max_m_per_min >= material.vc_min_m_per_min
            && band.min > 0.
            && band.max >= band.min
    }) else {
        return Answer::sentence("The material has no cutting data.");
    };
    if machine.max_rpm <= 0. || machine.max_feed <= 0. {
        return Answer::sentence("The machine has no maximum rpm or feed.");
    }
    if tool.diameter <= 0. || tool.flutes == 0 {
        return Answer::sentence(format!(
            "Tool {} has no diameter or flutes to compute with.",
            tool.number
        ));
    }
    let (diameter, flutes) = (f64::from(tool.diameter), f64::from(tool.flutes));
    // The surface speed range as rpm, up to what the machine gives. When even
    // the maximum is too slow for the least surface speed, it counts as in range.
    let rpm_at = |vc: f32| f64::from(vc) * 1000. / (std::f64::consts::PI * diameter);
    let max_rpm = f64::from(machine.max_rpm);
    let top = rpm_at(material.vc_max_m_per_min).min(max_rpm);
    let bottom = rpm_at(material.vc_min_m_per_min).max(MIN_RPM).min(top);
    // The middle, rounded to 10 while that stays in the range: a value above
    // the machine's maximum, or below the least surface speed at a maximum
    // that cannot reach it, would not pass the check.
    let rpm = tens((bottom + top) / 2.)
        .clamp(bottom, top)
        .round()
        .min(max_rpm.floor());
    // The feed range at that rpm, up to what the machine gives; when even the
    // maximum feed is below the least chip load, it counts as in range.
    let max_feed = f64::from(machine.max_feed);
    let feed_top = (f64::from(band.max) * flutes * rpm).min(max_feed);
    let feed_bottom = (f64::from(band.min) * flutes * rpm).min(feed_top);
    let feed = tens((feed_bottom + feed_top) / 2.)
        .clamp(feed_bottom, feed_top)
        .round()
        .min(max_feed.floor());
    let (rpm, feed) = (rpm as u32, feed as u32);
    let data = CuttingData {
        tool: tool.number,
        diameter: tool.diameter,
        flutes: tool.flutes,
        rpm: Range {
            min: (bottom.ceil() as u32).min(top.floor() as u32),
            max: top.floor() as u32,
            recommended: rpm,
        },
        feed_mm_per_min: Feed {
            at_rpm: rpm,
            min: (feed_bottom.ceil() as u32).min(feed_top.floor() as u32),
            max: feed_top.floor() as u32,
            recommended: feed,
        },
        chip_load_band: Band {
            up_to_mm: band.up_to_mm,
            min: band.min,
            max: band.max,
        },
    };
    Answer {
        output: serde_json::to_string(&data).expect("plain numbers serialize"),
        line: format!(
            "Cutting data for T{}: {} rpm, {} mm/min",
            tool.number,
            grouped(rpm),
            grouped(feed)
        ),
        picture: None,
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SurfaceArgs {
    surface: usize,
}
/// One orientation that lists the surface, with the plane that does.
#[derive(Serialize)]
struct Seen<'a> {
    up: &'static str,
    #[serde(flatten)]
    plane: &'a PlaneReport,
}
#[derive(Serialize)]
struct SurfaceInfo<'a> {
    surface: usize,
    /// This surface's faces alone, mm²; `area` of an entry is its whole plane's.
    surface_area: f64,
    /// The area-weighted mean of the faces' normals in setup 1's frame; null
    /// when they cancel out, as the faces of a closed round surface do.
    mean_normal: Option<[f64; 3]>,
    orientations: Vec<Seen<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<&'static str>,
}
/// `value` without trailing zeros, to two decimals: 12, 7.5, 0.25.
fn plain(value: f32) -> String {
    format!("{value:.2}")
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_string()
}
fn rounded(value: f64, places: i32) -> f64 {
    let scale = 10f64.powi(places);
    // Adding 0 turns a rounded -0.0 into 0.0, so it never prints as "-0.0".
    (value * scale).round() / scale + 0.
}

fn surface_info(arguments: &str, scene: &look::Scene) -> Answer {
    let args: SurfaceArgs = match parse(SURFACE_INFO, arguments) {
        Ok(args) => args,
        Err(answer) => return answer,
    };
    let id = args.surface;
    if id >= scene.mesh.surfaces.len() {
        return Answer::sentence(format!("Surface {id} is not a surface of this part."));
    }
    let seen: Vec<Seen> = scene
        .report
        .orientations
        .iter()
        .flat_map(|o| {
            o.planes
                .iter()
                .filter(move |p| p.surfaces.contains(&id))
                .map(|plane| Seen { up: o.up, plane })
        })
        .collect();
    // The surface's faces in setup 1's frame, in mm.
    let (mut area, mut sum) = (0f64, [0f64; 3]);
    if let Some(setup1) = scene.doc.setups.first() {
        for t in Geometry::new(setup1, &scene.mesh)
            .triangles
            .iter()
            .filter(|t| t.feature == Some(id))
        {
            // Twice the area, along the normal.
            let n = analysis::normal(&t.p).map(f64::from);
            area += n.iter().map(|v| v * v).sum::<f64>().sqrt() / 2.;
            sum = std::array::from_fn(|k| sum[k] + n[k]);
        }
    }
    let length = sum.iter().map(|v| v * v).sum::<f64>().sqrt();
    let mean_normal =
        (length > 1e-3 * 2. * area && length > 1e-9).then(|| sum.map(|v| rounded(v / length, 3)));
    let area = rounded(area, 2);
    let line = if seen.is_empty() {
        format!("Surface {id}: in no orientation's plane list ({area:.1} mm²)")
    } else {
        let places: Vec<String> = seen
            .iter()
            .map(|s| match s.plane.kind {
                "top" => format!("top in {}", s.up),
                kind => format!(
                    "{kind} {} mm deep in {}",
                    plain(s.plane.depth_below_top),
                    s.up
                ),
            })
            .collect();
        format!("Surface {id}: {}", places.join("; "))
    };
    let note = seen.is_empty().then_some(
        "No orientation lists this surface as a horizontal face: it is a wall, a slope, hidden from above, or beyond the report's plane list.",
    );
    let info = SurfaceInfo {
        surface: id,
        surface_area: area,
        mean_normal,
        orientations: seen,
        note,
    };
    Answer {
        output: serde_json::to_string(&info).expect("plain numbers serialize"),
        line,
        picture: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        analysis::{self, PartReport, PlaneReport},
        proposal::tests::block,
    };
    use serde_json::{Value, json};
    use std::sync::Arc;

    /// The example block's scene, with `edit` applied to its part report.
    fn state_with(edit: impl FnOnce(&mut PartReport)) -> ToolState {
        let (doc, mesh, mut report) = block();
        edit(&mut report);
        ToolState::new(look::Scene {
            doc: Arc::new(doc),
            mesh,
            report: Arc::new(report),
            job: None,
        })
    }
    fn state() -> ToolState {
        state_with(|_| {})
    }
    fn call(name: &str, arguments: Value, state: &mut ToolState) -> Answer {
        answer(name, &arguments.to_string(), state)
    }
    fn json_of(answer: &Answer) -> Value {
        serde_json::from_str(&answer.output).unwrap_or_else(|e| panic!("{e}: {}", answer.output))
    }
    /// `value` as the tool writes it: f32 numbers in their shortest form.
    fn as_json<T: serde::Serialize>(value: &T) -> Value {
        serde_json::from_str(&serde_json::to_string(value).unwrap()).unwrap()
    }
    fn near(value: &Value, expected: f64, tolerance: f64) {
        let v = value
            .as_f64()
            .unwrap_or_else(|| panic!("{value} is no number"));
        assert!((v - expected).abs() <= tolerance, "{v} is not {expected}");
    }
    /// A plane of `up`'s orientation: the first of `kind`.
    fn plane<'a>(report: &'a PartReport, up: &str, kind: &str) -> &'a PlaneReport {
        report
            .orientations
            .iter()
            .find(|o| o.up == up)
            .unwrap()
            .planes
            .iter()
            .find(|p| p.kind == kind)
            .unwrap()
    }
    /// The arguments of a `view_model` call at the whole part.
    fn view(focus: &[usize]) -> Value {
        json!({
            "orientation": null, "angle": "top", "focus_surfaces": focus, "highlight_surfaces": [],
            "show_stock": true, "show_workholding": true, "labels": false, "operation": null,
            "purpose": "check"
        })
    }
    fn walk(value: &Value, objects: &mut usize) {
        if value["type"] == "object" {
            *objects += 1;
            assert_eq!(value["additionalProperties"], false, "{value}");
            let mut keys: Vec<&String> = value["properties"].as_object().unwrap().keys().collect();
            let mut required: Vec<&str> = value["required"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_str().unwrap())
                .collect();
            keys.sort();
            required.sort();
            assert_eq!(keys, required, "every property is required");
        }
        match value {
            Value::Object(map) => map.values().for_each(|v| walk(v, objects)),
            Value::Array(items) => items.iter().for_each(|v| walk(v, objects)),
            _ => {}
        }
    }

    #[test]
    fn the_tools_are_offered_with_view_model_only_when_pictures_are_on() {
        let names =
            |pictures| -> Vec<&'static str> { offered(pictures).iter().map(|t| t.name).collect() };
        assert_eq!(
            names(true),
            [look::NAME, "cutting_data", "surface_info"],
            "view_model first"
        );
        assert_eq!(names(false), ["cutting_data", "surface_info"]);
        for tool in offered(true) {
            assert!(!tool.description.is_empty(), "{}", tool.name);
            let mut objects = 0;
            walk(&tool.parameters, &mut objects);
            assert!(objects >= 1, "{} is a strict object", tool.name);
        }
        let by = |name| offered(false).into_iter().find(|t| t.name == name).unwrap();
        assert_eq!(
            by("cutting_data").parameters["properties"]["tool"]["type"],
            "integer"
        );
        assert_eq!(
            by("surface_info").parameters["properties"]["surface"]["type"],
            "integer"
        );
    }

    #[test]
    fn cutting_data_for_the_drill_clips_the_rpm_to_the_machine_and_recommends_the_middle() {
        let mut s = state();
        let report = s.scene.report.clone();
        let t3 = report.tools.iter().find(|t| t.number == 3).unwrap();
        assert_eq!(
            (
                t3.diameter,
                t3.flutes,
                report.machine.max_rpm,
                report.machine.max_feed
            ),
            (4., 3, 12000., 5000.)
        );
        assert_eq!(
            (
                report.material.vc_min_m_per_min,
                report.material.vc_max_m_per_min
            ),
            (150., 300.)
        );
        let a = call("cutting_data", json!({"tool": 3}), &mut s);
        assert!(a.picture.is_none());
        let out = json_of(&a);
        assert_eq!((&out["tool"], &out["flutes"]), (&json!(3), &json!(3)));
        near(&out["diameter"], 4., 0.);
        // 150-300 m/min on a Ø4 tool is 11 937-23 873 rpm; the machine stops at 12 000.
        near(&out["rpm"]["min"], 11937., 1.);
        near(&out["rpm"]["max"], 12000., 0.);
        // The middle of the clipped range, 11 968.7, rounded to 10.
        assert_eq!(out["rpm"]["recommended"], 11970);
        // The chip load of the band up to 6 mm, 0.020-0.050 mm per tooth,
        // with 3 flutes at the recommended rpm.
        assert_eq!(out["feed_mm_per_min"]["at_rpm"], 11970);
        near(&out["feed_mm_per_min"]["min"], 0.02 * 3. * 11970., 1.);
        near(&out["feed_mm_per_min"]["max"], 0.05 * 3. * 11970., 1.);
        // The middle, 1 256.9, rounded to 10.
        assert_eq!(out["feed_mm_per_min"]["recommended"], 1260);
        near(&out["chip_load_band"]["up_to_mm"], 6., 0.);
        near(&out["chip_load_band"]["min"], 0.02, 1e-6);
        near(&out["chip_load_band"]["max"], 0.05, 1e-6);
        assert_eq!(a.line, "Cutting data for T3: 11 970 rpm, 1 260 mm/min");
    }

    #[test]
    fn cutting_data_reads_the_band_and_the_range_of_each_tools_diameter() {
        let mut s = state();
        // T1 is a Ø10 flat mill: the band up to 10 mm, 0.030-0.080.
        let out = json_of(&call("cutting_data", json!({"tool": 1}), &mut s));
        near(&out["rpm"]["min"], 4775., 1.);
        near(&out["rpm"]["max"], 9549., 1.);
        assert_eq!(out["rpm"]["recommended"], 7160);
        near(&out["feed_mm_per_min"]["min"], 0.03 * 3. * 7160., 1.);
        near(&out["feed_mm_per_min"]["max"], 0.08 * 3. * 7160., 1.);
        assert_eq!(out["feed_mm_per_min"]["recommended"], 1180);
        near(&out["chip_load_band"]["up_to_mm"], 10., 0.);
        // A tool over 16 mm is in the last band, which has no upper diameter.
        let mut big = state_with(|r| r.tools[0].diameter = 20.);
        let out = json_of(&call("cutting_data", json!({"tool": 1}), &mut big));
        assert_eq!(out["chip_load_band"]["up_to_mm"], Value::Null);
        near(&out["chip_load_band"]["min"], 0.06, 1e-6);
        near(&out["chip_load_band"]["max"], 0.12, 1e-6);
        near(&out["rpm"]["min"], 2387., 1.);
    }

    #[test]
    fn cutting_data_clips_the_feed_to_the_machine_maximum() {
        let mut s = state_with(|r| r.machine.max_feed = 1000.);
        let a = call("cutting_data", json!({"tool": 3}), &mut s);
        let out = json_of(&a);
        near(&out["feed_mm_per_min"]["min"], 718., 1.);
        near(&out["feed_mm_per_min"]["max"], 1000., 0.);
        // The middle of 718-1 000, 859, rounded to 10.
        assert_eq!(out["feed_mm_per_min"]["recommended"], 860);
        assert_eq!(a.line, "Cutting data for T3: 11 970 rpm, 860 mm/min");
        // When even the least chip load needs more, the maximum counts.
        let mut s = state_with(|r| r.machine.max_feed = 500.);
        let out = json_of(&call("cutting_data", json!({"tool": 3}), &mut s));
        near(&out["feed_mm_per_min"]["min"], 500., 0.);
        near(&out["feed_mm_per_min"]["max"], 500., 0.);
        assert_eq!(out["feed_mm_per_min"]["recommended"], 500);
    }

    #[test]
    fn cutting_data_runs_at_the_machines_maximum_when_the_material_wants_more() {
        // 150 m/min on a Ø4 tool needs 11 937 rpm; the machine gives 6 000.
        let mut s = state_with(|r| r.machine.max_rpm = 6000.);
        let a = call("cutting_data", json!({"tool": 3}), &mut s);
        let out = json_of(&a);
        near(&out["rpm"]["min"], 6000., 0.);
        near(&out["rpm"]["max"], 6000., 0.);
        assert_eq!(out["rpm"]["recommended"], 6000);
        near(&out["feed_mm_per_min"]["min"], 0.02 * 3. * 6000., 1.);
        near(&out["feed_mm_per_min"]["max"], 0.05 * 3. * 6000., 1.);
        assert_eq!(out["feed_mm_per_min"]["recommended"], 630);
        assert_eq!(a.line, "Cutting data for T3: 6 000 rpm, 630 mm/min");
        // Where rounding to 10 would leave the range, the recommendation
        // stays in it: 6 000 would run below the least surface speed with
        // the machine able to give 6 004, which the check refuses.
        let mut s = state_with(|r| r.machine.max_rpm = 6004.);
        let out = json_of(&call("cutting_data", json!({"tool": 3}), &mut s));
        assert_eq!(out["rpm"]["recommended"], 6004);
    }

    #[test]
    fn cutting_data_answers_a_sentence_for_what_it_cannot_tell() {
        let mut s = state();
        let a = call("cutting_data", json!({"tool": 9}), &mut s);
        assert_eq!(a.output, "The part report has no tool number 9.");
        assert_eq!(
            (a.line.as_str(), a.picture.is_none()),
            (a.output.as_str(), true)
        );
        for arguments in [
            "{}",
            "not json",
            r#"{"tool": "3"}"#,
            r#"{"tool": -1}"#,
            r#"{"tool": 3, "extra": 1}"#,
        ] {
            let a = answer("cutting_data", arguments, &mut s);
            assert!(
                a.output
                    .starts_with("The cutting_data arguments could not be read: "),
                "{arguments}: {}",
                a.output
            );
            assert!(a.output.chars().count() <= 300);
            assert_eq!(a.line, a.output);
        }
        let mut no_bands = state_with(|r| r.material.chip_load_mm.clear());
        let a = call("cutting_data", json!({"tool": 3}), &mut no_bands);
        assert_eq!(a.output, "The material has no cutting data.");
        let mut no_speed = state_with(|r| r.material.vc_max_m_per_min = 0.);
        let a = call("cutting_data", json!({"tool": 3}), &mut no_speed);
        assert_eq!(a.output, "The material has no cutting data.");
        let mut no_machine = state_with(|r| r.machine.max_rpm = 0.);
        let a = call("cutting_data", json!({"tool": 3}), &mut no_machine);
        assert_eq!(a.output, "The machine has no maximum rpm or feed.");
    }

    /// The area (mm²) of the mesh faces of surface `id`, from the mesh itself.
    fn mesh_area(s: &ToolState, id: usize) -> f64 {
        let scale = f64::from(s.scene.mesh.display_scale());
        s.scene
            .mesh
            .faces
            .iter()
            .filter(|f| f.feature == Some(id))
            .map(|f| {
                let p = |i: usize| f.points[i].map(f64::from);
                let (a, b) = (
                    std::array::from_fn::<f64, 3, _>(|k| p(1)[k] - p(0)[k]),
                    std::array::from_fn::<f64, 3, _>(|k| p(2)[k] - p(0)[k]),
                );
                let n = [
                    a[1] * b[2] - a[2] * b[1],
                    a[2] * b[0] - a[0] * b[2],
                    a[0] * b[1] - a[1] * b[0],
                ];
                (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt() / 2.
            })
            .sum::<f64>()
            / (scale * scale)
    }

    #[test]
    fn surface_info_of_the_top_lists_its_plane_and_the_surfaces_own_area_and_normal() {
        let mut s = state();
        let report = s.scene.report.clone();
        let top = plane(&report, "+Z", "top");
        let id = top.surfaces[0];
        let a = call("surface_info", json!({"surface": id}), &mut s);
        assert!(a.picture.is_none());
        let out = json_of(&a);
        assert_eq!(out["surface"], id);
        let seen: Vec<&Value> = out["orientations"].as_array().unwrap().iter().collect();
        let listed: Vec<&str> = report
            .orientations
            .iter()
            .filter(|o| o.planes.iter().any(|p| p.surfaces.contains(&id)))
            .map(|o| o.up)
            .collect();
        assert_eq!(
            seen.iter()
                .map(|o| o["up"].as_str().unwrap())
                .collect::<Vec<_>>(),
            listed,
            "every orientation that sees it, in the report's order"
        );
        let entry = seen[0];
        assert_eq!(entry["up"], "+Z");
        assert_eq!(entry["kind"], "top");
        assert_eq!(entry["surfaces"], json!(top.surfaces));
        near(&entry["depth_below_top"], 0., 0.);
        near(&entry["area"], f64::from(top.area), 0.01);
        assert_eq!(entry["bbox"], as_json(&top.bbox));
        assert_eq!(entry["max_cutter_diameter"], Value::Null);
        assert_eq!(entry["holes"], as_json(&top.holes));
        // The surface's own area, from the mesh, and its normal in setup 1's frame.
        near(&out["surface_area"], mesh_area(&s, id), 0.05);
        let normal = out["mean_normal"].as_array().unwrap();
        near(&normal[0], 0., 1e-3);
        near(&normal[1], 0., 1e-3);
        near(&normal[2], 1., 1e-3);
        assert_eq!(a.line, format!("Surface {id}: top in +Z"));
    }

    #[test]
    fn surface_info_of_a_floor_says_how_deep_it_is_and_what_cutter_fits() {
        let mut s = state();
        let report = s.scene.report.clone();
        let floor = plane(&report, "+Z", "floor");
        let id = floor.surfaces[0];
        let a = call("surface_info", json!({"surface": id}), &mut s);
        let out = json_of(&a);
        let entry = &out["orientations"][0];
        assert_eq!(
            (entry["up"].clone(), entry["kind"].clone()),
            (json!("+Z"), json!("floor"))
        );
        near(
            &entry["depth_below_top"],
            f64::from(floor.depth_below_top),
            1e-5,
        );
        near(
            &entry["max_cutter_diameter"],
            f64::from(floor.max_cutter_diameter.unwrap()),
            1e-5,
        );
        let depth = floor.depth_below_top;
        assert_eq!(
            depth.fract(),
            0.,
            "the fixture's floors are whole millimetres deep"
        );
        assert_eq!(a.line, format!("Surface {id}: floor {depth} mm deep in +Z"));
    }

    #[test]
    fn surface_info_lists_every_orientation_whose_planes_name_the_surface() {
        // The same surface also listed in the second orientation, 5 mm down.
        let mut s = state_with(|r| {
            let mut copy = r.orientations[0].planes[0].clone();
            copy.kind = "floor";
            copy.depth_below_top = 5.;
            r.orientations[1].planes.push(copy);
        });
        let report = s.scene.report.clone();
        let id = report.orientations[0].planes[0].surfaces[0];
        let a = call("surface_info", json!({"surface": id}), &mut s);
        let out = json_of(&a);
        let seen = out["orientations"].as_array().unwrap();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0]["up"], report.orientations[0].up);
        assert_eq!(seen[1]["up"], report.orientations[1].up);
        assert_eq!(seen[1]["kind"], "floor");
        near(&seen[1]["depth_below_top"], 5., 0.);
        let (first, second) = (report.orientations[0].up, report.orientations[1].up);
        let head = format!("Surface {id}: ");
        assert!(a.line.starts_with(&head) && a.line.contains(&format!(" in {first}; ")));
        assert!(
            a.line.ends_with(&format!("; floor 5 mm deep in {second}")),
            "{}",
            a.line
        );
    }

    #[test]
    fn surface_info_gives_the_normal_in_setup_1s_frame_not_the_models() {
        let (mut doc, mesh, _) = block();
        doc.setups[0].p.insert("rx".into(), "180".into());
        let report = analysis::report(&doc, &mesh).unwrap();
        assert_eq!(report.orientations[0].up, "-Z");
        let mut s = ToolState::new(look::Scene {
            doc: Arc::new(doc),
            mesh,
            report: Arc::new(report),
            job: None,
        });
        // A surface facing up in the model faces down with setup 1 turned over.
        let id = s
            .scene
            .mesh
            .surfaces
            .iter()
            .position(|surface| surface.normal[2] > 0.999)
            .unwrap();
        let out = json_of(&call("surface_info", json!({"surface": id}), &mut s));
        let normal = out["mean_normal"].as_array().unwrap();
        near(&normal[2], -1., 1e-3);
        near(&out["surface_area"], mesh_area(&s, id), 0.05);
    }

    #[test]
    fn a_surface_no_plane_lists_still_has_its_area_and_normal() {
        let mut s = state();
        let report = s.scene.report.clone();
        let wall = (0..s.scene.mesh.surfaces.len())
            .find(|id| {
                s.scene.mesh.surfaces[*id].normal[2].abs() < 0.01
                    && report
                        .orientations
                        .iter()
                        .all(|o| o.planes.iter().all(|p| !p.surfaces.contains(id)))
            })
            .expect("the block has walls");
        let a = call("surface_info", json!({"surface": wall}), &mut s);
        let out = json_of(&a);
        assert_eq!(out["orientations"], json!([]));
        assert!(out["note"].as_str().unwrap().contains("wall"), "{out}");
        let area = mesh_area(&s, wall);
        near(&out["surface_area"], area, 0.05);
        let normal = out["mean_normal"].as_array().unwrap();
        near(&normal[2], 0., 1e-3);
        let length: f64 = normal.iter().map(|v| v.as_f64().unwrap().powi(2)).sum();
        near(&json!(length), 1., 1e-2);
        assert_eq!(
            a.line,
            format!("Surface {wall}: in no orientation's plane list ({area:.1} mm²)")
        );
    }

    #[test]
    fn surface_info_answers_a_sentence_for_an_unknown_surface_or_unreadable_arguments() {
        let mut s = state();
        let count = s.scene.mesh.surfaces.len();
        for id in [99999, count] {
            let a = call("surface_info", json!({"surface": id}), &mut s);
            assert_eq!(
                a.output,
                format!("Surface {id} is not a surface of this part.")
            );
            assert_eq!(a.line, a.output);
            assert!(a.picture.is_none());
        }
        let last = call("surface_info", json!({"surface": count - 1}), &mut s);
        assert!(last.output.starts_with('{'), "{}", last.output);
        for arguments in [
            "{}",
            "[]",
            r#"{"surface": -1}"#,
            r#"{"surface": 1.5}"#,
            r#"{"surface": 1, "x": 2}"#,
        ] {
            let a = answer("surface_info", arguments, &mut s);
            assert!(
                a.output
                    .starts_with("The surface_info arguments could not be read: "),
                "{arguments}: {}",
                a.output
            );
        }
    }

    #[test]
    fn view_model_goes_through_the_registry_with_its_picture_limits() {
        let mut s = state();
        let a = call(look::NAME, view(&[]), &mut s);
        let picture = a.picture.as_ref().expect("a picture");
        assert_eq!(a.output, picture.description);
        assert_eq!(a.line, picture.description);
        assert_eq!((s.pictures_here, s.pictures_run), (1, 1));
        // A view that cannot be drawn is a sentence and counts too.
        let a = call(look::NAME, view(&[99999]), &mut s);
        assert!(a.picture.is_none());
        assert_eq!(a.output, "Surface 99999 is not a surface of this part.");
        assert_eq!(a.line, a.output);
        assert_eq!((s.pictures_here, s.pictures_run), (2, 2));
        // The request's limit, then the run's; refused calls are not counted.
        s.pictures_here = crate::planner::MAX_PICTURES_PER_REQUEST;
        let a = call(look::NAME, view(&[]), &mut s);
        assert_eq!(
            a.output,
            "No more pictures for this request; reply with the proposal."
        );
        assert!(a.picture.is_none());
        assert_eq!(s.pictures_run, 2);
        s.next_request();
        assert_eq!(
            (s.pictures_here, s.pictures_run),
            (0, 2),
            "a new request may look again"
        );
        s.pictures_run = crate::planner::MAX_PICTURES_PER_RUN;
        let a = call(look::NAME, view(&[]), &mut s);
        assert_eq!(
            a.output,
            "No more pictures in this run; reply with the proposal."
        );
        assert_eq!(
            (s.pictures_here, s.pictures_run),
            (0, crate::planner::MAX_PICTURES_PER_RUN)
        );
        // The limits are the picture tool's alone.
        let a = call("cutting_data", json!({"tool": 1}), &mut s);
        assert!(a.output.starts_with('{'));
    }

    #[test]
    fn an_unknown_tool_is_answered_with_a_sentence() {
        let mut s = state();
        let a = answer("make_coffee", "{}", &mut s);
        assert_eq!(a.output, "There is no tool named make_coffee.");
        assert_eq!(a.line, a.output);
        assert!(a.picture.is_none());
        let long = "x".repeat(100);
        let a = answer(&long, "{}", &mut s);
        assert!(a.output.chars().count() < 70, "{}", a.output);
        assert!(a.output.starts_with("There is no tool named xxx"));
    }

    #[test]
    fn answers_carry_no_holder_name_note_project_title_or_setup_name() {
        let (mut doc, mesh, _) = block();
        crate::proposal::tests::hold(&mut doc, "Vise", &[("vise_axis", "Y"), ("vise_grip", "8")]);
        for holder in &mut doc.workholding {
            holder.p.insert("name".into(), "ZZ-SECRET-HOLDER".into());
            holder.p.insert("notes".into(), "ZZ-SECRET-NOTE".into());
        }
        doc.title = "ZZ-SECRET-TITLE".into();
        doc.setups[0]
            .p
            .insert("name".into(), "ZZ-SECRET-SETUP".into());
        doc.setups[0]
            .p
            .insert("notes".into(), "ZZ-SECRET-SETUP-NOTE".into());
        let report = analysis::report(&doc, &mesh).unwrap();
        let surfaces = mesh.surfaces.len();
        let mut s = ToolState::new(look::Scene {
            doc: Arc::new(doc),
            mesh,
            report: Arc::new(report),
            job: None,
        });
        let mut all = String::new();
        for tool in 0..5 {
            let a = call("cutting_data", json!({"tool": tool}), &mut s);
            all += &format!("{} {}\n", a.output, a.line);
        }
        for surface in 0..=surfaces {
            let a = call("surface_info", json!({"surface": surface}), &mut s);
            all += &format!("{} {}\n", a.output, a.line);
        }
        assert!(all.len() > 1000, "the answers were collected");
        assert!(!all.contains("ZZ-SECRET"), "{all}");
    }
}
