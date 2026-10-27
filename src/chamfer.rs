//! Chamfer milling (LIO-245): a chamfer mill run level with the side of its
//! cone on a line, or on the edges of picked faces, and copied round a
//! vertical axis. It cuts V flanks at a depth (the clutch teeth of the power
//! feed, `MACHINING.md` → Teeth), edge chamfers and deburrs.
//!
//! # Where the tool goes
//!
//! A chamfer mill of included angle 2α with a flat tip `f` across is, at a
//! height `h` above its tip, a disc of radius
//!
//! ```text
//! r(h) = f/2 + h·tan α.
//! ```
//!
//! In every vertical plane through its axis its side is a straight line
//! leaning α from the axis. Moved level and straight along a direction `u`,
//! the cone sweeps, on each side, the plane that holds `u` and leans α from
//! the vertical: it touches that plane along its generator in the vertical
//! plane through its axis square to `u`, so the plane is cut exactly, with no
//! scallops. With its axis `o` across from a level line L (horizontally,
//! square to `u`), the swept plane passes through L at the height `z_L`
//! where the cone is `o` in radius, so its side is on L when
//!
//! ```text
//! o = r(z_L − z_tip) = f/2 + (z_L − z_tip)·tan α                    (1)
//! ```
//!
//! towards the side of L the tool is on: the air's, the material being on
//! the other. That is the Tip depth size: the tip's Z is given, (1) gives
//! the offset. The flat adds f/2 whatever the depth.
//!
//! The Chamfer width size breaks an edge at `z_e`, between a level face and
//! the wall under it, `w` wide on the face. Across the edge (x from the edge
//! towards the air) the chamfer runs from (−w, z_e) down the wall to
//! (0, z_e − w·cot α). The tip goes the tip offset `t` under its lower end,
//! so that the cone's side, not its tip, cuts all of it, and (1) through
//! (−w, z_e) gives
//!
//! ```text
//! z_tip = z_e − w·cot α − t,    o = r(z_e − z_tip) − w = f/2 + t·tan α   (2)
//! ```
//!
//! from the edge, towards the air. Run round a hole mouth or an outer arc
//! the cone's envelope at each height is the circle the path is, grown or
//! shrunk by r(h): the same cone, so (2) holds there too.
//!
//! # The clutch teeth
//!
//! The rotor's and the armature's 60 flanks are planes 30° from the axis,
//! each through a radial line at the pitch plane, 0.35 under the turned tip
//! surface (Z 0): on the rotor at θ = 3° + 6°·j (rotor.step). A 60° chamfer
//! mill (α = 30°) with its tip at the root floor, Z −1.30, puts its side on
//! the line at Z −0.35 by (1) at o = f/2 + 0.95·tan 30° = 0.5485 + f/2 mm:
//! 0.5485 for a pointed one, 0.6485 for one with a 0.2 flat. Each pass runs
//! out along its line from r 31.8, in the run-out groove (r 31…32, turned to
//! the root floor) and clear of the pole face inside r 31 (at the rotor's
//! pole face, 0.30 under the tips, the cone is 0.58 + f/2 in radius), to past
//! the OD. Seen from above, the gap is anticlockwise of the lines of even j
//! and clockwise of the odd ones: looking out along its line the tool is on
//! the Left of the first and on the Right of the second, so the program is
//! Alternate sides from the Left, 60 copies 6° apart from 3°. Only the cone's
//! side touches the flank; its far side leans the way the gap's other flank
//! does and stays 2.25 − f mm from it at r 32 (the floor's width there), more
//! further out.
use crate::{
    analysis::{self, Index},
    cam::{Builder, Geometry, Polygon, Region, area, number},
    mesh::Mesh,
    model::{Fields, Operation, Setup, Tool, get, num, pocket_setting},
    simulation::{Motion, Plan},
};
use clipper2::{EndType, JoinType};
use std::collections::{BTreeMap, HashMap, HashSet};

/// What the cone's side is put on (`chamfer_source`): the edges of the
/// selected faces, or a line given by its ends.
pub const SOURCES: [&str; 2] = ["Selected edges", "Line"];
/// Which side of a line the tool is on, looking from its start to its end
/// (`chamfer_side`): the side it cuts away. Alternate takes the Left for the
/// first copy of the pattern, the Right for the second, and so on.
pub const SIDES: [&str; 3] = ["Left", "Right", "Alternate"];
/// Which edges of the selected level faces are broken (`chamfer_edges`).
pub const EDGES: [&str; 3] = ["All edges", "Outer contour", "Hole mouths"];
/// How the cut is sized (`chamfer_size`): by the chamfer's width on the
/// face, (2), or by the tip's Z, (1).
pub const SIZES: [&str; 2] = ["Chamfer width", "Tip depth"];
/// A new Chamfer milling's settings, which one saved without a key also
/// takes (`setting`): the work coordinates (mm) of a line and its height,
/// the tip's Z, the chamfer's width and the tip's depth under it, how far
/// an open chain of edges is run on past its ends, and the polar pattern:
/// the number of copies, the angle between them and of the first (degrees,
/// anticlockwise seen from above) and the axis they turn about.
pub const DEFAULTS: [(&str, &str); 19] = [
    ("chamfer_source", SOURCES[0]),
    ("chamfer_edges", EDGES[0]),
    ("chamfer_side", SIDES[0]),
    ("chamfer_size", SIZES[0]),
    ("from_x", "0"),
    ("from_y", "0"),
    ("to_x", "10"),
    ("to_y", "0"),
    ("line_z", "0"),
    ("tip_z", "-1"),
    ("chamfer_width", "0.5"),
    ("tip_offset", "0.5"),
    ("extend", "1"),
    ("copies", "1"),
    ("copy_pitch", "0"),
    ("first_angle", "0"),
    ("axis_x", "0"),
    ("axis_y", "0"),
    ("pocket_milling", "Climb"),
];
/// The keys a Chamfer milling's settings are (`DEFAULTS`), but the
/// milling direction.
pub const KEYS: [&str; 18] = [
    "chamfer_source",
    "chamfer_edges",
    "chamfer_side",
    "chamfer_size",
    "from_x",
    "from_y",
    "to_x",
    "to_y",
    "line_z",
    "tip_z",
    "chamfer_width",
    "tip_offset",
    "extend",
    "copies",
    "copy_pitch",
    "first_angle",
    "axis_x",
    "axis_y",
];
/// The vertical depth of part inside the cone the gouge check lets pass,
/// mm: float noise where the cone's side lies on a flank.
pub const GOUGE: f32 = 0.01;

/// The setting `key` of a Chamfer milling, its default when it has none.
pub fn setting<'a>(p: &'a Fields, key: &str) -> &'a str {
    p.get(key).map(String::as_str).unwrap_or_else(|| {
        DEFAULTS
            .iter()
            .find(|(k, _)| *k == key)
            .map_or("", |(_, v)| v)
    })
}
/// Whether the operation puts the cone on the selected faces' edges, and so
/// needs a selection.
pub fn on_edges(p: &Fields) -> bool {
    setting(p, "chamfer_source") == SOURCES[0]
}
/// What is wrong with a Chamfer milling's settings, in the words of the
/// other operations' checks (each message starts with its field's label).
pub fn issues(p: &Fields) -> Vec<String> {
    let mut e = vec![];
    for (key, values) in [
        ("chamfer_source", &SOURCES[..]),
        ("chamfer_edges", &EDGES[..]),
        ("chamfer_side", &SIDES[..]),
        ("chamfer_size", &SIZES[..]),
    ] {
        if !values.contains(&setting(p, key)) {
            e.push(format!("Choose a valid {} setting.", key.replace('_', " ")));
        }
    }
    let value = |k: &str| {
        setting(p, k)
            .trim()
            .parse::<f32>()
            .ok()
            .filter(|v| v.is_finite())
    };
    let line = !on_edges(p);
    let depth = setting(p, "chamfer_size") == SIZES[1];
    let mut finite: Vec<&str> = vec!["axis_x", "axis_y", "first_angle", "copy_pitch"];
    if line {
        finite.extend(["from_x", "from_y", "to_x", "to_y", "line_z"]);
    }
    if depth {
        finite.push("tip_z");
    }
    for k in finite {
        if value(k).is_none() {
            e.push(format!("{} must be a finite number.", k.replace('_', " ")))
        }
    }
    if !depth {
        if !value("chamfer_width").is_some_and(|v| v > 0.) {
            e.push("chamfer width must be greater than zero.".into())
        }
        if !value("tip_offset").is_some_and(|v| v >= 0.) {
            e.push("tip offset cannot be negative or blank.".into())
        }
    }
    if !line && depth {
        e.push("Edges are sized by their chamfer width: choose Chamfer width, or put the cone on a line.".into())
    }
    if !line && !value("extend").is_some_and(|v| v >= 0.) {
        e.push("extend cannot be negative or blank.".into())
    }
    if !value("copies").is_some_and(|n| n.fract() == 0. && (1. ..=720.).contains(&n)) {
        e.push("copies must be a whole number from 1 to 720.".into())
    }
    if let (Some(n), Some(pitch)) = (value("copies"), value("copy_pitch"))
        && (n - 1.) * pitch.abs() >= 360. - 0.001
        && n > 1.
    {
        e.push(format!(
            "copy pitch {pitch}° makes {n} copies go round more than once: they would cut over each other."
        ))
    }
    if line
        && value("from_x")
            .zip(value("to_x"))
            .zip(value("from_y").zip(value("to_y")))
            .is_some_and(|((a, b), (c, d))| (b - a).hypot(d - c) < 0.01)
    {
        e.push("The line's two ends must be at least 0.01 mm apart.".into())
    }
    e
}

