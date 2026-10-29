//! What a job leaves uncut: the faces of the part that the stock its last
//! setup leaves still covers. Each face is sampled just outside itself, and
//! each sample tested against the cuts that made that stock as they swept it
//! (`volume::swept`), not against the coarse grid.
use crate::{
    mesh::{Bounds, Mesh},
    model::Setup,
    stock::Placement,
    volume::{Frame, Volume},
};
use std::collections::HashMap;

/// The existing magenta threshold (mm), kept for completeness checks.
const OFFSET: f32 = 0.5;
/// Ignore residuals below 0.1 mm: cutter safety margins, tessellation/arc
/// chords and fine finishing scallops. This still resolves the default
/// 0.2 mm axial and 0.3 mm radial allowances (see fixture measurements below).
const THIN_OFFSET: f32 = 0.1;
/// The longest side a sampled piece of a face has (mm): longer faces are
/// sampled in pieces.
const PIECE: f32 = 5.;
/// The longest edge a triangle has when it is sampled (mm): `PIECE` times the
/// 8 pieces a side is cut in at most. Longer edges are split first.
const SPLIT: f32 = 8. * PIECE;
/// How often a face is halved at most. That bounds a face to 2⁶ triangles of
/// 64 samples, 4096 sample positions (at most two stock queries each), and
/// the background run to that many per face of the mesh. A triangle needing
/// more splits (over about 300 mm across) is sampled in coarser pieces.
const SPLITS: u32 = 6;
/// The buckets of cuts along each axis of the stock.
const BUCKETS: usize = 24;
/// The sides of the part's box, in setup 1's axes, negative side first.
const SIDES: [&str; 6] = ["−X", "+X", "−Y", "+Y", "the bottom", "the top"];
/// Regions smaller than this (mm²) are left out: the slivers a round cutter
/// leaves in a sharp inside corner.
const SLIVER: f32 = 2.;

