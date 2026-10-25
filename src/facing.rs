//! Facing of the part's outline seen from above (`Operation::faces_outline`):
//! the stock above the part's top cleared in rows over that outline grown by
//! Clear past outline, and the stock beyond it left at its height. Each level
//! first goes once round the edge of the cutter centres' region, so every row
//! starts and ends in what that pass cleared; the rows then clear the inside.
use crate::{
    mesh::Bounds,
    model::{Operation, Setup, Strategy, num},
    pocket::{ENTRY, Polygon, Region, crosses, inside},
    simulation::{CutterShape, Motion, Move, Plan},
};
use clipper2::{EndType, FillRule, JoinType, Milli, difference, intersect};

/// A row's ends, in the setup frame.
type Stroke = ((f64, f64), (f64, f64));
/// The most times an entry ramp goes round its loop; a shorter loop plunges.
const RAMP_LAPS: f32 = 10.;

/// A Facing of `silhouette` (the part's outline's outer loops, in the setup
/// frame), its cutter's edge reaching `extend` past it, down to `target`.
pub(crate) struct Outline<'a> {
    pub setup: &'a Setup,
    pub op: &'a Operation,
    pub silhouette: Region,
    pub extend: f64,
    pub stock: Bounds,
    pub scale: f32,
    pub radius: f32,
    pub flute_length: f32,
    pub tool_length: f32,
    pub holder_radius: f32,
    pub feed: f32,
    pub stepdown: f32,
    pub stepover: f32,
    pub top: f32,
    pub clearance: f32,
    pub retract: f32,
    pub target: f32,
    /// The rows' direction in setup XY, degrees.
    pub direction: f32,
    pub both_ways: bool,
    pub keep_down: bool,
    pub climb: bool,
    pub spindle_clockwise: bool,
}

/// The stock's rectangle seen from above.
fn rectangle(b: &Bounds) -> Polygon {
    vec![
        (b.min[0] as f64, b.min[1] as f64),
        (b.max[0] as f64, b.min[1] as f64),
        (b.max[0] as f64, b.max[1] as f64),
        (b.min[0] as f64, b.max[1] as f64),
    ]
}
/// Where the centre of a cutter of radius `r` goes to clear `extend` past
/// `silhouette`: that far less its radius past it, and over the stock or
/// within its radius of it.
fn centres(silhouette: &Region, stock: &Polygon, extend: f64, r: f64) -> Result<Region, String> {
    let over = Region::from(vec![stock.clone()]).inflate(r, JoinType::Miter, EndType::Polygon, 2.);
    Ok(intersect::<Milli>(
        silhouette.inflate(extend - r, JoinType::Round, EndType::Polygon, 2.),
        over,
        FillRule::NonZero,
    )
    .map_err(|e| e.to_string())?
    .simplify(0.001, false))
}
/// Whether part of `silhouette` over `stock` is out of reach of a cutter of
/// radius `r` with its centre in `centres`: more than a speck, 0.02 mm across.
fn uncovered(
    silhouette: &Region,
    stock: &Polygon,
    centres: &Region,
    r: f64,
) -> Result<bool, String> {
    let over = intersect::<Milli>(silhouette.clone(), vec![stock.clone()], FillRule::NonZero)
        .map_err(|e| e.to_string())?;
    let reached = centres.inflate(r, JoinType::Round, EndType::Polygon, 2.);
    let left = difference::<Milli>(over, reached, FillRule::NonZero)
        .map_err(|e| e.to_string())?
        .inflate(-0.01, JoinType::Round, EndType::Polygon, 2.);
    Ok(left.contains_points())
}
/// Where the row at `v` (across the rows, `frame`'s second coordinate)
/// lies inside `loops`, as ranges along it.
fn spans(loops: &[Vec<(f64, f64)>], v: f64) -> Vec<(f64, f64)> {
    let mut at = vec![];
    for path in loops {
        for (a, b) in path
            .iter()
            .zip(path.iter().cycle().skip(1))
            .take(path.len())
        {
            if (a.1 > v) != (b.1 > v) {
                at.push(a.0 + (v - a.1) / (b.1 - a.1) * (b.0 - a.0));
            }
        }
    }
    at.sort_by(f64::total_cmp);
    at.chunks_exact(2)
        .map(|p| (p[0], p[1]))
        .filter(|(a, b)| b - a > 0.001)
        .collect()
}

