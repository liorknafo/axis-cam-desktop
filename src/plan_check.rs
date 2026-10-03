//! Checks an AI proposal with the real generators, the stock simulation and
//! the export checks, then measures what the plan leaves uncut. The gouge and
//! coverage numbers come from the coarse stock grid and are approximate.
use crate::{
    analysis::{self, Index, PartReport},
    cam::{self, Geometry},
    mesh::Mesh,
    model::Document,
    proposal::{self, Problem, Proposal},
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
                // Crossings of the vertical ray: height, and whether the face points up.
                let mut hits: Vec<(f32, bool)> = index
                    .near(xy)
                    .filter_map(|t| {
                        Some((cam::plane_height(&t.p, xy)?, analysis::normal(&t.p)[2] > 0.))
                    })
                    .collect();
                hits.sort_by(|a, b| a.0.total_cmp(&b.0));
                hits.dedup_by(|a, b| (a.0 - b.0).abs() < 1e-4 && a.1 == b.1);
                let mut spans = vec![];
                let (mut depth, mut start, mut consistent) = (0i32, 0f32, true);
                for (z, up) in &hits {
                    if !*up {
                        if depth == 0 {
                            start = *z;
                        }
                        depth += 1;
                    } else {
                        depth -= 1;
                        if depth == 0 {
                            spans.push((start, *z));
                        }
                        consistent &= depth >= 0;
                    }
                }
                if !consistent || depth != 0 {
                    // Inconsistent face directions: fall back to crossing parity.
                    spans = hits.chunks_exact(2).map(|c| (c[0].0, c[1].0)).collect();
                }
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
        let axis = (0..3)
            .max_by(|a, b| n[*a].abs().total_cmp(&n[*b].abs()))
            .unwrap();
        let side = &mut sides[axis * 2 + usize::from(n[axis] < 0.)];
        side.0 += area;
        if hidden {
            side.1 += area;
        }
    }
    let mut planar = std::collections::HashSet::new();
    let mut covered_planes = vec![];
    for orientation in &report.orientations {
        for plane in &orientation.planes {
            planar.extend(plane.surfaces.iter().copied());
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
                    covered_percent: percent(hidden, area),
                });
            }
        }
    }
    // Sides are named in setup 1's frame; the report's first orientation is setup 1.
    let names = ["+X", "-X", "+Y", "-Y", "top", "bottom"];
    let covered_other = (0..6)
        .filter(|i| sides[*i].1 > 0.)
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
/// Runs every stage on a proposal. Returns the report and, when the proposal
/// could be built, the job it describes.
pub fn check(
    doc: &Document,
    mesh: &Mesh,
    report: &PartReport,
    proposal: &Proposal,
) -> (CheckReport, Option<Document>) {
    let mut out = CheckReport {
        passed: false,
        problems: vec![],
        operations: vec![],
        coverage: None,
        total_minutes: 0.,
    };
    let built = match proposal::build(doc, mesh, report, proposal) {
        Ok(built) => built,
        Err(problems) => {
            out.problems = problems;
            return (out, None);
        }
    };
    // Generation and cumulative stock, with the stock after each operation.
    let geometry = Geometry::new(&built.setups[0], mesh);
    let mut part: Option<PartGrid> = None;
    let mut gouged = 0;
    let mut last: Option<Volume> = None;
    let mut operations = vec![];
    let mut problems = vec![];
    let mut gouges = vec![];
    crate::simulation::sequence_with(
        &built,
        mesh,
        None,
        |si, oi, result| match result {
            Ok(sim) => operations.push(OpResult {
                setup: si + 1,
                operation: oi + 1,
                name: built.setups[si].operations[oi].name().to_string(),
                minutes: (sim.plan.duration / 6.).round() / 10.,
            }),
            Err(message) => problems.push(Problem {
                setup: si + 1,
                operation: Some(oi + 1),
                stage: "generation",
                message,
            }),
        },
        |si, oi, stock| {
            let part = part.get_or_insert_with(|| PartGrid::new(stock, &geometry));
            let (count, bounds) = part.gouged(stock);
            if count >= gouged + GOUGE_CELLS {
                let size = geometry.bounds;
                gouges.push(Problem {
                    setup: si + 1,
                    operation: Some(oi + 1),
                    stage: "gouge",
                    message: format!(
                        "This operation cuts into the part: about {:.1} cm3 near X {:.0}-{:.0}, Y {:.0}-{:.0}, {:.0}-{:.0} mm below the top (setup 1 axes, from the model's -X/-Y corner).",
                        (count - gouged) as f32 * stock.cell.iter().product::<f32>() / 1000.,
                        bounds[0][0] - size.min[0],
                        bounds[1][0] - size.min[0],
                        bounds[0][1] - size.min[1],
                        bounds[1][1] - size.min[1],
                        size.max[2] - bounds[1][2],
                        size.max[2] - bounds[0][2],
                    ),
                });
            }
            gouged = gouged.max(count);
            last = Some(stock.clone());
        },
    );
    out.total_minutes = (operations.iter().map(|o| o.minutes).sum::<f32>() * 10.).round() / 10.;
    out.operations = operations;
    out.problems = problems;
    if out.problems.is_empty() {
        let job = crate::post::post_job(&built, mesh);
        out.problems
            .extend(job.errors.iter().map(|message| Problem {
                setup: 0,
                operation: None,
                stage: "export",
                message: message.clone(),
            }));
        for setup in &job.setups {
            for message in setup.result.as_ref().err().into_iter().flatten() {
                out.problems.push(Problem {
                    setup: setup.setup + 1,
                    operation: None,
                    stage: "export",
                    message: message.clone(),
                });
            }
        }
    }
    out.problems.extend(gouges);
    if let (Some(stock), Some(part)) = (&last, &part) {
        out.coverage = Some(coverage(stock, part, &geometry, report));
    }
    out.passed = out.problems.is_empty();
    (out, Some(built))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proposal::tests::{block, good, op};
    fn stages(report: &CheckReport) -> Vec<String> {
        report
            .problems
            .iter()
            .map(|p| format!("{} {}/{:?}: {}", p.stage, p.setup, p.operation, p.message))
            .collect()
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
        assert_eq!(built.unwrap().setups.len(), 2);
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
        assert!(!check.passed && built.is_some());
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
        assert_eq!(faced.leftover_cm3.above, 0.);
        assert!(faced.leftover_cm3.inside_bounding_box > full.leftover_cm3.inside_bounding_box);
        assert!(faced.leftover_cm3.around > full.leftover_cm3.around);
        assert!(faced.leftover_cm3.below > 0.);
        let top_pocket = &report.orientations[0]
            .planes
            .iter()
            .find(|p| p.depth_below_top == 12.)
            .unwrap()
            .surfaces;
        assert!(
            faced
                .covered_planes
                .iter()
                .any(|p| &p.surfaces == top_pocket && p.covered_percent > 90.)
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
        doc.material = "Aluminum 6061".into();
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("job.axis.json");
        crate::storage::save(&project, &doc).unwrap();
        let write = |name: &str, proposal: &Proposal| {
            let path = dir.path().join(name);
            std::fs::write(&path, serde_json::to_string(proposal).unwrap()).unwrap();
            path.to_string_lossy().into_owned()
        };
        let project = project.to_string_lossy().into_owned();
        let good_file = write("good.json", &good(&report));
        let passed = crate::check_plan(Some(&project), Some(&good_file)).unwrap();
        assert!(passed.starts_with("The plan passes every check: 6 operations"));
        let mut bad = good(&report);
        bad.setups[0].operations[0].tool = 9;
        let bad_file = write("bad.json", &bad);
        assert!(
            crate::check_plan(Some(&project), Some(&bad_file))
                .unwrap_err()
                .contains("1 problem")
        );
        assert!(
            crate::check_plan(Some(&project), None)
                .unwrap_err()
                .starts_with("Usage")
        );
        let printed = crate::part_report(Some(&project)).unwrap();
        let printed: serde_json::Value = serde_json::from_str(&printed).unwrap();
        assert_eq!(printed["orientations"][0]["up"], "+Z");
        doc.material.clear();
        crate::storage::save(std::path::Path::new(&project), &doc).unwrap();
        assert!(
            crate::check_plan(Some(&project), Some(&good_file))
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
}
