//! Internal groove (`Strategy::Groove`): a groove below the mouth of a bore,
//! like a DIN 472 circlip groove, milled by circular interpolation with a
//! keyseat (slot) cutter, a thin disc head on a narrower neck. The head goes
//! down the bore's axis to the groove, arcs out to depth, circles (and
//! circles again for a spring pass), arcs back to the axis and comes up it.
//! A groove wider than the head takes several passes along the axis. Picked
//! on a boss's outside wall instead, the same moves cut a groove round it,
//! entering and leaving outside the boss.
//!
//! The head is swept as a flat cylinder as tall as its width
//! (`CutterShape::Flat` with that flute length): the stock's continuous
//! cutter history (`stock_surface`) and its voxels (`volume`) are solids, so
//! the groove is an undercut in them, under the bore wall it leaves. The
//! neck and the shank do not cut; they are drawn (`Plan::neck`) and kept
//! clear of the bore and of the part's model here.
use crate::{
    cam::{Builder, Geometry, bored, number},
    mesh::Mesh,
    model::{Fields, KEYSEAT, Operation, Setup, Tool, get, num},
    simulation::{CutterShape, Motion, Plan},
};
use std::f32::consts::{PI, TAU};

pub const AVAILABILITY: &str = "Available · a groove inside a bore, or round a boss, by circular interpolation with a keyseat cutter";
/// Where the groove lies along the bore (`groove_from`): its near edge a
/// land from the bore's top (the face it is cut from), or from its bottom
/// (the far face), or its top edge a depth below the stock top.
pub const GROOVE_FROM: [&str; 3] = [
    "Land from the top of the bore",
    "Land from the bottom of the bore",
    "Depth below the stock top",
];
/// A new Internal groove's settings: a DIN 472-32 circlip groove's size,
/// which the operator changes, no spring pass.
pub const DEFAULTS: &[(&str, &str)] = &[
    ("name", "Internal groove"),
    ("groove_diameter", "33.7"),
    ("groove_width", "1.3"),
    ("groove_from", GROOVE_FROM[0]),
    ("groove_land", "2.6"),
    ("spring_pass", "No"),
    ("rpm", "3000"),
    ("feed", "150"),
];
/// A new keyseat cutter's sizes beyond a mill's: the head's `width`, the
/// `neck`'s diameter, the `reach` from the head's bottom to the shank, and
/// the `shank`'s diameter. Its `diameter` is the head's.
pub const KEYSEAT_DEFAULTS: &[(&str, &str)] = &[
    ("width", "1.2"),
    ("neck", "10"),
    ("reach", "20"),
    ("shank", "12"),
    ("flutes", "6"),
];
pub const NEEDS_KEYSEAT: &str = "Choose a keyseat cutter for an Internal groove.";
pub const ON_HAND: &str = "An internal groove needs a CNC: the cutter circles the bore, X and Y at once. Cut it on a lathe with a grooving tool, or on the CNC.";
/// How far clear of the bore wall the head goes down the bore's axis, mm a
/// side.
const HEAD_CLEARANCE: f32 = 0.2;
/// How far clear of the bore wall (or of the boss) the neck stays while the
/// head cuts, mm.
const NECK_CLEARANCE: f32 = 0.2;
/// How far outside a boss the head goes down to a groove round it, mm.
const BOSS_ENTRY: f32 = 1.;
/// How far inside a region a point of the part must lie to be hit, mm: the
/// tessellation's and the arithmetic's slack.
const SLACK: f32 = 0.02;

/// What is wrong with a keyseat cutter's own keys (`Tool::issues`): a
/// positive head width, a neck narrower than the head, a reach past the
/// head no more than the stickout, and a positive shank when given.
pub fn keyseat_issues(p: &Fields) -> Vec<String> {
    let mut e = vec![];
    for (k, what) in [
        ("width", "Head width"),
        ("neck", "Neck diameter"),
        ("reach", "Reach"),
    ] {
        if !num(p, k).is_some_and(|v| v > 0.) {
            e.push(format!("{what} must be a positive number."))
        }
    }
    if num(p, "neck")
        .zip(num(p, "diameter"))
        .is_some_and(|(n, d)| n >= d)
    {
        e.push("The neck must be narrower than the head.".into())
    }
    if num(p, "reach")
        .zip(num(p, "width"))
        .is_some_and(|(r, w)| r <= w)
    {
        e.push(
            "Reach (from the head's bottom to the shank) must be more than the head's width."
                .into(),
        )
    }
    if num(p, "reach")
        .zip(num(p, "stickout"))
        .is_some_and(|(r, s)| r > s)
    {
        e.push("Reach cannot be more than the stickout.".into())
    }
    if !get(p, "shank").trim().is_empty() && !num(p, "shank").is_some_and(|v| v > 0.) {
        e.push("Shank diameter must be a positive number.".into())
    }
    e
}
/// What is wrong with an Internal groove's own settings (`Operation::issues`).
pub fn setting_issues(p: &Fields) -> Vec<String> {
    let mut e = vec![];
    for (k, what) in [
        ("groove_diameter", "Groove diameter"),
        ("groove_width", "Groove width"),
    ] {
        if !num(p, k).is_some_and(|v| v > 0.) {
            e.push(format!("{what} must be a positive number."))
        }
    }
    if !GROOVE_FROM.contains(&get(p, "groove_from")) {
        e.push("Choose where the groove is measured from.".into())
    }
    if !num(p, "groove_land").is_some_and(|v| v >= 0.) {
        e.push("The groove's land (or depth) must be zero or more.".into())
    }
    if !["Yes", "No"].contains(&get(p, "spring_pass")) {
        e.push("Choose Yes or No for the spring pass.".into())
    }
    if !num(p, "tolerance").is_some_and(|v| (0.001..=0.1).contains(&v)) {
        e.push("Tolerance must be 0.001–0.1 mm.".into())
    }
    e
}

