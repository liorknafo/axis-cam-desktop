//! Checks of an indexed setup's moves with its part on the rotary A axis
//! (LIO-191). Where the A centreline is on the machine (the rotary unit's
//! geometry, measured or assumed) tells where every move is in machine
//! coordinates, as a 3-axis job's work offset does not: so each move is
//! checked against the machine's X, Y and Z travel and its A against the
//! machine's A range and the UC100 profile's soft limits. Z matters most:
//! the stock sits at the centre height, so a cut below it may be out of
//! reach and a retract over the stock out of the Z travel (the CNC4030 has
//! about 55 mm of Z). The tool, its holder and the spindle above it (the
//! holder's column, as for a vise) are kept clear of the chuck, its jaws, the
//! rotary head and the tailstock at each angle's cuts, during the index
//! moves and between operations, and of the bar's top where the holder
//! comes down onto it. The spindle above the holder is as wide as the
//! machine's spindle nose and starts the holder's length over the tool's
//! stick-out (`chuck::InSpindle`, LIO-204); a tool without a holder length
//! or a machine without a spindle nose takes an assumed one, and says so.
use crate::chuck::{InSpindle, Rig, ToolParts};
use crate::holding::{Hit, message, plane_message};
use crate::indexed::{angles, degrees, is_indexed};
use crate::mesh::Mesh;
use crate::model::{
    Document, HOLDER_LENGTH, Machine, NOSE_DEFAULT, POST_RETRACT, RotaryUnit, Setup, get, num,
    post_setting,
};
use crate::post::Held;
use crate::post_check::machine_mm;
use crate::simulation::{Motion, Plan, Simulation};
use std::collections::HashSet;

/// The results of a setup's operations: (index in the setup, simulation).
type Sims = [(usize, Result<Simulation, String>)];

/// Why a vise or clamps are not checked in an indexed setup.
pub const NOT_TURNED: &str =
    "it indexes A, and a vise or clamps do not turn with A: hold the stock in the chuck";
/// Why an indexed setup's moves cannot be placed on the machine.
const NOT_PLACED: &str = "its moves cannot be checked against the machine's travel: where the stock is along machine X is only known when it is held in the chuck (Modelled workholding → Chuck)";

/// `v` mm for a message, to the hundredth, without trailing zeros.
fn mm(v: f32) -> String {
    let s = format!("{v:.2}");
    match s.trim_end_matches('0').trim_end_matches('.') {
        "-0" => "0".into(),
        t => t.into(),
    }
}

/// The tool of operation `oi` of `setup`, whose plan is `plan`, in the
/// spindle of `unit` (LIO-204); and what of it is assumed, as a sentence
/// ending a message ("" when nothing is).
pub(crate) fn in_spindle(
    doc: &Document,
    setup: &Setup,
    oi: usize,
    plan: &Plan,
    unit: Option<RotaryUnit>,
) -> (InSpindle, String) {
    let id = &setup.operations[oi].tool;
    let (holder_length, holder_assumed) = (doc.tools.iter().find(|t| &t.id == id))
        .map_or((HOLDER_LENGTH, true), crate::model::Tool::holder_length);
    let (nose, nose_assumed) = unit.map_or((NOSE_DEFAULT[0], true), |u| {
        (u.nose_diameter, u.nose_assumed)
    });
    let mut assumed = vec![];
    if holder_assumed {
        assumed.push(format!(
            "the holder's {} mm length (the tool gives none: enter it in the Tool library)",
            mm(holder_length)
        ));
    }
    if nose_assumed {
        assumed.push(format!(
            "the spindle nose's Ø{} mm (the machine gives none: enter it in the Machine library)",
            mm(nose)
        ));
    }
    let tool = InSpindle {
        radius: plan.radius,
        stickout: plan.tool_length,
        holder_radius: plan.holder_radius,
        holder_length,
        nose_radius: nose / 2.,
    };
    let note = match assumed.is_empty() {
        true => String::new(),
        false => format!(" Assumed: {}.", assumed.join(" and ")),
    };
    (tool, note)
}

/// The stock an indexed setup's clamping starts from, as A turns it: its
/// centreline (Y, Z of the setup's frame), the farthest it reaches from it,
/// its ends along X and how it is said.
struct Bar {
    axis: [f32; 2],
    radius: f32,
    x: [f32; 2],
    words: String,
}
impl Bar {
    fn of(doc: &Document, base: usize, mesh: &Mesh) -> Option<Self> {
        let stock = doc.setup_stock(base, Some(mesh))?;
        let [_, dy, dz] = stock.dimensions();
        let clamped = &doc.setups[base];
        let round = match crate::chuck::holds(clamped) {
            true => doc.chuck_rig(clamped, stock).ok()?.round,
            false => crate::stock::round_bar(clamped),
        };
        let (radius, words) = if round {
            (dy.min(dz) / 2., format!("Ø{} mm bar", mm(dy.min(dz))))
        } else {
            (
                dy.hypot(dz) / 2.,
                format!(
                    "{} × {} mm stock (Ø{} mm across its corners)",
                    mm(dy),
                    mm(dz),
                    mm(dy.hypot(dz))
                ),
            )
        };
        Some(Self {
            axis: crate::indexed::axis_of(stock),
            radius,
            x: [stock.min[0], stock.max[0]],
            words,
        })
    }
    /// The highest the bar reaches under a column of radius `r` round the
    /// tip moving from `from` to `to` (setup frame), None when the column
    /// misses it.
    fn top_under(&self, from: [f32; 3], to: [f32; 3], r: f32) -> Option<f32> {
        let (x0, x1) = (from[0].min(to[0]) - r, from[0].max(to[0]) + r);
        if x1 < self.x[0] || x0 > self.x[1] {
            return None;
        }
        // The nearest the column's centre comes to the axis in Y.
        let (y0, y1) = (from[1] - self.axis[0], to[1] - self.axis[0]);
        let near = if y0.signum() != y1.signum() {
            0.
        } else {
            y0.abs().min(y1.abs())
        };
        let d = (near - r).max(0.);
        (d < self.radius).then(|| self.axis[1] + (self.radius.powi(2) - d.powi(2)).sqrt())
    }
}

/// A part over the tool (the holder or the spindle nose) coming down onto
/// the bar: how much it needs lifting to keep the gap off the bar's top,
/// which part, its height over the tip, the A, the bar's top under it, the
/// cut's bottom and the tip there.
struct Onto {
    need: f32,
    part: &'static str,
    low: f32,
    a: f32,
    top: f32,
    bottom: f32,
    tip: [f32; 3],
}

/// The limits one machine axis is checked against: what they are, low, high.
type Limits = Vec<(String, f32, f32)>;

