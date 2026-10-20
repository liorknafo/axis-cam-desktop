//! Persistent continuous cutter history and coarse volume estimates for stock in the first setup's frame. Setup changes transform
//! its display/cutter coordinates; they never rebuild or refill the material.
use crate::{
    mesh::{Bounds, Mesh, rotation},
    model::{Setup, num},
    simulation::CutterShape,
    stock::Placement,
};
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    basis: [[f32; 3]; 3],
    shift: [f32; 3],
}
impl Frame {
    pub fn new(setup: &Setup, mesh: &Mesh) -> Self {
        let placement = Placement::new(mesh, Some(setup));
        Self {
            shift: placement
                .transform([0.; 3])
                .map(|v| v / mesh.display_scale()),
            ..Self::axes(setup)
        }
    }
    /// The axes of `setup`'s frame, which its rotation alone gives, at no shift.
    fn axes(setup: &Setup) -> Self {
        let angles = ["rx", "ry", "rz"].map(|k| num(&setup.p, k).unwrap_or(0.));
        Self {
            basis: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]].map(|p| rotation(p, angles)),
            shift: [0.; 3],
        }
    }
    /// Whether the frame of `later` is square to that of `first`, from their
    /// rotations alone (`square_to`): no part is needed.
    pub fn setups_square(first: &Setup, later: &Setup) -> bool {
        Self::axes(first).square_to(Self::axes(later))
    }
    /// The point `p` of the part's own frame (its file's coordinates, mm)
    /// in this setup's.
    pub fn map(self, p: [f32; 3]) -> [f32; 3] {
        std::array::from_fn(|k| {
            self.shift[k] + (0..3).map(|i| self.basis[i][k] * p[i]).sum::<f32>()
        })
    }
    /// The point `p` of this setup's frame in the part's own (`map` back).
    pub fn unmap(self, p: [f32; 3]) -> [f32; 3] {
        std::array::from_fn(|i| {
            (0..3)
                .map(|k| (p[k] - self.shift[k]) * self.basis[i][k])
                .sum()
        })
    }
    pub fn transfer(self, p: [f32; 3], to: Self) -> [f32; 3] {
        to.map(self.unmap(p))
    }
    /// The box around `bounds` (in this frame) as seen in the frame `to`.
    pub fn transfer_bounds(self, bounds: Bounds, to: Self) -> Bounds {
        let mut out = Bounds {
            min: [f32::INFINITY; 3],
            max: [f32::NEG_INFINITY; 3],
        };
        for mask in 0..8 {
            let p = std::array::from_fn(|k| {
                if mask & (1 << k) == 0 {
                    bounds.min[k]
                } else {
                    bounds.max[k]
                }
            });
            let p = self.transfer(p, to);
            for (k, &v) in p.iter().enumerate() {
                out.min[k] = out.min[k].min(v);
                out.max[k] = out.max[k].max(v);
            }
        }
        out
    }
    /// Whether the axes of `self` are parallel to the axes of `to` (any multiple of 90°).
    pub fn square_to(self, to: Self) -> bool {
        [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]]
            .into_iter()
            .all(|axis| {
                self.transfer_vector(axis, to)
                    .into_iter()
                    .all(|v| v.abs() < 1e-4 || (v.abs() - 1.).abs() < 1e-4)
            })
    }
    pub fn transfer_vector(self, v: [f32; 3], to: Self) -> [f32; 3] {
        let part: [f32; 3] = std::array::from_fn(|i| (0..3).map(|k| v[k] * self.basis[i][k]).sum());
        std::array::from_fn(|k| (0..3).map(|i| to.basis[i][k] * part[i]).sum())
    }
}
/// A turn of the stock by whole quarter turns, from setup 1's frame to a
/// later setup's: `turn[k][i]` is how far the stock's axis `i` lies along
/// the machine's axis `k`, each -1, 0 or 1.
pub type Turn = [[f32; 3]; 3];
/// The faces of the stock a later setup can have up, each with the turn
/// from setup 1 that puts it there (rotations about X, Y, Z, degrees).
pub const FACES: [(&str, [f32; 3]); 6] = [
    ("Top (+Z)", [0., 0., 0.]),
    ("Bottom (−Z)", [180., 0., 0.]),
    ("Front (−Y)", [-90., 0., 0.]),
    ("Back (+Y)", [90., 0., 0.]),
    ("+X side", [0., -90., 0.]),
    ("−X side", [0., 90., 0.]),
];
/// The quarter turns on the table, about Z, after the face is up.
pub const TABLE_TURNS: [&str; 4] = ["0°", "90°", "180°", "270°"];
/// The matrix of a setup's rotation: column `i` is where the part's axis
/// `i` goes.
fn matrix(angles: [f32; 3]) -> Turn {
    let columns = [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]].map(|p| rotation(p, angles));
    std::array::from_fn(|k| std::array::from_fn(|i| columns[i][k]))
}
fn product(a: Turn, b: Turn) -> Turn {
    std::array::from_fn(|k| std::array::from_fn(|i| (0..3).map(|j| a[k][j] * b[j][i]).sum()))
}
fn transposed(a: Turn) -> Turn {
    std::array::from_fn(|k| std::array::from_fn(|i| a[i][k]))
}
fn angles_of(setup: &Setup) -> [f32; 3] {
    ["rx", "ry", "rz"].map(|k| num(&setup.p, k).unwrap_or(0.))
}
/// The rotations about X, Y and Z (degrees, as a setup holds them) whose
/// matrix is `m`: of the two that are, the one with the smaller angles, so
/// a flip reads 0, 180, -2 rather than 180, 0, 178. Rounded to 0.0001°.
fn angles(m: Turn) -> [f32; 3] {
    let tidy = |v: f32| {
        let v = (v.to_degrees() * 10_000.).round() / 10_000.;
        // Into (-180, 180], and a -0 printed as 0.
        let v = if v <= -180. {
            v + 360.
        } else if v > 180. {
            v - 360.
        } else {
            v
        };
        v + 0.
    };
    let cb = m[0][0].hypot(m[1][0]);
    let b = (-m[2][0]).atan2(cb);
    if cb < 1e-6 {
        // Y at ±90°: X and Z turn about the same axis, so X takes none of it.
        let c = (-m[0][1]).atan2(m[1][1]);
        return [0., tidy(b), tidy(c)];
    }
    let a = m[2][1].atan2(m[2][2]);
    let c = m[1][0].atan2(m[0][0]);
    let pi = std::f32::consts::PI;
    let first = [a, b, c].map(tidy);
    let second = [a + pi, pi - b, c + pi].map(tidy);
    let size = |r: [f32; 3]| r.iter().map(|v| v.abs()).sum::<f32>();
    if size(second) + 1e-3 < size(first) {
        second
    } else {
        first
    }
}
/// The turn from setup `first`'s frame to setup `later`'s, when it is by
/// whole quarter turns (`Frame::square_to`).
pub fn turn_between(first: &Setup, later: &Setup) -> Option<Turn> {
    let turn = product(
        matrix(angles_of(later)),
        transposed(matrix(angles_of(first))),
    );
    turn.iter()
        .flatten()
        .all(|v| v.abs() < 1e-4 || (v.abs() - 1.).abs() < 1e-4)
        .then(|| turn.map(|row| row.map(|v| v.round() + 0.)))
}
/// Every quarter-turn of the stock: each face up (`FACES`), turned on the
/// table (`TABLE_TURNS`), as (face, table turn, turn).
pub fn turns() -> impl Iterator<Item = (usize, usize, Turn)> {
    (0..FACES.len()).flat_map(|f| {
        (0..TABLE_TURNS.len()).map(move |q| {
            let turn = product(matrix([0., 0., 90. * q as f32]), matrix(FACES[f].1));
            (f, q, turn.map(|row| row.map(|v| v.round() + 0.)))
        })
    })
}
/// The face up and the table turn (`turns`) of a quarter turn.
pub fn face_and_table(turn: Turn) -> (usize, usize) {
    turns()
        .find(|(_, _, t)| *t == turn)
        .map_or((0, 0), |(f, q, _)| (f, q))
}
/// The quarter turn nearest the turn from `first`'s frame to `later`'s: the
/// one a later setup a few degrees off was meant to be.
pub fn nearest_turn(first: &Setup, later: &Setup) -> Turn {
    let turn = product(
        matrix(angles_of(later)),
        transposed(matrix(angles_of(first))),
    );
    let along = |t: &Turn| {
        (0..3)
            .flat_map(|k| (0..3).map(move |i| (k, i)))
            .map(|(k, i)| t[k][i] * turn[k][i])
            .sum::<f32>()
    };
    turns()
        .map(|(_, _, t)| t)
        .max_by(|a, b| along(a).total_cmp(&along(b)))
        .expect("there are 24 turns")
}
/// The rotations (X, Y, Z, degrees) that turn the part as setup `first`
/// has it, and then the stock by `turn`: a later setup square to setup 1.
pub fn turned(first: &Setup, turn: Turn) -> [f32; 3] {
    angles(product(turn, matrix(angles_of(first))))
}
#[derive(Debug, Clone)]
pub struct Volume {
    pub bounds: Bounds,
    pub frame: Frame,
    pub n: [usize; 3],
    pub cell: [f32; 3],
    pub occupied: Vec<u8>,
    pub remaining: usize,
    /// How many voxels held material at the start: all of them for a box,
    /// those inside the cylinder for a round bar.
    pub material: usize,
    pub cuts: std::sync::Arc<Vec<crate::stock_surface::Cut>>,
    pub version: u64,
    /// Whether the stock started as a round bar (`bar`): the cylinder in
    /// `bounds` (`crate::stock::bar`), not the whole box.
    pub round: bool,
}
impl Volume {
    /// A round bar's stock: `new`, with the voxels whose centres lie outside
    /// the bar's cylinder (`crate::stock::bar`, along `frame`'s Z) cleared.
    pub fn bar(bounds: Bounds, frame: Frame) -> Self {
        let mut v = Self::new(bounds, frame);
        v.round = true;
        let (centre, radius) = crate::stock::bar(bounds);
        for z in 0..v.n[2] {
            for y in 0..v.n[1] {
                for x in 0..v.n[0] {
                    let p = v.point([x, y, z], 0.5);
                    if (p[0] - centre[0]).hypot(p[1] - centre[1]) > radius {
                        let i = v.index([x, y, z]);
                        v.occupied[i] = 0;
                        v.remaining -= 1;
                    }
                }
            }
        }
        v.material = v.remaining;
        v
    }
    /// The stock a setup's resolved `bounds` make: a round bar (`bar`) when
    /// `round`, else the box (`new`).
    pub fn stock(bounds: Bounds, frame: Frame, round: bool) -> Self {
        if round {
            Self::bar(bounds, frame)
        } else {
            Self::new(bounds, frame)
        }
    }
    pub fn new(bounds: Bounds, frame: Frame) -> Self {
        let size = bounds.dimensions();
        let step = size.into_iter().fold(0., f32::max) / 96.;
        let n = size.map(|v| (v / step).ceil().clamp(1., 96.) as usize);
        let cell = std::array::from_fn(|k| size[k] / n[k] as f32);
        let count = n.iter().product();
        Self {
            bounds,
            frame,
            n,
            cell,
            occupied: vec![1; count],
            remaining: count,
            material: count,
            cuts: std::sync::Arc::new(vec![]),
            version: 0,
            round: false,
        }
    }
    pub fn index(&self, p: [usize; 3]) -> usize {
        (p[2] * self.n[1] + p[1]) * self.n[0] + p[0]
    }
    pub fn point(&self, p: [usize; 3], offset: f32) -> [f32; 3] {
        std::array::from_fn(|k| self.bounds.min[k] + (p[k] as f32 + offset) * self.cell[k])
    }
    /// Whether the voxel holding the point `p` of the frame `from` still has
    /// material: false outside the stock.
    pub fn filled(&self, p: [f32; 3], from: Frame) -> bool {
        let q = from.transfer(p, self.frame);
        let mut at = [0; 3];
        for k in 0..3 {
            let i = ((q[k] - self.bounds.min[k]) / self.cell[k]).floor();
            if !(0. ..self.n[k] as f32).contains(&i) {
                return false;
            }
            at[k] = i as usize;
        }
        self.occupied[self.index(at)] != 0
    }
    pub fn removed_volume(&self) -> f32 {
        (self.material - self.remaining) as f32 * self.cell.iter().product::<f32>()
    }
    pub fn transformed_bounds(&self, to: Frame) -> Bounds {
        self.frame.transfer_bounds(self.bounds, to)
    }
    /// The voxels, as index ranges, that a cutter of `radius` with `flute`
    /// long flutes moving from `a` to `b` in `current` can sweep: None when
    /// its box misses the stock, or when it goes nowhere.
    fn ranges(
        &self,
        a: [f32; 3],
        b: [f32; 3],
        radius: f32,
        flute: f32,
        shape: CutterShape,
        current: Frame,
    ) -> Option<[(usize, usize); 3]> {
        let (lo, hi) = if shape == CutterShape::Revolve {
            // A ring goes nowhere when it is empty.
            if !(a[0] < b[0] && a[1] < b[1]) {
                return None;
            }
            ([-b[1], -b[1], a[0]], [b[1], b[1], b[0]])
        } else {
            if (0..3).all(|k| (a[k] - b[k]).abs() < 1e-7) {
                return None;
            }
            (
                [
                    a[0].min(b[0]) - radius,
                    a[1].min(b[1]) - radius,
                    a[2].min(b[2]),
                ],
                [
                    a[0].max(b[0]) + radius,
                    a[1].max(b[1]) + radius,
                    a[2].max(b[2]) + flute,
                ],
            )
        };
        let mut low = [f32::INFINITY; 3];
        let mut high = [f32::NEG_INFINITY; 3];
        for mask in 0..8 {
            let p = std::array::from_fn(|k| if mask & (1 << k) == 0 { lo[k] } else { hi[k] });
            let p = current.transfer(p, self.frame);
            for k in 0..3 {
                low[k] = low[k].min(p[k]);
                high[k] = high[k].max(p[k]);
            }
        }
        if (0..3).any(|k| high[k] < self.bounds.min[k] || low[k] > self.bounds.max[k]) {
            return None;
        }
        Some(std::array::from_fn(|k| {
            (
                ((low[k] - self.bounds.min[k]) / self.cell[k])
                    .floor()
                    .clamp(0., self.n[k] as f32) as usize,
                ((high[k] - self.bounds.min[k]) / self.cell[k])
                    .ceil()
                    .clamp(0., self.n[k] as f32) as usize,
            )
        }))
    }
    /// Whether a cutter of `radius` with `flute` long flutes moving from
    /// `a` to `b` in `current` sweeps the centre of a voxel that still has
    /// material: what `cut` would remove, without removing it.
    pub fn touches(
        &self,
        a: [f32; 3],
        b: [f32; 3],
        radius: f32,
        flute: f32,
        shape: CutterShape,
        current: Frame,
    ) -> bool {
        let Some(ranges) = self.ranges(a, b, radius, flute, shape, current) else {
            return false;
        };
        for z in ranges[2].0..ranges[2].1 {
            for y in ranges[1].0..ranges[1].1 {
                for x in ranges[0].0..ranges[0].1 {
                    if self.occupied[self.index([x, y, z])] == 0 {
                        continue;
                    }
                    let p = self.frame.transfer(self.point([x, y, z], 0.5), current);
                    if swept(p, a, b, radius, flute, shape) {
                        return true;
                    }
                }
            }
        }
        false
    }
    #[allow(clippy::too_many_arguments)]
    pub fn cut(
        &mut self,
        a: [f32; 3],
        b: [f32; 3],
        radius: f32,
        flute: f32,
        shape: CutterShape,
        current: Frame,
        source: (u16, u16),
    ) {
        let Some(ranges) = self.ranges(a, b, radius, flute, shape, current) else {
            return;
        };
        let cut = crate::stock_surface::Cut {
            a,
            b,
            radius,
            flute,
            shape,
            frame: current,
            source,
        };
        let cuts = std::sync::Arc::make_mut(&mut self.cuts);
        if !cuts.last_mut().is_some_and(|last| last.extend(&cut)) {
            cuts.push(cut);
        }
        self.version = self.version.wrapping_add(1);
        for z in ranges[2].0..ranges[2].1 {
            for y in ranges[1].0..ranges[1].1 {
                for x in ranges[0].0..ranges[0].1 {
                    let i = self.index([x, y, z]);
                    if self.occupied[i] == 0 {
                        continue;
                    }
                    let p = self.frame.transfer(self.point([x, y, z], 0.5), current);
                    if swept(p, a, b, radius, flute, shape) {
                        self.occupied[i] = 0;
                        self.remaining -= 1;
                    }
                }
            }
        }
    }
    /// Turn away the ring of `current`'s frame between `z0` and `z1` along
    /// its Z axis and between the radii `r0` and `r1` from it (mm, in either
    /// order; a negative radius is taken as 0): a lathe cut, recorded as a
    /// `Revolve` cut with `a = [z0, r0, 0]`, `b = [z1, r1, 0]`, its radius
    /// `r1` and no flutes.
    pub fn revolve(
        &mut self,
        z0: f32,
        z1: f32,
        r0: f32,
        r1: f32,
        current: Frame,
        source: (u16, u16),
    ) {
        let (z0, z1) = (z0.min(z1), z0.max(z1));
        let (r0, r1) = (r0.min(r1).max(0.), r0.max(r1).max(0.));
        self.cut(
            [z0, r0, 0.],
            [z1, r1, 0.],
            r1,
            0.,
            CutterShape::Revolve,
            current,
            source,
        );
    }
}
pub(crate) fn swept(
    p: [f32; 3],
    a: [f32; 3],
    b: [f32; 3],
    r: f32,
    flute: f32,
    shape: CutterShape,
) -> bool {
    if shape == CutterShape::Revolve {
        // The ring between Z a[0]..b[0] and radii a[1]..b[1] about Z.
        let radial = p[0].hypot(p[1]);
        return p[2] >= a[0] && p[2] <= b[0] && radial >= a[1] && radial <= b[1];
    }
    let v = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let l2 = v[0] * v[0] + v[1] * v[1];
    let projection = if l2 < 1e-12 {
        0.
    } else {
        ((p[0] - a[0]) * v[0] + (p[1] - a[1]) * v[1]) / l2
    };
    let perp =
        (p[0] - a[0] - projection * v[0]).powi(2) + (p[1] - a[1] - projection * v[1]).powi(2);
    if perp > r * r {
        return false;
    }
    let (mut lo, mut hi) = if l2 < 1e-12 {
        (0., 1.)
    } else {
        let half = ((r * r - perp) / l2).sqrt();
        ((projection - half).max(0.), (projection + half).min(1.))
    };
    // The point must lie below the finite flute top at the same instant.
    if v[2].abs() < 1e-12 {
        if p[2] > a[2] + flute {
            return false;
        }
    } else {
        let t = (p[2] - flute - a[2]) / v[2];
        if v[2] > 0. {
            lo = lo.max(t);
        } else {
            hi = hi.min(t);
        }
    }
    if lo > hi {
        return false;
    }
    let rise = shape.rise().unwrap_or(0.);
    let cone = rise * rise * l2;
    let t = if shape == CutterShape::Ball && l2 > 1e-12 {
        (projection
            - v[2] * (r * r - perp).max(0.).sqrt() / (l2.sqrt() * (l2 + v[2] * v[2]).sqrt()))
        .clamp(lo, hi)
    } else if shape.rise().is_some() && l2 > 1e-12 && cone > v[2] * v[2] {
        // The point's cone is lowest over the interval where the tip's
        // descent balances the cone's rise; a move steeper than the cone is
        // lowest at an end of the interval, as a flat end is.
        (projection - v[2] * perp.sqrt() / (l2.sqrt() * (cone - v[2] * v[2]).sqrt())).clamp(lo, hi)
    } else if v[2] < 0. {
        hi
    } else {
        lo
    };
    let radial = (p[0] - a[0] - t * v[0]).powi(2) + (p[1] - a[1] - t * v[1]).powi(2);
    let bottom = a[2]
        + t * v[2]
        + match shape {
            CutterShape::Ball => r - (r * r - radial).max(0.).sqrt(),
            CutterShape::Flat => 0.,
            CutterShape::Drill | CutterShape::Chamfer(_) => rise * radial.sqrt(),
            // Rings return above.
            CutterShape::Revolve => 0.,
        };
    p[2] >= bottom - 0.00001
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::POINT_RISE;
    use crate::{mesh, model::Machine};
    fn frames() -> (Frame, Frame) {
        let mesh = mesh::normalize(vec![[[0., 0., 0.], [20., 0., 0.], [0., 20., 10.]]]).unwrap();
        let mut s = Setup::new(Machine::new(3), "G54");
        let a = Frame::new(&s, &mesh);
        s.p.insert("rx".into(), "180".into());
        s.p.insert("tx".into(), "12".into());
        (a, Frame::new(&s, &mesh))
    }
    #[test]
    fn frames_are_square_only_at_multiples_of_90_degrees() {
        let mesh = mesh::normalize(vec![[[0., 0., 0.], [20., 0., 0.], [0., 20., 10.]]]).unwrap();
        let frame = |angles: [&str; 3]| {
            let mut s = Setup::new(Machine::new(3), "G54");
            for (k, v) in ["rx", "ry", "rz"].into_iter().zip(angles) {
                s.p.insert(k.into(), v.into());
            }
            Frame::new(&s, &mesh)
        };
        let unrotated = frame(["0", "0", "0"]);
        for angles in [
            ["0", "0", "0"],
            ["180", "0", "0"],
            ["0", "-90", "0"],
            ["90", "0", "90"],
        ] {
            let f = frame(angles);
            assert!(f.square_to(unrotated), "{angles:?}");
            assert!(unrotated.square_to(f), "{angles:?}");
        }
        for angles in [["0", "0", "30"], ["45", "0", "0"]] {
            let f = frame(angles);
            assert!(!f.square_to(unrotated), "{angles:?}");
            assert!(!unrotated.square_to(f), "{angles:?}");
        }
    }
    fn setup_at(angles: [f32; 3]) -> Setup {
        let mut s = Setup::new(crate::model::Machine::new(3), "G54");
        for (k, v) in ["rx", "ry", "rz"].iter().zip(angles) {
            s.p.insert((*k).into(), v.to_string());
        }
        s
    }
    #[test]
    fn the_angles_of_a_rotation_give_it_back() {
        for a in [
            [0., 0., 0.],
            [180., 0., 0.],
            [0., 180., -2.],
            [12., -33., 71.],
            [-170., 89.99, 5.],
            [0., 90., 30.],
            [45., -90., 0.],
            [90., 0., 90.],
        ] {
            let back = angles(matrix(a));
            let (m, n) = (matrix(a), matrix(back));
            assert!(
                (0..3).all(|k| (0..3).all(|i| (m[k][i] - n[k][i]).abs() < 1e-4)),
                "{a:?} came back as {back:?}"
            );
        }
        // Of the two angle sets of a turn, the smaller.
        assert_eq!(angles(matrix([180., 0., 178.])), [0., 180., -2.]);
    }
    #[test]
    fn every_turn_of_the_stock_is_square_to_a_tilted_setup_1() {
        let all: Vec<_> = turns().collect();
        assert_eq!(all.len(), 24);
        for (n, (_, _, t)) in all.iter().enumerate() {
            assert!(all[..n].iter().all(|(_, _, u)| u != t), "turn {n} repeats");
        }
        for first in [
            [0., 0., 0.],
            [0., 0., 2.],
            [10., -20., 33.],
            [180., 0., 90.],
        ] {
            let first = setup_at(first);
            for &(f, q, t) in &all {
                let later = setup_at(turned(&first, t));
                assert_eq!(turn_between(&first, &later), Some(t), "{f} {q}");
                assert_eq!(face_and_table(t), (f, q));
                assert!(Frame::axes(&later).square_to(Frame::axes(&first)));
            }
        }
    }
    #[test]
    fn a_setup_a_few_degrees_off_is_nearest_the_turn_it_was_meant_to_be() {
        let first = setup_at([0., 0., 2.]);
        let flipped = setup_at([0., 180., 0.]);
        assert_eq!(turn_between(&first, &flipped), None);
        let t = nearest_turn(&first, &flipped);
        assert_eq!(face_and_table(t), (1, 2));
        assert_eq!(turned(&first, t), [0., 180., -2.]);
        // Already square: its own turn.
        let square = setup_at([180., 0., 88.]);
        assert_eq!(
            Some(nearest_turn(&first, &square)),
            turn_between(&first, &square)
        );
    }
    #[test]
    fn stock_frame_roundtrips_after_flip_and_translation() {
        let (a, b) = frames();
        for p in [[0., 0., 0.], [4., 7., 10.], [-3., 2., 8.]] {
            let q = b.transfer(a.transfer(p, b), a);
            assert!((0..3).all(|i| (q[i] - p[i]).abs() < 0.00001));
        }
        let mut stock = Volume::new(
            Bounds {
                min: [-10., -10., 0.],
                max: [10., 10., 10.],
            },
            a,
        );
        stock.cut(
            [0., 0., 8.],
            [0., 0., 2.],
            2.,
            20.,
            CutterShape::Flat,
            a,
            (0, 0),
        );
        let before = stock.occupied.clone();
        let count = stock.remaining;
        let _rotated = stock.transformed_bounds(b);
        assert_eq!(before, stock.occupied);
        assert_eq!(count, stock.remaining);
        // The same removed cells are still empty when observed from the new frame.
        for (i, occupied) in stock
            .occupied
            .iter()
            .enumerate()
            .filter(|(_, v)| **v == 0)
            .take(50)
        {
            let voxel = [
                i % stock.n[0],
                (i / stock.n[0]) % stock.n[1],
                i / (stock.n[0] * stock.n[1]),
            ];
            let p = stock.point(voxel, 0.5);
            let roundtrip = b.transfer(a.transfer(p, b), a);
            assert!((0..3).all(|k| (p[k] - roundtrip[k]).abs() < 0.00001));
            assert_eq!(*occupied, 0);
        }
    }
    #[test]
    fn swept_shapes_are_invariant_to_playback_subdivision() {
        for shape in [CutterShape::Flat, CutterShape::Ball, CutterShape::Drill] {
            let (a, _) = frames();
            let mut direct = Volume::new(
                Bounds {
                    min: [-10., -10., 0.],
                    max: [10., 10., 10.],
                },
                a,
            );
            let mut steps = direct.clone();
            let start = [-8., 0., 8.];
            let end = [8., 0., 2.];
            direct.cut(start, end, 2., 15., shape, a, (0, 0));
            for j in 0..20 {
                let p = |j: usize| {
                    std::array::from_fn(|k| start[k] + (end[k] - start[k]) * j as f32 / 20.)
                };
                steps.cut(p(j), p(j + 1), 2., 15., shape, a, (0, 0));
            }
            assert_eq!(direct.occupied, steps.occupied);
            assert!(direct.removed_volume() > 0.);
        }
    }
    #[test]
    fn cuts_record_their_operation_and_never_merge_across_operations() {
        let (a, _) = frames();
        let mut v = Volume::new(
            Bounds {
                min: [-10., -10., 0.],
                max: [10., 10., 10.],
            },
            a,
        );
        let mut x = -8.;
        for source in [(0, 0), (0, 0), (0, 1), (0, 1), (1, 1)] {
            v.cut(
                [x, 0., 6.],
                [x + 2., 0., 6.],
                1.,
                5.,
                CutterShape::Flat,
                a,
                source,
            );
            x += 2.;
        }
        // Collinear consecutive moves of one operation merge, but the next
        // operation always starts a new cut, even when its first move
        // continues the same straight line.
        let sources: Vec<_> = v.cuts.iter().map(|c| c.source).collect();
        assert_eq!(sources, [(0, 0), (0, 1), (1, 1)]);
        assert_eq!(v.cuts[0].a, [-8., 0., 6.]);
        assert_eq!(v.cuts[0].b, [-4., 0., 6.]);
        assert_eq!(v.cuts[1].b, [0., 0., 6.]);
        assert_eq!(v.cuts[2].b, [2., 0., 6.]);
    }
    #[test]
    fn a_drills_point_is_a_cone_under_its_flat_end() {
        // The cone rises tan 31° per mm from the axis, as `cam::holes` takes
        // a blind hole's floor to.
        assert_eq!(POINT_RISE, 31f32.to_radians().tan());
        let drill = CutterShape::Drill;
        // A Ø8 drill plunged to Z2: the axis is cut to the tip, the cone
        // rises 2.4 mm to the rim, and the flutes go on 10 mm up.
        let plunge = |p: [f32; 3]| swept(p, [0., 0., 8.], [0., 0., 2.], 4., 10., drill);
        assert!(plunge([0., 0., 2.]));
        assert!(!plunge([0., 0., 1.99]));
        for (distance, direction) in [(3.9, [1., 0.]), (2., [0., -1.]), (1., [0.6, 0.8])] {
            let at = |z: f32| [distance * direction[0], distance * direction[1], z];
            assert!(!plunge(at(2.)), "{distance}");
            assert!(!plunge(at(2. + POINT_RISE * distance - 0.01)), "{distance}");
            assert!(plunge(at(2. + POINT_RISE * distance + 0.01)), "{distance}");
            assert!(plunge(at(12.)), "{distance}");
            assert!(!plunge(at(18.01)), "{distance}");
        }
        assert!(!plunge([4.1, 0., 8.]));
        // A flat end of the same size takes the cone as well.
        assert!(swept(
            [3.9, 0., 2.],
            [0., 0., 8.],
            [0., 0., 2.],
            4.,
            10.,
            CutterShape::Flat
        ));
        // Along a ramp the cone is lowest where the tip's descent balances
        // its rise, and an end of the move is lowest when the move is
        // steeper than the cone: the lowest cutter sampled over the move.
        for (a, b) in [
            ([-4., 0., 1.], [4., 0., 6.]),
            ([-4., 0., 6.], [4., 0., 1.]),
            ([-4., 0., 3.], [4., 0., 3.]),
            ([-2., 0., 8.], [2., 0., 1.]),
        ] {
            let mut checked = 0;
            for x in -14..15 {
                for y in -7..8 {
                    let xy = [x as f32 * 0.53, y as f32 * 0.47];
                    let lowest = (0..=4000)
                        .map(|j| {
                            let t = j as f32 / 4000.;
                            let d = ((xy[0] - a[0] - t * (b[0] - a[0])).powi(2)
                                + (xy[1] - a[1] - t * (b[1] - a[1])).powi(2))
                            .sqrt();
                            if d <= 3. {
                                a[2] + t * (b[2] - a[2]) + POINT_RISE * d
                            } else {
                                f32::INFINITY
                            }
                        })
                        .fold(f32::INFINITY, f32::min);
                    for z in 0..20 {
                        let p = [xy[0], xy[1], z as f32 * 0.51];
                        if (p[2] - lowest).abs() > 0.02 && p[2] < 9. {
                            assert_eq!(
                                swept(p, a, b, 3., 30., drill),
                                p[2] > lowest,
                                "{a:?}->{b:?} at {p:?}, lowest {lowest}"
                            );
                            checked += 1;
                        }
                    }
                }
            }
            assert!(checked > 500);
        }
    }
    const BAR: Bounds = Bounds {
        min: [-25., -25., 0.],
        max: [25., 25., 100.],
    };
    #[test]
    fn a_round_bar_has_material_only_inside_its_cylinder() {
        let (a, _) = frames();
        let bar = Volume::bar(BAR, a);
        assert!(bar.round && !Volume::new(BAR, a).round);
        let mut inside = 0;
        for z in 0..bar.n[2] {
            for y in 0..bar.n[1] {
                for x in 0..bar.n[0] {
                    let p = bar.point([x, y, z], 0.5);
                    let occupied = bar.occupied[bar.index([x, y, z])] != 0;
                    assert_eq!(occupied, p[0].hypot(p[1]) <= 25., "{p:?}");
                    inside += usize::from(occupied);
                }
            }
        }
        assert_eq!(inside, bar.remaining);
        let volume = bar.remaining as f32 * bar.cell.iter().product::<f32>();
        let exact = std::f32::consts::PI * 25. * 25. * 100.;
        assert!((volume / exact - 1.).abs() < 0.01, "{volume} vs {exact}");
        assert_eq!(bar.removed_volume(), 0.);
        // A box stock is the whole box, as before.
        let block = Volume::stock(BAR, a, false);
        assert_eq!(block.remaining, block.occupied.len());
        assert!(block.occupied.iter().all(|v| *v == 1));
    }
    #[test]
    fn a_revolve_ring_removes_exactly_its_ring_in_its_own_frame() {
        let (a, b) = frames();
        let ring = |p: [f32; 3]| {
            swept(
                p,
                [30., 15., 0.],
                [60., 22., 0.],
                22.,
                0.,
                CutterShape::Revolve,
            )
        };
        // Each bound, inside and out.
        for (p, inside) in [
            ([18., 0., 45.], true),
            ([0., -15., 30.], true),
            ([22. * 0.6, 22. * 0.8, 60.], true),
            ([12., 12., 45.], true),
            ([18., 0., 29.9], false),
            ([18., 0., 60.1], false),
            ([14.9, 0., 45.], false),
            ([0., 22.1, 45.], false),
            ([0., 0., 45.], false),
        ] {
            assert_eq!(ring(p), inside, "{p:?}");
        }
        // In the volume's own frame, and from a frame turned over and moved,
        // whose axis lies 12 mm off the bar's.
        for current in [a, b] {
            let before = Volume::bar(BAR, a);
            let seen = before.transformed_bounds(current);
            let middle = (seen.min[2] + seen.max[2]) / 2.;
            let (z0, z1) = (middle - 15., middle + 15.);
            let ring = |p: [f32; 3]| {
                swept(
                    p,
                    [z0, 15., 0.],
                    [z1, 22., 0.],
                    22.,
                    0.,
                    CutterShape::Revolve,
                )
            };
            let mut after = before.clone();
            // Either order of the bounds is the same ring.
            after.revolve(z1, z0, 22., 15., current, (0, 3));
            let cut = &after.cuts[0];
            assert_eq!(cut.shape, CutterShape::Revolve);
            assert_eq!((cut.a, cut.b), ([z0, 15., 0.], [z1, 22., 0.]));
            assert_eq!((cut.frame, cut.source), (current, (0, 3)));
            let mut removed = 0;
            for i in 0..before.occupied.len() {
                let voxel = [
                    i % before.n[0],
                    (i / before.n[0]) % before.n[1],
                    i / (before.n[0] * before.n[1]),
                ];
                let p = a.transfer(before.point(voxel, 0.5), current);
                let expected = before.occupied[i] != 0 && !ring(p);
                assert_eq!(after.occupied[i] != 0, expected, "{voxel:?}");
                removed += usize::from(before.occupied[i] != after.occupied[i]);
            }
            assert_eq!(after.remaining, before.remaining - removed);
            assert!(removed > 1000, "{removed}");
            // Material is left inside the ring's inner radius, none in it.
            let touches = |r0: f32| {
                after.touches(
                    [z0, r0, 0.],
                    [z1, 22., 0.],
                    22.,
                    0.,
                    CutterShape::Revolve,
                    current,
                )
            };
            assert!(touches(10.));
            assert!(!touches(15.5));
        }
        // Rings never merge, even the same ring twice; an empty ring is no cut.
        let mut v = Volume::bar(BAR, a);
        v.revolve(30., 60., 15., 22., a, (0, 0));
        v.revolve(30., 60., 15., 22., a, (0, 0));
        v.revolve(30., 30., 15., 22., a, (0, 0));
        assert_eq!(v.cuts.len(), 2);
    }
    #[test]
    fn finite_cutter_does_not_erase_stock_above_its_flutes() {
        assert!(swept(
            [0., 0., 3.],
            [0., 0., 2.],
            [0., 0., 2.],
            2.,
            3.,
            CutterShape::Flat
        ));
        assert!(!swept(
            [0., 0., 8.],
            [0., 0., 2.],
            [0., 0., 2.],
            2.,
            3.,
            CutterShape::Flat
        ));
        assert!(!swept(
            [1.9, 0., 2.1],
            [0., 0., 2.],
            [0., 0., 2.],
            2.,
            3.,
            CutterShape::Ball
        ));
    }
}
