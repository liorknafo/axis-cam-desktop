//! The load on one tooth along a milling operation's path: at each point,
//! the peak tangential force (N) one flute takes over a turn of the spindle
//! there. The cutter's edge is sampled round and up its flutes, its end
//! too; an element of it cuts where it faces the feed and the point just
//! ahead of it is still material: inside the stock's box or bar and swept
//! by no cut recorded before it (`Volume::cuts`, the earlier setups'
//! included, then this operation's own path up to there), tested exactly
//! (`volume::swept`) as the stock's surface is meshed, not on the voxels,
//! which are too coarse at a tool's size. Its chip is the feed per tooth
//! times the cosine between the feed and the edge's normal there (the
//! chip thinning of a light cut, a ramp's and a plunge's alike), its force
//! kc1.1 × h^(1−mc) × its width (`forces::KC`). One tooth's force is the
//! sum over the elements it has in the cut at one spindle angle, its flute
//! wound up a `HELIX` helix, and the load the most over a turn. A simple
//! model: the engagement is exact, kc, the helix and the straight chip
//! are assumed. Milling only: drills, taps, thread mills and the lathe
//! have none (`of`).
use crate::material::Material;
use crate::mesh::Bounds;
use crate::model::{Document, Machine, Operation, Strategy, Tool, facing_setting, get, num};
use crate::simulation::{CutterShape, Motion, Move, Plan, Simulation};
use crate::stock_surface::Cut;
use crate::volume::{Frame, swept};
use std::f32::consts::TAU;

/// The flutes' helix angle, degrees: a common end mill's.
pub const HELIX: f32 = 30.;
/// The spindle angles a turn is sampled at: every 5°.
const ANGLES: usize = 72;
/// How far ahead of the edge, along its normal, material is looked for, mm.
const AHEAD: f32 = 0.005;
/// How tall the lowest slice of a cutter's side is, mm (`elements`).
const FOOT: f32 = 0.05;
/// The most samples an operation's path is cut in: a longer path is
/// sampled more coarsely.
const MOST: f32 = 150_000.;

/// The load on one tooth along a plan's moves (`of`).
#[derive(Debug, Clone, PartialEq)]
pub struct ToothLoad {
    /// Of each move of the plan, the force on one tooth (N) midway along
    /// each of the equal pieces it is sampled in; empty for a rapid or a
    /// dwell.
    pub moves: Vec<Vec<f32>>,
    /// Of each of those pieces, the most the feed's cosine with the edge's
    /// normal reaches over the edge in the cut: the thickest chip there is
    /// the feed per tooth times it. 0 where no tooth meets material.
    pub facing: Vec<Vec<f32>>,
    /// The most of them, N.
    pub peak: f32,
}
impl ToothLoad {
    /// The force on one tooth over the piece of move `index` that
    /// fraction `t` of it lies in; None for a move that cuts nothing.
    pub fn at(&self, index: usize, t: f32) -> Option<f32> {
        let pieces = self.moves.get(index).filter(|p| !p.is_empty())?;
        let n = pieces.len();
        let i = ((t.clamp(0., 1.) * n as f32).ceil() as usize).clamp(1, n) - 1;
        Some(pieces[i])
    }
}

/// The load on one tooth along `sim`'s operation of `doc`: its tool's
/// flutes, its spindle speed and the project's material (`measure`). None
/// for an operation that is not milled, or without the stock it starts
/// from, a tool, flutes or a spindle speed.
pub fn of(sim: &Simulation, doc: &Document) -> Option<ToothLoad> {
    let positive = |p, key| num(p, key).filter(|v| *v > 0.);
    let op = doc
        .setups
        .iter()
        .flat_map(|s| &s.operations)
        .find(|o| o.id == sim.plan.operation)?;
    let tool = doc.tools.iter().find(|t| t.id == op.tool)?;
    measure(
        sim,
        positive(&tool.p, "flutes")?,
        positive(&op.p, "rpm")?,
        doc.selected_material(),
        HELIX,
    )
}

/// An element of a cutter's edge: `rho` from its axis and `z` above its
/// tip, its outward normal `normal` (radial, up), `width` mm of edge, and
/// how far round (radians) its flute's helix has it behind the flute's
/// lowest point.
#[derive(Debug, Clone, Copy)]
struct Element {
    rho: f32,
    z: f32,
    normal: [f32; 2],
    width: f32,
    lag: f32,
}
/// The edge of a cutter of `shape` and `radius` with `flute` long flutes
/// wound at `helix` degrees: its end (a flat end's face, a ball, a cone)
/// and then its side up the flutes.
fn elements(shape: CutterShape, radius: f32, flute: f32, helix: f32) -> Vec<Element> {
    let mut out = vec![];
    let mut add = |rho: f32, z: f32, normal: [f32; 2], width: f32| {
        if z <= flute {
            out.push(Element {
                rho,
                z,
                normal,
                width,
                lag: z * helix.to_radians().tan() / radius,
            });
        }
    };
    let side_from = match (shape, shape.rise()) {
        (CutterShape::Ball, _) => {
            const ARCS: usize = 8;
            let step = std::f32::consts::FRAC_PI_2 / ARCS as f32;
            for i in 0..ARCS {
                let b = (i as f32 + 0.5) * step;
                add(
                    radius * b.sin(),
                    radius * (1. - b.cos()),
                    [b.sin(), -b.cos()],
                    radius * step,
                );
            }
            radius
        }
        (_, Some(rise)) => {
            const RINGS: usize = 6;
            let slant = (1. + rise * rise).sqrt();
            let step = radius / RINGS as f32;
            for i in 0..RINGS {
                let rho = (i as f32 + 0.5) * step;
                add(rho, rise * rho, [rise / slant, -1. / slant], step * slant);
            }
            rise * radius
        }
        _ => {
            const RINGS: usize = 4;
            let step = radius / RINGS as f32;
            for i in 0..RINGS {
                add((i as f32 + 0.5) * step, 0., [0., -1.], step);
            }
            0.
        }
    };
    // A quarter of a millimetre each, however long the flutes: a cut 1 mm
    // deep is four of them, not a part of one (those above the stock's top
    // cost nothing, `measure`). The lowest one's foot (`FOOT`) is its own,
    // so that a thin band at the foot of the flutes (what each of a
    // Waterline's close levels takes off a wall) is met.
    let height = (flute - side_from).max(0.);
    let slices = (height / 0.25).ceil().clamp(1., 400.);
    let dz = height / slices;
    let foot = FOOT.min(dz);
    add(radius, side_from + foot / 2., [1., 0.], foot);
    if dz > foot {
        add(radius, side_from + (foot + dz) / 2., [1., 0.], dz - foot);
    }
    for i in 1..slices as usize {
        add(radius, side_from + (i as f32 + 0.5) * dz, [1., 0.], dz);
    }
    out
}

