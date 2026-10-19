//! Bounded 3-axis Facing preview and sampled, top-down stock removal.
//! This is not a postprocessor, collision checker or general CAM kernel.
use crate::{
    mesh::{Bounds, Mesh},
    model::{Operation, Setup, Strategy, Tool, facing_setting, facing_setting_issues, get, num},
    stock::{self, Placement},
    surface,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Motion {
    Rapid,
    Feed,
    Dwell,
}
#[derive(Debug, Clone)]
pub struct Move {
    pub from: [f32; 3],
    pub to: [f32; 3],
    pub motion: Motion,
    pub cutting_pass: bool,
    /// Programmed rate (mm/min) that timed this move; 0 for dwells.
    pub feed: f32,
    pub start: f32,
    pub end: f32,
}
impl Move {
    pub fn at(&self, seconds: f32) -> [f32; 3] {
        if seconds <= self.start {
            return self.from;
        }
        if seconds >= self.end {
            return self.to;
        }
        let t = ((seconds - self.start) / (self.end - self.start)).clamp(0., 1.);
        std::array::from_fn(|i| self.from[i] + (self.to[i] - self.from[i]) * t)
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CutterShape {
    Flat,
    Ball,
    /// A drill: flat flutes over a 118° point, the cone `POINT_RISE` gives,
    /// with the programmed tip at its apex.
    Drill,
    /// A chamfer mill: a cone of this included angle, in tenths of a degree,
    /// from its pointed tip up to its diameter.
    Chamfer(u16),
    /// A lathe cut: the ring `volume::Volume::revolve` makes, every point of
    /// the cut's frame whose Z lies between `a[0]` and `b[0]` and whose
    /// distance from its Z axis lies between `a[1]` and `b[1]`.
    Revolve,
}
impl CutterShape {
    /// How far a pointed cutter's cone rises per mm from its axis: a drill's
    /// 118° point or a chamfer mill's angle; None for flat and ball ends.
    pub fn rise(self) -> Option<f32> {
        match self {
            CutterShape::Drill => Some(POINT_RISE),
            CutterShape::Chamfer(tenths) => Some(1. / (f32::from(tenths) / 20.).to_radians().tan()),
            CutterShape::Flat | CutterShape::Ball | CutterShape::Revolve => None,
        }
    }
}
/// The rise of a 118° drill point's cone per mm from the drill's axis, tan
/// 31°: what `cam::holes` takes a blind hole's floor to be.
pub const POINT_RISE: f32 = 0.600_860_6;
#[derive(Debug, Clone)]
pub struct Plan {
    pub shape: CutterShape,
    pub strategy: Strategy,
    pub operation: String,
    pub setup: String,
    pub moves: Vec<Move>,
    pub stock: Bounds,
    pub radius: f32,
    pub flute_length: f32,
    pub tool_length: f32,
    pub holder_radius: f32,
    pub display_scale: f32,
    pub duration: f32,
    pub target: f32,
    pub levels: usize,
    pub rest_top: Option<f32>,
    pub spindle_clockwise: bool,
    pub milling_direction: &'static str,
    /// A lathe plan's cuts (`turn::generate`), in the order they are made:
    /// the simulation turns each ring away when its move ends, instead of
    /// sweeping the cutter along the moves. Empty for every other plan.
    pub rings: Vec<Ring>,
}
impl Plan {
    /// Whether this is a lathe plan: its moves are a side view (X the
    /// tool's distance from the spindle axis, Y 0, Z along the axis) and its
    /// cuts are its `rings`.
    pub fn is_lathe(&self) -> bool {
        self.shape == CutterShape::Revolve
    }
}
/// A lathe cut: the ring of the plan's setup frame between `z[0]` and `z[1]`
/// along its Z axis and between the radii `r[0]` and `r[1]` from it (mm),
/// turned away when move `at` of the plan ends (`volume::Volume::revolve`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ring {
    pub at: usize,
    pub z: [f32; 2],
    pub r: [f32; 2],
}
fn number(operation: &Operation, key: &str) -> Result<f32, String> {
    num(&operation.p, key).ok_or_else(|| {
        format!(
            "Enter a finite {} value before generating the preview.",
            key.replace('_', " ")
        )
    })
}
/// The refusal for stock, part or tool coordinates the preview polygon
/// kernel's integer range cannot hold.
pub const COORDINATE_LIMIT_MESSAGE: &str =
    "Geometry/tool extent exceeds the ±1,000,000 mm preview coordinate limit.";
/// Whether a corner of `bounds` lies beyond ±1,000,000 mm.
pub fn exceeds_coordinate_limit(bounds: &Bounds) -> bool {
    bounds
        .min
        .iter()
        .chain(&bounds.max)
        .any(|v| v.abs() > 1_000_000.)
}
pub fn generate(
    setup: &Setup,
    operation: &Operation,
    tool: &Tool,
    mesh: &Mesh,
) -> Result<Plan, String> {
    // Turning and a lathe's centre drilling are planned in their own frame.
    if operation.strategy == Strategy::Turn || operation.on_centre(&setup.machine) {
        return crate::turn::generate(setup, operation, tool, mesh);
    }
    // Keep coordinates within the preview polygon kernel's integer range.
    // This applies to every strategy before its geometry-specific calculations.
    // A fixed-size box stays at the origin while the part follows the setup's
    // translation, so the placed part is checked as well as the stock.
    if operation.strategy.imported_preview() {
        let stock = stock::resolve(setup, Some(mesh))?;
        if exceeds_coordinate_limit(&stock.stock)
            || stock
                .part
                .is_some_and(|part| exceeds_coordinate_limit(&part))
            || ["diameter", "length", "stickout", "holder"]
                .iter()
                .any(|k| num(&tool.p, k).is_some_and(|v| v > 1_000_000.))
            || ["allowance", "axial_allowance"]
                .iter()
                .any(|k| num(&operation.p, k).is_some_and(|v| v.abs() > 1_000_000.))
        {
            return Err(COORDINATE_LIMIT_MESSAGE.into());
        }
    }
    if operation.strategy == Strategy::Pocket {
        return crate::pocket::generate(setup, operation, tool, mesh);
    }
    if operation.strategy == Strategy::Adaptive {
        return crate::adaptive::generate(setup, operation, tool, mesh);
    }
    if operation.strategy != Strategy::Face {
        return crate::cam::generate(setup, operation, tool, mesh);
    }
    if !operation.enabled {
        return Err("Enable this operation before simulating it.".into());
    }
    if let Some(issue) = facing_setting_issues(&operation.p).first() {
        return Err(issue.clone());
    }
    let both_ways = facing_setting(&operation.p, "facing_pattern") == "Both ways (zigzag)";
    let spindle_clockwise = facing_setting(&operation.p, "spindle_rotation") == "Clockwise (CW)";
    let climb = facing_setting(&operation.p, "facing_milling") == "Climb";
    let continue_depth = both_ways
        && facing_setting(&operation.p, "facing_depth_order") == "Continue from previous end";
    let keep_down = both_ways && facing_setting(&operation.p, "facing_link") == "Keep tool down";
    if let Some(issue) = surface::issues(operation, setup, Some(mesh)).first() {
        return Err(issue.clone());
    }
    let selection = operation
        .surfaces
        .as_ref()
        .filter(|s| !s.ids.is_empty())
        .ok_or("Select a top planar surface in Geometry first.")?;
    if !["Flat end mill", "Face mill"].contains(&get(&tool.p, "type")) {
        return Err("Facing simulation currently requires a flat end mill or face mill.".into());
    }
    if get(&setup.p, "stock_source") != "fresh" {
        return Err("Facing simulation currently needs fresh stock. Transfer of remaining stock between setups is not implemented.".into());
    }
    let stock = stock::resolve(setup, Some(mesh))?;
    let scale = stock.display_scale;
    let placement = Placement::new(mesh, Some(setup));
    let reference = mesh
        .faces
        .iter()
        .find(|f| f.feature == Some(selection.ids[0]))
        .and_then(|f| f.points.first())
        .ok_or("The selected surface has no triangles.")?;
    let plane = placement.transform(*reference)[2] / scale;
    let target = plane + number(operation, "bottom")?;
    if target < placement.bounds.max[2] / scale - 0.001 {
        return Err("Facing preview supports the highest plane of the placed part. This selection or bottom offset would cut below other model features; reposition the part or choose its top surface.".into());
    }
    if target < stock.stock.min[2] || target >= stock.stock.max[2] - 0.0001 {
        return Err("No stock exists above the facing target. Increase the setup's top stock offset or review the selected plane and bottom offset.".into());
    }
    let radius = num(&tool.p, "diameter")
        .filter(|v| *v > 0.)
        .ok_or("Enter a positive tool diameter.")?
        / 2.;
    let flute_length = num(&tool.p, "length")
        .filter(|v| *v > 0.)
        .ok_or("Enter a positive flute length.")?;
    let tool_length = num(&tool.p, "stickout")
        .filter(|v| *v >= flute_length)
        .ok_or("Tool stickout must cover its flute length.")?;
    let holder_radius = num(&tool.p, "holder")
        .filter(|v| *v > 0.)
        .ok_or("Enter a positive holder diameter.")?
        / 2.;
    let stepdown = number(operation, "stepdown")?;
    let stepover = number(operation, "stepover")? / 100. * radius * 2.;
    let feed = number(operation, "feed")?;
    if stepdown <= 0.
        || stepdown > flute_length
        || stepover <= 0.
        || stepover > radius * 2.
        || feed <= 0.
    {
        return Err("Use positive feed/step sizes, stepdown within flute length, and stepover up to 100% of the cutter diameter.".into());
    }
    let allowance = number(operation, "allowance")?;
    if allowance < 0. {
        return Err("Radial stock to leave cannot be negative.".into());
    }
    let top = stock.stock.max[2].max(plane + number(operation, "top")?);
    let clearance = top + number(operation, "clearance")?;
    let retract = top + number(operation, "retract")?;
    if retract <= top || clearance < retract {
        return Err(
            "Set a positive retract height and clearance at or above retract before simulation."
                .into(),
        );
    }
    if top - target >= tool_length {
        return Err("Facing depth from the cutting start to the target must be less than tool stickout. Choose a tool with adequate reach or reduce the stock above the target.".into());
    }
    if operation.faces_outline() {
        let extend: f32 = operation
            .outline_extend()
            .parse()
            .ok()
            .filter(|e: &f32| e.is_finite() && *e >= 0.)
            .ok_or("Clear past outline cannot be negative or blank.")?;
        // The outline's outer loops: the stock over a hole through the part
        // is above its top too.
        let loops: Vec<crate::pocket::Polygon> = crate::cam::Geometry::new(setup, mesh)
            .silhouette(false, operation)?
            .into();
        let silhouette = crate::cam::region(
            loops
                .into_iter()
                .filter(|p| crate::cam::area(p) > 0.)
                .collect(),
        )?;
        return crate::facing::outline(crate::facing::Outline {
            setup,
            op: operation,
            silhouette,
            extend: extend as f64,
            stock: stock.stock,
            scale,
            radius,
            flute_length,
            tool_length,
            holder_radius,
            feed,
            stepdown,
            stepover,
            top,
            clearance,
            retract,
            target,
            direction: number(operation, "direction")?,
            both_ways,
            keep_down,
            climb,
            spindle_clockwise,
        });
    }
    let mut area = match get(&operation.p, "boundary") {
        "Stock silhouette" => stock.stock,
        "Model silhouette" => placement.bounds.scaled(1. / scale),
        "Selected geometry" => {
            let mut b = Bounds {
                min: [f32::INFINITY; 3],
                max: [f32::NEG_INFINITY; 3],
            };
            for face in mesh
                .faces
                .iter()
                .filter(|f| f.feature.is_some_and(|id| selection.ids.contains(&id)))
            {
                for point in &face.points {
                    let p = placement.transform(*point).map(|v| v / scale);
                    for (k, &v) in p.iter().enumerate() {
                        b.min[k] = b.min[k].min(v);
                        b.max[k] = b.max[k].max(v);
                    }
                }
            }
            b
        }
        _ => return Err("Choose a supported machining boundary.".into()),
    };
    // The preview's supported boundary is explicitly the rectangular extent.
    let inset = allowance
        + match get(&operation.p, "containment") {
            "Tool inside boundary" => radius,
            "Tool center on boundary" => 0.,
            "Tool outside boundary" => -radius,
            _ => return Err("Choose a tool containment mode.".into()),
        };
    for k in 0..2 {
        area.min[k] += inset;
        area.max[k] -= inset;
    }
    if (0..2).any(|k| {
        !area.min[k].is_finite() || !area.max[k].is_finite() || area.max[k] - area.min[k] < 0.001
    }) {
        return Err("The cutter and radial allowance do not fit inside this boundary. Choose a smaller cutter or review containment.".into());
    }
    if (0..2).any(|k| {
        area.max[k] + radius < stock.stock.min[k] || area.min[k] - radius > stock.stock.max[k]
    }) {
        return Err("The selected boundary does not intersect the stock.".into());
    }
    let mut envelope = area;
    for k in 0..2 {
        envelope.min[k] -= radius;
        envelope.max[k] += radius;
    }
    let cutting_top = crate::rest::start(setup, envelope, target, top, number(operation, "top")?);
    let approach = (cutting_top + number(operation, "retract")?).min(retract);
    // Avoid a duplicate final level from sub-micron normalization roundoff.
    let levels = ((cutting_top - target - 0.0001) / stepdown).ceil().max(1.);
    let (sin, cos) = number(operation, "direction")?.to_radians().sin_cos();
    let direction = [cos, sin];
    let across = [-sin, cos];
    let corners = [
        [area.min[0], area.min[1]],
        [area.max[0], area.min[1]],
        [area.max[0], area.max[1]],
        [area.min[0], area.max[1]],
    ];
    let v: Vec<_> = corners
        .iter()
        .map(|p| p[0] * across[0] + p[1] * across[1])
        .collect();
    let low = v.iter().copied().fold(f32::INFINITY, f32::min);
    let high = v.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let rows = ((high - low) / stepover).ceil().max(1.);
    if !levels.is_finite() || !rows.is_finite() || levels * (rows + 1.) > crate::cam::CEILING as f32
    {
        return Err(crate::cam::too_many("moves"));
    }
    // Build one complete layer once. Reversing its order and endpoints makes
    // the next layer start exactly where the previous layer finished, including
    // angled scans whose corner rows were skipped.
    let mut strokes = Vec::new();
    for row in 0..=rows as usize {
        let cross = low + (high - low) * row as f32 / rows;
        let mut first = f32::NEG_INFINITY;
        let mut last = f32::INFINITY;
        for k in 0..2 {
            let base = across[k] * cross;
            if direction[k].abs() > 1e-6 {
                let a = (area.min[k] - base) / direction[k];
                let b = (area.max[k] - base) / direction[k];
                first = first.max(a.min(b));
                last = last.min(a.max(b));
            } else if base < area.min[k] - 0.001 || base > area.max[k] + 0.001 {
                last = f32::NEG_INFINITY;
            }
        }
        if last - first < 0.0001 {
            continue;
        }
        let point = |u: f32| {
            [
                direction[0] * u + across[0] * cross,
                direction[1] * u + across[1] * cross,
            ]
        };
        // Rows step toward +across, so the nominal uncut side is left of
        // +direction. Viewed from spindle toward stock, CW climb keeps that
        // stock on the right of cutter travel: reverse the cutting stroke.
        // Reverse rotation reverses this relationship. Zigzag stays mixed;
        // the spindle never reverses between rows. Entry/full-width engagement
        // can include both modes, even with a one-way preference.
        let reverse_cut = if both_ways {
            row % 2 != 0
        } else {
            climb == spindle_clockwise
        };
        let (a, b) = if reverse_cut {
            (last, first)
        } else {
            (first, last)
        };
        strokes.push((point(a), point(b)));
    }
    if strokes.is_empty() {
        return Err("No facing strokes fit this boundary.".into());
    }
    let mut plan = Plan {
        shape: CutterShape::Flat,
        strategy: Strategy::Face,
        operation: operation.id.clone(),
        setup: setup.id.clone(),
        moves: vec![],
        stock: stock.stock,
        radius,
        flute_length,
        tool_length,
        holder_radius,
        display_scale: scale,
        duration: 0.,
        target,
        levels: levels as usize,
        rest_top: (cutting_top < top - 0.0001).then_some(cutting_top),
        spindle_clockwise,
        milling_direction: if both_ways {
            "Mixed · zigzag"
        } else if climb {
            "Climb"
        } else {
            "Conventional"
        },
        rings: vec![],
    };
    let mut position = [area.min[0], area.min[1], clearance];
    let mut done = 0;
    crate::progress::expect(levels as usize * strokes.len());
    let mut push = |to: [f32; 3], motion: Motion, rate: f32, cutting_pass: bool| {
        let distance = (0..3)
            .map(|i| (position[i] - to[i]).powi(2))
            .sum::<f32>()
            .sqrt();
        if distance > 0.00001 {
            let end = plan.duration + distance / rate * 60.;
            plan.moves.push(Move {
                from: position,
                to,
                motion,
                cutting_pass,
                feed: rate,
                start: plan.duration,
                end,
            });
            plan.duration = end;
        }
        position = to;
    };
    let mut previous: Option<[f32; 3]> = None;
    for level in 1..=levels as usize {
        let z = if level == levels as usize {
            target
        } else {
            (cutting_top - level as f32 * stepdown).max(target)
        };
        let reverse = continue_depth && level % 2 == 0;
        for index in 0..strokes.len() {
            // A stroke at a time: how far, and whether it was cancelled.
            done += 1;
            crate::progress::step(done)?;
            let (a, b) = if reverse {
                let (a, b) = strokes[strokes.len() - 1 - index];
                (b, a)
            } else {
                strokes[index]
            };
            let linked = keep_down && previous.is_some() && (index > 0 || continue_depth);
            if linked {
                // The rectangle is convex; adjacent endpoints stay within its
                // cutter-center boundary. XY linking remains at the old depth,
                // followed by a vertical feed only when entering the next level.
                push(
                    [a[0], a[1], previous.unwrap()[2]],
                    Motion::Feed,
                    feed,
                    false,
                );
            } else {
                if let Some(end) = previous {
                    push([end[0], end[1], clearance], Motion::Rapid, 6000., false);
                }
                push([a[0], a[1], clearance], Motion::Rapid, 6000., false);
                push([a[0], a[1], approach], Motion::Rapid, 6000., false);
            }
            push([a[0], a[1], z], Motion::Feed, feed * 0.25, false);
            push([b[0], b[1], z], Motion::Feed, feed, true);
            previous = Some([b[0], b[1], z]);
        }
    }
    if let Some(end) = previous {
        push([end[0], end[1], clearance], Motion::Rapid, 6000., false);
    }
    if plan.moves.is_empty() || !plan.duration.is_finite() {
        return Err("No finite facing motion could be generated from these settings.".into());
    }
    Ok(plan)
}

#[derive(Debug, Clone)]
pub struct StockGrid {
    pub bounds: Bounds,
    pub nx: usize,
    pub ny: usize,
    pub dx: f32,
    pub dy: f32,
    pub heights: Vec<f32>,
}
impl StockGrid {
    fn new(bounds: Bounds) -> Self {
        let size = bounds.dimensions();
        let cell = size[0].max(size[1]) / 96.;
        let nx = (size[0] / cell).ceil().clamp(1., 96.) as usize;
        let ny = (size[1] / cell).ceil().clamp(1., 96.) as usize;
        Self {
            bounds,
            nx,
            ny,
            dx: size[0] / nx as f32,
            dy: size[1] / ny as f32,
            heights: vec![bounds.max[2]; nx * ny],
        }
    }
    #[cfg(test)]
    fn cut(&mut self, a: [f32; 3], b: [f32; 3], radius: f32) {
        self.cut_shape(a, b, radius, CutterShape::Flat);
    }
    /// The stock `volume` holds, seen from above in the frame `frame` over
    /// `bounds`: each cell as high as the highest voxel left under its centre,
    /// at the bottom where none is. A running setup's starting stock for the
    /// live carve (`run::Carving`).
    pub fn from_volume(
        bounds: Bounds,
        volume: &crate::volume::Volume,
        frame: crate::volume::Frame,
    ) -> Self {
        let mut grid = Self::new(bounds);
        let step = volume
            .cell
            .into_iter()
            .fold(f32::INFINITY, f32::min)
            .max(0.05)
            / 2.;
        let levels = ((bounds.max[2] - bounds.min[2]) / step).ceil().max(1.) as usize;
        for y in 0..grid.ny {
            for x in 0..grid.nx {
                let (px, py) = (
                    bounds.min[0] + (x as f32 + 0.5) * grid.dx,
                    bounds.min[1] + (y as f32 + 0.5) * grid.dy,
                );
                grid.heights[y * grid.nx + x] = (0..levels)
                    .map(|i| bounds.max[2] - (i as f32 + 0.5) * step)
                    .find(|&z| volume.filled([px, py, z], frame))
                    .map_or(bounds.min[2], |z| (z + step / 2.).min(bounds.max[2]));
            }
        }
        grid
    }
    /// Cuts the sweep of a cutter of `radius` and `shape` from `a` to `b`.
    pub(crate) fn cut_shape(&mut self, a: [f32; 3], b: [f32; 3], radius: f32, shape: CutterShape) {
        let ranges: [(usize, usize); 2] = std::array::from_fn(|k| {
            let cell = if k == 0 { self.dx } else { self.dy };
            let count = if k == 0 { self.nx } else { self.ny };
            (
                ((a[k].min(b[k]) - radius - self.bounds.min[k]) / cell)
                    .floor()
                    .clamp(0., count as f32) as usize,
                ((a[k].max(b[k]) + radius - self.bounds.min[k]) / cell)
                    .ceil()
                    .clamp(0., count as f32) as usize,
            )
        });
        let length = (b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2);
        for y in ranges[1].0..ranges[1].1 {
            for x in ranges[0].0..ranges[0].1 {
                let point = [
                    self.bounds.min[0] + (x as f32 + 0.5) * self.dx,
                    self.bounds.min[1] + (y as f32 + 0.5) * self.dy,
                ];
                let projection = if length < 1e-10 {
                    1.
                } else {
                    ((point[0] - a[0]) * (b[0] - a[0]) + (point[1] - a[1]) * (b[1] - a[1])) / length
                };
                let t = projection.clamp(0., 1.);
                let closest = [a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])];
                if (point[0] - closest[0]).powi(2) + (point[1] - closest[1]).powi(2)
                    <= radius * radius
                {
                    let height = if shape == CutterShape::Ball {
                        let perp = if length < 1e-10 {
                            (point[0] - a[0]).powi(2) + (point[1] - a[1]).powi(2)
                        } else {
                            (point[0] - a[0] - projection * (b[0] - a[0])).powi(2)
                                + (point[1] - a[1] - projection * (b[1] - a[1])).powi(2)
                        };
                        let dz = b[2] - a[2];
                        let contact = if length < 1e-10 {
                            if dz < 0. { 1. } else { 0. }
                        } else {
                            let half = ((radius * radius - perp).max(0.) / length).sqrt();
                            (projection
                                - dz * (radius * radius - perp).max(0.).sqrt()
                                    / (length.sqrt() * (length + dz * dz).sqrt()))
                            .clamp((projection - half).max(0.), (projection + half).min(1.))
                        };
                        let d = (point[0] - a[0] - contact * (b[0] - a[0])).powi(2)
                            + (point[1] - a[1] - contact * (b[1] - a[1])).powi(2);
                        a[2] + contact * dz + radius - (radius * radius - d).max(0.).sqrt()
                    } else if let Some(rise) = shape.rise() {
                        // The cone's lowest point over the covered interval:
                        // where the tip's descent balances the cone's rise,
                        // or an end of the interval when the move is steeper
                        // than the cone, as a flat end is lowest there.
                        let perp = if length < 1e-10 {
                            (point[0] - a[0]).powi(2) + (point[1] - a[1]).powi(2)
                        } else {
                            (point[0] - a[0] - projection * (b[0] - a[0])).powi(2)
                                + (point[1] - a[1] - projection * (b[1] - a[1])).powi(2)
                        };
                        let dz = b[2] - a[2];
                        let contact = if length < 1e-10 {
                            if dz < 0. { 1. } else { 0. }
                        } else {
                            let half = ((radius * radius - perp).max(0.) / length).sqrt();
                            let cone = rise * rise * length;
                            let t = if cone > dz * dz {
                                projection
                                    - dz * perp.sqrt() / (length.sqrt() * (cone - dz * dz).sqrt())
                            } else if dz < 0. {
                                1.
                            } else {
                                0.
                            };
                            t.clamp((projection - half).max(0.), (projection + half).min(1.))
                        };
                        let d = (point[0] - a[0] - contact * (b[0] - a[0])).powi(2)
                            + (point[1] - a[1] - contact * (b[1] - a[1])).powi(2);
                        a[2] + contact * dz + rise * d.sqrt()
                    } else if length < 1e-10 {
                        a[2].min(b[2])
                    } else {
                        // During a ramp a cell is covered over an interval of
                        // cutter positions. Use the lowest tip over that whole
                        // interval, not just the nearest centerline position.
                        // This also makes stock independent of playback chunking.
                        let perpendicular = (point[0] - a[0] - projection * (b[0] - a[0])).powi(2)
                            + (point[1] - a[1] - projection * (b[1] - a[1])).powi(2);
                        let half = ((radius * radius - perpendicular).max(0.) / length).sqrt();
                        let contact = if b[2] < a[2] {
                            projection + half
                        } else {
                            projection - half
                        }
                        .clamp(0., 1.);
                        a[2] + contact * (b[2] - a[2])
                    };
                    let i = y * self.nx + x;
                    self.heights[i] = self.heights[i].min(height.max(self.bounds.min[2]));
                }
            }
        }
    }
    pub fn removed_volume(&self) -> f32 {
        self.heights
            .iter()
            .map(|z| self.bounds.max[2] - z)
            .sum::<f32>()
            * self.dx
            * self.dy
    }
}
#[derive(Debug, Clone)]
pub struct Simulation {
    pub id: String,
    pub plan: Plan,
    pub stock: StockGrid,
    pub initial_stock: StockGrid,
    pub volume: Option<crate::volume::Volume>,
    pub initial_volume: Option<crate::volume::Volume>,
    pub frame: Option<crate::volume::Frame>,
    pub preceding: usize,
    /// Setup and operation (from 0, document order) this simulation plays.
    /// Stamped on every cut it makes; `(0, 0)` outside a sequence.
    pub source: (u16, u16),
    pub stock_epoch: u64,
    pub stock_surface: Option<std::sync::Arc<crate::stock_surface::SurfaceMesh>>,
    pub stock_surface_version: u64,
    pub stock_surface_detail: crate::stock_surface::Detail,
    pub stock_surface_seconds: f32,
    pub seconds: f32,
    pub position: [f32; 3],
    pub current: usize,
    pub revision: u64,
    /// What the operation cuts, stretch by stretch (`forces::bites`), once
    /// asked: the export and a run's checks both ask.
    pub bites: std::sync::OnceLock<Vec<crate::forces::Bite>>,
}
impl Simulation {
    pub fn new(plan: Plan) -> Self {
        let stock = StockGrid::new(plan.stock);
        let position = plan.moves[0].from;
        Self {
            id: crate::model::id(),
            plan,
            initial_stock: stock.clone(),
            stock,
            volume: None,
            initial_volume: None,
            frame: None,
            preceding: 0,
            source: (0, 0),
            stock_epoch: 0,
            stock_surface: None,
            stock_surface_version: 0,
            stock_surface_detail: Default::default(),
            stock_surface_seconds: 0.,
            seconds: 0.,
            position,
            current: 0,
            revision: 0,
            bites: Default::default(),
        }
    }
    /// `forces::bites` of this simulation, worked out once.
    pub fn bites(&self) -> &[crate::forces::Bite] {
        self.bites.get_or_init(|| crate::forces::bites(self))
    }
    pub fn removed_volume(&self) -> f32 {
        self.volume
            .as_ref()
            .map_or_else(|| self.stock.removed_volume(), |v| v.removed_volume())
    }
    pub fn inherited_volume(&self) -> f32 {
        self.initial_volume.as_ref().map_or_else(
            || self.initial_stock.removed_volume(),
            |v| v.removed_volume(),
        )
    }
    /// How many moves `seek(fraction)` replays: those from where it stands,
    /// or from the start when it goes back.
    pub fn replay(&self, fraction: f32) -> usize {
        let seconds = fraction.clamp(0., 1.) * self.plan.duration;
        let to = self.plan.moves.partition_point(|m| m.start <= seconds);
        if seconds < self.seconds {
            to
        } else {
            to.saturating_sub(self.current)
        }
    }
    pub fn seek(&mut self, fraction: f32) {
        let seconds = if fraction.is_finite() {
            fraction.clamp(0., 1.) * self.plan.duration
        } else {
            0.
        };
        if seconds < self.seconds {
            self.stock_epoch = self.stock_epoch.wrapping_add(1);
            self.stock_surface = None;
            self.stock = self.initial_stock.clone();
            self.volume = self.initial_volume.clone();
            self.current = 0;
            self.seconds = 0.;
        }
        while self.current < self.plan.moves.len() {
            // A cancelled generation's replay of an earlier operation.
            if crate::progress::stopped() {
                break;
            }
            let movement = &self.plan.moves[self.current];
            if seconds < movement.start {
                break;
            }
            let from = movement.at(self.seconds.max(movement.start));
            let to = movement.at(seconds.min(movement.end));
            // A lathe plan cuts with its rings, below, not along its moves.
            if movement.motion == Motion::Feed && !self.plan.is_lathe() {
                self.stock
                    .cut_shape(from, to, self.plan.radius, self.plan.shape);
                if let (Some(volume), Some(frame)) = (&mut self.volume, self.frame) {
                    volume.cut(
                        from,
                        to,
                        self.plan.radius,
                        self.plan.flute_length,
                        self.plan.shape,
                        frame,
                        self.source,
                    );
                }
            }
            self.position = to;
            if movement.end > seconds {
                break;
            }
            if let (Some(volume), Some(frame)) = (&mut self.volume, self.frame) {
                for ring in self.plan.rings.iter().filter(|r| r.at == self.current) {
                    volume.revolve(
                        ring.z[0],
                        ring.z[1],
                        ring.r[0],
                        ring.r[1],
                        frame,
                        self.source,
                    );
                }
            }
            self.current += 1;
        }
        if seconds == 0. {
            self.position = self.plan.moves[0].from;
        }
        self.seconds = seconds;
        self.revision = self.revision.wrapping_add(1);
    }
}

