//! Continuous swept-tool CSG, meshed with an adaptive octree and manifold dual
//! contouring. Coordinates remain attached to the first setup, across flips.
//! The small occupancy grid in Volume is only a removal-volume estimate; it is
//! never used to construct this surface. No dense fine-resolution grid exists.
use crate::{mesh::Bounds, simulation::CutterShape, volume::Frame};
use fidget::{
    context::Tree,
    mesh::{Octree, Settings},
    vm::VmShape,
};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Detail {
    Draft,
    #[default]
    Standard,
    Fine,
}
impl Detail {
    pub const ALL: [Self; 3] = [Self::Draft, Self::Standard, Self::Fine];
    pub fn depth(self) -> u8 {
        match self {
            Self::Draft => 7,
            Self::Standard => 8,
            Self::Fine => 9,
        }
    }
}
impl std::fmt::Display for Detail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Draft => "Draft",
            Self::Standard => "Standard",
            Self::Fine => "Fine",
        })
    }
}
#[derive(Debug, Clone, PartialEq)]
pub struct Cut {
    pub a: [f32; 3],
    pub b: [f32; 3],
    pub radius: f32,
    pub flute: f32,
    pub shape: CutterShape,
    pub frame: Frame,
    /// Setup and operation (both from 0, in document order) whose move made
    /// this cut. Cuts of different operations never merge.
    pub source: (u16, u16),
}
impl Cut {
    /// Merge playback fragments only when their union is the same straight
    /// sweep and both come from the same operation.
    pub fn extend(&mut self, next: &Self) -> bool {
        if self.source != next.source
            || self.radius != next.radius
            || self.flute != next.flute
            || self.shape != next.shape
            || self.frame != next.frame
        {
            return false;
        }
        if (0..3).any(|k| (self.b[k] - next.a[k]).abs() > 1e-5) {
            return false;
        }
        let u: [f32; 3] = std::array::from_fn(|k| self.b[k] - self.a[k]);
        let v: [f32; 3] = std::array::from_fn(|k| next.b[k] - next.a[k]);
        let dot: f32 = (0..3).map(|k| u[k] * v[k]).sum();
        let u2: f32 = u.iter().map(|x| x * x).sum();
        let v2: f32 = v.iter().map(|x| x * x).sum();
        let cross = [
            u[1] * v[2] - u[2] * v[1],
            u[2] * v[0] - u[0] * v[2],
            u[0] * v[1] - u[1] * v[0],
        ];
        if dot < -1e-6 || cross.iter().map(|x| x * x).sum::<f32>() > 1e-10 * (u2 * v2).max(1.) {
            return false;
        }
        self.b = next.b;
        true
    }
    fn field(&self, reference: Frame) -> Tree {
        let origin = reference.transfer([0.; 3], self.frame);
        let axes = [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]]
            .map(|p| reference.transfer_vector(p, self.frame));
        let xyz = [Tree::x(), Tree::y(), Tree::z()];
        let p: [Tree; 3] = std::array::from_fn(|k| {
            (0..3).fold(Tree::from(origin[k] - self.a[k]), |v, i| {
                v + xyz[i].clone() * axes[i][k]
            })
        });
        let d: [f32; 3] = std::array::from_fn(|k| self.b[k] - self.a[k]);
        let l = (d[0] * d[0] + d[1] * d[1]).sqrt();
        let r = self.radius;
        if l < 1e-6 {
            let radial = (p[0].square() + p[1].square()).sqrt() - r;
            let bottom = if self.shape == CutterShape::Flat {
                Tree::from(d[2].min(0.))
            } else {
                // Sphere lower hemisphere, with a cylindrical wall above it.
                Tree::from(d[2].min(0.) + r)
                    - (Tree::from(r * r) - p[0].square() - p[1].square())
                        .max(0.)
                        .sqrt()
            };
            return radial
                .max(bottom - p[2].clone())
                .max(p[2].clone() - (d[2].max(0.) + self.flute));
        }
        let u = (p[0].clone() * d[0] + p[1].clone() * d[1]) / l;
        let v = (p[1].clone() * d[0] - p[0].clone() * d[1]) / l;
        let nearest = u.max(0.).min(l);
        let radial = ((u.clone() - nearest).square() + v.square()).sqrt() - r;
        // At this XY position the disc intersects a closed interval of tool
        // travel. Min/max tip height over that interval gives the exact flat
        // cylinder sweep, including sloping ramps, without stamping samples.
        let half = (Tree::from(r * r) - v.square()).max(0.).sqrt();
        let mut lo = ((u.clone() - half.clone()) / l).max(0.).min(1.);
        let mut hi = ((u.clone() + half.clone()) / l).max(0.).min(1.);

