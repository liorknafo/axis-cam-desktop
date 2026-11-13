//! Round corners on the manual mill (LIO-443): a part's rounded outside
//! vertical corners, each a quarter round between a wall along X and one
//! along Y, cut by the side of an end mill walked round the round in small
//! DRO steps, one axis at a time, as a DRO's arc function steps it. The
//! scallops left between the steps are then filed to the line.
//!
//! The tool centre stays on or outside the round offset by the cutter's
//! radius: each step goes from one point on that offset round to the next
//! by the two one-axis moves whose corner lies outside it, so no move comes
//! nearer the part than the round. The steps are as long as leave at most
//! the operation's scallop between them. The walk goes round the part
//! counter-clockwise seen from above (conventional with the spindle
//! clockwise, the cutter pushed against the feed, as a manual mill's
//! backlash wants), each axis moving one way only, so the backlash is taken
//! up once, before the walk.
use crate::{
    cam::{Builder, Geometry, ON, arcs, meets, number, swept},
    mesh::{Bounds, Mesh},
    model::{Fields, Operation, Setup, Tool, get, num},
    simulation::{Motion, Move, Plan},
};
use std::collections::HashSet;

pub const AVAILABILITY: &str =
    "Available · rounded outside corners walked in DRO steps on a manual mill";
pub const HELP: &str = "Round a part's outside vertical corners on the manual mill: the end mill's side walks round each corner in DRO steps, one axis at a time; the scallops are filed to the line after. Click a rounded corner, or pick all.";
/// Why Round corners is refused on a CNC.
pub const ON_CNC: &str =
    "Round corners runs on a manual mill: a CNC cuts a corner's round with a Contour.";
/// What a pick that is no rounded outside corner says.
pub const NOT_A_CORNER: &str = "That is no rounded outside corner: click the round face of a corner of the part, where two upright walls meet.";
/// What a Round corners with nothing picked says.
pub const NONE_PICKED: &str = "Pick the rounded corners to cut: click the round face of each on the part, or Pick all outside corners.";
/// A new Round corners' settings: a slow gear and the hand feed's pace
/// for a cut on the side, the scallop the issue asks for, a stepover for
/// the walks that take the stock off before the last, and Auto stepdown.
pub const DEFAULTS: [(&str, &str); 5] = [
    ("rpm", "640"),
    ("feed", "120"),
    ("scallop", "0.05"),
    ("stepover", "40"),
    ("stepdown", ""),
];
/// Each quarter's corner by where it is on the part, quarter `q` facing
/// the way 45° + 90°·q does.
pub const WHERE: [&str; 4] = [
    "back right (X+ Y+)",
    "back left (X− Y+)",
    "front left (X− Y−)",
    "front right (X+ Y−)",
];
/// The DRO's resolution: every reading is a hundredth.
const READ: f64 = 0.01;
/// The most steps a walk round one corner takes: one a degree.
const MAX_STEPS: usize = 90;
/// How far clear of the stock the tool goes down beside it, mm.
const CLEAR: f32 = 2.;
/// How far short of the walk's first reading on the axis the walk moves
/// along first the tool stops, to come to it the way the walk goes and so
/// take up the backlash (mm).
pub const TAKE_UP: f32 = 1.;
/// How much a Corner's round may differ from a quarter, and its start from
/// a wall along X or Y, degrees.
const SQUARE: f32 = 1.;

/// What is wrong with a Round corners' settings.
pub fn setting_issues(p: &Fields) -> Vec<String> {
    let mut e = vec![];
    if !num(p, "scallop").is_some_and(|v| (0.01..=1.).contains(&v)) {
        e.push("Scallop must be 0.01–1 mm: the DRO reads a hundredth.".into());
    }
    if !num(p, "stepover").is_some_and(|v| (10. ..=100.).contains(&v)) {
        e.push("Stepover must be 10–100 % of the tool's diameter.".into());
    }
    if !get(p, "stepdown").trim().is_empty() && !num(p, "stepdown").is_some_and(|v| v > 0.) {
        e.push("Stepdown must be positive, or empty for Auto.".into());
    }
    e
}

/// A rounded outside vertical corner, as `setup` places the part (mm).
#[derive(Debug, Clone, PartialEq)]
pub struct Corner {
    pub centre: [f32; 2],
    pub r: f32,
    /// Which quarter round its centre it is: from 90°·q to 90°·(q + 1),
    /// the way the walk goes.
    pub quarter: usize,
    pub top: f32,
    pub bottom: f32,
    /// The surfaces of its facets.
    pub walls: Vec<usize>,
}
impl Corner {
    /// The corner `a` is, when it is a quarter round from a wall along X or
    /// Y to one along the other.
    pub(crate) fn from_arc(a: &arcs::Arc) -> Result<Corner, String> {
        let span = a.span;
        let start = a.middle.to_degrees() - span / 2.;
        let q = (start / 90.).round();
        if (span - 90.).abs() > SQUARE || (start - 90. * q).abs() > SQUARE {
            return Err(format!(
                "The round at X{:.2} Y{:.2} goes {span:.0}° round from {start:.0}°: Round corners rounds a square corner, a quarter round between a wall along X and one along Y.",
                a.centre[0], a.centre[1]
            ));
        }
        Ok(Corner {
            centre: a.centre,
            r: a.r,
            quarter: (q as i32).rem_euclid(4) as usize,
            top: a.top,
            bottom: a.bottom,
            walls: a.walls.clone(),
        })
    }
    /// Where on the part it is, in words.
    pub fn place(&self) -> &'static str {
        WHERE[self.quarter]
    }
}
/// The unit vector `t` radians on from the start of quarter `q`.
fn unit(q: usize, t: f64) -> [f64; 2] {
    let a = std::f64::consts::FRAC_PI_2 * q as f64 + t;
    [a.cos(), a.sin()]
}
/// Unit vectors to the start and the end of quarter `q`, exact.
fn ends(q: usize) -> ([f32; 2], [f32; 2]) {
    const AXES: [[f32; 2]; 4] = [[1., 0.], [0., 1.], [-1., 0.], [0., -1.]];
    (AXES[q % 4], AXES[(q + 1) % 4])
}
/// The corners among the surfaces `ids`, in the order cut: round the part
/// counter-clockwise from its back right one. A round that is no square
/// corner's says why.
pub fn among(setup: &Setup, mesh: &Mesh, ids: &HashSet<usize>) -> Result<Vec<Corner>, String> {
    let mut found = arcs::outside_arcs_among(setup, mesh, ids)
        .iter()
        .map(Corner::from_arc)
        .collect::<Result<Vec<_>, _>>()?;
    found.sort_by(|a, b| {
        (a.quarter.cmp(&b.quarter))
            .then(a.centre[0].total_cmp(&b.centre[0]))
            .then(a.centre[1].total_cmp(&b.centre[1]))
    });
    Ok(found)
}
/// Every square rounded outside corner of the part (`among` over all its
/// upright faces); rounds that are no square corner's are left out.
pub fn all(setup: &Setup, mesh: &Mesh) -> Vec<Corner> {
    let mut found: Vec<Corner> = arcs::outside_arcs_among(setup, mesh, &arcs::upright(setup, mesh))
        .iter()
        .filter_map(|a| Corner::from_arc(a).ok())
        .collect();
    found.sort_by(|a, b| {
        (a.quarter.cmp(&b.quarter))
            .then(a.centre[0].total_cmp(&b.centre[0]))
            .then(a.centre[1].total_cmp(&b.centre[1]))
    });
    found
}
/// The corners `op` rounds: those its picked surfaces make.
pub fn picked(setup: &Setup, mesh: &Mesh, op: &Operation) -> Result<Vec<Corner>, String> {
    let Some(selection) = op.surfaces.as_ref().filter(|s| !s.ids.is_empty()) else {
        return Err(NONE_PICKED.into());
    };
    if !selection.matches(mesh) {
        return Err(
            "The model geometry changed. Pick the corners again on the current part.".into(),
        );
    }
    let ids: HashSet<usize> = selection.ids.iter().copied().collect();
    let found = among(setup, mesh, &ids)?;
    if found.is_empty() {
        return Err(NONE_PICKED.into());
    }
    Ok(found)
}