/// A setup or operation position as stored in `Cut::source`.
fn index(i: usize) -> u16 {
    u16::try_from(i).unwrap_or(u16::MAX)
}
/// Build stock in document order. No failed enabled operation is silently skipped.
/// The continuous cutter history and volume estimate stay attached to the same part across setup rotations/translations.
pub fn sequence(
    doc: &crate::model::Document,
    mesh: &Mesh,
    stop: Option<(usize, usize)>,
    visit: impl FnMut(usize, usize, Result<Simulation, String>),
) {
    sequence_with(doc, mesh, stop, visit, |_, _, _| {})
}
/// `sequence`, also reporting the stock left after each completed operation.
pub fn sequence_with(
    doc: &crate::model::Document,
    mesh: &Mesh,
    stop: Option<(usize, usize)>,
    visit: impl FnMut(usize, usize, Result<Simulation, String>),
    after: impl FnMut(usize, usize, &crate::volume::Volume),
) {
    sequence_walk(doc, mesh, stop, visit, after, |_, _| {})
}
/// The box a later setup `setup` starts from, in its frame (mm), once the
/// cuts in `volume` (the stock left by the setups before it) are made: the
/// original block in that frame, less the layers those cuts provably removed
/// whole, never into the part (`rest::RestStock::left`).
pub fn leftover_box(volume: &crate::volume::Volume, setup: &Setup, mesh: &Mesh) -> Bounds {
    let frame = crate::volume::Frame::new(setup, mesh);
    crate::rest::RestStock::new(volume, frame).left(part_box(setup, mesh))
}
/// The part's bounds in `setup`'s frame, in mm.
pub fn part_box(setup: &Setup, mesh: &Mesh) -> Bounds {
    Placement::new(mesh, Some(setup))
        .bounds
        .scaled(1. / mesh.display_scale())
}
/// `sequence_with`, also telling `begin` as each setup begins, before its
/// operations, the box it starts from (`leftover_box`): None for the first
/// setup and when an earlier operation or setup failed, because the stock
/// left is not known then. Every setup is told, those without an enabled
/// operation too, up to the one `stop` is in; none after it.
pub fn sequence_walk(
    doc: &crate::model::Document,
    mesh: &Mesh,
    stop: Option<(usize, usize)>,
    mut visit: impl FnMut(usize, usize, Result<Simulation, String>),
    mut after: impl FnMut(usize, usize, &crate::volume::Volume),
    mut begin: impl FnMut(usize, Option<Bounds>),
) {
    use crate::volume::{Frame, Volume};
    let mut volume: Option<Volume> = None;
    let mut grid: Option<(Frame, StockGrid)> = None;
    let mut blocked: Option<String> = None;
    let mut preceding = 0;
    // Each enabled operation up to `stop` is an equal part of the progress
    // of a generation off the window's thread (`progress`).
    let parts = (doc.setups.iter().enumerate())
        .flat_map(|(si, s)| {
            s.operations
                .iter()
                .enumerate()
                .map(move |(oi, o)| (si, oi, o))
        })
        .take_while(|&(si, oi, _)| stop.is_none_or(|stop| (si, oi) <= stop))
        .filter(|(_, _, o)| o.enabled)
        .count();
    let mut part = 0;
    for (si, saved_setup) in doc.setups.iter().enumerate() {
        let frame = Frame::new(saved_setup, mesh);
        let mut setup = saved_setup.clone();
        // Stock is automatic in document order; inherited geometry supersedes
        // the later setup's nominal bounding-box settings for generation.
        setup.p.insert("stock_source".into(), "fresh".into());
        let mut starts = None;
        if let Some(v) = &volume {
            // The stock earlier setups left.
            starts = Some(leftover_box(v, &setup, mesh));
            setup.preview_stock = starts;
        } else if blocked.is_none() {
            match stock::resolve(&setup, Some(mesh)) {
                Ok(s) => volume = Some(Volume::stock(s.stock, frame, stock::is_round(&setup))),
                Err(error) => blocked = Some(format!("{}: {error}", setup.name())),
            }
        }
        begin(si, starts.filter(|_| blocked.is_none()));
        for (oi, op) in setup.operations.iter().enumerate() {
            if !op.enabled {
                if stop == Some((si, oi)) {
                    visit(
                        si,
                        oi,
                        Err("Enable this operation before simulation.".into()),
                    );
                    return;
                }
                continue;
            }
            crate::progress::part(part, parts, || {
                format!("{} / {}", saved_setup.name(), op.name())
            });
            part += 1;
            setup.preview_remaining = volume
                .as_ref()
                .filter(|v| !v.cuts.is_empty())
                .map(|v| std::sync::Arc::new(crate::rest::RestStock::new(v, frame)));
            let mut issues = setup.issues();
            issues.extend(setup.operation_issues(oi, &doc.tools));
            let plan = if let Some(issue) = issues.first() {
                Err(issue.clone())
            } else {
                doc.tools
                    .iter()
                    .find(|t| t.id == op.tool)
                    .ok_or_else(|| "Choose a saved tool.".to_string())
                    .and_then(|t| generate(&setup, op, t, mesh))
            };
            let result=plan.and_then(|plan| {
                if let Some(reason)=&blocked {return Err(format!("Cannot inherit remaining stock because an earlier operation/setup failed: {reason}"));}
                let mut sim=Simulation::new(plan);
                sim.volume=volume.clone();sim.initial_volume=volume.clone();sim.frame=Some(frame);sim.preceding=preceding;sim.source=(index(si),index(oi));
                if let Some((previous,stock))=&grid&& *previous==frame && stock.bounds==sim.plan.stock {sim.stock=stock.clone();sim.initial_stock=stock.clone();}
                Ok(sim)
            });
            match &result {
                Ok(sim) if stop != Some((si, oi)) => {
                    let mut finished = sim.clone();
                    finished.seek(1.);
                    if let Some(v) = &finished.volume {
                        after(si, oi, v);
                    }
                    volume = finished.volume;
                    grid = Some((frame, finished.stock));
                    preceding += 1;
                }
                Ok(_) => {}
                Err(error) => {
                    if blocked.is_none() {
                        blocked = Some(format!("{} / {}: {error}", setup.name(), op.name()));
                    }
                }
            }
            visit(si, oi, result);
            if stop == Some((si, oi)) {
                return;
            }
        }
    }
}
pub fn prepare(
    doc: &crate::model::Document,
    mesh: &Mesh,
    setup: usize,
    operation: usize,
) -> Result<Simulation, String> {
    let mut selected = None;
    sequence(doc, mesh, Some((setup, operation)), |s, o, result| {
        if s == setup && o == operation {
            selected = Some(result);
        }
    });
    selected.unwrap_or_else(|| Err("Select an enabled operation.".into()))
}