pub(crate) fn outline(f: Outline) -> Result<Plan, String> {
    let r = f.radius as f64;
    let stock = rectangle(&f.stock);
    let ramp_angle = num(&f.op.p, "ramp_angle")
        .filter(|a| (0.1..=15.).contains(a))
        .ok_or("Facing's entry ramp angle must be between 0.1° and 15°.")?;
    let slope = ramp_angle.to_radians().tan();
    let region = centres(&f.silhouette, &stock, f.extend, r)?;
    // The cutter must reach all of the outline: on a sharp outer corner
    // that takes more than a little past it.
    if uncovered(&f.silhouette, &stock, &region, r)? {
        let (mut low, mut high) = (f.extend, f.extend.max(r));
        for _ in 0..12 {
            let mid = (low + high) / 2.;
            if uncovered(
                &f.silhouette,
                &stock,
                &centres(&f.silhouette, &stock, mid, r)?,
                r,
            )? {
                low = mid;
            } else {
                high = mid;
            }
        }
        return Err(format!(
            "Clear past outline must be at least {:.1} mm for this Ø{} cutter: less leaves stock above the part's top on its outer corners.",
            (high * 10.).ceil() / 10.,
            2. * r
        ));
    }
    let loops: Vec<Polygon> = region.clone().into();
    let loops: Vec<Polygon> = loops.into_iter().filter(|p| p.len() > 2).collect();
    if loops.is_empty() {
        return Err("The part's outline, grown by Clear past outline, is too narrow for this tool. Choose a smaller cutter or clear further past the outline.".into());
    }
    // Links stay down only along lines inside the region, a hair grown so
    // that ones from its edge to its edge count.
    let allowed: Vec<Polygon> = region
        .inflate(0.01, JoinType::Round, EndType::Polygon, 2.)
        .into();
    let within = |a: (f64, f64), b: (f64, f64)| {
        !crosses(&allowed, a, b) && inside(&allowed, ((a.0 + b.0) / 2., (a.1 + b.1) / 2.))
    };
    // Where the cutter's centre may not go: within its radius of the stock
    // it leaves, a margin beyond what it clears.
    let margin = (r * 0.01).max(0.02);
    let kept: Vec<Polygon> = difference::<Milli>(
        vec![stock.clone()],
        region.inflate(r + margin, JoinType::Round, EndType::Polygon, 2.),
        FillRule::NonZero,
    )
    .map_err(|e| e.to_string())?
    .inflate(r, JoinType::Round, EndType::Polygon, 2.)
    .into();
    // From beside the stock straight in to `v` without touching what it
    // leaves: the nearest such point, if there is one.
    let b = &f.stock;
    let off = r + 2. * margin;
    let side = |v: (f64, f64)| {
        [
            (b.min[0] as f64 - off, v.1),
            (b.max[0] as f64 + off, v.1),
            (v.0, b.min[1] as f64 - off),
            (v.0, b.max[1] as f64 + off),
        ]
        .into_iter()
        .filter(|&p| !crosses(&kept, p, v) && !inside(&kept, p))
        .min_by(|p, q| {
            (p.0 - v.0)
                .hypot(p.1 - v.1)
                .total_cmp(&(q.0 - v.0).hypot(q.1 - v.1))
        })
    };
    // The edge passes: Clipper's outer loops run counterclockwise with the
    // region on their left, and a clockwise spindle climbs with the stock
    // beyond it on its right. Each starts where it can be come to from
    // beside the stock, the shortest way of a few hundred of its points.
    let mut edges: Vec<(Polygon, Option<(f64, f64)>)> = vec![];
    for mut path in loops.clone() {
        if f.spindle_clockwise != f.climb {
            path[1..].reverse();
        }
        let every = path.len().div_ceil(400);
        let entry = (0..path.len())
            .step_by(every)
            .filter_map(|k| side(path[k]).map(|p| (k, p)))
            .min_by(|(i, p), (j, q)| {
                let (a, c) = (path[*i], path[*j]);
                (p.0 - a.0)
                    .hypot(p.1 - a.1)
                    .total_cmp(&(q.0 - c.0).hypot(q.1 - c.1))
            });
        if let Some((k, _)) = entry {
            path.rotate_left(k);
        }
        edges.push((path, entry.map(|e| e.1)));
    }
    // The rows, in a frame along them (`u`) and across them (`v`).
    let (sin, cos) = (f.direction as f64).to_radians().sin_cos();
    let to = |p: (f64, f64)| (p.0 * cos + p.1 * sin, -p.0 * sin + p.1 * cos);
    let from = |q: (f64, f64)| (q.0 * cos - q.1 * sin, q.0 * sin + q.1 * cos);
    let framed: Vec<Polygon> = loops
        .iter()
        .map(|p| p.iter().map(|&q| to(q)).collect())
        .collect();
    let (low, high) = framed
        .iter()
        .flatten()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(l, h), q| {
            (l.min(q.1), h.max(q.1))
        });
    let count = ((high - low) / f.stepover as f64).ceil().max(1.);
    if !count.is_finite() || count > crate::cam::CEILING as f64 {
        return Err(crate::cam::too_many("moves"));
    }
    let spacing = (high - low) / count;
    // Each row midway in its band: the edge pass clears beyond the first
    // and last.
    let rows: Vec<Vec<Stroke>> = (0..count as usize)
        .map(|k| {
            let v = low + spacing * (k as f64 + 0.5);
            spans(&framed, v)
                .into_iter()
                .map(|(a, b)| (from((a, v)), from((b, v))))
                .collect()
        })
        .collect();
    let segments: usize = rows.iter().map(Vec::len).sum();
    // In the order they are cut: from the edge pass's end on, the nearest
    // row end not yet cut, both ways, or the nearest row start one way.
    let forward = f.spindle_clockwise != f.climb;
    let mut order: Vec<Stroke> = Vec::with_capacity(segments);
    let mut left: Vec<(usize, usize)> = rows
        .iter()
        .enumerate()
        .flat_map(|(i, row)| (0..row.len()).map(move |j| (i, j)))
        .collect();
    let mut at = edges[edges.len() - 1].0[0];
    while !left.is_empty() {
        let distance = |p: (f64, f64)| (p.0 - at.0).hypot(p.1 - at.1);
        let (k, flip) = left
            .iter()
            .enumerate()
            .flat_map(|(k, &(i, j))| {
                let (a, b) = rows[i][j];
                let starts = if f.both_ways {
                    vec![(k, false, a), (k, true, b)]
                } else {
                    vec![(k, !forward, if forward { a } else { b })]
                };
                starts
                    .into_iter()
                    .map(|(k, flip, p)| (k, flip, distance(p)))
            })
            .min_by(|x, y| x.2.total_cmp(&y.2))
            .map(|(k, flip, _)| (k, flip))
            .unwrap();
        let (i, j) = left.swap_remove(k);
        let (a, b) = rows[i][j];
        let stroke = if flip { (b, a) } else { (a, b) };
        at = stroke.1;
        order.push(stroke);
    }
    let edge_points: usize = edges.iter().map(|e| e.0.len()).sum();
    // Down from the highest material the cutter can meet over its centres'
    // region, not over that region's box.
    let cutting_top = crate::rest::start_over(
        f.setup,
        &crate::pocket::reach(&region, f.radius),
        f.target,
        f.top,
        num(&f.op.p, "top").unwrap_or(0.),
    );
    let approach = (cutting_top + num(&f.op.p, "retract").unwrap_or(0.)).min(f.retract);
    // Avoid a duplicate final level from sub-micron normalization roundoff.
    let levels = ((cutting_top - f.target - 0.0001) / f.stepdown)
        .ceil()
        .max(1.) as usize;
    if levels.saturating_mul(segments * 4 + edge_points * (RAMP_LAPS as usize + 1))
        > crate::cam::CEILING
    {
        return Err(crate::cam::too_many("moves"));
    }
    let mut plan = Plan {
        shape: CutterShape::Flat,
        strategy: Strategy::Face,
        operation: f.op.id.clone(),
        setup: f.setup.id.clone(),
        moves: vec![],
        stock: f.stock,
        radius: f.radius,
        flute_length: f.flute_length,
        tool_length: f.tool_length,
        holder_radius: f.holder_radius,
        neck: None,
        stops: vec![],
        display_scale: f.scale,
        duration: 0.,
        target: f.target,
        levels,
        rest_top: (cutting_top < f.top - 0.0001).then_some(cutting_top),
        spindle_clockwise: f.spindle_clockwise,
        milling_direction: if f.both_ways {
            "Mixed · zigzag"
        } else if f.climb {
            "Climb"
        } else {
            "Conventional"
        },
        rings: vec![],
    };
    let first = edges[0].1.unwrap_or(edges[0].0[0]);
    let mut position = [first.0 as f32, first.1 as f32, f.clearance];
    let feed = f.feed;
    let point = |p: (f64, f64), z: f32| [p.0 as f32, p.1 as f32, z];
    crate::progress::expect(levels * (segments + edges.len()));
    let mut done = 0;
    for level in 1..=levels {
        let z = if level == levels {
            f.target
        } else {
            (cutting_top - level as f32 * f.stepdown).max(f.target)
        };
        let previous_z = (cutting_top - (level - 1) as f32 * f.stepdown).max(f.target);
        // Over the region the level before is cut: the tool comes down to
        // just over it, the first level to over the material.
        let hop = if level == 1 {
            approach
        } else {
            (previous_z + ENTRY).min(approach)
        };
        for (path, entry) in &edges {
            done += 1;
            crate::progress::step(done)?;
            let start = path[0];
            let up = [position[0], position[1], f.clearance];
            push(&mut plan, &mut position, up, Motion::Rapid, 6000., false);
            // Down beside the stock and in to the loop at depth.
            if let &Some(outside) = entry {
                for (to, motion, rate) in [
                    (point(outside, f.clearance), Motion::Rapid, 6000.),
                    (point(outside, hop), Motion::Rapid, 6000.),
                    (point(outside, z), Motion::Feed, feed * 0.25),
                    (point(start, z), Motion::Feed, feed),
                ] {
                    push(&mut plan, &mut position, to, motion, rate, false);
                }
            } else {
                push(
                    &mut plan,
                    &mut position,
                    point(start, f.clearance),
                    Motion::Rapid,
                    6000.,
                    false,
                );
                push(
                    &mut plan,
                    &mut position,
                    point(start, hop),
                    Motion::Rapid,
                    6000.,
                    false,
                );
            }
            // Into the stock round the loop at the ramp angle, unless that
            // would wind round it more than `RAMP_LAPS` times; plunged then.
            let length: f64 = path
                .iter()
                .zip(path.iter().cycle().skip(1))
                .map(|(a, b)| (b.0 - a.0).hypot(b.1 - a.1))
                .sum();
            let laps = (previous_z - z) / (length as f32 * slope).max(1e-6);
            let ramp = entry.is_none() && laps <= RAMP_LAPS;
            push(
                &mut plan,
                &mut position,
                point(start, if ramp { previous_z } else { z }),
                Motion::Feed,
                feed * 0.25,
                false,
            );
            if ramp {
                while position[2] > z + 0.00001 {
                    for &p in path.iter().skip(1).chain(std::iter::once(&path[0])) {
                        let distance = (p.0 as f32 - position[0]).hypot(p.1 as f32 - position[1]);
                        let next = (position[2] - distance * slope).max(z);
                        push(
                            &mut plan,
                            &mut position,
                            point(p, next),
                            Motion::Feed,
                            feed * 0.25,
                            true,
                        );
                    }
                }
            }
            for &p in path.iter().skip(1).chain(std::iter::once(&path[0])) {
                push(
                    &mut plan,
                    &mut position,
                    point(p, z),
                    Motion::Feed,
                    feed,
                    true,
                );
            }
        }
        // The rows, each from the edge pass to the edge pass. Down along a
        // short line inside the region; otherwise up just over the level
        // before, or to clearance where the line leaves the region, and down
        // where the edge pass has already cut.
        for &(a, b) in &order {
            done += 1;
            if done % 64 == 0 {
                crate::progress::step(done)?;
            }
            let here = (position[0] as f64, position[1] as f64);
            let short = (a.0 - here.0).hypot(a.1 - here.1) <= 2. * f.stepover as f64;
            let inner = within(here, a);
            if f.both_ways && f.keep_down && short && inner {
                push(
                    &mut plan,
                    &mut position,
                    point(a, z),
                    Motion::Feed,
                    feed,
                    false,
                );
            } else {
                // No rapid moves sideways under the stock top, as the export
                // check has it (its blend could dip into what is left):
                // there it feeds across just over the level before, or goes
                // over the stock where plunging back down from there takes
                // less time than that feed.
                let over = if approach >= f.top {
                    approach
                } else {
                    f.clearance
                };
                let across = (a.0 - here.0).hypot(a.1 - here.1) as f32 / feed;
                let (height, motion, rate) = if !inner {
                    (f.clearance, Motion::Rapid, 6000.)
                } else if hop >= f.top {
                    (hop, Motion::Rapid, 6000.)
                } else if across <= (over - z) / (feed * 0.25) {
                    (hop, Motion::Feed, feed)
                } else {
                    (over, Motion::Rapid, 6000.)
                };
                let up = [position[0], position[1], height];
                push(&mut plan, &mut position, up, Motion::Rapid, 6000., false);
                push(
                    &mut plan,
                    &mut position,
                    point(a, height),
                    motion,
                    rate,
                    false,
                );
                push(
                    &mut plan,
                    &mut position,
                    point(a, z),
                    Motion::Feed,
                    feed * 0.25,
                    false,
                );
            }
            push(
                &mut plan,
                &mut position,
                point(b, z),
                Motion::Feed,
                feed,
                true,
            );
        }
        let up = [position[0], position[1], f.clearance];
        push(&mut plan, &mut position, up, Motion::Rapid, 6000., false);
    }
    if plan.moves.is_empty() || !plan.duration.is_finite() {
        return Err("No finite facing motion could be generated from these settings.".into());
    }
    Ok(plan)
}

