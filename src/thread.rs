//! Threads modeled in an imported part, found from its triangles alone, and
//! the standard sizes they are matched to.
//!
//! A modeled thread's flanks lean a fixed angle from the plane across its
//! axis (30° for a 60° thread, 27.5° for a 55° one), and the line of each
//! flank triangle's normal, seen along the axis, runs through the axis. The
//! triangles that agree on an axis are one thread's; their vertices, unwound
//! round it, line up on one helix for one pitch and hand only, which reads
//! the pitch and the hand off the table's sizes. A plain hole, a chamfer or
//! a countersink has no flank at that angle, or no helix in its vertices.
use crate::{
    mesh::{Mesh, rotation},
    model::Setup,
    stock::Placement,
};
use std::collections::HashMap;

/// A standard thread size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spec {
    pub name: &'static str,
    /// Major diameter, mm; a tapered thread's at its gauge plane.
    pub major: f32,
    pub pitch: f32,
    /// Included flank angle, degrees.
    pub angle: f32,
    /// Tapered 1:16 on the diameter (pipe threads).
    pub tapered: bool,
    /// The drill a tap of this size goes into, mm.
    pub tap_drill: f32,
}
impl Spec {
    /// The internal thread's minor diameter, the bore a thread mill goes
    /// down; for a tapered thread, at its gauge plane.
    pub fn minor(&self) -> f32 {
        self.major - 2. * depth(self.pitch, self.angle)
    }
}
/// How deep an internal thread's flanks run: 5/8 of the sharp-V height for
/// a 60° form (ISO 68), 2/3 of it for a 55° Whitworth form.
pub fn depth(pitch: f32, angle: f32) -> f32 {
    let sharp = pitch / 2. / (angle / 2.).to_radians().tan();
    if angle < 57. {
        sharp * 2. / 3.
    } else {
        sharp * 5. / 8.
    }
}
const INCH: f32 = 25.4;
const fn metric(name: &'static str, major: f32, pitch: f32) -> Spec {
    Spec {
        name,
        major,
        pitch,
        angle: 60.,
        tapered: false,
        tap_drill: major - pitch,
    }
}
const fn unified(name: &'static str, inches: f32, tpi: f32) -> Spec {
    let pitch = INCH / tpi;
    Spec {
        name,
        major: inches * INCH,
        pitch,
        angle: 60.,
        tapered: false,
        // 75 % of the thread's depth.
        tap_drill: inches * INCH - 0.974 * pitch,
    }
}
const fn pipe(
    name: &'static str,
    major: f32,
    tpi: f32,
    angle: f32,
    tapered: bool,
    drill: f32,
) -> Spec {
    Spec {
        name,
        major,
        pitch: INCH / tpi,
        angle,
        tapered,
        tap_drill: drill,
    }
}
/// Every size detection matches and the thread operations offer: ISO metric
/// coarse and fine, Unified coarse and fine, BSP parallel (G) and taper (R),
/// NPT.
pub const SPECS: &[Spec] = &[
    metric("M1.6×0.35", 1.6, 0.35),
    metric("M2×0.4", 2., 0.4),
    metric("M2.5×0.45", 2.5, 0.45),
    metric("M3×0.5", 3., 0.5),
    metric("M3×0.35", 3., 0.35),
    metric("M3.5×0.6", 3.5, 0.6),
    metric("M4×0.7", 4., 0.7),
    metric("M4×0.5", 4., 0.5),
    metric("M5×0.8", 5., 0.8),
    metric("M5×0.5", 5., 0.5),
    metric("M6×1", 6., 1.),
    metric("M6×0.75", 6., 0.75),
    metric("M8×1.25", 8., 1.25),
    metric("M8×1", 8., 1.),
    metric("M10×1.5", 10., 1.5),
    metric("M10×1.25", 10., 1.25),
    metric("M10×1", 10., 1.),
    metric("M12×1.75", 12., 1.75),
    metric("M12×1.5", 12., 1.5),
    metric("M12×1.25", 12., 1.25),
    metric("M14×2", 14., 2.),
    metric("M14×1.5", 14., 1.5),
    metric("M16×2", 16., 2.),
    metric("M16×1.5", 16., 1.5),
    metric("M18×2.5", 18., 2.5),
    metric("M18×1.5", 18., 1.5),
    metric("M20×2.5", 20., 2.5),
    metric("M20×1.5", 20., 1.5),
    metric("M22×2.5", 22., 2.5),
    metric("M22×1.5", 22., 1.5),
    metric("M24×3", 24., 3.),
    metric("M24×2", 24., 2.),
    metric("M27×3", 27., 3.),
    metric("M27×2", 27., 2.),
    metric("M30×3.5", 30., 3.5),
    metric("M30×2", 30., 2.),
    unified("#2-56 UNC", 0.086, 56.),
    unified("#2-64 UNF", 0.086, 64.),
    unified("#4-40 UNC", 0.112, 40.),
    unified("#4-48 UNF", 0.112, 48.),
    unified("#5-40 UNC", 0.125, 40.),
    unified("#5-44 UNF", 0.125, 44.),
    unified("#6-32 UNC", 0.138, 32.),
    unified("#6-40 UNF", 0.138, 40.),
    unified("#8-32 UNC", 0.164, 32.),
    unified("#8-36 UNF", 0.164, 36.),
    unified("#10-24 UNC", 0.19, 24.),
    unified("#10-32 UNF", 0.19, 32.),
    unified("#12-24 UNC", 0.216, 24.),
    unified("#12-28 UNF", 0.216, 28.),
    unified("1/4-20 UNC", 0.25, 20.),
    unified("1/4-28 UNF", 0.25, 28.),
    unified("5/16-18 UNC", 0.3125, 18.),
    unified("5/16-24 UNF", 0.3125, 24.),
    unified("3/8-16 UNC", 0.375, 16.),
    unified("3/8-24 UNF", 0.375, 24.),
    unified("7/16-14 UNC", 0.4375, 14.),
    unified("7/16-20 UNF", 0.4375, 20.),
    unified("1/2-13 UNC", 0.5, 13.),
    unified("1/2-20 UNF", 0.5, 20.),
    unified("9/16-12 UNC", 0.5625, 12.),
    unified("9/16-18 UNF", 0.5625, 18.),
    unified("5/8-11 UNC", 0.625, 11.),
    unified("5/8-18 UNF", 0.625, 18.),
    unified("3/4-10 UNC", 0.75, 10.),
    unified("3/4-16 UNF", 0.75, 16.),
    unified("7/8-9 UNC", 0.875, 9.),
    unified("7/8-14 UNF", 0.875, 14.),
    unified("1-8 UNC", 1., 8.),
    unified("1-12 UNF", 1., 12.),
    pipe("G1/16", 7.723, 28., 55., false, 6.8),
    pipe("G1/8", 9.728, 28., 55., false, 8.8),
    pipe("G1/4", 13.157, 19., 55., false, 11.8),
    pipe("G3/8", 16.662, 19., 55., false, 15.25),
    pipe("G1/2", 20.955, 14., 55., false, 19.),
    pipe("G3/4", 26.441, 14., 55., false, 24.5),
    pipe("G1", 33.249, 11., 55., false, 30.75),
    pipe("R1/16", 7.723, 28., 55., true, 6.6),
    pipe("R1/8", 9.728, 28., 55., true, 8.4),
    pipe("R1/4", 13.157, 19., 55., true, 11.2),
    pipe("R3/8", 16.662, 19., 55., true, 14.75),
    pipe("R1/2", 20.955, 14., 55., true, 18.25),
    pipe("R3/4", 26.441, 14., 55., true, 23.75),
    pipe("R1", 33.249, 11., 55., true, 30.),
    pipe("1/16-27 NPT", 7.895, 27., 60., true, 6.3),
    pipe("1/8-27 NPT", 10.272, 27., 60., true, 8.7),
    pipe("1/4-18 NPT", 13.572, 18., 60., true, 11.1),
    pipe("3/8-18 NPT", 17.055, 18., 60., true, 14.3),
    pipe("1/2-14 NPT", 21.223, 14., 60., true, 17.9),
    pipe("3/4-14 NPT", 26.568, 14., 60., true, 23.),
    pipe("1-11.5 NPT", 33.228, 11.5, 60., true, 29.),
];
/// The size named `name` (as `Spec::name`).
pub fn spec(name: &str) -> Option<&'static Spec> {
    SPECS.iter().find(|s| s.name == name)
}