/// A chamfer mill's cone (`Tool`): half its included angle (radians), its
/// flat tip's radius and its own radius (mm).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Cone {
    pub half: f32,
    pub flat: f32,
    pub radius: f32,
}
impl Cone {
    pub(crate) fn of(tool: &Tool) -> Result<Self, String> {
        if get(&tool.p, "type") != "Chamfer mill" {
            return Err("Chamfer milling cuts with a chamfer mill.".into());
        }
        let angle = tool
            .tip_angle()
            .ok_or("Enter a chamfer mill tip angle from 20° to 160°.")?;
        let radius = num(&tool.p, "diameter")
            .filter(|d| *d > 0.)
            .ok_or("Enter a positive tool diameter.")?
            / 2.;
        let flat = tool.tip_flat() / 2.;
        if flat >= radius {
            return Err("The chamfer mill's flat tip must be narrower than the tool.".into());
        }
        Ok(Self {
            half: (angle / 2.).to_radians(),
            flat,
            radius,
        })
    }
    /// The cone's radius `h` above its tip, r(h).
    pub(crate) fn at(&self, h: f32) -> f32 {
        self.flat + h.max(0.) * self.half.tan()
    }
}
/// How a cut is sized (`chamfer_size`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Size {
    /// The tip at this Z, setup frame (mm).
    Depth(f32),
    /// The chamfer this wide on the face, the tip this far under it (mm).
    Width { width: f32, tip_offset: f32 },
}
impl Size {
    /// Where the tool goes for a line or edge at `z_line`: its axis's
    /// offset from the line, towards the air, and its tip's Z, by (1) or
    /// (2), checked against the cone.
    pub(crate) fn cut(self, cone: &Cone, z_line: f32, flute: f32) -> Result<(f32, f32), String> {
        let (offset, tip) = match self {
            Size::Depth(tip) => (cone.at(z_line - tip), tip),
            Size::Width { width, tip_offset } => {
                let tip = z_line - width / cone.half.tan() - tip_offset;
                (cone.at(z_line - tip) - width, tip)
            }
        };
        if tip >= z_line - 0.0001 {
            return Err(format!(
                "The tip at Z {:.3} is not below the line at Z {:.3}: lower the tip.",
                tip, z_line
            ));
        }
        let wide = cone.at(z_line - tip);
        if wide > cone.radius + 0.0001 {
            return Err(format!(
                "The cone is Ø{:.2} where it meets the line or the face, wider than this Ø{:.2} chamfer mill: raise the tip, make the chamfer narrower, or take a wider chamfer mill.",
                2. * wide,
                2. * cone.radius
            ));
        }
        if z_line - tip > flute {
            return Err(format!(
                "The cut is {:.2} mm deep from the tip, deeper than the {flute} mm flutes.",
                z_line - tip
            ));
        }
        Ok((offset, tip))
    }
}
/// The polar pattern (`copies`): copy `k` turned `first + k·pitch` degrees
/// anticlockwise, seen from above, about the vertical axis through `centre`
/// (setup frame, mm).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Pattern {
    pub count: usize,
    pub pitch: f32,
    pub first: f32,
    pub centre: [f32; 2],
}
impl Pattern {
    fn read(op: &Operation, zero: [f32; 3]) -> Result<Self, String> {
        let count = number(op, "copies")?;
        if count.fract() != 0. || !(1. ..=720.).contains(&count) {
            return Err("copies must be a whole number from 1 to 720.".into());
        }
        Ok(Self {
            count: count as usize,
            pitch: number(op, "copy_pitch")?,
            first: number(op, "first_angle")?,
            centre: [
                number(op, "axis_x")? + zero[0],
                number(op, "axis_y")? + zero[1],
            ],
        })
    }
    /// Copy `k`'s angle, degrees.
    pub(crate) fn angle(&self, k: usize) -> f32 {
        self.first + k as f32 * self.pitch
    }
    /// `p` as copy `k` puts it.
    pub(crate) fn turn(&self, p: [f32; 2], k: usize) -> [f32; 2] {
        let (s, c) = f64::from(self.angle(k)).to_radians().sin_cos();
        let d = [
            f64::from(p[0] - self.centre[0]),
            f64::from(p[1] - self.centre[1]),
        ];
        [
            self.centre[0] + (d[0] * c - d[1] * s) as f32,
            self.centre[1] + (d[0] * s + d[1] * c) as f32,
        ]
    }
}
/// Passes, each a level polyline of the tool's tip (setup frame, mm).
type Passes = Vec<Vec<[f32; 3]>>;
/// A triangle's corners.
type Triangle = [[f32; 3]; 3];
/// The point `o` to the left of `p` (right for negative `o`), across the
/// direction `u`.
fn left(p: [f32; 2], u: [f32; 2], o: f32) -> [f32; 2] {
    [p[0] - u[1] * o, p[1] + u[0] * o]
}
fn unit(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    let l = (b[0] - a[0]).hypot(b[1] - a[1]).max(1e-12);
    [(b[0] - a[0]) / l, (b[1] - a[1]) / l]
}
/// The passes of a Line: the line (work frame) moved by the work zero
/// `zero`, each copy's with its axis off it by (1) or (2) to its side, at
/// the tip's Z, from the line's start to its end; with a Chamfer width,
/// each copy's chamfer too (`Allowance`).
fn line_passes(
    op: &Operation,
    cone: &Cone,
    size: Size,
    flute: f32,
    zero: [f32; 3],
    pattern: &Pattern,
) -> Result<(Passes, Vec<Allowance>), String> {
    let point = |x: &str, y: &str| -> Result<[f32; 2], String> {
        Ok([number(op, x)? + zero[0], number(op, y)? + zero[1]])
    };
    let (a, b) = (point("from_x", "from_y")?, point("to_x", "to_y")?);
    if (b[0] - a[0]).hypot(b[1] - a[1]) < 0.01 {
        return Err("The line's two ends must be at least 0.01 mm apart.".into());
    }
    let z_line = number(op, "line_z")? + zero[2];
    let (offset, tip) = size.cut(cone, z_line, flute)?;
    let side = setting(&op.p, "chamfer_side");
    let (mut passes, mut allowed) = (vec![], vec![]);
    for k in 0..pattern.count {
        let s = match side {
            "Left" => 1.,
            "Right" => -1.,
            "Alternate" if k % 2 == 0 => 1.,
            "Alternate" => -1.,
            _ => return Err("Choose Left, Right or Alternate for the side.".into()),
        };
        let (a, b) = (pattern.turn(a, k), pattern.turn(b, k));
        let u = unit(a, b);
        passes.push(
            [a, b]
                .map(|p| {
                    let q = left(p, u, s * offset);
                    [q[0], q[1], tip]
                })
                .to_vec(),
        );
        // The material is across the line from the tool: on its left
        // looking from its end to its start when the tool is on the left.
        if let Size::Width { width, .. } = size {
            allowed.push(Allowance {
                path: if s > 0. { vec![b, a] } else { vec![a, b] },
                closed: false,
                z: z_line,
                width,
                cot: 1. / cone.half.tan(),
            });
        }
    }
    Ok((passes, allowed))
}
/// A chain of edges of the selected level faces, at height `z`, the face on
/// its left looking along it: closed (its last point is not its first
/// again) or open.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Chain {
    pub points: Vec<[f32; 2]>,
    pub z: f32,
    pub closed: bool,
}
type Key = [i64; 3];
/// A directed edge: its start's and end's keys, and their XY.
type Edge = (Key, Key, [f32; 2], [f32; 2]);
fn key(p: [f32; 3]) -> Key {
    p.map(|v| (f64::from(v) * 10_000.).round() as i64)
}
fn undirected(a: Key, b: Key) -> (Key, Key) {
    if a <= b { (a, b) } else { (b, a) }
}
/// The edges the selection picks: the outlines of its level faces (facing
/// up), at each height the boundary of the faces there, which `which`
/// (`EDGES`) narrows to the outer contours or the hole mouths. When the
/// selection also holds other faces (walls), only the edges the level
/// faces share with them: a picked straight edge, or a chain of them.
pub(crate) fn chains(g: &Geometry, op: &Operation, which: &str) -> Result<Vec<Chain>, String> {
    let ids: HashSet<usize> = op
        .surfaces
        .as_ref()
        .map(|s| s.ids.iter().copied().collect())
        .unwrap_or_default();
    let picked: Vec<_> = (g.triangles.iter())
        .filter(|t| t.feature.is_some_and(|f| ids.contains(&f)))
        .collect();
    if picked.is_empty() {
        return Err("Select the level face whose edges the chamfer breaks.".into());
    }
    let up = |p: &[[f32; 3]; 3]| {
        let u = [0, 1, 2].map(|k| p[1][k] - p[0][k]);
        let v = [0, 1, 2].map(|k| p[2][k] - p[0][k]);
        let n = [
            u[1] * v[2] - u[2] * v[1],
            u[2] * v[0] - u[0] * v[2],
            u[0] * v[1] - u[1] * v[0],
        ];
        let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        l > 1e-12 && n[2] / l >= 0.9999
    };
    let (level, walls): (Vec<_>, Vec<_>) = picked.into_iter().partition(|t| up(&t.p));
    if level.is_empty() {
        return Err("Select a level face on top of the edges to chamfer: its outline and hole mouths are the edges. Add a wall beside it to chamfer only the edge they share.".into());
    }
    let shared: HashSet<(Key, Key)> = (walls.iter())
        .flat_map(|t| (0..3).map(|i| undirected(key(t.p[i]), key(t.p[(i + 1) % 3]))))
        .collect();
    // The level faces by height, to 0.001 mm, each at its first vertex's.
    let mut heights: BTreeMap<i64, (f32, Vec<Triangle>)> = BTreeMap::new();
    for t in &level {
        let z = (t.p.iter().map(|p| f64::from(p[2])).sum::<f64>() / 3. * 1000.).round() as i64;
        heights.entry(z).or_insert((t.p[0][2], vec![])).1.push(t.p);
    }
    let mut out = vec![];
    for (z, triangles) in heights.into_values().rev() {
        let mut count: HashMap<(Key, Key), usize> = HashMap::new();
        let mut edges: Vec<Edge> = vec![];
        for p in &triangles {
            for i in 0..3 {
                let (a, b) = (p[i], p[(i + 1) % 3]);
                *count.entry(undirected(key(a), key(b))).or_default() += 1;
                edges.push((key(a), key(b), [a[0], a[1]], [b[0], b[1]]));
            }
        }
        edges.retain(|(a, b, ..)| {
            count[&undirected(*a, *b)] == 1
                && (walls.is_empty() || shared.contains(&undirected(*a, *b)))
        });
        for chain in link(&edges) {
            let closed = chain.1;
            let points = chain.0;
            let polygon: Polygon = (points.iter())
                .map(|p| (f64::from(p[0]), f64::from(p[1])))
                .collect();
            let outer = area(&polygon) > 0.;
            let keep = !walls.is_empty()
                || !closed
                || match which {
                    "Outer contour" => outer,
                    "Hole mouths" => !outer,
                    _ => true,
                };
            if keep && points.len() >= 2 {
                out.push(Chain { points, z, closed });
            }
        }
    }
    if out.is_empty() {
        return Err(if walls.is_empty() {
            format!("The selected faces have no edges of the kind chosen ({which}).")
        } else {
            "The selected level face shares no edge with the other selected faces.".into()
        });
    }
    Ok(out)
}
/// The directed edges (start, end, and their XY) linked into chains: the
/// open ones from the starts nothing ends at, then the loops; each with
/// whether it is closed (its last point is not its first again).
fn link(edges: &[Edge]) -> Vec<(Vec<[f32; 2]>, bool)> {
    let mut from: HashMap<Key, Vec<usize>> = HashMap::new();
    let mut ends: HashSet<Key> = HashSet::new();
    for (i, e) in edges.iter().enumerate() {
        from.entry(e.0).or_default().push(i);
        ends.insert(e.1);
    }
    let mut used = vec![false; edges.len()];
    let follow = |start: usize, used: &mut Vec<bool>| -> (Vec<[f32; 2]>, bool) {
        let mut points = vec![edges[start].2];
        let mut at = start;
        used[at] = true;
        loop {
            let next = edges[at].1;
            if next == edges[start].0 {
                return (points, true);
            }
            match (from.get(&next).into_iter().flatten()).find(|i| !used[**i]) {
                Some(&i) => {
                    used[i] = true;
                    points.push(edges[i].2);
                    at = i;
                }
                None => {
                    points.push(edges[at].3);
                    return (points, false);
                }
            }
        }
    };
    let mut out = vec![];
    for i in 0..edges.len() {
        if !used[i] && !ends.contains(&edges[i].0) {
            out.push(follow(i, &mut used));
        }
    }
    for i in 0..edges.len() {
        if !used[i] {
            out.push(follow(i, &mut used));
        }
    }
    out
}
/// The tool's path for `chain`: `offset` off it towards the air (its
/// right), a closed one round its loop (grown round an outer contour,
/// shrunk inside a hole), an open one run on `extend` past both its ends.
/// Each with the face on its left, as the chain.
fn offset_chain(chain: &Chain, offset: f32, extend: f32) -> Result<Vec<Vec<[f32; 2]>>, String> {
    if chain.closed {
        let mut polygon: Polygon = (chain.points.iter())
            .map(|p| (f64::from(p[0]), f64::from(p[1])))
            .collect();
        let outer = area(&polygon) > 0.;
        if !outer {
            polygon.reverse();
        }
        let delta = if outer { offset } else { -offset };
        let paths: Vec<Polygon> = Region::from(vec![polygon])
            .inflate(f64::from(delta), JoinType::Round, EndType::Polygon, 2.)
            .simplify(0.001, false)
            .into();
        if paths.is_empty() {
            return Err(format!(
                "The tool's axis does not fit {:.2} mm inside a hole mouth at Z {:.2}: take a smaller tip offset or a pointed chamfer mill.",
                offset, chain.z
            ));
        }
        return Ok(paths
            .into_iter()
            .map(|mut p| {
                // The face on the left: anticlockwise round an outer
                // contour, clockwise in a hole.
                if (area(&p) > 0.) != outer {
                    p.reverse();
                }
                let mut points: Vec<[f32; 2]> =
                    p.iter().map(|q| [q.0 as f32, q.1 as f32]).collect();
                if let Some(&first) = points.first() {
                    points.push(first);
                }
                points
            })
            .collect());
    }
    let p = &chain.points;
    let n = p.len();
    let dirs: Vec<[f32; 2]> = p.windows(2).map(|w| unit(w[0], w[1])).collect();
    let mut points: Vec<[f32; 2]> = (0..n)
        .map(|i| {
            let (before, after) = (dirs[i.saturating_sub(1)], dirs[i.min(n - 2)]);
            // The mitred offset: along the two edges' bisector, as far as
            // keeps each edge `offset` away (at most 4 times it).
            let m = [before[0] + after[0], before[1] + after[1]];
            let l = m[0].hypot(m[1]);
            let m = if l < 1e-6 {
                after
            } else {
                [m[0] / l, m[1] / l]
            };
            let cos = (m[0] * after[0] + m[1] * after[1]).max(0.25);
            left(p[i], m, -offset / cos)
        })
        .collect();
    let (d0, d1) = (dirs[0], dirs[n - 2]);
    points[0] = [points[0][0] - d0[0] * extend, points[0][1] - d0[1] * extend];
    points[n - 1] = [
        points[n - 1][0] + d1[0] * extend,
        points[n - 1][1] + d1[1] * extend,
    ];
    Ok(vec![points])
}
/// A Chamfer milling's toolpath (`cam::generate`): the cone's side on a
/// line or on the selected faces' edges, sized by (1) or (2), copied by the
/// polar pattern, one level pass each.
pub fn generate(setup: &Setup, op: &Operation, tool: &Tool, mesh: &Mesh) -> Result<Plan, String> {
    let mut b = Builder::new(setup, op, tool, mesh)?;
    let cuts = cuts(setup, op, tool, mesh, &b)?;
    b.plan.shape = cuts.shape;
    b.plan.milling_direction = cuts.direction;
    let count = cuts.passes.len();
    crate::progress::expect(count);
    b.plan.levels = 1;
    for (i, pass) in cuts.passes.iter().enumerate() {
        crate::progress::report(i)?;
        b.plan.target = b.plan.target.min(pass[0][2]);
        b.stroke(pass)?;
    }
    b.finish()
}
/// What a Chamfer milling cuts (`cuts`): its passes in order, each a level
/// polyline of the tip, the chamfers they are meant to leave on the part
/// (`Allowance`), the cone the simulation sweeps and the milling direction.
pub(crate) struct Cuts {
    pub passes: Vec<Vec<[f32; 3]>>,
    pub allowed: Vec<Allowance>,
    pub shape: crate::simulation::CutterShape,
    pub direction: &'static str,
}
/// The cuts of the Chamfer milling `op` with the plan `b` starts.
fn cuts(
    setup: &Setup,
    op: &Operation,
    tool: &Tool,
    mesh: &Mesh,
    b: &Builder,
) -> Result<Cuts, String> {
    let cone = Cone::of(tool)?;
    let angle = tool
        .tip_angle()
        .ok_or("Enter a chamfer mill tip angle from 20° to 160°.")?;
    if let Some(issue) = issues(&op.p).first() {
        return Err(issue.clone());
    }
    let zero = crate::post::datum(setup, b.plan.stock, mesh)?;
    let pattern = Pattern::read(op, zero)?;
    let size = if setting(&op.p, "chamfer_size") == SIZES[1] {
        Size::Depth(number(op, "tip_z")? + zero[2])
    } else {
        Size::Width {
            width: number(op, "chamfer_width")?,
            tip_offset: number(op, "tip_offset")?,
        }
    };
    let flute = b.plan.flute_length;
    let (mut passes, allowed, direction) = if on_edges(&op.p) {
        let climb = pocket_setting(&op.p, "pocket_milling") == "Climb";
        if !crate::model::FACING_MILLING.contains(&pocket_setting(&op.p, "pocket_milling")) {
            return Err("Choose Climb or Conventional.".into());
        }
        let Size::Width { width, .. } = size else {
            return Err("Edges are sized by their chamfer width: choose Chamfer width.".into());
        };
        let g = Geometry::new(setup, mesh);
        let extend = number(op, "extend")?;
        let (mut passes, mut allowed) = (vec![], vec![]);
        for chain in chains(&g, op, setting(&op.p, "chamfer_edges"))? {
            let (offset, tip) = size.cut(&cone, chain.z, flute)?;
            for k in 0..pattern.count {
                allowed.push(Allowance {
                    path: chain.points.iter().map(|p| pattern.turn(*p, k)).collect(),
                    closed: chain.closed,
                    z: chain.z,
                    width,
                    cot: 1. / cone.half.tan(),
                });
            }
            for mut path in offset_chain(&chain, offset, extend)? {
                // The face is on the path's left: a clockwise spindle climbs
                // with it on the right.
                if climb == b.plan.spindle_clockwise {
                    path.reverse();
                }
                for k in 0..pattern.count {
                    passes.push(
                        (path.iter())
                            .map(|p| {
                                let q = pattern.turn(*p, k);
                                [q[0], q[1], tip]
                            })
                            .collect::<Vec<_>>(),
                    );
                }
            }
        }
        let direction = if climb { "Climb" } else { "Conventional" };
        (passes, allowed, direction)
    } else {
        let (passes, allowed) = line_passes(op, &cone, size, flute, zero, &pattern)?;
        (passes, allowed, "Along each line")
    };
    // A line's copies in the pattern's order; edges nearest next.
    let mut ordered = vec![];
    if !passes.is_empty() {
        ordered.push(passes.remove(0));
    }
    while !passes.is_empty() {
        let at = *ordered.last().unwrap().last().unwrap();
        let d = |p: &Vec<[f32; 3]>| (p[0][0] - at[0]).hypot(p[0][1] - at[1]);
        let next = if on_edges(&op.p) {
            (0..passes.len())
                .min_by(|&i, &j| d(&passes[i]).total_cmp(&d(&passes[j])))
                .unwrap()
        } else {
            0
        };
        ordered.push(passes.remove(next));
    }
    Ok(Cuts {
        passes: ordered,
        allowed,
        shape: tool.chamfer_shape(angle),
        direction,
    })
}
/// The chamfers the Chamfer milling `op` in `setup` is meant to cut into the
/// part (`Allowance`), for the gouge check.
pub(crate) fn allowances(
    setup: &Setup,
    op: &Operation,
    tool: &Tool,
    mesh: &Mesh,
) -> Result<Vec<Allowance>, String> {
    let b = Builder::new(setup, op, tool, mesh)?;
    Ok(cuts(setup, op, tool, mesh, &b)?.allowed)
}
/// A chamfer an edge is meant to get, which the gouge check lets the cone
/// cut: in the section across the edge `path` (closed or open, the part on
/// its left) at height `z`, the corner above the chamfer's plane, from `width`
/// in on the face down to `width × cot` on the wall (`cot` the cotangent of
/// half the tip angle).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Allowance {
    pub path: Vec<[f32; 2]>,
    pub closed: bool,
    pub z: f32,
    pub width: f32,
    pub cot: f32,
}
impl Allowance {
    /// The lowest the chamfer goes at `p`: None where `p` is not within its
    /// width of the edge on the part's side.
    fn floor(&self, p: [f32; 2]) -> Option<f32> {
        let n = self.path.len();
        let segments = if self.closed { n } else { n - 1 };
        let (mut best, mut side) = (f32::INFINITY, 0f32);
        for i in 0..segments {
            let (a, b) = (self.path[i], self.path[(i + 1) % n]);
            // Far from this edge: no nearer than what is known.
            let lo = [a[0].min(b[0]) - best, a[1].min(b[1]) - best];
            let hi = [a[0].max(b[0]) + best, a[1].max(b[1]) + best];
            if p[0] < lo[0] || p[1] < lo[1] || p[0] > hi[0] || p[1] > hi[1] {
                continue;
            }
            let v = [b[0] - a[0], b[1] - a[1]];
            let l2 = v[0] * v[0] + v[1] * v[1];
            let t = if l2 < 1e-12 {
                0.
            } else {
                (((p[0] - a[0]) * v[0] + (p[1] - a[1]) * v[1]) / l2).clamp(0., 1.)
            };
            let d = (p[0] - a[0] - t * v[0]).hypot(p[1] - a[1] - t * v[1]);
            if d < best {
                best = d;
                side = v[0] * (p[1] - a[1]) - v[1] * (p[0] - a[0]);
            }
        }
        let inside = if self.closed {
            // A loop's left is its inside when it runs anticlockwise.
            let polygon: Polygon = (self.path.iter())
                .map(|q| (f64::from(q[0]), f64::from(q[1])))
                .collect();
            crate::cam::encloses(&polygon, (f64::from(p[0]), f64::from(p[1])))
                == (area(&polygon) > 0.)
        } else {
            side >= 0.
        };
        // On the edge, within `GOUGE` of it, is the chamfer's lowest corner
        // whichever side the tessellation puts it.
        ((inside || best <= GOUGE) && best <= self.width + GOUGE)
            .then(|| self.z - (self.width - best).max(0.) * self.cot)
    }
}
/// The deepest the cone of a Chamfer milling `plan` goes into the part
/// along its feed moves, mm along Z, and where: at points 0.05 mm apart
/// under each move's sweep (a level move's band, a plunge's disc at its
/// bottom, points along any other move), the part above the cone's
/// surface there. `index` is in the plan's setup frame, `top` the model's
/// top there.
pub(crate) fn gouge(
    plan: &Plan,
    index: &Index,
    top: f32,
    allowed: &[Allowance],
) -> (f32, [f32; 3]) {
    const STEP: f32 = 0.05;
    let rise = plan.shape.rise().unwrap_or(0.);
    let flat = plan.shape.flat();
    let (mut worst, mut at) = (0f32, [0f32; 3]);
    // The part inside the cutter whose tip is at `tip`, at `xy` `d` from its
    // axis, but a chamfer meant there: what is under the chamfer's floor or
    // over its face.
    let mut probe = |xy: [f32; 2], d: f32, tip: f32| {
        let z = tip + rise * (d - flat).max(0.);
        if z >= top {
            return;
        }
        // The part's column and the chamfers at one point: the same tiny
        // offset as the part grid keeps the ray off mesh edges.
        let q = [xy[0] + 1e-4, xy[1] + 1e-4];
        let chamfer = (allowed.iter())
            .filter_map(|a| a.floor(q).map(|floor| (floor, a.z)))
            .min_by(|a, b| a.0.total_cmp(&b.0));
        for (lo, hi) in analysis::solid(index, q, top) {
            let (from, to) = (lo.max(z), hi.min(top));
            let depth = match chamfer {
                Some((floor, face)) => (to.min(floor) - from).max(to - from.max(face + GOUGE)),
                None => to - from,
            };
            if hi > z && depth > worst {
                (worst, at) = (depth, [xy[0], xy[1], z]);
            }
        }
    };
    for m in plan.moves.iter().filter(|m| m.motion == Motion::Feed) {
        let (a, b) = (m.from, m.to);
        let low = a[2].min(b[2]);
        // The cutter's radius at the model's top: past it nothing is cut.
        let reach = if rise > 0. {
            (flat + (top - low).max(0.) / rise).min(plan.radius)
        } else {
            plan.radius
        };
        let xy = (b[0] - a[0]).hypot(b[1] - a[1]);
        let level = (b[2] - a[2]).abs() < 1e-6;
        let (from, to) = if level || xy < 1e-6 {
            // A plunge's sweep is the cutter at its bottom.
            let bottom = if xy < 1e-6 && b[2] < a[2] { b } else { a };
            (bottom, if level { b } else { bottom })
        } else {
            // Any other move: its points every step, each a disc.
            let n = (xy / STEP).ceil() as usize;
            for i in 0..=n {
                let t = i as f32 / n as f32;
                let c: [f32; 3] = std::array::from_fn(|k| a[k] + (b[k] - a[k]) * t);
                disc(c, reach, STEP, &mut probe);
            }
            continue;
        };
        // A level move's band: every point within `reach` of it.
        let lo = [0, 1].map(|k| from[k].min(to[k]) - reach);
        let hi = [0, 1].map(|k| from[k].max(to[k]) + reach);
        let n = [0, 1].map(|k| ((hi[k] - lo[k]) / STEP).ceil() as usize);
        let v = [to[0] - from[0], to[1] - from[1]];
        let l2 = v[0] * v[0] + v[1] * v[1];
        for i in 0..=n[0] {
            for j in 0..=n[1] {
                let p = [lo[0] + i as f32 * STEP, lo[1] + j as f32 * STEP];
                let t = if l2 < 1e-12 {
                    0.
                } else {
                    (((p[0] - from[0]) * v[0] + (p[1] - from[1]) * v[1]) / l2).clamp(0., 1.)
                };
                let d = (p[0] - from[0] - t * v[0]).hypot(p[1] - from[1] - t * v[1]);
                if d <= reach {
                    probe(p, d, from[2]);
                }
            }
        }
    }
    (worst, at)
}
/// `probe` at the points `step` apart within `reach` of `c`'s XY, with the
/// tip at `c`'s Z.
fn disc(c: [f32; 3], reach: f32, step: f32, probe: &mut impl FnMut([f32; 2], f32, f32)) {
    let n = (reach / step).ceil() as i32;
    for i in -n..=n {
        for j in -n..=n {
            let p = [c[0] + i as f32 * step, c[1] + j as f32 * step];
            let d = (p[0] - c[0]).hypot(p[1] - c[1]);
            if d <= reach {
                probe(p, d, c[2]);
            }
        }
    }
}
/// The gouge check's problem for a Chamfer milling (`plan_check`): the cone
/// more than `GOUGE` into the part past the chamfers `allowed`, said where as
/// the tool axis problems say it.
pub(crate) fn gouge_problem(
    plan: &Plan,
    geometry: &Geometry,
    index: &Index,
    allowed: &[Allowance],
) -> Option<String> {
    let b = geometry.bounds;
    let (depth, at) = gouge(plan, index, b.max[2], allowed);
    (depth > GOUGE).then(|| {
        format!(
            "The chamfer mill's cone cuts {depth:.3} mm into the part at X {:.2}, Y {:.2}, {:.2} mm below the model top (this setup's axes, measured from the model's -X/-Y corner): its side is off the line or the edge, on the wrong side of it, or its tip is too deep.",
            at[0] - b.min[0],
            at[1] - b.min[1],
            b.max[2] - at[2]
        )
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        mesh,
        model::{Machine, Strategy},
        simulation::{CutterShape, StockGrid},
        surface::Selection,
        volume::swept,
    };
    const TAN30: f32 = 0.577_350_3;
    /// A chamfer mill of `angle`°, `flat` mm across its tip, Ø6.
    fn mill(angle: &str, flat: &str) -> Tool {
        let mut t = Tool::new("Spot drill", "Chamfer mill", "6");
        for (k, v) in [
            ("tip_angle", angle),
            ("tip_flat", flat),
            ("length", "10"),
            ("stickout", "30"),
        ] {
            t.p.insert(k.into(), v.into());
        }
        t
    }
    fn chamfer(tool: &Tool, fields: &[(&str, &str)]) -> Operation {
        let mut op = Operation::new(Strategy::Chamfer, tool.id.clone());
        for (k, v) in fields {
            op.p.insert((*k).into(), (*v).into());
        }
        op
    }
    /// The part of `triangles` in a setup with no stock margins, its work
    /// zero the stock top's centre: work coordinates are the triangles' own
    /// when they are centred on X and Y with their top at Z 0.
    fn part(triangles: Vec<[[f32; 3]; 3]>) -> (Setup, Mesh) {
        (
            Setup::new(Machine::new(3), "G54"),
            mesh::normalize(triangles).unwrap(),
        )
    }
    /// The section `s` (x, z), anticlockwise, from y `y0` to `y1`.
    fn extruded(s: &[(f32, f32)], y0: f32, y1: f32) -> Vec<[[f32; 3]; 3]> {
        let p = |i: usize, y: f32| [s[i % s.len()].0, y, s[i % s.len()].1];
        let mut t = vec![];
        for i in 1..s.len() - 1 {
            t.push([p(0, y0), p(i, y0), p(i + 1, y0)]);
            t.push([p(0, y1), p(i + 1, y1), p(i, y1)]);
        }
        for i in 0..s.len() {
            let (a, b, c, d) = (p(i, y0), p(i + 1, y0), p(i + 1, y1), p(i, y1));
            t.push([a, c, b]);
            t.push([a, d, c]);
        }
        t
    }
    /// A flank as the clutch's: the part on −X up to the plane 30° from the
    /// vertical through the line X 0 at Z −0.35, widening downward, its top
    /// at Z 0, and the root floor beside it at Z −1.30.
    fn flank() -> (Setup, Mesh) {
        let x = |z: f32| (-0.35 - z) * TAN30;
        part(extruded(
            &[
                (-10., -5.),
                (10., -5.),
                (10., -1.3),
                (x(-1.3), -1.3),
                (x(0.), 0.),
                (-10., 0.),
            ],
            -10.,
            10.,
        ))
    }
    /// The 40 × 30 × 5 plate with a 10 × 10 hole through its middle.
    fn plate() -> (Setup, Mesh) {
        let outer = [[-20., -15.], [20., -15.], [20., 15.], [-20., 15.]];
        let inner = [[-5., -5.], [5., -5.], [5., 5.], [-5., 5.]];
        let at = |p: [f32; 2], z: f32| [p[0], p[1], z];
        let mut t = vec![];
        for i in 0..4 {
            let j = (i + 1) % 4;
            let (o0, o1, i0, i1) = (outer[i], outer[j], inner[i], inner[j]);
            t.push([at(o0, 0.), at(o1, 0.), at(i1, 0.)]);
            t.push([at(o0, 0.), at(i1, 0.), at(i0, 0.)]);
            t.push([at(o0, -5.), at(i1, -5.), at(o1, -5.)]);
            t.push([at(o0, -5.), at(i0, -5.), at(i1, -5.)]);
            t.push([at(o0, -5.), at(o1, -5.), at(o1, 0.)]);
            t.push([at(o0, -5.), at(o1, 0.), at(o0, 0.)]);
            t.push([at(i0, -5.), at(i1, 0.), at(i1, -5.)]);
            t.push([at(i0, -5.), at(i0, 0.), at(i1, 0.)]);
        }
        part(t)
    }
    /// The Ø76 × 6 disc the rotor's teeth are cut in, its top the turned
    /// tip surface.
    fn disc() -> (Setup, Mesh) {
        part(crate::cam::tests::revolved(&[
            (0., 6.),
            (38., 6.),
            (38., 0.),
            (0., 0.),
        ]))
    }
    /// The rotor's tooth program (`MACHINING.md` → Teeth, the module doc).
    const TEETH: [(&str, &str); 13] = [
        ("chamfer_source", "Line"),
        ("from_x", "31.8"),
        ("from_y", "0"),
        ("to_x", "39.5"),
        ("to_y", "0"),
        ("line_z", "-0.35"),
        ("chamfer_size", "Tip depth"),
        ("tip_z", "-1.3"),
        ("chamfer_side", "Alternate"),
        ("copies", "60"),
        ("copy_pitch", "6"),
        ("first_angle", "3"),
        ("axis_x", "0"),
    ];
    fn zero(setup: &Setup, plan: &Plan, mesh: &Mesh) -> [f32; 3] {
        crate::post::datum(setup, plan.stock, mesh).unwrap()
    }
    fn cutting(plan: &Plan) -> Vec<&crate::simulation::Move> {
        plan.moves.iter().filter(|m| m.cutting_pass).collect()
    }
    /// Where the sweep of `m` ends going from `out` (cut) to `inn` (not):
    /// the fraction of the way, bisected.
    fn boundary(plan: &Plan, m: &crate::simulation::Move, at: impl Fn(f32) -> [f32; 3]) -> f32 {
        let cut = |s: f32| {
            swept(
                at(s),
                m.from,
                m.to,
                plan.radius,
                plan.flute_length,
                plan.shape,
            )
        };
        assert!(cut(1.) && !cut(0.), "{:?} {:?}", at(1.), at(0.));
        let (mut lo, mut hi) = (0f32, 1f32);
        for _ in 0..40 {
            let mid = (lo + hi) / 2.;
            if cut(mid) {
                hi = mid;
            } else {
                lo = mid;
            }
        }
        (lo + hi) / 2.
    }
    #[test]
    fn the_cone_side_offset_follows_the_tip_angle_flat_and_depth_or_the_width() {
        let cone = |angle: f32, flat: f32| Cone {
            half: (angle / 2.).to_radians(),
            flat: flat / 2.,
            radius: 3.,
        };
        let depth = |c: Cone, line: f32, tip: f32| Size::Depth(tip).cut(&c, line, 10.).unwrap();
        let near =
            |a: (f32, f32), b: (f32, f32)| (a.0 - b.0).abs() < 1e-5 && (a.1 - b.1).abs() < 1e-5;
        // The clutch's flank: a 60° mill's tip at the root floor, Z −1.30,
        // its side on the line at the pitch plane, Z −0.35: 0.95 × tan 30°.
        assert!(near(depth(cone(60., 0.), -0.35, -1.3), (0.548_483, -1.3)));
        // A 0.2 flat adds its radius, at any depth.
        assert!(near(depth(cone(60., 0.2), -0.35, -1.3), (0.648_483, -1.3)));
        assert!(near(
            depth(cone(60., 0.2), -0.35, -0.7),
            (0.1 + 0.35 * TAN30, -0.7)
        ));
        // 90°: as far off as the line is above the tip.
        assert!(near(depth(cone(90., 0.), 0., -1.), (1., -1.)));
        assert!(near(depth(cone(90., 0.4), 0., -2.), (2.2, -2.)));
        // By width: the tip the width down the wall and the tip offset under
        // it, the axis the tip offset's share of the cone off the edge.
        let wide = |c: Cone, w: f32, t: f32| {
            Size::Width {
                width: w,
                tip_offset: t,
            }
            .cut(&c, 0., 10.)
            .unwrap()
        };
        assert!(near(wide(cone(90., 0.), 0.5, 0.5), (0.5, -1.)));
        assert!(near(wide(cone(90., 0.), 1.5, 0.5), (0.5, -2.)));
        assert!(near(
            wide(cone(60., 0.), 0.5, 0.5),
            (0.5 * TAN30, -0.5 / TAN30 - 0.5)
        ));
        assert!(near(
            wide(cone(60., 0.2), 0.5, 0.5),
            (0.1 + 0.5 * TAN30, -0.5 / TAN30 - 0.5)
        ));
        // A tip above the line, or a cone wider than the mill where it
        // meets the line, is refused.
        let tip_above = Size::Depth(0.5).cut(&cone(60., 0.), 0., 10.).unwrap_err();
        assert!(tip_above.contains("not below the line"), "{tip_above}");
        let too_wide = Size::Depth(-6.).cut(&cone(60., 0.), 0., 10.).unwrap_err();
        assert!(too_wide.contains("wider than this Ø6.00"), "{too_wide}");
    }
    #[test]
    fn the_pattern_turns_copy_k_by_the_first_angle_and_k_pitches_about_its_axis() {
        let p = Pattern {
            count: 4,
            pitch: 90.,
            first: 45.,
            centre: [1., 2.],
        };
        for k in 0..4 {
            let a = (45. + 90. * k as f32).to_radians();
            let q = p.turn([11., 2.], k);
            assert!((q[0] - (1. + 10. * a.cos())).abs() < 1e-5, "{k}");
            assert!((q[1] - (2. + 10. * a.sin())).abs() < 1e-5, "{k}");
        }
        // Generated: four copies of a line out from the axis, each 90° on,
        // the first at 45°, the tool on the line's left.
        let (setup, mesh) = disc();
        let tool = mill("90", "0");
        let op = chamfer(
            &tool,
            &[
                ("chamfer_source", "Line"),
                ("from_x", "10"),
                ("to_x", "20"),
                ("line_z", "0"),
                ("chamfer_size", "Tip depth"),
                ("tip_z", "-1"),
                ("copies", "4"),
                ("copy_pitch", "90"),
                ("first_angle", "45"),
                ("axis_x", "2"),
                ("axis_y", "-3"),
            ],
        );
        let plan = generate(&setup, &op, &tool, &mesh).unwrap();
        let z = zero(&setup, &plan, &mesh);
        let cuts = cutting(&plan);
        assert_eq!(cuts.len(), 4);
        for (k, m) in cuts.iter().enumerate() {
            let a = (45. + 90. * k as f32).to_radians();
            let (s, c) = a.sin_cos();
            // (10, 1) and (20, 1) about (2, −3): the axis 1 off the line.
            for (got, x) in [(m.from, 10.), (m.to, 20.)] {
                let d = [x - 2., 1. + 3.];
                let want = [2. + d[0] * c - d[1] * s, -3. + d[0] * s + d[1] * c, -1.];
                for i in 0..3 {
                    assert!(
                        (got[i] - z[i] - want[i]).abs() < 1e-4,
                        "{k} {got:?} {want:?}"
                    );
                }
            }
        }
    }
    #[test]
    fn the_rotors_sixty_flanks_lie_on_their_radial_lines_at_the_pitch_plane_within_0_01_mm() {
        let (setup, mesh) = disc();
        for flat in [0., 0.2] {
            let tool = mill("60", &flat.to_string());
            let plan = generate(&setup, &chamfer(&tool, &TEETH), &tool, &mesh).unwrap();
            assert_eq!(plan.shape, CutterShape::Chamfer(600, (flat * 1000.) as u16));
            let z = zero(&setup, &plan, &mesh);
            let cuts = cutting(&plan);
            assert_eq!(cuts.len(), 60);
            for (j, m) in cuts.iter().enumerate() {
                let a = (3. + 6. * j as f32).to_radians();
                let (radial, across) = ([a.cos(), a.sin()], [-a.sin(), a.cos()]);
                // The gap is anticlockwise of the even lines, clockwise of
                // the odd ones: the tool on the left, then on the right.
                let gap = if j % 2 == 0 { 1. } else { -1. };
                // The axis 0.5485 + flat/2 across the line, into the gap,
                // the tip at the root floor, out from r 31.8 to r 39.5.
                for (p, r) in [(m.from, 31.8), (m.to, 39.5)] {
                    let q = [p[0] - z[0], p[1] - z[1]];
                    let off = q[0] * across[0] + q[1] * across[1];
                    let along = q[0] * radial[0] + q[1] * radial[1];
                    assert!(
                        (off - gap * (0.548_483 + flat / 2.)).abs() < 1e-4,
                        "{j} {off}"
                    );
                    assert!((along - r).abs() < 1e-4 && (p[2] - z[2] + 1.3).abs() < 1e-5);
                }
                // The sweep's flank, measured: across the line at r 33 and
                // r 37 at the pitch plane it is on the line (the flank plane
                // through the radial line), and at Z −1.0 it is 0.65 × tan 30°
                // into the gap (the plane 30° from the axis).
                for (r, height, into) in
                    [(33., -0.35, 0.), (37., -0.35, 0.), (35., -1., 0.65 * TAN30)]
                {
                    let at = |s: f32| {
                        // From 0.3 into the tooth (s 0) to 0.3 into the gap (s 1).
                        let t = gap * (into - 0.3 + 0.6 * s);
                        [
                            z[0] + r * radial[0] + t * across[0],
                            z[1] + r * radial[1] + t * across[1],
                            z[2] + height,
                        ]
                    };
                    let s = boundary(&plan, m, at);
                    let error = (into - 0.3 + 0.6 * s) - into;
                    assert!(
                        error.abs() <= 0.01,
                        "flank {j} r {r} Z {height}: {error:+.4} mm"
                    );
                    assert!(
                        error.abs() < 1e-3,
                        "flank {j} r {r} Z {height}: {error:+.5} mm"
                    );
                }
                // The far side of the cone stays off the gap's other flank,
                // 6° on: nothing on that line is cut, root to tip, at r 32.
                let b = a + gap * 6f32.to_radians();
                for height in [-1.29, -0.35, 0.] {
                    let p = [z[0] + 32. * b.cos(), z[1] + 32. * b.sin(), z[2] + height];
                    assert!(!swept(
                        p,
                        m.from,
                        m.to,
                        plan.radius,
                        plan.flute_length,
                        plan.shape
                    ));
                }
            }
        }
    }
    #[test]
    fn the_cone_check_passes_the_flank_pass_and_catches_a_wrong_side_or_too_deep_or_off_line_one() {
        let (setup, mesh) = flank();
        let tool = mill("60", "0");
        let g = Geometry::new(&setup, &mesh);
        let index = Index::new(&g);
        let depth = |changes: &[(&str, &str)]| {
            let mut fields = vec![
                ("chamfer_source", "Line"),
                ("from_x", "0"),
                ("from_y", "-12"),
                ("to_x", "0"),
                ("to_y", "12"),
                ("line_z", "-0.35"),
                ("chamfer_size", "Tip depth"),
                ("tip_z", "-1.3"),
                // Looking along +Y the air, +X, is on the right.
                ("chamfer_side", "Right"),
            ];
            fields.extend_from_slice(changes);
            let op = chamfer(&tool, &fields);
            let plan = generate(&setup, &op, &tool, &mesh).unwrap();
            let allowed = allowances(&setup, &op, &tool, &mesh).unwrap();
            assert!(allowed.is_empty());
            (
                gouge(&plan, &index, g.bounds.max[2], &allowed).0,
                gouge_problem(&plan, &g, &index, &allowed),
            )
        };
        let (right, problem) = depth(&[]);
        assert!(right <= GOUGE && problem.is_none(), "{right}");
        // On the wrong side the cone is in the part: down to its tip.
        let (wrong, problem) = depth(&[("chamfer_side", "Left")]);
        assert!(wrong > 1., "{wrong}");
        assert!(problem.unwrap().contains("cone cuts"));
        // 0.3 too deep: into the root floor, and the flank 0.3 × tan 30° in.
        let (deep, _) = depth(&[("tip_z", "-1.6")]);
        assert!(deep > 0.25, "{deep}");
        // The line 0.05 into the part: the flank cut 0.05 in, 0.087 down.
        let (off, _) = depth(&[("from_x", "-0.05"), ("to_x", "-0.05")]);
        assert!(off > 0.05, "{off}");
        // 0.02 shallower leaves the flank (no gouge): the check is not one
        // of the cut's position, only of what it takes from the part.
        let (shallow, _) = depth(&[("tip_z", "-1.28")]);
        assert!(shallow <= GOUGE, "{shallow}");
    }
    #[test]
    fn the_edges_of_a_picked_face_are_its_outline_and_its_hole_mouths_or_the_edge_it_shares() {
        let (setup, mesh) = plate();
        let g = Geometry::new(&setup, &mesh);
        let top = g.bounds.max[2];
        let face = |test: &dyn Fn(&[[f32; 3]; 3]) -> bool| {
            g.triangles
                .iter()
                .find(|t| test(&t.p))
                .unwrap()
                .feature
                .unwrap()
        };
        let upper = face(&|p| p.iter().all(|v| (v[2] - top).abs() < 1e-4));
        let east = face(&|p| p.iter().all(|v| (v[0] - 20.).abs() < 1e-4));
        let tool = mill("90", "0");
        let picked = |ids: Vec<usize>, edges: &str| {
            let mut op = chamfer(&tool, &[("chamfer_edges", edges)]);
            op.surfaces = Some(Selection {
                mesh_fingerprint: mesh.fingerprint.clone(),
                ids,
            });
            op
        };
        let all = chains(&g, &picked(vec![upper], "All edges"), "All edges").unwrap();
        assert_eq!(all.len(), 2);
        let areas: Vec<f64> = (all.iter())
            .map(|c| {
                assert!(c.closed && (c.z - top).abs() < 1e-3);
                area(
                    &c.points
                        .iter()
                        .map(|p| (p[0] as f64, p[1] as f64))
                        .collect(),
                )
            })
            .collect();
        assert!(areas.iter().any(|a| (a - 1200.).abs() < 0.01), "{areas:?}");
        assert!(areas.iter().any(|a| (a + 100.).abs() < 0.01), "{areas:?}");
        // 0.5 × 45°: round the outline 0.5 out, in the hole 0.5 in, the tip
        // 1 mm down; climbing with a clockwise spindle, clockwise round the
        // outline and anticlockwise in the hole.
        for (edges, half, clockwise) in [
            ("Outer contour", [20.5, 15.5], true),
            ("Hole mouths", [4.5, 4.5], false),
        ] {
            let op = picked(vec![upper], edges);
            let plan = generate(&setup, &op, &tool, &mesh).unwrap();
            let z = zero(&setup, &plan, &mesh);
            let cuts = cutting(&plan);
            let (mut lo, mut hi) = ([f32::INFINITY; 2], [f32::NEG_INFINITY; 2]);
            let mut turn = 0.;
            for m in &cuts {
                for k in 0..2 {
                    lo[k] = lo[k].min(m.to[k] - z[k]);
                    hi[k] = hi[k].max(m.to[k] - z[k]);
                }
                assert!((m.to[2] - z[2] + 1.).abs() < 1e-4);
                turn +=
                    (m.from[0] - z[0]) * (m.to[1] - z[1]) - (m.to[0] - z[0]) * (m.from[1] - z[1]);
            }
            for k in 0..2 {
                assert!(
                    (hi[k] - half[k]).abs() < 0.01 && (lo[k] + half[k]).abs() < 0.01,
                    "{edges} {lo:?} {hi:?}"
                );
            }
            assert_eq!(turn < 0., clockwise, "{edges}");
            // The chamfer it sweeps: 0.5 in on the face, 0.5 down the wall.
            if edges == "Outer contour" {
                let m = cuts
                    .iter()
                    .find(|m| {
                        (m.from[0] - z[0] - 20.5).abs() < 1e-3
                            && (m.to[0] - z[0] - 20.5).abs() < 1e-3
                    })
                    .unwrap();
                let on_face = boundary(&plan, m, |s| [z[0] + 19. + 1.2 * s, z[1], z[2] - 0.0001]);
                assert!((19. + 1.2 * on_face - 19.5).abs() < 0.01, "{on_face}");
                let on_wall = boundary(&plan, m, |s| [z[0] + 19.9999, z[1], z[2] - 1. + s]);
                assert!((-1. + on_wall + 0.5).abs() < 0.01, "{on_wall}");
                // The cone check lets that chamfer be, but no more of it.
                let index = Index::new(&g);
                let allowed = allowances(&setup, &op, &tool, &mesh).unwrap();
                assert!(gouge(&plan, &index, top, &allowed).0 <= GOUGE);
                let narrower: Vec<_> = (allowed.iter())
                    .map(|a| Allowance {
                        width: 0.3,
                        ..a.clone()
                    })
                    .collect();
                assert!(gouge(&plan, &index, top, &narrower).0 > 0.15);
            }
        }
        // The top and the east wall: only the edge they share, run on 1 mm
        // past its ends, against its direction to climb.
        let shared = chains(&g, &picked(vec![upper, east], "All edges"), "All edges").unwrap();
        assert_eq!(shared.len(), 1);
        assert!(!shared[0].closed);
        let op = picked(vec![upper, east], "All edges");
        let plan = generate(&setup, &op, &tool, &mesh).unwrap();
        let z = zero(&setup, &plan, &mesh);
        let cuts = cutting(&plan);
        let (start, end) = (cuts[0].from, cuts.last().unwrap().to);
        for (p, want) in [(start, [20.5, 16.]), (end, [20.5, -16.])] {
            assert!(
                (p[0] - z[0] - want[0]).abs() < 1e-3 && (p[1] - z[1] - want[1]).abs() < 1e-3,
                "{p:?}"
            );
        }
        // A face that is not level picks nothing to chamfer.
        assert!(chains(&g, &picked(vec![east], "All edges"), "All edges").is_err());
    }
    #[test]
    fn a_flat_tipped_cone_cuts_flat_under_its_flat_and_rises_from_its_edge() {
        // A 90° mill, 1 mm flat, plunged to Z 3 in a 10 mm block.
        let shape = CutterShape::Chamfer(900, 1000);
        assert!((shape.flat() - 0.5).abs() < 1e-6);
        let mut grid = StockGrid::new(crate::mesh::Bounds {
            min: [0., 0., 0.],
            max: [10., 10., 10.],
        });
        grid.cut_shape([5., 5., 9.], [5., 5., 3.], 3., shape);
        for y in 0..grid.ny {
            for x in 0..grid.nx {
                let p = [(x as f32 + 0.5) * grid.dx, (y as f32 + 0.5) * grid.dy];
                let d = (p[0] - 5.).hypot(p[1] - 5.);
                if d <= 3. {
                    let want = 3. + (d - 0.5).max(0.);
                    let got = grid.heights[y * grid.nx + x];
                    assert!((got - want).abs() < 1e-4, "{d} {got} {want}");
                }
            }
        }
        let (a, b) = ([0., 0., 3.], [10., 0., 3.]);
        assert!(swept([5., 0.4, 3.001], a, b, 3., 10., shape));
        assert!(!swept([5., 1.5, 3.9], a, b, 3., 10., shape));
        assert!(swept([5., 1.5, 4.1], a, b, 3., 10., shape));
        // A pointed one (0 flat) is the cone of old.
        assert_eq!(CutterShape::Chamfer(900, 0).flat(), 0.);
    }
    #[test]
    fn a_chamfer_mills_flat_tip_is_optional_and_narrower_than_the_mill() {
        let mut t = mill("60", "");
        assert!(t.issues().is_empty() && t.tip_flat() == 0.);
        t.p.remove("tip_flat");
        assert!(t.issues().is_empty() && t.tip_flat() == 0.);
        for bad in ["-0.1", "6", "x"] {
            t.p.insert("tip_flat".into(), bad.into());
            assert!(t.issues().iter().any(|e| e.contains("flat tip")), "{bad}");
        }
        t.p.insert("tip_flat".into(), "0.2".into());
        assert!(t.issues().is_empty() && (t.tip_flat() - 0.2).abs() < 1e-6);
    }
    #[test]
    fn a_chamfer_millings_settings_are_checked() {
        let tool = mill("60", "0");
        let issues = |fields: &[(&str, &str)]| issues(&chamfer(&tool, fields).p);
        assert!(issues(&TEETH).is_empty());
        assert!(issues(&[]).is_empty());
        assert!(
            issues(&[("chamfer_size", "Tip depth")])[0].contains("sized by their chamfer width")
        );
        assert!(issues(&[("copies", "61"), ("copy_pitch", "6")])[0].contains("more than once"));
        assert!(issues(&[("copies", "1.5")])[0].starts_with("copies"));
        assert!(issues(&[("chamfer_width", "0")])[0].starts_with("chamfer width"));
        assert!(issues(&[("chamfer_source", "Line"), ("to_x", "0")])[0].contains("0.01 mm apart"));
        assert!(issues(&[("chamfer_side", "Up")])[0].contains("chamfer side"));
        // Each number's message is at its field.
        assert_eq!(
            crate::model::issue_field(&issues(&[("chamfer_width", "0")])[0]),
            Some("chamfer_width")
        );
        // The operation takes a chamfer mill, and only needs a selection on
        // edges.
        let m = Machine::new(3);
        let mut op = chamfer(&tool, &[]);
        assert_eq!(op.tool_types(&m), ["Chamfer mill"]);
        assert!(op.needs_surfaces());
        op.p.insert("chamfer_source".into(), "Line".into());
        assert!(!op.needs_surfaces());
        let flat = Tool::new("Flat", "Flat end mill", "6");
        assert!(generate(&disc().0, &chamfer(&flat, &TEETH), &flat, &disc().1).is_err());
    }
    #[test]
    fn a_planned_chamfer_builds_describes_back_is_refused_when_wrong_and_passes_the_plan_check() {
        use crate::proposal::{self, ProposedChamfer, tests::op};
        let (mut doc, mesh, _) = proposal::tests::block();
        let mut spot = mill("90", "0");
        spot.p.insert("number".into(), "4".into());
        doc.tools.push(spot);
        let report = crate::analysis::report(&doc, &mesh).unwrap();
        let top = (report
            .orientations
            .iter()
            .find(|o| o.up == "+Z")
            .unwrap()
            .planes
            .iter())
        .find(|p| p.depth_below_top.abs() < 0.01 && !p.holes.is_empty())
        .unwrap()
        .surfaces
        .clone();
        // The holes' mouths of the top, deburred 0.3 wide after drilling.
        let mut deburr = op("Chamfer", 4, top.clone());
        deburr.chamfer = Some(ProposedChamfer {
            source: "edges".into(),
            edges: Some("holes".into()),
            width: Some(0.3),
            ..Default::default()
        });
        let mut plan = proposal::tests::good(&report);
        plan.setups[0].operations.push(deburr.clone());
        let built = proposal::build(&doc, &mesh, &report, &plan).unwrap();
        let built_op = built.setups[0].operations.last().unwrap();
        assert_eq!(built_op.strategy, Strategy::Chamfer);
        for (k, v) in [
            ("chamfer_source", "Selected edges"),
            ("chamfer_edges", "Hole mouths"),
            ("chamfer_size", "Chamfer width"),
            ("chamfer_width", "0.3"),
            ("tip_offset", "0.5"),
            ("copies", "1"),
        ] {
            assert_eq!(get(&built_op.p, k), v, "{k}");
        }
        let current = proposal::describe(&built, &report).unwrap();
        let back = current.current_plan.setups[0].operations.last().unwrap();
        assert_eq!(back.strategy, "Chamfer");
        let c = back.chamfer.as_ref().unwrap();
        assert_eq!(
            (c.source.as_str(), c.edges.as_deref(), c.width),
            ("edges", Some("holes"), Some(0.3))
        );
        // Generated, its cone checked against the part, exported.
        let (check, _) = crate::plan_check::check(&doc, &mesh, &report, &plan);
        assert!(check.passed, "{:?}", check.problems);
        // A line by its tip's depth, copied round an axis.
        let mut teeth = op("Chamfer", 4, vec![]);
        teeth.chamfer = Some(ProposedChamfer {
            source: "line".into(),
            line: Some([31.8, 0., 39.5, 0.]),
            line_z: Some(-0.35),
            side: Some("alternate".into()),
            tip_z: Some(-1.3),
            copies: Some(60),
            copy_pitch: Some(6.),
            first_angle: Some(3.),
            ..Default::default()
        });
        let mut lined = proposal::tests::good(&report);
        lined.setups[0].operations = vec![teeth.clone()];
        let built = proposal::build(&doc, &mesh, &report, &lined).unwrap();
        let o = &built.setups[0].operations[0];
        for (k, v) in [
            ("chamfer_source", "Line"),
            ("from_x", "31.8"),
            ("to_x", "39.5"),
            ("line_z", "-0.35"),
            ("chamfer_side", "Alternate"),
            ("chamfer_size", "Tip depth"),
            ("tip_z", "-1.3"),
            ("copies", "60"),
            ("copy_pitch", "6"),
            ("first_angle", "3"),
        ] {
            assert_eq!(get(&o.p, k), v, "{k}");
        }
        // What is refused, and why.
        let refused = |change: &dyn Fn(&mut proposal::ProposedOp), says: &str| {
            let mut p = proposal::tests::good(&report);
            let mut o = deburr.clone();
            change(&mut o);
            p.setups[0].operations.push(o);
            let problems = proposal::build(&doc, &mesh, &report, &p).unwrap_err();
            assert!(
                problems.iter().any(|e| e.message.contains(says)),
                "{says}: {problems:?}"
            );
        };
        refused(&|o| o.chamfer = None, "needs its `chamfer` settings");
        refused(
            &|o| o.chamfer.as_mut().unwrap().tip_z = Some(-1.),
            "either a positive `width`",
        );
        refused(
            &|o| *o.chamfer.as_mut().unwrap() = teeth.chamfer.clone().unwrap(),
            "uses no surfaces",
        );
        refused(&|o| o.tool = 1, "which Chamfer cannot use");
        refused(
            &|o| o.strategy = "Drill".into(),
            "only for the Chamfer strategy",
        );
        // The schema offers it, its settings required on every operation.
        let schema = proposal::schema();
        let ops = &schema["properties"]["setups"]["items"]["properties"]["operations"]["items"];
        assert!(
            ops["properties"]["strategy"]["enum"]
                .as_array()
                .unwrap()
                .contains(&"Chamfer".into())
        );
        assert!(
            ops["required"]
                .as_array()
                .unwrap()
                .contains(&"chamfer".into())
        );
        let fields = &ops["properties"]["chamfer"]["anyOf"][0]["properties"];
        for k in [
            "source",
            "edges",
            "line",
            "line_z",
            "side",
            "width",
            "tip_offset",
            "tip_z",
            "copies",
            "copy_pitch",
            "first_angle",
            "axis",
        ] {
            assert!(fields.get(k).is_some(), "{k}");
        }
    }
    #[test]
    fn a_chamfer_milling_posts_for_linuxcnc_and_runs_on_the_uc100() {
        let (mut doc, mesh) = crate::post::fixture::job();
        let mut spot = mill("90", "0");
        spot.p.insert("number".into(), "9".into());
        let op = chamfer(
            &spot,
            &[
                ("chamfer_source", "Line"),
                ("from_x", "-5"),
                ("to_x", "5"),
                ("line_z", "0"),
                ("chamfer_size", "Tip depth"),
                ("tip_z", "-1"),
            ],
        );
        doc.tools.push(spot);
        doc.setups[0].operations = vec![op];
        // LinuxCNC: down to the tip, then the level pass 1 mm off the line.
        let job = crate::post::post_job(&doc, &mesh);
        assert!(job.errors.is_empty(), "{:?}", job.errors);
        let programs = job.setups[0].result.as_ref().unwrap();
        let text = &programs[0].text;
        assert!(text.contains("G1 Z-1.000"), "{text}");
        assert!(
            text.contains("X-5.000 Y1.000\nZ5.000\nG1 Z-1.000"),
            "{text}"
        );
        assert!(text.contains("G1 Z-1.000 F75.0\nX5.000 F300.0"), "{text}");
        // The UC100: its run checks pass and the tool run has the pass.
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("template.toml");
        std::fs::write(&profile, uc100::config::TEMPLATE).unwrap();
        // The template profile has no coolant outputs.
        doc.setups[0].operations[0]
            .p
            .insert("coolant".into(), "Off".into());
        let m = &mut doc.setups[0].machine;
        m.p.insert("controller".into(), "UC100".into());
        m.p.insert("uc100_profile".into(), profile.display().to_string());
        let job = crate::post::post_job(&doc, &mesh);
        assert!(
            matches!(job.setups[0].run, Some(Ok(()))),
            "{:?}",
            job.setups[0].run
        );
        let runs = crate::run::programs(&doc, &mesh, 0).unwrap();
        let level = (runs.iter().flat_map(|r| &r.blocks)).any(|b| {
            matches!(b, crate::post::Block::Move { kind: crate::post::Kind::Feed, from, to, .. }
                if (from[2] - to[2]).abs() < 1e-6 && (to[0] - from[0]).abs() > 9.99)
        });
        assert!(level);
    }
    #[test]
    fn a_held_chamfer_holds_its_paths_force_and_plunges_no_faster() {
        let (mut doc, mesh) = crate::post::fixture::job();
        let spot = mill("90", "0");
        let mut op = chamfer(
            &spot,
            &[
                ("chamfer_source", "Line"),
                ("from_x", "-5"),
                ("to_x", "5"),
                ("line_z", "0"),
                ("chamfer_size", "Tip depth"),
                ("tip_z", "-1"),
            ],
        );
        op.p.insert(
            "feed_follows".into(),
            crate::tooth_load::FEED_FOLLOWS[1].into(),
        );
        doc.tools.push(spot);
        doc.setups[0].operations = vec![op];
        let mut sims = vec![];
        crate::simulation::sequence(&doc, &mesh, None, |_, _, r| sims.push(r.unwrap()));
        let h = sims[0].held.expect("held");
        assert!(h.target > 0., "{h:?}");
        // The plunge to the tip, at F75, is never fed faster; the level
        // pass takes about the force held.
        for m in sims[0].plan.moves.iter().filter(|m| m.cutting_pass) {
            if m.to[2] < m.from[2] - 1e-4 {
                assert!(m.feed <= 75., "{m:?}");
            }
        }
        let load = sims[0].tooth_load.as_ref().unwrap();
        assert!(load.peak > 0.5 * h.target && load.peak < 2. * h.target);
    }
}
