//! Deterministic summary of the imported part for the AI planner: what can be
//! reached from each of the six axis-aligned orientations. Lengths are mm.
use crate::{
    cam::{self, Geometry, Polygon},
    mesh::{Bounds, Mesh, rotation},
    model::{Document, Operation, Setup, Strategy, get, num},
    surface::Selection,
    volume::{Frame, Volume},
};
use clipper2::{EndType, JoinType};
use serde::Serialize;

/// Orientation name (the part axis that points up) and its setup rotation.
pub const ORIENTATIONS: [(&str, [f32; 3]); 6] = [
    ("+Z", [0., 0., 0.]),
    ("-Z", [180., 0., 0.]),
    ("+X", [0., -90., 0.]),
    ("-X", [0., 90., 0.]),
    ("+Y", [90., 0., 0.]),
    ("-Y", [-90., 0., 0.]),
];
const MAX_PLANES: usize = 40;
const MAX_HOLES: usize = 100;
/// cos 0.5°: a face this close to horizontal counts as planar-horizontal.
const FLAT: f32 = 0.99996;
/// cos 89.5°: steeper faces count as vertical walls.
const WALL: f32 = 0.0087;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PartReport {
    pub units: &'static str,
    pub part_size: [f32; 3],
    pub material: String,
    pub machine: MachineReport,
    pub tools: Vec<ToolReport>,
    /// The first entry is setup 1's orientation.
    pub orientations: Vec<OrientationReport>,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MachineReport {
    pub max_rpm: f32,
    pub max_feed: f32,
    pub travel: [f32; 3],
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ToolReport {
    pub number: u32,
    pub name: String,
    pub kind: String,
    pub diameter: f32,
    pub flute_length: f32,
    pub stickout: f32,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OrientationReport {
    pub up: &'static str,
    pub is_setup1: bool,
    /// Model extent in this orientation: X, Y and height.
    pub model_size: [f32; 3],
    /// Stock beyond the model before any machining: top, bottom, -X, +X, -Y, +Y.
    pub stock_margin: [f32; 6],
    /// Visible horizontal faces, largest first. Positions are measured from
    /// the model's -X/-Y corner in this orientation.
    pub planes: Vec<PlaneReport>,
    /// Visible area that is neither horizontal nor vertical (mm²).
    pub freeform_area: f32,
    pub truncated: bool,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PlaneReport {
    pub surfaces: Vec<usize>,
    /// "top" for the highest plane, otherwise "floor".
    pub kind: &'static str,
    pub depth_below_top: f32,
    pub area: f32,
    /// x0, y0, x1, y1
    pub bbox: [f32; 4],
    /// Largest flat cutter that fits inside the floor outline (floors only).
    pub max_cutter_diameter: Option<f32>,
    pub holes: Vec<HoleReport>,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HoleReport {
    pub center: [f32; 2],
    /// Narrowest diameter down to the bottom: the largest drill that fits.
    pub diameter: f32,
    pub opening_diameter: f32,
    pub depth: f32,
    pub through: bool,
}
fn r2(v: f32) -> f32 {
    // Adding 0 turns a rounded -0.0 into 0.0, so it never prints as "-0.0".
    (v * 100.).round() / 100. + 0.
}
/// The part axis that points up in this setup, when the setup is square to
/// the part (every rotation a multiple of 90°).
pub fn up_axis(setup: &Setup) -> Option<&'static str> {
    let angles = ["rx", "ry", "rz"].map(|k| num(&setup.p, k).unwrap_or(f32::NAN));
    let axes = [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]].map(|a| rotation(a, angles));
    if axes
        .iter()
        .flatten()
        .any(|v| !v.is_finite() || (v.abs() > 0.001 && v.abs() < 0.999))
    {
        return None;
    }
    let (axis, z) = (0..3)
        .map(|k| (k, axes[k][2]))
        .find(|(_, z)| z.abs() > 0.5)?;
    Some(match (axis, z > 0.) {
        (0, true) => "+X",
        (0, false) => "-X",
        (1, true) => "+Y",
        (1, false) => "-Y",
        (_, true) => "+Z",
        (_, false) => "-Z",
    })
}
/// The setup rotation for an orientation: setup 1's own rotation for its up
/// axis, the canonical rotation otherwise.
pub fn rotation_for(setup1: &Setup, up: &str) -> Option<[f32; 3]> {
    if up_axis(setup1) == Some(up) {
        return Some(["rx", "ry", "rz"].map(|k| num(&setup1.p, k).unwrap_or(0.)));
    }
    ORIENTATIONS
        .iter()
        .find(|(name, _)| *name == up)
        .map(|(_, r)| *r)
}
/// A copy of setup 1 turned to `up`. Later setups carry no translation.
pub fn oriented(setup1: &Setup, up: &str) -> Option<Setup> {
    let rotation = rotation_for(setup1, up)?;
    let mut setup = setup1.clone();
    let first = up_axis(setup1) == Some(up);
    for (key, value) in ["rx", "ry", "rz"].iter().zip(rotation) {
        setup.p.insert((*key).into(), value.to_string());
    }
    if !first {
        for key in ["tx", "ty", "tz"] {
            setup.p.insert(key.into(), "0".into());
        }
    }
    Some(setup)
}
/// XY bucket index over the placed triangles, for height queries.
pub(crate) struct Index<'a> {
    geometry: &'a Geometry,
    cell: f32,
    n: [usize; 2],
    cells: Vec<Vec<u32>>,
}
impl<'a> Index<'a> {
    pub(crate) fn new(geometry: &'a Geometry) -> Self {
        let d = geometry.bounds.dimensions();
        let cell = (d[0].max(d[1]) / 128.).max(0.01);
        let n = [0, 1].map(|k| (d[k] / cell).ceil() as usize + 1);
        let mut cells = vec![vec![]; n[0] * n[1]];
        let mut index = Self {
            geometry,
            cell,
            n,
            cells: vec![],
        };
        for (i, t) in geometry.triangles.iter().enumerate() {
            let lo = [0, 1].map(|k| t.p.iter().map(|p| p[k]).fold(f32::INFINITY, f32::min));
            let hi = [0, 1].map(|k| t.p.iter().map(|p| p[k]).fold(f32::NEG_INFINITY, f32::max));
            let (a, b) = (index.cell_of(lo), index.cell_of(hi));
            for y in a[1]..=b[1] {
                for x in a[0]..=b[0] {
                    cells[y * n[0] + x].push(i as u32);
                }
            }
        }
        index.cells = cells;
        index
    }
    fn cell_of(&self, xy: [f32; 2]) -> [usize; 2] {
        [0, 1].map(|k| {
            (((xy[k] - self.geometry.bounds.min[k]) / self.cell)
                .floor()
                .max(0.) as usize)
                .min(self.n[k] - 1)
        })
    }
    /// Triangles whose XY bucket contains `xy`.
    pub(crate) fn near(&self, xy: [f32; 2]) -> impl Iterator<Item = &'a cam::Triangle> + '_ {
        let c = self.cell_of(xy);
        self.cells[c[1] * self.n[0] + c[0]]
            .iter()
            .map(|i| &self.geometry.triangles[*i as usize])
    }
    /// Highest mesh point above `xy`.
    fn height(&self, xy: [f32; 2]) -> Option<f32> {
        self.near(xy)
            .filter_map(|t| cam::plane_height(&t.p, xy))
            .max_by(f32::total_cmp)
    }
}
/// Crossings of the vertical line through `xy` with the surface, lowest
/// first: height, and whether the face points up.
fn crossings(index: &Index, xy: [f32; 2]) -> Vec<(f32, bool)> {
    let mut hits: Vec<(f32, bool)> = index
        .near(xy)
        .filter_map(|t| Some((cam::plane_height(&t.p, xy)?, normal(&t.p)[2] > 0.)))
        .collect();
    hits.sort_by(|a, b| a.0.total_cmp(&b.0));
    hits.dedup_by(|a, b| (a.0 - b.0).abs() < 1e-4 && a.1 == b.1);
    hits
}
/// Solid spans from `hits`, lowest first, or `None` when the face directions
/// are inconsistent.
fn spans(hits: &[(f32, bool)]) -> Option<Vec<(f32, f32)>> {
    let mut spans = vec![];
    let (mut depth, mut start, mut consistent) = (0i32, 0f32, true);
    for (z, up) in hits {
        if !*up {
            if depth == 0 {
                start = *z;
            }
            depth += 1;
        } else {
            depth -= 1;
            if depth == 0 {
                spans.push((start, *z));
            }
            consistent &= depth >= 0;
        }
    }
    (consistent && depth == 0).then_some(spans)
}
/// The lowest crossing of an inconsistent column and whether it faces down,
/// and the highest and whether it faces up.
type Ends = [(f32, bool); 2];
/// The column at `xy` and, while it is inconsistent, at four points 0.002 mm
/// away: `Ok` with the first consistent spans, or `Err` with the ends of all
/// five when they are all inconsistent. Crossings within 0.0001 mm of one
/// height that face both ways count as facing the way that makes more of the
/// column solid.
fn retried(index: &Index, xy: [f32; 2]) -> Result<Vec<(f32, f32)>, Ends> {
    let (mut low, mut high) = ((f32::INFINITY, true), (f32::NEG_INFINITY, true));
    for d in [
        [0., 0.],
        [0.002, 0.],
        [-0.002, 0.],
        [0., 0.002],
        [0., -0.002],
    ] {
        let hits = crossings(index, [xy[0] + d[0], xy[1] + d[1]]);
        if let Some(spans) = spans(&hits) {
            return Ok(spans);
        }
        for (z, up) in hits {
            if z < low.0 - 1e-4 {
                low = (z, !up);
            } else if z <= low.0 + 1e-4 {
                low = (low.0.min(z), low.1 && !up);
            }
            if z > high.0 + 1e-4 {
                high = (z, up);
            } else if z >= high.0 - 1e-4 {
                high = (high.0.max(z), high.1 && up);
            }
        }
    }
    Err([low, high])
}
/// Solid spans along the vertical line through `xy`, erring towards solid: an
/// inconsistent column is retried at four nearby offsets, and if it stays
/// inconsistent it counts as solid from its lowest crossing if that faces
/// down, otherwise from the model's bottom, up to its highest crossing if
/// that faces up, otherwise up to `top`. A column with no crossing is empty.
pub(crate) fn solid(index: &Index, xy: [f32; 2], top: f32) -> Vec<(f32, f32)> {
    retried(index, xy).unwrap_or_else(|[(low, down), (high, up)]| {
        let from = if down {
            low
        } else {
            index.geometry.bounds.min[2]
        };
        let to = if up { high } else { top };
        vec![(from, to.max(from))]
    })
}
/// Share of a 48 × 48 grid of vertical rays over the model that cross its
/// surface inconsistently at all five positions `retried` tries; rays that
/// hit nothing are not counted. A model no ray hits counts as inconsistent.
fn inconsistent_share(geometry: &Geometry) -> f32 {
    let index = Index::new(geometry);
    let b = geometry.bounds;
    let (mut hit, mut bad) = (0, 0);
    for i in 0..48 * 48 {
        let xy = [i % 48, i / 48].map(|n| (n as f32 + 0.5) / 48.);
        let xy = [0, 1].map(|k| b.min[k] + (b.max[k] - b.min[k]) * xy[k] + 1e-4);
        match retried(&index, xy) {
            Ok(spans) if spans.is_empty() => {}
            Ok(_) => hit += 1,
            Err(_) => (hit, bad) = (hit + 1, bad + 1),
        }
    }
    if hit == 0 {
        1.
    } else {
        bad as f32 / hit as f32
    }
}
/// Vertices this close are one vertex, and a vertex this close to an open
/// edge, and farther than this from both its ends, splits it (mm).
const WELD: f64 = 0.001;
/// Copies of one vertex spread over no more than this (mm).
const SPREAD: f64 = 0.01;
/// Coordinates beyond this are not checked (mm).
const FAR: f64 = 1_000_000.;
/// The part's triangles in mm, in the mesh's own frame: whether a surface is
/// closed does not depend on where setup 1 puts it.
fn mm_triangles(mesh: &Mesh) -> Vec<[[f64; 3]; 3]> {
    let scale = f64::from(mesh.display_scale());
    mesh.faces
        .iter()
        .filter(|f| f.points.len() == 3)
        .map(|f| std::array::from_fn(|i| f.points[i].map(|v| f64::from(v) / scale)))
        .collect()
}
/// The number of open edges of `triangles` (mm); a closed surface has none.
/// Vertices within `WELD` of each other are one vertex (see `weld`),
/// zero-area triangles are ignored, and a triangle written more than once, in
/// either winding, counts once. An edge used by an odd number of triangles is
/// open. Each open edge is then split at every end of another open edge
/// within `WELD` of it, away from its own ends, as at a T-junction, and the
/// pieces are counted with the open edges they now match: those used an odd
/// number of times are the open edges left. Coordinates beyond `FAR` or not
/// finite make every edge open.
pub(crate) fn open_edges(triangles: &[[[f64; 3]; 3]]) -> usize {
    if triangles
        .iter()
        .flatten()
        .flatten()
        .any(|v| !(v.abs() <= FAR))
    {
        return 3 * triangles.len();
    }
    let (corners, points) = weld(triangles);
    let mut faces: Vec<[u32; 3]> = triangles
        .iter()
        .zip(corners)
        .filter_map(|(t, mut ids)| {
            let n = cross(sub(t[1], t[0]), sub(t[2], t[0]));
            ids.sort_unstable();
            (dot(n, n) >= 1e-18 && ids[0] != ids[1] && ids[1] != ids[2]).then_some(ids)
        })
        .collect();
    faces.sort_unstable();
    faces.dedup();
    let open = odd(faces
        .iter()
        .flat_map(|f| [(f[0], f[1]), (f[1], f[2]), (f[0], f[2])])
        .collect());
    if open.is_empty() {
        return 0;
    }
    // Only the ends of open edges can split one.
    let mut ends: Vec<u32> = open.iter().flat_map(|e| [e.0, e.1]).collect();
    ends.sort_unstable();
    ends.dedup();
    let tree = Tree::new(&points, ends);
    let (mut pieces, mut cuts) = (vec![], vec![]);
    for &(a, b) in &open {
        let (pa, pb) = (points[a as usize], points[b as usize]);
        let d = sub(pb, pa);
        cuts.clear();
        tree.near(pa, pb, |v| {
            let p = points[v as usize];
            let t = (dot(sub(p, pa), d) / dot(d, d)).clamp(0., 1.);
            let off = sub(p, std::array::from_fn(|k| pa[k] + d[k] * t));
            let far = |q: [f64; 3]| dot(sub(p, q), sub(p, q)) > WELD * WELD;
            if dot(off, off) <= WELD * WELD && far(pa) && far(pb) {
                cuts.push((t, v));
            }
        });
        cuts.sort_by(|x, y| x.0.total_cmp(&y.0));
        let mut from = a;
        for &(_, v) in &cuts {
            pieces.push((from.min(v), from.max(v)));
            from = v;
        }
        pieces.push((from.min(b), from.max(b)));
    }
    odd(pieces).len()
}
/// The edges that occur an odd number of times in `edges`.
fn odd(mut edges: Vec<(u32, u32)>) -> Vec<(u32, u32)> {
    edges.sort_unstable();
    edges
        .chunk_by(|a, b| a == b)
        .filter(|run| run.len() % 2 == 1)
        .map(|run| run[0])
        .collect()
}
/// Welded vertex ids of the corners of `triangles`, and a position for each
/// (one of its copies). Copies within `WELD` of each other are one vertex,
/// and so are chains of such copies, unless a chain spreads over more than
/// `SPREAD`: that is a row of distinct vertices too close to weld, such as a
/// finely divided rim, and only its bit-identical copies are one vertex.
/// Copies are grouped in cells small enough that all copies in one are within
/// `WELD` of each other, so a cluster of copies costs no more than its few
/// cells, and a cell is compared only with the cells near it, found through
/// blocks of 8 × 8 × 8 cells.
fn weld(triangles: &[[[f64; 3]; 3]]) -> (Vec<[u32; 3]>, Vec<[f64; 3]>) {
    use std::collections::HashMap;
    // Distinct positions first: most copies of a vertex are bit-identical.
    let mut seen: HashMap<[u64; 3], u32> = HashMap::new();
    let mut points: Vec<[f64; 3]> = vec![];
    let corners: Vec<[u32; 3]> = triangles
        .iter()
        .map(|t| {
            t.map(|p| {
                *seen.entry(p.map(f64::to_bits)).or_insert_with(|| {
                    points.push(p);
                    points.len() as u32 - 1
                })
            })
        })
        .collect();
    let side = WELD / 1.7321;
    let cell = |p: [f64; 3]| p.map(|v| (v / side).floor() as i64);
    let block = |c: [i64; 3]| c.map(|v| v.div_euclid(8));
    let mut order: Vec<(u32, [i64; 3])> = (0..points.len())
        .map(|i| (i as u32, cell(points[i])))
        .collect();
    order.sort_unstable_by_key(|(i, c)| (block(*c), *c, *i));
    // Each cell's copies (a range of `order`) and their box.
    type Cell = ([i64; 3], std::ops::Range<usize>, [f64; 3], [f64; 3]);
    let mut cells: Vec<Cell> = vec![];
    for run in order.chunk_by(|a, b| a.1 == b.1) {
        let start = cells.last().map_or(0, |c| c.1.end);
        let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        for (i, _) in run {
            let p = points[*i as usize];
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        cells.push((run[0].1, start..start + run.len(), lo, hi));
    }
    let mut blocks: HashMap<[i64; 3], std::ops::Range<usize>> = HashMap::new();
    let mut start = 0;
    for run in cells.chunk_by(|a, b| block(a.0) == block(b.0)) {
        blocks.insert(block(run[0].0), start..start + run.len());
        start += run.len();
    }
    fn root(parent: &mut [u32], mut i: u32) -> u32 {
        while parent[i as usize] != i {
            parent[i as usize] = parent[parent[i as usize] as usize];
            i = parent[i as usize];
        }
        i
    }
    let mut parent: Vec<u32> = (0..cells.len() as u32).collect();
    let near = |p: u32, q: u32| {
        let (p, q) = (points[p as usize], points[q as usize]);
        dot(sub(p, q), sub(p, q)) <= WELD * WELD
    };
    for (c, (_, own, lo, hi)) in cells.iter().enumerate() {
        let from = block(cell(lo.map(|v| v - WELD)));
        let to = block(cell(hi.map(|v| v + WELD)));
        for x in from[0]..=to[0] {
            for y in from[1]..=to[1] {
                for z in from[2]..=to[2] {
                    let Some(range) = blocks.get(&[x, y, z]) else {
                        continue;
                    };
                    for (d, (_, other, olo, ohi)) in cells[range.clone()]
                        .iter()
                        .enumerate()
                        .map(|(k, cell)| (range.start + k, cell))
                        .filter(|(d, _)| *d > c)
                    {
                        let gap: f64 = (0..3)
                            .map(|k| (olo[k] - hi[k]).max(lo[k] - ohi[k]).max(0.).powi(2))
                            .sum();
                        if gap > WELD * WELD {
                            continue;
                        }
                        let (a, b) = (root(&mut parent, c as u32), root(&mut parent, d as u32));
                        if a != b
                            && order[own.clone()].iter().any(|(p, _)| {
                                order[other.clone()].iter().any(|(q, _)| near(*p, *q))
                            })
                        {
                            parent[a.max(b) as usize] = a.min(b);
                        }
                    }
                }
            }
        }
    }
    let mut roots = vec![0usize; points.len()];
    for (c, (_, range, _, _)) in cells.iter().enumerate() {
        let r = root(&mut parent, c as u32) as usize;
        for (i, _) in &order[range.clone()] {
            roots[*i as usize] = r;
        }
    }
    // How far each vertex's copies spread.
    let (mut lo, mut hi) = (
        vec![[f64::INFINITY; 3]; cells.len()],
        vec![[f64::NEG_INFINITY; 3]; cells.len()],
    );
    for (p, r) in points.iter().zip(&roots) {
        for k in 0..3 {
            lo[*r][k] = lo[*r][k].min(p[k]);
            hi[*r][k] = hi[*r][k].max(p[k]);
        }
    }
    // Ids in order of first use, each placed at its first copy.
    let (mut id_of_root, mut ids, mut welded) = (vec![u32::MAX; cells.len()], vec![], vec![]);
    for (p, r) in points.iter().zip(roots) {
        let spread = sub(hi[r], lo[r]);
        if dot(spread, spread) > SPREAD * SPREAD {
            ids.push(welded.len() as u32);
            welded.push(*p);
            continue;
        }
        if id_of_root[r] == u32::MAX {
            id_of_root[r] = welded.len() as u32;
            welded.push(*p);
        }
        ids.push(id_of_root[r]);
    }
    let corners = corners
        .into_iter()
        .map(|c| c.map(|i| ids[i as usize]))
        .collect();
    (corners, welded)
}
/// Points in a k-d tree, to find those near a segment.
struct Tree<'a> {
    points: &'a [[f64; 3]],
    ids: Vec<u32>,
    /// Each node's box, its points (a range of `ids`) and its children.
    nodes: Vec<(
        [f64; 3],
        [f64; 3],
        std::ops::Range<usize>,
        Option<[usize; 2]>,
    )>,
}
impl<'a> Tree<'a> {
    fn new(points: &'a [[f64; 3]], ids: Vec<u32>) -> Self {
        let mut tree = Self {
            points,
            ids,
            nodes: vec![],
        };
        tree.build(0..tree.ids.len());
        tree
    }
    fn build(&mut self, range: std::ops::Range<usize>) -> usize {
        let points = self.points;
        let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        for i in &self.ids[range.clone()] {
            let p = points[*i as usize];
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        let node = self.nodes.len();
        self.nodes.push((lo, hi, range.clone(), None));
        if range.len() > 8 {
            let axis = (0..3)
                .max_by(|a, b| (hi[*a] - lo[*a]).total_cmp(&(hi[*b] - lo[*b])))
                .unwrap_or(0);
            let mid = range.start + range.len() / 2;
            self.ids[range.clone()].select_nth_unstable_by(mid - range.start, |a, b| {
                points[*a as usize][axis].total_cmp(&points[*b as usize][axis])
            });
            let children = [self.build(range.start..mid), self.build(mid..range.end)];
            self.nodes[node].3 = Some(children);
        }
        node
    }
    /// Calls `visit` with every point within `WELD` of the segment from `a`
    /// to `b`, and with some others near it.
    fn near(&self, a: [f64; 3], b: [f64; 3], mut visit: impl FnMut(u32)) {
        let mut stack = vec![0];
        while let Some(n) = stack.pop() {
            let (lo, hi, range, children) = &self.nodes[n];
            if !meets(a, b, lo.map(|v| v - WELD), hi.map(|v| v + WELD)) {
                continue;
            }
            match children {
                Some(c) => stack.extend(c),
                None => self.ids[range.clone()].iter().for_each(|i| visit(*i)),
            }
        }
    }
}
/// Whether the segment from `a` to `b` meets the box from `lo` to `hi`.
fn meets(a: [f64; 3], b: [f64; 3], lo: [f64; 3], hi: [f64; 3]) -> bool {
    let (mut t0, mut t1) = (0f64, 1f64);
    for k in 0..3 {
        let d = b[k] - a[k];
        if d == 0. {
            if a[k] < lo[k] || a[k] > hi[k] {
                return false;
            }
            continue;
        }
        let (u, v) = ((lo[k] - a[k]) / d, (hi[k] - a[k]) / d);
        t0 = t0.max(u.min(v));
        t1 = t1.min(u.max(v));
        if t0 > t1 {
            return false;
        }
    }
    true
}
fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|k| a[k] - b[k])
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    (0..3).map(|k| a[k] * b[k]).sum()
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
/// Unnormalized normal; its length is twice the triangle's area.
pub(crate) fn normal(p: &[[f32; 3]; 3]) -> [f32; 3] {
    let a: [f32; 3] = std::array::from_fn(|i| p[1][i] - p[0][i]);
    let b: [f32; 3] = std::array::from_fn(|i| p[2][i] - p[0][i]);
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
pub(crate) fn centroid(p: &[[f32; 3]; 3]) -> [f32; 3] {
    std::array::from_fn(|i| (p[0][i] + p[1][i] + p[2][i]) / 3.)
}
#[derive(Default)]
struct Face {
    z: f32,
    area: f32,
    visible: f32,
    flat: bool,
    bbox: [f32; 4],
}
/// The largest cutter diameter that still fits inside the plane's outline.
fn fitting_diameter(geometry: &Geometry, surfaces: &[usize], bbox: [f32; 4]) -> Option<f32> {
    let outline: Vec<Polygon> = geometry
        .triangles
        .iter()
        .filter(|t| t.feature.is_some_and(|id| surfaces.contains(&id)))
        .map(|t| t.p.iter().map(|v| (v[0] as f64, v[1] as f64)).collect())
        .collect();
    let region = cam::region(outline).ok()?;
    let (mut lo, mut hi) = (
        0f32,
        ((bbox[2] - bbox[0]).min(bbox[3] - bbox[1]) / 2.).max(0.),
    );
    for _ in 0..20 {
        let mid = (lo + hi) / 2.;
        let shrunk: Vec<Polygon> = region
            .inflate(-(mid as f64), JoinType::Round, EndType::Polygon, 2.)
            .into();
        if shrunk.iter().any(|p| cam::area(p).abs() > 1e-6) {
            lo = mid
        } else {
            hi = mid
        }
    }
    Some(lo * 2.)
}
fn orientation(
    setup1: &Setup,
    stock: &Volume,
    mesh: &Mesh,
    up: &'static str,
    max_holes: usize,
) -> Option<OrientationReport> {
    let setup = oriented(setup1, up)?;
    let geometry = Geometry::new(&setup, mesh);
    let index = Index::new(&geometry);
    let model = geometry.bounds;
    let mut faces: std::collections::BTreeMap<usize, Face> = Default::default();
    let mut freeform = 0.;
    for t in &geometry.triangles {
        let n = normal(&t.p);
        let twice_area = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        if twice_area < 1e-9 {
            continue;
        }
        let (c, slope) = (centroid(&t.p), n[2] / twice_area);
        let visible = index.height([c[0], c[1]]).is_none_or(|h| h <= c[2] + 0.01);
        if slope.abs() < FLAT {
            if slope > WALL && visible {
                freeform += twice_area / 2.;
            }
            continue;
        }
        let Some(id) = t.feature else { continue };
        let face = faces.entry(id).or_insert(Face {
            z: c[2],
            flat: true,
            bbox: [
                f32::INFINITY,
                f32::INFINITY,
                f32::NEG_INFINITY,
                f32::NEG_INFINITY,
            ],
            ..Default::default()
        });
        face.area += twice_area / 2.;
        if visible {
            face.visible += twice_area / 2.;
        }
        for p in &t.p {
            face.flat &= (p[2] - face.z).abs() <= 0.002;
            face.bbox = [
                face.bbox[0].min(p[0]),
                face.bbox[1].min(p[1]),
                face.bbox[2].max(p[0]),
                face.bbox[3].max(p[1]),
            ];
        }
    }
    // Coplanar visible faces at one height form one plane.
    let mut planes: std::collections::BTreeMap<i64, PlaneReport> = Default::default();
    for (id, face) in &faces {
        if !face.flat || face.visible < face.area * 0.5 {
            continue;
        }
        let plane = planes
            .entry((face.z * 500.).round() as i64)
            .or_insert(PlaneReport {
                surfaces: vec![],
                kind: "floor",
                depth_below_top: model.max[2] - face.z,
                area: 0.,
                bbox: face.bbox,
                max_cutter_diameter: None,
                holes: vec![],
            });
        plane.surfaces.push(*id);
        plane.area += face.area;
        plane.bbox = [
            plane.bbox[0].min(face.bbox[0]),
            plane.bbox[1].min(face.bbox[1]),
            plane.bbox[2].max(face.bbox[2]),
            plane.bbox[3].max(face.bbox[3]),
        ];
    }
    let mut planes: Vec<PlaneReport> = planes.into_values().filter(|p| p.area >= 1.).collect();
    planes.sort_by(|a, b| b.area.total_cmp(&a.area));
    let mut truncated = planes.len() > MAX_PLANES;
    planes.truncate(MAX_PLANES);
    let mut holes_left = max_holes;
    for plane in &mut planes {
        if plane.depth_below_top < 0.01 {
            plane.kind = "top";
        } else {
            plane.max_cutter_diameter =
                fitting_diameter(&geometry, &plane.surfaces, plane.bbox).map(r2);
        }
        let mut probe = Operation::new(Strategy::Drill, String::new());
        probe.geometry.clear();
        probe.surfaces = Some(Selection {
            mesh_fingerprint: mesh.fingerprint.clone(),
            ids: plane.surfaces.clone(),
        });
        // A plane's holes are listed completely or not at all, and planes
        // with too many openings are not searched.
        if cam::hole_candidates(&setup, &probe, mesh) > holes_left {
            truncated = true;
        } else {
            plane.holes = cam::holes(&setup, &probe, mesh)
                .unwrap_or_default()
                .iter()
                .map(|h| HoleReport {
                    center: [
                        r2(h.center[0] - model.min[0]),
                        r2(h.center[1] - model.min[1]),
                    ],
                    diameter: r2(h.diameter),
                    opening_diameter: r2(h.opening),
                    depth: r2(h.top - h.bottom),
                    through: h.bottom <= model.min[2] + 0.01,
                })
                .collect();
            holes_left = holes_left.saturating_sub(plane.holes.len());
        }
        plane.depth_below_top = r2(plane.depth_below_top);
        plane.area = r2(plane.area);
        plane.bbox = [
            r2(plane.bbox[0] - model.min[0]),
            r2(plane.bbox[1] - model.min[1]),
            r2(plane.bbox[2] - model.min[0]),
            r2(plane.bbox[3] - model.min[1]),
        ];
    }
    let stock: Bounds = stock.transformed_bounds(Frame::new(&setup, mesh));
    Some(OrientationReport {
        up,
        is_setup1: up_axis(setup1) == Some(up),
        model_size: model.dimensions().map(r2),
        stock_margin: [
            stock.max[2] - model.max[2],
            model.min[2] - stock.min[2],
            model.min[0] - stock.min[0],
            stock.max[0] - model.max[0],
            model.min[1] - stock.min[1],
            stock.max[1] - model.max[1],
        ]
        .map(r2),
        planes,
        freeform_area: r2(freeform),
        truncated,
    })
}
/// One line of the planner's readiness checklist.
#[derive(Debug, Clone, PartialEq)]
pub struct Precondition {
    pub label: &'static str,
    pub ok: bool,
    /// What to do when it is not met.
    pub hint: String,
}
/// What must be in place before a plan can be requested or checked.
pub fn preconditions(doc: &Document, mesh: Option<&Mesh>) -> Vec<Precondition> {
    let line = |label, problem: Option<String>| Precondition {
        label,
        ok: problem.is_none(),
        hint: problem.unwrap_or_default(),
    };
    let part = (doc.file.is_none() || mesh.is_none()).then(|| {
        "Import a part and wait for it to load. The built-in example cannot be planned.".to_string()
    });
    let setup = match doc.setups.first() {
        None => Some("Create setup 1 with your machine and stock.".to_string()),
        Some(setup1) => crate::post::PostSettings::from_machine(&setup1.machine)
            .err()
            .and_then(|e| e.into_iter().next())
            .or_else(|| crate::stock::resolve(setup1, mesh).err())
            .or_else(|| {
                up_axis(setup1).is_none().then(|| {
                    "Setup 1 must be square to the part: use rotations that are multiples of 90°.".into()
                })
            }),
    };
    let material = doc
        .material
        .trim()
        .is_empty()
        .then(|| "Enter the material.".to_string());
    let tools = if doc.tools.is_empty() {
        Some("Add at least one tool to the library.".to_string())
    } else {
        doc.tools
            .iter()
            .find_map(|t| doc.tool_number_issues(t).into_iter().next())
    };
    vec![
        line("Imported part", part),
        line("Setup 1: 3-axis LinuxCNC machine and stock", setup),
        line("Material", material),
        line("Tool library", tools),
    ]
}
/// Setup 1, the part axis that points up in it and its stock, or the first
/// reason planning cannot start.
fn basics<'a>(doc: &'a Document, mesh: &Mesh) -> Result<(&'a Setup, &'static str, Volume), String> {
    let setup1 = doc
        .setups
        .first()
        .ok_or("Create a machining setup first.")?;
    let first = up_axis(setup1)
        .ok_or("Setup 1 must be square to the part: use rotations that are multiples of 90°.")?;
    let stock = crate::stock::resolve(setup1, Some(mesh))?.stock;
    Ok((setup1, first, Volume::new(stock, Frame::new(setup1, mesh))))
}
/// The whole report, or the first reason planning cannot start. The part must
/// be a closed surface: no open edges, and vertical rays (in setup 1's frame)
/// that cross it consistently.
pub fn report(doc: &Document, mesh: &Mesh) -> Result<PartReport, String> {
    let (setup1, first, stock) = basics(doc, mesh)?;
    let open = open_edges(&mm_triangles(mesh));
    if open > 0 {
        return Err(format!(
            "The part is not a closed solid ({open} open edges), so the planner cannot check its plans against it. Repair or re-export the model."
        ));
    }
    if inconsistent_share(&Geometry::new(setup1, mesh)) > 0.02 {
        return Err("The part is not a closed solid, so the planner cannot check its plans against it. Repair or re-export the model.".into());
    }
    Ok(describe(doc, mesh, setup1, first, &stock))
}
/// The report without the closed-surface checks, so tests can run the plan
/// checks on open meshes.
#[cfg(test)]
pub(crate) fn report_unchecked(doc: &Document, mesh: &Mesh) -> Result<PartReport, String> {
    let (setup1, first, stock) = basics(doc, mesh)?;
    Ok(describe(doc, mesh, setup1, first, &stock))
}
fn describe(
    doc: &Document,
    mesh: &Mesh,
    setup1: &Setup,
    first: &'static str,
    stock: &Volume,
) -> PartReport {
    let mut order: Vec<&'static str> = vec![first];
    order.extend(
        ORIENTATIONS
            .iter()
            .map(|(up, _)| *up)
            .filter(|up| *up != first),
    );
    let machine = &setup1.machine;
    let inch = if get(&machine.p, "units") == "inch" {
        25.4
    } else {
        1.
    };
    PartReport {
        units: "mm",
        part_size: mesh.dimensions.map(r2),
        material: doc.material.clone(),
        machine: MachineReport {
            max_rpm: num(&machine.p, "rpm").unwrap_or(0.),
            max_feed: r2(num(&machine.p, "feed").unwrap_or(0.) * inch),
            travel: ["x", "y", "z"].map(|k| r2(num(&machine.p, k).unwrap_or(0.) * inch)),
        },
        tools: doc
            .tools
            .iter()
            .filter_map(|t| {
                Some(ToolReport {
                    number: t.number()?,
                    name: t.name().to_string(),
                    kind: get(&t.p, "type").to_string(),
                    diameter: num(&t.p, "diameter")?,
                    flute_length: num(&t.p, "length")?,
                    stickout: num(&t.p, "stickout")?,
                })
            })
            .collect(),
        orientations: order
            .into_iter()
            .filter_map(|up| orientation(setup1, stock, mesh, up, MAX_HOLES))
            .collect(),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::post::fixture;
    fn block() -> (Document, Mesh) {
        fixture::job()
    }
    fn plane<'a>(o: &'a OrientationReport, depth: f32) -> &'a PlaneReport {
        o.planes
            .iter()
            .find(|p| (p.depth_below_top - depth).abs() < 0.01)
            .unwrap_or_else(|| panic!("no plane at depth {depth} in {}", o.up))
    }
    #[test]
    fn the_example_block_reports_its_top_pocket_and_holes() {
        let (doc, mesh) = block();
        let report = report(&doc, &mesh).unwrap();
        assert_eq!(report.part_size, [120., 80., 40.]);
        assert_eq!(report.units, "mm");
        let ups: Vec<_> = report.orientations.iter().map(|o| o.up).collect();
        assert_eq!(ups, vec!["+Z", "-Z", "+X", "-X", "+Y", "-Y"]);
        let top = &report.orientations[0];
        assert!(top.is_setup1 && !report.orientations[1].is_setup1);
        assert_eq!(top.model_size, [120., 80., 40.]);
        // fixture stock: top 4, bottom 3, sides 2
        assert_eq!(top.stock_margin, [4., 3., 2., 2., 2., 2.]);
        let face = plane(top, 0.);
        assert_eq!(face.kind, "top");
        assert_eq!(face.max_cutter_diameter, None);
        assert_eq!(face.holes.len(), 5);
        let through: Vec<_> = face.holes.iter().filter(|h| h.through).collect();
        assert_eq!(through.len(), 4);
        assert!(
            through
                .iter()
                .all(|h| h.diameter == 6.6 && h.opening_diameter == 11. && h.depth == 40.)
        );
        let blind = face.holes.iter().find(|h| !h.through).unwrap();
        assert_eq!((blind.diameter, blind.depth), (6., 8.));
        assert_eq!(blind.center, [42., 40.]);
        let pocket = plane(top, 12.);
        assert_eq!(pocket.kind, "floor");
        assert_eq!(pocket.bbox, [16., 20., 88., 60.]);
        assert!((pocket.max_cutter_diameter.unwrap() - 37.05).abs() < 0.2);
        let counterbores = plane(top, 6.);
        assert_eq!(counterbores.surfaces.len(), 4);
        assert!(counterbores.max_cutter_diameter.unwrap() < 2.3);
        assert!(top.freeform_area > 100.);
        assert!(!top.truncated);
        assert!(!serde_json::to_string(&report).unwrap().contains("-0.0"));
    }
    #[test]
    fn each_orientation_lists_only_what_is_visible_from_above() {
        let (doc, mesh) = block();
        let report = report(&doc, &mesh).unwrap();
        let by_up = |up: &str| report.orientations.iter().find(|o| o.up == up).unwrap();
        let bottom = by_up("-Z");
        assert_eq!(bottom.stock_margin, [3., 4., 2., 2., 2., 2.]);
        assert!(plane(bottom, 6.).max_cutter_diameter.unwrap() > 37.);
        assert_eq!(plane(bottom, 0.).holes.len(), 6);
        // The top pocket floor is not reachable from below or from the sides.
        let top_pocket = plane(by_up("+Z"), 12.).surfaces.clone();
        for up in ["-Z", "+X", "-X", "+Y", "-Y"] {
            assert!(
                by_up(up).planes.iter().all(|p| p.surfaces != top_pocket),
                "{up}"
            );
        }
        assert_eq!(by_up("+X").model_size, [40., 80., 120.]);
        assert_eq!(plane(by_up("+X"), 0.).holes.len(), 2);
        assert_eq!(by_up("-X").planes.len(), 1);
        let side = by_up("-Y");
        assert_eq!(side.model_size, [120., 40., 80.]);
        let bore = plane(side, 0.)
            .holes
            .iter()
            .find(|h| h.opening_diameter == 24.)
            .unwrap();
        assert!((bore.diameter - 16.).abs() < 0.02 && bore.depth == 14. && !bore.through);
        // every surface is listed in at most one orientation
        let mut seen = std::collections::HashSet::new();
        for id in report
            .orientations
            .iter()
            .flat_map(|o| &o.planes)
            .flat_map(|p| &p.surfaces)
        {
            assert!(seen.insert(*id));
        }
    }
    #[test]
    fn setup_one_must_be_square_and_keeps_its_own_rotation() {
        let (mut doc, mesh) = block();
        doc.setups[0].p.insert("rz".into(), "90".into());
        assert_eq!(up_axis(&doc.setups[0]), Some("+Z"));
        assert_eq!(rotation_for(&doc.setups[0], "+Z"), Some([0., 0., 90.]));
        assert_eq!(rotation_for(&doc.setups[0], "-Z"), Some([180., 0., 0.]));
        let turned = report(&doc, &mesh).unwrap();
        assert_eq!(turned.orientations[0].model_size, [80., 120., 40.]);
        doc.setups[0].p.insert("rx".into(), "180".into());
        doc.setups[0].p.insert("rz".into(), "0".into());
        assert_eq!(report(&doc, &mesh).unwrap().orientations[0].up, "-Z");
        doc.setups[0].p.insert("rx".into(), "30".into());
        assert!(report(&doc, &mesh).unwrap_err().contains("multiples of 90"));
        doc.setups.clear();
        assert!(report(&doc, &mesh).is_err());
        for (up, rotation) in ORIENTATIONS {
            let mut setup = Setup::new(crate::model::Machine::new(3), "G54");
            for (k, v) in ["rx", "ry", "rz"].iter().zip(rotation) {
                setup.p.insert((*k).into(), v.to_string());
            }
            assert_eq!(up_axis(&setup), Some(up));
        }
    }
    #[test]
    fn preconditions_name_what_is_missing() {
        let (mut doc, mesh) = block();
        let unmet = |doc: &Document, mesh: Option<&Mesh>| -> Vec<&'static str> {
            preconditions(doc, mesh)
                .into_iter()
                .filter(|p| !p.ok)
                .map(|p| p.label)
                .collect()
        };
        assert_eq!(unmet(&doc, Some(&mesh)), vec!["Material"]);
        doc.material = "Aluminum 6061".into();
        assert!(unmet(&doc, Some(&mesh)).is_empty());
        assert_eq!(
            unmet(&doc, None),
            vec![
                "Imported part",
                "Setup 1: 3-axis LinuxCNC machine and stock"
            ]
        );
        let mut other = doc.clone();
        other.setups[0]
            .machine
            .p
            .insert("controller".into(), "GRBL".into());
        other.tools[1].p.insert("number".into(), "1".into());
        other.file = None;
        let lines = preconditions(&other, Some(&mesh));
        assert_eq!(lines.iter().filter(|p| !p.ok).count(), 3);
        assert!(lines[1].hint.contains("LinuxCNC") && lines[3].hint.contains("also used"));
        other.setups[0].machine = doc.setups[0].machine.clone();
        other.setups[0].p.insert("ry".into(), "45".into());
        assert!(
            preconditions(&other, Some(&mesh))[1]
                .hint
                .contains("multiples of 90")
        );
        other.setups.clear();
        other.tools.clear();
        let lines = preconditions(&other, Some(&mesh));
        assert!(
            lines[1].hint.contains("Create setup 1") && lines[3].hint.contains("at least one tool")
        );
    }
    #[test]
    fn the_report_lists_tools_machine_limits_and_serializes() {
        let (mut doc, mesh) = block();
        doc.material = "Aluminum 6061".into();
        doc.tools[1].p.remove("number");
        let report = report(&doc, &mesh).unwrap();
        assert_eq!(
            report.tools.iter().map(|t| t.number).collect::<Vec<_>>(),
            vec![1, 3]
        );
        assert_eq!(report.tools[0].kind, "Flat end mill");
        assert_eq!(
            (report.machine.max_rpm, report.machine.max_feed),
            (12000., 5000.)
        );
        let json = serde_json::to_string(&report).unwrap();
        assert!(
            json.contains("\"material\":\"Aluminum 6061\"")
                && json.contains("\"max_cutter_diameter\"")
        );
        assert!(json.len() < 20_000, "{}", json.len());
    }
    #[test]
    fn a_plane_with_more_hole_candidates_than_the_limit_lists_none() {
        let (doc, mesh) = block();
        let setup1 = &doc.setups[0];
        let stock = crate::stock::resolve(setup1, Some(&mesh)).unwrap().stock;
        let stock = Volume::new(stock, Frame::new(setup1, &mesh));
        let full = orientation(setup1, &stock, &mesh, "+Z", MAX_HOLES).unwrap();
        assert_eq!(plane(&full, 0.).holes.len(), 5);
        assert!(!full.truncated);
        let limited = orientation(setup1, &stock, &mesh, "+Z", 3).unwrap();
        assert!(limited.truncated);
        assert!(plane(&limited, 0.).holes.is_empty());
    }
    type Tri = [[f32; 3]; 3];
    fn quad(t: &mut Vec<Tri>, p: [[f32; 3]; 4]) {
        t.push([p[0], p[1], p[2]]);
        t.push([p[0], p[2], p[3]]);
    }
    /// The four outward-facing walls of an `x` × `y` × `z` box.
    fn walls(t: &mut Vec<Tri>, x: f32, y: f32, z: f32) {
        quad(t, [[0., 0., 0.], [x, 0., 0.], [x, 0., z], [0., 0., z]]);
        quad(t, [[x, y, 0.], [0., y, 0.], [0., y, z], [x, y, z]]);
        quad(t, [[0., y, 0.], [0., 0., 0.], [0., 0., z], [0., y, z]]);
        quad(t, [[x, 0., 0.], [x, y, 0.], [x, y, z], [x, 0., z]]);
    }
    /// A 40 × 30 × 20 box, with or without its top and bottom faces. The last
    /// two triangles are the +X wall.
    fn box_triangles(top: bool, bottom: bool) -> Vec<Tri> {
        let mut t = vec![];
        let (x, y, z) = (40., 30., 20.);
        if top {
            quad(&mut t, [[0., 0., z], [x, 0., z], [x, y, z], [0., y, z]]);
        }
        if bottom {
            quad(&mut t, [[0., 0., 0.], [0., y, 0.], [x, y, 0.], [x, 0., 0.]]);
        }
        walls(&mut t, x, y, z);
        t
    }
    fn box_mesh(top: bool, bottom: bool) -> Mesh {
        crate::mesh::normalize(box_triangles(top, bottom)).unwrap()
    }
    /// A 120 × 80 × 40 box with a centred `top` × `top` square missing from
    /// its top face and a `bottom` × `bottom` one missing from its bottom face.
    pub(crate) fn holed(top: f32, bottom: f32) -> Mesh {
        let (x, y, z) = (120f32, 80f32, 40f32);
        let mut t = vec![];
        for (h, zz, up) in [(top, z, true), (bottom, 0., false)] {
            let (a0, a1, b0, b1) = ((x - h) / 2., (x + h) / 2., (y - h) / 2., (y + h) / 2.);
            let quads = if h > 0. {
                vec![
                    [[0., 0., zz], [x, 0., zz], [a1, b0, zz], [a0, b0, zz]],
                    [[x, 0., zz], [x, y, zz], [a1, b1, zz], [a1, b0, zz]],
                    [[x, y, zz], [0., y, zz], [a0, b1, zz], [a1, b1, zz]],
                    [[0., y, zz], [0., 0., zz], [a0, b0, zz], [a0, b1, zz]],
                ]
            } else {
                vec![[[0., 0., zz], [x, 0., zz], [x, y, zz], [0., y, zz]]]
            };
            for mut q in quads {
                if !up {
                    q.reverse();
                }
                quad(&mut t, q);
            }
        }
        walls(&mut t, x, y, z);
        crate::mesh::normalize(t).unwrap()
    }
    /// A 120 × 80 × 40 box with a 40 × 30 × 6 pocket open from below, and a
    /// centred `patch` × `patch` square missing from the top face, over the pocket.
    pub(crate) fn pocket_box(patch: f32) -> Mesh {
        let (x, y, z, d) = (120f32, 80f32, 40f32, 6f32);
        let (p0, p1, q0, q1) = (40f32, 80f32, 25f32, 55f32);
        let (a0, a1) = ((x - patch) / 2., (x + patch) / 2.);
        let (b0, b1) = ((y - patch) / 2., (y + patch) / 2.);
        let mut t = vec![];
        if patch > 0. {
            quad(&mut t, [[0., 0., z], [x, 0., z], [a1, b0, z], [a0, b0, z]]);
            quad(&mut t, [[x, 0., z], [x, y, z], [a1, b1, z], [a1, b0, z]]);
            quad(&mut t, [[x, y, z], [0., y, z], [a0, b1, z], [a1, b1, z]]);
            quad(&mut t, [[0., y, z], [0., 0., z], [a0, b0, z], [a0, b1, z]]);
        } else {
            quad(&mut t, [[0., 0., z], [x, 0., z], [x, y, z], [0., y, z]]);
        }
        let down = |t: &mut Vec<Tri>, p: [[f32; 3]; 4]| quad(t, [p[3], p[2], p[1], p[0]]);
        // The bottom face around the pocket's opening, and the pocket's ceiling.
        down(
            &mut t,
            [[0., 0., 0.], [x, 0., 0.], [p1, q0, 0.], [p0, q0, 0.]],
        );
        down(
            &mut t,
            [[x, 0., 0.], [x, y, 0.], [p1, q1, 0.], [p1, q0, 0.]],
        );
        down(
            &mut t,
            [[x, y, 0.], [0., y, 0.], [p0, q1, 0.], [p1, q1, 0.]],
        );
        down(
            &mut t,
            [[0., y, 0.], [0., 0., 0.], [p0, q0, 0.], [p0, q1, 0.]],
        );
        down(&mut t, [[p0, q0, d], [p1, q0, d], [p1, q1, d], [p0, q1, d]]);
        quad(
            &mut t,
            [[p0, q0, 0.], [p0, q0, d], [p1, q0, d], [p1, q0, 0.]],
        );
        quad(
            &mut t,
            [[p1, q0, 0.], [p1, q0, d], [p1, q1, d], [p1, q1, 0.]],
        );
        quad(
            &mut t,
            [[p1, q1, 0.], [p1, q1, d], [p0, q1, d], [p0, q1, 0.]],
        );
        quad(
            &mut t,
            [[p0, q1, 0.], [p0, q1, d], [p0, q0, d], [p0, q0, 0.]],
        );
        walls(&mut t, x, y, z);
        crate::mesh::normalize(t).unwrap()
    }
    /// UV sphere triangles of radius `r`: many triangles meet at its poles,
    /// and its seam's vertices are computed twice.
    fn sphere_triangles(n: usize, m: usize, r: f32) -> Vec<Tri> {
        let point = |i: usize, j: usize| {
            let theta = std::f32::consts::TAU * i as f32 / n as f32;
            let phi = std::f32::consts::PI * j as f32 / m as f32;
            [
                r * phi.sin() * theta.cos(),
                r * phi.sin() * theta.sin(),
                r * phi.cos(),
            ]
        };
        let mut t = vec![];
        for i in 0..n {
            for j in 0..m {
                let (a, b) = (point(i, j), point(i + 1, j));
                let (c, d) = (point(i + 1, j + 1), point(i, j + 1));
                if j != 0 {
                    t.push([a, d, b]);
                }
                if j != m - 1 {
                    t.push([b, d, c]);
                }
            }
        }
        t
    }
    fn sphere() -> Mesh {
        crate::mesh::normalize(sphere_triangles(48, 24, 30.)).unwrap()
    }
    /// `t` with the longest edge of every `every`-th triangle split into
    /// `pieces` by a fan from the opposite corner, and its neighbours left
    /// alone: T-junctions, still closed.
    fn split(t: &[Tri], every: usize, pieces: usize) -> Vec<Tri> {
        let mut out = vec![];
        for (i, p) in t.iter().enumerate() {
            if i % every != 0 {
                out.push(*p);
                continue;
            }
            let length = |k: usize| {
                (0..3)
                    .map(|c| (p[k][c] - p[(k + 1) % 3][c]).powi(2))
                    .sum::<f32>()
            };
            let e = (0..3)
                .max_by(|a, b| length(*a).total_cmp(&length(*b)))
                .unwrap();
            let (a, b, c) = (p[e], p[(e + 1) % 3], p[(e + 2) % 3]);
            let at = |s: usize| -> [f32; 3] {
                let s = s as f32 / pieces as f32;
                std::array::from_fn(|k| a[k] + (b[k] - a[k]) * s)
            };
            for s in 0..pieces {
                out.push([at(s), at(s + 1), c]);
            }
        }
        out
    }
    /// The mesh's triangles in mm in its own frame, moved by `offset`.
    fn source(mesh: &Mesh, offset: [f32; 3]) -> Vec<Tri> {
        let scale = mesh.display_scale();
        mesh.faces
            .iter()
            .filter(|f| f.points.len() == 3)
            .map(|f| {
                std::array::from_fn(|i| std::array::from_fn(|k| f.points[i][k] / scale + offset[k]))
            })
            .collect()
    }
    /// `mesh` with every third triangle split at the midpoint of its longest
    /// edge and its neighbours left alone: T-junctions everywhere, still closed.
    fn t_junctions(mesh: &Mesh) -> Mesh {
        crate::mesh::normalize(split(&source(mesh, [0.; 3]), 3, 2)).unwrap()
    }
    /// A closed cylinder (radius 30, height 20) whose caps are fans of `n`
    /// triangles around their centres, with every third triangle split at the
    /// midpoint of its longest edge: long open edges that meet at the centres.
    fn fan_cylinder(n: usize) -> Vec<Tri> {
        let at = |i: usize, z: f32| {
            let a = std::f32::consts::TAU * (i % n) as f32 / n as f32;
            [30. * a.cos(), 30. * a.sin(), z]
        };
        let mut t = vec![];
        for i in 0..n {
            t.push([[0., 0., 20.], at(i, 20.), at(i + 1, 20.)]);
            t.push([[0., 0., 0.], at(i + 1, 0.), at(i, 0.)]);
            t.push([at(i, 0.), at(i + 1, 0.), at(i + 1, 20.)]);
            t.push([at(i, 0.), at(i + 1, 20.), at(i, 20.)]);
        }
        split(&t, 3, 2)
    }
    /// `t` with every corner moved `by` mm along a diagonal of its own, so
    /// copies of one vertex are up to twice that apart.
    fn jittered(t: &[Tri], by: f32) -> Vec<Tri> {
        let step = by / 3f32.sqrt();
        let mut seed = 7u64;
        t.iter()
            .map(|p| {
                p.map(|q| {
                    seed = seed
                        .wrapping_mul(6364136223846793005)
                        .wrapping_add(1442695040888963407);
                    std::array::from_fn(|k| {
                        q[k] + if (seed >> (40 + k)) & 1 == 0 {
                            step
                        } else {
                            -step
                        }
                    })
                })
            })
            .collect()
    }
    /// Open edges of `t`.
    fn count(t: &[Tri]) -> usize {
        open_edges(
            &t.iter()
                .map(|p| p.map(|q| q.map(f64::from)))
                .collect::<Vec<_>>(),
        )
    }
    /// Open edges of `mesh`.
    fn edges(mesh: &Mesh) -> usize {
        open_edges(&mm_triangles(mesh))
    }
    fn not_closed(n: usize) -> String {
        format!(
            "The part is not a closed solid ({n} open edges), so the planner cannot check its plans against it. Repair or re-export the model."
        )
    }
    const NOT_SOLID: &str = "The part is not a closed solid, so the planner cannot check its plans against it. Repair or re-export the model.";
    #[test]
    fn open_edges_are_the_edges_welding_and_splitting_leave_unmatched() {
        let (o, a, b, c) = ([0., 0., 0.], [10., 0., 0.], [0., 10., 0.], [0., 0., 10.]);
        let tetrahedron = |moved: [f32; 3]| vec![[moved, b, a], [o, a, c], [o, c, b], [a, b, c]];
        assert_eq!(count(&tetrahedron(o)), 0);
        assert_eq!(count(&[[o, a, b]]), 3);
        // Copies of a vertex up to 0.001 mm apart are one vertex.
        for off in [0.00005, 0.0005, 0.00099] {
            assert_eq!(count(&tetrahedron([0., 0., -off])), 0, "{off}");
        }
        // 0.01 mm apart: four open edges.
        assert_eq!(count(&tetrahedron([0., 0., -0.01])), 4);
        // A zero-area triangle, a duplicated triangle and a copy with its
        // winding reversed change nothing.
        let mut t = tetrahedron(o);
        t.extend([[o, [5., 0., 0.], a], [o, a, c], [o, c, a]]);
        assert_eq!(count(&t), 0);
        // A triangle written with both windings is one open triangle.
        assert_eq!(count(&[[o, a, b], [o, b, a]]), 3);
        // A T-junction: the long edge is split at the vertex in its middle.
        let m = [5., 5., 0.];
        let flat = |top: Vec<Tri>| {
            let mut t = top;
            t.extend([[o, b, c], [o, c, a], [a, c, b]]);
            t
        };
        assert_eq!(count(&flat(vec![[o, a, m], [o, m, b]])), 0);
        // Moved 0.01 mm off the long edge, the vertex does not split it.
        let off = [5.007, 5.007, 0.];
        assert_eq!(count(&flat(vec![[o, a, off], [o, off, b]])), 3);
        // Coordinates beyond 1 000 000 mm or not finite: every edge is open.
        let far: Vec<Tri> = tetrahedron(o)
            .iter()
            .map(|p| p.map(|q| q.map(|v| v * 2e5)))
            .collect();
        assert_eq!(count(&far), 12);
        let mut nan = tetrahedron(o);
        nan[1][0][0] = f32::NAN;
        assert_eq!(count(&nan), 12);
        assert_eq!(count(&[[[1e16, 0., 0.], [1e16, 1e16, 0.], b]]), 3);
    }
    #[test]
    fn the_part_must_be_a_closed_surface() {
        let (doc, mesh) = block();
        // The reviewers' case: matching 12 × 12 squares missing from the top
        // and the bottom. No vertical ray crosses the gap, as in a through hole.
        let patched = holed(12., 12.);
        assert_eq!(edges(&patched), 8);
        assert_eq!(report(&doc, &patched).unwrap_err(), not_closed(8));
        // One triangle missing from a wall, which vertical rays cannot see.
        let mut t = box_triangles(true, true);
        t.pop();
        let missing = crate::mesh::normalize(t).unwrap();
        assert_eq!(edges(&missing), 3);
        assert_eq!(report(&doc, &missing).unwrap_err(), not_closed(3));
        // A UV sphere with T-junctions and every tenth triangle removed.
        let holes: Vec<Tri> = split(&sphere_triangles(48, 24, 30.), 3, 2)
            .into_iter()
            .enumerate()
            .filter(|(i, _)| i % 10 != 0)
            .map(|(_, p)| p)
            .collect();
        let holes = crate::mesh::normalize(holes).unwrap();
        let n = edges(&holes);
        assert!(n >= 300, "{n}");
        assert_eq!(report(&doc, &holes).unwrap_err(), not_closed(n));
        // The box and an open cup (radius 1, height 5) whose 10 000 rim
        // vertices are 0.0006 mm apart: a row too fine to weld into one vertex.
        let n = 10_000;
        let at = |i: usize, z: f32| {
            let a = std::f32::consts::TAU * (i % n) as f32 / n as f32;
            [20. + a.cos(), 15. + a.sin(), z]
        };
        let mut t = box_triangles(true, true);
        for i in 0..n {
            t.push([[20., 15., 2.], at(i + 1, 2.), at(i, 2.)]);
            t.push([at(i, 2.), at(i + 1, 2.), at(i + 1, 7.)]);
            t.push([at(i, 2.), at(i + 1, 7.), at(i, 7.)]);
        }
        let cup = crate::mesh::normalize(t).unwrap();
        assert_eq!(edges(&cup), n);
        assert_eq!(report(&doc, &cup).unwrap_err(), not_closed(n));
        // The +X wall re-triangulated with a vertex in the middle of its top
        // edge, which the top face does not have: a T-junction.
        let (x, y, z) = (40., 30., 20.);
        let mut t = box_triangles(true, true);
        t.truncate(t.len() - 2);
        let mid = [x, y / 2., z];
        t.extend([
            [[x, 0., 0.], [x, y, 0.], mid],
            [[x, y, 0.], [x, y, z], mid],
            [mid, [x, 0., z], [x, 0., 0.]],
        ]);
        let mut twice = mesh.clone();
        twice.faces.push(mesh.faces[0].clone());
        let closed = [
            ("box", box_mesh(true, true)),
            ("T-junction box", crate::mesh::normalize(t).unwrap()),
            ("UV sphere", sphere()),
            ("T-junction example block", t_junctions(&mesh)),
            ("example block with a triangle written twice", twice),
            (
                "UV sphere with vertex copies up to 0.0002 mm apart",
                crate::mesh::normalize(jittered(&sphere_triangles(96, 48, 30.), 0.0001)).unwrap(),
            ),
            (
                "fan-capped T-junction cylinder",
                crate::mesh::normalize(fan_cylinder(300)).unwrap(),
            ),
            ("example block", mesh),
        ];
        for (name, m) in &closed {
            assert_eq!(edges(m), 0, "{name}");
            assert!(report(&doc, m).is_ok(), "{name}");
        }
    }
    #[test]
    fn t_junctions_far_from_the_origin_are_closed() {
        let (doc, mesh) = block();
        for (offset, every, pieces) in [
            ([412.3, -287.6, 95.1], 10, 30),
            ([1012.3, 687.6, 0.], 10, 30),
            ([0.; 3], 50, 100),
        ] {
            let m = crate::mesh::normalize(split(&source(&mesh, offset), every, pieces)).unwrap();
            assert_eq!(edges(&m), 0, "{offset:?}");
            assert!(report(&doc, &m).is_ok(), "{offset:?}");
        }
    }
    #[test]
    fn closedness_does_not_depend_on_where_setup_1_puts_the_part() {
        let (mut doc, mesh) = block();
        for (key, value) in [("tx", "500"), ("ty", "-300"), ("tz", "80")] {
            doc.setups[0].p.insert(key.into(), value.into());
        }
        let tj = t_junctions(&mesh);
        assert!(report(&doc, &mesh).is_ok());
        assert!(report(&doc, &tj).is_ok());
    }
    #[test]
    fn the_edge_test_is_quick_where_long_open_edges_meet() {
        let t = fan_cylinder(3750);
        assert_eq!(t.len(), 20_000);
        let start = std::time::Instant::now();
        assert_eq!(count(&t), 0);
        let took = start.elapsed();
        assert!(took < std::time::Duration::from_secs(2), "{took:?}");
    }
    #[test]
    fn the_example_models_are_closed_surfaces() {
        let examples = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples");
        for file in [
            "four-setup-block.3mf",
            "four-setup-block.step",
            "inch-block.step",
            "placed-assembly.step",
            "preview-cube.obj",
        ] {
            if file.ends_with(".step") && option_env!("AXIS_BUILT_STEP_HELPER").is_none() {
                continue;
            }
            let mesh = crate::mesh::load(&examples.join(file)).unwrap();
            assert_eq!(edges(&mesh), 0, "{file}");
        }
    }
    #[test]
    fn an_inconsistent_column_is_solid_where_its_crossings_say_so() {
        let (doc, _) = block();
        let column = |setup: &Setup, mesh: &Mesh| {
            let geometry = Geometry::new(setup, mesh);
            let index = Index::new(&geometry);
            let b = geometry.bounds;
            let centre = [0, 1].map(|k| (b.min[k] + b.max[k]) / 2. + 1e-4);
            let spans = solid(&index, centre, b.max[2]);
            // From the model's bottom, in mm.
            spans
                .iter()
                .map(|(lo, hi)| [lo - b.min[2], hi - b.min[2]].map(|v| (v * 1000.).round() / 1000.))
                .collect::<Vec<_>>()
        };
        let top = &doc.setups[0];
        // Only the top crossing (the underside square is missing): it faces
        // up, so the column is solid from the model's bottom up to it.
        assert_eq!(column(top, &holed(0., 12.)), vec![[0., 40.]]);
        // Only the bottom crossing, facing down: solid from it to the model top.
        assert_eq!(column(top, &holed(12., 0.)), vec![[0., 40.]]);
        // The pocket open from below, under a missing top square, turned over:
        // only the pocket's floor, facing up, so the pocket above it is empty.
        let flipped = oriented(top, "-Z").unwrap();
        assert_eq!(column(&flipped, &pocket_box(12.)), vec![[0., 34.]]);
        assert_eq!(column(top, &pocket_box(12.)), vec![[6., 40.]]);
        // Consistent columns are unchanged.
        assert_eq!(column(&flipped, &pocket_box(0.)), vec![[0., 34.]]);
        assert_eq!(column(top, &holed(0., 0.)), vec![[0., 40.]]);
    }
    #[test]
    fn the_part_must_be_a_closed_solid() {
        let (doc, mesh) = block();
        assert!(report(&doc, &mesh).is_ok());
        assert_eq!(
            inconsistent_share(&Geometry::new(&doc.setups[0], &mesh)),
            0.
        );
        assert!(report(&doc, &box_mesh(true, true)).is_ok());
        assert_eq!(
            report(&doc, &box_mesh(false, true)).unwrap_err(),
            not_closed(4)
        );
        assert_eq!(
            report(&doc, &box_mesh(false, false)).unwrap_err(),
            not_closed(8)
        );
        // Closed, but its top faces inward, so every test ray crosses the
        // surface inconsistently.
        let mut t = box_triangles(true, true);
        for p in t.iter_mut().take(2) {
            p.swap(1, 2);
        }
        let flipped = crate::mesh::normalize(t).unwrap();
        assert_eq!(edges(&flipped), 0);
        assert_eq!(report(&doc, &flipped).unwrap_err(), NOT_SOLID);
    }
}