/// A piece of a face (`Uncut::pieces`): its corners, in the mesh's own
/// coordinates as `Mesh::faces` has them, and whether it is under stock.
pub type Piece = ([[f32; 3]; 3], bool);
/// The faces of the part still under stock.
#[derive(Debug, Clone, Default)]
pub struct Uncut {
    /// Per face of the mesh (`Mesh::faces`): whether any of it is still under
    /// stock: all of it, or where `pieces` says.
    pub faces: Vec<bool>,
    /// Per face only partly under stock: the pieces it is sampled in, which
    /// make up the whole face, so it is drawn piece by piece and magenta only
    /// where the stock is. Empty for every other face (or all of them).
    pub pieces: Vec<Vec<Piece>>,
    /// Thin stock, sampled at `THIN_OFFSET` but not at `OFFSET`. Display only:
    /// the existing faces, regions and share retain their magenta meaning.
    pub thin: Coverage,
    /// Those faces, joined where they share an edge, largest first, of the
    /// area under stock. Those of regions under `SLIVER` are left out, here,
    /// in `faces` and in `share`.
    pub regions: Vec<Region>,
    /// The share of the part's surface still under stock, from 0 to 1.
    pub share: f32,
}
/// One disjoint category of stock-covered surface, with the same piece and
/// region rules as the existing magenta category.
#[derive(Debug, Clone, Default)]
pub struct Coverage {
    pub faces: Vec<bool>,
    pub pieces: Vec<Vec<Piece>>,
    pub regions: Vec<Region>,
    pub share: f32,
}
pub static EMPTY: Coverage = Coverage {
    faces: vec![],
    pieces: vec![],
    regions: vec![],
    share: 0.,
};
impl Uncut {
    pub fn any(&self) -> bool {
        self.faces.iter().chain(&self.thin.faces).any(|f| *f)
    }
}
type Tri = [[f32; 3]; 3];
#[derive(Default)]
struct Samples {
    faces: Vec<bool>,
    pieces: Vec<Vec<Piece>>,
    found: HashMap<usize, (f32, Tri, Vec<Tri>)>,
    covered: f32,
}
impl Samples {
    fn new(n: usize) -> Self {
        Self {
            faces: vec![false; n],
            pieces: vec![vec![]; n],
            ..Self::default()
        }
    }
}
/// Faces still under stock that share edges.
#[derive(Debug, Clone, PartialEq)]
pub struct Region {
    /// mm².
    pub area: f32,
    /// In setup 1's axes, mm, from the part's -X/-Y/bottom corner.
    pub bounds: Bounds,
    /// The sides of the part's box it is reached from (`SIDES`): those its
    /// faces lie on, else those its edges meet, as a hole's or slot's meet
    /// the side it opens onto. Empty inside the box.
    pub from: Vec<&'static str>,
}
impl Region {
    /// "120 mm² reached from −Y: X 48–72, Y 0–14, Z 8–32".
    pub fn line(&self) -> String {
        let b = self.bounds;
        let span = |k: usize| format!("{:.0}–{:.0}", b.min[k].max(0.), b.max[k].max(0.));
        let at = format!("X {}, Y {}, Z {}", span(0), span(1), span(2));
        if self.from.is_empty() {
            format!("{:.0} mm² inside: {at}", self.area)
        } else {
            format!(
                "{:.0} mm² reached from {}: {at}",
                self.area,
                self.from.join(", ")
            )
        }
    }
}
/// The faces of `mesh`, placed as `setup1` places it, that `stock` still
/// covers: `stock` is the stock the job's last setup leaves, in setup 1's
/// frame as `simulation::sequence_walk` keeps it.
pub fn uncut(stock: &Volume, setup1: &Setup, mesh: &Mesh) -> Uncut {
    let placement = Placement::new(mesh, Some(setup1));
    let scale = mesh.display_scale();
    let part = placement.bounds.scaled(1. / scale);
    let first = Frame::new(setup1, mesh);
    let cuts = Cuts::new(stock);
    let mut layers = [
        Samples::new(mesh.faces.len()),
        Samples::new(mesh.faces.len()),
    ];
    let mut total = 0.;
    for (i, face) in mesh.faces.iter().enumerate() {
        let (raw, [a, b, c]) = match face.points[..] {
            [a, b, c] => (
                [a, b, c],
                [a, b, c].map(|p| placement.transform(p).map(|v| v / scale)),
            ),
            _ => continue,
        };
        let (u, v) = (sub(b, a), sub(c, a));
        let n = [
            u[1] * v[2] - u[2] * v[1],
            u[2] * v[0] - u[0] * v[2],
            u[0] * v[1] - u[1] * v[0],
        ];
        let twice = n.iter().map(|x| x * x).sum::<f32>().sqrt();
        if twice < 1e-9 {
            continue;
        }
        let n = n.map(|x| x / twice);
        // ponytail: trusts the mesh's winding for which side is outside; a
        // face wound inwards samples the part and reads as under stock.
        // The face in triangles with no edge over `SPLIT`, each sampled in
        // pieces of at most `PIECE`, all the same size within a triangle. A
        // triangle under stock or cut all over is kept whole.
        let mut parts = vec![];
        split([a, b, c], 0, &mut parts);
        let mut drawn: [Vec<Piece>; 2] = [vec![], vec![]];
        for [a, b, c] in parts {
            let (u, v) = (sub(b, a), sub(c, a));
            let k = (longest(a, b, c) / PIECE).ceil().clamp(1., 8.) as usize;
            let at = |s: f32, t: f32| -> [f32; 3] {
                std::array::from_fn(|q| a[q] + s / k as f32 * u[q] + t / k as f32 * v[q])
            };
            let mut cells = vec![];
            // The k² pieces of the part, each sampled at its centre: k(k+1)/2
            // with a corner at its first corner, and the k(k-1)/2 turned the
            // other way.
            for i in 0..k {
                for j in 0..k - i {
                    let (x, y) = (i as f32, j as f32);
                    let turned = [at(x + 1., y), at(x + 1., y + 1.), at(x, y + 1.)];
                    for (shift, piece) in [
                        (1. / 3., Some([at(x, y), at(x + 1., y), at(x, y + 1.)])),
                        (2. / 3., (i + j + 1 < k).then_some(turned)),
                    ] {
                        let Some(piece) = piece else { continue };
                        let centre = at(x + shift, y + shift);
                        let p: [f32; 3] = std::array::from_fn(|q| centre[q] + n[q] * OFFSET);
                        let thick = cuts.left(moved(p, first, stock.frame));
                        let thin = !thick
                            && cuts.left(moved(
                                std::array::from_fn(|q| centre[q] + n[q] * THIN_OFFSET),
                                first,
                                stock.frame,
                            ));
                        cells.push((piece, [thick, thin]));
                    }
                }
            }
            let uniform = cells.iter().all(|(_, under)| *under == cells[0].1);
            for (layer, drawn) in drawn.iter_mut().enumerate() {
                if uniform {
                    drawn.push(([a, b, c], cells[0].1[layer]));
                } else {
                    drawn.extend(cells.iter().map(|(t, under)| (*t, under[layer])));
                }
            }
        }
        let area = twice / 2.;
        total += area;
        for (layer, drawn) in layers.iter_mut().zip(drawn) {
            let under: Vec<_> = drawn
                .iter()
                .filter(|(_, under)| *under)
                .map(|(t, _)| *t)
                .collect();
            if under.is_empty() {
                continue;
            }
            let left: f32 = under
                .iter()
                .map(|t| cross(sub(t[1], t[0]), sub(t[2], t[0])) / 2.)
                .sum();
            layer.covered += left;
            layer.faces[i] = true;
            if under.len() < drawn.len() {
                layer.pieces[i] = drawn
                    .into_iter()
                    .map(|(t, under)| (t.map(|q| carried(q, [a, b, c], raw)), under))
                    .collect();
                layer.found.insert(i, (left, [a, b, c], under));
            } else {
                layer.found.insert(i, (area, [a, b, c], vec![[a, b, c]]));
            }
        }
    }
    let [thick, thin] = layers.map(|layer| coverage(layer, part, total));
    Uncut {
        faces: thick.faces,
        pieces: thick.pieces,
        regions: thick.regions,
        share: thick.share,
        thin,
    }
}
/// Join and filter each category independently, so a corner sliver cannot
/// become a warning merely because it is thinner than the magenta threshold.
fn coverage(samples: Samples, part: Bounds, total: f32) -> Coverage {
    let Samples {
        mut faces,
        mut pieces,
        found,
        mut covered,
    } = samples;
    // Faces under stock that share an edge make one region.
    let mut root: HashMap<usize, usize> = found.keys().map(|i| (*i, *i)).collect();
    fn top(root: &mut HashMap<usize, usize>, mut i: usize) -> usize {
        while root[&i] != i {
            i = root[&i];
        }
        i
    }
    // ponytail: a face only partly under stock joins its neighbours by all
    // its edges, also those its pieces under stock do not reach.
    let mut edges: HashMap<[[i64; 3]; 2], usize> = HashMap::new();
    for (&i, (_, p, _)) in &found {
        for e in 0..3 {
            let mut key = [p[e], p[(e + 1) % 3]].map(|v| v.map(|x| (x * 1000.).round() as i64));
            key.sort_unstable();
            match edges.get(&key) {
                Some(&other) => {
                    let (a, b) = (top(&mut root, i), top(&mut root, other));
                    root.insert(a, b);
                }
                None => {
                    edges.insert(key, i);
                }
            }
        }
    }
    // Per region: its faces, area, bounds, and the sides of the part's box
    // its faces lie on and its corners meet (`SIDES` order), of what of its
    // faces is under stock.
    struct Group {
        faces: Vec<usize>,
        area: f32,
        bounds: Bounds,
        on: [bool; 6],
        meets: [bool; 6],
    }
    let mut grouped: HashMap<usize, Group> = HashMap::new();
    for (&i, (area, _, under)) in &found {
        let group = grouped.entry(top(&mut root, i)).or_insert(Group {
            faces: vec![],
            area: 0.,
            bounds: Bounds {
                min: [f32::INFINITY; 3],
                max: [f32::NEG_INFINITY; 3],
            },
            on: [false; 6],
            meets: [false; 6],
        });
        group.faces.push(i);
        group.area += area;
        for p in under {
            for v in p {
                for (k, &x) in v.iter().enumerate().take(3) {
                    group.bounds.min[k] = group.bounds.min[k].min(x - part.min[k]);
                    group.bounds.max[k] = group.bounds.max[k].max(x - part.min[k]);
                }
            }
            for side in 0..6 {
                let (k, edge) = (side / 2, [part.min, part.max][side % 2][side / 2]);
                let at = |v: &[f32; 3]| (v[k] - edge).abs() < 0.01;
                group.on[side] |= p.iter().all(at);
                group.meets[side] |= p.iter().any(at);
            }
        }
    }
    let mut regions = vec![];
    for group in grouped.into_values() {
        if group.area < SLIVER {
            covered -= group.area;
            for i in group.faces {
                faces[i] = false;
                pieces[i].clear();
            }
            continue;
        }
        let sides = if group.on.contains(&true) {
            group.on
        } else {
            group.meets
        };
        regions.push(Region {
            area: group.area,
            bounds: group.bounds,
            from: (0..6).filter(|d| sides[*d]).map(|d| SIDES[d]).collect(),
        });
    }
    regions.sort_by(|a, b| b.area.total_cmp(&a.area));
    Coverage {
        faces,
        pieces,
        regions,
        share: if total > 0. {
            covered.max(0.) / total
        } else {
            0.
        },
    }
}
/// `p` in frame `from` as frame `to` sees it.
fn moved(p: [f32; 3], from: Frame, to: Frame) -> [f32; 3] {
    if from == to { p } else { from.transfer(p, to) }
}
/// `q`, a point of the triangle `from`, at the same place in the triangle
/// `to`, which `from` is a moved copy of.
fn carried(q: [f32; 3], from: [[f32; 3]; 3], to: [[f32; 3]; 3]) -> [f32; 3] {
    let dot = |x: [f32; 3], y: [f32; 3]| (0..3).map(|k| x[k] * y[k]).sum::<f32>();
    let (u, v, d) = (
        sub(from[1], from[0]),
        sub(from[2], from[0]),
        sub(q, from[0]),
    );
    let (uu, uv, vv, du, dv) = (dot(u, u), dot(u, v), dot(v, v), dot(d, u), dot(d, v));
    let den = uu * vv - uv * uv;
    let (s, t) = ((vv * du - uv * dv) / den, (uu * dv - uv * du) / den);
    std::array::from_fn(|k| to[0][k] + s * (to[1][k] - to[0][k]) + t * (to[2][k] - to[0][k]))
}
fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|k| a[k] - b[k])
}
/// The length of the cross product of `u` and `v`: twice the triangle's area.
fn cross(u: [f32; 3], v: [f32; 3]) -> f32 {
    let n = [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ];
    n.iter().map(|x| x * x).sum::<f32>().sqrt()
}
fn longest(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> f32 {
    [sub(b, a), sub(c, b), sub(a, c)]
        .map(|e| e.iter().map(|x| x * x).sum::<f32>().sqrt())
        .into_iter()
        .fold(0., f32::max)
}
/// `tri` as triangles with no edge over `SPLIT`, halved across the longest
/// edge (the winding kept) up to `SPLITS` times.
fn split(tri: [[f32; 3]; 3], depth: u32, out: &mut Vec<[[f32; 3]; 3]>) {
    let edge = |e: usize| {
        let d = sub(tri[(e + 1) % 3], tri[e]);
        d.iter().map(|x| x * x).sum::<f32>()
    };
    let e = (0..3).fold(0, |best, e| if edge(e) > edge(best) { e } else { best });
    if depth == SPLITS || edge(e) <= SPLIT * SPLIT {
        out.push(tri);
        return;
    }
    let (p, q, r) = (tri[e], tri[(e + 1) % 3], tri[(e + 2) % 3]);
    let m = std::array::from_fn(|k| (p[k] + q[k]) / 2.);
    split([p, m, r], depth + 1, out);
    split([m, q, r], depth + 1, out);
}
/// The cuts that made a stock, bucketed by where they can have removed
/// material, so a point is tested against the few that reach it.
struct Cuts<'a> {
    stock: &'a Volume,
    buckets: Vec<Vec<u32>>,
}
impl<'a> Cuts<'a> {
    fn new(stock: &'a Volume) -> Self {
        let mut buckets = vec![vec![]; BUCKETS.pow(3)];
        let boxes = crate::rest::removal_boxes(stock, stock.frame);
        for (i, b) in boxes.iter().enumerate() {
            let range = |k: usize| {
                let at = |v: f32| {
                    ((v - stock.bounds.min[k]) / stock.bounds.dimensions()[k] * BUCKETS as f32)
                        .floor()
                        .clamp(0., BUCKETS as f32 - 1.) as usize
                };
                at(b.min[k])..=at(b.max[k])
            };
            if (0..3).any(|k| b.max[k] < stock.bounds.min[k] || b.min[k] > stock.bounds.max[k]) {
                continue;
            }
            for z in range(2) {
                for y in range(1) {
                    for x in range(0) {
                        buckets[(z * BUCKETS + y) * BUCKETS + x].push(i as u32);
                    }
                }
            }
        }
        Self { stock, buckets }
    }
    /// Whether stock is left at `p` (the stock's frame): inside the block it
    /// started as (the round bar, when it is one), and swept by none of its
    /// cuts.
    fn left(&self, p: [f32; 3]) -> bool {
        let b = self.stock.bounds;
        if (0..3).any(|k| p[k] < b.min[k] || p[k] > b.max[k])
            || self.stock.bar_x.is_some_and(|bar| !bar.contains(p))
        {
            return false;
        }
        let cell: [usize; 3] = std::array::from_fn(|k| {
            ((p[k] - b.min[k]) / b.dimensions()[k] * BUCKETS as f32).clamp(0., BUCKETS as f32 - 1.)
                as usize
        });
        !self.buckets[(cell[2] * BUCKETS + cell[1]) * BUCKETS + cell[0]]
            .iter()
            .any(|&i| {
                let cut = &self.stock.cuts[i as usize];
                let q = moved(p, self.stock.frame, cut.frame);
                crate::volume::swept(q, cut.a, cut.b, cut.radius, cut.flute, cut.shape)
            })
    }
}

