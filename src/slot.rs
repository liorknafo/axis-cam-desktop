//! Manual round-ended slots. A matching flat slot drill sweeps one straight
//! centre line; the same Plan drives stock, holding and the operator's Steps.
use crate::{
    cam::{self, Builder, Geometry, number},
    mesh::{Bounds, Mesh},
    model::{Fields, Operation, Setup, Tool, get, num},
    simulation::{Motion, Plan},
    surface::Selection,
};

pub const HELP: &str = "Cut a round-ended counterbore or through slot on the manual mill with a matching flat slot drill. Pick its floor or rounded end, then check the editable sizes and depths. Tool centres move along X or Y; DRO compensation stays off.";
pub const DEFAULTS: [(&str, &str); 14] = [
    ("slot_x", "0"),
    ("slot_y", "0"),
    ("slot_length", "18"),
    ("slot_width", "13"),
    ("slot_axis", "X"),
    ("z_top", "0"),
    ("z_bottom", "-10"),
    ("through", "No"),
    ("breakthrough", "0"),
    ("allowance", "0"),
    ("axial_allowance", "0"),
    ("stepdown", "0.5"),
    ("rpm", "640"),
    ("feed", "120"),
];
pub const ON_CNC: &str = "Slot runs on a manual mill; use Pocket on a CNC.";

#[derive(Debug, Clone)]
pub struct Slot {
    pub a: [f32; 2],
    pub b: [f32; 2],
    pub width: f32,
    pub top: f32,
    pub bottom: f32,
}
impl Slot {
    pub fn read(op: &Operation, zero: [f32; 3]) -> Result<Self, String> {
        let x = number(op, "slot_x")? + zero[0];
        let y = number(op, "slot_y")? + zero[1];
        let length = number(op, "slot_length")?;
        let width = number(op, "slot_width")?;
        let axis =
            match get(&op.p, "slot_axis") {
                "X" => 0,
                "Y" => 1,
                _ => return Err(
                    "A manual Slot must run along X or Y. Reorient the setup for a rotated slot."
                        .into(),
                ),
            };
        let top = number(op, "z_top")? + zero[2];
        let bottom = number(op, "z_bottom")? + zero[2];
        if width <= 0. || length <= width || bottom >= top {
            return Err("Give the slot a positive width, a length greater than its width, and bottom below top.".into());
        }
        let (mut a, mut b) = ([x, y], [x, y]);
        a[axis] -= (length - width) / 2.;
        b[axis] += (length - width) / 2.;
        Ok(Self {
            a,
            b,
            width,
            top,
            bottom,
        })
    }
    pub fn bounds(&self, radius: f32, bottom: f32, top: f32) -> Bounds {
        Bounds {
            min: [self.a[0] - radius, self.a[1] - radius, bottom],
            max: [self.b[0] + radius, self.b[1] + radius, top],
        }
    }
}
pub fn setting_issues(p: &Fields) -> Vec<String> {
    let op = Operation {
        p: p.clone(),
        ..Operation::new(crate::model::Strategy::Box, String::new())
    };
    let mut e = Slot::read(&op, [0.; 3])
        .err()
        .into_iter()
        .collect::<Vec<_>>();
    for k in ["allowance", "axial_allowance", "breakthrough"] {
        if !num(p, k).is_some_and(|v| v >= 0.) {
            e.push(format!("Enter a nonnegative {k}."));
        }
    }
    if !num(p, "stepdown").is_some_and(|v| v > 0.) {
        e.push("Slot stepdown must be positive.".into());
    }
    if !["Yes", "No"].contains(&get(p, "through")) {
        e.push("Choose blind or through for the slot.".into());
    }
    e
}