/// The load on one tooth along `sim`'s moves (`ToothLoad`), cut by a tool
/// with `flutes` wound at `helix` degrees at `rpm` in `material` (None
/// counts as steel, `forces::kc`), each move in the frame it cuts in (an
/// indexed setup's at its A angle). None for a plan that is not milled
/// (a lathe's, a drill's, a tap's or a thread mill's), or without the
/// stock it starts from.
pub fn measure(
    sim: &Simulation,
    flutes: f32,
    rpm: f32,
    material: Option<&Material>,
    helix: f32,
) -> Option<ToothLoad> {
    let plan = &sim.plan;
    let (Some(volume), Some(frame)) = (sim.initial_volume.as_ref(), sim.frame) else {
        return None;
    };
    let drilled = plan.strategy == Strategy::Drill
        && (plan.shape != CutterShape::Flat
            || plan.milling_direction == crate::forces::THREAD_MILLING_DIRECTION
            || plan.milling_direction == crate::forces::TAPPING_DIRECTION);
    if plan.is_lathe() || drilled || plan.shape.teeth().is_some() || plan.radius <= 0. {
        return None;
    }
    let (kc, mc) = crate::forces::kc(material);
    // The frames the moves cut in: `frame`, and in an indexed setup the
    // part posed at each A angle (`Simulation::frame_of`). Each keeps every
    // cut, those at the angles before included.
    let mut frames = vec![frame];
    for &(_, f) in &sim.turns {
        if !frames.contains(&f) {
            frames.push(f);
        }
    }
    let in_frame = |m: &Move| {
        let f = sim.frame_of(m);
        frames.iter().position(|g| Some(*g) == f).unwrap_or(0)
    };
    let stocks: Vec<Bounds> = (frames.iter())
        .map(|&f| volume.frame.transfer_bounds(volume.bounds, f))
        .collect();
    let bar = volume.round.then(|| crate::stock::bar(volume.bounds));
    let mut histories: Vec<_> = (frames.iter().zip(&stocks))
        .map(|(&f, stock)| {
            let extent = (stock.max[0] - stock.min[0]).max(stock.max[1] - stock.min[1]);
            crate::volume::History::new(f, *stock, plan.radius.max(extent / 400.))
        })
        .collect();
    let mut add = |cut: Cut| {
        (histories.iter_mut())
            .map(|h| h.add(cut.clone()))
            .last()
            .unwrap_or(0)
    };
    for cut in volume.cuts.iter() {
        add(cut.clone());
    }
    let length = |m: &crate::simulation::Move| {
        (0..3)
            .map(|k| (m.to[k] - m.from[k]).powi(2))
            .sum::<f32>()
            .sqrt()
    };
    let cutting = |m: &crate::simulation::Move| m.motion == Motion::Feed && m.feed > 0.;
    // Every cutting move is kept, its number with it: a point of a move is
    // tested against the cuts before it only, so the moves are measured
    // apart, side by side.
    let numbers: Vec<Option<u32>> = plan
        .moves
        .iter()
        .map(|m| {
            (cutting(m) && length(m) > 0.).then(|| {
                add(Cut {
                    a: m.from,
                    b: m.to,
                    radius: plan.radius,
                    flute: plan.flute_length,
                    shape: plan.shape,
                    frame: frames[in_frame(m)],
                    source: sim.source,
                })
            })
        })
        .collect();
    // Whether `p` of frame `f` is still material: in the stock (a rotary
    // setup's bar too), swept by no cut before cut `before`, nor by the
    // move under way from `a` to `b`.
    let solid = |f: usize, p: [f32; 3], before: u32, a: [f32; 3], b: [f32; 3]| {
        let q = frames[f].transfer(p, volume.frame);
        (0..3).all(|k| (volume.bounds.min[k]..=volume.bounds.max[k]).contains(&q[k]))
            && bar.is_none_or(|(c, r)| (q[0] - c[0]).hypot(q[1] - c[1]) <= r)
            && volume.bar_x.is_none_or(|bar| bar.contains(q))
            && !swept(p, a, b, plan.radius, plan.flute_length, plan.shape)
            && !histories[f].swept(p, before)
    };
    let edge = elements(plan.shape, plan.radius, plan.flute_length, helix);
    let step_angle = TAU / ANGLES as f32;
    let around: Vec<(f32, f32)> = (0..ANGLES)
        .map(|j| (j as f32 * step_angle).sin_cos())
        .map(|(s, c)| (c, s))
        .collect();
    // How many sampled angles round each element's flute is behind its
    // lowest point: ahead of the turn for a counter-clockwise spindle.
    let behind: Vec<usize> = edge
        .iter()
        .map(|e| {
            let k = (e.lag / step_angle).round() as isize;
            let k = if plan.spindle_clockwise { k } else { -k };
            k.rem_euclid(ANGLES as isize) as usize
        })
        .collect();
    let step = sample_step(plan);
    // The force on one tooth midway along each piece of move `index`, and
    // how thin the chip is there (`ToothLoad::facing`).
    let measure_move = |index: usize| -> (Vec<f32>, Vec<f32>) {
        let (m, Some(before)) = (&plan.moves[index], numbers[index]) else {
            return (vec![], vec![]);
        };
        let f = in_frame(m);
        let top = plan.stock.max[2].max(stocks[f].max[2]);
        let l = length(m);
        let v: [f32; 3] = std::array::from_fn(|k| (m.to[k] - m.from[k]) / l);
        let chip = m.feed / (rpm * flutes);
        let pieces = (l / step).ceil().max(1.) as usize;
        let at =
            |t: f32| -> [f32; 3] { std::array::from_fn(|k| m.from[k] + (m.to[k] - m.from[k]) * t) };
        let mut loads = Vec::with_capacity(pieces);
        let mut facings = Vec::with_capacity(pieces);
        for i in 0..pieces {
            // Midway along it, where the cutter goes on along the move: at its
            // end it may turn away from what lies ahead.
            let here = at((i as f32 + 0.5) / pieces as f32);
            let mut tooth = [0_f32; ANGLES];
            let mut most = 0_f32;
            for (e, &behind) in edge.iter().zip(&behind) {
                if here[2] + e.z > top {
                    continue;
                }
                for (j, &(c, s)) in around.iter().enumerate() {
                    let n = [e.normal[0] * c, e.normal[0] * s, e.normal[1]];
                    let facing = v[0] * n[0] + v[1] * n[1] + v[2] * n[2];
                    if facing <= 1e-4 {
                        continue;
                    }
                    let p = [
                        here[0] + e.rho * c + AHEAD * n[0],
                        here[1] + e.rho * s + AHEAD * n[1],
                        here[2] + e.z + AHEAD * n[2],
                    ];
                    if solid(f, p, before, m.from, here) {
                        most = most.max(facing);
                        // The tooth whose flute's lowest point is `behind`
                        // angles back from here.
                        tooth[(j + ANGLES - behind) % ANGLES] +=
                            kc * (chip * facing).powf(1. - mc) * e.width;
                    }
                }
            }
            loads.push(tooth.into_iter().fold(0., f32::max));
            facings.push(most);
        }
        (loads, facings)
    };
    // The moves dealt round the cores in turn.
    let workers = std::thread::available_parallelism().map_or(1, |n| n.get());
    let progress = crate::progress::current();
    let mut moves = vec![vec![]; plan.moves.len()];
    let mut facing = vec![vec![]; plan.moves.len()];
    let dealt = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..workers)
            .map(|w| {
                let (measure_move, progress) = (&measure_move, &progress);
                scope.spawn(move || {
                    let mut out = vec![];
                    for index in (w..plan.moves.len()).step_by(workers) {
                        // A cancelled generation's.
                        if progress.as_ref().is_some_and(|p| p.cancelled()) {
                            return None;
                        }
                        out.push((index, measure_move(index)));
                    }
                    Some(out)
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().ok().flatten())
            .collect::<Vec<_>>()
    });
    for (index, (loads, facings)) in dealt
        .into_iter()
        .collect::<Option<Vec<_>>>()?
        .into_iter()
        .flatten()
    {
        moves[index] = loads;
        facing[index] = facings;
    }
    let peak = moves.iter().flatten().copied().fold(0., f32::max);
    Some(ToothLoad {
        moves,
        facing,
        peak,
    })
}
/// How far apart along a plan's cutting moves `measure` samples the load:
/// a third of the cutter's radius, coarser on a path too long for `MOST`
/// samples.
fn sample_step(plan: &Plan) -> f32 {
    let total: f32 = (plan.moves.iter())
        .filter(|m| m.motion == Motion::Feed && m.feed > 0.)
        .map(|m| distance(m.from, m.to))
        .sum();
    (plan.radius / 3.).clamp(0.05, 1.).max(total / MOST)
}
fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    (0..3).map(|k| (b[k] - a[k]).powi(2)).sum::<f32>().sqrt()
}
/// The choice of what an operation's feed follows (`feed_follows`): the
/// programmed feed throughout, or a feed changed along the path so the
/// force on one tooth stays at a target (`hold`). An operation without
/// the setting (one saved before it) keeps its programmed feed.
pub const FEED_FOLLOWS: [&str; 2] = ["Programmed feed", "Hold the force per tooth"];
/// Whether `op`'s feed follows the force on its teeth (`FEED_FOLLOWS`).
pub fn follows(op: &Operation, machine: &Machine) -> bool {
    offered(op.strategy, machine) && get(&op.p, "feed_follows") == FEED_FOLLOWS[1]
}
/// The strategies whose feed can follow the force on a tooth: every one a
/// CNC mill cuts with a milling cutter's side and end, whose load `measure`
/// works out: the 2.5D clearing and profiling ones, whose nominal cut is a
/// stepover by a stepdown, Waterline, Parallel, Chamfer and Groove, on a
/// machine with a program (not a manual mill or a lathe). Not Drill, Box
/// (a manual mill's) or Turn, nor the 4th- and 5-axis ones.
pub fn offered(strategy: Strategy, machine: &Machine) -> bool {
    matches!(
        strategy,
        Strategy::Face
            | Strategy::Adaptive
            | Strategy::Pocket
            | Strategy::Contour
            | Strategy::Waterline
            | Strategy::Parallel
            | Strategy::Chamfer
            | Strategy::Groove
    ) && !crate::manual::is_hand(machine)
}
/// Whether `strategy`'s automatic force is the one along its own path
/// (`typical`), not a nominal cut's: a Parallel's, which follows the
/// surface with no stepdown, a Chamfer's and a Groove's, whose cut is the
/// edge's or the groove's, not a stepover by a stepdown.
pub fn from_the_path(strategy: Strategy) -> bool {
    matches!(
        strategy,
        Strategy::Parallel | Strategy::Chamfer | Strategy::Groove
    )
}
/// Each feed a held operation's moves take is the programmed feed times a
/// whole power of this: a new F word only where the feed wanted changes
/// by about a tenth.
const FEED_STEP: f32 = 1.1;
/// The least a held feed is, of the move's programmed feed.
const SLOWEST: f32 = 0.25;
/// The thickest chip a held feed cuts, of the programmed feed per tooth: a
/// light cut's thin chip is fed up (chip thinning), to no more than this.
const THICKEST: f32 = 1.5;
/// The fastest a held feed is, of the move's programmed feed: however
/// thin a light cut's chip, no more.
const FASTEST: f32 = 3.;

