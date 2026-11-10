//! A Drill's arcs: part of a round wall of the part, picked by a click on it
//! where the click picks no whole round hole: a half hole on the part's
//! edge, the round end of a slot, a hole a pocket or a slot breaks into, a
//! round inside corner. A Drill drills the arc's whole circle round its
//! centre, as wide as the circle, down to the floor under the arc.
use super::{Geometry, Hole, bore_at, circle_centre, plane_height};
use crate::{mesh::Mesh, model::Setup};
use std::collections::{HashMap, HashSet};

/// Part of a round wall, as `setup` places the part (mm, setup frame).
#[derive(Debug, Clone, PartialEq)]
pub struct Arc {
    /// The circle's centre and radius: what its facets' corners lie on.
    pub centre: [f32; 2],
    pub r: f32,
    /// The surfaces of its facets, in order of id.
    pub walls: Vec<usize>,
    /// How much of the round its facets go round, degrees (less than 360).
    pub span: f32,
    /// The angle about the centre of its middle, radians: the side the part
    /// is on.
    pub middle: f32,
    /// The highest and the lowest of its facets' corners.
    pub top: f32,
    pub bottom: f32,
    /// How far inside `r` its facets' chords come.
    pub sagitta: f32,
}
/// Why a wall is no arc a Drill drills.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoArc {
    /// Not part of a round upright wall.
    NotRound,
    /// It curves round the part, not round a hole (a boss, a rounded
    /// outside corner).
    Outside,
    /// It goes all the way round: a whole hole, read as one.
    Whole,
}
/// The refusal for a wall that curves round the part (`NoArc::Outside`).
pub const OUTSIDE: &str = "That wall curves round the outside of the part (a boss or a rounded corner), not round a hole: a Drill drills holes. Click the wall of a hole, or part of one.";
/// One upright face of the part (a mesh surface) seen from above.
struct Facet {
    /// Its corners seen from above, each once.
    xy: Vec<(f64, f64)>,
    low: f32,
    high: f32,
    /// Its normal, level and of unit length: out of the part.
    normal: [f32; 2],
}
/// The facets at each corner seen from above, by its key (`key`).
type Corners = HashMap<(i64, i64), Vec<usize>>;
#[cfg(test)]
thread_local! {
    /// How many circles `fit` has fitted on this thread: what a click on
    /// a wall costs.
    pub static FITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
/// Two corners seen from above are one within this, mm.
const SAME: f64 = 1e-3;
/// A corner's key, for finding the facets that meet at it.
fn key(p: (f64, f64)) -> (i64, i64) {
    ((p.0 / SAME).round() as i64, (p.1 / SAME).round() as i64)
}
/// The upright faces of `g` (mesh surfaces whose triangles lean less than
/// about 1° from vertical) that `keep` keeps.
fn facets(g: &Geometry, keep: impl Fn(usize) -> bool) -> HashMap<usize, Facet> {
    // Per surface: its corners, and its normal's sum, by area.
    let mut found: HashMap<usize, (Vec<[f32; 3]>, [f32; 3])> = HashMap::new();
    for t in &g.triangles {
        let Some(id) = t.feature.filter(|&id| keep(id)) else {
            continue;
        };
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
        let entry = found.entry(id).or_default();
        entry.0.extend(t.p);
        for (sum, v) in entry.1.iter_mut().zip(n) {
            *sum += v;
        }
    }
    let mut out = HashMap::new();
    for (id, (corners, n)) in found {
        let length = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        let level = n[0].hypot(n[1]);
        if length < 1e-9 || n[2].abs() / length > 0.02 {
            continue;
        }
        let mut xy: Vec<(f64, f64)> = vec![];
        for p in &corners {
            let q = (p[0] as f64, p[1] as f64);
            if !xy.iter().any(|r| (r.0 - q.0).hypot(r.1 - q.1) < SAME) {
                xy.push(q);
            }
        }
        if xy.len() < 2 {
            continue;
        }
        out.insert(
            id,
            Facet {
                xy,
                low: corners.iter().map(|p| p[2]).fold(f32::INFINITY, f32::min),
                high: (corners.iter().map(|p| p[2])).fold(f32::NEG_INFINITY, f32::max),
                normal: [n[0] / level, n[1] / level],
            },
        );
    }
    out
}
/// The circle the corners of `members` lie on, and how far the farthest is
/// off it: None for fewer than four corners, corners on no circle within
/// 0.005 mm and 0.01 % of its radius (a CAD tessellation's corners are on
/// it to a thousandth: a spline's are not), or facets going more than 45°
/// round it, or not as far round as each other.
fn fit(facets: &HashMap<usize, Facet>, members: &[usize]) -> Option<([f64; 2], f64, f64)> {
    #[cfg(test)]
    FITS.with(|n| n.set(n.get() + 1));
    let mut seen = HashSet::new();
    let points: Vec<(f64, f64)> = (members.iter())
        .flat_map(|m| facets[m].xy.iter().copied())
        .filter(|q| seen.insert(key(*q)))
        .collect();
    if points.len() < 4 {
        return None;
    }
    let c = circle_centre(&points)?;
    let distance = |p: &(f64, f64)| (p.0 - c.0).hypot(p.1 - c.1);
    let r = points.iter().map(distance).sum::<f64>() / points.len() as f64;
    let off = (points.iter())
        .map(|p| (distance(p) - r).abs())
        .fold(0., f64::max);
    if !(r > 0.05 && r < 5000. && off <= 0.005 + r * 1e-4) {
        return None;
    }
    // How far round each facet goes: its longest chord's angle.
    let angles: Vec<f64> = (members.iter().map(|m| &facets[m].xy))
        .map(|xy| {
            let longest = (xy.iter().enumerate())
                .flat_map(|(i, a)| {
                    xy[i + 1..]
                        .iter()
                        .map(move |b| (b.0 - a.0).hypot(b.1 - a.1))
                })
                .fold(0., f64::max);
            2. * (longest / (2. * r)).min(1.).asin()
        })
        .collect();
    // A facet going more than 45° round is a flat that ends on the circle
    // (a slot's side between the first facets of its ends): no circle's
    // facet. Facets may go round unevenly: each two next to each other are
    // checked as chords of one circle (`inscribed`).
    let widest = angles.iter().copied().fold(0., f64::max);
    (widest <= 45f64.to_radians()).then_some(([c.0, c.1], r, off))
}
/// The longest chord of facet `f` seen from above.
fn chord(f: &Facet) -> f64 {
    (f.xy.iter().enumerate())
        .flat_map(|(i, a)| {
            f.xy[i + 1..]
                .iter()
                .map(move |b| (b.0 - a.0).hypot(b.1 - a.1))
        })
        .fold(0., f64::max)
}
/// How far facet `b` turns from `a`, by their normals, radians.
fn turn(a: &Facet, b: &Facet) -> f64 {
    ((a.normal[0] * b.normal[0] + a.normal[1] * b.normal[1]) as f64)
        .clamp(-1., 1.)
        .acos()
}
/// Whether facets `a` and `b`, next to each other, are two chords of one
/// circle of radius `r`: `b` turns from `a` half of the two's angles round
/// it together (3 % give or take): exact for any two chords of a circle,
/// however unevenly it is cut, and off for two of an ellipse's or a
/// spline's, whose curve bends more or less from one to the next. A flat tangent to the circle at the
/// end of `a` turns half of `a`'s only, however close its far end is to
/// the circle (a slot's short side).
fn inscribed(a: &Facet, b: &Facet, r: f64) -> bool {
    let angle = |f: &Facet| 2. * (chord(f) / (2. * r)).min(1.).asin();
    let expected = (angle(a) + angle(b)) / 2.;
    (turn(a, b) - expected).abs() <= 0.03 * expected + 1e-4
}
/// The facets of `facets` at each of their corners, by its key.
fn corners(facets: &HashMap<usize, Facet>) -> Corners {
    let mut at: Corners = HashMap::new();
    for (&id, f) in facets {
        for p in &f.xy {
            at.entry(key(*p)).or_default().push(id);
        }
    }
    at
}
/// The facets of `facets` that meet any of `members` at a corner, not among
/// them, in order of id: at a corner seen from above and at a height both
/// reach. Facets over each other only (a modeled thread's crests, a turn
/// apart) do not meet: taken as meeting, a thread's hundreds made every
/// click on it grow arcs for minutes.
fn neighbours(facets: &HashMap<usize, Facet>, at: &Corners, members: &[usize]) -> Vec<usize> {
    let mut out: Vec<usize> = (members.iter())
        .flat_map(|m| {
            let a = &facets[m];
            (a.xy.iter())
                .flat_map(|p| at.get(&key(*p)).into_iter().flatten())
                .filter(move |f| {
                    let b = &facets[*f];
                    b.low <= a.high + SAME as f32 && a.low <= b.high + SAME as f32
                })
        })
        .copied()
        .filter(|f| !members.contains(f))
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}
/// The round wall `seed` is a facet of, of `facets`: the seed and two
/// facets next to it whose corners fit a circle best, and on round while
/// the next facet's corners stay on the circle. None when no three do.
/// `at` holds the facets at each corner (`corners`).
fn grow(facets: &HashMap<usize, Facet>, at: &Corners, seed: usize) -> Option<Vec<usize>> {
    // Each three that fit, grown on round: the one that goes on round
    // farthest (a slot's short side and its ends' first facets fit a circle
    // too, the one the side is a chord of), the closest of those.
    let mut best: Option<(usize, f64, Vec<usize>)> = None;
    for a in neighbours(facets, at, &[seed]) {
        for b in neighbours(facets, at, &[seed, a]) {
            let three = vec![seed, a, b];
            let Some((_, r, off)) = fit(facets, &three) else {
                continue;
            };
            // `b` is next to `a` or to the seed.
            let next_to = |x: usize, y: usize| neighbours(facets, at, &[x]).contains(&y);
            let to = if next_to(a, b) { a } else { seed };
            if !inscribed(&facets[&seed], &facets[&a], r)
                || !inscribed(&facets[&to], &facets[&b], r)
            {
                continue;
            }
            let grown = grow_from(facets, at, three);
            if best
                .as_ref()
                .is_none_or(|(n, o, _)| grown.len() > *n || (grown.len() == *n && off < *o))
            {
                best = Some((grown.len(), off, grown));
            }
        }
    }
    best.map(|(_, _, members)| members)
}
/// `members` of `facets` (by corner `at`), on round from each facet taken
/// to the next it meets at a corner while their corners fit one circle
/// (`fit`), in order of id.
fn grow_from(facets: &HashMap<usize, Facet>, at: &Corners, mut members: Vec<usize>) -> Vec<usize> {
    let mut taken: HashSet<usize> = members.iter().copied().collect();
    let mut from = members.clone();
    while let Some(m) = from.pop() {
        for n in neighbours(facets, at, &[m]) {
            if taken.contains(&n) {
                continue;
            }
            members.push(n);
            if fit(facets, &members).is_some_and(|(_, r, _)| inscribed(&facets[&m], &facets[&n], r))
            {
                taken.insert(n);
                from.push(n);
            } else {
                members.pop();
            }
        }
    }
    members.sort_unstable();
    members
}
/// The arc `members` of `facets` make: its circle, how far round it goes,
/// its middle, its heights and sagitta. `NoArc::Outside` when it curves
/// round the part, `NoArc::Whole` when it goes all the way round.
fn arc(facets: &HashMap<usize, Facet>, members: Vec<usize>) -> Result<Arc, NoArc> {
    let (c, r, _) = fit(facets, &members).ok_or(NoArc::NotRound)?;
    let tau = std::f64::consts::TAU;
    // A hole's wall faces its centre: its normal, out of the part, points in.
    let inward = |f: &Facet| {
        let n = f.xy.len() as f64;
        let mid = (
            f.xy.iter().map(|p| p.0).sum::<f64>() / n,
            f.xy.iter().map(|p| p.1).sum::<f64>() / n,
        );
        f.normal[0] as f64 * (c[0] - mid.0) + f.normal[1] as f64 * (c[1] - mid.1) > 0.
    };
    let concave = members.iter().filter(|m| inward(&facets[m])).count();
    if concave * 2 <= members.len() {
        return Err(NoArc::Outside);
    }
    // Each facet's stretch of the round, then their union.
    let mut spans: Vec<(f64, f64)> = vec![];
    for f in members.iter().map(|m| &facets[m]) {
        let angle = |p: &(f64, f64)| (p.1 - c[1]).atan2(p.0 - c[0]);
        let first = angle(&f.xy[0]);
        let turned = |p: &(f64, f64)| {
            let d = angle(p) - first;
            d - tau * (d / tau).round()
        };
        let (lo, hi) = (f.xy.iter()).fold((0f64, 0f64), |(lo, hi), p| {
            (lo.min(turned(p)), hi.max(turned(p)))
        });
        spans.push(((first + lo).rem_euclid(tau), hi - lo));
    }
    spans.sort_by(|a, b| a.0.total_cmp(&b.0));
    // Merged, round the circle: each run's start and end.
    let mut runs: Vec<(f64, f64)> = vec![];
    for (start, length) in spans {
        match runs.last_mut() {
            Some(last) if start <= last.1 + 1e-6 => last.1 = last.1.max(start + length),
            _ => runs.push((start, start + length)),
        }
    }
    if runs.len() > 1 && runs[runs.len() - 1].1 >= runs[0].0 + tau - 1e-6 {
        let last = runs.pop().unwrap();
        runs[0] = (last.0 - tau, runs[0].1.max(last.1 - tau));
    }
    let covered: f64 = runs.iter().map(|(a, b)| b - a).sum();
    if covered >= tau - 1e-3 {
        return Err(NoArc::Whole);
    }
    // The widest gap, from a run's end to the next's start; the arc's
    // middle is across the round from its middle.
    let (mut gap, mut middle) = (-1f64, 0f64);
    for (i, run) in runs.iter().enumerate() {
        let next = runs[(i + 1) % runs.len()].0 + if i + 1 == runs.len() { tau } else { 0. };
        if next - run.1 > gap {
            gap = next - run.1;
            middle = (run.1 + next) / 2. + std::f64::consts::PI;
        }
    }
    let middle = (middle + std::f64::consts::PI).rem_euclid(tau) - std::f64::consts::PI;
    // The nearest a facet's chord comes to the centre.
    let mut inner = r;
    for f in members.iter().map(|m| &facets[m]) {
        for (i, a) in f.xy.iter().enumerate() {
            for b in &f.xy[i + 1..] {
                let d = (b.0 - a.0, b.1 - a.1);
                let q = (c[0] - a.0, c[1] - a.1);
                let t =
                    ((q.0 * d.0 + q.1 * d.1) / (d.0 * d.0 + d.1 * d.1).max(1e-12)).clamp(0., 1.);
                inner = inner.min((q.0 - t * d.0).hypot(q.1 - t * d.1));
            }
        }
    }
    let of = |m: &usize| &facets[m];
    Ok(Arc {
        centre: [c[0] as f32, c[1] as f32],
        r: r as f32,
        top: members
            .iter()
            .map(|m| of(m).high)
            .fold(f32::NEG_INFINITY, f32::max),
        bottom: members
            .iter()
            .map(|m| of(m).low)
            .fold(f32::INFINITY, f32::min),
        span: covered.to_degrees() as f32,
        middle: middle as f32,
        sagitta: (r - inner).max(0.) as f32,
        walls: members,
    })
}
/// Whether the arc `members` of `facets` (`at`: by corner), round `c` at
/// radius `r`, ends cleanly at both ends, as a circle's arc in a part does.
/// Past each end facet, each facet of the part's walls there turns from the
/// arc's tangent at that end (the end facet turned half its angle on):
/// - back from the way the arc bends by 10° or more (a corner: a half hole's
///   on the part's edge, a hole a slot breaks into);
/// - on the way it bends by 10° or more, and twice as far as the arc's
///   facets turn from each other or onto a flat at least three times as long
///   as the arc's middle facet (a corner too);
/// - or not at all (5 % of the end facet's angle), along a line touching
///   the circle (to 0.0002 mm and 0.002 % of `r`): a flat tangent to it,
///   however short (a slot's side, a pocket's wall past its corner's round).
///
/// Or there is none. A curve that goes on past it in facets like its own
/// (an ellipse, a spline: a patch of one fits a circle as closely as a
/// circle's facets do) is no circle's arc.
fn ends_cleanly(
    facets: &HashMap<usize, Facet>,
    at: &Corners,
    members: &[usize],
    c: [f64; 2],
    r: f64,
) -> bool {
    // How far the line along facet `f` (its two corners farthest apart)
    // passes from the centre.
    let line = |f: &Facet| {
        let mut ends = (f.xy[0], f.xy[1]);
        for (i, a) in f.xy.iter().enumerate() {
            for b in &f.xy[i + 1..] {
                if (b.0 - a.0).hypot(b.1 - a.1) > (ends.1.0 - ends.0.0).hypot(ends.1.1 - ends.0.1) {
                    ends = (*a, *b);
                }
            }
        }
        let (a, b) = ends;
        let d = (b.0 - a.0, b.1 - a.1);
        ((c[0] - a.0) * d.1 - (c[1] - a.1) * d.0).abs() / d.0.hypot(d.1).max(1e-12)
    };
    let mut chords: Vec<f64> = members.iter().map(|m| chord(&facets[m])).collect();
    chords.sort_by(f64::total_cmp);
    let middle = chords[chords.len() / 2];
    let set: HashSet<usize> = members.iter().copied().collect();
    for &end in members {
        let next = neighbours(facets, at, &[end]);
        let inside: Vec<usize> = next.iter().copied().filter(|n| set.contains(n)).collect();
        if inside.len() != 1 {
            continue;
        }
        let (e, before) = (&facets[&end], &facets[&inside[0]]);
        let last = turn(before, e);
        // Which way the arc bends, by its facets' normals.
        let bend =
            |a: &Facet, b: &Facet| (a.normal[0] * b.normal[1] - a.normal[1] * b.normal[0]) as f64;
        let way = bend(before, e).signum();
        // Half the end facet's angle: its turn from the arc's tangent there.
        let half = (chord(e) / (2. * r)).min(1.).asin();
        for n in next.iter().filter(|n| !set.contains(n)) {
            let f = &facets[n];
            let on = turn(e, f);
            // Its turn from the arc's tangent at the end: on round the way the
            // arc bends, or back.
            let onward = bend(e, f) * way >= 0.;
            let off = if onward { on - half } else { -on - half };
            let long = chord(f) >= 3. * middle;
            let corner = if onward {
                off >= 10f64.to_radians() && (on >= 2. * last || long)
            } else {
                -off >= 10f64.to_radians()
            };
            let flat =
                off.abs() <= 0.05 * 2. * half + 1e-3 && (line(f) - r).abs() <= 0.0002 + r * 2e-5;
            if !corner && !flat {
                return false;
            }
        }
    }
    true
}
/// Whether the facets `members` of `facets` go round a circle of radius
/// `r` as far as each other (2 %), but for two shorter ones (its ends'): a
/// CAD part's or a DXF's arc's facets do. Asked of an arc of four facets or
/// fewer, whose few corners fit a circle by chance on a curve that is none
/// (a coarse ellipse's); a longer one may be cut unevenly.
fn alike(facets: &HashMap<usize, Facet>, members: &[usize], r: f64) -> bool {
    let mut angles: Vec<f64> = (members.iter())
        .map(|m| 2. * (chord(&facets[m]) / (2. * r)).min(1.).asin())
        .collect();
    angles.sort_by(f64::total_cmp);
    let (median, widest) = (angles[angles.len() / 2], angles[angles.len() - 1]);
    let short = angles.iter().filter(|a| **a < 0.98 * median).count();
    widest <= 1.02 * median + 1e-4 && short <= 2
}
/// How far across the stock `setup` holds is seen from above, mm (the
/// part's box's, when the stock cannot be sized): no circle a Drill drills
/// is wider.
fn across(setup: &Setup, mesh: &Mesh, g: &Geometry) -> f32 {
    let b = crate::stock::resolve(setup, Some(mesh)).map_or(g.bounds, |r| r.stock);
    (b.max[0] - b.min[0]).hypot(b.max[1] - b.min[1])
}
/// The arc `members` of `facets` is (`arc`), unless its radius is more
/// than `across` (`across`: a nearly flat stretch of a wall), it is four
/// facets or fewer not alike (`alike`), or it does not
/// end cleanly (`ends_cleanly`, over `all`, the part's upright facets, by
/// corner `at`): then it is no circle's.
fn whole_arc(
    across: f32,
    facets: &HashMap<usize, Facet>,
    all: &HashMap<usize, Facet>,
    at: &Corners,
    members: Vec<usize>,
) -> Result<Arc, NoArc> {
    let a = arc(facets, members)?;
    if a.r > across || !(a.walls.len() > 4 || alike(facets, &a.walls, a.r as f64)) {
        return Err(NoArc::NotRound);
    }
    let c = [a.centre[0] as f64, a.centre[1] as f64];
    if !ends_cleanly(all, at, &a.walls, c, a.r as f64) {
        return Err(NoArc::NotRound);
    }
    Ok(a)
}
/// The arc of a round wall of the part that surface `id` is a facet of, as
/// `setup` places it: the facets on round from it on the circle it is on.
pub fn arc_at(setup: &Setup, mesh: &Mesh, id: usize) -> Result<Arc, NoArc> {
    let g = Geometry::new(setup, mesh);
    let facets = facets(&g, |_| true);
    if !facets.contains_key(&id) {
        return Err(NoArc::NotRound);
    }
    let at = corners(&facets);
    let members = grow(&facets, &at, id).ok_or(NoArc::NotRound)?;
    whole_arc(across(setup, mesh, &g), &facets, &facets, &at, members)
}
/// The arcs a Drill drills among the surfaces `ids` (its selection), as
/// `setup` places the part: each run of selected upright facets on one
/// circle that goes part of the way round it, round a hole, and is no part
/// of a whole round hole the selection picks by its walls (as a selected
/// part of a hole's walls is, or of a modeled thread's).
pub fn drilled_arcs(setup: &Setup, mesh: &Mesh, ids: &HashSet<usize>) -> Vec<Arc> {
    let g = Geometry::new(setup, mesh);
    let facets = facets(&g, |id| ids.contains(&id));
    if facets.len() < 3 {
        return vec![];
    }
    let at = corners(&facets);
    let wide = across(setup, mesh, &g);
    // The part's upright facets, for where a curve goes on past an arc.
    let mut all: Option<(HashMap<usize, Facet>, Corners)> = None;
    let mut seeds: Vec<usize> = facets.keys().copied().collect();
    seeds.sort_unstable();
    let (mut done, mut found) = (HashSet::new(), vec![]);
    for seed in seeds {
        if done.contains(&seed) {
            continue;
        }
        done.insert(seed);
        let Some(members) = grow(&facets, &at, seed) else {
            continue;
        };
        done.extend(members.iter().copied());
        if arc(&facets, members.clone()).is_err() {
            continue;
        }
        let (every, by_corner) = all.get_or_insert_with(|| {
            let every = self::facets(&g, |_| true);
            let by_corner = corners(&every);
            (every, by_corner)
        });
        if let Ok(a) = whole_arc(wide, &facets, every, by_corner, members) {
            found.push(a);
        }
    }
    if found.is_empty() {
        return found;
    }
    // A whole round hole the selection reads as, round the arc's centre,
    // that holds all the arc's walls is drilled as that hole: a modeled
    // thread's crests are many arcs round its axis, and a wall of a thread
    // past a leaning flank picks no hole by itself (`pocket::LEANING`), but
    // its picked hole's walls read as the hole together. So is a whole round
    // hole a wall of the arc picks (each such hole read once). One round
    // another centre (a slot so short it reads as round) leaves the arc its
    // own.
    let threaded = Geometry::with_threads(setup, mesh);
    let rims = super::rims(&threaded);
    let read: Vec<(HashSet<usize>, Hole)> = (crate::pocket::whole_holes(setup, ids, mesh))
        .into_iter()
        .filter_map(|h| {
            let round = super::drill_hole(&threaded, &h, &rims, false)?;
            Some((h.walls.iter().copied().collect(), round))
        })
        .collect();
    found.retain(|a| {
        !read.iter().any(|(walls, round)| {
            same_centre(round, a) && a.walls.iter().all(|w| walls.contains(w))
        })
    });
    let mut whole: HashSet<usize> = HashSet::new();
    found.retain(|a| {
        if a.walls.iter().any(|w| whole.contains(w)) {
            return false;
        }
        let Ok(walls) = crate::pocket::hole_walls(setup, mesh, a.walls[0]) else {
            return true;
        };
        let walls: HashSet<usize> = walls.into_iter().collect();
        let round = crate::pocket::holes(setup, &walls, mesh).is_ok_and(|holes| {
            (holes.iter())
                .filter_map(|h| super::drill_hole(&threaded, h, &rims, false))
                .any(|h| same_centre(&h, a))
        });
        if round {
            whole.extend(walls);
        }
        !round
    });
    found
}
/// Whether the centre of the arc `a` is over the stock `setup` holds
/// (within 0.01 mm), where a drill there cuts: one off it (the centre of
/// an arc of a large radius, or of a short one on the part's edge) would
/// go down beside the part, where it is held. True when the stock cannot
/// be sized.
pub fn on_stock(setup: &Setup, mesh: &Mesh, a: &Arc) -> bool {
    crate::stock::resolve(setup, Some(mesh)).map_or(true, |r| {
        (0..2).all(|k| a.centre[k] >= r.stock.min[k] - 0.01 && a.centre[k] <= r.stock.max[k] + 0.01)
    })
}
/// Whether the hole `h` is round the arc `a`'s centre (within 0.05 mm, as
/// two holes are one): a whole hole, the arc part of its wall.
pub fn same_centre(h: &Hole, a: &Arc) -> bool {
    (h.center[0] - a.centre[0]).hypot(h.center[1] - a.centre[1]) <= 0.05
}
/// Whether the arc `a` opens up in `g`, a Drill drilling it from above:
/// nothing of the part is over its top inside its circle near it (where
/// its floor is looked for, `arc_hole`), and a drill fits down it there.
/// Turned over, a blind arc's floor and the part past it are over it.
pub fn opens_up(g: &Geometry, a: &Arc) -> bool {
    let over = near_arc(a).iter().any(|p| {
        (g.triangles.iter()).any(|t| plane_height(&t.p, *p).is_some_and(|h| h > a.top + 0.05))
    });
    !over && arc_hole(g, a).is_some()
}
/// Points inside the circle of the arc `a`, at its middle and on either
/// side of it, between its facets and the chord across its ends (half way
/// from that chord to the arc; 0.8 of the radius out for an arc going half
/// round or more): where its
/// floor is, however little of the round the arc goes (a shallow scallop's
/// floor is a thin crescent).
fn near_arc(a: &Arc) -> [[f32; 2]; 3] {
    let half = (a.span.to_radians() / 2.).min(std::f32::consts::PI);
    [-0.6f32, 0., 0.6].map(|k| {
        let t = a.middle + k * half;
        // Half way out from the chord across the arc's ends at `t`, when
        // the arc goes less than half round; else 0.8 of the radius.
        let out = if half < std::f32::consts::FRAC_PI_2 {
            (a.r * half.cos() / (k * half).cos() + a.r) / 2.
        } else {
            0.8 * a.r
        };
        let out = out.min(a.r - 2. * a.sagitta - 0.001);
        [a.centre[0] + out * t.cos(), a.centre[1] + out * t.sin()]
    })
}
/// The hole a Drill drills for the arc `a` in `g`: its whole circle round
/// its centre, from its top down to the floor under it (the highest of the
/// part under it, inside the circle near the arc, below its top), or
/// through the part when nothing is under it; read as a hole's bore is
/// (`bore_at`), so that nothing of the part inside the circle over that
/// floor is cut. None when the part comes inside the circle over the floor
/// (a drill as wide as it would cut the part).
pub fn arc_hole(g: &Geometry, a: &Arc) -> Option<Hole> {
    let floor = (near_arc(a).into_iter())
        .flat_map(|p| {
            (g.triangles.iter())
                .filter_map(move |t| plane_height(&t.p, p))
                .filter(|h| *h < a.top - 0.01)
        })
        .fold(f32::NEG_INFINITY, f32::max);
    let bottom = if floor.is_finite() {
        floor
    } else {
        g.bounds.min[2]
    };
    let mut hole = bore_at(g, a.centre, a.r, a.sagitta, a.top, bottom, None)?;
    // The part inside the circle over the floor narrows the bore: a drill as
    // wide as the circle would cut it, and a narrower one at the centre
    // would not reach the arc.
    if hole.diameter < 2. * (a.r - a.sagitta) - 0.05 {
        return None;
    }
    hole.arc = Some(a.clone());
    Some(hole)
}
#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::{
        cam::{generate, holes, tests::fixture},
        mesh,
        model::Strategy,
        simulation::Motion,
        surface,
    };
    /// One code/value pair of DXF text.
    fn g(code: i32, value: impl std::fmt::Display) -> String {
        format!("{code:>3}\n{value}\n")
    }
    /// A millimetre DXF of closed polylines, each (x, y, bulge) corners.
    fn dxf(loops: &[&[(f64, f64, f64)]]) -> String {
        let mut d = g(0, "SECTION") + &g(2, "HEADER") + &g(9, "$INSUNITS") + &g(70, 4);
        d += &(g(0, "ENDSEC") + &g(0, "SECTION") + &g(2, "ENTITIES"));
        for corners in loops {
            d += &(g(0, "LWPOLYLINE") + &g(8, "0") + &g(90, corners.len()) + &g(70, 1));
            for &(x, y, b) in corners.iter() {
                d += &(g(10, x) + &g(20, y));
                if b != 0. {
                    d += &g(42, b);
                }
            }
        }
        d + &g(0, "ENDSEC") + &g(0, "EOF")
    }
    /// The solid of `loops` (`dxf`) `thickness` mm thick from Z `z`, less
    /// its top (`top` false) or its bottom (`bottom` false).
    fn slab(
        loops: &[&[(f64, f64, f64)]],
        thickness: f32,
        z: f32,
        top: bool,
        bottom: bool,
    ) -> Vec<[[f32; 3]; 3]> {
        let flat = crate::dxf::load(dxf(loops).as_bytes(), thickness as f64).unwrap();
        let low = (flat.triangles.iter().flatten())
            .map(|p| p[2])
            .fold(f32::INFINITY, f32::min);
        (flat.triangles.iter().zip(&flat.faces))
            .filter(|(_, f)| (top || **f != 1) && (bottom || **f != 2))
            .map(|(t, _)| t.map(|p| [p[0], p[1], p[2] - low + z]))
            .collect()
    }
    /// The front edge of a 40 x 20 block (X −20 to 20, Y 0 to 20), with a
    /// half hole of Ø10 round X0 Y0 in it, 10 mm high: through, or `depth`
    /// deep from its top to a half-round floor.
    pub(crate) fn notched_block(depth: Option<f32>) -> Vec<[[f32; 3]; 3]> {
        let notch: &[(f64, f64, f64)] = &[
            (-20., 0., 0.),
            (-5., 0., -1.),
            (5., 0., 0.),
            (20., 0., 0.),
            (20., 20., 0.),
            (-20., 20., 0.),
        ];
        let Some(depth) = depth else {
            return slab(&[notch], 10., 0., true, true);
        };
        let block: &[(f64, f64, f64)] = &[
            (-20., 0., 0.),
            (20., 0., 0.),
            (20., 20., 0.),
            (-20., 20., 0.),
        ];
        let floor = 10. - depth;
        let mut tris = slab(&[notch], depth, floor, true, false);
        tris.extend(slab(&[block], floor, 0., false, true));
        // The half-round floor, its edge the notch's 36 facets'.
        for k in 0..36 {
            let at = |k: usize| {
                let t = k as f32 * std::f32::consts::PI / 36.;
                [5. * t.cos(), 5. * t.sin(), floor]
            };
            tris.push([[0., 0., floor], at(k), at(k + 1)]);
        }
        tris
    }
    /// A 40 x 30 plate 8 mm thick, X −20 to 20 and Y −15 to 15, with a slot
    /// through it 6 mm wide, its round ends' centres at X ±6 Y0.
    fn slotted_plate() -> Vec<[[f32; 3]; 3]> {
        slab(
            &[
                &[
                    (-20., -15., 0.),
                    (20., -15., 0.),
                    (20., 15., 0.),
                    (-20., 15., 0.),
                ],
                &[(-6., -3., 0.), (6., -3., 1.), (6., 3., 0.), (-6., 3., 1.)],
            ],
            8.,
            0.,
            true,
            true,
        )
    }
    /// A 40 x 30 plate 8 mm thick with a slot through it 10 mm wide, its
    /// R5 ends round X±`length`/2 Y0.
    pub(crate) fn slot(length: f64) -> Vec<[[f32; 3]; 3]> {
        let h = length / 2.;
        plate_with(&[(-h, -5., 0.), (h, -5., 1.), (h, 5., 0.), (-h, 5., 1.)])
    }
    /// The upright surfaces of `mesh` whose centres, in `setup`'s frame
    /// (mm), `at` holds.
    fn walls_where(setup: &Setup, mesh: &Mesh, at: impl Fn([f32; 2]) -> bool) -> Vec<usize> {
        let g = Geometry::new(setup, mesh);
        let mut found: Vec<usize> = facets(&g, |_| true)
            .into_iter()
            .filter(|(_, f)| {
                let n = f.xy.len() as f64;
                let mid = [
                    (f.xy.iter().map(|p| p.0).sum::<f64>() / n) as f32,
                    (f.xy.iter().map(|p| p.1).sum::<f64>() / n) as f32,
                ];
                at(mid)
            })
            .map(|(id, _)| id)
            .collect();
        found.sort_unstable();
        found
    }
    /// `op` with the surfaces `ids` selected.
    fn picking(op: &mut crate::model::Operation, mesh: &Mesh, ids: &[usize]) {
        op.surfaces = Some(surface::Selection {
            mesh_fingerprint: mesh.fingerprint.clone(),
            ids: ids.to_vec(),
        });
    }
    #[test]
    fn a_half_hole_on_the_parts_edge_is_drilled_round_its_centre_as_wide_as_its_circle() {
        let mesh = mesh::normalize(notched_block(None)).unwrap();
        let (setup, mut op, mut tool) = fixture(Strategy::Drill);
        let g = Geometry::new(&setup, &mesh);
        // The part is centred: the block's front edge is at Y −10.
        let centre = [0f32, g.bounds.min[1]];
        let on_arc = |c: [f32; 2]| ((c[0] - centre[0]).hypot(c[1] - centre[1]) - 5.).abs() < 0.1;
        let notch = walls_where(&setup, &mesh, on_arc);
        assert_eq!(notch.len(), 36, "{notch:?}");
        // A click on any of its facets picks the whole half round.
        for &id in &notch {
            let a = arc_at(&setup, &mesh, id).unwrap();
            assert_eq!(a.walls, notch);
            assert!(
                (a.r - 5.).abs() < 0.001 && (a.span - 180.).abs() < 0.01,
                "{a:?}"
            );
            assert!((a.centre[0] - centre[0]).abs() < 0.001, "{a:?}");
            assert!((a.centre[1] - centre[1]).abs() < 0.001, "{a:?}");
            // Its middle is into the part, +Y.
            assert!(
                (a.middle - std::f32::consts::FRAC_PI_2).abs() < 0.01,
                "{a:?}"
            );
        }
        // The flat front wall beside it is none.
        let front = walls_where(&setup, &mesh, |c| (c[1] - centre[1]).abs() < 0.01);
        assert_eq!(front.len(), 2);
        for id in front {
            assert_eq!(arc_at(&setup, &mesh, id), Err(NoArc::NotRound));
        }
        // Picked, it is a Ø10 hole through the part round that centre: no
        // hole picked by its walls, which would refuse it.
        picking(&mut op, &mesh, &notch);
        assert_eq!(
            crate::pocket::picked_holes(&setup, &op, &mesh)
                .unwrap()
                .len(),
            0
        );
        let found = holes(&setup, &op, &mesh).unwrap();
        assert_eq!(found.len(), 1, "{found:?}");
        let h = &found[0];
        assert!(h.through && (h.diameter - 10.).abs() < 0.01, "{h:?}");
        assert!((h.top - g.bounds.max[2]).abs() < 0.01, "{h:?}");
        assert!((h.bottom - g.bounds.min[2]).abs() < 0.01, "{h:?}");
        assert!((h.center[0] - centre[0]).abs() < 0.001 && (h.center[1] - centre[1]).abs() < 0.001);
        assert_eq!(h.arc.as_ref().map(|a| a.walls.clone()), Some(notch));
        assert_eq!(crate::cam::largest_drill(h), 10.);
        // A Ø10 drill plunges at its centre down through the part.
        tool.p.insert("diameter".into(), "10".into());
        let plan = generate(&setup, &op, &tool, &mesh).unwrap();
        let plunges: Vec<[f32; 3]> = (plan.moves.iter())
            .filter(|m| m.motion == Motion::Feed)
            .map(|m| m.to)
            .collect();
        assert!(!plunges.is_empty());
        for p in &plunges {
            assert!(
                (p[0] - centre[0]).abs() < 0.001 && (p[1] - centre[1]).abs() < 0.001,
                "{p:?}"
            );
        }
        let lowest = plunges.iter().map(|p| p[2]).fold(f32::INFINITY, f32::min);
        assert!(lowest <= g.bounds.min[2] + 0.001, "{lowest}");
    }
    #[test]
    fn a_blind_half_hole_is_drilled_down_to_its_floor() {
        let mesh = mesh::normalize(notched_block(Some(6.))).unwrap();
        let (setup, mut op, _) = fixture(Strategy::Drill);
        let g = Geometry::new(&setup, &mesh);
        let centre = [0f32, g.bounds.min[1]];
        let notch = walls_where(&setup, &mesh, |c| {
            ((c[0] - centre[0]).hypot(c[1] - centre[1]) - 5.).abs() < 0.1
        });
        let a = arc_at(&setup, &mesh, notch[0]).unwrap();
        assert_eq!(a.walls, notch);
        picking(&mut op, &mesh, &a.walls);
        let found = holes(&setup, &op, &mesh).unwrap();
        assert_eq!(found.len(), 1, "{found:?}");
        let h = &found[0];
        assert!(!h.through && (h.diameter - 10.).abs() < 0.01, "{h:?}");
        assert!((h.top - h.bottom - 6.).abs() < 0.01, "{h:?}");
        assert!((h.bottom - (g.bounds.min[2] + 4.)).abs() < 0.01, "{h:?}");
    }
    #[test]
    fn a_slots_round_end_is_drilled_as_its_circle_though_the_slot_is_not_round() {
        let mesh = mesh::normalize(slotted_plate()).unwrap();
        let (setup, mut op, _) = fixture(Strategy::Drill);
        // The right end's half round, round X6 Y0.
        let end = walls_where(&setup, &mesh, |c| {
            c[0] > 6.01 && (c[0] - 6.).hypot(c[1]) < 3.1
        });
        assert!(end.len() >= 8, "{end:?}");
        // The slot its walls pick is no round hole.
        let slot = crate::pocket::hole_walls(&setup, &mesh, end[0]).unwrap();
        let slot: HashSet<usize> = slot.into_iter().collect();
        let picked = crate::pocket::holes(&setup, &slot, &mesh).unwrap();
        assert!(crate::cam::drill_holes_of(&setup, &mesh, &picked)[0].is_none());
        let a = arc_at(&setup, &mesh, end[0]).unwrap();
        assert_eq!(a.walls, end);
        assert!(
            (a.r - 3.).abs() < 0.001 && (a.span - 180.).abs() < 0.01,
            "{a:?}"
        );
        assert!(
            (a.centre[0] - 6.).abs() < 0.001 && a.centre[1].abs() < 0.001,
            "{a:?}"
        );
        picking(&mut op, &mesh, &a.walls);
        let found = holes(&setup, &op, &mesh).unwrap();
        assert_eq!(found.len(), 1, "{found:?}");
        let h = &found[0];
        assert!(h.through && (h.diameter - 6.).abs() < 0.01, "{h:?}");
        assert!(
            (h.center[0] - 6.).abs() < 0.001 && h.center[1].abs() < 0.001,
            "{h:?}"
        );
    }
    #[test]
    fn a_bosss_wall_is_no_arc_and_a_whole_bores_walls_are_its_hole() {
        // A Ø24 disc with a Ø8 bore 8 mm deep (`cam::tests::cup`, on a base).
        let section = [
            (0., 0.),
            (12., 0.),
            (12., 10.),
            (4., 10.),
            (4., 2.),
            (0., 2.),
        ];
        let mesh = mesh::normalize(crate::cam::tests::revolved(&section)).unwrap();
        let (setup, mut op, _) = fixture(Strategy::Drill);
        let outside = walls_where(&setup, &mesh, |c| c[0].hypot(c[1]) > 11.);
        let bore = walls_where(&setup, &mesh, |c| c[0].hypot(c[1]) < 4.1);
        assert_eq!((outside.len(), bore.len()), (64, 64));
        assert_eq!(arc_at(&setup, &mesh, outside[0]), Err(NoArc::Outside));
        assert_eq!(arc_at(&setup, &mesh, bore[0]), Err(NoArc::Whole));
        // Picked by its walls, the bore is drilled as before: whole, no arc.
        picking(&mut op, &mesh, &bore);
        assert!(drilled_arcs(&setup, &mesh, &bore.iter().copied().collect()).is_empty());
        let found = holes(&setup, &op, &mesh).unwrap();
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].arc.is_none() && (found[0].diameter - 8.).abs() < 0.02);
        // Half its walls are part of that whole hole: refused as before, as
        // picked only in part, not drilled as an arc.
        let half: Vec<usize> = (bore.iter().copied())
            .filter(|&id| {
                let g = Geometry::new(&setup, &mesh);
                (g.triangles.iter())
                    .filter(|t| t.feature == Some(id))
                    .all(|t| t.p.iter().all(|p| p[1] >= -1e-4))
            })
            .collect();
        assert!(half.len() >= 30 && half.len() < 64, "{}", half.len());
        assert!(drilled_arcs(&setup, &mesh, &half.iter().copied().collect()).is_empty());
        picking(&mut op, &mesh, &half);
        assert!(holes(&setup, &op, &mesh).is_err());
    }
    /// A 40 x 30 plate 8 mm thick with an opening through it, its corners
    /// `inner`.
    pub(crate) fn plate_with(inner: &[(f64, f64, f64)]) -> Vec<[[f32; 3]; 3]> {
        slab(
            &[
                &[
                    (-20., -15., 0.),
                    (20., -15., 0.),
                    (20., 15., 0.),
                    (-20., 15., 0.),
                ],
                inner,
            ],
            8.,
            0.,
            true,
            true,
        )
    }
    #[test]
    fn no_wall_of_a_curve_that_is_no_circle_is_an_arc_however_it_is_faceted() {
        // Openings round ellipses and superellipses (|x/a|^p + |y/b|^p = 1,
        // p 2, 2.5 and 4) of five sizes, nearly round too, their facets
        // spaced evenly or unevenly three ways, and a parabola, of 12 to 256
        // facets: a patch of each fits a circle as closely as a circle's
        // facets do, but none ends cleanly, so no wall of them picks an arc
        // and no hole is invented (the reviewer's probes, rounds 1 to 3).
        let mut picked = vec![];
        let mut check = |what: String, inner: Vec<(f64, f64, f64)>| {
            let mesh = mesh::normalize(plate_with(&inner)).unwrap();
            let (setup, _, _) = fixture(Strategy::Drill);
            for id in walls_where(&setup, &mesh, |c| c[0].abs() < 14.5 && c[1].abs() < 11.) {
                if let Ok(a) = arc_at(&setup, &mesh, id) {
                    picked.push(format!("{what}: {:.0}° r{:.2}", a.span, a.r));
                }
            }
        };
        for n in [12usize, 16, 24, 32, 48, 64, 96, 128, 256] {
            for (a, b) in [(8., 4.), (10., 9.5), (14., 5.), (6., 5.), (9., 3.)] {
                for p in [2., 2.5, 4.] {
                    for (k, waves) in [(0., 0.), (0.22, 3.), (0.15, 5.), (0.08, 7.)] {
                        let inner = (0..n)
                            .map(|i| {
                                let t = i as f64 * std::f64::consts::TAU / n as f64;
                                let t = t + k * (waves * t).sin();
                                let along =
                                    |v: f64, size: f64| size * v.signum() * v.abs().powf(2. / p);
                                (along(t.cos(), a), along(t.sin(), b), 0.)
                            })
                            .collect();
                        check(format!("{a}x{b} p{p} uneven {k} of {n}"), inner);
                    }
                }
            }
            let mut parabola: Vec<_> = (0..=n)
                .map(|i| {
                    let x = -8. + 16. * i as f64 / n as f64;
                    (x, 0.125 * x * x - 4., 0.)
                })
                .collect();
            parabola.extend([(8., 10., 0.), (-8., 10., 0.)]);
            check(format!("parabola of {n}"), parabola);
        }
        assert!(picked.is_empty(), "{} picked: {picked:?}", picked.len());
    }
    #[test]
    fn a_short_arc_of_a_circle_meeting_flats_at_corners_is_picked_from_any_facet() {
        // R5 arcs of 30° to 180° round X0 Y0, in 6 to 24 facets of up to
        // 30°, closed by two flats to X−12 Y±10.
        for span in [30f64, 45., 60., 90., 180.] {
            for n in [3usize, 6, 12, 24] {
                if span / n as f64 > 30. || (span >= 90. && n < 6) {
                    continue;
                }
                let start = -span.to_radians() / 2.;
                let mut inner: Vec<(f64, f64, f64)> = (0..=n)
                    .map(|k| {
                        let t = start + span.to_radians() * k as f64 / n as f64;
                        (5. * t.cos(), 5. * t.sin(), 0.)
                    })
                    .collect();
                inner.extend([(-12., 10., 0.), (-12., -10., 0.)]);
                let mesh = mesh::normalize(plate_with(&inner)).unwrap();
                let (setup, _, _) = fixture(Strategy::Drill);
                let g = Geometry::new(&setup, &mesh);
                // The part is centred: X0 Y0 is at the middle of its box.
                let mid = [
                    (g.bounds.min[0] + g.bounds.max[0]) / 2.,
                    (g.bounds.min[1] + g.bounds.max[1]) / 2.,
                ];
                let walls = walls_where(&setup, &mesh, |c| {
                    ((c[0] - mid[0]).hypot(c[1] - mid[1]) - 5.).abs() < 0.2 && c[0] - mid[0] > 0.
                });
                assert_eq!(walls.len(), n, "{span}° in {n}");
                for id in walls {
                    let a = arc_at(&setup, &mesh, id)
                        .unwrap_or_else(|e| panic!("{span}° in {n}: {e:?}"));
                    assert!(
                        (a.r - 5.).abs() < 0.01 && (a.span - span as f32).abs() < 0.1,
                        "{span}° in {n}: {a:?}"
                    );
                }
            }
        }
    }
    #[test]
    fn a_rounded_openings_corners_are_quarter_circle_arcs() {
        // A 20 x 10 opening with R3 corners round X±7 Y±2.
        let q = 22.5f64.to_radians().tan();
        let mesh = mesh::normalize(plate_with(&[
            (-7., -5., 0.),
            (7., -5., q),
            (10., -2., 0.),
            (10., 2., q),
            (7., 5., 0.),
            (-7., 5., q),
            (-10., 2., 0.),
            (-10., -2., q),
        ]))
        .unwrap();
        let (setup, mut op, _) = fixture(Strategy::Drill);
        for (x, y) in [(7f32, -2f32), (7., 2.), (-7., 2.), (-7., -2.)] {
            let corner = walls_where(&setup, &mesh, |c| {
                (c[0] - x) * x > 0.
                    && (c[1] - y) * y > 0.
                    && ((c[0] - x).hypot(c[1] - y) - 3.).abs() < 0.1
            });
            assert!(corner.len() >= 4, "{corner:?}");
            let a = arc_at(&setup, &mesh, corner[0]).unwrap();
            assert_eq!(a.walls, corner);
            assert!(
                (a.r - 3.).abs() < 0.001 && (a.span - 90.).abs() < 0.01,
                "{a:?}"
            );
            assert!(
                (a.centre[0] - x).abs() < 0.001 && (a.centre[1] - y).abs() < 0.001,
                "{a:?}"
            );
            picking(&mut op, &mesh, &a.walls);
            let found = holes(&setup, &op, &mesh).unwrap();
            assert_eq!(found.len(), 1, "{found:?}");
            assert!(
                (found[0].diameter - 6.).abs() < 0.01 && found[0].through,
                "{found:?}"
            );
        }
        // Its straight sides are none.
        let side = walls_where(&setup, &mesh, |c| {
            (c[1].abs() - 5.).abs() < 0.1 && c[0].abs() < 1.
        });
        assert_eq!(side.len(), 2);
        for id in side {
            assert_eq!(arc_at(&setup, &mesh, id), Err(NoArc::NotRound));
        }
    }
    #[test]
    fn the_example_blocks_round_pocket_corners_are_quarter_circle_arcs() {
        // The four-setup block's STEP: its pockets' R4 and R5 corners, each
        // a CAD cylinder face, read as 90° arcs round their centres.
        if crate::step::skip_without_helper() {
            return;
        }
        let mesh = mesh::load(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("examples/four-setup-block.step"),
        )
        .unwrap();
        let (setup, _, _) = fixture(Strategy::Drill);
        let mut corners: Vec<Arc> = vec![];
        for id in walls_where(&setup, &mesh, |_| true) {
            if let Ok(a) = arc_at(&setup, &mesh, id)
                && !corners.iter().any(|c| c.walls == a.walls)
            {
                corners.push(a);
            }
        }
        let quarter = |r: f32| {
            (corners.iter())
                .filter(|a| (a.r - r).abs() < 0.001 && (a.span - 90.).abs() < 0.05)
                .count()
        };
        assert!(quarter(4.) >= 4 && quarter(5.) >= 4, "{corners:?}");
    }
    #[test]
    fn a_short_slots_round_ends_are_picked_however_short_its_sides() {
        // Slots 10 mm wide with R5 ends round X±L/2 Y0, their straight sides
        // L long, down to 0.25 mm (the reviewer's 1 mm and 0.25 mm slots,
        // rounds 3 and 4): each end is a 180° arc from any of its facets,
        // though the slot reads as a whole round hole round X0 (to 2 % of
        // its radius), since its ends are round other centres. (Shorter
        // than 0.1 mm, its ends' centres are within 0.05 mm of its middle,
        // where two holes are one: a click picks the whole hole.)
        for length in [4f64, 1., 0.5, 0.25] {
            let h = length / 2.;
            let mesh = mesh::normalize(slot(length)).unwrap();
            let (setup, mut op, _) = fixture(Strategy::Drill);
            for side in [-1f32, 1.] {
                let x = side * h as f32;
                let end = walls_where(&setup, &mesh, |c| {
                    (c[0] - x) * side > 0. && ((c[0] - x).hypot(c[1]) - 5.).abs() < 0.1
                });
                assert!(end.len() >= 8, "{length}: {end:?}");
                for &id in &end {
                    let a = arc_at(&setup, &mesh, id).unwrap_or_else(|e| panic!("{length}: {e:?}"));
                    assert_eq!(a.walls, end, "{length}");
                    assert!(
                        (a.r - 5.).abs() < 0.001 && (a.span - 180.).abs() < 0.01,
                        "{a:?}"
                    );
                    assert!(
                        (a.centre[0] - x).abs() < 0.001 && a.centre[1].abs() < 0.001,
                        "{a:?}"
                    );
                }
                picking(&mut op, &mesh, &end);
                let found = holes(&setup, &op, &mesh).unwrap();
                assert_eq!(found.len(), 1, "{found:?}");
                assert!(
                    (found[0].center[0] - x).abs() < 0.001 && found[0].through,
                    "{found:?}"
                );
            }
        }
    }
    #[test]
    fn a_finely_faceted_half_circle_is_picked_from_any_facet() {
        // An R5 half round on the edge of a plate's opening, in 36 to 720
        // facets (a degree to a quarter of one each), closed by a flat: its
        // corners square (the reviewer's 360 and 450 facets, round 4).
        for n in [36usize, 180, 360, 450, 720] {
            let mut inner: Vec<(f64, f64, f64)> = (0..=n)
                .map(|k| {
                    let t =
                        -std::f64::consts::FRAC_PI_2 + std::f64::consts::PI * k as f64 / n as f64;
                    (5. * t.cos(), 5. * t.sin(), 0.)
                })
                .collect();
            inner.extend([(-6., 5., 0.), (-6., -5., 0.)]);
            let mesh = mesh::normalize(plate_with(&inner)).unwrap();
            let (setup, mut op, _) = fixture(Strategy::Drill);
            let g = Geometry::new(&setup, &mesh);
            let mid = [
                (g.bounds.min[0] + g.bounds.max[0]) / 2.,
                (g.bounds.min[1] + g.bounds.max[1]) / 2.,
            ];
            let arc = walls_where(&setup, &mesh, |c| {
                ((c[0] - mid[0]).hypot(c[1] - mid[1]) - 5.).abs() < 0.1 && c[0] - mid[0] > 0.
            });
            assert_eq!(arc.len(), n);
            for &id in arc.iter().step_by(n / 36) {
                let a = arc_at(&setup, &mesh, id).unwrap_or_else(|e| panic!("{n}: {id} {e:?}"));
                assert_eq!(a.walls, arc, "{n}");
                assert!(
                    (a.r - 5.).abs() < 0.001 && (a.span - 180.).abs() < 0.01,
                    "{a:?}"
                );
            }
            picking(&mut op, &mesh, &arc);
            let found = holes(&setup, &op, &mesh).unwrap();
            assert_eq!(found.len(), 1, "{found:?}");
            assert!((found[0].diameter - 10.).abs() < 0.01, "{found:?}");
        }
    }
    #[test]
    fn a_shallow_arc_on_a_small_part_is_picked_in_its_stock() {
        // A 1.1 x 3 part 4 mm thick whose -X side is an R5 arc of 20° round
        // a centre 5.5 mm off it, in a 20 x 10 stock: wider round than the
        // part is across, but not than the stock (the reviewer's, round 5).
        let mut edge: Vec<(f64, f64, f64)> = (0..=20)
            .map(|k| {
                let t = (-10. + k as f64).to_radians();
                (5. * t.cos(), 5. * t.sin(), 0.)
            })
            .collect();
        edge.extend([(6., 1.5, 0.), (6., -1.5, 0.)]);
        let mesh = mesh::normalize(slab(&[&edge], 4., 0., true, true)).unwrap();
        let (mut setup, mut op, mut tool) = fixture(Strategy::Drill);
        for (k, v) in [
            ("stock_type", "Fixed size box"),
            ("stock_x", "20"),
            ("stock_y", "10"),
            ("stock_z", "6"),
        ] {
            setup.p.insert(k.into(), v.into());
        }
        let wall = walls_where(&setup, &mesh, |c| c[0] < -0.4 && c[1].abs() < 0.9);
        assert_eq!(wall.len(), 20);
        let a = arc_at(&setup, &mesh, wall[0]).unwrap();
        assert_eq!(a.walls, wall);
        assert!(
            (a.r - 5.).abs() < 0.001 && (a.span - 20.).abs() < 0.01,
            "{a:?}"
        );
        assert!(
            (a.centre[0] + 5.462).abs() < 0.002 && a.centre[1].abs() < 0.001,
            "{a:?}"
        );
        assert!(on_stock(&setup, &mesh, &a));
        picking(&mut op, &mesh, &a.walls);
        let found = holes(&setup, &op, &mesh).unwrap();
        assert_eq!(found.len(), 1, "{found:?}");
        tool.p.insert("diameter".into(), "10".into());
        let plan = generate(&setup, &op, &tool, &mesh).unwrap();
        assert!(
            (plan.moves.iter())
                .filter(|m| m.motion == Motion::Feed)
                .all(|m| (m.to[0] - a.centre[0]).abs() < 0.001 && m.to[1].abs() < 0.001)
        );
    }
    /// A 7 mm wide bar from X4.33 to X6, 10 mm high, whose -X side is an R5
    /// scallop of 60° round X0 Y0 (in 60 facets): through, or 6 mm deep
    /// from its top to a floor, the crescent between the arc and its chord
    /// (the reviewer's, stage 2).
    fn scallop(blind: bool) -> Vec<[[f32; 3]; 3]> {
        let arc: Vec<(f64, f64, f64)> = (0..=60)
            .map(|k| {
                let t = (-30. + k as f64).to_radians();
                (5. * t.cos(), 5. * t.sin(), 0.)
            })
            .collect();
        let x = 5. * 30f64.to_radians().cos();
        let mut edge = arc.clone();
        edge.extend([(x, 3.5, 0.), (6., 3.5, 0.), (6., -3.5, 0.), (x, -3.5, 0.)]);
        if !blind {
            return slab(&[&edge], 10., 0., true, true);
        }
        let base = [(x, -3.5, 0.), (6., -3.5, 0.), (6., 3.5, 0.), (x, 3.5, 0.)];
        let mut tris = slab(&[&edge], 6., 4., true, false);
        tris.extend(slab(&[&base], 4., 0., false, true));
        tris.extend(
            (slab(&[&arc], 4., 0., true, false).into_iter())
                .filter(|t| t.iter().all(|p| (p[2] - 4.).abs() < 1e-5)),
        );
        tris
    }
    #[test]
    fn a_shallow_scallops_floor_is_found_however_thin_its_crescent() {
        for blind in [true, false] {
            let mesh = mesh::normalize(scallop(blind)).unwrap();
            let (mut setup, mut op, mut tool) = fixture(Strategy::Drill);
            for (k, v) in [
                ("stock_type", "Fixed size box"),
                ("stock_x", "20"),
                ("stock_y", "10"),
                ("stock_z", "20"),
            ] {
                setup.p.insert(k.into(), v.into());
            }
            let g = Geometry::new(&setup, &mesh);
            let wall = walls_where(&setup, &mesh, |c| {
                (c[0] - g.bounds.min[0] - 0.6).abs() < 0.2 && c[1].abs() < 0.9
            });
            let a = arc_at(&setup, &mesh, wall[0]).unwrap();
            assert!(
                (a.r - 5.).abs() < 0.001 && (a.span - 60.).abs() < 0.01,
                "{a:?}"
            );
            picking(&mut op, &mesh, &a.walls);
            let found = holes(&setup, &op, &mesh).unwrap();
            assert_eq!(found.len(), 1, "{found:?}");
            let h = &found[0];
            assert!(
                (h.diameter - 10.).abs() < 0.01 && h.through != blind,
                "{blind}: {h:?}"
            );
            let depth = if blind { 6. } else { 10. };
            assert!((h.top - h.bottom - depth).abs() < 0.01, "{blind}: {h:?}");
            tool.p.insert("diameter".into(), "10".into());
            let plan = generate(&setup, &op, &tool, &mesh).unwrap();
            let lowest = (plan.moves.iter())
                .filter(|m| m.motion == Motion::Feed)
                .map(|m| m.to[2])
                .fold(f32::INFINITY, f32::min);
            if blind {
                assert!((lowest - h.bottom).abs() < 0.01, "{lowest} {h:?}");
            }
        }
    }
    #[test]
    fn a_circle_cut_unevenly_is_an_arc_from_any_facet() {
        // An R5 half round in facets of 4° and 6° by turns (the reviewer's,
        // stage 2): its chords are all of one circle.
        let mut inner = vec![];
        let mut degrees = -90f64;
        let at = |d: f64| (5. * d.to_radians().cos(), 5. * d.to_radians().sin(), 0.);
        inner.push(at(degrees));
        for i in 0..36 {
            degrees += if i % 2 == 0 { 4. } else { 6. };
            inner.push(at(degrees));
        }
        inner.extend([(-6., 5., 0.), (-6., -5., 0.)]);
        let mesh = mesh::normalize(plate_with(&inner)).unwrap();
        let (setup, _, _) = fixture(Strategy::Drill);
        let g = Geometry::new(&setup, &mesh);
        let mid = [
            (g.bounds.min[0] + g.bounds.max[0]) / 2.,
            (g.bounds.min[1] + g.bounds.max[1]) / 2.,
        ];
        let wall = walls_where(&setup, &mesh, |c| {
            ((c[0] - mid[0]).hypot(c[1] - mid[1]) - 5.).abs() < 0.1 && c[0] - mid[0] > 0.
        });
        assert_eq!(wall.len(), 36);
        for &id in &wall {
            let a = arc_at(&setup, &mesh, id).unwrap_or_else(|e| panic!("{id}: {e:?}"));
            assert_eq!(a.walls, wall);
            assert!(
                (a.r - 5.).abs() < 0.001 && (a.span - 180.).abs() < 0.01,
                "{a:?}"
            );
        }
    }
    #[test]
    fn a_patch_of_a_coarse_ellipse_is_no_hole_and_a_small_drill_is_refused_on_an_arc() {
        // An 18 x 6 ellipse in 10 uneven facets (the reviewer's, stage 2,
        // round 2): three of its facets fit an R21 circle round a centre off
        // the opening. The part comes inside that circle, so no hole is read
        // there: no drill as wide as it goes down, and a narrower one at its
        // centre would cut the stock beside the part.
        let mesh = mesh::normalize(plate_with(&[
            (8.98314391805965, -0.1835227347187729, 0.),
            (7.5123225767760395, 1.6521033564137095, 0.),
            (2.469909175663699, 2.884817511347069, 0.),
            (-2.7152041230991566, 2.8602188449663415, 0.),
            (-7.572124048518679, 1.6214855121100489, 0.),
            (-8.999888200921667, -0.014953152439612531, 0.),
            (-7.0890800692943, -1.8482707765528754, 0.),
            (-2.5067506178780103, -2.881284461635667, 0.),
            (2.9208423975826814, -2.837617930755056, 0.),
            (7.471317446788217, -1.6726364953210178, 0.),
        ]))
        .unwrap();
        let (setup, mut op, _) = fixture(Strategy::Drill);
        let g = Geometry::new(&setup, &mesh);
        let mut patches = 0;
        for id in walls_where(&setup, &mesh, |c| c[0].abs() < 9.5 && c[1].abs() < 3.5) {
            if let Ok(a) = arc_at(&setup, &mesh, id) {
                patches += 1;
                assert!(arc_hole(&g, &a).is_none(), "{id}: {a:?}");
                picking(&mut op, &mesh, &a.walls);
                assert!(holes(&setup, &op, &mesh).is_err(), "{id}");
            }
        }
        // Its three facets read as an arc (a mesh's limit), but no hole.
        assert_eq!(patches, 3);
        // On a half hole, a drill narrower than its circle is refused.
        let mesh = mesh::normalize(notched_block(None)).unwrap();
        let (setup, mut op, mut tool) = fixture(Strategy::Drill);
        let g = Geometry::new(&setup, &mesh);
        let wall = walls_where(&setup, &mesh, |c| {
            (c[0].hypot(c[1] - g.bounds.min[1]) - 5.).abs() < 0.1
        });
        picking(&mut op, &mesh, &wall);
        tool.p.insert("diameter".into(), "6".into());
        let refused = generate(&setup, &op, &tool, &mesh).unwrap_err();
        assert!(
            refused.starts_with("The Ø6.00 drill at the centre of the 180° arc"),
            "{refused}"
        );
        tool.p.insert("diameter".into(), "9.9".into());
        assert!(generate(&setup, &op, &tool, &mesh).is_ok());
    }
}