/// `v` on the DRO's hundredths, rounded away from `c` (either way when on
/// it): a point so rounded off a round about `c` is no nearer to it.
fn outward(v: f64, c: f64) -> f64 {
    let d = v - c;
    let at = v / READ;
    let n = if d > 1e-9 {
        (at - 1e-6).ceil()
    } else if d < -1e-9 {
        (at + 1e-6).floor()
    } else {
        at.round()
    };
    n * READ
}
/// The corner between `a` and `b` that a one-axis move then another one
/// make, the one farther from `c`: outside the round both are on.
fn outer(c: [f64; 2], a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    let (p, q) = ([b[0], a[1]], [a[0], b[1]]);
    let d = |p: [f64; 2]| (p[0] - c[0]).hypot(p[1] - c[1]);
    if d(p) >= d(q) { p } else { q }
}
fn segment(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let l = dx * dx + dy * dy;
    let t = if l == 0. {
        0.
    } else {
        (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / l).clamp(0., 1.)
    };
    (p[0] - a[0] - t * dx).hypot(p[1] - a[1] - t * dy)
}
/// How far along the ray from `a` in the unit direction `n` the first point
/// within `rc` of the segment `p`–`q` is (the capsule a tool of radius `rc`
/// sweeps along it); 0 when `a` is in it, None when the ray misses it.
fn ray_capsule(a: [f64; 2], n: [f64; 2], p: [f64; 2], q: [f64; 2], rc: f64) -> Option<f64> {
    let disc = |c: [f64; 2]| {
        let v = [a[0] - c[0], a[1] - c[1]];
        let b = v[0] * n[0] + v[1] * n[1];
        let k = v[0] * v[0] + v[1] * v[1] - rc * rc;
        if k <= 0. {
            return Some(0.);
        }
        let d = b * b - k;
        let t = -b - d.max(0.).sqrt();
        (d >= 0. && t >= 0.).then_some(t)
    };
    // The band along the segment, in its own frame: along it 0..l, across
    // it −rc..rc.
    let l = (q[0] - p[0]).hypot(q[1] - p[1]);
    let band = (l > 1e-12).then(|| {
        let d = [(q[0] - p[0]) / l, (q[1] - p[1]) / l];
        let m = [-d[1], d[0]];
        let v = [a[0] - p[0], a[1] - p[1]];
        let o = [v[0] * d[0] + v[1] * d[1], v[0] * m[0] + v[1] * m[1]];
        let w = [n[0] * d[0] + n[1] * d[1], n[0] * m[0] + n[1] * m[1]];
        let (mut lo, mut hi) = (0f64, f64::INFINITY);
        for (k, (a, b)) in [(0., l), (-rc, rc)].into_iter().enumerate() {
            if w[k].abs() < 1e-15 {
                if o[k] < a || o[k] > b {
                    return None;
                }
            } else {
                let (t1, t2) = ((a - o[k]) / w[k], (b - o[k]) / w[k]);
                lo = lo.max(t1.min(t2));
                hi = hi.min(t1.max(t2));
            }
        }
        (lo <= hi).then_some(lo)
    });
    [disc(p), disc(q), band.flatten()]
        .into_iter()
        .flatten()
        .reduce(f64::min)
}
/// At the point `t` radians round quarter `q` of the part's round of
/// radius `r` about `c`: how much material a tool of radius `rc` whose
/// centre walks `path` leaves standing proud of the round there, measured
/// out along the round's normal to where the tool's sweep starts (the
/// scallop's height there); and how far the path is from that point less
/// `rc`, negative where the tool would cut into the round.
fn left_at(c: [f64; 2], r: f64, rc: f64, q: usize, path: &[[f64; 2]], t: f64) -> (f64, f64) {
    let n = unit(q, t);
    let a = [c[0] + r * n[0], c[1] + r * n[1]];
    let height = (path.windows(2))
        .filter_map(|w| ray_capsule(a, n, w[0], w[1], rc))
        .fold(f64::INFINITY, f64::min);
    let gap = (path.windows(2))
        .map(|w| segment(a, w[0], w[1]))
        .fold(f64::INFINITY, f64::min)
        - rc;
    (height, gap)
}
/// The most material `left_at` finds proud of the round from `from` to `to`
/// radians round quarter `q`, at `samples` points over it, and the least
/// gap.
fn left(
    c: [f64; 2],
    r: f64,
    rc: f64,
    q: usize,
    path: &[[f64; 2]],
    (from, to): (f64, f64),
    samples: usize,
) -> (f64, f64) {
    let (mut most, mut least) = (f64::NEG_INFINITY, f64::INFINITY);
    for k in 0..=samples {
        let (h, g) = left_at(
            c,
            r,
            rc,
            q,
            path,
            from + (to - from) * k as f64 / samples as f64,
        );
        most = most.max(h);
        least = least.min(g);
    }
    (most, least)
}
/// The most material `left_at` leaves proud of the round over the whole
/// quarter, and the least gap. The scallops peak where two cuts' edges
/// meet, between two samples: each sample higher than both its neighbours
/// is followed to its peak between them (the height rises to a peak and
/// falls past it), to a ten-millionth of a radian.
fn bound(c: [f64; 2], r: f64, rc: f64, q: usize, path: &[[f64; 2]]) -> (f64, f64) {
    let quarter = std::f64::consts::FRAC_PI_2;
    let samples = (r * quarter / 0.002).ceil().clamp(2000., 50_000.) as usize;
    let at = |k: usize| quarter * k as f64 / samples as f64;
    let heights: Vec<(f64, f64)> = (0..=samples)
        .map(|k| left_at(c, r, rc, q, path, at(k)))
        .collect();
    let least = heights.iter().map(|h| h.1).fold(f64::INFINITY, f64::min);
    let mut most = heights
        .iter()
        .map(|h| h.0)
        .fold(f64::NEG_INFINITY, f64::max);
    let height = |t: f64| left_at(c, r, rc, q, path, t).0;
    for k in 1..samples {
        if heights[k].0 < heights[k - 1].0 || heights[k].0 < heights[k + 1].0 {
            continue;
        }
        let (mut lo, mut hi) = (at(k - 1), at(k + 1));
        while hi - lo > 1e-7 {
            let (m1, m2) = (lo + (hi - lo) / 3., hi - (hi - lo) / 3.);
            if height(m1) < height(m2) {
                lo = m1;
            } else {
                hi = m2;
            }
        }
        most = most.max(height((lo + hi) / 2.));
    }
    (most, least)
}
/// One walk round a corner: the tool centre's readings in order (work X,
/// Y), from the first on the start wall's line to the last on the end
/// wall's, each next one a move along one axis.
#[derive(Debug, Clone, PartialEq)]
pub struct Stair {
    pub points: Vec<[f64; 2]>,
    /// The most it leaves standing proud of the round (`left`).
    pub scallop: f64,
}
/// The point `t` radians round quarter `q` of the round of radius `rho`
/// about `c`, on the DRO's hundredths, outward.
fn on(c: [f64; 2], rho: f64, q: usize, t: f64) -> [f64; 2] {
    let u = unit(q, t);
    [
        outward(c[0] + rho * u[0], c[0]),
        outward(c[1] + rho * u[1], c[1]),
    ]
}
/// The angles of the steps round quarter `q` (0 to π/2) that each leave
/// at most `h` proud of the round of radius `r` (`left` over that step
/// alone, which is never less than over the whole walk), each as long as
/// it may be; None when that takes more than `MAX_STEPS`.
fn angles(c: [f64; 2], r: f64, rc: f64, q: usize, h: f64) -> Option<Vec<f64>> {
    let rho = r + rc;
    let quarter = std::f64::consts::FRAC_PI_2;
    let step = |a: f64, b: f64| {
        let (p, n) = (on(c, rho, q, a), on(c, rho, q, b));
        left(c, r, rc, q, &[p, outer(c, p, n), n], (a, b), 24).0
    };
    let mut at = vec![0.];
    let mut t = 0.;
    while t < quarter - 1e-9 {
        if at.len() > MAX_STEPS {
            return None;
        }
        let next = if step(t, quarter) <= h {
            quarter
        } else {
            let (mut lo, mut hi) = (t, quarter);
            for _ in 0..40 {
                let mid = (lo + hi) / 2.;
                if step(t, mid) <= h {
                    lo = mid
                } else {
                    hi = mid
                }
            }
            lo
        };
        if next - t < 1e-4 {
            return None;
        }
        t = next;
        at.push(t);
    }
    Some(at)
}
/// The walk round quarter `q` of a round of radius `r` about `c` (work
/// coordinates, mm) by a tool of radius `rc` whose centre keeps `offset`
/// outside the round offset by `rc`, in as few steps as leave at most
/// `scallop` proud of the round `offset` outside the part's; the steps
/// then made as even as that number allows. Every reading on the DRO's
/// hundredths, rounded outward. Refused when the scallop takes more than
/// `MAX_STEPS` steps.
pub fn stair(
    c: [f64; 2],
    r: f64,
    rc: f64,
    q: usize,
    offset: f64,
    scallop: f64,
) -> Result<Stair, String> {
    let r = r + offset;
    let rho = r + rc;
    let walk = |angles: &[f64]| {
        let mut points = vec![on(c, rho, q, angles[0])];
        for w in angles.windows(2) {
            let (p, n) = (*points.last().unwrap(), on(c, rho, q, w[1]));
            for v in [outer(c, p, n), n] {
                if *points.last().unwrap() != v {
                    points.push(v);
                }
            }
        }
        points
    };
    let too_many = || {
        format!(
            "A scallop of {scallop} mm takes more than {MAX_STEPS} steps round an R{} corner: allow a larger scallop.",
            r - offset
        )
    };
    // Rounding to the DRO's hundredths may leave a little more than each
    // step was worked out to: then the steps are worked out again to less.
    let mut h = scallop;
    for _ in 0..8 {
        let first = angles(c, r, rc, q, h).ok_or_else(too_many)?;
        let n = first.len();
        // The least that still takes that many steps: the steps even.
        let (mut lo, mut hi, mut best) = (0.25 * h, h, first);
        for _ in 0..20 {
            let mid = (lo + hi) / 2.;
            match angles(c, r, rc, q, mid) {
                Some(a) if a.len() <= n => {
                    hi = mid;
                    best = a;
                }
                _ => lo = mid,
            }
        }
        let points = walk(&best);
        let (most, _) = bound(c, r, rc, q, &points);
        if most <= scallop + 1e-9 {
            return Ok(Stair {
                points,
                scallop: most.max(0.),
            });
        }
        h *= 0.8;
    }
    Err(too_many())
}

