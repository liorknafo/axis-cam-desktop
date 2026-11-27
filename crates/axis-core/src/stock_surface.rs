//! Continuous swept-tool CSG, meshed by dual contouring: on the GPU over a
//! grid of the detail's cell, kept to the bricks the surface can cross
//! (`gpu_mesh`), or else on the CPU with fidget's adaptive octree. Coordinates
//! remain attached to the first setup, across flips. The small occupancy grid
//! in Volume is only a removal-volume estimate; it is never used to construct
//! this surface.
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
    /// Fine enough for a thread's teeth: a 1 mm pitch is about 14 cells on
    /// a 70 mm block.
    Ultra,
}
impl Detail {
    pub const ALL: [Self; 4] = [Self::Draft, Self::Standard, Self::Fine, Self::Ultra];
    pub fn depth(self) -> u8 {
        match self {
            Self::Draft => 7,
            Self::Standard => 8,
            Self::Fine => 9,
            Self::Ultra => 10,
        }
    }
}
impl std::fmt::Display for Detail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Draft => "Draft",
            Self::Standard => "Standard",
            Self::Fine => "Fine",
            Self::Ultra => "Ultra",
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
        // A lathe's rings never merge: each is one pass.
        if self.shape == CutterShape::Revolve
            || self.source != next.source
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
    /// The box, in this cut's frame, of the ring a `Revolve` cut removes:
    /// as wide as its outer radius round the Z axis, from Z `a[0]` to
    /// `b[0]`. None for other cutter shapes.
    pub fn ring(&self) -> Option<Bounds> {
        (self.shape == CutterShape::Revolve).then(|| Bounds {
            min: [-self.b[1], -self.b[1], self.a[0]],
            max: [self.b[1], self.b[1], self.b[0]],
        })
    }
    fn field(&self, reference: Frame) -> Tree {
        let origin = reference.transfer([0.; 3], self.frame);
        let axes = [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]]
            .map(|p| reference.transfer_vector(p, self.frame));
        let xyz = [Tree::x(), Tree::y(), Tree::z()];
        if self.shape == CutterShape::Revolve {
            // The point in the cut's frame, whose Z axis the ring is round.
            let q: [Tree; 3] = std::array::from_fn(|k| {
                (0..3).fold(Tree::from(origin[k]), |v, i| {
                    v + xyz[i].clone() * axes[i][k]
                })
            });
            let radial = (q[0].square() + q[1].square()).sqrt();
            return (Tree::from(self.a[0]) - q[2].clone())
                .max(q[2].clone() - self.b[0])
                .max(Tree::from(self.a[1]) - radial.clone())
                .max(radial - self.b[1]);
        }
        let p: [Tree; 3] = std::array::from_fn(|k| {
            (0..3).fold(Tree::from(origin[k] - self.a[k]), |v, i| {
                v + xyz[i].clone() * axes[i][k]
            })
        });
        let d: [f32; 3] = std::array::from_fn(|k| self.b[k] - self.a[k]);
        let l = (d[0] * d[0] + d[1] * d[1]).sqrt();
        let r = self.radius;
        if let Some((pitch, teeth, cot)) = self.shape.teeth() {
            // As `volume::swept`: the teeth, as far from the move's track as
            // the tip radius, anywhere along its rise (`thread_field`).
            let rho = if l < 1e-6 {
                (p[0].square() + p[1].square()).sqrt()
            } else {
                let u = (p[0].clone() * d[0] + p[1].clone() * d[1]) / l;
                let v = (p[1].clone() * d[0] - p[0].clone() * d[1]) / l;
                let nearest = u.clone().max(0.).min(l);
                ((u - nearest).square() + v.square()).sqrt()
            };
            let h = p[2].clone() - d[2].min(0.);
            let rise = d[2].abs();
            let depth = crate::simulation::THREAD_TOOTH * pitch;
            let mut e: Option<Tree> = None;
            for k in 0..teeth as usize {
                let at = k as f32 * pitch;
                let gap = (Tree::from(at) - h.clone())
                    .max(h.clone() - (at + rise))
                    .max(0.);
                e = Some(match e {
                    Some(e) => e.min(gap),
                    None => gap,
                });
            }
            let e = e.expect("a thread mill has a tooth");
            let ends = (-h.clone()).max(h - ((teeth - 1.) * pitch + rise)).max(0.);
            return (rho - (Tree::from(r) - e * cot).max(r - depth)).max(ends - depth / cot);
        }
        if l < 1e-6 {
            let radial = (p[0].square() + p[1].square()).sqrt() - r;
            let bottom = match self.shape {
                CutterShape::Flat => Tree::from(d[2].min(0.)),
                // Sphere lower hemisphere, with a cylindrical wall above it.
                CutterShape::Ball => {
                    Tree::from(d[2].min(0.) + r)
                        - (Tree::from(r * r) - p[0].square() - p[1].square())
                            .max(0.)
                            .sqrt()
                }
                // The point's cone, rising from the tip, under the wall.
                CutterShape::Drill | CutterShape::Chamfer(..) => {
                    Tree::from(d[2].min(0.))
                        + ((p[0].square() + p[1].square()).sqrt() - self.shape.flat()).max(0.)
                            * self.shape.rise().unwrap()
                }
                // Rings and thread mills return above.
                CutterShape::Revolve | CutterShape::Thread { .. } => Tree::from(d[2].min(0.)),
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

        let lower = match self.shape {
            CutterShape::Ball => {
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
            }
            CutterShape::Drill | CutterShape::Chamfer(..) => {
                let rise = self.shape.rise().unwrap();
                // As the ball, with the point's cone as the lower envelope:
                // lowest where the tip's descent balances the cone's rise,
                // or at an end of the interval under a move steeper than
                // the cone (`volume::swept`).
                if d[2] > 1e-7 {
                    lo = lo.max(((p[2].clone() - self.flute) / d[2]).max(0.).min(1.));
                } else if d[2] < -1e-7 {
                    hi = hi.min(((p[2].clone() - self.flute) / d[2]).max(0.).min(1.));
                }
                let cone = rise * rise * l * l;
                let t = if cone > d[2] * d[2] {
                    u.clone() / l - v.abs() * (d[2] / (l * (cone - d[2] * d[2]).sqrt()))
                } else if d[2] < 0. {
                    hi.clone()
                } else {
                    lo.clone()
                };
                let t = t.max(lo.clone()).min(hi.clone());
                t.clone() * d[2]
                    + (((u - t * l).square() + v.square()).sqrt() - self.shape.flat()).max(0.)
                        * rise
            }
            // Rings and thread mills return above.
            CutterShape::Flat | CutterShape::Revolve | CutterShape::Thread { .. } => {
                if d[2] < 0. {
                    hi.clone() * d[2]
                } else {
                    lo.clone() * d[2]
                }
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
    /// Whether the stock is the round bar in `bounds` (`Volume::round`).
    pub round: bool,
    /// The round bar along X inside its box (`Volume::bar_x`).
    pub bar_x: Option<crate::stock::Round>,
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
    /// The stock `volume` holds, meshed at `detail`.
    pub fn of(volume: &crate::volume::Volume, detail: Detail) -> Self {
        Self {
            bounds: volume.bounds,
            frame: volume.frame,
            cuts: volume.cuts.clone(),
            detail,
            round: volume.round,
            bar_x: volume.bar_x,
        }
    }
    pub(crate) fn field(&self) -> Tree {
        let p = [Tree::x(), Tree::y(), Tree::z()];
        let box_field = if self.round {
            // The bar's cylinder between the box's ends.
            let (centre, radius) = crate::stock::bar(self.bounds);
            ((p[0].clone() - centre[0]).square() + (p[1].clone() - centre[1]).square()).sqrt()
                - radius
        } else {
            Tree::from(f32::NEG_INFINITY)
        };
        let box_field = (0..3)
            .filter(|&k| !self.round || k == 2)
            .fold(box_field, |t, k| {
                t.max(self.bounds.min[k] - p[k].clone())
                    .max(p[k].clone() - self.bounds.max[k])
            });
        // A rotary setup's bar along X (`Volume::bar_x`).
        let box_field = match self.bar_x {
            Some(bar) => {
                let (y, z) = (p[1].clone() - bar.axis[0], p[2].clone() - bar.axis[1]);
                box_field.max((y.square() + z.square()).sqrt() - bar.radius)
            }
            None => box_field,
        };
        if self.cuts.is_empty() {
            return box_field;
        }
        let mut cuts: Vec<_> = self.cuts.iter().map(|cut| cut.field(self.frame)).collect();
        box_field.max(-balanced_union(&mut cuts))
    }
    /// The surface, meshed on the GPU when there is one (`gpu_mesh`), else
    /// by fidget's octree on the CPU (`build_cpu`).
    pub fn build(&self) -> Result<SurfaceMesh, String> {
        match self.build_on_gpu() {
            Some(Ok(mesh)) => return Ok(mesh),
            Some(Err(e)) => eprintln!("GPU stock mesher: {e}; meshing on the CPU."),
            None => {}
        }
        self.build_cpu()
    }
    /// The surface meshed on the GPU (`gpu_mesh`), which keeps the CPU's
    /// cores free: None without a GPU, and for a rotary setup's bar along X
    /// (`bar_x`), which the GPU's field does not have (it has the bar along
    /// Z, `round`).
    pub fn build_on_gpu(&self) -> Option<Result<SurfaceMesh, String>> {
        if self.bar_x.is_some() {
            return None;
        }
        Some(crate::gpu_mesh::shared()?.mesh(self))
    }
    /// The surface meshed by fidget's adaptive octree on the CPU.
    pub fn build_cpu(&self) -> Result<SurfaceMesh, String> {
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
    use crate::simulation::POINT_RISE;
    use fidget::shape::EzShape;
    fn frame() -> Frame {
        let mesh =
            crate::mesh::normalize(vec![[[0., 0., 0.], [20., 0., 0.], [0., 20., 10.]]]).unwrap();
        Frame::new(
            &crate::model::Setup::new(crate::model::Machine::new(3), "G54"),
            &mesh,
        )
    }
    /// A round bar's stock has no material outside the bar: its voxels
    /// there are empty and its surface keeps inside it, touching it.
    #[test]
    fn a_round_bar_stock_is_meshed_and_counted_inside_the_bar() {
        let bounds = Bounds {
            min: [-30., -10., 0.],
            max: [30., 10., 20.],
        };
        let bar = crate::stock::Round {
            axis: [0., 10.],
            radius: 10.,
        };
        let v = crate::volume::Volume::new(bounds, frame()).rounded(Some(bar));
        let share = v.remaining as f32 / v.occupied.len() as f32;
        // π/4 of the box, within the voxels' rounding.
        assert!(
            (share - std::f32::consts::FRAC_PI_4).abs() < 0.03,
            "{share}"
        );
        let mesh = Snapshot::of(&v, Detail::Draft).build().unwrap();
        let r = mesh
            .vertices
            .iter()
            .map(|p| (p[1] - 0.).hypot(p[2] - 10.))
            .fold(0f32, f32::max);
        assert!((r - 10.).abs() <= mesh.cell_mm, "{r}");
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
            round: false,
            bar_x: None,
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
    fn continuous_cutter_fields_match_analytic_swept_solids() {
        let thread = CutterShape::Thread {
            pitch: 1000,
            teeth: 3,
            angle: 60,
        };
        for shape in [
            CutterShape::Flat,
            CutterShape::Ball,
            CutterShape::Drill,
            thread,
        ] {
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
        let coarse = snapshot(vec![c.clone()], Detail::Draft)
            .build_cpu()
            .unwrap();
        let fine = snapshot(vec![c], Detail::Standard).build_cpu().unwrap();
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
        .build_cpu()
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
        for shape in [CutterShape::Flat, CutterShape::Ball, CutterShape::Drill] {
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
    fn ring(frame: Frame) -> Cut {
        Cut {
            a: [3., 4., 0.],
            b: [8., 7., 0.],
            radius: 7.,
            flute: 0.,
            shape: CutterShape::Revolve,
            frame,
            source: (0, 0),
        }
    }
    #[test]
    fn a_revolve_field_is_its_ring_in_its_own_frame() {
        let mesh =
            crate::mesh::normalize(vec![[[0., 0., 0.], [20., 0., 0.], [0., 20., 10.]]]).unwrap();
        let mut setup = crate::model::Setup::new(crate::model::Machine::new(3), "G55");
        for (key, value) in [("ry", "90"), ("rz", "30"), ("tx", "5"), ("ty", "-3")] {
            setup.p.insert(key.into(), value.into());
        }
        let reference = frame();
        for current in [reference, Frame::new(&setup, &mesh)] {
            let c = ring(current);
            assert_eq!(
                c.ring().unwrap(),
                Bounds {
                    min: [-7., -7., 3.],
                    max: [7., 7., 8.]
                }
            );
            let vm = VmShape::from(c.field(reference));
            let tape = vm.ez_point_tape();
            let mut eval = VmShape::new_point_eval();
            let (mut inside, mut outside) = (0, 0);
            for x in -18..19 {
                for y in -18..19 {
                    for z in -2..22 {
                        let p = [x as f32 * 0.53, y as f32 * 0.47, z as f32 * 0.51];
                        let q = current.transfer(p, reference);
                        let (value, _) = eval.eval(&tape, q[0], q[1], q[2]).unwrap();
                        if value.abs() > 0.002 {
                            let swept = crate::volume::swept(p, c.a, c.b, 7., 0., c.shape);
                            assert_eq!(value < 0., swept, "{p:?} {value}");
                            inside += usize::from(swept);
                            outside += usize::from(!swept);
                        }
                    }
                }
            }
            assert!(inside > 500 && outside > 500, "{inside} {outside}");
        }
        // Rings never merge.
        let mut first = ring(reference);
        let mut next = ring(reference);
        next.a = first.b;
        next.b = [13., 7., 0.];
        assert!(!first.extend(&next));
        assert!(
            first.ring().is_some()
                && cut([0.; 3], [1., 0., 0.], CutterShape::Flat)
                    .ring()
                    .is_none()
        );
    }
    #[test]
    fn a_round_bar_meshes_to_its_cylinder_and_end_faces() {
        let bar = Bounds {
            min: [-25., -25., 0.],
            max: [25., 25., 100.],
        };
        let mut v = crate::volume::Volume::bar(bar, frame());
        let whole = Snapshot::of(&v, Detail::Draft).build_cpu().unwrap();
        watertight(&whole);
        let cell = whole.cell_mm;
        let on = |p: &[f32; 3], radius: f32, low: f32| {
            let r = p[0].hypot(p[1]);
            assert!(
                r <= radius + cell && p[2] >= low - cell && p[2] <= 100. + cell,
                "{p:?}"
            );
            (r - radius).abs() < cell || p[2].abs() < cell || (p[2] - 100.).abs() < cell
        };
        assert!(whole.vertices.iter().all(|p| on(p, 25., 0.)));
        let side = whole
            .vertices
            .iter()
            .filter(|p| p[2] > 1. && p[2] < 99.)
            .map(|p| (p[0].hypot(p[1]) - 25.).abs())
            .fold(0., f32::max);
        // Dual contouring puts the vertices of a curved face within a few
        // hundredths of a cell of it (`viewport::on_bar` allows 6 %).
        assert!(
            side < 0.06 * cell,
            "side vertices up to {side} mm off the cylinder"
        );
        // Turned to Ø40 over its top half: the step and the new face, the
        // cylinder below.
        v.revolve(50., 101., 20., 26., frame(), (0, 0));
        let turned = Snapshot::of(&v, Detail::Draft).build_cpu().unwrap();
        watertight(&turned);
        for p in &turned.vertices {
            let r = p[0].hypot(p[1]);
            assert!(r <= 25. + cell, "{p:?}");
            if p[2] > 51. {
                assert!(r <= 20. + cell, "{p:?}");
            } else if p[2] > 1. && p[2] < 49. {
                assert!((r - 25.).abs() < 0.06 * cell, "{p:?}");
            }
        }
        // The box field is unchanged for a box stock.
        let block = Snapshot::of(&crate::volume::Volume::new(bar, frame()), Detail::Draft)
            .build_cpu()
            .unwrap();
        assert!(
            block
                .vertices
                .iter()
                .any(|p| p[0].abs() > 24.99 && p[1].abs() > 24.99)
        );
    }
    #[test]
    fn ball_cutter_mesh_keeps_spherical_bottom_and_closed_surface() {
        let mesh = snapshot(
            vec![cut([0., 0., 12.], [0., 0., 3.], CutterShape::Ball)],
            Detail::Draft,
        )
        .build_cpu()
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
    fn a_drills_mesh_keeps_its_point_cone_and_closed_surface() {
        let mesh = snapshot(
            vec![cut([0., 0., 12.], [0., 0., 3.], CutterShape::Drill)],
            Detail::Standard,
        )
        .build_cpu()
        .unwrap();
        watertight(&mesh);
        // Under the wall the floor is the 118° cone from the tip at Z3 to
        // the rim 1.8 mm higher, and the wall keeps the radius above it.
        let (mut cone, mut wall) = (0, 0);
        for p in &mesh.vertices {
            let distance = (p[0] * p[0] + p[1] * p[1]).sqrt();
            if distance > 0.3 && distance < 2.8 && p[2] > 2.5 && p[2] < 5. {
                assert!(
                    (p[2] - (3. + POINT_RISE * distance)).abs() < 0.03,
                    "cone surface {p:?}"
                );
                cone += 1;
            } else if p[2] > 5.2 && p[2] < 9.8 && p[0].abs() < 4. && p[1].abs() < 4. {
                assert!((distance - 3.).abs() < 0.01, "wall {p:?}");
                wall += 1;
            }
        }
        assert!(
            cone > 30 && wall > 30,
            "{cone} cone and {wall} wall vertices"
        );
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