        let lower = if self.shape == CutterShape::Ball {
            // Constrain the minimizer to instants where the point is below the
            // finite flute top, then solve the spherical lower envelope.
            if d[2] > 1e-7 {
                lo = lo.max(((p[2].clone() - self.flute) / d[2]).max(0.).min(1.));
            } else if d[2] < -1e-7 {
                hi = hi.min(((p[2].clone() - self.flute) / d[2]).max(0.).min(1.));
            }
            let t = (u.clone() / l - half * (d[2] / (l * (l * l + d[2] * d[2]).sqrt())))
                .max(lo.clone())
                .min(hi.clone());
            t.clone() * d[2] + r
                - (Tree::from(r * r) - (u - t * l).square() - v.square())
                    .max(0.)
                    .sqrt()
        } else {
            if d[2] < 0. {
                hi.clone() * d[2]
            } else {
                lo.clone() * d[2]
            }
        };
        let upper = if d[2] < 0. {
            lo * d[2] + self.flute
        } else {
            hi * d[2] + self.flute
        };
        radial.max(lower - p[2].clone()).max(p[2].clone() - upper)
    }
}
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub bounds: Bounds,
    pub frame: Frame,
    pub cuts: Arc<Vec<Cut>>,
    pub detail: Detail,
}
#[derive(Debug)]
pub struct SurfaceMesh {
    pub vertices: Vec<[f32; 3]>,
    pub triangles: Vec<[usize; 3]>,
    pub cell_mm: f32,
    pub milliseconds: u128,
}
fn balanced_union(trees: &mut [Tree]) -> Tree {
    if trees.len() == 1 {
        return trees[0].clone();
    }
    let (a, b) = trees.split_at_mut(trees.len() / 2);
    balanced_union(a).min(balanced_union(b))
}
impl Snapshot {
    fn field(&self) -> Tree {
        let p = [Tree::x(), Tree::y(), Tree::z()];
        let box_field = (0..3).fold(Tree::from(f32::NEG_INFINITY), |t, k| {
            t.max(self.bounds.min[k] - p[k].clone())
                .max(p[k].clone() - self.bounds.max[k])
        });
        if self.cuts.is_empty() {
            return box_field;
        }
        let mut cuts: Vec<_> = self.cuts.iter().map(|cut| cut.field(self.frame)).collect();
        box_field.max(-balanced_union(&mut cuts))
    }
    pub fn build(&self) -> Result<SurfaceMesh, String> {
        let start = std::time::Instant::now();
        let size = self.bounds.dimensions();
        let span = size.into_iter().fold(0., f32::max) * 1.04;
        if !span.is_finite() || span <= 0. {
            return Err("Stock bounds are invalid.".into());
        }
        let mut settings = Settings {
            depth: self.detail.depth(),
            ..Default::default()
        };
        for k in 0..3 {
            settings.world_to_model[(k, k)] = span / 2.;
            settings.world_to_model[(k, 3)] = (self.bounds.min[k] + self.bounds.max[k]) / 2.;
        }
        let shape = VmShape::from(self.field());
        let octree =
            Octree::build(&shape, &settings).ok_or("Stock surface update was cancelled.")?;
        let mesh = octree.walk_dual();
        if mesh
            .vertices
            .iter()
            .any(|v| v.iter().any(|x| !x.is_finite()))
        {
            return Err(
                "Stock surface contains non-finite vertices. Try a different detail level.".into(),
            );
        }
        Ok(SurfaceMesh {
            vertices: mesh.vertices.iter().map(|v| [v.x, v.y, v.z]).collect(),
            triangles: mesh.triangles.iter().map(|v| [v.x, v.y, v.z]).collect(),
            cell_mm: span / (1u32 << self.detail.depth()) as f32,
            milliseconds: start.elapsed().as_millis(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fidget::shape::EzShape;
    fn frame() -> Frame {
        let mesh =
            crate::mesh::normalize(vec![[[0., 0., 0.], [20., 0., 0.], [0., 20., 10.]]]).unwrap();
        Frame::new(
            &crate::model::Setup::new(crate::model::Machine::new(3), "G54"),
            &mesh,
        )
    }
    fn snapshot(cuts: Vec<Cut>, detail: Detail) -> Snapshot {
        Snapshot {
            bounds: Bounds {
                min: [-10., -10., 0.],
                max: [10., 10., 10.],
            },
            frame: frame(),
            cuts: Arc::new(cuts),
            detail,
        }
    }
    fn cut(a: [f32; 3], b: [f32; 3], shape: CutterShape) -> Cut {
        Cut {
            a,
            b,
            radius: 3.,
            flute: 12.,
            shape,
            frame: frame(),
            source: (0, 0),
        }
    }
    #[test]
    fn cuts_merge_only_within_one_operation() {
        let mut first = cut([0., 0., 3.], [2., 0., 3.], CutterShape::Flat);
        let mut next = cut([2., 0., 3.], [4., 0., 3.], CutterShape::Flat);
        assert!(first.extend(&next));
        assert_eq!(first.b, [4., 0., 3.]);
        assert_eq!(first.source, (0, 0));
        for other in [(0, 1), (1, 0), (1, 1)] {
            let mut first = cut([0., 0., 3.], [2., 0., 3.], CutterShape::Flat);
            next.source = other;
            assert!(!first.extend(&next), "{other:?}");
            assert_eq!(first.b, [2., 0., 3.], "{other:?}");
        }
    }
    #[test]
    fn continuous_flat_and_ball_fields_match_analytic_swept_solids() {
        for shape in [CutterShape::Flat, CutterShape::Ball] {
            for (a, b) in [
                ([-4., 0., 2.], [4., 0., 2.]),
                ([-4., 0., 1.], [4., 0., 6.]),
                ([-4., 0., 6.], [4., 0., 1.]),
                ([0., 0., 8.], [0., 0., 1.]),
                ([0., 0., 1.], [0., 0., 8.]),
            ] {
                let c = cut(a, b, shape);
                let vm = VmShape::from(c.field(frame()));
                let tape = vm.ez_point_tape();
                let mut eval = VmShape::new_point_eval();
                for x in -15..16 {
                    for y in -8..9 {
                        for z in -2..45 {
                            let p = [x as f32 * 0.53, y as f32 * 0.47, z as f32 * 0.51];
                            let (value, _) = eval.eval(&tape, p[0], p[1], p[2]).unwrap();
                            if value.abs() > 0.001 {
                                assert_eq!(
                                    value < 0.,
                                    crate::volume::swept(p, a, b, c.radius, c.flute, shape),
                                    "{shape:?} {a:?}->{b:?} at {p:?}: {value}"
                                );
                            }
                        }
                    }
                }
            }
        }
    }
    fn watertight(mesh: &SurfaceMesh) {
        let mut edges = std::collections::HashMap::new();
        for t in &mesh.triangles {
            for i in 0..3 {
                let a = t[i];
                let b = t[(i + 1) % 3];
                *edges.entry((a.min(b), a.max(b))).or_insert(0) += 1;
            }
        }
        assert!(!edges.is_empty());
        assert!(edges.values().all(|n| *n == 2));
    }
    #[test]
    fn octree_mesh_retains_round_tool_radius_and_exact_flat_faces() {
        let c = cut([0., 0., 12.], [0., 0., 3.], CutterShape::Flat);
        let coarse = snapshot(vec![c.clone()], Detail::Draft).build().unwrap();
        let fine = snapshot(vec![c], Detail::Standard).build().unwrap();
        watertight(&coarse);
        watertight(&fine);
        let error = |mesh: &SurfaceMesh| {
            let wall: Vec<_> = mesh
                .vertices
                .iter()
                .filter(|p| p[2] > 3.2 && p[2] < 9.8 && p[0].abs() < 4. && p[1].abs() < 4.)
                .collect();
            assert!(wall.len() > 30);
            wall.iter()
                .map(|p| ((p[0] * p[0] + p[1] * p[1]).sqrt() - 3.).abs())
                .fold(0., f32::max)
        };
        assert!(error(&fine) < 0.01, "fine radius error {}", error(&fine));
        assert!(error(&fine) < error(&coarse));
        // Standard uses a 256^3 virtual domain, without allocating those cells.
        assert!(fine.vertices.len() < 256usize.pow(3) / 100);
        assert!(
            fine.vertices
                .iter()
                .filter(|p| p[0].abs() < 2. && p[1].abs() < 2.)
                .all(|p| p[2].abs() < 0.001 || (p[2] - 3.).abs() < 0.001)
        );
    }
    #[test]
    fn sloped_cut_surface_is_a_plane_without_voxel_stair_steps() {
        let mesh = snapshot(
            vec![cut([-12., 0., 2.], [12., 0., 7.], CutterShape::Flat)],
            Detail::Standard,
        )
        .build()
        .unwrap();
        watertight(&mesh);
        let mut checked = 0;
        for p in &mesh.vertices {
            if p[0].abs() < 8. && p[1].abs() < 2. && p[2] > 0.5 && p[2] < 8. {
                let half = (9. - p[1] * p[1]).sqrt();
                let expected = 2. + (p[0] + 12. - half) / 24. * 5.;
                assert!((p[2] - expected).abs() < 0.02, "{p:?} expected {expected}");
                checked += 1;
            }
        }
        assert!(checked > 20);
    }
    #[test]
    fn continuous_cut_field_follows_rotated_and_translated_setup() {
        let mesh =
            crate::mesh::normalize(vec![[[0., 0., 0.], [20., 0., 0.], [0., 20., 10.]]]).unwrap();
        let mut setup = crate::model::Setup::new(crate::model::Machine::new(3), "G55");
        for (key, value) in [
            ("rx", "180"),
            ("ry", "35"),
            ("rz", "70"),
            ("tx", "12"),
            ("ty", "-7"),
        ] {
            setup.p.insert(key.into(), value.into());
        }
        let current = Frame::new(&setup, &mesh);
        let reference = frame();
        for shape in [CutterShape::Flat, CutterShape::Ball] {
            let mut c = cut([-4., 0., 2.], [4., 0., 6.], shape);
            c.frame = current;
            let vm = VmShape::from(c.field(reference));
            let tape = vm.ez_point_tape();
            let mut eval = VmShape::new_point_eval();
            for x in -12..13 {
                for y in -8..9 {
                    for z in 0..35 {
                        let p = [x as f32 * 0.63, y as f32 * 0.47, z as f32 * 0.51];
                        let q = current.transfer(p, reference);
                        let (value, _) = eval.eval(&tape, q[0], q[1], q[2]).unwrap();
                        if value.abs() > 0.002 {
                            assert_eq!(
                                value < 0.,
                                crate::volume::swept(p, c.a, c.b, c.radius, c.flute, c.shape),
                                "{p:?} {q:?} {value}"
                            );
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn ball_cutter_mesh_keeps_spherical_bottom_and_closed_surface() {
        let mesh = snapshot(
            vec![cut([0., 0., 12.], [0., 0., 3.], CutterShape::Ball)],
            Detail::Draft,
        )
        .build()
        .unwrap();
        watertight(&mesh);
        let mut count = 0;
        for p in &mesh.vertices {
            if p[2] > 3.05 && p[2] < 5.7 && p[0].abs() < 3.1 && p[1].abs() < 3.1 {
                let radius = (p[0] * p[0] + p[1] * p[1] + (p[2] - 6.).powi(2)).sqrt();
                assert!(
                    (radius - 3.).abs() < 0.03,
                    "ball surface {p:?}, radius {radius}"
                );
                count += 1;
            }
        }
        assert!(count > 30);
    }
    #[test]
    fn cut_history_is_shared_and_playback_fragments_coalesce() {
        let f = frame();
        let mut v = crate::volume::Volume::new(snapshot(vec![], Detail::Standard).bounds, f);
        let initial = v.clone();
        for i in 0..100 {
            v.cut(
                [-5. + i as f32 * 0.1, 0., 3.],
                [-5. + (i + 1) as f32 * 0.1, 0., 3.],
                3.,
                12.,
                CutterShape::Flat,
                f,
                (0, 0),
            );
        }
        assert_eq!(v.cuts.len(), 1);
        assert!(initial.cuts.is_empty());
        let inherited = v.clone();
        assert!(Arc::ptr_eq(&v.cuts, &inherited.cuts));
        v.cut(
            [5., 0., 3.],
            [5., 5., 3.],
            3.,
            12.,
            CutterShape::Flat,
            f,
            (0, 0),
        );
        assert_eq!(v.cuts.len(), 2);
        assert_eq!(inherited.cuts.len(), 1);
    }
}