/// A face 100 mm long along X and 20 mm wide, its points (mm, setup 1), and
/// a block over it that one cut clears up to 45 mm along it, so a little over
/// half of it is still under stock: the part with X over the corner's + 45.
#[cfg(test)]
pub(crate) fn half_cut_face() -> (Mesh, Setup, [[f32; 3]; 3], Volume) {
    let mesh =
        crate::mesh::normalize(vec![[[0., 0., 0.], [100., 0., 0.], [50., 20., 0.]]]).unwrap();
    let setup = Setup::new(crate::model::Machine::new(3), "G54");
    let placement = Placement::new(&mesh, Some(&setup));
    let scale = mesh.display_scale();
    let [a, b, c]: [[f32; 3]; 3] = std::array::from_fn(|k| {
        placement
            .transform(mesh.faces[0].points[k])
            .map(|v| v / scale)
    });
    let frame = Frame::new(&setup, &mesh);
    let bounds = Bounds {
        min: [a[0], a[1], a[2]],
        max: [b[0], c[1], a[2] + 10.],
    };
    let mut stock = Volume::new(bounds, frame);
    // A flat Ø60 cutter along Y with its centre 15 mm along: X up to 45.
    std::sync::Arc::make_mut(&mut stock.cuts).push(crate::stock_surface::Cut {
        a: [a[0] + 15., a[1] - 40., a[2]],
        b: [a[0] + 15., c[1] + 40., a[2]],
        radius: 30.,
        flute: 10.,
        shape: crate::simulation::CutterShape::Flat,
        frame,
        source: (0, 0),
    });
    (mesh, setup, [a, b, c], stock)
}