pub fn generate(setup: &Setup, op: &Operation, tool: &Tool, mesh: &Mesh) -> Result<Plan, String> {
    if !crate::manual::is_manual(&setup.machine) {
        return Err(ON_CNC.into());
    }
    if let Some(e) = setting_issues(&op.p).first() {
        return Err(e.clone());
    }
    if get(&tool.p, "type") != "Flat end mill" {
        return Err("Slot needs a flat end mill (slot drill).".into());
    }
    let mut b = Builder::new(setup, op, tool, mesh)?;
    let zero = crate::post::datum(setup, b.plan.stock, mesh)?;
    let slot = Slot::read(op, zero)?;
    let leave = number(op, "allowance")?;
    let axial = number(op, "axial_allowance")?;
    let diameter = 2. * b.plan.radius;
    if (diameter - (slot.width - 2. * leave)).abs() > 0.0001 {
        return Err(format!(
            "Slot needs a cutter matching its width after stock to leave ({} mm). Smaller cutters do not finish the round ends; larger cutters overcut them.",
            slot.width - 2. * leave
        ));
    }
    let through = get(&op.p, "through") == "Yes";
    let extra = number(op, "breakthrough")?;
    let stock = b.plan.stock;
    if through && ((slot.bottom - stock.min[2]).abs() > 0.001 || axial != 0.) {
        return Err("A through slot's bottom must be the stock bottom in work coordinates, with zero floor stock to leave.".into());
    }
    if !through && (slot.bottom < stock.min[2] - 0.001 || extra != 0.) {
        return Err(
            "A blind slot cannot go below the stock bottom or use breakthrough; choose Through."
                .into(),
        );
    }
    let target = if through {
        slot.bottom - extra
    } else {
        slot.bottom + axial
    };
    if target >= slot.top {
        return Err("Floor stock to leave leaves no slot depth.".into());
    }
    let cut = slot.bounds(b.plan.radius, target, stock.max[2]);
    if (0..2).any(|k| cut.min[k] < stock.min[k] - 0.001 || cut.max[k] > stock.max[k] + 0.001) {
        return Err("Keep the complete round-ended slot inside the stock.".into());
    }
    // Deliberately conservative: no claim that a short flute is safe just
    // because the wide counterbore gives the shank some room.
    let full_depth = stock.max[2] - target;
    if full_depth > b.plan.flute_length + 0.0001 || full_depth > b.plan.tool_length + 0.0001 {
        return Err("Slot depth from the original stock top (including breakthrough) exceeds flute length or stickout.".into());
    }
    let down = number(op, "stepdown")?;
    if down > b.plan.flute_length {
        return Err("Use a stepdown within the flute length.".into());
    }
    // A rectangular cap is a conservative *start-height* proof only. It
    // never becomes a removal volume or a part/holding clearance test.
    let top = crate::rest::start(setup, cut, target.max(stock.min[2]), stock.max[2], 0.);
    if slot.top < top - 0.001 {
        return Err(format!(
            "Slot Z top is below remaining stock: material still starts at work Z{:.3}. Raise Z top or clear it first.",
            top - zero[2]
        ));
    }
    if slot.top > stock.max[2] + 0.001 {
        return Err("Slot Z top cannot be above the original stock top.".into());
    }
    b.plan.rest_top = Some(top);
    let entry_clear = setup
        .preview_remaining
        .as_ref()
        .is_some_and(|rest| rest.entry_cleared(slot.a, b.plan.radius, target, stock.max[2]));
    if get(&tool.p, "center_cutting") != "Yes" && !entry_clear {
        return Err("Slot entry cuts material: confirm Center cutting = Yes on the actual tool, or first clear/pre-drill the entire cutter diameter to the final depth. A tool name or two flutes is not proof.".into());
    }
    check_part(setup, mesh, &slot, b.plan.radius, target, stock.max[2])?;
    let count = (((top - target) / down) - 0.00001).ceil().max(1.) as usize;
    if count > cam::CEILING / 10 {
        return Err(cam::too_many("slot levels"));
    }
    for i in 1..=count {
        let z = (top - i as f32 * down).max(target);
        // Return clear above the original stock. Descend at quarter feed,
        // including an already cleared entry, never rapid into uncertain stock.
        b.push(
            [slot.a[0], slot.a[1], b.retract],
            Motion::Rapid,
            false,
            b.rapid,
        )?;
        b.push(
            [slot.a[0], slot.a[1], z],
            Motion::Feed,
            false,
            b.feed * 0.25,
        )?;
        b.dwell(num(&setup.machine.p, "pass_time").unwrap_or(0.).max(0.));
        b.push([slot.b[0], slot.b[1], z], Motion::Feed, true, b.feed)?;
        b.push(
            [slot.b[0], slot.b[1], b.retract],
            Motion::Rapid,
            false,
            b.rapid,
        )?;
    }
    b.push(
        [slot.b[0], slot.b[1], b.clearance],
        Motion::Rapid,
        false,
        b.rapid,
    )?;
    b.plan.target = target;
    b.plan.levels = count;
    b.plan.milling_direction = "Full-width slot · feed in one direction";
    b.finish()
}

