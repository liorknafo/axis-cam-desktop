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
pub(crate) type Polygon = Vec<(f64, f64)>;
pub(crate) type Region = Paths<Milli>;
pub(crate) const LIMIT: usize = 30_000;
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
        }
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
    fn silhouette(&self, selected: bool, op: &Operation) -> Result<Region, String> {
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
                display_scale: stock.display_scale,
                duration: 0.,
                target: top,
                levels: 0,
                rest_top: None,
                spindle_clockwise,
                milling_direction: "Mixed · preview",
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
        })
    }
    fn push(&mut self, to: [f32; 3], motion: Motion, cut: bool, rate: f32) -> Result<(), String> {
        if self.plan.moves.len() >= LIMIT {
            return Err("Preview exceeds 30,000 moves. Increase spacing or stepdown.".into());
        }
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
            });
            self.plan.duration = end;
        }
        self.pos = to;
        Ok(())
    }
    fn retract(&mut self) -> Result<(), String> {
        self.push(
            [self.pos[0], self.pos[1], self.clearance],
            Motion::Rapid,
            false,
            6000.,
        )
    }
    fn start(&mut self, p: [f32; 3]) -> Result<(), String> {
        self.start_at(p, self.plan.rest_top)
    }
    fn start_at(&mut self, p: [f32; 3], top: Option<f32>) -> Result<(), String> {
        self.retract()?;
        self.push([p[0], p[1], self.clearance], Motion::Rapid, false, 6000.)?;
        let approach = top.map_or(self.retract, |z| {
            (z + self.retract_margin).min(self.retract)
        });
        self.push([p[0], p[1], approach], Motion::Rapid, false, 6000.)?;
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
    match op.strategy {
        Strategy::Contour => contour(setup, op, tool, mesh),
        Strategy::Drill => drill(setup, op, tool, mesh),
        Strategy::Parallel => parallel(setup, op, tool, mesh),
        Strategy::Waterline => waterline(setup, op, tool, mesh),
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
    if get(&op.p, "boundary") != "Model silhouette" || get(&op.p, "side") != "Outside" {
        return Err("Choose Model silhouette / Outside, or Selected geometry / Inside and select a recessed planar floor.".into());
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
    if target < b.plan.stock.min[2] || target >= top || top - target >= b.plan.tool_length {
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
    let mut envelope = g.bounds;
    for k in 0..2 {
        envelope.min[k] -= 2. * b.plan.radius + radial + (b.plan.radius * 0.01).max(0.02);
        envelope.max[k] += 2. * b.plan.radius + radial + (b.plan.radius * 0.01).max(0.02);
    }
    let cutting_top = crate::rest::start(setup, envelope, target, top, number(op, "top")?);
    b.plan.rest_top = (cutting_top < top - 0.0001).then_some(cutting_top);
    let levels = ((cutting_top - target - 0.0001) / down).ceil().max(1.) as usize;
    if levels > 1000 {
        return Err("Too many contour levels.".into());
    }
    let climb = pocket_setting(&op.p, "pocket_milling") == "Climb";
    if !crate::model::FACING_MILLING.contains(&pocket_setting(&op.p, "pocket_milling")) {
        return Err("Choose Climb or Conventional.".into());
    }
    b.plan.milling_direction = if climb { "Climb" } else { "Conventional" };
    b.plan.target = target;
    b.plan.levels = levels;
    let tabs = if pocket_setting(&op.p, "tabs") == CONTOUR_TABS[1] {
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
            // The tool's centre rises over the tab and its own diameter.
            lift: width + 2. * b.plan.radius,
            top: tab_top,
        })
    } else {
        None
    };
    let loops: Vec<Vec<[f32; 2]>> = paths
        .iter()
        .filter(|p| area(p) > 0.)
        .map(|path| {
            let mut points: Vec<_> = path.iter().map(|p| [p.0 as f32, p.1 as f32]).collect();
            // Solid is inside the outer profile: CW spindle climb travels CW.
            if climb == b.plan.spindle_clockwise {
                points.reverse();
            }
            if let Some(&first) = points.first() {
                points.push(first);
            }
            points
        })
        .collect();
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
            for (start, end) in outline.spans(tabs) {
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
        }
        if through && loose > 0 {
            return Err(format!(
                "{loose} of {all} tabs join the part to no stock: the cut runs past the stock's edge there, so the part still comes loose. Leave at least {:.1} mm of stock beside the part (stock to leave, the tool's diameter and a {WALL} mm wall), or use a narrower tool.",
                // Rounded up, so that the margin it names is enough.
                ((edge + WALL) * 10.).ceil() / 10.
            ));
        }
    }
    for level in 1..=levels {
        let z = (cutting_top - level as f32 * down).max(target);
        for points in &loops {
            let points = match &tabs {
                Some(tabs) if z < tabs.top => tabbed(points, z, tabs),
                _ => points.iter().map(|p| [p[0], p[1], z]).collect(),
            };
            for _ in 0..=if level == levels { finish as usize } else { 0 } {
                b.stroke_plunging(&points)?;
            }
        }
    }
    b.finish()
}
/// The stock, mm, a tab must reach past the cut's outer edge.
const WALL: f32 = 2.;
/// An outside Contour's holding tabs: `count` on each outline, evenly
/// spaced, over each of which the tool's centre rises to `top` along
/// `lift` mm of its path.
struct Tabs {
    count: usize,
    lift: f32,
    top: f32,
}
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
    /// Where each of `tabs` starts and ends along the path. Their centres
    /// are spaced evenly, half a spacing from the start, so that none spans
    /// the start while they fit (`lift` × `count` under the path's length).
    fn spans(&self, tabs: &Tabs) -> Vec<(f32, f32)> {
        let spacing = self.length() / tabs.count as f32;
        (0..tabs.count)
            .map(|i| {
                let centre = (i as f32 + 0.5) * spacing;
                (centre - tabs.lift / 2., centre + tabs.lift / 2.)
            })
            .collect()
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
fn tabbed(points: &[[f32; 2]], z: f32, tabs: &Tabs) -> Vec<[f32; 3]> {
    let outline = Outline::new(points);
    let along = &outline.along;
    // Along the path: each vertex (None), each tab's start (Some(true))
    // and its end (Some(false)).
    let mut events: Vec<(f32, Option<bool>, usize)> = along
        .iter()
        .enumerate()
        .map(|(i, &s)| (s, None, i))
        .collect();
    for (start, end) in outline.spans(tabs) {
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
    pub diameter: f32,
    pub opening: f32,
    /// How far the opening's chords lie inside its circle, R(1 − cos(π/n))
    /// for its n vertices at radius R. A drill fits up to `diameter` plus
    /// twice this.
    pub sagitta: f32,
    pub top: f32,
    pub bottom: f32,
    /// The bottom is the model's bottom: nothing of the part is under it.
    pub through: bool,
}
/// Circular holes bounded by the selected horizontal rim, verified against mesh rays.
/// Noncircular openings and boss outlines are rejected, never converted to drills.
pub fn holes(setup: &Setup, op: &Operation, mesh: &Mesh) -> Result<Vec<Hole>, String> {
    let g = Geometry::new(setup, mesh);
    // The Drill geometry list and the part report ask without `simulation::generate`'s check.
    if crate::simulation::exceeds_coordinate_limit(&g.bounds) {
        return Err(crate::simulation::COORDINATE_LIMIT_MESSAGE.into());
    }
    let selected = g.selected(op);
    if selected.is_empty() {
        return Err("Select the horizontal face containing the hole openings.".into());
    }
    let holes = round_holes(&g, &selected)?;
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
    let mut holes = vec![];
    for path in loops.iter().filter(|p| area(p) < 0. && p.len() >= 8) {
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
        let center =
            circle_centre(path).unwrap_or(((low[0] + high[0]) / 2., (low[1] + high[1]) / 2.));
        let center = [center.0 as f32, center.1 as f32];
        let distance = |p: &(f64, f64)| {
            ((p.0 as f32 - center[0]).powi(2) + (p.1 as f32 - center[1]).powi(2)).sqrt()
        };
        // The vertex radius: a regular polygon's vertices are all on its circle.
        let radius = path.iter().map(distance).sum::<f32>() / path.len() as f32;
        if radius <= 0.
            || path
                .iter()
                .any(|p| (distance(p) - radius).abs() > 0.03f32.max(radius * 0.02))
        {
            continue;
        }
        // Its chords' midpoints lie this far inside the circle.
        let sagitta = radius * (1. - (std::f32::consts::PI / path.len() as f32).cos());
        let bottom = g.height(center).unwrap_or(g.bounds.min[2]);
        if bottom >= top - 0.01 {
            continue;
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
        let above = |p: &[f32; 3]| {
            p[2] - point * ((p[0] - center[0]).powi(2) + (p[1] - center[1]).powi(2)).sqrt()
        };
        let mut corridor = radius;
        // The nearest vertex of the triangle that sets the corridor: on a
        // bore's wall an end of the chord whose midpoint set it, a sagitta
        // past it; on a ledge or a boss much farther than its nearest point.
        let mut ring = None;
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
            let z = bottom + 0.02;
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
                corridor = 0.;
                break;
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
            if nearest < corridor {
                corridor = nearest;
                ring = Some(
                    triangle
                        .p
                        .iter()
                        .map(|p| ((p[0] - center[0]).powi(2) + (p[1] - center[1]).powi(2)).sqrt())
                        .fold(f32::INFINITY, f32::min),
                );
            }
        }
        if corridor < 0.1 {
            continue;
        }
        // A corridor set by a chord whose ends lie a sagitta past it (of the
        // opening's tessellation or a finer one) is the chord's ring, a circle
        // through those ends: the bore is read there, its nominal size, where
        // the chord's midpoint is the inscribed polygon's.
        let diameter = match ring {
            Some(v) if v >= corridor && v - corridor <= sagitta * 1.1 + 0.001 => v * 2.,
            _ => corridor * 2.,
        };
        holes.push(Hole {
            center,
            diameter,
            opening: radius * 2.,
            sagitta,
            top,
            bottom,
            through,
        });
    }
    holes.sort_by(|a, b| {
        a.center[0]
            .total_cmp(&b.center[0])
            .then(a.center[1].total_cmp(&b.center[1]))
    });
    Ok(holes)
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
fn drill(setup: &Setup, op: &Operation, tool: &Tool, mesh: &Mesh) -> Result<Plan, String> {
    let mut b = Builder::new(setup, op, tool, mesh)?;
    // A chamfer mill countersinks each opening rather than drilling it.
    let chamfer = get(&tool.p, "type") == "Chamfer mill";
    if get(&tool.p, "type") != "Drill" && !chamfer {
        return Err(
            "Choose a drill, or a chamfer mill to countersink, for a drilling cycle.".into(),
        );
    }
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
    if holes.is_empty() {
        return Err("No detected holes match the diameter filter.".into());
    }
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
    if !["Yes", "No"].contains(&get(&op.p, "through")) {
        return Err("Choose Yes or No for breakthrough.".into());
    }
    // Breakthrough is travel past the exit of a hole that goes through the
    // part. Under a blind hole's bottom is the part, so such a hole is
    // refused rather than drilled into, as the planner's plans are.
    let through = get(&op.p, "through") == "Yes" && !chamfer;
    let breakthrough = if through {
        number(op, "breakthrough")?
    } else {
        0.
    };
    if breakthrough < 0. {
        return Err("Breakthrough cannot be negative.".into());
    }
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
    if !["Optimize travel", "By X then Y"].contains(&get(&op.p, "hole_order")) {
        return Err("Choose a valid hole order.".into());
    }
    while !holes.is_empty() {
        let index = if get(&op.p, "hole_order") == "Optimize travel" {
            holes
                .iter()
                .enumerate()
                .min_by(|(_, a), (_, c)| {
                    let distance = |h: &Hole| {
                        (h.center[0] - b.pos[0]).powi(2) + (h.center[1] - b.pos[1]).powi(2)
                    };
                    distance(a).total_cmp(&distance(c))
                })
                .unwrap()
                .0
        } else {
            0
        };
        let h = holes.remove(index);
        // The drill may be as wide as the bore at its tessellation's vertices,
        // which its chords lie a sagitta inside, and 0.01 over that: the part
        // report rounds the bore to 0.01 and the planner's drill is that size.
        if chamfer && b.plan.radius * 2. < h.opening - 2. * h.sagitta - 0.01 {
            return Err(format!(
                "The Ø{:.2} chamfer mill is narrower than an opening Ø{:.2} it would countersink. Choose a wider one or a diameter filter.",
                b.plan.radius * 2.,
                h.opening
            ));
        }
        if !chamfer && b.plan.radius * 2. > h.diameter + 2. * h.sagitta + 0.01 {
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
        let envelope = Bounds {
            min: [
                h.center[0] - b.plan.radius,
                h.center[1] - b.plan.radius,
                target,
            ],
            max: [
                h.center[0] + b.plan.radius,
                h.center[1] + b.plan.radius,
                top,
            ],
        };
        let cutting_top = crate::rest::start(setup, envelope, target, top, number(op, "top")?);
        let lower = (cutting_top < top - 0.0001).then_some(cutting_top);
        b.plan.rest_top = if b.plan.levels == 0 {
            lower
        } else {
            b.plan.rest_top.zip(lower).map(|(a, b)| a.max(b))
        };
        let levels = ((cutting_top - target) / peck).ceil().max(1.) as usize;
        if levels > 1000 {
            return Err("Peck cycle exceeds 1,000 steps per hole.".into());
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
                    6000.,
                )?;
                b.push(
                    [h.center[0], h.center[1], (depth + 0.1).min(top)],
                    Motion::Rapid,
                    false,
                    6000.,
                )?;
            }
        }
        if cycle == "Counterboring" {
            let dwell = number(op, "dwell")?;
            if !(0. ..=600.).contains(&dwell) {
                return Err("Dwell must be 0–600 seconds.".into());
            }
            if dwell > 0. {
                let end = b.plan.duration + dwell;
                b.plan.moves.push(Move {
                    from: b.pos,
                    to: b.pos,
                    motion: Motion::Dwell,
                    cutting_pass: false,
                    feed: 0.,
                    start: b.plan.duration,
                    end,
                });
                b.plan.duration = end;
            }
        }
        b.retract()?;
    }
    b.finish()
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
struct SurfaceGrid {
    nx: usize,
    ny: usize,
    step: f32,
    origin: [f32; 2],
    z: Vec<Option<f32>>,
}
fn finish_grid(g: &Geometry, op: &Operation, b: &Builder) -> Result<SurfaceGrid, String> {
    let spacing = number(op, "surface_stepover")?;
    let tolerance = number(op, "tolerance")?;
    let offset = number(op, "allowance")? + number(op, "bottom")?;
    if spacing <= 0. || spacing > b.plan.radius || tolerance <= 0. || tolerance > 1. || offset < 0.
    {
        return Err("Use surface spacing up to cutter radius, tolerance 0–1 mm and nonnegative finishing offset.".into());
    }
    if get(&op.p, "boundary") != "Model silhouette" {
        return Err("Finishing preview currently uses the whole Model silhouette.".into());
    }
    let step = spacing.min((8. * b.plan.radius * tolerance).sqrt()).min(1.);
    if step < 0.05 {
        return Err("Requested finishing sampling is below the 0.05 mm preview limit.".into());
    }
    let dims = g.bounds.dimensions();
    if dims[..2]
        .iter()
        .any(|d| !d.is_finite() || d / step > 500_000.)
    {
        return Err("Finishing extent exceeds the preview sample limit.".into());
    }
    let nx = (dims[0] / step).ceil() as usize + 1;
    let ny = (dims[1] / step).ceil() as usize + 1;
    if nx.saturating_mul(ny) > 500_000 {
        return Err("Finishing grid exceeds 500,000 samples. Increase spacing/tolerance or reduce model size.".into());
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
    let mut work = 0usize;
    for t in &g.triangles {
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
        work = work.saturating_add((ranges[0].1 - ranges[0].0) * (ranges[1].1 - ranges[1].0));
        if work > 60_000_000 {
            return Err("Finishing mesh is too dense for this bounded preview. Increase spacing or simplify the mesh.".into());
        }
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
        for y in ranges[1].0..ranges[1].1 {
            for x in ranges[0].0..ranges[0].1 {
                let xy = [
                    g.bounds.min[0] + x as f32 * step,
                    g.bounds.min[1] + y as f32 * step,
                ];
                let i = y * nx + x;
                if let Some(height) = drop_triangle(&t.p, xy, r, b.plan.shape == CutterShape::Ball)
                    && z[i].is_none_or(|v| height > v)
                {
                    z[i] = Some(height);
                    selected[i] = slope >= low - 0.001 && slope <= high + 0.001;
                }
            }
        }
    }
    // Only cutter centers over the model XY silhouette. This also avoids joining disconnected islands.
    let silhouette: Vec<Polygon> = g.silhouette(false, op)?.into();
    for y in 0..ny {
        for x in 0..nx {
            let i = y * nx + x;
            let xy = (
                g.bounds.min[0] as f64 + x as f64 * step as f64,
                g.bounds.min[1] as f64 + y as f64 * step as f64,
            );
            if !selected[i] || !contains_or_edge(&silhouette, xy) {
                z[i] = None;
            } else {
                z[i] = z[i].map(|h| h + offset + tolerance);
            }
        }
    }
    Ok(SurfaceGrid {
        nx,
        ny,
        step,
        origin: [g.bounds.min[0], g.bounds.min[1]],
        z,
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
        if x + 1 >= self.nx || y + 1 >= self.ny {
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
    let grid = finish_grid(&g, op, &b)?;
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
    if rows > LIMIT || count > LIMIT || rows.saturating_mul(count) > LIMIT {
        return Err(
            "Parallel preview exceeds 30,000 samples. Increase surface spacing/tolerance.".into(),
        );
    }
    b.plan.levels = rows;
    b.plan.target = g.bounds.min[2];
    for row in 0..=rows {
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
    b.finish()
}
fn waterline(setup: &Setup, op: &Operation, tool: &Tool, mesh: &Mesh) -> Result<Plan, String> {
    let mut b = Builder::new(setup, op, tool, mesh)?;
    if get(&tool.p, "type") != "Flat end mill" {
        return Err("Waterline preview currently requires a flat end mill.".into());
    }
    if get(&op.p, "boundary") != "Model silhouette" {
        return Err("Waterline preview uses Model silhouette.".into());
    }
    let g = Geometry::new(setup, mesh);
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
    let mut envelope = g.bounds;
    for k in 0..2 {
        envelope.min[k] -= 2. * b.plan.radius + allowance + (b.plan.radius * 0.01).max(0.02);
        envelope.max[k] += 2. * b.plan.radius + allowance + (b.plan.radius * 0.01).max(0.02);
    }
    let cutting_top = crate::rest::start(setup, envelope, target, top, number(op, "top")?);
    b.plan.rest_top = (cutting_top < top - 0.0001).then_some(cutting_top);
    let levels = ((cutting_top - target) / down).ceil().max(1.) as usize;
    if levels > 300 {
        return Err("Waterline preview exceeds 300 layers. Increase stepdown.".into());
    }
    b.plan.levels = levels;
    b.plan.target = target;
    let climb = pocket_setting(&op.p, "pocket_milling") == "Climb";
    if !crate::model::FACING_MILLING.contains(&pocket_setting(&op.p, "pocket_milling")) {
        return Err("Choose Climb or Conventional.".into());
    }
    b.plan.milling_direction = if climb { "Climb" } else { "Conventional" };
    for level in 1..=levels {
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
            points.push(points[0]);
            b.stroke(&points)?;
        }
    }
    b.finish()
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
    fn all_eleven_strategies_have_an_explicit_executable_or_unavailable_contract() {
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
            6
        );
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
}