/// The side of each cutting row the uncut material is on, for successive
/// rows `cuts` at the same Z: the feed crossed with the step to the next
/// row's midpoint. Positive: the material is on the left of the feed (with
/// CW, M3, conventional); negative: on the right (CW climb).
#[cfg(test)]
pub(crate) fn uncut_sides(cuts: &[&Move]) -> Vec<f32> {
    let mid = |m: &Move| [(m.from[0] + m.to[0]) / 2., (m.from[1] + m.to[1]) / 2.];
    cuts.windows(2)
        .filter(|p| p[0].to[2] == p[1].to[2])
        .map(|pair| {
            let (a, b) = (mid(pair[0]), mid(pair[1]));
            let feed = [
                pair[0].to[0] - pair[0].from[0],
                pair[0].to[1] - pair[0].from[1],
            ];
            feed[0] * (b[1] - a[1]) - feed[1] * (b[0] - a[0])
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_heightmap_of_a_volume_is_as_high_as_the_material_left_under_each_cell() {
        let mesh =
            crate::mesh::normalize(vec![[[0., 0., 0.], [20., 0., 0.], [0., 20., 10.]]]).unwrap();
        let frame = crate::volume::Frame::new(
            &crate::model::Setup::new(crate::model::Machine::new(3), "G54"),
            &mesh,
        );
        let bounds = Bounds {
            min: [0., 0., 0.],
            max: [96., 48., 20.],
        };
        let mut volume = crate::volume::Volume::new(bounds, frame);
        // A 10 mm slot 5 mm deep along X at Y 24, and nothing else.
        volume.cut(
            [-10., 24., 15.],
            [110., 24., 15.],
            5.,
            30.,
            CutterShape::Flat,
            frame,
            (0, 0),
        );
        let grid = StockGrid::from_volume(bounds, &volume, frame);
        let at = |x: f32, y: f32| {
            grid.heights[((y / grid.dy) as usize) * grid.nx + (x / grid.dx) as usize]
        };
        assert!((at(48., 5.) - 20.).abs() < 0.01, "{}", at(48., 5.));
        assert!((at(48., 24.) - 15.).abs() < 0.6, "{}", at(48., 24.));
        // Carving the grid on deepens the slot where the tool goes.
        let mut carved = grid.clone();
        carved.cut_shape([0., 24., 12.], [50., 24., 12.], 2., CutterShape::Flat);
        assert!((carved.heights[..].iter().zip(&grid.heights)).all(|(c, g)| c <= g));
        let at = |x: f32, y: f32| {
            carved.heights[((y / grid.dy) as usize) * grid.nx + (x / grid.dx) as usize]
        };
        assert!((at(25., 24.) - 12.).abs() < 0.01 && (at(80., 24.) - 15.).abs() < 0.6);
    }
    #[test]
    fn sloped_stock_sweep_is_independent_of_playback_subdivision() {
        let bounds = Bounds {
            min: [0., 0., 0.],
            max: [10., 10., 10.],
        };
        for (a, b) in [([1., 5., 8.], [9., 5., 2.]), ([9., 5., 2.], [1., 5., 8.])] {
            let mut direct = StockGrid::new(bounds);
            let mut split = direct.clone();
            direct.cut(a, b, 1.);
            for i in 0..100 {
                let p = std::array::from_fn(|k| a[k] + (b[k] - a[k]) * i as f32 / 100.);
                let q = std::array::from_fn(|k| a[k] + (b[k] - a[k]) * (i + 1) as f32 / 100.);
                split.cut(p, q, 1.);
            }
            for (x, y) in direct.heights.iter().zip(split.heights) {
                assert!((x - y).abs() < 0.0001, "{x} != {y}");
            }
        }
    }
    #[test]
    fn a_drills_sweep_leaves_its_point_cone_in_the_stock_grid() {
        let bounds = Bounds {
            min: [0., 0., 0.],
            max: [10., 10., 10.],
        };
        let cell = |grid: &StockGrid, x: usize, y: usize| {
            [(x as f32 + 0.5) * grid.dx, (y as f32 + 0.5) * grid.dy]
        };
        // A Ø4 drill plunged to 3 mm leaves the cone its point makes over
        // its tip, a 118° cone 1.2 mm tall at its rim, under flat stock.
        let mut grid = StockGrid::new(bounds);
        grid.cut_shape([5., 5., 9.], [5., 5., 3.], 2., CutterShape::Drill);
        let mut cone = 0;
        for y in 0..grid.ny {
            for x in 0..grid.nx {
                let p = cell(&grid, x, y);
                let distance = ((p[0] - 5.).powi(2) + (p[1] - 5.).powi(2)).sqrt();
                let height = grid.heights[y * grid.nx + x];
                if distance <= 2. {
                    assert!(
                        (height - (3. + POINT_RISE * distance)).abs() < 0.0001,
                        "{p:?}"
                    );
                    cone += 1;
                } else {
                    assert_eq!(height, 10., "{p:?}");
                }
            }
        }
        assert!(cone > 50);
        // Along a ramp the cone is lowest where the tip's descent balances
        // its rise: the sampled lowest cutter over the move, whatever the
        // playback's chunking.
        for (a, b) in [([1., 5., 8.], [9., 5., 2.]), ([9., 5., 2.], [1., 5., 8.])] {
            let mut direct = StockGrid::new(bounds);
            let mut split = direct.clone();
            direct.cut_shape(a, b, 1., CutterShape::Drill);
            for i in 0..100 {
                let p = std::array::from_fn(|k| a[k] + (b[k] - a[k]) * i as f32 / 100.);
                let q = std::array::from_fn(|k| a[k] + (b[k] - a[k]) * (i + 1) as f32 / 100.);
                split.cut_shape(p, q, 1., CutterShape::Drill);
            }
            let mut checked = 0;
            for y in 0..direct.ny {
                for x in 0..direct.nx {
                    let i = y * direct.nx + x;
                    assert!((direct.heights[i] - split.heights[i]).abs() < 0.0001);
                    let p = cell(&direct, x, y);
                    let lowest = (0..=4000)
                        .map(|j| {
                            let t = j as f32 / 4000.;
                            let d = ((p[0] - a[0] - t * (b[0] - a[0])).powi(2)
                                + (p[1] - a[1] - t * (b[1] - a[1])).powi(2))
                            .sqrt();
                            if d <= 1. {
                                a[2] + t * (b[2] - a[2]) + POINT_RISE * d
                            } else {
                                10.
                            }
                        })
                        .fold(10., f32::min);
                    assert!((direct.heights[i] - lowest).abs() < 0.01, "{p:?}");
                    if lowest < 10. {
                        checked += 1;
                    }
                }
            }
            assert!(checked > 100);
        }
    }
    fn fixture() -> (Setup, Operation, Tool, Mesh) {
        let mesh = crate::mesh::normalize(vec![
            [[0., 0., 0.], [10., 0., 0.], [0., 10., 0.]],
            [[10., 0., 0.], [10., 10., 0.], [0., 10., 0.]],
            [[0., 0., 10.], [10., 0., 10.], [0., 10., 10.]],
            [[10., 0., 10.], [10., 10., 10.], [0., 10., 10.]],
        ])
        .unwrap();
        let mut setup = crate::model::Document::sample().setup().clone();
        for (key, _) in stock::OFFSETS {
            setup.p.insert(key.into(), "0".into());
        }
        setup.p.insert("stock_top".into(), "4".into());
        let tool = Tool::new("2 mm cutter", "Flat end mill", "2");
        let mut op = Operation::new(Strategy::Face, tool.id.clone());
        op.geometry.clear();
        op.surfaces = Some(surface::Selection {
            mesh_fingerprint: mesh.fingerprint.clone(),
            ids: vec![1],
        });
        op.p.insert("allowance".into(), "0".into());
        op.p.insert("boundary".into(), "Stock silhouette".into());
        op.p.insert("containment".into(), "Tool center on boundary".into());
        op.p.insert("stepover".into(), "50".into());
        (setup, op, tool, mesh)
    }
    #[test]
    fn facing_motion_follows_tool_steps_height_feed_and_clearance() {
        let (setup, mut op, tool, mesh) = fixture();
        let plan = generate(&setup, &op, &tool, &mesh).unwrap();
        assert_eq!(plan.levels, 2);
        assert_eq!(plan.radius, 1.);
        assert_eq!(plan.holder_radius, 16.);
        for pair in plan.moves.windows(2) {
            assert_eq!(pair[0].to, pair[1].from);
            assert_eq!(pair[0].end, pair[1].start);
        }
        assert!(
            plan.moves
                .iter()
                .all(|m| m.end > m.start && m.to[2] >= plan.target - 1e-5)
        );
        for m in plan
            .moves
            .iter()
            .filter(|m| m.motion == Motion::Rapid && (m.from[0] != m.to[0] || m.from[1] != m.to[1]))
        {
            assert!(m.from[2] > plan.stock.max[2] && m.to[2] > plan.stock.max[2]);
        }
        op.p.insert("feed".into(), "2000".into());
        let faster = generate(&setup, &op, &tool, &mesh).unwrap();
        assert!(faster.duration < plan.duration && faster.duration > plan.duration / 2.);
        op.p.insert("stepover".into(), "25".into());
        assert!(generate(&setup, &op, &tool, &mesh).unwrap().moves.len() > plan.moves.len());
    }
    #[test]
    fn stock_removal_matches_swept_cutter_and_rewind_is_repeatable() {
        let (setup, op, tool, mesh) = fixture();
        let plan = generate(&setup, &op, &tool, &mesh).unwrap();
        let mut sim = Simulation::new(plan.clone());
        sim.seek(0.);
        assert_eq!(sim.stock.removed_volume(), 0.);
        sim.seek(0.55);
        let partial = sim.stock.heights.clone();
        assert!(sim.stock.removed_volume() > 0. && sim.stock.removed_volume() < 400.);
        sim.seek(1.);
        assert!((sim.stock.removed_volume() - 400.).abs() < 0.1);
        assert!(
            sim.stock
                .heights
                .iter()
                .all(|z| (*z - plan.target).abs() < 0.001)
        );
        assert_eq!(sim.position, plan.moves.last().unwrap().to);
        sim.seek(0.55);
        assert_eq!(partial, sim.stock.heights);
        sim.seek(0.);
        assert_eq!(sim.stock.removed_volume(), 0.);
        let mut incremental = Simulation::new(plan);
        for i in 1..=200 {
            incremental.seek(i as f32 / 200.);
        }
        sim.seek(1.);
        assert_eq!(incremental.stock.heights, sim.stock.heights);
    }
    #[test]
    fn rapids_do_not_remove_stock_and_cut_is_limited_to_radius() {
        let bounds = Bounds {
            min: [0.; 3],
            max: [10., 10., 10.],
        };
        let mut grid = StockGrid::new(bounds);
        grid.cut([2., 5., 8.], [8., 5., 8.], 1.);
        for y in 0..grid.ny {
            for x in 0..grid.nx {
                let px = (x as f32 + 0.5) * grid.dx;
                let py = (y as f32 + 0.5) * grid.dy;
                let distance = (px - px.clamp(2., 8.)).powi(2) + (py - 5.).powi(2);
                assert_eq!(
                    grid.heights[y * grid.nx + x],
                    if distance <= 1. { 8. } else { 10. }
                );
            }
        }
        let (setup, op, tool, mesh) = fixture();
        let mut plan = generate(&setup, &op, &tool, &mesh).unwrap();
        for m in &mut plan.moves {
            m.motion = Motion::Rapid;
        }
        let mut sim = Simulation::new(plan);
        sim.seek(1.);
        assert_eq!(sim.stock.removed_volume(), 0.);
    }
    #[test]
    fn reject_gouging_unsupported_tools_stale_mesh_and_unbounded_paths() {
        let (setup, mut op, mut tool, mesh) = fixture();
        op.p.insert("bottom".into(), "-1".into());
        assert!(
            generate(&setup, &op, &tool, &mesh)
                .unwrap_err()
                .contains("below")
        );
        op.p.insert("bottom".into(), "0".into());
        tool.p.insert("type".into(), "Ball end mill".into());
        assert!(
            generate(&setup, &op, &tool, &mesh)
                .unwrap_err()
                .contains("flat end")
        );
        tool.p.insert("type".into(), "Flat end mill".into());
        op.p.insert("stepdown".into(), "0.000001".into());
        assert!(
            generate(&setup, &op, &tool, &mesh)
                .unwrap_err()
                .contains("5 million")
        );
        op.p.insert("stepdown".into(), "2".into());
        op.surfaces.as_mut().unwrap().mesh_fingerprint = "0".repeat(64);
        assert!(
            generate(&setup, &op, &tool, &mesh)
                .unwrap_err()
                .contains("changed")
        );
    }
    #[test]
    fn a_facing_deeper_than_the_tool_stickout_is_refused() {
        let (mut setup, op, tool, mesh) = fixture();
        // The 2 mm cutter's stickout is 35 mm. With 40 mm of stock above the
        // target, the holder would be in the stock, as the other kernels refuse.
        setup.p.insert("stock_top".into(), "40".into());
        let error = generate(&setup, &op, &tool, &mesh).unwrap_err();
        assert!(error.contains("stickout"), "{error}");
        setup.p.insert("stock_top".into(), "34".into());
        assert!(generate(&setup, &op, &tool, &mesh).is_ok());
    }
    #[test]
    fn a_part_translated_beyond_the_coordinate_limit_is_refused_in_a_fixed_box() {
        let (mut setup, face, tool, mesh) = fixture();
        setup.p.insert("stock_type".into(), "Fixed size box".into());
        setup.p.insert("tx".into(), "1e20".into());
        // The box itself is within range; only the placed part is not.
        let stock = stock::resolve(&setup, Some(&mesh)).unwrap();
        assert!(!exceeds_coordinate_limit(&stock.stock));
        for strategy in [
            Strategy::Face,
            Strategy::Pocket,
            Strategy::Contour,
            Strategy::Drill,
            Strategy::Parallel,
            Strategy::Waterline,
        ] {
            let mut op = Operation::new(strategy, tool.id.clone());
            op.geometry.clear();
            op.surfaces = face.surfaces.clone();
            assert_eq!(
                generate(&setup, &op, &tool, &mesh).unwrap_err(),
                COORDINATE_LIMIT_MESSAGE,
                "{strategy:?}"
            );
        }
        // The hole list is also asked for outside `generate`.
        let mut drill = Operation::new(Strategy::Drill, tool.id.clone());
        drill.surfaces = face.surfaces.clone();
        assert_eq!(
            crate::cam::holes(&setup, &drill, &mesh).unwrap_err(),
            COORDINATE_LIMIT_MESSAGE
        );
        setup.p.insert("tx".into(), "0".into());
        assert_ne!(
            crate::cam::holes(&setup, &drill, &mesh).err().as_deref(),
            Some(COORDINATE_LIMIT_MESSAGE)
        );
    }
    #[test]
    fn rotated_setups_and_pass_angles_share_model_stock_coordinates() {
        let (mut setup, mut op, tool, mesh) = fixture();
        setup.p.insert("rx".into(), "180".into());
        setup.p.insert("tx".into(), "25".into());
        op.surfaces.as_mut().unwrap().ids = vec![0];
        op.p.insert("direction".into(), "35".into());
        let plan = generate(&setup, &op, &tool, &mesh).unwrap();
        assert!((plan.stock.min[0] - 20.).abs() < 0.001);
        assert!((plan.stock.max[0] - 30.).abs() < 0.001);
        assert!(
            plan.moves
                .iter()
                .flat_map(|m| [m.from, m.to])
                .all(|p| p[0] >= 20. - 0.001 && p[0] <= 30. + 0.001)
        );
        let mut sim = Simulation::new(plan);
        sim.seek(1.);
        assert!(sim.stock.removed_volume() > 350.);
    }
    #[test]
    fn both_ways_continues_each_depth_at_previous_endpoint_for_odd_even_and_angled_scans() {
        let (setup, mut op, tool, mesh) = fixture();
        for angle in [0., 35., 90., 180., 225.] {
            for stepover in [50, 100] {
                for link in crate::model::FACING_LINKS {
                    op.p.insert("direction".into(), angle.to_string());
                    op.p.insert("stepover".into(), stepover.to_string());
                    op.p.insert("facing_link".into(), link.into());
                    let plan = generate(&setup, &op, &tool, &mesh).unwrap();
                    let passes: Vec<_> = plan
                        .moves
                        .iter()
                        .enumerate()
                        .filter(|(_, m)| m.cutting_pass)
                        .collect();
                    let first_layer: Vec<_> = passes
                        .iter()
                        .filter(|(_, m)| (m.to[2] - passes[0].1.to[2]).abs() < 0.0001)
                        .collect();
                    assert!(first_layer.len() > 1);
                    let count = first_layer.len();
                    for depth in 1..plan.levels {
                        let (last_index, last) = passes[depth * count - 1];
                        let (next_index, next) = passes[depth * count];
                        assert_eq!(&last.to[..2], &next.from[..2]);
                        // The first stroke at the next depth exactly retraces the last
                        // row in reverse XY, at the newly lowered Z.
                        assert_eq!(&last.from[..2], &next.to[..2]);
                        assert!(next.from[2] < last.to[2]);
                        assert!(
                            plan.moves[last_index + 1..next_index]
                                .iter()
                                .all(|m| m.from[..2] == m.to[..2])
                        );
                    }
                    if link == "Keep tool down" {
                        let start = passes[0].0;
                        let end = passes.last().unwrap().0;
                        assert!(
                            plan.moves[start..=end]
                                .iter()
                                .all(|m| m.motion == Motion::Feed)
                        );
                        assert!(
                            plan.moves[start..=end]
                                .iter()
                                .all(|m| m.from[2] == m.to[2] || m.from[..2] == m.to[..2])
                        );
                    }
                }
            }
        }
    }
    #[test]
    fn one_way_keeps_cut_direction_and_returns_at_clearance() {
        let (setup, mut op, tool, mesh) = fixture();
        op.p.insert("facing_pattern".into(), "One way".into());
        op.p.insert("facing_milling".into(), "Conventional".into());
        let plan = generate(&setup, &op, &tool, &mesh).unwrap();
        let passes: Vec<_> = plan
            .moves
            .iter()
            .enumerate()
            .filter(|(_, m)| m.cutting_pass)
            .collect();
        assert!(passes.iter().all(|(_, m)| m.to[0] > m.from[0]));
        for pair in passes.windows(2) {
            assert!(
                plan.moves[pair[0].0 + 1..pair[1].0]
                    .iter()
                    .any(|m| m.motion == Motion::Rapid && m.from[0] != m.to[0])
            );
        }
        let next_depth = passes
            .iter()
            .find(|(_, m)| m.to[2] < passes[0].1.to[2] - 0.001)
            .unwrap()
            .1;
        assert_eq!(&passes[0].1.from[..2], &next_depth.from[..2]);
        for m in plan
            .moves
            .iter()
            .filter(|m| m.motion == Motion::Rapid && m.from[..2] != m.to[..2])
        {
            assert!(m.from[2] > plan.stock.max[2] && m.to[2] > plan.stock.max[2]);
        }
    }
    #[test]
    fn keep_down_reduces_return_motion_without_changing_final_stock_and_rewinds() {
        let (setup, mut op, tool, mesh) = fixture();
        let continuous = generate(&setup, &op, &tool, &mesh).unwrap();
        op.p.insert("facing_link".into(), "Retract each pass".into());
        op.p.insert("facing_depth_order".into(), "Return to first start".into());
        let returning = generate(&setup, &op, &tool, &mesh).unwrap();
        assert!(continuous.duration < returning.duration);
        assert!(
            continuous
                .moves
                .iter()
                .filter(|m| m.motion == Motion::Rapid)
                .count()
                < returning
                    .moves
                    .iter()
                    .filter(|m| m.motion == Motion::Rapid)
                    .count()
        );
        let passes: Vec<_> = returning.moves.iter().filter(|m| m.cutting_pass).collect();
        let next = passes
            .iter()
            .find(|m| m.to[2] < passes[0].to[2] - 0.001)
            .unwrap();
        assert_eq!(&passes[0].from[..2], &next.from[..2]);
        let mut a = Simulation::new(continuous);
        let mut b = Simulation::new(returning);
        a.seek(0.4);
        let partial = a.stock.heights.clone();
        a.seek(1.);
        b.seek(1.);
        assert_eq!(a.stock.heights, b.stock.heights);
        a.seek(0.4);
        assert_eq!(a.stock.heights, partial);
    }
    #[test]
    fn saved_facing_preferences_roundtrip_and_legacy_jobs_keep_original_linking() {
        let (setup, mut op, tool, mesh) = fixture();
        let serialized = serde_json::to_string(&op).unwrap();
        let restored: Operation = serde_json::from_str(&serialized).unwrap();
        assert_eq!(
            facing_setting(&restored.p, "facing_depth_order"),
            "Continue from previous end"
        );
        assert_eq!(facing_setting(&restored.p, "facing_link"), "Keep tool down");
        for key in ["facing_pattern", "facing_depth_order", "facing_link"] {
            op.p.remove(key);
        }
        let legacy = generate(&setup, &op, &tool, &mesh).unwrap();
        op.p.insert("facing_pattern".into(), "Both ways (zigzag)".into());
        op.p.insert("facing_depth_order".into(), "Return to first start".into());
        op.p.insert("facing_link".into(), "Retract each pass".into());
        let explicit = generate(&setup, &op, &tool, &mesh).unwrap();
        assert_eq!(legacy.duration, explicit.duration);
        assert_eq!(
            legacy.moves.iter().map(|m| m.to).collect::<Vec<_>>(),
            explicit.moves.iter().map(|m| m.to).collect::<Vec<_>>()
        );
        op.p.insert("facing_pattern".into(), "unknown".into());
        assert!(
            generate(&setup, &op, &tool, &mesh)
                .unwrap_err()
                .contains("valid facing pattern")
        );
    }
    #[test]
    fn one_way_milling_tracks_uncut_side_and_spindle_for_rotated_passes() {
        let (setup, mut op, tool, mesh) = fixture();
        op.p.insert("facing_pattern".into(), "One way".into());
        for angle in [0_f32, 35., 90., 180., 225., 270.] {
            op.p.insert("direction".into(), angle.to_string());
            let mut expected_stock = None;
            for rotation in crate::model::SPINDLE_ROTATIONS {
                for method in crate::model::FACING_MILLING {
                    op.p.insert("spindle_rotation".into(), rotation.into());
                    op.p.insert("facing_milling".into(), method.into());
                    let plan = generate(&setup, &op, &tool, &mesh).unwrap();
                    assert_eq!(plan.milling_direction, method);
                    assert_eq!(plan.spindle_clockwise, rotation == "Clockwise (CW)");
                    let cuts: Vec<_> = plan.moves.iter().filter(|m| m.cutting_pass).collect();
                    // Use successive row midpoints to measure the nominal uncut
                    // side, independent of generator's direction/across vectors.
                    let sides = uncut_sides(&cuts);
                    assert!(!sides.is_empty());
                    for side in sides {
                        // CW climb: material on right of moving cutter. CCW:
                        // material on left. Conventional has the opposite side.
                        let right = (rotation == "Clockwise (CW)") == (method == "Climb");
                        assert!(
                            if right { side < -0.001 } else { side > 0.001 },
                            "{angle} {rotation} {method}"
                        );
                    }
                    let mut sim = Simulation::new(plan);
                    sim.seek(1.);
                    if let Some(expected) = &expected_stock {
                        assert_eq!(&sim.stock.heights, expected);
                    } else {
                        expected_stock = Some(sim.stock.heights.clone());
                    }
                }
            }
        }
    }
    #[test]
    fn reverse_spindle_reverses_one_way_strokes_but_zigzag_stays_mixed() {
        let (setup, mut op, tool, mesh) = fixture();
        op.p.insert("facing_pattern".into(), "One way".into());
        let cw = generate(&setup, &op, &tool, &mesh).unwrap();
        op.p.insert("spindle_rotation".into(), "Counterclockwise (CCW)".into());
        let ccw = generate(&setup, &op, &tool, &mesh).unwrap();
        for (a, b) in cw
            .moves
            .iter()
            .filter(|m| m.cutting_pass)
            .zip(ccw.moves.iter().filter(|m| m.cutting_pass))
        {
            assert_eq!(a.from, b.to);
            assert_eq!(a.to, b.from);
        }
        op.p.insert("facing_pattern".into(), "Both ways (zigzag)".into());
        let ccw = generate(&setup, &op, &tool, &mesh).unwrap();
        op.p.insert("spindle_rotation".into(), "Clockwise (CW)".into());
        op.p.insert("facing_milling".into(), "Conventional".into());
        let cw = generate(&setup, &op, &tool, &mesh).unwrap();
        assert_eq!(cw.milling_direction, "Mixed · zigzag");
        assert_eq!(cw.milling_direction, ccw.milling_direction);
        assert_eq!(
            cw.moves.iter().map(|m| m.to).collect::<Vec<_>>(),
            ccw.moves.iter().map(|m| m.to).collect::<Vec<_>>()
        );
    }
    #[test]
    fn milling_preferences_persist_and_legacy_one_way_motion_is_preserved() {
        let (setup, mut op, tool, mesh) = fixture();
        op.p.insert("facing_pattern".into(), "One way".into());
        op.p.insert("spindle_rotation".into(), "Counterclockwise (CCW)".into());
        let restored: Operation =
            serde_json::from_str(&serde_json::to_string(&op).unwrap()).unwrap();
        assert_eq!(
            facing_setting(&restored.p, "spindle_rotation"),
            "Counterclockwise (CCW)"
        );
        assert_eq!(facing_setting(&restored.p, "facing_milling"), "Climb");
        op.p.remove("spindle_rotation");
        op.p.remove("facing_milling");
        let legacy = generate(&setup, &op, &tool, &mesh).unwrap();
        assert!(legacy.spindle_clockwise);
        assert_eq!(legacy.milling_direction, "Conventional");
        assert!(
            legacy
                .moves
                .iter()
                .filter(|m| m.cutting_pass)
                .all(|m| m.to[0] > m.from[0])
        );
        op.p.insert("spindle_rotation".into(), "Clockwise (CW)".into());
        op.p.insert("facing_milling".into(), "Conventional".into());
        let explicit = generate(&setup, &op, &tool, &mesh).unwrap();
        assert_eq!(legacy.duration, explicit.duration);
        assert_eq!(
            legacy.moves.iter().map(|m| m.to).collect::<Vec<_>>(),
            explicit.moves.iter().map(|m| m.to).collect::<Vec<_>>()
        );
    }
    #[test]
    fn unknown_milling_or_spindle_settings_are_rejected() {
        let (setup, op, tool, mesh) = fixture();
        for key in ["facing_milling", "spindle_rotation"] {
            let mut invalid = op.clone();
            invalid.p.insert(key.into(), "invalid".into());
            assert!(
                generate(&setup, &invalid, &tool, &mesh)
                    .unwrap_err()
                    .contains(&key.replace('_', " "))
            );
        }
    }
}

#[cfg(test)]
mod cumulative_tests {
    use super::*;
    use crate::{
        model::{Document, Machine},
        surface::Selection,
        volume::Frame,
    };
    fn fixture() -> (Document, Mesh) {
        let mesh = crate::mesh::load(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/four-setup-block.3mf"),
        )
        .unwrap();
        let mut doc = Document::default();
        let tool = Tool::new("Fixture flat Ø10", "Flat end mill", "10");
        let mut setup = Setup::new(Machine::new(3), "G54");
        for (k, v) in [
            ("stock_top", "4"),
            ("stock_bottom", "3"),
            ("stock_left", "2"),
            ("stock_right", "2"),
            ("stock_front", "2"),
            ("stock_back", "2"),
        ] {
            setup.p.insert(k.into(), v.into());
        }
        let mut face = Operation::new(Strategy::Face, tool.id.clone());
        face.geometry.clear();
        face.p.insert("boundary".into(), "Stock silhouette".into());
        face.p
            .insert("containment".into(), "Tool outside boundary".into());
        face.p.insert("allowance".into(), "0".into());
        select_top(&mut face, &setup, &mesh);
        let placement = Placement::new(&mesh, Some(&setup));
        let floor = mesh
            .surfaces
            .iter()
            .enumerate()
            .filter(|(_, s)| {
                s.normal[2].abs() > 0.999999
                    && placement.transform(s.center)[2] < placement.bounds.max[2] - 0.1
                    && placement.transform(s.center)[2] > placement.bounds.min[2] + 0.1
            })
            .max_by(|(_, a), (_, b)| a.area.total_cmp(&b.area))
            .unwrap()
            .0;
        let mut pocket = Operation::new(Strategy::Pocket, tool.id.clone());
        pocket.geometry.clear();
        pocket.surfaces = Some(Selection {
            mesh_fingerprint: mesh.fingerprint.clone(),
            ids: vec![floor],
        });
        pocket.p.insert("pocket_entry".into(), "Plunge".into());
        setup.operations = vec![face, pocket];
        let mut flipped = Setup::new(Machine::new(3), "G55");
        flipped.p.insert("rx".into(), "180".into());
        flipped.p.insert("tx".into(), "12".into());
        flipped.p.insert("ty".into(), "-7".into());
        let mut face = setup.operations[0].clone();
        face.id = crate::model::id();
        select_top(&mut face, &flipped, &mesh);
        flipped.operations.push(face);
        doc.tools.push(tool);
        doc.active = setup.id.clone();
        doc.setups = vec![setup, flipped];
        doc.sync_stock_sources();
        (doc, mesh)
    }
    fn select_top(op: &mut Operation, setup: &Setup, mesh: &Mesh) {
        let placement = Placement::new(mesh, Some(setup));
        let scale = mesh.display_scale();
        let id = mesh
            .surfaces
            .iter()
            .enumerate()
            .filter(|(_, s)| {
                (placement.transform(s.center)[2] - placement.bounds.max[2]).abs() < 0.001 * scale
            })
            .max_by(|(_, a), (_, b)| a.area.total_cmp(&b.area))
            .unwrap()
            .0;
        op.surfaces = Some(Selection {
            mesh_fingerprint: mesh.fingerprint.clone(),
            ids: vec![id],
        });
    }
    #[test]
    fn each_setup_is_told_the_box_it_starts_from_with_or_without_operations() {
        let (mut doc, mesh) = fixture();
        let starts = |doc: &crate::model::Document| {
            let (mut told, mut firsts) = (vec![], vec![]);
            sequence_walk(
                doc,
                &mesh,
                None,
                |si, _, result| {
                    if !firsts.iter().any(|(s, _)| *s == si) {
                        firsts.push((si, result.map(|sim| sim.plan.stock)));
                    }
                },
                |_, _, _| {},
                |si, stock| told.push((si, stock)),
            );
            (told, firsts)
        };
        // The first setup starts from its own stock; a later one from what
        // the earlier cut away, which its first operation's plan has too.
        let (told, firsts) = starts(&doc);
        assert_eq!(told.len(), 2);
        assert_eq!(told[0], (0, None));
        let left = told[1].1.unwrap();
        assert_eq!(firsts[1], (1, Ok(left)));
        let facing = doc.setups[0].operations[0].clone();
        // The same without operations of its own.
        doc.setups[1].operations.clear();
        assert_eq!(starts(&doc).0[1], (1, Some(left)));
        // A setup with nothing before it cut has the original block.
        doc.setups[0].operations.clear();
        let original = starts(&doc).0[1].1.unwrap();
        let carried = crate::volume::Volume::new(
            stock::resolve(&doc.setups[0], Some(&mesh)).unwrap().stock,
            Frame::new(&doc.setups[0], &mesh),
        )
        .transformed_bounds(Frame::new(&doc.setups[1], &mesh));
        assert!(
            (0..3).all(|k| (original.min[k] - carried.min[k]).abs() < 0.001
                && (original.max[k] - carried.max[k]).abs() < 0.001),
            "{original:?} {carried:?}"
        );
        // When an earlier operation fails the box is not known.
        doc.setups[0].operations = vec![facing];
        doc.setups[0].operations[0].tool = "gone".into();
        assert_eq!(starts(&doc).0[1], (1, None));
        // The shared computation gives what the plans start from.
        let (doc, _) = fixture();
        let mut volume = None;
        sequence_with(
            &doc,
            &mesh,
            Some((1, 0)),
            |_, _, _| {},
            |si, _, v| {
                if si == 0 {
                    volume = Some(v.clone());
                }
            },
        );
        assert_eq!(
            leftover_box(&volume.unwrap(), &doc.setups[1], &mesh),
            prepare(&doc, &mesh, 1, 0).unwrap().plan.stock
        );
    }
    #[test]
    fn pocket_starts_from_facing_and_restart_restores_inherited_stock() {
        let (doc, mesh) = fixture();
        let mut face = prepare(&doc, &mesh, 0, 0).unwrap();
        face.seek(1.);
        let mut pocket = prepare(&doc, &mesh, 0, 1).unwrap();
        let original = generate(
            &doc.setups[0],
            &doc.setups[0].operations[1],
            &doc.tools[0],
            &mesh,
        )
        .unwrap();
        assert_eq!(pocket.plan.rest_top, Some(face.plan.target));
        assert!(pocket.plan.levels < original.levels);
        assert!(pocket.plan.duration < original.duration);
        assert!(
            pocket
                .plan
                .moves
                .iter()
                .filter(|m| m.cutting_pass)
                .all(|m| m.to[2] < face.plan.target)
        );
        let rapid_height = |p: &Plan| {
            p.moves
                .iter()
                .filter(|m| m.motion == Motion::Rapid)
                .flat_map(|m| [m.from[2], m.to[2]])
                .fold(f32::NEG_INFINITY, f32::max)
        };
        assert_eq!(rapid_height(&pocket.plan), rapid_height(&original));
        let mut baseline = Simulation::new(original);
        baseline.volume = face.volume.clone();
        baseline.initial_volume = face.volume.clone();
        baseline.frame = face.frame;
        baseline.seek(1.);
        assert_eq!(pocket.preceding, 1);
        assert_eq!(
            face.volume.as_ref().unwrap().occupied,
            pocket.volume.as_ref().unwrap().occupied
        );
        assert_eq!(
            face.volume.as_ref().unwrap().cuts,
            pocket.volume.as_ref().unwrap().cuts
        );
        assert!(pocket.inherited_volume() > 0.);
        pocket.seek(0.6);
        let mid = pocket.volume.as_ref().unwrap().occupied.clone();
        pocket.seek(1.);
        assert_eq!(
            pocket.volume.as_ref().unwrap().occupied,
            baseline.volume.as_ref().unwrap().occupied
        );
        assert!(pocket.removed_volume() > pocket.inherited_volume());
        pocket.seek(0.6);
        assert_eq!(pocket.volume.as_ref().unwrap().occupied, mid);
        pocket.seek(0.);
        assert_eq!(
            pocket.volume.as_ref().unwrap().occupied,
            face.volume.as_ref().unwrap().occupied
        );
        assert_eq!(pocket.removed_volume(), pocket.inherited_volume());
    }
    #[test]
    fn remaining_start_rebuilds_after_facing_edits_suppression_and_reorder() {
        let (mut doc, mesh) = fixture();
        let original_top = stock::resolve(&doc.setups[0], Some(&mesh))
            .unwrap()
            .stock
            .max[2];
        let first = prepare(&doc, &mesh, 0, 1).unwrap();
        let face_top = first.plan.rest_top.unwrap();
        doc.setups[0].operations[0]
            .p
            .insert("bottom".into(), "0.6".into());
        let allowance = prepare(&doc, &mesh, 0, 1).unwrap();
        assert!((allowance.plan.rest_top.unwrap() - face_top - 0.6).abs() < 0.001);
        doc.setups[0].operations[0].enabled = false;
        let suppressed = prepare(&doc, &mesh, 0, 1).unwrap();
        assert_eq!(suppressed.plan.rest_top, None);
        assert!(suppressed.plan.levels > first.plan.levels);
        assert!(
            suppressed
                .plan
                .moves
                .iter()
                .any(|m| m.cutting_pass && m.to[2] > face_top)
        );
        doc.setups[0].operations[0].enabled = true;
        doc.setups[0].operations.swap(0, 1);
        assert_eq!(prepare(&doc, &mesh, 0, 0).unwrap().plan.rest_top, None);
        assert!(original_top > face_top);
        let saved = serde_json::to_string(&doc).unwrap();
        assert!(!saved.contains("preview_remaining"));
    }
    #[test]
    fn next_setup_preserves_all_previous_cavities_after_flip_and_translation() {
        let (mut doc, mesh) = fixture();
        let mut previous = prepare(&doc, &mesh, 0, 1).unwrap();
        previous.seek(1.);
        // Later setup stock boxes do not replace inherited material.
        doc.setups[1].p.insert("stock_top".into(), "80".into());
        let mut next = prepare(&doc, &mesh, 1, 0).unwrap();
        assert_eq!(next.preceding, 2);
        assert_ne!(previous.frame, next.frame);
        assert_eq!(
            previous.volume.as_ref().unwrap().occupied,
            next.volume.as_ref().unwrap().occupied
        );
        assert_eq!(previous.removed_volume(), next.inherited_volume());
        let mut expected = previous
            .volume
            .as_ref()
            .unwrap()
            .transformed_bounds(Frame::new(&doc.setups[1], &mesh));
        // Setup 1 faced 4 mm (its stock_top) off the top, which is now the
        // bottom; the pocket is inside the part and trims no side.
        expected.min[2] += 4.;
        let stock = next.plan.stock;
        assert!(
            (0..3).all(|k| (stock.min[k] - expected.min[k]).abs() < 0.001
                && (stock.max[k] - expected.max[k]).abs() < 0.001),
            "{stock:?} {expected:?}"
        );
        let prior = next.volume.as_ref().unwrap().occupied.clone();
        next.seek(1.);
        assert!(next.removed_volume() > next.inherited_volume());
        for (a, b) in prior.iter().zip(&next.volume.as_ref().unwrap().occupied) {
            assert!(*a != 0 || *b == 0, "a cavity was refilled");
        }
        next.seek(0.);
        assert_eq!(prior, next.volume.as_ref().unwrap().occupied);
        assert_eq!(
            previous.volume.as_ref().unwrap().cuts,
            next.volume.as_ref().unwrap().cuts
        );
    }
    #[test]
    fn a_flipped_setup_starts_from_the_stock_the_facing_left() {
        let (mut doc, mesh) = fixture();
        // Setup 1 only faces 2 mm off the top; setup 2 is flipped.
        doc.setups[0].p.insert("stock_top".into(), "2".into());
        doc.setups[0].operations.truncate(1);
        let first = stock::resolve(&doc.setups[0], Some(&mesh)).unwrap().stock;
        let carried = crate::volume::Volume::new(first, Frame::new(&doc.setups[0], &mesh))
            .transformed_bounds(Frame::new(&doc.setups[1], &mesh));
        let stock = prepare(&doc, &mesh, 1, 0).unwrap().plan.stock;
        let close = |a: f32, b: f32| (a - b).abs() < 0.001;
        assert!(
            close(stock.dimensions()[2], first.dimensions()[2] - 2.),
            "{stock:?} {first:?}"
        );
        assert!(
            close(stock.min[2], carried.min[2] + 2.),
            "{stock:?} {carried:?}"
        );
        for k in 0..3 {
            assert!(close(stock.max[k], carried.max[k]), "{stock:?} {carried:?}");
        }
        for k in 0..2 {
            assert!(close(stock.min[k], carried.min[k]), "{stock:?} {carried:?}");
        }
        // The vise grips from the new bottom up.
        let vise = crate::holding::starters()
            .into_iter()
            .find(|h| h.kind() == "Vise")
            .unwrap();
        let layout = crate::holding::Layout::Vise {
            holder: vise.id.clone(),
            axis_y: false,
            offset: 0.,
            grip: 5.,
            gap: 2.,
        };
        let jaws = crate::holding::boxes(&layout, &vise, stock).unwrap();
        assert_eq!(jaws.len(), 2);
        for jaw in jaws {
            assert_eq!(jaw.bounds.max[2], stock.min[2] + 5., "{jaw:?}");
            assert!(close(jaw.bounds.max[2], carried.min[2] + 7.), "{jaw:?}");
        }
    }
    #[test]
    fn the_facing_still_trims_the_flipped_stock_among_thousands_of_other_cuts() {
        // Setup 1 faces 4 mm off the top, then pockets, contours to the
        // part's underside and drills: thousands of cuts whose long flutes
        // reach the faced layer. Setup 2 is flipped.
        let (doc, mesh) = crate::post::fixture::flipped_after(4);
        let first = stock::resolve(&doc.setups[0], Some(&mesh)).unwrap().stock;
        let carried = crate::volume::Volume::new(first, Frame::new(&doc.setups[0], &mesh))
            .transformed_bounds(Frame::new(&doc.setups[1], &mesh));
        let sim = prepare(&doc, &mesh, 1, 0).unwrap();
        assert!(sim.volume.as_ref().unwrap().cuts.len() > 8_000);
        let stock = sim.plan.stock;
        assert!(
            (stock.min[2] - (carried.min[2] + 4.)).abs() < 0.001,
            "{stock:?} {carried:?}"
        );
        // The 3 mm of stock below the part's underside is uncut all round,
        // so the sides stay.
        assert_eq!(
            (stock.max, &stock.min[..2]),
            (carried.max, &carried.min[..2])
        );
    }
    #[test]
    fn suppression_upstream_failure_and_reload_follow_document_order() {
        let (mut doc, mesh) = fixture();
        let full = prepare(&doc, &mesh, 1, 0).unwrap();
        doc.setups[0].operations[1].enabled = false;
        let suppressed = prepare(&doc, &mesh, 1, 0).unwrap();
        assert_eq!(suppressed.preceding, 1);
        assert!(suppressed.inherited_volume() < full.inherited_volume());
        doc.setups[0].operations[0]
            .p
            .insert("feed".into(), "0".into());
        assert!(
            prepare(&doc, &mesh, 1, 0)
                .unwrap_err()
                .contains("earlier operation")
        );
        doc.setups[0].operations[0].enabled = false;
        assert_eq!(prepare(&doc, &mesh, 1, 0).unwrap().preceding, 0);
        let saved: Document = serde_json::from_str(&serde_json::to_string(&doc).unwrap()).unwrap();
        let restored = prepare(&saved, &mesh, 1, 0).unwrap();
        assert_eq!(restored.inherited_volume(), 0.);
        assert!(saved.setups.iter().all(|s| s.preview_stock.is_none()));
    }
    #[test]
    fn upstream_edit_rebuilds_downstream_baseline_and_volume_is_time_step_independent() {
        let (mut doc, mesh) = fixture();
        let original = prepare(&doc, &mesh, 0, 1).unwrap();
        doc.setups[0].operations[0]
            .p
            .insert("bottom".into(), "1".into());
        let mut changed = prepare(&doc, &mesh, 0, 1).unwrap();
        assert!(changed.inherited_volume() < original.inherited_volume());
        let mut stepped = changed.clone();
        changed.seek(0.65);
        for i in 0..=65 {
            stepped.seek(i as f32 / 100.);
        }
        assert_eq!(
            changed.volume.as_ref().unwrap().occupied,
            stepped.volume.as_ref().unwrap().occupied
        );
    }
    #[test]
    fn cuts_remember_the_setup_and_operation_that_made_them() {
        let (doc, mesh) = fixture();
        // Setup 1 has two operations and setup 2 one; each is simulated in
        // document order and hands its volume to the next.
        let mut seen = vec![];
        sequence_with(
            &doc,
            &mesh,
            None,
            |_, _, result| {
                result.unwrap();
            },
            |si, oi, volume| {
                seen.push((
                    (si as u16, oi as u16),
                    volume.cuts.iter().map(|c| c.source).collect::<Vec<_>>(),
                ));
            },
        );
        assert_eq!(seen.len(), 3);
        let mut sources = vec![];
        for (operation, cuts) in &seen {
            // The volume after an operation holds everything cut so far; only
            // the newest cuts belong to that operation.
            assert_eq!(cuts.last(), Some(operation), "{operation:?}");
            assert!(cuts.windows(2).all(|w| w[0] <= w[1]), "{cuts:?}");
            sources = cuts.clone();
        }
        sources.dedup();
        assert_eq!(sources, [(0, 0), (0, 1), (1, 0)]);
        // A preview of the last operation starts from the inherited cuts and
        // adds its own while playing.
        let mut preview = prepare(&doc, &mesh, 1, 0).unwrap();
        assert_eq!(preview.source, (1, 0));
        let inherited = preview.volume.as_ref().unwrap().cuts.len();
        assert!(
            preview
                .volume
                .as_ref()
                .unwrap()
                .cuts
                .iter()
                .all(|c| c.source != (1, 0))
        );
        preview.seek(1.);
        let cuts = &preview.volume.as_ref().unwrap().cuts;
        assert!(cuts.len() > inherited);
        assert!(cuts[inherited..].iter().all(|c| c.source == (1, 0)));
        // Rewinding drops the preview's own cuts again.
        preview.seek(0.);
        assert_eq!(preview.volume.as_ref().unwrap().cuts.len(), inherited);
    }
    #[test]
    fn every_move_records_the_rate_its_timing_uses() {
        let (doc, mesh) = fixture();
        let mut plans = 0;
        sequence(&doc, &mesh, None, |_, _, result| {
            let plan = result.unwrap().plan;
            plans += 1;
            for m in &plan.moves {
                let distance = (0..3)
                    .map(|i| (m.to[i] - m.from[i]).powi(2))
                    .sum::<f32>()
                    .sqrt();
                if m.motion == Motion::Dwell {
                    assert_eq!(m.feed, 0.);
                } else {
                    assert!(m.feed > 0.);
                    assert!(
                        ((m.end - m.start) - distance / m.feed * 60.).abs() < 0.01,
                        "{m:?}"
                    );
                }
            }
            if plan.strategy == Strategy::Face {
                assert!(
                    plan.moves
                        .iter()
                        .filter(|m| m.motion == Motion::Feed)
                        .all(|m| m.feed == 1000. || m.feed == 250.)
                );
            }
        });
        assert_eq!(plans, 3);
    }
}