/// The limits of machine X, Y, Z (mm) and A (degrees) of `m`: its travel,
/// X and Y from 0 and Z down from 0 at the top, and its A range; on a UC100
/// also its profile's soft limits. Or why the profile cannot be read.
fn limits(m: &Machine) -> Result<[Limits; 4], String> {
    let axes = ["X", "Y", "Z"];
    let mut out: [Limits; 4] = Default::default();
    for (k, axis) in axes.iter().enumerate() {
        let travel = machine_mm(m, &axis.to_lowercase());
        let (lo, hi) = if k == 2 { (-travel, 0.) } else { (0., travel) };
        out[k].push((
            format!("the machine's {} mm {axis} travel", mm(travel)),
            lo,
            hi,
        ));
    }
    if let (Some(lo), Some(hi)) = (num(&m.p, "rotary_min"), num(&m.p, "rotary_max")) {
        out[3].push(("the machine's A range".into(), lo, hi));
    }
    if get(&m.p, "controller") == "UC100" {
        let config = crate::control::profile(m)?;
        for (k, axis) in ["X", "Y", "Z", "A"].iter().enumerate() {
            let soft = config.axes.get(k).and_then(|x| x.soft_min.zip(x.soft_max));
            match soft {
                Some((lo, hi)) => out[k].push((
                    format!("the UC100 profile's {axis} soft limits"),
                    lo as f32,
                    hi as f32,
                )),
                None if k == 3 => {
                    return Err("the UC100 profile sets no soft limits on A".into());
                }
                None => {}
            }
        }
    }
    Ok(out)
}

/// The A the machine is at when setup `si` starts, as the sequence has it
/// (`simulation::sequence_walk`): 0 after a setup that is not indexed, else
/// the last angle the indexed setups after it turned to.
fn a_before(doc: &Document, si: usize) -> f32 {
    let mut a = 0.;
    for (i, s) in doc.setups[..si].iter().enumerate() {
        if !(is_indexed(s) && doc.index_base(i).is_some()) {
            a = 0.;
        } else if s.operations.iter().any(|o| o.enabled)
            && let Some(last) = angles(s).ok().and_then(|v| v.last().copied())
        {
            a = last;
        }
    }
    a
}