#[cfg(test)]
mod tests {
    use crate::post::fixture;
    fn floor_under(height: f32, cut_height: Option<f32>) -> super::Uncut {
        let (mesh, setup, [a, _, _], mut stock) = super::half_cut_face();
        stock.bounds.max[2] = a[2] + height;
        let cuts = std::sync::Arc::make_mut(&mut stock.cuts);
        cuts.clear();
        if let Some(z) = cut_height {
            cuts.push(crate::stock_surface::Cut {
                a: [a[0], a[1], a[2] + z],
                b: [a[0] + 100., a[1], a[2] + z],
                radius: 100.,
                flute: 20.,
                shape: crate::simulation::CutterShape::Flat,
                frame: stock.frame,
                source: (0, 0),
            });
        }
        super::uncut(&stock, &setup, &mesh)
    }
    #[test]
    fn an_uncut_04_mm_recess_is_thin() {
        let left = floor_under(0.4, None);
        assert_eq!(left.faces, [false]);
        assert_eq!(left.thin.faces, [true]);
        assert_eq!((left.share, left.thin.share), (0., 1.));
        assert!(left.regions.is_empty());
        assert_eq!(left.thin.regions.len(), 1);
        assert!(left.any());
    }
    #[test]
    fn a_floor_with_02_mm_axial_stock_to_leave_is_thin() {
        let left = floor_under(10., Some(0.2));
        assert_eq!(left.faces, [false]);
        assert_eq!(left.thin.faces, [true]);
        assert_eq!((left.share, left.thin.share), (0., 1.));
    }
    #[test]
    fn a_floor_cut_exactly_to_height_is_finished() {
        let left = floor_under(10., Some(0.));
        assert!(!left.any());
        assert_eq!((left.share, left.thin.share), (0., 0.));
        // A small numerical/machining residual is also finished.
        assert!(!floor_under(10., Some(0.05)).any());
    }
    #[test]
    fn more_than_05_mm_remains_magenta() {
        let left = floor_under(10., Some(0.6));
        assert_eq!(left.faces, [true]);
        assert_eq!(left.thin.faces, [false]);
        assert_eq!((left.share, left.thin.share), (1., 0.));
    }
    #[test]
    fn finished_thin_and_magenta_pieces_share_one_face_without_overlap() {
        let (mesh, setup, [a, _, c], mut stock) = super::half_cut_face();
        let mut cut = stock.cuts[0].clone();
        // Exact cut to X 45, then a cut leaving 0.2 mm to X 75.
        cut.a = [a[0] + 45., a[1] - 40., a[2] + 0.2];
        cut.b = [a[0] + 45., c[1] + 40., a[2] + 0.2];
        std::sync::Arc::make_mut(&mut stock.cuts).push(cut);
        let left = super::uncut(&stock, &setup, &mesh);
        assert!(left.faces[0] && left.thin.faces[0]);
        assert_eq!(left.pieces[0].len(), left.thin.pieces[0].len());
        let mut areas = [0.; 3];
        let placement = super::Placement::new(&mesh, Some(&setup));
        for ((tri, thick), (thin_tri, thin)) in left.pieces[0].iter().zip(&left.thin.pieces[0]) {
            assert_eq!(tri, thin_tri);
            assert!(!(*thick && *thin));
            let t = tri.map(|p| placement.transform(p).map(|v| v / mesh.display_scale()));
            let x = t.iter().map(|p| p[0] - a[0]).sum::<f32>() / 3.;
            assert_eq!((*thick, *thin), (x > 75., x > 45. && x <= 75.));
            let area = super::cross(super::sub(t[1], t[0]), super::sub(t[2], t[0])) / 2.;
            areas[if *thick {
                0
            } else if *thin {
                1
            } else {
                2
            }] += area;
        }
        assert!(areas.iter().all(|a| *a > 0.));
        assert!((areas[0] / 1000. - left.share).abs() < 1e-4);
        assert!((areas[1] / 1000. - left.thin.share).abs() < 1e-4);
        assert!((areas.iter().sum::<f32>() - 1000.).abs() < 0.01);
    }
    /// Build the exact sweep history without spending time carving a grid.
    fn swept_plan(plan: &crate::simulation::Plan, frame: super::Frame) -> super::Volume {
        let mut stock = super::Volume::new(plan.stock, frame);
        std::sync::Arc::make_mut(&mut stock.cuts).extend(
            plan.moves
                .iter()
                .filter(|m| m.motion == crate::simulation::Motion::Feed)
                .map(|m| crate::stock_surface::Cut {
                    a: m.from,
                    b: m.to,
                    radius: plan.radius,
                    flute: plan.flute_length,
                    shape: plan.shape,
                    frame,
                    source: (0, 0),
                }),
        );
        stock
    }
    /// Residual along a face normal to 1 µm. These finish fixtures have no
    /// disconnected stock above the face, so the swept interval is monotone.
    fn residual(cuts: &super::Cuts<'_>, p: [f32; 3], n: [f32; 3]) -> f32 {
        let left = |d| cuts.left(std::array::from_fn(|k| p[k] + d * n[k]));
        assert!(!left(0.5), "unfinished at {p:?}");
        let (mut lo, mut hi) = (0., 0.5);
        while hi - lo > 0.001 {
            let mid = (lo + hi) / 2.;
            if left(mid) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        hi
    }
    #[test]
    fn uncut_threshold_exceeds_finished_fixture_residuals() {
        use crate::{cam, model::Strategy};
        // The existing pocket fixture: Ø2, zero allowances, its real 0.02 mm
        // conservative margin, including offset-loop arc chords.
        let (setup, op, tool, mesh) = crate::pocket::tests::fixture();
        let plan = crate::pocket::generate(&setup, &op, &tool, &mesh).unwrap();
        let stock = swept_plan(&plan, super::Frame::new(&setup, &mesh));
        let cuts = super::Cuts::new(&stock);
        let placement = super::Placement::new(&mesh, Some(&setup));
        let z = placement.bounds.min[2] / mesh.display_scale() + 4.;
        let mut pocket = 0f32;
        for i in 0..101 {
            let y = -7. + 14. * i as f32 / 100.;
            pocket = pocket.max(residual(&cuts, [-10., y, z + 2.], [1., 0., 0.]));
        }
        eprintln!("Pocket wall maximum normal residual: {pocket:.4} mm");
        assert!(pocket < super::THIN_OFFSET);
        // The shipped block fixture uses a Ø10 cutter: a larger 0.05 mm
        // Pocket margin, and the actual meshed pocket walls.
        let (doc, mesh) = fixture::job();
        let setup = &doc.setups[0];
        let mut op = setup.operations[1].clone();
        op.p.insert("allowance".into(), "0".into());
        op.p.insert("axial_allowance".into(), "0".into());
        let tool = doc.tools.iter().find(|t| t.id == op.tool).unwrap();
        let plan = crate::pocket::generate(setup, &op, tool, &mesh).unwrap();
        let stock = swept_plan(&plan, super::Frame::new(setup, &mesh));
        let cuts = super::Cuts::new(&stock);
        let placement = super::Placement::new(&mesh, Some(setup));
        let mut most = 0f32;
        let mut samples = 0;
        for face in &mesh.faces {
            let normal = mesh.surfaces[face.feature.unwrap()].normal;
            if normal[2].abs() > 0.01 {
                continue;
            }
            let p = std::array::from_fn(|k| {
                face.points
                    .iter()
                    .map(|p| placement.transform(*p)[k] / mesh.display_scale())
                    .sum::<f32>()
                    / 3.
            });
            // Other operations, not this pocket, must finish the other walls.
            if cuts.left(std::array::from_fn(|k| p[k] + 0.5 * normal[k])) {
                continue;
            }
            most = most.max(residual(&cuts, p, normal));
            samples += 1;
        }
        eprintln!("Ø10 block Pocket: {samples} wall samples, maximum normal residual {most:.4} mm");
        assert!(samples > 0 && most < super::THIN_OFFSET);
        // Existing ramp and tessellated R10 half-ball: Ø4 ball, fine 0.25 mm
        // stepover and 0.01 mm tolerance, zero stock to leave.
        for (name, mesh) in [
            ("21 degree ramp", cam::tests::ramp().0),
            ("R10 tessellated dome", cam::tests::half_ball_on_plate()),
        ] {
            let (setup, mut op, tool) = cam::tests::fixture(Strategy::Parallel);
            op.p.insert("surface_stepover".into(), "0.25".into());
            op.p.insert("tolerance".into(), "0.01".into());
            let plan = cam::generate(&setup, &op, &tool, &mesh).unwrap();
            let stock = swept_plan(&plan, super::Frame::new(&setup, &mesh));
            let cuts = super::Cuts::new(&stock);
            let placement = super::Placement::new(&mesh, Some(&setup));
            let scale = mesh.display_scale();
            let mut most = 0f32;
            let mut samples = 0;
            for face in &mesh.faces {
                let t: [[f32; 3]; 3] =
                    std::array::from_fn(|k| placement.transform(face.points[k]).map(|v| v / scale));
                let (u, v) = (super::sub(t[1], t[0]), super::sub(t[2], t[0]));
                let n = [
                    u[1] * v[2] - u[2] * v[1],
                    u[2] * v[0] - u[0] * v[2],
                    u[0] * v[1] - u[1] * v[0],
                ];
                let length = super::cross(u, v);
                let n = n.map(|x| x / length);
                let centre: [f32; 3] =
                    std::array::from_fn(|k| t.iter().map(|p| p[k]).sum::<f32>() / 3.);
                if n[2] < 0.3
                    || (name.contains("dome") && centre[2] < placement.bounds.min[2] / scale + 4.01)
                {
                    continue;
                }
                // Dense barycentric samples on finishable slopes, away from
                // the concave plate/dome junction where an R2 ball leaves a fillet.
                for i in 0..8 {
                    for j in 0..8 - i {
                        let p = std::array::from_fn(|k| {
                            t[0][k]
                                + (i as f32 + 1. / 3.) / 8. * u[k]
                                + (j as f32 + 1. / 3.) / 8. * v[k]
                        });
                        most = most.max(residual(&cuts, p, n));
                        samples += 1;
                    }
                }
            }
            eprintln!("{name}: {samples} samples, maximum normal residual {most:.4} mm");
            assert!(samples > 0 && most < super::THIN_OFFSET, "{name}: {most}");
            let left = super::uncut(&stock, &setup, &mesh);
            for (i, face) in mesh.faces.iter().enumerate() {
                let z = face
                    .points
                    .iter()
                    .map(|p| placement.transform(*p)[2] / scale)
                    .sum::<f32>()
                    / 3.;
                if mesh.surfaces[face.feature.unwrap()].normal[2] > 0.3
                    && (!name.contains("dome") || z > placement.bounds.min[2] / scale + 4.01)
                {
                    assert!(!left.faces[i] && !left.thin.faces[i], "{name} face {i}");
                }
            }
        }
    }
    #[test]
    fn a_long_face_half_cut_is_under_stock_only_where_the_stock_is() {
        let (mesh, setup, [a, b, c], stock) = super::half_cut_face();
        assert!((b[0] - a[0] - 100.).abs() < 0.01 && (c[1] - a[1] - 20.).abs() < 0.01);
        let uncut = super::uncut(&stock, &setup, &mesh);
        assert!(uncut.faces[0]);
        // The face in pieces that make it up, back in setup 1 as `uncut`
        // places them, each under stock just where its centre is past 45.
        let placement = crate::stock::Placement::new(&mesh, Some(&setup));
        let scale = mesh.display_scale();
        let placed = |t: &[[f32; 3]; 3]| t.map(|p| placement.transform(p).map(|v| v / scale));
        let area =
            |t: [[f32; 3]; 3]| super::cross(super::sub(t[1], t[0]), super::sub(t[2], t[0])) / 2.;
        let along = |t: [[f32; 3]; 3]| t.iter().map(|p| p[0] - a[0]).sum::<f32>() / 3.;
        let pieces = &uncut.pieces[0];
        assert!(pieces.len() > 2, "{pieces:?}");
        let whole: f32 = pieces.iter().map(|(t, _)| area(placed(t))).sum();
        assert!((whole - 1000.).abs() < 0.5, "{whole}");
        let mut under = 0.;
        for (t, left) in pieces {
            let t = placed(t);
            assert!(t.iter().all(|p| (p[2] - a[2]).abs() < 0.01), "{t:?}");
            assert_eq!(*left, along(t) > 45., "{t:?}");
            if *left {
                under += area(t);
            }
        }
        // What lies past 45: 595 mm² of 1000, in pieces of up to 5 mm.
        assert!((under - 595.).abs() < 40., "{under}");
        assert!(
            (uncut.share - under / 1000.).abs() < 0.001,
            "{}",
            uncut.share
        );
        let [region] = &uncut.regions[..] else {
            panic!("{:?}", uncut.regions)
        };
        assert!((region.area - under).abs() < 0.01, "{region:?}");
        assert!(region.bounds.min[0] > 40., "{region:?}");
    }
    #[test]
    fn a_round_bars_box_corners_are_no_stock() {
        let bounds = crate::mesh::Bounds {
            min: [0., -20., 0.],
            max: [100., 20., 40.],
        };
        let mesh = crate::mesh::sample();
        let setup = crate::model::Document::sample().setups[0].clone();
        let frame = super::Frame::new(&setup, &mesh);
        let bar = crate::stock::Round {
            axis: [0., 20.],
            radius: 20.,
        };
        let stock = super::Volume::new(bounds, frame).rounded(Some(bar));
        let cuts = super::Cuts::new(&stock);
        assert!(cuts.left([50., 0., 20.]));
        assert!(cuts.left([50., 13., 34.]));
        // In the box round the bar, outside the bar: never material.
        assert!(!cuts.left([50., 18., 38.]));
        assert!(!cuts.left([50., -19., 1.]));
    }
    /// The faces of `mesh` that lie on a side of its box in setup 1: the
    /// side across `axis` (2 for Z) at its high end for a positive `sign`.
    fn outer(
        mesh: &crate::mesh::Mesh,
        setup1: &crate::model::Setup,
        axis: usize,
        sign: f32,
    ) -> Vec<usize> {
        let placement = crate::stock::Placement::new(mesh, Some(setup1));
        let edge = if sign > 0. {
            placement.bounds.max[axis]
        } else {
            placement.bounds.min[axis]
        };
        mesh.faces
            .iter()
            .enumerate()
            .filter(|(_, f)| {
                f.points
                    .iter()
                    .all(|p| (placement.transform(*p)[axis] - edge).abs() < 0.01)
            })
            .map(|(i, _)| i)
            .collect()
    }
    #[test]
    fn faces_left_under_stock_are_found_until_a_later_setup_cuts_them() {
        // Setup 1 faces the top: the top is cut, the bottom and the 2 mm of
        // stock on the sides are not.
        let (doc, mesh) = fixture::flipped_after(1);
        let mut top_only = doc.clone();
        top_only.setups.truncate(1);
        let left = crate::leftover_stock(&top_only, &mesh, 0);
        let uncut = left.uncut.unwrap().unwrap();
        let setup1 = &doc.setups[0];
        let (top, bottom, side) = (
            outer(&mesh, setup1, 2, 1.),
            outer(&mesh, setup1, 2, -1.),
            outer(&mesh, setup1, 0, -1.),
        );
        assert!(!top.is_empty() && !bottom.is_empty() && !side.is_empty());
        assert!(top.iter().all(|i| !uncut.faces[*i]), "the faced top is cut");
        assert!(
            bottom.iter().all(|i| uncut.faces[*i]),
            "the bottom is under stock"
        );
        assert!(
            side.iter().all(|i| uncut.faces[*i]),
            "the -X side is under stock"
        );
        // The sides and the bottom, which the through holes join; the top's
        // pocket, reached from the top.
        let shell = &uncut.regions[0];
        assert_eq!(
            shell.from,
            ["−X", "+X", "−Y", "+Y", "the bottom"],
            "{shell:?}"
        );
        let pocket = uncut.regions[1].line();
        assert!(
            pocket.starts_with("5868 mm² reached from the top: X 16–88, Y 20–60, Z 28–40"),
            "{pocket}"
        );
        assert!(uncut.share > 0.3 && uncut.share < 1., "{}", uncut.share);
        // Setup 2, flipped, faces the bottom too.
        let both = crate::leftover_stock(&doc, &mesh, 0)
            .uncut
            .unwrap()
            .unwrap();
        assert!(bottom.iter().all(|i| !both.faces[*i]), "the bottom is cut");
        assert!(
            side.iter().all(|i| both.faces[*i]),
            "the sides are still under stock"
        );
        assert!(both.share < uncut.share);
        // The through holes are no longer joined to the bottom: each is
        // reached from either end.
        let through = both
            .regions
            .iter()
            .filter(|r| r.from == ["the bottom", "the top"]);
        assert_eq!(through.count(), 4, "{:?}", both.regions);
    }
    #[test]
    fn a_narrow_uncut_strip_in_a_large_flat_face_is_found() {
        // One 120 × 120 mm face (two triangles with edges up to 170 mm), the
        // size the mesher leaves a large planar face in, under a block cut
        // everywhere but a 4 mm strip along X at Y -48…-44: sampled in 15 mm
        // pieces its samples fall either side of the strip.
        let mesh = crate::mesh::normalize(vec![
            [[0., 0., 0.], [120., 0., 0.], [120., 120., 0.]],
            [[0., 0., 0.], [120., 120., 0.], [0., 120., 0.]],
        ])
        .unwrap();
        let setup = crate::model::Setup::new(crate::model::Machine::new(3), "G54");
        let frame = crate::volume::Frame::new(&setup, &mesh);
        let bounds = crate::mesh::Bounds {
            min: [-60., -60., 0.],
            max: [60., 60., 10.],
        };
        let mut stock = crate::volume::Volume::new(bounds, frame);
        let cut = |y: f32, radius: f32| crate::stock_surface::Cut {
            a: [-70., y, 0.],
            b: [70., y, 0.],
            radius,
            flute: 10.,
            shape: crate::simulation::CutterShape::Flat,
            frame,
            source: (0, 0),
        };
        // Y -68…-48 and -44…64.
        std::sync::Arc::make_mut(&mut stock.cuts).extend([cut(-58., 10.), cut(10., 54.)]);
        let uncut = super::uncut(&stock, &setup, &mesh);
        // The strip is 4 of the face's 120 mm.
        assert!(
            uncut.share > 0.01 && uncut.share < 0.1,
            "the strip is not found: {}",
            uncut.share
        );
        // Without the cut over the strip's side the whole face is under stock.
        std::sync::Arc::make_mut(&mut stock.cuts).truncate(1);
        let uncut = super::uncut(&stock, &setup, &mesh);
        assert!(uncut.share > 0.8, "{}", uncut.share);
        assert!(uncut.faces.iter().all(|f| *f));
    }
    /// After the TOTEM bottom's Adaptive and Parallel over its ramp, nothing
    /// along the ramp's top edge is magenta: the long triangles of the face
    /// the ramp runs into are under stock only away from it. The part is not
    /// ours to ship: point `AXIS_RAMP_WS` at the workspace with both
    /// operations (its part a binary STL that numbers its CAD faces) and run
    /// `cargo test --release totem_ramps_top_edge -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn the_totem_ramps_top_edge_is_not_magenta() {
        let Ok(path) = std::env::var("AXIS_RAMP_WS") else {
            eprintln!("skipped: set AXIS_RAMP_WS to the TOTEM bottom's workspace");
            return;
        };
        let doc = crate::storage::load(std::path::Path::new(&path)).unwrap();
        let mesh = crate::mesh::load(&doc.file.as_ref().unwrap().path).unwrap();
        let setup = &doc.setups[0];
        let ramp: Vec<usize> = (setup.operations.iter())
            .filter_map(|o| o.surfaces.as_ref())
            .flat_map(|s| s.ids.clone())
            .collect();
        assert!(!ramp.is_empty());
        let mut last = None;
        crate::simulation::sequence_with(
            &doc,
            &mesh,
            None,
            |_, _, r| assert!(r.is_ok()),
            |_, _, v| last = Some(v.clone()),
        );
        let uncut = super::uncut(last.as_ref().unwrap(), setup, &mesh);
        let placement = crate::stock::Placement::new(&mesh, Some(setup));
        let scale = mesh.display_scale();
        let placed = |p: [f32; 3]| placement.transform(p).map(|v| v / scale);
        let on_ramp = |i: usize| mesh.faces[i].feature.is_some_and(|id| ramp.contains(&id));
        // The ramp's top edge: its corners at its highest.
        let corners: Vec<[f32; 3]> = (0..mesh.faces.len())
            .filter(|i| on_ramp(*i))
            .flat_map(|i| mesh.faces[i].points.iter().map(|p| placed(*p)))
            .collect();
        let top = corners
            .iter()
            .map(|p| p[2])
            .fold(f32::NEG_INFINITY, f32::max);
        let edge: Vec<_> = corners.iter().filter(|p| p[2] > top - 0.01).collect();
        let lo: [f32; 3] =
            std::array::from_fn(|k| edge.iter().map(|p| p[k]).fold(f32::INFINITY, f32::min));
        let hi: [f32; 3] =
            std::array::from_fn(|k| edge.iter().map(|p| p[k]).fold(f32::NEG_INFINITY, f32::max));
        // How far a point lies from the edge (mm).
        let far = |c: [f32; 3]| {
            (0..3)
                .map(|k| (lo[k] - c[k]).max(c[k] - hi[k]).max(0.).powi(2))
                .sum::<f32>()
                .sqrt()
        };
        let on_edge = |p: [f32; 3]| far(p) < 0.01;
        // That, beside it: None past its ends or within half a piece of
        // them. There the wall keeps a band of stock, about 1 mm wide, in its
        // corners with the walls at the ramp's ends, past its picked faces,
        // and the pieces it is magenta in reach up to half their size more.
        let long = (0..3).fold(0, |m, k| if hi[k] - lo[k] > hi[m] - lo[m] { k } else { m });
        let ends = super::PIECE / 2.;
        let beside = |c: [f32; 3]| {
            (lo[long] + ends..=hi[long] - ends)
                .contains(&c[long])
                .then(|| far(c))
        };
        let placed_face = |i: usize| [0, 1, 2].map(|k| placed(mesh.faces[i].points[k]));
        let area =
            |t: [[f32; 3]; 3]| super::cross(super::sub(t[1], t[0]), super::sub(t[2], t[0])) / 2.;
        // The face the ramp runs into: that of the other triangles on its
        // top edge.
        let wall: Vec<usize> = (0..mesh.faces.len())
            .filter(|i| {
                !on_ramp(*i) && placed_face(*i).iter().filter(|p| on_edge(**p)).count() >= 2
            })
            .filter_map(|i| mesh.faces[i].feature)
            .collect();
        eprintln!("top edge {lo:.2?}..{hi:.2?}, into surfaces {wall:?}");
        // What is magenta, drawn as now (whole faces and pieces under stock)
        // or as before, a face whole once half of it was under stock: the
        // nearest a point of it comes to the edge, beside it (mm), with the
        // triangle it is on. Faces looking down (under the ramp's lip, where
        // no cutter from above reaches) or along the edge (the walls at the
        // ramp's ends, past its picked faces) are left out.
        let nearest = |before: bool| {
            let mut best: Option<(f32, usize, [f32; 3])> = None;
            for i in (0..mesh.faces.len()).filter(|i| uncut.faces[*i]) {
                let normal = mesh.surfaces[mesh.faces[i].feature.unwrap()].normal;
                let normal = crate::mesh::rotation(normal, placement.rotation());
                if normal[2] < 0. || normal[long].abs() > 0.9 {
                    continue;
                }
                let under: Vec<[[f32; 3]; 3]> = (uncut.pieces[i].iter())
                    .filter(|(_, under)| *under)
                    .map(|(t, _)| t.map(placed))
                    .collect();
                let half = under.iter().map(|t| area(*t)).sum::<f32>() >= area(placed_face(i)) / 2.;
                let magenta = match (uncut.pieces[i].is_empty(), before) {
                    (true, _) => vec![placed_face(i)],
                    (false, true) if half => vec![placed_face(i)],
                    (false, true) => vec![],
                    (false, false) => under,
                };
                // Each triangle at the centres of its pieces of up to 0.5 mm.
                for [a, b, c] in magenta {
                    let n = (super::longest(a, b, c) / 0.5).ceil().clamp(1., 200.) as usize;
                    for (x, y) in (0..n).flat_map(|x| (0..n - x).map(move |y| (x, y))) {
                        let (s, t) = (
                            (x as f32 + 1. / 3.) / n as f32,
                            (y as f32 + 1. / 3.) / n as f32,
                        );
                        let p =
                            std::array::from_fn(|k| a[k] + s * (b[k] - a[k]) + t * (c[k] - a[k]));
                        if let Some(off) = beside(p)
                            && best.is_none_or(|(d, _, _)| off < d)
                        {
                            best = Some((off, i, p));
                        }
                    }
                }
            }
            best
        };
        let (before, now) = (nearest(true), nearest(false));
        eprintln!("magenta nearest the edge: before {before:.2?}, now {now:.2?}");
        // Before, the wall's long triangle ran along it; now nothing within
        // 2 mm of it is magenta.
        let on_wall = |i: usize| mesh.faces[i].feature.is_some_and(|id| wall.contains(&id));
        assert!(
            before.is_some_and(|(off, i, _)| off < 0.5 && on_wall(i)),
            "{before:?}"
        );
        assert!(now.is_none_or(|(off, _, _)| off > 2.), "{now:?}");
    }
    #[test]
    fn a_face_is_split_into_at_most_the_samples_a_run_can_afford() {
        let mut out = vec![];
        super::split(
            [[0., 0., 0.], [120., 0., 0.], [120., 120., 0.]],
            0,
            &mut out,
        );
        assert!(out.len() > 1 && out.len() <= 1 << super::SPLITS);
        for [a, b, c] in &out {
            assert!(super::longest(*a, *b, *c) <= super::SPLIT + 0.01);
        }
        let area = |t: &[[f32; 3]; 3]| super::cross(super::sub(t[1], t[0]), super::sub(t[2], t[0]));
        assert!((out.iter().map(area).sum::<f32>() - 120. * 120.).abs() < 0.1);
        // A face far larger than 40 mm × 2⁶ stops at the bound.
        let mut huge = vec![];
        super::split(
            [[0., 0., 0.], [5000., 0., 0.], [0., 5000., 0.]],
            0,
            &mut huge,
        );
        assert_eq!(huge.len(), 1 << super::SPLITS);
    }
    #[test]
    fn completeness_is_not_checked_without_a_cut_or_after_a_failure() {
        let (mut doc, mesh) = fixture::job();
        doc.setups[0].operations.clear();
        let left = crate::leftover_stock(&doc, &mesh, 0);
        assert_eq!(
            left.uncut.unwrap().unwrap_err(),
            "No operation has cut the stock yet."
        );
        let (mut doc, mesh) = fixture::job();
        doc.setups[0].operations.truncate(1);
        doc.setups[0].operations[0].tool = "gone".into();
        let left = crate::leftover_stock(&doc, &mesh, 0);
        let why = left.uncut.unwrap().unwrap_err();
        assert!(why.starts_with("Top / "), "{why}");
    }
}