/// One walk of a Round corners' plan: round corner `corner` (of the plan's
/// corners, in order), `offset` outside the round, at work Z `z`.
#[derive(Debug, Clone, PartialEq)]
pub struct Walk {
    pub corner: usize,
    pub offset: f32,
    pub z: f32,
    pub level: usize,
    pub levels: usize,
    /// The walk's readings (work X, Y) and what it leaves.
    pub stair: Stair,
    /// Its moves in the plan: from the first after the walk before up to
    /// its last, the tool up at clearance.
    pub moves: std::ops::Range<usize>,
}
/// The round-the-part fields `generate` needs of `op` and the material.
struct Settings {
    scallop: f32,
    band: f32,
    stepdown: Option<f32>,
}
fn settings(op: &Operation, r: f32) -> Result<Settings, String> {
    if let Some(e) = setting_issues(&op.p).into_iter().next() {
        return Err(e);
    }
    Ok(Settings {
        scallop: number(op, "scallop")?,
        band: number(op, "stepover")? / 100. * 2. * r,
        stepdown: if get(&op.p, "stepdown").trim().is_empty() {
            None
        } else {
            Some(number(op, "stepdown")?)
        },
    })
}
/// A closed outline of the ring between radii `a` and `b` about `c` over
/// quarter `q` (setup frame), for `rest::Reach`.
fn ring(c: [f32; 2], a: f32, b: f32, q: usize) -> Vec<(f64, f64)> {
    const SIDES: usize = 24;
    let point = |r: f32, t: f64| {
        let u = unit(q, t);
        (c[0] as f64 + r as f64 * u[0], c[1] as f64 + r as f64 * u[1])
    };
    let quarter = std::f64::consts::FRAC_PI_2;
    // Out past the arc's chords on the outside.
    let b = b / (quarter / 2. / SIDES as f64).cos() as f32;
    let mut out: Vec<(f64, f64)> = (0..=SIDES)
        .map(|i| point(b, quarter * i as f64 / SIDES as f64))
        .collect();
    out.extend(
        (0..=SIDES)
            .rev()
            .map(|i| point(a, quarter * i as f64 / SIDES as f64)),
    );
    out
}
/// Whether the stock as the setup's earlier operations left it has
/// material between `bottom` and `top` in the outline `area` (setup frame);
/// with none of them, whether `area` meets the stock box `s`.
fn material(setup: &Setup, area: Vec<(f64, f64)>, s: &Bounds, bottom: f32, top: f32) -> bool {
    let (mut lo, mut hi) = ([f32::INFINITY; 2], [f32::NEG_INFINITY; 2]);
    for p in &area {
        for (k, v) in [p.0 as f32, p.1 as f32].into_iter().enumerate() {
            (lo[k], hi[k]) = (lo[k].min(v), hi[k].max(v));
        }
    }
    if (0..2).any(|k| hi[k] <= s.min[k] + ON || lo[k] >= s.max[k] - ON) {
        return false;
    }
    setup.preview_remaining.is_none()
        || crate::rest::start_over(setup, &crate::rest::Reach::new(&[area]), bottom, top, 0.)
            > bottom + ON
}
/// The capsule a tool of radius `r` sweeps from `a` to `b` (setup frame).
fn capsule(a: [f32; 2], b: [f32; 2], r: f32) -> Vec<(f64, f64)> {
    let heading = (b[1] - a[1]).atan2(b[0] - a[0]) as f64;
    let mut out = vec![];
    for (p, from) in [
        (b, heading - std::f64::consts::FRAC_PI_2),
        (a, heading + std::f64::consts::FRAC_PI_2),
    ] {
        for i in 0..=12 {
            let t = from + std::f64::consts::PI * i as f64 / 12.;
            out.push((
                p[0] as f64 + r as f64 * t.cos(),
                p[1] as f64 + r as f64 * t.sin(),
            ));
        }
    }
    out
}
/// Equal steps of at most `by` from `from` (left out) down to exactly `to`.
fn down(from: f32, to: f32, by: f32) -> Vec<f32> {
    let n = ((from - to) / by - 0.0001).ceil().clamp(1., 10_000.) as usize;
    (1..=n)
        .map(|i| {
            if i == n {
                to
            } else {
                from - (from - to) * i as f32 / n as f32
            }
        })
        .collect()
}
/// A Round corners on a manual mill: for each picked corner, from the
/// top of the material down to the corner's foot in levels no deeper than
/// the stepdown, at each level walks round it from outside in, a band no
/// wider than the stepover each, the last on the round itself (`stair`).
/// A walk with no material left in its band is left out, but for the last.
/// Each walk, one axis at a time: up at clearance; there out along the start
/// wall's normal, clear of the stock; along the walk's other axis to a
/// millimetre short of its first reading, then on to it, so the backlash is
/// taken up the way the walk goes; straight down beside the stock; in along
/// the wall's normal to the first reading, the way the walk goes too; the
/// steps; out along the end wall's normal clear of the stock; up. Every
/// move is checked against the part, and the Steps check each walk's
/// numbers (`walk_problem`).
pub fn generate(setup: &Setup, op: &Operation, tool: &Tool, mesh: &Mesh) -> Result<Plan, String> {
    Ok(walks(setup, op, tool, mesh)?.0)
}
/// `generate`'s plan and its walks, in order.
pub fn walks(
    setup: &Setup,
    op: &Operation,
    tool: &Tool,
    mesh: &Mesh,
) -> Result<(Plan, Vec<Walk>), String> {
    if !crate::manual::is_manual(&setup.machine) {
        return Err(ON_CNC.into());
    }
    let mut b = Builder::new(setup, op, tool, mesh)?;
    if get(&tool.p, "type") != "Flat end mill" {
        return Err("Round corners needs a flat end mill.".into());
    }
    let (s, rc) = (b.plan.stock, b.plan.radius);
    let set = settings(op, rc)?;
    let zero = crate::post::datum(setup, s, mesh)?;
    let corners = picked(setup, mesh, op)?;
    let geometry = Geometry::new(setup, mesh);
    let pause = num(&setup.machine.p, "pass_time")
        .filter(|t| *t >= 0.)
        .unwrap_or(0.);
    let (rapid, feed) = (b.rapid, b.feed);
    let mut walks = vec![];
    let mut levels_total = 0;
    let mut deepest = f32::INFINITY;
    let mut rest_top: Option<f32> = None;
    for (ci, corner) in corners.iter().enumerate() {
        let (c, r, q) = (corner.centre, corner.r, corner.quarter);
        let (u0, u1) = ends(q);
        let bottom = corner.bottom.max(s.min[2]);
        if bottom >= s.max[2] - ON {
            return Err(format!(
                "The {} corner is at the stock's top: there is nothing to round.",
                corner.place()
            ));
        }
        // How far the stock reaches out past the round: to the stock box's
        // corner off it.
        let far = [
            if u0[0] + u1[0] > 0. {
                s.max[0]
            } else {
                s.min[0]
            },
            if u0[1] + u1[1] > 0. {
                s.max[1]
            } else {
                s.min[1]
            },
        ];
        let thick = ((far[0] - c[0]).hypot(far[1] - c[1]) - r).max(0.);
        // Outside in: bands no wider than the stepover, the last on the
        // round; one that finds no material left is not walked.
        let n = ((thick - 0.001) / set.band).ceil().max(1.) as usize;
        let top_of = |a: f32, w: f32| {
            crate::rest::start_over(
                setup,
                &crate::rest::Reach::new(&[ring(c, r + a, r + a + w, q)]),
                bottom,
                s.max[2],
                0.,
            )
        };
        let mut offsets: Vec<f32> = (1..n)
            .rev()
            .map(|k| thick * k as f32 / n as f32)
            .filter(|&a| {
                material(
                    setup,
                    ring(c, r + a, r + a + 2. * rc, q),
                    &s,
                    bottom,
                    s.max[2],
                )
            })
            .collect();
        offsets.push(0.);
        // The levels: from the highest material outside the round.
        let top = top_of(0.05, thick + 2. * rc).max(bottom + ON);
        if top < s.max[2] - 0.0001 {
            rest_top = Some(rest_top.map_or(top, |t: f32| t.max(top)));
        }
        // The stairs, in work coordinates.
        let cw = [(c[0] - zero[0]) as f64, (c[1] - zero[1]) as f64];
        let stairs = offsets
            .iter()
            .map(|&a| {
                let h = if a > 0. {
                    // A band's steps are taken by the next walk: coarser.
                    // A fifth of the band, up to 0.5 mm, never finer than
                    // the scallop asked for.
                    (0.2 * set.band).min(0.5).max(set.scallop)
                } else {
                    set.scallop
                };
                stair(cw, r as f64, rc as f64, q, a as f64, h as f64)
            })
            .collect::<Result<Vec<_>, _>>()?;
        // Conventional: counter-clockwise round the part with the spindle
        // clockwise (M3), the other way with it counter-clockwise (M4).
        let stairs: Vec<Stair> = if b.plan.spindle_clockwise {
            stairs
        } else {
            (stairs.into_iter())
                .map(|mut s| {
                    s.points.reverse();
                    s
                })
                .collect()
        };
        let stock_frame = |p: [f64; 2]| [p[0] as f32 + zero[0], p[1] as f32 + zero[1]];
        // Into the first reading along its wall's normal, out of the last
        // along the other wall's, each from or to clear of the stock: `e`
        // the axis in along, toward `-sign_in`, `o` the axis out along.
        let start_axis = if u0[0] != 0. { 0 } else { 1 };
        let (e, sign_in, o, sign_out) = if b.plan.spindle_clockwise {
            (
                start_axis,
                u0[start_axis],
                1 - start_axis,
                u1[1 - start_axis],
            )
        } else {
            (
                1 - start_axis,
                u1[1 - start_axis],
                start_axis,
                u0[start_axis],
            )
        };
        // Always some way: a walk's first and last moves are its way in and
        // out (`read`).
        let beyond = |k: usize, sign: f32, at: f32| {
            if sign > 0. {
                (s.max[k] + rc + CLEAR).max(at + CLEAR)
            } else {
                (s.min[k] - rc - CLEAR).min(at - CLEAR)
            }
        };
        // How wide the widest cut is: the whole diameter where the way in or
        // out goes through stock, else a band.
        let mut width: f32 = 0.;
        for (stair, &a) in stairs.iter().zip(&offsets) {
            let (first, last) = (
                stock_frame(stair.points[0]),
                stock_frame(*stair.points.last().unwrap()),
            );
            let mut start = first;
            start[e] = beyond(e, sign_in, first[e]);
            let mut end = last;
            end[o] = beyond(o, sign_out, last[o]);
            let through = material(setup, capsule(start, first, rc), &s, bottom, s.max[2])
                || material(setup, capsule(last, end, rc), &s, bottom, s.max[2]);
            let band = if a > 0. {
                thick / n as f32
            } else {
                thick.min(set.band)
            };
            width = width.max(if through {
                2. * rc
            } else {
                (band + 0.5).min(2. * rc)
            });
        }
        let stepdown = match set.stepdown {
            Some(d) => d,
            None => {
                let rpm = number(op, "rpm").ok().filter(|n| *n > 0.);
                let flutes = num(&tool.p, "flutes").filter(|z| *z >= 1.);
                let (Some(rpm), Some(flutes)) = (rpm, flutes) else {
                    return Err(
                        "An automatic stepdown needs a positive rpm and the tool's flutes.".into(),
                    );
                };
                crate::forces::power_stepdown(
                    &setup.machine,
                    setup.job_material.as_ref(),
                    2. * rc,
                    width,
                    feed,
                    rpm,
                    flutes,
                )
                .min(b.plan.flute_length)
            }
        };
        if !(stepdown > 0. && stepdown <= b.plan.flute_length) {
            return Err("Use a stepdown within the flute length.".into());
        }
        // On the DRO's hundredths, but the last, the corner's foot.
        let levels: Vec<f32> = down(top, bottom, stepdown)
            .into_iter()
            .map(|z| {
                if z <= bottom + ON {
                    bottom
                } else {
                    ((z - zero[2]) / READ as f32).ceil() * READ as f32 + zero[2]
                }
            })
            .collect();
        levels_total += levels.len();
        deepest = deepest.min(bottom);
        // Every feed checked against the part, from the deepest level up:
        // the tool's side and its end.
        let near = Bounds {
            min: [
                c[0] - r - thick - 4. * rc - CLEAR,
                c[1] - r - thick - 4. * rc - CLEAR,
                bottom,
            ],
            max: [
                c[0] + r + thick + 4. * rc + CLEAR,
                c[1] + r + thick + 4. * rc + CLEAR,
                s.max[2],
            ],
        };
        let part: Vec<&[[f32; 3]; 3]> = (geometry.triangles.iter())
            .filter(|t| meets(&t.p, &near))
            .map(|t| &t.p)
            .collect();
        for (stair, &a) in stairs.iter().zip(&offsets) {
            let mut path: Vec<[f32; 2]> = stair.points.iter().map(|p| stock_frame(*p)).collect();
            let mut start = path[0];
            start[e] = beyond(e, sign_in, path[0][e]);
            let mut end = *path.last().unwrap();
            end[o] = beyond(o, sign_out, end[o]);
            path.insert(0, start);
            path.push(end);
            for w in path.windows(2) {
                let lo = [w[0][0].min(w[1][0]), w[0][1].min(w[1][1])];
                let hi = [w[0][0].max(w[1][0]), w[0][1].max(w[1][1])];
                let cut = Bounds {
                    min: [lo[0] - rc, lo[1] - rc, bottom + ON],
                    max: [hi[0] + rc, hi[1] + rc, s.max[2]],
                };
                if let Some(t) = part
                    .iter()
                    .find(|t| meets(t, &cut) && swept(t, [lo, hi], rc, [cut.min[2], cut.max[2]]))
                {
                    let at: [f32; 3] = std::array::from_fn(|k| {
                        (t.iter().map(|p| p[k]).sum::<f32>() / 3.) - zero[k]
                    });
                    return Err(format!(
                        "Round corners cuts into the part at X{:.2}, Y{:.2}, Z{:.2}, walking the {} corner{}.",
                        at[0],
                        at[1],
                        at[2],
                        corner.place(),
                        if a > 0. {
                            format!(" {} mm off the round", crate::envelope::mm(a))
                        } else {
                            String::new()
                        }
                    ));
                }
            }
        }
        // The walks: each level, outside in.
        for (li, &z) in levels.iter().enumerate() {
            for (stair, &a) in stairs.iter().zip(&offsets) {
                let first_move = b.plan.moves.len();
                let points: Vec<[f32; 2]> = stair.points.iter().map(|p| stock_frame(*p)).collect();
                let (first, last) = (points[0], *points.last().unwrap());
                let walk_o = (last[o] - first[o]).signum();
                // Up clear of the stock; there, one axis at a time: the axis
                // fed in along to beyond the stock, the other a millimetre
                // short of the first reading and then on to it the way the
                // walk goes; then straight down beside the stock. The feed
                // in comes to the first reading the way the walk goes too
                // (LIO-447).
                let mut at = b.pos;
                at[2] = b.clearance;
                b.push(at, Motion::Rapid, false, rapid)?;
                at[e] = beyond(e, sign_in, first[e]);
                b.push(at, Motion::Rapid, false, rapid)?;
                at[o] = first[o] - walk_o * TAKE_UP;
                b.push(at, Motion::Rapid, false, rapid)?;
                at[o] = first[o];
                b.push(at, Motion::Rapid, false, rapid)?;
                at[2] = z;
                b.push(at, Motion::Rapid, false, rapid)?;
                b.dwell(pause);
                for p in &points {
                    at[0] = p[0];
                    at[1] = p[1];
                    b.push(at, Motion::Feed, true, feed)?;
                }
                at[o] = beyond(o, sign_out, last[o]);
                b.push(at, Motion::Feed, true, feed)?;
                at[2] = b.clearance;
                b.push(at, Motion::Rapid, false, rapid)?;
                walks.push(Walk {
                    corner: ci,
                    offset: a,
                    z: z - zero[2],
                    level: li + 1,
                    levels: levels.len(),
                    stair: stair.clone(),
                    moves: first_move..b.plan.moves.len(),
                });
            }
        }
    }
    b.plan.milling_direction = "Conventional";
    b.plan.target = deepest;
    b.plan.levels = levels_total;
    b.plan.rest_top = rest_top;
    let plan = b.finish()?;
    Ok((plan, walks))
}
/// What a Round corners whose Steps cannot be followed says, after the
/// operation's name, the walk and what is wrong with it (LIO-447); the plan
/// checks find it by this.
pub const BROKEN: &str = "These Steps cannot be followed.";
/// What the Steps say of operation `name`'s walk `index` of `of`, round the
/// corner at `place`, that cannot be followed for `why` (`walk_problem`).
pub fn broken(name: &str, (index, of): (usize, usize), place: &str, why: &str) -> String {
    format!("{name}: walk {index} of {of}, {place}: {why}. {BROKEN}")
}
/// What is wrong with one walk's DRO numbers as the Steps give them
/// (`targets`, tool centres, work coordinates), if anything (LIO-447): each
/// number finite; Z above the stock's top (`top`) before X and Y are set,
/// both set before the tool goes down to the walk's level `z`, where its
/// side (radius `rc`) stands clear of the stock's box `lo`–`hi` (X, Y); each
/// axis coming to the first reading the way the walk goes, so its backlash
/// is taken up; then the feeds, each along the way its axis goes round the
/// walk (`points`, its readings) and only that way, onto each reading in
/// turn and out; then up above the stock.
pub fn walk_problem(
    targets: &[crate::guide::Target],
    points: &[[f32; 2]],
    z: f32,
    (lo, hi, top): ([f32; 2], [f32; 2], f32),
    round: &Round,
) -> Option<String> {
    const EPS: f32 = 1e-3;
    const AXES: [&str; 3] = ["X", "Y", "Z"];
    let rc = round.tool;
    if points.len() < 2 || points.iter().flatten().any(|v| !v.is_finite()) {
        return Some("its readings are not all numbers".into());
    }
    if !(round.offset.is_finite() && round.scallop.is_finite() && z.is_finite()) {
        return Some("its offset, scallop or level is not a number".into());
    }
    // Its own corner's: from one wall's line to the other's, on the round
    // offset by the tool and the walk's offset at both ends, never inside
    // the round offset by the tool.
    let c = round.centre;
    let reach = round.radius + rc + round.offset;
    let d = |p: [f32; 2]| (p[0] - c[0]).hypot(p[1] - c[1]);
    let (first, last) = (points[0], points[points.len() - 1]);
    let walls = on_walls(
        [c[0] as f64, c[1] as f64],
        round.quarter,
        [first[0] as f64, first[1] as f64],
        [last[0] as f64, last[1] as f64],
    );
    if !walls.is_some_and(|e| e < 0.05)
        || (d(first) - reach).abs() > 0.02
        || (d(last) - reach).abs() > 0.02
    {
        return Some("its readings are not round its own corner".into());
    }
    if let Some(p) = points.iter().find(|p| d(**p) < round.radius + rc - EPS) {
        return Some(format!(
            "its reading X {:.2} Y {:.2} cuts into the round",
            p[0], p[1]
        ));
    }
    if let Some(t) = targets.iter().find(|t| !t.centre.is_finite()) {
        return Some(format!("its {} number is not a number", AXES[t.axis]));
    }
    let (first, last) = (points[0], points[points.len() - 1]);
    let way: [f32; 2] = std::array::from_fn(|k| {
        let d = last[k] - first[k];
        if d.abs() < EPS { 0. } else { d.signum() }
    });
    let Some(down) = targets.iter().position(|t| t.axis == 2 && t.lock[2]) else {
        return Some("it never goes down to its level".into());
    };
    let missing: Vec<&str> = (0..3)
        .filter(|&k| !targets[..down].iter().any(|t| t.axis == k))
        .map(|k| AXES[k])
        .collect();
    if !missing.is_empty() {
        return Some(format!(
            "it goes down with no {} to go down at",
            missing.join(" or ")
        ));
    }
    let mut pos: [Option<f32>; 3] = [None; 3];
    // The way each of X and Y last moved, to the walk's start.
    let mut came = [0f32; 2];
    for t in &targets[..down] {
        if t.feed {
            return Some(format!("it feeds {} before it goes down", AXES[t.axis]));
        }
        if t.axis < 2 {
            if !pos[2].is_some_and(|z| z >= top - EPS) {
                return Some(format!(
                    "it moves {} before the tool is up above the stock",
                    AXES[t.axis]
                ));
            }
            if let Some(p) = pos[t.axis]
                && (t.centre - p).abs() >= EPS
            {
                came[t.axis] = (t.centre - p).signum();
            }
        }
        pos[t.axis] = Some(t.centre);
    }
    let (Some(x), Some(y), Some(above)) = (pos[0], pos[1], pos[2]) else {
        unreachable!("each axis is set before it goes down")
    };
    let level = targets[down].centre;
    if (level - z).abs() >= EPS || level >= above {
        return Some(format!(
            "it goes down to Z {level:.2}, not its level Z {z:.2}"
        ));
    }
    let off = |v: f32, k: usize| (lo[k] - v).max(v - hi[k]).max(0.);
    if off(x, 0).hypot(off(y, 1)) < rc - EPS {
        return Some(format!(
            "it goes down at X {x:.2} Y {y:.2}, onto the stock, not beside it"
        ));
    }
    // The feeds: in to the first reading, each reading in turn, then out.
    let feeds = &targets[down + 1..];
    let n = feeds.iter().take_while(|t| t.feed).count();
    if n != points.len() + 1 {
        return Some(format!(
            "it has {n} feeds for {} readings and the way out",
            points.len()
        ));
    }
    let mut at = [x, y];
    for (i, t) in feeds[..n].iter().enumerate() {
        if t.axis == 2 {
            return Some("it moves Z during the walk".into());
        }
        let k = t.axis;
        let d = t.centre - at[k];
        if d.abs() < EPS || way[k] == 0. || d.signum() != way[k] {
            return Some(format!(
                "its feed to {} {:.2} goes against the walk's one way on {}",
                AXES[k], t.centre, AXES[k]
            ));
        }
        at[k] = t.centre;
        if i == 0 {
            // At the start: the other axis came to it the walk's way too.
            let other = 1 - k;
            if way[other] != 0. && came[other] != way[other] {
                return Some(format!(
                    "it comes to {} {:.2} against the walk's way: the backlash is not taken up",
                    AXES[other], at[other]
                ));
            }
        }
        if let Some(p) = points.get(i)
            && ((at[0] - p[0]).abs() >= EPS || (at[1] - p[1]).abs() >= EPS)
        {
            return Some(format!(
                "its feed {} ends at X {:.2} Y {:.2}, not on reading X {:.2} Y {:.2}",
                i + 1,
                at[0],
                at[1],
                p[0],
                p[1]
            ));
        }
    }
    match &feeds[n..] {
        [up] if up.axis == 2 && up.centre >= top - EPS => None,
        _ => Some("it does not end up above the stock".into()),
    }
}
/// How far a walk from `first` to `last` (work X, Y) is from starting on one
/// wall's line of the corner about `c` in quarter `q` and ending on the
/// other's, either way round, as far from `c` at both: None when it starts
/// or ends on the wrong side of `c` (LIO-447).
fn on_walls(c: [f64; 2], q: usize, first: [f64; 2], last: [f64; 2]) -> Option<f64> {
    let (u0, u1) = ends(q);
    // How far `p` is off the line from `c` along `u`, when on its side.
    let off = |p: [f64; 2], u: [f32; 2]| {
        let along = (p[0] - c[0]) * u[0] as f64 + (p[1] - c[1]) * u[1] as f64;
        let across = (p[0] - c[0]) * -u[1] as f64 + (p[1] - c[1]) * u[0] as f64;
        (along > 0.).then_some(across.abs())
    };
    let d = |p: [f64; 2]| (p[0] - c[0]).hypot(p[1] - c[1]);
    [(u0, u1), (u1, u0)]
        .into_iter()
        .filter_map(|(a, b)| Some(off(first, a)? + off(last, b)? + (d(first) - d(last)).abs()))
        .min_by(f64::total_cmp)
}
/// The corner a walk goes round, as the Steps say it (work X, Y, mm): its
/// round's centre and radius, quarter, the tool's radius, the walk's offset
/// out from the round and the scallop it leaves.
#[derive(Debug, Clone, Copy)]
pub struct Round {
    pub centre: [f32; 2],
    pub radius: f32,
    pub quarter: usize,
    pub tool: f32,
    pub offset: f32,
    pub scallop: f32,
}
/// The walks of a Round corners' `plan` (`walks`), read back from its
/// moves: each from the first move after the walk before to its climb to
/// clearance; its corner the picked one whose walls' lines it starts and
/// ends on (`on_walls`); its offset that reading's distance from the round
/// offset by the tool, and
/// its readings and scallop worked out again from its feeds.
pub fn read(plan: &Plan, corners: &[Corner], zero: [f32; 3]) -> Vec<Walk> {
    let mut out = vec![];
    let mut start = 0;
    let rc = plan.radius;
    let moves = &plan.moves;
    let mut i = 0;
    while i < moves.len() {
        let m = &moves[i];
        // A walk ends where the tool climbs from a cut.
        let climbs = m.motion == Motion::Rapid
            && m.to[2] > m.from[2]
            && i > 0
            && moves[..i]
                .iter()
                .rev()
                .find(|m| m.motion != Motion::Dwell)
                .is_some_and(|m| m.cutting_pass);
        if climbs {
            let walk: &[Move] = &moves[start..=i];
            let feeds: Vec<&Move> = walk.iter().filter(|m| m.cutting_pass).collect();
            if let Some(first) = feeds.first() {
                let z = first.to[2];
                // The feeds' ends but the way in's start and the way out's end.
                // On the DRO's hundredths, as they were worked out.
                let hundredth = |v: f32| ((v as f64) / READ).round() * READ;
                let mut points: Vec<[f64; 2]> = feeds
                    .iter()
                    .map(|m| [hundredth(m.to[0] - zero[0]), hundredth(m.to[1] - zero[1])])
                    .collect();
                points.pop();
                let p0 = points.first().copied().unwrap_or_default();
                let pn = points.last().copied().unwrap_or_default();
                // The corner whose walls' lines the walk starts and ends on,
                // as far from its centre at both (LIO-447: the circle
                // nearest the first reading was a neighbour's on the same
                // side, and drew the wrong corner).
                let near = (0..corners.len())
                    .filter_map(|k| {
                        let c = corners[k].centre;
                        let c = [(c[0] - zero[0]) as f64, (c[1] - zero[1]) as f64];
                        on_walls(c, corners[k].quarter, p0, pn).map(|e| (k, e))
                    })
                    .min_by(|a, b| a.1.total_cmp(&b.1))
                    .map(|(k, _)| k);
                if let Some(ci) = near {
                    let k = &corners[ci];
                    let c = [
                        (k.centre[0] - zero[0]) as f64,
                        (k.centre[1] - zero[1]) as f64,
                    ];
                    let d = (p0[0] - c[0]).hypot(p0[1] - c[1]) - (k.r + rc) as f64;
                    let offset = ((d / READ).round() * READ).max(0.) as f32;
                    let offset = if offset < 0.02 { 0. } else { offset };
                    let r = k.r as f64 + offset as f64;
                    let (scallop, _) = bound(c, r, rc as f64, k.quarter, &points);
                    out.push(Walk {
                        corner: ci,
                        offset,
                        z: z - zero[2],
                        level: 0,
                        levels: 0,
                        stair: Stair {
                            points,
                            scallop: scallop.max(0.),
                        },
                        moves: start..i + 1,
                    });
                }
            }
            start = i + 1;
        }
        i += 1;
    }
    // The levels of each corner's walks, by their Z.
    for ci in 0..corners.len() {
        let mut zs: Vec<f32> = out.iter().filter(|w| w.corner == ci).map(|w| w.z).collect();
        zs.sort_by(|a, b| b.total_cmp(a));
        zs.dedup_by(|a, b| (*a - *b).abs() < 0.001);
        for w in out.iter_mut().filter(|w| w.corner == ci) {
            w.level = zs.iter().position(|z| (z - w.z).abs() < 0.001).unwrap_or(0) + 1;
            w.levels = zs.len();
        }
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::model::{Strategy, Tool};

    /// A `w` × `h` × `t` plate, its four vertical corners rounded `r` in
    /// `n` facets each, its bottom at Z 0 and its middle on X 0 Y 0; with
    /// `square` its +X +Y corner left square instead.
    pub(crate) fn plate(w: f32, h: f32, t: f32, r: f32, n: usize, square: bool) -> Mesh {
        let (x, y) = (w / 2. - r, h / 2. - r);
        let mut outline: Vec<[f32; 2]> = vec![];
        for (q, c) in [[x, y], [-x, y], [-x, -y], [x, -y]].into_iter().enumerate() {
            if square && q == 0 {
                outline.push([w / 2., h / 2.]);
                continue;
            }
            for i in 0..=n {
                let a = std::f32::consts::FRAC_PI_2 * (q as f32 + i as f32 / n as f32);
                outline.push([c[0] + r * a.cos(), c[1] + r * a.sin()]);
            }
        }
        let mut tris = vec![];
        let m = outline.len();
        for i in 0..m {
            let (a, b) = (outline[i], outline[(i + 1) % m]);
            // Top and bottom, a fan from the middle (the outline is convex).
            tris.push([[0., 0., t], [a[0], a[1], t], [b[0], b[1], t]]);
            tris.push([[0., 0., 0.], [b[0], b[1], 0.], [a[0], a[1], 0.]]);
            // The wall, facing out.
            tris.push([[a[0], a[1], 0.], [b[0], b[1], 0.], [b[0], b[1], t]]);
            tris.push([[a[0], a[1], 0.], [b[0], b[1], t], [a[0], a[1], t]]);
        }
        crate::mesh::normalize(tris).unwrap()
    }
    /// The adapter plate's outline: 134 × 88 × 20, R5 corners.
    pub(crate) fn adapter() -> Mesh {
        plate(134., 88., 20., 5., 12, false)
    }
    pub(crate) fn end_mill(d: &str) -> Tool {
        let mut tool = Tool::new(&format!("End mill {d}"), "Flat end mill", d);
        for (k, v) in [("length", "25"), ("stickout", "40"), ("flutes", "3")] {
            tool.p.insert(k.into(), v.into());
        }
        tool
    }
    /// The plate face up on the manual mill, `margin` mm of stock round its
    /// sides, the work zero on the stock top's centre.
    pub(crate) fn setup(margin: &str) -> Setup {
        let mut setup = Setup::new(crate::manual::defaults(), "G54");
        for side in ["left", "right", "front", "back"] {
            setup.p.insert(format!("stock_{side}"), margin.into());
        }
        for side in ["top", "bottom"] {
            setup.p.insert(format!("stock_{side}"), "0".into());
        }
        setup.p.insert("origin".into(), "Stock top center".into());
        setup
    }
    pub(crate) fn corner_op(mesh: &Mesh, tool: &Tool, ids: Vec<usize>) -> Operation {
        let mut op = Operation::new(Strategy::Corner, tool.id.clone());
        op.surfaces = Some(crate::surface::Selection {
            mesh_fingerprint: mesh.fingerprint.clone(),
            ids,
        });
        op
    }
    fn all_ids(setup: &Setup, mesh: &Mesh) -> Vec<usize> {
        all(setup, mesh).into_iter().flat_map(|c| c.walls).collect()
    }

    #[test]
    fn readings_round_away_from_the_centre_to_hundredths() {
        assert_eq!(outward(10.001, 0.), 10.01);
        assert_eq!(outward(-10.001, 0.), -10.01);
        assert_eq!(outward(10.0, 0.), 10.0);
        assert!((outward(3.4567, 5.) - 3.45).abs() < 1e-9);
        assert!((outward(62.0, 62.0) - 62.0).abs() < 1e-9);
    }
    #[test]
    fn the_outer_l_corner_is_outside_the_round() {
        let c = [0., 0.];
        let (a, b) = ([10., 0.], [0., 10.]);
        assert_eq!(outer(c, a, b), [10., 10.]);
        let (a, b) = ([-10., 0.], [0., -10.]);
        assert_eq!(outer(c, a, b), [-10., -10.]);
    }
    #[test]
    fn a_stair_leaves_at_most_the_scallop_and_never_gouges() {
        // R5 with a Ø10 cutter, 0.05 mm, in every quarter, about a centre
        // off the hundredths.
        for q in 0..4 {
            let c = [61.997, -38.993];
            let s = stair(c, 5., 5., q, 0., 0.05).unwrap();
            assert!(s.scallop <= 0.05 + 1e-9, "{q}: {}", s.scallop);
            // Checked again, finer: nothing over the scallop, nothing cut
            // into the round (the tool's edge never nearer than R).
            let (most, least) = left(
                c,
                5.,
                5.,
                q,
                &s.points,
                (0., std::f64::consts::FRAC_PI_2),
                20_000,
            );
            assert!(most <= 0.05 + 1e-6, "{q}: {most}");
            assert!(least >= -1e-9, "{q}: gouges {least}");
            // Every reading on a hundredth, every move along one axis.
            for p in &s.points {
                for v in p {
                    assert!(((v / READ).round() * READ - v).abs() < 1e-6, "{v}");
                }
            }
            for w in s.points.windows(2) {
                let moved = (0..2).filter(|&k| (w[0][k] - w[1][k]).abs() > 1e-9).count();
                assert_eq!(moved, 1, "{w:?}");
            }
            // Every point is on or outside the round offset by the tool.
            for p in &s.points {
                assert!((p[0] - c[0]).hypot(p[1] - c[1]) >= 10. - 1e-9, "{p:?}");
            }
            // From the start wall's line to the end wall's.
            let (u0, u1) = ends(q);
            let (first, last) = (s.points[0], *s.points.last().unwrap());
            for k in 0..2 {
                if u0[k] == 0. {
                    assert!((first[k] - c[k]).abs() <= 0.005, "{q} {first:?}");
                }
                if u1[k] == 0. {
                    assert!((last[k] - c[k]).abs() <= 0.005, "{q} {last:?}");
                }
            }
        }
    }
    #[test]
    fn each_axis_moves_one_way_round_the_walk() {
        for q in 0..4 {
            let s = stair([0., 0.], 5., 5., q, 0., 0.05).unwrap();
            for k in 0..2 {
                let signs: HashSet<i32> = (s.points.windows(2))
                    .map(|w| {
                        (w[1][k] - w[0][k]).signum() as i32
                            * i32::from((w[1][k] - w[0][k]).abs() > 1e-9)
                    })
                    .filter(|s| *s != 0)
                    .collect();
                assert_eq!(signs.len(), 1, "{q} axis {k}: {signs:?}");
            }
        }
        // Counter-clockwise round the part: at the back right corner X goes
        // down and Y up.
        let s = stair([0., 0.], 5., 5., 0, 0., 0.05).unwrap();
        let (first, last) = (s.points[0], *s.points.last().unwrap());
        assert!(last[0] < first[0] && last[1] > first[1]);
    }
    #[test]
    fn a_finer_scallop_takes_more_steps_and_a_smaller_cutter_more_still() {
        let count =
            |rc: f64, h: f64| (stair([0., 0.], 5., rc, 0, 0., h).unwrap().points.len() - 1) / 2;
        let (coarse, fine) = (count(5., 0.2), count(5., 0.05));
        assert!(coarse < fine, "{coarse} {fine}");
        // Ø10 on R5: about eight steps for 0.05 mm, as worked out by hand.
        assert!((6..=10).contains(&fine), "{fine}");
        assert!(count(2., 0.05) > fine);
        // Too fine for 90 steps: said.
        let e = stair([0., 0.], 50., 5., 0, 0., 0.0001).unwrap_err();
        assert!(e.contains("allow a larger scallop"), "{e}");
    }
    #[test]
    fn an_offset_walk_keeps_its_offset_off_the_round() {
        let s = stair([0., 0.], 5., 5., 0, 2., 0.2).unwrap();
        let (_, least) = left(
            [0., 0.],
            7.,
            5.,
            0,
            &s.points,
            (0., std::f64::consts::FRAC_PI_2),
            4000,
        );
        assert!(least >= -1e-9, "{least}");
        let (_, least) = left(
            [0., 0.],
            5.,
            5.,
            0,
            &s.points,
            (0., std::f64::consts::FRAC_PI_2),
            4000,
        );
        assert!(least >= 2. - 1e-6, "{least}");
    }
    #[test]
    fn the_four_r5_corners_of_the_adapter_plate_are_found_and_nothing_else() {
        let mesh = adapter();
        let setup = setup("3");
        let found = all(&setup, &mesh);
        assert_eq!(found.len(), 4, "{found:?}");
        // The setup puts the part's bottom-left corner of its box at the
        // stock's: the plate's middle is 70, 47 mm in from it.
        for (k, c) in found.iter().enumerate() {
            assert_eq!(c.quarter, k);
            assert!((c.r - 5.).abs() < 0.01, "{c:?}");
            assert!((c.top - c.bottom - 20.).abs() < 0.01, "{c:?}");
        }
        // A click on one facet of a corner picks that corner.
        let one = &found[2];
        let a = arcs::outside_arc_at(&setup, &mesh, one.walls[3]).unwrap();
        assert_eq!(a.walls, one.walls);
        let picked = among(&setup, &mesh, &a.walls.iter().copied().collect()).unwrap();
        assert_eq!(picked, vec![one.clone()]);
        // A hole's round wall is no outside corner.
        assert_eq!(
            arcs::outside_arc_at(
                &setup,
                &mesh,
                (0..mesh.surfaces.len())
                    .find(
                        |&i| !one.walls.contains(&i) && !found.iter().any(|c| c.walls.contains(&i))
                    )
                    .unwrap()
            )
            .map(|_| ()),
            Err(arcs::NoArc::NotRound)
        );
        // A square corner is no rounded one; a flat wall is no corner.
        let square = plate(134., 88., 20., 5., 12, true);
        assert_eq!(all(&setup, &square).len(), 3);
    }
    #[test]
    fn the_adapter_plates_corners_are_cut_in_one_axis_moves_clear_of_the_part() {
        let mesh = adapter();
        let setup = setup("3");
        let tool = end_mill("10");
        let op = corner_op(&mesh, &tool, all_ids(&setup, &mesh));
        let (plan, walks) = walks(&setup, &op, &tool, &mesh).unwrap();
        for m in &plan.moves {
            assert!(
                (0..3).filter(|&k| m.from[k] != m.to[k]).count() <= 1,
                "{m:?}"
            );
        }
        // Four corners, each to the full 20 mm.
        let corners: HashSet<usize> = walks.iter().map(|w| w.corner).collect();
        assert_eq!(corners.len(), 4);
        assert!((plan.target - plan.stock.min[2]).abs() < 0.001);
        // Each walk leaves at most the scallop on the round; the last at
        // each level is on the round.
        for w in walks.iter().filter(|w| w.offset == 0.) {
            assert!(w.stair.scallop <= 0.05 + 1e-6, "{w:?}");
        }
        // With 3 mm of stock off each side, the stock's corner stands
        // 6.3 mm off the round: a band first, then the round.
        assert!(walks.iter().any(|w| w.offset > 0.));
        // The plan's walks read back from its moves as made.
        let zero = crate::post::datum(&setup, plan.stock, &mesh).unwrap();
        let back = read(&plan, &picked(&setup, &mesh, &op).unwrap(), zero);
        assert_eq!(back.len(), walks.len());
        for (a, b) in back.iter().zip(&walks) {
            assert_eq!(a.corner, b.corner);
            assert_eq!(a.stair.points.len(), b.stair.points.len());
            for (p, q) in a.stair.points.iter().zip(&b.stair.points) {
                assert!(
                    (p[0] - q[0]).abs() < 1e-6 && (p[1] - q[1]).abs() < 1e-6,
                    "{p:?} {q:?}"
                );
            }
            assert!(
                (a.offset - b.offset).abs() < 0.02,
                "{} {}",
                a.offset,
                b.offset
            );
            assert!((a.z - b.z).abs() < 0.001);
            assert_eq!((a.level, a.levels), (b.level, b.levels));
            if b.offset == 0. {
                assert!((a.stair.scallop - b.stair.scallop).abs() < 1e-6);
            }
        }
    }
    #[test]
    fn a_stepdown_past_the_flutes_is_refused() {
        let mesh = adapter();
        let setup = setup("3");
        let tool = end_mill("10");
        let mut op = corner_op(&mesh, &tool, all_ids(&setup, &mesh));
        op.p.insert("stepdown".into(), "30".into());
        let e = generate(&setup, &op, &tool, &mesh).unwrap_err();
        assert!(e.contains("stepdown within the flute length"), "{e}");
        op.p.insert("stepdown".into(), "10".into());
        let (plan, walks) = walks(&setup, &op, &tool, &mesh).unwrap();
        // 20 mm in two levels of 10 at each corner.
        assert!(walks.iter().all(|w| w.levels == 2), "{walks:?}");
        assert_eq!(plan.levels, 8);
    }
    #[test]
    fn a_walk_that_would_cut_the_part_is_refused() {
        // The plate with a lug standing off its back right corner, in the
        // way of the walk round it.
        let plate = adapter();
        let mut tris: Vec<[[f32; 3]; 3]> = plate
            .faces
            .iter()
            .map(|f| [f.points[0], f.points[1], f.points[2]])
            .collect();
        let (x0, x1, y0, y1, z0, z1) = (67.5f32, 68.5, 44.5, 45.5, 0., 20.);
        let c = |x: f32, y: f32, z: f32| [x, y, z];
        for (a, b, cc, d) in [
            (c(x0, y0, z1), c(x1, y0, z1), c(x1, y1, z1), c(x0, y1, z1)),
            (c(x0, y1, z0), c(x1, y1, z0), c(x1, y0, z0), c(x0, y0, z0)),
            (c(x0, y0, z0), c(x1, y0, z0), c(x1, y0, z1), c(x0, y0, z1)),
            (c(x1, y0, z0), c(x1, y1, z0), c(x1, y1, z1), c(x1, y0, z1)),
            (c(x1, y1, z0), c(x0, y1, z0), c(x0, y1, z1), c(x1, y1, z1)),
            (c(x0, y1, z0), c(x0, y0, z0), c(x0, y0, z1), c(x0, y1, z1)),
        ] {
            tris.push([a, b, cc]);
            tris.push([a, cc, d]);
        }
        let mesh = crate::mesh::normalize(tris).unwrap();
        let setup = setup("3");
        let tool = end_mill("10");
        let op = corner_op(&mesh, &tool, all_ids(&setup, &mesh));
        let e = generate(&setup, &op, &tool, &mesh).unwrap_err();
        assert!(
            e.starts_with("Round corners cuts into the part at X"),
            "{e}"
        );
        assert!(e.contains("back right"), "{e}");
    }
    #[test]
    fn round_corners_runs_on_the_manual_mill_only() {
        let mesh = adapter();
        let mut setup = setup("3");
        let tool = end_mill("10");
        let op = corner_op(&mesh, &tool, all_ids(&setup, &mesh));
        setup.machine = crate::model::Machine::new(3);
        assert_eq!(generate(&setup, &op, &tool, &mesh).unwrap_err(), ON_CNC);
        let issues = op.issues(&setup.machine, std::slice::from_ref(&tool), &[]);
        assert!(issues.iter().any(|i| i == ON_CNC), "{issues:?}");
    }
    #[test]
    fn nothing_picked_says_what_to_pick() {
        let mesh = adapter();
        let setup = setup("3");
        let tool = end_mill("10");
        let op = corner_op(&mesh, &tool, vec![]);
        assert_eq!(
            generate(&setup, &op, &tool, &mesh).unwrap_err(),
            NONE_PICKED
        );
    }
    /// The adapter plate face up on the manual mill, in the 100 mm vise
    /// across `across` (its jaws on the ±Y sides for "Y"), gripping 8 mm, 3
    /// mm of stock round it, the work zero on the stock top's centre, its
    /// four corners rounded with a Ø10 end mill.
    pub(crate) fn plate_job(across: &str) -> (crate::model::Document, Mesh) {
        let mesh = adapter();
        let mut doc = crate::model::Document::default();
        let machine = crate::manual::defaults();
        let mut tool = end_mill("10");
        tool.p.insert("machines".into(), machine.id.clone());
        doc.machines.push(machine);
        doc.workholding = crate::holding::starters();
        let manual = doc.machines[0].id.clone();
        doc.workholding[0].p.insert("machines".into(), manual);
        let vise = doc.workholding[0].id.clone();
        doc.add_manual_setup().unwrap();
        let ids = all_ids(&setup("3"), &mesh);
        let s = &mut doc.setups[0];
        for (k, v) in [
            ("holding", "Vise"),
            ("holder", vise.as_str()),
            ("vise_axis", across),
            ("vise_grip", "8"),
            ("holding_gap", "2"),
            ("origin", "Stock top center"),
            ("stock_type", "Relative size box"),
            ("stock_source", "fresh"),
            ("offset", "G54"),
        ] {
            s.p.insert(k.into(), v.into());
        }
        for side in ["left", "right", "front", "back"] {
            s.p.insert(format!("stock_{side}"), "3".into());
        }
        for side in ["top", "bottom"] {
            s.p.insert(format!("stock_{side}"), "0".into());
        }
        s.operations = vec![corner_op(&mesh, &tool, ids)];
        doc.tools.push(tool);
        doc.active = doc.setups[0].id.clone();
        (doc, mesh)
    }
    #[test]
    fn the_plates_corners_clear_the_vise_jaws_across_y_and_not_across_x() {
        // Across X (a vise that opens to 200 mm) the jaws' 100 mm width
        // covers the corners.
        for (across, clear) in [("Y", true), ("X", false)] {
            let (mut doc, mesh) = plate_job(across);
            doc.workholding[0]
                .p
                .insert("max_opening".into(), "200".into());
            let setup = &doc.setups[0];
            let plan = generate(setup, &setup.operations[0], &doc.tools[0], &mesh).unwrap();
            let boxes = doc
                .holding_boxes(setup, crate::holding::Gripped::whole(plan.stock))
                .unwrap();
            let hits = crate::holding::hits(&plan, &boxes, 2.);
            assert_eq!(hits.is_empty(), clear, "across {across}: {hits:?}");
        }
    }
    #[test]
    fn the_plates_steps_walk_each_corner_by_numbers_and_end_filed() {
        use crate::guide::{Step, build};
        let (doc, mesh) = plate_job("Y");
        let guide = build(&doc, &mesh, 0).unwrap();
        let walks: Vec<&Step> = guide
            .steps
            .iter()
            .filter(|s| matches!(s, Step::Corner { .. }))
            .collect();
        assert!(walks.len() >= 4, "{}", walks.len());
        // The section counts its walks.
        let section = guide.sections.iter().find(|s| s.op == Some(0)).unwrap();
        assert_eq!(section.count, walks.len());
        let mut seen = HashSet::new();
        for (k, w) in walks.iter().enumerate() {
            let Step::Corner {
                index,
                of,
                corner,
                corners,
                radius,
                centre,
                offset,
                z,
                scallop,
                points,
                targets,
                ..
            } = w
            else {
                unreachable!()
            };
            assert_eq!((*index, *of, *corners), (k + 1, walks.len(), 4));
            seen.insert(*corner);
            assert!((radius - 5.).abs() < 0.01);
            // Work coordinates: the corners' centres at ±62, ±39.
            assert!(
                (centre[0].abs() - 62.).abs() < 0.01 && (centre[1].abs() - 39.).abs() < 0.01,
                "{centre:?}"
            );
            assert!(*z < 0. && *z >= -20.001);
            if *offset == 0. {
                assert!(*scallop <= 0.05 + 1e-6, "{scallop}");
            }
            // Each target moves one axis to a hundredth; the walk's feeds
            // are the readings, then out.
            for t in targets {
                assert!(
                    ((t.centre * 100.).round() / 100. - t.centre).abs() < 2e-3,
                    "{t:?}"
                );
                assert!(!t.edge && t.shown == t.centre);
            }
            let feeds: Vec<&crate::guide::Target> = targets.iter().filter(|t| t.feed).collect();
            assert_eq!(feeds.len(), points.len() + 1);
            for (t, p) in feeds.iter().zip(points) {
                assert!((t.centre - p[t.axis]).abs() < 1e-3, "{t:?} {p:?}");
            }
            // Down once to the level, locked, before the first feed.
            let down = targets
                .iter()
                .position(|t| t.axis == 2 && t.lock[2])
                .unwrap();
            assert!((targets[down].centre - z).abs() < 1e-3);
            assert!(down < targets.iter().position(|t| t.feed).unwrap());
            // The backlash taken up: the axis read before going down is
            // first a millimetre short, then on it, the way the walk goes.
            let (a, b) = (&targets[down - 2], &targets[down - 1]);
            let along = if (points[0][0] - points.last().unwrap()[0]).abs() > 0.5
                && (points[0][1] - points.last().unwrap()[1]).abs() > 0.5
            {
                b.axis
            } else {
                unreachable!()
            };
            if a.axis == along {
                let way = (points.last().unwrap()[along] - points[0][along]).signum();
                assert!(
                    ((b.centre - a.centre) - way * TAKE_UP).abs() < 1e-3,
                    "{a:?} {b:?}"
                );
            }
        }
        assert_eq!(seen.len(), 4);
        // Then the scallops are filed.
        let last = guide
            .sections
            .iter()
            .find(|s| s.op == Some(0))
            .unwrap()
            .steps
            .end
            - 1;
        assert!(matches!(guide.steps[last], Step::Check { op: 0 }));
    }
    /// The most material the tool sweeping `path` leaves proud of the round,
    /// by brute force: along each of `rays` normals out of it, stepped out
    /// 0.0005 mm at a time to the first point within `rc` of the path, then
    /// halved down to it. Independent of `ray_capsule`.
    fn brute(c: [f64; 2], r: f64, rc: f64, q: usize, path: &[[f64; 2]], rays: usize) -> f64 {
        let inside = |p: [f64; 2]| {
            path.windows(2)
                .map(|w| segment(p, w[0], w[1]))
                .fold(f64::INFINITY, f64::min)
                <= rc
        };
        let mut most: f64 = 0.;
        for k in 0..=rays {
            let n = unit(q, std::f64::consts::FRAC_PI_2 * k as f64 / rays as f64);
            let at = |t: f64| [c[0] + (r + t) * n[0], c[1] + (r + t) * n[1]];
            let mut t = 0.;
            while !inside(at(t)) {
                t += 0.0005;
                assert!(t < 5., "the ray at {k} meets no cut");
            }
            let (mut lo, mut hi) = ((t - 0.0005f64).max(0.), t);
            if !inside(at(lo)) {
                for _ in 0..30 {
                    let mid = (lo + hi) / 2.;
                    if inside(at(mid)) { hi = mid } else { lo = mid }
                }
            } else {
                hi = lo;
            }
            most = most.max(hi);
        }
        most
    }
    #[test]
    fn the_scallop_is_the_height_left_along_the_rounds_normal_checked_by_brute_force() {
        // R5 with Ø10, Ø6 and Ø1 cutters (the Ø1 left 0.053 mm under the
        // nearest-distance measure), about a centre off the hundredths.
        for (rc, q) in [(5., 0), (3., 1), (0.5, 2), (0.5, 0)] {
            let c = [-61.997, 38.993];
            let s = stair(c, 5., rc, q, 0., 0.05).unwrap();
            let found = brute(c, 5., rc, q, &s.points, 20_000);
            assert!(found <= 0.05 + 1e-6, "Ø{} q{q}: {found}", 2. * rc);
            assert!(
                found <= s.scallop + 1e-6,
                "Ø{} q{q}: {found} over the {} said",
                2. * rc,
                s.scallop
            );
            assert!(
                s.scallop - found < 1e-3,
                "Ø{} q{q}: {} said, {found} found",
                2. * rc,
                s.scallop
            );
        }
    }
    #[test]
    fn a_counter_clockwise_spindle_walks_the_other_way_round_still_conventional() {
        let mesh = adapter();
        let setup = setup("3");
        let tool = end_mill("10");
        let mut op = corner_op(&mesh, &tool, all_ids(&setup, &mesh));
        let (_, cw) = walks(&setup, &op, &tool, &mesh).unwrap();
        op.p.insert("spindle_rotation".into(), "Counterclockwise (CCW)".into());
        let (plan, ccw) = walks(&setup, &op, &tool, &mesh).unwrap();
        assert!(!plan.spindle_clockwise);
        assert_eq!(plan.milling_direction, "Conventional");
        for m in &plan.moves {
            assert!(
                (0..3).filter(|&k| m.from[k] != m.to[k]).count() <= 1,
                "{m:?}"
            );
        }
        assert_eq!(cw.len(), ccw.len());
        for (a, b) in cw.iter().zip(&ccw) {
            let mut back = a.stair.points.clone();
            back.reverse();
            assert_eq!(back, b.stair.points);
        }
        // At the back right corner: in along Y from the back, X then only
        // ever up, Y only ever down, out along X to the right.
        let w = &ccw[0];
        let feeds: Vec<&Move> = plan.moves[w.moves.clone()]
            .iter()
            .filter(|m| m.cutting_pass)
            .collect();
        let (first, last) = (feeds[0], feeds[feeds.len() - 1]);
        assert!(
            first.to[1] < first.from[1] && first.to[0] == first.from[0],
            "{first:?}"
        );
        assert!(
            last.to[0] > last.from[0] && last.to[1] == last.from[1],
            "{last:?}"
        );
        for m in &feeds[1..feeds.len() - 1] {
            assert!(m.to[0] >= m.from[0] && m.to[1] <= m.from[1], "{m:?}");
        }
    }
    #[test]
    fn steps_refuse_walks_into_the_jaws_and_say_where() {
        let (mut doc, mesh) = plate_job("X");
        doc.workholding[0]
            .p
            .insert("max_opening".into(), "200".into());
        let e = crate::guide::build(&doc, &mesh, 0).unwrap_err();
        assert!(e.contains("hits the vise jaw"), "{e}");
        assert!(e.contains("stand clear of the vise's jaws"), "{e}");
        // A vise that does not open to the stock, or a holder no longer in
        // the library, cannot be checked: no Steps either, and why.
        let (doc, mesh) = plate_job("X");
        let e = crate::guide::build(&doc, &mesh, 0).unwrap_err();
        assert!(
            e.contains("the workholding cannot be checked against the walks"),
            "{e}"
        );
        assert!(e.contains("opens to 120 mm"), "{e}");
        let (mut doc, mesh) = plate_job("Y");
        doc.workholding.clear();
        let e = crate::guide::build(&doc, &mesh, 0).unwrap_err();
        assert!(
            e.contains("the workholding cannot be checked against the walks"),
            "{e}"
        );
        // The vise across Y, the ends out of it: the Steps are built; and
        // with no workholding modelled, as any manual setup's.
        let (mut doc, mesh) = plate_job("Y");
        assert!(crate::guide::build(&doc, &mesh, 0).is_ok());
        doc.setups[0].p.insert("holding".into(), "None".into());
        let built = crate::guide::build(&doc, &mesh, 0);
        assert!(built.is_ok(), "{:?}", built.err());
    }
    #[test]
    fn every_allowed_scallop_plans() {
        let mesh = adapter();
        let setup = setup("3");
        let tool = end_mill("10");
        let mut op = corner_op(&mesh, &tool, all_ids(&setup, &mesh));
        for h in ["0.01", "0.05", "0.5", "0.6", "1"] {
            op.p.insert("scallop".into(), h.into());
            let (_, walks) = walks(&setup, &op, &tool, &mesh).unwrap();
            let h: f64 = h.parse().unwrap();
            assert!(
                walks.iter().all(|w| w.stair.scallop <= h.max(0.5) + 1e-9),
                "{h}"
            );
            assert!(
                walks
                    .iter()
                    .filter(|w| w.offset == 0.)
                    .all(|w| w.stair.scallop <= h + 1e-9),
                "{h}"
            );
        }
    }
    #[test]
    fn the_settings_are_checked() {
        let mut p = Fields::new();
        for (k, v) in DEFAULTS {
            p.insert(k.into(), v.into());
        }
        assert!(setting_issues(&p).is_empty());
        p.insert("scallop".into(), "0.001".into());
        p.insert("stepover".into(), "5".into());
        p.insert("stepdown".into(), "-1".into());
        assert_eq!(setting_issues(&p).len(), 3);
    }
    /// The adapter plate's setup 3 as planned on the manual mill (LIO-447):
    /// face up on its finished 88 mm sides in the vise across Y, the 140 mm
    /// stock 3 mm past each end, the work zero on the stock top's centre;
    /// its four R5 corners rounded with T62, a Ø10 3-flute end mill, at
    /// 1600 rpm and 150 mm/min, Auto stepdown, a 40 % band, 0.05 scallop.
    pub(crate) fn adapter_plate_setup_3() -> (crate::model::Document, Mesh) {
        let (mut doc, mesh) = plate_job("Y");
        // The shop's 800 W mill, 90–1600 rpm, cutting aluminium 5052.
        doc.materials = crate::material::starters();
        doc.material_id = (doc.materials.iter())
            .find(|m| m.name() == "Aluminium 5052-H32")
            .unwrap()
            .id
            .clone();
        doc.machines[0].p.insert("power".into(), "800".into());
        doc.machines[0].p.insert("speeds".into(), "90-1600".into());
        doc.setups[0].machine = doc.machines[0].clone();
        let s = &mut doc.setups[0];
        for side in ["front", "back"] {
            s.p.insert(format!("stock_{side}"), "0".into());
        }
        // The two ends first, squared to X ±67 over the full 20 mm.
        let tool = doc.tools[0].id.clone();
        for (name, x) in [("right", ["67", "80"]), ("left", ["-80", "-67"])] {
            let mut end = Operation::new(crate::model::Strategy::Box, tool.clone());
            for (k, v) in [
                ("name", format!("Finish {name} end")),
                ("x_min", x[0].into()),
                ("x_max", x[1].into()),
                ("y_min", "-49".into()),
                ("y_max", "49".into()),
                ("z_top", "0".into()),
                ("z_bottom", "-20".into()),
                ("rpm", "1600".into()),
                ("feed", "150".into()),
                ("stepdown", String::new()),
                ("stepover", "40".into()),
            ] {
                end.p.insert(k.into(), v);
            }
            s.operations.insert(s.operations.len() - 1, end);
        }
        let op = s.operations.last_mut().unwrap();
        for (k, v) in [
            ("rpm", "1600"),
            ("feed", "150"),
            ("stepdown", ""),
            ("stepover", "40"),
            ("scallop", "0.05"),
            ("clearance", "15"),
            ("retract", "5"),
        ] {
            op.p.insert(k.into(), v.into());
        }
        let tool = &mut doc.tools[0];
        for (k, v) in [("stickout", "35"), ("holder", "32"), ("length", "25")] {
            tool.p.insert(k.into(), v.into());
        }
        (doc, mesh)
    }
    /// A walk's DRO numbers as the Steps say them: "X-77.00", a feed "fX…".
    fn rows(targets: &[crate::guide::Target]) -> Vec<String> {
        (targets.iter())
            .map(|t| {
                format!(
                    "{}{}{:.2}",
                    if t.feed { "f" } else { "" },
                    ["X", "Y", "Z"][t.axis],
                    t.centre
                )
            })
            .collect()
    }
    #[test]
    fn every_walk_of_the_adapter_plate_enters_safely_one_way_and_draws() {
        use crate::guide::{Step, build};
        let (doc, mesh) = adapter_plate_setup_3();
        let guide = build(&doc, &mesh, 0).unwrap();
        let walks: Vec<&Step> = (guide.steps.iter())
            .filter(|s| matches!(s, Step::Corner { .. }))
            .collect();
        // Four corners, two levels of 10 mm, a band 2.22 mm off the round
        // and the round: 16 walks, as on the saved plate.
        assert_eq!(walks.len(), 16);
        let (stock, zero) = (guide.done[0].plan.stock, guide.zero);
        let top = stock.max[2] - zero[2];
        for w in &walks {
            let Step::Corner {
                index,
                place,
                radius,
                centre,
                offset,
                z,
                level,
                scallop,
                points,
                targets,
                ..
            } = w
            else {
                unreachable!()
            };
            let r = rows(targets);
            // The numbers the walk's page draws, from its first: every one
            // known, so the drawing has no NaN (walk 9 had no X).
            let mut at = [f32::NAN; 3];
            let first = targets.iter().position(|t| t.feed).unwrap();
            for t in &targets[..first] {
                at[t.axis] = t.centre;
            }
            assert!(at.iter().all(|v| v.is_finite()), "walk {index}: {r:?}");
            // Up above the stock first; there the axis it feeds in along set
            // beside the stock, the other a millimetre short and then on to
            // the first reading the walk's way; down to the level; in.
            let way = |k: usize| (points[points.len() - 1][k] - points[0][k]).signum();
            let [up, safe, short, on, down] = &targets[..5] else {
                unreachable!()
            };
            assert!(
                up.axis == 2 && up.centre >= top + 10.,
                "walk {index}: {r:?}"
            );
            assert!(
                safe.axis < 2 && short.axis == 1 - safe.axis,
                "walk {index}: {r:?}"
            );
            assert_eq!(on.axis, short.axis, "walk {index}: {r:?}");
            assert!(
                ((on.centre - short.centre) - way(on.axis) * TAKE_UP).abs() < 1e-3,
                "walk {index}: {r:?}"
            );
            assert!((on.centre - points[0][on.axis]).abs() < 1e-3);
            // The tool goes down with its side clear of the stock.
            let edge = safe.centre.abs() - 5.;
            let half = [stock.max[0] - stock.min[0], stock.max[1] - stock.min[1]][safe.axis] / 2.;
            assert!(edge >= half + 1.9, "walk {index}: {r:?}");
            assert!(down.axis == 2 && down.lock[2] && (down.centre - z).abs() < 1e-3);
            // Every feed the walk's one way on its axis, in and out too.
            let mut was = [safe.centre, on.centre];
            if safe.axis == 1 {
                was = [on.centre, safe.centre];
            }
            for t in targets[5..].iter().filter(|t| t.feed) {
                assert!(
                    (t.centre - was[t.axis]).signum() == way(t.axis),
                    "walk {index}: {r:?}"
                );
                was[t.axis] = t.centre;
            }
            assert!(
                crate::corner::walk_problem(
                    targets,
                    points,
                    *z,
                    (
                        [stock.min[0] - zero[0], stock.min[1] - zero[1]],
                        [stock.max[0] - zero[0], stock.max[1] - zero[1]],
                        top
                    ),
                    &Round {
                        centre: *centre,
                        radius: *radius,
                        quarter: WHERE.iter().position(|p| p == place).unwrap(),
                        tool: 5.,
                        offset: *offset,
                        scallop: *scallop,
                    }
                )
                .is_none()
            );
            // Walk 9: the front left corner's band at Z −10, from X −77.
            if *index == 9 {
                assert_eq!(*place, WHERE[2]);
                assert_eq!((*level, (offset * 100.).round()), (1, 222.));
                assert_eq!(
                    r[..6],
                    [
                        "Z15.00", "X-77.00", "Y-38.00", "Y-39.00", "Z-10.00", "fX-74.22"
                    ],
                    "{r:?}"
                );
            }
        }
    }
    /// A 134 × 88 × 20 plate whose back right is stepped: two R5 outside
    /// corners facing the same way, about X 62 Y 19 and X 42 Y 39, 20 mm
    /// apart each way (LIO-447 review). Its outline, counter-clockwise,
    /// extruded with its top and bottom cut into ears.
    fn stepped_plate() -> Mesh {
        let mut p: Vec<[f32; 2]> = vec![[67., -44.], [67., 19.]];
        for (cx, cy) in [(62., 19.), (42., 39.)] {
            if cx == 42. {
                p.extend([[47., 24.], [47., 39.]]);
            }
            for i in 1..=12 {
                let a = std::f32::consts::FRAC_PI_2 * i as f32 / 12.;
                p.push([cx + 5. * a.cos(), cy + 5. * a.sin()]);
            }
        }
        p.extend([[-67., 44.], [-67., -44.]]);
        let cross = |a: [f32; 2], b: [f32; 2], c: [f32; 2]| {
            (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
        };
        let at = |i: usize, z: f32| [p[i][0], p[i][1], z];
        let mut tris = vec![];
        for i in 0..p.len() {
            let j = (i + 1) % p.len();
            tris.push([at(i, 0.), at(j, 0.), at(j, 20.)]);
            tris.push([at(i, 0.), at(j, 20.), at(i, 20.)]);
        }
        let mut left: Vec<usize> = (0..p.len()).collect();
        while left.len() > 2 {
            let n = left.len();
            let corner = |e: usize| [left[(e + n - 1) % n], left[e], left[(e + 1) % n]];
            let ear = (0..n)
                .find(|&e| {
                    let [a, b, c] = corner(e);
                    cross(p[a], p[b], p[c]) > 1e-5
                        && !left.iter().any(|&k| {
                            ![a, b, c].contains(&k)
                                && cross(p[a], p[b], p[k]) >= -1e-5
                                && cross(p[b], p[c], p[k]) >= -1e-5
                                && cross(p[c], p[a], p[k]) >= -1e-5
                        })
                })
                .unwrap();
            let [a, b, c] = corner(ear);
            tris.push([at(a, 20.), at(b, 20.), at(c, 20.)]);
            tris.push([at(c, 0.), at(b, 0.), at(a, 0.)]);
            left.remove(ear);
        }
        crate::mesh::normalize(tris).unwrap()
    }
    #[test]
    fn two_corners_facing_the_same_way_each_keep_their_own_walks() {
        let mesh = stepped_plate();
        for spindle in crate::model::SPINDLE_ROTATIONS {
            let (mut doc, _) = plate_job("Y");
            doc.setups[0].p.insert("vise_offset".into(), "-35".into());
            let corners = all(&doc.setups[0], &mesh);
            assert_eq!(corners.len(), 2, "{corners:?}");
            assert!(corners.iter().all(|c| c.quarter == 0));
            let ids = corners.iter().flat_map(|c| c.walls.clone()).collect();
            let mut op = corner_op(&mesh, &doc.tools[0], ids);
            op.p.insert("stepdown".into(), "10".into());
            op.p.insert("spindle_rotation".into(), spindle.into());
            doc.setups[0].operations = vec![op];
            let setup = &doc.setups[0];
            let (plan, made) = walks(setup, &setup.operations[0], &doc.tools[0], &mesh).unwrap();
            let zero = crate::post::datum(setup, plan.stock, &mesh).unwrap();
            // Read back, each walk is round the corner it was made round,
            // its offset its own.
            let back = read(&plan, &corners, zero);
            assert_eq!(back.len(), made.len());
            assert!(made.iter().any(|w| w.offset > 15.), "{spindle}");
            for (k, (a, b)) in made.iter().zip(&back).enumerate() {
                assert_eq!(a.corner, b.corner, "{spindle}: walk {}", k + 1);
                assert!(
                    (a.offset - b.offset).abs() < 0.02,
                    "{spindle}: walk {}",
                    k + 1
                );
                assert!(b.stair.scallop.is_finite(), "{spindle}: walk {}", k + 1);
            }
            // And the Steps say so, every number finite.
            let guide = crate::guide::build(&doc, &mesh, 0).unwrap();
            let steps: Vec<_> = (guide.steps.iter())
                .filter_map(|s| match s {
                    crate::guide::Step::Corner {
                        corner,
                        offset,
                        scallop,
                        ..
                    } => Some((*corner, *offset, *scallop)),
                    _ => None,
                })
                .collect();
            assert_eq!(steps.len(), made.len());
            for (k, (w, s)) in made.iter().zip(&steps).enumerate() {
                assert_eq!(s.0, w.corner + 1, "{spindle}: walk {}", k + 1);
                assert!(
                    (s.1 - w.offset).abs() < 0.02 && s.2.is_finite(),
                    "{spindle}: {s:?}"
                );
            }
        }
    }
    /// Walk 9's numbers as the Steps gave them before LIO-447.
    fn walk_9_before() -> (Vec<crate::guide::Target>, Vec<[f32; 2]>) {
        let t = |axis: usize, centre: f32, feed: bool, down: bool| crate::guide::Target {
            order: 0,
            axis,
            centre,
            shown: centre,
            edge: false,
            feed,
            lock: [false, false, down],
        };
        let points = vec![
            [-74.22, -39.],
            [-74.22, -45.61],
            [-72.28, -45.61],
            [-72.28, -49.29],
            [-68.6, -49.29],
            [-68.6, -51.22],
            [-62., -51.22],
        ];
        let mut targets = vec![
            t(1, -38., false, false),
            t(2, 5., false, false),
            t(1, -39., false, false),
            t(2, -10., false, true),
            t(0, -74.22, true, false),
        ];
        let mut was = points[0];
        for p in &points[1..] {
            let k = if p[0] != was[0] { 0 } else { 1 };
            targets.push(t(k, p[k], true, false));
            was = *p;
        }
        targets.push(t(1, -53.22, true, false));
        targets.push(t(2, 15., false, false));
        (targets, points)
    }
    #[test]
    fn steps_that_cannot_be_followed_are_named() {
        let bounds = ([-70., -44.], [70., 44.], 0.);
        let (before, points) = walk_9_before();
        let front_left = Round {
            centre: [-62., -39.],
            radius: 5.,
            quarter: 2,
            tool: 5.,
            offset: 2.22,
            scallop: 0.48,
        };
        let problem = |targets: &[crate::guide::Target]| {
            walk_problem(targets, &points, -10., bounds, &front_left)
        };
        // Walk 9 as it was: down with no X.
        let e = problem(&before).unwrap();
        assert!(e.contains("goes down with no X"), "{e}");
        // As it is now: none.
        let mut fixed = before.clone();
        let t = |axis: usize, centre: f32| crate::guide::Target {
            order: 0,
            axis,
            centre,
            shown: centre,
            edge: false,
            feed: false,
            lock: [false; 3],
        };
        fixed.splice(0..2, [t(2, 15.), t(0, -77.), t(1, -38.)]);
        assert_eq!(problem(&fixed), None);
        // Not a number.
        let mut nan = fixed.clone();
        nan[1].centre = f32::NAN;
        assert!(problem(&nan).unwrap().contains("not a number"));
        // X and Y set before the tool is up.
        let mut low = fixed.clone();
        low[0].centre = -5.;
        assert!(problem(&low).unwrap().contains("before the tool is up"));
        // Down onto the stock.
        let mut onto = fixed.clone();
        onto[1].centre = -72.;
        assert!(problem(&onto).unwrap().contains("onto the stock"));
        // Y on to its first reading against the walk's way: no backlash
        // taken up.
        let mut against = fixed.clone();
        against[2].centre = -40.;
        assert!(
            problem(&against).unwrap().contains("backlash"),
            "{against:?}"
        );
        // A feed back against the walk's one way.
        let mut back = fixed.clone();
        let i = back.iter().position(|t| t.feed && t.axis == 0).unwrap() + 2;
        back[i].centre = -75.;
        assert!(
            problem(&back)
                .unwrap()
                .contains("against the walk's one way")
        );
        // Not up above the stock at the end.
        let mut stays = fixed.clone();
        stays.pop();
        assert!(problem(&stays).unwrap().contains("above the stock"));
        // Said of another corner: a neighbour's on the same side, or the
        // walk's offset not its own; a scallop not a number (LIO-447).
        for (other, why) in [
            (
                Round {
                    centre: [-42., -59.],
                    ..front_left
                },
                "not round its own corner",
            ),
            (
                Round {
                    offset: 0.,
                    ..front_left
                },
                "not round its own corner",
            ),
            (
                Round {
                    scallop: f32::INFINITY,
                    ..front_left
                },
                "not a number",
            ),
        ] {
            let e = walk_problem(&fixed, &points, -10., bounds, &other).unwrap();
            assert!(e.contains(why), "{e}");
        }
    }
    /// The saved adapter plate's Corner (LIO-447), when `AXIS_ADAPTER_PLATE`
    /// names a copy of the project and the build reads STEP: every walk's
    /// Steps built and checked.
    #[test]
    #[ignore]
    fn the_saved_adapter_plates_walks_all_follow() {
        let Ok(path) = std::env::var("AXIS_ADAPTER_PLATE") else {
            return;
        };
        let mut doc = crate::storage::load(std::path::Path::new(&path)).unwrap();
        doc.sync_stock_sources();
        let mesh = crate::mesh::load_file(doc.file.as_ref().unwrap()).unwrap();
        let si = (doc.setups.iter())
            .position(|s| s.operations.iter().any(|o| o.strategy == Strategy::Corner))
            .unwrap();
        let guide = crate::guide::build(&doc, &mesh, si).unwrap();
        let walks: Vec<_> = (guide.steps.iter())
            .filter_map(|s| match s {
                crate::guide::Step::Corner { targets, .. } => Some(rows(targets)),
                _ => None,
            })
            .collect();
        assert_eq!(walks.len(), 16);
        assert_eq!(
            walks[8][..6],
            [
                "Z15.00", "X-77.00", "Y-38.00", "Y-39.00", "Z-10.00", "fX-74.22"
            ]
        );
    }
}
