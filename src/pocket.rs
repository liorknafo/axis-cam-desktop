//! Bounded 2.5D pocket preview from imported planar floor triangles.
//! Offset contours preserve holes/islands. No machine postprocessor or fixture solver.
use crate::{
    mesh::Mesh,
    model::{
        Operation, POCKET_OPENINGS, Setup, Strategy, Tool, facing_setting, get, num,
        pocket_setting, pocket_setting_issues,
    },
    simulation::{Motion, Move, Plan},
    stock::{self, Placement},
    surface,
};
use clipper2::{EndType, FillRule, JoinType, Milli, Paths, difference, intersect, union};

type Polygon = Vec<(f64, f64)>;
type Region = Paths<Milli>;
const MAX_MOVES: usize = 30_000;
/// How far above the level before, mm, a loop's rapid descent stops below
/// the first level.
const ENTRY: f32 = 0.5;
/// The most times a profile ramp goes round its loop; a shorter loop plunges.
const RAMP_LAPS: f32 = 10.;
/// `RAMP_LAPS` in what earlier cuts left.
const REST_RAMP_LAPS: f32 = 2.;
fn area(p: &Polygon) -> f64 {
    p.iter()
        .zip(p.iter().cycle().skip(1))
        .take(p.len())
        .map(|(a, b)| a.0 * b.1 - b.0 * a.1)
        .sum::<f64>()
        / 2.
}
fn region(triangles: Vec<Polygon>) -> Result<Region, String> {
    union::<Milli>(triangles, Vec::<Polygon>::new(), FillRule::NonZero)
        .map_err(|e| format!("Could not join pocket floor triangles: {e}"))
}
fn number(op: &Operation, key: &str) -> Result<f32, String> {
    num(&op.p, key).ok_or_else(|| format!("Enter a finite {} value.", key.replace('_', " ")))
}
fn positive(tool: &Tool, key: &str) -> Result<f32, String> {
    num(&tool.p, key)
        .filter(|v| *v > 0.)
        .ok_or_else(|| format!("Enter a positive tool {key}."))
}
/// Clip a triangle to the half-space above the floor before projecting it.
fn above(points: &[[f32; 3]], z: f32) -> Polygon {
    let mut result = Vec::new();
    for (a, b) in points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .take(points.len())
    {
        if a[2] >= z {
            result.push((a[0] as f64, a[1] as f64));
        }
        if (a[2] >= z) != (b[2] >= z) {
            let t = (z - a[2]) / (b[2] - a[2]);
            result.push((
                (a[0] + t * (b[0] - a[0])) as f64,
                (a[1] + t * (b[1] - a[1])) as f64,
            ));
        }
    }
    result
}
fn push(
    plan: &mut Plan,
    position: &mut [f32; 3],
    to: [f32; 3],
    motion: Motion,
    rate: f32,
    cutting: bool,
) -> Result<(), String> {
    if plan.moves.len() >= MAX_MOVES {
        return Err(
            "Pocket preview exceeds 30,000 moves. Increase stepover, stepdown or ramp angle."
                .into(),
        );
    }
    let distance = (0..3)
        .map(|i| (position[i] - to[i]).powi(2))
        .sum::<f32>()
        .sqrt();
    if distance > 0.00001 {
        let end = plan.duration + distance / rate * 60.;
        plan.moves.push(Move {
            from: *position,
            to,
            motion,
            cutting_pass: cutting,
            feed: rate,
            start: plan.duration,
            end,
        });
        plan.duration = end;
    }
    *position = to;
    Ok(())
}

