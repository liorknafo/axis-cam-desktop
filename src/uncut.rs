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

/// How far outside a face its samples are taken (mm): stock thinner than this
/// against a face counts as cut, as a finishing allowance left on it would.
const OFFSET: f32 = 0.5;
/// The longest side a sampled piece of a face has (mm): longer faces are
/// sampled in pieces.
const PIECE: f32 = 5.;
/// The longest edge a triangle has when it is sampled (mm): `PIECE` times the
/// 8 pieces a side is cut in at most. Longer edges are split first.
const SPLIT: f32 = 8. * PIECE;
/// How often a face is halved at most. That bounds a face to 2⁶ triangles of
/// 64 samples, 4096 samples, and the background run to that many per face of
/// the mesh; a triangle needing more splits (over about 300 mm across) is
/// sampled in coarser pieces.
const SPLITS: u32 = 6;
/// The buckets of cuts along each axis of the stock.
const BUCKETS: usize = 24;
/// The sides of the part's box, in setup 1's axes, negative side first.
const SIDES: [&str; 6] = ["−X", "+X", "−Y", "+Y", "the bottom", "the top"];
/// Regions smaller than this (mm²) are left out: the slivers a round cutter
/// leaves in a sharp inside corner.
const SLIVER: f32 = 2.;

/// The faces of the part still under stock.
#[derive(Debug, Clone, Default)]
pub struct Uncut {
    /// Per face of the mesh (`Mesh::faces`): whether at least half of it is
    /// still under stock.
    pub faces: Vec<bool>,
    /// Those faces, joined where they share an edge, largest first. Those
    /// of regions under `SLIVER` are left out, here and in `faces`.
    pub regions: Vec<Region>,
    /// The share of the part's surface still under stock, from 0 to 1.
    pub share: f32,
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
    let mut faces = vec![false; mesh.faces.len()];
    // Per face still under stock: its area and points (mm, setup 1).
    let mut found: HashMap<usize, (f32, [[f32; 3]; 3])> = HashMap::new();
    let (mut total, mut covered) = (0., 0.);
    for (i, face) in mesh.faces.iter().enumerate() {
        let [a, b, c] = match face.points[..] {
            [a, b, c] => [a, b, c].map(|p| placement.transform(p).map(|v| v / scale)),
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
        // pieces of at most `PIECE`; their shares weigh by area.
        let mut parts = vec![];
        split([a, b, c], 0, &mut parts);
        let mut share = 0.;
        for [a, b, c] in parts {
            let (u, v) = (sub(b, a), sub(c, a));
            let k = (longest(a, b, c) / PIECE).ceil().clamp(1., 8.) as usize;
            let (mut samples, mut under) = (0, 0);
            // The centres of the k² pieces of the part: k(k+1)/2 with a corner
            // at its first corner, and the k(k-1)/2 turned the other way.
            for i in 0..k {
                for j in 0..k - i {
                    for (shift, piece) in [(1. / 3., true), (2. / 3., i + j + 1 < k)] {
                        if !piece {
                            continue;
                        }
                        let (s, t) = ((i as f32 + shift) / k as f32, (j as f32 + shift) / k as f32);
                        let p: [f32; 3] =
                            std::array::from_fn(|q| a[q] + s * u[q] + t * v[q] + n[q] * OFFSET);
                        samples += 1;
                        under += usize::from(cuts.left(moved(p, first, stock.frame)));
                    }
                }
            }
            share += cross(u, v) / 2. * under as f32 / samples as f32;
        }
        let area = twice / 2.;
        share /= area;
        total += area;
        covered += area * share;
        if share >= 0.5 {
            faces[i] = true;
            found.insert(i, (area, [a, b, c]));
        }
    }
    // Faces under stock that share an edge make one region.
    let mut root: HashMap<usize, usize> = found.keys().map(|i| (*i, *i)).collect();
    fn top(root: &mut HashMap<usize, usize>, mut i: usize) -> usize {
        while root[&i] != i {
            i = root[&i];
        }
        i
    }
    let mut edges: HashMap<[[i64; 3]; 2], usize> = HashMap::new();
    for (&i, (_, p)) in &found {
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
    // its faces lie on and its corners meet (`SIDES` order).
    struct Group {
        faces: Vec<usize>,
        area: f32,
        bounds: Bounds,
        on: [bool; 6],
        meets: [bool; 6],
    }
    let mut grouped: HashMap<usize, Group> = HashMap::new();
    for (&i, (area, p)) in &found {
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
    let mut regions = vec![];
    for group in grouped.into_values() {
        if group.area < SLIVER {
            for i in group.faces {
                faces[i] = false;
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
    Uncut {
        faces,
        regions,
        share: if total > 0. { covered / total } else { 0. },
    }
}
/// `p` in frame `from` as frame `to` sees it.
fn moved(p: [f32; 3], from: Frame, to: Frame) -> [f32; 3] {
    if from == to { p } else { from.transfer(p, to) }
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

#[cfg(test)]
mod tests {
    use crate::post::fixture;
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
