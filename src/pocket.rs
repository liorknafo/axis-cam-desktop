//! Bounded 2.5D pocket preview from imported planar floor triangles.
//! Offset contours preserve holes/islands. No machine postprocessor or fixture solver.
use crate::{
    mesh::Mesh,
    model::{
        Operation, Setup, Strategy, Tool, facing_setting, get, num, pocket_setting,
        pocket_setting_issues,
    },
    simulation::{Motion, Move, Plan},
    stock::{self, Placement},
    surface,
};
use clipper2::{EndType, FillRule, JoinType, Milli, Paths, intersect, union};

type Polygon = Vec<(f64, f64)>;
type Region = Paths<Milli>;
/// A loop to cut and, when the tool reaches it at depth from the loop
/// before, the points it feeds through to its start (`step`).
type Loop = (Polygon, Option<Vec<(f64, f64)>>);
const MAX_MOVES: usize = 30_000;
fn area(p: &Polygon) -> f64 {
    p.iter()
        .zip(p.iter().cycle().skip(1))
        .take(p.len())
        .map(|(a, b)| a.0 * b.1 - b.0 * a.1)
        .sum::<f64>()
        / 2.
}
fn region(triangles: Vec<Polygon>) -> Result<Region, String> {
    union::<Milli>(triangles, Vec::<Polygon>::new(), FillRule::NonZero)
        .map_err(|e| format!("Could not join pocket floor triangles: {e}"))
}
fn number(op: &Operation, key: &str) -> Result<f32, String> {
    num(&op.p, key).ok_or_else(|| format!("Enter a finite {} value.", key.replace('_', " ")))
}
fn positive(tool: &Tool, key: &str) -> Result<f32, String> {
    num(&tool.p, key)
        .filter(|v| *v > 0.)
        .ok_or_else(|| format!("Enter a positive tool {key}."))
}
/// Clip a triangle to the half-space above the floor before projecting it.
fn above(points: &[[f32; 3]], z: f32) -> Polygon {
    let mut result = Vec::new();
    for (a, b) in points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .take(points.len())
    {
        if a[2] >= z {
            result.push((a[0] as f64, a[1] as f64));
        }
        if (a[2] >= z) != (b[2] >= z) {
            let t = (z - a[2]) / (b[2] - a[2]);
            result.push((
                (a[0] + t * (b[0] - a[0])) as f64,
                (a[1] + t * (b[1] - a[1])) as f64,
            ));
        }
    }
    result
}
/// Whether `p` is inside `region` (even-odd over all its paths).
fn inside(region: &[Polygon], p: (f64, f64)) -> bool {
    let mut odd = false;
    for path in region {
        for (a, b) in path
            .iter()
            .zip(path.iter().cycle().skip(1))
            .take(path.len())
        {
            if (a.1 > p.1) != (b.1 > p.1) && p.0 < a.0 + (p.1 - a.1) / (b.1 - a.1) * (b.0 - a.0) {
                odd = !odd;
            }
        }
    }
    odd
}
/// Whether the segment `a`–`b` crosses or touches an edge of `region`.
fn crosses(region: &[Polygon], a: (f64, f64), b: (f64, f64)) -> bool {
    let turn = |p: (f64, f64), q: (f64, f64), r: (f64, f64)| {
        (q.0 - p.0) * (r.1 - p.1) - (q.1 - p.1) * (r.0 - p.0)
    };
    // `r`, on the line through `p` and `q`, lies between them.
    let between = |p: (f64, f64), q: (f64, f64), r: (f64, f64)| {
        r.0 >= p.0.min(q.0) && r.0 <= p.0.max(q.0) && r.1 >= p.1.min(q.1) && r.1 <= p.1.max(q.1)
    };
    region.iter().any(|path| {
        path.iter()
            .zip(path.iter().cycle().skip(1))
            .take(path.len())
            .any(|(&c, &d)| {
                let (d1, d2) = (turn(c, d, a), turn(c, d, b));
                let (d3, d4) = (turn(a, b, c), turn(a, b, d));
                (d1 * d2 < 0. && d3 * d4 < 0.)
                    || (d1 == 0. && between(c, d, a))
                    || (d2 == 0. && between(c, d, b))
                    || (d3 == 0. && between(a, b, c))
                    || (d4 == 0. && between(a, b, d))
            })
    })
}
/// The point of the closed `path` nearest `p`: its distance, the point, and
/// the edge it lies on (from vertex `edge` to the next).
fn nearest(path: &[(f64, f64)], p: (f64, f64)) -> (f64, (f64, f64), usize) {
    let n = path.len();
    let mut best = (f64::INFINITY, p, 0);
    for i in 0..n {
        let (c, d) = (path[i], path[(i + 1) % n]);
        let (dx, dy) = (d.0 - c.0, d.1 - c.1);
        let length = dx * dx + dy * dy;
        let t = if length > 0. {
            (((p.0 - c.0) * dx + (p.1 - c.1) * dy) / length).clamp(0., 1.)
        } else {
            0.
        };
        let q = (c.0 + t * dx, c.1 + t * dy);
        let distance = (q.0 - p.0).hypot(q.1 - p.1);
        if distance < best.0 {
            best = (distance, q, i);
        }
    }
    best
}
/// The closed `path` begun at `at`, a point on its edge `edge`.
fn begun(path: &[(f64, f64)], at: (f64, f64), edge: usize) -> Polygon {
    let n = path.len();
    let mut out = vec![at];
    out.extend((1..=n).map(|k| path[(edge + k) % n]));
    out
}
/// How the tool gets from the end of the loop `from`, just cut, to the loop
/// `to` without leaving the depth: along `from`, whose cut floor it is
/// already on, either way round, to the point nearest `to`, then one step
/// no longer than `reach` along a line inside `allowed` that touches none of
/// its edges. The shortest such way: the points to feed through, the last
/// `to`'s start, and `to` begun there. None when there is none.
fn step(
    from: &[(f64, f64)],
    to: &[(f64, f64)],
    reach: f64,
    allowed: &[Polygon],
) -> Option<(Vec<(f64, f64)>, Polygon)> {
    let n = from.len();
    // Distance along `from` from its start to each vertex.
    let mut along = vec![0.];
    for i in 0..n {
        let (c, d) = (from[i], from[(i + 1) % n]);
        along.push(along[i] + (d.0 - c.0).hypot(d.1 - c.1));
    }
    let around = along[n];
    // Pairs of a point on `from` (on its edge) and one on `to` (on its
    // edge): each vertex with the nearest point of the other loop.
    let mut pairs = vec![];
    for (i, &p) in from.iter().enumerate() {
        let (d, q, e) = nearest(to, p);
        pairs.push((d, p, i, q, e));
    }
    for (j, &q) in to.iter().enumerate() {
        let (d, p, e) = nearest(from, q);
        pairs.push((d, p, e, q, j));
    }
    let mut ways: Vec<_> = pairs
        .into_iter()
        .filter(|w| w.0 <= reach)
        .map(|(d, p, e, q, f)| {
            let s = along[e] + (p.0 - from[e].0).hypot(p.1 - from[e].1);
            let forward = s <= around - s;
            (s.min(around - s) + d, forward, p, e, q, f)
        })
        .collect();
    ways.sort_by(|a, b| a.0.total_cmp(&b.0));
    let (_, forward, p, e, q, f) = ways
        .into_iter()
        .find(|w| inside(allowed, w.2) && !crosses(allowed, w.2, w.4))?;
    let mut points: Vec<(f64, f64)> = if forward {
        from[1..=e].to_vec()
    } else {
        from[e + 1..].iter().rev().copied().collect()
    };
    points.extend([p, q]);
    Some((points, begun(to, q, f)))
}
/// The least distance between the closed paths `a` and `b`.
fn gap(a: &[(f64, f64)], b: &[(f64, f64)]) -> f64 {
    let to = |p: (f64, f64), path: &[(f64, f64)]| {
        path.iter()
            .zip(path.iter().cycle().skip(1))
            .take(path.len())
            .map(|(&c, &d)| {
                let (dx, dy) = (d.0 - c.0, d.1 - c.1);
                let length = dx * dx + dy * dy;
                let t = if length > 0. {
                    (((p.0 - c.0) * dx + (p.1 - c.1) * dy) / length).clamp(0., 1.)
                } else {
                    0.
                };
                (c.0 + t * dx - p.0).hypot(c.1 + t * dy - p.1)
            })
            .fold(f64::INFINITY, f64::min)
    };
    let one = a.iter().map(|&p| to(p, b)).fold(f64::INFINITY, f64::min);
    b.iter().map(|&p| to(p, a)).fold(one, f64::min)
}
/// The loops of `rings` (outermost first) in cutting order, each with how
/// the tool reaches it at depth from the loop before (`step`), None where it
/// retracts and enters again. A loop waits for every loop of the next ring
/// inward within `reach` of it, so it cuts beside cleared floor. Of the
/// loops ready, the next is the one the tool reaches at depth by the
/// shortest way; else the deepest, nearest. Each outermost loop is then cut
/// `finish` times more.
fn ordered(rings: Vec<Vec<Polygon>>, finish: usize, reach: f64, allowed: &[Polygon]) -> Vec<Loop> {
    let paths: Vec<(usize, Polygon)> = rings
        .into_iter()
        .enumerate()
        .flat_map(|(ring, paths)| paths.into_iter().map(move |p| (ring, p)))
        .collect();
    let bounds: Vec<[f64; 4]> = paths
        .iter()
        .map(|(_, p)| {
            p.iter().fold(
                [
                    f64::INFINITY,
                    f64::INFINITY,
                    f64::NEG_INFINITY,
                    f64::NEG_INFINITY,
                ],
                |b, q| [b[0].min(q.0), b[1].min(q.1), b[2].max(q.0), b[3].max(q.1)],
            )
        })
        .collect();
    let close = |a: [f64; 4], b: [f64; 4]| {
        a[0] - reach <= b[2] && b[0] - reach <= a[2] && a[1] - reach <= b[3] && b[1] - reach <= a[3]
    };
    let waits: Vec<Vec<usize>> = (0..paths.len())
        .map(|i| {
            (0..paths.len())
                .filter(|&j| {
                    paths[j].0 == paths[i].0 + 1
                        && close(bounds[i], bounds[j])
                        && gap(&paths[i].1, &paths[j].1) <= reach
                })
                .collect()
        })
        .collect();
    let mut done = vec![false; paths.len()];
    let mut out: Vec<Loop> = vec![];
    // The loop just cut and its bounds.
    let mut last: Option<(Polygon, [f64; 4])> = None;
    while done.iter().any(|d| !d) {
        let ready: Vec<usize> = (0..paths.len())
            .filter(|&i| !done[i] && waits[i].iter().all(|&j| done[j]))
            .collect();
        let way = last.as_ref().and_then(|(from, around)| {
            ready
                .iter()
                .filter(|&&i| close(*around, bounds[i]))
                .filter_map(|&i| {
                    let (points, path) = step(from, &paths[i].1, reach, allowed)?;
                    let length: f64 = std::iter::once(from[0])
                        .chain(points.iter().copied())
                        .collect::<Vec<_>>()
                        .windows(2)
                        .map(|w| (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1))
                        .sum();
                    Some((length, i, points, path))
                })
                .min_by(|a, b| a.0.total_cmp(&b.0))
        });
        let (i, path, points) = match way {
            Some((_, i, points, path)) => (i, path, Some(points)),
            None => {
                let here = last.as_ref().map(|(p, _)| p[0]);
                let distance = |i: usize| {
                    here.map_or(0., |h| {
                        paths[i]
                            .1
                            .iter()
                            .map(|q| (q.0 - h.0).hypot(q.1 - h.1))
                            .fold(f64::INFINITY, f64::min)
                    })
                };
                // `ready` is never empty: the deepest loops wait for none.
                let i = *ready
                    .iter()
                    .min_by(|&&a, &&b| {
                        paths[b]
                            .0
                            .cmp(&paths[a].0)
                            .then(distance(a).total_cmp(&distance(b)))
                    })
                    .unwrap();
                (i, paths[i].1.clone(), None)
            }
        };
        done[i] = true;
        out.push((path.clone(), points));
        if paths[i].0 == 0 {
            for _ in 0..finish {
                out.push((path.clone(), Some(vec![])));
            }
        }
        last = Some((path, bounds[i]));
    }
    out
}
fn push(
    plan: &mut Plan,
    position: &mut [f32; 3],
    to: [f32; 3],
    motion: Motion,
    rate: f32,
    cutting: bool,
) -> Result<(), String> {
    if plan.moves.len() >= MAX_MOVES {
        return Err(
            "Pocket preview exceeds 30,000 moves. Increase stepover, stepdown or ramp angle."
                .into(),
        );
    }
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
            cutting_pass: cutting,
            feed: rate,
            start: plan.duration,
            end,
        });
        plan.duration = end;
    }
    *position = to;
    Ok(())
}