/// Appends the move from `position` to `to` to `plan`, timed at `rate`.
fn push(
    plan: &mut Plan,
    position: &mut [f32; 3],
    to: [f32; 3],
    motion: Motion,
    rate: f32,
    cutting_pass: bool,
) {
    let distance = (0..3)
        .map(|i| (position[i] - to[i]).powi(2))
        .sum::<f32>()
        .sqrt();
    if distance > 0.00001 {
        let end = plan.duration + distance / rate * 60.;
        plan.moves.push(Move {
            from: *position,
            to,
            motion,
            cutting_pass,
            feed: rate,
            start: plan.duration,
            end,
            a: None,
        });
        plan.duration = end;
    }
    *position = to;
}

#[cfg(test)]
mod tests {
    use crate::{
        mesh::{self, Mesh},
        model::{Machine, Operation, Setup, Strategy, Tool},
        simulation::{Motion, Plan, Simulation},
        stock, surface,
    };
    fn quad(tris: &mut Vec<[[f32; 3]; 3]>, x: f32, y: f32, xx: f32, yy: f32, z: f32) {
        tris.push([[x, y, z], [xx, y, z], [xx, yy, z]]);
        tris.push([[x, y, z], [xx, yy, z], [x, yy, z]]);
    }
    /// `tris` in stock `sides` mm over its sides and 3 mm over its top,
    /// faced over its outline 2 mm past it by a Ø`diameter` flat end mill,
    /// both ways at 40 % stepover and 2 mm stepdown, its top selected.
    fn part(tris: Vec<[[f32; 3]; 3]>, sides: f32, diameter: f32) -> (Setup, Operation, Tool, Mesh) {
        let mesh = mesh::normalize(tris).unwrap();
        let mut setup = Setup::new(Machine::new(3), "G54");
        setup
            .p
            .insert("stock_type".into(), "Relative size box".into());
        for (k, _) in stock::OFFSETS {
            setup.p.insert(k.into(), "0".into());
        }
        for k in ["stock_left", "stock_right", "stock_front", "stock_back"] {
            setup.p.insert(k.into(), sides.to_string());
        }
        setup.p.insert("stock_top".into(), "3".into());
        let mut tool = Tool::new("Facing test cutter", "Flat end mill", "2");
        tool.p.insert("diameter".into(), diameter.to_string());
        let mut op = Operation::new(Strategy::Face, tool.id.clone());
        op.geometry.clear();
        let highest = mesh
            .surfaces
            .iter()
            .map(|s| s.center[2])
            .fold(f32::NEG_INFINITY, f32::max);
        op.surfaces = Some(surface::Selection {
            mesh_fingerprint: mesh.fingerprint.clone(),
            ids: mesh
                .surfaces
                .iter()
                .enumerate()
                .filter(|(_, s)| s.normal[2] > 0.999 && (s.center[2] - highest).abs() < 0.001)
                .map(|(i, _)| i)
                .collect(),
        });
        for (k, v) in [
            ("boundary", "Model silhouette"),
            ("silhouette_extend", "2"),
            ("stepdown", "2"),
            ("stepover", "40"),
        ] {
            op.p.insert(k.into(), v.into());
        }
        (setup, op, tool, mesh)
    }
    /// An L-shaped part, its top at Z 10 over X 0–15 and a step at Z 5 over
    /// X 15–30, Y 0–15, with a Ø6 cutter (`part`).
    fn l_part(sides: f32) -> (Setup, Operation, Tool, Mesh) {
        let mut tris = vec![];
        quad(&mut tris, 0., 0., 15., 30., 10.);
        quad(&mut tris, 15., 0., 30., 15., 5.);
        quad(&mut tris, 0., 0., 15., 30., 0.);
        quad(&mut tris, 15., 0., 30., 15., 0.);
        part(tris, sides, 6.)
    }
    /// How far (x, y), from the L's origin, is outside the L; 0 inside it.
    fn outside_l(x: f32, y: f32) -> f32 {
        let d = |x0: f32, y0: f32, x1: f32, y1: f32| {
            (x0 - x)
                .max(x - x1)
                .max(0.)
                .hypot((y0 - y).max(y - y1).max(0.))
        };
        d(0., 0., 15., 30.).min(d(15., 0., 30., 15.))
    }
    /// The stock grid's cells once `plan` has run: each cell's centre from
    /// `origin` and its height.
    fn cut(plan: Plan, origin: (f32, f32)) -> (Vec<(f32, f32, f32)>, f32, f32, f32) {
        let mut sim = Simulation::new(plan);
        sim.seek(1.);
        let s = &sim.stock;
        let mut cells = vec![];
        for y in 0..s.ny {
            for x in 0..s.nx {
                cells.push((
                    s.bounds.min[0] + (x as f32 + 0.5) * s.dx - origin.0,
                    s.bounds.min[1] + (y as f32 + 0.5) * s.dy - origin.1,
                    s.heights[y * s.nx + x],
                ));
            }
        }
        (cells, sim.plan.target, s.bounds.max[2], s.dx.max(s.dy))
    }
    #[test]
    fn faces_the_parts_outline_to_its_top_in_rows_and_leaves_the_stock_around_it() {
        let (setup, op, tool, mesh) = l_part(6.);
        let plan = crate::simulation::generate(&setup, &op, &tool, &mesh).unwrap();
        assert_eq!(plan.strategy, Strategy::Face);
        assert_eq!(plan.levels, 2);
        // The part's top, wherever the setup puts it, under 3 mm of stock.
        assert!((plan.stock.max[2] - plan.target - 3.).abs() < 0.001);
        let origin = (plan.stock.min[0] + 6., plan.stock.min[1] + 6.);
        let mut rows = 0;
        for m in &plan.moves {
            assert!(m.to[2] >= plan.target - 0.001, "{m:?}");
            if m.motion == Motion::Feed && m.to[2] < plan.stock.max[2] {
                // At depth the cutter's centre stays 1 mm inside the L: its
                // edge reaches 2 mm past it.
                let (x, y) = (m.to[0] - origin.0, m.to[1] - origin.1);
                assert!(outside_l(x, y) <= 0.001, "{m:?}");
                assert!(
                    (x - 1.).min(29. - x).min(y - 1.).min(29. - y) >= -0.001,
                    "{m:?}"
                );
            }
            // Rows along X, the pass direction 0°.
            rows += usize::from(
                m.cutting_pass
                    && m.from[2] == m.to[2]
                    && (m.from[1] - m.to[1]).abs() < 1e-4
                    && (m.from[0] - m.to[0]).abs() > 5.,
            );
        }
        assert!(rows >= 2 * 12, "{rows} rows");
        let (cells, target, top, cell) = cut(plan, origin);
        let mut over = 0;
        for (x, y, height) in cells {
            let d = outside_l(x, y);
            if d == 0. {
                // Over all of the L, the step too, down to its top.
                over += 1;
                assert!((height - target).abs() < 0.01, "{x} {y}: {height}");
            } else if d > 2. + cell {
                assert_eq!(height, top, "{x} {y}");
            }
        }
        assert!(over > 1000, "{over}");
    }
    #[test]
    fn rows_link_at_depth_both_ways_and_lift_between_them_one_way() {
        let (setup, mut op, tool, mesh) = l_part(6.);
        let lifts = |op: &Operation| {
            let plan = crate::simulation::generate(&setup, op, &tool, &mesh).unwrap();
            let lifts = plan
                .moves
                .iter()
                .filter(|m| m.motion == Motion::Rapid && m.to[2] > m.from[2])
                .count();
            (plan, lifts)
        };
        let (_, both) = lifts(&op);
        // Up after each level and between loops; the rows themselves stay
        // down but where the L's corner breaks them.
        assert!(both <= 8, "{both} lifts");
        op.p.insert("facing_pattern".into(), "One way".into());
        let (plan, one) = lifts(&op);
        assert!(one > 20, "{one} lifts");
        // Every row cut the same way; the edge pass's sides along X, 1 mm
        // inside the L, go round it.
        let y0 = plan.stock.min[1] + 6.;
        let ways: Vec<f32> = plan
            .moves
            .iter()
            .filter(|m| {
                m.cutting_pass
                    && m.from[2] == m.to[2]
                    && (m.from[1] - m.to[1]).abs() < 1e-4
                    && (m.from[0] - m.to[0]).abs() > 5.
                    && [1., 14., 29.]
                        .iter()
                        .all(|y| (m.from[1] - y0 - y).abs() > 0.01)
            })
            .map(|m| (m.to[0] - m.from[0]).signum())
            .collect();
        assert!(ways.len() > 20);
        assert!(ways.iter().all(|w| *w == ways[0]), "{ways:?}");
    }
    #[test]
    fn under_the_stock_top_rows_link_at_feed_or_over_the_stock_never_by_a_rapid() {
        // The L under 3 mm of stock, faced in two levels: on the second the
        // rows link just over the first, under the stock top, where the
        // export check refuses a rapid that moves sideways. They feed across
        // there, or go over the stock where plunging back down from 1 mm
        // over it is quicker: one way, the long way back to each row.
        for (pattern, retract, fed_links, over_links) in [
            ("Both ways (zigzag)", "5", true, false),
            ("One way", "5", true, false),
            ("One way", "1", true, true),
        ] {
            let (setup, mut op, tool, mesh) = l_part(6.);
            op.p.insert("facing_pattern".into(), pattern.into());
            op.p.insert("retract".into(), retract.into());
            let plan = crate::simulation::generate(&setup, &op, &tool, &mesh).unwrap();
            assert_eq!(plan.levels, 2);
            let top = plan.stock.max[2];
            let (mut fed, mut over) = (0, 0);
            for m in &plan.moves {
                if m.from[..2] == m.to[..2] {
                    continue;
                }
                if m.from[2].min(m.to[2]) < top - 1e-4 {
                    assert_eq!(m.motion, Motion::Feed, "{pattern}: {m:?}");
                    fed += usize::from(!m.cutting_pass && m.to[2] > plan.target + 0.1);
                } else if m.motion == Motion::Rapid && (m.to[2] - top - 1.).abs() < 1e-4 {
                    over += 1;
                }
            }
            assert!(fed > 0 || !fed_links, "{pattern} {retract}: {fed}");
            assert!(over > 0 || !over_links, "{pattern} {retract}: {over}");
        }
    }
    #[test]
    fn ramps_into_stock_round_the_edge_but_comes_down_beside_a_part_flush_with_it() {
        // In 6 mm of stock all round: it ramps down round the edge pass.
        let ramped = |sides: f32| {
            let (setup, op, tool, mesh) = l_part(sides);
            let plan = crate::simulation::generate(&setup, &op, &tool, &mesh).unwrap();
            let ramps = plan
                .moves
                .iter()
                .filter(|m| m.cutting_pass && m.to[2] < m.from[2] - 1e-5)
                .count();
            (plan, ramps)
        };
        assert!(ramped(6.).1 > 0);
        // Flush with the stock: down beside it, no ramp, and the stock in
        // the L's notch more than 2 mm in is left.
        let (plan, ramps) = ramped(0.);
        assert_eq!(ramps, 0);
        let origin = (plan.stock.min[0], plan.stock.min[1]);
        let (cells, target, top, cell) = cut(plan, origin);
        for (x, y, height) in cells {
            if x < 15. || y < 15. {
                assert!((height - target).abs() < 0.01, "{x} {y}: {height}");
            } else if x > 17. + cell && y > 17. + cell {
                assert_eq!(height, top, "{x} {y}");
            }
        }
    }
    #[test]
    fn refuses_to_leave_the_parts_corners_and_says_how_far_clears_them() {
        // A 40 × 30 mm block with square corners faced by a Ø20 cutter.
        let mut tris = vec![];
        quad(&mut tris, 0., 0., 40., 30., 10.);
        quad(&mut tris, 0., 0., 40., 30., 0.);
        let (setup, mut op, tool, mesh) = part(tris, 6., 20.);
        let refused = crate::simulation::generate(&setup, &op, &tool, &mesh).unwrap_err();
        // A square corner takes (1 − 1/√2) of the radius past it: 2.93 mm.
        assert_eq!(
            refused,
            "Clear past outline must be at least 3.0 mm for this Ø20 cutter: less leaves stock above the part's top on its outer corners."
        );
        op.p.insert("silhouette_extend".into(), "3".into());
        let plan = crate::simulation::generate(&setup, &op, &tool, &mesh).unwrap();
        let origin = (plan.stock.min[0] + 6., plan.stock.min[1] + 6.);
        let (cells, target, _, _) = cut(plan, origin);
        let mut over = 0;
        for (x, y, height) in cells {
            if (0. ..40.).contains(&x) && (0. ..30.).contains(&y) {
                over += 1;
                assert!((height - target).abs() < 0.01, "{x} {y}: {height}");
            }
        }
        assert!(over > 1000, "{over}");
    }
    #[test]
    fn the_example_blocks_outline_facing_posts_and_replays_exactly_before_its_pocket() {
        let (mut doc, mesh) = crate::post::fixture::job();
        let setup = &mut doc.setups[0];
        setup.operations.truncate(2);
        assert_eq!(setup.operations[0].strategy, Strategy::Face);
        setup.operations[0]
            .p
            .insert("boundary".into(), "Model silhouette".into());
        let job = crate::post::post_job(&doc, &mesh);
        let errors: Vec<String> = job
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
        assert!(job.ready(), "{errors:?}");
        let program = &job.setups[0].result.as_ref().unwrap()[0];
        assert_eq!(program.operations.len(), 2);
        assert!(program.lines > 100, "{} lines", program.lines);
    }
}
