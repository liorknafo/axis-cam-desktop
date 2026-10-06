//! Deterministic summary of the imported part for the AI planner: what can be
//! reached from each of the six axis-aligned orientations. Lengths are mm.
use crate::{
    cam::{self, Geometry, Polygon},
    material::BANDS,
    mesh::{Bounds, Mesh, rotation},
    model::{Document, Operation, Setup, Strategy, get, num},
    proposal::clip,
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
    pub material: MaterialReport,
    pub machine: MachineReport,
    pub tools: Vec<ToolReport>,
    /// Setup 1's modelled vise or clamps; absent without them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workholding: Option<WorkholdingReport>,
    /// The first entry is setup 1's orientation.
    pub orientations: Vec<OrientationReport>,
}
/// Setup 1's vise or clamps as keep-out zones. Only their kind, size, place
/// and clearance gap are told: never the library entry's name or notes.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WorkholdingReport {
    /// "Vise" or "Clamps".
    pub kind: &'static str,
    /// Each jaw or clamp as x0, y0, z0, x1, y1, z1 in setup 1's axes, measured
    /// from the model's -X/-Y/bottom corner, without the gap. A jaw is shown
    /// from the stock's bottom up.
    pub boxes: Vec<[f32; 6]>,
    /// The clearance every box is grown by for the check.
    pub gap: f32,
    /// How far above the stock top the highest box and its gap reach; absent
    /// when none reaches above the stock top.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clearance_above_stock_top: Option<f32>,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MachineReport {
    pub max_rpm: f32,
    pub max_feed: f32,
    pub travel: [f32; 3],
}
/// The project material and its cutting data, as the model sees them.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MaterialReport {
    pub name: String,
    pub group: String,
    pub hardness: String,
    pub notes: String,
    pub vc_min_m_per_min: f32,
    pub vc_max_m_per_min: f32,
    pub chip_load_mm: Vec<ChipLoadBand>,
}
/// Chip load per tooth (mm) for tool diameters up to `up_to_mm` (and above
/// the previous band); `None` is every larger diameter.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ChipLoadBand {
    pub up_to_mm: Option<f32>,
    pub min: f32,
    pub max: f32,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ToolReport {
    pub number: u32,
    pub name: String,
    pub kind: String,
    pub diameter: f32,
    pub flutes: u32,
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
/// first: height, and whether the face points up. Crossings within 0.0001 mm
/// of the one below them are at one height, and there those facing down come
/// first, whatever the triangle order, so a pair facing both ways errs
/// towards solid.
fn crossings(index: &Index, xy: [f32; 2]) -> Vec<(f32, bool)> {
    let mut hits: Vec<(f32, bool)> = index
        .near(xy)
        .filter_map(|t| Some((cam::plane_height(&t.p, xy)?, normal(&t.p)[2] > 0.)))
        .collect();
    hits.sort_by(|a, b| a.0.total_cmp(&b.0));
    for run in hits.chunk_by_mut(|a, b| b.0 - a.0 < 1e-4) {
        run.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.total_cmp(&b.0)));
    }
    hits.dedup_by(|a, b| (a.0 - b.0).abs() < 1e-4 && a.1 == b.1);
    hits
}
/// Solid spans from `hits`, lowest first, or `None` when the face directions
/// are inconsistent. A column whose spans are all of zero length (a crossing
/// facing down and one facing up within 0.0001 mm of one height, with
/// nothing solid on either side, as through a sheet written with both
/// windings) is inconsistent too.
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
    let solid = spans.is_empty() || spans.iter().any(|(lo, hi)| hi - lo >= 1e-4);
    (consistent && depth == 0 && solid).then_some(spans)
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
/// Coordinates beyond this are not checked (mm).
const FAR: f64 = 1_000_000.;
/// The part's triangles in mm, in the mesh's own frame (whether a surface is
/// closed does not depend on where setup 1 puts it), and the largest absolute
/// coordinate the part had in its file (mm).
fn mm_triangles(mesh: &Mesh) -> (Vec<[[f64; 3]; 3]>, f64) {
    let scale = f64::from(mesh.display_scale());
    let origin = mesh.origin.map(f64::from);
    let mut largest = 0f64;
    let triangles = mesh
        .faces
        .iter()
        .filter(|f| f.points.len() == 3)
        .map(|f| {
            std::array::from_fn(|i| {
                let p = f.points[i].map(f64::from);
                for k in 0..3 {
                    largest = largest.max(((p[k] - origin[k]) / scale).abs());
                }
                p.map(|v| v / scale)
            })
        })
        .collect();
    (triangles, largest)
}
/// What the closed-surface test finds.
#[derive(Debug, PartialEq)]
struct Closure {
    open_edges: usize,
    encloses_volume: bool,
    /// A coordinate lies beyond `FAR` or is not finite.
    far: bool,
}
/// How far apart two points may be and still be one point, in a part whose
/// coordinates reach `largest` mm: 8 f32 steps at that coordinate (float
/// rounding), or √3 times the last digit of 7 significant digits there if
/// that is more (coordinates written as text, as ASCII STL's `%.6e`), with
/// the reach taken as at least 1 mm.
fn tolerance(largest: f64) -> f64 {
    let f = largest as f32;
    let float = 8. * f64::from(f32::from_bits(f.to_bits() + 1) - f);
    // The power of ten at or below the reach.
    let mut decade = 1f64;
    while decade < FAR && decade * 10. <= largest {
        decade *= 10.;
    }
    float.max(3f64.sqrt() * decade / 1e6)
}
/// The closed-surface test on `triangles` (mm), whose coordinates reached
/// `largest` mm in the part's file. The tolerance comes from the largest
/// absolute coordinate in either frame (see `tolerance`). Points within it of
/// each other are one vertex unless they span more than twice the tolerance
/// (see `weld`), zero-area triangles are ignored, and a triangle written more
/// than once with the same winding counts once; with opposite windings both
/// count, and their edges cancel. An edge used by an odd number of triangles
/// is open. Each open edge is then split at every end of an open edge that
/// lies within the tolerance of it and farther than that from both its ends,
/// as at a T-junction, and the pieces used an odd number of times are the
/// open edges left. The enclosed volume is about zero when it is under 1e-9
/// of the bounding box's, as for a sheet written with both windings.
/// Coordinates beyond `FAR` or not finite set `far` and make every edge open.
fn closure(triangles: &[[[f64; 3]; 3]], largest: f64) -> Closure {
    let mut largest = largest;
    for v in triangles.iter().flatten().flatten() {
        if !(v.abs() <= FAR) {
            largest = f64::INFINITY;
            break;
        }
        largest = largest.max(v.abs());
    }
    if !(largest <= FAR) {
        return Closure {
            open_edges: 3 * triangles.len(),
            encloses_volume: false,
            far: true,
        };
    }
    let tol = tolerance(largest);
    let (corners, points) = weld(triangles, tol);
    let mut faces: Vec<[u32; 3]> = triangles
        .iter()
        .zip(corners)
        .filter_map(|(t, ids)| {
            let n = cross(sub(t[1], t[0]), sub(t[2], t[0]));
            let longest = (0..3)
                .map(|k| dot(sub(t[(k + 1) % 3], t[k]), sub(t[(k + 1) % 3], t[k])))
                .fold(0., f64::max);
            // Zero area: corners welded together, or flat to f64 rounding.
            let zero = ids[0] == ids[1]
                || ids[1] == ids[2]
                || ids[0] == ids[2]
                || dot(n, n) <= 1e-24 * longest * longest;
            // Written from its smallest id, a triangle with the same winding
            // is always the same triple.
            let k = (0..3).min_by_key(|k| ids[*k]).unwrap_or(0);
            (!zero).then_some([ids[k], ids[(k + 1) % 3], ids[(k + 2) % 3]])
        })
        .collect();
    faces.sort_unstable();
    faces.dedup();
    let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    for p in triangles.iter().flatten() {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    // Measured from the box's centre, which keeps the rounding small.
    let centre: [f64; 3] = std::array::from_fn(|k| (lo[k] + hi[k]) / 2.);
    let volume: f64 = faces
        .iter()
        .map(|f| {
            let [a, b, c] = f.map(|i| sub(points[i as usize], centre));
            dot(a, cross(b, c))
        })
        .sum::<f64>()
        / 6.;
    let encloses_volume = volume.abs() > 1e-9 * (0..3).map(|k| hi[k] - lo[k]).product::<f64>();
    let open = odd(faces
        .iter()
        .flat_map(|f| [(f[0], f[1]), (f[1], f[2]), (f[2], f[0])])
        .map(|(a, b)| (a.min(b), a.max(b)))
        .collect());
    if open.is_empty() {
        return Closure {
            open_edges: 0,
            encloses_volume,
            far: false,
        };
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
        tree.near(pa, pb, tol, |v| {
            let p = points[v as usize];
            let t = (dot(sub(p, pa), d) / dot(d, d)).clamp(0., 1.);
            let off = sub(p, std::array::from_fn(|k| pa[k] + d[k] * t));
            let far = |q: [f64; 3]| dot(sub(p, q), sub(p, q)) > tol * tol;
            if dot(off, off) <= tol * tol && far(pa) && far(pb) {
                cuts.push((t, v));
            }
        });
        cuts.sort_by(|x, y| x.0.total_cmp(&y.0).then(x.1.cmp(&y.1)));
        let mut from = a;
        for &(_, v) in &cuts {
            pieces.push((from.min(v), from.max(v)));
            from = v;
        }
        pieces.push((from.min(b), from.max(b)));
    }
    Closure {
        open_edges: odd(pieces).len(),
        encloses_volume,
        far: false,
    }
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
/// (its first copy). Copies within `tol` of each other are grouped, and so
/// are chains of such copies; a group is one vertex when its points fit in a
/// box whose diagonal is at most 2 × `tol` (so no group wider than that is
/// welded), and otherwise its points stay distinct, as a row of distinct
/// points is not copies of one. Copies that differ only by rounding are
/// within `tol` of each other, so their box's diagonal is at most √3 × `tol`.
/// Equal copies are found first. The rest are grouped in cells `tol` / √3
/// across, so all copies in one cell are within `tol` of each other and need
/// no comparison, and each cell is compared only with the cells up to two
/// cells away.
fn weld(triangles: &[[[f64; 3]; 3]], tol: f64) -> (Vec<[u32; 3]>, Vec<[f64; 3]>) {
    use std::collections::HashMap;
    let mut seen: HashMap<[u64; 3], u32> = HashMap::new();
    let mut points: Vec<[f64; 3]> = vec![];
    let corners: Vec<[u32; 3]> = triangles
        .iter()
        .map(|t| {
            t.map(|p| {
                // Adding zero makes -0 and 0 one key.
                *seen
                    .entry(p.map(|v| (v + 0.).to_bits()))
                    .or_insert_with(|| {
                        points.push(p);
                        points.len() as u32 - 1
                    })
            })
        })
        .collect();
    let side = tol / 1.7321;
    let mut order: Vec<([i64; 3], u32)> = points
        .iter()
        .enumerate()
        .map(|(i, p)| (p.map(|v| (v / side).floor() as i64), i as u32))
        .collect();
    order.sort_unstable();
    // Each cell's key and its copies (a range of `order`).
    let mut cells: Vec<([i64; 3], std::ops::Range<usize>)> = vec![];
    for run in order.chunk_by(|a, b| a.0 == b.0) {
        let start = cells.last().map_or(0, |c| c.1.end);
        cells.push((run[0].0, start..start + run.len()));
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
        let d = sub(points[p as usize], points[q as usize]);
        dot(d, d) <= tol * tol
    };
    // Each pair of cells up to two apart once, the later one in key order:
    // for each X and Y step, the cells with Z up to two either way (only
    // above, when X and Y are the same). The keys are sorted, so each step's
    // cursor only moves forward.
    let steps: Vec<[i64; 2]> = (0..=2)
        .flat_map(|x| (-2..=2).map(move |y| [x, y]))
        .filter(|s| *s >= [0, 0])
        .collect();
    let mut cursors = vec![0; steps.len()];
    for c in 0..cells.len() {
        let key = cells[c].0;
        for (step, cursor) in steps.iter().zip(&mut cursors) {
            let below = if *step == [0, 0] { 1 } else { -2 };
            let lo = [key[0] + step[0], key[1] + step[1], key[2] + below];
            let hi = [lo[0], lo[1], key[2] + 2];
            while *cursor < cells.len() && cells[*cursor].0 < lo {
                *cursor += 1;
            }
            for d in (*cursor..cells.len()).take_while(|d| cells[*d].0 <= hi) {
                let (a, b) = (root(&mut parent, c as u32), root(&mut parent, d as u32));
                if a != b
                    && order[cells[c].1.clone()]
                        .iter()
                        .any(|(_, p)| order[cells[d].1.clone()].iter().any(|(_, q)| near(*p, *q)))
                {
                    parent[a.max(b) as usize] = a.min(b);
                }
            }
        }
    }
    let mut group_of = vec![0usize; points.len()];
    for (c, (_, range)) in cells.iter().enumerate() {
        let r = root(&mut parent, c as u32) as usize;
        for (_, i) in &order[range.clone()] {
            group_of[*i as usize] = r;
        }
    }
    // Each group's box.
    let (mut lo, mut hi) = (
        vec![[f64::INFINITY; 3]; cells.len()],
        vec![[f64::NEG_INFINITY; 3]; cells.len()],
    );
    for (p, &r) in points.iter().zip(&group_of) {
        for k in 0..3 {
            lo[r][k] = lo[r][k].min(p[k]);
            hi[r][k] = hi[r][k].max(p[k]);
        }
    }
    // Ids in order of first use, each placed at its first copy.
    let (mut id_of_root, mut ids, mut welded) = (vec![u32::MAX; cells.len()], vec![], vec![]);
    for (p, r) in points.iter().zip(group_of) {
        let diagonal = sub(hi[r], lo[r]);
        if dot(diagonal, diagonal) > 4. * tol * tol {
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
    /// Calls `visit` with every point within `reach` of the segment from `a`
    /// to `b`, and with some others near it.
    fn near(&self, a: [f64; 3], b: [f64; 3], reach: f64, mut visit: impl FnMut(u32)) {
        let mut stack = vec![0];
        while let Some(n) = stack.pop() {
            let (lo, hi, range, children) = &self.nodes[n];
            if !meets(a, b, lo.map(|v| v - reach), hi.map(|v| v + reach)) {
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
            })
            .or_else(|| holding_problem(doc, mesh)),
    };
    let material = material_report(doc).err();
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
/// The project's material and its cutting data, or why planning cannot use
/// the project's material. The user's text is cleaned and bounded.
fn material_report(doc: &Document) -> Result<MaterialReport, String> {
    let material = doc
        .selected_material()
        .ok_or("Choose a material on the setup page.")?;
    let data = material.cutting_data().ok_or_else(|| {
        format!(
            "Add cutting data to {} on the Materials page.",
            material.name()
        )
    })?;
    let text = |key: &str, limit| clip(get(&material.p, key), limit);
    Ok(MaterialReport {
        name: text("name", 60),
        group: text("group", 40),
        hardness: text("hardness", 40),
        notes: text("notes", 500),
        vc_min_m_per_min: data.vc.0,
        vc_max_m_per_min: data.vc.1,
        chip_load_mm: BANDS
            .iter()
            .zip(data.fz)
            .map(|((_, up_to), (min, max))| ChipLoadBand {
                up_to_mm: up_to.is_finite().then_some(*up_to),
                min,
                max,
            })
            .collect(),
    })
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
    if let Some(problem) = holding_problem(doc, Some(mesh)) {
        return Err(problem);
    }
    Ok((setup1, first, Volume::new(stock, Frame::new(setup1, mesh))))
}
/// The whole report, or the first reason planning cannot start. The part must
/// be a closed solid: no open edges, an enclosed volume, and vertical rays (in
/// setup 1's frame) that cross it consistently.
pub fn report(doc: &Document, mesh: &Mesh) -> Result<PartReport, String> {
    let (setup1, first, stock) = basics(doc, mesh)?;
    closed_solid(setup1, mesh)?;
    describe(doc, mesh, setup1, first, &stock)
}
/// `Ok` when the part is a closed solid, otherwise why it is not.
fn closed_solid(setup1: &Setup, mesh: &Mesh) -> Result<(), String> {
    let (triangles, largest) = mm_triangles(mesh);
    let found = closure(&triangles, largest);
    if found.far {
        return Err("The part lies more than 1 000 000 mm from its file origin. Move it closer to the origin and re-export it.".into());
    }
    if found.open_edges > 0 {
        return Err(format!(
            "The part is not a closed solid ({} open edges), so the planner cannot check its plans against it. Repair or re-export the model.",
            found.open_edges
        ));
    }
    if !found.encloses_volume {
        return Err("The part encloses no volume, so the planner cannot check its plans against it. Repair or re-export the model.".into());
    }
    if inconsistent_share(&Geometry::new(setup1, mesh)) > 0.02 {
        return Err("The part is not a closed solid, so the planner cannot check its plans against it. Repair or re-export the model.".into());
    }
    Ok(())
}
/// The report without the closed-surface checks, so tests can run the plan
/// checks on open meshes.
#[cfg(test)]
pub(crate) fn report_unchecked(doc: &Document, mesh: &Mesh) -> Result<PartReport, String> {
    let (setup1, first, stock) = basics(doc, mesh)?;
    describe(doc, mesh, setup1, first, &stock)
}
/// The first thing wrong with setup 1's modelled workholding, said for the
/// planner: the plan could never pass the export check of setup 1 with it.
fn holding_problem(doc: &Document, mesh: Option<&Mesh>) -> Option<String> {
    doc.holding_issues(0, mesh)
        .into_iter()
        .next()
        .map(|issue| format!("Setup 1's workholding: {issue}"))
}
/// `v` rounded up to the hundredth, after dropping what the coordinates' own
/// rounding (f32, about 1e-5 mm here) adds below the thousandth.
fn up2(v: f32) -> f32 {
    ((v * 1000.).round() / 10.).ceil() / 100.
}
/// Setup 1's workholding for the report, or None without it.
fn workholding_report(
    doc: &Document,
    mesh: &Mesh,
    setup1: &Setup,
) -> Result<Option<WorkholdingReport>, String> {
    let kind = match get(&setup1.p, "holding") {
        "Vise" => "Vise",
        "Clamps" => "Clamps",
        _ => return Ok(None),
    };
    let resolved = crate::stock::resolve(setup1, Some(mesh))?;
    let (stock, corner) = (resolved.stock, resolved.part.map_or([0.; 3], |p| p.min));
    let boxes = doc.holding_boxes(setup1, stock)?;
    let gap = crate::holding::gap(setup1)?;
    let top = boxes
        .iter()
        .map(|b| b.bounds.max[2])
        .fold(f32::NEG_INFINITY, f32::max);
    let needed = top + gap - stock.max[2];
    Ok(Some(WorkholdingReport {
        kind,
        boxes: crate::holding::shown(&boxes, stock)
            .iter()
            .map(|b| {
                let from_corner = |p: [f32; 3]| [0, 1, 2].map(|k| r2(p[k] - corner[k]));
                let ([x0, y0, z0], [x1, y1, z1]) =
                    (from_corner(b.bounds.min), from_corner(b.bounds.max));
                [x0, y0, z0, x1, y1, z1]
            })
            .collect(),
        gap: r2(gap),
        clearance_above_stock_top: (needed > 0.).then(|| up2(needed)),
    }))
}
fn describe(
    doc: &Document,
    mesh: &Mesh,
    setup1: &Setup,
    first: &'static str,
    stock: &Volume,
) -> Result<PartReport, String> {
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
    Ok(PartReport {
        units: "mm",
        part_size: mesh.dimensions.map(r2),
        material: material_report(doc)?,
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
                    flutes: num(&t.p, "flutes").filter(|f| *f >= 1. && f.fract() == 0.)? as u32,
                    flute_length: num(&t.p, "length")?,
                    stickout: num(&t.p, "stickout")?,
                })
            })
            .collect(),
        workholding: workholding_report(doc, mesh, setup1)?,
        orientations: order
            .into_iter()
            .filter_map(|up| orientation(setup1, stock, mesh, up, MAX_HOLES))
            .collect(),
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::post::fixture;
    fn block() -> (Document, Mesh) {
        let (mut doc, mesh) = fixture::job();
        crate::proposal::tests::aluminium(&mut doc);
        (doc, mesh)
    }
    fn plane<'a>(o: &'a OrientationReport, depth: f32) -> &'a PlaneReport {
        o.planes
            .iter()
            .find(|p| (p.depth_below_top - depth).abs() < 0.01)
            .unwrap_or_else(|| panic!("no plane at depth {depth} in {}", o.up))
    }
    /// The example block with the starter clamp or vise on setup 1.
    fn held(kind: &str, pairs: &[(&str, &str)]) -> (Document, Mesh) {
        let (mut doc, mesh) = block();
        crate::proposal::tests::hold(&mut doc, kind, pairs);
        (doc, mesh)
    }
    #[test]
    fn the_report_carries_setup_1s_clamps_gap_and_the_clearance_they_need() {
        let (doc, mesh) = block();
        assert_eq!(report(&doc, &mesh).unwrap().workholding, None);
        // Without workholding the report is what it was.
        assert!(
            !serde_json::to_string(&report(&doc, &mesh).unwrap())
                .unwrap()
                .contains("workholding")
        );
        // The fixture's stock: the model is 120 x 80 x 40, with 4 mm above,
        // 3 below and 2 on each side; the clamp reaches 48 mm out and 12 onto
        // the stock, 20 wide at the middle of the side, 25 above the stock top.
        let (mut doc, mesh) = held("Clamp", &[("clamps", "-X:0;+Y:10"), ("holding_gap", "3")]);
        let r = report(&doc, &mesh).unwrap();
        let w = r.workholding.as_ref().unwrap();
        assert_eq!(w.kind, "Clamps");
        assert_eq!(
            w.boxes,
            [
                [-50., 30., 44., 10., 50., 69.],
                [60., 70., 44., 80., 130., 69.]
            ]
        );
        assert_eq!(w.gap, 3.);
        // 69 + 3 - 44 above the stock top.
        assert_eq!(w.clearance_above_stock_top, Some(28.));
        let json = serde_json::to_string(&r).unwrap();
        assert!(
            json.contains(
                "\"workholding\":{\"kind\":\"Clamps\",\"boxes\":[[-50.0,30.0,44.0,10.0,50.0,69.0],"
            ),
            "{json}"
        );
        assert!(
            json.contains("\"gap\":3.0,\"clearance_above_stock_top\":28.0}"),
            "{json}"
        );
        // The report holds no name, note or id of the library entry.
        let id = get(&doc.setups[0].p, "holder").to_string();
        let holder = doc.workholding.iter_mut().find(|h| h.id == id).unwrap();
        holder.p.insert("name".into(), "ZZ-SECRET-HOLDER".into());
        holder.p.insert("notes".into(), "ZZ-SECRET-NOTE".into());
        let json = serde_json::to_string(&report(&doc, &mesh).unwrap()).unwrap();
        assert!(!json.contains("ZZ-SECRET") && !json.contains(&id), "{json}");
    }
    #[test]
    fn a_vise_is_reported_as_two_jaws_from_the_stock_bottom_and_needs_no_clearance() {
        // The jaws on the Y sides: 20 thick, 100 wide, up to 8 above the stock
        // bottom (3 below the model's); no clearance is needed above the stock.
        let (doc, mesh) = held("Vise", &[("vise_axis", "Y"), ("vise_grip", "8")]);
        let w = report(&doc, &mesh).unwrap().workholding.unwrap();
        assert_eq!(w.kind, "Vise");
        assert_eq!(
            w.boxes,
            [
                [10., -22., -3., 110., -2., 5.],
                [10., 82., -3., 110., 102., 5.]
            ]
        );
        assert_eq!((w.gap, w.clearance_above_stock_top), (2., None));
        let json = serde_json::to_string(&w).unwrap();
        assert!(!json.contains("clearance_above_stock_top"), "{json}");
    }
    #[test]
    fn workholding_that_cannot_be_modelled_stops_the_report_and_the_checklist() {
        // The vise opens to 120 mm: the stock is 124 mm across X.
        let (mut doc, mesh) = held("Vise", &[("vise_axis", "X"), ("vise_grip", "8")]);
        let problem = "Setup 1's workholding: The stock is 124 mm across the jaws; this vise opens to 120 mm.";
        assert_eq!(report(&doc, &mesh).unwrap_err(), problem);
        let unmet: Vec<_> = preconditions(&doc, Some(&mesh))
            .into_iter()
            .filter(|p| !p.ok)
            .map(|p| (p.label, p.hint))
            .collect();
        assert_eq!(
            unmet,
            [(
                "Setup 1: 3-axis LinuxCNC machine and stock",
                problem.to_string()
            )]
        );
        doc.setups[0].p.insert("vise_axis".into(), "Y".into());
        assert!(report(&doc, &mesh).is_ok());
        assert!(preconditions(&doc, Some(&mesh)).iter().all(|p| p.ok));
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
    /// A 40 × 20 × 10 mm block, its sides cut into 1 mm strips along X so
    /// that its top face is 80 triangles, under a separate 40 × 20 × 5 mm
    /// slab 10 mm above it whose −X end overhangs the block's top from X 0
    /// to `cover`.
    fn overhung(cover: f32) -> Mesh {
        let (y, z) = (20f32, 10f32);
        let mut t = vec![];
        for i in 0..40 {
            let (a, b) = (i as f32, i as f32 + 1.);
            quad(&mut t, [[a, 0., z], [b, 0., z], [b, y, z], [a, y, z]]);
            quad(&mut t, [[a, 0., 0.], [a, y, 0.], [b, y, 0.], [b, 0., 0.]]);
            quad(&mut t, [[a, 0., 0.], [b, 0., 0.], [b, 0., z], [a, 0., z]]);
            quad(&mut t, [[b, y, 0.], [a, y, 0.], [a, y, z], [b, y, z]]);
        }
        quad(&mut t, [[0., y, 0.], [0., 0., 0.], [0., 0., z], [0., y, z]]);
        quad(
            &mut t,
            [[40., 0., 0.], [40., y, 0.], [40., y, z], [40., 0., z]],
        );
        t.extend(cuboid([cover - 40., 0., 20.], [cover, y, 25.]));
        crate::mesh::normalize(t).unwrap()
    }
    #[test]
    fn a_face_is_listed_only_when_at_least_half_of_it_is_visible_from_above() {
        let (doc, _) = block();
        // The block's top is 15 mm below the slab's top. With 6 of its 40 mm
        // under the slab (85% visible) and with 16 (60%), the whole face is
        // listed; with 24 (40%) and 34 (15%) it is not.
        for (cover, listed) in [(6., true), (16., true), (24., false), (34., false)] {
            let report = report(&doc, &overhung(cover)).unwrap();
            let top = &report.orientations[0];
            assert_eq!(top.up, "+Z");
            assert_eq!(plane(top, 0.).kind, "top");
            let step = top
                .planes
                .iter()
                .find(|p| (p.depth_below_top - 15.).abs() < 0.01);
            assert_eq!(step.is_some(), listed, "cover {cover}: {:?}", top.planes);
            if let Some(step) = step {
                assert!((step.area - 800.).abs() < 0.5, "{}", step.area);
            }
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
        let (mut doc, mesh) = fixture::job();
        let unmet = |doc: &Document, mesh: Option<&Mesh>| -> Vec<&'static str> {
            preconditions(doc, mesh)
                .into_iter()
                .filter(|p| !p.ok)
                .map(|p| p.label)
                .collect()
        };
        assert_eq!(unmet(&doc, Some(&mesh)), vec!["Material"]);
        assert_eq!(
            preconditions(&doc, Some(&mesh))[2].hint,
            "Choose a material on the setup page."
        );
        // A library with no material chosen is the same.
        doc.adopt_materials();
        assert_eq!(
            preconditions(&doc, Some(&mesh))[2].hint,
            "Choose a material on the setup page."
        );
        // A material without cutting data cannot be planned with.
        let bare = crate::material::Material::new("Inconel 718");
        let id = bare.id.clone();
        doc.materials.push(bare);
        assert!(doc.select_material(&id));
        assert_eq!(unmet(&doc, Some(&mesh)), vec!["Material"]);
        assert_eq!(
            preconditions(&doc, Some(&mesh))[2].hint,
            "Add cutting data to Inconel 718 on the Materials page."
        );
        // A name alone, with no library material behind it, is not enough.
        doc.material_id.clear();
        doc.material = "Aluminium 6061-T6".into();
        assert_eq!(unmet(&doc, Some(&mesh)), vec!["Material"]);
        crate::proposal::tests::aluminium(&mut doc);
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
            json.contains("\"material\":{\"name\":\"Aluminium 6061-T6\",\"group\":\"Aluminium\"")
                && json.contains("\"max_cutter_diameter\"")
                && json.contains("\"flutes\":3")
                && json.contains("{\"up_to_mm\":null,\"min\":0.06,\"max\":0.12}")
        );
        assert!(json.len() < 20_000, "{}", json.len());
    }
    #[test]
    fn the_report_carries_the_materials_cutting_data() {
        let (mut doc, mesh) = block();
        let report = report(&doc, &mesh).unwrap();
        let m = &report.material;
        assert_eq!(
            (m.name.as_str(), m.group.as_str(), m.hardness.as_str()),
            ("Aluminium 6061-T6", "Aluminium", "95 HB")
        );
        assert!(m.notes.starts_with("Starting point for carbide tools"));
        assert!(m.notes.ends_with("Flood or mist; clear chips."));
        assert_eq!((m.vc_min_m_per_min, m.vc_max_m_per_min), (150., 300.));
        assert_eq!(m.chip_load_mm.len(), 5);
        let limits: Vec<_> = m.chip_load_mm.iter().map(|b| b.up_to_mm).collect();
        assert_eq!(limits, vec![Some(3.), Some(6.), Some(10.), Some(16.), None]);
        let band = &m.chip_load_mm[2];
        assert_eq!((band.min, band.max), (0.03, 0.08));
        let last = &m.chip_load_mm[4];
        assert_eq!((last.min, last.max), (0.06, 0.12));
        // Every tool says how many flutes it has: the chip load needs them.
        assert_eq!(
            report.tools.iter().map(|t| t.flutes).collect::<Vec<_>>(),
            vec![3, 3, 3]
        );
        // The user's text is one cleaned line of bounded length.
        let id = doc.material_id.clone();
        let chosen = doc.materials.iter_mut().find(|m| m.id == id).unwrap();
        let long = format!("Line one\n\ttwo {}", "x".repeat(600));
        chosen.p.insert("notes".into(), long.clone());
        chosen.p.insert("name".into(), "N".repeat(80));
        chosen.p.insert("group".into(), "G".repeat(80));
        chosen
            .p
            .insert("hardness".into(), format!("{}\u{202e}", "H".repeat(80)));
        let m = super::report(&doc, &mesh).unwrap().material;
        assert_eq!(m.notes, crate::proposal::clip(&long, 500));
        assert!(m.notes.starts_with("Line one two xxx"));
        assert_eq!(m.notes.chars().count(), 500);
        assert_eq!(m.name, "N".repeat(60));
        assert_eq!(m.group, "G".repeat(40));
        assert_eq!(m.hardness, "H".repeat(40));
    }
    #[test]
    fn a_report_needs_a_material_with_cutting_data() {
        let (mut doc, mesh) = block();
        doc.material_id.clear();
        assert_eq!(
            report(&doc, &mesh).unwrap_err(),
            "Choose a material on the setup page."
        );
        let bare = crate::material::Material::new("Inconel 718");
        let id = bare.id.clone();
        doc.materials.push(bare);
        assert!(doc.select_material(&id));
        assert_eq!(
            report(&doc, &mesh).unwrap_err(),
            "Add cutting data to Inconel 718 on the Materials page."
        );
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
        crate::mesh::normalize(holed_triangles([top; 2], [bottom; 2])).unwrap()
    }
    /// The triangles of a 120 × 80 × 40 box from the origin with a centred
    /// rectangle (X and Y sizes) missing from its top face and another from
    /// its bottom face, with no walls between them.
    fn holed_triangles(top: [f32; 2], bottom: [f32; 2]) -> Vec<Tri> {
        let (x, y, z) = (120f32, 80f32, 40f32);
        let mut t = vec![];
        for (h, zz, up) in [(top, z, true), (bottom, 0., false)] {
            let (a0, a1) = ((x - h[0]) / 2., (x + h[0]) / 2.);
            let (b0, b1) = ((y - h[1]) / 2., (y + h[1]) / 2.);
            let quads = if h[0] > 0. {
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
        t
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
    /// Open edges of `t`, taken as mm.
    fn count(t: &[Tri]) -> usize {
        let t: Vec<_> = t.iter().map(|p| p.map(|q| q.map(f64::from))).collect();
        closure(&t, 0.).open_edges
    }
    /// Open edges of `mesh`.
    fn edges(mesh: &Mesh) -> usize {
        let (t, largest) = mm_triangles(mesh);
        closure(&t, largest).open_edges
    }
    fn not_closed(n: usize) -> String {
        format!(
            "The part is not a closed solid ({n} open edges), so the planner cannot check its plans against it. Repair or re-export the model."
        )
    }
    const NOT_SOLID: &str = "The part is not a closed solid, so the planner cannot check its plans against it. Repair or re-export the model.";
    const NO_VOLUME: &str = "The part encloses no volume, so the planner cannot check its plans against it. Repair or re-export the model.";
    /// `t` turned `angles` degrees about X and then Y, scaled by `scale` and
    /// moved by `offset`, computed exactly and rounded once to f32, as an
    /// exporter writes it.
    fn placed(t: &[Tri], angles: [f64; 2], scale: f64, offset: [f64; 3]) -> Vec<Tri> {
        let [(sx, cx), (sy, cy)] = angles.map(|a| a.to_radians().sin_cos());
        let m = [
            [cy, sy * sx, sy * cx],
            [0., cx, -sx],
            [-sy, cy * sx, cy * cx],
        ];
        t.iter()
            .map(|p| {
                p.map(|q| {
                    std::array::from_fn(|i| {
                        ((0..3).map(|k| m[i][k] * f64::from(q[k])).sum::<f64>() * scale + offset[i])
                            as f32
                    })
                })
            })
            .collect()
    }
    /// A point on a circle of radius `r` around the Z axis, at the `i`-th of
    /// `n` steps and height `z`.
    fn rim(r: f32, n: usize, i: usize, z: f32) -> [f32; 3] {
        let a = std::f64::consts::TAU * (i % n) as f64 / n as f64;
        [
            (f64::from(r) * a.cos()) as f32,
            (f64::from(r) * a.sin()) as f32,
            z,
        ]
    }
    /// A cylinder of radius `r` and height `h` with `n` segments whose caps
    /// are fans from rim vertex 0, as CAD often writes a disc.
    fn rim_fan_cylinder(r: f32, n: usize, h: f32) -> Vec<Tri> {
        let mut t = vec![];
        for i in 0..n {
            let (a0, a1, b0, b1) = (
                rim(r, n, i, 0.),
                rim(r, n, i + 1, 0.),
                rim(r, n, i, h),
                rim(r, n, i + 1, h),
            );
            t.push([a0, a1, b1]);
            t.push([a0, b1, b0]);
        }
        for k in 1..n - 1 {
            t.push([rim(r, n, 0, h), rim(r, n, k, h), rim(r, n, k + 1, h)]);
            t.push([rim(r, n, 0, 0.), rim(r, n, k + 1, 0.), rim(r, n, k, 0.)]);
        }
        t
    }
    /// A 5 mm tall cylinder of radius `r` with `n` segments whose caps are
    /// fans around their centres, with every other top spoke split into
    /// `pieces` by a fan from the rim: T-junctions along the spokes.
    fn split_spokes(r: f32, n: usize, pieces: usize) -> Vec<Tri> {
        let centre = [0f32, 0., 5.];
        let mut t = vec![];
        for i in 0..n {
            let (a0, a1, b0, b1) = (
                rim(r, n, i, 0.),
                rim(r, n, i + 1, 0.),
                rim(r, n, i, 5.),
                rim(r, n, i + 1, 5.),
            );
            t.push([a0, a1, b1]);
            t.push([a0, b1, b0]);
            t.push([[0., 0., 0.], a1, a0]);
            if i % 2 == 1 {
                t.push([centre, b0, b1]);
                continue;
            }
            let at = |s: usize| -> [f32; 3] {
                let f = s as f32 / pieces as f32;
                std::array::from_fn(|k| centre[k] + (b0[k] - centre[k]) * f)
            };
            for s in 0..pieces {
                t.push([at(s), at(s + 1), b1]);
            }
        }
        t
    }
    /// An outward-facing box from `lo` to `hi`.
    fn cuboid(lo: [f32; 3], hi: [f32; 3]) -> Vec<Tri> {
        box_triangles(true, true)
            .iter()
            .map(|p| p.map(|q| std::array::from_fn(|k| if q[k] > 0. { hi[k] } else { lo[k] })))
            .collect()
    }
    /// `t` with every triangle's winding reversed.
    fn reversed(t: &[Tri]) -> Vec<Tri> {
        t.iter().map(|p| [p[0], p[2], p[1]]).collect()
    }
    type Tri64 = [[f64; 3]; 3];
    /// The example block in mm with its lowest corner at the origin, scaled
    /// so that its longest side is `length` mm, as an exporter's doubles.
    fn cornered(mesh: &Mesh, length: f64) -> Vec<Tri64> {
        let t = source(mesh, [0.; 3]);
        let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
        for q in t.iter().flatten() {
            for k in 0..3 {
                lo[k] = lo[k].min(f64::from(q[k]));
                hi[k] = hi[k].max(f64::from(q[k]));
            }
        }
        let scale = length / (0..3).map(|k| hi[k] - lo[k]).fold(0., f64::max);
        t.iter()
            .map(|p| p.map(|q| std::array::from_fn(|k| (f64::from(q[k]) - lo[k]) * scale)))
            .collect()
    }
    /// `split`, computed in f64 as an exporter computes it.
    fn split64(t: &[Tri64], every: usize, pieces: usize) -> Vec<Tri64> {
        let mut out = vec![];
        for (i, p) in t.iter().enumerate() {
            if i % every != 0 {
                out.push(*p);
                continue;
            }
            let length = |k: usize| dot(sub(p[(k + 1) % 3], p[k]), sub(p[(k + 1) % 3], p[k]));
            let e = (0..3)
                .max_by(|a, b| length(*a).total_cmp(&length(*b)))
                .unwrap();
            let (a, b, c) = (p[e], p[(e + 1) % 3], p[(e + 2) % 3]);
            let at = |s: usize| -> [f64; 3] {
                let s = s as f64 / pieces as f64;
                std::array::from_fn(|k| a[k] + (b[k] - a[k]) * s)
            };
            for s in 0..pieces {
                out.push([at(s), at(s + 1), c]);
            }
        }
        out
    }
    /// `t` written as ASCII STL with `digits` significant digits (C's `%.6e`
    /// for 7) and read back by the app's STL reader.
    fn ascii_stl(t: &[Tri64], digits: usize) -> Vec<Tri> {
        let number = |v: f64| {
            let text = format!("{v:.*e}", digits - 1);
            let (mantissa, exponent) = text.split_once('e').unwrap();
            format!("{mantissa}e{:+03}", exponent.parse::<i32>().unwrap())
        };
        let mut s = String::from("solid part\n");
        for p in t {
            s.push_str("  facet normal 0 0 0\n    outer loop\n");
            for q in p {
                let [x, y, z] = q.map(number);
                s.push_str(&format!("      vertex {x} {y} {z}\n"));
            }
            s.push_str("    endloop\n  endfacet\n");
        }
        s.push_str("endsolid part\n");
        crate::mesh::stl(s.as_bytes()).unwrap()
    }
    /// Zero-area triangles whose corners are `step` mm apart along the line
    /// from `p` to `q` (computed in f64 and rounded once to f32): each repeats
    /// a corner, or, when `collinear`, has three corners on the line.
    fn chain(p: [f64; 3], q: [f64; 3], step: f64, collinear: bool) -> Vec<Tri> {
        let n = (dot(sub(q, p), sub(q, p)).sqrt() / step).ceil() as usize;
        let at = |i: usize| -> [f32; 3] {
            std::array::from_fn(|k| (p[k] + (q[k] - p[k]) * i.min(n) as f64 / n as f64) as f32)
        };
        if collinear {
            (0..n)
                .step_by(2)
                .map(|i| [at(i), at(i + 1), at(i + 2)])
                .collect()
        } else {
            (0..n).map(|i| [at(i), at(i), at(i + 1)]).collect()
        }
    }
    #[test]
    fn open_edges_are_the_edges_left_once_float_rounding_is_absorbed() {
        let (o, a, b, c) = ([0., 0., 0.], [10., 0., 0.], [0., 10., 0.], [0., 0., 10.]);
        let tetrahedron = |moved: [f32; 3]| vec![[moved, b, a], [o, a, c], [o, c, b], [a, b, c]];
        assert_eq!(count(&tetrahedron(o)), 0);
        assert_eq!(count(&[[o, a, b]]), 3);
        // The tolerance is 8 f32 steps at the largest coordinate, or √3 times
        // the last digit of 7 significant digits there if that is more.
        let root3 = 3f64.sqrt();
        for (largest, tol) in [
            (0.5, root3 * 1e-6),
            (10., root3 * 1e-5),
            (99.9, 8. * 2f64.powi(-17)),
            (120., root3 * 1e-4),
            (1012.3, root3 * 1e-3),
            (3000., 8. * 2f64.powi(-12)),
        ] {
            assert!((tolerance(largest) - tol).abs() < tol * 1e-9, "{largest}");
        }
        // Here it is √3 × 0.00001 mm: copies of a vertex that close are one
        // vertex.
        for off in [0.000001, 0.000005, 0.00001, 0.000015] {
            assert_eq!(count(&tetrahedron([0., 0., -off])), 0, "{off}");
        }
        // Copies farther apart are distinct vertices: four open edges.
        for off in [0.00002, 0.00005, 0.0005, 0.00099, 0.01] {
            assert_eq!(count(&tetrahedron([0., 0., -off])), 4, "{off}");
        }
        // Copies 0.00001 mm apart in a row (joined by zero-area triangles) are
        // welded only while the row spans at most twice the tolerance.
        let row = |length: usize| {
            let mut t = tetrahedron([0., 0., -0.00001 * length as f32]);
            let z = |k: usize| [0., 0., -0.00001 * k as f32];
            t.extend((0..length).map(|k| [z(k), z(k), z(k + 1)]));
            t
        };
        assert_eq!(count(&row(3)), 0);
        assert_eq!(count(&row(4)), 4);
        // A zero-area triangle and a triangle written again with the same
        // winding, from another corner, change nothing.
        let mut t = tetrahedron(o);
        t.extend([[o, [5., 0., 0.], a], [o, a, c], [a, c, o]]);
        assert_eq!(count(&t), 0);
        // A copy with the opposite winding stays: with it, the triangle's
        // edges are used three times.
        let mut t = tetrahedron(o);
        t.push([o, c, a]);
        assert_eq!(count(&t), 3);
        // A triangle written with both windings has no open edge (the volume
        // rule refuses it).
        assert_eq!(count(&[[o, a, b], [o, b, a]]), 0);
        // A T-junction: the long edge is split at the vertex in its middle,
        // also when that vertex is within the tolerance of the edge.
        let flat = |top: Vec<Tri>| {
            let mut t = top;
            t.extend([[o, b, c], [o, c, a], [a, c, b]]);
            t
        };
        for m in [[5., 5., 0.], [5.000003, 5.000003, 0.]] {
            assert_eq!(count(&flat(vec![[o, a, m], [o, m, b]])), 0, "{m:?}");
        }
        // Moved 0.00003 or 0.01 mm off the long edge, the vertex does not
        // split it.
        for m in [[5.00002, 5.00002, 0.], [5.007, 5.007, 0.]] {
            assert_eq!(count(&flat(vec![[o, a, m], [o, m, b]])), 3, "{m:?}");
        }
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
        // A UV sphere whose vertex copies are up to 0.0002 mm apart, far more
        // than the tolerance at 30 mm (0.000017 mm): the copies are distinct
        // vertices.
        let soup =
            crate::mesh::normalize(jittered(&sphere_triangles(96, 48, 30.), 0.0001)).unwrap();
        let n = edges(&soup);
        assert!(n > 10_000, "{n}");
        assert_eq!(report(&doc, &soup).unwrap_err(), not_closed(n));
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
                "UV sphere with vertex copies up to 0.00001 mm apart",
                crate::mesh::normalize(jittered(&sphere_triangles(96, 48, 30.), 0.000005)).unwrap(),
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
        let (doc, _) = block();
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
            assert_eq!(closed_solid(&doc.setups[0], &mesh), Ok(()), "{file}");
        }
    }
    /// The solid spans through the middle of the model in `setup`'s frame,
    /// from the model's bottom, in mm.
    fn column(setup: &Setup, mesh: &Mesh) -> Vec<[f32; 2]> {
        let geometry = Geometry::new(setup, mesh);
        let index = Index::new(&geometry);
        let b = geometry.bounds;
        let centre = [0, 1].map(|k| (b.min[k] + b.max[k]) / 2. + 1e-4);
        solid(&index, centre, b.max[2])
            .iter()
            .map(|(lo, hi)| [lo - b.min[2], hi - b.min[2]].map(|v| (v * 1000.).round() / 1000.))
            .collect()
    }
    #[test]
    fn an_inconsistent_column_is_solid_where_its_crossings_say_so() {
        let (doc, _) = block();
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
    fn sound_small_and_far_parts_with_t_junctions_are_closed() {
        let (doc, mesh) = block();
        let src = source(&mesh, [0.; 3]);
        let lower = cuboid([0.; 3], [40., 30., 20.]);
        // The same box on top shares the lower one's top triangles, in the
        // opposite winding; the mirrored box beside it shares its +X wall.
        let mut stacked = lower.clone();
        stacked.extend(cuboid([0., 0., 20.], [40., 30., 40.]));
        let mut beside = lower.clone();
        beside.extend(reversed(
            &lower
                .iter()
                .map(|p| p.map(|q| [80. - q[0], q[1], q[2]]))
                .collect::<Vec<_>>(),
        ));
        let mut cases: Vec<(String, Vec<Tri>)> = vec![
            (
                "4 mm, 256-segment cylinder with rim-fanned caps, every other triangle split".into(),
                split(&rim_fan_cylinder(2., 256, 3.), 2, 2),
            ),
            (
                "1 mm, 128-segment cylinder with rim-fanned caps, every other triangle split".into(),
                split(&rim_fan_cylinder(0.5, 128, 3.), 2, 2),
            ),
            (
                "fan-capped cylinder, 1 024 segments, every other top spoke split into 100".into(),
                split_spokes(5., 1024, 100),
            ),
            (
                "example block turned 30/30 degrees at (412.3, -287.6, 95.1) mm, every 10th longest edge split into 120".into(),
                placed(&split(&src, 10, 120), [30., 30.], 1., [412.3, -287.6, 95.1]),
            ),
            ("two boxes stacked, sharing a face".into(), stacked),
            ("two boxes side by side, sharing a face".into(), beside),
        ];
        for length in [10., 15., 6., 4.7] {
            cases.push((
                format!("example block {length} mm long with T-junctions"),
                split(&placed(&src, [0.; 2], length / 120., [0.; 3]), 3, 2),
            ));
        }
        // Small parts with T-junctions, rounded where their files put them,
        // away from the origin: the tolerance follows the file's coordinates.
        cases.push((
            "1 mm cylinder with T-junctions, turned 30/30 degrees at (40, -20, 10) mm".into(),
            placed(
                &split(&rim_fan_cylinder(0.5, 128, 3.), 3, 2),
                [30., 30.],
                1.,
                [40., -20., 10.],
            ),
        ));
        cases.push((
            "example block 10 mm long with T-junctions at (412.3, -287.6, 95.1) mm".into(),
            placed(
                &split(&placed(&src, [0.; 2], 10. / 120., [0.; 3]), 3, 2),
                [0.; 2],
                1.,
                [412.3, -287.6, 95.1],
            ),
        ));
        for (name, t) in cases {
            let m = crate::mesh::normalize(t).unwrap();
            assert_eq!(edges(&m), 0, "{name}");
            assert_eq!(closed_solid(&doc.setups[0], &m), Ok(()), "{name}");
        }
    }
    #[test]
    fn a_part_that_encloses_no_volume_is_refused() {
        let (doc, _) = block();
        let one = vec![[[0., 0., 0.], [10., 0., 5.], [0., 10., 5.]]];
        let single = crate::mesh::normalize([one.clone(), reversed(&one)].concat()).unwrap();
        assert_eq!(edges(&single), 0);
        assert_eq!(report(&doc, &single).unwrap_err(), NO_VOLUME);
        // The upper half of a UV sphere and a flat 10 x 10 grid, each written
        // with both windings.
        let cap: Vec<Tri> = sphere_triangles(48, 24, 30.)
            .into_iter()
            .filter(|p| p.iter().all(|q| q[2] > -0.001))
            .collect();
        let mut grid = vec![];
        for i in 0..10 {
            for j in 0..10 {
                let (x, y) = (i as f32, j as f32);
                quad(
                    &mut grid,
                    [
                        [x, y, 0.],
                        [x + 1., y, 0.],
                        [x + 1., y + 1., 0.],
                        [x, y + 1., 0.],
                    ],
                );
            }
        }
        for (name, sheet) in [("hemisphere", cap), ("flat grid", grid)] {
            let both = crate::mesh::normalize([sheet.clone(), reversed(&sheet)].concat()).unwrap();
            assert_eq!(edges(&both), 0, "{name}");
            assert_eq!(
                closed_solid(&doc.setups[0], &both).unwrap_err(),
                NO_VOLUME,
                "{name}"
            );
        }
    }
    #[test]
    fn coincident_faces_facing_both_ways_read_the_same_in_either_order() {
        let (doc, _) = block();
        let setup1 = &doc.setups[0];
        // The reviewer's case: a 20 x 20 x 20 void (an inward box) whose top
        // lies on the block's top face, listed before or after the block.
        let outer = cuboid([0.; 3], [120., 80., 40.]);
        let void = reversed(&cuboid([50., 30., 20.], [70., 50., 40.]));
        let first = crate::mesh::normalize([void.clone(), outer.clone()].concat()).unwrap();
        let last = crate::mesh::normalize([outer, void].concat()).unwrap();
        let share = |m: &Mesh| inconsistent_share(&Geometry::new(setup1, m));
        assert_eq!(share(&first), share(&last));
        assert_eq!(column(setup1, &first), column(setup1, &last));
        assert_eq!(closed_solid(setup1, &first), closed_solid(setup1, &last));
        // A crossing facing down comes first: the void reads as empty under a
        // lid of no thickness, and the part is accepted.
        assert_eq!(column(setup1, &first), vec![[0., 20.], [40., 40.]]);
        assert_eq!(closed_solid(setup1, &first), Ok(()));
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
        // Two closed 0.1 mm cubes at opposite corners of a 100 mm square:
        // no test ray hits either, and that counts as inconsistent.
        let apart = crate::mesh::normalize(
            [
                cuboid([0.; 3], [0.1; 3]),
                cuboid([99.9, 99.9, 0.], [100., 100., 0.1]),
            ]
            .concat(),
        )
        .unwrap();
        assert_eq!(edges(&apart), 0);
        assert_eq!(
            inconsistent_share(&Geometry::new(&doc.setups[0], &apart)),
            1.
        );
        assert_eq!(report(&doc, &apart).unwrap_err(), NOT_SOLID);
    }
    #[test]
    fn sound_parts_written_as_text_with_7_significant_digits_are_closed() {
        let (doc, mesh) = block();
        // T-junctions computed in f64 and written as ASCII STL with %.6e, so
        // every T-vertex and edge end is rounded to 7 significant digits.
        for (name, t) in [
            (
                "example block at 0-120 mm, every 10th longest edge split into 30",
                split64(&cornered(&mesh, 120.), 10, 30),
            ),
            (
                "example block 101.6 mm (4 in) long with T-junctions",
                split64(&cornered(&mesh, 101.6), 3, 2),
            ),
            (
                "example block 101.6 mm (4 in) long, every 10th longest edge split into 30",
                split64(&cornered(&mesh, 101.6), 10, 30),
            ),
        ] {
            let m = crate::mesh::normalize(ascii_stl(&t, 7)).unwrap();
            assert_eq!(edges(&m), 0, "{name}");
            assert_eq!(closed_solid(&doc.setups[0], &m), Ok(()), "{name}");
        }
    }
    #[test]
    fn rows_of_nearby_points_do_not_weld_an_opening_shut() {
        let (doc, _) = block();
        // The reviewer's 12 x 12 openings in the top and the bottom, with no
        // walls, placed at (412.3, -287.6, 95.1) mm, where the tolerance is
        // 0.00049 mm. Each opening's corners are joined across it in pairs by
        // rows of zero-area triangles whose points are 0.0003 mm apart.
        let at = [412.3, -287.6, 95.1];
        let mut t = placed(&holed_triangles([12.; 2], [12.; 2]), [0.; 2], 1., at);
        for z in [40., 0.] {
            for x in [54., 66.] {
                let p = [x + at[0], 34. + at[1], z + at[2]];
                let q = [x + at[0], 46. + at[1], z + at[2]];
                t.extend(chain(p, q, 0.0003, false));
            }
        }
        let openings = crate::mesh::normalize(t).unwrap();
        assert_eq!(edges(&openings), 8);
        assert_eq!(report(&doc, &openings).unwrap_err(), not_closed(8));
        // The reviewer's 40 x 3 mm slot through the top and the bottom at the
        // origin, its ends bridged by collinear zero-area triangles whose
        // points are 0.000055 mm apart.
        let mut t = holed_triangles([40., 3.], [40., 3.]);
        for z in [40., 0.] {
            for x in [40., 80.] {
                t.extend(chain([x, 38.5, z], [x, 41.5, z], 0.000055, true));
            }
        }
        let slot = crate::mesh::normalize(t).unwrap();
        assert_eq!(edges(&slot), 8);
        assert_eq!(report(&doc, &slot).unwrap_err(), not_closed(8));
    }
    #[test]
    fn a_double_sided_sheet_with_opposite_diagonals_is_refused() {
        let (doc, _) = block();
        // The reviewer's OBJ: 100 x 100 quads of 1 mm on z = 0.00005 (x - 50)
        // (y - 50), each written as `f p q r s` and again as `f s r q p`. The
        // OBJ reader fans a face from its first vertex, so the two sides use
        // opposite diagonals and enclose about 0.08 mm³ between them.
        let z = |x: f32, y: f32| 5e-5 * (x - 50.) * (y - 50.);
        let mut t = vec![];
        for i in 0..100 {
            for j in 0..100 {
                let (x, y) = (i as f32, j as f32);
                let [p, q, r, s] = [[x, y], [x + 1., y], [x + 1., y + 1.], [x, y + 1.]]
                    .map(|[x, y]| [x, y, z(x, y)]);
                t.extend([[p, q, r], [p, r, s], [s, r, q], [s, q, p]]);
            }
        }
        let sheet = crate::mesh::normalize(t).unwrap();
        let (triangles, largest) = mm_triangles(&sheet);
        let found = closure(&triangles, largest);
        assert_eq!(found.open_edges, 0);
        assert!(found.encloses_volume);
        // Every ray meets one crossing facing down and one facing up at one
        // height, with nothing solid on either side: inconsistent, and so
        // solid from the bottom of the model to its top.
        let setup1 = &doc.setups[0];
        assert_eq!(inconsistent_share(&Geometry::new(setup1, &sheet)), 1.);
        assert_eq!(column(setup1, &sheet), vec![[0., 0.25]]);
        assert_eq!(report(&doc, &sheet).unwrap_err(), NOT_SOLID);
    }
    #[test]
    fn a_part_far_from_its_file_origin_has_its_own_message() {
        let (doc, _) = block();
        let far = crate::mesh::normalize(cuboid([1.1e6, 0., 0.], [1.1e6 + 40., 30., 20.])).unwrap();
        assert_eq!(
            report(&doc, &far).unwrap_err(),
            "The part lies more than 1 000 000 mm from its file origin. Move it closer to the origin and re-export it."
        );
        // Just inside the limit it is checked as usual.
        let near =
            crate::mesh::normalize(cuboid([9.9e5, 0., 0.], [9.9e5 + 40., 30., 20.])).unwrap();
        assert_eq!(edges(&near), 0);
        assert!(report(&doc, &near).is_ok());
    }
}