/// How far a wall may lean over the floor below it, mm, before the floor
/// counts as overhung: the chord deviation of the STEP reader's tessellation,
/// by which a curved vertical wall's triangles stand off it.
const LEAN: f64 = 0.05;
/// Material earlier cuts left in a band thinner than twice this, mm, is not
/// cut again: two cutters' margins along a wall, and the outlines' chords,
/// differ by about this much.
const SLIVER: f64 = 0.015;
/// A selected pocket floor, in its setup's frame (mm).
pub struct Floor {
    pub plane: f32,
    /// The selected faces, joined.
    pub area: Region,
    /// Everything of the part above the floor, seen from above: its walls
    /// and islands.
    pub above: Region,
    /// Holes in the floor with nothing of the part over or under them (a
    /// slot or window through the part), each with the height its walls
    /// reach down to.
    pub openings: Vec<(Region, f32)>,
}
/// The selected floor `ids` (mesh surfaces) as `setup` places the part.
pub fn floor(
    setup: &Setup,
    ids: &std::collections::HashSet<usize>,
    mesh: &Mesh,
) -> Result<Floor, String> {
    let placement = Placement::new(mesh, Some(setup));
    let scale = mesh.display_scale();
    let reference = mesh
        .faces
        .iter()
        .find(|f| f.feature.is_some_and(|id| ids.contains(&id)))
        .and_then(|f| f.points.first())
        .ok_or("The selected floor contains no triangles.")?;
    let plane = placement.transform(*reference)[2] / scale;
    let mut floors = Vec::new();
    let mut obstacles = Vec::new();
    // Every face as placed, with its outline seen from above (None for a
    // wall, which has no area there).
    let mut faces = Vec::with_capacity(mesh.faces.len());
    for face in &mesh.faces {
        let points: Vec<_> = face
            .points
            .iter()
            .map(|p| placement.transform(*p).map(|v| v / scale))
            .collect();
        let selected = face.feature.is_some_and(|id| ids.contains(&id));
        if selected && points.iter().any(|p| (p[2] - plane).abs() > 0.002) {
            return Err("Pocket preview needs a flat floor perpendicular to setup Z. Reposition the part or select another floor.".into());
        }
        let mut p: Polygon = if selected {
            points.iter().map(|p| (p[0] as f64, p[1] as f64)).collect()
        } else {
            above(&points, plane + 0.01)
        };
        let a = area(&p);
        if a < 0. {
            p.reverse();
        }
        if a.abs() >= 0.000001 {
            if selected {
                floors.push(p);
            } else {
                obstacles.push(p);
            }
        }
        faces.push(points);
    }
    let area = region(floors)?;
    if !area.contains_points() {
        return Err("The selected floor has no usable XY area.".into());
    }
    let above = region(obstacles)?;
    // Reject overhanging/tapered geometry rather than assuming vertical
    // access; a wall's tessellation may lean over the floor by `LEAN`.
    let blocked = intersect::<Milli>(
        area.clone(),
        above.inflate(-LEAN, JoinType::Round, EndType::Polygon, 2.),
        FillRule::NonZero,
    )
    .map_err(|e| e.to_string())?;
    if blocked.signed_area().abs() > 0.05 {
        return Err("Geometry above this floor obstructs vertical access. Pocket preview supports open, vertical-wall 2.5D pockets; choose another floor or setup.".into());
    }
    let mut openings = vec![];
    let loops: Vec<Polygon> = area.clone().into();
    for mut hole in loops.into_iter().filter(|p| area_of(p) < 0.) {
        hole.reverse();
        let (low, high) = extent(&hole);
        // A face whose box overlaps the hole's, even with every corner
        // outside it (a large face under the hole).
        let near = |points: &[[f32; 3]]| {
            let x = points.iter().map(|p| p[0] as f64);
            let y = points.iter().map(|p| p[1] as f64);
            x.clone().fold(f64::INFINITY, f64::min) <= high.0 + LEAN
                && x.fold(f64::NEG_INFINITY, f64::max) >= low.0 - LEAN
                && y.clone().fold(f64::INFINITY, f64::min) <= high.1 + LEAN
                && y.fold(f64::NEG_INFINITY, f64::max) >= low.1 - LEAN
        };
        let mut covering = vec![];
        let mut bottom = f32::INFINITY;
        for points in &faces {
            if !near(points) {
                continue;
            }
            let mut p: Polygon = points.iter().map(|p| (p[0] as f64, p[1] as f64)).collect();
            let a = area_of(&p);
            if a.abs() >= 0.000001 {
                if a < 0. {
                    p.reverse();
                }
                covering.push(p);
            }
            // Its walls: faces with an edge on the hole's outline.
            let on: Vec<&[f32; 3]> = points
                .iter()
                .filter(|p| distance(&hole, (p[0] as f64, p[1] as f64)) <= LEAN)
                .collect();
            if on.len() >= 2 {
                bottom = on.iter().map(|p| p[2]).fold(bottom, f32::min);
            }
        }
        let inside = Paths::<Milli>::from(vec![hole.clone()]).inflate(
            -LEAN,
            JoinType::Round,
            EndType::Polygon,
            2.,
        );
        let covered = intersect::<Milli>(inside, region(covering)?, FillRule::NonZero)
            .map_err(|e| e.to_string())?;
        if bottom < plane - 0.01 && covered.signed_area().abs() <= 0.05 {
            openings.push((Paths::from(vec![hole]), bottom));
        }
    }
    Ok(Floor {
        plane,
        area,
        above,
        openings,
    })
}
impl Floor {
    /// Whether a flat cutter of `diameter` has somewhere to stand on this
    /// floor with no radial allowance, as `generate` places it.
    pub fn fits(&self, diameter: f32) -> bool {
        let radius = diameter / 2.;
        Centres::Floor {
            area: outline(&self.area),
            above: self.above.clone(),
            reach: radius as f64,
            keepout: (radius + (radius * 0.01).max(0.02)) as f64,
        }
        .at(0.)
        .is_ok_and(|c| c.contains_points())
    }
    /// The largest flat cutter `fits` allows, to 0.01 mm, up to `limit`.
    pub fn largest(&self, limit: f32) -> f32 {
        let (mut lo, mut hi) = (0f32, limit);
        if self.fits(hi) {
            return hi;
        }
        while hi - lo > 0.005 {
            let mid = (lo + hi) / 2.;
            if self.fits(mid) { lo = mid } else { hi = mid }
        }
        lo
    }
    /// Each opening's outline box (x0, y0, x1, y1) and the largest flat
    /// cutter that fits inside it.
    pub fn opening_sizes(&self) -> Vec<([f32; 4], f32)> {
        self.openings
            .iter()
            .map(|(hole, _)| {
                let loops: Vec<Polygon> = hole.clone().into();
                let (low, high) = extent(&loops[0]);
                let (mut lo, mut hi) = (0f64, ((high.0 - low.0).min(high.1 - low.1) / 2.).max(0.));
                while hi - lo > 0.0025 {
                    let mid = (lo + hi) / 2.;
                    if Centres::Region(hole.clone())
                        .at(mid + (mid * 0.01).max(0.02))
                        .is_ok_and(|c| c.contains_points())
                    {
                        lo = mid
                    } else {
                        hi = mid
                    }
                }
                (
                    [low.0 as f32, low.1 as f32, high.0 as f32, high.1 as f32],
                    (lo * 2.) as f32,
                )
            })
            .collect()
    }
}
fn area_of(p: &Polygon) -> f64 {
    area(p)
}
fn extent(p: &Polygon) -> ((f64, f64), (f64, f64)) {
    p.iter().fold(
        (
            (f64::INFINITY, f64::INFINITY),
            (f64::NEG_INFINITY, f64::NEG_INFINITY),
        ),
        |(lo, hi), q| {
            (
                (lo.0.min(q.0), lo.1.min(q.1)),
                (hi.0.max(q.0), hi.1.max(q.1)),
            )
        },
    )
}
/// Distance from `q` to the closed outline `p`.
fn distance(p: &Polygon, q: (f64, f64)) -> f64 {
    p.iter()
        .zip(p.iter().cycle().skip(1))
        .take(p.len())
        .map(|(a, b)| {
            let v = (b.0 - a.0, b.1 - a.1);
            let l2 = v.0 * v.0 + v.1 * v.1;
            let t = if l2 == 0. {
                0.
            } else {
                (((q.0 - a.0) * v.0 + (q.1 - a.1) * v.1) / l2).clamp(0., 1.)
            };
            ((q.0 - a.0 - t * v.0).powi(2) + (q.1 - a.1 - t * v.1).powi(2)).sqrt()
        })
        .fold(f64::INFINITY, f64::min)
}
/// `region` with its holes filled.
fn outline(region: &Region) -> Region {
    let loops: Vec<Polygon> = region.clone().into();
    Paths::from(
        loops
            .into_iter()
            .filter(|p| area(p) > 0.)
            .collect::<Vec<_>>(),
    )
}
/// The box the tool sweeps with its centre in `centres`, at every height.
fn envelope(centres: &Region, radius: f32) -> crate::mesh::Bounds {
    let mut b = crate::mesh::Bounds {
        min: [f32::INFINITY, f32::INFINITY, f32::NEG_INFINITY],
        max: [f32::NEG_INFINITY, f32::NEG_INFINITY, f32::INFINITY],
    };
    let loops: Vec<Polygon> = centres.clone().into();
    for point in loops.iter().flatten() {
        for (k, v) in [point.0 as f32, point.1 as f32].into_iter().enumerate() {
            b.min[k] = b.min[k].min(v - radius);
            b.max[k] = b.max[k].max(v + radius);
        }
    }
    b
}
/// Where a pocket's tool centre may go.
enum Centres {
    /// Within `reach` of `area` and `keepout` from `above`: offset by `d`,
    /// `area` grown by `reach - d` less `above` grown by `keepout + d`, each
    /// ring built from the outlines themselves as a single offset would be.
    Floor {
        area: Region,
        above: Region,
        reach: f64,
        keepout: f64,
    },
    Region(Region),
}
impl Centres {
    /// The centres at least `d` inside these.
    fn at(&self, d: f64) -> Result<Region, String> {
        let grown = |r: &Region, by: f64| r.inflate(by, JoinType::Round, EndType::Polygon, 2.);
        Ok(match self {
            Centres::Floor {
                area,
                above,
                reach,
                keepout,
            } => difference::<Milli>(
                grown(area, reach - d),
                grown(above, keepout + d),
                FillRule::NonZero,
            )
            .map_err(|e| e.to_string())?,
            // A rest region's small loops are mostly arcs: kept to 5 µm.
            Centres::Region(r) => grown(r, -d).simplify(0.005, false),
        }
        .simplify(0.001, false))
    }
}
/// `region`'s separate pieces: each outline with the holes inside it.
fn components(region: &Region) -> Vec<Region> {
    let loops: Vec<Polygon> = region.clone().into();
    let (outer, holes): (Vec<Polygon>, Vec<Polygon>) =
        loops.into_iter().partition(|p| area(p) > 0.);
    let mut pieces: Vec<Vec<Polygon>> = outer.into_iter().map(|p| vec![p]).collect();
    for hole in holes {
        // The smallest outline holding the hole's first point.
        if let Some(piece) = pieces
            .iter_mut()
            .filter(|piece| inside(&piece[0], hole[0]))
            .min_by(|a, b| area(&a[0]).total_cmp(&area(&b[0])))
        {
            piece.push(hole);
        }
    }
    pieces.into_iter().map(Paths::from).collect()
}
/// Whether `q` is inside the closed outline `p` (even-odd).
fn inside(p: &Polygon, q: (f64, f64)) -> bool {
    let mut odd = false;
    for (a, b) in p.iter().zip(p.iter().cycle().skip(1)).take(p.len()) {
        if (a.1 > q.1) != (b.1 > q.1) && q.0 < a.0 + (q.1 - a.1) * (b.0 - a.0) / (b.1 - a.1) {
            odd = !odd;
        }
    }
    odd
}
/// One region a pocket clears: where the tool centre may go, from the
/// height its material starts at down to its target.
struct Zone {
    centres: Centres,
    top: f32,
    target: f32,
}
/// The part of `centres` within the tool's reach of material that earlier
/// cuts did not provably remove between `low` and `high`: the tool goes
/// only where something is left, as a rest pass. Bands thinner than twice
/// `SLIVER` count as removed.
fn remaining(
    centres: &Region,
    radius: f32,
    rest: &crate::rest::RestStock,
    low: f32,
    high: f32,
) -> Result<Region, String> {
    let reach = centres.inflate(radius as f64, JoinType::Round, EndType::Polygon, 2.);
    let near = envelope(centres, radius);
    let gone = rest.cleared(near, low, high);
    let stock = rest.extent();
    let inside: Polygon = vec![
        (stock.min[0] as f64, stock.min[1] as f64),
        (stock.max[0] as f64, stock.min[1] as f64),
        (stock.max[0] as f64, stock.max[1] as f64),
        (stock.min[0] as f64, stock.max[1] as f64),
    ];
    let reach =
        intersect::<Milli>(reach, vec![inside], FillRule::NonZero).map_err(|e| e.to_string())?;
    let left = if gone.is_empty() {
        reach
    } else {
        difference::<Milli>(reach, region(gone)?, FillRule::NonZero).map_err(|e| e.to_string())?
    };
    let left = left
        .inflate(-SLIVER, JoinType::Round, EndType::Polygon, 2.)
        .inflate(SLIVER, JoinType::Round, EndType::Polygon, 2.);
    if !left.contains_points() {
        return Ok(Paths::new(vec![]));
    }
    intersect::<Milli>(
        centres.clone(),
        left.inflate(radius as f64, JoinType::Round, EndType::Polygon, 2.),
        FillRule::NonZero,
    )
    .map(|r| r.simplify(0.001, false))
    .map_err(|e| e.to_string())
}

