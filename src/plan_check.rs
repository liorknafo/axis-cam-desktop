//! Checks an AI proposal with the real generators, the stock simulation, the
//! export checks, the speeds and feeds against the material's cutting data and
//! the tool's centre line against the mesh, then measures what the plan leaves
//! uncut. The coarse gouge and coverage numbers come from the stock grid and
//! are approximate.
use crate::{
    analysis::{self, Index, PartReport},
    cam::Geometry,
    material::SpeedFeed,
    mesh::{Bounds, Mesh},
    model::Document,
    proposal::{self, Problem, Proposal},
    simulation::{Motion, Plan, Simulation},
    volume::Volume,
};
use serde::Serialize;

/// Removed part cells below this count are grid noise, not a gouge.
const GOUGE_CELLS: usize = 3;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CheckReport {
    pub passed: bool,
    pub problems: Vec<Problem>,
    pub operations: Vec<OpResult>,
    pub coverage: Option<Coverage>,
    pub total_minutes: f32,
    /// Per setup, for a Model-origin zero that passed the export stage: the
    /// part's box and the stock top in work coordinates (mm), which locate it
    /// on the preview; the stock top is None when an earlier setup has cut it
    /// (`post::model_zero`). Not sent to the model.
    #[serde(skip)]
    pub zeros: Vec<Option<(Bounds, Option<f32>)>>,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OpResult {
    pub setup: usize,
    pub operation: usize,
    pub name: String,
    pub minutes: f32,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Coverage {
    /// Share of the part's surface with no stock left against it.
    pub exposed_percent: f32,
    /// Report planes still more than half covered by stock.
    pub covered_planes: Vec<CoveredPlane>,
    /// Other surface (walls, freeform) still covered, by the direction it faces.
    pub covered_other: Vec<CoveredSide>,
    pub leftover_cm3: Leftover,
    pub note: &'static str,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CoveredPlane {
    pub orientation: &'static str,
    pub surfaces: Vec<usize>,
    pub depth_below_top: f32,
    /// The plane's area in the part report (mm²).
    pub area: f32,
    pub covered_percent: f32,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CoveredSide {
    pub facing: &'static str,
    pub area: f32,
    pub covered_percent: f32,
}
/// Stock outside the part, by where it sits relative to the part's bounding box.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Leftover {
    pub above: f32,
    pub around: f32,
    pub inside_bounding_box: f32,
    pub below: f32,
}
/// The part on the stock grid: which cells have their centre inside it.
pub struct PartGrid {
    inside: Vec<bool>,
    /// Inside, with all six neighbours inside too.
    deep: Vec<bool>,
}
impl PartGrid {
    /// `geometry` and `stock` must both be in setup 1's frame.
    pub fn new(stock: &Volume, geometry: &Geometry) -> Self {
        let index = Index::new(geometry);
        let mut inside = vec![false; stock.occupied.len()];
        for y in 0..stock.n[1] {
            for x in 0..stock.n[0] {
                let p = stock.point([x, y, 0], 0.5);
                // A tiny offset keeps the ray off mesh edges that sit on cell centres.
                let xy = [p[0] + 1e-4, p[1] + 1e-4];
                if (0..2).any(|k| xy[k] < geometry.bounds.min[k] || xy[k] > geometry.bounds.max[k])
                {
                    continue;
                }
                let spans = analysis::solid(&index, xy, geometry.bounds.max[2]);
                for z in 0..stock.n[2] {
                    let height = stock.point([x, y, z], 0.5)[2];
                    if spans.iter().any(|(a, b)| height > *a && height < *b) {
                        inside[stock.index([x, y, z])] = true;
                    }
                }
            }
        }
        let mut deep = vec![false; inside.len()];
        for z in 1..stock.n[2].saturating_sub(1) {
            for y in 1..stock.n[1].saturating_sub(1) {
                for x in 1..stock.n[0].saturating_sub(1) {
                    let neighbours = [
                        [x - 1, y, z],
                        [x + 1, y, z],
                        [x, y - 1, z],
                        [x, y + 1, z],
                        [x, y, z - 1],
                        [x, y, z + 1],
                    ];
                    deep[stock.index([x, y, z])] = inside[stock.index([x, y, z])]
                        && neighbours.iter().all(|p| inside[stock.index(*p)]);
                }
            }
        }
        Self { inside, deep }
    }
    /// Cells well inside the part that the simulation has removed, and their bounds.
    pub fn gouged(&self, stock: &Volume) -> (usize, [[f32; 3]; 2]) {
        let mut count = 0;
        let mut bounds = [[f32::INFINITY; 3], [f32::NEG_INFINITY; 3]];
        for z in 0..stock.n[2] {
            for y in 0..stock.n[1] {
                for x in 0..stock.n[0] {
                    let i = stock.index([x, y, z]);
                    if self.deep[i] && stock.occupied[i] == 0 {
                        count += 1;
                        let p = stock.point([x, y, z], 0.5);
                        for k in 0..3 {
                            bounds[0][k] = bounds[0][k].min(p[k]);
                            bounds[1][k] = bounds[1][k].max(p[k]);
                        }
                    }
                }
            }
        }
        (count, bounds)
    }
}
/// Whether an operation that takes the gouged cells from `before` to `after`
/// (all operations so far) is flagged.
fn gouge_added(before: usize, after: usize) -> bool {
    after >= GOUGE_CELLS && after > before
}
fn volume_text(mm3: f32) -> String {
    if mm3 < 1000. {
        format!("{mm3:.0} mm3")
    } else {
        format!("{:.1} cm3", mm3 / 1000.)
    }
}
/// Along every move (tips sampled every 1 mm of XY travel): the most part
/// material directly above the tool tip and where, and the lowest tip height.
/// `index` is in the plan's setup frame and `top` is the model's top there.
fn tool_axis(plan: &Plan, index: &Index, top: f32) -> (f32, [f32; 3], f32) {
    let (mut worst, mut at, mut lowest) = (0f32, [0f32; 3], f32::INFINITY);
    for m in plan.moves.iter().filter(|m| m.motion != Motion::Dwell) {
        lowest = lowest.min(m.from[2]).min(m.to[2]);
        let travel = (m.to[0] - m.from[0]).hypot(m.to[1] - m.from[1]);
        let steps = (travel.ceil() as usize).clamp(1, 1999);
        for i in 0..=steps {
            let t = i as f32 / steps as f32;
            let p: [f32; 3] = std::array::from_fn(|k| m.from[k] + (m.to[k] - m.from[k]) * t);
            // The same tiny offset as the part grid keeps the ray off mesh edges.
            let spans = analysis::solid(index, [p[0] + 1e-4, p[1] + 1e-4], top);
            let above = spans
                .iter()
                .filter(|(_, hi)| *hi > p[2])
                .map(|(lo, hi)| hi - lo.max(p[2]))
                .fold(0., f32::max);
            if above > worst {
                (worst, at) = (above, p);
            }
        }
    }
    (worst, at, lowest)
}
/// The tool axis problems of one generated operation in setup `si` (from 0):
/// more than 0.1 mm of part above the tool tip, whatever the strategy, and
/// moves below the floor. The floor is the stock's bottom in setup 1; later
/// setups rest on the part, whose underside may be all that is left there.
fn axis_problems(plan: &Plan, geometry: &Geometry, index: &Index, si: usize) -> Vec<String> {
    let b = geometry.bounds;
    let (worst, at, lowest) = tool_axis(plan, index, b.max[2]);
    let mut problems = vec![];
    if worst > 0.1 {
        problems.push(format!(
            "The tool's centre line passes through {} mm of the part at X {:.1}, Y {:.1}, {:.1} mm below the model top (this setup's axes, measured from the model's -X/-Y corner): the cut is deeper than the surface there.",
            mm(worst),
            at[0] - b.min[0],
            at[1] - b.min[1],
            b.max[2] - at[2]
        ));
    }
    let floor = if si == 0 {
        plan.stock.min[2]
    } else {
        plan.stock.min[2].max(b.min[2])
    };
    let below = floor - lowest;
    if below > 0.001 {
        problems.push(if si == 0 {
            format!(
                "The tool goes {} mm below the bottom of the stock, where the vise or table is. Reduce the depth or the breakthrough.",
                mm(below)
            )
        } else {
            format!(
                "The tool goes {} mm below the part's underside in this setup. Earlier setups may have removed the stock there, and the part rests on it. Reduce the depth or the breakthrough.",
                mm(below)
            )
        });
    }
    problems
}
/// A distance in mm for messages: three decimals under 0.01 mm, so a few
/// thousandths never print as 0.00, otherwise two.
fn mm(v: f32) -> String {
    if v < 0.01 {
        format!("{v:.3}")
    } else {
        format!("{v:.2}")
    }
}
fn percent(part: f32, whole: f32) -> f32 {
    if whole <= 0. {
        0.
    } else {
        (part / whole * 1000.).round() / 10.
    }
}
/// What `stock` still covers. `geometry` and `stock` are in setup 1's frame.
pub fn coverage(
    stock: &Volume,
    part: &PartGrid,
    geometry: &Geometry,
    report: &PartReport,
) -> Coverage {
    let cell = stock.cell.iter().product::<f32>() / 1000.;
    let mut leftover = [0usize; 4];
    for z in 0..stock.n[2] {
        for y in 0..stock.n[1] {
            for x in 0..stock.n[0] {
                let i = stock.index([x, y, z]);
                if stock.occupied[i] == 0 || part.inside[i] {
                    continue;
                }
                let p = stock.point([x, y, z], 0.5);
                let b = geometry.bounds;
                let region = if p[2] > b.max[2] {
                    0
                } else if p[2] < b.min[2] {
                    3
                } else if (0..2).all(|k| p[k] >= b.min[k] && p[k] <= b.max[k]) {
                    2
                } else {
                    1
                };
                leftover[region] += 1;
            }
        }
    }
    // A face is covered when stock remains 1.5 cells outside it.
    let step = stock.cell.iter().copied().fold(0., f32::max) * 1.5;
    let mut faces: std::collections::HashMap<usize, (f32, f32)> = Default::default();
    // Report planes are listed on their own, not in the side totals.
    let planar: std::collections::HashSet<usize> = report
        .orientations
        .iter()
        .flat_map(|o| &o.planes)
        .flat_map(|p| p.surfaces.iter().copied())
        .collect();
    let mut sides = [(0f32, 0f32); 6];
    let (mut total, mut covered) = (0f32, 0f32);
    for t in &geometry.triangles {
        let n = analysis::normal(&t.p);
        let length = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        if length < 1e-9 {
            continue;
        }
        let c = analysis::centroid(&t.p);
        let q: [f32; 3] = std::array::from_fn(|k| c[k] + n[k] / length * step);
        let cell = (0..3)
            .all(|k| q[k] >= stock.bounds.min[k] && q[k] < stock.bounds.max[k])
            .then(|| {
                stock.index(std::array::from_fn(|k| {
                    (((q[k] - stock.bounds.min[k]) / stock.cell[k]) as usize).min(stock.n[k] - 1)
                }))
            });
        // A sample that lands inside the part (a narrow slot) says nothing.
        if cell.is_some_and(|i| part.inside[i]) {
            continue;
        }
        let (area, hidden) = (length / 2., cell.is_some_and(|i| stock.occupied[i] == 1));
        total += area;
        if hidden {
            covered += area;
        }
        if let Some(id) = t.feature {
            let face = faces.entry(id).or_default();
            face.0 += area;
            if hidden {
                face.1 += area;
            }
        }
        if t.feature.is_some_and(|id| planar.contains(&id)) {
            continue;
        }
        let axis = (0..3)
            .max_by(|a, b| n[*a].abs().total_cmp(&n[*b].abs()))
            .unwrap();
        let side = &mut sides[axis * 2 + usize::from(n[axis] < 0.)];
        side.0 += area;
        if hidden {
            side.1 += area;
        }
    }
    let mut covered_planes = vec![];
    for orientation in &report.orientations {
        for plane in &orientation.planes {
            let (area, hidden) = plane
                .surfaces
                .iter()
                .filter_map(|id| faces.get(id))
                .fold((0., 0.), |sum, face| (sum.0 + face.0, sum.1 + face.1));
            if hidden > area * 0.5 {
                covered_planes.push(CoveredPlane {
                    orientation: orientation.up,
                    surfaces: plane.surfaces.clone(),
                    depth_below_top: plane.depth_below_top,
                    area: (plane.area * 10.).round() / 10.,
                    covered_percent: percent(hidden, area),
                });
            }
        }
    }
    // Sides are named in setup 1's frame; the report's first orientation is setup 1.
    let names = ["+X", "-X", "+Y", "-Y", "top", "bottom"];
    let covered_other = (0..6)
        .filter(|i| sides[*i].1 > sides[*i].0 * 0.1)
        .map(|i| CoveredSide {
            facing: names[i],
            area: sides[i].0.round(),
            covered_percent: percent(sides[i].1, sides[i].0),
        })
        .collect();
    let cm3 = |n: usize| (n as f32 * cell * 10.).round() / 10.;
    Coverage {
        exposed_percent: percent(total - covered, total),
        covered_planes,
        covered_other,
        leftover_cm3: Leftover {
            above: cm3(leftover[0]),
            around: cm3(leftover[1]),
            inside_bounding_box: cm3(leftover[2]),
            below: cm3(leftover[3]),
        },
        note: "Approximate: measured on the coarse stock grid. Stock thinner than about 1.5 grid cells is not seen.",
    }
}
/// One problem per operation whose speed or feed lies outside the project
/// material's cutting data, per value, or that cannot be checked (no
/// material, no cutting data, a tool or operation without the numbers),
/// naming its setup and operation. Each message is one line of at most 300
/// characters: it names the material.
fn cutting_data_problems(built: &Document) -> Vec<Problem> {
    let mut problems = vec![];
    for (si, setup) in built.setups.iter().enumerate() {
        for (oi, op) in setup.operations.iter().enumerate() {
            let messages = match built.speed_feed(setup, op) {
                Some(SpeedFeed::InRange { .. }) => vec![],
                Some(SpeedFeed::Out(messages)) => messages,
                Some(SpeedFeed::Unchecked(why)) => vec![unchecked(&why)],
                None => vec![unchecked("Choose a material with cutting data.")],
            };
            problems.extend(messages.into_iter().map(|message| Problem {
                setup: si + 1,
                operation: Some(oi + 1),
                stage: "cutting data",
                message: proposal::clip(&message, 300),
            }));
        }
    }
    problems
}
fn unchecked(why: &str) -> String {
    format!("Speeds and feeds could not be checked: {why}")
}
/// Runs every stage on a proposal. Returns the report and, when the plan
/// passes, the job it describes.
pub fn check(
    doc: &Document,
    mesh: &Mesh,
    report: &PartReport,
    proposal: &Proposal,
) -> (CheckReport, Option<Document>) {
    match proposal::build(doc, mesh, report, proposal) {
        Ok(built) => {
            let out = check_built(&built, mesh, report);
            let job = out.passed.then_some(built);
            (out, job)
        }
        Err(problems) => (
            CheckReport {
                passed: false,
                problems,
                operations: vec![],
                coverage: None,
                total_minutes: 0.,
                zeros: vec![],
            },
            None,
        ),
    }
}
/// Runs the stages that follow building on a job, starting with the saved
/// project's structure check, so tests can check jobs `build` would refuse.
pub(crate) fn check_built(built: &Document, mesh: &Mesh, report: &PartReport) -> CheckReport {
    let mut out = CheckReport {
        passed: false,
        problems: vec![],
        operations: vec![],
        coverage: None,
        total_minutes: 0.,
        zeros: vec![],
    };
    if let Err(error) = built.validate_structure() {
        out.problems.push(Problem {
            setup: 0,
            operation: None,
            stage: "structure",
            message: format!("The plan builds a job the app cannot save: {error}"),
        });
        return out;
    }
    // Speeds and feeds against the material: reported beside the stages that
    // follow, so one round of feedback carries every problem.
    let cutting = cutting_data_problems(built);
    // Generation and cumulative stock, with the stock after each operation.
    let geometries: Vec<Geometry> = built
        .setups
        .iter()
        .map(|s| Geometry::new(s, mesh))
        .collect();
    let indexes: Vec<Index> = geometries.iter().map(Index::new).collect();
    let mut axis = vec![];
    let mut part: Option<PartGrid> = None;
    let mut gouged = 0;
    let mut last: Option<Volume> = None;
    let mut operations = vec![];
    let mut problems = vec![];
    let mut gouges = vec![];
    // Kept for the export stage, so the job is not simulated twice.
    let mut results: Vec<Vec<(usize, Result<Simulation, String>)>> =
        vec![vec![]; built.setups.len()];
    crate::simulation::sequence_with(
        built,
        mesh,
        None,
        |si, oi, result| {
            match &result {
                Ok(sim) => {
                    let op = &built.setups[si].operations[oi];
                    operations.push(OpResult {
                        setup: si + 1,
                        operation: oi + 1,
                        name: op.name().to_string(),
                        minutes: (sim.plan.duration / 6.).round() / 10.,
                    });
                    for message in axis_problems(&sim.plan, &geometries[si], &indexes[si], si) {
                        axis.push(Problem {
                            setup: si + 1,
                            operation: Some(oi + 1),
                            stage: "gouge",
                            message,
                        });
                    }
                }
                Err(message) => problems.push(Problem {
                    setup: si + 1,
                    operation: Some(oi + 1),
                    stage: "generation",
                    message: message.clone(),
                }),
            }
            results[si].push((oi, result));
        },
        |si, oi, stock| {
            let part = part.get_or_insert_with(|| PartGrid::new(stock, &geometries[0]));
            let (count, bounds) = part.gouged(stock);
            if gouge_added(gouged, count) {
                let size = geometries[0].bounds;
                gouges.push(Problem {
                    setup: si + 1,
                    operation: Some(oi + 1),
                    stage: "gouge",
                    message: format!(
                        "This operation cuts into the part: about {} near X {:.0}-{:.0}, Y {:.0}-{:.0}, {:.0}-{:.0} mm below the top (setup 1 axes, from the model's -X/-Y corner).",
                        volume_text((count - gouged) as f32 * stock.cell.iter().product::<f32>()),
                        bounds[0][0] - size.min[0],
                        bounds[1][0] - size.min[0],
                        bounds[0][1] - size.min[1],
                        bounds[1][1] - size.min[1],
                        size.max[2] - bounds[1][2],
                        size.max[2] - bounds[0][2],
                    ),
                });
            }
            gouged = count;
            last = Some(stock.clone());
        },
    );
    out.total_minutes = (operations.iter().map(|o| o.minutes).sum::<f32>() * 10.).round() / 10.;
    out.operations = operations;
    out.problems = problems;
    if out.problems.is_empty() {
        let job = crate::post::post_simulated(built, mesh, &results);
        let mut export: Vec<Problem> = job
            .errors
            .iter()
            .map(|message| Problem {
                setup: 0,
                operation: None,
                stage: "export",
                message: message.clone(),
            })
            .collect();
        for setup in &job.setups {
            let ops = &built.setups[setup.setup].operations;
            for message in setup.result.as_ref().err().into_iter().flatten() {
                // A message that starts with one operation's name is about it.
                let named: Vec<usize> = (0..ops.len())
                    .filter(|i| message.starts_with(&format!("{}: ", ops[*i].name())))
                    .collect();
                export.push(Problem {
                    setup: setup.setup + 1,
                    operation: (named.len() == 1).then(|| named[0] + 1),
                    stage: "export",
                    message: message.clone(),
                });
            }
        }
        if export.is_empty() {
            out.zeros = built
                .setups
                .iter()
                .zip(&results)
                .map(|(setup, sims)| {
                    let first = sims.iter().find_map(|(_, r)| r.as_ref().ok())?;
                    crate::post::model_zero(setup, mesh, first)
                })
                .collect();
        }
        if !job.ready() && export.is_empty() {
            export.push(Problem {
                setup: 0,
                operation: None,
                stage: "export",
                message: "The job is not ready to export.".into(),
            });
        }
        out.problems.extend(export);
    }
    out.problems.extend(cutting);
    out.problems.extend(axis);
    out.problems.extend(gouges);
    if let (Some(stock), Some(part)) = (&last, &part) {
        out.coverage = Some(coverage(stock, part, &geometries[0], report));
    }
    out.passed = out.problems.is_empty();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        model::{Operation, Tool},
        proposal::tests::{block, good, op},
    };
    fn stages(report: &CheckReport) -> Vec<String> {
        report
            .problems
            .iter()
            .map(|p| format!("{} {}/{:?}: {}", p.stage, p.setup, p.operation, p.message))
            .collect()
    }
    #[test]
    fn a_side_up_setup_after_a_contour_is_located_without_the_cut_stock_top() {
        use crate::proposal::tests::side_up;
        let (doc, mesh, report) = block();
        let (check, built) = check(&doc, &mesh, &report, &side_up(&report, "-Y"));
        assert!(check.passed, "{:?}", stages(&check));
        let (part, top) = check.zeros[1].unwrap();
        assert_eq!(top, None, "{part:?}");
        let line = crate::ui::work_zero_line(&built.unwrap().setups[1], check.zeros[1]);
        assert!(
            line.ends_with(
                "Z0 40 mm below the part's top. The stock top was cut by an earlier setup; touch off on a part face."
            ),
            "{line}"
        );
        assert!(!line.contains("Stock top") && !line.contains("below the stock top"));
    }
    #[test]
    fn a_good_two_setup_plan_passes_every_stage() {
        let (doc, mesh, report) = block();
        let (check, built) = check(&doc, &mesh, &report, &good(&report));
        assert!(check.passed, "{:?}", stages(&check));
        assert_eq!(check.operations.len(), 6);
        assert_eq!(
            (check.operations[5].setup, check.operations[5].operation),
            (2, 2)
        );
        assert!(check.operations.iter().all(|o| o.minutes > 0.));
        let sum: f32 = check.operations.iter().map(|o| o.minutes).sum();
        assert!((check.total_minutes - sum).abs() < 0.11);
        assert_eq!(built.as_ref().unwrap().setups.len(), 2);
        // Setup 2's Model origin is located for the preview: the part's box
        // and the stock top in its work coordinates.
        assert_eq!(check.zeros.len(), 2);
        assert!(check.zeros[0].is_none());
        let (part, top) = check.zeros[1].unwrap();
        let near = |a: [f32; 3], b: [f32; 3]| (0..3).all(|k| (a[k] - b[k]).abs() < 0.01);
        assert!(
            near(part.min, [-60., -40., -40.]) && near(part.max, [60., 40., 0.]),
            "{part:?}"
        );
        assert!(top.is_some_and(|top| (top - 3.).abs() < 0.01), "{top:?}");
        assert!(
            crate::ui::work_zero_line(&built.as_ref().unwrap().setups[1], check.zeros[1])
                .ends_with("Z0 3 mm below the stock top")
        );
        let coverage = check.coverage.unwrap();
        assert!(
            coverage.exposed_percent > 60. && coverage.exposed_percent < 100.,
            "{coverage:?}"
        );
        assert_eq!(coverage.leftover_cm3.above, 0.);
        assert_eq!(coverage.leftover_cm3.below, 0.);
        // The pocket floors and both faces are machined; they are not listed.
        let machined: Vec<usize> = good(&report)
            .setups
            .iter()
            .flat_map(|s| &s.operations)
            .flat_map(|o| o.surfaces.clone())
            .collect();
        for plane in &coverage.covered_planes {
            assert!(
                plane.surfaces.iter().all(|id| !machined.contains(id)),
                "{plane:?}"
            );
        }
        // Side features need other setups: something there is still covered.
        assert!(
            coverage
                .covered_planes
                .iter()
                .any(|p| p.orientation == "+X" || p.orientation == "-Y")
        );
    }
    #[test]
    fn speeds_and_feeds_outside_the_materials_data_are_refused_beside_the_other_problems() {
        let (doc, mesh, report) = block();
        // Setup 1's Pocket at 2000 rpm: 63 m/min, and 0.167 mm per tooth at 1000 mm/min.
        let mut slow = good(&report);
        slow.setups[0].operations[1].rpm = 2000.;
        let (check, job) = check(&doc, &mesh, &report, &slow);
        assert!(!check.passed && job.is_none());
        assert_eq!(
            stages(&check),
            vec![
                "cutting data 1/Some(2): Surface speed 63 m/min is below Aluminium 6061-T6's 150–300 m/min.",
                "cutting data 1/Some(2): Chip load 0.167 mm per tooth is above Aluminium 6061-T6's 0.030–0.080 mm for a Ø10 mm tool.",
            ]
        );
        // The stage does not stop generation or the export checks.
        assert_eq!(check.operations.len(), 6);
        assert!(check.coverage.is_some());
        // One round carries every problem: a generation failure in setup 1
        // and a chip load that is too light in setup 2.
        let mut both = good(&report);
        both.setups[0].operations[1].stepdown = Some(500.);
        both.setups[1].operations[0].feed = 100.;
        let (check, _) = super::check(&doc, &mesh, &report, &both);
        let found = stages(&check);
        assert!(
            found
                .iter()
                .any(|m| m.starts_with("generation 1/Some(2): ")),
            "{found:?}"
        );
        assert!(
            found.iter().any(|m| m.starts_with(
                "cutting data 2/Some(1): Chip load 0.004 mm per tooth is below Aluminium 6061-T6's 0.030–0.080 mm"
            )),
            "{found:?}"
        );
        // The export checks still run beside it: the flipped setup's stock-based
        // zero is refused after setup 1's Contour cut the stock's sides.
        let mut export = slow.clone();
        export.setups[1].origin = "Stock top center".into();
        let (check, _) = super::check(&doc, &mesh, &report, &export);
        let found = stages(&check);
        assert!(
            found.iter().any(|m| m.starts_with("export 2/None: ")),
            "{found:?}"
        );
        assert!(
            found
                .iter()
                .any(|m| m.starts_with("cutting data 1/Some(2): Surface speed")),
            "{found:?}"
        );
        // Drills are checked like mills: feed per tooth from the drill's flutes.
        let mut drill = good(&report);
        drill.setups[0].operations[3].feed = 280.;
        let (check, _) = super::check(&doc, &mesh, &report, &drill);
        assert_eq!(
            stages(&check),
            vec![
                "cutting data 1/Some(4): Chip load 0.008 mm per tooth is below Aluminium 6061-T6's 0.020–0.050 mm for a Ø4 mm tool."
            ]
        );
    }
    #[test]
    fn a_plan_whose_speeds_and_feeds_cannot_be_checked_is_refused() {
        let (doc, mesh, report) = block();
        // Without a material in the job nothing can be checked.
        let mut none = doc.clone();
        none.material_id.clear();
        let (check, job) = check(&none, &mesh, &report, &good(&report));
        assert!(!check.passed && job.is_none());
        let refusals: Vec<String> = [(1, 1), (1, 2), (1, 3), (1, 4), (2, 1), (2, 2)]
            .into_iter()
            .map(|(s, o)| {
                format!(
                    "cutting data {s}/Some({o}): Speeds and feeds could not be checked: Choose a material with cutting data."
                )
            })
            .collect();
        assert_eq!(stages(&check), refusals);
        // The other stages still run beside it.
        assert_eq!(check.operations.len(), 6);
        // A material without cutting data says so, once for each operation.
        let mut bare = doc.clone();
        let id = bare.material_id.clone();
        let material = bare.materials.iter_mut().find(|m| m.id == id).unwrap();
        material.p.remove("vc_min");
        let (check, _) = super::check(&bare, &mesh, &report, &good(&report));
        let found = stages(&check);
        assert_eq!(found.len(), 6, "{found:?}");
        assert_eq!(
            found[0],
            "cutting data 1/Some(1): Speeds and feeds could not be checked: Aluminium 6061-T6 has no cutting data."
        );
        // A tool with no flutes cannot be checked either.
        let mut flat = doc.clone();
        for tool in &mut flat.tools {
            tool.p.insert("flutes".into(), "0".into());
        }
        let (check, _) = super::check(&flat, &mesh, &report, &good(&report));
        let found: Vec<_> = stages(&check)
            .into_iter()
            .filter(|m| m.starts_with("cutting data "))
            .collect();
        assert_eq!(found.len(), 6, "{found:?}");
        assert_eq!(
            found[5],
            "cutting data 2/Some(2): Speeds and feeds could not be checked: The tool or the operation has no positive speed data."
        );
        // A long name stays one clipped line.
        let mut odd = doc.clone();
        let id = odd.material_id.clone();
        let material = odd.materials.iter_mut().find(|m| m.id == id).unwrap();
        material.p.remove("vc_min");
        material
            .p
            .insert("name".into(), format!("Odd\n{}", "x".repeat(500)));
        let (check, _) = super::check(&odd, &mesh, &report, &good(&report));
        for problem in check.problems.iter().filter(|p| p.stage == "cutting data") {
            assert!(
                problem.message.chars().count() <= 300 && !problem.message.contains('\n'),
                "{}",
                problem.message
            );
            assert!(problem.message.ends_with(" has no cutting data."));
        }
    }
    #[test]
    fn a_cutting_data_message_is_one_clipped_line_whatever_the_materials_name() {
        let (mut doc, mesh, report) = block();
        let id = doc.material_id.clone();
        let odd = format!("Odd\nalloy \u{202e}{}", "x".repeat(500));
        let material = doc.materials.iter_mut().find(|m| m.id == id).unwrap();
        material.p.insert("name".into(), odd);
        let mut slow = good(&report);
        slow.setups[0].operations[1].rpm = 2000.;
        let (check, _) = check(&doc, &mesh, &report, &slow);
        let found: Vec<_> = check
            .problems
            .iter()
            .filter(|p| p.stage == "cutting data")
            .collect();
        assert_eq!(found.len(), 2, "{:?}", stages(&check));
        // The name is clipped to 60 characters, so the message keeps its
        // range numbers whatever the name.
        let named = |message: &str, before: &str, after: &str| {
            message
                .strip_prefix(before)
                .and_then(|rest| rest.strip_suffix(after))
                .map(str::to_string)
        };
        let speed = named(
            &found[0].message,
            "Surface speed 63 m/min is below ",
            "'s 150–300 m/min.",
        );
        let load = named(
            &found[1].message,
            "Chip load 0.167 mm per tooth is above ",
            "'s 0.030–0.080 mm for a Ø10 mm tool.",
        );
        for (problem, name) in found.iter().zip([speed, load]) {
            let message = &problem.message;
            let name = name.unwrap_or_else(|| panic!("the range is cut off: {message}"));
            assert!(name.chars().count() <= 60, "{name}");
            assert!(name.starts_with("Odd alloy xxx"), "{name}");
            assert!(message.chars().count() <= 300, "{message}");
            assert!(!message.contains(['\n', '\u{202e}']), "{message}");
        }
    }
    #[test]
    fn reference_problems_stop_before_generation() {
        let (doc, mesh, report) = block();
        let mut bad = good(&report);
        bad.setups[0].operations[0].tool = 9;
        let (check, built) = check(&doc, &mesh, &report, &bad);
        assert!(!check.passed && built.is_none());
        assert_eq!(
            stages(&check),
            vec!["structure 1/Some(1): T9 is not in the tool library."]
        );
        assert!(check.operations.is_empty() && check.coverage.is_none());
    }
    #[test]
    fn generation_failures_name_the_operation_and_block_later_ones() {
        let (doc, mesh, report) = block();
        let mut bad = good(&report);
        // A stepdown longer than the flute cannot be generated.
        bad.setups[0].operations[1].stepdown = Some(500.);
        let (check, built) = check(&doc, &mesh, &report, &bad);
        assert!(
            !check.passed && built.is_none(),
            "a failing plan returns no job"
        );
        let found = stages(&check);
        assert!(found[0].starts_with("generation 1/Some(2): "), "{found:?}");
        assert!(
            found
                .iter()
                .any(|m| m.starts_with("generation 1/Some(3): Cannot inherit")),
            "{found:?}"
        );
        assert!(found.iter().all(|m| !m.starts_with("export")));
        assert_eq!(check.operations.len(), 1);
    }
    #[test]
    fn export_refusals_are_reported_for_their_setup() {
        let (doc, mesh, report) = block();
        let mut bad = good(&report);
        // The contour in setup 1 cuts the stock's sides, so a stock-based work
        // zero in the flipped setup is refused.
        bad.setups[1].origin = "Stock top center".into();
        let (check, _) = check(&doc, &mesh, &report, &bad);
        assert!(!check.passed);
        let found = stages(&check);
        assert!(
            found
                .iter()
                .any(|m| m.starts_with("export 2/None: ") && m.contains("Model origin")),
            "{found:?}"
        );
        assert_eq!(check.operations.len(), 6);
        assert!(check.coverage.is_some());
    }
    #[test]
    fn coverage_follows_the_machining() {
        let (doc, mesh, report) = block();
        let mut face_only = good(&report);
        face_only.setups.truncate(1);
        face_only.setups[0].operations.truncate(1);
        let (check, _) = check(&doc, &mesh, &report, &face_only);
        assert!(check.passed, "{:?}", stages(&check));
        let faced = check.coverage.unwrap();
        let (full, _) = super::check(&doc, &mesh, &report, &good(&report));
        let full = full.coverage.unwrap();
        assert!(faced.exposed_percent > 5. && faced.exposed_percent < full.exposed_percent);
        // The finished good plan leaves no stock above or below the part.
        assert_eq!(full.leftover_cm3.above, 0.);
        assert_eq!(full.leftover_cm3.below, 0.);
        assert_eq!(faced.leftover_cm3.above, 0.);
        assert!(faced.leftover_cm3.inside_bounding_box > full.leftover_cm3.inside_bounding_box);
        assert!(faced.leftover_cm3.around > full.leftover_cm3.around);
        assert!(faced.leftover_cm3.below > 0.);
        let top_pocket = &report.orientations[0]
            .planes
            .iter()
            .find(|p| p.depth_below_top == 12.)
            .unwrap();
        assert!(
            faced
                .covered_planes
                .iter()
                .any(|p| p.surfaces == top_pocket.surfaces
                    && p.covered_percent > 90.
                    && (p.area - top_pocket.area).abs() < 0.06)
        );
        assert!(
            faced
                .covered_other
                .iter()
                .any(|s| s.facing == "+X" && s.covered_percent > 50.)
        );
        assert!(
            serde_json::to_string(&faced)
                .unwrap()
                .contains("\"note\":\"Approximate")
        );
    }
    #[test]
    fn a_cut_into_the_part_is_detected_as_a_gouge() {
        let (doc, mesh, report) = block();
        let built = proposal::build(&doc, &mesh, &report, &good(&report)).unwrap();
        let geometry = Geometry::new(&built.setups[0], &mesh);
        let mut stock = None;
        crate::simulation::sequence_with(
            &built,
            &mesh,
            None,
            |_, _, _| {},
            |_, _, v| stock = Some(v.clone()),
        );
        let mut stock = stock.unwrap();
        let part = PartGrid::new(&stock, &geometry);
        assert_eq!(part.gouged(&stock).0, 0, "a checked plan does not gouge");
        // Plunge a 10 mm cutter 8 mm into solid material beside the pocket.
        let b = geometry.bounds;
        let at = [b.max[0] - 15., (b.min[1] + b.max[1]) / 2., b.max[2] - 8.];
        let frame = stock.frame;
        let flat = crate::simulation::CutterShape::Flat;
        stock.cut([at[0], at[1], b.max[2] + 5.], at, 5., 25., flat, frame);
        stock.cut(at, [at[0] + 4., at[1], at[2]], 5., 25., flat, frame);
        let (cells, bounds) = part.gouged(&stock);
        assert!(cells >= 20, "{cells}");
        assert!(bounds[0][0] > b.max[0] - 25. && bounds[1][2] <= b.max[2]);
    }
    #[test]
    fn the_command_line_check_matches_the_library() {
        let (mut doc, _, report) = block();
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("job.axis.json");
        crate::storage::save(&project, &doc).unwrap();
        let write = |name: &str, text: &str| {
            let path = dir.path().join(name);
            std::fs::write(&path, text).unwrap();
            path.to_string_lossy().into_owned()
        };
        let project = project.to_string_lossy().into_owned();
        let good_file = write("good.json", &serde_json::to_string(&good(&report)).unwrap());
        let (json, summary, passed) =
            crate::check_plan(&[project.clone(), good_file.clone()]).unwrap();
        assert!(passed && summary.starts_with("The plan passes every check: 6 operations"));
        let printed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(printed["passed"], true);
        assert_eq!(crate::exit_code(&Ok(passed)), 0);
        let mut bad = good(&report);
        bad.setups[0].operations[0].tool = 9;
        let bad_file = write("bad.json", &serde_json::to_string(&bad).unwrap());
        let (json, summary, passed) = crate::check_plan(&[project.clone(), bad_file]).unwrap();
        assert!(!passed && summary.contains("1 problem"));
        assert!(json.contains("T9 is not in the tool library"));
        assert_eq!(crate::exit_code(&Ok(passed)), 1);
        // The check could not run: usage, unreadable or invalid files.
        let not_json = write("notes.txt", "Here is the plan: {}");
        let missing = dir
            .path()
            .join("missing.json")
            .to_string_lossy()
            .into_owned();
        for args in [
            vec![project.clone()],
            vec![project.clone(), good_file.clone(), "extra".into()],
            vec![project.clone(), not_json],
            vec![missing.clone(), good_file.clone()],
        ] {
            let result = crate::check_plan(&args);
            assert_eq!(crate::exit_code(&result.map(|r| r.2)), 2, "{args:?}");
        }
        assert!(
            crate::check_plan(&[project.clone()])
                .unwrap_err()
                .starts_with("Usage")
        );
        let printed = crate::part_report(std::slice::from_ref(&project)).unwrap();
        let printed: serde_json::Value = serde_json::from_str(&printed).unwrap();
        assert_eq!(printed["orientations"][0]["up"], "+Z");
        assert!(
            crate::part_report(&[project.clone(), "extra".into()])
                .unwrap_err()
                .starts_with("Usage")
        );
        assert!(crate::part_report(&[missing]).is_err());
        let schema: serde_json::Value =
            serde_json::from_str(&crate::proposal_schema(&[]).unwrap()).unwrap();
        assert_eq!(schema["additionalProperties"], false);
        assert!(
            crate::proposal_schema(&["extra".into()])
                .unwrap_err()
                .starts_with("Usage")
        );
        doc.material.clear();
        doc.material_id.clear();
        crate::storage::save(std::path::Path::new(&project), &doc).unwrap();
        assert!(
            crate::check_plan(&[project, good_file])
                .unwrap_err()
                .starts_with("Material")
        );
    }
    #[test]
    fn unknown_parts_of_a_plan_never_pass_silently() {
        let (doc, mesh, report) = block();
        let mut plan = good(&report);
        plan.setups[0].operations.push(op("Waterline", 1, vec![]));
        plan.setups[0].operations.push(op("Parallel", 2, vec![]));
        let (check, _) = check(&doc, &mesh, &report, &plan);
        // Whatever the outcome, every operation is either timed or has a problem.
        let accounted = check.operations.len()
            + check
                .problems
                .iter()
                .filter(|p| p.stage == "generation")
                .count();
        assert_eq!(accounted, 8, "{:?}", stages(&check));
    }
    /// The job `good()` builds, for checks of jobs `build` would refuse.
    fn good_job() -> (Document, Mesh, PartReport) {
        let (doc, mesh, report) = block();
        let job = proposal::build(&doc, &mesh, &report, &good(&report)).unwrap();
        (job, mesh, report)
    }
    /// Adds T4, a Ø1 drill with 45 mm of flute, and returns its ID.
    fn thin_drill(job: &mut Document) -> String {
        let mut tool = Tool::new("Drill 1", "Drill", "1");
        for (k, v) in [("length", "45"), ("stickout", "50"), ("number", "4")] {
            tool.p.insert(k.into(), v.into());
        }
        job.tools.push(tool);
        job.tools[3].id.clone()
    }
    fn drill_through(op: &mut Operation, diameter: &str, breakthrough: &str) {
        for (k, v) in [
            ("hole_diameter", diameter),
            ("through", "Yes"),
            ("breakthrough", breakthrough),
        ] {
            op.p.insert(k.into(), v.into());
        }
    }
    #[test]
    fn a_job_the_app_cannot_save_stops_at_structure() {
        let (mut job, mesh, report) = good_job();
        let face = job.setups[0].operations[0].clone();
        job.setups[0].operations.push(face);
        let check = check_built(&job, &mesh, &report);
        assert_eq!(
            stages(&check),
            vec![
                "structure 0/None: The plan builds a job the app cannot save: Invalid operation identifiers or geometry."
            ]
        );
        assert!(!check.passed && check.operations.is_empty() && check.coverage.is_none());
    }
    #[test]
    fn a_drill_sent_past_a_blind_holes_bottom_is_refused() {
        let (mut job, mesh, report) = good_job();
        let unaltered = check_built(&job, &mesh, &report);
        assert!(unaltered.passed && unaltered.problems.is_empty());
        // The reviewers' case: a Ø1 drill sent 25 mm past the Ø6 x 8 hole.
        let tool = thin_drill(&mut job);
        let drill = &mut job.setups[0].operations[3];
        drill.tool = tool;
        // 0.02 mm per tooth at 12000 rpm: inside the material's band for a Ø1 tool.
        drill.p.insert("feed".into(), "720".into());
        drill_through(drill, "6", "25");
        assert_eq!(
            stages(&check_built(&job, &mesh, &report)),
            vec![
                "gouge 1/Some(4): The tool's centre line passes through 25.00 mm of the part at X 42.0, Y 40.0, 33.0 mm below the model top (this setup's axes, measured from the model's -X/-Y corner): the cut is deeper than the surface there."
            ]
        );
    }
    #[test]
    fn a_drill_below_the_stock_is_refused() {
        let (mut job, mesh, report) = good_job();
        drill_through(&mut job.setups[0].operations[3], "6.6", "15");
        assert_eq!(
            stages(&check_built(&job, &mesh, &report)),
            vec![
                "gouge 1/Some(4): The tool goes 12.00 mm below the bottom of the stock, where the vise or table is. Reduce the depth or the breakthrough."
            ]
        );
    }
    #[test]
    fn the_tool_axis_check_works_in_each_setups_frame() {
        let (doc, mesh, report) = block();
        let mut plan = good(&report);
        let mut drill = plan.setups[0].operations[3].clone();
        drill.surfaces = report.orientations[1].planes[0].surfaces.clone();
        plan.setups[1].operations.push(drill);
        let (check, job) = check(&doc, &mesh, &report, &plan);
        assert!(check.passed, "{:?}", stages(&check));
        let mut job = job.unwrap();
        let tool = thin_drill(&mut job);
        let drill = &mut job.setups[1].operations[2];
        drill.tool = tool;
        drill_through(drill, "5", "25");
        let found = stages(&check_built(&job, &mesh, &report));
        assert!(
            found.iter().any(|m| m.starts_with(
                "gouge 2/Some(3): The tool's centre line passes through 25.00 mm of the part"
            )),
            "{found:?}"
        );
        assert!(found.iter().all(|m| !m.contains(" 1/")), "{found:?}");
    }
    #[test]
    fn finishing_passes_keep_the_tool_axis_out_of_the_part() {
        let (doc, mesh, report) = block();
        let mut plan = good(&report);
        plan.setups[0].operations.push(op("Parallel", 2, vec![]));
        plan.setups[0].operations.push(op("Waterline", 1, vec![]));
        let (check, _) = check(&doc, &mesh, &report, &plan);
        let found = stages(&check);
        assert_eq!(check.operations.len(), 8, "{found:?}");
        // Parallel lifts its path by its tolerance; the 0.1 mm limit holds for it too.
        assert!(check.passed && found.is_empty(), "{found:?}");
    }
    #[test]
    fn coarse_gouges_count_every_operation_together() {
        assert!(!gouge_added(0, 2));
        assert!(gouge_added(2, 4));
        assert!(!gouge_added(4, 4));
        assert!(gouge_added(4, 5));
        assert_eq!(volume_text(250.), "250 mm3");
        assert_eq!(volume_text(1500.), "1.5 cm3");
    }
    #[test]
    fn coverage_sides_leave_out_the_reported_planes() {
        let (doc, mesh, report) = block();
        let (check, _) = check(&doc, &mesh, &report, &good(&report));
        let sides = check.coverage.unwrap().covered_other;
        assert!(!sides.is_empty() && sides.iter().all(|s| s.covered_percent > 10.));
        // Report planes are not counted again: sides and planes fit in the part's area.
        let geometry = Geometry::new(&doc.setups[0], &mesh);
        let total: f32 = geometry
            .triangles
            .iter()
            .map(|t| {
                analysis::normal(&t.p)
                    .iter()
                    .map(|v| v * v)
                    .sum::<f32>()
                    .sqrt()
                    / 2.
            })
            .sum();
        let planes: f32 = report
            .orientations
            .iter()
            .flat_map(|o| &o.planes)
            .map(|p| p.area)
            .sum();
        let listed: f32 = sides.iter().map(|s| s.area).sum();
        assert!(listed <= total - planes + 3., "{sides:?} {total} {planes}");
    }
    #[test]
    fn export_problems_name_their_operation_when_the_name_is_unique() {
        let (mut job, mesh, report) = good_job();
        job.setups[0]
            .machine
            .p
            .insert("post_air".into(), "Not available".into());
        job.setups[0].operations[1]
            .p
            .insert("coolant".into(), "Air".into());
        let found = stages(&check_built(&job, &mesh, &report));
        assert!(
            found
                .iter()
                .any(|m| m.starts_with("export 1/Some(2): Pocket by planner: coolant \"Air\"")),
            "{found:?}"
        );
        let name = job.setups[0].operations[1].name().to_string();
        job.setups[0].operations[2].p.insert("name".into(), name);
        let found = stages(&check_built(&job, &mesh, &report));
        assert!(
            found
                .iter()
                .any(|m| m.starts_with("export 1/None: Pocket by planner: coolant")),
            "{found:?}"
        );
    }
    /// The -Z orientation's top plane: the part's original underside.
    fn underside(report: &PartReport) -> Vec<usize> {
        report.orientations[1]
            .planes
            .iter()
            .find(|p| p.kind == "top")
            .unwrap()
            .surfaces
            .clone()
    }
    #[test]
    fn in_a_later_setup_the_floor_is_the_parts_underside() {
        let (doc, mesh, report) = block();
        // Setup 1's Face removed the stock that the flipped setup's stock box
        // still puts below the part. A plan cannot drill through in a later
        // setup, so the drill is made a through drill after building.
        let mut plan = good(&report);
        let mut drill = op("Drill", 3, underside(&report));
        drill.hole_diameter = Some(6.6);
        plan.setups[1].operations.push(drill);
        let job = proposal::build(&doc, &mesh, &report, &plan).unwrap();
        let through = |breakthrough: &str| {
            let mut job = job.clone();
            drill_through(&mut job.setups[1].operations[2], "6.6", breakthrough);
            check_built(&job, &mesh, &report)
        };
        assert_eq!(
            stages(&through("3")),
            vec![
                "gouge 2/Some(3): The tool goes 3.00 mm below the part's underside in this setup. Earlier setups may have removed the stock there, and the part rests on it. Reduce the depth or the breakthrough."
            ]
        );
        let flush = through("0");
        assert!(flush.passed, "{:?}", stages(&flush));
    }
    #[test]
    fn later_setups_cannot_drill_through() {
        let (doc, mesh, report) = block();
        let mut plan = good(&report);
        let mut drill = op("Drill", 3, underside(&report));
        drill.through = Some(true);
        drill.hole_diameter = Some(6.6);
        plan.setups[1].operations.push(drill);
        let (refused, job) = check(&doc, &mesh, &report, &plan);
        assert!(job.is_none());
        assert_eq!(
            stages(&refused),
            vec![
                "structure 2/Some(3): through is true, but in setup 2 the part rests on the face where the drill would exit, so it cannot break through. Drill through holes in setup 1, or set through to false and list the unfinished exit in unmachined."
            ]
        );
        plan.setups[1].operations[2].through = Some(false);
        let (passed, job) = check(&doc, &mesh, &report, &plan);
        assert!(passed.passed && job.is_some(), "{:?}", stages(&passed));
    }
    #[test]
    fn a_negative_bottom_offset_is_refused_before_generation() {
        let (doc, mesh, report) = block();
        let mut plan = good(&report);
        let mut contour = op("Contour", 1, vec![]);
        contour.bottom_offset = Some(-3.9);
        plan.setups[1].operations.push(contour);
        let (check, job) = check(&doc, &mesh, &report, &plan);
        assert!(job.is_none());
        assert_eq!(
            stages(&check),
            vec!["structure 2/Some(3): bottom_offset cannot be negative."]
        );
    }
    /// A plan with one setup per orientation in `setups`, in order.
    fn plan_of(setups: Vec<(&str, Vec<proposal::ProposedOp>)>) -> Proposal {
        Proposal {
            summary: String::new(),
            setups: setups
                .into_iter()
                .enumerate()
                .map(|(i, (up, operations))| proposal::ProposedSetup {
                    orientation: up.into(),
                    name: format!("Setup {}", i + 1),
                    origin: if i == 0 {
                        "Stock top center"
                    } else {
                        "Model origin"
                    }
                    .into(),
                    workholding: String::new(),
                    operations,
                })
                .collect(),
            unmachined: vec![],
            warnings: vec![],
        }
    }
    /// The surfaces of an orientation's top plane, or of its first floor.
    fn plane(report: &PartReport, up: &str, kind: &str) -> Vec<usize> {
        report
            .orientations
            .iter()
            .find(|o| o.up == up)
            .unwrap()
            .planes
            .iter()
            .find(|p| p.kind == kind)
            .unwrap()
            .surfaces
            .clone()
    }
    #[test]
    fn a_column_where_the_mesh_is_open_counts_as_solid() {
        let (doc, _, _) = block();
        let face_and_waterline = |report: &PartReport| {
            plan_of(vec![(
                "+Z",
                vec![
                    op("Face", 1, plane(report, "+Z", "top")),
                    op("Waterline", 1, vec![]),
                ],
            )])
        };
        // The reviewers' case: a 12 × 12 square missing from the top. The
        // report refuses the part for its open edges; behind it, the checks
        // still catch the Waterline cutting into the square.
        let open = analysis::tests::holed(12., 0.);
        assert!(analysis::report(&doc, &open).is_err());
        let report = analysis::report_unchecked(&doc, &open).unwrap();
        let (check, job) = check(&doc, &open, &report, &face_and_waterline(&report));
        let found = stages(&check);
        assert!(!check.passed && job.is_none(), "{found:?}");
        assert!(
            found.iter().any(|m| m.starts_with("gouge 1/Some(2): ")),
            "{found:?}"
        );
        // Both gouge checks see it: the tool axis, and the coarse check on
        // the stock grid, which names the Waterline and where it cut.
        assert!(
            found
                .iter()
                .any(|m| m.starts_with("gouge 1/Some(2): The tool's centre line passes through ")),
            "{found:?}"
        );
        let coarse: Vec<&String> = found
            .iter()
            .filter(|m| m.contains("This operation cuts into the part: about "))
            .collect();
        assert_eq!(coarse.len(), 1, "{found:?}");
        assert!(
            coarse[0].starts_with("gouge 1/Some(2): ")
                && coarse[0]
                    .contains(" mm below the top (setup 1 axes, from the model's -X/-Y corner)."),
            "{found:?}"
        );
        let closed = analysis::tests::holed(0., 0.);
        let report = analysis::report(&doc, &closed).unwrap();
        let (check, _) = super::check(&doc, &closed, &report, &face_and_waterline(&report));
        assert!(check.passed, "{:?}", stages(&check));
    }
    #[test]
    fn a_plunge_where_the_underside_is_open_is_refused() {
        let (doc, _, _) = block();
        // A 12 × 12 square missing from the underside: the column through it
        // crosses only the top face. The report refuses the part for its open
        // edges; the tool axis check is the second line of defence.
        let open = analysis::tests::holed(0., 12.);
        assert!(analysis::report(&doc, &open).is_err());
        let geometry = Geometry::new(&doc.setups[0], &open);
        let index = Index::new(&geometry);
        let b = geometry.bounds;
        let (x, y) = ((b.min[0] + b.max[0]) / 2., (b.min[1] + b.max[1]) / 2.);
        let z = b.max[2] - 10.;
        let mut plan = crate::post::fixture::plan(
            [x, y, b.max[2] + 5.],
            &[
                (Motion::Feed, [x, y, z], 300.),
                (Motion::Feed, [x + 3., y, z], 300.),
            ],
        );
        plan.stock.min[2] = b.min[2] - 3.;
        assert_eq!(
            axis_problems(&plan, &geometry, &index, 0),
            vec![
                "The tool's centre line passes through 10.00 mm of the part at X 60.0, Y 40.0, 10.0 mm below the model top (this setup's axes, measured from the model's -X/-Y corner): the cut is deeper than the surface there."
            ]
        );
    }
    #[test]
    fn a_pocket_under_an_open_top_square_passes_the_part_checks() {
        let (doc, _, _) = block();
        // A pocket open from below with a 12 × 12 square missing from the top
        // face above it. The report refuses the part for its open edges; the
        // checks behind it must still accept this sound plan.
        let mesh = analysis::tests::pocket_box(12.);
        assert!(analysis::report(&doc, &mesh).is_err());
        let report = analysis::report_unchecked(&doc, &mesh).unwrap();
        let plan = plan_of(vec![
            ("+Z", vec![op("Face", 1, plane(&report, "+Z", "top"))]),
            (
                "-Z",
                vec![
                    op("Face", 1, plane(&report, "-Z", "top")),
                    op("Pocket", 1, plane(&report, "-Z", "floor")),
                ],
            ),
        ]);
        let (check, job) = check(&doc, &mesh, &report, &plan);
        assert!(check.passed && job.is_some(), "{:?}", stages(&check));
    }
    #[test]
    fn distances_under_0_01_mm_print_three_decimals() {
        let (doc, mesh) = crate::post::fixture::job();
        let geometry = Geometry::new(&doc.setups[0], &mesh);
        let index = Index::new(&geometry);
        let b = geometry.bounds;
        // A plunge beside the part, to just below the floor.
        let (x, y) = (b.min[0] - 8., b.min[1] - 8.);
        let plunge = |to: f32, si: usize| {
            let mut plan = crate::post::fixture::plan(
                [x, y, b.max[2] + 5.],
                &[(Motion::Feed, [x, y, to], 300.)],
            );
            plan.stock.min[2] = b.min[2] - 3.;
            axis_problems(&plan, &geometry, &index, si)
        };
        assert_eq!(
            plunge(b.min[2] - 3.003, 0),
            vec![
                "The tool goes 0.003 mm below the bottom of the stock, where the vise or table is. Reduce the depth or the breakthrough."
            ]
        );
        assert_eq!(
            plunge(b.min[2] - 0.004, 1),
            vec![
                "The tool goes 0.004 mm below the part's underside in this setup. Earlier setups may have removed the stock there, and the part rests on it. Reduce the depth or the breakthrough."
            ]
        );
        assert!(plunge(b.min[2] - 3.02, 0)[0].starts_with("The tool goes 0.02 mm below"));
        assert_eq!(
            (mm(0.0099), mm(0.01), mm(1.5)),
            ("0.010".into(), "0.01".into(), "1.50".into())
        );
    }
    #[test]
    fn the_tool_axis_allows_0_1_mm_for_every_strategy() {
        let (doc, mesh) = crate::post::fixture::job();
        let geometry = Geometry::new(&doc.setups[0], &mesh);
        let index = Index::new(&geometry);
        let b = geometry.bounds;
        // A finishing pass along solid material beside the pocket.
        let (x, y) = (b.max[0] - 15., (b.min[1] + b.max[1]) / 2.);
        let pass = |depth: f32| {
            let z = b.max[2] - depth;
            let mut plan = crate::post::fixture::plan(
                [x, y - 2., z + 5.],
                &[
                    (Motion::Feed, [x, y - 2., z], 500.),
                    (Motion::Feed, [x, y + 2., z], 500.),
                ],
            );
            plan.strategy = crate::model::Strategy::Parallel;
            plan.stock.min[2] = b.min[2] - 3.;
            axis_problems(&plan, &geometry, &index, 0)
        };
        assert!(pass(0.09).is_empty());
        assert_eq!(
            pass(0.2),
            vec![
                "The tool's centre line passes through 0.20 mm of the part at X 105.0, Y 38.0, 0.2 mm below the model top (this setup's axes, measured from the model's -X/-Y corner): the cut is deeper than the surface there."
            ]
        );
    }
}