/// A straight cut a tool makes along +X into a block: `depth` mm deep from
/// its tip, `width` mm into the material at its side (None: a full slot).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NominalCut {
    pub shape: CutterShape,
    pub radius: f32,
    pub flute: f32,
    pub clockwise: bool,
    pub climb: bool,
    pub width: Option<f32>,
    pub depth: f32,
}
/// `op`'s nominal cut with `tool`: its stepover (a Pocket's and a Facing's
/// in % of the diameter, an Adaptive's optimal load) by its stepdown, or a
/// full slot as deep as its stepdown where it has no stepover (a Contour, a
/// Waterline: a loop at each level, round what stands above it);
/// no deeper than `level`, the depth its levels cut (`level_depth`), where
/// that is known, nor than the flutes. None for a strategy whose feed
/// cannot follow the load (`offered`), or without the numbers.
pub fn nominal_cut(op: &Operation, tool: &Tool, level: Option<f32>) -> Option<NominalCut> {
    let positive = |p, key| num(p, key).filter(|v| *v > 0.);
    let radius = positive(&tool.p, "diameter")? / 2.;
    let flute = positive(&tool.p, "length")?;
    let width = match op.strategy {
        Strategy::Pocket | Strategy::Face => {
            Some(positive(&op.p, "stepover")? / 100. * 2. * radius)
        }
        Strategy::Adaptive => Some(positive(&op.p, "optimal_load")?),
        Strategy::Contour | Strategy::Waterline => None,
        _ => return None,
    };
    let milling = if op.strategy == Strategy::Face {
        "facing_milling"
    } else {
        "pocket_milling"
    };
    Some(NominalCut {
        shape: if get(&tool.p, "type") == "Ball end mill" {
            CutterShape::Ball
        } else {
            CutterShape::Flat
        },
        radius,
        flute,
        clockwise: facing_setting(&op.p, "spindle_rotation") == "Clockwise (CW)",
        climb: facing_setting(&op.p, milling) != "Conventional",
        width: width.filter(|w| *w < 2. * radius),
        depth: positive(&op.p, "stepdown")?
            .min(flute)
            .min(level.unwrap_or(f32::INFINITY)),
    })
}
/// The force on one tooth (N) at `feed` mm/min and `rpm` with `flutes`, in
/// `material`, along `cut` once the cutter is fully in it: what `measure`
/// works out there. Worked out once for each set of numbers.
pub fn nominal(
    cut: &NominalCut,
    flutes: f32,
    rpm: f32,
    feed: f32,
    material: Option<&Material>,
) -> Option<f32> {
    use std::hash::{Hash, Hasher};
    static KNOWN: std::sync::Mutex<Vec<(u64, f32)>> = std::sync::Mutex::new(vec![]);
    let (kc, mc) = crate::forces::kc(material);
    let mut h = std::collections::hash_map::DefaultHasher::new();
    format!("{cut:?}").hash(&mut h);
    [flutes, rpm, feed, kc, mc].map(f32::to_bits).hash(&mut h);
    let key = h.finish();
    if let Some(known) = KNOWN.lock().ok()?.iter().find(|k| k.0 == key) {
        return Some(known.1);
    }
    let r = cut.radius;
    let frame = Frame::identity();
    let stock = Bounds {
        min: [-4. * r, -4. * r, -4. * r],
        max: [12. * r, 4. * r, cut.depth],
    };
    let mut volume = crate::volume::Volume::new(stock, frame);
    // The material on the side a climb cut keeps it, with a clockwise
    // spindle, on the right of the feed (`simulation::uncut_sides`).
    let side = if cut.climb == cut.clockwise { -1. } else { 1. };
    if let Some(width) = cut.width {
        let y = -side * width;
        volume.cut(
            [-20. * r, y, 0.],
            [20. * r, y, 0.],
            r,
            cut.flute,
            cut.shape,
            frame,
            (0, 0),
        );
    }
    let at = |x: f32| [x, 0., 0.];
    let moves = [
        (at(-8. * r), at(-8. * r), Motion::Rapid),
        (at(-8. * r), at(8. * r), Motion::Feed),
    ]
    .map(|(from, to, motion)| Move {
        from,
        to,
        motion,
        cutting_pass: true,
        feed,
        start: 0.,
        end: 0.,
        a: None,
    })
    .to_vec();
    let plan = Plan {
        shape: cut.shape,
        strategy: Strategy::Pocket,
        operation: String::new(),
        setup: String::new(),
        moves,
        stock,
        radius: r,
        flute_length: cut.flute,
        tool_length: cut.flute,
        holder_radius: r,
        neck: None,
        display_scale: 1.,
        duration: 0.,
        target: 0.,
        levels: 1,
        rest_top: None,
        spindle_clockwise: cut.clockwise,
        milling_direction: if cut.climb { "Climb" } else { "Conventional" },
        rings: vec![],
        stops: vec![],
        keep_air: vec![],
    };
    let mut sim = Simulation::new(plan);
    (sim.initial_volume, sim.frame) = (Some(volume), Some(frame));
    let load = measure(&sim, flutes, rpm, material, HELIX)?;
    // The middle fifth of the cut, well past where the cutter entered.
    let samples = &load.moves[1];
    let n = samples.len();
    let middle = &samples[n * 2 / 5..(n * 3 / 5).max(n * 2 / 5 + 1)];
    let force = middle.iter().sum::<f32>() / middle.len() as f32;
    let mut known = KNOWN.lock().ok()?;
    if known.len() >= 32 {
        known.remove(0);
    }
    known.push((key, force));
    Some(force)
}
/// The force on one tooth `op`'s feed holds (`follows`), N: the one typed
/// in, else the one at its programmed feed on its nominal cut
/// (`nominal_cut`, its levels `level` deep where that is known), or, for a
/// strategy without one (`from_the_path`), the one along most of its path
/// as programmed (`typical`, once `path` is measured); with whether it is
/// that automatic one.
pub fn target(
    op: &Operation,
    tool: &Tool,
    material: Option<&Material>,
    level: Option<f32>,
    path: Option<(&Plan, &ToothLoad)>,
) -> Option<(f32, bool)> {
    if let Some(typed) = num(&op.p, "tooth_force").filter(|f| *f > 0.) {
        return Some((typed, false));
    }
    if from_the_path(op.strategy) {
        let (plan, load) = path?;
        return typical(plan, load).map(|force| (force, true));
    }
    let positive = |p, key| num(p, key).filter(|v| *v > 0.);
    let force = nominal(
        &nominal_cut(op, tool, level)?,
        positive(&tool.p, "flutes")?,
        positive(&op.p, "rpm")?,
        positive(&op.p, "feed")?,
        material,
    )?;
    (force > 0.).then_some((force, true))
}
/// The force on one tooth along most of `plan`'s path where a tooth meets
/// material (`load`): the median over the length cut, N. None where no
/// tooth does.
fn typical(plan: &Plan, load: &ToothLoad) -> Option<f32> {
    let mut cut: Vec<(f32, f32)> = (plan.moves.iter().zip(&load.moves))
        .flat_map(|(m, forces)| {
            let piece = distance(m.from, m.to) / forces.len().max(1) as f32;
            (forces.iter().filter(|f| **f > 0.)).map(move |&f| (f, piece))
        })
        .collect();
    cut.sort_by(|a, b| a.0.total_cmp(&b.0));
    let half = cut.iter().map(|c| c.1).sum::<f32>() / 2.;
    let mut along = 0.;
    cut.into_iter()
        .find(|c| {
            along += c.1;
            along >= half
        })
        .map(|c| c.0)
}

/// What holding the force on a tooth made of an operation's feeds
/// (`hold`): the force held, N, and the slowest and the fastest feed
/// (mm/min) the moves that cut take.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Held {
    pub target: f32,
    pub slowest: f32,
    pub fastest: f32,
}
/// A plan's moves fed to hold the force on a tooth (`refeed`), re-timed,
/// with the load along them.
#[derive(Debug, Clone)]
struct Refed {
    moves: Vec<Move>,
    /// Of each of the plan's moves, the index of its first piece.
    starts: Vec<usize>,
    keep_air: Vec<bool>,
    duration: f32,
    load: std::sync::Arc<ToothLoad>,
    held: Held,
}