/// Match opposite half-round walls, then permit a click on either rounded
/// end or its horizontal floor. Arc fits recover nominal bounds rather than
/// the slightly short extrema of a faceted semicircle.
pub fn from_face(
    setup: &Setup,
    mesh: &Mesh,
    selection: &Selection,
    stock: Bounds,
) -> Result<Fields, String> {
    let &[id] = selection.ids.as_slice() else {
        return Err("Pick one slot floor or rounded end.".into());
    };
    if !selection.matches(mesh) {
        return Err("The model changed; pick the slot again.".into());
    }
    let g = Geometry::new(setup, mesh);
    let arcs = cam::arcs::arcs_among(setup, mesh, &cam::arcs::upright(setup, mesh));
    let zero = crate::post::datum(setup, stock, mesh)?;
    for (i, a) in arcs
        .iter()
        .enumerate()
        .filter(|(_, a)| (a.span - 180.).abs() < 1.)
    {
        for b in arcs
            .iter()
            .skip(i + 1)
            .filter(|b| (b.span - 180.).abs() < 1.)
        {
            if (a.r - b.r).abs() > 0.001
                || (a.bottom - b.bottom).abs() > 0.001
                || (a.top - b.top).abs() > 0.001
            {
                continue;
            }
            let d = [b.centre[0] - a.centre[0], b.centre[1] - a.centre[1]];
            let axis = if d[1].abs() < 0.001 && d[0].abs() > 0.001 {
                0
            } else if d[0].abs() < 0.001 && d[1].abs() > 0.001 {
                1
            } else {
                continue;
            };
            let (mut start, mut end) = (a.centre, b.centre);
            if start[axis] > end[axis] {
                std::mem::swap(&mut start, &mut end);
            }
            // The rounded ends must face away from each other.
            let outward = |arc: &cam::arcs::Arc, sign: f32| {
                let n = [arc.middle.cos(), arc.middle.sin()];
                n[axis] * sign > 0.999
            };
            let (left, right) = if a.centre[axis] < b.centre[axis] {
                (a, b)
            } else {
                (b, a)
            };
            if !outward(left, -1.) || !outward(right, 1.) {
                continue;
            }
            if !connected_pair(&g, start, end, a.r, a.bottom, a.top) {
                continue;
            }
            let floor = g
                .triangles
                .iter()
                .filter(|t| t.feature == Some(id))
                .collect::<Vec<_>>();
            let centre = [(start[0] + end[0]) / 2., (start[1] + end[1]) / 2.];
            let on_floor = !floor.is_empty()
                && floor.iter().flat_map(|t| t.p).all(|p| {
                    let nearest = p[axis].clamp(start[axis], end[axis]);
                    (p[2] - a.bottom).abs() < 0.001
                        && (p[axis] - nearest).hypot(p[1 - axis] - centre[1 - axis]) <= a.r + 0.001
                });
            if !a.walls.contains(&id) && !b.walls.contains(&id) && !on_floor {
                continue;
            }
            let slot = Slot {
                a: start,
                b: end,
                width: 2. * a.r,
                top: stock.max[2],
                bottom: a.bottom,
            };
            let top = crate::rest::start(
                setup,
                slot.bounds(a.r, a.bottom, stock.max[2]),
                a.bottom,
                stock.max[2],
                0.,
            );
            let vals = [
                centre[0] - zero[0],
                centre[1] - zero[1],
                (end[axis] - start[axis]) + 2. * a.r,
                2. * a.r,
                top - zero[2],
                a.bottom - zero[2],
            ];
            let mut p = Fields::new();
            for (k, v) in [
                "slot_x",
                "slot_y",
                "slot_length",
                "slot_width",
                "z_top",
                "z_bottom",
            ]
            .into_iter()
            .zip(vals)
            {
                p.insert(k.into(), format!("{v:.3}"));
            }
            p.insert("slot_axis".into(), ["X", "Y"][axis].into());
            p.insert(
                "through".into(),
                if (a.bottom - stock.min[2]).abs() < 0.001 {
                    "Yes"
                } else {
                    "No"
                }
                .into(),
            );
            return Ok(p);
        }
    }
    Err("Pick the horizontal floor or rounded end of an X- or Y-aligned capsule slot. Rotated or irregular pockets need another setup or CNC Pocket.".into())
}

