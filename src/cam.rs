//! Imported-mesh 3-axis preview kernels. Coordinates are millimetres in setup space.
//! These bounded previews are not a controller postprocessor or a collision solver.
use crate::{
    mesh::{Bounds, Mesh},
    model::{
        CONTOUR_TABS, Operation, Setup, Strategy, Tool, facing_setting, get, num, pocket_setting,
    },
    simulation::{CutterShape, Motion, Move, Plan},
    stock::{self, Placement},
    surface,
};
use clipper2::{EndType, FillRule, JoinType, Milli, Paths, union};
use std::collections::{HashMap, HashSet};
pub(crate) type Polygon = Vec<(f64, f64)>;
pub(crate) type Region = Paths<Milli>;
/// The most moves (or samples) a toolpath may have: what memory takes, not
/// what a preview can draw. A move takes about 190 bytes with the stock cut
/// it leaves and the copies the simulation keeps (a Pocket of 15.3 million
/// peaked at 2.9 GB), so this is about 1 GB. A long toolpath is generated
/// off the window's thread, with progress and Cancel (`progress`); this only
/// stops a mistyped stepdown or stepover from filling the memory. The
/// largest real job seen, the TOTEM case's holes in one Pocket at 0.25 mm
/// stepdown, is 270,000.
pub(crate) const CEILING: usize = 5_000_000;
/// The refusal past `CEILING`.
pub(crate) fn too_many(what: &str) -> String {
    format!(
        "This toolpath would have more than 5 million {what}, more than memory holds. Check its stepdown, stepover and tolerance."
    )
}
pub(crate) fn number(op: &Operation, key: &str) -> Result<f32, String> {
    num(&op.p, key).ok_or_else(|| format!("Enter a finite {} value.", key.replace('_', " ")))
}
pub(crate) fn area(p: &Polygon) -> f64 {
    p.iter()
        .zip(p.iter().cycle().skip(1))
        .take(p.len())
        .map(|(a, b)| a.0 * b.1 - b.0 * a.1)
        .sum::<f64>()
        / 2.
}
pub(crate) fn region(mut polygons: Vec<Polygon>) -> Result<Region, String> {
    polygons.retain(|p| area(p).abs() > 0.000001);
    for p in &mut polygons {
        if area(p) < 0. {
            p.reverse();
        }
    }
    union::<Milli>(polygons, Vec::<Polygon>::new(), FillRule::NonZero).map_err(|e| e.to_string())
}
#[derive(Clone)]
pub(crate) struct Triangle {
    pub p: [[f32; 3]; 3],
    pub feature: Option<usize>,
}
pub(crate) struct Geometry {
    pub triangles: Vec<Triangle>,
    pub bounds: Bounds,
    /// The threaded holes along Z (`thread::holes_in_setup`) that hole
    /// reading knows of; none until `with_threads`.
    pub threads: Vec<crate::thread::Thread>,
}
impl Geometry {
    pub(crate) fn new(setup: &Setup, mesh: &Mesh) -> Self {
        let placement = Placement::new(mesh, Some(setup));
        let scale = mesh.display_scale();
        Self {
            triangles: mesh
                .faces
                .iter()
                .filter(|f| f.points.len() == 3)
                .map(|f| Triangle {
                    p: std::array::from_fn(|i| placement.transform(f.points[i]).map(|v| v / scale)),
                    feature: f.feature,
                })
                .collect(),
            bounds: placement.bounds.scaled(1. / scale),
            threads: vec![],
        }
    }
    /// The geometry, knowing the part's threaded holes along the setup's Z:
    /// a modeled thread's rim is no circle, but it is a hole.
    pub(crate) fn with_threads(setup: &Setup, mesh: &Mesh) -> Self {
        let mut g = Self::new(setup, mesh);
        g.threads = crate::thread::holes_in_setup(setup, mesh);
        g
    }
    fn selected(&self, op: &Operation) -> Vec<&Triangle> {
        self.triangles
            .iter()
            .filter(|t| {
                op.surfaces
                    .as_ref()
                    .is_some_and(|s| t.feature.is_some_and(|id| s.ids.contains(&id)))
            })
            .collect()
    }
    pub(crate) fn silhouette(&self, selected: bool, op: &Operation) -> Result<Region, String> {
        region(
            self.triangles
                .iter()
                .filter(|t| {
                    !selected
                        || op
                            .surfaces
                            .as_ref()
                            .is_some_and(|s| t.feature.is_some_and(|id| s.ids.contains(&id)))
                })
                .map(|t| t.p.iter().map(|v| (v[0] as f64, v[1] as f64)).collect())
                .collect(),
        )
    }
    fn height(&self, xy: [f32; 2]) -> Option<f32> {
        self.triangles
            .iter()
            .filter_map(|t| plane_height(&t.p, xy))
            .max_by(f32::total_cmp)
    }
}
pub(crate) fn plane_height(p: &[[f32; 3]; 3], xy: [f32; 2]) -> Option<f32> {
    let a = [p[1][0] - p[0][0], p[1][1] - p[0][1]];
    let b = [p[2][0] - p[0][0], p[2][1] - p[0][1]];
    let v = [xy[0] - p[0][0], xy[1] - p[0][1]];
    let det = a[0] * b[1] - a[1] * b[0];
    if det.abs() < 1e-8 {
        return None;
    }
    let u = (v[0] * b[1] - v[1] * b[0]) / det;
    let w = (a[0] * v[1] - a[1] * v[0]) / det;
    (u >= -0.00001 && w >= -0.00001 && u + w <= 1.00001)
        .then_some(p[0][2] + u * (p[1][2] - p[0][2]) + w * (p[2][2] - p[0][2]))
}
/// The centre of the circle through the loop's vertices: least squares on
/// x² + y² + Dx + Ey + F = 0, exact for vertices on a circle whatever their
/// spacing or phase (their box's centre and their mean are not); none for a
/// loop that fits no circle.
fn circle_centre(path: &Polygon) -> Option<(f64, f64)> {
    // About the box's centre, so that the sums stay small.
    let (low, high) = path.iter().fold(
        (
            (f64::INFINITY, f64::INFINITY),
            (f64::NEG_INFINITY, f64::NEG_INFINITY),
        ),
        |((x0, y0), (x1, y1)), p| ((x0.min(p.0), y0.min(p.1)), (x1.max(p.0), y1.max(p.1))),
    );
    let mid = ((low.0 + high.0) / 2., (low.1 + high.1) / 2.);
    // The normal equations, with their right-hand side as a fourth column.
    let mut m = [[0f64; 4]; 3];
    for p in path {
        let (x, y) = (p.0 - mid.0, p.1 - mid.1);
        let row = [x, y, 1., -(x * x + y * y)];
        for (sums, w) in m.iter_mut().zip([x, y, 1.]) {
            for (sum, v) in sums.iter_mut().zip(row) {
                *sum += w * v;
            }
        }
    }
    let det = |c: [[f64; 3]; 3]| {
        c[0][0] * (c[1][1] * c[2][2] - c[1][2] * c[2][1])
            - c[0][1] * (c[1][0] * c[2][2] - c[1][2] * c[2][0])
            + c[0][2] * (c[1][0] * c[2][1] - c[1][1] * c[2][0])
    };
    let a = m.map(|r| [r[0], r[1], r[2]]);
    let d = det(a);
    if d.abs() <= 1e-9 * (a[0][0] * a[1][1] * a[2][2]).abs() {
        return None;
    }
    let solve = |k: usize| {
        let mut c = a;
        for (row, sums) in c.iter_mut().zip(m) {
            row[k] = sums[3];
        }
        det(c) / d
    };
    Some((mid.0 - solve(0) / 2., mid.1 - solve(1) / 2.))
}
/// The gradient of the triangle's plane, rise per horizontal distance up its
/// steepest line; none for a vertical triangle.
fn plane_slope(p: &[[f32; 3]; 3]) -> Option<f32> {
    let a = [p[1][0] - p[0][0], p[1][1] - p[0][1], p[1][2] - p[0][2]];
    let b = [p[2][0] - p[0][0], p[2][1] - p[0][1], p[2][2] - p[0][2]];
    let det = a[0] * b[1] - a[1] * b[0];
    if det.abs() < 1e-8 {
        return None;
    }
    let gx = (a[2] * b[1] - b[2] * a[1]) / det;
    let gy = (a[0] * b[2] - b[0] * a[2]) / det;
    Some((gx * gx + gy * gy).sqrt())
}
struct Builder {
    plan: Plan,
    pos: [f32; 3],
    feed: f32,
    clearance: f32,
    retract: f32,
    retract_margin: f32,
    /// Rapid rate, mm/min: the hand rapid on a manual mill.
    rapid: f32,
    /// A manual mill moves one axis at a time (`push`).
    single_axis: bool,
}
impl Builder {
    fn new(setup: &Setup, op: &Operation, tool: &Tool, mesh: &Mesh) -> Result<Self, String> {
        if !op.enabled {
            return Err("Enable the operation before generating it.".into());
        }
        if let Some(issue) = surface::issues(op, setup, Some(mesh)).first() {
            return Err(issue.clone());
        }
        if get(&setup.p, "stock_source") != "fresh" {
            return Err("Preview requires fresh stock. Stock transfer is not implemented.".into());
        }
        let positive = |key| {
            num(&tool.p, key)
                .filter(|v| *v > 0.)
                .ok_or_else(|| format!("Enter a positive tool {key}."))
        };
        let radius = positive("diameter")? / 2.;
        let flute_length = positive("length")?;
        let tool_length = positive("stickout")?;
        if tool_length < flute_length {
            return Err("Tool stickout must cover flute length.".into());
        }
        let feed = number(op, "feed")?;
        if feed <= 0. {
            return Err("Cutting feed must be positive.".into());
        }
        let stock = stock::resolve(setup, Some(mesh))?;
        let top = stock.stock.max[2] + number(op, "top")?.max(0.);
        let clearance = top + number(op, "clearance")?;
        let retract = top + number(op, "retract")?;
        if retract <= top || clearance < retract {
            return Err("Use positive retract and clearance at or above retract.".into());
        }
        let spindle_clockwise = facing_setting(&op.p, "spindle_rotation") == "Clockwise (CW)";
        if !crate::model::SPINDLE_ROTATIONS.contains(&facing_setting(&op.p, "spindle_rotation")) {
            return Err("Choose a valid spindle rotation.".into());
        }
        let single_axis = crate::manual::is_manual(&setup.machine);
        let rapid = if single_axis {
            num(&setup.machine.p, "hand_rapid")
                .filter(|v| *v > 0.)
                .ok_or("Enter a positive hand rapid for the manual mill.")?
        } else {
            6000.
        };
        Ok(Self {
            plan: Plan {
                strategy: op.strategy,
                operation: op.id.clone(),
                setup: setup.id.clone(),
                moves: vec![],
                stock: stock.stock,
                radius,
                flute_length,
                tool_length,
                holder_radius: positive("holder")? / 2.,
                neck: None,
                display_scale: stock.display_scale,
                duration: 0.,
                target: top,
                levels: 0,
                rest_top: None,
                spindle_clockwise,
                milling_direction: "Mixed · preview",
                rings: vec![],
                shape: if get(&tool.p, "type") == "Ball end mill" {
                    CutterShape::Ball
                } else {
                    CutterShape::Flat
                },
            },
            pos: [stock.stock.min[0], stock.stock.min[1], clearance],
            feed,
            clearance,
            retract,
            retract_margin: number(op, "retract")?,
            rapid,
            single_axis,
        })
    }
    /// A move to `to`. With `single_axis`, legs of one axis each: Z first
    /// when it rises, then X, then Y, then Z when it falls.
    fn push(&mut self, to: [f32; 3], motion: Motion, cut: bool, rate: f32) -> Result<(), String> {
        if !self.single_axis {
            return self.leg(to, motion, cut, rate);
        }
        let order = if to[2] > self.pos[2] {
            [2, 0, 1]
        } else {
            [0, 1, 2]
        };
        let mut at = self.pos;
        for k in order {
            at[k] = to[k];
            self.leg(at, motion, cut, rate)?;
        }
        Ok(())
    }
    fn leg(&mut self, to: [f32; 3], motion: Motion, cut: bool, rate: f32) -> Result<(), String> {
        if self.plan.moves.len() >= CEILING {
            return Err(too_many("moves"));
        }
        // Each strategy counts its own steps (levels, holes, rows).
        crate::progress::check()?;
        let distance = (0..3)
            .map(|i| (to[i] - self.pos[i]).powi(2))
            .sum::<f32>()
            .sqrt();
        if !distance.is_finite() || rate <= 0. || !rate.is_finite() {
            return Err("Nonfinite motion or invalid feed.".into());
        }
        if distance > 0.00001 {
            let end = self.plan.duration + distance / rate * 60.;
            self.plan.moves.push(Move {
                from: self.pos,
                to,
                motion,
                cutting_pass: cut,
                feed: rate,
                start: self.plan.duration,
                end,
                a: None,
            });
            self.plan.duration = end;
        }
        self.pos = to;
        Ok(())
    }
    /// A stop of `seconds` where the tool is; none for 0.
    fn dwell(&mut self, seconds: f32) {
        if seconds > 0. {
            let end = self.plan.duration + seconds;
            self.plan.moves.push(Move {
                from: self.pos,
                to: self.pos,
                motion: Motion::Dwell,
                cutting_pass: false,
                feed: 0.,
                start: self.plan.duration,
                end,
                a: None,
            });
            self.plan.duration = end;
        }
    }
    fn retract(&mut self) -> Result<(), String> {
        self.push(
            [self.pos[0], self.pos[1], self.clearance],
            Motion::Rapid,
            false,
            self.rapid,
        )
    }
    fn start(&mut self, p: [f32; 3]) -> Result<(), String> {
        self.start_at(p, self.plan.rest_top)
    }
    fn start_at(&mut self, p: [f32; 3], top: Option<f32>) -> Result<(), String> {
        self.retract()?;
        self.push(
            [p[0], p[1], self.clearance],
            Motion::Rapid,
            false,
            self.rapid,
        )?;
        let approach = top.map_or(self.retract, |z| {
            (z + self.retract_margin).min(self.retract)
        });
        self.push([p[0], p[1], approach], Motion::Rapid, false, self.rapid)?;
        self.push(p, Motion::Feed, false, self.feed * 0.25)
    }
    fn stroke(&mut self, points: &[[f32; 3]]) -> Result<(), String> {
        if points.len() < 2 {
            return Ok(());
        }
        self.start(points[0])?;
        for &p in &points[1..] {
            self.push(p, Motion::Feed, true, self.feed)?;
        }
        self.retract()
    }
    /// `stroke`, but each move straight down feeds at the plunge rate the
    /// entry takes: the drops off a tab's far end into the uncut level.
    fn stroke_plunging(&mut self, points: &[[f32; 3]]) -> Result<(), String> {
        if points.len() < 2 {
            return Ok(());
        }
        self.start(points[0])?;
        for &p in &points[1..] {
            let down = p[2] < self.pos[2] - 0.00001
                && (p[0] - self.pos[0]).abs() < 0.00001
                && (p[1] - self.pos[1]).abs() < 0.00001;
            let rate = if down { self.feed * 0.25 } else { self.feed };
            self.push(p, Motion::Feed, true, rate)?;
        }
        self.retract()
    }
    fn finish(mut self) -> Result<Plan, String> {
        self.retract()?;
        if self
            .plan
            .moves
            .iter()
            .filter(|m| m.motion == Motion::Feed)
            .flat_map(|m| [m.from, m.to])
            .any(|p| {
                p[2] > self.clearance || self.plan.stock.max[2] - p[2] >= self.plan.tool_length
            })
        {
            return Err("Generated cut exceeds tool reach or clearance height. Increase stickout/clearance or reduce depth.".into());
        }
        if !self.plan.moves.iter().any(|m| m.motion == Motion::Feed)
            || !self.plan.duration.is_finite()
        {
            return Err("No cutting motion fits the selected geometry and settings.".into());
        }
        Ok(self.plan)
    }
}
pub fn generate(setup: &Setup, op: &Operation, tool: &Tool, mesh: &Mesh) -> Result<Plan, String> {
    // Turning, and drilling on a lathe's centre, are planned on the lathe.
    if op.strategy == Strategy::Turn || op.on_centre(&setup.machine) {
        return crate::turn::generate(setup, op, tool, mesh);
    }
    match op.strategy {
        Strategy::Contour => contour(setup, op, tool, mesh),
        Strategy::Drill if op.drills_pin_holes() => pin_holes(setup, op, tool, mesh),
        Strategy::Drill => drill(setup, op, tool, mesh),
        Strategy::Parallel => parallel(setup, op, tool, mesh),
        Strategy::Waterline => waterline(setup, op, tool, mesh),
        Strategy::Box => boxed(setup, op, tool, mesh),
        _ => Err(op.strategy.availability().into()),
    }
}
fn contour(setup: &Setup, op: &Operation, tool: &Tool, mesh: &Mesh) -> Result<Plan, String> {
    let mut b = Builder::new(setup, op, tool, mesh)?;
    if get(&tool.p, "type") != "Flat end mill" {
        return Err("Contour preview requires a flat end mill.".into());
    }
    if get(&op.p, "boundary") == "Selected geometry" && get(&op.p, "side") == "Inside" {
        if get(&op.p, "compensation") != "In computer" {
            return Err("Only In computer compensation is supported.".into());
        }
        let mut floor_profile = op.clone();
        floor_profile
            .p
            .insert("containment".into(), "Tool inside boundary".into());
        // Contour keeps its requested radial allowance and floor height. Pocket's
        // axial allowance/stepover fields are unrelated to wall-only machining.
        floor_profile.p.insert("axial_allowance".into(), "0".into());
        floor_profile.p.insert("stepover".into(), "40".into());
        return crate::pocket::generate(setup, &floor_profile, tool, mesh);
    }

    if get(&op.p, "compensation") != "In computer" {
        return Err("Only In computer compensation is supported. Controller compensation needs a postprocessor.".into());
    }
    // Holes: inside every hole through the part's outline, the cut-outs of
    // a flat part, which a Drill does not make.
    let holes = get(&op.p, "side") == crate::model::CONTOUR_HOLES;
    if get(&op.p, "boundary") != "Model silhouette" || !(holes || get(&op.p, "side") == "Outside") {
        return Err("Choose Model silhouette / Outside or Holes, or Selected geometry / Inside and select a recessed planar floor.".into());
    }
    let g = Geometry::new(setup, mesh);
    let radial = number(op, "allowance")?;
    if radial < 0. {
        return Err("Stock to leave cannot be negative.".into());
    }
    let paths: Vec<Polygon> = g
        .silhouette(false, op)?
        .inflate(
            (b.plan.radius + radial + (b.plan.radius * 0.01).max(0.02)) as f64,
            JoinType::Round,
            EndType::Polygon,
            2.,
        )
        .simplify(0.001, false)
        .into();
    let target = g.bounds.min[2] + number(op, "bottom")?;
    let top = b.plan.stock.max[2] + number(op, "top")?.max(0.);
    // A sheet's cut goes on into the spoilboard, by its allowance at most.
    let board = stock::below(setup);
    if target < b.plan.stock.min[2] - board - 0.0001 {
        return Err(if board > 0. {
            format!(
                "The contour would go {:.2} mm into the spoilboard, deeper than its {board} mm allowance: raise Bottom offset.",
                b.plan.stock.min[2] - target
            )
        } else {
            "Contour depth must be within stock and tool reach; bottom is relative to model bottom."
                .into()
        });
    }
    if target >= top || top - target >= b.plan.tool_length {
        return Err("Contour depth must be within stock and tool reach; bottom is relative to model bottom.".into());
    }
    let down = number(op, "stepdown")?;
    let finish = number(op, "finish_passes")?;
    if down <= 0.
        || down > b.plan.flute_length
        || !(0. ..=20.).contains(&finish)
        || finish.fract() != 0.
    {
        return Err(
            "Use a positive stepdown within flute length and 0–20 whole finishing passes.".into(),
        );
    }
    // The outer outlines, or for Holes the holes the tool fits in that are
    // at least `hole_min` wide (narrower ones are left for a Drill), each
    // turned to run anticlockwise.
    let offset = f64::from(b.plan.radius + radial + (b.plan.radius * 0.01).max(0.02));
    let hole_min = if holes {
        num(&op.p, "hole_min").unwrap_or(0.).max(0.)
    } else {
        0.
    };
    let paths: Vec<Polygon> = (paths.into_iter())
        .filter(|p| if holes { area(p) < 0. } else { area(p) > 0. })
        .filter(|p| {
            let (lo, hi) = p.iter().fold(
                ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]),
                |(lo, hi), q| {
                    (
                        [lo[0].min(q.0), lo[1].min(q.1)],
                        [hi[0].max(q.0), hi[1].max(q.1)],
                    )
                },
            );
            // A round hole's chords read it up to `HOLE_MIN_SLACK` narrower.
            ((hi[0] - lo[0]).min(hi[1] - lo[1]) + 2. * offset) as f32 >= hole_min - HOLE_MIN_SLACK
        })
        .map(|mut p| {
            if holes {
                p.reverse();
            }
            p
        })
        .collect();
    if holes && paths.is_empty() {
        return Err(format!(
            "The part has no hole through its outline that a Ø{:.2} tool fits in{}.",
            2. * b.plan.radius,
            if hole_min > 0. {
                format!(" and that is at least {hole_min} mm wide")
            } else {
                String::new()
            }
        ));
    }
    // From the highest material in the ring the cutter sweeps round each
    // outline, not in the outlines' box: the part inside the ring stands
    // above where earlier cuts may have cleared round it.
    let outer: Vec<Polygon> = paths.clone();
    let ring: Vec<Polygon> = Region::from(outer)
        .inflate(b.plan.radius as f64, JoinType::Round, EndType::Joined, 2.)
        .into();
    let cutting_top = crate::rest::start_over(
        setup,
        &crate::rest::Reach::new(&ring),
        target,
        top,
        number(op, "top")?,
    );
    b.plan.rest_top = (cutting_top < top - 0.0001).then_some(cutting_top);
    let levels = ((cutting_top - target - 0.0001) / down).ceil().max(1.) as usize;
    if levels > CEILING {
        return Err(too_many("moves"));
    }
    let climb = pocket_setting(&op.p, "pocket_milling") == "Climb";
    if !crate::model::FACING_MILLING.contains(&pocket_setting(&op.p, "pocket_milling")) {
        return Err("Choose Climb or Conventional.".into());
    }
    b.plan.milling_direction = if climb { "Climb" } else { "Conventional" };
    b.plan.target = target;
    b.plan.levels = levels;
    // Tabs hold the part in the sheet or stock round it: an outside contour's.
    let tabs = if pocket_setting(&op.p, "tabs") == CONTOUR_TABS[1] && !holes {
        let count = number(op, "tab_count")?;
        let width = number(op, "tab_width")?;
        let height = number(op, "tab_height")?;
        if !(1. ..=50.).contains(&count) || count.fract() != 0. || width <= 0. || height <= 0. {
            return Err("Use 1–50 whole tabs with a positive tab width and tab height.".into());
        }
        // Measured up from where the cut leaves the stock: a cut below
        // the stock's bottom leaves no thicker tab.
        let tab_top = target.max(b.plan.stock.min[2]) + height;
        if tab_top >= cutting_top {
            return Err(format!(
                "Tab height must be less than the {:.2} mm the contour cuts down through the stock.",
                cutting_top - target.max(b.plan.stock.min[2])
            ));
        }
        Some(Tabs {
            count: count as usize,
            width,
            // The tool's centre rises over the tab and its own diameter.
            lift: width + 2. * b.plan.radius,
            top: tab_top,
        })
    } else {
        None
    };
    if tabs.is_none()
        && !holes
        && target <= b.plan.stock.min[2] + 0.001
        && let Some(loose) = crate::sheet::loose_part(setup)
    {
        return Err(loose.into());
    }
    let loops: Vec<Vec<[f32; 2]>> = paths
        .iter()
        .map(|path| {
            let mut points: Vec<_> = path.iter().map(|p| [p.0 as f32, p.1 as f32]).collect();
            // Solid is inside the outer profile: CW spindle climb travels CW.
            // Round a hole the solid is outside: climb travels anticlockwise.
            if (climb == b.plan.spindle_clockwise) != holes {
                points.reverse();
            }
            if let Some(&first) = points.first() {
                points.push(first);
            }
            points
        })
        .collect();
    let mut spans: Vec<Vec<(f32, f32)>> = vec![];
    if let Some(tabs) = &tabs {
        let stock = b.plan.stock;
        let through = target <= stock.min[2] + 0.001;
        // Past the cut's outer edge, where the tabs join the stock.
        let edge = 2. * b.plan.radius + radial + (b.plan.radius * 0.01).max(0.02);
        // From the tool's centre: its radius, then a 2 mm wall of stock,
        // enough to stay standing in the vise or under the clamps.
        let reach = b.plan.radius + WALL;
        let (mut loose, mut all) = (0, 0);
        for points in &loops {
            let outline = Outline::new(points);
            if tabs.count as f32 * tabs.lift >= outline.length() {
                return Err(format!(
                    "{} tabs {:.2} mm wide do not fit around a {:.2} mm outline: each takes its width plus the tool's diameter. Use fewer or narrower tabs.",
                    tabs.count,
                    tabs.lift - 2. * b.plan.radius,
                    outline.length()
                ));
            }
            // The part is inside the outline: on its left when it runs
            // anticlockwise, so the stock is on the right.
            let anticlockwise = points
                .windows(2)
                .map(|w| w[0][0] * w[1][1] - w[1][0] * w[0][1])
                .sum::<f32>()
                > 0.;
            let placed = outline.spans(tabs)?;
            for &(start, end) in &placed {
                let (c, d) = outline.at((start + end) / 2.);
                let out = if anticlockwise {
                    [d[1], -d[0]]
                } else {
                    [-d[1], d[0]]
                };
                let q = [c[0] + out[0] * reach, c[1] + out[1] * reach];
                all += 1;
                loose += usize::from(
                    !(stock.min[0] < q[0]
                        && q[0] < stock.max[0]
                        && stock.min[1] < q[1]
                        && q[1] < stock.max[1]),
                );
            }
            spans.push(placed);
        }
        if through && loose > 0 {
            return Err(format!(
                "{loose} of {all} tabs join the part to no stock: the cut runs past the stock's edge there, so the part still comes loose. Leave at least {:.1} mm of stock beside the part (stock to leave, the tool's diameter and a {WALL} mm wall), or use a narrower tool.",
                // Rounded up, so that the margin it names is enough.
                ((edge + WALL) * 10.).ceil() / 10.
            ));
        }
    }
    crate::progress::expect(levels);
    for level in 1..=levels {
        crate::progress::report(level)?;
        let z = (cutting_top - level as f32 * down).max(target);
        for (i, points) in loops.iter().enumerate() {
            let points = match &tabs {
                Some(tabs) if z < tabs.top => tabbed(points, z, tabs, &spans[i]),
                _ => points.iter().map(|p| [p[0], p[1], z]).collect(),
            };
            for _ in 0..=if level == levels { finish as usize } else { 0 } {
                b.stroke_plunging(&points)?;
            }
        }
    }
    b.finish()
}
/// How near a box side must be to a stock face to be open, and how far the
/// cut may touch the part, mm.
pub(crate) const ON: f32 = 0.001;
/// A Box's block in the stock frame: its keys (`BOX_FIELDS`, work frame)
/// moved by the work zero `zero`, as (low corner, high corner).
pub(crate) fn box_block(op: &Operation, zero: [f32; 3]) -> Result<([f32; 3], [f32; 3]), String> {
    let [x0, x1, y0, y1, z1, z0] = crate::model::BOX_FIELDS.map(|k| number(op, k));
    Ok((
        [x0? + zero[0], y0? + zero[1], z0? + zero[2]],
        [x1? + zero[0], y1? + zero[1], z1? + zero[2]],
    ))
}
/// Which sides of the block `low`..`high` are open on the stock `s`, as
/// [low side, high side] per axis: a side on or beyond the stock's face.
/// The others, and a floor (`[2][0]`) above the stock's bottom, are closed
/// and keep the allowance; the top keeps none whatever it is.
pub(crate) fn open_sides(low: [f32; 3], high: [f32; 3], s: &Bounds) -> [[bool; 2]; 3] {
    std::array::from_fn(|k| [low[k] <= s.min[k] + ON, high[k] >= s.max[k] - ON])
}
/// What a Box cuts of the block `low`..`high` on the stock `s`: the block
/// inside the stock, its closed sides and floor (`open_sides`) moved in by
/// `allowance`, as (low corner, high corner).
pub(crate) fn box_cut(
    low: [f32; 3],
    high: [f32; 3],
    s: &Bounds,
    allowance: f32,
) -> ([f32; 3], [f32; 3]) {
    let open = open_sides(low, high, s);
    let keep = |open: bool| if open { 0. } else { allowance };
    (
        std::array::from_fn(|k| low[k].max(s.min[k]) + keep(open[k][0])),
        std::array::from_fn(|k| high[k].min(s.max[k]) - if k == 2 { 0. } else { keep(open[k][1]) }),
    )
}
/// Whether triangle `t` meets box `b`: no separating axis among the box's
/// faces, the triangle's plane and the nine edge crossings.
fn meets(t: &[[f32; 3]; 3], b: &Bounds) -> bool {
    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let cross = |a: [f32; 3], b: [f32; 3]| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let half: [f32; 3] = std::array::from_fn(|k| (b.max[k] - b.min[k]) / 2.);
    let v = t.map(|p| std::array::from_fn::<f32, 3, _>(|k| p[k] - (b.min[k] + b.max[k]) / 2.));
    let edges: [[f32; 3]; 3] =
        std::array::from_fn(|i| std::array::from_fn(|k| v[(i + 1) % 3][k] - v[i][k]));
    let apart = |axis: [f32; 3]| {
        let p = v.map(|q| dot(q, axis));
        let reach: f32 = (0..3).map(|k| half[k] * axis[k].abs()).sum();
        p.iter().copied().fold(f32::INFINITY, f32::min) > reach
            || p.iter().copied().fold(f32::NEG_INFINITY, f32::max) < -reach
    };
    let units: [[f32; 3]; 3] =
        std::array::from_fn(|i| std::array::from_fn(|k| (i == k) as u8 as f32));
    !(units.iter().any(|&u| apart(u))
        || apart(cross(edges[0], edges[1]))
        || units
            .iter()
            .any(|&u| edges.iter().any(|&e| apart(cross(u, e)))))
}
/// A Box on a manual mill: the block `x_min`..`z_bottom`, in the setup's work
/// frame (`post::datum` on the stock the setup starts from), cleared in rows
/// along its longer side, every move along one axis. Its sides on or beyond
/// the stock's faces are open; the others, and a floor above the stock's
/// bottom, keep `allowance`. Rows start at the open side and step toward the
/// uncut material, which stays on the left of the feed (conventional, M3).
fn boxed(setup: &Setup, op: &Operation, tool: &Tool, mesh: &Mesh) -> Result<Plan, String> {
    if !crate::manual::is_manual(&setup.machine) {
        return Err("Box runs on a manual mill.".into());
    }
    let mut b = Builder::new(setup, op, tool, mesh)?;
    if get(&tool.p, "type") != "Flat end mill" {
        return Err("Box needs a flat end mill.".into());
    }
    let (s, r) = (b.plan.stock, b.plan.radius);
    let zero = crate::post::datum(setup, s, mesh)?;
    let (low, high) = box_block(op, zero)?;
    let allowance = number(op, "allowance")?;
    let stepover = number(op, "stepover")?;
    if (0..3).any(|k| low[k] >= high[k]) || allowance < 0. || !(10. ..=100.).contains(&stepover) {
        return Err("Give the box a positive size, a stock to leave of 0 or more and a stepover of 10–100 %.".into());
    }
    // 1. Open sides, and the cut inside the stock and the allowance.
    let open = open_sides(low, high, &s);
    let (lo, hi) = box_cut(low, high, &s, allowance);
    if lo[2] >= hi[2] - ON || (0..2).any(|k| lo[k] >= hi[k]) {
        return Err(
            "The box leaves nothing to cut inside the stock and the stock to leave.".into(),
        );
    }
    // 2. Passes along the longer side.
    let size = |k: usize| high[k].min(s.max[k]) - low[k].max(s.min[k]);
    let (p, c) = if size(0) >= size(1) { (0, 1) } else { (1, 0) };
    // The stepdown: given, or what the spindle takes at a full stepover,
    // at the operation's rpm with the tool's flutes. The project's material
    // does not reach `generate`: it is counted as steel, the hardest common
    // group but stainless. Rows cut one stepover wide, except the first row
    // of a box closed on both cross-axis sides: it starts inside the stock
    // and cuts a full slot, the tool's whole diameter, so the depth is
    // sized for that width.
    let width = stepover / 100. * 2. * r;
    let down = if get(&op.p, "stepdown").trim().is_empty() {
        let rpm = number(op, "rpm").ok().filter(|n| *n > 0.);
        let flutes = num(&tool.p, "flutes").filter(|z| *z >= 1.);
        let (Some(rpm), Some(flutes)) = (rpm, flutes) else {
            return Err("An automatic stepdown needs a positive rpm and the tool's flutes.".into());
        };
        let slot = open[c] == [false, false];
        let cut_width = if slot { 2. * r } else { width };
        crate::forces::power_stepdown(&setup.machine, None, 2. * r, cut_width, b.feed, rpm, flutes)
            .min(b.plan.flute_length)
    } else {
        number(op, "stepdown")?
    };
    if !(down > 0. && down <= b.plan.flute_length) {
        return Err("Use a stepdown within the flute length.".into());
    }
    // Equal steps of at most `by` from `from` (left out) to exactly `to`.
    let steps = |from: f32, to: f32, by: f32| -> Vec<f32> {
        let n = ((to - from).abs() / by - 0.0001)
            .ceil()
            .clamp(1., CEILING as f32) as usize;
        (1..=n)
            .map(|i| {
                if i == n {
                    to
                } else {
                    from + (to - from) * i as f32 / n as f32
                }
            })
            .collect()
    };
    // 4. Rows from the open side (else −) toward the closed side (else +).
    let side = open[c];
    let (first, last) = match side {
        [false, true] => ((hi[c] - width + r).max(hi[c]), lo[c] + r),
        [false, false] => (lo[c] + r, hi[c] - r),
        _ if side[1] => (
            (lo[c] + width - r).min(lo[c]),
            (hi[c] - width + r).max(hi[c]),
        ),
        _ => ((lo[c] + width - r).min(lo[c]), hi[c] - r),
    };
    let toward = if side == [false, true] { -1. } else { 1. };
    if (last - first) * toward < -ON && side == [false, false] {
        return Err("The box is narrower than the tool after the stock to leave.".into());
    }
    let rows = if (last - first) * toward > ON {
        [vec![first], steps(first, last, width)].concat()
    } else {
        vec![last]
    };
    // Conventional with M3: the uncut material (toward) on the left of the
    // feed. Left of +X is +Y, left of +Y is −X.
    let forward = if p == 0 { toward } else { -toward };
    let ends = open[p];
    let near = [
        if ends[0] {
            s.min[p] - r - 2.
        } else {
            lo[p] + r
        },
        if ends[1] {
            s.max[p] + r + 2.
        } else {
            hi[p] - r
        },
    ];
    if near[1] < near[0] {
        return Err("The box is shorter than the tool after the stock to leave.".into());
    }
    let (start, end, outside) = if forward > 0. {
        (near[0], near[1], ends[0])
    } else {
        (near[1], near[0], ends[1])
    };
    // 6. The cut, from the floor up through the stock top, clear of the part.
    let mut cut = Bounds {
        min: [0., 0., lo[2] + ON],
        max: [0., 0., s.max[2]],
    };
    for (k, a, z) in [(p, start, end), (c, rows[0], rows[rows.len() - 1])] {
        cut.min[k] = (a.min(z) - r).max(s.min[k]) + ON;
        cut.max[k] = (a.max(z) + r).min(s.max[k]) - ON;
    }
    if let Some(t) = Geometry::new(setup, mesh)
        .triangles
        .iter()
        .find(|t| meets(&t.p, &cut))
    {
        let at: [f32; 3] = std::array::from_fn(|k| {
            let (a, z) =
                t.p.iter()
                    .fold((f32::INFINITY, f32::NEG_INFINITY), |(a, z), q| {
                        (a.min(q[k]), z.max(q[k]))
                    });
            (a.max(cut.min[k]) + z.min(cut.max[k])) / 2. - zero[k]
        });
        return Err(format!(
            "The box cuts into the part at X{:.2}, Y{:.2}, Z{:.2}.",
            at[0], at[1], at[2]
        ));
    }
    // 3. Levels from the highest material left in the cut, whatever `z_top`
    // says: the flutes take all of it, so no level is deeper than `stepdown`.
    let top = crate::rest::start(setup, cut, lo[2], s.max[2], 0.);
    b.plan.rest_top = (top < s.max[2] - 0.0001).then_some(top);
    let levels = steps(top, lo[2], down);
    // 5. Each row: up, along, across, down, a pause to set the axes, one
    // feed, up. A row that starts inside the stock does not go up between
    // rows of a level: one plunge at the level's first row; after each
    // pass the tool backs off the wall it just cut (at most 0.2 mm, never
    // past the stock to leave), returns along the cleared row and feeds
    // across into the next row.
    let pause = num(&setup.machine.p, "pass_time")
        .filter(|t| *t >= 0.)
        .unwrap_or(0.);
    let rapid = b.rapid;
    // The room behind a row, away from the uncut material, before the cut's
    // edge on that side: unbounded when that side is open.
    let behind = usize::from(toward < 0.);
    let room = |row: f32| match open[c][behind] {
        true => f32::INFINITY,
        false => ((row - toward * r) - [lo[c], hi[c]][behind]) * toward,
    };
    for &z in &levels {
        for (i, &row) in rows.iter().enumerate() {
            let mut at = b.pos;
            if i > 0 && !outside {
                at[c] -= toward * room(at[c]).clamp(0., 0.2);
                b.push(at, Motion::Rapid, false, rapid)?;
                at[p] = start;
                b.push(at, Motion::Rapid, false, rapid)?;
                at[c] = row;
                b.push(at, Motion::Feed, false, b.feed)?;
            } else {
                at[2] = b.retract;
                b.push(at, Motion::Rapid, false, rapid)?;
                at[p] = start;
                b.push(at, Motion::Rapid, false, rapid)?;
                at[c] = row;
                b.push(at, Motion::Rapid, false, rapid)?;
                at[2] = z;
                if outside {
                    b.push(at, Motion::Rapid, false, rapid)?;
                } else {
                    b.push(at, Motion::Feed, false, b.feed * 0.25)?;
                }
            }
            b.dwell(pause);
            at[p] = end;
            b.push(at, Motion::Feed, true, b.feed)?;
            if outside || i + 1 == rows.len() {
                at[2] = b.retract;
                b.push(at, Motion::Rapid, false, rapid)?;
            }
        }
    }
    b.plan.milling_direction = "Conventional";
    b.plan.spindle_clockwise = true;
    b.plan.target = lo[2];
    b.plan.levels = levels.len();
    b.finish()
}
/// The stock, mm, a tab must reach past the cut's outer edge.
const WALL: f32 = 2.;
/// How much narrower than a Holes contour's smallest hole a hole may read
/// and still be cut, mm: a round hole's chords lie inside its circle.
const HOLE_MIN_SLACK: f32 = 0.05;
/// An outside Contour's holding tabs: `count` on each outline, evenly
/// spaced, over each of which the tool's centre rises to `top` along
/// `lift` mm of its path.
struct Tabs {
    count: usize,
    /// Each tab's own width, at its narrowest.
    width: f32,
    lift: f32,
    top: f32,
}
/// How far a tab's stretch of the path may turn, over the tab and its
/// margins: a straight side does not, an arc of radius R does by its length
/// over R, a corner by its angle.
const TAB_TURN: f32 = 0.5;
/// The path either side of a tab that must not turn either, mm.
const TAB_MARGIN: f32 = 1.;
/// The least path between two tabs, mm.
const TAB_GAP: f32 = 1.;
/// The step along the path at which tab centres are tried, mm.
const TAB_STEP: f32 = 0.25;
/// A closed tool path (its last point its first) and how far along it
/// each of its points is, mm.
struct Outline<'a> {
    points: &'a [[f32; 2]],
    along: Vec<f32>,
}
impl<'a> Outline<'a> {
    fn new(points: &'a [[f32; 2]]) -> Self {
        let mut along = vec![0f32];
        for w in points.windows(2) {
            let step = ((w[1][0] - w[0][0]).powi(2) + (w[1][1] - w[0][1]).powi(2)).sqrt();
            along.push(along.last().unwrap() + step);
        }
        Self { points, along }
    }
    fn length(&self) -> f32 {
        *self.along.last().unwrap()
    }
    /// How much the path turns at each of its points, radians, as a running
    /// sum along it: `turned[i]` is the turning at points 1..=i.
    fn turned(&self) -> Vec<f32> {
        let p = self.points;
        let mut sum = vec![0f32; p.len()];
        for i in 1..p.len().saturating_sub(1) {
            let (a, b) = (
                [p[i][0] - p[i - 1][0], p[i][1] - p[i - 1][1]],
                [p[i + 1][0] - p[i][0], p[i + 1][1] - p[i][1]],
            );
            let turn = (a[0] * b[1] - a[1] * b[0]).atan2(a[0] * b[0] + a[1] * b[1]);
            sum[i] = sum[i - 1] + turn.abs();
        }
        if let Some(last) = sum.len().checked_sub(1).filter(|&l| l > 0) {
            sum[last] = sum[last - 1];
        }
        sum
    }
    /// Where each of `tabs` starts and ends along the path, placed
    /// automatically: each on a stretch of the path that turns by at most
    /// `TAB_TURN` over the tab and `TAB_MARGIN` either side of it (a
    /// straight side, or an arc of a large enough radius; never a corner, a
    /// short segment between corners or a tight arc), as near as such
    /// stretches allow to centres spaced evenly half a spacing from the
    /// start, `TAB_GAP` apart at least, none spanning the start. Refused
    /// when fewer fit than asked.
    fn spans(&self, tabs: &Tabs) -> Result<Vec<(f32, f32)>, String> {
        let length = self.length();
        let half = tabs.lift / 2. + TAB_MARGIN;
        let turned = self.turned();
        // The turning strictly inside the stretch `from`..`to`.
        let turning = |from: f32, to: f32| {
            let i = self.along.partition_point(|&a| a <= from);
            let j = self.along.partition_point(|&a| a < to);
            if j <= i {
                0.
            } else {
                turned[j - 1] - if i == 0 { 0. } else { turned[i - 1] }
            }
        };
        let steps = ((length - 2. * half) / TAB_STEP).floor().max(-1.) as i64;
        let fits: Vec<f32> = (0..=steps)
            .map(|k| half + k as f32 * TAB_STEP)
            .filter(|&c| turning(c - half, c + half) <= TAB_TURN)
            .collect();
        let spacing = length / tabs.count as f32;
        let mut centres: Vec<f32> = vec![];
        for i in 0..tabs.count {
            let ideal = (i as f32 + 0.5) * spacing;
            let best = (fits.iter().copied())
                .filter(|c| centres.iter().all(|o| (c - o).abs() >= tabs.lift + TAB_GAP))
                .min_by(|a, b| (a - ideal).abs().total_cmp(&(b - ideal).abs()));
            match best {
                Some(c) => centres.push(c),
                None => {
                    return Err(format!(
                        "Only {} of {} tabs {:.2} mm wide fit on the straight sides and gentle curves of a {:.2} mm outline: a tab needs {:.2} mm of it turning by {:.0}° at most, away from corners and short segments. Use fewer or narrower tabs.",
                        centres.len(),
                        tabs.count,
                        tabs.width,
                        length,
                        2. * half,
                        TAB_TURN.to_degrees()
                    ));
                }
            }
        }
        centres.sort_by(f32::total_cmp);
        Ok(centres
            .into_iter()
            .map(|c| (c - tabs.lift / 2., c + tabs.lift / 2.))
            .collect())
    }
    /// The point `s` mm along the path, and the way the path runs there
    /// (a unit vector).
    fn at(&self, s: f32) -> ([f32; 2], [f32; 2]) {
        let (points, along) = (self.points, &self.along);
        let i = along
            .partition_point(|&a| a <= s)
            .clamp(1, points.len() - 1);
        let (a, b) = (points[i - 1], points[i]);
        let step = (along[i] - along[i - 1]).max(1e-9);
        let t = ((s - along[i - 1]) / step).clamp(0., 1.);
        (
            [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t],
            [(b[0] - a[0]) / step, (b[1] - a[1]) / step],
        )
    }
}
/// The closed path `points` (its last point its first) at height `z`,
/// rising straight up to the tabs' top at the start of each tab
/// (`Outline::spans`) and dropping straight back down at its end.
fn tabbed(points: &[[f32; 2]], z: f32, tabs: &Tabs, spans: &[(f32, f32)]) -> Vec<[f32; 3]> {
    let outline = Outline::new(points);
    let along = &outline.along;
    // Along the path: each vertex (None), each tab's start (Some(true))
    // and its end (Some(false)).
    let mut events: Vec<(f32, Option<bool>, usize)> = along
        .iter()
        .enumerate()
        .map(|(i, &s)| (s, None, i))
        .collect();
    for &(start, end) in spans {
        events.extend([(start, Some(true), 0), (end, Some(false), 0)]);
    }
    events.sort_by(|a, b| a.0.total_cmp(&b.0));
    let (mut out, mut up) = (vec![], false);
    for (s, edge, i) in events {
        match edge {
            None => out.push([points[i][0], points[i][1], if up { tabs.top } else { z }]),
            Some(rise) => {
                let ([x, y], _) = outline.at(s);
                let (from, to) = if rise { (z, tabs.top) } else { (tabs.top, z) };
                out.extend([[x, y, from], [x, y, to]]);
                up = rise;
            }
        }
    }
    out
}
#[derive(Debug, Clone)]
pub struct Hole {
    pub center: [f32; 2],
    /// Narrowest diameter down to the bottom: the largest drill that fits. A
    /// bore whose corridor is a ring of its own chords is read at their ends,
    /// its nominal size, not at the chords' midpoints a sagitta inside it.
    /// For Helical milling, the bore a flat end mill fits down to `bottom`.
    pub diameter: f32,
    pub opening: f32,
    /// How far the opening's chords lie inside its circle, R(1 − cos(π/n))
    /// for its n vertices at radius R. A drill fits up to `diameter` plus
    /// twice this.
    pub sagitta: f32,
    pub top: f32,
    /// The height of its centre's floor; for Helical milling, of the flat
    /// floor a flat end mill stops on.
    pub bottom: f32,
    /// The bottom is the model's bottom: nothing of the part is under it.
    pub through: bool,
    /// The cone its opening narrows down in, when it is countersunk.
    pub countersink: Option<Countersink>,
    /// The thread modeled in it.
    pub thread: Option<crate::thread::Thread>,
    /// The A angle (degrees) it is read at when an operation of an indexed
    /// setup of several angles lists it (`Document::holes_listed`): its
    /// centre is in the setup's frame at that angle. None otherwise.
    pub a: Option<f32>,
}
impl Hole {
    /// The widest drill the hole takes: its bore, which a drill may fill
    /// out to its tessellation's vertices; a threaded hole's tap drill,
    /// which cuts a little of its thread's crests, up to its major diameter
    /// less 0.9 of a pitch (Ø5.1 for M6×1, whose tap drill is Ø5).
    pub fn widest_drill(&self) -> f32 {
        let bore = self.diameter + 2. * self.sagitta + 0.01;
        self.thread
            .as_ref()
            .map_or(bore, |t| bore.max(t.major - 0.9 * t.pitch))
    }
}
/// The surfaces facing up at height `z` in `setup`'s frame (mm): the
/// plane a hole opening at `z` is picked by, as its rim.
pub fn rim_surfaces(setup: &Setup, mesh: &Mesh, z: f32) -> Vec<usize> {
    let placement = crate::stock::Placement::new(mesh, Some(setup));
    let (angles, _) = placement.parts();
    let scale = mesh.display_scale();
    (mesh.surfaces.iter().enumerate())
        .filter(|(_, s)| crate::mesh::rotation(s.normal, angles)[2] > 0.999)
        .filter(|(_, s)| (placement.transform(s.center)[2] / scale - z).abs() < 0.02)
        .map(|(id, _)| id)
        .collect()
}
/// A countersink: a cone from a hole's opening, `Hole::opening` wide at the
/// face, straight down to the wall or floor under it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Countersink {
    /// Its included angle, degrees: 82° for an inch flat head screw's, 90°
    /// for a metric one's.
    pub angle: f32,
    /// From the face down to where the cone ends.
    pub depth: f32,
}
/// Circular holes bounded by the selected horizontal rim, and the round holes
/// picked by their walls, verified against mesh rays.
/// Noncircular openings and boss outlines are rejected, never converted to drills.
pub fn holes(setup: &Setup, op: &Operation, mesh: &Mesh) -> Result<Vec<Hole>, String> {
    let g = Geometry::with_threads(setup, mesh);
    // The Drill geometry list and the part report ask without `simulation::generate`'s check.
    if crate::simulation::exceeds_coordinate_limit(&g.bounds) {
        return Err(crate::simulation::COORDINATE_LIMIT_MESSAGE.into());
    }
    let selected = g.selected(op);
    if selected.is_empty() {
        return Err(
            "Select the horizontal face containing the hole openings, or click the holes.".into(),
        );
    }
    // A selected rim face's round openings, and each round hole picked by
    // its walls (as a Pocket picks holes), once each.
    let walls: std::collections::HashSet<usize> = (op.surfaces.iter())
        .flat_map(|s| s.ids.iter().copied())
        .filter(|&id| crate::pocket::is_wall(setup, mesh, id))
        .collect();
    let rim: Vec<&Triangle> = (selected.iter().copied())
        .filter(|t| t.feature.is_none_or(|id| !walls.contains(&id)))
        .collect();
    let mut holes = if rim.is_empty() {
        vec![]
    } else {
        round_holes(&g, &rim)?
    };
    if !walls.is_empty() {
        let rims = rims(&g);
        // At several A angles a hole over a cavity may stop through its own
        // wall (`stops_at_own_wall`).
        let own_wall = crate::indexed::holes_at_angles(op, setup);
        for picked in crate::pocket::holes(setup, &walls, mesh)? {
            if let Some(h) = drill_hole(&g, &picked, &rims, own_wall)
                && !holes.iter().any(|o| same_centre(o, &h))
            {
                holes.push(h);
            }
        }
        holes.sort_by(|a, b| {
            a.center[0]
                .total_cmp(&b.center[0])
                .then(a.center[1].total_cmp(&b.center[1]))
        });
    }
    // Helical milling's flat end mill: each hole as it mills it, the bore it
    // fits down to the flat floor it stops on.
    if op.helical() {
        for h in &mut holes {
            (h.bottom, h.diameter) = milled_bore(&g, h);
        }
    }
    if holes.is_empty() {
        Err("No unobstructed circular holes were found in the selected face. Select the top rim containing hole openings.".into())
    } else {
        Ok(holes)
    }
}
/// The round holes whose openings are loops in the horizontal face
/// `selected` (of `g`'s triangles), going down from it along Z, sorted by X
/// then Y.
fn round_holes(g: &Geometry, selected: &[&Triangle]) -> Result<Vec<Hole>, String> {
    let top = selected[0].p[0][2];
    let loops: Vec<Polygon> = region(
        (selected.iter())
            .map(|t| t.p.iter().map(|v| (v[0] as f64, v[1] as f64)).collect())
            .collect(),
    )?
    .into();
    let mut holes: Vec<Hole> = (loops.iter())
        .filter(|p| area(p) < 0. && p.len() >= 8)
        .filter_map(|path| round_hole(g, path, top))
        .collect();
    holes.sort_by(|a, b| {
        a.center[0]
            .total_cmp(&b.center[0])
            .then(a.center[1].total_cmp(&b.center[1]))
    });
    Ok(holes)
}
/// Whether two holes are one: their centres within 0.05 mm.
fn same_centre(a: &Hole, b: &Hole) -> bool {
    (a.center[0] - b.center[0]).hypot(a.center[1] - b.center[1]) <= 0.05
}
/// The hole a Pocket's wall click picks (`pocket::Hole`) read as a drill
/// reads a rim's opening. Its walls' middle is found from their top edge in
/// the part (the corners of its wall triangles at their highest). Then, of
/// the part's flat faces facing up from that height (`rims`: each height,
/// lowest first, with its loops, by `rim_loops`), the first with a loop
/// round that middle is its rim, as a selected rim face is (over a
/// countersink, the face round its top): that loop at that height, when it
/// is round. Else that top edge, at its height. None unless the loop read
/// is round. (The pocket's own outline is cut from a section and offset, a
/// few hundredths inside the part's edge, and not round where a lip is over
/// it.)
fn drill_hole(
    g: &Geometry,
    hole: &crate::pocket::Hole,
    rims: &[(f32, Vec<Polygon>)],
    own_wall: bool,
) -> Option<Hole> {
    let corners: Vec<[f32; 3]> = (g.triangles.iter())
        .filter(|t| t.feature.is_some_and(|id| hole.walls.contains(&id)))
        .flat_map(|t| t.p)
        .collect();
    let top = corners
        .iter()
        .map(|p| p[2])
        .fold(f32::NEG_INFINITY, f32::max);
    let mut ring: Vec<(f64, f64)> = vec![];
    for p in corners.iter().filter(|p| p[2] >= top - 0.01) {
        let q = (p[0] as f64, p[1] as f64);
        if !ring.iter().any(|r| (r.0 - q.0).hypot(r.1 - q.1) < 1e-4) {
            ring.push(q);
        }
    }
    if ring.len() < 8 {
        return None;
    }
    let n = ring.len() as f64;
    let middle = (
        ring.iter().map(|q| q.0).sum::<f64>() / n,
        ring.iter().map(|q| q.1).sum::<f64>() / n,
    );
    ring.sort_by(|a, b| {
        let angle = |q: &(f64, f64)| (q.1 - middle.1).atan2(q.0 - middle.0);
        angle(a).total_cmp(&angle(b))
    });
    // The first rim up with a hole round the walls' middle (its smallest),
    // and on up while the next rim's is round about the same centre: a
    // counterbore over a bore is one hole, read from its top as a selected
    // top face reads it, whichever of its walls was clicked.
    let mut found: Option<Hole> = None;
    for (z, loops) in rims.iter().filter(|(z, _)| *z >= top - 0.005) {
        let Some(l) = (loops.iter())
            .filter(|l| area(l) < 0. && l.len() >= 8 && encloses(l, middle))
            .min_by(|a, b| area(b).total_cmp(&area(a)))
        else {
            continue;
        };
        match (round_hole(g, l, *z), &found) {
            (Some(h), None) => found = Some(h),
            (Some(h), Some(below)) if same_centre(&h, below) => found = Some(h),
            _ => break,
        }
    }
    let mut found = found.or_else(|| round_hole(g, &ring, top))?;
    // With `own_wall`, in an indexed setup of several A angles, a hole that
    // opens into a cavity under its walls (a radial hole in a tube, into its
    // bore) and whose plunge would go on into the part past it is drilled
    // through its own wall only (`stops_at_own_wall`). Else, and everywhere
    // else, it goes on as main drills it: through both walls of a tube
    // whose far wall has a hole opposite, as a cross-hole in a vise is.
    let bottom = corners.iter().map(|p| p[2]).fold(f32::INFINITY, f32::min);
    let r = corners
        .iter()
        .map(|p| (p[0] - found.center[0]).hypot(p[1] - found.center[1]))
        .sum::<f32>()
        / corners.len() as f32;
    if own_wall && found.bottom < bottom - 0.05 && stops_at_own_wall(g, found.center, r, bottom) {
        (found.bottom, found.through) = (bottom, true);
    }
    Some(found)
}
/// Whether a hole of radius `r` at `centre` whose walls end at the height
/// `z` stops just through its own wall in an indexed setup of several A
/// angles: it opens into a cavity under them (`into_cavity`), and a plunge
/// on across it would cut the part past it (`part_under_disc`), where no
/// hole opposite at least as wide lets the drill out.
pub(crate) fn stops_at_own_wall(g: &Geometry, centre: [f32; 2], r: f32, z: f32) -> bool {
    into_cavity(g, centre, r, z) && part_under_disc(g, centre, r, z)
}
/// Whether anything of the part lies under the disc of a hole of radius `r`
/// at `centre` below the height `z` (at its centre and 16 points 0.9 `r`
/// from it): what a plunge on through it would cut. Nothing there under a
/// hole whose far side has a hole opposite at least as wide.
pub(crate) fn part_under_disc(g: &Geometry, centre: [f32; 2], r: f32, z: f32) -> bool {
    std::iter::once(centre)
        .chain((0..16).map(|k| {
            let t = k as f32 * std::f32::consts::PI / 8.;
            [centre[0] + 0.9 * r * t.cos(), centre[1] + 0.9 * r * t.sin()]
        }))
        .any(|p| {
            (g.triangles.iter())
                .filter_map(|tri| plane_height(&tri.p, p))
                .any(|h| h < z - 0.05)
        })
}
/// The ring of a hole's walls `walls` in `g`: the middle and the mean
/// radius of their triangles' corners, and their top and bottom. None
/// without a triangle of them.
pub(crate) fn wall_ring(g: &Geometry, walls: &[usize]) -> Option<([f32; 2], f32, f32, f32)> {
    let corners: Vec<[f32; 3]> = (g.triangles.iter())
        .filter(|t| t.feature.is_some_and(|id| walls.contains(&id)))
        .flat_map(|t| t.p)
        .collect();
    if corners.is_empty() {
        return None;
    }
    let n = corners.len() as f32;
    let centre = [0, 1].map(|k| corners.iter().map(|p| p[k]).sum::<f32>() / n);
    let r = (corners.iter())
        .map(|p| (p[0] - centre[0]).hypot(p[1] - centre[1]))
        .sum::<f32>()
        / n;
    let top = corners
        .iter()
        .map(|p| p[2])
        .fold(f32::NEG_INFINITY, f32::max);
    let bottom = corners.iter().map(|p| p[2]).fold(f32::INFINITY, f32::min);
    Some((centre, r, top, bottom))
}
/// Whether a hole of radius `r` at `centre` whose walls end at the height
/// `z` opens there into a cavity: nothing solid under it (at 16 points 0.8
/// `r` from its centre, the first face under `z` is no underside of the
/// part), and the part past the cavity round it (`part_past_gap`), as a
/// tube's bore is under a hole through its wall. Not a counterbore over its
/// bore, whose shoulder is solid under it.
fn into_cavity(g: &Geometry, centre: [f32; 2], r: f32, z: f32) -> bool {
    let solid_under = (0..16).any(|k| {
        let t = k as f32 * std::f32::consts::PI / 8.;
        let p = [centre[0] + 0.8 * r * t.cos(), centre[1] + 0.8 * r * t.sin()];
        (g.triangles.iter())
            .filter_map(|tri| plane_height(&tri.p, p).map(|h| (h, tri)))
            .filter(|(h, _)| *h < z - 0.05)
            .max_by(|a, b| a.0.total_cmp(&b.0))
            .is_some_and(|(_, tri)| up_of(tri) < -0.1)
    });
    !solid_under && part_past_gap(g, centre, r, z, false)
}
/// The Z of triangle `t`'s unit normal: up for a face of the part facing up.
fn up_of(t: &Triangle) -> f32 {
    let [a, b, c] = t.p;
    let (u, v) = (
        [b[0] - a[0], b[1] - a[1], b[2] - a[2]],
        [c[0] - a[0], c[1] - a[1], c[2] - a[2]],
    );
    let n = [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ];
    n[2] / (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-12)
}
/// Whether the part has a face past a gap from the height `z` round a hole
/// of radius `r` at `centre` (at 16 points 1.2 `r` from it): with `above`,
/// one facing down over `z`, the underside of something of the part over
/// the hole; else one facing up under `z`, the top of something under it.
pub(crate) fn part_past_gap(g: &Geometry, centre: [f32; 2], r: f32, z: f32, above: bool) -> bool {
    (0..16).any(|k| {
        let t = k as f32 * std::f32::consts::PI / 8.;
        let p = [centre[0] + 1.2 * r * t.cos(), centre[1] + 1.2 * r * t.sin()];
        g.triangles.iter().any(|tri| {
            let Some(h) = plane_height(&tri.p, p) else {
                return false;
            };
            let up = up_of(tri);
            if above {
                h > z + 0.05 && up < -0.1
            } else {
                h < z - 0.05 && up > 0.1
            }
        })
    })
}
/// The heights of the part's flat faces facing up, lowest first, each with
/// its loops (`rim_loops`): where a hole's rim is looked for.
fn rims(g: &Geometry) -> Vec<(f32, Vec<Polygon>)> {
    let mut heights: Vec<f32> = vec![];
    for t in g.triangles.iter().filter(|t| facing_up(t)) {
        let z = t.p[0][2];
        if !heights.iter().any(|h| (h - z).abs() < 0.005) {
            heights.push(z);
        }
    }
    heights.sort_by(f32::total_cmp);
    heights.into_iter().map(|z| (z, rim_loops(g, z))).collect()
}
/// Whether the loop `l` goes round the point `p` (even-odd).
fn encloses(l: &Polygon, p: (f64, f64)) -> bool {
    let mut odd = false;
    for (a, b) in l.iter().zip(l.iter().cycle().skip(1)) {
        if (a.1 > p.1) != (b.1 > p.1) && p.0 < a.0 + (p.1 - a.1) / (b.1 - a.1) * (b.0 - a.0) {
            odd = !odd;
        }
    }
    odd
}
/// Whether triangle `t` is flat and faces up (its corners anticlockwise
/// seen from above).
fn facing_up(t: &Triangle) -> bool {
    let p = &t.p;
    let z = p[0][2];
    let (a, b) = (
        [p[1][0] - p[0][0], p[1][1] - p[0][1]],
        [p[2][0] - p[0][0], p[2][1] - p[0][1]],
    );
    p.iter().all(|q| (q[2] - z).abs() < 0.005) && a[0] * b[1] - a[1] * b[0] > 1e-8
}
/// The loops of the part's faces facing up at the height `z`, as a selected
/// rim face's are read (`round_holes`): its holes are loops running clockwise.
fn rim_loops(g: &Geometry, z: f32) -> Vec<Polygon> {
    let flat: Vec<Vec<(f64, f64)>> = (g.triangles.iter())
        .filter(|t| facing_up(t) && (t.p[0][2] - z).abs() < 0.005)
        .map(|t| t.p.iter().map(|v| (v[0] as f64, v[1] as f64)).collect())
        .collect();
    if flat.is_empty() {
        return vec![];
    }
    region(flat).map_or(vec![], |r| r.into())
}
/// The largest drill, in steps of 0.1 mm, a Drill takes in `hole`: what its
/// bore reads plus twice its opening's sagitta and 0.01 mm, as the drill
/// check allows. What its label in the view says.
pub fn largest_drill(hole: &Hole) -> f32 {
    ((hole.diameter + 2. * hole.sagitta + 0.01) * 10. + 1e-3).floor() / 10.
}
/// Each of `holes` (the part's holes a wall click picks) read as a drill
/// reads it (`drill_hole`, to main's depth): None for one that is not
/// round.
pub fn drill_holes_of(
    setup: &Setup,
    mesh: &Mesh,
    holes: &[crate::pocket::Hole],
) -> Vec<Option<Hole>> {
    let g = Geometry::with_threads(setup, mesh);
    let rims = rims(&g);
    holes
        .iter()
        .map(|h| drill_hole(&g, h, &rims, false))
        .collect()
}
/// The round hole whose opening is the loop `path` at the height `top`,
/// going down from it along Z; None when the loop is not round or nothing
/// a drill fits goes down from it.
fn round_hole(g: &Geometry, path: &Polygon, top: f32) -> Option<Hole> {
    let low = [
        path.iter().map(|p| p.0).fold(f64::INFINITY, f64::min),
        path.iter().map(|p| p.1).fold(f64::INFINITY, f64::min),
    ];
    let high = [
        path.iter().map(|p| p.0).fold(f64::NEG_INFINITY, f64::max),
        path.iter().map(|p| p.1).fold(f64::NEG_INFINITY, f64::max),
    ];
    // The circle through the loop's vertices: its box's centre is off by
    // up to half a chord's sagitta, with the polygon's phase.
    let center = circle_centre(path).unwrap_or(((low[0] + high[0]) / 2., (low[1] + high[1]) / 2.));
    let center = [center.0 as f32, center.1 as f32];
    let distance = |p: &(f64, f64)| {
        ((p.0 as f32 - center[0]).powi(2) + (p.1 as f32 - center[1]).powi(2)).sqrt()
    };
    // The vertex radius: a regular polygon's vertices are all on its circle.
    let mut radius = path.iter().map(distance).sum::<f32>() / path.len() as f32;
    let mut center = center;
    if radius <= 0.
        || path
            .iter()
            .any(|p| (distance(p) - radius).abs() > 0.03f32.max(radius * 0.02))
    {
        // A modeled thread cuts the face in a spiral, between its minor
        // and major diameters round its axis: the hole is round about that.
        let thread = g.threads.iter().find(|t| {
            path.iter().all(|p| {
                let d = (p.0 as f32 - t.centre[0]).hypot(p.1 as f32 - t.centre[1]);
                d >= t.minor / 2. - 0.05 && d <= t.major / 2. + 0.05
            })
        })?;
        center = thread.centre;
        radius = thread.major / 2.;
    }
    // Its chords' midpoints lie this far inside the circle.
    let sagitta = radius * (1. - (std::f32::consts::PI / path.len() as f32).cos());
    let bottom = g.height(center).unwrap_or(g.bounds.min[2]);
    if bottom >= top - 0.01 {
        return None;
    }
    let through = bottom <= g.bounds.min[2] + 0.01;
    // The selected opening may be counterbored. Measure the narrowest
    // vertical corridor above its bottom, instead of treating a shoulder
    // as an obstruction for every smaller drill. A blind hole's drill
    // stops with its point at the bottom, so the floor a 118° point
    // leaves (a cone, a fillet, flat) is not in its way: only what rises
    // above that cone is. Under a hole that goes through the drill goes
    // on, so everything down to the bottom counts there.
    let point = if through {
        0.
    } else {
        // A floor sharper than a 118° point (a 90° one rises 1 per mm)
        // would cross that cone a fraction of a millimetre from the
        // centre, so the clip takes the floor's own cone where that is
        // steeper: the gradient of the floor triangles that hold the
        // centre, taken only where one has a vertex there, the cone's
        // apex, so that a tilted floor keeps the 118° clip. A fan
        // triangle's plane is the cone's slope over cos(π/n), exactly
        // the slope that puts the whole fan and the rings beyond it
        // under the clip.
        let near = 0.05f32.max(radius * 0.01);
        let cone = g
            .triangles
            .iter()
            .filter(|t| {
                plane_height(&t.p, center).is_some_and(|z| (z - bottom).abs() < 0.01)
                    && t.p.iter().any(|p| {
                        ((p[0] - center[0]).powi(2) + (p[1] - center[1]).powi(2)).sqrt() <= near
                    })
            })
            .filter_map(|t| plane_slope(&t.p))
            .fold(0., f32::max);
        31f32.to_radians().tan().max(cone)
    };
    let (corridor, bore) = corridor(g, center, bottom + 0.02, point, radius);
    if corridor < 0.1 {
        return None;
    }
    let diameter = bore * 2.;
    // The thread it holds: one round this centre, inside the hole's depth.
    let thread = g
        .threads
        .iter()
        .find(|t| {
            (t.centre[0] - center[0]).hypot(t.centre[1] - center[1]) < radius.min(0.5)
                && t.from < top + 0.01
                && t.to > bottom - 0.01
        })
        .cloned();
    Some(Hole {
        center,
        diameter,
        opening: radius * 2.,
        sagitta,
        top,
        bottom,
        through,
        countersink: countersink(g, center, top, radius, path.len()),
        thread,
        a: None,
    })
}
/// The countersink of the opening of radius `radius` round `center` in the
/// face at `top`, its rim a loop of `sides` chords: a cone from each chord
/// of the rim down to a wall or a floor, read at its tessellation's
/// vertices, which all lie on it. A rim that drops straight down (a plain
/// hole or a counterbore), rounds over (a fillet, a cone of changing slope)
/// or bevels in two slopes is none.
fn countersink(
    g: &Geometry,
    center: [f32; 2],
    top: f32,
    radius: f32,
    sides: usize,
) -> Option<Countersink> {
    let tolerance = 0.01f32.max(radius * 0.005);
    let out = |p: &[f32; 3]| (p[0] - center[0]).hypot(p[1] - center[1]);
    let rim = |p: &[f32; 3]| (out(p) - radius).abs() < tolerance && (p[2] - top).abs() < tolerance;
    // Sloping faces inside the rim and under it.
    let sloped = |t: &Triangle| {
        plane_slope(&t.p).is_some_and(|s| s > 0.01)
            && t.p
                .iter()
                .all(|p| out(p) < radius + tolerance && p[2] < top + tolerance)
    };
    // Each chord of the rim has a sloping face under it; its other corners
    // give the cone's slope, how far in it comes per mm down.
    let (mut chords, mut slopes) = (0, vec![]);
    for t in g.triangles.iter().filter(|t| sloped(t)) {
        if t.p.iter().filter(|p| rim(p)).count() >= 2 {
            chords += 1;
            slopes.extend(
                (t.p.iter())
                    .filter(|p| !rim(p) && top - p[2] > tolerance)
                    .map(|p| (radius - out(p)) / (top - p[2])),
            );
        }
    }
    if chords < sides || slopes.is_empty() {
        return None;
    }
    let (low, high) = (slopes.iter()).fold((f32::INFINITY, f32::NEG_INFINITY), |(a, b), &s| {
        (a.min(s), b.max(s))
    });
    let angle = 2. * ((low + high) / 2.).atan().to_degrees();
    if 2. * (high.atan() - low.atan()).to_degrees() > 0.5 || !(20. ..=160.).contains(&angle) {
        return None;
    }
    let slope = (low + high) / 2.;
    let on_cone = |p: &[f32; 3]| {
        p[2] < top + tolerance && (out(p) - (radius - (top - p[2]) * slope)).abs() < tolerance
    };
    let cone = |t: &Triangle| sloped(t) && t.p.iter().all(on_cone);
    let end = (g.triangles.iter().filter(|t| cone(t)))
        .flat_map(|t| t.p.iter().map(|p| p[2]))
        .fold(top, f32::min);
    // It ends on a wall or a floor, not on more slope: the corners of its
    // lowest ring are only the cone's, upright faces' and flat ones'.
    let ring = |p: &[f32; 3]| (p[2] - end).abs() < tolerance && on_cone(p);
    let ends_square = (g.triangles.iter())
        .filter(|t| t.p.iter().any(ring) && !cone(t))
        .all(|t| plane_slope(&t.p).is_none_or(|s| s < 0.01));
    (top - end > tolerance && ends_square).then_some(Countersink {
        angle,
        depth: top - end,
    })
}
/// The narrowest vertical corridor round `center` above the height `z`,
/// starting from `radius`: how close to the centre the part comes anywhere
/// above `z`, a drill's point (rising `point` per mm from the axis) taken
/// off it, 0 when the part covers the centre. With it, the distance from
/// the centre of the nearest vertex of the triangle that sets it.
fn corridor(g: &Geometry, center: [f32; 2], z: f32, point: f32, radius: f32) -> (f32, f32) {
    let above = |p: &[f32; 3]| {
        p[2] - point * ((p[0] - center[0]).powi(2) + (p[1] - center[1]).powi(2)).sqrt()
    };
    let mut corridor = radius;
    // Each triangle standing over `z`: its nearest point to the centre, its
    // nearest corner's distance, the angle at the centre between its corners
    // about that near (0 for one), and whether it is upright (a wall's).
    let mut faces: Vec<(f32, f32, f32, bool)> = vec![];
    for triangle in &g.triangles {
        // Each edge is split at its point nearest the centre, where the
        // cone is lowest under it, so that a ledge whose edge passes the
        // centre closer than its ends is not taken for floor.
        let mut outline = vec![];
        for (a, b) in triangle
            .p
            .iter()
            .zip(triangle.p.iter().cycle().skip(1))
            .take(3)
        {
            outline.push(*a);
            let v = [b[0] - a[0], b[1] - a[1]];
            let l2 = v[0] * v[0] + v[1] * v[1];
            if l2 >= 1e-12 {
                let t = ((center[0] - a[0]) * v[0] + (center[1] - a[1]) * v[1]) / l2;
                if t > 0. && t < 1. {
                    outline.push(std::array::from_fn(|k| a[k] + t * (b[k] - a[k])));
                }
            }
        }
        let mut polygon = vec![];
        for (a, b) in outline
            .iter()
            .zip(outline.iter().cycle().skip(1))
            .take(outline.len())
        {
            let (above_a, above_b) = (above(a), above(b));
            if above_a >= z {
                polygon.push((a[0] as f64, a[1] as f64));
            }
            if (above_a >= z) != (above_b >= z) {
                let t = (z - above_a) / (above_b - above_a);
                polygon.push((
                    (a[0] + t * (b[0] - a[0])) as f64,
                    (a[1] + t * (b[1] - a[1])) as f64,
                ));
            }
        }
        if polygon.len() < 2 {
            continue;
        }
        if contains(
            std::slice::from_ref(&polygon),
            (center[0] as f64, center[1] as f64),
        ) {
            return (0., 0.);
        }
        let mut nearest = f32::INFINITY;
        for (a, b) in polygon
            .iter()
            .zip(polygon.iter().cycle().skip(1))
            .take(polygon.len())
        {
            let v = [(b.0 - a.0) as f32, (b.1 - a.1) as f32];
            let l2 = v[0] * v[0] + v[1] * v[1];
            let t = if l2 < 1e-12 {
                0.
            } else {
                (((center[0] - a.0 as f32) * v[0] + (center[1] - a.1 as f32) * v[1]) / l2)
                    .clamp(0., 1.)
            };
            nearest = nearest.min(
                ((center[0] - a.0 as f32 - t * v[0]).powi(2)
                    + (center[1] - a.1 as f32 - t * v[1]).powi(2))
                .sqrt(),
            );
        }
        corridor = corridor.min(nearest);
        let distance = |p: &[f32; 3]| (p[0] - center[0]).hypot(p[1] - center[1]);
        let corners = triangle
            .p
            .iter()
            .map(distance)
            .fold(f32::INFINITY, f32::min);
        let angles: Vec<f32> = (triangle.p.iter())
            .filter(|p| distance(p) <= corners * 1.03 + 0.002 && distance(p) > 1e-4)
            .map(|p| (p[1] - center[1]).atan2(p[0] - center[0]))
            .collect();
        let span = (angles.iter())
            .flat_map(|a| angles.iter().map(move |b| (a - b).abs()))
            .map(|d| d.min(std::f32::consts::TAU - d))
            .fold(0., f32::max);
        faces.push((nearest, corners, span, plane_slope(&triangle.p).is_none()));
    }
    // A face whose middle lies within 8 % of its corners' distance, and that
    // spans no more of the hole's round than its other sides do, is a chord
    // of a round wall of 8 sides or more: the bore is read at its corners,
    // its nominal size, as a drill of that size fits and a mill is sent. A
    // STEP's bore may have coarser sides than its rim, and corners not quite
    // on one circle: the nearest corner sets it. Any other face, a flat or a
    // ledge, is read at its nearest point: a D-flat's ends lie on the round
    // too, but it spans many of its sides. Walls are matched with walls and
    // the rim's faces with the rim's: a wall may have coarser sides.
    let side = |upright: bool| {
        let mut spans: Vec<f32> = (faces.iter())
            .filter(|f| f.3 == upright && f.2 > 1e-4 && f.0 <= corridor * 1.1 + 0.01)
            .map(|f| f.2)
            .collect();
        spans.sort_by(f32::total_cmp);
        spans.get(spans.len() / 2).copied().unwrap_or(0.)
    };
    let sides = [side(false), side(true)];
    let bore = (faces.iter())
        .map(|&(nearest, corners, span, upright)| {
            let side = sides[usize::from(upright)];
            if nearest >= corners * 0.92 - 0.001 && span <= side * 1.5 + 0.02 {
                corners
            } else {
                nearest
            }
        })
        .fold(radius, f32::min);
    (corridor, bore.max(corridor))
}
/// How many hole openings `holes` would examine for this selection (0 on any error).
pub(crate) fn hole_candidates(setup: &Setup, op: &Operation, mesh: &Mesh) -> usize {
    let g = Geometry::new(setup, mesh);
    if g.selected(op).is_empty() {
        return 0;
    }
    g.silhouette(true, op).map_or(0, |region| {
        let loops: Vec<Polygon> = region.into();
        loops
            .iter()
            .filter(|p| area(p) < 0. && p.len() >= 8)
            .count()
    })
}
/// A round hole the part opens on its bottom in a setup, the kind a part
/// drops over a fixture pin by (plan mm): its centre, its narrowest
/// diameter and that of its opening, the height of the face it opens on,
/// and how deep it goes up into the part at its full diameter (to its top
/// when it goes through).
#[derive(Debug, Clone, PartialEq)]
pub struct BottomHole {
    pub center: [f32; 2],
    pub diameter: f32,
    pub opening: f32,
    pub z: f32,
    pub depth: f32,
    pub through: bool,
}
/// The round holes the part (`mesh`) opens on its lowest face in `setup`,
/// going up into it along Z, sorted by X then Y: `holes` as a drill from
/// below would find them in that face.
pub fn bottom_holes(setup: &Setup, mesh: &Mesh) -> Result<Vec<BottomHole>, String> {
    let mut g = Geometry::new(setup, mesh);
    if crate::simulation::exceeds_coordinate_limit(&g.bounds) {
        return Err(crate::simulation::COORDINATE_LIMIT_MESSAGE.into());
    }
    // Upside down, the bottom is the top a drill comes down on; X and Y
    // stay as they are.
    for t in &mut g.triangles {
        for p in &mut t.p {
            p[2] = -p[2];
        }
    }
    let [low, high] = [g.bounds.min[2], g.bounds.max[2]];
    (g.bounds.min[2], g.bounds.max[2]) = (-high, -low);
    let face: Vec<&Triangle> = (g.triangles.iter())
        .filter(|t| t.p.iter().all(|p| (p[2] + low).abs() < 0.01))
        .collect();
    if face.is_empty() {
        return Ok(vec![]);
    }
    Ok(round_holes(&g, &face)?
        .into_iter()
        .map(|h| {
            // A blind hole's depth at its full diameter: its end where a
            // pin's edge would meet it, round its rim (a drill's point or a
            // fillet rises from the centre's depth towards it).
            let r = h.diameter / 2. * 0.95;
            let depth = if h.through {
                h.top - h.bottom
            } else {
                (0..16)
                    .filter_map(|k| {
                        let a = k as f32 * std::f32::consts::TAU / 16.;
                        g.height([h.center[0] + r * a.cos(), h.center[1] + r * a.sin()])
                    })
                    .map(|z| h.top - z)
                    .fold(h.top - h.bottom, f32::min)
            };
            BottomHole {
                center: h.center,
                diameter: h.diameter,
                opening: h.opening,
                z: -h.top,
                depth,
                through: h.through,
            }
        })
        .collect())
}
/// Which of `holes` (`bottom_holes` of `setup`) the part's surface
/// `surface` (`Mesh::surfaces`) is in: a facet of its wall or its far end,
/// not the face it opens on.
pub fn surface_hole(
    setup: &Setup,
    mesh: &Mesh,
    holes: &[BottomHole],
    surface: usize,
) -> Option<usize> {
    let s = mesh.surfaces.get(surface)?;
    let at = Placement::new(mesh, Some(setup))
        .transform(s.center)
        .map(|v| v / mesh.display_scale());
    holes.iter().position(|h| {
        let r = h.diameter.max(h.opening) / 2.;
        (at[0] - h.center[0]).hypot(at[1] - h.center[1]) <= r + 0.1
            && at[2] > h.z + 0.01
            && at[2] <= h.z + h.depth + 0.01
    })
}
/// A Drill of the pin holes in the stock's leftover
/// (`Operation::drills_pin_holes`): a hole for each of the machine's
/// fixture pins where a later setup's Work zero step placed them
/// (`post::drill_pin_points`), going down from the face its pins stand on,
/// which must be up here, `post::PIN_DEPTH_EXTRA` deeper than they stand
/// at their full width (a drill's point on under that).
/// A drill from the pins' diameter to `post::PIN_FIT` over drills them all
/// round, pecking as its cycle says. A flat end mill mills them
/// `post::PIN_CLEARANCE` wider than the pins: pin 1's hole round, on a
/// helix, and a slot `post::SLOT_PLAY` longer than its pin along the line
/// from pin 1 for each of the others, ramping down round it, then a lap at
/// the bottom of each.
fn pin_holes(setup: &Setup, op: &Operation, tool: &Tool, mesh: &Mesh) -> Result<Plan, String> {
    use crate::post::{PIN_CLEARANCE, PIN_DEPTH_EXTRA, PIN_FIT, PIN_WALL, SLOT_PLAY, exact_mm};
    let mut b = Builder::new(setup, op, tool, mesh)?;
    let drill = get(&tool.p, "type") == "Drill";
    if !drill && get(&tool.p, "type") != "Flat end mill" {
        return Err("Choose a drill, or a flat end mill to mill them, for the pin holes.".into());
    }
    let pins = match crate::control::pins_settings(&setup.machine.p) {
        Ok(Some(pins)) => pins,
        Ok(None) => {
            return Err(format!(
                "This setup's machine {} has no fixture pins: the pin holes are sized and placed for them.",
                setup.machine.name()
            ));
        }
        Err(e) => {
            return Err(format!(
                "The machine's fixture pins do not read: {}",
                e.join(" ")
            ));
        }
    };
    let points = crate::post::drill_pin_points(op);
    let issue = get(&op.p, "pin_issue");
    if !issue.is_empty() {
        return Err(format!(
            "The pins these holes are for do not fit where they are placed, so no hole is made: {issue} Move them in the Work zero step of the setup that sits on them."
        ));
    }
    if points.len() != pins.at.len() {
        return Err("Place the pins on the stock's leftover in the Work zero step of the setup that sits on them: this operation has no holes for the machine's pins yet.".into());
    }
    let frame = crate::volume::Frame::new(setup, mesh);
    let o = frame.map([0.; 3]);
    let down = crate::post::drill_pin_axis(op)
        .map(|v| frame.map(v)[2] - o[2])
        .is_some_and(|z| z < -0.999);
    if !down {
        return Err("The pin holes open on a face that is not up in this setup: drill them in a setup that has the face the pins stand on facing up.".into());
    }
    let at: Vec<[f32; 2]> = (points.iter())
        .map(|p| {
            let q = frame.map(*p);
            [q[0], q[1]]
        })
        .collect();
    let d = pins.diameter as f32;
    let width = d + PIN_CLEARANCE;
    let r = b.plan.radius;
    let fits = if drill {
        (d - 0.001..=d + PIN_FIT + 0.001).contains(&(2. * r))
    } else {
        2. * r >= width / 2. + 0.05 && 2. * r <= width - 0.05
    };
    if !fits {
        return Err(format!(
            "Make the holes for the Ø{} pins with a Ø{} to Ø{} drill, or mill them with a flat end mill from Ø{} to Ø{}.",
            exact_mm(d),
            exact_mm(d),
            exact_mm(d + PIN_FIT),
            exact_mm(width / 2. + 0.05),
            exact_mm(width - 0.05)
        ));
    }
    let peck = if drill && ["Chip breaking", "Deep drilling"].contains(&get(&op.p, "cycle")) {
        number(op, "peck")?
    } else {
        f32::MAX
    };
    if peck <= 0. {
        return Err("Peck depth must be positive.".into());
    }
    if drill {
        b.plan.milling_direction = "Drilling · axial feed";
        b.plan.shape = CutterShape::Drill;
    } else {
        b.plan.milling_direction = "Helical and slot milling · axial ramp";
    }
    let depth = pins.height as f32 + PIN_DEPTH_EXTRA;
    let top = b.plan.stock.max[2] + number(op, "top")?.max(0.);
    // A milled hole's ramp: down a quarter of the tool's diameter a lap.
    let pitch = (r / 2.).min(1.);
    for (i, c) in at.iter().enumerate() {
        let reach = width / 2. + SLOT_PLAY / 2.;
        let region = Bounds {
            min: [c[0] - reach, c[1] - reach, b.plan.stock.min[2]],
            max: [c[0] + reach, c[1] + reach, top],
        };
        // The face the pins stand on, as the cuts before this one leave it.
        let face = crate::rest::start(setup, region, b.plan.stock.min[2], b.plan.stock.max[2], 0.);
        // A drill goes on by its point, so the hole is its full width all
        // the way down.
        let point = if drill {
            r * crate::simulation::POINT_RISE
        } else {
            0.
        };
        let target = face - depth - point;
        if target < b.plan.stock.min[2] + PIN_WALL - 0.001 {
            return Err(format!(
                "Pin {}'s hole, {} mm deep for the {} mm pins, would leave less than a {} mm floor under it: the stock is too thin there.",
                i + 1,
                exact_mm(depth),
                exact_mm(pins.height as f32),
                exact_mm(PIN_WALL)
            ));
        }
        if top - target >= b.plan.flute_length {
            return Err("The pin holes are deeper than the tool's flutes reach.".into());
        }
        b.plan.target = b.plan.target.min(target);
        if drill {
            let levels = ((face - target) / peck).ceil().max(1.) as usize;
            b.plan.levels += levels;
            b.start_at([c[0], c[1], face], Some(face))?;
            for level in 1..=levels {
                let z = (face - level as f32 * peck).max(target);
                b.push([c[0], c[1], z], Motion::Feed, true, b.feed)?;
                if level < levels {
                    // Deep drilling clears the hole between pecks; chip
                    // breaking only lifts off the chip.
                    let up = if get(&op.p, "cycle") == "Deep drilling" {
                        b.retract
                    } else {
                        (z + 0.5).min(face + 0.5)
                    };
                    b.push([c[0], c[1], up], Motion::Rapid, false, b.rapid)?;
                    b.push([c[0], c[1], z + 0.1], Motion::Rapid, false, b.rapid)?;
                }
            }
            b.retract()?;
            continue;
        }
        // The tool's centre goes round a circle for pin 1, round a slot
        // along the line from pin 1 for the others.
        let (along, length) = if i == 0 {
            ([1., 0.], 0.)
        } else {
            let v = [c[0] - at[0][0], c[1] - at[0][1]];
            let l = v[0].hypot(v[1]).max(1e-6);
            ([v[0] / l, v[1] / l], SLOT_PLAY)
        };
        let lap = slot_lap(*c, along, length, width / 2. - r);
        let total: f32 = lap
            .windows(2)
            .map(|w| (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]))
            .sum();
        let laps = ((face - target) / pitch).ceil().max(1.) as usize;
        b.plan.levels += laps + 1;
        b.start_at([lap[0][0], lap[0][1], face], Some(face))?;
        for k in 0..=laps {
            let mut gone = 0.;
            for w in lap.windows(2) {
                gone += (w[1][0] - w[0][0]).hypot(w[1][1] - w[0][1]);
                let z =
                    (face - (k as f32 + gone / total) * (face - target) / laps as f32).max(target);
                b.push([w[1][0], w[1][1], z], Motion::Feed, true, b.feed)?;
            }
        }
        // Off the wall to the middle before rising.
        b.push([c[0], c[1], target], Motion::Feed, true, b.feed)?;
        b.retract()?;
    }
    b.finish()
}
/// One lap of a tool's centre `offset` in from the wall of a slot: round
/// the slot centred on `c`, its ends' centres `length` apart along the unit
/// `along`, a circle at `c` for 0, starting and ending at the same point
/// (plan mm).
fn slot_lap(c: [f32; 2], along: [f32; 2], length: f32, offset: f32) -> Vec<[f32; 2]> {
    use std::f32::consts::PI;
    let h = length / 2.;
    let ends = [
        [c[0] - along[0] * h, c[1] - along[1] * h],
        [c[0] + along[0] * h, c[1] + along[1] * h],
    ];
    let base = along[1].atan2(along[0]);
    let mut lap = vec![];
    // Round the far end, then back round the near one: each half a circle.
    for (end, from) in [(ends[1], -PI / 2.), (ends[0], PI / 2.)] {
        for k in 0..=16 {
            let a = base + from + k as f32 * PI / 16.;
            lap.push([end[0] + offset * a.cos(), end[1] + offset * a.sin()]);
        }
    }
    lap.push(lap[0]);
    lap
}
/// The holes of the selected face a Drill operation cuts: those of its Hole
/// diameter, or all of them for 0.
fn drill_holes(setup: &Setup, op: &Operation, mesh: &Mesh) -> Result<Vec<Hole>, String> {
    let mut holes = holes(setup, op, mesh)?;
    let filter = if op.p.contains_key("hole_diameter") {
        number(op, "hole_diameter")?
    } else {
        0.
    };
    if filter < 0. {
        return Err("Hole diameter filter must be zero (all) or positive.".into());
    }
    if filter > 0. {
        holes.retain(|h| (h.diameter - filter).abs() <= 0.1);
    }
    if op.threaded_holes_only() {
        holes.retain(|h| h.thread.is_some());
        if holes.is_empty() {
            return Err("None of the selected holes has a thread modeled in it. Set Holes to Part's holes to drill them.".into());
        }
    }
    if holes.is_empty() {
        return Err("No detected holes match the diameter filter.".into());
    }
    if !["Optimize travel", "By X then Y"].contains(&get(&op.p, "hole_order")) {
        return Err("Choose a valid hole order.".into());
    }
    Ok(holes)
}
/// The hole of `holes` a Drill operation cuts next, from `pos`, taken out of
/// them: the nearest, or the next by X then Y, by its Hole order.
fn next_hole(holes: &mut Vec<Hole>, pos: [f32; 3], op: &Operation) -> Hole {
    let index = if get(&op.p, "hole_order") == "Optimize travel" {
        let distance = |h: &Hole| (h.center[0] - pos[0]).powi(2) + (h.center[1] - pos[1]).powi(2);
        (holes.iter().enumerate())
            .min_by(|(_, a), (_, c)| distance(a).total_cmp(&distance(c)))
            .map_or(0, |(i, _)| i)
    } else {
        0
    };
    holes.remove(index)
}
/// Whether a Drill operation goes through, and how far past a through
/// hole's exit (Breakthrough, 0 when it does not).
fn breakthrough(op: &Operation) -> Result<(bool, f32), String> {
    if !["Yes", "No"].contains(&get(&op.p, "through")) {
        return Err("Choose Yes or No for breakthrough.".into());
    }
    if get(&op.p, "through") != "Yes" {
        return Ok((false, 0.));
    }
    let breakthrough = number(op, "breakthrough")?;
    if breakthrough < 0. {
        return Err("Breakthrough cannot be negative.".into());
    }
    Ok((true, breakthrough))
}
fn drill(setup: &Setup, op: &Operation, tool: &Tool, mesh: &Mesh) -> Result<Plan, String> {
    if op.helical() {
        return helical(setup, op, tool, mesh);
    }
    if op.thread_milling() {
        return thread_mill(setup, op, tool, mesh);
    }
    if op.tapping() {
        return tap(setup, op, tool, mesh);
    }
    let mut b = Builder::new(setup, op, tool, mesh)?;
    // A chamfer mill countersinks each opening rather than drilling it.
    let chamfer = get(&tool.p, "type") == "Chamfer mill";
    if get(&tool.p, "type") != "Drill" && !chamfer {
        return Err(
            "Choose a drill, or a chamfer mill to countersink, for a drilling cycle. Helical milling takes a flat end mill.".into(),
        );
    }
    let mut holes = drill_holes(setup, op, mesh)?;
    let cycle = get(&op.p, "cycle");
    if ![
        "Drilling",
        "Chip breaking",
        "Deep drilling",
        "Counterboring",
    ]
    .contains(&cycle)
    {
        return Err("Choose a supported drilling cycle.".into());
    }
    let peck = if ["Chip breaking", "Deep drilling"].contains(&cycle) {
        number(op, "peck")?
    } else {
        f32::MAX
    };
    if peck <= 0. {
        return Err("Peck depth must be positive.".into());
    }
    // Breakthrough is travel past the exit of a hole that goes through the
    // part. Under a blind hole's bottom is the part, so such a hole is
    // refused rather than drilled into, as the planner's plans are.
    let (through, breakthrough) = if !chamfer {
        breakthrough(op)?
    } else if ["Yes", "No"].contains(&get(&op.p, "through")) {
        (false, 0.)
    } else {
        return Err("Choose Yes or No for breakthrough.".into());
    };
    b.plan.milling_direction = "Drilling · axial feed";
    // The sweep is the drill's shape: its 118° point under its flutes, so
    // that a blind hole's drill-point floor is left as it is (#155); a
    // chamfer mill's is the cone of its tip angle.
    b.plan.shape = if chamfer {
        let angle = tool
            .tip_angle()
            .ok_or("Enter a chamfer mill tip angle from 20° to 160°.")?;
        b.plan.milling_direction = "Countersinking · axial feed";
        CutterShape::Chamfer((angle * 10.).round() as u16)
    } else {
        CutterShape::Drill
    };
    let rise = b.plan.shape.rise().unwrap_or(0.);
    let count = holes.len();
    crate::progress::expect(count);
    while !holes.is_empty() {
        crate::progress::report(count - holes.len())?;
        let h = next_hole(&mut holes, b.pos, op);
        // The drill may be as wide as the bore at its tessellation's vertices,
        // which its chords lie a sagitta inside, and 0.01 over that: the part
        // report rounds the bore to 0.01 and the planner's drill is that size.
        // A countersink is cut by a cone of its own angle: a sharper one
        // would cut into its wall and the bore under it, a blunter one would
        // leave the cone but for its rim.
        if let (CutterShape::Chamfer(tenths), Some(c)) = (b.plan.shape, h.countersink)
            && (f32::from(tenths) / 10. - c.angle).abs() > 0.5
        {
            let angle = (c.angle * 10.).round() / 10.;
            return Err(format!(
                "The hole at X{:.2} Y{:.2} has a countersink of {angle}°, and this chamfer mill's tip is {}°. Choose a chamfer mill of {angle}°.",
                h.center[0],
                h.center[1],
                f32::from(tenths) / 10.,
            ));
        }
        if chamfer && b.plan.radius * 2. < h.opening - 2. * h.sagitta - 0.01 {
            return Err(format!(
                "The Ø{:.2} chamfer mill is narrower than an opening Ø{:.2} it would countersink. Choose a wider one or a diameter filter.",
                b.plan.radius * 2.,
                h.opening
            ));
        }
        if !chamfer && b.plan.radius * 2. > h.widest_drill() {
            return Err(format!(
                "The Ø{:.2} drill exceeds an opening Ø{:.2}. Choose a smaller drill or a diameter filter.",
                b.plan.radius * 2.,
                h.diameter
            ));
        }
        if through && !h.through {
            return Err(format!(
                "The Ø{:.2} hole at X{:.2} Y{:.2} is blind: the part goes on under its bottom, so it cannot be drilled through. Set Through to No, or use the diameter filter to drill only the holes that go through.",
                h.diameter, h.center[0], h.center[1]
            ));
        }
        let top = b.plan.stock.max[2] + number(op, "top")?.max(0.);
        // A chamfer mill's cone is as wide as the opening at its rim.
        let target = if chamfer {
            h.top - h.opening / 2. * rise + number(op, "bottom")?
        } else {
            h.bottom + number(op, "bottom")? - breakthrough
        };
        if target >= h.top
            || target < h.bottom - breakthrough - 0.001
            || target < b.plan.stock.min[2] - breakthrough - 0.001
            || top - target >= b.plan.flute_length
        {
            return Err("Drilling depth must fit the flute length, and the bottom offset cannot take the drill below the hole bottom: only Through does, below a hole that goes through the part.".into());
        }
        let cutting_top = crate::rest::start_over(
            setup,
            &crate::rest::Reach::disc([h.center[0], h.center[1]], b.plan.radius),
            target,
            top,
            number(op, "top")?,
        );
        let lower = (cutting_top < top - 0.0001).then_some(cutting_top);
        b.plan.rest_top = if b.plan.levels == 0 {
            lower
        } else {
            b.plan.rest_top.zip(lower).map(|(a, b)| a.max(b))
        };
        let levels = ((cutting_top - target) / peck).ceil().max(1.) as usize;
        if levels > CEILING {
            return Err(too_many("moves"));
        }
        b.plan.target = b.plan.target.min(target);
        b.plan.levels += levels;
        b.start_at([h.center[0], h.center[1], cutting_top], Some(cutting_top))?;
        for level in 1..=levels {
            let depth = (cutting_top - level as f32 * peck).max(target);
            b.push(
                [h.center[0], h.center[1], depth],
                Motion::Feed,
                true,
                b.feed,
            )?;
            if level < levels {
                let retract = if cycle == "Deep drilling" {
                    b.retract
                } else {
                    (depth + 0.5).min(top)
                };
                b.push(
                    [h.center[0], h.center[1], retract],
                    Motion::Rapid,
                    false,
                    b.rapid,
                )?;
                b.push(
                    [h.center[0], h.center[1], (depth + 0.1).min(top)],
                    Motion::Rapid,
                    false,
                    b.rapid,
                )?;
            }
        }
        if cycle == "Counterboring" {
            let dwell = number(op, "dwell")?;
            if !(0. ..=600.).contains(&dwell) {
                return Err("Dwell must be 0–600 seconds.".into());
            }
            b.dwell(dwell);
        }
        b.retract()?;
    }
    b.finish()
}
/// Where a flat end mill can mill the hole `h` of `g`: the height of its
/// floor, the highest point of the part's sloping or flat faces inside the
/// bore below its rim (its bottom when it goes through), and the bore's
/// diameter above that floor. A drill's point would sink into a cone or a
/// fillet under a blind hole; a flat end stops on top of it, and is no
/// wider than the bore is all the way down to it.
fn milled_bore(g: &Geometry, h: &Hole) -> (f32, f32) {
    let floor = if h.through {
        h.bottom
    } else {
        let reach = h.diameter / 2. + 0.01;
        (g.triangles.iter())
            .filter(|t| {
                plane_slope(&t.p).is_some()
                    && t.p.iter().all(|p| {
                        (p[0] - h.center[0]).hypot(p[1] - h.center[1]) <= reach
                            && p[2] < h.top - 0.01
                    })
            })
            .flat_map(|t| t.p.map(|p| p[2]))
            .fold(h.bottom, f32::max)
    };
    let (_, bore) = corridor(g, h.center, floor + 0.02, 0., h.opening / 2.);
    (floor, bore * 2.)
}
/// Helical milling (`HELICAL_MILLING`): each round hole of the selected face
/// milled with a flat end mill, down a helix of Stepdown per turn to the
/// hole's flat floor (past its exit by Breakthrough when it goes through),
/// once round there, then to the centre and out. A hole whose bore leaves
/// the cutter's centre more than its radius to travel is cut in rings, the
/// innermost first, each Stepover (% of the cutter's diameter) wider than
/// the one inside it: the cutter clears the centre first and never takes
/// more than that from the side. It climbs: counterclockwise round the hole
/// with the spindle clockwise. Tolerance is the most a turn's chords stray
/// inside its circle.
fn helical(setup: &Setup, op: &Operation, tool: &Tool, mesh: &Mesh) -> Result<Plan, String> {
    let mut b = Builder::new(setup, op, tool, mesh)?;
    if get(&tool.p, "type") != "Flat end mill" {
        return Err("Choose a flat end mill for Helical milling.".into());
    }
    if b.single_axis {
        return Err(crate::manual::HELICAL_ON_MANUAL.into());
    }
    let mut holes = drill_holes(setup, op, mesh)?;
    let (through, breakthrough) = breakthrough(op)?;
    let pitch = number(op, "stepdown")?;
    if pitch <= 0. || pitch > b.plan.flute_length {
        return Err(
            "Helical milling needs a stepdown per turn above 0 and at most the flute length."
                .into(),
        );
    }
    let stepover = number(op, "stepover")?;
    if !(1. ..=100.).contains(&stepover) {
        return Err("Helical milling needs a stepover of 1–100% of the tool diameter.".into());
    }
    let tolerance = number(op, "tolerance")?;
    if !(0.001..=0.1).contains(&tolerance) {
        return Err("Helical milling needs a tolerance of 0.001–0.1 mm.".into());
    }
    let r = b.plan.radius;
    // Climb inside a hole: counterclockwise with the spindle clockwise.
    let turn = if b.plan.spindle_clockwise { 1. } else { -1. };
    b.plan.milling_direction = "Climb · helical";
    let bottom = number(op, "bottom")?;
    let top = b.plan.stock.max[2] + number(op, "top")?.max(0.);
    let count = holes.len();
    crate::progress::expect(count);
    while !holes.is_empty() {
        crate::progress::report(count - holes.len())?;
        let h = next_hole(&mut holes, b.pos, op);
        let (floor, diameter) = (h.bottom, h.diameter);
        // As for a drill, a cutter of the bore's size fits: it may be as
        // wide as the bore at its tessellation's vertices.
        if 2. * r > diameter + 2. * h.sagitta + 0.01 {
            return Err(format!(
                "The Ø{:.2} end mill is wider than the Ø{:.2} hole at X{:.2} Y{:.2}. Choose a smaller end mill or a diameter filter.",
                2. * r,
                diameter,
                h.center[0],
                h.center[1]
            ));
        }
        if through && !h.through {
            return Err(format!(
                "The Ø{:.2} hole at X{:.2} Y{:.2} is blind: the part goes on under its bottom, so it cannot be milled through. Set Through to No, or use the diameter filter to mill only the holes that go through.",
                h.diameter, h.center[0], h.center[1]
            ));
        }
        let lowest = if through { floor - breakthrough } else { floor };
        let target = lowest + bottom;
        if target >= h.top
            || target < lowest - 0.001
            || target < b.plan.stock.min[2] - breakthrough - 0.001
        {
            return Err("The bottom offset must leave the end mill in the hole, and cannot take it below the hole's floor: only Through does, below a hole that goes through the part.".into());
        }
        // The cutter's centre travels up to `outer` from the hole's.
        let outer = (diameter / 2. - r).max(0.);
        let reach = outer + r;
        let cutting_top = crate::rest::start_over(
            setup,
            &crate::rest::Reach::disc([h.center[0], h.center[1]], reach),
            target,
            top,
            number(op, "top")?,
        );
        // The flutes cut the hole's wall from where the material starts
        // down; the tool's reach from the stock's top is its stickout's.
        if cutting_top - target >= b.plan.flute_length {
            return Err(format!(
                "The Ø{:.2} hole at X{:.2} Y{:.2} is milled {:.2} mm deep, as deep as the end mill's flutes or deeper. Choose an end mill with longer flutes.",
                h.diameter,
                h.center[0],
                h.center[1],
                cutting_top - target
            ));
        }
        let lower = (cutting_top < top - 0.0001).then_some(cutting_top);
        b.plan.rest_top = if b.plan.levels == 0 {
            lower
        } else {
            b.plan.rest_top.zip(lower).map(|(a, b)| a.max(b))
        };
        // Rings out to `outer`: one when its turn already sweeps the centre,
        // else from half the cutter's radius out, Stepover apart at most.
        let rings: Vec<f32> = if outer <= r {
            vec![outer]
        } else {
            let inner = r / 2.;
            let n = ((outer - inner) / (stepover / 100. * 2. * r))
                .ceil()
                .max(1.) as usize;
            (0..=n)
                .map(|k| inner + (outer - inner) * k as f32 / n as f32)
                .collect()
        };
        let turns = ((cutting_top - target) / pitch).ceil().max(1.) as usize;
        let approach = (cutting_top + b.retract_margin).min(b.retract);
        for (i, &radius) in rings.iter().enumerate() {
            let segments = if radius < 0.001 {
                1
            } else {
                (std::f32::consts::PI / (1. - tolerance.min(radius) / radius).acos())
                    .ceil()
                    .clamp(12., 360.) as usize
            };
            if (turns + 1) * segments > CEILING {
                return Err(too_many("moves"));
            }
            let at = |step: usize, z: f32| {
                let a = turn * step as f32 * std::f32::consts::TAU / segments as f32;
                [
                    h.center[0] + radius * a.cos(),
                    h.center[1] + radius * a.sin(),
                    z,
                ]
            };
            let start = at(0, cutting_top);
            if i == 0 {
                b.start_at(start, Some(cutting_top))?;
            } else {
                // Over the cleared middle to the next ring's start, above the
                // material, then down to it.
                b.push(
                    [b.pos[0], b.pos[1], approach],
                    Motion::Rapid,
                    false,
                    b.rapid,
                )?;
                b.push(
                    [start[0], start[1], approach],
                    Motion::Rapid,
                    false,
                    b.rapid,
                )?;
                b.push(start, Motion::Feed, false, b.feed * 0.25)?;
            }
            // Straight down when the cutter is the hole's size: a plunge.
            let rate = if radius < 0.001 {
                b.feed * 0.25
            } else {
                b.feed
            };
            let steps = turns * segments;
            for step in 1..=steps {
                let z = cutting_top - (cutting_top - target) * step as f32 / steps as f32;
                b.push(at(step, z), Motion::Feed, true, rate)?;
            }
            if radius >= 0.001 {
                for step in 1..=segments {
                    b.push(at(step, target), Motion::Feed, true, b.feed)?;
                }
            }
        }
        b.plan.target = b.plan.target.min(target);
        b.plan.levels += turns * rings.len();
        // Off the wall to the cleared centre before rising out of the hole.
        b.push(
            [h.center[0], h.center[1], target],
            Motion::Feed,
            false,
            b.feed,
        )?;
        b.retract()?;
    }
    b.finish()
}
/// The thread a threading Drill cuts in one hole: its size and hand, and
/// how far down from the hole's mouth it runs.
#[derive(Debug, Clone)]
pub(crate) struct HoleThread {
    pub name: String,
    pub major: f32,
    pub minor: f32,
    pub pitch: f32,
    pub angle: f32,
    pub tapered: bool,
    pub right_hand: bool,
    /// The drill the hole is drilled with first.
    pub tap_drill: f32,
    /// Its extent along Z: from the hole's mouth down to `bottom`.
    pub top: f32,
    pub bottom: f32,
}
/// The thread `op` (Thread milling or Tapping) cuts in `h`: the one modeled
/// in it (Thread "From the model") or the size chosen, its hand as modeled
/// or as chosen, down from the mouth the thread depth when one is entered,
/// else as deep as the model's thread runs; a size chosen for a hole with
/// none modeled runs through a hole that goes through, and stops a pitch
/// and a half above a blind hole's bottom.
pub(crate) fn hole_thread(op: &Operation, h: &Hole) -> Result<HoleThread, String> {
    let at = format!(
        "Ø{:.2} hole at X{:.2} Y{:.2}",
        h.diameter, h.center[0], h.center[1]
    );
    let chosen = get(&op.p, "thread");
    let (name, major, minor, pitch, angle, tapered, mut right_hand, tap_drill) = if chosen
        == crate::model::FROM_MODEL
    {
        let t = h.thread.as_ref().ok_or_else(|| {
            format!("The {at} has no thread modeled in it: choose its size in Thread.")
        })?;
        let (major, minor, drill) = t.spec.map_or((t.major, t.minor, t.major - t.pitch), |s| {
            (s.major, s.minor(), s.tap_drill)
        });
        (
            t.name(),
            major,
            minor,
            t.pitch,
            t.angle,
            t.tapered,
            t.right_hand,
            drill,
        )
    } else {
        let s = crate::thread::spec(chosen).ok_or("Choose the thread size, or From the model.")?;
        (
            s.name.to_string(),
            s.major,
            s.minor(),
            s.pitch,
            s.angle,
            s.tapered,
            true,
            s.tap_drill,
        )
    };
    match get(&op.p, "thread_hand") {
        "Right hand" => right_hand = true,
        "Left hand" => right_hand = false,
        _ => {}
    }
    let entered = get(&op.p, "thread_depth").trim();
    let depth = if !entered.is_empty() {
        number(op, "thread_depth")?
    } else if let (crate::model::FROM_MODEL, Some(t)) = (chosen, &h.thread) {
        h.top - t.from.max(h.bottom)
    } else if h.through {
        h.top - h.bottom
    } else {
        (h.top - h.bottom - 1.5 * pitch).max(pitch)
    };
    if !(depth > 0.) {
        return Err("Thread depth must be above 0.".into());
    }
    let bottom = h.top - depth;
    if !h.through && bottom < h.bottom - 0.001 {
        return Err(format!(
            "The {at} is {:.2} deep, less than its {:.2} of thread. Enter a thread depth it holds.",
            h.top - h.bottom,
            depth
        ));
    }
    Ok(HoleThread {
        name,
        major,
        minor,
        pitch,
        angle,
        tapered,
        right_hand,
        tap_drill,
        top: h.top,
        bottom,
    })
}
/// Thread milling (`THREAD_MILLING`): each hole's thread cut by a thread
/// mill orbiting inside it. The cutter goes down the middle of the drilled
/// hole, arcs out to the wall, and goes round it one pitch along its axis a
/// turn, Thread passes times, each further out to the thread's full depth
/// (the cut thinning as it deepens), then once more where Spring pass is
/// Yes, arcing back to the middle after each. It climbs: counterclockwise
/// round the hole with the spindle clockwise, so a right-hand thread is cut
/// from its bottom up and a left-hand one from its mouth down. A tapered
/// thread's orbit narrows 1:16 on the diameter into the hole. The feed is
/// the cutting edge's: the centre's, on its smaller orbit, is slower.
fn thread_mill(setup: &Setup, op: &Operation, tool: &Tool, mesh: &Mesh) -> Result<Plan, String> {
    let mut b = Builder::new(setup, op, tool, mesh)?;
    if get(&tool.p, "type") != "Thread mill" {
        return Err("Choose a thread mill for Thread milling.".into());
    }
    if crate::manual::is_hand(&setup.machine) {
        return Err(crate::model::THREAD_MILL_ON_HAND.into());
    }
    let mut holes = drill_holes(setup, op, mesh)?;
    let tolerance = number(op, "tolerance")?;
    if !(0.001..=0.1).contains(&tolerance) {
        return Err("Thread milling needs a tolerance of 0.001–0.1 mm.".into());
    }
    let passes = number(op, "thread_passes")?;
    if !(1. ..=10.).contains(&passes) || passes.fract() != 0. {
        return Err("Thread passes must be a whole number from 1 to 10.".into());
    }
    let spring = get(&op.p, "spring_pass") == "Yes";
    let (finest, coarsest) = tool
        .thread_pitches()
        .ok_or("Enter the thread mill's pitches, like 0.5-1.25.")?;
    let angle = num(&tool.p, "thread_angle").ok_or("Enter the thread mill's thread angle.")?;
    let reach = num(&tool.p, "reach").ok_or("Enter the thread mill's reach.")?;
    let teeth = tool.thread_teeth();
    if teeth > 1 && coarsest - finest > 1e-4 {
        return Err(crate::model::TEETH_ONE_PITCH.into());
    }
    // The sweep is the teeth: each as tall as the coarsest pitch it cuts.
    b.plan.shape = CutterShape::Thread {
        pitch: (coarsest * 1000.).round().clamp(1., f32::from(u16::MAX)) as u16,
        teeth: teeth as u8,
        angle: angle.round() as u8,
    };
    let r = b.plan.radius;
    // Its body, as drawn: the neck as narrow as the teeth's roots up to its
    // reach, then the shank (as wide as the teeth when not saved).
    b.plan.neck = Some([
        (r - crate::simulation::THREAD_TOOTH * coarsest).max(r * 0.3),
        reach,
        num(&tool.p, "shank")
            .filter(|d| *d > 0.)
            .map_or(r, |d| d / 2.),
    ]);
    // Climb inside a hole: counterclockwise with the spindle clockwise.
    let turn = if b.plan.spindle_clockwise { 1. } else { -1. };
    b.plan.milling_direction = crate::forces::THREAD_MILLING_DIRECTION;
    let count = holes.len();
    crate::progress::expect(count);
    while !holes.is_empty() {
        crate::progress::report(count - holes.len())?;
        let h = next_hole(&mut holes, b.pos, op);
        let t = hole_thread(op, &h)?;
        let at = format!("{} hole at X{:.2} Y{:.2}", t.name, h.center[0], h.center[1]);
        if (t.angle - angle).abs() > 0.5 {
            return Err(format!(
                "The {at} is a {}° thread and this thread mill cuts {}°. Choose a {}° thread mill.",
                t.angle, angle, t.angle
            ));
        }
        if t.pitch < finest - 1e-4 || t.pitch > coarsest + 1e-4 {
            return Err(format!(
                "The {at} has a pitch of {} mm, outside this thread mill's {finest}-{coarsest} mm. Choose a thread mill that cuts it, or tap it.",
                t.pitch
            ));
        }
        // The cutter goes down the drilled hole: it must be narrower than
        // the bore it goes down.
        if 2. * r > h.diameter + 2. * h.sagitta - 0.02 {
            return Err(format!(
                "The Ø{:.2} thread mill is too wide to go down the Ø{:.2} bore of the {at}. Choose a smaller thread mill.",
                2. * r,
                h.diameter
            ));
        }
        // Its tooth reaches half a pitch below the thread's last turn.
        let lowest = t.bottom - t.pitch / 2.;
        if h.top - lowest > reach + 0.001 {
            return Err(format!(
                "The {at} runs {:.2} deep and this thread mill reaches {reach} mm. Choose one that reaches further, or tap it.",
                h.top - lowest
            ));
        }
        if !h.through && lowest < h.bottom - 0.001 {
            return Err(format!(
                "The {at} stops {:.2} above the bottom of its hole: the thread mill's tooth needs {:.2} below the thread. Drill the hole deeper, or enter a shorter thread depth.",
                t.bottom - h.bottom,
                t.pitch / 2.
            ));
        }
        if !bored(setup, &b, h.center, r * 0.9, h.top, lowest) {
            return Err(format!(
                "Drill the {at} first: Ø{:.1}, {:.1} deep. The thread mill goes down the middle of the drilled hole.",
                t.tap_drill,
                h.top - lowest
            ));
        }
        // The orbit: where the tooth's tip touches the bore, out to the
        // thread's major diameter, narrowing into a tapered hole.
        let full = t.major / 2. - r;
        let first = (h.diameter / 2. - r).clamp(0., full);
        let taper = |z: f32| if t.tapered { (t.top - z) / 32. } else { 0. };
        // One pitch up the axis a turn the way it goes round: up for a
        // right-hand thread cut counterclockwise.
        let rise = turn * if t.right_hand { 1. } else { -1. };
        // Half a pitch past an open end: the mouth, and the exit of a hole
        // that goes through.
        let mouth = t.top + t.pitch / 2.;
        let end = if h.through {
            t.bottom - t.pitch / 2.
        } else {
            t.bottom
        };
        // The teeth above the lowest cut the thread's last pitches: the
        // lowest stops that much short of the mouth, a turn at the least.
        let short = (mouth - (teeth - 1) as f32 * t.pitch).max(end + t.pitch);
        let (start, stop) = if rise > 0. {
            (end, short)
        } else {
            (short, end)
        };
        let segments = (std::f32::consts::PI / (1. - tolerance.min(full) / full.max(1e-3)).acos())
            .ceil()
            .clamp(12., 360.) as usize;
        let steps = ((stop - start).abs() / t.pitch * segments as f32).floor() as usize;
        let mut radii: Vec<f32> = (1..=passes as usize)
            .map(|k| first + (full - first) * (k as f32 / passes).sqrt())
            .collect();
        if spring {
            radii.push(full);
        }
        if (radii.len() * (steps + segments)) > CEILING {
            return Err(too_many("moves"));
        }
        let c = h.center;
        let around =
            |a: f32, radius: f32, z: f32| [c[0] + radius * a.cos(), c[1] + radius * a.sin(), z];
        b.start_at([c[0], c[1], start], Some(h.top))?;
        for &radius in &radii {
            let radius = radius.max(0.001);
            let rate = b.feed * radius / (radius + r);
            // Down (or up) the middle to where the helix starts.
            b.push([c[0], c[1], start], Motion::Feed, false, b.feed)?;
            // In on a half circle to the wall, tangent to the orbit there.
            let arc = segments / 2;
            let o = (radius - taper(start)) / 2.;
            for k in 1..=arc {
                let phi =
                    std::f32::consts::PI + turn * std::f32::consts::PI * k as f32 / arc as f32;
                b.push(
                    [c[0] + o + o * phi.cos(), c[1] + o * phi.sin(), start],
                    Motion::Feed,
                    true,
                    rate,
                )?;
            }
            // Round and along the axis, one pitch a turn.
            for k in 1..=steps {
                let a = turn * k as f32 * std::f32::consts::TAU / segments as f32;
                let z = start + (stop - start).signum() * t.pitch * k as f32 / segments as f32;
                b.push(around(a, radius - taper(z), z), Motion::Feed, true, rate)?;
            }
            // Out on a half circle back to the middle.
            let a_end = turn * steps as f32 * std::f32::consts::TAU / segments as f32;
            let z_end = start + (stop - start).signum() * t.pitch * steps as f32 / segments as f32;
            let o = (radius - taper(z_end)) / 2.;
            let mid = around(a_end, o, z_end);
            for k in 1..=arc {
                let phi = a_end + turn * std::f32::consts::PI * k as f32 / arc as f32;
                b.push(
                    [mid[0] + o * phi.cos(), mid[1] + o * phi.sin(), z_end],
                    Motion::Feed,
                    true,
                    rate,
                )?;
            }
        }
        b.plan.target = b.plan.target.min(end);
        b.plan.levels += radii.len() * (steps / segments).max(1);
        b.retract()?;
    }
    b.finish()
}
/// Tapping (`TAPPING`), by hand: each hole tapped to its thread's depth,
/// the tap held square by the manual mill's spindle (turned by hand, the
/// spindle off) or the lathe's tailstock. The plan is the tap going down
/// and back out, so the simulation and the time are right; the guide says
/// how to turn it. A tap's lead (its tapered first threads) cuts no full
/// thread, so a blind hole must go that far below the thread.
fn tap(setup: &Setup, op: &Operation, tool: &Tool, mesh: &Mesh) -> Result<Plan, String> {
    let mut b = Builder::new(setup, op, tool, mesh)?;
    let spec = tool
        .tap_spec()
        .ok_or("Choose a tap, with its thread size, for Tapping.")?;
    if !crate::manual::is_hand(&setup.machine) {
        return Err(crate::model::TAP_ON_CNC.into());
    }
    let mut holes = drill_holes(setup, op, mesh)?;
    let lead = tap_lead(tool) * spec.pitch;
    // The tap cuts to its major diameter.
    b.plan.radius = spec.major / 2.;
    b.plan.milling_direction = crate::forces::TAPPING_DIRECTION;
    let count = holes.len();
    crate::progress::expect(count);
    while !holes.is_empty() {
        crate::progress::report(count - holes.len())?;
        let h = next_hole(&mut holes, b.pos, op);
        let t = hole_thread(op, &h)?;
        let at = format!("{} hole at X{:.2} Y{:.2}", t.name, h.center[0], h.center[1]);
        if (spec.major - t.major).abs() > 0.3 || (spec.pitch - t.pitch).abs() > 1e-3 {
            return Err(format!(
                "The {at} takes an {} tap; this tap is {}. Choose an {} tap.",
                t.name, spec.name, t.name
            ));
        }
        let lowest = t.bottom - lead;
        if !h.through && lowest < h.bottom - 0.001 {
            return Err(format!(
                "The {at} needs {:.1} below its thread for the tap's lead, and its hole stops {:.1} below it. Drill it {:.1} deep, or use a bottoming tap and enter a shorter thread depth.",
                lead,
                t.bottom - h.bottom,
                h.top - lowest
            ));
        }
        // Drilled to its full width down the thread, and below it as deep
        // as the tap's narrow, chamfered tip goes: else the tap is forced
        // into the stock and breaks.
        if !bored(setup, &b, h.center, t.minor / 2. * 0.85, h.top, t.bottom)
            || !bored(setup, &b, h.center, t.tap_drill / 4., t.bottom, lowest)
        {
            return Err(format!(
                "Drill the {at} first: Ø{:.1}, {:.1} deep. The tap cuts its thread in the drilled hole.",
                t.tap_drill,
                h.top - lowest
            ));
        }
        let depth = lowest;
        // The tap starts to cut where the material over the hole is: the
        // stock's top when nothing has cut it, the hole's mouth when
        // something has. The guide counts its turns from there.
        let top = b.plan.stock.max[2] + number(op, "top")?.max(0.);
        let mouth = crate::rest::start_over(
            setup,
            &crate::rest::Reach::disc(h.center, spec.major / 2.),
            depth,
            top,
            number(op, "top")?,
        )
        .max(h.top);
        b.start_at([h.center[0], h.center[1], mouth], Some(mouth))?;
        // In one pitch a turn, then back out.
        let rate = b.feed;
        b.push([h.center[0], h.center[1], depth], Motion::Feed, true, rate)?;
        b.push([h.center[0], h.center[1], mouth], Motion::Feed, false, rate)?;
        b.plan.target = b.plan.target.min(depth);
        b.plan.levels += 1;
        b.retract()?;
    }
    b.finish()
}
/// Whether the stock is gone from the upright cylinder of `radius` round
/// `centre` between `bottom` and `top` before this operation: earlier cuts
/// emptied it (`RestStock::bored`), or, with none, it is clear of the stock.
fn bored(setup: &Setup, b: &Builder, centre: [f32; 2], radius: f32, top: f32, bottom: f32) -> bool {
    match &setup.preview_remaining {
        Some(rest) => rest.bored(centre, radius, top, bottom),
        None => {
            let s = b.plan.stock;
            top <= s.min[2]
                || bottom >= s.max[2]
                || centre[0] + radius <= s.min[0]
                || centre[0] - radius >= s.max[0]
                || centre[1] + radius <= s.min[1]
                || centre[1] - radius >= s.max[1]
        }
    }
}
/// How many pitches long a tap's lead is: a spiral flute (form C) tap's
/// 2-3 threads, a spiral point or straight flute plug tap's 3.5-5.
pub(crate) fn tap_lead(tool: &Tool) -> f32 {
    if get(&tool.p, "tap_kind") == crate::model::TAP_KINDS[0] {
        2.5
    } else {
        4.5
    }
}
/// Highest collision-free vertical tool tip against a triangle (flat or spherical end).
/// Tests the triangle interior stationary point and the constrained edge extrema.
fn drop_triangle(p: &[[f32; 3]; 3], xy: [f32; 2], r: f32, ball: bool) -> Option<f32> {
    let mut best = f32::NEG_INFINITY;
    let mut consider = |q: [f32; 3]| {
        let d = (q[0] - xy[0]).powi(2) + (q[1] - xy[1]).powi(2);
        if d <= r * r + 0.00001 {
            best = best.max(
                q[2] + if ball {
                    (r * r - d).max(0.).sqrt() - r
                } else {
                    0.
                },
            );
        }
    };
    for i in 0..3 {
        let a = p[i];
        let b = p[(i + 1) % 3];
        consider(a);
        let v = [b[0] - a[0], b[1] - a[1]];
        let l2 = v[0] * v[0] + v[1] * v[1];
        if l2 < 1e-12 {
            continue;
        }
        let proj = ((xy[0] - a[0]) * v[0] + (xy[1] - a[1]) * v[1]) / l2;
        let perp = (a[0] + proj * v[0] - xy[0]).powi(2) + (a[1] + proj * v[1] - xy[1]).powi(2);
        if perp > r * r {
            continue;
        }
        let half = ((r * r - perp) / l2).sqrt();
        let lo = (proj - half).max(0.);
        let hi = (proj + half).min(1.);
        if lo > hi {
            continue;
        }
        let dz = b[2] - a[2];
        let t = if ball {
            (proj + dz * (r * r - perp).sqrt() / (l2.sqrt() * (l2 + dz * dz).sqrt())).clamp(lo, hi)
        } else if dz > 0. {
            hi
        } else {
            lo
        };
        consider([a[0] + t * v[0], a[1] + t * v[1], a[2] + t * dz]);
    }
    let a = [p[1][0] - p[0][0], p[1][1] - p[0][1], p[1][2] - p[0][2]];
    let b = [p[2][0] - p[0][0], p[2][1] - p[0][1], p[2][2] - p[0][2]];
    let det = a[0] * b[1] - a[1] * b[0];
    if det.abs() > 1e-8 {
        let dx = (a[2] * b[1] - a[1] * b[2]) / det;
        let dy = (a[0] * b[2] - a[2] * b[0]) / det;
        let denom = (dx * dx + dy * dy + if ball { 1. } else { 0. }).sqrt();
        let q = if denom > 1e-8 {
            [xy[0] + r * dx / denom, xy[1] + r * dy / denom]
        } else {
            xy
        };
        if let Some(z) = plane_height(p, q) {
            consider([q[0], q[1], z]);
        }
    }
    best.is_finite().then_some(best)
}
/// How close two heights of the cutter on different triangles are taken as
/// one (`finish_grid`), mm.
const TIE: f32 = 0.0005;
struct SurfaceGrid {
    nx: usize,
    ny: usize,
    step: f32,
    origin: [f32; 2],
    z: Vec<Option<f32>>,
    /// The cells cut, by cell (`nx - 1` a row), when only picked faces are;
    /// None when every cell with all four heights is.
    cells: Option<Vec<bool>>,
}
/// The faces a finishing operation (Parallel, Waterline) is given: None for
/// the whole part (Model silhouette), or the picked faces in groups that
/// touch along an edge (`surface::connected`), the group picked first first.
/// Each group is finished before the next.
fn picked_groups(op: &Operation, mesh: &Mesh) -> Result<Option<Vec<HashSet<usize>>>, String> {
    match get(&op.p, "boundary") {
        "Model silhouette" => Ok(None),
        "Selected geometry" => {
            let ids: Vec<usize> = (op.surfaces.iter())
                .flat_map(|s| s.ids.iter().copied())
                .collect();
            if ids.is_empty() {
                return Err("Select the surfaces to finish on the part, or use Model silhouette to finish the whole part.".into());
            }
            Ok(Some(
                surface::connected(mesh, &ids)
                    .into_iter()
                    .map(|g| g.into_iter().collect())
                    .collect(),
            ))
        }
        _ => Err(
            "Choose Model silhouette, or Selected geometry and pick the surfaces to finish.".into(),
        ),
    }
}
/// The heights of the cutter over the part, and where it cuts: everywhere
/// or (`picked`) where it touches one of those faces. Stages `stage` and the
/// next of `stages` (`progress::stage`).
fn finish_grid(
    g: &Geometry,
    op: &Operation,
    b: &Builder,
    picked: Option<&HashSet<usize>>,
    (stage, stages): (usize, usize),
) -> Result<SurfaceGrid, String> {
    let spacing = number(op, "surface_stepover")?;
    let tolerance = number(op, "tolerance")?;
    let offset = number(op, "allowance")? + number(op, "bottom")?;
    if spacing <= 0. || spacing > b.plan.radius || tolerance <= 0. || tolerance > 1. || offset < 0.
    {
        return Err("Use surface spacing up to cutter radius, tolerance 0–1 mm and nonnegative finishing offset.".into());
    }
    // Picked faces only: a sample is cut where the cutter touches one of
    // them, and the rest of the part still holds the cutter up where it is
    // higher.
    let step = spacing.min((8. * b.plan.radius * tolerance).sqrt()).min(1.);
    if step < 0.05 {
        return Err("Requested finishing sampling is below the 0.05 mm preview limit.".into());
    }
    let dims = g.bounds.dimensions();
    if dims[..2]
        .iter()
        .any(|d| !d.is_finite() || d / step > CEILING as f32)
    {
        return Err(too_many("samples"));
    }
    let nx = (dims[0] / step).ceil() as usize + 1;
    let ny = (dims[1] / step).ceil() as usize + 1;
    if nx.saturating_mul(ny) > CEILING {
        return Err(too_many("samples"));
    }
    let mut z = vec![None; nx * ny];
    let mut selected = vec![false; nx * ny];
    let low = number(op, "slope_min")?;
    let high = number(op, "slope_max")?;
    if low < 0. || high > 90. || low > high {
        return Err("Set slope limits in increasing order within 0–90°.".into());
    }
    // Triangle rasterization evaluates only its expanded footprint, not the entire mesh per sample.
    let r = b.plan.radius;
    crate::progress::stage(stage, stages);
    crate::progress::expect(g.triangles.len());
    for (i, t) in g.triangles.iter().enumerate() {
        let ranges: [(usize, usize); 2] = std::array::from_fn(|k| {
            let lo = t.p.iter().map(|p| p[k]).fold(f32::INFINITY, f32::min);
            let hi = t.p.iter().map(|p| p[k]).fold(f32::NEG_INFINITY, f32::max);
            let count = if k == 0 { nx } else { ny };
            (
                ((lo - r - g.bounds.min[k]) / step)
                    .floor()
                    .clamp(0., count as f32) as usize,
                ((hi + r - g.bounds.min[k]) / step)
                    .ceil()
                    .clamp(0., count as f32) as usize,
            )
        });
        // A long sampling runs off the window's thread: how far, and Cancel,
        // at each triangle (a large one takes milliseconds).
        crate::progress::report(i)?;
        let a = std::array::from_fn::<_, 3, _>(|i| t.p[1][i] - t.p[0][i]);
        let c = std::array::from_fn::<_, 3, _>(|i| t.p[2][i] - t.p[0][i]);
        let n = [
            a[1] * c[2] - a[2] * c[1],
            a[2] * c[0] - a[0] * c[2],
            a[0] * c[1] - a[1] * c[0],
        ];
        let slope = (n[2].abs() / (n.iter().map(|v| v * v).sum::<f32>().sqrt()))
            .clamp(0., 1.)
            .acos()
            .to_degrees();
        let wanted = slope >= low - 0.001
            && slope <= high + 0.001
            && picked.is_none_or(|ids| t.feature.is_some_and(|f| ids.contains(&f)));
        for y in ranges[1].0..ranges[1].1 {
            for x in ranges[0].0..ranges[0].1 {
                let xy = [
                    g.bounds.min[0] + x as f32 * step,
                    g.bounds.min[1] + y as f32 * step,
                ];
                let i = y * nx + x;
                if let Some(height) = drop_triangle(&t.p, xy, r, b.plan.shape == CutterShape::Ball)
                {
                    // Where two faces hold the cutter at the same height (at
                    // the edge between them) a wanted one keeps the sample.
                    match z[i] {
                        Some(v) if height <= v - TIE => {}
                        Some(v) if height < v + TIE => {
                            z[i] = Some(height.max(v));
                            selected[i] |= wanted;
                        }
                        _ => {
                            z[i] = Some(height);
                            selected[i] = wanted;
                        }
                    }
                }
            }
        }
    }
    // Only cutter centers over the model XY silhouette. This also avoids joining disconnected islands.
    let silhouette: Vec<Polygon> = g.silhouette(false, op)?.into();
    crate::progress::stage(stage + 1, stages);
    crate::progress::expect(ny);
    for y in 0..ny {
        crate::progress::report(y)?;
        for x in 0..nx {
            let i = y * nx + x;
            let xy = (
                g.bounds.min[0] as f64 + x as f64 * step as f64,
                g.bounds.min[1] as f64 + y as f64 * step as f64,
            );
            // Picked faces keep every height: the cells round them hold the
            // cutter up where it reaches past their edge (`cells`).
            if (!selected[i] && picked.is_none()) || !contains_or_edge(&silhouette, xy) {
                z[i] = None;
            } else {
                z[i] = z[i].map(|h| h + offset + tolerance);
            }
        }
    }
    // A cell with a corner where the cutter touches a picked face is cut, and
    // so is each cell beside one: a pass then runs on past the face's edge
    // into the corner it makes with the next face, as it does over the whole
    // part, rather than lifting off where a steep foot leaves no cell all of
    // whose corners touch the face.
    let cells = picked.map(|_| {
        let hit: Vec<bool> = (0..(nx - 1) * (ny - 1))
            .map(|c| {
                let (x, y) = (c % (nx - 1), c / (nx - 1));
                [(x, y), (x + 1, y), (x, y + 1), (x + 1, y + 1)]
                    .iter()
                    .any(|&(x, y)| selected[y * nx + x])
            })
            .collect();
        (0..hit.len())
            .map(|c| {
                let (x, y) = ((c % (nx - 1)) as isize, (c / (nx - 1)) as isize);
                (-1..=1).any(|dy| {
                    (-1..=1).any(|dx| {
                        let (x, y) = (x + dx, y + dy);
                        x >= 0
                            && y >= 0
                            && (x as usize) < nx - 1
                            && (y as usize) < ny - 1
                            && hit[y as usize * (nx - 1) + x as usize]
                    })
                })
            })
            .collect()
    });
    Ok(SurfaceGrid {
        nx,
        ny,
        step,
        origin: [g.bounds.min[0], g.bounds.min[1]],
        z,
        cells,
    })
}
fn contains(paths: &[Polygon], p: (f64, f64)) -> bool {
    let mut inside = false;
    for path in paths {
        for (a, b) in path
            .iter()
            .zip(path.iter().cycle().skip(1))
            .take(path.len())
        {
            if (a.1 > p.1) != (b.1 > p.1) && p.0 < (b.0 - a.0) * (p.1 - a.1) / (b.1 - a.1) + a.0 {
                inside = !inside;
            }
        }
    }
    inside
}
fn contains_or_edge(paths: &[Polygon], p: (f64, f64)) -> bool {
    contains(paths, p)
        || paths.iter().any(|path| {
            path.iter()
                .zip(path.iter().cycle().skip(1))
                .take(path.len())
                .any(|(a, b)| {
                    let v = (b.0 - a.0, b.1 - a.1);
                    let l2 = v.0 * v.0 + v.1 * v.1;
                    let t = if l2 < 1e-16 {
                        0.
                    } else {
                        (((p.0 - a.0) * v.0 + (p.1 - a.1) * v.1) / l2).clamp(0., 1.)
                    };
                    (p.0 - a.0 - t * v.0).powi(2) + (p.1 - a.1 - t * v.1).powi(2) < 0.000004
                })
        })
}
impl SurfaceGrid {
    // Conservative cell maximum; do not interpolate below sampled tool clearance.
    fn sample(&self, xy: [f32; 2]) -> Option<f32> {
        let u = (xy[0] - self.origin[0]) / self.step;
        let v = (xy[1] - self.origin[1]) / self.step;
        if u < -0.0001 || v < -0.0001 {
            return None;
        }
        let u = u.max(0.);
        let v = v.max(0.);
        let x = u.floor() as usize;
        let y = v.floor() as usize;
        if x + 1 >= self.nx
            || y + 1 >= self.ny
            || self
                .cells
                .as_ref()
                .is_some_and(|cells| !cells[y * (self.nx - 1) + x])
        {
            return None;
        }
        let values = [
            self.z[y * self.nx + x]?,
            self.z[y * self.nx + x + 1]?,
            self.z[(y + 1) * self.nx + x]?,
            self.z[(y + 1) * self.nx + x + 1]?,
        ];
        Some(values.into_iter().fold(f32::NEG_INFINITY, f32::max))
    }
}
fn parallel(setup: &Setup, op: &Operation, tool: &Tool, mesh: &Mesh) -> Result<Plan, String> {
    let mut b = Builder::new(setup, op, tool, mesh)?;
    if !["Ball end mill", "Flat end mill"].contains(&get(&tool.p, "type")) {
        return Err("Parallel finishing requires a ball or flat end mill.".into());
    }
    let g = Geometry::new(setup, mesh);
    let groups = picked_groups(op, mesh)?;
    let groups: Vec<Option<&HashSet<usize>>> = match &groups {
        None => vec![None],
        Some(groups) => groups.iter().map(Some).collect(),
    };
    b.plan.levels = 0;
    b.plan.target = g.bounds.min[2];
    // Each group of touching picked faces is finished before the next.
    // How far: each grid's triangles, its outline, then its rows (stages).
    let stages = 3 * groups.len();
    for (k, picked) in groups.into_iter().enumerate() {
        let grid = finish_grid(&g, op, &b, picked, (3 * k, stages))?;
        parallel_rows(&mut b, &g, op, &grid, (3 * k + 2, stages))?;
    }
    b.finish()
}
/// The zigzag rows of Parallel finishing over `grid`.
fn parallel_rows(
    b: &mut Builder,
    g: &Geometry,
    op: &Operation,
    grid: &SurfaceGrid,
    (stage, stages): (usize, usize),
) -> Result<(), String> {
    let spacing = number(op, "surface_stepover")?;
    let (sin, cos) = number(op, "direction")?.to_radians().sin_cos();
    let corners = [
        [g.bounds.min[0], g.bounds.min[1]],
        [g.bounds.max[0], g.bounds.min[1]],
        [g.bounds.max[0], g.bounds.max[1]],
        [g.bounds.min[0], g.bounds.max[1]],
    ];
    let along: Vec<_> = corners.iter().map(|p| p[0] * cos + p[1] * sin).collect();
    let across: Vec<_> = corners.iter().map(|p| -p[0] * sin + p[1] * cos).collect();
    let min = |v: &Vec<f32>| v.iter().copied().fold(f32::INFINITY, f32::min);
    let max = |v: &Vec<f32>| v.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let rows = ((max(&across) - min(&across)) / spacing).ceil() as usize;
    let count = ((max(&along) - min(&along)) / grid.step).ceil() as usize;
    if rows > CEILING || count > CEILING || rows.saturating_mul(count) > CEILING {
        return Err(too_many("samples"));
    }
    b.plan.levels += rows;
    crate::progress::stage(stage, stages);
    crate::progress::expect(rows + 1);
    for row in 0..=rows {
        crate::progress::report(row)?;
        let v = min(&across) + row as f32 * spacing;
        let mut stroke = vec![];
        for j in 0..=count {
            let index = if row % 2 == 0 { j } else { count - j };
            let u = min(&along) + (max(&along) - min(&along)) * index as f32 / count.max(1) as f32;
            let xy = [u * cos - v * sin, u * sin + v * cos];
            if let Some(z) = grid.sample(xy) {
                stroke.push([xy[0], xy[1], z]);
            } else {
                b.stroke(&stroke)?;
                stroke.clear();
            }
        }
        b.stroke(&stroke)?;
    }
    Ok(())
}
fn waterline(setup: &Setup, op: &Operation, tool: &Tool, mesh: &Mesh) -> Result<Plan, String> {
    let mut b = Builder::new(setup, op, tool, mesh)?;
    if get(&tool.p, "type") != "Flat end mill" {
        return Err("Waterline preview currently requires a flat end mill.".into());
    }
    let g = Geometry::new(setup, mesh);
    // Selected geometry finishes only the picked faces: a loop is cut where
    // the face nearest the cutter is one of them, and only down to the
    // lowest of them. Each group of touching picked faces gets all its
    // levels before the next.
    let picked = picked_groups(op, mesh)?;
    let group_of = |t: &Triangle| {
        let f = t.feature?;
        picked.as_ref()?.iter().position(|g| g.contains(&f))
    };
    let allowance = number(op, "allowance")?;
    let target = g.bounds.min[2] + number(op, "bottom")?;
    let top = g.bounds.max[2] + number(op, "top")?.max(0.);
    let down = number(op, "stepdown")?;
    if allowance < 0.
        || down <= 0.
        || down > b.plan.flute_length
        || target < g.bounds.min[2]
        || target >= top
        || top - target >= b.plan.tool_length
    {
        return Err("Use nonnegative allowances, a target above model bottom, positive stepdown within flute length, and adequate tool reach.".into());
    }
    // Every level's loops go round what of the model stands above it, so
    // the cutter stays within its diameter and the allowance of the model's
    // outline: material beyond that, as in the corners of the outline's box,
    // does not keep the start up.
    let near: Vec<Polygon> = g
        .silhouette(false, op)?
        .inflate(
            (2. * b.plan.radius + allowance + (b.plan.radius * 0.01).max(0.02)) as f64,
            JoinType::Round,
            EndType::Polygon,
            2.,
        )
        .into();
    let cutting_top = crate::rest::start_over(
        setup,
        &crate::rest::Reach::new(&near),
        target,
        top,
        number(op, "top")?,
    );
    b.plan.rest_top = (cutting_top < top - 0.0001).then_some(cutting_top);
    // No level under the lowest picked face touches one.
    let target = match picked {
        Some(_) => {
            let lowest = (g.triangles.iter())
                .filter(|t| group_of(t).is_some())
                .flat_map(|t| t.p.iter().map(|p| p[2]))
                .fold(f32::INFINITY, f32::min);
            if lowest >= cutting_top {
                return Err("The picked faces are above the remaining stock: nothing is left on them to finish.".into());
            }
            target.max(lowest)
        }
        None => target,
    };
    let levels = ((cutting_top - target) / down).ceil().max(1.) as usize;
    if levels > CEILING {
        return Err(too_many("moves"));
    }
    b.plan.levels = levels;
    b.plan.target = target;
    let climb = pocket_setting(&op.p, "pocket_milling") == "Climb";
    if !crate::model::FACING_MILLING.contains(&pocket_setting(&op.p, "pocket_milling")) {
        return Err("Choose Climb or Conventional.".into());
    }
    b.plan.milling_direction = if climb { "Climb" } else { "Conventional" };
    // The cuts of each group of picked faces, level by level, made once
    // every level is worked out.
    let mut later: Vec<Vec<Vec<[f32; 3]>>> = vec![vec![]; picked.as_ref().map_or(0, Vec::len)];
    crate::progress::expect(levels);
    for level in 1..=levels {
        crate::progress::report(level)?;
        let z = (cutting_top - level as f32 * down).max(target);
        let mut polys = vec![];
        // Projection of all geometry above the level accounts for the vertical cutter/shank corridor.
        for triangle in &g.triangles {
            let mut p = vec![];
            for (a, c) in triangle
                .p
                .iter()
                .zip(triangle.p.iter().cycle().skip(1))
                .take(3)
            {
                if a[2] >= z {
                    p.push((a[0] as f64, a[1] as f64));
                }
                if (a[2] >= z) != (c[2] >= z) {
                    let t = (z - a[2]) / (c[2] - a[2]);
                    p.push((
                        (a[0] + t * (c[0] - a[0])) as f64,
                        (a[1] + t * (c[1] - a[1])) as f64,
                    ));
                }
            }
            polys.push(p);
        }
        // The faces each loop point is nearest, for picked faces only.
        let near_faces = picked.as_ref().map(|_| {
            Nearest::new(
                (g.triangles.iter().zip(&polys))
                    .filter(|(_, p)| !p.is_empty())
                    .map(|(t, p)| (group_of(t), p.clone()))
                    .collect(),
                b.plan.radius.max(0.5),
            )
        });
        let paths: Vec<Polygon> = region(polys)?
            .inflate(
                (b.plan.radius + allowance + (b.plan.radius * 0.01).max(0.02)) as f64,
                JoinType::Round,
                EndType::Polygon,
                2.,
            )
            .simplify(0.001, false)
            .into();
        for path in paths {
            if path.len() < 3 {
                continue;
            }
            let mut points: Vec<_> = path.iter().map(|p| [p.0 as f32, p.1 as f32, z]).collect();
            if climb == b.plan.spindle_clockwise {
                points.reverse();
            }
            let Some(near) = &near_faces else {
                points.push(points[0]);
                b.stroke(&points)?;
                continue;
            };
            // The loop in short steps, each point kept where the cutter
            // touches a picked face there, and one point on past each end of
            // a kept run, so the cutter finishes up to the face's edge.
            let reach = b.plan.radius + allowance + (b.plan.radius * 0.01).max(0.02);
            let spacing = (b.plan.radius / 4.).clamp(0.05, 0.5);
            let dense = densify(&points, spacing);
            let touched: Vec<Option<usize>> = dense
                .iter()
                .map(|p| near.group(p[0] as f64, p[1] as f64, (reach + spacing) as f64))
                .collect();
            let n = dense.len();
            for (group, cuts) in later.iter_mut().enumerate() {
                let on = |i: usize| touched[i % n] == Some(group);
                let kept: Vec<bool> = (0..n)
                    .map(|i| on(i + n - 1) || on(i) || on(i + 1))
                    .collect();
                if !kept.iter().any(|k| *k) {
                    continue;
                }
                if kept.iter().all(|k| *k) {
                    let mut closed = dense.clone();
                    closed.push(closed[0]);
                    cuts.push(closed);
                    continue;
                }
                // Runs, from the first point not kept round the loop.
                let start = kept.iter().position(|k| !k).unwrap();
                let mut run = vec![];
                for i in 1..=n {
                    let i = (start + i) % n;
                    if kept[i] {
                        run.push(dense[i]);
                    } else if !run.is_empty() {
                        cuts.push(std::mem::take(&mut run));
                    }
                }
                if !run.is_empty() {
                    cuts.push(run);
                }
            }
        }
    }
    for cut in later.iter().flatten() {
        b.stroke(cut)?;
    }
    b.finish()
}
/// `points`, a closed loop, with points between its corners at most
/// `spacing` apart; not closed again.
fn densify(points: &[[f32; 3]], spacing: f32) -> Vec<[f32; 3]> {
    let mut out = vec![];
    for (i, a) in points.iter().enumerate() {
        let b = points[(i + 1) % points.len()];
        let steps = ((a[0] - b[0]).hypot(a[1] - b[1]) / spacing).ceil().max(1.) as usize;
        for k in 0..steps {
            let t = k as f32 / steps as f32;
            out.push([a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1]), a[2]]);
        }
    }
    out
}
/// The faces of one waterline level as seen from above (the part of each
/// triangle over the level), bucketed for asking which is nearest a point.
struct Nearest {
    /// The group of picked faces each is of (`picked_groups`), and its
    /// outline.
    faces: Vec<(Option<usize>, Polygon)>,
    cell: f64,
    buckets: HashMap<(i64, i64), Vec<usize>>,
}
impl Nearest {
    fn new(faces: Vec<(Option<usize>, Polygon)>, cell: f32) -> Self {
        let cell = cell as f64;
        let mut buckets: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
        for (i, (_, p)) in faces.iter().enumerate() {
            let lo = |k: fn(&(f64, f64)) -> f64| p.iter().map(k).fold(f64::INFINITY, f64::min);
            let hi = |k: fn(&(f64, f64)) -> f64| p.iter().map(k).fold(f64::NEG_INFINITY, f64::max);
            let (x0, x1) = (
                (lo(|q| q.0) / cell).floor() as i64,
                (hi(|q| q.0) / cell).floor() as i64,
            );
            let (y0, y1) = (
                (lo(|q| q.1) / cell).floor() as i64,
                (hi(|q| q.1) / cell).floor() as i64,
            );
            for x in x0..=x1 {
                for y in y0..=y1 {
                    buckets.entry((x, y)).or_default().push(i);
                }
            }
        }
        Self {
            faces,
            cell,
            buckets,
        }
    }
    /// The group of a picked face nearest (x, y) within `within`, or as
    /// near to 5 µm: at a corner between two faces the cutter touches both.
    fn group(&self, x: f64, y: f64, within: f64) -> Option<usize> {
        let span = (within / self.cell).ceil() as i64;
        let (cx, cy) = (
            (x / self.cell).floor() as i64,
            (y / self.cell).floor() as i64,
        );
        let mut seen = HashSet::new();
        let mut found: Vec<(f64, Option<usize>)> = vec![];
        for gx in cx - span..=cx + span {
            for gy in cy - span..=cy + span {
                for &i in self.buckets.get(&(gx, gy)).into_iter().flatten() {
                    if seen.insert(i) {
                        let (group, p) = &self.faces[i];
                        found.push((distance_to(p, (x, y)), *group));
                    }
                }
            }
        }
        let nearest = found.iter().map(|f| f.0).fold(f64::INFINITY, f64::min);
        if nearest > within {
            return None;
        }
        (found.iter())
            .filter(|f| f.0 <= nearest + 0.005)
            .find_map(|f| f.1)
    }
}
/// How far (x, y) is from the outline `p` (a point or segment too), 0 inside it.
fn distance_to(p: &Polygon, q: (f64, f64)) -> f64 {
    if p.len() >= 3 && contains(std::slice::from_ref(p), q) {
        return 0.;
    }
    (0..p.len())
        .map(|i| {
            let (a, b) = (p[i], p[(i + 1) % p.len()]);
            let v = (b.0 - a.0, b.1 - a.1);
            let l2 = v.0 * v.0 + v.1 * v.1;
            let t = if l2 < 1e-16 {
                0.
            } else {
                (((q.0 - a.0) * v.0 + (q.1 - a.1) * v.1) / l2).clamp(0., 1.)
            };
            (q.0 - a.0 - t * v.0).hypot(q.1 - a.1 - t * v.1)
        })
        .fold(f64::INFINITY, f64::min)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::{
        mesh,
        model::Machine,
        simulation::{self, Simulation},
    };
    fn quad(t: &mut Vec<[[f32; 3]; 3]>, p: [[f32; 3]; 4]) {
        t.push([p[0], p[1], p[2]]);
        t.push([p[0], p[2], p[3]]);
    }
    fn block() -> Mesh {
        let mut t = vec![];
        quad(
            &mut t,
            [[0., 0., 8.], [20., 0., 8.], [20., 16., 8.], [0., 16., 8.]],
        );
        quad(
            &mut t,
            [[0., 0., 0.], [0., 16., 0.], [20., 16., 0.], [20., 0., 0.]],
        );
        quad(
            &mut t,
            [[0., 0., 0.], [20., 0., 0.], [20., 0., 8.], [0., 0., 8.]],
        );
        quad(
            &mut t,
            [[20., 16., 0.], [0., 16., 0.], [0., 16., 8.], [20., 16., 8.]],
        );
        quad(
            &mut t,
            [[0., 16., 0.], [0., 0., 0.], [0., 0., 8.], [0., 16., 8.]],
        );
        quad(
            &mut t,
            [[20., 0., 0.], [20., 16., 0.], [20., 16., 8.], [20., 0., 8.]],
        );
        mesh::normalize(t).unwrap()
    }
    fn fixture(s: Strategy) -> (Setup, Operation, Tool) {
        let mut setup = Setup::new(Machine::new(3), "G54");
        setup.p.insert("stock_top".into(), "2".into());
        for k in ["stock_left", "stock_right", "stock_front", "stock_back"] {
            setup.p.insert(k.into(), "3".into());
        }
        let mut tool = Tool::new(
            "Audit cutter",
            if s == Strategy::Drill {
                "Drill"
            } else if s == Strategy::Parallel {
                "Ball end mill"
            } else {
                "Flat end mill"
            },
            "4",
        );
        tool.p.insert("length".into(), "60".into());
        tool.p.insert("stickout".into(), "65".into());
        let mut op = Operation::new(s, tool.id.clone());
        op.geometry.clear();
        for (k, v) in [
            ("allowance", "0"),
            ("axial_allowance", "0"),
            ("surface_stepover", "1"),
            ("tolerance", "0.05"),
        ] {
            op.p.insert(k.into(), v.into());
        }
        (setup, op, tool)
    }
    fn assert_plan(plan: Plan) {
        assert!(!plan.moves.is_empty());
        assert!(plan.duration > 0. && plan.duration.is_finite());
        for pair in plan.moves.windows(2) {
            assert_eq!(pair[0].to, pair[1].from);
            assert_eq!(pair[0].end, pair[1].start);
        }
        for m in &plan.moves {
            assert!(m.start < m.end && m.to.iter().chain(&m.from).all(|v| v.is_finite()));
        }
        let mut direct = Simulation::new(plan.clone());
        let mut incremental = Simulation::new(plan);
        direct.seek(0.63);
        for i in 0..=63 {
            incremental.seek(i as f32 / 100.);
        }

        for (a, b) in direct.stock.heights.iter().zip(&incremental.stock.heights) {
            assert!((a - b).abs() < 0.003, "stock mismatch {a} {b}");
        }
        let partial = direct.stock.heights.clone();
        direct.seek(1.);
        assert!(direct.stock.removed_volume() > 0.);
        direct.seek(0.63);
        assert_eq!(direct.stock.heights, partial);
        direct.seek(0.);
        assert_eq!(direct.stock.removed_volume(), 0.);
    }
    #[test]
    fn depth_kernels_use_prior_flat_cuts_and_keep_final_targets_and_links() {
        for strategy in [Strategy::Contour, Strategy::Drill, Strategy::Waterline] {
            let mesh = if strategy == Strategy::Drill {
                crate::mesh::load(
                    &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("examples/four-setup-block.3mf"),
                )
                .unwrap()
            } else {
                block()
            };
            let (mut setup, mut op, tool) = fixture(strategy);
            if strategy == Strategy::Drill {
                select_top(&mut op, &mesh);
                op.p.insert("cycle".into(), "Deep drilling".into());
                op.p.insert("peck".into(), "2".into());
            }
            let original = generate(&setup, &op, &tool, &mesh).unwrap();
            let frame = crate::volume::Frame::new(&setup, &mesh);
            let mut volume = crate::volume::Volume::new(original.stock, frame);
            let top = Geometry::new(&setup, &mesh).bounds.max[2] - 2.;
            // Known prior machining across the complete cap, including the
            // outside-contour envelope; no dependence on renderer resolution.
            let r = original.stock.dimensions()[1];
            volume.cut(
                [original.stock.min[0] - r, 0., top],
                [original.stock.max[0] + r, 0., top],
                r,
                80.,
                CutterShape::Flat,
                frame,
                (0, 0),
            );
            setup.preview_remaining = Some(std::sync::Arc::new(crate::rest::RestStock::new(
                &volume, frame,
            )));
            let planned = generate(&setup, &op, &tool, &mesh).unwrap();
            assert_eq!(planned.rest_top, Some(top), "{strategy:?}");
            assert!(planned.levels < original.levels, "{strategy:?}");
            assert_eq!(planned.target, original.target);
            assert!(
                planned.duration < original.duration,
                "{strategy:?}: {} vs {}",
                planned.duration,
                original.duration
            );
            assert_plan(planned);
        }
    }
    #[test]
    fn contour_offsets_actual_profile_and_reverses_for_spindle_and_milling() {
        let mesh = block();
        let (setup, mut op, tool) = fixture(Strategy::Contour);
        let g = Geometry::new(&setup, &mesh);
        let plan = generate(&setup, &op, &tool, &mesh).unwrap();
        for m in plan.moves.iter().filter(|m| m.cutting_pass) {
            for p in [m.from, m.to] {
                let dx = (g.bounds.min[0] - p[0]).max(p[0] - g.bounds.max[0]).max(0.);
                let dy = (g.bounds.min[1] - p[1]).max(p[1] - g.bounds.max[1]).max(0.);
                assert!(dx * dx + dy * dy >= plan.radius.powi(2) - 0.01);
            }
        }
        let winding = |p: &Plan| {
            let points: Polygon = p
                .moves
                .iter()
                .filter(|m| m.cutting_pass && (m.to[2] - p.target).abs() < 0.001)
                .map(|m| (m.to[0] as f64, m.to[1] as f64))
                .collect();
            area(&points)
        };
        assert!(winding(&plan) < 0.);
        op.p.insert("pocket_milling".into(), "Conventional".into());
        let other = generate(&setup, &op, &tool, &mesh).unwrap();
        assert!(winding(&other) > 0.);
        op.p.insert("spindle_rotation".into(), "Counterclockwise (CCW)".into());
        assert!(winding(&generate(&setup, &op, &tool, &mesh).unwrap()) < 0.);
        assert_plan(plan);
    }
    #[test]
    fn an_outside_contour_starts_under_what_was_cleared_round_the_part_not_its_box() {
        let mesh = block();
        let (mut setup, op, tool) = fixture(Strategy::Contour);
        for k in ["stock_left", "stock_right", "stock_front", "stock_back"] {
            setup.p.insert(k.into(), "8".into());
        }
        let g = Geometry::new(&setup, &mesh);
        let plan = generate(&setup, &op, &tool, &mesh).unwrap();
        let top = plan.stock.max[2];
        let frame = crate::volume::Frame::new(&setup, &mesh);
        // A Ø`2 × radius` cutter once round the part, its edge on the
        // part's sides, down to the part's top: the stock over the part
        // stays.
        let cleared = |radius: f32| {
            let mut v = crate::volume::Volume::new(plan.stock, frame);
            let (lo, hi) = (g.bounds.min, g.bounds.max);
            let corners = [
                [lo[0] - radius, lo[1] - radius],
                [hi[0] + radius, lo[1] - radius],
                [hi[0] + radius, hi[1] + radius],
                [lo[0] - radius, hi[1] + radius],
            ];
            for i in 0..4 {
                let (a, b) = (corners[i], corners[(i + 1) % 4]);
                v.cut(
                    [a[0], a[1], hi[2]],
                    [b[0], b[1], hi[2]],
                    radius,
                    20.,
                    CutterShape::Flat,
                    frame,
                    (0, 0),
                );
            }
            let mut setup = setup.clone();
            setup.preview_remaining =
                Some(std::sync::Arc::new(crate::rest::RestStock::new(&v, frame)));
            setup
        };
        // A Ø8 cutter cleared all the Ø4 contour sweeps round the part: it
        // starts at the part's top, though the box round it holds the
        // stock over the part.
        let wide = cleared(4.);
        let rest = wide.preview_remaining.as_ref().unwrap();
        assert_eq!(rest.top(g.bounds, plan.target, top), top);
        let lower = generate(&wide, &op, &tool, &mesh).unwrap();
        assert!(
            lower
                .rest_top
                .is_some_and(|z| (z - g.bounds.max[2]).abs() < 0.001),
            "{:?}",
            lower.rest_top
        );
        assert!(lower.levels < plan.levels);
        assert!(
            (lower.moves.iter())
                .filter(|m| m.cutting_pass)
                .all(|m| m.to[2] < g.bounds.max[2])
        );
        // A Ø2 one left stock in the outer half of the sweeps: from the top.
        let narrow = generate(&cleared(1.), &op, &tool, &mesh).unwrap();
        assert_eq!(narrow.rest_top, None);
        assert_eq!(narrow.levels, plan.levels);
    }
    /// A closed path through `corners` (its last point its first), with
    /// points every `step` mm along its sides.
    fn path(corners: &[[f32; 2]], step: f32) -> Vec<[f32; 2]> {
        let mut out = vec![];
        for (i, a) in corners.iter().enumerate() {
            let b = corners[(i + 1) % corners.len()];
            let n = (((b[0] - a[0]).hypot(b[1] - a[1])) / step).ceil().max(1.) as usize;
            out.extend((0..n).map(|k| {
                let t = k as f32 / n as f32;
                [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t]
            }));
        }
        out.push(out[0]);
        out
    }
    fn circle(r: f32, n: usize) -> Vec<[f32; 2]> {
        let mut out: Vec<[f32; 2]> = (0..n)
            .map(|i| {
                let a = i as f32 * std::f32::consts::TAU / n as f32;
                [r * a.cos(), r * a.sin()]
            })
            .collect();
        out.push(out[0]);
        out
    }
    #[test]
    fn tabs_go_on_straight_sides_evenly_spaced_never_on_corners_short_segments_or_tight_arcs() {
        let tabs = |count, width: f32| Tabs {
            count,
            width,
            lift: width + 4.,
            top: 1.,
        };
        // A 100 × 40 rectangle, its sides split into 1 mm segments: 4 tabs
        // 6 mm wide, each wholly on one side, clear of the corners by the
        // margin, 1 mm apart at least, near the even spacing of 70 mm.
        let points = path(&[[0., 0.], [100., 0.], [100., 40.], [0., 40.]], 1.);
        let outline = Outline::new(&points);
        let spans = outline.spans(&tabs(4, 6.)).unwrap();
        assert_eq!(spans.len(), 4);
        let corners = [0., 100., 140., 240., 280.];
        for (i, &(a, b)) in spans.iter().enumerate() {
            assert!((b - a - 10.).abs() < 1e-4);
            assert!(
                corners
                    .iter()
                    .all(|&c| a - TAB_MARGIN >= c - 1e-3 || b + TAB_MARGIN <= c + 1e-3),
                "{a}..{b} spans a corner"
            );
            let even = (i as f32 + 0.5) * 70.;
            assert!(((a + b) / 2. - even).abs() <= 6., "{i}: {a}..{b}");
            if i > 0 {
                assert!(a - spans[i - 1].1 >= TAB_GAP - 1e-3);
            }
        }
        // A notch of 2 mm steps in the bottom side: none lands on it, though
        // the even spacing puts the first there.
        let notched = path(
            &[
                [0., 0.],
                [30., 0.],
                [30., 2.],
                [32., 2.],
                [32., 0.],
                [34., 0.],
                [34., 2.],
                [36., 2.],
                [36., 0.],
                [100., 0.],
                [100., 40.],
                [0., 40.],
            ],
            1.,
        );
        let outline = Outline::new(&notched);
        let at = |q: [f32; 2]| outline.along[notched.iter().position(|p| *p == q).unwrap()];
        let notch = (at([30., 0.]), at([36., 0.]));
        for (a, b) in outline.spans(&tabs(4, 6.)).unwrap() {
            assert!(
                b + TAB_MARGIN <= notch.0 + 1e-3 || a - TAB_MARGIN >= notch.1 - 1e-3,
                "{a}..{b}"
            );
        }
        // A Ø10 circle turns too much under any tab; a Ø400 one barely.
        let small = circle(5., 72);
        let error = Outline::new(&small).spans(&tabs(2, 3.)).unwrap_err();
        assert!(
            error.starts_with("Only 0 of 2 tabs 3.00 mm wide fit"),
            "{error}"
        );
        let large = circle(200., 720);
        assert_eq!(Outline::new(&large).spans(&tabs(6, 6.)).unwrap().len(), 6);
        // More tabs than the sides hold are refused with how many fit.
        let error = Outline::new(&points).spans(&tabs(30, 6.)).unwrap_err();
        assert!(error.starts_with("Only "), "{error}");
    }
    #[test]
    fn contour_tabs_keep_the_part_joined_to_the_stock_around_it() {
        let mesh = block();
        let (mut setup, mut op, tool) = fixture(Strategy::Contour);
        // Stock beyond the Ø4 cut for the tabs to hold on to.
        for k in ["stock_left", "stock_right", "stock_front", "stock_back"] {
            setup.p.insert(k.into(), "8".into());
        }
        let g = Geometry::new(&setup, &mesh);
        // What stays beside the part, above 1 mm, once the contour is cut.
        let left = |plan: &Plan| {
            let mut sim = Simulation::new(plan.clone());
            sim.seek(1.);
            let s = &sim.stock;
            (0..s.ny)
                .flat_map(|y| (0..s.nx).map(move |x| (x, y)))
                .filter(|&(x, y)| {
                    let px = s.bounds.min[0] + (x as f32 + 0.5) * s.dx;
                    let py = s.bounds.min[1] + (y as f32 + 0.5) * s.dy;
                    // In the band the cutter sweeps, 0.5–3.5 mm from the part.
                    let dx = (g.bounds.min[0] - px).max(px - g.bounds.max[0]).max(0.);
                    let dy = (g.bounds.min[1] - py).max(py - g.bounds.max[1]).max(0.);
                    let band = (0.5..3.5).contains(&(dx * dx + dy * dy).sqrt());
                    band && s.heights[y * s.nx + x] > g.bounds.min[2] + 1.
                })
                .count()
        };
        let through = generate(&setup, &op, &tool, &mesh).unwrap();
        assert_eq!(left(&through), 0, "without tabs the part is cut free");
        op.p.insert("tabs".into(), "Yes".into());
        op.p.insert("tab_count".into(), "4".into());
        op.p.insert("tab_width".into(), "3".into());
        op.p.insert("tab_height".into(), "1.5".into());
        let plan = generate(&setup, &op, &tool, &mesh).unwrap();
        let top = g.bounds.min[2] + 1.5;
        assert!(left(&plan) > 0, "the tabs are left");
        // Only the last level, 0 mm, and its finishing pass are below the
        // tabs' top: 4 tabs on each, rising straight up and coming back down
        // at the plunge feed.
        let vertical = |m: &&&Move| m.from[0] == m.to[0] && m.from[1] == m.to[1];
        let cutting: Vec<&Move> = plan.moves.iter().filter(|m| m.cutting_pass).collect();
        let rises: Vec<_> = cutting
            .iter()
            .filter(vertical)
            .filter(|m| m.to[2] > m.from[2])
            .collect();
        let drops: Vec<_> = cutting
            .iter()
            .filter(vertical)
            .filter(|m| m.to[2] < m.from[2])
            .collect();
        assert_eq!((rises.len(), drops.len()), (8, 8));
        for m in rises.iter().chain(&drops) {
            assert!((m.to[2].max(m.from[2]) - top).abs() < 0.0001);
            assert!((m.to[2].min(m.from[2]) - plan.target).abs() < 0.0001);
        }
        for m in &drops {
            assert_eq!(m.feed, number(&op, "feed").unwrap() * 0.25);
        }
        // Over each tab the tool's centre travels its width plus the tool's
        // diameter at the tabs' top.
        let mut over = 0.;
        for m in cutting
            .iter()
            .filter(|m| m.from[2] == top && m.to[2] == top)
        {
            over += ((m.to[0] - m.from[0]).powi(2) + (m.to[1] - m.from[1]).powi(2)).sqrt();
        }
        assert!((over - 8. * (3. + 4.)).abs() < 0.01, "{over}");
        // The levels above the tabs go round in full, as without tabs.
        let above = |p: &Plan| {
            p.moves
                .iter()
                .filter(|m| m.cutting_pass && m.to[2] > top)
                .count()
        };
        assert_eq!(above(&plan), above(&through));
        assert_plan(plan);

        op.p.insert("tab_count".into(), "10".into());
        op.p.insert("tab_width".into(), "6".into());
        let error = generate(&setup, &op, &tool, &mesh).unwrap_err();
        assert!(
            error.starts_with("10 tabs 6.00 mm wide do not fit"),
            "{error}"
        );
        op.p.insert("tab_count".into(), "4".into());
        op.p.insert("tab_height".into(), "100".into());
        let error = generate(&setup, &op, &tool, &mesh).unwrap_err();
        assert!(error.starts_with("Tab height must be less than"), "{error}");
        op.p.insert("tab_height".into(), "0".into());
        assert!(generate(&setup, &op, &tool, &mesh).is_err());
        // With 3 mm of stock beside the part the Ø4 cut runs past its edge:
        // the tabs would join the part to nothing.
        op.p.insert("tab_height".into(), "1.5".into());
        for k in ["stock_left", "stock_right", "stock_front", "stock_back"] {
            setup.p.insert(k.into(), "3".into());
        }
        let error = generate(&setup, &op, &tool, &mesh).unwrap_err();
        assert!(
            error.starts_with("4 of 4 tabs join the part to no stock")
                && error.contains("at least 6.1 mm"),
            "{error}"
        );
        // A cut that stops above the stock's bottom leaves the part joined
        // by its skin: the tabs only thicken it.
        setup.p.insert("stock_bottom".into(), "2".into());
        assert!(generate(&setup, &op, &tool, &mesh).is_ok());
    }
    #[test]
    fn waterline_obeys_z_levels_and_preserves_model() {
        let mesh = block();
        let (setup, mut op, tool) = fixture(Strategy::Waterline);
        let g = Geometry::new(&setup, &mesh);
        let a = generate(&setup, &op, &tool, &mesh).unwrap();
        for m in a.moves.iter().filter(|m| m.cutting_pass) {
            assert!((m.from[2] - m.to[2]).abs() < 0.0001);
        }
        let mut sim = Simulation::new(a.clone());
        sim.seek(1.);
        for y in 0..sim.stock.ny {
            for x in 0..sim.stock.nx {
                let px = sim.stock.bounds.min[0] + (x as f32 + 0.5) * sim.stock.dx;
                let py = sim.stock.bounds.min[1] + (y as f32 + 0.5) * sim.stock.dy;
                if px > g.bounds.min[0]
                    && px < g.bounds.max[0]
                    && py > g.bounds.min[1]
                    && py < g.bounds.max[1]
                {
                    assert_eq!(
                        sim.stock.heights[y * sim.stock.nx + x],
                        sim.stock.bounds.max[2]
                    );
                }
            }
        }
        op.p.insert("stepdown".into(), "1".into());
        let c = generate(&setup, &op, &tool, &mesh).unwrap();
        assert!(c.levels > a.levels);
        assert_plan(a);
    }
    #[test]
    fn drop_cutter_matches_analytic_ramp_for_ball_and_flat_and_vertical_edge() {
        let p = [[0., 0., 0.], [100., 0., 50.], [0., 100., 0.]];
        let xy = [20., 20.];
        let r = 3.;
        let ball = drop_triangle(&p, xy, r, true).unwrap();
        let flat = drop_triangle(&p, xy, r, false).unwrap();
        assert!((ball - (10. + r * ((1.25f32).sqrt() - 1.))).abs() < 0.0001);
        assert!((flat - 11.5).abs() < 0.0001);
        let vertical = [[0., 0., 0.], [0., 10., 0.], [0., 5., 10.]];
        assert!(
            (drop_triangle(&vertical, [1., 5.], r, true).unwrap() - (10. + 8f32.sqrt() - 3.)).abs()
                < 0.0001
        );
        assert!(drop_triangle(&p, [-10., -10.], r, true).is_none());
    }
    #[test]
    fn parallel_uses_ball_compensation_angle_slope_and_real_heights() {
        let mesh = mesh::normalize(vec![
            [[0., 0., 0.], [20., 0., 10.], [20., 16., 10.]],
            [[0., 0., 0.], [20., 16., 10.], [0., 16., 0.]],
        ])
        .unwrap();
        let (setup, mut op, tool) = fixture(Strategy::Parallel);
        let plan = generate(&setup, &op, &tool, &mesh).unwrap();
        assert_eq!(plan.shape, CutterShape::Ball);
        let heights: Vec<_> = plan
            .moves
            .iter()
            .filter(|m| m.cutting_pass)
            .map(|m| m.to[2])
            .collect();
        assert!(
            heights.iter().copied().fold(f32::NEG_INFINITY, f32::max)
                - heights.iter().copied().fold(f32::INFINITY, f32::min)
                > 5.
        );
        op.p.insert("direction".into(), "90".into());
        let rotated = generate(&setup, &op, &tool, &mesh).unwrap();
        assert_ne!(plan.moves[3].to, rotated.moves[3].to);
        op.p.insert("slope_max".into(), "10".into());
        assert!(generate(&setup, &op, &tool, &mesh).is_err());
        assert_plan(plan);
    }
    /// A half ball R10 on a Ø40 x 4 mm round plate: 64 sides, the dome in
    /// 16 rings from its equator to its pole.
    pub(crate) fn half_ball_on_plate() -> Mesh {
        let mut section = vec![(0., 0.), (20., 0.), (20., 4.)];
        section.extend((0..=16).map(|i| {
            let a = i as f32 * std::f32::consts::FRAC_PI_2 / 16.;
            (if i == 16 { 0. } else { 10. * a.cos() }, 4. + 10. * a.sin())
        }));
        mesh::normalize(revolved(&section)).unwrap()
    }
    #[test]
    fn parallel_finishes_only_the_picked_curved_face() {
        let mesh = half_ball_on_plate();
        let (setup, mut op, tool) = fixture(Strategy::Parallel);
        let g = Geometry::new(&setup, &mesh);
        let centre = [
            (g.bounds.min[0] + g.bounds.max[0]) / 2.,
            (g.bounds.min[1] + g.bounds.max[1]) / 2.,
            g.bounds.min[2] + 4.,
        ];
        // The dome: the face of the highest triangle, picked whole.
        let top = (mesh.faces.iter())
            .filter(|f| f.feature.is_some())
            .max_by(|a, b| {
                let z = |f: &mesh::Face| f.points.iter().map(|p| p[2]).sum::<f32>();
                z(a).total_cmp(&z(b))
            })
            .and_then(|f| f.feature)
            .unwrap();
        let dome = mesh.region_of(top);
        assert!(dome.len() > 64, "{}", dome.len());
        let feeds = |plan: &Plan| -> Vec<[f32; 3]> {
            (plan.moves.iter())
                .filter(|m| m.motion == Motion::Feed)
                .flat_map(|m| [m.from, m.to])
                .collect()
        };
        let off_axis = |p: &[f32; 3]| (p[0] - centre[0]).hypot(p[1] - centre[1]);
        let whole = generate(&setup, &op, &tool, &mesh).unwrap();
        // The whole part: the plate's top is finished too.
        assert!(feeds(&whole).iter().any(|p| off_axis(p) > 15.));

        op.p.insert("boundary".into(), "Selected geometry".into());
        assert!(op.needs_surfaces());
        assert!(generate(&setup, &op, &tool, &mesh).is_err());
        op.surfaces = Some(surface::Selection {
            mesh_fingerprint: mesh.fingerprint.clone(),
            ids: dome,
        });
        let setup_with_op = {
            let mut s = setup.clone();
            s.operations = vec![op.clone()];
            s
        };
        assert!(
            surface::issues(&op, &setup_with_op, Some(&mesh)).is_empty(),
            "{:?}",
            surface::issues(&op, &setup_with_op, Some(&mesh))
        );
        let plan = generate(&setup, &op, &tool, &mesh).unwrap();
        let points = feeds(&plan);
        let r = plan.radius;
        // Only the dome: the cutter goes no further out than into the corner
        // at its foot, at most two sampling cells (each at most 1 mm square)
        // past where it touches the plate there, and never below the plate.
        let corner = ((10. + r).powi(2) - r * r).sqrt();
        assert!(
            points
                .iter()
                .all(|p| off_axis(p) <= corner + 2. * std::f32::consts::SQRT_2),
            "{:?}",
            points.iter().map(off_axis).fold(0., f32::max)
        );
        assert!(points.iter().all(|p| p[2] > centre[2] - 0.01));
        assert!(
            points
                .iter()
                .any(|p| off_axis(p) < 0.5 && p[2] > centre[2] + 9.9)
        );
        // All of it, down to its foot: on rings of the dome near the plate
        // the material left is what finishing the whole part leaves. A ball
        // reaches down to r·R/(R + r), 1.7 mm, in the corner.
        let left = |plan: &Plan, h: f32| {
            let centres: Vec<([f32; 3], [f32; 3])> = (plan.moves.iter())
                .filter(|m| m.motion == Motion::Feed)
                .map(|m| {
                    (
                        [m.from[0], m.from[1], m.from[2] + r],
                        [m.to[0], m.to[1], m.to[2] + r],
                    )
                })
                .collect();
            let ring = (100. - h * h).sqrt();
            (0..72)
                .map(|i| {
                    let a = i as f32 * std::f32::consts::TAU / 72.;
                    let p = [
                        centre[0] + ring * a.cos(),
                        centre[1] + ring * a.sin(),
                        centre[2] + h,
                    ];
                    let near = (centres.iter())
                        .map(|(a, b)| {
                            let v: [f32; 3] = std::array::from_fn(|k| b[k] - a[k]);
                            let l2 = v.iter().map(|x| x * x).sum::<f32>();
                            let t = if l2 < 1e-12 {
                                0.
                            } else {
                                ((0..3).map(|k| (p[k] - a[k]) * v[k]).sum::<f32>() / l2)
                                    .clamp(0., 1.)
                            };
                            (0..3)
                                .map(|k| (p[k] - a[k] - t * v[k]).powi(2))
                                .sum::<f32>()
                                .sqrt()
                        })
                        .fold(f32::INFINITY, f32::min);
                    (near - r).max(0.)
                })
                .collect::<Vec<f32>>()
        };
        // At each of 72 points round each ring.
        for h in [2., 3., 4., 6.] {
            let (dome_left, whole_left) = (left(&plan, h), left(&whole, h));
            for (d, w) in dome_left.iter().zip(&whole_left) {
                assert!(
                    *d <= w + 0.05,
                    "at {h} mm up the dome leaves {d}, the whole part {w}"
                );
            }
        }
        // The ball rides on the dome, never into it (the tessellation lies
        // up to 0.012 mm inside the true sphere).
        for p in &points {
            let ball = [p[0], p[1], p[2] + r];
            let d = (0..3)
                .map(|k| (ball[k] - centre[k]).powi(2))
                .sum::<f32>()
                .sqrt();
            assert!(d >= 10. + r - 0.02, "gouge at {p:?}: {d}");
        }
        // Fewer passes than the whole part.
        assert!(points.len() < feeds(&whole).len());
        assert_plan(plan);
    }
    /// Two half balls R10, 30 mm apart along X, on a 70 x 30 x 4 plate.
    pub(crate) fn two_half_balls() -> Mesh {
        let dome: Vec<(f32, f32)> = (0..=16)
            .map(|i| {
                let a = i as f32 * std::f32::consts::FRAC_PI_2 / 16.;
                (if i == 16 { 0. } else { 10. * a.cos() }, 4. + 10. * a.sin())
            })
            .collect();
        let mut t = vec![];
        for x in [-15., 15.] {
            t.extend(
                revolved(&dome)
                    .into_iter()
                    .map(|tri| tri.map(|p| [p[0] + x, p[1], p[2]])),
            );
        }
        let (a, b) = ([-35., -15., 0.], [35., 15., 4.]);
        quad(
            &mut t,
            [
                [a[0], a[1], b[2]],
                [b[0], a[1], b[2]],
                [b[0], b[1], b[2]],
                [a[0], b[1], b[2]],
            ],
        );
        quad(
            &mut t,
            [
                [a[0], a[1], a[2]],
                [a[0], b[1], a[2]],
                [b[0], b[1], a[2]],
                [b[0], a[1], a[2]],
            ],
        );
        quad(
            &mut t,
            [
                [a[0], a[1], a[2]],
                [b[0], a[1], a[2]],
                [b[0], a[1], b[2]],
                [a[0], a[1], b[2]],
            ],
        );
        quad(
            &mut t,
            [
                [b[0], b[1], a[2]],
                [a[0], b[1], a[2]],
                [a[0], b[1], b[2]],
                [b[0], b[1], b[2]],
            ],
        );
        quad(
            &mut t,
            [
                [a[0], b[1], a[2]],
                [a[0], a[1], a[2]],
                [a[0], a[1], b[2]],
                [a[0], b[1], b[2]],
            ],
        );
        quad(
            &mut t,
            [
                [b[0], a[1], a[2]],
                [b[0], b[1], a[2]],
                [b[0], b[1], b[2]],
                [b[0], a[1], b[2]],
            ],
        );
        mesh::normalize(t).unwrap()
    }
    #[test]
    fn faces_apart_are_finished_one_after_the_other_in_the_order_picked() {
        let mesh = two_half_balls();
        // The surfaces of each dome: those above the plate, by side.
        let dome = |right: bool| -> Vec<usize> {
            let s = mesh.display_scale();
            let mut ids: Vec<usize> = (mesh.faces.iter())
                .filter(|f| f.points.iter().all(|p| p[2] > 4. * s + 4. + 0.01))
                .filter(|f| (f.points[0][0] > 0.) == right)
                .filter_map(|f| f.feature)
                .collect();
            ids.sort();
            ids.dedup();
            ids
        };
        let (left, right) = (dome(false), dome(true));
        // The right dome picked first.
        let ids: Vec<usize> = right.iter().chain(&left).copied().collect();
        let groups = surface::connected(&mesh, &ids);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].len(), right.len());
        for (strategy, tool_type) in [
            (Strategy::Parallel, "Ball end mill"),
            (Strategy::Waterline, "Flat end mill"),
        ] {
            let (setup, mut op, mut tool) = fixture(strategy);
            tool.p.insert("type".into(), tool_type.into());
            op.p.insert("stepdown".into(), "1".into());
            op.p.insert("boundary".into(), "Selected geometry".into());
            op.surfaces = Some(surface::Selection {
                mesh_fingerprint: mesh.fingerprint.clone(),
                ids: ids.clone(),
            });
            let plan = generate(&setup, &op, &tool, &mesh).unwrap();
            let g = Geometry::new(&setup, &mesh);
            let middle = (g.bounds.min[0] + g.bounds.max[0]) / 2.;
            // Which dome each cut is on, in order: all of the right one,
            // then all of the left one.
            let mut sides: Vec<bool> = (plan.moves.iter())
                .filter(|m| m.cutting_pass)
                .map(|m| m.to[0] > middle)
                .collect();
            assert!(sides[0], "{strategy:?} starts on the left dome");
            sides.dedup();
            assert_eq!(sides, vec![true, false], "{strategy:?}");
            assert_plan(plan);
        }
    }
    #[test]
    fn waterline_finishes_only_the_picked_curved_face() {
        let mesh = half_ball_on_plate();
        let (setup, mut op, tool) = fixture(Strategy::Waterline);
        op.p.insert("stepdown".into(), "0.5".into());
        let g = Geometry::new(&setup, &mesh);
        let centre = [
            (g.bounds.min[0] + g.bounds.max[0]) / 2.,
            (g.bounds.min[1] + g.bounds.max[1]) / 2.,
            g.bounds.min[2] + 4.,
        ];
        let off_axis = |p: &[f32; 3]| (p[0] - centre[0]).hypot(p[1] - centre[1]);
        let feeds = |plan: &Plan| -> Vec<[f32; 3]> {
            (plan.moves.iter())
                .filter(|m| m.cutting_pass)
                .flat_map(|m| [m.from, m.to])
                .collect()
        };
        // The whole part: round the plate too.
        let whole = generate(&setup, &op, &tool, &mesh).unwrap();
        assert!(feeds(&whole).iter().any(|p| off_axis(p) > 20.));
        let top = (mesh.faces.iter())
            .filter(|f| f.feature.is_some())
            .max_by(|a, b| {
                let z = |f: &mesh::Face| f.points.iter().map(|p| p[2]).sum::<f32>();
                z(a).total_cmp(&z(b))
            })
            .and_then(|f| f.feature)
            .unwrap();
        op.p.insert("boundary".into(), "Selected geometry".into());
        assert!(op.needs_surfaces());
        assert!(generate(&setup, &op, &tool, &mesh).is_err());
        op.surfaces = Some(surface::Selection {
            mesh_fingerprint: mesh.fingerprint.clone(),
            ids: mesh.region_of(top),
        });
        let plan = generate(&setup, &op, &tool, &mesh).unwrap();
        let points = feeds(&plan);
        let r = plan.radius;
        // Every loop goes round the dome at its own height, the cutter's side
        // on the dome, and none goes round the plate or below its top.
        for p in &points {
            let h = p[2] - centre[2];
            assert!(h > -0.001, "{p:?}");
            let ring = (100. - h.min(10.).powi(2)).max(0.).sqrt();
            assert!(
                (off_axis(p) - ring - r).abs() < 0.1,
                "{p:?}: {} from the axis, the dome {ring} there, centre {centre:?}",
                off_axis(p)
            );
        }
        // From the foot up: a level every 0.5 mm over the dome's 10.
        let levels: std::collections::BTreeSet<i32> = points
            .iter()
            .map(|p| (p[2] * 1000.).round() as i32)
            .collect();
        assert!(levels.len() >= 19, "{}", levels.len());
        assert!(points.iter().any(|p| p[2] - centre[2] < 0.6));
        assert_plan(plan);
    }
    #[test]
    fn waterline_follows_only_a_picked_wall_up_to_its_corners() {
        let mesh = block();
        let (setup, mut op, tool) = fixture(Strategy::Waterline);
        let g = Geometry::new(&setup, &mesh);
        // The wall facing -Y.
        let wall = (0..mesh.surfaces.len())
            .find(|&i| mesh.surfaces[i].normal[1] < -0.99)
            .unwrap();
        op.p.insert("boundary".into(), "Selected geometry".into());
        op.surfaces = Some(surface::Selection {
            mesh_fingerprint: mesh.fingerprint.clone(),
            ids: mesh.region_of(wall),
        });
        let plan = generate(&setup, &op, &tool, &mesh).unwrap();
        let r = plan.radius;
        let feeds: Vec<[f32; 3]> = (plan.moves.iter())
            .filter(|m| m.cutting_pass)
            .flat_map(|m| [m.from, m.to])
            .collect();
        // Along that wall, round its two corners, and at most one short step
        // on along the next walls: never along the far wall.
        assert!(
            feeds.iter().all(|p| p[1] < g.bounds.min[1] + 0.6),
            "{feeds:?}"
        );
        assert!(feeds.iter().any(|p| p[0] < g.bounds.min[0] - r + 0.1));
        assert!(feeds.iter().any(|p| p[0] > g.bounds.max[0] + r - 0.1));
        assert!(
            feeds
                .iter()
                .any(|p| (p[1] - (g.bounds.min[1] - r)).abs() < 0.05)
        );
        assert_plan(plan);
    }
    #[test]
    fn a_step_half_ball_s_dome_is_one_cad_face_and_is_finished_alone() {
        if crate::step::skip_without_helper() {
            return;
        }
        let examples = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");
        // The reader numbers each triangle's CAD face: the dome's and the
        // bottom's.
        let (_, cad) = crate::step::load(&examples.join("half-ball.step")).unwrap();
        assert_eq!(
            cad.unwrap()
                .into_iter()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            2
        );
        // On its own, a half ball is its dome and its flat bottom.
        let ball = mesh::load(&examples.join("half-ball.step")).unwrap();
        assert_eq!(
            ball.regions
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            2
        );
        // On a 70 x 70 x 10 plate: the dome and the plate's six faces.
        let mesh = mesh::load(&examples.join("half-ball-on-plate.step")).unwrap();
        assert_eq!(
            mesh.regions
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            7
        );
        let s = mesh.display_scale();
        let plate_top = 10. * s + 4.;
        let above: Vec<usize> = (mesh.faces.iter())
            .filter(|f| f.points.iter().any(|p| p[2] > plate_top + 0.01))
            .filter_map(|f| f.feature)
            .collect();
        let dome = mesh.region_of(above[0]);
        assert!(above.iter().all(|id| dome.contains(id)));
        let (mut setup, mut op, tool) = fixture(Strategy::Parallel);
        op.p.insert("boundary".into(), "Selected geometry".into());
        op.surfaces = Some(surface::Selection {
            mesh_fingerprint: mesh.fingerprint.clone(),
            ids: dome,
        });
        setup.operations = vec![op.clone()];
        assert!(surface::issues(&op, &setup, Some(&mesh)).is_empty());
        let plan = generate(&setup, &op, &tool, &mesh).unwrap();
        let g = Geometry::new(&setup, &mesh);
        let centre = [
            (g.bounds.min[0] + g.bounds.max[0]) / 2.,
            (g.bounds.min[1] + g.bounds.max[1]) / 2.,
        ];
        let feeds: Vec<[f32; 3]> = (plan.moves.iter())
            .filter(|m| m.motion == Motion::Feed)
            .map(|m| m.to)
            .collect();
        let reach = (feeds.iter())
            .map(|p| (p[0] - centre[0]).hypot(p[1] - centre[1]))
            .fold(0., f32::max);
        // Down into the corner at the dome's foot, where the ball touches
        // the plate, and no more than two sampling cells past it.
        let r = plan.radius;
        let corner = ((25. + r).powi(2) - r * r).sqrt();
        assert!(
            (corner..=corner + 2. * std::f32::consts::SQRT_2).contains(&reach),
            "{reach}"
        );
        let plate = g.bounds.min[2] + 10.;
        let lowest = feeds.iter().map(|p| p[2]).fold(f32::INFINITY, f32::min);
        assert!((plate - 0.01..plate + 0.1).contains(&lowest), "{lowest}");
        assert_plan(plan);
    }
    fn hole_part() -> Mesh {
        cup(false)
    }
    /// A Ø8 bore 8 mm deep in a Ø24 x 10 mm disc: annulus, cylindrical wall
    /// and floor, 64 sides. The floor is the lowest point of the open cup
    /// (`base` false), so the bore goes through; on a base the bore is blind,
    /// with 2 mm of part under its floor.
    fn cup(base: bool) -> Mesh {
        let section: &[(f32, f32)] = if base {
            &[
                (0., 0.),
                (12., 0.),
                (12., 10.),
                (4., 10.),
                (4., 2.),
                (0., 2.),
            ]
        } else {
            &[(12., 10.), (4., 10.), (4., 2.), (0., 2.)]
        };
        mesh::normalize(revolved(section)).unwrap()
    }
    /// `section` turned about Z, 64 sides: (radius, height) points along the
    /// part's section, each pair a ring of quads, a point on the axis at
    /// either end a fan to it.
    pub(crate) fn revolved(section: &[(f32, f32)]) -> Vec<[[f32; 3]; 3]> {
        let mut t = vec![];
        for i in 0..64 {
            let a = i as f32 * std::f32::consts::TAU / 64.;
            let b = (i + 1) as f32 * std::f32::consts::TAU / 64.;
            let p = |r: f32, z: f32, a: f32| [r * a.cos(), r * a.sin(), z];
            for pair in section.windows(2) {
                let ((r0, z0), (r1, z1)) = (pair[0], pair[1]);
                if r0 == 0. {
                    t.push([[0., 0., z0], p(r1, z1, b), p(r1, z1, a)]);
                } else if r1 == 0. {
                    t.push([[0., 0., z1], p(r0, z0, a), p(r0, z0, b)]);
                } else {
                    quad(
                        &mut t,
                        [p(r0, z0, a), p(r0, z0, b), p(r1, z1, b), p(r1, z1, a)],
                    );
                }
            }
        }
        t
    }
    #[test]
    fn a_hole_open_below_is_as_deep_as_its_full_diameter_goes() {
        // A Ø24 x 10 disc with a Ø8 hole up from its bottom: full diameter
        // for 6 mm, then a 118° drill point up to its tip 2.4 mm higher.
        let tip = 6. + 4. * 31f32.to_radians().tan();
        let section = [
            (0., 10.),
            (12., 10.),
            (12., 0.),
            (4., 0.),
            (4., 6.),
            (0., tip),
        ];
        let mesh = mesh::normalize(revolved(&section)).unwrap();
        let setup = Setup::new(Machine::new(3), "G54");
        let holes = bottom_holes(&setup, &mesh).unwrap();
        assert_eq!(holes.len(), 1, "{holes:?}");
        let h = &holes[0];
        assert!(!h.through && (h.diameter - 8.).abs() < 0.05, "{h:?}");
        // Read where a pin's edge would meet the point, not at its tip.
        assert!(h.depth > 6. && h.depth < 6.2, "{h:?}");
    }
    fn select_top(op: &mut Operation, mesh: &Mesh) {
        let top = mesh
            .surfaces
            .iter()
            .filter(|s| s.normal[2].abs() > 0.999999)
            .map(|s| s.center[2])
            .fold(f32::NEG_INFINITY, f32::max);
        let ids = mesh
            .surfaces
            .iter()
            .enumerate()
            .filter(|(_, s)| s.normal[2].abs() > 0.999999 && (s.center[2] - top).abs() < 0.001)
            .map(|(id, _)| id)
            .collect();
        op.surfaces = Some(surface::Selection {
            mesh_fingerprint: mesh.fingerprint.clone(),
            ids,
        });
    }
    #[test]
    fn drilling_detects_actual_hole_and_exercises_all_cycles() {
        let mesh = hole_part();
        let (setup, mut op, tool) = fixture(Strategy::Drill);
        select_top(&mut op, &mesh);
        let found = holes(&setup, &op, &mesh).unwrap();
        assert_eq!(found.len(), 1);
        assert!((found[0].diameter - 8.).abs() < 0.02);
        assert!((found[0].top - found[0].bottom - 8.).abs() < 0.001);
        let straight = generate(&setup, &op, &tool, &mesh).unwrap();
        for cycle in [
            "Drilling",
            "Chip breaking",
            "Deep drilling",
            "Counterboring",
        ] {
            op.p.insert("cycle".into(), cycle.into());
            let plan = generate(&setup, &op, &tool, &mesh).unwrap();
            if cycle == "Counterboring" {
                assert!(
                    plan.moves.iter().any(
                        |m| m.motion == Motion::Dwell && (m.end - m.start - 0.5).abs() < 0.0001
                    )
                );
            } else if cycle != "Drilling" {
                assert!(plan.moves.len() > straight.moves.len());
            }
            assert_plan(plan);
        }
        op.p.insert("through".into(), "Yes".into());
        let extended = generate(&setup, &op, &tool, &mesh).unwrap();
        assert!((extended.target - straight.target + 1.).abs() < 0.001);
        op.p.insert("hole_diameter".into(), "5".into());
        assert!(
            generate(&setup, &op, &tool, &mesh)
                .unwrap_err()
                .contains("filter")
        );
    }
    #[test]
    fn a_chamfer_mill_countersinks_each_opening_to_its_diameter_at_the_rim() {
        let mesh = hole_part();
        let (setup, mut op, mut tool) = fixture(Strategy::Drill);
        select_top(&mut op, &mesh);
        let found = holes(&setup, &op, &mesh).unwrap();
        let rim = found[0].top;
        tool.p.insert("type".into(), "Chamfer mill".into());
        tool.p.insert("diameter".into(), "12".into());
        for (angle, rise) in [("90", 1.), ("82", 1. / 41f32.to_radians().tan())] {
            tool.p.insert("tip_angle".into(), angle.into());
            let plan = generate(&setup, &op, &tool, &mesh).unwrap();
            // The cone is as wide as the Ø8 opening at the rim.
            assert!(
                (rim - plan.target - found[0].opening / 2. * rise).abs() < 0.001,
                "{angle}"
            );
            assert_eq!(
                plan.shape,
                CutterShape::Chamfer(angle.parse::<u16>().unwrap() * 10)
            );
            assert_plan(plan);
        }
        // Bottom offset raises it for a narrower chamfer; Through adds nothing.
        op.p.insert("bottom".into(), "1".into());
        op.p.insert("through".into(), "Yes".into());
        let raised = generate(&setup, &op, &tool, &mesh).unwrap();
        assert!((rim - raised.target - (found[0].opening / 2. * 1.150_368 - 1.)).abs() < 0.001);
        // Narrower than the opening it cannot make it; no angle, no cutter.
        tool.p.insert("diameter".into(), "6".into());
        assert!(
            generate(&setup, &op, &tool, &mesh)
                .unwrap_err()
                .contains("narrower")
        );
        tool.p.insert("diameter".into(), "12".into());
        tool.p.insert("tip_angle".into(), "5".into());
        assert!(!tool.issues().is_empty());
        assert!(
            generate(&setup, &op, &tool, &mesh)
                .unwrap_err()
                .contains("tip angle")
        );
    }
    #[test]
    fn a_blind_hole_is_not_drilled_past_its_floor() {
        let mesh = cup(true);
        let (setup, mut op, tool) = fixture(Strategy::Drill);
        select_top(&mut op, &mesh);
        let found = holes(&setup, &op, &mesh).unwrap();
        assert_eq!(found.len(), 1);
        assert!(!found[0].through && (found[0].top - found[0].bottom - 8.).abs() < 0.001);
        // A new operation starts with Through = No: the drill stops at the floor.
        assert_eq!(get(&op.p, "through"), "No");
        let plan = generate(&setup, &op, &tool, &mesh).unwrap();
        assert!((plan.target - found[0].bottom).abs() < 0.001);
        assert!(
            plan.moves
                .iter()
                .all(|m| m.to[2] >= found[0].bottom - 0.001)
        );
        assert_plan(plan);
        // Through = Yes with the 1 mm of breakthrough it starts with: the blind
        // hole is refused by name, not drilled 1 mm into the part under its floor.
        op.p.insert("through".into(), "Yes".into());
        let refused = format!(
            "The Ø{:.2} hole at X{:.2} Y{:.2} is blind: the part goes on under its bottom, so it cannot be drilled through. Set Through to No, or use the diameter filter to drill only the holes that go through.",
            found[0].diameter, found[0].center[0], found[0].center[1]
        );
        assert_eq!(generate(&setup, &op, &tool, &mesh).unwrap_err(), refused);
        // Whatever the breakthrough.
        op.p.insert("breakthrough".into(), "0".into());
        assert_eq!(generate(&setup, &op, &tool, &mesh).unwrap_err(), refused);
        // The open cup's floor is its lowest point, so breakthrough applies there.
        op.p.insert("breakthrough".into(), "1".into());
        let open = hole_part();
        select_top(&mut op, &open);
        let floor = holes(&setup, &op, &open).unwrap()[0].clone();
        assert!(floor.through);
        let through = generate(&setup, &op, &tool, &open).unwrap();
        assert!((through.target - (floor.bottom - 1.)).abs() < 0.001);
    }
    /// A Helical milling operation on the top face of `mesh`, with a flat
    /// end mill of `diameter`.
    fn helical_op(mesh: &Mesh, diameter: &str) -> (Setup, Operation, Tool) {
        let (setup, mut op, mut tool) = fixture(Strategy::Drill);
        tool.p.insert("type".into(), "Flat end mill".into());
        tool.p.insert("diameter".into(), diameter.into());
        op.p.insert("cycle".into(), crate::model::HELICAL_MILLING.into());
        select_top(&mut op, mesh);
        (setup, op, tool)
    }
    /// How far round `center` the cutting moves of `plan` turn, radians:
    /// positive counterclockwise.
    fn turning(plan: &Plan, center: [f32; 2]) -> f32 {
        let angle = |p: [f32; 3]| (p[1] - center[1]).atan2(p[0] - center[0]);
        (plan.moves.iter())
            .filter(|m| m.cutting_pass && m.from[..2] != m.to[..2])
            .map(|m| {
                let d = angle(m.to) - angle(m.from);
                (d + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI
            })
            .sum()
    }
    #[test]
    fn helical_milling_mills_a_blind_hole_to_size_down_to_its_flat_floor() {
        // The Ø8 bore 8 mm deep, flat-floored, with a Ø4 end mill.
        let mesh = cup(true);
        let (mut setup, mut op, mut tool) = helical_op(&mesh, "4");
        let hole = holes(&setup, &op, &mesh).unwrap()[0].clone();
        assert!(!hole.through);
        let plan = generate(&setup, &op, &tool, &mesh).unwrap();
        assert_eq!(plan.shape, CutterShape::Flat);
        assert_eq!(plan.milling_direction, "Climb · helical");
        assert!((plan.target - hole.bottom).abs() < 0.001, "{}", plan.target);
        // Its centre goes round 2 mm from the hole's, its edge on the wall,
        // only ever down, to the floor and no deeper.
        let cuts: Vec<&Move> = plan.moves.iter().filter(|m| m.cutting_pass).collect();
        for m in &cuts {
            let d = (m.to[0] - hole.center[0]).hypot(m.to[1] - hole.center[1]);
            assert!((d - 2.).abs() < 0.001, "{m:?}");
            assert!(m.to[2] <= m.from[2] + 0.0001 && m.to[2] >= hole.bottom - 0.001);
        }
        // From the stock's top, 2 mm over the part, 10 mm down at 2 per
        // turn: five turns, then once round on the floor.
        assert_eq!(plan.levels, 5);
        let on_floor = cuts
            .iter()
            .filter(|m| {
                (m.from[2] - hole.bottom).abs() < 0.001 && (m.to[2] - hole.bottom).abs() < 0.001
            })
            .count();
        assert_eq!(cuts.len(), 6 * on_floor);
        let round = std::f32::consts::TAU;
        // Climb: counterclockwise with the spindle clockwise, and back.
        assert!((turning(&plan, hole.center) - 6. * round).abs() < 0.01);
        op.p.insert("spindle_rotation".into(), "Counterclockwise (CCW)".into());
        let reversed = generate(&setup, &op, &tool, &mesh).unwrap();
        assert!((turning(&reversed, hole.center) + 6. * round).abs() < 0.01);
        assert_plan(plan);
        // A cutter the hole's size goes straight down; a wider one cannot.
        tool.p.insert("diameter".into(), "8".into());
        let plunge = generate(&setup, &op, &tool, &mesh).unwrap();
        assert!(
            plunge
                .moves
                .iter()
                .filter(|m| m.cutting_pass)
                .all(|m| { (m.to[0] - hole.center[0]).hypot(m.to[1] - hole.center[1]) < 0.001 })
        );
        tool.p.insert("diameter".into(), "9".into());
        assert_eq!(
            generate(&setup, &op, &tool, &mesh).unwrap_err(),
            format!(
                "The Ø9.00 end mill is wider than the Ø8.00 hole at X{:.2} Y{:.2}. Choose a smaller end mill or a diameter filter.",
                hole.center[0], hole.center[1]
            )
        );
        // A drill does not mill, and a manual mill moves one axis at a time.
        tool.p.insert("diameter".into(), "4".into());
        tool.p.insert("type".into(), "Drill".into());
        assert_eq!(
            generate(&setup, &op, &tool, &mesh).unwrap_err(),
            "Choose a flat end mill for Helical milling."
        );
        tool.p.insert("type".into(), "Flat end mill".into());
        setup.machine = crate::manual::defaults();
        assert_eq!(
            generate(&setup, &op, &tool, &mesh).unwrap_err(),
            crate::manual::HELICAL_ON_MANUAL
        );
    }
    #[test]
    fn helical_milling_cuts_a_wide_hole_in_rings_from_the_centre_out() {
        // The Ø8 bore with a Ø1.5 end mill: its centre travels out to 3.25 mm,
        // over twice its radius, so it clears the middle first.
        let mesh = cup(true);
        let (setup, op, tool) = helical_op(&mesh, "1.5");
        let hole = holes(&setup, &op, &mesh).unwrap()[0].clone();
        let plan = generate(&setup, &op, &tool, &mesh).unwrap();
        let mut rings: Vec<f32> = vec![];
        for m in plan.moves.iter().filter(|m| m.cutting_pass) {
            let d = (m.to[0] - hole.center[0]).hypot(m.to[1] - hole.center[1]);
            if rings.last().is_none_or(|r| (r - d).abs() > 0.001) {
                rings.push(d);
            }
        }
        // Innermost first, each at most 40 % of the cutter's diameter wider
        // than the one inside it, the first sweeping the centre and the last
        // putting the cutter's edge on the wall.
        assert!(rings.len() > 2, "{rings:?}");
        assert!(rings[0] <= 0.75);
        assert!((rings.last().unwrap() - 3.25).abs() < 0.001, "{rings:?}");
        assert!(
            rings
                .windows(2)
                .all(|w| w[1] > w[0] && w[1] - w[0] <= 0.6 + 0.001),
            "{rings:?}"
        );
        // Each ring goes all the way down to the floor.
        assert!(rings.iter().all(|r| plan.moves.iter().any(|m| {
            m.cutting_pass
                && (m.to[2] - hole.bottom).abs() < 0.001
                && ((m.to[0] - hole.center[0]).hypot(m.to[1] - hole.center[1]) - r).abs() < 0.001
        })));
        assert_plan(plan);
    }
    #[test]
    fn helical_milling_cuts_a_coarse_walled_hole_to_the_size_it_lists() {
        // The TOTEM tenting bottom's holes: a rim of 64 sides over a wall of
        // 16, whose corners are not on one circle (3.98 and 4.02 mm from the
        // centre in turn). Read at the chords' middles the bore was Ø7.85
        // and milled 0.15 mm small; read at the nearest corners, it lists
        // and mills Ø7.96, cutting past no corner of the wall.
        let mut t = vec![];
        let at = |r: f32, a: f32, z: f32| [r * a.cos(), r * a.sin(), z];
        let step = |n: usize, i: usize| i as f32 * std::f32::consts::TAU / n as f32;
        for i in 0..64 {
            let (a, b) = (step(64, i), step(64, i + 1));
            quad(
                &mut t,
                [
                    at(4., a, 10.),
                    at(4., b, 10.),
                    at(12., b, 10.),
                    at(12., a, 10.),
                ],
            );
            quad(
                &mut t,
                [
                    at(12., a, 10.),
                    at(12., b, 10.),
                    at(12., b, 0.),
                    at(12., a, 0.),
                ],
            );
            t.push([[0., 0., 0.], at(12., b, 0.), at(12., a, 0.)]);
        }
        let corner = |i: usize| if i.is_multiple_of(2) { 3.98 } else { 4.02 };
        for i in 0..16 {
            let (a, b) = (step(16, i), step(16, i + 1));
            let (ra, rb) = (corner(i), corner(i + 1));
            quad(
                &mut t,
                [at(ra, a, 2.), at(rb, b, 2.), at(rb, b, 10.), at(ra, a, 10.)],
            );
            t.push([[0., 0., 2.], at(ra, a, 2.), at(rb, b, 2.)]);
        }
        let mesh = mesh::normalize(t).unwrap();
        let (setup, op, mut tool) = helical_op(&mesh, "4");
        let hole = holes(&setup, &op, &mesh).unwrap()[0].clone();
        assert!((hole.diameter - 7.96).abs() < 0.002, "{hole:?}");
        for d in ["4", "2"] {
            tool.p.insert("diameter".into(), d.into());
            let plan = generate(&setup, &op, &tool, &mesh).unwrap();
            let cut = (plan.moves.iter())
                .filter(|m| m.cutting_pass)
                .map(|m| (m.to[0] - hole.center[0]).hypot(m.to[1] - hole.center[1]))
                .fold(0f32, f32::max)
                + plan.radius;
            assert!((2. * cut - hole.diameter).abs() < 0.001, "Ø{d}: {cut}");
        }
        // The refusal names the size listed.
        tool.p.insert("diameter".into(), "8.2".into());
        assert!(
            generate(&setup, &op, &tool, &mesh)
                .unwrap_err()
                .contains("wider than the Ø7.96 hole")
        );
    }
    #[test]
    fn a_shallow_d_flat_in_a_bore_is_not_read_as_round() {
        // A Ø8 bore of 64 sides, 8 mm deep and blind, with one flat face
        // across it at X = `d`: a flat 0.1–0.3 mm deep. Its ends lie on the
        // round, as a coarse side's do, but it spans many sides: the hole is
        // read at the flat, refuses a drill of the round's size, and is
        // milled clear of it.
        for d in [3.7f32, 3.8, 3.9] {
            let step = |i: usize| i as f32 * std::f32::consts::TAU / 64.;
            let round = |i: usize| [4. * step(i).cos(), 4. * step(i).sin()];
            let mut inner: Vec<[f32; 2]> = vec![];
            for i in 0..64 {
                let (a, b) = (round(i), round(i + 1));
                if a[0] <= d {
                    inner.push(a);
                }
                if (a[0] <= d) != (b[0] <= d) {
                    let t = (d - a[0]) / (b[0] - a[0]);
                    inner.push([d, a[1] + t * (b[1] - a[1])]);
                }
            }
            let mut t = vec![];
            let out = |p: [f32; 2]| {
                let l = p[0].hypot(p[1]);
                [12. * p[0] / l, 12. * p[1] / l]
            };
            for i in 0..inner.len() {
                let (p, q) = (inner[i], inner[(i + 1) % inner.len()]);
                let (pp, qq) = (out(p), out(q));
                quad(
                    &mut t,
                    [
                        [p[0], p[1], 10.],
                        [q[0], q[1], 10.],
                        [qq[0], qq[1], 10.],
                        [pp[0], pp[1], 10.],
                    ],
                );
                quad(
                    &mut t,
                    [
                        [p[0], p[1], 2.],
                        [q[0], q[1], 2.],
                        [q[0], q[1], 10.],
                        [p[0], p[1], 10.],
                    ],
                );
                t.push([[0., 0., 2.], [p[0], p[1], 2.], [q[0], q[1], 2.]]);
                quad(
                    &mut t,
                    [
                        [pp[0], pp[1], 10.],
                        [qq[0], qq[1], 10.],
                        [qq[0], qq[1], 0.],
                        [pp[0], pp[1], 0.],
                    ],
                );
                t.push([[0., 0., 0.], [qq[0], qq[1], 0.], [pp[0], pp[1], 0.]]);
            }
            let mesh = mesh::normalize(t).unwrap();
            // Drilling: listed at the flat, a Ø8 drill refused.
            let (setup, mut op, mut tool) = fixture(Strategy::Drill);
            select_top(&mut op, &mesh);
            let hole = holes(&setup, &op, &mesh).unwrap()[0].clone();
            assert!((hole.diameter - 2. * d).abs() < 0.01, "{d}: {hole:?}");
            tool.p.insert("diameter".into(), "8".into());
            assert!(
                generate(&setup, &op, &tool, &mesh)
                    .unwrap_err()
                    .contains("exceeds"),
                "{d}"
            );
            // Helical milling with a Ø4 mill: listed at the flat, its edge
            // never past it.
            let (setup, op, tool) = helical_op(&mesh, "4");
            let hole = holes(&setup, &op, &mesh).unwrap()[0].clone();
            assert!((hole.diameter - 2. * d).abs() < 0.01, "{d}: {hole:?}");
            let plan = generate(&setup, &op, &tool, &mesh).unwrap();
            let flat = hole.center[0] + d;
            let reach = (plan.moves.iter())
                .filter(|m| m.cutting_pass)
                .map(|m| m.to[0] + plan.radius)
                .fold(f32::NEG_INFINITY, f32::max);
            assert!(reach <= flat + 0.001, "{d}: {reach} past {flat}");
        }
    }
    #[test]
    fn helical_milling_goes_through_a_through_hole_and_stops_on_a_pointed_floors_rim() {
        // Through the open cup's bore, Breakthrough past its exit.
        let open = hole_part();
        let (setup, mut op, tool) = helical_op(&open, "4");
        let hole = holes(&setup, &op, &open).unwrap()[0].clone();
        assert!(hole.through);
        op.p.insert("through".into(), "Yes".into());
        let plan = generate(&setup, &op, &tool, &open).unwrap();
        assert!((plan.target - (hole.bottom - 1.)).abs() < 0.001);
        // A blind hole cannot be milled through.
        let blind = cup(true);
        select_top(&mut op, &blind);
        assert!(
            generate(&setup, &op, &tool, &blind)
                .unwrap_err()
                .contains("is blind")
        );
        // A Ø8 bore whose floor is the cone a 118° drill point leaves: the
        // flat end stops where the cone meets the wall, not at its apex,
        // which would cut the cone away.
        let rise = 4. * 31f32.to_radians().tan();
        let pointed = mesh::normalize(revolved(&[
            (0., -4.),
            (12., -4.),
            (12., 10.),
            (4., 10.),
            (4., 2.),
            (0., 2. - rise),
        ]))
        .unwrap();
        op.p.insert("through".into(), "No".into());
        select_top(&mut op, &pointed);
        // Listed, as it mills it: 8 mm under the rim, where the bore's wall
        // ends, the cone's apex `rise` under that, where a drill goes.
        let hole = holes(&setup, &op, &pointed).unwrap()[0].clone();
        assert!((hole.top - hole.bottom - 8.).abs() < 0.01, "{hole:?}");
        let mut drilling = op.clone();
        drilling.p.insert("cycle".into(), "Drilling".into());
        let apex = holes(&setup, &drilling, &pointed).unwrap()[0].bottom;
        assert!((hole.bottom - apex - rise).abs() < 0.01, "{apex}");
        let plan = generate(&setup, &op, &tool, &pointed).unwrap();
        assert!((plan.target - hole.bottom).abs() < 0.001, "{}", plan.target);
    }
    #[test]
    fn helical_milling_mills_the_totem_cases_top_holes_without_touching_the_part() {
        if crate::step::skip_without_helper() {
            return;
        }
        let mesh = mesh::load(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("examples/totem-case-top-right.step"),
        )
        .unwrap();
        // Its top face's holes, Ø2 through the screw bosses, 14.5 mm deep
        // from the stock's top, with a Ø1.5 flat end mill of 16 mm of flute,
        // 0.5 mm a turn.
        let (setup, mut op, mut tool) = helical_op(&mesh, "1.5");
        tool.p.insert("length".into(), "16".into());
        op.p.insert("stepdown".into(), "0.5".into());
        let found = holes(&setup, &op, &mesh).unwrap();
        assert!(!found.is_empty());
        let plan = generate(&setup, &op, &tool, &mesh).unwrap();
        let g = Geometry::new(&setup, &mesh);
        // Every hole is milled down to its floor.
        for h in &found {
            let floor = h.bottom;
            let lowest = (plan.moves.iter())
                .filter(|m| {
                    m.cutting_pass
                        && (m.to[0] - h.center[0]).hypot(m.to[1] - h.center[1]) < h.diameter
                })
                .map(|m| m.to[2])
                .fold(f32::INFINITY, f32::min);
            // Through is No: to a through hole's exit, no further.
            assert!((lowest - floor).abs() < 0.001, "{h:?}: {lowest}");
        }
        // No cutting position brings the cutter's side into the part over
        // its tip, nor its tip into the part under it: against every face,
        // clipped to what stands 0.01 mm over the tip, the cutter's axis
        // stays its radius away, less the bore's chords' sagitta.
        let r = 0.75 - 0.01;
        for m in plan.moves.iter().filter(|m| m.cutting_pass) {
            let p = m.to;
            for t in g.triangles.iter().filter(|t| {
                let span = |k: usize| {
                    let (lo, hi) = (t.p.iter().map(|q| q[k]))
                        .fold((f32::INFINITY, f32::NEG_INFINITY), |(a, b), v| {
                            (a.min(v), b.max(v))
                        });
                    lo <= p[k] + 1. && hi >= p[k] - 1.
                };
                t.p.iter().any(|q| q[2] > p[2] + 0.01) && span(0) && span(1)
            }) {
                let part = crate::pocket::above(&t.p, p[2] + 0.01);
                if part.len() < 2 {
                    continue;
                }
                let xy = (p[0] as f64, p[1] as f64);
                let inside = contains(std::slice::from_ref(&part), xy);
                let near = (0..part.len())
                    .map(|i| {
                        let (a, b) = (part[i], part[(i + 1) % part.len()]);
                        let v = (b.0 - a.0, b.1 - a.1);
                        let l2 = v.0 * v.0 + v.1 * v.1;
                        let u = if l2 < 1e-12 {
                            0.
                        } else {
                            (((xy.0 - a.0) * v.0 + (xy.1 - a.1) * v.1) / l2).clamp(0., 1.)
                        };
                        (xy.0 - a.0 - u * v.0).hypot(xy.1 - a.1 - u * v.1)
                    })
                    .fold(f64::INFINITY, f64::min);
                assert!(!inside && near >= r, "{m:?} meets {:?}", t.p);
            }
        }
    }
    #[test]
    fn a_blind_holes_drill_point_or_fillet_floor_does_not_narrow_its_bore() {
        // Each hole is a Ø8 bore 8 mm deep in a Ø24 disc on a base 4 mm under
        // its floor, so that it is blind; the drill of a hole with a drill
        // point stops with its point at the cone's apex.
        let blind = |hole: &[(f32, f32)]| {
            let mut section = vec![(0., -4.), (12., -4.)];
            section.extend_from_slice(hole);
            revolved(&section)
        };
        // The height of a 118° drill point under a bore of radius `r`.
        let point = |r: f32| r * 31f32.to_radians().tan();
        let mut fillet = vec![(12., 10.), (4., 10.), (4., 2.5)];
        fillet.extend((1..=8).map(|i| {
            let a = i as f32 * std::f32::consts::FRAC_PI_2 / 8.;
            (3.5 + 0.5 * a.cos(), 2.5 - 0.5 * a.sin())
        }));
        fillet.push((0., 2.));
        let mut pin = blind(&[(12., 10.), (4., 10.), (4., 2.), (0., 2.)]);
        for i in 0..16 {
            let a = i as f32 * std::f32::consts::TAU / 16.;
            let b = (i + 1) as f32 * std::f32::consts::TAU / 16.;
            let p = |z: f32, a: f32| [2. + 0.5 * a.cos(), 0.5 * a.sin(), z];
            quad(&mut pin, [p(8., a), p(8., b), p(2., b), p(2., a)]);
            pin.push([[2., 0., 8.], p(8., a), p(8., b)]);
        }
        // Ledges 1 mm tall either side of a 2 mm channel along X across the
        // floor: their walls are under a Ø8 drill's point at their ends and
        // above it in the middle, 1 mm from the centre.
        let mut ledges = blind(&[(12., 10.), (4., 10.), (4., 2.), (0., 2.)]);
        for (y, far) in [(1., 3.), (-1., -3.)] {
            quad(
                &mut ledges,
                [[-3., y, 2.], [3., y, 2.], [3., y, 3.], [-3., y, 3.]],
            );
            quad(
                &mut ledges,
                [[-3., y, 3.], [3., y, 3.], [3., far, 3.], [-3., far, 3.]],
            );
        }
        for (name, triangles, bore, opening, depth) in [
            (
                "a 118° drill-point floor",
                blind(&[(12., 10.), (4., 10.), (4., 2.), (0., 2. - point(4.))]),
                8.,
                8.,
                8. + point(4.),
            ),
            // Sharper points than the 118° the clip assumes: a 90° point's
            // floor rises at 1.0 per mm, a 100° one's at 0.84.
            (
                "a 90° drill-point floor",
                blind(&[(12., 10.), (4., 10.), (4., 2.), (0., 2. - 4.)]),
                8.,
                8.,
                12.,
            ),
            (
                "a 100° drill-point floor",
                blind(&[
                    (12., 10.),
                    (4., 10.),
                    (4., 2.),
                    (0., 2. - 4. * 40f32.to_radians().tan()),
                ]),
                8.,
                8.,
                8. + 4. * 40f32.to_radians().tan(),
            ),
            ("a 0.5 mm bottom fillet", blind(&fillet), 8., 8., 8.),
            (
                "a Ø5 bore 3 mm deep under the Ø8 bore",
                blind(&[
                    (12., 10.),
                    (4., 10.),
                    (4., 5.),
                    (2.5, 5.),
                    (2.5, 2.),
                    (0., 2.),
                ]),
                5.,
                8.,
                8.,
            ),
            ("a Ø1 pin 2 mm off the centre, 6 mm tall", pin, 3., 8., 8.),
            (
                "ledges 1 mm tall either side of a 2 mm channel",
                ledges,
                2.,
                8.,
                8.,
            ),
            (
                "a blind Ø11 counterbore over a drilled Ø6.6 bore",
                blind(&[
                    (12., 10.),
                    (5.5, 10.),
                    (5.5, 7.),
                    (3.3, 7.),
                    (3.3, 2.),
                    (0., 2. - point(3.3)),
                ]),
                6.6,
                11.,
                8. + point(3.3),
            ),
        ] {
            let mesh = mesh::normalize(triangles).unwrap();
            let (setup, mut op, mut tool) = fixture(Strategy::Drill);
            select_top(&mut op, &mesh);
            let found = holes(&setup, &op, &mesh).unwrap_or_else(|e| panic!("{name}: {e}"));
            assert_eq!(found.len(), 1, "{name}");
            let h = &found[0];
            assert!(!h.through, "{name}: {h:?}");
            assert!((h.diameter - bore).abs() < 0.02, "{name}: {h:?}");
            assert!((h.opening - opening).abs() < 0.02, "{name}: {h:?}");
            assert!((h.top - h.bottom - depth).abs() < 0.001, "{name}: {h:?}");
            // A drill just under the bore's size is accepted; 0.5 mm over is not.
            tool.p.insert("diameter".into(), (bore - 0.05).to_string());
            generate(&setup, &op, &tool, &mesh).unwrap_or_else(|e| panic!("{name}: {e}"));
            tool.p.insert("diameter".into(), (bore + 0.5).to_string());
            assert!(
                generate(&setup, &op, &tool, &mesh)
                    .unwrap_err()
                    .contains("exceeds an opening"),
                "{name}"
            );
        }
    }
    /// The model's lowest horizontal face: the top once the setup is turned over.
    fn select_bottom(op: &mut Operation, mesh: &Mesh) {
        let bottom = mesh
            .surfaces
            .iter()
            .filter(|s| s.normal[2] < -0.999999)
            .map(|s| s.center[2])
            .fold(f32::INFINITY, f32::min);
        let ids = mesh
            .surfaces
            .iter()
            .enumerate()
            .filter(|(_, s)| s.normal[2] < -0.999999 && (s.center[2] - bottom).abs() < 0.001)
            .map(|(id, _)| id)
            .collect();
        op.surfaces = Some(surface::Selection {
            mesh_fingerprint: mesh.fingerprint.clone(),
            ids,
        });
    }
    #[test]
    fn a_step_bores_nominal_drill_is_accepted_and_the_bore_reads_its_nominal_size() {
        // The STEP helper tessellates the example's circles in 42 segments, so
        // the chords of a Ø6.6 bore are Ø6.582 across: the bore is read at its
        // vertices, and the drill of its nominal size goes.
        if crate::step::skip_without_helper() {
            return;
        }
        let mesh = mesh::load(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("examples/four-setup-block.step"),
        )
        .unwrap();
        let r2 = |v: f32| (v * 100.).round() / 100.;
        // Turned over, the bottom plane is up and its four Ø6.6 bores open straight.
        let (setup, mut op, mut tool) = fixture(Strategy::Drill);
        let setup = crate::analysis::oriented(&setup, "-Z").unwrap();
        select_bottom(&mut op, &mesh);
        let found = holes(&setup, &op, &mesh).unwrap();
        eprintln!("step, bottom plane: {found:#?}");
        let bores: Vec<_> = found
            .iter()
            .filter(|h| (h.opening - 6.6).abs() < 0.05)
            .collect();
        assert_eq!(bores.len(), 4, "{found:?}");
        for h in &bores {
            assert!(h.through, "{h:?}");
            assert_eq!(r2(h.diameter), 6.6, "{h:?}");
            assert_eq!(r2(h.opening), 6.6, "{h:?}");
            assert!((h.top - h.bottom - 40.).abs() < 0.01, "{h:?}");
        }
        tool.p.insert("diameter".into(), "6.6".into());
        op.p.insert("hole_diameter".into(), "6.6".into());
        let plan = generate(&setup, &op, &tool, &mesh).unwrap();
        assert_eq!(plan.levels, 4);
        assert_plan(plan);
        // A drill 0.5 mm over the bore is still refused.
        tool.p.insert("diameter".into(), "7.1".into());
        assert!(
            generate(&setup, &op, &tool, &mesh)
                .unwrap_err()
                .contains("exceeds an opening")
        );
        // From the top the same bores lie under Ø11 counterbores: the corridor
        // is their chords, read at their vertices just the same.
        let (setup, mut op, mut tool) = fixture(Strategy::Drill);
        select_top(&mut op, &mesh);
        let found = holes(&setup, &op, &mesh).unwrap();
        eprintln!("step, top plane: {found:#?}");
        let counterbored: Vec<_> = found
            .iter()
            .filter(|h| (h.opening - 11.).abs() < 0.05)
            .collect();
        assert_eq!(counterbored.len(), 4, "{found:?}");
        for h in &counterbored {
            assert!(h.through, "{h:?}");
            assert_eq!(r2(h.diameter), 6.6, "{h:?}");
            assert_eq!(r2(h.opening), 11., "{h:?}");
        }
        tool.p.insert("diameter".into(), "6.6".into());
        op.p.insert("hole_diameter".into(), "6.6".into());
        let plan = generate(&setup, &op, &tool, &mesh).unwrap();
        assert_eq!(plan.levels, 4);
        assert_plan(plan);
    }
    /// A cross-hole through both walls of a tube in a plain 3-axis setup, +Y
    /// up: the top hole picked by its walls is drilled on through the bore
    /// and the hole opposite, to the part's bottom, as main drills it (the
    /// own-wall depth is for indexed setups of several angles only, where
    /// the opposite hole gets its own angle; LIO-246, review 3).
    #[test]
    fn a_tubes_cross_hole_picked_by_its_walls_goes_through_both_walls_in_3_axis() {
        let mesh = mesh::load(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/radial-tube.3mf"),
        )
        .unwrap();
        let (setup, mut op, _) = fixture(Strategy::Drill);
        let setup = crate::analysis::oriented(&setup, "+Y").unwrap();
        let top = crate::pocket::part_holes(&setup, &mesh)
            .into_iter()
            .max_by(|a, b| a.top.total_cmp(&b.top))
            .unwrap();
        op.geometry.clear();
        op.surfaces = Some(crate::surface::Selection {
            mesh_fingerprint: mesh.fingerprint.clone(),
            ids: top.walls.clone(),
        });
        let g = Geometry::new(&setup, &mesh);
        let found = holes(&setup, &op, &mesh).unwrap();
        assert_eq!(found.len(), 1, "{found:?}");
        let h = &found[0];
        assert!(h.through, "{h:?}");
        assert!(
            (h.bottom - g.bounds.min[2]).abs() < 0.01,
            "{h:?} {:?}",
            g.bounds
        );
        assert!(h.top - h.bottom > 39.5, "{h:?}");
    }
    #[test]
    fn a_hole_picked_by_its_wall_reads_as_its_rim_face_reads_it_counterbored_or_not() {
        // The example's Ø6.6 bores under Ø11 counterbores, from the top, and
        // straight from the bottom: each hole a wall click picks (by the
        // counterbore's walls or the bore's) reads the same bore, opening,
        // rim and floor as the selected rim face's, and a Drill of those
        // walls drills it.
        if crate::step::skip_without_helper() {
            return;
        }
        let mesh = mesh::load(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("examples/four-setup-block.step"),
        )
        .unwrap();
        let (top_setup, mut top_op, _) = fixture(Strategy::Drill);
        select_top(&mut top_op, &mesh);
        let (bottom_setup, mut bottom_op, _) = fixture(Strategy::Drill);
        let bottom_setup = crate::analysis::oriented(&bottom_setup, "-Z").unwrap();
        select_bottom(&mut bottom_op, &mesh);
        let mut counterbored = 0;
        for (setup, op) in [(&top_setup, &top_op), (&bottom_setup, &bottom_op)] {
            let by_rim = holes(setup, op, &mesh).unwrap();
            let part = crate::pocket::part_holes(setup, &mesh);
            let read = drill_holes_of(setup, &mesh, &part);
            for r in &by_rim {
                let mine: Vec<_> = (part.iter().zip(&read))
                    .filter_map(|(p, h)| h.as_ref().map(|h| (&p.walls, h)))
                    .filter(|(_, h)| same_centre(h, r))
                    .collect();
                assert!(!mine.is_empty(), "{r:?}");
                counterbored += usize::from(mine.len() > 1);
                for (walls, c) in mine {
                    for (a, b) in [
                        (c.diameter, r.diameter),
                        (c.opening, r.opening),
                        (c.top, r.top),
                        (c.bottom, r.bottom),
                    ] {
                        assert!((a - b).abs() < 0.01, "{c:?} {r:?}");
                    }
                    assert_eq!(c.countersink, r.countersink, "{c:?} {r:?}");
                    let mut by_walls = op.clone();
                    by_walls.surfaces.as_mut().unwrap().ids = walls.clone();
                    let drilled = holes(setup, &by_walls, &mesh).unwrap();
                    assert_eq!(drilled.len(), 1);
                    assert!(same_centre(&drilled[0], r), "{drilled:?} {r:?}");
                    assert!((drilled[0].opening - r.opening).abs() < 0.01);
                }
            }
        }
        // The four counterbored holes were picked both ways.
        assert!(counterbored >= 4, "{counterbored}");
    }
    #[test]
    fn new_milling_parameters_change_motion_and_setup_placement_is_respected() {
        let mesh = block();
        for strategy in [Strategy::Contour, Strategy::Parallel, Strategy::Waterline] {
            let (mut setup, op, tool) = fixture(strategy);
            let original = generate(&setup, &op, &tool, &mesh).unwrap();
            let mut changed = op.clone();
            changed.p.insert("feed".into(), "500".into());
            assert!(generate(&setup, &changed, &tool, &mesh).unwrap().duration > original.duration);
            changed = op.clone();
            changed.p.insert("allowance".into(), "0.7".into());
            let allowance = generate(&setup, &changed, &tool, &mesh).unwrap();
            let cuts = |p: &Plan| {
                p.moves
                    .iter()
                    .filter(|m| m.cutting_pass)
                    .map(|m| m.to)
                    .collect::<Vec<_>>()
            };
            assert_ne!(cuts(&original), cuts(&allowance));
            setup.p.insert("tx".into(), "25".into());
            setup.p.insert("ty".into(), "-15".into());
            let moved = generate(&setup, &op, &tool, &mesh).unwrap();
            for (a, b) in cuts(&original).iter().zip(cuts(&moved)) {
                assert!(
                    (b[0] - a[0] - 25.).abs() < 0.003
                        && (b[1] - a[1] + 15.).abs() < 0.003
                        && (b[2] - a[2]).abs() < 0.003,
                    "{strategy:?}: {a:?} -> {b:?}"
                );
            }
        }
    }
    #[test]
    fn contour_segments_clear_corners_at_several_tool_radii() {
        let mesh = block();
        let (setup, op, mut tool) = fixture(Strategy::Contour);
        let g = Geometry::new(&setup, &mesh);
        for diameter in [2., 6., 20., 40.] {
            tool.p.insert("diameter".into(), diameter.to_string());
            let plan = generate(&setup, &op, &tool, &mesh).unwrap();
            for m in plan.moves.iter().filter(|m| m.cutting_pass) {
                for i in 0..=20 {
                    let p = m.at(m.start + (m.end - m.start) * i as f32 / 20.);
                    let dx = (g.bounds.min[0] - p[0]).max(p[0] - g.bounds.max[0]).max(0.);
                    let dy = (g.bounds.min[1] - p[1]).max(p[1] - g.bounds.max[1]).max(0.);
                    assert!(
                        dx * dx + dy * dy >= plan.radius.powi(2) - 0.001,
                        "diameter {diameter}, point {p:?}"
                    );
                }
            }
        }
    }
    #[test]
    fn invalid_tools_geometry_and_bounds_fail_before_simulation() {
        let mesh = block();
        for s in [Strategy::Contour, Strategy::Parallel, Strategy::Waterline] {
            let (setup, op, tool) = fixture(s);
            for (k, v) in [
                ("feed", "NaN"),
                ("feed", "0"),
                ("retract", "0"),
                ("clearance", "-1"),
                ("allowance", "-1"),
            ] {
                let mut bad = op.clone();
                bad.p.insert(k.into(), v.into());
                assert!(generate(&setup, &bad, &tool, &mesh).is_err(), "{s:?} {k}");
            }
            let mut badtool = tool.clone();
            badtool.p.insert("type".into(), "Drill".into());
            assert!(generate(&setup, &op, &badtool, &mesh).is_err());
            let mut suppressed = op.clone();
            suppressed.enabled = false;
            assert!(generate(&setup, &suppressed, &tool, &mesh).is_err());
        }
    }
    #[test]
    fn turning_and_a_lathes_centre_drilling_are_planned_by_the_lathe() {
        let mesh = block();
        let mut setup = Setup::new(crate::lathe::defaults(), "G54");
        let mut tool = Tool::lathe("Turn", "Turning tool");
        tool.p.insert("machines".into(), setup.machine.id.clone());
        let turn = Operation::new(Strategy::Turn, tool.id.clone());
        let mut drill = Operation::for_machine(Strategy::Drill, tool.id.clone(), &setup.machine);
        drill.p.insert("depth".into(), "10".into());
        // Both entries give the lathe's plan, or its refusal, alike.
        for op in [&turn, &drill] {
            let lathe = crate::turn::generate(&setup, op, &tool, &mesh).map(|p| p.moves.len());
            for plan in [
                generate(&setup, op, &tool, &mesh),
                simulation::generate(&setup, op, &tool, &mesh),
            ] {
                assert_eq!(plan.map(|p| p.moves.len()), lathe);
            }
        }
        // The drill needs a drill in the tailstock, not the turning tool.
        let refusal = "A lathe drills on the centre with a drill in the tailstock.";
        assert_eq!(generate(&setup, &drill, &tool, &mesh).unwrap_err(), refusal);
        // A Drill on a mill still selects holes.
        setup.machine = Machine::new(3);
        let plain = Operation::new(Strategy::Drill, tool.id.clone());
        assert_ne!(generate(&setup, &plain, &tool, &mesh).unwrap_err(), refusal);
    }
    #[test]
    fn all_thirteen_strategies_have_an_explicit_executable_or_unavailable_contract() {
        let mesh = block();
        for strategy in Strategy::ALL {
            assert!(!strategy.availability().is_empty());
            if !strategy.imported_preview() {
                let (setup, op, tool) = fixture(strategy);
                assert_eq!(
                    simulation::generate(&setup, &op, &tool, &mesh).unwrap_err(),
                    strategy.availability()
                );
            }
        }
        assert_eq!(
            Strategy::ALL
                .iter()
                .filter(|s| s.imported_preview())
                .count(),
            9
        );
    }
    /// FNV-1a over every bit of every move, and the move count and duration.
    fn fingerprint(plan: &Plan) -> (usize, u32, u64) {
        let mut h = 0xcbf29ce484222325u64;
        for m in &plan.moves {
            let words = m
                .from
                .iter()
                .chain(&m.to)
                .chain([&m.feed, &m.start, &m.end]);
            for w in words
                .map(|v| v.to_bits())
                .chain([m.motion as u32, m.cutting_pass as u32])
            {
                h = (h ^ u64::from(w)).wrapping_mul(0x100000001b3);
            }
        }
        (plan.moves.len(), plan.duration.to_bits(), h)
    }
    #[test]
    fn cnc_drill_and_contour_plans_are_unchanged() {
        let (setup, op, tool) = fixture(Strategy::Contour);
        let contour = generate(&setup, &op, &tool, &block()).unwrap();
        let mesh = hole_part();
        let (setup, mut op, tool) = fixture(Strategy::Drill);
        select_top(&mut op, &mesh);
        op.p.insert("cycle".into(), "Chip breaking".into());
        let drill = generate(&setup, &op, &tool, &mesh).unwrap();
        // Taken before the manual mill's rapid and single-axis moves.
        assert_eq!(
            fingerprint(&contour),
            (355, 1111861812, 9253085170742949517)
        );
        assert_eq!(fingerprint(&drill), (14, 1088593833, 9009359337400031547));
    }
    /// Every move of `plan` changes at most one of X, Y and Z, and a dwell none.
    fn assert_single_axis(plan: &Plan) {
        for m in &plan.moves {
            let moved = (0..3).filter(|&k| m.from[k] != m.to[k]).count();
            assert!(moved <= 1, "{m:?}");
            assert!(m.motion != Motion::Dwell || moved == 0, "{m:?}");
        }
    }
    /// A Box on the manual mill's defaults: a 120 × 80 × 40 block (plan X
    /// −60..60, Y −40..40, Z 0..40, the work zero on the top's centre, so a
    /// work Z is the plan Z less 40) round the `block()` part (X −10..10,
    /// Y −8..8, Z ≈ 0.57..8.57), a Ø25 end mill with 40 mm flutes.
    /// `b` is the box: x_min, x_max, y_min, y_max, z_top, z_bottom.
    fn big_mill(b: [f32; 6]) -> (Setup, Operation, Tool) {
        let mut setup = Setup::new(crate::manual::defaults(), "G54");
        for (k, v) in [
            ("stock_type", "Fixed size box"),
            ("stock_x", "120"),
            ("stock_y", "80"),
            ("stock_z", "40"),
        ] {
            setup.p.insert(k.into(), v.into());
        }
        let mut tool = Tool::new("End mill 25", "Flat end mill", "25");
        for (k, v) in [("length", "40"), ("stickout", "60")] {
            tool.p.insert(k.into(), v.into());
        }
        tool.p.insert("machines".into(), setup.machine.id.clone());
        let mut op = Operation::new(Strategy::Box, tool.id.clone());
        for (k, v) in crate::model::BOX_FIELDS.iter().zip(b) {
            op.p.insert((*k).into(), v.to_string());
        }
        (setup, op, tool)
    }
    /// Every row of a Box `plan` is conventional with M3, by the rule the
    /// CNC's one-way facing is checked with (`simulation::uncut_sides`): the
    /// uncut material on the left of the feed. The rows' sides, for checks
    /// that there are some.
    fn assert_conventional(plan: &Plan) -> Vec<f32> {
        let along = |m: &Move| usize::from(m.from[1] != m.to[1]);
        let cuts = cuts(plan);
        let rows: Vec<&Move> = (cuts.iter().copied())
            .filter(|m| cuts.first().is_some_and(|f| along(f) == along(m)))
            .collect();
        let sides = crate::simulation::uncut_sides(&rows);
        assert!(plan.spindle_clockwise && plan.milling_direction == "Conventional");
        assert!(sides.iter().all(|s| *s > 0.001), "climb: {sides:?}");
        sides
    }
    fn boxed_plan(b: [f32; 6], fields: &[(&str, &str)]) -> Result<Plan, String> {
        let (setup, mut op, tool) = big_mill(b);
        for (k, v) in fields {
            op.p.insert((*k).into(), (*v).into());
        }
        let plan = generate(&setup, &op, &tool, &block());
        if let Ok(plan) = &plan {
            assert_single_axis(plan);
            assert_conventional(plan);
        }
        plan
    }
    /// The cutting moves: feeds along X or Y.
    fn cuts(plan: &Plan) -> Vec<&Move> {
        plan.moves.iter().filter(|m| m.cutting_pass).collect()
    }
    #[test]
    fn manual_box_and_drill_plans_move_one_axis_at_a_time() {
        let plan = boxed_plan([-100., 100., -100., 15.5, 0., -5.], &[]).unwrap();
        assert!(cuts(&plan).len() > 3);
        assert_plan(plan);
        let mesh = mesh::load(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/four-setup-block.3mf"),
        )
        .unwrap();
        let (mut setup, mut op, tool) = fixture(Strategy::Drill);
        select_top(&mut op, &mesh);
        assert!(holes(&setup, &op, &mesh).unwrap().len() >= 3);
        let cnc = generate(&setup, &op, &tool, &mesh).unwrap();
        // The CNC still moves X and Y together between the holes, at 6000 mm/min.
        assert!(
            cnc.moves
                .iter()
                .any(|m| m.from[0] != m.to[0] && m.from[1] != m.to[1])
        );
        setup.machine = crate::manual::defaults();
        let manual = generate(&setup, &op, &tool, &mesh).unwrap();
        assert_single_axis(&manual);
        let rapids = |plan: &Plan| -> Vec<f32> {
            plan.moves
                .iter()
                .filter(|m| m.motion == Motion::Rapid)
                .map(|m| m.feed)
                .collect()
        };
        assert!(rapids(&cnc).iter().all(|f| *f == 6000.));
        assert!(rapids(&manual).iter().all(|f| *f == 1500.));
        // The same holes, drilled to the same depths.
        let plunges = |plan: &Plan| -> Vec<[f32; 3]> {
            plan.moves
                .iter()
                .filter(|m| m.cutting_pass)
                .map(|m| m.to)
                .collect()
        };
        let (mut a, mut b) = (plunges(&cnc), plunges(&manual));
        for v in [&mut a, &mut b] {
            v.sort_by(|p, q| p.partial_cmp(q).unwrap());
        }
        assert_eq!(a, b);
        assert_plan(manual);
    }
    #[test]
    fn box_coordinates_are_work_coordinates_on_the_setups_zero() {
        let (mut setup, mut op, tool) = big_mill([0., 10., -10., 200., 0., -2.]);
        setup
            .p
            .insert("origin".into(), "Stock top front-left".into());
        op.p.insert("allowance".into(), "0".into());
        let plan = generate(&setup, &op, &tool, &block()).unwrap();
        assert_single_axis(&plan);
        let s = plan.stock;
        assert_eq!((s.min[0], s.max[2]), (-60., 40.));
        // X 0..10 from the stock's front-left corner: the tool's edge stops at X 10.
        let edge = cuts(&plan)
            .iter()
            .map(|m| m.to[0] + 12.5)
            .fold(f32::MIN, f32::max);
        assert!((edge - (s.min[0] + 10.)).abs() < 1e-4, "{edge}");
        assert!(
            cuts(&plan)
                .iter()
                .all(|m| (m.to[2] - (s.max[2] - 2.)).abs() < 1e-4)
        );
        // Passes along Y, with the material (+X) on the left: feeds toward −Y.
        assert!(
            cuts(&plan)
                .iter()
                .all(|m| m.to[1] < m.from[1] && m.to[0] == m.from[0])
        );
        // One row: its direction is the rule's, checked on wider boxes in
        // `a_step_stops_at_the_allowance_and_cuts_conventionally`.
    }
    #[test]
    fn meets_finds_only_triangles_that_reach_into_the_box() {
        let unit = Bounds {
            min: [0.; 3],
            max: [1.; 3],
        };
        // Through the box, and lying on its top face.
        assert!(meets(
            &[[0.5, 0.5, -1.], [0.5, 0.5, 2.], [3., 3., 0.5]],
            &unit
        ));
        assert!(meets(
            &[[-1., -1., 1.], [3., -1., 1.], [-1., 3., 1.]],
            &unit
        ));
        // Their boxes overlap the box, but past its corner: the plane and an
        // edge's crossing separate them.
        assert!(!meets(
            &[[3.5, 0., 0.], [0., 3.5, 0.], [0., 0., 3.5]],
            &unit
        ));
        assert!(!meets(
            &[[0.8, 2.5, 0.5], [2.5, 0.8, 0.5], [2.5, 2.5, 0.5]],
            &unit
        ));
    }
    #[test]
    fn box_runs_only_on_a_manual_mill() {
        let (mut setup, op, tool) = big_mill([-100., 100., -100., 100., 0., -2.]);
        setup.machine = Machine::new(3);
        let error = generate(&setup, &op, &tool, &block()).unwrap_err();
        assert_eq!(error, "Box runs on a manual mill.");
    }
    #[test]
    fn a_box_over_the_whole_top_faces_it_in_one_level() {
        let plan = boxed_plan([-100., 100., -100., 100., 0., -2.], &[("allowance", "0")]).unwrap();
        assert_eq!(plan.levels, 1);
        assert_eq!(plan.milling_direction, "Conventional");
        assert!(plan.spindle_clockwise);
        let rows: Vec<f32> = cuts(&plan).iter().map(|m| m.to[1]).collect();
        // From the − side's face to the + side's in five equal steps of
        // 16 (at most 17.5), every row's tool inside Y ± the radius.
        assert_eq!(rows, [-40., -24., -8., 8., 24., 40.]);
        assert!(
            rows.windows(2)
                .all(|w| w[1] > w[0] && w[1] - w[0] <= 17.5 + 1e-4)
        );
        for m in cuts(&plan) {
            assert!((-52.5..=52.5).contains(&m.to[1]));
            assert_eq!(m.to[2], 38.);
            // Off the stock at both ends, toward +X (the material on +Y, on the left).
            assert_eq!((m.from[0], m.to[0]), (-74.5, 74.5));
        }
        // Down outside the stock: no feed into it from above.
        assert!(
            plan.moves
                .iter()
                .all(|m| m.motion == Motion::Rapid || m.to[2] == m.from[2])
        );
    }
    #[test]
    fn a_step_stops_at_the_allowance_and_cuts_conventionally() {
        let plan = boxed_plan([-100., 100., -100., 15.5, 0., -5.], &[]).unwrap();
        let cuts = cuts(&plan);
        let edges: Vec<f32> = cuts.iter().map(|m| m.to[1] + 12.5).collect();
        assert!(edges.iter().all(|e| *e <= 15. + 1e-4), "{edges:?}");
        assert!((edges[edges.len() - 1] - 15.).abs() < 1e-4);
        // From the open face at −40 to 2.5 in three equal steps.
        let rows: Vec<f32> = cuts.iter().map(|m| m.to[1]).collect();
        for (row, want) in rows
            .iter()
            .zip([-40., -40. + 42.5 / 3., 2.5 - 42.5 / 3., 2.5])
        {
            assert!((row - want).abs() < 1e-4, "{rows:?}");
        }
        // Rows step toward +Y (the uncut material), so they feed along +X
        // with it on their left.
        assert!(
            cuts.windows(2)
                .all(|w| w[1].to[1] > w[0].to[1] || w[1].to[2] < w[0].to[2])
        );
        assert!(
            cuts.iter()
                .all(|m| m.to[0] > m.from[0] && m.from[1] == m.to[1])
        );
        assert_eq!(assert_conventional(&plan).len(), 3);
        // The mirror step, open on +Y: rows step toward −Y and feed along −X.
        let back = boxed_plan([-100., 100., -15.5, 100., 0., -5.], &[]).unwrap();
        assert!(
            (back.moves.iter())
                .filter(|m| m.cutting_pass)
                .all(|m| m.to[0] < m.from[0] && m.from[1] == m.to[1])
        );
        assert_eq!(assert_conventional(&back).len(), 3);
        // Longer in Y, open on +X: rows step toward −X and feed along +Y.
        let side = boxed_plan([15.5, 100., -100., 100., 0., -5.], &[]).unwrap();
        let rows: Vec<&Move> = (side.moves.iter()).filter(|m| m.cutting_pass).collect();
        assert!(
            rows.iter()
                .all(|m| m.to[1] > m.from[1] && m.from[0] == m.to[0])
        );
        assert!(rows.windows(2).all(|w| w[1].to[0] < w[0].to[0]));
        assert!(assert_conventional(&side).len() >= 2);
        // The floor is closed: 5 mm down less the allowance.
        assert!(cuts.iter().all(|m| (m.to[2] - 35.5).abs() < 1e-4));
        assert_eq!(plan.target, 35.5);
    }
    /// The tool's edge of every move of `plan` below the stock's top (Z 40)
    /// stays inside `lo`..`hi` on X and Y (the box less its allowance) and
    /// above `lo[2]`.
    fn assert_inside(plan: &Plan, lo: [f32; 3], hi: [f32; 3]) {
        for m in plan.moves.iter().filter(|m| m.from[2].min(m.to[2]) < 40.) {
            for q in [m.from, m.to] {
                for k in 0..2 {
                    assert!(q[k] - 12.5 >= lo[k] - 1e-4, "{k} {m:?}");
                    assert!(q[k] + 12.5 <= hi[k] + 1e-4, "{k} {m:?}");
                }
                assert!(q[2] >= lo[2] - 1e-4, "{m:?}");
            }
        }
    }
    #[test]
    fn a_closed_pocket_plunges_once_per_level_and_feeds_across_between_rows() {
        // X −50..50, Y −30..30, 6 mm deep in levels of 2, 0.5 left on every
        // side and the floor: three rows (Y −17, 0, 17) on three levels.
        let fields = [("stepdown", "2"), ("allowance", "0.5")];
        let plan = boxed_plan([-50., 50., -30., 30., 0., -6.], &fields).unwrap();
        assert_inside(&plan, [-49.5, -29.5, 34.5], [49.5, 29.5, 40.]);
        let down: Vec<&Move> = (plan.moves.iter())
            .filter(|m| m.to[2] < m.from[2] && m.to[2] < 40.)
            .collect();
        assert_eq!(plan.levels, 3);
        assert_eq!(down.len(), 3, "{down:?}");
        let rows = cuts(&plan);
        assert_eq!(rows.len(), 9);
        for (level, d) in down.iter().enumerate() {
            // At a quarter feed, onto the level, where its first row starts.
            assert!(d.motion == Motion::Feed && d.feed == 30., "{d:?}");
            let first = rows[level * 3];
            assert_eq!(d.to, first.from);
        }
        // Between rows: off the wall 0.2 mm (a rapid), back along the row
        // at the hand rapid, then a cutting feed across, a pause, the pass.
        // The first row lies on the −Y wall's allowance: no room behind it,
        // so it goes back along its own cut.
        for pair in rows.windows(2).filter(|w| w[0].to[2] == w[1].to[2]) {
            let mut between: Vec<&Move> = (plan.moves.iter())
                .filter(|m| m.start >= pair[0].end && m.end <= pair[1].start)
                .collect();
            let first = (pair[0].to[1] + 17.).abs() < 1e-4;
            let kinds: Vec<Motion> = between.iter().map(|m| m.motion).collect();
            assert_eq!(
                kinds,
                [Motion::Rapid, Motion::Rapid, Motion::Feed, Motion::Dwell][usize::from(first)..],
                "{between:?}"
            );
            if first {
                between.insert(0, pair[0]);
            }
            let (off, back, across) = (between[0], between[1], between[2]);
            let backed = if first { 0. } else { 0.2 };
            assert!(
                (off.to[1] - (pair[0].to[1] - backed)).abs() < 1e-4,
                "{off:?}"
            );
            assert_eq!(back.feed, 1500.);
            assert_eq!(back.to[0], pair[1].from[0]);
            assert!(back.to[2] == pair[0].to[2] && back.from[1] == back.to[1]);
            assert!(across.to[1] > across.from[1] && across.feed == 120.);
            assert_eq!(across.to, pair[1].from);
        }
        assert_eq!(assert_conventional(&plan).len(), 6);
        // The first row's cross side is closed with nothing behind it: it
        // goes back along its own cut, not into the wall's allowance.
        let fields = [("stepdown", "6"), ("allowance", "0")];
        let plan = boxed_plan([-50., 50., -30., 30., 0., -6.], &fields).unwrap();
        assert_inside(&plan, [-50., -30., 34.], [50., 30., 40.]);
        let first = cuts(&plan)[0];
        let next = (plan.moves.iter())
            .find(|m| m.start >= first.end && m.motion != Motion::Dwell)
            .unwrap();
        assert!(
            next.from[1] == next.to[1] && next.to[0] < next.from[0],
            "{next:?}"
        );
        // A step closed at both X ends, open on −Y: off the open side
        // without limit, once per level.
        let fields = [("stepdown", "5"), ("allowance", "0.5")];
        let plan = boxed_plan([-50., 50., -100., 15.5, 0., -5.], &fields).unwrap();
        assert_inside(&plan, [-49.5, -100., 35.5], [49.5, 15., 40.]);
        let plunges = (plan.moves.iter())
            .filter(|m| m.motion == Motion::Feed && m.to[2] < m.from[2])
            .count();
        assert_eq!((plunges, plan.levels), (1, 1));
        assert!(assert_conventional(&plan).len() >= 2);
    }
    #[test]
    fn a_closed_pocket_plunges_at_a_quarter_feed_onto_its_floor_and_allowance() {
        // One level: the slot is 4.5 mm deep, the stepdown given.
        let plan = boxed_plan([-55., -15., 12., 38., 0., -5.], &[("stepdown", "5")]).unwrap();
        let downs: Vec<&Move> = plan
            .moves
            .iter()
            .filter(|m| m.to[2] < m.from[2] && m.to[2] < 40.)
            .collect();
        assert!(!downs.is_empty());
        assert!(
            downs
                .iter()
                .all(|m| m.motion == Motion::Feed && m.feed == 30.)
        );
        assert!(downs.iter().all(|m| (m.to[2] - 35.5).abs() < 1e-4));
        assert_eq!(plan.target, 35.5);
        // Inside the walls less the allowance.
        for m in cuts(&plan) {
            for x in [m.from[0], m.to[0]] {
                assert!(x - 12.5 >= -54.5 - 1e-4 && x + 12.5 <= -15.5 + 1e-4, "{x}");
            }
            assert!(m.to[1] - 12.5 >= 12.5 - 1e-4 && m.to[1] + 12.5 <= 37.5 + 1e-4);
        }
    }
    #[test]
    fn box_levels_step_down_to_the_floor() {
        let levels = |plan: &Plan| {
            let mut z: Vec<f32> = cuts(plan).iter().map(|m| m.to[2] - 40.).collect();
            z.dedup();
            z
        };
        let deep = [-100., 100., -100., 100., 0., -9.5];
        let plan = boxed_plan(deep, &[("allowance", "0"), ("stepdown", "2.5")]).unwrap();
        // Equal steps of at most 2.5: four of 2.375, the last on the floor.
        assert_eq!(levels(&plan), [-2.375, -4.75, -7.125, -9.5]);
        assert_eq!(plan.levels, 4);
        // The closed floor keeps 0.5: four of 2.25 down to −9.
        let plan = boxed_plan(deep, &[("stepdown", "2.5")]).unwrap();
        assert_eq!(levels(&plan), [-2.25, -4.5, -6.75, -9.]);
        // Automatic: what the 800 W spindle takes at 17.5 mm wide in steel
        // with the 0.0625 mm chip of 120 mm/min at 640 rpm on 3 flutes
        // (4.71 mm), here below the flute length and the diameter, so three
        // of 9.5 / 3.
        let (setup, op, tool) = big_mill(deep);
        assert_eq!(
            [
                num(&op.p, "rpm"),
                num(&op.p, "feed"),
                num(&tool.p, "flutes")
            ],
            [Some(640.), Some(120.), Some(3.)]
        );
        let auto = crate::forces::power_stepdown(&setup.machine, None, 25., 17.5, 120., 640., 3.);
        assert!((auto - 4.7059).abs() < 0.001, "{auto}");
        let plan = boxed_plan(deep, &[("allowance", "0")]).unwrap();
        let z = levels(&plan);
        assert_eq!((z.len(), z[2]), (3, -9.5), "{z:?}");
        assert!((z[0] + 9.5 / 3.).abs() < 1e-4, "{z:?}");
        // Without the rpm or the flutes it cannot say.
        let error = boxed_plan(deep, &[("rpm", "")]).unwrap_err();
        assert!(error.contains("automatic stepdown"), "{error}");
        // Capped at 4 mm flutes: three of 9.5 / 3.
        let (setup, mut op, mut tool) = big_mill(deep);
        tool.p.insert("length".into(), "4".into());
        op.p.insert("allowance".into(), "0".into());
        let plan = generate(&setup, &op, &tool, &block()).unwrap();
        let z = levels(&plan);
        assert_eq!((z.len(), z[2]), (3, -9.5));
        assert!(
            (z[0] + 9.5 / 3.).abs() < 1e-4 && (z[1] + 19. / 3.).abs() < 1e-4,
            "{z:?}"
        );
        op.p.insert("stepdown".into(), "5".into());
        assert!(generate(&setup, &op, &tool, &block()).is_err());
    }
    /// A box closed on both cross sides cuts a full slot with its first row:
    /// its automatic stepdown is sized at the tool's diameter, not the
    /// stepover. An open face only ever engages the stepover.
    #[test]
    fn a_closed_pockets_automatic_stepdown_is_sized_for_the_full_slot() {
        let (setup, ..) = big_mill([0.; 6]);
        // Ø25 at 120 mm/min, 640 rpm, 3 flutes; stepover 70 % is 17.5 mm.
        let power = |width: f32| {
            crate::forces::power_stepdown(&setup.machine, None, 25., width, 120., 640., 3.)
        };
        let (narrow, full) = (power(17.5), power(25.));
        assert!(full < narrow && narrow < 12., "{full} {narrow}");
        // Equal levels of at most `down` down a 12 mm block.
        let levels = |plan: &Plan| {
            let mut z: Vec<f32> = cuts(plan).iter().map(|m| m.to[2] - 40.).collect();
            z.dedup();
            z
        };
        let steps = |down: f32| (12. / down - 0.0001).ceil() as usize;
        assert_ne!(steps(full), steps(narrow));
        let closed = boxed_plan([-50., 50., -30., 30., 0., -12.], &[("allowance", "0")]).unwrap();
        assert_eq!(closed.levels, steps(full), "{:?}", levels(&closed));
        let open = boxed_plan([-100., 100., -100., 100., 0., -12.], &[("allowance", "0")]).unwrap();
        assert_eq!(open.levels, steps(narrow), "{:?}", levels(&open));
        // Closed on one cross side only, the first row hangs off the open one.
        let step = boxed_plan([-50., 50., -100., 30., 0., -12.], &[("allowance", "0")]).unwrap();
        assert_eq!(step.levels, steps(narrow));
    }
    /// Steps between levels (from the top) and between rows: none under half
    /// of the stepdown or the stepover, and none over it, unless it is the
    /// only one.
    #[test]
    fn box_levels_and_rows_are_never_thin() {
        for b in [
            [-100., 100., -100., 100., 0., -9.5],
            [-100., 100., -100., 15.5, 0., -5.3],
            [-100., 100., -31., 100., 0., -12.],
            [-55., -15., 12., 38., 0., -2.],
        ] {
            for (down, over) in [("1.7", "30"), ("2.5", "50"), ("3", "70"), ("4.1", "100")] {
                let fields = [("stepdown", down), ("stepover", over)];
                let plan = boxed_plan(b, &fields).unwrap();
                let (down, width) = (
                    down.parse::<f32>().unwrap(),
                    over.parse::<f32>().unwrap() / 4.,
                );
                let mut z = vec![40.];
                z.extend(cuts(&plan).iter().map(|m| m.to[2]));
                z.dedup();
                let mut levels = z.windows(2).map(|w| w[0] - w[1]).collect::<Vec<_>>();
                let rows: Vec<f32> = cuts(&plan)
                    .iter()
                    .take_while(|m| m.to[2] == z[1])
                    .map(|m| m.to[1])
                    .collect();
                let mut across = rows
                    .windows(2)
                    .map(|w| (w[1] - w[0]).abs())
                    .collect::<Vec<_>>();
                for (steps, by) in [(&mut levels, down), (&mut across, width)] {
                    assert!(
                        steps.iter().all(|s| *s <= by + 1e-4),
                        "{b:?} {steps:?} {by}"
                    );
                    if steps.len() > 1 {
                        assert!(
                            steps.iter().all(|s| *s >= by / 2. - 1e-4),
                            "{b:?} {steps:?} {by}"
                        );
                    }
                }
            }
        }
    }
    #[test]
    fn a_box_into_the_part_is_refused() {
        let error = boxed_plan([-100., 100., -100., 100., 0., -35.], &[]).unwrap_err();
        assert!(
            error.starts_with("The box cuts into the part at X"),
            "{error}"
        );
        let (setup, ..) = big_mill([0.; 6]);
        let part = stock::resolve(&setup, Some(&block()))
            .unwrap()
            .part
            .unwrap();
        // 0.5 mm over the part's top, and 0.5 mm off its +X side.
        let top = part.max[2] - 40. + 0.5;
        let face = [-100., 100., -100., 100., 0., top];
        assert!(boxed_plan(face, &[("allowance", "0")]).is_ok());
        let side = [part.max[0] + 0.5, 100., -100., 100., 0., -39.];
        assert!(boxed_plan(side, &[("allowance", "0")]).is_ok());
        let into = [part.max[0] - 0.5, 100., -100., 100., 0., -39.];
        let error = boxed_plan(into, &[("allowance", "0")]).unwrap_err();
        let x: f32 = error
            .strip_prefix("The box cuts into the part at X")
            .and_then(|rest| rest.split(',').next())
            .and_then(|x| x.parse().ok())
            .unwrap_or_else(|| panic!("{error}"));
        assert!((part.max[0] - 0.5..=part.max[0]).contains(&x), "{error}");
    }
    #[test]
    fn a_one_row_square_checks_only_its_row_against_the_part() {
        // The +X side of a 24 mm wide block round the 20 mm part squared 1
        // mm, down through the stock's bottom: one row, 1 mm off the part.
        let (mut setup, mut op, tool) = big_mill([11., 100., -100., 100., 0., -100.]);
        setup.p.insert("stock_x".into(), "24".into());
        op.p.insert("allowance".into(), "0".into());
        let plan = generate(&setup, &op, &tool, &block()).unwrap();
        assert_single_axis(&plan);
        assert_eq!(cuts(&plan).len(), plan.levels);
        assert!(cuts(&plan).iter().all(|m| m.to[0] == 23.5));
        assert_eq!(plan.target, 0.);
    }
    #[test]
    fn a_single_row_box_takes_its_feeds_rapids_and_pause() {
        let (setup, mut op, tool) = big_mill([-55., -15., 12., 38., 0., -5.]);
        // One level: the stepdown given (automatic, the full slot takes two).
        op.p.insert("stepdown".into(), "5".into());
        let plan = generate(&setup, &op, &tool, &block()).unwrap();
        assert_eq!(cuts(&plan).len(), 1);
        let clearance = 40. + num(&op.p, "clearance").unwrap();
        let retract = 45.;
        // From the corner at clearance: down to retract, along X to −42,
        // across Y to 25, plunge at 30 mm/min to 35.5, pause 20 s, feed
        // 14 mm at 120 mm/min, up to retract and clearance.
        let rapids = (clearance - retract) * 2. + 18. + 65. + (retract - 35.5);
        let expected = rapids / 1500. * 60. + (retract - 35.5) / 30. * 60. + 20. + 14. / 120. * 60.;
        assert!(
            (plan.duration - expected).abs() < 0.1,
            "{} {expected}",
            plan.duration
        );
    }
    #[test]
    fn a_faced_top_on_the_manual_mill_lowers_the_next_setups_stock() {
        let (mut doc, mesh) = crate::post::fixture::job();
        let machine = crate::manual::defaults();
        let mut tool = Tool::new("End mill 25", "Flat end mill", "25");
        for (k, v) in [
            ("length", "40"),
            ("stickout", "60"),
            ("machines", &machine.id),
        ] {
            tool.p.insert(k.into(), v.into());
        }
        let mut face = Operation::new(Strategy::Box, tool.id.clone());
        for (k, v) in crate::model::BOX_FIELDS
            .iter()
            .zip(["-500", "500", "-500", "500", "0", "-2"])
            .chain([(&"allowance", "0")])
        {
            face.p.insert((*k).into(), v.into());
        }
        doc.machines.push(machine);
        doc.tools.push(tool);
        doc.add_manual_setup().unwrap();
        doc.setups[0].operations = vec![face];
        let mut starts = vec![];
        let mut first = None;
        simulation::sequence_walk(
            &doc,
            &mesh,
            Some((1, 0)),
            |si, _, result| {
                if si == 0 {
                    let sim = result.unwrap();
                    assert_single_axis(&sim.plan);
                    first = Some(sim.plan.stock);
                }
            },
            |_, _, _| {},
            |si, start| starts.push((si, start)),
        );
        let (first, second) = (first.unwrap(), starts[1].1.unwrap());
        assert_eq!(starts[1].0, 1);
        assert!(
            (first.max[2] - 2. - second.max[2]).abs() < 0.05,
            "{first:?} {second:?}"
        );
        for k in 0..2 {
            assert!((first.min[k] - second.min[k]).abs() < 0.05);
            assert!((first.max[k] - second.max[k]).abs() < 0.05);
        }
    }
    #[test]
    fn real_step_and_3mf_new_operations_generate_and_survive_save_reload() {
        for ext in ["step", "3mf"] {
            if ext == "step" && crate::step::skip_without_helper() {
                continue;
            }
            let mesh = mesh::load(
                &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join(format!("examples/four-setup-block.{ext}")),
            )
            .unwrap();
            for s in [
                Strategy::Contour,
                Strategy::Drill,
                Strategy::Parallel,
                Strategy::Waterline,
            ] {
                let (setup, mut op, mut tool) = fixture(s);
                tool.p.insert(
                    "diameter".into(),
                    if s == Strategy::Drill { "4" } else { "6" }.into(),
                );
                op.p.insert("surface_stepover".into(), "2".into());
                op.p.insert("tolerance".into(), "0.1".into());
                if s == Strategy::Drill {
                    select_top(&mut op, &mesh);
                    let detected = holes(&setup, &op, &mesh).unwrap();
                    eprintln!("{ext}: {} holes: {:?}", detected.len(), detected);
                    assert!(detected.len() >= 4);
                }
                let saved: Operation =
                    serde_json::from_str(&serde_json::to_string(&op).unwrap()).unwrap();
                let plan = generate(&setup, &saved, &tool, &mesh)
                    .unwrap_or_else(|e| panic!("{ext} {s:?}: {e}"));
                eprintln!(
                    "{ext} {s:?}: {} moves, {} levels",
                    plan.moves.len(),
                    plan.levels
                );
                assert_plan(plan);
            }
        }
    }

    /// A Ø24 × 10 disc with a Ø4 hole through it, 64 sides, whose opening
    /// in the top is the section `rim` from the top face's inner edge down
    /// to the bore's wall: (radius, height) points.
    fn rimmed(rim: &[(f32, f32)]) -> Mesh {
        let mut section = vec![(2., 0.), (12., 0.), (12., 10.)];
        section.extend_from_slice(rim);
        section.push((2., 0.));
        mesh::normalize(revolved(&section)).unwrap()
    }
    /// The one hole of the top of `mesh`, with the Drill fixture.
    fn top_hole(mesh: &Mesh) -> Hole {
        let (setup, mut op, _) = fixture(Strategy::Drill);
        select_top(&mut op, mesh);
        let found = holes(&setup, &op, mesh).unwrap();
        assert_eq!(found.len(), 1);
        found[0].clone()
    }
    #[test]
    fn modeled_threads_are_holes_with_their_sizes() {
        // Two M6×1 and an M8×1.25 tapped through, an M5×0.8 blind, and two
        // plain Ø6.6 holes (scripts/make-threaded-plate.py).
        let mesh = crate::mesh::load(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/threaded-plate.3mf"),
        )
        .unwrap();
        let (setup, mut op, _) = fixture(Strategy::Drill);
        select_top(&mut op, &mesh);
        let found = holes(&setup, &op, &mesh).unwrap();
        let mut names: Vec<(String, bool)> = found
            .iter()
            .map(|h| {
                (
                    h.thread.as_ref().map_or("plain".into(), |t| t.name()),
                    h.through,
                )
            })
            .collect();
        names.sort();
        assert_eq!(
            names,
            [
                ("M5×0.8".into(), false),
                ("M6×1".into(), true),
                ("M6×1".into(), true),
                ("M8×1.25".into(), true),
                ("plain".into(), true),
                ("plain".into(), true),
            ]
        );
        // A threaded hole's drill size is its minor diameter.
        let m8 = found
            .iter()
            .find(|h| h.thread.as_ref().is_some_and(|t| t.name() == "M8×1.25"))
            .unwrap();
        assert!((m8.diameter - 6.65).abs() < 0.1, "{m8:?}");
    }
    fn threaded_plate() -> Mesh {
        crate::mesh::load(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/threaded-plate.3mf"),
        )
        .unwrap()
    }
    fn thread_mill_tool(diameter: &str) -> Tool {
        let mut t = Tool::new("Thread mill", "Thread mill", diameter);
        t.set_type("Thread mill");
        for (k, v) in [
            ("pitches", "0.5-1.25"),
            ("reach", "16"),
            ("length", "1"),
            ("stickout", "40"),
        ] {
            t.p.insert(k.into(), v.into());
        }
        t
    }
    /// The threaded plate on `machine`: its holes of bore `bore` (±0.1)
    /// drilled Ø`drill` through the part's depth when `drilled`, then
    /// `tool`'s Drill of `cycle` on them, edited by `edit`; the threading
    /// operation's result.
    fn threaded_job(
        machine: Machine,
        bore: &str,
        drilled: Option<&str>,
        tool: Tool,
        cycle: &str,
        edit: impl Fn(&mut Operation),
    ) -> (Result<Simulation, String>, Mesh) {
        let (result, mesh, _) = threaded_doc(machine, bore, drilled, tool, cycle, edit);
        (result, mesh)
    }
    /// `threaded_job`, with its document and the stock it leaves.
    fn threaded_doc(
        machine: Machine,
        bore: &str,
        drilled: Option<&str>,
        tool: Tool,
        cycle: &str,
        edit: impl Fn(&mut Operation),
    ) -> (
        Result<Simulation, String>,
        Mesh,
        (crate::model::Document, Option<crate::volume::Volume>),
    ) {
        let mesh = threaded_plate();
        let (mut setup, mut drill, _) = fixture(Strategy::Drill);
        setup.machine = machine;
        select_top(&mut drill, &mesh);
        drill.p.insert("hole_diameter".into(), bore.into());
        let mut doc = crate::model::Document::default();
        let mut thread = drill.clone();
        thread.id = "threading".into();
        thread.tool = tool.id.clone();
        thread.p.insert("cycle".into(), cycle.into());
        edit(&mut thread);
        if let Some(d) = drilled {
            // Through holes drilled through: a drill's point leaves a cone
            // at the bottom a thread mill would run into.
            if bore != "4.1" {
                drill.p.insert("through".into(), "Yes".into());
                drill.p.insert("breakthrough".into(), "1.5".into());
            }
            // A speed the manual mill's gears have, too.
            drill.p.insert("rpm".into(), "640".into());
            drill.p.insert("feed".into(), "60".into());
            let mut t = Tool::new("Tap drill", "Drill", d);
            t.p.insert("machines".into(), setup.machine.id.clone());
            t.p.insert("length".into(), "40".into());
            t.p.insert("stickout".into(), "45".into());
            drill.tool = t.id.clone();
            doc.tools.push(t);
            setup.operations.push(drill);
        }
        let mut tool = tool;
        tool.p.insert("machines".into(), setup.machine.id.clone());
        doc.tools.push(tool);
        setup.operations.push(thread);
        doc.machines = vec![setup.machine.clone()];
        doc.setups = vec![setup];
        let mut all = vec![];
        let mut stock = None;
        crate::simulation::sequence_with(
            &doc,
            &mesh,
            None,
            |_, _, r| all.push(r),
            |_, _, v| stock = Some(v.clone()),
        );
        let last = all.pop().unwrap();
        for r in &all {
            if let Err(e) = r {
                panic!("the drilling before failed: {e}");
            }
        }
        (last, mesh, (doc, stock))
    }
    /// The cutting moves of `plan` on the orbit of radius `radius` round
    /// `centre`, as (angle, z) of their ends, in order.
    fn orbit(plan: &Plan, centre: [f32; 2], radius: f32) -> Vec<(f32, f32)> {
        plan.moves
            .iter()
            .filter(|m| m.cutting_pass)
            .map(|m| m.to)
            .filter(|p| ((p[0] - centre[0]).hypot(p[1] - centre[1]) - radius).abs() < 0.002)
            .map(|p| ((p[1] - centre[1]).atan2(p[0] - centre[0]), p[2]))
            .collect()
    }
    #[test]
    fn thread_milling_climbs_one_pitch_a_turn_the_threads_way() {
        // The M6×1 holes (bore Ø4.9), drilled Ø4.5, a Ø3.15 thread mill:
        // its full orbit is 3 − 1.575 from the axis.
        for (hand, rises) in [("From the model", true), ("Left hand", false)] {
            let (result, _) = threaded_job(
                Machine::new(3),
                "4.9",
                Some("4.5"),
                thread_mill_tool("3.15"),
                crate::model::THREAD_MILLING,
                |o| {
                    o.p.insert("thread_hand".into(), hand.into());
                },
            );
            let sim = result.unwrap_or_else(|e| panic!("{hand}: {e}"));
            let plan = &sim.plan;
            let centre = [-25., -10.];
            // The hole's centre in the setup frame: find it from the moves.
            let cut: Vec<[f32; 3]> = plan
                .moves
                .iter()
                .filter(|m| m.cutting_pass)
                .map(|m| m.to)
                .collect();
            assert!(!cut.is_empty());
            let lowest = cut.iter().map(|p| p[2]).fold(f32::INFINITY, f32::min);
            let _ = centre;
            // Group by hole: the two M6 holes are 20 apart in Y.
            let ys: Vec<f32> = cut.iter().map(|p| p[1]).collect();
            let split = (ys.iter().copied().fold(f32::INFINITY, f32::min)
                + ys.iter().copied().fold(f32::NEG_INFINITY, f32::max))
                / 2.;
            let one: Vec<[f32; 3]> = cut.iter().copied().filter(|p| p[1] < split).collect();
            let (mx, my) = one.iter().fold((0., 0.), |(a, b), p| (a + p[0], b + p[1]));
            let rough = [mx / one.len() as f32, my / one.len() as f32];
            let full = 3. - 1.575;
            // The orbit's centre: the middle of its extreme points.
            let far: Vec<&[f32; 3]> = one
                .iter()
                .filter(|p| ((p[0] - rough[0]).hypot(p[1] - rough[1]) - full).abs() < 0.2)
                .collect();
            let xs = far.iter().map(|p| p[0]);
            let ys = far.iter().map(|p| p[1]);
            let centre = [
                (xs.clone().fold(f32::INFINITY, f32::min) + xs.fold(f32::NEG_INFINITY, f32::max))
                    / 2.,
                (ys.clone().fold(f32::INFINITY, f32::min) + ys.fold(f32::NEG_INFINITY, f32::max))
                    / 2.,
            ];
            let path = orbit(plan, centre, full);
            assert!(path.len() > 100, "{hand}: {} orbit points", path.len());
            for pair in path.windows(2) {
                let mut da = pair[1].0 - pair[0].0;
                if da > std::f32::consts::PI {
                    da -= std::f32::consts::TAU;
                }
                if da < -std::f32::consts::PI {
                    da += std::f32::consts::TAU;
                }
                // Steps of one helix only: not from one pass's end to the next's start.
                if da.abs() < 1e-6 || da.abs() > 0.6 || (pair[1].1 - pair[0].1).abs() > 0.5 {
                    continue;
                }
                // Counterclockwise (climb, spindle clockwise), one pitch a turn.
                assert!(da > 0., "{hand}: clockwise step {da}");
                let dz = pair[1].1 - pair[0].1;
                let expected = 1. * da / std::f32::consts::TAU * if rises { 1. } else { -1. };
                assert!(
                    (dz - expected).abs() < 2e-4,
                    "{hand}: dz {dz} for {da} rad, expected {expected}"
                );
            }
            // Through holes: half a pitch past the exit.
            assert!(lowest < 4. / mesh_scale_free() + 0., "{lowest}");
        }
    }
    #[test]
    fn a_three_tooth_mill_cuts_the_teeth_of_the_thread_in_fewer_turns() {
        let mut three = thread_mill_tool("4");
        three.p.insert("pitches".into(), "1".into());
        three.p.insert("teeth".into(), "3".into());
        let mut feeds = vec![];
        for tool in [thread_mill_tool("4"), three] {
            let (result, _, (_, stock)) = threaded_doc(
                Machine::new(3),
                "4.9",
                Some("5"),
                tool,
                crate::model::THREAD_MILLING,
                |_| {},
            );
            let sim = result.unwrap();
            feeds.push(sim.plan.moves.iter().filter(|m| m.cutting_pass).count());
            let cuts: Vec<_> = stock
                .unwrap()
                .cuts
                .iter()
                .filter(|c| c.shape.teeth().is_some())
                .cloned()
                .collect();
            // One M6×1 hole's orbit: its centre, and how far up it runs.
            let ys: Vec<f32> = cuts.iter().map(|c| c.b[1]).collect();
            let split = (ys.iter().copied().fold(f32::INFINITY, f32::min)
                + ys.iter().copied().fold(f32::NEG_INFINITY, f32::max))
                / 2.;
            let one: Vec<_> = cuts.iter().filter(|c| c.b[1] < split).collect();
            let extent = |k: usize| {
                let v = one.iter().map(|c| c.b[k]);
                (
                    v.clone().fold(f32::INFINITY, f32::min),
                    v.fold(f32::NEG_INFINITY, f32::max),
                )
            };
            let (x, y, z) = (extent(0), extent(1), extent(2));
            let centre = [(x.0 + x.1) / 2., (y.0 + y.1) / 2.];
            let cut = |rho: f32, z: f32| {
                let p = [centre[0] + rho, centre[1], z];
                one.iter()
                    .any(|c| crate::volume::swept(p, c.a, c.b, c.radius, c.flute, c.shape))
            };
            // Down the thread, between its major and minor radius, the
            // teeth's grooves and the crests left between them alternate
            // a pitch apart; past the major radius nothing is cut.
            let samples: Vec<bool> = (0..200)
                .map(|k| cut(2.75, z.0 + 1. + k as f32 * 0.03))
                .collect();
            let grooves = samples.iter().filter(|&&c| c).count();
            let changes = samples.windows(2).filter(|w| w[0] != w[1]).count();
            assert!((50..150).contains(&grooves), "{grooves} of 200 cut");
            // 6 mm down the thread: 6 grooves, each in and out.
            assert!((10..=14).contains(&changes), "{changes} changes");
            assert!((0..200).all(|k| !cut(3.1, z.0 + 1. + k as f32 * 0.03)));
            assert!(z.1 - z.0 > 5.);
        }
        // The two teeth above the lowest cut the last two turns.
        assert!(feeds[1] < feeds[0], "{feeds:?}");
    }
    #[test]
    fn a_three_tooth_mill_with_a_range_of_pitches_is_refused() {
        let mut three = thread_mill_tool("4");
        three.p.insert("teeth".into(), "3".into());
        assert!(
            three
                .issues()
                .contains(&crate::model::TEETH_ONE_PITCH.to_string())
        );
        let (result, _) = threaded_job(
            Machine::new(3),
            "4.9",
            Some("5"),
            three,
            crate::model::THREAD_MILLING,
            |_| {},
        );
        assert_eq!(result.err().as_deref(), Some(crate::model::TEETH_ONE_PITCH));
    }
    #[test]
    fn a_milled_thread_is_no_gouge() {
        // The M6 holes (bore Ø4.9): drilled Ø5, their tap drill, then milled
        // to their roots by a Ø4 thread mill. Both cut cells of the part's
        // thread, which is finer than the cells.
        let (result, mesh, (doc, stock)) = threaded_doc(
            Machine::new(3),
            "4.9",
            Some("5"),
            thread_mill_tool("4"),
            crate::model::THREAD_MILLING,
            |_| {},
        );
        result.unwrap();
        let stock = stock.unwrap();
        let masked = crate::plan_check::PartGrid::in_job(&stock, &doc, &mesh)
            .gouged(&stock)
            .0;
        assert_eq!(masked, 0);
        // The band round each thread is not deep part: on a part small
        // enough that its cells are finer, a thread's would be.
        let geometry = Geometry::new(&doc.setups[0], &mesh);
        let threads = crate::thread::in_setup(&doc.setups[0], &mesh);
        assert_eq!(threads.len(), 4);
        let banded = crate::plan_check::PartGrid::new(&stock, &geometry, &threads).deep_cells();
        let bare = crate::plan_check::PartGrid::new(&stock, &geometry, &[]).deep_cells();
        assert!(banded < bare, "{banded} {bare}");
    }
    #[test]
    fn a_thread_milling_job_posts_and_replays_exactly() {
        let (result, mesh, (mut doc, _)) = threaded_doc(
            Machine::new(3),
            "4.9",
            Some("5"),
            thread_mill_tool("4"),
            crate::model::THREAD_MILLING,
            |_| {},
        );
        result.unwrap();
        doc.file = Some(crate::model::ModelFile {
            name: "threaded-plate.3mf".into(),
            path: "examples/threaded-plate.3mf".into(),
            format: "3MF".into(),
            bytes: 1,
            thickness: None,
        });
        for (n, t) in doc.tools.iter_mut().enumerate() {
            t.p.insert("number".into(), (n + 1).to_string());
        }
        let job = crate::post::post_job(&doc, &mesh);
        let problems: Vec<String> = job
            .errors
            .iter()
            .cloned()
            .chain(
                job.setups
                    .iter()
                    .filter_map(|s| s.result.as_ref().err())
                    .flatten()
                    .cloned(),
            )
            .collect();
        assert!(job.ready(), "{problems:?}");
        let program = &job.setups[0].result.as_ref().unwrap()[0];
        assert!(
            program.text.contains("T2 M6") && program.text.contains("\nG1 "),
            "{}",
            &program.text[..500]
        );
    }
    /// Writes the M6 thread milling's moves and the plate, in the setup's
    /// frame, to `$AXIS_RENDER_DIR` (default /tmp/threading) for a
    /// picture. Not a check: run it with --ignored.
    #[test]
    #[ignore]
    fn export_a_milled_thread() {
        use std::io::Write;
        let out = std::path::PathBuf::from(
            std::env::var("AXIS_RENDER_DIR").unwrap_or("/tmp/threading".into()),
        );
        std::fs::create_dir_all(&out).unwrap();
        // The owner's 3-tooth Ø4 mill, its teeth a pitch apart.
        let mut tool = thread_mill_tool("4");
        tool.p.insert("pitches".into(), "1".into());
        tool.p.insert("teeth".into(), "3".into());
        let (result, mesh, (doc, stock)) = threaded_doc(
            Machine::new(3),
            "4.9",
            Some("5"),
            tool,
            crate::model::THREAD_MILLING,
            |_| {},
        );
        let plan = result.unwrap().plan;
        // The stock it leaves, at each detail.
        let stock = stock.unwrap();
        for detail in [
            crate::stock_surface::Detail::Standard,
            crate::stock_surface::Detail::Ultra,
        ] {
            let surface = crate::stock_surface::Snapshot::of(&stock, detail)
                .build()
                .unwrap();
            let mut f = std::fs::File::create(out.join(format!("stock_{detail}.stl"))).unwrap();
            writeln!(f, "solid stock").unwrap();
            for t in &surface.triangles {
                writeln!(f, "facet normal 0 0 0\nouter loop").unwrap();
                for i in t {
                    // The first setup's frame: the moves'.
                    let p = surface.vertices[*i];
                    writeln!(f, "vertex {} {} {}", p[0], p[1], p[2]).unwrap();
                }
                writeln!(f, "endloop\nendfacet").unwrap();
            }
            writeln!(f, "endsolid stock").unwrap();
            eprintln!(
                "{detail}: {} triangles, {:.3} mm cells, {} ms",
                surface.triangles.len(),
                surface.cell_mm,
                surface.milliseconds
            );
        }
        let mut f = std::fs::File::create(out.join("m6_path.csv")).unwrap();
        for m in &plan.moves {
            writeln!(
                f,
                "{},{},{},{},{},{},{}",
                m.from[0],
                m.from[1],
                m.from[2],
                m.to[0],
                m.to[1],
                m.to[2],
                u8::from(m.cutting_pass)
            )
            .unwrap();
        }
        let g = Geometry::new(&doc.setups[0], &mesh);
        let mut f = std::fs::File::create(out.join("plate_setup.stl")).unwrap();
        writeln!(f, "solid plate").unwrap();
        for t in &g.triangles {
            writeln!(f, "facet normal 0 0 0\nouter loop").unwrap();
            for p in t.p {
                writeln!(f, "vertex {} {} {}", p[0], p[1], p[2]).unwrap();
            }
            writeln!(f, "endloop\nendfacet").unwrap();
        }
        writeln!(f, "endsolid plate").unwrap();
        eprintln!(
            "radius {} levels {} duration {}",
            plan.radius, plan.levels, plan.duration
        );
    }
    /// The plate's bottom in a setup frame: placement puts it a fixed 4
    /// display units up, which `threaded_plate`'s 70 mm makes 2 mm.
    fn mesh_scale_free() -> f32 {
        2.
    }
    #[test]
    fn thread_milling_refuses_an_undrilled_hole_a_wrong_pitch_and_a_short_reach() {
        let (result, _) = threaded_job(
            Machine::new(3),
            "4.9",
            None,
            thread_mill_tool("3.15"),
            crate::model::THREAD_MILLING,
            |_| {},
        );
        let e = result.unwrap_err();
        assert!(e.starts_with("Drill the M6×1 hole"), "{e}");
        let mut fine = thread_mill_tool("3.15");
        fine.p.insert("pitches".into(), "0.5-0.8".into());
        let e = threaded_job(
            Machine::new(3),
            "4.9",
            Some("4.5"),
            fine,
            crate::model::THREAD_MILLING,
            |_| {},
        )
        .0
        .unwrap_err();
        assert!(e.contains("pitch of 1 mm, outside"), "{e}");
        let mut short = thread_mill_tool("3.15");
        short.p.insert("reach".into(), "8".into());
        let e = threaded_job(
            Machine::new(3),
            "4.9",
            Some("4.5"),
            short,
            crate::model::THREAD_MILLING,
            |_| {},
        )
        .0
        .unwrap_err();
        assert!(e.contains("reaches 8 mm"), "{e}");
        let e = threaded_job(
            Machine::new(3),
            "4.9",
            Some("4.5"),
            thread_mill_tool("4.95"),
            crate::model::THREAD_MILLING,
            |_| {},
        )
        .0
        .unwrap_err();
        assert!(e.contains("too wide"), "{e}");
    }
    #[test]
    fn tapping_is_by_hand_and_needs_the_tap_drill_and_the_taps_lead() {
        let mut tap = Tool::new("M6 tap", "Tap", "6");
        tap.set_type("Tap");
        tap.p.insert("thread".into(), "M6×1".into());
        tap.p.insert("length".into(), "20".into());
        tap.p.insert("stickout".into(), "40".into());
        // The CNC cannot tap.
        let e = threaded_job(
            Machine::new(3),
            "4.9",
            Some("5"),
            tap.clone(),
            crate::model::TAPPING,
            |_| {},
        )
        .0
        .unwrap_err();
        assert_eq!(e, crate::model::TAP_ON_CNC);
        let mut manual = Machine::new(3);
        crate::manual::set_kind(&mut manual, "Manual");
        let slow = |o: &mut Operation| {
            o.p.insert("rpm".into(), "90".into());
            o.p.insert("feed".into(), "90".into());
        };
        let sim = threaded_job(
            manual.clone(),
            "4.9",
            Some("5"),
            tap.clone(),
            crate::model::TAPPING,
            slow,
        )
        .0
        .unwrap();
        assert_eq!(sim.plan.milling_direction, "Tapping · by hand");
        let e = threaded_job(
            manual.clone(),
            "4.9",
            None,
            tap.clone(),
            crate::model::TAPPING,
            slow,
        )
        .0
        .unwrap_err();
        assert!(e.starts_with("Drill the M6×1 hole"), "{e}");
        // An M5 hole takes an M5 tap.
        let e = threaded_job(manual, "4.1", Some("4.2"), tap, crate::model::TAPPING, slow)
            .0
            .unwrap_err();
        assert!(e.contains("takes an M5×0.8 tap"), "{e}");
    }
    #[test]
    fn a_countersink_is_found_with_its_angle_and_depth() {
        // Ø8 at the face down to the Ø4 bore: 82° and 90° cones.
        for angle in [82f32, 90.] {
            let depth = 2. / (angle / 2.).to_radians().tan();
            let h = top_hole(&rimmed(&[(4., 10.), (2., 10. - depth)]));
            let c = h.countersink.unwrap_or_else(|| panic!("{angle}: {h:?}"));
            assert!((c.angle - angle).abs() < 0.05, "{c:?}");
            assert!((c.depth - depth).abs() < 0.01, "{c:?}");
            assert!((h.opening - 8.).abs() < 0.01 && (h.diameter - 4.).abs() < 0.01);
        }
        // Down to a flat floor, then the bore: still a countersink.
        let c = top_hole(&rimmed(&[(4., 10.), (3., 9.), (2., 9.)]))
            .countersink
            .unwrap();
        assert!(
            (c.angle - 90.).abs() < 0.05 && (c.depth - 1.).abs() < 0.01,
            "{c:?}"
        );
        // A plain hole, a counterbore, a rounded edge and a bevel of two
        // slopes have none.
        let round: Vec<(f32, f32)> = (0..=8)
            .map(|i| {
                let a = i as f32 * std::f32::consts::FRAC_PI_2 / 8.;
                (2. + 2. * (1. - a.sin()), 8. + 2. * a.cos())
            })
            .collect();
        for (name, rim) in [
            ("plain", vec![(2., 10.)]),
            ("counterbore", vec![(4., 10.), (4., 7.), (2., 7.)]),
            ("rounded", round),
            ("two slopes", vec![(4., 10.), (3., 9.5), (2., 7.5)]),
        ] {
            let h = top_hole(&rimmed(&rim));
            assert_eq!(h.countersink, None, "{name}: {h:?}");
        }
    }
    #[test]
    fn a_chamfer_mill_countersinks_only_at_its_countersinks_angle() {
        let depth = 2. / 41f32.to_radians().tan();
        let mesh = rimmed(&[(4., 10.), (2., 10. - depth)]);
        let h = top_hole(&mesh);
        let (setup, mut op, mut tool) = fixture(Strategy::Drill);
        select_top(&mut op, &mesh);
        tool.p.insert("type".into(), "Chamfer mill".into());
        tool.p.insert("diameter".into(), "12".into());
        // Of its angle, its cone is the countersink's: its point goes down
        // to the countersink's apex.
        tool.p.insert("tip_angle".into(), "82".into());
        let plan = generate(&setup, &op, &tool, &mesh).unwrap();
        assert!((h.top - plan.target - 4. / 41f32.to_radians().tan()).abs() < 0.005);
        assert_plan(plan);
        // A sharper one would cut into the cone's wall and the bore, a
        // blunter one leave the cone.
        for angle in ["60", "90", "83"] {
            tool.p.insert("tip_angle".into(), angle.into());
            let error = generate(&setup, &op, &tool, &mesh).unwrap_err();
            assert!(
                error.contains("countersink of 82°") && error.contains("chamfer mill of 82°."),
                "{angle}: {error}"
            );
        }
        // A drill drills it as it did.
        let (_, _, drill) = fixture(Strategy::Drill);
        assert!(generate(&setup, &op, &drill, &mesh).is_ok());
    }
    #[test]
    fn the_totem_cases_screw_holes_are_82_degree_countersinks_cut_by_an_82_degree_chamfer_mill() {
        if crate::step::skip_without_helper() {
            return;
        }
        let mesh = mesh::load(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("examples/totem-case-top-right.step"),
        )
        .unwrap();
        // Its four screw holes in the top: Ø4.3 at the face, 82° down to
        // the Ø2 bore 1.32 mm under it (the STEP's cones).
        let (setup, mut op, mut tool) = fixture(Strategy::Drill);
        select_top(&mut op, &mesh);
        let found = holes(&setup, &op, &mesh).unwrap();
        assert_eq!(found.len(), 4);
        for h in &found {
            let c = h.countersink.unwrap_or_else(|| panic!("{h:?}"));
            assert!((c.angle - 82.).abs() < 0.05, "{c:?}");
            assert!((c.depth - 1.323).abs() < 0.01, "{c:?}");
            assert!((h.opening - 4.3).abs() < 0.01 && (h.diameter - 2.).abs() < 0.01);
        }
        // Picked by a click on their walls, they read the same: the opening
        // at the face round the countersink, and the countersink.
        let part = crate::pocket::part_holes(&setup, &mesh);
        let clicked: Vec<Hole> = drill_holes_of(&setup, &mesh, &part)
            .into_iter()
            .flatten()
            .collect();
        for h in &found {
            let c = (clicked.iter())
                .find(|c| same_centre(c, h))
                .expect("picked by a click");
            assert_eq!(c.countersink, h.countersink, "{c:?} {h:?}");
            assert!((c.opening - h.opening).abs() < 0.01 && (c.top - h.top).abs() < 0.01);
        }
        tool.p.insert("type".into(), "Chamfer mill".into());
        tool.p.insert("diameter".into(), "6".into());
        tool.p.insert("tip_angle".into(), "90".into());
        assert!(
            generate(&setup, &op, &tool, &mesh)
                .unwrap_err()
                .contains("Choose a chamfer mill of 82°.")
        );
        tool.p.insert("tip_angle".into(), "82".into());
        let plan = generate(&setup, &op, &tool, &mesh).unwrap();
        // Each countersink is cut to its size, and the cone stays off the
        // part: every corner of the part within its reach is on or outside
        // it, less the chords' sagitta.
        let g = Geometry::new(&setup, &mesh);
        let tan = 41f32.to_radians().tan();
        for h in &found {
            let tip = (plan.moves.iter())
                .filter(|m| {
                    m.cutting_pass && (m.to[0] - h.center[0]).hypot(m.to[1] - h.center[1]) < 0.001
                })
                .map(|m| m.to[2])
                .fold(f32::INFINITY, f32::min);
            assert!((h.top - tip - 2.15 / tan).abs() < 0.005, "{h:?}: {tip}");
            for p in g.triangles.iter().flat_map(|t| t.p.iter()) {
                let out = (p[0] - h.center[0]).hypot(p[1] - h.center[1]);
                if p[2] > tip && p[2] < h.top + 0.001 && out < 4. {
                    assert!(
                        out >= (p[2] - tip) * tan - h.sagitta - 0.005,
                        "{p:?} of {h:?}"
                    );
                }
            }
        }
    }
}