/// Changes the feed along `sim`'s moves so the force on one tooth stays at
/// `op`'s target (`target`), when its feed follows it (`follows`): splits
/// the cutting moves where the feed wanted changes (`refeed`), re-times the
/// plan, and keeps the load along the new moves (`Simulation::tooth_load`)
/// and what was held (`Simulation::held`). The load is measured once for
/// the same plan from the same stock. An indexed setup's operation is
/// measured at each A angle in that angle's frame (`measure`). Nothing
/// changes for an operation whose feed does not follow the load, or one
/// whose load cannot be measured; an error only for a cancelled generation.
pub fn hold(
    sim: &mut Simulation,
    doc: &Document,
    machine: &Machine,
    op: &Operation,
) -> Result<(), String> {
    use std::hash::{Hash, Hasher};
    static KNOWN: std::sync::Mutex<Vec<(u64, std::sync::Arc<ToothLoad>)>> =
        std::sync::Mutex::new(vec![]);
    if !follows(op, machine) {
        return Ok(());
    }
    let positive = |p, key| num(p, key).filter(|v| *v > 0.);
    let Some(tool) = doc.tools.iter().find(|t| t.id == op.tool) else {
        return Ok(());
    };
    let material = doc.selected_material();
    // A target from the path is known once it is measured.
    let (Some(flutes), Some(rpm), Some(feed), true) = (
        positive(&tool.p, "flutes"),
        positive(&op.p, "rpm"),
        positive(&op.p, "feed"),
        from_the_path(op.strategy) || target(op, tool, material, None, None).is_some(),
    ) else {
        return Ok(());
    };
    let inch = if get(&machine.p, "units") == "inch" {
        25.4
    } else {
        1.
    };
    let fastest = positive(&machine.p, "feed").map_or(f32::INFINITY, |f| f * inch);
    let (kc, mc) = crate::forces::kc(material);
    // What the load depends on: the plan, the stock it starts from, the
    // cutter and the material.
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let plan = &sim.plan;
    for m in &plan.moves {
        for v in [
            m.from,
            m.to,
            [m.feed, m.end - m.start, m.a.unwrap_or(f32::NAN)],
        ] {
            v.map(f32::to_bits).hash(&mut h);
        }
        (m.motion as u8).hash(&mut h);
    }
    format!(
        "{:?} {} {} {} {} {:?} {:?}",
        plan.shape,
        plan.radius,
        plan.flute_length,
        plan.spindle_clockwise,
        plan.milling_direction,
        sim.frame,
        sim.turns
    )
    .hash(&mut h);
    [flutes, rpm, kc, mc, HELIX].map(f32::to_bits).hash(&mut h);
    if let Some(v) = &sim.initial_volume {
        format!("{:?} {:?} {} {:?}", v.bounds, v.frame, v.round, v.bar_x).hash(&mut h);
        for c in v.cuts.iter() {
            for v in [c.a, c.b, [c.radius, c.flute, 0.]] {
                v.map(f32::to_bits).hash(&mut h);
            }
            shape_key(c.shape).hash(&mut h);
            c.frame.key().hash(&mut h);
        }
    }
    let key = h.finish();
    let known = KNOWN
        .lock()
        .ok()
        .and_then(|k| k.iter().find(|k| k.0 == key).map(|k| k.1.clone()));
    let load = match known {
        Some(load) => load,
        None => {
            let Some(load) = measure(sim, flutes, rpm, material, HELIX) else {
                if crate::progress::current().is_some_and(|p| p.cancelled()) {
                    return Err(crate::progress::CANCELLED.into());
                }
                return Ok(());
            };
            let load = std::sync::Arc::new(load);
            if let Ok(mut known) = KNOWN.lock() {
                if known.len() >= 6 {
                    known.remove(0);
                }
                known.push((key, load.clone()));
            }
            load
        }
    };
    // The nominal cut as deep as the levels that cut are, or the path's.
    let Some((target, _)) = target(
        op,
        tool,
        material,
        level_depth(&sim.plan, &load, |a| {
            match (a, &sim.initial_volume, sim.frame_at(a)) {
                (Some(_), Some(v), Some(f)) => top_in(v, f),
                _ => sim.plan.rest_top.unwrap_or(sim.plan.stock.max[2]),
            }
        }),
        Some((&sim.plan, &load)),
    ) else {
        return Ok(());
    };
    let refed = refeed(
        &sim.plan,
        &load,
        Feeding {
            target,
            mc,
            programmed: feed,
            fastest,
            most: crate::cam::CEILING,
        },
    );
    // A stop before a move is before its first piece.
    for stop in &mut sim.plan.stops {
        stop.at = refed
            .starts
            .get(stop.at)
            .copied()
            .unwrap_or(refed.moves.len());
    }
    sim.plan.moves = refed.moves;
    sim.plan.keep_air = refed.keep_air;
    sim.plan.duration = refed.duration;
    sim.tooth_load = Some(refed.load);
    sim.held = Some(refed.held);
    Ok(())
}
/// How deep a plan's levels cut, mm: the most its cuts along a level step
/// down from the level above at the same A angle, the first from where it
/// starts cutting at that angle (`top`: `Plan::rest_top`, else the stock's
/// top). A level is the Z of feed moves along which a tooth takes a tenth
/// of the operation's most force or more (`load`): not one the cutter only
/// travels along, over what it has cleared (an Adaptive's, half a
/// millimetre over the level before). None without a level below that.
fn level_depth(plan: &Plan, load: &ToothLoad, top: impl Fn(Option<f32>) -> f32) -> Option<f32> {
    let cuts = |i: usize| {
        load.moves[i]
            .iter()
            .any(|f| *f >= 0.1 * load.peak && *f > 0.)
    };
    let mut levels: Vec<(Option<u32>, f32)> = (plan.moves.iter().enumerate())
        .filter(|(i, m)| m.motion == Motion::Feed && m.from != m.to && cuts(*i))
        .filter(|(_, m)| (m.to[2] - m.from[2]).abs() < 1e-4 && m.to[2] < top(m.a) - 1e-3)
        .map(|(_, m)| (m.a.map(f32::to_bits), m.to[2]))
        .collect();
    levels.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.total_cmp(&a.1)));
    levels.dedup_by(|a, b| a.0 == b.0 && (a.1 - b.1).abs() < 1e-3);
    let mut above: Option<(Option<u32>, f32)> = None;
    let mut most = 0_f32;
    for (a, z) in levels {
        let from = match above {
            Some((b, from)) if b == a => from,
            _ => top(a.map(f32::from_bits)),
        };
        most = most.max(from - z);
        above = Some((a, z));
    }
    (most > 0.).then_some(most)
}
/// The top of the stock `v` started as, in frame `f`: its box's, or a
/// rotary setup's bar's (`Volume::bar_x`), the higher of its axis's ends
/// and its radius over it.
fn top_in(v: &crate::volume::Volume, f: Frame) -> f32 {
    let Some(bar) = v.bar_x else {
        return v.frame.transfer_bounds(v.bounds, f).max[2];
    };
    let [y, z] = bar.axis;
    let ends = [v.bounds.min[0], v.bounds.max[0]].map(|x| v.frame.transfer([x, y, z], f)[2]);
    ends[0].max(ends[1]) + bar.radius
}
/// `shape` as numbers, for a key.
fn shape_key(shape: CutterShape) -> [u16; 4] {
    match shape {
        CutterShape::Flat => [0; 4],
        CutterShape::Ball => [1, 0, 0, 0],
        CutterShape::Drill => [2, 0, 0, 0],
        CutterShape::Chamfer(a, b) => [3, a, b, 0],
        CutterShape::Revolve => [4, 0, 0, 0],
        CutterShape::Thread {
            pitch,
            teeth,
            angle,
        } => [5, pitch, teeth.into(), angle.into()],
    }
}
/// What `refeed` holds the force on a tooth at and between.
#[derive(Debug, Clone, Copy)]
struct Feeding {
    /// The force on one tooth to hold, N.
    target: f32,
    /// The material's mc: a tooth's force goes as the feed to 1 − mc.
    mc: f32,
    /// The operation's programmed feed, mm/min, which the feeds are a
    /// whole power of `FEED_STEP` times.
    programmed: f32,
    /// The machine's most feed, mm/min.
    fastest: f32,
    /// The most moves the plan may have.
    most: usize,
}
/// `plan`'s moves fed so the force on one tooth `load` measured along them
/// stays at `f.target`. At each sample the force goes as the feed to the
/// 1 − mc, the engagement staying as it is, so the feed wanted there is the
/// move's times (target ÷ force)^(1/(1 − mc)): no more than gives
/// `THICKEST` times the move's programmed chip at the thickest chip there
/// (`ToothLoad::facing`),
/// nor, down a plunge or a ramp, than its programmed feed; at most the
/// machine's most feed and `FASTEST` times the move's, and at least
/// `SLOWEST` of it; rounded down
/// to a whole power of `FEED_STEP` times the programmed feed. Where no
/// tooth meets material there is no chip to keep: the machine's most feed,
/// or `FASTEST` times the programmed one where it has none; within the
/// cutter's radius and a sample of a cut, the slowest that cut wants there,
/// and of the ends of a run of feeds, the move's own. A feed faster than those round it for less
/// than two samples along the path (two thirds of the cutter's radius) is
/// brought down to theirs: at a sample, never faster than wanted. Each cutting move is split
/// where its feed changes, each piece taking its own; every move is timed
/// again at its feed (a rapid and a dwell as they were). Past `f.most`
/// moves, each move takes the slowest feed of its pieces instead.
fn refeed(plan: &Plan, load: &ToothLoad, f: Feeding) -> Refed {
    // A faster feed for less than two samples is not taken: the least of
    // those within a sample, then the most of those.
    let reach = sample_step(plan) as f64;
    // Each cutting move's feed at each of its samples, then smoothed over
    // each run of cutting moves the cutter goes along without a rapid.
    // In the air a sample wants no feed of its own: the fastest there, but
    // within the cutter's radius and a sample of a cut the slowest that cut
    // wants, so that the tool slows before it meets material between two
    // samples and gets back up only once clear of it; and as near the ends
    // of a run, where material no sample saw may lie past the last one, the
    // move's own.
    let flying = f.fastest.min(f.programmed * FASTEST);
    let guard = plan.radius as f64 + reach;
    let mut feeds: Vec<Vec<f32>> = vec![vec![]; plan.moves.len()];
    let mut run: Vec<(usize, Option<f32>)> = vec![];
    let mut wanted = vec![];
    let mut along = vec![];
    let mut travelled = 0_f64;
    let mut smooth =
        |run: &mut Vec<(usize, Option<f32>)>, wanted: &mut Vec<f32>, along: &mut Vec<f64>| {
            let low = slide(wanted, along, reach, false);
            let smoothed = slide(&low, along, reach, true);
            let cuts: Vec<f32> = (run.iter().zip(wanted.iter()))
                .map(|(&(_, air), &w)| if air.is_some() { f32::INFINITY } else { w })
                .collect();
            let near = slide(&cuts, along, guard, false);
            let ends = |a: f64| {
                along.first().is_some_and(|&f| a - f < guard)
                    || along.last().is_some_and(|&l| l - a < guard)
            };
            for (k, ((&(i, air), feed), near)) in run.iter().zip(smoothed).zip(near).enumerate() {
                let own = if ends(along[k]) {
                    plan.moves[i].feed
                } else {
                    f32::INFINITY
                };
                feeds[i].push(air.map_or(feed, |fastest| feed.min(fastest).min(near).min(own)));
            }
            run.clear();
            wanted.clear();
            along.clear();
        };
    for (i, m) in plan.moves.iter().enumerate() {
        let forces = &load.moves[i];
        if forces.is_empty() {
            if m.motion != Motion::Feed || distance(m.from, m.to) > 0. {
                smooth(&mut run, &mut wanted, &mut along);
            }
            continue;
        }
        let n = forces.len();
        let piece = (distance(m.from, m.to) / n as f32) as f64;
        let down = m.to[2] < m.from[2] - 1e-4;
        for (k, &force) in forces.iter().enumerate() {
            let feed = if force <= 0. {
                f32::INFINITY
            } else {
                let mut w = m.feed * (f.target / force).powf(1. / (1. - f.mc));
                let facing = load.facing[i][k];
                if facing > 0. {
                    w = w.min(THICKEST * m.feed / facing);
                }
                if down {
                    w = w.min(m.feed);
                }
                let steps = ((w / f.programmed).ln() / FEED_STEP.ln() + 1e-4).floor();
                (f.programmed * FEED_STEP.powf(steps))
                    .min(f.fastest)
                    .min(m.feed * FASTEST)
                    .max(m.feed * SLOWEST)
            };
            // A support pass (`Plan::keep_air`) may shave a sliver no
            // sample measures: never fed up.
            let support = plan.keep_air.get(i).copied().unwrap_or(false);
            let air = if support { m.feed } else { flying };
            run.push((i, (force <= 0.).then_some(air)));
            wanted.push(feed);
            along.push(travelled + (k as f64 + 0.5) * piece);
        }
        travelled += piece * n as f64;
    }
    smooth(&mut run, &mut wanted, &mut along);
    // The pieces of each move: its samples from and to, and its feed.
    let mut pieces: Vec<Vec<(usize, usize, f32)>> = feeds
        .iter()
        .map(|feeds| {
            let mut out: Vec<(usize, usize, f32)> = vec![];
            for (k, &feed) in feeds.iter().enumerate() {
                match out.last_mut() {
                    Some(last) if last.2 == feed => last.1 = k + 1,
                    _ => out.push((k, k + 1, feed)),
                }
            }
            out
        })
        .collect();
    let count: usize = pieces.iter().map(|p| p.len().max(1)).sum();
    if count > f.most {
        for p in pieces.iter_mut().filter(|p| p.len() > 1) {
            let slowest = p.iter().map(|p| p.2).fold(f32::INFINITY, f32::min);
            *p = vec![(0, p.last().unwrap().1, slowest)];
        }
    }
    let mut moves = Vec::with_capacity(count.min(f.most));
    let mut starts = Vec::with_capacity(plan.moves.len());
    let mut forces = vec![];
    let mut facing = vec![];
    let mut time = 0.;
    let mut held = Held {
        target: f.target,
        slowest: f32::INFINITY,
        fastest: 0.,
    };
    for (i, m) in plan.moves.iter().enumerate() {
        starts.push(moves.len());
        if pieces[i].is_empty() {
            let seconds = m.end - m.start;
            moves.push(Move {
                start: time,
                end: time + seconds,
                ..m.clone()
            });
            time += seconds;
            forces.push(load.moves[i].clone());
            facing.push(load.facing[i].clone());
            continue;
        }
        let n = load.moves[i].len() as f32;
        let at = |k: usize| -> [f32; 3] {
            std::array::from_fn(|c| m.from[c] + (m.to[c] - m.from[c]) * k as f32 / n)
        };
        for &(a, b, feed) in &pieces[i] {
            let (from, to) = (at(a), at(b));
            let seconds = distance(from, to) / feed * 60.;
            moves.push(Move {
                from,
                to,
                feed,
                start: time,
                end: time + seconds,
                ..m.clone()
            });
            time += seconds;
            // A tooth's force goes as the feed to 1 − mc.
            let scale = (feed / m.feed).powf(1. - f.mc);
            let piece: Vec<f32> = load.moves[i][a..b].iter().map(|f| f * scale).collect();
            if piece.iter().any(|f| *f > 0.) {
                held.slowest = held.slowest.min(feed);
                held.fastest = held.fastest.max(feed);
            }
            forces.push(piece);
            facing.push(load.facing[i][a..b].to_vec());
        }
    }
    if held.fastest == 0. {
        (held.slowest, held.fastest) = (f.programmed, f.programmed);
    }
    let peak = forces.iter().flatten().copied().fold(0., f32::max);
    let keep_air = if plan.keep_air.is_empty() {
        vec![]
    } else {
        starts
            .iter()
            .enumerate()
            .flat_map(|(i, &start)| {
                let end = starts.get(i + 1).copied().unwrap_or(moves.len());
                std::iter::repeat_n(plan.keep_air.get(i).copied().unwrap_or(false), end - start)
            })
            .collect()
    };
    Refed {
        moves,
        starts,
        keep_air,
        duration: time,
        load: std::sync::Arc::new(ToothLoad {
            moves: forces,
            facing,
            peak,
        }),
        held,
    }
}
/// Of each of `values`, at `along` (rising), the least (or the most, with
/// `most`) of those within `reach` of it.
fn slide(values: &[f32], along: &[f64], reach: f64, most: bool) -> Vec<f32> {
    let beats = |a: f32, b: f32| if most { a >= b } else { a <= b };
    let mut out = Vec::with_capacity(values.len());
    let mut window = std::collections::VecDeque::new();
    let mut next = 0;
    for j in 0..values.len() {
        while next < values.len() && along[next] <= along[j] + reach {
            while window
                .back()
                .is_some_and(|&b| beats(values[next], values[b]))
            {
                window.pop_back();
            }
            window.push_back(next);
            next += 1;
        }
        while window.front().is_some_and(|&f| along[f] < along[j] - reach) {
            window.pop_front();
        }
        out.push(values[window[0]]);
    }
    out
}
/// The colour a path is drawn in where one tooth takes `force` of the
/// operation's `peak`: blue light, through violet, to red at the peak; grey
/// where it cuts nothing.
pub fn colour(force: f32, peak: f32) -> [u8; 4] {
    if force <= 0. || peak <= 0. {
        return [150, 150, 150, 255];
    }
    let t = (force / peak).clamp(0., 1.);
    let stops = [[34., 114., 176.], [140., 70., 170.], [214., 39., 40.]];
    let (a, b, u) = if t < 0.5 {
        (stops[0], stops[1], t * 2.)
    } else {
        (stops[1], stops[2], t * 2. - 1.)
    };
    let c: [f32; 3] = std::array::from_fn(|k| a[k] + (b[k] - a[k]) * u);
    [
        c[0].round() as u8,
        c[1].round() as u8,
        c[2].round() as u8,
        255,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Machine, Setup};
    use crate::volume::Volume;
    const STOCK: Bounds = Bounds {
        min: [-50., -30., 0.],
        max: [50., 30., 40.],
    };
    fn frame() -> Frame {
        let mesh =
            crate::mesh::normalize(vec![[[0., 0., 0.], [20., 0., 0.], [0., 20., 10.]]]).unwrap();
        Frame::new(&Setup::new(Machine::new(3), "G54"), &mesh)
    }
    /// Steel's kc1.1 and mc (`forces::KC`).
    const KC: (f32, f32) = (1700., 0.25);
    /// A Ø10 flat end mill's simulation of feed moves through `points` at
    /// 600 mm/min, on `STOCK` less `earlier`'s cuts.
    fn sim(
        points: &[[f32; 3]],
        shape: CutterShape,
        earlier: &[([f32; 3], [f32; 3])],
    ) -> Simulation {
        let moves: Vec<_> = points[1..]
            .iter()
            .map(|&p| (Motion::Feed, p, 600.))
            .collect();
        let mut plan = crate::post::fixture::plan(points[0], &moves);
        (plan.stock, plan.radius, plan.flute_length, plan.shape) = (STOCK, 5., 20., shape);
        plan.strategy = Strategy::Pocket;
        let mut volume = Volume::new(STOCK, frame());
        for &(a, b) in earlier {
            volume.cut(a, b, 5., 20., CutterShape::Flat, frame(), (0, 0));
        }
        let mut sim = Simulation::new(plan);
        (sim.initial_volume, sim.frame) = (Some(volume), Some(frame()));
        sim
    }
    /// Its load, 2 flutes at 6000 rpm: a 0.05 mm feed per tooth.
    fn load(sim: &Simulation, helix: f32) -> ToothLoad {
        measure(sim, 2., 6000., None, helix).unwrap()
    }
    /// Its load midway along move 0.
    fn middle(sim: &Simulation, helix: f32) -> f32 {
        load(sim, helix).at(0, 0.5).unwrap()
    }
    #[test]
    fn a_slots_tooth_takes_the_whole_feed_per_tooth_up_its_depth() {
        // A Ø10 slot 4 mm deep: with straight flutes the tooth facing the
        // feed takes a 0.05 mm chip all the way up, kc1.1 × h^0.75 × ap.
        let slot = sim(&[[-60., 0., 36.], [60., 0., 36.]], CutterShape::Flat, &[]);
        let expected = KC.0 * 0.05_f32.powf(1. - KC.1) * 4.;
        let straight = middle(&slot, 0.);
        assert!(
            (straight / expected - 1.).abs() < 0.03,
            "{straight} vs {expected}"
        );
        // A helix spreads the flute round the cut, so it never has all of it
        // at the thickest chip: less, but not far less over 4 mm.
        let wound = middle(&slot, HELIX);
        assert!(
            wound < straight && wound > 0.85 * straight,
            "{wound} vs {straight}"
        );
        // In the air, and over what an earlier cut took, nothing.
        let above = sim(&[[-60., 0., 41.], [60., 0., 41.]], CutterShape::Flat, &[]);
        assert_eq!(load(&above, HELIX).peak, 0.);
        let cleared = sim(
            &[[-60., 0., 36.], [60., 0., 36.]],
            CutterShape::Flat,
            &[([-60., 0., 35.], [60., 0., 35.])],
        );
        assert_eq!(load(&cleared, HELIX).peak, 0.);
        // Back along the slot and over it again to its end, where the wall
        // ahead is never cut: nothing, to the last piece.
        let again = load(
            &sim(
                &[
                    [-60., 0., 36.],
                    [0., 0., 36.],
                    [-60., 0., 36.],
                    [0., 0., 36.],
                ],
                CutterShape::Flat,
                &[],
            ),
            HELIX,
        );
        assert!(again.moves[0].last().is_some_and(|f| *f > 0.));
        assert!(
            again.moves[1..].iter().flatten().all(|f| *f == 0.),
            "{:?}",
            again.moves
        );
    }
    #[test]
    fn a_shallow_cut_with_long_flutes_is_measured_at_its_depth() {
        // 60 mm flutes, slots 1 and 0.75 mm deep, straight flutes: the force
        // goes with the depth, to a quarter of a millimetre, not with how
        // long a piece of the flute is.
        let slot = |depth: f32| {
            let mut s = sim(
                &[[-60., 0., 40. - depth], [60., 0., 40. - depth]],
                CutterShape::Flat,
                &[],
            );
            s.plan.flute_length = 60.;
            middle(&s, 0.)
        };
        let full = KC.0 * 0.05_f32.powf(1. - KC.1) * 1.;
        assert!(
            (slot(1.) / full - 1.).abs() < 0.03,
            "{} vs {full}",
            slot(1.)
        );
        assert!(
            (slot(0.75) / slot(1.) - 0.75).abs() < 0.01,
            "{} vs {}",
            slot(0.75),
            slot(1.)
        );
    }
    #[test]
    fn a_light_side_cut_takes_a_thinner_chip() {
        // An earlier slot along Y = 0 leaves a wall at Y 5; 1.25 mm (a
        // quarter of the radius) into it, the edge cuts from 90° to where
        // sin ψ = 0.75: the thickest chip is 0.05 × cos ψ = 0.033 mm.
        let earlier = [([-60., 0., 35.], [60., 0., 35.])];
        let side = sim(
            &[[-60., 1.25, 36.], [60., 1.25, 36.]],
            CutterShape::Flat,
            &earlier,
        );
        let thinned = 0.05 * (1. - 0.75_f32.powi(2)).sqrt();
        let expected = KC.0 * thinned.powf(1. - KC.1) * 4.;
        let got = middle(&side, 0.);
        assert!((got / expected - 1.).abs() < 0.05, "{got} vs {expected}");
        // Half the cutter in the wall, the tooth facing the feed is in it:
        // the slot's load.
        let half = sim(
            &[[-60., 5., 36.], [60., 5., 36.]],
            CutterShape::Flat,
            &earlier,
        );
        let slot = KC.0 * 0.05_f32.powf(1. - KC.1) * 4.;
        let got = middle(&half, 0.);
        assert!((got / slot - 1.).abs() < 0.03, "{got} vs {slot}");
    }
    #[test]
    fn a_plunge_cuts_with_the_end_and_a_corner_loads_more_than_the_wall() {
        // Straight down 2 mm into the top: the end's edge, centre to rim,
        // takes the whole feed per tooth.
        let plunge = sim(&[[0., 0., 42.], [0., 0., 38.]], CutterShape::Flat, &[]);
        let expected = KC.0 * 0.05_f32.powf(1. - KC.1) * 5.;
        let got = load(&plunge, 0.).at(0, 1.).unwrap();
        assert!((got / expected - 1.).abs() < 0.03, "{got} vs {expected}");
        // Passes along X and along Y clear X and Y under 0 (to 5 beyond):
        // the material left is an L round an inside corner at (0, 0).
        let mut earlier = vec![];
        for at in [-30., -22., -14., -6., 0.] {
            earlier.push(([-60., at, 35.], [0., at, 35.]));
            earlier.push(([at, -60., 35.], [at, 0., 35.]));
        }
        // A light cut 1 mm into the wall along X, round the corner and 1 mm
        // into the wall along Y: there the cutter wraps into the material,
        // more of the tooth in the cut than along either wall.
        let corner = sim(
            &[[-60., 1., 36.], [1., 1., 36.], [1., -60., 36.]],
            CutterShape::Flat,
            &earlier,
        );
        let l = load(&corner, HELIX);
        let wall = l.at(0, 0.5).unwrap();
        let worst = l.moves[0..2].iter().flatten().copied().fold(0., f32::max);
        assert!(wall > 0. && worst > 1.3 * wall, "{worst} vs {wall}");
    }
    #[test]
    fn a_ball_ends_load_counts_its_round_end() {
        // A Ø10 ball slotting 4 mm deep, straight flutes: the tooth facing
        // the feed has its round edge in the cut from the tip up to where
        // cos β = 1 − 4/5, each bit of it a chip of 0.05 × sin β over R dβ.
        // Thinner chips over a longer edge than a flat end mill's 4 mm.
        let points = [[-60., 0., 36.], [60., 0., 36.]];
        let ball = middle(&sim(&points, CutterShape::Ball, &[]), 0.);
        let top = 0.2_f32.acos();
        let expected: f32 = (0..1000)
            .map(|i| (i as f32 + 0.5) * top / 1000.)
            .map(|b| KC.0 * (0.05 * b.sin()).powf(1. - KC.1) * 5. * top / 1000.)
            .sum();
        assert!((ball / expected - 1.).abs() < 0.04, "{ball} vs {expected}");
    }
    /// Holding `target` N at 600 mm/min programmed, at most `fastest`.
    fn feeding(target: f32, fastest: f32) -> Feeding {
        Feeding {
            target,
            mc: KC.1,
            programmed: 600.,
            fastest,
            most: usize::MAX,
        }
    }
    /// The feed and the force on a tooth of `refed`'s piece of the move
    /// that lies over `p`, with that move's index.
    fn piece_at(refed: &Refed, p: [f32; 3]) -> (f32, f32) {
        let (i, m) = (refed.moves.iter().enumerate())
            .filter(|(_, m)| m.motion == Motion::Feed)
            .find(|(_, m)| {
                let (a, b) = (distance(m.from, p), distance(p, m.to));
                (a + b - distance(m.from, m.to)).abs() < 1e-3
            })
            .unwrap();
        let force = refed.load.moves[i].iter().copied().fold(0., f32::max);
        (m.feed, force)
    }
    /// A Ø10 slot 4 mm deep along Y 0, out of the stock and over, then back
    /// 2.5 mm (a quarter of the diameter) into its wall at Y 5.
    const SLOT_THEN_SIDE: [[f32; 3]; 4] = [
        [-60., 0., 36.],
        [60., 0., 36.],
        [60., 2.5, 36.],
        [-60., 2.5, 36.],
    ];
    #[test]
    fn a_light_side_cut_after_a_slot_is_fed_up_by_the_force_ratio() {
        let s = sim(&SLOT_THEN_SIDE, CutterShape::Flat, &[]);
        let load = load(&s, HELIX);
        let middle = |i: usize| {
            let p = &load.moves[i];
            let n = p.len();
            let m = &p[n / 3..2 * n / 3];
            (
                m.iter().copied().fold(f32::MAX, f32::min),
                m.iter().copied().fold(0., f32::max),
            )
        };
        let (slot, side) = (middle(0), middle(2));
        assert!(side.1 < 0.95 * slot.0, "{side:?} vs {slot:?}");
        // Holding the slot's force: the slot keeps 600 mm/min, the side cut
        // goes (F1 / F2)^(1 / (1 − mc)) faster, rounded down to the tenth.
        let target = slot.1;
        let refed = refeed(&s.plan, &load, feeding(target, f32::INFINITY));
        let (feed, force) = piece_at(&refed, [0., 0., 36.]);
        assert_eq!(feed, 600.);
        assert!(force <= target * 1.0001, "{force} vs {target}");
        let (feed, force) = piece_at(&refed, [0., 2.5, 36.]);
        let ratio = |f: f32| (target / f).powf(1. / (1. - KC.1));
        assert!(
            feed <= 600. * ratio(side.0) && feed > 600. * ratio(side.1) / FEED_STEP,
            "{feed} vs {} – {}",
            600. * ratio(side.1),
            600. * ratio(side.0)
        );
        // The force there is the target's, no more and less only by the
        // rounding.
        assert!(
            force <= target * 1.0001 && force > target / FEED_STEP.powf(1. - KC.1) * 0.97,
            "{force} vs {target}"
        );
        assert_eq!(refed.held.target, target);
        assert!(refed.held.slowest <= 600. && refed.held.fastest >= feed);
    }
    #[test]
    fn a_held_feed_stays_between_the_machines_most_and_a_quarter_of_the_programmed() {
        let s = sim(&SLOT_THEN_SIDE, CutterShape::Flat, &[]);
        let load = load(&s, HELIX);
        let slot = load.at(0, 0.5).unwrap();
        // The side cut wants more than the machine's 650 mm/min.
        let refed = refeed(&s.plan, &load, feeding(slot, 650.));
        assert_eq!(piece_at(&refed, [0., 2.5, 36.]).0, 650.);
        assert!(refed.moves.iter().all(|m| m.feed <= 650. || m.feed == 600.));
        // A tenth of the slot's force wants a twentieth of the feed: a
        // quarter of it is the least.
        let refed = refeed(&s.plan, &load, feeding(slot / 10., f32::INFINITY));
        assert_eq!(piece_at(&refed, [0., 0., 36.]).0, 150.);
        assert_eq!(piece_at(&refed, [0., 2.5, 36.]).0, 150.);
        assert!(refed.moves.iter().all(|m| m.feed >= 150.));
    }
    #[test]
    fn moves_in_the_air_take_the_fastest_feed() {
        // Above the stock throughout: the machine's most feed, but within the
        // cutter's radius and a sample (6 mm) of either end, where it may
        // meet material no sample saw, its own.
        let above = sim(&[[-60., 0., 41.], [60., 0., 41.]], CutterShape::Flat, &[]);
        let ends = |refed: &Refed, middle: f32| {
            assert_eq!(piece_at(refed, [0., 0., 41.]).0, middle);
            for m in &refed.moves {
                let far = m.from[0].abs().max(m.to[0].abs());
                assert!(
                    m.feed == middle || (m.feed == 600. && far > 60. - 7.),
                    "{m:?}"
                );
            }
            assert_eq!(refed.moves[0].feed, 600.);
            assert_eq!(refed.moves.last().unwrap().feed, 600.);
        };
        ends(
            &refeed(&above.plan, &load(&above, HELIX), feeding(100., 1000.)),
            1000.,
        );
        // On a machine with no most feed, `FASTEST` times the programmed.
        let load_above = load(&above, HELIX);
        ends(
            &refeed(&above.plan, &load_above, feeding(100., f32::INFINITY)),
            600. * FASTEST,
        );
        // A support pass (`Plan::keep_air`) keeps its own feed throughout.
        let mut support = above.clone();
        support.plan.keep_air = vec![true; support.plan.moves.len()];
        let refed = refeed(&support.plan, &load_above, feeding(100., 1000.));
        assert!(
            refed.moves.iter().all(|m| m.feed == 600.),
            "{:?}",
            refed.moves
        );
        // Back along the slot it cut, over nothing: slowed within a cutter's
        // reach of the slot's end and of the end of the pass, 1000 mm/min
        // between.
        let back = sim(
            &[[-60., 0., 36.], [60., 0., 36.], [-60., 0., 36.]],
            CutterShape::Flat,
            &[],
        );
        let load = load(&back, HELIX);
        let refed = refeed(&back.plan, &load, feeding(load.peak / 2., 1000.));
        assert!(piece_at(&refed, [0., 0., 36.]).0 < 400.);
        let last = refed.moves.last().unwrap();
        assert_eq!((last.to, last.feed), ([-60., 0., 36.], 600.));
        let returning = (refed.moves.iter())
            .find(|m| m.to[0] < m.from[0] && m.from[0] >= 0. && m.to[0] <= 0.)
            .unwrap();
        assert_eq!(returning.feed, 1000.);
        assert!(
            returning.from[0] > 60. - 3. * 5. - 1. && returning.to[0] < -60. + 7.,
            "{returning:?}"
        );
    }
    #[test]
    fn the_tool_slows_from_the_air_feed_before_it_meets_material() {
        // A Ø10 and a Ø3 slot into the stock's −X side from 1 to 20 mm out,
        // starting at points that put the samples anywhere about the
        // contact: nowhere the cutter touches material is it faster than a
        // cut's own feed (`flying` only in the air).
        for radius in [5., 1.5] {
            for out in [1., 2.3, 6., 20.] {
                for shift in [0., 0.07, 0.19, 0.31, 0.43] {
                    let start = -50. - radius - out - shift;
                    let mut s = sim(&[[start, 0., 36.], [0., 0., 36.]], CutterShape::Flat, &[]);
                    s.plan.radius = radius;
                    let load = load(&s, HELIX);
                    let slot = load.at(0, 0.99).unwrap();
                    let refed = refeed(&s.plan, &load, feeding(slot, 2000.));
                    let cutting = cut_feed(&refed);
                    let touching = touching(&refed, radius);
                    for &(feed, x) in &touching {
                        assert!(
                            feed <= cutting,
                            "Ø{} {out} {shift}: F{feed} at X{x}",
                            2. * radius
                        );
                    }
                    // Fed up only clear of both the cut and the run's start,
                    // where material no sample saw may lie; where the
                    // samples fall decides in between.
                    let step = sample_step(&s.plan);
                    let fastest = (refed.moves.iter()).map(|m| m.feed).fold(0., f32::max);
                    if out <= radius || out >= 2. * (radius + step) + step {
                        assert_eq!(
                            fastest > cutting,
                            out > radius,
                            "Ø{} {out} {shift}: F{fastest}",
                            2. * radius
                        );
                    }
                }
            }
        }
    }
    #[test]
    fn a_feed_that_ends_just_inside_the_stock_keeps_its_own_feed_there() {
        // A feed from the air that stops just past where it meets the stock
        // and retracts, too short in it for a sample to find the cut, or so
        // that one does: it touches the material at no more than its
        // programmed feed, or than a cut's own.
        for radius in [5., 1.5] {
            for out in [1., 3., 8.] {
                for into in [0.05, 0.2, 0.4] {
                    for shift in [0., 0.13, 0.29] {
                        let start = -50. - radius - out - shift;
                        let end = -50. - radius + into;
                        let moves = [
                            (Motion::Feed, [end, 0., 36.], 600.),
                            (Motion::Rapid, [end, 0., 50.], 6000.),
                        ];
                        let mut plan = crate::post::fixture::plan([start, 0., 36.], &moves);
                        (plan.stock, plan.radius, plan.flute_length) = (STOCK, radius, 20.);
                        plan.strategy = Strategy::Pocket;
                        let mut s = Simulation::new(plan);
                        (s.initial_volume, s.frame) =
                            (Some(Volume::new(STOCK, frame())), Some(frame()));
                        let load = load(&s, HELIX);
                        let refed = refeed(&s.plan, &load, feeding(100., 2000.));
                        let touching = touching(&refed, radius);
                        assert!(
                            !touching.is_empty(),
                            "Ø{} {out} {into}: no contact",
                            2. * radius
                        );
                        let most = cut_feed(&refed).max(600.);
                        for &(feed, x) in &touching {
                            assert!(
                                feed <= most,
                                "Ø{} {out} {into} {shift}: F{feed} at X{x}",
                                2. * radius
                            );
                        }
                    }
                }
            }
        }
    }
    /// The fastest feed of `refed`'s pieces where a sample finds the cut.
    fn cut_feed(refed: &Refed) -> f32 {
        (refed.moves.iter().zip(&refed.load.moves))
            .filter(|(_, f)| f.iter().any(|f| *f > 0.))
            .map(|(m, _)| m.feed)
            .fold(0., f32::max)
    }
    /// Where `refed`'s feeds, measured again every 0.02 mm, touch the
    /// stock: the feed there and its X.
    fn touching(refed: &Refed, radius: f32) -> Vec<(f32, f32)> {
        let feeds: Vec<&Move> = (refed.moves.iter())
            .filter(|m| m.motion == Motion::Feed)
            .collect();
        let pieces = |m: &Move| (distance(m.from, m.to) / 0.02).ceil().max(1.) as usize;
        let mut fine = vec![feeds[0].from];
        let mut rate = vec![];
        for m in &feeds {
            let n = pieces(m);
            for k in 1..=n {
                fine.push(std::array::from_fn(|i| {
                    m.from[i] + (m.to[i] - m.from[i]) * k as f32 / n as f32
                }));
                rate.push(m.feed);
            }
        }
        let mut dense = sim(&fine, CutterShape::Flat, &[]);
        dense.plan.radius = radius;
        (load_of(&dense).iter().enumerate())
            .filter(|(_, f)| **f > 0.)
            .map(|(k, _)| (rate[k], fine[k + 1][0]))
            .collect()
    }
    /// The force on a tooth at each move of `sim`, one sample each.
    fn load_of(sim: &Simulation) -> Vec<f32> {
        (load(sim, HELIX).moves.iter())
            .map(|m| m.iter().copied().fold(0., f32::max))
            .collect()
    }
    #[test]
    fn a_short_fast_stretch_is_smoothed_away_and_a_short_slow_one_kept() {
        // 30 samples 0.5 mm apart along a 15 mm move, the force on a tooth
        // the target throughout but for one sample.
        let plan = crate::post::fixture::plan([0., 0., 0.], &[(Motion::Feed, [15., 0., 0.], 600.)]);
        let with = |force: f32| ToothLoad {
            moves: vec![
                (0..30)
                    .map(|k| if k == 15 { force } else { 100. })
                    .collect(),
            ],
            facing: vec![vec![1.; 30]],
            peak: 100_f32.max(force),
        };
        // Lighter there, wanting a faster feed for 0.5 mm: kept at 600.
        let refed = refeed(&plan, &with(25.), feeding(100., f32::INFINITY));
        assert_eq!(refed.moves.len(), 1);
        assert_eq!(refed.moves[0].feed, 600.);
        // Twice as heavy there: slower there, rounded down to the tenth.
        let refed = refeed(&plan, &with(200.), feeding(100., f32::INFINITY));
        assert_eq!(refed.moves.len(), 3, "{:?}", refed.moves);
        let slow = &refed.moves[1];
        let wanted = 600. * 0.5_f32.powf(4. / 3.);
        assert!(
            slow.feed <= wanted && slow.feed > wanted / FEED_STEP,
            "{}",
            slow.feed
        );
        assert_eq!((slow.from[0], slow.to[0]), (7.5, 8.));
        assert!(refed.load.moves.iter().flatten().all(|f| *f <= 100.0001));
    }
    #[test]
    fn splitting_stays_within_the_move_count_and_retimes_the_plan() {
        let mut s = sim(&SLOT_THEN_SIDE, CutterShape::Flat, &[]);
        s.plan.keep_air = (0..s.plan.moves.len()).map(|i| i % 2 == 0).collect();
        let load = load(&s, HELIX);
        let target = load.at(0, 0.5).unwrap() * 0.8;
        let refed = refeed(&s.plan, &load, feeding(target, 5000.));
        let samples: usize = load.moves.iter().map(|m| m.len().max(1)).sum();
        assert!(refed.moves.len() > s.plan.moves.len() && refed.moves.len() <= samples);
        // One after the other, each timed at its feed.
        let mut time = 0.;
        for (a, b) in refed.moves.iter().zip(&refed.moves[1..]) {
            assert_eq!(a.to, b.from);
            assert_eq!(a.end, b.start);
        }
        for m in &refed.moves {
            assert!((m.end - m.start - distance(m.from, m.to) / m.feed * 60.).abs() < 1e-3);
            time += m.end - m.start;
        }
        assert!((refed.duration - time).abs() < 1e-3);
        // Each move's first piece starts where it did.
        for (i, (m, &start)) in s.plan.moves.iter().zip(&refed.starts).enumerate() {
            assert_eq!(refed.moves[start].from, m.from);
            let end = refed
                .starts
                .get(i + 1)
                .copied()
                .unwrap_or(refed.moves.len());
            assert!(
                refed.keep_air[start..end]
                    .iter()
                    .all(|&keep| keep == s.plan.keep_air[i])
            );
        }
        assert_eq!(refed.moves.last().unwrap().end, refed.duration);
        // As many pieces as the load has: one per move, at its slowest.
        let most = refeed(
            &s.plan,
            &load,
            Feeding {
                most: s.plan.moves.len(),
                ..feeding(target, 5000.)
            },
        );
        assert_eq!(most.moves.len(), s.plan.moves.len());
        for (m, planned) in most.moves.iter().zip(&s.plan.moves) {
            assert_eq!((m.from, m.to), (planned.from, planned.to));
            // The slowest of the pieces it was split in.
            let on = |p: &Move| {
                [p.from, p.to].iter().all(|&q| {
                    distance(planned.from, q) + distance(q, planned.to)
                        - distance(planned.from, planned.to)
                        < 1e-3
                })
            };
            let slowest = (refed.moves.iter().filter(|p| on(p)))
                .map(|p| p.feed)
                .fold(f32::INFINITY, f32::min);
            assert_eq!(m.feed, slowest);
        }
    }
    #[test]
    fn the_levels_depth_counts_the_levels_that_cut_not_the_travel_over_them() {
        // Slots 1 mm deep from the stock's top at 40, each followed, as an
        // Adaptive does, by travel at the feed half a millimetre over the
        // level it cleared; the last level only 0.5 mm deeper.
        let points = [
            [-60., 0., 39.],
            [60., 0., 39.],
            [60., 0., 39.5],
            [-60., 0., 39.5],
            [-60., 0., 38.],
            [60., 0., 38.],
            [60., 0., 38.5],
            [-60., 0., 38.5],
            [-60., 0., 37.5],
            [60., 0., 37.5],
        ];
        let s = sim(&points, CutterShape::Flat, &[]);
        let load = load(&s, HELIX);
        // Nothing is cut along the travel.
        assert!(
            load.moves[2].iter().all(|f| *f == 0.),
            "{:?}",
            load.moves[2]
        );
        assert_eq!(
            level_depth(&s.plan, &load, |_| s.plan.stock.max[2]),
            Some(1.)
        );
    }
    #[test]
    fn the_nominal_force_is_the_one_measured_along_that_cut() {
        let cut = |width| NominalCut {
            shape: CutterShape::Flat,
            radius: 5.,
            flute: 20.,
            clockwise: true,
            climb: true,
            width,
            depth: 4.,
        };
        let nominal = |width| nominal(&cut(width), 2., 6000., 600., None).unwrap();
        let slot = middle(
            &sim(&[[-60., 0., 36.], [60., 0., 36.]], CutterShape::Flat, &[]),
            HELIX,
        );
        assert!(
            (nominal(None) / slot - 1.).abs() < 0.03,
            "{} vs {slot}",
            nominal(None)
        );
        let earlier = [([-60., 0., 35.], [60., 0., 35.])];
        let side = sim(
            &[[-60., 1.25, 36.], [60., 1.25, 36.]],
            CutterShape::Flat,
            &earlier,
        );
        let side = middle(&side, HELIX);
        let light = nominal(Some(1.25));
        assert!((light / side - 1.).abs() < 0.05, "{light} vs {side}");
        // The same numbers are worked out once.
        assert_eq!(nominal(Some(1.25)), light);
    }
    #[test]
    fn a_held_pocket_exports_and_checks_the_feeds_it_was_timed_at() {
        use std::collections::BTreeSet;
        let (mut doc, mesh) = crate::post::fixture::job();
        // The Face and the Pocket.
        for op in &mut doc.setups[0].operations[2..] {
            op.enabled = false;
        }
        let run = |doc: &Document| {
            let mut sims = vec![];
            crate::simulation::sequence(doc, &mesh, None, |_, _, r| sims.push(r.unwrap()));
            sims
        };
        // A project saved before the choice, without it, keeps its feeds:
        // those a new operation's programmed feed gives.
        let programmed = run(&doc);
        let mut old = doc.clone();
        for op in &mut old.setups[0].operations {
            op.p.remove("feed_follows");
            op.p.remove("tooth_force");
        }
        let old = run(&old);
        for (a, b) in programmed.iter().zip(&old) {
            assert!(a.held.is_none() && b.held.is_none());
            assert_eq!(a.plan.moves.len(), b.plan.moves.len());
            assert!((a.plan.moves.iter().zip(&b.plan.moves)).all(|(a, b)| a.feed == b.feed));
        }
        doc.setups[0].operations[1]
            .p
            .insert("feed_follows".into(), FEED_FOLLOWS[1].into());
        let held = run(&doc);
        // The Face is as it was; the Pocket's cutting moves take several
        // feeds, timed at them.
        assert_eq!(held[0].plan.moves.len(), programmed[0].plan.moves.len());
        let pocket = &held[1];
        let h = pocket.held.unwrap();
        assert!(h.slowest < h.fastest, "{h:?}");
        assert!(pocket.plan.moves.len() > programmed[1].plan.moves.len());
        assert!(pocket.plan.duration != programmed[1].plan.duration);
        let timed: f32 = (pocket.plan.moves.iter()).map(|m| m.end - m.start).sum();
        assert!((timed - pocket.plan.duration).abs() < 0.01);
        let feeds: BTreeSet<String> = (pocket.plan.moves.iter())
            .filter(|m| m.motion == Motion::Feed)
            .map(|m| format!("{:.1}", m.feed))
            .collect();
        assert!(feeds.len() >= 3, "{feeds:?}");
        // The export writes each of them as an F word.
        let job = crate::post::post_job(&doc, &mesh);
        let program = &job.setups[0].result.as_ref().unwrap()[0].text;
        let words: BTreeSet<&str> = (program.split_whitespace())
            .filter_map(|w| w.strip_prefix('F'))
            .collect();
        for f in &feeds {
            assert!(words.contains(f.as_str()), "F{f} not in {words:?}");
        }
        // The holding check sees them too.
        let bites: BTreeSet<String> = (pocket.bites().iter())
            .map(|b| format!("{:.0}", b.feed))
            .collect();
        assert!(bites.len() >= 3, "{bites:?}");
        // The load along the moves came with them, at the target or under.
        let load = pocket.tooth_load.as_ref().unwrap();
        assert_eq!(load.moves.len(), pocket.plan.moves.len());
        assert!(load.peak > 0.5 * h.target);
    }
    #[test]
    fn every_cnc_milling_strategy_offers_the_held_feed() {
        let mill = Machine::new(3);
        for s in [
            Strategy::Face,
            Strategy::Adaptive,
            Strategy::Pocket,
            Strategy::Contour,
            Strategy::Waterline,
            Strategy::Parallel,
            Strategy::Chamfer,
            Strategy::Groove,
        ] {
            assert!(offered(s, &mill), "{s:?}");
            assert!(!offered(s, &crate::manual::defaults()), "{s:?}");
        }
        for s in [
            Strategy::Drill,
            Strategy::Box,
            Strategy::Turn,
            Strategy::Scallop,
            Strategy::Indexed,
            Strategy::Rotary,
            Strategy::FiveAxis,
        ] {
            assert!(!offered(s, &mill), "{s:?}");
        }
    }
    #[test]
    fn the_paths_force_is_the_median_over_the_length_cut() {
        // 10 mm at 100 N in two pieces, then 30 mm in three: one in the
        // air, 10 mm at 20 N and 10 mm at 300 N.
        let plan = crate::post::fixture::plan(
            [0., 0., 0.],
            &[
                (Motion::Feed, [10., 0., 0.], 600.),
                (Motion::Feed, [40., 0., 0.], 600.),
            ],
        );
        let load = |second: Vec<f32>| ToothLoad {
            moves: vec![vec![100., 100.], second],
            facing: vec![vec![1.; 2], vec![1.; 3]],
            peak: 300.,
        };
        assert_eq!(typical(&plan, &load(vec![0., 20., 300.])), Some(100.));
        // By length, not by sample: 10 mm at 100 N, 20 mm at 20 N.
        assert_eq!(typical(&plan, &load(vec![0., 20., 20.])), Some(20.));
        let air = ToothLoad {
            moves: vec![vec![0.; 2], vec![0.; 3]],
            ..load(vec![])
        };
        assert_eq!(typical(&plan, &air), None);
    }
    #[test]
    fn a_held_waterline_holds_a_full_slots_force_and_a_parallel_its_paths() {
        let (mut doc, mesh) = crate::post::fixture::job();
        // The Face, then the Parallel and the Waterline.
        for (k, op) in doc.setups[0].operations.iter_mut().enumerate() {
            op.enabled = matches!(k, 0 | 4 | 5);
            op.p.insert("stepdown".into(), "3".into());
        }
        let run = |doc: &Document| {
            let mut sims = vec![];
            crate::simulation::sequence(doc, &mesh, None, |_, _, r| sims.push(r.unwrap()));
            sims
        };
        let programmed = run(&doc);
        assert!(programmed.iter().all(|s| s.held.is_none()));
        for op in &mut doc.setups[0].operations[4..] {
            op.p.insert("feed_follows".into(), FEED_FOLLOWS[1].into());
        }
        let held = run(&doc);
        let ops = &doc.setups[0].operations;
        let tool = |op: &Operation| doc.tools.iter().find(|t| t.id == op.tool).unwrap();
        for (k, sim) in [(4, &held[1]), (5, &held[2])] {
            let h = sim.held.unwrap_or_else(|| panic!("{:?}", ops[k].strategy));
            assert!(h.slowest < h.fastest, "{:?} {h:?}", ops[k].strategy);
            assert!(sim.plan.moves.len() > programmed[k - 3].plan.moves.len());
        }
        // The Parallel's target: the force along most of its path as
        // programmed.
        let parallel = of(&programmed[1], &doc).unwrap();
        let want = typical(&programmed[1].plan, &parallel).unwrap();
        assert_eq!(held[1].held.unwrap().target, want);
        // The Waterline's: a full slot as deep as its levels, at most its
        // 3 mm stepdown.
        let slot = nominal_cut(&ops[5], tool(&ops[5]), None).unwrap();
        assert_eq!((slot.width, slot.depth), (None, 3.));
        let material = doc.selected_material();
        let (full, _) = target(&ops[5], tool(&ops[5]), material, None, None).unwrap();
        let target = held[2].held.unwrap().target;
        assert!(target > 0. && target <= full * 1.001, "{target} vs {full}");
    }
    #[test]
    fn a_held_indexed_facing_is_measured_at_each_angle_in_its_own_frame() {
        use crate::indexed::tests::{bar_job, cutter, faced_square, prism};
        use crate::simulation::prepare;
        // The Ø40 bar faced square, a side at each of four angles.
        let mesh = prism(30.);
        let op = faced_square(&cutter("12"), &mesh);
        let mut doc = bar_job(&mesh, 20., "100", "-150", "90, 180, 270, 0", op);
        let programmed = prepare(&doc, &mesh, 1, 0).unwrap();
        assert!(programmed.held.is_none());
        let op = &mut doc.setups[1].operations[0];
        op.p.insert("feed_follows".into(), FEED_FOLLOWS[1].into());
        let held = prepare(&doc, &mesh, 1, 0).unwrap();
        let h = held.held.expect("an indexed setup's feed holds the force");
        assert!(h.slowest < h.fastest, "{h:?}");
        assert!(held.plan.moves.len() > programmed.plan.moves.len());
        let timed: f32 = (held.plan.moves.iter()).map(|m| m.end - m.start).sum();
        assert!((timed - held.plan.duration).abs() < 0.01);
        // Each piece keeps its move's A, the angles in the same order.
        let angles = |s: &Simulation| {
            let mut v: Vec<f32> = vec![];
            for a in s.plan.moves.iter().map(|m| m.a.unwrap()) {
                if v.last() != Some(&a) {
                    v.push(a);
                }
            }
            v
        };
        assert_eq!(angles(&held), [90., 180., 270., 0.]);
        assert_eq!(angles(&held), angles(&programmed));
        // Every angle meets its own side of the bar, the stock the angles
        // before left: in the setup's frame alone the later angles would
        // pass where the first one had cut, through nothing.
        let load = held.tooth_load.as_ref().unwrap();
        assert_eq!(load.moves.len(), held.plan.moves.len());
        for a in [90., 180., 270., 0.] {
            let most = (held.plan.moves.iter().zip(&load.moves))
                .filter(|(m, _)| m.a == Some(a))
                .flat_map(|(_, f)| f.iter().copied())
                .fold(0., f32::max);
            assert!(most > 0.5 * h.target, "A{a}: {most} vs {}", h.target);
        }
        // The target: the Facing's nominal cut, no deeper than its stepdown.
        let tool = &doc.tools[0];
        let op = &doc.setups[1].operations[0];
        let (full, _) = target(op, tool, doc.selected_material(), None, None).unwrap();
        assert!(h.target <= full * 1.001, "{} vs {full}", h.target);
    }
    #[test]
    fn colours_run_from_blue_to_red() {
        assert_eq!(colour(0., 10.), [150, 150, 150, 255]);
        assert_eq!(colour(1e-6, 10.)[0..3], [34, 114, 176]);
        assert_eq!(colour(10., 10.)[0..3], [214, 39, 40]);
    }
}