pub fn generate(setup: &Setup, op: &Operation, tool: &Tool, mesh: &Mesh) -> Result<Plan, String> {
    if !op.enabled {
        return Err("Enable this Pocket operation before simulation.".into());
    }
    if let Some(issue) = pocket_setting_issues(&op.p).first() {
        return Err(issue.clone());
    }
    if let Some(issue) = surface::issues(op, setup, Some(mesh)).first() {
        return Err(issue.clone());
    }
    let selection = op
        .surfaces
        .as_ref()
        .filter(|s| !s.ids.is_empty())
        .ok_or("Select a recessed planar pocket floor in Geometry first.")?;
    if get(&tool.p, "type") != "Flat end mill" {
        return Err("Pocket preview requires a flat end mill.".into());
    }
    if get(&setup.p, "stock_source") != "fresh" {
        return Err("Pocket preview currently uses fresh stock. Stock transfer between setups is not implemented.".into());
    }
    if get(&op.p, "boundary") != "Selected geometry"
        || get(&op.p, "containment") != "Tool inside boundary"
    {
        return Err("Pocket preview requires Selected geometry with Tool inside boundary. Its walls and islands come from the selected floor.".into());
    }
    let stock = stock::resolve(setup, Some(mesh))?;
    let placement = Placement::new(mesh, Some(setup));
    let scale = stock.display_scale;
    let ids: std::collections::HashSet<_> = selection.ids.iter().copied().collect();
    // The selected faces by height, highest first: each floor is cleared to
    // its own height, as one pocket.
    let mut heights: Vec<(f32, usize)> = ids
        .iter()
        .filter_map(|&id| {
            let s = mesh.surfaces.get(id)?;
            Some((placement.transform(s.center)[2] / scale, id))
        })
        .collect();
    heights.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    let mut groups: Vec<std::collections::HashSet<usize>> = vec![];
    let mut last = f32::INFINITY;
    for (z, id) in heights {
        if groups.is_empty() || last - z > 0.01 {
            groups.push(Default::default());
            last = z;
        }
        groups.last_mut().unwrap().insert(id);
    }
    let floors = groups
        .iter()
        .map(|g| floor(setup, g, mesh))
        .collect::<Result<Vec<_>, _>>()?;
    if floors.is_empty() {
        return Err("The selected floor contains no triangles.".into());
    }
    if floors
        .iter()
        .any(|f| f.plane >= placement.bounds.max[2] / scale - 0.001)
    {
        return Err("Select a recessed pocket floor. Use Facing for the top surface.".into());
    }
    let axial = number(op, "axial_allowance")?;
    let radial = number(op, "allowance")?;
    let bottom = number(op, "bottom")?;
    let top = stock.stock.max[2].max(stock.stock.max[2] + number(op, "top")?);
    if axial < 0. || radial < 0. || bottom + axial < -0.0001 {
        return Err("Pocket target cannot cut below the selected floor; use nonnegative stock allowances and review Bottom offset.".into());
    }
    if floors.iter().any(|f| {
        let target = f.plane + bottom + axial;
        target < stock.stock.min[2] || target >= stock.stock.max[2] - 0.0001
    }) {
        return Err("No pocket stock exists above the requested target. Review stock and bottom/axial offsets.".into());
    }
    let radius = positive(tool, "diameter")? / 2.;
    let flute_length = positive(tool, "length")?;
    let tool_length = positive(tool, "stickout")?;
    let holder_radius = positive(tool, "holder")? / 2.;
    let feed = number(op, "feed")?;
    let stepdown = number(op, "stepdown")?;
    let stepover = number(op, "stepover")? / 100. * radius * 2.;
    if feed <= 0.
        || stepdown <= 0.
        || stepdown > flute_length
        || stepover <= 0.
        || stepover > radius
    {
        return Err("Use positive feed/step sizes, stepdown within flute length, and Pocket stepover up to 50% for overlapping offset contours.".into());
    }
    let clearance = top + number(op, "clearance")?;
    let retract = top + number(op, "retract")?;
    if retract <= top || clearance < retract {
        return Err("Set positive retract and clearance at or above retract.".into());
    }
    let finish = number(op, "finish_passes")?;
    if !(0. ..=20.).contains(&finish) || finish.fract() != 0. {
        return Err("Finishing passes must be a whole number from 0 to 20.".into());
    }
    let ramp = pocket_setting(&op.p, "pocket_entry") == "Profile ramp";
    let ramp_slope = if ramp {
        let angle = number(op, "ramp_angle")?;
        if !(0.1..=15.).contains(&angle) {
            return Err("Pocket profile ramp angle must be between 0.1° and 15°.".into());
        }
        angle.to_radians().tan()
    } else {
        0.
    };
    let profile_only = op.strategy == Strategy::Contour;
    // A small conservative margin covers integer rounding and offset arc chords.
    let margin = (radius * 0.01).max(0.02);
    let inset = (radius + radial + margin) as f64;
    let through = !profile_only && pocket_setting(&op.p, "openings") == POCKET_OPENINGS[1];
    let breakthrough = if through {
        number(op, "breakthrough")?
    } else {
        0.
    };
    if breakthrough < 0. {
        return Err("Breakthrough cannot be negative.".into());
    }
    if through && floors.iter().all(|f| f.openings.is_empty()) {
        return Err("The selected floor has no openings that go through the part. Set Openings to Leave, or select the floor around the openings.".into());
    }
    let several = floors.len() > 1;
    let mut zones = vec![];
    let mut openings = vec![];
    // Where each floor zone's tool reaches, with its target: a deeper floor
    // inside a shallower one starts where that one stops.
    let mut cleared: Vec<(Region, f32)> = vec![];
    for floor in floors {
        let target = floor.plane + bottom + axial;
        // The tool centre keeps its radius, the radial allowance and the
        // margin from everything above the floor, and may stand anywhere
        // within its radius of the floor: over a hole in the floor, and past
        // an edge where the part falls away below it, as over a ledge.
        let centres = Centres::Floor {
            area: outline(&floor.area),
            above: floor.above,
            reach: radius as f64,
            keepout: inset,
        };
        let ring = centres.at(0.)?;
        if !ring.contains_points() {
            // Of several floors, one too narrow for this tool is left to a
            // smaller one.
            if several {
                continue;
            }
            return Err("The selected pocket is too narrow for this tool and radial allowance. Choose a smaller cutter.".into());
        }
        let reach = ring.inflate(radius as f64, JoinType::Round, EndType::Polygon, 2.);
        let mut start = top;
        for (above, height) in &cleared {
            let outside = difference::<Milli>(reach.clone(), above.clone(), FillRule::NonZero)
                .map_err(|e| e.to_string())?;
            if outside.signed_area().abs() < 0.01 {
                start = start.min(*height);
            }
        }
        cleared.push((reach, target));
        zones.push(Zone {
            centres,
            top: start,
            target,
        });
        if through {
            for (hole, bottom) in &floor.openings {
                let centres = Centres::Region(hole.clone());
                if centres.at(inset)?.contains_points() {
                    openings.push(Zone {
                        centres: Centres::Region(centres.at(inset)?),
                        top: target,
                        target: (bottom - breakthrough).max(stock.stock.min[2] - breakthrough),
                    });
                }
            }
        }
    }
    if zones.is_empty() {
        return Err("Every selected floor is too narrow for this tool and radial allowance. Choose a smaller cutter.".into());
    }
    if through && openings.is_empty() {
        return Err("Every opening in the selected floor is too narrow for this tool and radial allowance. Choose a smaller cutter.".into());
    }
    zones.extend(openings);
    let deepest = zones.iter().map(|z| z.target).fold(f32::INFINITY, f32::min);
    if tool_length < flute_length || top - deepest >= tool_length {
        return Err("Pocket depth must be less than tool stickout, with stickout at least the flute length. Choose a tool with adequate reach.".into());
    }
    let offset = number(op, "top")?;
    let rest = setup.preview_remaining.as_deref();
    // Each zone's offset loops, outermost first, and its first cutting height.
    let start = |zone: &Zone, centres: &Region| {
        crate::rest::start(
            setup,
            envelope(centres, radius),
            zone.target,
            zone.top,
            if zone.top == top { offset } else { 0. },
        )
    };
    // Each zone only where earlier cuts left material between its heights;
    // where they left none anywhere, the whole pocket, as without them.
    let mut left = vec![];
    if let Some(rest) = rest.filter(|_| !profile_only) {
        for zone in &zones {
            let centres = zone.centres.at(0.)?;
            let full = centres.signed_area();
            let region = remaining(&centres, radius, rest, zone.target, start(zone, &centres))?;
            // Offsetting rounds off a few hundredths of a mm²: only a real
            // reduction makes the zone what earlier cuts left.
            left.push((region.signed_area() < full * 0.999 - 0.01).then_some(region));
        }
        if left
            .iter()
            .all(|l| l.as_ref().is_some_and(|r| !r.contains_points()))
        {
            left.clear();
        }
    }
    // A rest zone's separate pieces (corners, slots) each start where
    // their own material does.
    let mut pieces = vec![];
    for (i, zone) in zones.into_iter().enumerate() {
        match left.get_mut(i) {
            Some(Some(region)) => {
                for piece in components(region) {
                    let piece = Zone {
                        centres: Centres::Region(piece),
                        ..zone
                    };
                    pieces.push((piece, true));
                }
            }
            _ => pieces.push((zone, false)),
        }
    }
    // Each zone's offset loops, its first cutting height, its target and
    // whether it is what earlier cuts left.
    let mut planned: Vec<(Vec<Vec<Polygon>>, f32, f32, bool)> = vec![];
    for (zone, trimmed) in pieces {
        let cutting_top = start(&zone, &zone.centres.at(0.)?);
        let mut rings: Vec<Vec<Polygon>> = Vec::new();
        for ring in 0..=if profile_only { 0 } else { 256 } {
            let paths = zone.centres.at(ring as f64 * stepover as f64)?;
            if !paths.contains_points() {
                break;
            }
            if ring == 256 {
                return Err(
                    "Pocket preview exceeds 256 offset rings. Increase stepover or reduce pocket size."
                        .into(),
                );
            }
            rings.push(paths.into());
        }
        if !rings.is_empty() {
            planned.push((rings, cutting_top, zone.target, trimmed));
        }
    }
    if planned.is_empty() {
        return Err("No finite pocket motion could be generated.".into());
    }
    let mut segments = 0usize;
    let mut total_levels = 0usize;
    for (rings, cutting_top, target, _) in &planned {
        let levels = ((cutting_top - target - 0.0001) / stepdown).ceil().max(1.) as usize;
        total_levels += levels;
        let loops: usize = rings.iter().flatten().map(|p| p.len() + 5).sum::<usize>()
            + rings[0].iter().map(|p| p.len() + 5).sum::<usize>() * finish as usize;
        segments = segments.saturating_add(loops.saturating_mul(levels));
    }
    if total_levels > 1000 || segments > MAX_MOVES {
        return Err("Pocket preview exceeds its move limit. Increase stepdown or stepover.".into());
    }
    let clockwise = facing_setting(&op.p, "spindle_rotation") == "Clockwise (CW)";
    let climb = pocket_setting(&op.p, "pocket_milling") == "Climb";
    let highest = planned
        .iter()
        .map(|p| p.1)
        .fold(f32::NEG_INFINITY, f32::max);
    let mut plan = Plan {
        shape: crate::simulation::CutterShape::Flat,
        strategy: op.strategy,
        operation: op.id.clone(),
        setup: setup.id.clone(),
        moves: vec![],
        stock: stock.stock,
        radius,
        flute_length,
        tool_length,
        holder_radius,
        display_scale: scale,
        duration: 0.,
        target: planned.iter().map(|p| p.2).fold(f32::INFINITY, f32::min),
        levels: total_levels,
        rest_top: (highest < top - 0.0001).then_some(highest),
        spindle_clockwise: clockwise,
        milling_direction: if climb { "Climb" } else { "Conventional" },
    };
    let first = planned[0].0[0][0][0];
    let mut position = [first.0 as f32, first.1 as f32, clearance];
    for (mut rings, cutting_top, target, trimmed) in planned {
        // Clipper outer boundaries are CCW and islands CW (free area on left).
        // CW climb keeps remaining material on the right of cutter travel.
        if clockwise != climb {
            for p in rings.iter_mut().flatten() {
                p.reverse();
            }
        }
        let finishing = rings[0].clone();
        let loops: Vec<_> = rings
            .into_iter()
            .rev()
            .flatten()
            .chain((0..finish as usize).flat_map(|_| finishing.clone()))
            .collect();
        // Every zone comes down where the highest begins: an opening's
        // column was cleared by this pocket's floor zone, which the export's
        // check of rapids cannot see.
        let approach = (highest + number(op, "retract")?).min(retract);
        let levels = ((cutting_top - target - 0.0001) / stepdown).ceil().max(1.) as usize;
        for level in 1..=levels {
            let z = (cutting_top - level as f32 * stepdown).max(target);
            let previous_z = (cutting_top - (level - 1) as f32 * stepdown).max(target);
            // Below the first level each loop comes down to just over the
            // level before, where the tool has already been at this point.
            let entry = if level == 1 {
                approach
            } else {
                (previous_z + ENTRY).min(approach)
            };
            for path in &loops {
                let start = [path[0].0 as f32, path[0].1 as f32];
                push(
                    &mut plan,
                    &mut position,
                    [start[0], start[1], clearance],
                    Motion::Rapid,
                    6000.,
                    false,
                )?;
                push(
                    &mut plan,
                    &mut position,
                    [start[0], start[1], entry],
                    Motion::Rapid,
                    6000.,
                    false,
                )?;
                // A ramp that would wind round its loop more than
                // `RAMP_LAPS` times plunges instead: a loop that small is a
                // narrow strip or corner. In what earlier cuts left (a rest
                // pass's corners) little of the cutter meets material, and
                // more than `REST_RAMP_LAPS` laps plunge there.
                let length: f32 = path
                    .iter()
                    .zip(path.iter().cycle().skip(1))
                    .map(|(a, b)| ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt() as f32)
                    .sum();
                let laps = (previous_z - z) / (length * ramp_slope).max(1e-6);
                let ramp = ramp && laps <= if trimmed { REST_RAMP_LAPS } else { RAMP_LAPS };
                push(
                    &mut plan,
                    &mut position,
                    [start[0], start[1], if ramp { previous_z } else { z }],
                    Motion::Feed,
                    feed * 0.25,
                    false,
                )?;
                if ramp {
                    while position[2] > z + 0.00001 {
                        for point in path.iter().skip(1).chain(std::iter::once(&path[0])) {
                            let xy = [point.0 as f32, point.1 as f32];
                            let distance = ((xy[0] - position[0]).powi(2)
                                + (xy[1] - position[1]).powi(2))
                            .sqrt();
                            let next_z = (position[2] - distance * ramp_slope).max(z);
                            push(
                                &mut plan,
                                &mut position,
                                [xy[0], xy[1], next_z],
                                Motion::Feed,
                                feed * 0.25,
                                true,
                            )?;
                        }
                    }
                }
                for point in path.iter().skip(1).chain(std::iter::once(&path[0])) {
                    push(
                        &mut plan,
                        &mut position,
                        [point.0 as f32, point.1 as f32, z],
                        Motion::Feed,
                        feed,
                        true,
                    )?;
                }
                push(
                    &mut plan,
                    &mut position,
                    [start[0], start[1], clearance],
                    Motion::Rapid,
                    6000.,
                    false,
                )?;
            }
        }
    }
    if plan.moves.is_empty() || !plan.duration.is_finite() {
        return Err("No finite pocket motion could be generated.".into());
    }
    Ok(plan)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{mesh, model::Machine, simulation::Simulation};
    fn quad(tris: &mut Vec<[[f32; 3]; 3]>, x: f32, y: f32, xx: f32, yy: f32, z: f32) {
        tris.push([[x, y, z], [xx, y, z], [xx, yy, z]]);
        tris.push([[x, y, z], [xx, yy, z], [x, yy, z]]);
    }
    fn fixture() -> (Setup, Operation, Tool, Mesh) {
        let mut tris = Vec::new();
        // Floor ring, central island, enclosing rim and lower model bound.
        for (x, y, xx, yy) in [
            (-10., -8., 10., -2.),
            (-10., 2., 10., 8.),
            (-10., -2., -2., 2.),
            (2., -2., 10., 2.),
        ] {
            quad(&mut tris, x, y, xx, yy, 0.);
        }
        for (x, y, xx, yy) in [
            (-15., -12., 15., -8.),
            (-15., 8., 15., 12.),
            (-15., -8., -10., 8.),
            (10., -8., 15., 8.),
            (-2., -2., 2., 2.),
        ] {
            quad(&mut tris, x, y, xx, yy, 6.);
        }
        quad(&mut tris, -15., -12., 15., 12., -4.);
        let mesh = mesh::normalize(tris).unwrap();
        let mut setup = Setup::new(Machine::new(3), "G54");
        setup
            .p
            .insert("stock_type".into(), "Relative size box".into());
        for (k, _) in stock::OFFSETS {
            setup.p.insert(k.into(), "0".into());
        }
        let mut tool = Tool::new("Pocket test cutter", "Flat end mill", "2");
        tool.p.insert("diameter".into(), "2".into());
        let mut op = Operation::new(Strategy::Pocket, tool.id.clone());
        op.geometry.clear();
        let center_z = mesh.surfaces[0].center[2];
        op.surfaces = Some(surface::Selection {
            mesh_fingerprint: mesh.fingerprint.clone(),
            ids: mesh
                .surfaces
                .iter()
                .enumerate()
                .filter(|(_, s)| (s.center[2] - center_z).abs() < 0.001)
                .map(|(i, _)| i)
                .collect(),
        });
        for (k, v) in [
            ("allowance", "0"),
            ("axial_allowance", "0"),
            ("stepover", "40"),
            ("pocket_entry", "Plunge"),
            ("finish_passes", "0"),
        ] {
            op.p.insert(k.into(), v.into());
        }
        (setup, op, tool, mesh)
    }
    fn loops(plan: &Plan) -> Vec<Polygon> {
        let mut loops = Vec::new();
        let mut current = Vec::new();
        for m in &plan.moves {
            if m.cutting_pass {
                if current.is_empty() {
                    current.push((m.from[0] as f64, m.from[1] as f64));
                }
                current.push((m.to[0] as f64, m.to[1] as f64));
            } else if !current.is_empty() {
                loops.push(std::mem::take(&mut current));
            }
        }
        loops
    }
    #[test]
    fn inside_contour_follows_walls_without_clearing_the_pocket_interior() {
        let (setup, mut op, tool, mesh) = fixture();
        let pocket = generate(&setup, &op, &tool, &mesh).unwrap();
        op.strategy = Strategy::Contour;
        op.p.insert("side".into(), "Inside".into());
        let plan = crate::simulation::generate(&setup, &op, &tool, &mesh).unwrap();
        assert_eq!(plan.strategy, Strategy::Contour);
        assert!(plan.moves.len() < pocket.moves.len());
        assert_eq!(plan.target, pocket.target);
        let mut contour_sim = Simulation::new(plan.clone());
        contour_sim.seek(1.);
        let mut pocket_sim = Simulation::new(pocket);
        pocket_sim.seek(1.);
        assert!(contour_sim.stock.removed_volume() < pocket_sim.stock.removed_volume());
        let paths = loops(&plan);
        assert!(paths.iter().any(|p| area(p) > 0.) && paths.iter().any(|p| area(p) < 0.));
        op.p.insert("pocket_milling".into(), "Conventional".into());
        let reversed = crate::simulation::generate(&setup, &op, &tool, &mesh).unwrap();
        for (a, b) in paths.iter().zip(loops(&reversed)) {
            assert!((area(a) + area(&b)).abs() < 0.001);
        }
        op.surfaces = None;
        assert!(crate::simulation::generate(&setup, &op, &tool, &mesh).is_err());
    }
    #[test]
    fn pocket_preserves_walls_island_and_floor_and_removes_known_volume() {
        let (setup, op, tool, mesh) = fixture();
        let plan = generate(&setup, &op, &tool, &mesh).unwrap();
        assert_eq!(plan.levels, 3);
        for pair in plan.moves.windows(2) {
            assert_eq!(pair[0].to, pair[1].from);
            assert_eq!(pair[0].end, pair[1].start);
        }
        for m in &plan.moves {
            if m.motion == Motion::Rapid && m.from[..2] != m.to[..2] {
                assert!(m.from[2] > plan.stock.max[2] && m.to[2] > plan.stock.max[2]);
            }
            if m.motion == Motion::Feed {
                for i in 0..=10 {
                    let p = m.at(m.start + (m.end - m.start) * i as f32 / 10.);
                    assert!(
                        p[0].abs() + plan.radius <= 10. + 0.001
                            && p[1].abs() + plan.radius <= 8. + 0.001
                    );
                    let dx = (p[0].abs() - 2.).max(0.);
                    let dy = (p[1].abs() - 2.).max(0.);
                    assert!(dx * dx + dy * dy >= plan.radius * plan.radius);
                    assert!(p[2] >= plan.target - 0.001);
                }
            }
        }
        let mut sim = Simulation::new(plan);
        sim.seek(1.);
        let expected = (20. * 16. - 4. * 4.) * 6.;
        assert!(
            (sim.stock.removed_volume() - expected).abs() / expected < 0.04,
            "{} vs {expected}",
            sim.stock.removed_volume()
        );
        for y in 0..sim.stock.ny {
            for x in 0..sim.stock.nx {
                let px = sim.stock.bounds.min[0] + (x as f32 + 0.5) * sim.stock.dx;
                let py = sim.stock.bounds.min[1] + (y as f32 + 0.5) * sim.stock.dy;
                if px.abs() > 10. || py.abs() > 8. || (px.abs() < 2. && py.abs() < 2.) {
                    assert_eq!(
                        sim.stock.heights[y * sim.stock.nx + x],
                        sim.stock.bounds.max[2]
                    );
                }
            }
        }
    }
    #[test]
    fn pocket_direction_and_spindle_reverse_loops_and_keep_final_stock_equal() {
        let (setup, mut op, tool, mesh) = fixture();
        let cw = generate(&setup, &op, &tool, &mesh).unwrap();
        let paths = loops(&cw);
        assert!(paths.iter().any(|p| area(p) > 0.) && paths.iter().any(|p| area(p) < 0.));
        op.p.insert("pocket_milling".into(), "Conventional".into());
        let conventional = generate(&setup, &op, &tool, &mesh).unwrap();
        for (a, b) in paths.iter().zip(loops(&conventional)) {
            assert!((area(a) + area(&b)).abs() < 0.001);
        }
        op.p.insert("spindle_rotation".into(), "Counterclockwise (CCW)".into());
        let ccw = generate(&setup, &op, &tool, &mesh).unwrap();
        assert_eq!(loops(&ccw), paths);
        let mut expected = None;
        for plan in [cw, conventional, ccw] {
            let mut sim = Simulation::new(plan);
            sim.seek(1.);
            if let Some(ref stock) = expected {
                assert_eq!(&sim.stock.heights, stock);
            } else {
                expected = Some(sim.stock.heights.clone());
            }
        }
    }
    #[test]
    fn profile_ramp_obeys_angle_and_rewind_matches_incremental_playback() {
        let (setup, mut op, tool, mesh) = fixture();
        op.p.insert("pocket_entry".into(), "Profile ramp".into());
        let plan = generate(&setup, &op, &tool, &mesh).unwrap();
        assert!(
            plan.moves
                .iter()
                .any(|m| m.cutting_pass && m.to[2] < m.from[2])
        );
        for m in plan.moves.iter().filter(|m| m.cutting_pass) {
            let xy = ((m.to[0] - m.from[0]).powi(2) + (m.to[1] - m.from[1]).powi(2)).sqrt();
            assert!((m.to[2] - m.from[2]).abs() <= xy * 2_f32.to_radians().tan() + 0.0001);
        }
        let mut a = Simulation::new(plan.clone());
        let mut b = Simulation::new(plan);
        a.seek(0.5);
        for i in 0..=50 {
            b.seek(i as f32 / 100.);
        }
        for (x, y) in a.stock.heights.iter().zip(&b.stock.heights) {
            assert!((x - y).abs() < 0.001);
        }
        let partial = a.stock.heights.clone();
        a.seek(1.);
        a.seek(0.5);
        assert_eq!(partial, a.stock.heights);
        a.seek(0.);
        assert_eq!(a.stock.removed_volume(), 0.);
    }
    #[test]
    fn pocket_rejects_stale_geometry_bad_targets_tools_and_parameters() {
        let (setup, op, tool, mesh) = fixture();
        for (key, value, expected) in [
            ("bottom", "-1", "below"),
            ("stepover", "100", "50%"),
            ("stepdown", "0.000001", "limit"),
            ("pocket_milling", "unknown", "valid"),
            ("pocket_entry", "Helix", "valid"),
            ("containment", "Tool outside boundary", "inside"),
            ("boundary", "Stock silhouette", "Selected"),
            ("finish_passes", "0.5", "whole"),
        ] {
            let mut bad = op.clone();
            bad.p.insert(key.into(), value.into());
            assert!(
                generate(&setup, &bad, &tool, &mesh)
                    .unwrap_err()
                    .contains(expected),
                "{key}"
            );
        }
        let mut bad = op.clone();
        bad.surfaces.as_mut().unwrap().mesh_fingerprint = "0".repeat(64);
        assert!(
            generate(&setup, &bad, &tool, &mesh)
                .unwrap_err()
                .contains("changed")
        );
        let mut huge = tool.clone();
        huge.p.insert("diameter".into(), "40".into());
        assert!(
            generate(&setup, &op, &huge, &mesh)
                .unwrap_err()
                .contains("narrow")
        );
        huge.p.insert("type".into(), "Ball end mill".into());
        assert!(
            generate(&setup, &op, &huge, &mesh)
                .unwrap_err()
                .contains("flat end")
        );
    }
    #[test]
    fn pocket_rotation_translation_allowances_and_preferences_persist() {
        let (mut setup, mut op, tool, mesh) = fixture();
        let original = generate(&setup, &op, &tool, &mesh).unwrap();
        setup.p.insert("rz".into(), "90".into());
        setup.p.insert("tx".into(), "40".into());
        op.p.insert("allowance".into(), "0.5".into());
        op.p.insert("axial_allowance".into(), "0.4".into());
        let restored: Operation =
            serde_json::from_str(&serde_json::to_string(&op).unwrap()).unwrap();
        assert_eq!(pocket_setting(&restored.p, "pocket_entry"), "Plunge");
        let plan = generate(&setup, &restored, &tool, &mesh).unwrap();
        assert!((plan.target - original.target - 0.4).abs() < 0.001);
        assert!(
            plan.moves
                .iter()
                .filter(|m| m.cutting_pass)
                .all(|m| m.to[0] > 32. && m.to[0] < 48.)
        );
        let mut a = Simulation::new(original);
        let mut b = Simulation::new(plan);
        a.seek(1.);
        b.seek(1.);
        assert!(b.stock.removed_volume() < a.stock.removed_volume());
    }
    #[test]
    fn pocket_rejects_overhangs_and_floors_that_are_not_horizontal() {
        let (mut setup, mut op, tool, mesh) = fixture();
        let scale = mesh.display_scale();
        let mut triangles: Vec<[[f32; 3]; 3]> = mesh
            .faces
            .iter()
            .map(|f| std::array::from_fn(|i| f.points[i].map(|v| v / scale)))
            .collect();
        let floor_z = mesh.surfaces[0].center[2] / scale;
        quad(&mut triangles, 4., -1., 7., 1., floor_z + 2.);
        let covered = mesh::normalize(triangles).unwrap();
        op.surfaces.as_mut().unwrap().mesh_fingerprint = covered.fingerprint.clone();
        assert!(
            generate(&setup, &op, &tool, &covered)
                .unwrap_err()
                .contains("obstructs")
        );
        op.surfaces.as_mut().unwrap().mesh_fingerprint = mesh.fingerprint.clone();
        setup.p.insert("rx".into(), "10".into());
        assert!(
            generate(&setup, &op, &tool, &mesh)
                .unwrap_err()
                .contains("perpendicular")
        );
    }
    /// `tris` as a part on a zero-margin stock, with a Pocket of a flat
    /// cutter of `diameter` on its faces at the heights `floors`.
    /// Heights are given as `tris` has them; the plan's are those plus the
    /// fourth value returned. The parts here are centred on X and Y, which
    /// the plan keeps.
    fn part(
        tris: Vec<[[f32; 3]; 3]>,
        floors: &[f32],
        diameter: f32,
    ) -> (Setup, Operation, Tool, Mesh, f32) {
        let bottom = tris
            .iter()
            .flatten()
            .map(|p| p[2])
            .fold(f32::INFINITY, f32::min);
        let mesh = mesh::normalize(tris).unwrap();
        let (mut setup, mut op, mut tool, _) = fixture();
        setup.preview_remaining = None;
        tool.p.insert("diameter".into(), diameter.to_string());
        let scale = mesh.display_scale();
        let placement = Placement::new(&mesh, Some(&setup));
        let dz = placement.bounds.min[2] / scale - bottom;
        op.surfaces = Some(surface::Selection {
            mesh_fingerprint: mesh.fingerprint.clone(),
            ids: mesh
                .surfaces
                .iter()
                .enumerate()
                .filter(|(_, s)| {
                    let z = placement.transform(s.center)[2] / scale - dz;
                    s.normal[2] > 0.999 && floors.iter().any(|f| (z - f).abs() < 0.001)
                })
                .map(|(i, _)| i)
                .collect(),
        });
        (setup, op, tool, mesh, dz)
    }
    /// A 2 mm ledge at Z 3 between a wall up to Z 6 (X −15 to −5) and a
    /// floor at Z 0 (X −3 to 15), on a bottom at Z −4.
    fn ledge() -> Vec<[[f32; 3]; 3]> {
        let mut tris = vec![];
        quad(&mut tris, -15., -10., -5., 10., 6.);
        quad(&mut tris, -5., -10., -3., 10., 3.);
        quad(&mut tris, -3., -10., 15., 10., 0.);
        quad(&mut tris, -15., -10., 15., 10., -4.);
        tris
    }
    /// A floor at Z 0 round a 6 mm square window through the part, inside
    /// a rim up to Z 6 and over a bottom at Z −4 with the same window.
    fn window() -> Vec<[[f32; 3]; 3]> {
        let mut tris = vec![];
        for (x, y, xx, yy) in [
            (-10., -10., 10., -3.),
            (-10., 3., 10., 10.),
            (-10., -3., -3., 3.),
            (3., -3., 10., 3.),
        ] {
            quad(&mut tris, x, y, xx, yy, 0.);
        }
        for (x, y, xx, yy) in [
            (-15., -15., 15., -10.),
            (-15., 10., 15., 15.),
            (-15., -10., -10., 10.),
            (10., -10., 15., 10.),
        ] {
            quad(&mut tris, x, y, xx, yy, 6.);
        }
        for (x, y, xx, yy) in [
            (-15., -15., 15., -3.),
            (-15., 3., 15., 15.),
            (-15., -3., -3., 3.),
            (3., -3., 15., 3.),
        ] {
            quad(&mut tris, x, y, xx, yy, -4.);
        }
        tris
    }
    /// Whether a cutter of `radius` at `p` keeps clear of the box x0, y0, x1, y1.
    fn clear(p: &[f32; 3], (x0, y0, x1, y1): (f32, f32, f32, f32), radius: f32) -> bool {
        let dx = (x0 - p[0]).max(p[0] - x1).max(0.);
        let dy = (y0 - p[1]).max(p[1] - y1).max(0.);
        dx.hypot(dy) >= radius - 0.001
    }
    fn cutting(plan: &Plan) -> Vec<[f32; 3]> {
        plan.moves
            .iter()
            .filter(|m| m.cutting_pass)
            .map(|m| m.to)
            .collect()
    }
    #[test]
    fn a_ledge_open_to_a_lower_floor_takes_a_cutter_wider_than_itself() {
        // The 2 mm ledge has a wall on one side only: a Ø6 cutter stands over
        // the lower floor beside it and cuts up to the wall without touching it.
        let (setup, op, tool, mesh, dz) = part(ledge(), &[3.], 6.);
        let plan = generate(&setup, &op, &tool, &mesh).unwrap();
        assert!((plan.target - dz - 3.).abs() < 0.001);
        let points = cutting(&plan);
        assert!(
            points
                .iter()
                .all(|p| clear(p, (-15., -10., -5., 10.), 3.) && p[2] - dz >= 3. - 0.001)
        );
        assert!(points.iter().any(|p| p[0] - 3. < -4.9));
        // The report gives the floor a cutter of any size its wall allows.
        let ids = op.surfaces.as_ref().unwrap().ids.iter().copied().collect();
        assert_eq!(floor(&setup, &ids, &mesh).unwrap().largest(50.), 50.);
    }
    #[test]
    fn openings_are_cleared_over_or_cut_through_past_their_walls() {
        let (setup, mut op, tool, mesh, dz) = part(window(), &[0.], 2.);
        let ids = op.surfaces.as_ref().unwrap().ids.iter().copied().collect();
        let shape = floor(&setup, &ids, &mesh).unwrap();
        assert_eq!(shape.openings.len(), 1);
        assert!((shape.openings[0].1 - dz + 4.).abs() < 0.001);
        // Leave: the window is cleared over at the floor's height, never into.
        let left = generate(&setup, &op, &tool, &mesh).unwrap();
        assert!(cutting(&left).iter().all(|p| p[2] - dz >= -0.001));
        assert!(
            cutting(&left)
                .iter()
                .any(|p| p[0].abs() < 1. && p[1].abs() < 1.)
        );
        // Cut through: down past the window's walls by Breakthrough, inside them.
        op.p.insert("openings".into(), "Cut through".into());
        op.p.insert("breakthrough".into(), "0.5".into());
        let through = generate(&setup, &op, &tool, &mesh).unwrap();
        assert!((through.target - dz + 4.5).abs() < 0.001);
        let deep: Vec<_> = cutting(&through)
            .into_iter()
            .filter(|p| p[2] - dz < -0.001)
            .collect();
        assert!(!deep.is_empty());
        assert!(
            deep.iter()
                .all(|p| p[0].abs() + 1. <= 3. + 0.001 && p[1].abs() + 1. <= 3. + 0.001)
        );
        assert!(deep.iter().any(|p| (p[2] - dz + 4.5).abs() < 0.001));
        // A floor with no opening refuses Cut through.
        let (setup, mut op, tool, mesh, _) = part(ledge(), &[0.], 2.);
        op.p.insert("openings".into(), "Cut through".into());
        assert!(
            generate(&setup, &op, &tool, &mesh)
                .unwrap_err()
                .contains("no openings")
        );
    }
    #[test]
    fn a_pocket_clears_each_selected_floor_to_its_own_height() {
        let (setup, op, tool, mesh, dz) = part(ledge(), &[3., 0.], 2.);
        let plan = generate(&setup, &op, &tool, &mesh).unwrap();
        assert!((plan.target - dz).abs() < 0.001);
        let points: Vec<_> = cutting(&plan)
            .into_iter()
            .map(|p| [p[0], p[1], p[2] - dz])
            .collect();
        // On the ledge at Z 3 beside the wall, and on the floor at Z 0.
        assert!(
            points
                .iter()
                .any(|p| (p[2] - 3.).abs() < 0.001 && p[0] < -3.)
        );
        assert!(points.iter().any(|p| p[2].abs() < 0.001 && p[0] > 0.));
        // Clear of the wall, and of the ledge below its height.
        assert!(points.iter().all(|p| clear(p, (-15., -10., -5., 10.), 1.)
            && (p[2] >= 3. - 0.001 || clear(p, (-15., -10., -3., 10.), 1.))));
    }
    #[test]
    fn a_wall_leaning_over_its_floor_by_its_tessellation_is_not_an_overhang() {
        let (_, _, _, base) = fixture();
        let scale = base.display_scale();
        let floor_z = base.surfaces[0].center[2] / scale;
        let tris: Vec<[[f32; 3]; 3]> = base
            .faces
            .iter()
            .map(|f| std::array::from_fn(|i| f.points[i].map(|v| v / scale)))
            .collect();
        for (lean, accepted) in [(0.03, true), (0.5, false)] {
            let mut leaning = tris.clone();
            // A strip of the rim's wall standing over the floor's west edge.
            quad(&mut leaning, -10., -8., -10. + lean, 8., floor_z + 3.);
            let (setup, op, tool, mesh, _) = part(leaning, &[floor_z], 2.);
            let result = generate(&setup, &op, &tool, &mesh);
            assert_eq!(result.is_ok(), accepted, "{lean}: {result:?}");
        }
    }
    #[test]
    fn a_smaller_cutter_after_a_larger_one_cuts_only_the_corners_it_left() {
        // A 20 × 16 mm pocket in a rim: a Ø6 cutter leaves R3 corners.
        let mut tris = vec![];
        quad(&mut tris, -10., -8., 10., 8., 0.);
        for (x, y, xx, yy) in [
            (-15., -12., 15., -8.),
            (-15., 8., 15., 12.),
            (-15., -8., -10., 8.),
            (10., -8., 15., 8.),
        ] {
            quad(&mut tris, x, y, xx, yy, 6.);
        }
        quad(&mut tris, -15., -12., 15., 12., -4.);
        let (mut setup, op, tool, mesh, _) = part(tris, &[0.], 6.);
        let large = generate(&setup, &op, &tool, &mesh).unwrap();
        let mut small_tool = tool.clone();
        small_tool.p.insert("diameter".into(), "2".into());
        let whole = generate(&setup, &op, &small_tool, &mesh).unwrap();
        let frame = crate::volume::Frame::new(&setup, &mesh);
        let mut sim = Simulation::new(large);
        sim.volume = Some(crate::volume::Volume::new(sim.plan.stock, frame));
        sim.initial_volume = sim.volume.clone();
        sim.frame = Some(frame);
        sim.seek(1.);
        setup.preview_remaining = Some(std::sync::Arc::new(crate::rest::RestStock::new(
            sim.volume.as_ref().unwrap(),
            frame,
        )));
        let rest = generate(&setup, &op, &small_tool, &mesh).unwrap();
        let length = |plan: &Plan| -> f32 {
            plan.moves
                .iter()
                .filter(|m| m.cutting_pass)
                .map(|m| (m.to[0] - m.from[0]).hypot(m.to[1] - m.from[1]))
                .sum()
        };
        assert!(
            length(&rest) * 5. < length(&whole),
            "{} vs {}",
            length(&rest),
            length(&whole)
        );
        let corner = |p: &[f32; 3]| {
            [(-10., -8.), (10., -8.), (-10., 8.), (10., 8.)]
                .iter()
                .any(|c: &(f32, f32)| (p[0] - c.0).hypot(p[1] - c.1) < 4.)
        };
        let points = cutting(&rest);
        assert!(!points.is_empty() && points.iter().all(corner));
        // With nothing left anywhere the pocket runs whole, as without earlier cuts.
        let again = generate(&setup, &op, &tool, &mesh).unwrap();
        assert_eq!(
            again.moves.len(),
            generate(
                &{
                    let mut s = setup.clone();
                    s.preview_remaining = None;
                    s
                },
                &op,
                &tool,
                &mesh
            )
            .unwrap()
            .moves
            .len()
        );
    }
    #[test]
    fn actual_step_and_3mf_pocket_with_boss_generate_and_leave_the_boss() {
        for extension in ["step", "3mf"] {
            if extension == "step" && crate::step::skip_without_helper() {
                continue;
            }
            let mesh = mesh::load(
                &std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join(format!("examples/four-setup-block.{extension}")),
            )
            .unwrap();
            let (mut setup, mut op, mut tool, _) = fixture();
            setup.p.insert("stock_top".into(), "10".into());
            tool.p.insert("diameter".into(), "10".into());
            let placement = Placement::new(&mesh, Some(&setup));
            let top = placement.bounds.max[2];
            let bottom = placement.bounds.min[2];
            let (id, floor) = mesh
                .surfaces
                .iter()
                .enumerate()
                .filter(|(_, s)| {
                    s.normal[2].abs() > 0.999999
                        && placement.transform(s.center)[2] < top - 0.1
                        && placement.transform(s.center)[2] > bottom + 0.1
                })
                .max_by(|(_, a), (_, b)| a.area.total_cmp(&b.area))
                .unwrap();
            eprintln!(
                "{extension}: floor {} area {:.2} depth {:.2} mm",
                id + 1,
                floor.area / mesh.display_scale().powi(2),
                (top - placement.transform(floor.center)[2]) / mesh.display_scale()
            );
            op.surfaces = Some(surface::Selection {
                mesh_fingerprint: mesh.fingerprint.clone(),
                ids: vec![id],
            });
            let plan = generate(&setup, &op, &tool, &mesh).unwrap();
            eprintln!(
                "{extension}: {} moves, {} levels, {:.2} min",
                plan.moves.len(),
                plan.levels,
                plan.duration / 60.
            );
            assert!(plan.moves.len() > 100 && plan.levels > 1);
            let mut sim = Simulation::new(plan);
            sim.seek(1.);
            assert!(sim.stock.removed_volume() > 10_000.);
            // The fixture boss is centered at X=-18, Y=0 (not the part origin).
            let x = ((-18. - sim.stock.bounds.min[0]) / sim.stock.dx) as usize;
            let y = ((0. - sim.stock.bounds.min[1]) / sim.stock.dy) as usize;
            assert_eq!(
                sim.stock.heights[y * sim.stock.nx + x],
                sim.stock.bounds.max[2]
            );
        }
    }
}