/// Maximum outward support of the whole capsule minus the nominal arc
/// radius. For every radial normal on an inside arc, the cutter's support
/// is max(dot(a-c,n), dot(b-c,n)) + r. Maxima occur at the arc's ends or
/// along the vector from c to either centre endpoint. No path sampling.
fn arc_overcut(arc: &cam::arcs::Arc, a: [f32; 2], b: [f32; 2], r: f32) -> f64 {
    let mid = arc.middle as f64;
    let half = (arc.span as f64).to_radians() / 2.;
    let mut worst = f64::NEG_INFINITY;
    for p in [a, b] {
        let v = [(p[0] - arc.centre[0]) as f64, (p[1] - arc.centre[1]) as f64];
        let theta = v[1].atan2(v[0]);
        let delta = (theta - mid + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU)
            - std::f64::consts::PI;
        let mut angles = vec![mid - half, mid + half];
        if delta.abs() <= half {
            angles.push(theta);
        }
        for t in angles {
            worst = worst.max(v[0] * t.cos() + v[1] * t.sin() + r as f64 - arc.r as f64);
        }
    }
    worst
}

/// The exact continuous stock preceding an edited slot; the slot itself is
/// excluded so unfinished geometry cannot block picking it.
pub fn pick_setup(
    doc: &crate::model::Document,
    mesh: &Mesh,
    si: usize,
    operation: &str,
) -> Result<Setup, String> {
    let mut prior = doc.clone();
    let oi = prior
        .setups
        .get(si)
        .and_then(|s| s.operations.iter().position(|o| o.id == operation))
        .ok_or("Choose a Slot operation.")?;
    prior.setups.truncate(si + 1);
    prior.setups[si].operations.truncate(oi);
    let mut last = None;
    let mut error = None;
    crate::simulation::sequence_with(
        &prior,
        mesh,
        None,
        |_, _, result| {
            if let Err(e) = result {
                error.get_or_insert(e);
            }
        },
        |_, _, v| last = Some(v.clone()),
    );
    if let Some(e) = error {
        return Err(format!(
            "Resolve the preceding cut before picking the slot: {e}"
        ));
    }
    let mut here = doc.setups[si].clone();
    here.p.insert("stock_source".into(), "fresh".into());
    if let Some(v) = last {
        let frame = crate::volume::Frame::new(&here, mesh);
        here.preview_stock = Some(crate::simulation::leftover_box(&v, &here, mesh));
        here.preview_remaining = Some(std::sync::Arc::new(crate::rest::RestStock::new(&v, frame)));
    }
    Ok(here)
}

