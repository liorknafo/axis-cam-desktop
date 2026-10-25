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
use crate::model::{Document, Strategy, num};
use crate::simulation::{CutterShape, Motion, Simulation};
use crate::stock_surface::Cut;
use crate::volume::{Frame, swept};
use std::f32::consts::TAU;

/// The flutes' helix angle, degrees: a common end mill's.
pub const HELIX: f32 = 30.;
/// The spindle angles a turn is sampled at: every 5°.
const ANGLES: usize = 72;
/// How far ahead of the edge, along its normal, material is looked for, mm.
const AHEAD: f32 = 0.005;
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
    let height = (flute - side_from).max(0.);
    let slices = (height / 0.25).ceil().clamp(1., 40.);
    let dz = height / slices;
    for i in 0..slices as usize {
        add(radius, side_from + (i as f32 + 0.5) * dz, [1., 0.], dz);
    }
    out
}

/// The load on one tooth along `sim`'s moves (`ToothLoad`), cut by a tool
/// with `flutes` wound at `helix` degrees at `rpm` in `material` (None
/// counts as steel, `forces::kc`). None for a plan that is not milled
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
    let stock = volume.frame.transfer_bounds(volume.bounds, frame);
    let bar = volume.round.then(|| crate::stock::bar(volume.bounds));
    let extent = (stock.max[0] - stock.min[0]).max(stock.max[1] - stock.min[1]);
    let mut history = crate::volume::History::new(frame, stock, plan.radius.max(extent / 400.));
    for cut in volume.cuts.iter() {
        history.add(cut.clone());
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
                history.add(Cut {
                    a: m.from,
                    b: m.to,
                    radius: plan.radius,
                    flute: plan.flute_length,
                    shape: plan.shape,
                    frame,
                    source: sim.source,
                })
            })
        })
        .collect();
    // Whether `p` is still material: in the stock, swept by no cut before
    // cut `before`, nor by the move under way from `a` to `b`.
    let solid = |p: [f32; 3], before: u32, a: [f32; 3], b: [f32; 3]| {
        let q = frame.transfer(p, volume.frame);
        (0..3).all(|k| (volume.bounds.min[k]..=volume.bounds.max[k]).contains(&q[k]))
            && bar.is_none_or(|(c, r)| (q[0] - c[0]).hypot(q[1] - c[1]) <= r)
            && !swept(p, a, b, plan.radius, plan.flute_length, plan.shape)
            && !history.swept(p, before)
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
    let total: f32 = plan.moves.iter().filter(|m| cutting(m)).map(length).sum();
    let step = (plan.radius / 3.).clamp(0.05, 1.).max(total / MOST);
    let top = plan.stock.max[2].max(stock.max[2]);
    // The force on one tooth midway along each piece of move `index`.
    let measure_move = |index: usize| -> Vec<f32> {
        let (m, Some(before)) = (&plan.moves[index], numbers[index]) else {
            return vec![];
        };
        let l = length(m);
        let v: [f32; 3] = std::array::from_fn(|k| (m.to[k] - m.from[k]) / l);
        let chip = m.feed / (rpm * flutes);
        let pieces = (l / step).ceil().max(1.) as usize;
        let at =
            |t: f32| -> [f32; 3] { std::array::from_fn(|k| m.from[k] + (m.to[k] - m.from[k]) * t) };
        let mut loads = Vec::with_capacity(pieces);
        for i in 0..pieces {
            // Midway along it, where the cutter goes on along the move: at its
            // end it may turn away from what lies ahead.
            let here = at((i as f32 + 0.5) / pieces as f32);
            let mut tooth = [0_f32; ANGLES];
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
                    if solid(p, before, m.from, here) {
                        // The tooth whose flute's lowest point is `behind`
                        // angles back from here.
                        tooth[(j + ANGLES - behind) % ANGLES] +=
                            kc * (chip * facing).powf(1. - mc) * e.width;
                    }
                }
            }
            loads.push(tooth.into_iter().fold(0., f32::max));
        }
        loads
    };
    // The moves dealt round the cores in turn.
    let workers = std::thread::available_parallelism().map_or(1, |n| n.get());
    let progress = crate::progress::current();
    let mut moves = vec![vec![]; plan.moves.len()];
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
    for (index, loads) in dealt
        .into_iter()
        .collect::<Option<Vec<_>>>()?
        .into_iter()
        .flatten()
    {
        moves[index] = loads;
    }
    let peak = moves.iter().flatten().copied().fold(0., f32::max);
    Some(ToothLoad { moves, peak })
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
    #[test]
    fn colours_run_from_blue_to_red() {
        assert_eq!(colour(0., 10.), [150, 150, 150, 255]);
        assert_eq!(colour(1e-6, 10.)[0..3], [34, 114, 176]);
        assert_eq!(colour(10., 10.)[0..3], [214, 39, 40]);
    }
}
