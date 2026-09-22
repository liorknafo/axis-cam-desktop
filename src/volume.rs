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
        let angles = ["rx", "ry", "rz"].map(|k| num(&setup.p, k).unwrap_or(0.));
        let placement = Placement::new(mesh, Some(setup));
        Self {
            basis: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]].map(|p| rotation(p, angles)),
            shift: placement
                .transform([0.; 3])
                .map(|v| v / mesh.display_scale()),
        }
    }
    fn map(self, p: [f32; 3]) -> [f32; 3] {
        std::array::from_fn(|k| {
            self.shift[k] + (0..3).map(|i| self.basis[i][k] * p[i]).sum::<f32>()
        })
    }
    fn unmap(self, p: [f32; 3]) -> [f32; 3] {
        std::array::from_fn(|i| {
            (0..3)
                .map(|k| (p[k] - self.shift[k]) * self.basis[i][k])
                .sum()
        })
    }
    pub fn transfer(self, p: [f32; 3], to: Self) -> [f32; 3] {
        to.map(self.unmap(p))
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
    pub fn removed_volume(&self) -> f32 {
        (self.occupied.len() - self.remaining) as f32 * self.cell.iter().product::<f32>()
    }
    pub fn transformed_bounds(&self, to: Frame) -> Bounds {
        let mut bounds = Bounds {
            min: [f32::INFINITY; 3],
            max: [f32::NEG_INFINITY; 3],
        };
        for mask in 0..8 {
            let p = std::array::from_fn(|k| {
                if mask & (1 << k) == 0 {
                    self.bounds.min[k]
                } else {
                    self.bounds.max[k]
                }
            });
            let p = self.frame.transfer(p, to);
            for k in 0..3 {
                bounds.min[k] = bounds.min[k].min(p[k]);
                bounds.max[k] = bounds.max[k].max(p[k]);
            }
        }
        bounds
    }
    pub fn cut(
        &mut self,
        a: [f32; 3],
        b: [f32; 3],
        radius: f32,
        flute: f32,
        shape: CutterShape,
        current: Frame,
    ) {
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
            return;
        }
        if (0..3).all(|k| (a[k] - b[k]).abs() < 1e-7) {
            return;
        }
        let cut = crate::stock_surface::Cut {
            a,
            b,
            radius,
            flute,
            shape,
            frame: current,
        };
        let cuts = std::sync::Arc::make_mut(&mut self.cuts);
        if !cuts.last_mut().is_some_and(|last| last.extend(&cut)) {
            cuts.push(cut);
        }
        self.version = self.version.wrapping_add(1);
        let ranges: [(usize, usize); 3] = std::array::from_fn(|k| {
            (
                ((low[k] - self.bounds.min[k]) / self.cell[k])
                    .floor()
                    .clamp(0., self.n[k] as f32) as usize,
                ((high[k] - self.bounds.min[k]) / self.cell[k])
                    .ceil()
                    .clamp(0., self.n[k] as f32) as usize,
            )
        });
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
    let t = if shape == CutterShape::Ball && l2 > 1e-12 {
        (projection
            - v[2] * (r * r - perp).max(0.).sqrt() / (l2.sqrt() * (l2 + v[2] * v[2]).sqrt()))
        .clamp(lo, hi)
    } else if v[2] < 0. {
        hi
    } else {
        lo
    };
    let radial = (p[0] - a[0] - t * v[0]).powi(2) + (p[1] - a[1] - t * v[1]).powi(2);
    let bottom = a[2]
        + t * v[2]
        + if shape == CutterShape::Ball {
            r - (r * r - radial).max(0.).sqrt()
        } else {
            0.
        };
    p[2] >= bottom - 0.00001
}
#[cfg(test)]
mod tests {
    use super::*;
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
        stock.cut([0., 0., 8.], [0., 0., 2.], 2., 20., CutterShape::Flat, a);
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
    fn swept_flat_and_ball_are_invariant_to_playback_subdivision() {
        for shape in [CutterShape::Flat, CutterShape::Ball] {
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
            direct.cut(start, end, 2., 15., shape, a);
            for j in 0..20 {
                let p = |j: usize| {
                    std::array::from_fn(|k| start[k] + (end[k] - start[k]) * j as f32 / 20.)
                };
                steps.cut(p(j), p(j + 1), 2., 15., shape, a);
            }
            assert_eq!(direct.occupied, steps.occupied);
            assert!(direct.removed_volume() > 0.);
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