#[cfg(test)]
pub(crate) mod tests;

/// Both straight walls must connect these ends at this level. Equal radius
/// and alignment alone can accidentally bridge two separate same-row slots.
fn connected_pair(g: &Geometry, a: [f32; 2], b: [f32; 2], r: f32, bottom: f32, top: f32) -> bool {
    let axis = usize::from((b[1] - a[1]).abs() > (b[0] - a[0]).abs());
    [-1., 1.].iter().all(|side| {
        let cross = a[1 - axis] + side * r;
        let mut faces = std::collections::HashMap::<usize, Vec<[f32; 3]>>::new();
        for t in &g.triangles {
            if t.p.iter().all(|p| {
                (p[1 - axis] - cross).abs() < 0.0001
                    && p[2] >= bottom - 0.0001
                    && p[2] <= top + 0.0001
            }) && let Some(id) = t.feature
            {
                faces.entry(id).or_default().extend(t.p);
            }
        }
        faces.values().any(|points| {
            let lo = points.iter().map(|p| p[axis]).fold(f32::INFINITY, f32::min);
            let hi = points
                .iter()
                .map(|p| p[axis])
                .fold(f32::NEG_INFINITY, f32::max);
            let low = points.iter().map(|p| p[2]).fold(f32::INFINITY, f32::min);
            let high = points
                .iter()
                .map(|p| p[2])
                .fold(f32::NEG_INFINITY, f32::max);
            (lo - a[axis]).abs() < 0.0001
                && (hi - b[axis]).abs() < 0.0001
                && (low - bottom).abs() < 0.0001
                && (high - top).abs() < 0.0001
        })
    })
}