/// What is wrong with where the moves of indexed setup `si` (its `sims`, the
/// generated operations) are on the machine: each operation's moves outside
/// the machine's X, Y or Z travel or the UC100 profile's soft limits, its A
/// outside the machine's A range or the profile's, a cut below the reach of
/// the Z travel, a retract above it, an index move inside what A swings, the
/// holder brought down onto the bar, and too little room over the stock to
/// turn A at all; the holder and the spindle nose above it as the tool's
/// stick-out and holder length place them (`in_spindle`). Each with the
/// index of its operation in the setup (None
/// for the setup's), named by the setup, the A angle and the operation.
/// Empty for a setup that is not indexed.
pub fn problems(
    doc: &Document,
    mesh: &Mesh,
    si: usize,
    sims: &Sims,
) -> Vec<(Option<usize>, String)> {
    let setup = &doc.setups[si];
    let Some(base) = doc.index_base(si).filter(|_| is_indexed(setup)) else {
        return vec![];
    };
    let name = setup.name();
    let machine = &doc.setups[base].machine;
    let mut out = vec![];
    // Room to turn A over all it swings, with no clearance at all.
    let swing = match doc.swing(si, mesh, 0.) {
        Ok(swing) => Some(swing),
        Err(e) => {
            out.push((None, format!("{name}: {e}")));
            None
        }
    };
    let bar = Bar::of(doc, base, mesh);
    let words = bar.as_ref().map_or("stock".into(), |b| b.words.clone());
    let unit = doc.rotary_unit(machine);
    let travel_z = machine_mm(machine, "z");
    if let (Some(swing), Some(unit)) = (swing, unit) {
        let room = -swing.machine_z;
        let least = crate::control::RETRACT_ROOM as f32;
        if room < least {
            out.push((None, format!(
                "{name}: A turns all within {} mm of its centreline (the {words}, the part and the chuck's jaws at any angle), which at machine Z {} reaches machine Z {}: {} mm under the top of the machine's {} mm Z travel (Z 0), less than the {} mm a retract over it needs. There is no room to lift the tool over it between angles. Measure the A centreline (Rotary A on the Control page), or use smaller stock.",
                mm(swing.radius),
                mm(unit.centre_z),
                mm(swing.machine_z),
                mm(room),
                mm(travel_z),
                mm(least),
            )));
        }
    }
    let first = angles(setup)
        .ok()
        .and_then(|v| v.first().copied())
        .unwrap_or(0.);
    let at = match doc.at_angle(si, first, mesh) {
        Ok(at) => at,
        Err(e) => {
            out.push((None, format!("{name}: {e}")));
            return out;
        }
    };
    let Some(machine_zero) = at.machine_zero else {
        let why = if unit.is_none() {
            crate::indexed::NO_GEOMETRY
        } else {
            NOT_PLACED
        };
        // Said once: the swing finds the geometry missing too.
        if !out.iter().any(|(_, m)| m.ends_with(why)) {
            out.push((None, format!("{name}: {why}")));
        }
        return out;
    };
    let limits = match limits(machine) {
        Ok(limits) => limits,
        Err(e) => {
            out.push((
                None,
                format!("{name}: its moves cannot be checked against the soft limits: {e}."),
            ));
            return out;
        }
    };
    let on_machine =
        |p: [f32; 3]| -> [f32; 3] { std::array::from_fn(|k| p[k] - at.zero[k] + machine_zero[k]) };
    let centre_z = unit.map_or(0., |u| u.centre_z);
    let probe_length = (crate::control::probe_settings(&machine.p).ok())
        .and_then(|p| p.length)
        .map(|l| l as f32);
    // The holder is kept the setup's gap over the bar, as off the jaws.
    let gap = crate::holding::gap(&doc.setups[base]).unwrap_or(2.);
    // The A the machine is at, from one operation to the next.
    let mut a_now = a_before(doc, si);
    for (oi, sim) in sims
        .iter()
        .filter_map(|(oi, r)| r.as_ref().ok().map(|s| (*oi, s)))
    {
        let op = setup.operations[oi].name();
        let plan = &sim.plan;
        let (tool, assumed) = in_spindle(doc, setup, oi, plan, unit);
        // The centreline's Z is the probe's tip's: a tool longer than the
        // probe holds the spindle that much higher over the same cut
        // (LIO-207). The cut's depth comes from its touch-off on the Z
        // reference; only the travel is estimated from the lengths.
        let (longer, length_note) = match probe_length {
            Some(l) => (
                tool.stickout + tool.holder_length - l,
                format!(
                    " Its tip is taken {} mm under the probe's: its {} mm stick-out and {} mm holder less the probe's {} mm length.{assumed}",
                    mm(tool.stickout + tool.holder_length - l),
                    mm(tool.stickout),
                    mm(tool.holder_length),
                    mm(l),
                ),
            ),
            None => (
                0.,
                " Assumed: the tool as long as the probe the A centreline was probed with (enter the probe's length in the Machine library).".into(),
            ),
        };
        // Per axis the least and the most, with the A they are at.
        let mut low = [(f32::INFINITY, 0.); 4];
        let mut high = [(f32::NEG_INFINITY, 0.); 4];
        let mut keep = |k: usize, v: f32, a: f32| {
            if v < low[k].0 {
                low[k] = (v, a);
            }
            if v > high[k].0 {
                high[k] = (v, a);
            }
        };
        // The part above the tool that most needs lifting off the bar's top.
        let mut onto: Option<Onto> = None;
        let mut index = None;
        for m in &plan.moves {
            let a = m.a.unwrap_or(a_now);
            for p in [m.from, m.to] {
                let mut q = on_machine(p);
                q[2] += longer;
                for (k, v) in q.into_iter().enumerate() {
                    keep(k, v, a);
                }
            }
            keep(3, a, a);
            if m.motion == Motion::Index {
                if let Some(s) = swing
                    && index.is_none()
                    && m.from[2] < s.z - 1e-3
                {
                    index = Some((a_now, a, m.from[2], s));
                }
                a_now = a;
                continue;
            }
            a_now = a;
            if m.motion == Motion::Dwell {
                continue;
            }
            let bottom = m.from[2].min(m.to[2]);
            // The holder and the spindle nose, not the cutter.
            for (part, r, low, _) in tool.parts().into_iter().skip(1) {
                if let Some(top) = bar
                    .as_ref()
                    .and_then(|b| b.top_under(m.from, m.to, r + gap))
                    && let need = top + gap - (bottom + low)
                    && need > 1e-3
                    && onto.as_ref().is_none_or(|o| need > o.need)
                {
                    onto = Some(Onto {
                        need,
                        part,
                        low,
                        a,
                        top,
                        bottom,
                        tip: if m.from[2] <= m.to[2] { m.from } else { m.to },
                    });
                }
            }
        }
        let say = |a: f32, what: String| {
            (
                Some(oi),
                format!("{name} at A {}°, {op}: {what}", degrees(a)),
            )
        };
        // Each end of each axis, against the first of its limits it is past.
        for (k, below) in (0..4).flat_map(|k| [(k, true), (k, false)]) {
            let axis = ["X", "Y", "Z", "A"][k];
            for (what, lo, hi) in &limits[k] {
                let ((v, a), beyond) = match below {
                    true if low[k].0 < lo - 1e-3 => (low[k], lo - low[k].0),
                    false if high[k].0 > hi + 1e-3 => (high[k], high[k].0 - hi),
                    _ => continue,
                };
                // The tip's machine Z, as the probe's would be there.
                let tip = v - longer;
                let line = match (k, below) {
                    (2, true) => format!(
                        "the cut's bottom is at machine Z {}, {} mm short: below Z {}, the bottom of {what}. The {words} is centred at machine Z {}{}. Cut it from another angle, or measure the A centreline (Rotary A on the Control page).{length_note}",
                        mm(v),
                        mm(beyond),
                        mm(*lo),
                        mm(centre_z),
                        if tip < centre_z {
                            format!(
                                ", so this cut goes {} mm below its centreline",
                                mm(centre_z - tip)
                            )
                        } else {
                            String::new()
                        },
                    ),
                    (2, false) => format!(
                        "the tool goes up to machine Z {}, {} mm above Z {}, the top of {what}. The {words}'s top is at machine Z {} (its centreline at Z {} + {} mm), so the retract over it is out of reach. Lower the clearance, or measure the A centreline (Rotary A on the Control page).{length_note}",
                        mm(v),
                        mm(beyond),
                        mm(*hi),
                        mm(centre_z + bar.as_ref().map_or(0., |b| b.radius)),
                        mm(centre_z),
                        mm(bar.as_ref().map_or(0., |b| b.radius)),
                    ),
                    (3, _) => format!(
                        "A {}° is outside {what}, {}° to {}°.",
                        degrees(v),
                        degrees(*lo),
                        degrees(*hi)
                    ),
                    _ => format!(
                        "the tool would reach machine {axis} {}, {} mm outside {what}, {} to {} mm.",
                        mm(v),
                        mm(beyond),
                        mm(*lo),
                        mm(*hi)
                    ),
                };
                out.push(say(a, line));
                // The first limit it is past is enough.
                break;
            }
        }
        if let Some((from, to, z, s)) = index {
            out.push(say(to, format!(
                "the index move from A {}° to A {}° turns A with the tool tip at Z {} of the setup, {} mm inside the {} mm that the {words}, the part and the chuck's jaws reach from the A centreline.",
                degrees(from),
                degrees(to),
                mm(z - at.zero[2]),
                mm(s.z - z),
                mm(s.radius),
            )));
        }
        if let Some(Onto {
            need,
            part,
            low,
            a,
            top,
            bottom,
            tip: p,
        }) = onto
        {
            // How far over the bar's top the part's bottom is at the cut's.
            let over = bottom + low - top;
            let place = format!(
                "would come down onto the {words} at X{} Y{}: the cut goes {} mm below the bar's top there",
                mm(p[0] - at.zero[0]),
                mm(p[1] - at.zero[1]),
                mm(top - bottom),
            );
            // Where the holder ends and how wide the nose is matter only
            // to the nose.
            let fix = format!(
                "The tool needs {} mm more stick-out{}, or a shallower cut.{}",
                mm(need),
                if part == "holder" {
                    ""
                } else {
                    " (or a longer holder)"
                },
                if part == "holder" {
                    ""
                } else {
                    assumed.as_str()
                },
            );
            let line = match part {
                "holder" => format!(
                    "the holder {place}, and the tool's {} mm stick-out leaves the holder {}. {fix}",
                    mm(plan.tool_length),
                    match over < 0. {
                        true => format!("{} mm into it", mm(-over)),
                        false => format!(
                            "only {} mm over it, inside the {} mm gap",
                            mm(over),
                            mm(gap)
                        ),
                    },
                ),
                _ => format!(
                    "the spindle nose (Ø{} mm) {place}, and the nose is {} mm over the tip (the tool's {} mm stick-out and the holder's {} mm), so {}. {fix}",
                    mm(2. * tool.nose_radius),
                    mm(low),
                    mm(plan.tool_length),
                    mm(tool.holder_length),
                    match over < 0. {
                        true => format!("it hits the bar {} mm before the cut's bottom", mm(-over)),
                        false => format!(
                            "at the cut's bottom it is only {} mm over the bar, inside the {} mm gap",
                            mm(over),
                            mm(gap)
                        ),
                    },
                ),
            };
            out.push(say(a, line));
        }
    }
    out
}