/// A thread found in a part, in the frame it was looked for in (a setup's,
/// in mm).
#[derive(Debug, Clone, PartialEq)]
pub struct Thread {
    /// The frame axis it runs along: 0 X, 1 Y, 2 Z.
    pub axis: usize,
    /// Where its axis crosses the plane across `axis`, in the frame's
    /// next two axes in turn (for Z: X, Y; for X: Y, Z; for Y: Z, X).
    pub centre: [f32; 2],
    /// Its extent along `axis`.
    pub from: f32,
    pub to: f32,
    /// A hole's thread (material round it), not a stud's.
    pub internal: bool,
    pub right_hand: bool,
    pub pitch: f32,
    /// Included flank angle: 60 or 55.
    pub angle: f32,
    /// The diameters measured on the model; a tapered thread's at its wide
    /// end.
    pub major: f32,
    pub minor: f32,
    pub tapered: bool,
    /// The standard size it matches; none for a size off the table.
    pub spec: Option<&'static Spec>,
}
impl Thread {
    /// What the app calls it: the size's name, or Ø×pitch when it is none.
    pub fn name(&self) -> String {
        match self.spec {
            Some(s) => s.name.to_string(),
            None => format!("Ø{:.2}×{}", self.major, trim(self.pitch)),
        }
    }
    /// Its name with left-hand marked, and inside or outside.
    pub fn label(&self) -> String {
        format!(
            "{}{} {}",
            self.name(),
            if self.right_hand { "" } else { " LH" },
            if self.internal { "hole" } else { "outside" }
        )
    }
    pub fn length(&self) -> f32 {
        self.to - self.from
    }
}
fn trim(v: f32) -> String {
    let s = format!("{v:.3}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// The threads of the part, placed as `setup` places it, along any of the
/// setup's three axes. Worked out once per placement angle set
/// (`Mesh::threads`).
pub fn in_setup(setup: &Setup, mesh: &Mesh) -> Vec<Thread> {
    let placement = Placement::new(mesh, Some(setup));
    let (angles, shift) = placement.parts();
    let scale = mesh.display_scale();
    let found = mesh.threads(angles, || {
        let triangles: Vec<[[f32; 3]; 3]> = mesh
            .faces
            .iter()
            .filter(|f| f.points.len() == 3)
            .map(|f| std::array::from_fn(|i| rotation(f.points[i], angles).map(|v| v / scale)))
            .collect();
        detect(&triangles)
    });
    let shift = shift.map(|v| v / scale);
    found
        .iter()
        .map(|t| {
            let (u, v) = ((t.axis + 1) % 3, (t.axis + 2) % 3);
            Thread {
                centre: [t.centre[0] + shift[u], t.centre[1] + shift[v]],
                from: t.from + shift[t.axis],
                to: t.to + shift[t.axis],
                ..t.clone()
            }
        })
        .collect()
}
/// The internal threads of `setup` along its Z: the threaded holes a Drill
/// of that setup can reach from above.
pub fn holes_in_setup(setup: &Setup, mesh: &Mesh) -> Vec<Thread> {
    in_setup(setup, mesh)
        .into_iter()
        .filter(|t| t.axis == 2 && t.internal)
        .collect()
}

/// The pitches the table holds, each once.
fn pitches() -> Vec<f32> {
    let mut p: Vec<f32> = SPECS.iter().map(|s| s.pitch).collect();
    p.sort_by(f32::total_cmp);
    p.dedup_by(|a, b| (*a - *b).abs() < 1e-4);
    p
}
fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
/// Votes are cast this far along each flank's normal line, mm: a thread up
/// to about M30 has its axis inside it.
const REACH: f32 = 16.;
/// The vote grid's cell, mm.
const CELL: f32 = 0.25;
/// Least flank triangles that make a thread.
const LEAST: usize = 40;
/// Least helix coherence of a thread's flank vertices.
const COHERENT: f32 = 0.8;

/// The threads in `triangles` (mm), along each of the frame's axes. The
/// triangles' winding gives their outside.
pub fn detect(triangles: &[[[f32; 3]; 3]]) -> Vec<Thread> {
    let mut found: Vec<Thread> = vec![];
    let normals: Vec<Option<[f32; 3]>> = triangles
        .iter()
        .map(|t| {
            let n = cross(sub(t[1], t[0]), sub(t[2], t[0]));
            let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            (l > 1e-9).then(|| n.map(|v| v / l))
        })
        .collect();
    let pitches = pitches();
    for axis in 0..3 {
        let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
        // Flank triangles: their centroid and normal seen along the axis,
        // and which way along the axis they face.
        struct Flank {
            index: usize,
            at: [f32; 2],
            toward: [f32; 2],
            up: bool,
            lean: f32,
        }
        let flanks: Vec<Flank> = (0..triangles.len())
            .filter_map(|i| {
                let n = normals[i]?;
                let lean = n[axis].abs();
                if !(0.80..0.93).contains(&lean) {
                    return None;
                }
                let t = &triangles[i];
                let at = [
                    (t[0][u] + t[1][u] + t[2][u]) / 3.,
                    (t[0][v] + t[1][v] + t[2][v]) / 3.,
                ];
                let l = (n[u] * n[u] + n[v] * n[v]).sqrt();
                Some(Flank {
                    index: i,
                    at,
                    toward: [n[u] / l, n[v] / l],
                    up: n[axis] > 0.,
                    lean,
                })
            })
            .collect();
        if flanks.len() < LEAST {
            continue;
        }
        // Each flank votes once for every cell its normal line crosses.
        let mut votes: HashMap<(i32, i32), u32> = HashMap::new();
        let steps = (2. * REACH / (CELL / 2.)) as i32;
        for f in &flanks {
            let mut last = None;
            for s in 0..=steps {
                let t = -REACH + s as f32 * CELL / 2.;
                let cell = (
                    ((f.at[0] + t * f.toward[0]) / CELL).round() as i32,
                    ((f.at[1] + t * f.toward[1]) / CELL).round() as i32,
                );
                if last != Some(cell) {
                    *votes.entry(cell).or_insert(0) += 1;
                    last = Some(cell);
                }
            }
        }
        let mut cells: Vec<((i32, i32), u32)> = votes
            .into_iter()
            .filter(|(_, n)| *n as usize >= LEAST)
            .collect();
        cells.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        let mut tried: Vec<[f32; 2]> = vec![];
        for (cell, _) in cells.into_iter().take(600) {
            let mut at = [cell.0 as f32 * CELL, cell.1 as f32 * CELL];
            if tried
                .iter()
                .any(|t| (t[0] - at[0]).hypot(t[1] - at[1]) < 1.5)
            {
                continue;
            }
            tried.push(at);
            // The axis: the point nearest the normal lines that pass close
            // to it, by least squares, a few times over.
            let mut members: Vec<&Flank> = vec![];
            for _ in 0..3 {
                members = flanks
                    .iter()
                    .filter(|f| {
                        let off = [f.at[0] - at[0], f.at[1] - at[1]];
                        let dist = off[0].hypot(off[1]);
                        let perp = (off[0] * f.toward[1] - off[1] * f.toward[0]).abs();
                        let radial =
                            (off[0] * f.toward[0] + off[1] * f.toward[1]).abs() / dist.max(1e-9);
                        perp < 1. && dist > 0.3 && dist < REACH && radial > 0.97
                    })
                    .collect();
                if members.len() < LEAST {
                    break;
                }
                let (mut a, mut b) = ([0f64; 3], [0f64; 2]);
                for f in &members {
                    let (dx, dy) = (f.toward[0] as f64, f.toward[1] as f64);
                    let m = [1. - dx * dx, -dx * dy, 1. - dy * dy];
                    a[0] += m[0];
                    a[1] += m[1];
                    a[2] += m[2];
                    b[0] += m[0] * f.at[0] as f64 + m[1] * f.at[1] as f64;
                    b[1] += m[1] * f.at[0] as f64 + m[2] * f.at[1] as f64;
                }
                let det = a[0] * a[2] - a[1] * a[1];
                if det.abs() < 1e-9 {
                    break;
                }
                at = [
                    ((a[2] * b[0] - a[1] * b[1]) / det) as f32,
                    ((a[0] * b[1] - a[1] * b[0]) / det) as f32,
                ];
            }
            if members.len() < LEAST
                || found.iter().any(|t| {
                    t.axis == axis && (t.centre[0] - at[0]).hypot(t.centre[1] - at[1]) < 0.5
                })
            {
                continue;
            }
            if let Some(thread) = read(
                axis,
                at,
                &members
                    .iter()
                    .map(|f| (f.index, f.up, f.lean))
                    .collect::<Vec<_>>(),
                triangles,
                &pitches,
            ) {
                found.push(thread);
            }
        }
    }
    found.sort_by(|a, b| {
        a.axis
            .cmp(&b.axis)
            .then(a.centre[0].total_cmp(&b.centre[0]))
            .then(a.centre[1].total_cmp(&b.centre[1]))
    });
    found
}

/// The thread round the axis through `at` along `axis` whose flank
/// triangles are `members` (index, faces up the axis, lean): its pitch and
/// hand, sizes, extent and standard size; none when its vertices make no
/// helix of a table pitch, or its flanks are not a thread's depth.
fn read(
    axis: usize,
    at: [f32; 2],
    members: &[(usize, bool, f32)],
    triangles: &[[[f32; 3]; 3]],
    pitches: &[f32],
) -> Option<Thread> {
    let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
    // The flank angle: 60° flanks lean cos 30° = 0.866 towards the axis,
    // 55° ones cos 27.5° = 0.887.
    let mut leans: Vec<f32> = members.iter().map(|m| m.2).collect();
    leans.sort_by(f32::total_cmp);
    let angle = if leans[leans.len() / 2] > 0.877 {
        55.
    } else {
        60.
    };
    let slope = (angle / 2f32).to_radians().tan();
    // (r, θ, z) of each flank's vertices, by the side the flank faces.
    let polar = |p: [f32; 3]| {
        let (x, y) = (p[u] - at[0], p[v] - at[1]);
        (x.hypot(y), y.atan2(x), p[axis])
    };
    let sides: [Vec<(f32, f32, f32)>; 2] = [true, false].map(|up| {
        members
            .iter()
            .filter(|m| m.1 == up)
            .flat_map(|m| triangles[m.0].iter().map(|p| polar(*p)))
            .collect()
    });
    let all: Vec<(f32, f32, f32)> = sides.iter().flatten().copied().collect();
    let (from, to) = all
        .iter()
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(a, b), p| {
            (a.min(p.2), b.max(p.2))
        });
    // How well the vertices of one flank side line up on a helix of
    // `pitch` and `hand`: they all sit at one phase of it once their
    // height along the flank is taken off.
    let coherence = |side: &[(f32, f32, f32)], pitch: f32, hand: f32, k: f32| {
        let (mut c, mut s) = (0f64, 0f64);
        for &(r, th, z) in side {
            let phase = std::f32::consts::TAU / pitch
                * (z - hand * pitch * th / std::f32::consts::TAU - k * r);
            c += phase.cos() as f64;
            s += phase.sin() as f64;
        }
        (c.hypot(s) / side.len().max(1) as f64) as f32
    };
    // The flanks' depth: the spread of radius over each pitch-long band
    // along the axis, the middle band's.
    let depth_at = |pitch: f32| {
        let bands = ((to - from) / pitch).floor() as usize;
        if bands == 0 {
            return 0.;
        }
        let mut spread: Vec<f32> = (0..bands)
            .map(|b| {
                let (lo, hi) = (from + b as f32 * pitch, from + (b + 1) as f32 * pitch);
                let (mn, mx) = all
                    .iter()
                    .filter(|p| p.2 >= lo && p.2 < hi)
                    .fold((f32::INFINITY, f32::NEG_INFINITY), |(a, b), p| {
                        (a.min(p.0), b.max(p.0))
                    });
                if mx > mn { mx - mn } else { 0. }
            })
            .collect();
        spread.sort_by(f32::total_cmp);
        spread[spread.len() / 2]
    };
    let mut best: Option<(f32, f32, f32)> = None;
    for &pitch in pitches {
        // A pitch half the true one fits the helix too: the flanks' depth,
        // about 0.54-0.61 of the pitch, tells them apart.
        let ratio = depth_at(pitch) / pitch;
        if !(0.4..=0.8).contains(&ratio) {
            continue;
        }
        for hand in [1f32, -1.] {
            let c = sides
                .iter()
                .filter(|s| s.len() >= 9)
                .map(|s| coherence(s, pitch, hand, slope).max(coherence(s, pitch, hand, -slope)))
                .fold(0., f32::max);
            if best.is_none_or(|b| c > b.0) {
                best = Some((c, pitch, hand));
            }
        }
    }
    let (c, pitch, hand) = best?;
    if c < COHERENT {
        return None;
    }
    // Inside or out: a hole's flanks face its axis.
    let facing: f32 = members
        .iter()
        .map(|m| {
            let t = &triangles[m.0];
            let n = cross(sub(t[1], t[0]), sub(t[2], t[0]));
            let mid = [
                (t[0][u] + t[1][u] + t[2][u]) / 3. - at[0],
                (t[0][v] + t[1][v] + t[2][v]) / 3. - at[1],
            ];
            (n[u] * mid[0] + n[v] * mid[1]).signum()
        })
        .sum();
    let internal = facing < 0.;
    // A taper shows as a steady change of radius along the axis.
    let n = all.len() as f64;
    let (mz, mr) = all
        .iter()
        .fold((0f64, 0f64), |(a, b), p| (a + p.2 as f64, b + p.0 as f64));
    let (mz, mr) = (mz / n, mr / n);
    let (sxy, sxx) = all.iter().fold((0f64, 0f64), |(a, b), p| {
        let dz = p.2 as f64 - mz;
        (a + dz * (p.0 as f64 - mr), b + dz * dz)
    });
    let rate = if sxx > 0. { (sxy / sxx) as f32 } else { 0. };
    // 1:16 on the diameter is 1/32 on the radius.
    let tapered = rate.abs() > 0.015 && to - from > 3. * pitch;
    let mut radii: Vec<f32> = all.iter().map(|p| p.0).collect();
    radii.sort_by(f32::total_cmp);
    let pick = |q: f32| radii[((radii.len() - 1) as f32 * q).round() as usize];
    let (major, minor) = if tapered {
        // At the wide end: the ends' bands, a pitch long.
        let end = if rate > 0. { to } else { from };
        let near: Vec<f32> = all
            .iter()
            .filter(|p| (p.2 - end).abs() <= pitch)
            .map(|p| p.0)
            .collect();
        let mx = near.iter().copied().fold(0., f32::max);
        (2. * mx, 2. * (mx - depth_at(pitch)))
    } else {
        (2. * pick(0.995), 2. * pick(0.005))
    };
    let spec = SPECS
        .iter()
        .filter(|s| (s.pitch - pitch).abs() < 1e-3 && s.angle == angle && s.tapered == tapered)
        .min_by(|a, b| (a.major - major).abs().total_cmp(&(b.major - major).abs()))
        .filter(|s| {
            let off = (s.major - major).abs();
            if tapered {
                off <= 0.1 * s.major
            } else {
                off <= 0.3f32.max(0.05 * s.major)
            }
        });
    Some(Thread {
        axis,
        centre: at,
        from,
        to,
        internal,
        right_hand: hand > 0.,
        pitch,
        angle,
        major,
        minor,
        tapered,
        spec,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A modeled thread as a triangle mesh along Z: the basic ISO/Whitworth
    /// profile swept round `centre` from z = 0 to `length`, `steps` facets
    /// per turn. `internal` faces its triangles in towards the axis (a
    /// hole's); `taper` is the diameter's change per mm, the wide end at
    /// z = 0 for a hole.
    pub(crate) fn modeled(
        centre: [f32; 2],
        major: f32,
        pitch: f32,
        angle: f32,
        right_hand: bool,
        internal: bool,
        length: f32,
        taper: f32,
    ) -> Vec<[[f32; 3]; 3]> {
        let h = depth(pitch, angle);
        let slope = (angle / 2f32).to_radians().tan();
        let flank = h * slope;
        // Root and crest flats share what the flanks leave of a pitch.
        let flat = (pitch - 2. * flank) / 2.;
        let profile = |phase: f32| {
            let p = phase.rem_euclid(pitch);
            // From the crest (smallest radius inside, largest outside).
            let along = if p < flat / 2. {
                0.
            } else if p < flat / 2. + flank {
                (p - flat / 2.) / flank
            } else if p < flat / 2. + flank + flat {
                1.
            } else if p < flat / 2. + 2. * flank + flat {
                1. - (p - flat / 2. - flank - flat) / flank
            } else {
                0.
            };
            along * h
        };
        let hand = if right_hand { 1. } else { -1. };
        let steps = 96;
        let rows = (length / (pitch / 24.)).ceil() as usize;
        let point = |i: usize, j: usize| {
            let th = i as f32 / steps as f32 * std::f32::consts::TAU;
            let z = j as f32 * length / rows as f32;
            let phase = z - hand * pitch * th / std::f32::consts::TAU;
            let r0 = (major - taper * z) / 2.;
            let r = if internal {
                r0 - h + profile(phase)
            } else {
                r0 - profile(phase)
            };
            [centre[0] + r * th.cos(), centre[1] + r * th.sin(), z]
        };
        let mut out = vec![];
        for i in 0..steps {
            for j in 0..rows {
                let (a, b, c, d) = (
                    point(i, j),
                    point(i + 1, j),
                    point(i + 1, j + 1),
                    point(i, j + 1),
                );
                if internal {
                    out.push([a, c, b]);
                    out.push([a, d, c]);
                } else {
                    out.push([a, b, c]);
                    out.push([a, c, d]);
                }
            }
        }
        out
    }
    /// A plain cylinder (a hole facing in) and a 120° cone: flank-like
    /// lean, no helix.
    fn plain(centre: [f32; 2], r: f32, length: f32) -> Vec<[[f32; 3]; 3]> {
        let steps = 64;
        let at = |i: usize, rr: f32, z: f32| {
            let th = i as f32 / steps as f32 * std::f32::consts::TAU;
            [centre[0] + rr * th.cos(), centre[1] + rr * th.sin(), z]
        };
        let mut out = vec![];
        for i in 0..steps {
            for k in 0..20 {
                let (z0, z1) = (k as f32 * length / 20., (k + 1) as f32 * length / 20.);
                out.push([at(i, r, z0), at(i + 1, r, z1), at(i + 1, r, z0)]);
                out.push([at(i, r, z0), at(i, r, z1), at(i + 1, r, z1)]);
                // The cone: 60° from the axis, so its normal leans 30°.
                let (c0, c1) = (r + 2. + z0 * 3f32.sqrt(), r + 2. + z1 * 3f32.sqrt());
                out.push([
                    at(i, c0, z0 + 30.),
                    at(i + 1, c1, z1 + 30.),
                    at(i + 1, c0, z0 + 30.),
                ]);
                out.push([
                    at(i, c0, z0 + 30.),
                    at(i, c1, z1 + 30.),
                    at(i + 1, c1, z1 + 30.),
                ]);
            }
        }
        out
    }

    fn one(found: &[Thread]) -> &Thread {
        assert_eq!(found.len(), 1, "{found:#?}");
        &found[0]
    }

    #[test]
    fn a_tapped_m6_hole_reads_as_m6_right_hand_inside() {
        let found = detect(&modeled([12., -4.], 6., 1., 60., true, true, 10., 0.));
        let t = one(&found);
        assert_eq!(t.spec.map(|s| s.name), Some("M6×1"));
        assert!(t.internal && t.right_hand && !t.tapered && t.axis == 2);
        assert!(
            (t.centre[0] - 12.).abs() < 0.05 && (t.centre[1] + 4.).abs() < 0.05,
            "{t:?}"
        );
        assert!((t.length() - 10.).abs() < 0.3, "{t:?}");
        assert!(
            (t.major - 6.).abs() < 0.1 && (t.minor - 4.917).abs() < 0.15,
            "{t:?}"
        );
    }
    #[test]
    fn left_hand_fine_and_small_threads_read_true() {
        let t = one(&detect(&modeled(
            [0., 0.],
            8.,
            1.,
            60.,
            false,
            true,
            12.,
            0.,
        )))
        .clone();
        assert_eq!(t.spec.map(|s| s.name), Some("M8×1"));
        assert!(!t.right_hand);
        let t = one(&detect(&modeled(
            [3., 3.],
            3.,
            0.5,
            60.,
            true,
            true,
            6.,
            0.,
        )))
        .clone();
        assert_eq!(t.spec.map(|s| s.name), Some("M3×0.5"));
    }
    #[test]
    fn a_stud_reads_as_outside() {
        let t = one(&detect(&modeled(
            [0., 0.],
            10.,
            1.5,
            60.,
            true,
            false,
            15.,
            0.,
        )))
        .clone();
        assert_eq!(t.spec.map(|s| s.name), Some("M10×1.5"));
        assert!(!t.internal && t.right_hand);
    }
    #[test]
    fn inch_and_whitworth_sizes_match_their_tables() {
        let t = one(&detect(&modeled(
            [0., 0.],
            6.35,
            1.27,
            60.,
            true,
            true,
            12.,
            0.,
        )))
        .clone();
        assert_eq!(t.spec.map(|s| s.name), Some("1/4-20 UNC"));
        let t = one(&detect(&modeled(
            [0., 0.],
            13.157,
            25.4 / 19.,
            55.,
            true,
            true,
            12.,
            0.,
        )))
        .clone();
        assert_eq!((t.angle, t.spec.map(|s| s.name)), (55., Some("G1/4")));
    }
    #[test]
    fn a_tapered_pipe_thread_reads_as_npt() {
        // Narrowing 1:16 into the hole from z = 0.
        let t = one(&detect(&modeled(
            [0., 0.],
            10.272,
            25.4 / 27.,
            60.,
            true,
            true,
            10.,
            1. / 16.,
        )))
        .clone();
        assert!(t.tapered, "{t:?}");
        assert_eq!(t.spec.map(|s| s.name), Some("1/8-27 NPT"));
    }
    #[test]
    fn plain_holes_and_cones_are_no_threads() {
        assert!(detect(&plain([0., 0.], 2.5, 10.)).is_empty());
    }
    #[test]
    fn threads_along_x_keep_their_hand() {
        // The M6 laid along X: (x, y, z) → (z, x, y) keeps the frame right-handed.
        let along_x: Vec<[[f32; 3]; 3]> = modeled([0., 0.], 6., 1., 60., true, true, 10., 0.)
            .into_iter()
            .map(|t| t.map(|p| [p[2], p[0], p[1]]))
            .collect();
        let t = one(&detect(&along_x)).clone();
        assert!(t.axis == 0 && t.right_hand && t.internal, "{t:?}");
    }
    #[test]
    fn two_holes_side_by_side_are_two_threads() {
        let mut both = modeled([0., 0.], 5., 0.8, 60., true, true, 8., 0.);
        both.extend(modeled([20., 0.], 5., 0.8, 60., true, true, 8., 0.));
        let found = detect(&both);
        assert_eq!(found.len(), 2, "{found:#?}");
        assert!(
            found
                .iter()
                .all(|t| t.spec.map(|s| s.name) == Some("M5×0.8"))
        );
    }
}