/// Continuous part check, including nominal analytic bounds at faceted
/// semicircles. Mesh error is not a license to translate/oversize a cutter.
fn check_part(
    setup: &Setup,
    mesh: &Mesh,
    slot: &Slot,
    r: f32,
    target: f32,
    top: f32,
) -> Result<(), String> {
    let g = Geometry::new(setup, mesh);
    let arcs = cam::arcs::arcs_among(setup, mesh, &cam::arcs::upright(setup, mesh));
    let cut = slot.bounds(r, target, top);
    let z = [target + 0.0001, top + 0.0001];
    let mut proven = vec![];
    for a in &arcs {
        if a.top <= z[0] || a.bottom >= z[1] {
            continue;
        }
        let reaches = g
            .triangles
            .iter()
            .filter(|t| t.feature.is_some_and(|id| a.walls.contains(&id)))
            .any(|t| {
                cam::meets(&t.p, &cut) && cam::swept_with_slack(&t.p, [slot.a, slot.b], r, z, 0.)
            });
        if !reaches {
            continue;
        }
        if arc_overcut(a, slot.a, slot.b, r) > 0.00003 {
            return Err("The slot's cutter sweep crosses a nominal rounded end of the part. Check centre, length and cutter diameter.".into());
        }
        proven.push(a);
    }
    for t in &g.triangles {
        if !cam::meets(&t.p, &cut) {
            continue;
        }
        if t.feature
            .is_some_and(|id| proven.iter().any(|a| a.walls.contains(&id)))
        {
            continue;
        }
        if !cam::swept_with_slack(&t.p, [slot.a, slot.b], r, z, 0.00003) {
            continue;
        }
        let mut pieces = vec![t.p.to_vec()];
        // Only subtract the actual circular segments between a wall chord
        // and its nominal arc. The entire sector, or a globally shrunken
        // cutter, would hide real planar material beside a round boundary.
        let horizontal = t.p.iter().all(|p| (p[2] - t.p[0][2]).abs() < 0.0001);
        if horizontal {
            for a in proven.iter().filter(|a| {
                (t.p[0][2] - a.top).abs() < 0.0001 || (t.p[0][2] - a.bottom).abs() < 0.0001
            }) {
                for wall in g
                    .triangles
                    .iter()
                    .filter(|w| w.feature.is_some_and(|id| a.walls.contains(&id)))
                {
                    for i in 0..3 {
                        let (u, v) = (wall.p[i], wall.p[(i + 1) % 3]);
                        if (u[2] - a.top).abs() > 0.0001 || (v[2] - a.top).abs() > 0.0001 {
                            continue;
                        }
                        let mid = [
                            (u[0] + v[0]) / 2. - a.centre[0],
                            (u[1] + v[1]) / 2. - a.centre[1],
                        ];
                        let distance = mid[0].hypot(mid[1]);
                        let len = (u[0] - v[0]).hypot(u[1] - v[1]);
                        if len < 0.00001 || distance <= 0. {
                            continue;
                        }
                        let theta = mid[1].atan2(mid[0]);
                        let half = (len / 2.).atan2(distance);
                        pieces = pieces
                            .into_iter()
                            .flat_map(|poly| outside_lens(poly, a.centre, theta, half, distance))
                            .collect();
                    }
                }
            }
        }
        for poly in pieces {
            for i in 1..poly.len().saturating_sub(1) {
                let tri = [poly[0], poly[i], poly[i + 1]];
                if horizontal {
                    let area = ((tri[1][0] - tri[0][0]) as f64 * (tri[2][1] - tri[0][1]) as f64
                        - (tri[1][1] - tri[0][1]) as f64 * (tri[2][0] - tri[0][0]) as f64)
                        .abs()
                        / 2.;
                    // Polygon partitions share their chord edge. Single
                    // precision can leave a zero-area edge as a tiny sliver.
                    if area < 0.00001 {
                        continue;
                    }
                }
                if cam::swept_with_slack(&tri, [slot.a, slot.b], r, z, 0.00003) {
                    return Err("The slot's cutter sweep cuts into the part. Check centre, width, length, floor and stock to leave.".into());
                }
            }
        }
    }
    Ok(())
}
/// Partition a cap triangle by the complement of one circular segment:
/// between the chord's angular rays and beyond its chord plane. Analytic
/// support has already proven the cutter inside the nominal circle there.
fn outside_lens(
    poly: Vec<[f32; 3]>,
    centre: [f32; 2],
    theta: f32,
    half: f32,
    distance: f32,
) -> Vec<Vec<[f32; 3]>> {
    let lo = theta - half;
    let hi = theta + half;
    let mut inside = poly;
    let mut outside = vec![];
    for (n, offset) in [
        ([-lo.sin(), lo.cos()], 0.),
        ([hi.sin(), -hi.cos()], 0.),
        ([theta.cos(), theta.sin()], distance),
    ] {
        let clip = |poly: &Vec<[f32; 3]>, keep_inside: bool| {
            let value = |p: [f32; 3]| {
                (((p[0] - centre[0]) as f64 * n[0] as f64
                    + (p[1] - centre[1]) as f64 * n[1] as f64)
                    - offset as f64)
                    * if keep_inside { 1. } else { -1. }
            };
            let mut out = vec![];
            for i in 0..poly.len() {
                let (p, q) = (poly[i], poly[(i + 1) % poly.len()]);
                let (v, w) = (value(p), value(q));
                if v >= 0. {
                    out.push(p);
                }
                if (v >= 0.) != (w >= 0.) {
                    let t = (v / (v - w)) as f32;
                    out.push(std::array::from_fn(|k| p[k] + t * (q[k] - p[k])));
                }
            }
            out
        };
        let piece = clip(&inside, false);
        if piece.len() >= 3 {
            outside.push(piece);
        }
        inside = clip(&inside, true);
    }
    outside
}
