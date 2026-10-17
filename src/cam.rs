//! Imported-mesh 3-axis preview kernels. Coordinates are millimetres in setup space.
//! These bounded previews are not a controller postprocessor or a collision solver.
use crate::{
    mesh::{Bounds, Mesh},
    model::{Operation, Setup, Strategy, Tool, facing_setting, get, num, pocket_setting},
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
    for level in 1..=levels {
        for path in paths.iter().filter(|p| area(p) > 0.) {
            let mut points: Vec<_> = path
                .iter()
                .map(|p| {
                    [
                        p.0 as f32,
                        p.1 as f32,
                        (cutting_top - level as f32 * down).max(target),
                    ]
                })
                .collect();
            // Solid is inside the outer profile: CW spindle climb travels CW.
            if climb == b.plan.spindle_clockwise {
                points.reverse();
            }
            if let Some(&first) = points.first() {
                points.push(first);
            }
            for _ in 0..=if level == levels { finish as usize } else { 0 } {
                b.stroke(&points)?;
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
            .clamp(1., LIMIT as f32) as usize;
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
    let top = selected
        .first()
        .ok_or("Select the horizontal face containing the hole openings.")?
        .p[0][2];
    let loops: Vec<Polygon> = g.silhouette(true, op)?.into();
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
    if holes.is_empty() {
        Err("No unobstructed circular holes were found in the selected face. Select the top rim containing hole openings.".into())
    } else {
        Ok(holes)
    }
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
fn drill(setup: &Setup, op: &Operation, tool: &Tool, mesh: &Mesh) -> Result<Plan, String> {
    let mut b = Builder::new(setup, op, tool, mesh)?;
    if get(&tool.p, "type") != "Drill" {
        return Err("Choose a drill for a drilling cycle.".into());
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
    let through = get(&op.p, "through") == "Yes";
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
    // that a blind hole's drill-point floor is left as it is (#155).
    b.plan.shape = CutterShape::Drill;
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
        if b.plan.radius * 2. > h.diameter + 2. * h.sagitta + 0.01 {
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
        let target = h.bottom + number(op, "bottom")? - breakthrough;
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
    fn all_twelve_strategies_have_an_explicit_executable_or_unavailable_contract() {
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
            7
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
}