/// The workholding problems of indexed setup `si` from its `sims`, as
/// `post::held_walked` gives a 3-axis setup's, or why they cannot be
/// checked. Held in the chuck: earlier cuts leaving the jaws too little to
/// grip, else the cutting forces against the grip; and the tool and its
/// holder within the setup's gap (and the blend tolerance) of the chuck, its
/// jaws, the rotary head, the tailstock or the stock gripped in the jaws:
/// along each angle's moves at that angle, over every index move's whole
/// turn, and on the way between operations (straight up, across and down,
/// at machine Z home when the machine retracts there), into the first and
/// out of the last.
pub fn held(doc: &Document, mesh: &Mesh, si: usize, sims: &Sims) -> Result<Vec<Held>, Vec<String>> {
    let setup = &doc.setups[si];
    let name = setup.name();
    let base = doc.index_base(si).ok_or_else(|| doc.index_issues(si))?;
    let clamped = &doc.setups[base];
    let mut errors = vec![];
    if sims.is_empty() {
        errors.push("This setup has no enabled operations. Add one or remove the setup.".into());
    }
    for (oi, result) in sims {
        if let Err(e) = result {
            errors.push(format!("{}: {e}", setup.operations[*oi].name()));
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    if !crate::chuck::holds(clamped) {
        return match crate::holding::modelled(clamped) {
            true => Err(vec![NOT_TURNED.into()]),
            false => Ok(vec![]),
        };
    }
    let stock = doc
        .setup_stock(base, Some(mesh))
        .ok_or("The stock the setup starts from is not known: check the first setup's stock.")
        .map_err(|e| vec![e.to_string()])?;
    let rig = match doc.chuck_rig(clamped, stock) {
        Ok(rig) => rig,
        Err(e) => {
            return Ok(vec![Held {
                operation: None,
                message: format!("{name}: {e}"),
            }]);
        }
    };
    let gap = crate::holding::gap(clamped).map_err(|e| vec![e])?;
    let list = angles(setup).map_err(|e| vec![e])?;
    let at = doc.at_angle(si, list[0], mesh).map_err(|e| vec![e])?;
    let sims: Vec<(usize, &Simulation)> = sims
        .iter()
        .filter_map(|(oi, r)| r.as_ref().ok().map(|s| (*oi, s)))
        .collect();
    // The jaws turn with the stock: what they grip is where they are at A0.
    let a0 = doc.at_angle(si, 0., mesh).map_err(|e| vec![e])?.frame;
    let removed = sims[0]
        .1
        .initial_volume
        .as_ref()
        .map(|v| crate::rest::Removed::new(v, a0));
    let mut found: Vec<Held> = rig
        .grip_issue(removed.as_ref())
        .into_iter()
        .map(|m| Held {
            operation: None,
            message: format!("{name}: {m}"),
        })
        .collect();
    if found.is_empty() {
        let bites = sims.iter().map(|(oi, sim)| (*oi, sim.bites()));
        for (oi, margin) in crate::forces::chuck_margins(doc, si, bites, &rig) {
            let op = setup.operations[oi].name();
            for message in margin
                .iter()
                .flat_map(|m| crate::forces::chuck_issues(op, m))
            {
                found.push(Held {
                    operation: Some(oi),
                    message,
                });
            }
        }
    }
    let m = &clamped.machine;
    let tools = sims
        .iter()
        .map(|(oi, sim)| (*oi, in_spindle(doc, setup, *oi, &sim.plan, Some(rig.unit))))
        .collect();
    let walk = Walk {
        rig: &rig,
        tools,
        gap,
        blend: crate::post::blend(m),
        datum: at.zero,
        // Machine Z 0, the top of the Z travel, in the setup's frame.
        home: (post_setting(&m.p, "post_retract") == POST_RETRACT[0])
            .then(|| at.machine_zero.map_or(f32::INFINITY, |z| at.zero[2] - z[2])),
    };
    for problem in walk.collisions(name, setup, &sims, a_before(doc, si)) {
        if !found.contains(&problem) {
            found.push(problem);
        }
    }
    Ok(found)
}

/// What the tool's way through an indexed setup is checked against.
struct Walk<'a> {
    rig: &'a Rig,
    /// Each operation's tool in the spindle and what of it is assumed
    /// (`in_spindle`), by its index in the setup.
    tools: std::collections::HashMap<usize, (InSpindle, String)>,
    gap: f32,
    blend: f32,
    /// The work zero in the setup's frame, for the messages.
    datum: [f32; 3],
    /// The height the machine retracts to between operations (setup frame):
    /// machine Z home, or None for the clearance plane.
    home: Option<f32>,
}
impl Walk<'_> {
    fn hits_at(&self, a: f32, segments: &[([f32; 3], [f32; 3])], oi: usize) -> Vec<Hit> {
        self.rig
            .hits_at(a, segments, self.tools[&oi].0, self.gap + self.blend)
    }
    /// What of operation `oi`'s tool is assumed, ending a message about a
    /// hit by `part`: nothing when the cutter itself hits.
    fn assumed(&self, oi: usize, part: &str) -> &str {
        match part {
            "tool" => "",
            _ => &self.tools[&oi].1,
        }
    }
    /// One problem per operation, kind of move, box and part of the tool.
    fn collisions(
        &self,
        name: &str,
        setup: &crate::model::Setup,
        sims: &[(usize, &Simulation)],
        start: f32,
    ) -> Vec<Held> {
        // (operation, kind of move, hit, message), the first of each kept.
        let mut hits: Vec<(usize, &str, Hit, String)> = vec![];
        let mut planes = vec![];
        let cut = |a: f32, oi: usize, op: &str, hit: &Hit| {
            let said = message(
                &format!("{name} at A {}°, {op}", degrees(a)),
                hit,
                self.gap,
                self.datum,
            );
            format!("{said}{}", self.assumed(oi, hit.part))
        };
        let (mut a_now, mut tip): (f32, Option<[f32; 3]>) = (start, None);
        let last = sims.len().saturating_sub(1);
        for (n, (oi, sim)) in sims.iter().enumerate() {
            let (oi, plan) = (*oi, &sim.plan);
            let op = setup.operations[oi].name();
            let Some(begin) = plan.moves.first().map(|m| m.from) else {
                continue;
            };
            // The way in: from the last operation's end, or from wherever
            // the tool was.
            let way: Vec<([f32; 3], [f32; 3])> = match (tip, self.home) {
                (Some(end), home) => {
                    let h = home.unwrap_or(end[2].max(begin[2]));
                    let (up, over) = ([end[0], end[1], h], [begin[0], begin[1], h]);
                    vec![(end, up), (up, over), (over, begin)]
                }
                (None, Some(h)) => vec![([begin[0], begin[1], h], begin)],
                (None, None) => {
                    // Arriving sideways at its height, it has to be over every box.
                    let boxes = self.rig.boxes(a_now);
                    let highest = boxes
                        .iter()
                        .fold(None::<&crate::holding::HoldBox>, |best, b| {
                            best.filter(|x| x.bounds.max[2] >= b.bounds.max[2])
                                .or(Some(b))
                        });
                    if let Some(b) = highest
                        && begin[2] <= b.bounds.max[2] + self.gap + self.blend
                    {
                        let plane = plane_message(
                            &b.label,
                            begin[2],
                            self.gap,
                            self.blend,
                            b.bounds.max[2],
                            self.datum[2],
                            plan.stock.max[2],
                        );
                        planes.push(Held {
                            operation: Some(oi),
                            message: format!("{name} at A {}°, {op}: {plane}", degrees(a_now)),
                        });
                    }
                    vec![]
                }
            };
            for hit in self.hits_at(a_now, &way, oi) {
                let message = cut(a_now, oi, op, &hit);
                hits.push((oi, "way", hit, message));
            }
            // The moves, a run of them at each angle, and each turn of A.
            let mut run: Vec<([f32; 3], [f32; 3])> = vec![];
            let mut moves = plan.moves.iter().peekable();
            while let Some(m) = moves.next() {
                let a = m.a.unwrap_or(a_now);
                if m.motion == Motion::Index || (a - a_now).abs() > 1e-4 {
                    let tool = self.tools[&oi].0;
                    let turn = [(m.from, m.to)];
                    for hit in self
                        .rig
                        .hits_turning(a_now, a, &turn, tool, self.gap + self.blend)
                    {
                        let at = |k: usize| mm(hit.at[k] - self.datum[k]);
                        let message = format!(
                            "{name}, {op}: the index move from A {}° to A {}° brings the {} within the {} mm gap of the {} at X{} Y{} Z{}.{}",
                            degrees(a_now),
                            degrees(a),
                            hit.part,
                            mm(self.gap),
                            hit.label,
                            at(0),
                            at(1),
                            at(2),
                            self.assumed(oi, hit.part),
                        );
                        hits.push((oi, "index", hit, message));
                    }
                    a_now = a;
                    if m.motion == Motion::Index {
                        continue;
                    }
                }
                run.push((m.from, m.to));
                // A run ends where A is about to turn, and with the moves.
                let ends = moves.peek().is_none_or(|next| {
                    next.motion == Motion::Index || next.a.is_some_and(|b| (b - a_now).abs() > 1e-4)
                });
                if ends {
                    for hit in self.hits_at(a_now, &run, oi) {
                        let message = cut(a_now, oi, op, &hit);
                        hits.push((oi, "cut", hit, message));
                    }
                    run.clear();
                }
            }
            tip = plan.moves.last().map(|m| m.to);
            // The way out of the last: up to machine Z home.
            if n == last
                && let (Some(end), Some(h)) = (tip, self.home)
            {
                for hit in self.hits_at(a_now, &[(end, [end[0], end[1], h])], oi) {
                    let message = cut(a_now, oi, op, &hit);
                    hits.push((oi, "way", hit, message));
                }
            }
        }
        let mut seen = HashSet::new();
        planes
            .into_iter()
            .chain(
                hits.into_iter()
                    .filter(|(oi, _, hit, _)| seen.insert((*oi, hit.label.clone(), hit.part)))
                    .map(|(oi, _, _, message)| Held {
                        operation: Some(oi),
                        message,
                    }),
            )
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::indexed::tests::{bar_job, contour, cutter, prism};
    use crate::simulation::{Move, prepare};

    /// `indexed`'s bar job: the part in a Ø40 × 120 bar 80 mm out of a 3-jaw
    /// chuck, on an A centreline at machine Z `centre_z`, setup 2 a contour
    /// indexed to `angles`; and setup 2's simulation.
    fn job(centre_z: &str, angles: &str) -> (Document, Mesh, Simulation) {
        let mesh = prism(30.);
        let doc = bar_job(&mesh, 0., "80", centre_z, angles, contour(&cutter("12")));
        let sim = prepare(&doc, &mesh, 1, 0).unwrap();
        (doc, mesh, sim)
    }
    /// `sim` with its moves replaced: (from, to, motion, A), in the setup's frame.
    fn with(sim: &Simulation, moves: &[([f32; 3], [f32; 3], Motion, f32)]) -> Simulation {
        let mut sim = sim.clone();
        sim.plan.moves = moves
            .iter()
            .map(|&(from, to, motion, a)| Move {
                from,
                to,
                motion,
                cutting_pass: motion == Motion::Feed,
                feed: 100.,
                start: 0.,
                end: 1.,
                a: Some(a),
            })
            .collect();
        sim
    }
    /// The point of setup 2's frame at machine `p`.
    fn frame(doc: &Document, mesh: &Mesh, p: [f32; 3]) -> [f32; 3] {
        let at = doc.at_angle(1, 90., mesh).unwrap();
        let zero = at.machine_zero.unwrap();
        std::array::from_fn(|k| p[k] - zero[k] + at.zero[k])
    }
    fn found(doc: &Document, mesh: &Mesh, sim: Simulation) -> Vec<String> {
        problems(doc, mesh, 1, &[(0, Ok(sim))])
            .into_iter()
            .map(|(_, m)| m)
            .collect()
    }
    /// One feed move from machine (300, 300, -100), well clear of the bar,
    /// to machine `to` at A `a`.
    fn reach(doc: &Document, mesh: &Mesh, sim: &Simulation, to: [f32; 3], a: f32) -> Vec<String> {
        let from = frame(doc, mesh, [300., 300., -100.]);
        let to = frame(doc, mesh, to);
        found(doc, mesh, with(sim, &[(from, to, Motion::Feed, a)]))
    }

    #[test]
    fn every_move_is_checked_on_both_sides_of_the_machines_travel_and_a_range() {
        // Machine::new's travel: X 600, Y 400, Z 300; A -360° to 360°.
        let (doc, mesh, sim) = job("-150", "90");
        // The job as generated is inside every limit.
        assert!(found(&doc, &mesh, sim.clone()).is_empty());
        let x = |v| [v, 300., -100.];
        let y = |v| [300., v, -100.];
        let z = |v| [300., 300., v];
        for (to, a, refused) in [
            (
                x(-0.01),
                90.,
                Some(
                    "Flats at A 90°, Contour: the tool would reach machine X -0.01, 0.01 mm outside the machine's 600 mm X travel, 0 to 600 mm.",
                ),
            ),
            (x(0.01), 90., None),
            (
                x(600.01),
                90.,
                Some("machine X 600.01, 0.01 mm outside the machine's 600 mm X travel"),
            ),
            (x(599.99), 90., None),
            (
                y(-0.01),
                90.,
                Some(
                    "machine Y -0.01, 0.01 mm outside the machine's 400 mm Y travel, 0 to 400 mm.",
                ),
            ),
            (y(0.01), 90., None),
            (y(400.01), 90., Some("machine Y 400.01")),
            (y(399.99), 90., None),
            (
                z(-300.01),
                90.,
                Some(
                    "the cut's bottom is at machine Z -300.01, 0.01 mm short: below Z -300, the bottom of the machine's 300 mm Z travel. The Ø40 mm bar is centred at machine Z -150, so this cut goes 150.01 mm below its centreline.",
                ),
            ),
            (z(-299.99), 90., None),
            (
                z(0.01),
                90.,
                Some(
                    "the tool goes up to machine Z 0.01, 0.01 mm above Z 0, the top of the machine's 300 mm Z travel. The Ø40 mm bar's top is at machine Z -130 (its centreline at Z -150 + 20 mm), so the retract over it is out of reach.",
                ),
            ),
            (z(-0.01), 90., None),
            (
                z(-100.),
                360.01,
                Some(
                    "Flats at A 360.01°, Contour: A 360.01° is outside the machine's A range, -360° to 360°.",
                ),
            ),
            (z(-100.), 359.99, None),
            (
                z(-100.),
                -360.01,
                Some("A -360.01° is outside the machine's A range"),
            ),
            (z(-100.), -359.99, None),
        ] {
            let said = reach(&doc, &mesh, &sim, to, a);
            match refused {
                Some(words) => {
                    assert_eq!(said.len(), 1, "{to:?} {a}: {said:?}");
                    assert!(said[0].contains(words), "{to:?} {a}: {said:?}");
                }
                None => assert!(said.is_empty(), "{to:?} {a}: {said:?}"),
            }
        }
    }

    #[test]
    fn a_cut_too_low_whose_retract_is_too_high_is_refused_at_both_ends() {
        let (doc, mesh, sim) = job("-150", "90");
        let said = reach(&doc, &mesh, &sim, [300., 300., 0.01], 90.);
        assert_eq!(said.len(), 1, "{said:?}");
        let (from, to) = (
            frame(&doc, &mesh, [300., 300., 0.01]),
            frame(&doc, &mesh, [300., 300., -300.01]),
        );
        let said = found(&doc, &mesh, with(&sim, &[(from, to, Motion::Feed, 90.)]));
        assert_eq!(said.len(), 2, "{said:?}");
        assert!(
            said[0].contains("the cut's bottom is at machine Z -300.01")
                && said[1].contains("the tool goes up to machine Z 0.01"),
            "{said:?}"
        );
    }

    /// LIO-207: the centreline's Z is the probe's tip's, so a tool longer
    /// than the probe holds the spindle that much higher over the same cut.
    /// With a 50 mm probe, a tool 50 mm out of a 15 mm holder is 15 mm
    /// longer: its Z travel is checked 15 mm higher; with a 60 mm probe and
    /// no holder, 10 mm lower; without a probe length, as long as the
    /// probe, and said so.
    #[test]
    fn the_z_travel_is_checked_with_the_tools_length_against_the_probes() {
        let (mut doc, mesh, sim) = job("-150", "90");
        let note = "Assumed: the tool as long as the probe the A centreline was probed with";
        let said = reach(&doc, &mesh, &sim, [300., 300., 0.01], 90.);
        assert!(said.len() == 1 && said[0].contains(note), "{said:?}");
        for (probe, holder, longer) in [("50", "15", 15.), ("60", "0", -10.)] {
            (doc.setups[0].machine.p).insert("probe_length".into(), probe.into());
            (doc.tools[0].p).insert("holder_length".into(), holder.into());
            // The tip's machine Z as the probe's would be there.
            for (tip, refused) in [
                (
                    0.01 - longer,
                    Some("the tool goes up to machine Z 0.01, 0.01 mm above Z 0"),
                ),
                (-0.01 - longer, None),
                (
                    -300.01 - longer,
                    Some("the cut's bottom is at machine Z -300.01, 0.01 mm short"),
                ),
                (-299.99 - longer, None),
            ] {
                let said = reach(&doc, &mesh, &sim, [300., 300., tip], 90.);
                match refused {
                    Some(words) => {
                        assert_eq!(said.len(), 1, "{longer} {tip}: {said:?}");
                        assert!(
                            said[0].contains(words)
                                && said[0].contains(&format!(
                                    "Its tip is taken {} mm under the probe's: its 50 mm stick-out and {holder} mm holder less the probe's {probe} mm length.",
                                    mm(longer)
                                ))
                                && !said[0].contains(note),
                            "{longer} {tip}: {said:?}"
                        );
                    }
                    None => assert!(said.is_empty(), "{longer} {tip}: {said:?}"),
                }
            }
        }
    }

    #[test]
    fn a_uc100s_profile_soft_limits_are_kept_too() {
        let (mut doc, mesh, sim) = job("-150", "90");
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("jy5800-rotary.toml");
        std::fs::write(&path, crate::control::rotary_profile()).unwrap();
        let m = &mut doc.setups[0].machine;
        m.p.insert("controller".into(), "UC100".into());
        m.p.insert("uc100_profile".into(), path.display().to_string());
        doc.sync_indexed();
        // The profile: X 0 to 390, A -180° to 180°; the machine's own
        // travel and A range are wider.
        for (to, a, refused) in [
            (
                [390.01, 200., -30.],
                90.,
                Some(
                    "machine X 390.01, 0.01 mm outside the UC100 profile's X soft limits, 0 to 390 mm.",
                ),
            ),
            ([389.99, 200., -30.], 90., None),
            (
                [300., 200., -55.01],
                90.,
                Some(
                    "the cut's bottom is at machine Z -55.01, 0.01 mm short: below Z -55, the bottom of the UC100 profile's Z soft limits. The Ø40 mm bar is centred at machine Z -150. Cut it",
                ),
            ),
            ([300., 200., -54.99], 90., None),
            (
                [300., 200., -30.],
                180.01,
                Some("A 180.01° is outside the UC100 profile's A soft limits, -180° to 180°."),
            ),
            ([300., 200., -30.], 179.99, None),
        ] {
            let from = frame(&doc, &mesh, [300., 200., -30.]);
            let said = found(
                &doc,
                &mesh,
                with(&sim, &[(from, frame(&doc, &mesh, to), Motion::Feed, a)]),
            );
            match refused {
                Some(words) => assert!(said.len() == 1 && said[0].contains(words), "{said:?}"),
                None => assert!(said.is_empty(), "{to:?} {a}: {said:?}"),
            }
        }
        // A profile that cannot be read: nothing is taken as inside it.
        doc.setups[0]
            .machine
            .p
            .insert("uc100_profile".into(), "/nowhere.toml".into());
        doc.sync_indexed();
        let said = found(&doc, &mesh, sim);
        assert!(
            said[0].starts_with("Flats: its moves cannot be checked against the soft limits"),
            "{said:?}"
        );
    }

    #[test]
    fn the_holder_coming_down_onto_the_bar_is_refused_with_how_much() {
        // The tool is 50 mm out of its holder and the gap is 2 mm; the Ø40
        // bar's top is at Z 40.
        let (doc, mesh, sim) = job("-150", "90");
        let into = "the cut goes 50.01 mm below the bar's top there, and the tool's 50 mm stick-out leaves the holder 0.01 mm into it. The tool needs 2.01 mm more stick-out, or a shallower cut.";
        let near = "the cut goes 48.01 mm below the bar's top there, and the tool's 50 mm stick-out leaves the holder only 1.99 mm over it, inside the 2 mm gap. The tool needs 0.01 mm more stick-out, or a shallower cut.";
        for (deep, refused) in [(50.01, Some(into)), (48.01, Some(near)), (47.99, None)] {
            let (from, to) = ([0., 0., 60.], [10., 0., 40. - deep]);
            let said = found(&doc, &mesh, with(&sim, &[(from, to, Motion::Feed, 90.)]));
            match refused {
                Some(words) => {
                    assert_eq!(said.len(), 1, "{said:?}");
                    let start = "Flats at A 90°, Contour: the holder would come down onto the Ø40 mm bar at X-50 Y0: ";
                    assert_eq!(said[0], format!("{start}{words}"));
                }
                None => assert!(said.is_empty(), "{said:?}"),
            }
        }
        // Beside the bar, past the spindle nose's reach (Ø65, the wider)
        // and the gap, it is clear; inside it, the nose comes down onto it.
        let r = 65. / 2.;
        for (y, clear) in [(20. + r + 2.01, true), (20. + r + 1.99, false)] {
            let beside = [0., y, -40.];
            let said = found(
                &doc,
                &mesh,
                with(&sim, &[(beside, beside, Motion::Feed, 90.)]),
            );
            match clear {
                true => assert!(said.is_empty(), "{said:?}"),
                false => assert!(
                    said.len() == 1
                        && said[0].contains(
                            ": the spindle nose (Ø65 mm) would come down onto the Ø40 mm bar"
                        ),
                    "{said:?}"
                ),
            }
        }
    }

    #[test]
    fn a_cut_out_of_the_holders_reach_is_reachable_with_more_stick_out() {
        // A plunge over the bar's axis 55 mm under its top (Z 40): with 50
        // mm of stick-out the holder comes 5 mm into the bar, 7 with the
        // gap; with 7.01 mm more it is clear, the nose 10 mm higher still.
        let (doc, mesh, sim) = job("-150", "90");
        let plunge = |stickout: f32| {
            let mut sim = with(
                &sim,
                &[([10., 0., 60.], [10., 0., -15.], Motion::Feed, 90.)],
            );
            sim.plan.tool_length = stickout;
            found(&doc, &mesh, sim)
        };
        let said = plunge(50.);
        assert_eq!(said.len(), 1, "{said:?}");
        assert!(
            said[0].ends_with("the cut goes 55 mm below the bar's top there, and the tool's 50 mm stick-out leaves the holder 5 mm into it. The tool needs 7 mm more stick-out, or a shallower cut."),
            "{said:?}"
        );
        let said = plunge(56.99);
        assert!(
            said.len() == 1
                && said[0].ends_with("The tool needs 0.01 mm more stick-out, or a shallower cut."),
            "{said:?}"
        );
        assert!(plunge(57.01).is_empty(), "{:?}", plunge(57.01));
    }

    #[test]
    fn the_spindle_nose_coming_down_onto_the_bar_is_refused_on_both_sides() {
        // A Ø10 holder 10 mm long (given), 50 mm of stick-out: the nose is
        // 60 mm over the tip. A plunge 27 mm beside the bar's axis keeps
        // the holder (5 + 2 mm of gap) off the Ø40 bar, not the Ø65 nose.
        let (mut doc, mesh, sim) = job("-150", "90");
        doc.tools[0].p.insert("holder_length".into(), "10".into());
        let plunge_at = |doc: &Document, y: f32, bottom: f32| {
            let mut sim = with(
                &sim,
                &[([10., y, 80.], [10., y, bottom], Motion::Feed, 90.)],
            );
            sim.plan.holder_radius = 5.;
            found(doc, &mesh, sim)
        };
        let plunge = |doc: &Document, bottom: f32| plunge_at(doc, 27., bottom);
        let start = "Flats at A 90°, Contour: the spindle nose (Ø65 mm) would come down onto the Ø40 mm bar at X-50 Y27: the cut goes ";
        for (bottom, refused) in [
            (
                -20.01,
                Some(
                    "60.01 mm below the bar's top there, and the nose is 60 mm over the tip (the tool's 50 mm stick-out and the holder's 10 mm), so it hits the bar 0.01 mm before the cut's bottom. The tool needs 2.01 mm more stick-out (or a longer holder), or a shallower cut.",
                ),
            ),
            (
                -18.01,
                Some(
                    "58.01 mm below the bar's top there, and the nose is 60 mm over the tip (the tool's 50 mm stick-out and the holder's 10 mm), so at the cut's bottom it is only 1.99 mm over the bar, inside the 2 mm gap. The tool needs 0.01 mm more stick-out (or a longer holder), or a shallower cut.",
                ),
            ),
            (-17.99, None),
        ] {
            let said = plunge(&doc, bottom);
            match refused {
                Some(words) => {
                    assert_eq!(said.len(), 1, "{said:?}");
                    assert_eq!(said[0], format!("{start}{words}"));
                }
                None => assert!(said.is_empty(), "{bottom}: {said:?}"),
            }
        }
        // A longer holder lifts the nose by as much.
        doc.tools[0]
            .p
            .insert("holder_length".into(), "12.02".into());
        assert!(plunge(&doc, -20.01).is_empty());
        // A tool without a holder length takes 10 mm, a machine without a
        // spindle nose Ø65: both said to be assumed.
        doc.tools[0].p.remove("holder_length");
        for s in &mut doc.setups {
            s.machine.p.remove("a_nose_diameter");
        }
        let said = plunge(&doc, -18.01);
        assert!(
            said.len() == 1 && said[0].ends_with("or a shallower cut. Assumed: the holder's 10 mm length (the tool gives none: enter it in the Tool library) and the spindle nose's Ø65 mm (the machine gives none: enter it in the Machine library)."),
            "{said:?}"
        );
        // A wider nose reaches the bar from farther off: 55 mm beside its
        // axis, past the Ø65 nose and the gap, the Ø80 one is over it.
        assert!(plunge_at(&doc, 55., -30.).is_empty());
        for s in &mut doc.setups {
            s.machine.p.insert("a_nose_diameter".into(), "80".into());
        }
        let said = plunge_at(&doc, 55., -30.);
        assert!(
            said.len() == 1 && said[0].contains("the spindle nose (Ø80 mm)"),
            "{said:?}"
        );
    }

    #[test]
    fn an_index_move_inside_what_a_swings_is_refused() {
        let (doc, mesh, sim) = job("-150", "90");
        let swing = doc.swing(1, &mesh, 0.).unwrap();
        for (z, refused) in [(swing.z - 0.01, true), (swing.z + 0.01, false)] {
            let p = [30., 0., z];
            let said = found(&doc, &mesh, with(&sim, &[(p, p, Motion::Index, 90.)]));
            if refused {
                assert_eq!(said.len(), 1, "{said:?}");
                assert!(
                    said[0].contains(
                        "the index move from A 0° to A 90° turns A with the tool tip at Z"
                    ),
                    "{said:?}"
                );
                assert!(said[0].contains("0.01 mm inside the 55.9 mm that the Ø40 mm bar, the part and the chuck's jaws reach"), "{said:?}");
            } else {
                assert!(said.is_empty(), "{said:?}");
            }
        }
    }

    #[test]
    fn too_little_room_over_what_a_swings_is_refused_with_the_numbers() {
        // The jaws reach 55.9 mm from the centreline: a retract over them
        // needs 5 mm under the top of the Z travel.
        let (mut doc, mesh, _) = job("-150", "90");
        let radius = doc.swing(1, &mesh, 0.).unwrap().radius;
        for (centre, room) in [
            (-(5. + radius) + 0.01, Some("4.99")),
            (-(5. + radius) - 0.01, None),
        ] {
            doc.setups[0]
                .machine
                .p
                .insert("a_centre_z".into(), centre.to_string());
            doc.sync_indexed();
            // Its operation is refused when generated (15 mm of clearance
            // over the jaws is above the Z travel): the room is said anyway.
            assert!(prepare(&doc, &mesh, 1, 0).is_err());
            let said: Vec<String> = problems(&doc, &mesh, 1, &[])
                .into_iter()
                .map(|(_, m)| m)
                .collect();
            match room {
                Some(room) => {
                    assert_eq!(said.len(), 1, "{said:?}");
                    assert!(said[0].starts_with(&format!("Flats: A turns all within 55.9 mm of its centreline (the Ø40 mm bar, the part and the chuck's jaws at any angle), which at machine Z {} reaches machine Z -4.99: {room} mm under the top of the machine's 300 mm Z travel (Z 0), less than the 5 mm a retract over it needs.", mm(centre))), "{said:?}");
                }
                None => assert!(said.is_empty(), "{said:?}"),
            }
        }
    }

    #[test]
    fn the_assumed_geometry_leaves_no_room_over_the_largest_part() {
        // LIO-184's comment on LIO-191: the assumed rotary unit puts the A
        // centreline at machine Z -40 and takes a part up to Ø80. Its top
        // is at Z 0, the top of the CNC4030's 55 mm Z travel: no room to
        // lift the tool over it.
        let dir = tempfile::tempdir().unwrap();
        let machine = crate::control::rotary_machine(dir.path()).unwrap();
        let unit = machine.rotary_unit().unwrap();
        assert_eq!(
            (unit.centre_z, unit.fit_diameter, unit.grip_max),
            (-40., 80., 80.)
        );
        let mesh = prism(30.);
        // A contour from the bar's top to 15 mm over the part's bottom.
        let mut op = contour(&cutter("12"));
        op.p.insert("bottom".into(), "15".into());
        let mut doc = bar_job(&mesh, 0., "80", "-40", "0", op);
        doc.setups[0].machine = machine;
        doc.setups[0].p.insert("stock_diameter".into(), "80".into());
        // The part on the bar's axis (Z40).
        let s = mesh.display_scale();
        let tz = 40. - mesh.dimensions[2] / 2. - 4. / s;
        doc.setups[0].p.insert("tz".into(), tz.to_string());
        doc.sync_indexed();
        doc.sync_stock_sources();
        doc.sync_turns(Some(&mesh));
        // At A0 alone it is generated: its retract over the bar is refused.
        let sim = prepare(&doc, &mesh, 1, 0).unwrap();
        let said = found(&doc, &mesh, sim);
        assert!(
            said[0].starts_with("Flats: Index moves turn A with the tool above all that A swings: at machine Z 35.66 (the A centreline at Z -40 + 75.66 mm, as far as the chuck's jaws reach from it at any angle, + 0 mm clearance). That is 35.66 mm above the top of the machine's 55 mm Z travel (Z 0)"),
            "{said:?}"
        );
        assert!(
            said.iter().any(|s| s.contains("The Ø80 mm bar's top is at machine Z 0 (its centreline at Z -40 + 40 mm), so the retract over it is out of reach.")),
            "{said:?}"
        );
        // Turning A is refused when it is generated, with its numbers.
        doc.setups[1]
            .p
            .insert(crate::indexed::ANGLES.into(), "0, 90".into());
        doc.sync_turns(Some(&mesh));
        let refused = prepare(&doc, &mesh, 1, 0).unwrap_err();
        assert!(
            refused.contains("at machine Z 50.66 (the A centreline at Z -40 + 75.66 mm"),
            "{refused}"
        );
        assert!(
            refused.contains("above the top of the machine's 55 mm Z travel (Z 0)"),
            "{refused}"
        );
    }

    /// Down into the place jaw 3 is at A90 (up and to +Y), from over all A
    /// swings, at A `a`, after turning there from A0.
    fn into_jaw_3(sim: &Simulation, a: f32) -> Simulation {
        let (high, over, inside) = ([-32., 20., 150.], [-32., 20., 60.], [-32., 20., 54.]);
        with(
            sim,
            &[
                (high, high, Motion::Index, a),
                (high, over, Motion::Rapid, a),
                (over, inside, Motion::Feed, a),
            ],
        )
    }
    fn held_of(doc: &Document, mesh: &Mesh, sim: Simulation) -> Vec<String> {
        held(doc, mesh, 1, &[(0, Ok(sim))])
            .unwrap()
            .into_iter()
            .map(|h| h.message)
            .collect()
    }

    #[test]
    fn a_cut_at_a_90_into_a_jaw_is_refused_at_that_angle() {
        let (doc, mesh, sim) = job("-150", "90");
        let said = held_of(&doc, &mesh, into_jaw_3(&sim, 90.));
        assert!(!said.is_empty(), "{said:?}");
        assert!(
            said.iter().any(|s| s.starts_with(
                "Flats at A 90°, Contour: the tool hits the chuck jaw 3 at X-92.0 Y20.0 Z54.6"
            ) && s.ends_with("(within the 2 mm gap).")),
            "{said:?}"
        );
        // The same cut at A0, where jaw 1 is straight up and jaw 3 below:
        // clear of them all.
        assert!(held_of(&doc, &mesh, into_jaw_3(&sim, 0.)).is_empty());
    }

    #[test]
    fn an_index_move_through_the_jaws_is_refused_over_its_turn() {
        let (doc, mesh, sim) = job("-150", "90");
        // A turns 0 to 90 with the tool 30 mm over the axis, over the jaws.
        let p = [-32., 0., 50.];
        let said = held_of(&doc, &mesh, with(&sim, &[(p, p, Motion::Index, 90.)]));
        assert!(
            said.iter().any(|s| s.starts_with("Flats, Contour: the index move from A 0° to A 90° brings the tool within the 2 mm gap of the chuck jaw")),
            "{said:?}"
        );
    }

    #[test]
    fn a_chuck_setup_at_a0_keeps_the_spindle_nose_off_the_jaws() {
        // Setup 1 cuts the bar in the chuck at A0, with the contour's tool
        // (50 mm out of a Ø32 holder, the holder 10 mm long, assumed): the
        // Ø65 nose is 60 mm over the tip. A plunge to Z10 beside jaw 1's
        // front (it stands up to Z75 at A0) brings the nose, not the tool
        // or the holder, within the 2 mm gap of it.
        let (mut doc, mesh, _) = job("-150", "90");
        doc.tools[0].p.insert("number".into(), "1".into());
        doc.setups[0].operations = doc.setups[1].operations.clone();
        let sim = prepare(&doc, &mesh, 0, 0).unwrap();
        let stock = doc.setup_stock(0, Some(&mesh)).unwrap();
        let rig = doc.chuck_rig(&doc.setups[0], stock).unwrap();
        let jaw = rig
            .boxes(0.)
            .into_iter()
            .find(|b| b.label == "chuck jaw 1")
            .unwrap();
        assert!((jaw.bounds.max[2] - 75.).abs() < 0.1, "{:?}", jaw.bounds);
        let front = jaw.bounds.max[0];
        // The gap and the machine's 0.01 mm blend.
        for (x, hit) in [(front + 32.5 + 2.02, false), (front + 32.5 + 2., true)] {
            let (high, low) = ([x, 0., 150.], [x, 0., 10.]);
            let said: Vec<String> = crate::post::held(
                &doc,
                &mesh,
                0,
                &[(0, Ok(with(&sim, &[(high, low, Motion::Feed, 0.)])))],
            )
            .into_iter()
            .map(|h| h.message)
            .collect();
            assert!(
                !said
                    .iter()
                    .any(|s| s.contains("the tool hits") || s.contains("the holder hits")),
                "{said:?}"
            );
            let nose = said.iter().any(|s| {
                s.starts_with("Contour: the spindle nose hits the chuck jaw 1 at ")
                    && s.ends_with("(within the 2 mm gap). Assumed: the holder's 10 mm length (the tool gives none: enter it in the Tool library).")
            });
            assert_eq!(nose, hit, "{said:?}");
        }
    }

    #[test]
    fn a_setup_that_is_not_indexed_is_not_checked_here() {
        // Setup 1 holds the bar in the chuck at A0: its checks are the
        // 3-axis ones, unchanged.
        let (doc, mesh, sim) = job("-150", "90");
        assert!(problems(&doc, &mesh, 0, &[(0, Ok(sim))]).is_empty());
    }
}