pub fn generate(setup: &Setup, op: &Operation, tool: &Tool, mesh: &Mesh) -> Result<Plan, String> {
    if !op.enabled {
        return Err("Enable this Pocket operation before simulation.".into());
    }
    if let Some(issue) = pocket_setting_issues(&op.p).first() {
        return Err(issue.clone());
    }
    if let Some(issue) = surface::issues(op, setup, Some(mesh)).first() {
        return Err(issue.clone());
    }
    let selection = op
        .surfaces
        .as_ref()
        .filter(|s| !s.ids.is_empty())
        .ok_or("Select a recessed planar pocket floor in Geometry first.")?;
    if get(&tool.p, "type") != "Flat end mill" {
        return Err("Pocket preview requires a flat end mill.".into());
    }
    if get(&setup.p, "stock_source") != "fresh" {
        return Err("Pocket preview currently uses fresh stock. Stock transfer between setups is not implemented.".into());
    }
    if get(&op.p, "boundary") != "Selected geometry"
        || get(&op.p, "containment") != "Tool inside boundary"
    {
        return Err("Pocket preview requires Selected geometry with Tool inside boundary. Its walls and islands come from the selected floor.".into());
    }
    let stock = stock::resolve(setup, Some(mesh))?;
    let placement = Placement::new(mesh, Some(setup));
    let scale = stock.display_scale;
    let ids: std::collections::HashSet<_> = selection.ids.iter().copied().collect();
    let reference = mesh
        .faces
        .iter()
        .find(|f| f.feature.is_some_and(|id| ids.contains(&id)))
        .and_then(|f| f.points.first())
        .ok_or("The selected floor contains no triangles.")?;
    let plane = placement.transform(*reference)[2] / scale;
    if plane >= placement.bounds.max[2] / scale - 0.001 {
        return Err("Select a recessed pocket floor. Use Facing for the top surface.".into());
    }
    let axial = number(op, "axial_allowance")?;
    let radial = number(op, "allowance")?;
    let target = plane + number(op, "bottom")? + axial;
    let top = stock.stock.max[2].max(stock.stock.max[2] + number(op, "top")?);
    if axial < 0. || radial < 0. || target < plane - 0.0001 {
        return Err("Pocket target cannot cut below the selected floor; use nonnegative stock allowances and review Bottom offset.".into());
    }
    if target < stock.stock.min[2] || target >= stock.stock.max[2] - 0.0001 {
        return Err("No pocket stock exists above the requested target. Review stock and bottom/axial offsets.".into());
    }
    let radius = positive(tool, "diameter")? / 2.;
    let flute_length = positive(tool, "length")?;
    let tool_length = positive(tool, "stickout")?;
    let holder_radius = positive(tool, "holder")? / 2.;
    if tool_length < flute_length || top - target >= tool_length {
        return Err("Pocket depth must be less than tool stickout, with stickout at least the flute length. Choose a tool with adequate reach.".into());
    }
    let feed = number(op, "feed")?;
    let stepdown = number(op, "stepdown")?;
    let stepover = number(op, "stepover")? / 100. * radius * 2.;
    if feed <= 0.
        || stepdown <= 0.
        || stepdown > flute_length
        || stepover <= 0.
        || stepover > radius
    {
        return Err("Use positive feed/step sizes, stepdown within flute length, and Pocket stepover up to 50% for overlapping offset contours.".into());
    }
    let clearance = top + number(op, "clearance")?;
    let retract = top + number(op, "retract")?;
    if retract <= top || clearance < retract {
        return Err("Set positive retract and clearance at or above retract.".into());
    }
    let finish = number(op, "finish_passes")?;
    if !(0. ..=20.).contains(&finish) || finish.fract() != 0. {
        return Err("Finishing passes must be a whole number from 0 to 20.".into());
    }
    let ramp = pocket_setting(&op.p, "pocket_entry") == "Profile ramp";
    let ramp_slope = if ramp {
        let angle = number(op, "ramp_angle")?;
        if !(0.1..=15.).contains(&angle) {
            return Err("Pocket profile ramp angle must be between 0.1° and 15°.".into());
        }
        angle.to_radians().tan()
    } else {
        0.
    };

    let mut floors = Vec::new();
    let mut obstacles = Vec::new();
    for face in &mesh.faces {
        let points: Vec<_> = face
            .points
            .iter()
            .map(|p| placement.transform(*p).map(|v| v / scale))
            .collect();
        let selected = face.feature.is_some_and(|id| ids.contains(&id));
        if selected && points.iter().any(|p| (p[2] - plane).abs() > 0.002) {
            return Err("Pocket preview needs a flat floor perpendicular to setup Z. Reposition the part or select another floor.".into());
        }
        let mut p: Polygon = if selected {
            points.iter().map(|p| (p[0] as f64, p[1] as f64)).collect()
        } else {
            above(&points, plane + 0.01)
        };
        let a = area(&p);
        if a.abs() < 0.000001 {
            continue;
        }
        if a < 0. {
            p.reverse();
        }
        if selected {
            floors.push(p);
        } else {
            obstacles.push(p);
        }
    }
    let floor = region(floors)?;
    if !floor.contains_points() {
        return Err("The selected floor has no usable XY area.".into());
    }
    // Reject overhanging/tapered geometry rather than assuming vertical access.
    let blocked = intersect::<Milli>(floor.clone(), region(obstacles)?, FillRule::NonZero)
        .map_err(|e| e.to_string())?;
    if blocked.signed_area().abs() > 0.05 {
        return Err("Geometry above this floor obstructs vertical access. Pocket preview supports open, vertical-wall 2.5D pockets; choose another floor or setup.".into());
    }
    // A small conservative margin covers integer rounding and offset arc chords.
    let margin = (radius * 0.01).max(0.02);
    let inset = (radius + radial + margin) as f64;
    let boundary = floor
        .inflate(-inset, JoinType::Round, EndType::Polygon, 2.)
        .simplify(0.001, false);
    if !boundary.contains_points() {
        return Err("The selected pocket is too narrow for this tool and radial allowance. Choose a smaller cutter.".into());
    }
    // Where the tool's centre may step over at depth: clear of every wall and
    // island, half the margin inside the loops' own limit, so a loop on
    // that limit is still inside it.
    let allowed: Vec<Polygon> = floor
        .inflate(
            -(inset - margin as f64 / 2.),
            JoinType::Round,
            EndType::Polygon,
            2.,
        )
        .simplify(0.001, false)
        .into();
    let reach = stepover as f64 * 1.5;
    let profile_only = op.strategy == Strategy::Contour;
    let mut rings: Vec<Vec<Polygon>> = Vec::new();
    for ring in 0..=if profile_only { 0 } else { 256 } {
        let paths = floor
            .inflate(
                -(inset + ring as f64 * stepover as f64),
                JoinType::Round,
                EndType::Polygon,
                2.,
            )
            .simplify(0.001, false);
        if !paths.contains_points() {
            break;
        }
        if ring == 256 {
            return Err(
                "Pocket preview exceeds 256 offset rings. Increase stepover or reduce pocket size."
                    .into(),
            );
        }
        rings.push(paths.into());
    }
    let mut envelope = crate::mesh::Bounds {
        min: [f32::INFINITY; 3],
        max: [f32::NEG_INFINITY; 3],
    };
    for point in rings.iter().flatten().flatten() {
        for (k, v) in [point.0 as f32, point.1 as f32].into_iter().enumerate() {
            envelope.min[k] = envelope.min[k].min(v - radius);
            envelope.max[k] = envelope.max[k].max(v + radius);
        }
    }
    let cutting_top = crate::rest::start(setup, envelope, target, top, number(op, "top")?);
    let approach = (cutting_top + number(op, "retract")?).min(retract);
    let levels = ((cutting_top - target - 0.0001) / stepdown).ceil().max(1.) as usize;
    let segments: usize = rings.iter().flatten().map(|p| p.len() + 5).sum();
    if levels > 1000 || segments.saturating_mul(levels) > MAX_MOVES {
        return Err("Pocket preview exceeds its move limit. Increase stepdown or stepover.".into());
    }
    let clockwise = facing_setting(&op.p, "spindle_rotation") == "Clockwise (CW)";
    let climb = pocket_setting(&op.p, "pocket_milling") == "Climb";
    // Clipper outer boundaries are CCW and islands CW (free area on left).
    // CW climb keeps remaining material on the right of cutter travel.
    if clockwise != climb {
        // Each loop keeps its start, so both directions cut in one order.
        for p in rings.iter_mut().flatten() {
            p[1..].reverse();
        }
    }
    let loops = ordered(rings, finish as usize, reach, &allowed);
    let mut plan = Plan {
        shape: crate::simulation::CutterShape::Flat,
        strategy: op.strategy,
        operation: op.id.clone(),
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
        levels,
        rest_top: (cutting_top < top - 0.0001).then_some(cutting_top),
        spindle_clockwise: clockwise,
        milling_direction: if climb { "Climb" } else { "Conventional" },
    };
    let mut position = [loops[0].0[0].0 as f32, loops[0].0[0].1 as f32, clearance];
    for level in 1..=levels {
        let z = (cutting_top - level as f32 * stepdown).max(target);
        let previous_z = (cutting_top - (level - 1) as f32 * stepdown).max(target);
        for (i, (path, way)) in loops.iter().enumerate() {
            if let Some(points) = way {
                // To the next loop at depth: along the floor just cut, then
                // a short step clear of every wall and island (`step`). The
                // step takes up to the cutter's whole front edge, so it feeds
                // as an entry does.
                for (k, point) in points.iter().enumerate() {
                    let rate = if k + 1 == points.len() {
                        feed * 0.25
                    } else {
                        feed
                    };
                    push(
                        &mut plan,
                        &mut position,
                        [point.0 as f32, point.1 as f32, z],
                        Motion::Feed,
                        rate,
                        false,
                    )?;
                }
            } else {
                if i > 0 {
                    let above = [position[0], position[1], clearance];
                    push(&mut plan, &mut position, above, Motion::Rapid, 6000., false)?;
                }
                let start = [path[0].0 as f32, path[0].1 as f32];
                push(
                    &mut plan,
                    &mut position,
                    [start[0], start[1], clearance],
                    Motion::Rapid,
                    6000.,
                    false,
                )?;
                push(
                    &mut plan,
                    &mut position,
                    [start[0], start[1], approach],
                    Motion::Rapid,
                    6000.,
                    false,
                )?;
                push(
                    &mut plan,
                    &mut position,
                    [start[0], start[1], if ramp { previous_z } else { z }],
                    Motion::Feed,
                    feed * 0.25,
                    false,
                )?;
                if ramp {
                    while position[2] > z + 0.00001 {
                        for point in path.iter().skip(1).chain(std::iter::once(&path[0])) {
                            let xy = [point.0 as f32, point.1 as f32];
                            let distance = ((xy[0] - position[0]).powi(2)
                                + (xy[1] - position[1]).powi(2))
                            .sqrt();
                            let next_z = (position[2] - distance * ramp_slope).max(z);
                            push(
                                &mut plan,
                                &mut position,
                                [xy[0], xy[1], next_z],
                                Motion::Feed,
                                feed * 0.25,
                                true,
                            )?;
                        }
                    }
                }
            }
            for point in path.iter().skip(1).chain(std::iter::once(&path[0])) {
                push(
                    &mut plan,
                    &mut position,
                    [point.0 as f32, point.1 as f32, z],
                    Motion::Feed,
                    feed,
                    true,
                )?;
            }
        }
        let above = [position[0], position[1], clearance];
        push(&mut plan, &mut position, above, Motion::Rapid, 6000., false)?;
    }
    if plan.moves.is_empty() || !plan.duration.is_finite() {
        return Err("No finite pocket motion could be generated.".into());
    }
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{mesh, model::Machine, simulation::Simulation};
    fn quad(tris: &mut Vec<[[f32; 3]; 3]>, x: f32, y: f32, xx: f32, yy: f32, z: f32) {
        tris.push([[x, y, z], [xx, y, z], [xx, yy, z]]);
        tris.push([[x, y, z], [xx, yy, z], [x, yy, z]]);
    }
    fn fixture() -> (Setup, Operation, Tool, Mesh) {
        let mut tris = Vec::new();
        // Floor ring, central island, enclosing rim and lower model bound.
        for (x, y, xx, yy) in [
            (-10., -8., 10., -2.),
            (-10., 2., 10., 8.),
            (-10., -2., -2., 2.),
            (2., -2., 10., 2.),
        ] {
            quad(&mut tris, x, y, xx, yy, 0.);
        }
        for (x, y, xx, yy) in [
            (-15., -12., 15., -8.),
            (-15., 8., 15., 12.),
            (-15., -8., -10., 8.),
            (10., -8., 15., 8.),
            (-2., -2., 2., 2.),
        ] {
            quad(&mut tris, x, y, xx, yy, 6.);
        }
        quad(&mut tris, -15., -12., 15., 12., -4.);
        let mesh = mesh::normalize(tris).unwrap();
        let mut setup = Setup::new(Machine::new(3), "G54");
        setup
            .p
            .insert("stock_type".into(), "Relative size box".into());
        for (k, _) in stock::OFFSETS {
            setup.p.insert(k.into(), "0".into());
        }
        let mut tool = Tool::new("Pocket test cutter", "Flat end mill", "2");
        tool.p.insert("diameter".into(), "2".into());
        let mut op = Operation::new(Strategy::Pocket, tool.id.clone());
        op.geometry.clear();
        let center_z = mesh.surfaces[0].center[2];
        op.surfaces = Some(surface::Selection {
            mesh_fingerprint: mesh.fingerprint.clone(),
            ids: mesh
                .surfaces
                .iter()
                .enumerate()
                .filter(|(_, s)| (s.center[2] - center_z).abs() < 0.001)
                .map(|(i, _)| i)
                .collect(),
        });
        for (k, v) in [
            ("allowance", "0"),
            ("axial_allowance", "0"),
            ("stepover", "40"),
            ("pocket_entry", "Plunge"),
            ("finish_passes", "0"),
        ] {
            op.p.insert(k.into(), v.into());
        }
        (setup, op, tool, mesh)
    }
    fn loops(plan: &Plan) -> Vec<Polygon> {
        let mut loops = Vec::new();
        let mut current = Vec::new();
        for m in &plan.moves {
            if m.cutting_pass {
                if current.is_empty() {
                    current.push((m.from[0] as f64, m.from[1] as f64));
                }
                current.push((m.to[0] as f64, m.to[1] as f64));
            } else if !current.is_empty() {
                loops.push(std::mem::take(&mut current));
            }
        }
        loops
    }
    #[test]
    fn inside_contour_follows_walls_without_clearing_the_pocket_interior() {
        let (setup, mut op, tool, mesh) = fixture();
        let pocket = generate(&setup, &op, &tool, &mesh).unwrap();
        op.strategy = Strategy::Contour;
        op.p.insert("side".into(), "Inside".into());
        let plan = crate::simulation::generate(&setup, &op, &tool, &mesh).unwrap();
        assert_eq!(plan.strategy, Strategy::Contour);
        assert!(plan.moves.len() < pocket.moves.len());
        assert_eq!(plan.target, pocket.target);
        let mut contour_sim = Simulation::new(plan.clone());
        contour_sim.seek(1.);
        let mut pocket_sim = Simulation::new(pocket);
        pocket_sim.seek(1.);
        assert!(contour_sim.stock.removed_volume() < pocket_sim.stock.removed_volume());
        let paths = loops(&plan);
        assert!(paths.iter().any(|p| area(p) > 0.) && paths.iter().any(|p| area(p) < 0.));
        op.p.insert("pocket_milling".into(), "Conventional".into());
        let reversed = crate::simulation::generate(&setup, &op, &tool, &mesh).unwrap();
        for (a, b) in paths.iter().zip(loops(&reversed)) {
            assert!((area(a) + area(&b)).abs() < 0.001);
        }
        op.surfaces = None;
        assert!(crate::simulation::generate(&setup, &op, &tool, &mesh).is_err());
    }
    #[test]
    fn pocket_preserves_walls_island_and_floor_and_removes_known_volume() {
        let (setup, op, tool, mesh) = fixture();
        let plan = generate(&setup, &op, &tool, &mesh).unwrap();
        assert_eq!(plan.levels, 3);
        for pair in plan.moves.windows(2) {
            assert_eq!(pair[0].to, pair[1].from);
            assert_eq!(pair[0].end, pair[1].start);
        }
        for m in &plan.moves {
            if m.motion == Motion::Rapid && m.from[..2] != m.to[..2] {
                assert!(m.from[2] > plan.stock.max[2] && m.to[2] > plan.stock.max[2]);
            }
            if m.motion == Motion::Feed {
                for i in 0..=10 {
                    let p = m.at(m.start + (m.end - m.start) * i as f32 / 10.);
                    assert!(
                        p[0].abs() + plan.radius <= 10. + 0.001
                            && p[1].abs() + plan.radius <= 8. + 0.001
                    );
                    let dx = (p[0].abs() - 2.).max(0.);
                    let dy = (p[1].abs() - 2.).max(0.);
                    assert!(dx * dx + dy * dy >= plan.radius * plan.radius);
                    assert!(p[2] >= plan.target - 0.001);
                }
            }
        }
        let mut sim = Simulation::new(plan);
        sim.seek(1.);
        let expected = (20. * 16. - 4. * 4.) * 6.;
        assert!(
            (sim.stock.removed_volume() - expected).abs() / expected < 0.04,
            "{} vs {expected}",
            sim.stock.removed_volume()
        );
        for y in 0..sim.stock.ny {
            for x in 0..sim.stock.nx {
                let px = sim.stock.bounds.min[0] + (x as f32 + 0.5) * sim.stock.dx;
                let py = sim.stock.bounds.min[1] + (y as f32 + 0.5) * sim.stock.dy;
                if px.abs() > 10. || py.abs() > 8. || (px.abs() < 2. && py.abs() < 2.) {
                    assert_eq!(
                        sim.stock.heights[y * sim.stock.nx + x],
                        sim.stock.bounds.max[2]
                    );
                }
            }
        }
    }
    #[test]
    fn pocket_direction_and_spindle_reverse_loops_and_keep_final_stock_equal() {
        let (setup, mut op, tool, mesh) = fixture();
        let cw = generate(&setup, &op, &tool, &mesh).unwrap();
        let paths = loops(&cw);
        assert!(paths.iter().any(|p| area(p) > 0.) && paths.iter().any(|p| area(p) < 0.));
        op.p.insert("pocket_milling".into(), "Conventional".into());
        let conventional = generate(&setup, &op, &tool, &mesh).unwrap();
        for (a, b) in paths.iter().zip(loops(&conventional)) {
            assert!((area(a) + area(&b)).abs() < 0.001);
        }
        op.p.insert("spindle_rotation".into(), "Counterclockwise (CCW)".into());
        let ccw = generate(&setup, &op, &tool, &mesh).unwrap();
        assert_eq!(loops(&ccw), paths);
        let mut expected = None;
        for plan in [cw, conventional, ccw] {
            let mut sim = Simulation::new(plan);
            sim.seek(1.);
            if let Some(ref stock) = expected {
                assert_eq!(&sim.stock.heights, stock);
            } else {
                expected = Some(sim.stock.heights.clone());
            }
        }
    }
    /// The moves that step over between loops at depth: feed moves that
    /// are not part of a loop and stay at one height.
    fn links(plan: &Plan) -> Vec<&Move> {
        plan.moves
            .iter()
            .filter(|m| m.motion == Motion::Feed && !m.cutting_pass && m.from[2] == m.to[2])
            .collect()
    }
    /// The entries: feed moves down that are not part of a loop.
    fn entries(plan: &Plan) -> usize {
        plan.moves
            .iter()
            .filter(|m| m.motion == Motion::Feed && !m.cutting_pass && m.to[2] < m.from[2])
            .count()
    }
    #[test]
    fn a_profile_ramp_steps_over_at_depth_clear_of_the_walls_and_the_island() {
        let (setup, mut op, tool, mesh) = fixture();
        op.p.insert("pocket_entry".into(), "Profile ramp".into());
        op.p.insert("finish_passes".into(), "1".into());
        let plan = generate(&setup, &op, &tool, &mesh).unwrap();
        let loops = loops(&plan).len();
        let steps = links(&plan);
        // Each level ramps in where it can, then steps from ring to ring
        // (and into the finish pass) without leaving the floor.
        assert!(!steps.is_empty());
        assert!(entries(&plan) + steps.len() >= loops, "{loops} loops");
        assert!(
            entries(&plan) < loops / 2,
            "{} entries, {loops} loops",
            entries(&plan)
        );
        for m in &steps {
            assert!(m.to[2] < plan.stock.max[2]);
        }
        for m in plan.moves.iter().filter(|m| m.motion == Motion::Feed) {
            for i in 0..=20 {
                let p = m.at(m.start + (m.end - m.start) * i as f32 / 20.);
                assert!(
                    p[0].abs() + plan.radius <= 10. + 0.001
                        && p[1].abs() + plan.radius <= 8. + 0.001,
                    "{p:?}"
                );
                let dx = (p[0].abs() - 2.).max(0.);
                let dy = (p[1].abs() - 2.).max(0.);
                assert!(
                    dx * dx + dy * dy >= plan.radius * plan.radius - 0.001,
                    "{p:?}"
                );
            }
        }
        let mut sim = Simulation::new(plan);
        sim.seek(1.);
        let expected = (20. * 16. - 4. * 4.) * 6.;
        assert!((sim.stock.removed_volume() - expected).abs() / expected < 0.04);
    }
    #[test]
    fn a_plate_of_square_islands_ramps_in_at_2_degrees_without_ramping_every_ring() {
        // A 70 × 70 mm floor 6 mm down with a 3 × 3 grid of 14 mm square
        // islands 5 mm apart, as a keyboard plate's switch holes stand
        // before they are cut through.
        let mut tris = Vec::new();
        let cells = [-24.5_f32, -5.5, 13.5];
        // Floor: every cell of the grid lines not covered by an island.
        let edges: Vec<f32> = {
            let mut e = vec![-35_f32];
            for c in cells {
                e.extend([c, c + 14.]);
            }
            e.push(35.);
            e
        };
        for (i, w) in edges.windows(2).enumerate() {
            for (j, h) in edges.windows(2).enumerate() {
                let island = i % 2 == 1 && j % 2 == 1;
                quad(
                    &mut tris,
                    w[0],
                    h[0],
                    w[1],
                    h[1],
                    if island { 6. } else { 0. },
                );
            }
        }
        for (x, y, xx, yy) in [
            (-40., -40., 40., -35.),
            (-40., 35., 40., 40.),
            (-40., -35., -35., 35.),
            (35., -35., 40., 35.),
        ] {
            quad(&mut tris, x, y, xx, yy, 6.);
        }
        quad(&mut tris, -40., -40., 40., 40., -4.);
        let mesh = mesh::normalize(tris).unwrap();
        let (setup, mut op, mut tool, _) = fixture();
        tool.p.insert("diameter".into(), "3".into());
        // The floor: the faces looking up between the bottom and the top.
        let heights: Vec<f32> = mesh.surfaces.iter().map(|s| s.center[2]).collect();
        let (bottom, top) = heights
            .iter()
            .fold((f32::INFINITY, f32::NEG_INFINITY), |(a, b), &z| {
                (a.min(z), b.max(z))
            });
        op.surfaces = Some(surface::Selection {
            mesh_fingerprint: mesh.fingerprint.clone(),
            ids: mesh
                .surfaces
                .iter()
                .enumerate()
                .filter(|(_, s)| {
                    s.normal[2] > 0.99 && s.center[2] > bottom + 0.001 && s.center[2] < top - 0.001
                })
                .map(|(i, _)| i)
                .collect(),
        });
        for (k, v) in [
            ("pocket_entry", "Profile ramp"),
            ("ramp_angle", "2"),
            ("stepdown", "2"),
            ("allowance", "0.3"),
        ] {
            op.p.insert(k.into(), v.into());
        }
        let plan = generate(&setup, &op, &tool, &mesh).unwrap();
        assert_eq!(plan.levels, 3);
        let loops = loops(&plan).len();
        // Not an entry for every loop: the tool stays down from loop to loop
        // and enters again only for the small loops where the webs cross and
        // in the corners, each the deepest of its spot.
        assert!(
            entries(&plan) * 3 < loops * 2,
            "{} entries, {loops} loops",
            entries(&plan)
        );
        assert!(!links(&plan).is_empty());
        let mut sim = Simulation::new(plan);
        sim.seek(1.);
        // The middle of each island still stands at the stock top.
        for c in cells {
            for d in cells {
                let x = ((c + 7. - sim.stock.bounds.min[0]) / sim.stock.dx) as usize;
                let y = ((d + 7. - sim.stock.bounds.min[1]) / sim.stock.dy) as usize;
                assert_eq!(
                    sim.stock.heights[y * sim.stock.nx + x],
                    sim.stock.bounds.max[2]
                );
            }
        }
    }
    #[test]
    fn the_tool_stays_down_by_the_shortest_way_clear_of_islands_or_not_at_all() {
        let square = |x: f64, y: f64, size: f64| -> Polygon {
            vec![(x, y), (x + size, y), (x + size, y + size), (x, y + size)]
        };
        let open = vec![square(-5., -5., 20.)];
        // Just cut: a 10 mm square loop that began, and so ended, at (10, 5).
        let cut = begun(&square(0., 0., 10.), (10., 5.), 1);
        assert_eq!(cut, [(10., 5.), (10., 10.), (0., 10.), (0., 0.), (10., 0.)]);
        // A small loop off its (0, 0) corner: back along the cut floor the
        // short way (15 mm, not 25), then one step to the loop's nearest point.
        let corner = square(-1.5, -1.5, 1.);
        let (points, path) = step(&cut, &corner, 1.5, &open).unwrap();
        assert_eq!(points, [(10., 0.), (0., 0.), (-0.5, -0.5)]);
        assert_eq!(path[0], (-0.5, -0.5));
        assert_eq!(path.len(), 5);
        // Too far for one step.
        assert!(step(&cut, &square(-4., -4., 1.), 1.5, &open).is_none());
        // A no-go area (wound clockwise) on the short way: it goes round
        // another way, or not at all when every way is blocked.
        let mut island = square(-0.3, -0.3, 0.2);
        island.reverse();
        let blocked = vec![open[0].clone(), island];
        assert!(crosses(&blocked, (0., 0.), (-0.5, -0.5)));
        if let Some((points, _)) = step(&cut, &corner, 1.5, &blocked) {
            let n = points.len();
            assert!(!crosses(&blocked, points[n - 2], points[n - 1]));
        }
        let mut moat = square(-1.45, -1.45, 1.4);
        moat.reverse();
        assert!(step(&cut, &corner, 1.5, &[open[0].clone(), moat]).is_none());
        // Leaving the allowed area.
        assert!(step(&cut, &corner, 1.5, &[square(0., 0., 10.)]).is_none());
    }
    #[test]
    fn inner_rings_come_before_the_ring_around_them() {
        let square = |x: f64, y: f64, size: f64| -> Polygon {
            vec![(x, y), (x + size, y), (x + size, y + size), (x, y + size)]
        };
        // Two separate areas, each with an inner ring; the second also
        // has an island path.
        let mut island = square(42., 2., 2.);
        island.reverse();
        let rings = vec![
            vec![square(0., 0., 10.), square(30., -5., 20.), island.clone()],
            vec![square(1., 1., 8.), square(31., -4., 18.)],
        ];
        let open = vec![square(-100., -100., 200.)];
        let order: Vec<Polygon> = ordered(rings, 1, 1.5, &open)
            .into_iter()
            .map(|(p, _)| p)
            .collect();
        // A loop the tool steps over to starts elsewhere: compare as sets.
        let at = |p: &Polygon| {
            let mut p = p.clone();
            p.sort_by(|a, b| a.partial_cmp(b).unwrap());
            order
                .iter()
                .position(|q| {
                    let mut q: Polygon = q.iter().copied().filter(|v| p.contains(v)).collect();
                    q.sort_by(|a, b| a.partial_cmp(b).unwrap());
                    q.dedup();
                    q == p
                })
                .unwrap()
        };
        assert!(at(&square(1., 1., 8.)) < at(&square(0., 0., 10.)));
        assert!(at(&square(31., -4., 18.)) < at(&square(30., -5., 20.)));
        assert!(at(&square(31., -4., 18.)) < at(&island));
        // Each area is finished before the next begins, its outermost paths
        // repeated once for the finish pass.
        assert_eq!(order.len(), 5 + 3);
        let first: Vec<_> = order.iter().map(|p| p[0].0 < 20.).collect();
        assert!(
            first.windows(2).filter(|w| w[0] != w[1]).count() == 1,
            "{order:?}"
        );
    }
    #[test]
    fn profile_ramp_obeys_angle_and_rewind_matches_incremental_playback() {
        let (setup, mut op, tool, mesh) = fixture();
        op.p.insert("pocket_entry".into(), "Profile ramp".into());
        let plan = generate(&setup, &op, &tool, &mesh).unwrap();
        assert!(
            plan.moves
                .iter()
                .any(|m| m.cutting_pass && m.to[2] < m.from[2])
        );
        for m in plan.moves.iter().filter(|m| m.cutting_pass) {
            let xy = ((m.to[0] - m.from[0]).powi(2) + (m.to[1] - m.from[1]).powi(2)).sqrt();
            assert!((m.to[2] - m.from[2]).abs() <= xy * 2_f32.to_radians().tan() + 0.0001);
        }
        let mut a = Simulation::new(plan.clone());
        let mut b = Simulation::new(plan);
        a.seek(0.5);
        for i in 0..=50 {
            b.seek(i as f32 / 100.);
        }
        for (x, y) in a.stock.heights.iter().zip(&b.stock.heights) {
            assert!((x - y).abs() < 0.001);
        }
        let partial = a.stock.heights.clone();
        a.seek(1.);
        a.seek(0.5);
        assert_eq!(partial, a.stock.heights);
        a.seek(0.);
        assert_eq!(a.stock.removed_volume(), 0.);
    }
    #[test]
    fn pocket_rejects_stale_geometry_bad_targets_tools_and_parameters() {
        let (setup, op, tool, mesh) = fixture();
        for (key, value, expected) in [
            ("bottom", "-1", "below"),
            ("stepover", "100", "50%"),
            ("stepdown", "0.000001", "limit"),
            ("pocket_milling", "unknown", "valid"),
            ("pocket_entry", "Helix", "valid"),
            ("containment", "Tool outside boundary", "inside"),
            ("boundary", "Stock silhouette", "Selected"),
            ("finish_passes", "0.5", "whole"),
        ] {
            let mut bad = op.clone();
            bad.p.insert(key.into(), value.into());
            assert!(
                generate(&setup, &bad, &tool, &mesh)
                    .unwrap_err()
                    .contains(expected),
                "{key}"
            );
        }
        let mut bad = op.clone();
        bad.surfaces.as_mut().unwrap().mesh_fingerprint = "0".repeat(64);
        assert!(
            generate(&setup, &bad, &tool, &mesh)
                .unwrap_err()
                .contains("changed")
        );
        let mut huge = tool.clone();
        huge.p.insert("diameter".into(), "40".into());
        assert!(
            generate(&setup, &op, &huge, &mesh)
                .unwrap_err()
                .contains("narrow")
        );
        huge.p.insert("type".into(), "Ball end mill".into());
        assert!(
            generate(&setup, &op, &huge, &mesh)
                .unwrap_err()
                .contains("flat end")
        );
    }
    #[test]
    fn pocket_rotation_translation_allowances_and_preferences_persist() {
        let (mut setup, mut op, tool, mesh) = fixture();
        let original = generate(&setup, &op, &tool, &mesh).unwrap();
        setup.p.insert("rz".into(), "90".into());
        setup.p.insert("tx".into(), "40".into());
        op.p.insert("allowance".into(), "0.5".into());
        op.p.insert("axial_allowance".into(), "0.4".into());
        let restored: Operation =
            serde_json::from_str(&serde_json::to_string(&op).unwrap()).unwrap();
        assert_eq!(pocket_setting(&restored.p, "pocket_entry"), "Plunge");
        let plan = generate(&setup, &restored, &tool, &mesh).unwrap();
        assert!((plan.target - original.target - 0.4).abs() < 0.001);
        assert!(
            plan.moves
                .iter()
                .filter(|m| m.cutting_pass)
                .all(|m| m.to[0] > 32. && m.to[0] < 48.)
        );
        let mut a = Simulation::new(original);
        let mut b = Simulation::new(plan);
        a.seek(1.);
        b.seek(1.);
        assert!(b.stock.removed_volume() < a.stock.removed_volume());
    }
    #[test]
    fn pocket_rejects_overhangs_and_floors_that_are_not_horizontal() {
        let (mut setup, mut op, tool, mesh) = fixture();
        let scale = mesh.display_scale();
        let mut triangles: Vec<[[f32; 3]; 3]> = mesh
            .faces
            .iter()
            .map(|f| std::array::from_fn(|i| f.points[i].map(|v| v / scale)))
            .collect();
        let floor_z = mesh.surfaces[0].center[2] / scale;
        quad(&mut triangles, 4., -1., 7., 1., floor_z + 2.);
        let covered = mesh::normalize(triangles).unwrap();
        op.surfaces.as_mut().unwrap().mesh_fingerprint = covered.fingerprint.clone();
        assert!(
            generate(&setup, &op, &tool, &covered)
                .unwrap_err()
                .contains("obstructs")
        );
        op.surfaces.as_mut().unwrap().mesh_fingerprint = mesh.fingerprint.clone();
        setup.p.insert("rx".into(), "10".into());
        assert!(
            generate(&setup, &op, &tool, &mesh)
                .unwrap_err()
                .contains("perpendicular")
        );
    }
    /// The 2D distance from `p` to the polygon `poly`: 0 inside it.
    fn distance_to(poly: &[(f64, f64)], p: (f64, f64)) -> f64 {
        if inside(&[poly.to_vec()], p) {
            return 0.;
        }
        nearest(poly, p).0
    }
    #[test]
    fn the_totem_keyboard_case_top_pockets_at_2_degrees_without_touching_the_part() {
        if crate::step::skip_without_helper() {
            return;
        }
        let mesh = mesh::load(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("examples/totem-case-top-right.step"),
        )
        .unwrap();
        // The job its user runs: a Ø3 flat end mill, 40 % stepover, 2 mm
        // stepdown, a 2° profile ramp, 0.3 mm radial and 0.2 mm axial
        // allowance and a finish pass, down to the plate between the keys.
        let (setup, mut op, mut tool, _) = fixture();
        tool.p.insert("diameter".into(), "3".into());
        for (k, v) in [
            ("pocket_entry", "Profile ramp"),
            ("ramp_angle", "2"),
            ("stepover", "40"),
            ("stepdown", "2"),
            ("allowance", "0.3"),
            ("axial_allowance", "0.2"),
            ("finish_passes", "1"),
        ] {
            op.p.insert(k.into(), v.into());
        }
        let placement = Placement::new(&mesh, Some(&setup));
        let scale = mesh.display_scale();
        let (top, bottom) = (placement.bounds.max[2], placement.bounds.min[2]);
        let (id, floor) = mesh
            .surfaces
            .iter()
            .enumerate()
            .filter(|(_, s)| {
                s.normal[2] > 0.999999
                    && placement.transform(s.center)[2] < top - 0.1
                    && placement.transform(s.center)[2] > bottom + 0.1
            })
            .max_by(|(_, a), (_, b)| a.area.total_cmp(&b.area))
            .unwrap();
        let depth = (top - placement.transform(floor.center)[2]) / scale;
        assert!((depth - 6.85).abs() < 0.01, "{depth}");
        op.surfaces = Some(surface::Selection {
            mesh_fingerprint: mesh.fingerprint.clone(),
            ids: vec![id],
        });
        let plan = generate(&setup, &op, &tool, &mesh).unwrap();
        // It used to ramp into every ring at every level: over the limit.
        assert!(plan.moves.len() < 20_000, "{} moves", plan.moves.len());
        assert_eq!(plan.levels, 4);
        let loops = loops(&plan).len();
        // Entries only where a loop is the deepest of its spot (11 of 19
        // loops a level), not one for every loop.
        assert!(
            entries(&plan) * 3 < loops * 2,
            "{} entries, {loops} loops",
            entries(&plan)
        );
        // No feed move brings the cutter nearer the part than the radial
        // allowance: every 0.25 mm of each, against what of every face of
        // the part stands above it.
        let faces: Vec<Polygon> = mesh
            .faces
            .iter()
            .map(|f| {
                let points: Vec<[f32; 3]> = f
                    .points
                    .iter()
                    .map(|p| placement.transform(*p).map(|v| v / scale))
                    .collect();
                above(&points, plan.target + 0.01)
            })
            .filter(|p| p.len() >= 2)
            .collect();
        // Faces by 5 mm cell of their bounds, to look up those near a point.
        let cell = 5.;
        let mut grid: std::collections::HashMap<(i64, i64), Vec<usize>> = Default::default();
        for (i, face) in faces.iter().enumerate() {
            let (x0, x1) = face
                .iter()
                .fold((f64::MAX, f64::MIN), |a, p| (a.0.min(p.0), a.1.max(p.0)));
            let (y0, y1) = face
                .iter()
                .fold((f64::MAX, f64::MIN), |a, p| (a.0.min(p.1), a.1.max(p.1)));
            for gx in (x0 / cell).floor() as i64..=(x1 / cell).floor() as i64 {
                for gy in (y0 / cell).floor() as i64..=(y1 / cell).floor() as i64 {
                    grid.entry((gx, gy)).or_default().push(i);
                }
            }
        }
        let clear = (plan.radius + 0.3) as f64;
        let mut checked = 0;
        for m in plan.moves.iter().filter(|m| m.motion == Motion::Feed) {
            let length = (m.to[0] - m.from[0]).hypot(m.to[1] - m.from[1]);
            let pieces = (length / 0.25).ceil().max(1.) as usize;
            for k in 0..=pieces {
                let p = m.at(m.start + (m.end - m.start) * k as f32 / pieces as f32);
                if p[2] >= plan.stock.max[2] {
                    continue;
                }
                let xy = (p[0] as f64, p[1] as f64);
                let (gx, gy) = ((xy.0 / cell).floor() as i64, (xy.1 / cell).floor() as i64);
                for dx in -1..=1 {
                    for dy in -1..=1 {
                        for &i in grid.get(&(gx + dx, gy + dy)).into_iter().flatten() {
                            let face = &faces[i];
                            let d = distance_to(face, xy);
                            assert!(d >= clear - 0.001, "{d} from a face at {p:?}");
                        }
                    }
                }
                checked += 1;
            }
        }
        assert!(checked > 10_000, "{checked}");
        // And it clears the plate between the keys.
        let mut sim = Simulation::new(plan);
        sim.seek(1.);
        assert!(sim.stock.removed_volume() > 10_000.);
    }
    #[test]
    fn actual_step_and_3mf_pocket_with_boss_generate_and_leave_the_boss() {
        for extension in ["step", "3mf"] {
            if extension == "step" && crate::step::skip_without_helper() {
                continue;
            }
            let mesh = mesh::load(
                &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join(format!("examples/four-setup-block.{extension}")),
            )
            .unwrap();
            let (mut setup, mut op, mut tool, _) = fixture();
            setup.p.insert("stock_top".into(), "10".into());
            tool.p.insert("diameter".into(), "10".into());
            let placement = Placement::new(&mesh, Some(&setup));
            let top = placement.bounds.max[2];
            let bottom = placement.bounds.min[2];
            let (id, floor) = mesh
                .surfaces
                .iter()
                .enumerate()
                .filter(|(_, s)| {
                    s.normal[2].abs() > 0.999999
                        && placement.transform(s.center)[2] < top - 0.1
                        && placement.transform(s.center)[2] > bottom + 0.1
                })
                .max_by(|(_, a), (_, b)| a.area.total_cmp(&b.area))
                .unwrap();
            eprintln!(
                "{extension}: floor {} area {:.2} depth {:.2} mm",
                id + 1,
                floor.area / mesh.display_scale().powi(2),
                (top - placement.transform(floor.center)[2]) / mesh.display_scale()
            );
            op.surfaces = Some(surface::Selection {
                mesh_fingerprint: mesh.fingerprint.clone(),
                ids: vec![id],
            });
            let plan = generate(&setup, &op, &tool, &mesh).unwrap();
            eprintln!(
                "{extension}: {} moves, {} levels, {:.2} min",
                plan.moves.len(),
                plan.levels,
                plan.duration / 60.
            );
            assert!(plan.moves.len() > 100 && plan.levels > 1);
            let mut sim = Simulation::new(plan);
            sim.seek(1.);
            assert!(sim.stock.removed_volume() > 10_000.);
            // The fixture boss is centered at X=-18, Y=0 (not the part origin).
            let x = ((-18. - sim.stock.bounds.min[0]) / sim.stock.dx) as usize;
            let y = ((0. - sim.stock.bounds.min[1]) / sim.stock.dy) as usize;
            assert_eq!(
                sim.stock.heights[y * sim.stock.nx + x],
                sim.stock.bounds.max[2]
            );
        }
    }
}