/// The round wall a groove is cut in: a bore's, or a boss's outside.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Wall {
    pub centre: [f32; 2],
    /// The bore's (narrowest) or the boss's (widest) radius, mm.
    pub radius: f32,
    pub top: f32,
    pub bottom: f32,
    /// A bore: the material is outside the wall. False for a boss.
    pub bore: bool,
}
/// The wall `op` picks in `setup`: the one bore `cam::holes` reads from its
/// selection, or, when the picked walls face away from their axis, the boss
/// they are the outside of.
pub fn wall(setup: &Setup, op: &Operation, mesh: &Mesh) -> Result<Wall, String> {
    let g = Geometry::new(setup, mesh);
    let walls: Vec<usize> = (op.surfaces.iter())
        .flat_map(|s| s.ids.iter().copied())
        .filter(|&id| crate::pocket::is_wall(setup, mesh, id))
        .collect();
    let picked: Vec<&crate::cam::Triangle> = (g.triangles.iter())
        .filter(|t| t.feature.is_some_and(|id| walls.contains(&id)))
        .collect();
    if let Some((centre, _, top, bottom)) = crate::cam::wall_ring(&g, &walls) {
        // Which way the walls face: away from the axis round a boss.
        let outward: f32 = (picked.iter())
            .map(|t| {
                let (u, v) = (sub(t.p[1], t.p[0]), sub(t.p[2], t.p[0]));
                let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2]];
                let m = [0, 1].map(|k| (t.p[0][k] + t.p[1][k] + t.p[2][k]) / 3. - centre[k]);
                n[0] * m[0] + n[1] * m[1]
            })
            .sum();
        if outward > 0. {
            let radius = (picked.iter())
                .flat_map(|t| t.p)
                .map(|p| (p[0] - centre[0]).hypot(p[1] - centre[1]))
                .fold(0., f32::max);
            return Ok(Wall {
                centre,
                radius,
                top,
                bottom,
                bore: false,
            });
        }
    }
    let mut holes = crate::cam::holes(setup, op, mesh).map_err(|_| {
        "Pick the bore's wall (or a boss's outside wall) for the groove.".to_string()
    })?;
    // A rim face picks every round hole in it: the bore filter keeps one.
    let filter = num(&op.p, "hole_diameter").unwrap_or(0.);
    if filter > 0. {
        holes.retain(|h| (h.diameter - filter).abs() <= 0.1);
    }
    match holes.as_slice() {
        [h] => Ok(Wall {
            centre: h.center,
            radius: h.diameter / 2.,
            top: h.top,
            bottom: h.bottom,
            bore: true,
        }),
        [] => Err(format!(
            "None of the picked holes is Ø{filter:.2}: pick the bore's wall, or enter its diameter."
        )),
        _ => Err(format!(
            "Pick one bore for the groove: the selection has {}. Click its wall alone, or enter its diameter to keep it.",
            holes.len()
        )),
    }
}
fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|k| a[k] - b[k])
}
/// A groove's size and place: its diameter, and its edges along the axis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Groove {
    pub radius: f32,
    pub low: f32,
    pub high: f32,
}
/// The groove `op` asks for in `wall`, with the stock's top `stock_top`.
pub fn groove(op: &Operation, wall: &Wall, stock_top: f32) -> Result<Groove, String> {
    let d = number(op, "groove_diameter")?;
    let width = number(op, "groove_width")?;
    let land = number(op, "groove_land")?;
    if d <= 0. || width <= 0. || land < 0. {
        return Err("Enter the groove's diameter and width, and a land of zero or more.".into());
    }
    let (low, high) = match get(&op.p, "groove_from") {
        f if f == GROOVE_FROM[0] => (wall.top - land - width, wall.top - land),
        f if f == GROOVE_FROM[1] => (wall.bottom + land, wall.bottom + land + width),
        f if f == GROOVE_FROM[2] => (stock_top - land - width, stock_top - land),
        _ => return Err("Choose where the groove is measured from.".into()),
    };
    Ok(Groove {
        radius: d / 2.,
        low,
        high,
    })
}
/// A keyseat cutter's sizes: its head's radius and width, its neck's
/// radius, its reach from the head's bottom to the shank, and the shank's
/// radius (the neck's when not saved).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Keyseat {
    pub radius: f32,
    pub width: f32,
    pub neck: f32,
    pub reach: f32,
    pub shank: f32,
}
impl Keyseat {
    pub fn of(tool: &Tool) -> Result<Self, String> {
        if get(&tool.p, "type") != KEYSEAT {
            return Err(NEEDS_KEYSEAT.into());
        }
        if let Some(issue) = keyseat_issues(&tool.p).into_iter().next() {
            return Err(issue);
        }
        let d = num(&tool.p, "diameter")
            .filter(|d| *d > 0.)
            .ok_or("Enter the keyseat cutter's head diameter.")?;
        let neck = num(&tool.p, "neck").unwrap_or(0.) / 2.;
        Ok(Self {
            radius: d / 2.,
            width: num(&tool.p, "width").unwrap_or(0.),
            neck,
            reach: num(&tool.p, "reach").unwrap_or(0.),
            shank: num(&tool.p, "shank")
                .filter(|s| *s > 0.)
                .map_or(neck, |s| s / 2.),
        })
    }
}
/// Why the cutter `k` cannot cut `groove` in `wall` from its top, if it
/// cannot: a groove no deeper than the wall, a head too wide for the bore,
/// a neck that would rub the bore (or the boss) while the head cuts, a
/// head wider than the groove, which would cut the wall above or below it,
/// a groove outside the wall's length, and too little reach.
pub fn refusal(k: &Keyseat, wall: &Wall, groove: &Groove) -> Option<String> {
    let (bore_d, groove_d) = (2. * wall.radius, 2. * groove.radius);
    let what = if wall.bore { "bore" } else { "boss" };
    if wall.bore && groove.radius <= wall.radius + 0.005 {
        return Some(format!(
            "The Ø{groove_d:.2} groove is no wider than the Ø{bore_d:.2} bore: there is nothing to cut. Enter the groove's diameter, wider than the bore."
        ));
    }
    if !wall.bore && groove.radius >= wall.radius - 0.005 {
        return Some(format!(
            "The Ø{groove_d:.2} groove is no smaller than the Ø{bore_d:.2} boss: there is nothing to cut. Enter the groove's diameter, smaller than the boss."
        ));
    }
    if !wall.bore && groove.radius <= 0. {
        return Some("Enter a groove diameter more than 0.".into());
    }
    if wall.bore && k.radius > wall.radius - HEAD_CLEARANCE {
        return Some(format!(
            "The Ø{:.2} keyseat head is too big for the Ø{bore_d:.2} bore: it must go down the bore's axis with {HEAD_CLEARANCE} mm clear each side. Choose a head of Ø{:.2} or less.",
            2. * k.radius,
            bore_d - 2. * HEAD_CLEARANCE
        ));
    }
    // The neck circles at the head's centre, as far out as the cut needs.
    let depth = (groove.radius - wall.radius).abs();
    if depth > k.radius - k.neck - NECK_CLEARANCE {
        return Some(format!(
            "The Ø{:.2} neck would rub the {what}: the groove is {depth:.2} mm deep and this cutter cuts {:.2} mm past its neck, {NECK_CLEARANCE} mm of it kept clear. Choose a cutter with a bigger head or a thinner neck.",
            2. * k.neck,
            k.radius - k.neck
        ));
    }
    let width = groove.high - groove.low;
    if k.width > width + 0.001 {
        return Some(format!(
            "The {:.2} mm wide head is wider than the {width:.2} mm groove: it would cut the {what} wall beside the groove. Choose a cutter no wider than the groove.",
            k.width
        ));
    }
    if groove.high > wall.top + 0.001 || groove.low < wall.bottom - 0.001 {
        return Some(format!(
            "The groove, {:.2} to {:.2} mm below the top of the {what}, is not inside the {what}, which is {:.2} mm long. Enter a land (or depth) that puts the groove inside it.",
            wall.top - groove.high,
            wall.top - groove.low,
            wall.top - wall.bottom
        ));
    }
    let deep = wall.top - groove.low;
    if deep > k.reach + 0.001 {
        return Some(format!(
            "The groove's far edge is {deep:.2} mm down the {what}, past the {:.2} mm this cutter reaches from its head's bottom to its shank. Choose a cutter that reaches further, or cut the groove from the other face.",
            k.reach
        ));
    }
    None
}
/// The tip heights of the passes that cut `groove` with a head `width`
/// wide: the fewest that cover it, evenly spread from its top edge down.
pub fn passes(groove: &Groove, width: f32) -> Vec<f32> {
    let span = groove.high - groove.low;
    let n = ((span - 0.001) / width).ceil().max(1.) as usize;
    let top = groove.high - width;
    if n == 1 {
        return vec![groove.low.max(top)];
    }
    (0..n)
        .map(|i| top - (top - groove.low) * i as f32 / (n - 1) as f32)
        .collect()
}
/// A point of `g`'s part inside the region `inside` says, within `bounds`
/// (min and max corners) round it, if there is one: each triangle there
/// sampled at least every `step` mm.
fn part_inside(
    g: &Geometry,
    bounds: [[f32; 3]; 2],
    step: f32,
    inside: impl Fn([f32; 3]) -> bool,
) -> Option<[f32; 3]> {
    for t in &g.triangles {
        let (lo, hi) = (0..3).fold(
            ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]),
            |(lo, hi), i| {
                (
                    std::array::from_fn(|k| lo[k].min(t.p[i][k])),
                    std::array::from_fn(|k| hi[k].max(t.p[i][k])),
                )
            },
        );
        if (0..3).any(|k| hi[k] < bounds[0][k] || lo[k] > bounds[1][k]) {
            continue;
        }
        let edge = (0..3)
            .map(|i| {
                let d = sub(t.p[(i + 1) % 3], t.p[i]);
                (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
            })
            .fold(0., f32::max);
        let n = ((edge / step).ceil() as usize).clamp(1, 2000);
        for i in 0..=n {
            for j in 0..=n - i {
                let (a, b) = (i as f32 / n as f32, j as f32 / n as f32);
                let p = std::array::from_fn(|k| {
                    t.p[0][k] + a * (t.p[1][k] - t.p[0][k]) + b * (t.p[2][k] - t.p[0][k])
                });
                if inside(p) {
                    return Some(p);
                }
            }
        }
    }
    None
}
/// Why the cutter `k`, cutting `groove` in `wall` at the tip heights
/// `levels`, would hit the part's model in `g`, if it would: the head on its
/// way down to the groove (or round the boss), the neck while the head
/// cuts, or the shank above the reach. What the head sweeps inside the
/// groove's own band is the groove, there or not in the model.
fn model_hit(
    g: &Geometry,
    k: &Keyseat,
    wall: &Wall,
    groove: &Groove,
    levels: &[f32],
    entry: f32,
    orbit: f32,
) -> Option<String> {
    let c = wall.centre;
    let lowest = levels.iter().copied().fold(f32::INFINITY, f32::min);
    let highest = levels.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let far = orbit.max(entry) + k.radius.max(k.shank) + 1.;
    let box_from = |z: f32| {
        [
            [c[0] - far, c[1] - far, z],
            [c[0] + far, c[1] + far, f32::MAX],
        ]
    };
    let rho = |p: [f32; 3]| (p[0] - c[0]).hypot(p[1] - c[1]);
    let at = |p: [f32; 3]| format!("X{:.2} Y{:.2} Z{:.2}", p[0], p[1], p[2]);
    // Down to the groove: on the axis in a bore, at the entry outside a boss.
    let down = |p: [f32; 3]| {
        p[2] > lowest + SLACK
            && !(p[2] < groove.high - SLACK && p[2] > groove.low + SLACK)
            && (p[0] - c[0] - entry).hypot(p[1] - c[1]) < k.radius - SLACK
    };
    if let Some(p) = part_inside(g, box_from(lowest), 0.25, down) {
        return Some(format!(
            "The Ø{:.2} keyseat head would hit the part at {} on its way down to the groove: the {} is narrower there. Choose a smaller head.",
            2. * k.radius,
            at(p),
            if wall.bore {
                "bore"
            } else {
                "way round the boss"
            }
        ));
    }
    // The neck sweeps from the axis (or the entry) round the orbit.
    let (inner, outer) = if wall.bore {
        (0., orbit + k.neck)
    } else {
        (orbit - k.neck, entry + k.neck)
    };
    let neck = |p: [f32; 3]| {
        p[2] > lowest + k.width + SLACK
            && p[2] < highest + k.reach - SLACK
            && rho(p) > inner + SLACK
            && rho(p) < outer - SLACK
    };
    if let Some(p) = part_inside(g, box_from(lowest + k.width), 0.25, neck) {
        return Some(format!(
            "The Ø{:.2} neck would rub the part at {} while the head cuts the groove. Choose a cutter with a thinner neck or a bigger head.",
            2. * k.neck,
            at(p)
        ));
    }
    let (inner, outer) = if wall.bore {
        (0., orbit + k.shank)
    } else {
        (orbit - k.shank, entry + k.shank)
    };
    let shank = |p: [f32; 3]| {
        p[2] > lowest + k.reach + SLACK && rho(p) > inner + SLACK && rho(p) < outer - SLACK
    };
    if let Some(p) = part_inside(g, box_from(lowest + k.reach), 0.25, shank) {
        return Some(format!(
            "The shank would hit the part at {}: this cutter reaches {:.2} mm from its head's bottom to its shank. Choose a cutter that reaches further, or cut the groove from the other face.",
            at(p),
            k.reach
        ));
    }
    None
}
/// The Internal groove `op` on `setup`'s part `mesh` with `tool`.
pub fn generate(setup: &Setup, op: &Operation, tool: &Tool, mesh: &Mesh) -> Result<Plan, String> {
    let k = Keyseat::of(tool)?;
    if crate::manual::is_hand(&setup.machine) {
        return Err(ON_HAND.into());
    }
    if let Some(issue) = setting_issues(&op.p).into_iter().next() {
        return Err(issue);
    }
    // The head is the flutes: as tall as it is wide.
    let mut cutter = tool.clone();
    cutter.p.insert("length".into(), k.width.to_string());
    let mut b = Builder::new(setup, op, &cutter, mesh)?;
    b.plan.shape = CutterShape::Flat;
    b.plan.neck = Some([k.neck, k.reach, k.shank]);
    b.plan.milling_direction = "Climb · circular interpolation";
    let wall = wall(setup, op, mesh)?;
    let groove = groove(op, &wall, b.plan.stock.max[2])?;
    if let Some(why) = refusal(&k, &wall, &groove) {
        return Err(why);
    }
    let levels = passes(&groove, k.width);
    // The head's centre circles where its edge reaches the groove's
    // diameter; round a boss it comes down outside it, clear.
    let orbit = if wall.bore {
        groove.radius - k.radius
    } else {
        groove.radius + k.radius
    };
    let entry = if wall.bore {
        0.
    } else {
        wall.radius + k.radius + BOSS_ENTRY
    };
    let g = Geometry::new(setup, mesh);
    if let Some(why) = model_hit(&g, &k, &wall, &groove, &levels, entry, orbit) {
        return Err(why);
    }
    let c = wall.centre;
    let start = [c[0] + entry, c[1]];
    let lowest = levels.iter().copied().fold(f32::INFINITY, f32::min);
    // From the stock's top: material left over the bore's mouth is in the
    // head's way down too.
    let mouth = wall.top.max(b.plan.stock.max[2]);
    if !bored(setup, &b, start, k.radius, mouth, lowest) {
        return Err(if wall.bore {
            format!(
                "Bore the Ø{:.2} hole first, at least {:.2} deep: the keyseat head goes down its axis to the groove.",
                2. * wall.radius,
                wall.top - lowest
            )
        } else {
            "Clear round the boss first, down to the groove: the keyseat head goes down beside it."
                .into()
        });
    }
    let tolerance = number(op, "tolerance")?;
    let spring = get(&op.p, "spring_pass") == "Yes";
    // Climb: counterclockwise inside a bore with the spindle clockwise,
    // clockwise round a boss.
    let turn = match (b.plan.spindle_clockwise, wall.bore) {
        (true, true) | (false, false) => 1.,
        _ => -1.,
    };
    let segments = (PI / (1. - tolerance.min(orbit) / orbit.max(1e-3)).acos())
        .ceil()
        .clamp(12., 360.) as usize;
    // The feed at the head's edge, on the groove's diameter.
    let rate = b.feed * orbit / groove.radius;
    // The lead in and out: half circles between the entry and the orbit,
    // tangent to the orbit where they meet it (each the other half of one
    // circle round the point midway).
    let mid = [c[0] + (entry + orbit) / 2., c[1]];
    let half = (orbit - entry).abs() / 2.;
    let sweep = if wall.bore { turn } else { -turn };
    let lead = |b: &mut Builder, from: f32, z: f32| -> Result<(), String> {
        let arc = (segments / 2).max(6);
        for i in 1..=arc {
            let phi = from + sweep * PI * i as f32 / arc as f32;
            b.push(
                [mid[0] + half * phi.cos(), mid[1] + half * phi.sin(), z],
                Motion::Feed,
                true,
                rate,
            )?;
        }
        Ok(())
    };
    let (lead_in, lead_out) = if wall.bore { (PI, 0.) } else { (0., PI) };
    let rounds = if spring { 2 } else { 1 };
    b.start_at([start[0], start[1], wall.top + 1.], Some(wall.top))?;
    for &z in &levels {
        // Down the axis (or beside the boss) to the pass.
        b.push([start[0], start[1], z], Motion::Feed, false, b.feed)?;
        lead(&mut b, lead_in, z)?;
        for i in 1..=rounds * segments {
            let a = turn * TAU * i as f32 / segments as f32;
            b.push(
                [c[0] + orbit * a.cos(), c[1] + orbit * a.sin(), z],
                Motion::Feed,
                true,
                rate,
            )?;
        }
        lead(&mut b, lead_out, z)?;
    }
    // Back up the axis, out of the bore, before the retract.
    b.push(
        [start[0], start[1], wall.top + 1.],
        Motion::Feed,
        false,
        b.feed,
    )?;
    b.plan.target = groove.low;
    b.plan.levels = levels.len();
    b.retract()?;
    b.finish()
}
#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::{
        model::{Document, Machine, Strategy},
        simulation::Simulation,
        stock_surface::{Detail, Snapshot},
        volume::{Frame, Volume},
    };
    use fidget::{shape::EzShape, vm::VmShape};

    type Tri = [[f32; 3]; 3];
    const N: usize = 96;
    /// The plate's thickness, mm.
    pub(crate) const T: f32 = 28.;
    fn quad(t: &mut Vec<Tri>, q: [[f32; 3]; 4]) {
        t.push([q[0], q[1], q[2]]);
        t.push([q[0], q[2], q[3]]);
    }
    fn at(r: f32, i: usize, z: f32) -> [f32; 3] {
        let a = TAU * (i % N) as f32 / N as f32;
        [r * a.cos(), r * a.sin(), z]
    }
    /// The surface of revolution of `profile` ((radius, z) points) about Z,
    /// facing the axis (`inward`, a bore's wall) or away from it.
    fn revolve(t: &mut Vec<Tri>, profile: &[(f32, f32)], inward: bool) {
        for w in profile.windows(2) {
            let ((r0, z0), (r1, z1)) = (w[0], w[1]);
            for i in 0..N {
                let q = [
                    at(r0, i, z0),
                    at(r0, i + 1, z0),
                    at(r1, i + 1, z1),
                    at(r1, i, z1),
                ];
                quad(t, if inward { q } else { [q[0], q[3], q[2], q[1]] });
            }
        }
    }
    /// The point at angle step `i` on the 60 × 60 square round the axis.
    fn square(i: usize) -> [f32; 2] {
        let a = TAU * (i % N) as f32 / N as f32;
        let d = 30. / a.cos().abs().max(a.sin().abs());
        [d * a.cos(), d * a.sin()]
    }
    /// A 60 × 60 × `T` plate on Z 0 with a bore about Z along the profile
    /// `bore` ((radius, z) from its top down to its bottom).
    pub(crate) fn plate(bore: &[(f32, f32)]) -> crate::mesh::Mesh {
        let mut t = vec![];
        let (top, bottom) = (bore[0], bore[bore.len() - 1]);
        for i in 0..N {
            let (s0, s1) = (square(i), square(i + 1));
            quad(
                &mut t,
                [
                    at(top.0, i, T),
                    [s0[0], s0[1], T],
                    [s1[0], s1[1], T],
                    at(top.0, i + 1, T),
                ],
            );
            quad(
                &mut t,
                [
                    at(bottom.0, i + 1, 0.),
                    [s1[0], s1[1], 0.],
                    [s0[0], s0[1], 0.],
                    at(bottom.0, i, 0.),
                ],
            );
            quad(
                &mut t,
                [
                    [s0[0], s0[1], 0.],
                    [s1[0], s1[1], 0.],
                    [s1[0], s1[1], T],
                    [s0[0], s0[1], T],
                ],
            );
        }
        revolve(&mut t, bore, true);
        crate::mesh::normalize(t).unwrap()
    }
    /// The plate's Ø32 bore through, with its two DIN 472-32 grooves,
    /// Ø33.7 × 1.3, a 2.6 land from each face.
    pub(crate) fn circlip_plate() -> crate::mesh::Mesh {
        let (r, g) = (16., 16.85);
        plate(&[
            (r, T),
            (r, T - 2.6),
            (g, T - 2.6),
            (g, T - 3.9),
            (r, T - 3.9),
            (r, 3.9),
            (g, 3.9),
            (g, 2.6),
            (r, 2.6),
            (r, 0.),
        ])
    }
    /// A Ø30 × 20 boss standing on Z 0, with a Ø27 × 1.2 groove 3 below
    /// its top when `grooved`.
    fn boss(grooved: bool) -> crate::mesh::Mesh {
        let mut t = vec![];
        for i in 0..N {
            t.push([[0., 0., 20.], at(15., i, 20.), at(15., i + 1, 20.)]);
            t.push([[0., 0., 0.], at(15., i + 1, 0.), at(15., i, 0.)]);
        }
        let profile = if grooved {
            vec![
                (15., 20.),
                (15., 17.),
                (13.5, 17.),
                (13.5, 15.8),
                (15., 15.8),
                (15., 0.),
            ]
        } else {
            vec![(15., 20.), (15., 0.)]
        };
        revolve(&mut t, &profile, false);
        crate::mesh::normalize(t).unwrap()
    }
    pub(crate) fn keyseat(head: f32, width: f32, neck: f32, reach: f32) -> Tool {
        let mut t = Tool::new("Keyseat", KEYSEAT, &head.to_string());
        t.set_type(KEYSEAT);
        for (k, v) in [
            ("width", width),
            ("neck", neck),
            ("reach", reach),
            ("shank", 12.),
            ("stickout", 40.),
            ("holder", 32.),
        ] {
            t.p.insert(k.into(), v.to_string());
        }
        t.p.insert("number".into(), "4".into());
        t
    }
    /// Selects the part's top face (`up`) or its bottom one, as the mesh
    /// has them: `mesh::normalize` moves the part, so not at Z `T` and 0.
    fn select_face(op: &mut Operation, mesh: &crate::mesh::Mesh, up: bool) {
        let sign = if up { 1. } else { -1. };
        let flat = |s: &&crate::surface::Surface| sign * s.normal[2] > 0.999;
        let z = (mesh.surfaces.iter().filter(flat))
            .map(|s| sign * s.center[2])
            .fold(f32::NEG_INFINITY, f32::max);
        let ids = (mesh.surfaces.iter().enumerate())
            .filter(|(_, s)| flat(s) && (sign * s.center[2] - z).abs() < 0.01)
            .map(|(id, _)| id)
            .collect();
        op.surfaces = Some(crate::surface::Selection {
            mesh_fingerprint: mesh.fingerprint.clone(),
            ids,
        });
    }
    /// A setup with the stock the part's own box, on a 3-axis CNC.
    fn setup() -> Setup {
        let mut s = Setup::new(Machine::new(3), "G54");
        for k in [
            "stock_left",
            "stock_right",
            "stock_front",
            "stock_back",
            "stock_top",
            "stock_bottom",
        ] {
            s.p.insert(k.into(), "0".into());
        }
        s
    }
    /// An Internal groove with `tool` on the bore (or boss) the top face's
    /// openings (or the walls `walls`) pick.
    fn groove_op(tool: &Tool, mesh: &crate::mesh::Mesh, walls: Option<Vec<usize>>) -> Operation {
        let mut op = Operation::new(Strategy::Groove, tool.id.clone());
        op.p.insert("rpm".into(), "2000".into());
        op.p.insert("feed".into(), "400".into());
        match walls {
            Some(ids) => {
                op.surfaces = Some(crate::surface::Selection {
                    mesh_fingerprint: mesh.fingerprint.clone(),
                    ids,
                })
            }
            None => select_face(&mut op, mesh, true),
        }
        op
    }
    /// A Ø32 drill through the bore, then each of `ops`, in the setups
    /// `setups` (the drill in the first): every operation's simulation and
    /// the stock left at the end.
    pub(crate) fn job(
        mesh: &crate::mesh::Mesh,
        mut setups: Vec<Setup>,
        drill: bool,
        tools: Vec<Tool>,
    ) -> (Document, Vec<Result<Simulation, String>>, Option<Volume>) {
        let mut doc = Document::default();
        if drill {
            let mut t = Tool::new("Ø32 drill", "Drill", "32");
            t.p.insert("length".into(), "45".into());
            t.p.insert("stickout".into(), "50".into());
            t.p.insert("number".into(), "1".into());
            let mut op = Operation::new(Strategy::Drill, t.id.clone());
            op.geometry.clear();
            select_face(&mut op, mesh, true);
            for (k, v) in [
                ("through", "Yes"),
                // Past its 118° point's cone, so the bore is open at face B.
                ("breakthrough", "10"),
                ("rpm", "600"),
                ("feed", "60"),
            ] {
                op.p.insert(k.into(), v.into());
            }
            setups[0].operations.insert(0, op);
            doc.tools.push(t);
        }
        doc.tools.extend(tools);
        doc.machines = vec![setups[0].machine.clone()];
        doc.setups = setups;
        let (mut all, mut stock) = (vec![], None);
        crate::simulation::sequence_with(
            &doc,
            mesh,
            None,
            |_, _, r| all.push(r),
            |_, _, v| stock = Some(v.clone()),
        );
        (doc, all, stock)
    }
    /// The plate's job: the bore drilled, then `edit`'s Internal groove with
    /// `tool`; the groove's simulation and the stock it leaves.
    fn plate_groove(
        tool: Tool,
        edit: impl Fn(&mut Operation),
    ) -> (Result<Simulation, String>, Option<Volume>, Document) {
        let mesh = circlip_plate();
        let mut s = setup();
        let mut op = groove_op(&tool, &mesh, None);
        edit(&mut op);
        s.operations.push(op);
        let (doc, mut all, stock) = job(&mesh, vec![s], true, vec![tool]);
        assert!(all[0].is_ok(), "the bore: {:?}", all[0].as_ref().err());
        (all.pop().unwrap(), stock, doc)
    }
    /// The material field of `volume`: negative inside the stock left.
    fn field(volume: &Volume) -> impl FnMut([f32; 3]) -> f32 + use<> {
        let vm = VmShape::from(Snapshot::of(volume, Detail::Standard).field());
        let tape = vm.ez_point_tape();
        let mut eval = VmShape::new_point_eval();
        move |p| eval.eval(&tape, p[0], p[1], p[2]).unwrap().0
    }
    /// Where the stock along the ray from `from` towards `to` first has
    /// material, by bisection to a micron: the distance from `from`.
    fn first_material(f: &mut impl FnMut([f32; 3]) -> f32, from: [f32; 3], to: [f32; 3]) -> f32 {
        let (mut lo, mut hi) = (0f32, 1f32);
        assert!(
            f(from) > 0. && f(to) < 0.,
            "{from:?} is not clear or {to:?} not solid"
        );
        for _ in 0..40 {
            let m = (lo + hi) / 2.;
            let p = std::array::from_fn(|k| from[k] + (to[k] - from[k]) * m);
            if f(p) > 0. { lo = m } else { hi = m }
        }
        let d: f32 = (0..3)
            .map(|k| (to[k] - from[k]).powi(2))
            .sum::<f32>()
            .sqrt();
        lo * d
    }
    /// The cutting moves' ends of `plan`, as their distance from `c` and Z.
    fn reach(plan: &Plan, c: [f32; 2]) -> Vec<(f32, f32)> {
        (plan.moves.iter())
            .filter(|m| m.cutting_pass)
            .map(|m| ((m.to[0] - c[0]).hypot(m.to[1] - c[1]), m.to[2]))
            .collect()
    }
    fn wall_of(mesh: &crate::mesh::Mesh, op: &Operation) -> Wall {
        wall(&setup(), op, mesh).unwrap()
    }

    #[test]
    fn the_head_reaches_the_groove_diameter_at_the_groove_in_two_passes() {
        // The plate's groove from face A: Ø33.7 × 1.3, a 2.6 land, with a
        // Ø25 × 1.2 head; its centre circles 16.85 − 12.5 from the axis.
        let tool = keyseat(25., 1.2, 10., 20.);
        let (sim, _, _) = plate_groove(tool.clone(), |_| {});
        let sim = sim.unwrap();
        let mesh = circlip_plate();
        let w = wall_of(&mesh, &groove_op(&tool, &mesh, None));
        assert!(w.bore && (w.radius - 16.).abs() < 0.02, "{w:?}");
        let top = w.top;
        let ends = reach(&sim.plan, w.centre);
        let far = ends.iter().map(|e| e.0).fold(0., f32::max);
        assert!((far - (16.85 - 12.5)).abs() < 0.002, "{far}");
        // Two passes: the head's top at the groove's top edge, then its
        // bottom at the groove's bottom edge.
        let mut levels: Vec<f32> = ends.iter().map(|e| e.1).collect();
        levels.sort_by(f32::total_cmp);
        levels.dedup_by(|a, b| (*a - *b).abs() < 1e-4);
        let want = [top - 3.9, top - 2.6 - 1.2];
        assert_eq!(levels.len(), 2, "{levels:?}");
        for (got, want) in levels.iter().zip(want) {
            assert!((got - want).abs() < 1e-3, "{levels:?}");
        }
        // The orbit is the groove less the head's radius, at each pass.
        for &z in &levels {
            let on = ends
                .iter()
                .filter(|e| (e.1 - z).abs() < 1e-4 && (e.0 - far).abs() < 0.002);
            assert!(on.count() >= 12, "no full circle at Z{z}");
        }
        assert_eq!(sim.plan.shape, CutterShape::Flat);
        assert_eq!(sim.plan.flute_length, 1.2);
        assert_eq!(sim.plan.neck, Some([5., 20., 6.]));
    }
    #[test]
    fn passes_cover_a_groove_wider_than_the_head() {
        let g = |w: f32| Groove {
            radius: 17.,
            low: 10.,
            high: 10. + w,
        };
        assert_eq!(passes(&g(1.2), 1.2), vec![10.]);
        let two = passes(&g(1.3), 1.2);
        assert_eq!(two.len(), 2);
        assert!(
            (two[0] - 10.1).abs() < 1e-5 && (two[1] - 10.).abs() < 1e-5,
            "{two:?}"
        );
        let three = passes(&g(3.), 1.2);
        assert_eq!(three.len(), 3, "{three:?}");
        assert!((three[0] - 11.8).abs() < 1e-5 && (three[2] - 10.).abs() < 1e-5);
        assert!((three[1] - 10.9).abs() < 1e-5);
    }
    #[test]
    fn a_spring_pass_circles_the_groove_again() {
        let tool = keyseat(25., 1.2, 10., 20.);
        let count = |spring: &str| {
            let (sim, _, _) = plate_groove(tool.clone(), |o| {
                o.p.insert("spring_pass".into(), spring.into());
            });
            let plan = sim.unwrap().plan;
            let c = plan.moves.iter().find(|m| m.cutting_pass).unwrap().from;
            let on = |p: [f32; 3]| ((p[0] - c[0]).hypot(p[1] - c[1]) - 4.35).abs() < 0.002;
            // The orbit's moves, round it from end to end.
            (plan.moves.iter())
                .filter(|m| m.cutting_pass && on(m.from) && on(m.to))
                .count()
        };
        let (once, twice) = (count("No"), count("Yes"));
        assert!(once > 0 && twice == 2 * once, "{once} {twice}");
    }
    #[test]
    fn the_head_goes_down_the_axis_and_returns_to_it_before_it_retracts() {
        let tool = keyseat(25., 1.2, 10., 20.);
        let (sim, _, _) = plate_groove(tool.clone(), |_| {});
        let plan = sim.unwrap().plan;
        let mesh = circlip_plate();
        let w = wall_of(&mesh, &groove_op(&tool, &mesh, None));
        let on_axis = |p: [f32; 3]| (p[0] - w.centre[0]).hypot(p[1] - w.centre[1]) < 1e-3;
        let first = plan.moves.iter().position(|m| m.cutting_pass).unwrap();
        let last = plan.moves.iter().rposition(|m| m.cutting_pass).unwrap();
        // Every move above the groove, before and after, is on the axis.
        for m in &plan.moves[..first] {
            if m.to[2] < w.top {
                assert!(on_axis(m.from) && on_axis(m.to), "{m:?}");
            }
        }
        assert!(on_axis(plan.moves[first].from));
        assert!(
            on_axis(plan.moves[last].to),
            "the last cut ends off the axis"
        );
        let after = &plan.moves[last + 1..];
        assert!(after.iter().all(|m| on_axis(m.from) && on_axis(m.to)));
        // Up the axis out of the bore at the feed, then the retract.
        assert_eq!(after[0].motion, Motion::Feed);
        assert!(after[0].to[2] > w.top);
        assert!(after.iter().all(|m| m.to[2] >= m.from[2]));
        // Between the passes the head moves along the axis only.
        let mut was_cutting = false;
        for m in &plan.moves[first..=last] {
            if !m.cutting_pass {
                assert!(was_cutting && on_axis(m.from) && on_axis(m.to), "{m:?}");
            }
            was_cutting = m.cutting_pass;
        }
        // The lead in leaves the axis tangent to nothing; the orbit is
        // reached tangent to it: the last lead move ends on the orbit
        // going round it.
        let ends = reach(&plan, w.centre);
        assert!(ends.iter().all(|e| e.0 <= 4.35 + 0.002));
    }
    #[test]
    fn the_bore_neck_reach_width_and_land_are_checked() {
        let bore = Wall {
            centre: [0., 0.],
            radius: 16.,
            top: T,
            bottom: 0.,
            bore: true,
        };
        let groove = Groove {
            radius: 16.85,
            low: T - 3.9,
            high: T - 2.6,
        };
        let cutter = |head: f32, width: f32, neck: f32, reach: f32| Keyseat {
            radius: head / 2.,
            width,
            neck: neck / 2.,
            reach,
            shank: 6.,
        };
        assert_eq!(refusal(&cutter(28., 1.2, 10., 20.), &bore, &groove), None);
        assert_eq!(refusal(&cutter(25., 1., 10., 20.), &bore, &groove), None);
        let says = |k: Keyseat, g: &Groove, what: &str| {
            let why = refusal(&k, &bore, g).unwrap_or_default();
            assert!(why.contains(what), "{what}: {why}");
        };
        says(
            cutter(31.8, 1.2, 10., 20.),
            &groove,
            "too big for the Ø32.00 bore",
        );
        says(
            cutter(25., 1.2, 23., 20.),
            &groove,
            "neck would rub the bore",
        );
        says(
            cutter(25., 1.5, 10., 20.),
            &groove,
            "wider than the 1.30 mm groove",
        );
        says(
            cutter(25., 1.2, 10., 3.),
            &groove,
            "past the 3.00 mm this cutter reaches",
        );
        let outside = Groove {
            low: T - 0.5,
            high: T + 0.8,
            ..groove
        };
        says(
            cutter(25., 1.2, 10., 20.),
            &outside,
            "below the top of the bore, is not inside the bore",
        );
        let shallow = Groove {
            radius: 15.9,
            ..groove
        };
        says(
            cutter(25., 1.2, 10., 20.),
            &shallow,
            "no wider than the Ø32.00 bore",
        );
    }
    #[test]
    fn a_wrong_cutter_or_land_is_refused_with_its_reason() {
        let tool = |head, width, neck, reach| keyseat(head, width, neck, reach);
        let refused = |t: Tool, land: &str| {
            plate_groove(t, |o| {
                o.p.insert("groove_land".into(), land.into());
            })
            .0
            .unwrap_err()
        };
        assert!(refused(tool(31.8, 1.2, 10., 20.), "2.6").contains("too big for the Ø32.00 bore"));
        assert!(
            refused(tool(25., 1.2, 10., 3.), "2.6")
                .contains("past the 3.00 mm this cutter reaches")
        );
        assert!(
            refused(tool(25., 1.2, 10., 20.), "27.5")
                .contains("below the top of the bore, is not inside the bore")
        );
        // Without the bore drilled first, the head has nowhere to go down.
        let mesh = circlip_plate();
        let t = tool(25., 1.2, 10., 20.);
        let mut s = setup();
        s.operations.push(groove_op(&t, &mesh, None));
        let (_, all, _) = job(&mesh, vec![s], false, vec![t]);
        let e = all[0].as_ref().unwrap_err();
        assert!(e.contains("Bore the Ø32.00 hole first"), "{e}");
    }
    #[test]
    fn the_model_check_finds_a_neck_rubbing_a_lip_over_the_groove() {
        // A Ø20 lip 2 mm deep over a Ø32 bore: the narrowest bore refuses
        // a Ø28 head on its numbers; the model alone sees the lip when the
        // numbers are those of the wide bore below it.
        let mesh = plate(&[(10., T), (10., T - 2.), (16., T - 2.), (16., 0.)]);
        let g = Geometry::new(&setup(), &mesh);
        let mut op = Operation::new(Strategy::Groove, String::new());
        select_face(&mut op, &mesh, true);
        let (c, top) = (wall_of(&mesh, &op).centre, wall_of(&mesh, &op).top);
        let bore = Wall {
            centre: c,
            radius: 16.,
            top,
            bottom: top - T,
            bore: true,
        };
        let groove = Groove {
            radius: 16.85,
            low: top - 6.,
            high: top - 4.8,
        };
        let k = Keyseat {
            radius: 9.,
            width: 1.2,
            neck: 5.,
            reach: 20.,
            shank: 6.,
        };
        let orbit = groove.radius - k.radius;
        let hit = model_hit(&g, &k, &bore, &groove, &[groove.low], 0., orbit).unwrap();
        assert!(hit.contains("neck would rub the part"), "{hit}");
        // A head wider than the lip hits it on the way down.
        let k = Keyseat { radius: 11., ..k };
        let orbit = groove.radius - k.radius;
        let hit = model_hit(&g, &k, &bore, &groove, &[groove.low], 0., orbit).unwrap();
        assert!(hit.contains("on its way down"), "{hit}");
        // A short reach puts the shank in the lip.
        let k = Keyseat {
            radius: 9.,
            neck: 2.,
            reach: 4.,
            shank: 9.,
            ..k
        };
        let orbit = groove.radius - k.radius;
        let hit = model_hit(&g, &k, &bore, &groove, &[groove.low], 0., orbit).unwrap();
        assert!(hit.contains("shank would hit the part"), "{hit}");
        // Without the lip nothing is hit.
        let open = plate(&[(16., T), (16., 0.)]);
        let g = Geometry::new(&setup(), &open);
        let k = Keyseat {
            radius: 12.5,
            width: 1.2,
            neck: 5.,
            reach: 20.,
            shank: 6.,
        };
        assert_eq!(
            model_hit(
                &g,
                &k,
                &bore,
                &groove,
                &[groove.low],
                0.,
                groove.radius - k.radius
            ),
            None
        );
    }
    #[test]
    fn the_simulated_stock_holds_the_groove_as_an_undercut() {
        let tool = keyseat(25., 1.2, 10., 20.);
        let (sim, stock, _) = plate_groove(tool.clone(), |_| {});
        let mut sim = sim.unwrap();
        sim.seek(1.);
        let mesh = circlip_plate();
        let w = wall_of(&mesh, &groove_op(&tool, &mesh, None));
        let volume = stock.unwrap();
        let mut f = field(&volume);
        let c = w.centre;
        let p = |r: f32, z: f32| [c[0] + r, c[1], z];
        // Across the groove's middle, the material starts at its Ø33.7.
        let mid = w.top - 3.25;
        let r = 15. + first_material(&mut f, p(15., mid), p(18., mid));
        assert!((r - 16.85).abs() < 0.01, "groove Ø{}", 2. * r);
        // Along the bore wall's inside, the groove runs from the land's
        // 2.6 to 3.9 below the top: the land and the wall below are solid.
        let down = first_material(&mut f, p(16.4, mid), p(16.4, w.top - 1.));
        let up = first_material(&mut f, p(16.4, mid), p(16.4, w.top - 6.));
        let (high, low) = (mid + down, mid - up);
        assert!(
            (high - (w.top - 2.6)).abs() < 0.01 && (low - (w.top - 3.9)).abs() < 0.01,
            "{low}..{high}"
        );
        // Round the other side too, and not past the groove's diameter.
        let q = |r: f32| [c[0] - r * 0.6, c[1] + r * 0.8, mid];
        assert!(f(q(16.8)) > 0. && f(q(16.9)) < 0.);
        // The bore wall stays at Ø32 over the land and below the groove.
        assert!(f(p(15.9, w.top - 1.)) > 0. && f(p(16.1, w.top - 1.)) < 0.);
        assert!(f(p(15.9, w.top - 5.)) > 0. && f(p(16.1, w.top - 5.)) < 0.);
        // The groove removed material the voxels count, under the wall.
        assert!(sim.removed_volume() > sim.inherited_volume());
        // No column of the top-down grid holds it: the grid is untouched.
        assert_eq!(sim.stock.heights, sim.initial_stock.heights);
    }
    #[test]
    fn the_flipped_plate_grooves_its_other_face() {
        // Face A, then the plate turned over about X for face B: each
        // groove a 2.6 land from the face up.
        let mesh = circlip_plate();
        let tool = keyseat(25., 1.2, 10., 20.);
        let mut a = setup();
        a.operations.push(groove_op(&tool, &mesh, None));
        let mut b = setup();
        b.id = "face B".into();
        b.p.insert("rx".into(), "180".into());
        b.p.insert("reposition".into(), "Reclamp and probe origin".into());
        // Face B's top is the plate's bottom face, turned up.
        let mut op = groove_op(&tool, &mesh, None);
        select_face(&mut op, &mesh, false);
        b.operations.push(op.clone());
        let wb = wall(&b, &op, &mesh).unwrap();
        let (doc, all, stock) = job(&mesh, vec![a, b], true, vec![tool]);
        for r in &all {
            assert!(r.is_ok(), "{:?}", r.as_ref().err());
        }
        let plan_b = &all[2].as_ref().unwrap().plan;
        let lowest = plan_b
            .moves
            .iter()
            .map(|m| m.to[2])
            .fold(f32::INFINITY, f32::min);
        assert!(
            (lowest - (wb.top - 3.9)).abs() < 1e-3,
            "{lowest} {}",
            wb.top
        );
        // Both grooves are in the stock, in setup 1's frame.
        let volume = stock.unwrap();
        let mut f = field(&volume);
        let fa = Frame::new(&doc.setups[0], &mesh);
        let fb = Frame::new(&doc.setups[1], &mesh);
        let wa = wall(&doc.setups[0], &doc.setups[0].operations[1], &mesh).unwrap();
        let ca = wa.centre;
        for z in [wa.top - 3.25, wa.bottom + 3.25] {
            assert!(f([ca[0] + 16.5, ca[1], z]) > 0., "no groove at Z{z}");
            assert!(f([ca[0] + 16.9, ca[1], z]) < 0.);
        }
        assert!(
            f([ca[0] + 16.5, ca[1], (wa.top + wa.bottom) / 2.]) < 0.,
            "between the grooves"
        );
        // Setup 2's groove, mapped from its own frame, is the one at face B.
        let pb = fb.transfer([wb.centre[0] + 16.5, wb.centre[1], wb.top - 3.25], fa);
        assert!(
            (pb[2] - wa.bottom - 3.25).abs() < 1e-3 && f(pb) > 0.,
            "{pb:?}"
        );
    }
    #[test]
    fn a_groove_round_a_boss_circles_outside_it() {
        // A Ø27 × 1.2 groove 3 below the top of a Ø30 boss, picked by its
        // outside walls, with a Ø12 head on a Ø4 neck.
        for grooved in [true, false] {
            let mesh = boss(grooved);
            let walls: Vec<usize> = (0..mesh.surfaces.len())
                .filter(|&id| crate::pocket::is_wall(&setup(), &mesh, id))
                .collect();
            let tool = keyseat(12., 1.2, 4., 15.);
            let mut op = groove_op(&tool, &mesh, Some(walls));
            for (k, v) in [
                ("groove_diameter", "27"),
                ("groove_width", "1.2"),
                ("groove_land", "3"),
            ] {
                op.p.insert(k.into(), v.into());
            }
            let w = wall(&setup(), &op, &mesh).unwrap();
            assert!(!w.bore && (w.radius - 15.).abs() < 0.01, "{w:?}");
            let mut s = setup();
            s.operations.push(op);
            let (_, all, stock) = job(&mesh, vec![s], false, vec![tool]);
            let sim = all[0].as_ref().unwrap_or_else(|e| panic!("{e}"));
            let ends = reach(&sim.plan, w.centre);
            let near = ends.iter().map(|e| e.0).fold(f32::INFINITY, f32::min);
            assert!((near - 19.5).abs() < 0.002, "{near}");
            assert!(ends.iter().all(|e| (e.1 - (w.top - 4.2)).abs() < 1e-3));
            let mut f = field(&stock.unwrap());
            let c = w.centre;
            let r = first_material(
                &mut f,
                [c[0] + 16., c[1], w.top - 3.6],
                [c[0] + 12., c[1], w.top - 3.6],
            );
            assert!((16. - r - 13.5).abs() < 0.01, "groove Ø{}", 2. * (16. - r));
            assert!(
                f([c[0] + 14.5, c[1], w.top - 2.]) < 0.,
                "the boss over the groove"
            );
            assert!(
                f([c[0] + 14.5, c[1], w.top - 6.]) < 0.,
                "the boss under the groove"
            );
        }
    }
    #[test]
    fn a_keyseat_cutters_sizes_are_checked() {
        let mut t = keyseat(25., 1.2, 10., 20.);
        assert_eq!(t.issues(), Vec::<String>::new());
        t.p.insert("neck".into(), "25".into());
        assert!(
            t.issues()
                .iter()
                .any(|e| e.contains("narrower than the head"))
        );
        t.p.insert("neck".into(), "10".into());
        t.p.insert("reach".into(), "1".into());
        assert!(
            t.issues()
                .iter()
                .any(|e| e.contains("more than the head's width"))
        );
        t.p.insert("reach".into(), "50".into());
        assert!(
            t.issues()
                .iter()
                .any(|e| e.contains("more than the stickout"))
        );
        // Its flute length is its head's width: none is asked for.
        t.p.insert("reach".into(), "20".into());
        t.p.remove("length");
        assert_eq!(t.issues(), Vec::<String>::new());
        // An Internal groove takes one; it is refused a flat end mill.
        let m = Machine::new(3);
        let op = Operation::new(Strategy::Groove, t.id.clone());
        assert_eq!(op.tool_types(&m), vec![KEYSEAT]);
        let flat = Tool::new("Flat", "Flat end mill", "6");
        let op = Operation::new(Strategy::Groove, flat.id.clone());
        assert!(
            op.issues_in(false, &m, &[flat], &[])
                .contains(&NEEDS_KEYSEAT.to_string())
        );
    }
    #[test]
    fn the_groove_is_posted_for_linuxcnc_and_passes_the_uc100_run_check() {
        let tool = keyseat(25., 1.2, 10., 20.);
        let (_, _, mut doc) = plate_groove(tool, |o| {
            o.p.insert("coolant".into(), "Off".into());
        });
        let mesh = circlip_plate();
        doc.setups[0].operations[0]
            .p
            .insert("coolant".into(), "Off".into());
        doc.file = Some(crate::model::ModelFile {
            name: "plate.stl".into(),
            path: "plate.stl".into(),
            format: "stl".into(),
            bytes: 0,
        });
        let job = crate::post::post_job(&doc, &mesh);
        let programs = job.setups[0]
            .result
            .as_ref()
            .unwrap_or_else(|e| panic!("{e:?}"));
        let text: String = programs.iter().map(|p| p.text.as_str()).collect();
        assert!(text.contains("T4"), "{text}");
        // The UC100 runs the same moves: its run check passes them.
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("jy5800.toml");
        std::fs::write(&profile, include_str!("../assets/jy5800.toml")).unwrap();
        for (k, v) in [
            ("controller", "UC100"),
            ("uc100_profile", profile.to_str().unwrap()),
        ] {
            doc.setups[0].machine.p.insert(k.into(), v.into());
        }
        let job = crate::post::post_job(&doc, &mesh);
        assert_eq!(job.setups[0].run, Some(Ok(())), "{:?}", job.setups[0].run);
    }
    /// The grooves a saved project's Internal grooves leave, measured in
    /// the simulated stock: run with `AXIS_GROOVE_PROJECT=<project.json>
    /// cargo test measure_the_grooves -- --ignored --nocapture`. It prints
    /// the part's fingerprint and its faces facing up and down, for a
    /// project's selections, and each groove's diameter and width.
    #[test]
    #[ignore]
    fn measure_the_grooves_of_a_project() {
        let path = std::env::var("AXIS_GROOVE_PROJECT").expect("AXIS_GROOVE_PROJECT");
        let (doc, _) = crate::storage::load_reporting(std::path::Path::new(&path)).unwrap();
        let file = doc.file.clone().unwrap();
        let mesh = crate::mesh::load(&file.path).unwrap();
        println!("fingerprint {}", mesh.fingerprint);
        for (id, s) in mesh.surfaces.iter().enumerate() {
            if s.normal[2].abs() > 0.999 {
                println!(
                    "face {id}: normal Z {:.0} at Z {:.3}",
                    s.normal[2], s.center[2]
                );
            }
        }
        let (mut sims, mut stock) = (vec![], None);
        crate::simulation::sequence_with(
            &doc,
            &mesh,
            None,
            |s, o, r| sims.push((s, o, r)),
            |_, _, v| stock = Some(v.clone()),
        );
        for (s, o, r) in &sims {
            println!(
                "setup {} op {}: {}",
                s + 1,
                o + 1,
                r.as_ref().map_or_else(|e| e.clone(), |_| "ok".into())
            );
        }
        let volume = stock.expect("no stock");
        let mut f = field(&volume);
        let first = Frame::new(&doc.setups[0], &mesh);
        for (si, setup) in doc.setups.iter().enumerate() {
            for op in setup
                .operations
                .iter()
                .filter(|o| o.strategy == Strategy::Groove)
            {
                let w = wall(setup, op, &mesh).unwrap();
                let tool = doc.tools.iter().find(|t| t.id == op.tool).unwrap();
                let g = groove(
                    op,
                    &w,
                    crate::stock::resolve(setup, Some(&mesh)).unwrap().stock.max[2],
                )
                .unwrap();
                let frame = Frame::new(setup, &mesh);
                let p = |r: f32, z: f32| frame.transfer([w.centre[0] + r, w.centre[1], z], first);
                let mid = (g.low + g.high) / 2.;
                let span = 2. * (g.radius - w.radius) + 1.;
                let out = first_material(&mut f, p(w.radius - 0.5, mid), p(g.radius + span, mid));
                let inside = (w.radius + g.radius) / 2.;
                let up = first_material(&mut f, p(inside, mid), p(inside, g.high + 2.));
                let down = first_material(&mut f, p(inside, mid), p(inside, g.low - 2.));
                println!(
                    "setup {} {}: {} Ø{:.2}, top Z{:.2}; the stock's groove Ø{:.3} × {:.3} wide, Z{:.3} to Z{:.3} of the setup ({} passes)",
                    si + 1,
                    op.name(),
                    if w.bore { "bore" } else { "boss" },
                    2. * w.radius,
                    w.top,
                    2. * (w.radius - 0.5 + out),
                    up + down,
                    mid - down,
                    mid + up,
                    passes(&g, Keyseat::of(tool).unwrap().width).len()
                );
            }
        }
    }
}
