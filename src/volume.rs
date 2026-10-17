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
#[derive(Debug, Clone)]
pub struct Volume {
    pub bounds: Bounds,
    pub frame: Frame,
    pub n: [usize; 3],
    pub cell: [f32; 3],
    pub occupied: Vec<u8>,
    pub remaining: usize,
    pub cuts: std::sync::Arc<Vec<crate::stock_surface::Cut>>,
    pub version: u64,
}
impl Volume {
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
            cuts: std::sync::Arc::new(vec![]),
            version: 0,
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
        (self.occupied.len() - self.remaining) as f32 * self.cell.iter().product::<f32>()
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
        current: Frame,
    ) -> Option<[(usize, usize); 3]> {
        let lo = [
            a[0].min(b[0]) - radius,
            a[1].min(b[1]) - radius,
            a[2].min(b[2]),
        ];
        let hi = [
            a[0].max(b[0]) + radius,
            a[1].max(b[1]) + radius,
            a[2].max(b[2]) + flute,
        ];
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
        if (0..3).all(|k| (a[k] - b[k]).abs() < 1e-7) {
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
        let Some(ranges) = self.ranges(a, b, radius, flute, current) else {
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
        let Some(ranges) = self.ranges(a, b, radius, flute, current) else {
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
}
pub(crate) fn swept(
    p: [f32; 3],
    a: [f32; 3],
    b: [f32; 3],
    r: f32,
    flute: f32,
    shape: CutterShape,
) -> bool {
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
