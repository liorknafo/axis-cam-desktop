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
use crate::model::{Document, HOLDER_LENGTH, Machine, NOSE_DEFAULT, RotaryUnit, get, num};
use crate::post::Held;
use crate::post_check::machine_mm;
use crate::simulation::{Motion, Plan, Simulation};
use std::collections::HashSet;
use std::f32::consts::PI;

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
    id: &str,
    plan: &Plan,
    unit: Option<RotaryUnit>,
) -> (InSpindle, String) {
    let (holder_length, holder_assumed) = (doc.tools.iter().find(|t| t.id == id))
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
/// Then, in any setup on the bar in the chuck, indexed or not, a cut that
/// parts the stock off the bar the chuck holds, or leaves it on a web too
/// thin to hold it against the cut (`parted`, LIO-208, LIO-211). Empty for
/// a setup that is neither.
pub fn problems(
    doc: &Document,
    mesh: &Mesh,
    si: usize,
    sims: &Sims,
) -> Vec<(Option<usize>, String)> {
    let mut out = placed(doc, mesh, si, sims);
    out.extend(parted(doc, mesh, si, sims));
    // What the machine's clearances refuse in any setup, with this module's
    // Z reach left to it (LIO-252, `envelope`).
    out.extend(crate::envelope::problems(doc, mesh, si, sims));
    out
}

/// How far the stock of setup `base` reaches from its axis in the chuck and
/// along X (`Bar`), for `envelope`.
pub(crate) fn bar_reach(doc: &Document, base: usize, mesh: &Mesh) -> Option<(f32, [f32; 2])> {
    Bar::of(doc, base, mesh).map(|b| (b.radius, b.x))
}

/// `problems` of an indexed setup's moves on the machine.
fn placed(doc: &Document, mesh: &Mesh, si: usize, sims: &Sims) -> Vec<(Option<usize>, String)> {
    let setup = &doc.setups[si];
    let Some(base) = doc.index_base(si).filter(|_| is_indexed(setup)) else {
        return vec![];
    };
    let name = setup.name();
    let machine = &doc.setups[base].machine;
    let mut out = vec![];
    // The stock and the chuck must be known to tell what A swings; how
    // high each operation turns A is told at its own cuts below.
    if let Err(e) = doc.swing_unchecked(si, mesh, 0.) {
        out.push((None, format!("{name}: {e}")));
    }
    let bar = Bar::of(doc, base, mesh);
    let words = bar.as_ref().map_or("stock".into(), |b| b.words.clone());
    let unit = doc.rotary_unit(machine);
    let travel_z = machine_mm(machine, "z");
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
        let (tool, assumed) = in_spindle(doc, &setup.operations[oi].tool, plan, unit);
        // Room to turn A over what it swings under the tool where the
        // operation keeps it, with no clearance at all, as its index moves
        // are made (`Document::index_height`).
        let (parts, _) = crate::post::tool_parts(doc, setup, &setup.operations[oi].tool, plan);
        let over = crate::indexed::cut_span([&plan.moves[..]]);
        let swing = match doc.index_height(si, mesh, 0., over, &parts) {
            Ok(swing) => Some(swing),
            Err(e) => {
                out.push((Some(oi), format!("{name}, {op}: {e}")));
                None
            }
        };
        if let (Some(swing), Some(unit)) = (swing, unit) {
            let room = -swing.machine_z;
            let least = crate::control::RETRACT_ROOM as f32;
            if room < least {
                out.push((Some(oi), format!(
                    "{name}, {op}: A turns all within {} mm of its centreline under the tool ({} that far at any angle), which at machine Z {} reaches machine Z {}: {} mm under the top of the machine's {} mm Z travel (Z 0), less than the {} mm a retract over it needs. There is no room to lift the tool over it between angles. Measure the A centreline (Rotary A on the Control page), or use smaller stock.",
                    mm(swing.radius),
                    swing.what,
                    mm(unit.centre_z),
                    mm(swing.machine_z),
                    mm(room),
                    mm(travel_z),
                    mm(least),
                )));
            }
        }
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
                // Its height wherever the tool turns A, the operation's
                // cuts and where the index move is (`post::index_problems`).
                let at = over.map(|[lo, hi]| [lo.min(m.from[0]), hi.max(m.from[0])]);
                if index.is_none()
                    && swing.is_some()
                    && let Ok(s) = doc.index_height_unchecked(si, mesh, 0., at, &parts)
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

/// How near the axis, mm, the tailstock's centre holds the bar's free end.
const CENTRE: f32 = 3.;
/// The part-off check's cells, mm on a side at most (LIO-211): the
/// simulation's are a 96th of the stock's longest side, 2.6 mm on a 250 mm
/// bar, too coarse to tell a chip from a piece or to measure a web, so the
/// check cuts its own finer grid of the same stock with the same cuts.
pub const CELL: f32 = 1.;
/// The most a loose piece of the stock may measure and still be taken for a
/// chip: mm along its longest side, and mm³. A real chip is no thicker than
/// the chip load (0.2 mm at most here), and the stock model never keeps
/// one, as it takes away all the cutter sweeps: what it leaves loose is
/// uncut stock. Over two of the check's 1 mm cells long, or two cells'
/// worth, it is a piece that can be thrown or break the tool, not what the
/// cells leave between two passes.
pub const CHIP: f32 = 2.;
pub const CHIP_VOLUME: f32 = 2.;
/// The thinnest web, mm across (`Section::across`), that is taken to hold
/// the stock beyond it at all, whatever the cut: three of the check's
/// cells, so that a web the cells barely see is never taken for a sound
/// one.
pub const WEB: f32 = 3.;

/// The stock's voxels (`volume::Volume`) as the chuck sees them: each
/// one's X along the A axis in the chuck's frame and where it is across the
/// axis, and the ones the jaws (or the tailstock's centre) hold. The jaws
/// turn with the stock, so what they hold is the same at every angle.
struct Gripped {
    x: Vec<f32>,
    /// Y and Z from the axis.
    yz: Vec<[f32; 2]>,
    r: Vec<f32>,
    seeds: Vec<usize>,
    n: [usize; 3],
    /// A voxel's length along the axis, and its area across it, mm and mm².
    along: f32,
    area: f32,
    /// The chuck's frame at A0, which the cuts are taken to.
    a0: crate::volume::Frame,
}
impl Gripped {
    /// The voxels of `v`: held are those inside the jaws' grip, between the
    /// chuck's face and the jaws' front and no more than a voxel and a half
    /// in from where the jaws close, and those the tailstock's centre holds
    /// when it supports the bar. Material behind the jaws' front nearer the
    /// axis is held only through what joins it to those.
    fn new(v: &crate::volume::Volume, a0: crate::volume::Frame, rig: &Rig) -> Self {
        let mut x = Vec::with_capacity(v.occupied.len());
        let mut yz = Vec::with_capacity(v.occupied.len());
        let mut radii = Vec::with_capacity(v.occupied.len());
        let mut seeds = vec![];
        let cell = v.cell.iter().copied().fold(0., f32::max);
        for k in 0..v.n[2] {
            for j in 0..v.n[1] {
                for i in 0..v.n[0] {
                    let q = v.frame.transfer(v.point([i, j, k], 0.5), a0);
                    let d = [q[1] - rig.axis[0], q[2] - rig.axis[1]];
                    let r = d[0].hypot(d[1]);
                    let tail = match rig.tail {
                        crate::chuck::Tail::Supports(end) => {
                            q[0] >= end - 1.5 * cell && r <= CENTRE.max(1.5 * cell)
                        }
                        _ => false,
                    };
                    let gripped =
                        (rig.face..=rig.front).contains(&q[0]) && r >= rig.inner - 1.5 * cell;
                    if gripped || tail {
                        seeds.push(x.len());
                    }
                    x.push(q[0]);
                    yz.push(d);
                    radii.push(r);
                }
            }
        }
        // The grid's axis that runs along A.
        let k = (0..3)
            .max_by(|&a, &b| {
                let along = |k: usize| {
                    let mut e = [0.; 3];
                    e[k] = 1.;
                    v.frame.transfer_vector(e, a0)[0].abs()
                };
                along(a).total_cmp(&along(b))
            })
            .unwrap_or(0);
        Self {
            x,
            yz,
            r: radii,
            seeds,
            n: v.n,
            along: v.cell[k],
            area: v.cell.iter().product::<f32>() / v.cell[k],
            a0,
        }
    }
    /// The voxels next to voxel `i` across its faces.
    fn next(&self, i: usize) -> impl Iterator<Item = usize> {
        let n = self.n;
        let p = [i % n[0], i / n[0] % n[1], i / (n[0] * n[1])];
        let step = [1, n[0], n[0] * n[1]];
        (0..3).flat_map(move |k| {
            [
                (p[k] > 0).then(|| i - step[k]),
                (p[k] + 1 < n[k]).then(|| i + step[k]),
            ]
            .into_iter()
            .flatten()
        })
    }
    /// The voxels of `filled` reached from `from` across faces through
    /// voxels `filled` keeps, marked in `reached`.
    fn fill_into(&self, from: &[usize], filled: impl Fn(usize) -> bool, reached: &mut [bool]) {
        let mut stack: Vec<usize> = from
            .iter()
            .copied()
            .filter(|&i| filled(i) && !reached[i])
            .collect();
        for &i in &stack {
            reached[i] = true;
        }
        while let Some(i) = stack.pop() {
            for j in self.next(i) {
                if !reached[j] && filled(j) {
                    reached[j] = true;
                    stack.push(j);
                }
            }
        }
    }
    /// The material of `v` joined to what the chuck holds.
    fn held(&self, v: &crate::volume::Volume) -> Vec<bool> {
        let mut reached = vec![false; self.x.len()];
        self.fill_into(&self.seeds, |i| v.occupied[i] != 0, &mut reached);
        reached
    }
    /// The pieces of `v` that were held (`before`) and are no longer
    /// (`now`), each as its voxels, larger than a chip (`CHIP`,
    /// `CHIP_VOLUME`): the largest first.
    fn loose(&self, v: &crate::volume::Volume, before: &[bool], now: &[bool]) -> Vec<Piece> {
        let freed = |i: usize| v.occupied[i] != 0 && before[i] && !now[i];
        let mut seen = vec![false; self.x.len()];
        let mut pieces = vec![];
        for i in 0..self.x.len() {
            if seen[i] || !freed(i) {
                continue;
            }
            let mut stack = vec![i];
            seen[i] = true;
            let mut voxels = vec![];
            while let Some(j) = stack.pop() {
                voxels.push(j);
                for k in self.next(j) {
                    if !seen[k] && freed(k) {
                        seen[k] = true;
                        stack.push(k);
                    }
                }
            }
            let piece = Piece::of(v, voxels);
            if piece.size.iter().copied().fold(0., f32::max) > CHIP + 1e-3
                || piece.volume > CHIP_VOLUME + 1e-3
            {
                pieces.push(piece);
            }
        }
        pieces.sort_by(|a, b| b.volume.total_cmp(&a.volume));
        pieces
    }
    /// The section across the axis of the material `held` in each slab of
    /// the stock a voxel long whose middle is beyond the jaws' `front` and
    /// short of `far`: (its X, its section).
    fn sections(&self, held: &[bool], front: f32, far: f32) -> Vec<(f32, Section)> {
        let w = self.along;
        let low = self.x.iter().copied().fold(f32::INFINITY, f32::min) - w / 2.;
        let bin = |x: f32| ((x - low) / w).floor().max(0.) as usize;
        let count = bin(self.x.iter().copied().fold(f32::NEG_INFINITY, f32::max)) + 1;
        let mut members: Vec<Vec<usize>> = vec![vec![]; count];
        for i in (0..self.x.len()).filter(|&i| held[i] && self.x[i] > front && self.x[i] < far) {
            members[bin(self.x[i])].push(i);
        }
        (members.iter().enumerate())
            .filter(|(_, m)| !m.is_empty())
            .map(|(b, m)| {
                let x = low + (b as f32 + 0.5) * w;
                (x, Section::of(m.iter().map(|&i| self.yz[i]), self.area))
            })
            .collect()
    }
}

/// A piece of the stock cut loose: its voxels, its size along the grid's
/// axes, mm, and its volume, mm³.
struct Piece {
    voxels: Vec<usize>,
    size: [f32; 3],
    volume: f32,
}
impl Piece {
    fn of(v: &crate::volume::Volume, voxels: Vec<usize>) -> Self {
        let n = v.n;
        let mut lo = [usize::MAX; 3];
        let mut hi = [0; 3];
        for &i in &voxels {
            let p = [i % n[0], i / n[0] % n[1], i / (n[0] * n[1])];
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
        Self {
            size: std::array::from_fn(|k| (hi[k] + 1 - lo[k]) as f32 * v.cell[k]),
            volume: voxels.len() as f32 * v.cell.iter().product::<f32>(),
            voxels,
        }
    }
}

/// A section of the stock across the A axis, from the voxels in it (their
/// Y and Z from the axis, each `cell` mm² across).
#[derive(Debug, Clone, Copy)]
struct Section {
    /// mm²
    area: f32,
    /// The least of its second moments about its centroid, and the polar
    /// one, mm⁴.
    least: f32,
    polar: f32,
    /// The most, over the directions a bending moment can take, of how far
    /// its material reaches from the neutral axis over its second moment
    /// about it, 1/mm³: the bending stress per N·mm.
    bending: f32,
}
impl Section {
    fn of(points: impl Iterator<Item = [f32; 2]> + Clone, cell: f32) -> Self {
        let n = points.clone().count() as f32;
        let (sy, sz) = points
            .clone()
            .fold((0., 0.), |s, p| (s.0 + p[0], s.1 + p[1]));
        let c = [sy / n, sz / n];
        // A voxel's own second moment about its middle, as a square.
        let own = n * cell * cell / 12.;
        let (mut iyy, mut izz, mut iyz) = (own, own, 0.);
        // How far the material reaches each way across a neutral axis
        // every 5°.
        let ways: Vec<[f32; 2]> = (0..36)
            .map(|k| (k as f32 * 5f32.to_radians()).sin_cos())
            .map(|(s, c)| [-s, c])
            .collect();
        let mut reach = vec![0f32; ways.len()];
        for p in points {
            let d = [p[0] - c[0], p[1] - c[1]];
            iyy += d[1] * d[1] * cell;
            izz += d[0] * d[0] * cell;
            iyz += d[0] * d[1] * cell;
            for (r, w) in reach.iter_mut().zip(&ways) {
                *r = r.max((d[0] * w[0] + d[1] * w[1]).abs());
            }
        }
        let mean = (iyy + izz) / 2.;
        // About the axis along (cos, sin): the second moment of what lies
        // across it, along `ways`.
        let bending = (ways.iter().zip(&reach))
            .map(|(w, r)| {
                let i = izz * w[0] * w[0] + iyy * w[1] * w[1] + 2. * iyz * w[0] * w[1];
                (r + cell.sqrt() / 2.) / i.max(1e-6)
            })
            .fold(0., f32::max);
        Self {
            area: n * cell,
            least: mean - ((iyy - izz) / 2.).hypot(iyz),
            polar: iyy + izz,
            bending,
        }
    }
    /// How far across it is at its thinnest, mm: the width of the rectangle
    /// of its area and least second moment (a square's side; 0.87 of a
    /// circle's diameter).
    fn across(&self) -> f32 {
        (12. * self.least / self.area).max(0.).sqrt()
    }
    /// The stress in it, N/mm², under a bending moment `m` (N·mm) in any
    /// direction and a twist `t` (N·mm): the bending stress where its
    /// material reaches farthest from the neutral axis the moment bends it
    /// about, the worst way, the shear of the twist at its width over
    /// Saint-Venant's torsion constant (A⁴ / 4π² J, the polar moment J's for
    /// a round section, less for any other), together as von Mises'.
    fn stress(&self, m: f32, t: f32) -> (f32, f32, f32) {
        let bend = m * self.bending;
        let torsion = self.area.powi(4) / (4. * PI * PI * self.polar.max(1e-6));
        let shear = t * self.across() / torsion.max(1e-6);
        (bend, shear, bend.hypot(3f32.sqrt() * shear))
    }
}
/// The side of the square web, mm, whose stress (`Section::stress`) under
/// `m` and `t` is `allowed` (N/mm²): a square's reach is its half-diagonal,
/// its least second moment a⁴/12, its torsion constant 0.152 a⁴.
fn square_for(m: f32, t: f32, allowed: f32) -> f32 {
    let torsion = 6. / (4. * PI * PI);
    let bend = m / std::f32::consts::SQRT_2 * 12.;
    let shear = t / torsion;
    (bend.hypot(3f32.sqrt() * shear) / allowed).cbrt()
}

/// A cut that parts the stock off the bar the chuck holds (LIO-208), or
/// leaves it on a web too thin to hold it (LIO-211), in setup `si` on a bar
/// in the chuck, indexed or not. The stock is followed through its `sims`
/// (`simulation::sequence`'s in-process stock across the angles), cut again
/// on a grid of `CELL` mm cells, finer than the simulation's: after each
/// angle's cuts of each operation the material still joined to what the
/// jaws grip (the stock between the chuck's face and the jaws' front where
/// they close on it) or the tailstock's centre holds is flood-filled across
/// the voxels' faces. A piece that was joined before and is not now, over a
/// chip's size (`CHIP`, `CHIP_VOLUME`), comes loose, to be thrown or to
/// break the tool: refused, naming the operation, the angle, the piece's
/// size and the X range (work coordinates) where nothing of the stock is
/// left between the bar and it. What stays joined is checked slab by slab
/// out of the jaws, as far as the angle's cuts reach: each section must be
/// at least `WEB` mm across, and hold the operation's largest cutting
/// force (`forces::op_pushes`) at the end of what it holds, as a
/// cantilever out of the section (the tailstock taken as no help) and
/// twisted by it about A, with `forces::SAFETY` against the material's
/// `forces::STRENGTH`. Refused with the web's size and the square web it
/// needs. One message for the setup. A deliberate part-off is not an
/// operation yet.
fn parted(doc: &Document, mesh: &Mesh, si: usize, sims: &Sims) -> Vec<(Option<usize>, String)> {
    let setup = &doc.setups[si];
    let base = match doc.index_base(si) {
        Some(base) => base,
        None if is_indexed(setup) => return vec![],
        None => si,
    };
    let clamped = &doc.setups[base];
    if !crate::chuck::holds(clamped) {
        return vec![];
    }
    let Some(stock) = doc.setup_stock(base, Some(mesh)) else {
        return vec![];
    };
    let (Ok(rig), Ok(at)) = (doc.chuck_rig(clamped, stock), doc.at_angle(si, 0., mesh)) else {
        return vec![];
    };
    let mut sims = sims
        .iter()
        .filter_map(|(oi, r)| r.as_ref().ok().map(|s| (*oi, s)))
        .peekable();
    let Some(volume) = sims.peek().and_then(|(_, s)| s.initial_volume.as_ref()) else {
        return vec![];
    };
    let mut fine =
        crate::volume::Volume::finer(volume.bounds, volume.frame, CELL).rounded(volume.bar_x);
    for cut in volume.cuts.iter() {
        fine.carve(cut);
    }
    let mut done = volume.cuts.len();
    let grid = Gripped::new(&fine, at.frame, &rig);
    let mut held = grid.held(&fine);
    let name = setup.name();
    let words = Bar::of(doc, base, mesh).map_or("stock".into(), |b| b.words);
    for (oi, sim) in sims {
        let mut play = sim.clone();
        play.seek(0.);
        // When each angle's cuts end, with the angle.
        let mut ends = vec![];
        let mut a_now = play.plan.moves.first().and_then(|m| m.a).unwrap_or(0.);
        let mut cut = false;
        for m in &play.plan.moves {
            if m.motion == Motion::Index {
                if cut {
                    ends.push((m.start, a_now));
                }
                cut = false;
            }
            cut |= m.motion == Motion::Feed;
            a_now = m.a.unwrap_or(a_now);
        }
        if cut {
            ends.push((play.plan.duration, a_now));
        }
        let op = setup.operations.get(oi);
        // The operation's largest push, its bending force and its twist
        // about A, N and N·mm: worked out when a web needs it.
        let mut push: Option<Option<(f32, f32)>> = None;
        let mut force = || {
            *push.get_or_insert_with(|| {
                let pushes = crate::forces::op_pushes(doc, setup, op?, &crate::forces::bites(sim))?;
                let bend = pushes.iter().map(|p| p.side.hypot(p.up)).fold(0., f32::max);
                let twist = (pushes.iter())
                    .map(|p| p.side * (p.z - rig.axis[1]).abs() + p.up * rig.outer)
                    .fold(0., f32::max);
                Some((bend, twist))
            })
        };
        for (t, a) in ends {
            play.seek(t / play.plan.duration.max(1e-6));
            let Some(v) = play.volume.as_ref() else {
                return vec![];
            };
            // The angle's cuts on the finer grid, and how far along A they
            // reach. The last one before may have grown since.
            let mut reach = f32::NEG_INFINITY;
            for c in &v.cuts[done.saturating_sub(1).min(v.cuts.len())..] {
                fine.carve(c);
                for p in [c.a, c.b] {
                    reach = reach.max(c.frame.transfer(p, grid.a0)[0] + c.radius);
                }
            }
            done = v.cuts.len();
            let now = grid.held(&fine);
            let say = |line: String| {
                vec![(
                    Some(oi),
                    format!(
                        "{name} at A {}°, {}: {line}",
                        degrees(a),
                        op.map_or(String::new(), |o| o.name().to_string())
                    ),
                )]
            };
            if let Some(piece) = grid.loose(&fine, &held, &now).first() {
                return say(loose_line(&grid, &fine, piece, at.zero[0], &words));
            }
            held = now;
            if let Some(line) = web_line(&grid, &held, &rig, reach, at.zero[0], doc, &mut force) {
                return say(line);
            }
        }
    }
    vec![]
}

/// What is said of `piece`, cut loose from the `words` the chuck holds in
/// `v`: where nothing of the stock is left on the A axis on its side
/// towards the jaws (work X, `datum` being the work zero's X in the
/// chuck's frame), its own size and place, and the cells' size.
fn loose_line(
    grid: &Gripped,
    v: &crate::volume::Volume,
    piece: &Piece,
    datum: f32,
    words: &str,
) -> String {
    let w = grid.along;
    let cell = v.cell.iter().copied().fold(0., f32::max);
    let low = grid.x.iter().copied().fold(f32::INFINITY, f32::min) - w / 2.;
    let high = grid.x.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    // Each slab of the stock across the axis, a voxel thick: whether
    // anything of it is left on the axis, within a voxel of it.
    let bin = |x: f32| ((x - low) / w).floor().max(0.) as usize;
    let mut on_axis = vec![false; bin(high) + 1];
    for i in (0..grid.x.len()).filter(|&i| v.occupied[i] != 0 && grid.r[i] <= cell) {
        on_axis[bin(grid.x[i])] = true;
    }
    let ends = (piece.voxels.iter()).fold([f32::INFINITY, f32::NEG_INFINITY], |e, &i| {
        [e[0].min(grid.x[i]), e[1].max(grid.x[i])]
    });
    let (start, end) = (ends[0] - w / 2., ends[1] + w / 2.);
    let at = |x: f32| mm(x - datum);
    // The slabs bare on the axis next to the piece's own, on the jaws' side.
    let near = (piece.voxels.iter())
        .filter(|&&i| grid.r[i] <= cell)
        .map(|&i| grid.x[i])
        .fold(f32::INFINITY, f32::min);
    let first = bin(if near.is_finite() { near } else { ends[0] });
    let mut k = first;
    while k > 0 && !on_axis[k - 1] {
        k -= 1;
    }
    // Its sides, the longest first.
    let mut size = piece.size;
    size.sort_by(|a, b| b.total_cmp(a));
    let size = size.map(mm);
    let fix = format!(
        "it comes loose, to be thrown or to break the tool. Leave a web of the stock that holds it (the stock is checked in {} mm cells: a thinner web counts as cut through, and a loose piece over {} mm long or {} mm³ is not taken for a chip); a part-off is not an operation yet.",
        mm(cell),
        mm(CHIP),
        mm(CHIP_VOLUME)
    );
    let piece = format!(
        "the {} mm piece of it ({} mm³) from X{} to X{}",
        size.join(" × "),
        piece.volume.round(),
        at(start),
        at(end)
    );
    match k < first {
        true => format!(
            "the cuts leave nothing of the {words} on the A axis from X{} to X{}, and nothing else joins {piece} to the bar the chuck holds, so {fix}",
            at(low + k as f32 * w),
            at(low + first as f32 * w),
        ),
        false => format!(
            "the cuts leave {piece} joined to nothing the {words} in the chuck holds, so {fix}"
        ),
    }
}

/// The weakest web of the material `held` (`Gripped::sections`) out of the
/// jaws of `rig` and short of `reach`, the farthest an angle's cuts went
/// along A, and what is said of it when it is under `WEB` mm across or does
/// not hold the operation's `force` (`parted`) at the end of what it holds:
/// None when every one does. Work X from `datum`.
fn web_line(
    grid: &Gripped,
    held: &[bool],
    rig: &Rig,
    reach: f32,
    datum: f32,
    doc: &Document,
    force: &mut impl FnMut() -> Option<(f32, f32)>,
) -> Option<String> {
    use crate::forces::{SAFETY, strength};
    let far = (0..grid.x.len())
        .filter(|&i| held[i])
        .map(|i| grid.x[i] + grid.along / 2.)
        .fold(f32::NEG_INFINITY, f32::max)
        .min(reach);
    let sections = grid.sections(held, rig.front, far);
    if sections.is_empty() {
        return None;
    }
    let (group, yields) = strength(doc.selected_material());
    let what = match (doc.selected_material(), group) {
        (None, _) => "the stock (no material is chosen: taken as copper, the least of the metals)"
            .to_string(),
        (_, "Other") => "the material (taken as copper, the least of the metals)".to_string(),
        (_, group) => group.to_lowercase(),
    };
    let allowed = yields / SAFETY;
    let thinnest = sections
        .iter()
        .min_by(|a, b| a.1.across().total_cmp(&b.1.across()))?;
    // The most stressed section under the force, with its stresses.
    let stressed = force().and_then(|(f, t)| {
        sections
            .iter()
            .map(|&(x, s)| (x, s, s.stress(f * (far - x), t)))
            .max_by(|a, b| a.2.2.total_cmp(&b.2.2))
            .map(|w| (w, f, t))
    });
    let at = |x: f32| mm(x - datum);
    let size = |x: f32, s: &Section| {
        format!(
            "the web the cuts leave at X{}, between the bar the chuck holds and the {} mm of it out to X{}, is {} mm² and {} mm across at its thinnest",
            at(x),
            mm(far - x),
            at(far),
            s.area.round(),
            mm(s.across())
        )
    };
    let fix = format!(
        "Leave a thicker web or take a lighter cut (the stock is checked in {} mm cells); a part-off is not an operation yet.",
        mm(CELL)
    );
    let (x, s) = *thinnest;
    if s.across() < WEB {
        let needs = match stressed {
            Some(((xs, _, _), f, t)) => format!(
                ", and to hold the cut's {} N {} mm out a square one about {} mm across",
                f.round(),
                mm(far - xs),
                mm(square_for(f * (far - xs), t, allowed).max(WEB))
            ),
            None => String::new(),
        };
        return Some(format!(
            "{}: a web under {} mm across does not hold what is beyond it{needs}. {fix}",
            size(x, &s),
            mm(WEB)
        ));
    }
    let ((x, s, (bend, shear, total)), f, t) = stressed?;
    if total <= allowed {
        return None;
    }
    Some(format!(
        "{}: the cut's {} N {} mm out bends it at {} N/mm² and its {} N·mm about A twist it at {} N/mm², {} N/mm² together, against the {} N/mm² {what} yields at ({}, under the {} wanted). It needs to be about {} mm square. {fix}",
        size(x, &s),
        f.round(),
        mm(far - x),
        bend.round(),
        t.round(),
        shear.round(),
        total.round(),
        yields.round(),
        crate::forces::times(yields / total),
        crate::forces::times(SAFETY),
        mm(square_for(f * (far - x), t, allowed)),
    ))
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
        .map(|(oi, sim)| {
            (
                *oi,
                in_spindle(doc, &setup.operations[*oi].tool, &sim.plan, Some(rig.unit)),
            )
        })
        .collect();
    let walk = Walk {
        rig: &rig,
        tools,
        gap,
        blend: crate::post::blend(m),
        datum: at.zero,
        // Machine Z 0, the top of the Z travel, in the setup's frame, when
        // the programs retract there: the settings they are made with, a
        // run's or export's (LIO-212).
        home: crate::post::PostSettings::for_holding(m)
            .is_ok_and(|s| s.machine_home)
            .then(|| at.machine_zero.map_or(f32::INFINITY, |z| at.zero[2] - z[2])),
    };
    for problem in walk.collisions(name, setup, &sims, a_before(doc, si)) {
        if !found.contains(&problem) {
            found.push(problem);
        }
    }
    Ok(found)
}

/// What the way a run takes from wherever the machine is into a setup in
/// the chuck is checked against when Cycle Start is pressed (LIO-212,
/// `run::way_in_problems`): the chuck round the stock the setup starts
/// from, each operation's tool in the spindle (by its index in the
/// program) and what of it is assumed, and the setup's gap, to which the
/// machine's blend tolerance is added; and the machine's A range (machine
/// degrees), which a way out that turns A first keeps to (LIO-213).
#[derive(Debug, Clone, PartialEq)]
pub struct Chuck {
    pub rig: Rig,
    pub tools: Vec<(InSpindle, String)>,
    pub gap: f32,
    pub blend: f32,
    pub range: [f32; 2],
}

/// The `Chuck` of setup `si`, indexed or not, whose operations `ops` (index
/// in the setup, plan) make a program; `held` is the stock a setup that is
/// not indexed grips (`post::gripped`). None for a setup not in the chuck;
/// why it cannot be modelled otherwise.
pub fn chuck(
    doc: &Document,
    mesh: &Mesh,
    si: usize,
    held: crate::mesh::Bounds,
    ops: &[(usize, &Plan)],
    blend: f32,
) -> Result<Option<Chuck>, String> {
    let setup = &doc.setups[si];
    let (clamped, stock) = match doc.index_base(si) {
        Some(base) => (
            &doc.setups[base],
            doc.setup_stock(base, Some(mesh)).ok_or(
                "The stock the setup starts from is not known: check the first setup's stock.",
            )?,
        ),
        None => (setup, held),
    };
    if !crate::chuck::holds(clamped) {
        return Ok(None);
    }
    let rig = doc.chuck_rig(clamped, stock)?;
    let tools = ops
        .iter()
        .map(|(oi, plan)| in_spindle(doc, &setup.operations[*oi].tool, plan, Some(rig.unit)))
        .collect();
    let range = ["rotary_min", "rotary_max"]
        .map(|k| crate::model::num(&setup.machine.p, k).unwrap_or(f32::NAN));
    Ok(Some(Chuck {
        rig,
        tools,
        gap: crate::holding::gap(clamped)?,
        blend,
        range,
    }))
}

impl Chuck {
    /// The A angles (commanded) nearest A `a` at which no jaw stands up, the
    /// jaws even on both sides of straight up (with 3 jaws at ±60° and 180°,
    /// with 4 at ±45° and ±135°): the one each way, inside the machine's A
    /// range at machine A = angle + `offset`, the shorter turn first.
    /// Nothing when no jaw is up at `a` already (LIO-213).
    pub fn jaws_down(&self, a: f32, offset: f32) -> Vec<f32> {
        let pitch = 360. / self.rig.jaws as f32;
        let base = self.rig.sign * (pitch / 2. - self.rig.phase);
        let below = base + ((a - base) / pitch).floor() * pitch;
        let each = [below, below + pitch];
        if each.iter().any(|t| (t - a).abs() <= 1e-3) {
            return vec![];
        }
        let mut out: Vec<f32> = (each.into_iter())
            .filter(|t| (self.range[0]..=self.range[1]).contains(&(t + offset)))
            .collect();
        out.sort_by(|p, q| (p - a).abs().total_cmp(&(q - a).abs()));
        out
    }
    /// Why a run cannot take operation `op`'s tool along `moves` (from, to
    /// and the A at each end, commanded, in the setup's frame): its way in
    /// from where the machine is. Each move is checked at the A it is made
    /// at against the chuck, its jaws, the rotary head, the tailstock and
    /// the stock gripped in the jaws; across, also against the whole stock
    /// the setup starts from at that A; while A turns, against all that the
    /// turn sweeps, the stock included. Straight up from where the tool is,
    /// and down onto the first point, the free stock is not counted: the
    /// tool is in it or over it there. Heights said in work coordinates
    /// (`datum`, the work zero in the setup's frame).
    pub fn way_in(
        &self,
        op: usize,
        moves: &[([f32; 3], [f32; 3], f32, f32)],
        datum: [f32; 3],
    ) -> Vec<String> {
        let Some((tool, assumed)) = self.tools.get(op) else {
            return vec![format!(
                "The way in from where the machine is cannot be checked: operation {} has no tool. Check the setup again.",
                op + 1
            )];
        };
        let reach = self.gap + self.blend;
        let mut out = vec![];
        for &(from, to, a, b) in moves {
            let segment = [(from, to)];
            let (hits, what) = if (b - a).abs() > 1e-4 {
                (
                    self.rig.hits_turning(a, b, &segment, *tool, reach),
                    format!("while A turns from A {}° to A {}°", degrees(a), degrees(b)),
                )
            } else if from[..2] != to[..2] {
                (
                    self.rig.hits_turning(a, a, &segment, *tool, reach),
                    format!("on the way across at A {}°", degrees(a)),
                )
            } else {
                let way = if to[2] > from[2] { "up" } else { "down" };
                (
                    self.rig.hits_at(a, &segment, *tool, reach),
                    format!("on the way {way} at A {}°", degrees(a)),
                )
            };
            if let Some(hit) = hits.first() {
                let at = |k: usize| mm(hit.at[k] - datum[k]);
                out.push(format!(
                    "From where the machine is, Cycle Start would bring the {} within the {} mm gap of the {} {what}, at X{} Y{} Z{}.{} The way in goes up to the retract height, across, turns A and comes down: jog the tool and A to where that way is clear first.",
                    hit.part,
                    mm(self.gap),
                    hit.label,
                    at(0),
                    at(1),
                    at(2),
                    if hit.part == "tool" { "" } else { assumed },
                ));
            }
        }
        out
    }
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

    /// `indexed`'s bar job: the part in a Ø40 × 120 aluminium bar 80 mm out
    /// of a 3-jaw chuck, on an A centreline at machine Z `centre_z`, setup 2
    /// a contour indexed to `angles`; and setup 2's simulation.
    fn job(centre_z: &str, angles: &str) -> (Document, Mesh, Simulation) {
        let mesh = prism(30.);
        let mut doc = bar_job(&mesh, 0., "80", centre_z, angles, contour(&cutter("12")));
        crate::proposal::tests::aluminium(&mut doc);
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
    fn the_angles_with_no_jaw_up_are_the_nearest_each_way_inside_a_range() {
        // LIO-213: a 3-jaw chuck, jaw 1 straight up at A0: no jaw is up at
        // ±60° and 180°; a 4-jaw one at ±45° and ±135°.
        let (doc, mesh, sim) = job("-150", "90");
        let tools = [(0, &sim.plan)];
        let held = doc.setup_stock(0, Some(&mesh)).unwrap();
        let mut chuck = chuck(&doc, &mesh, 1, held, &tools, 0.).unwrap().unwrap();
        assert_eq!(
            (chuck.rig.jaws, chuck.rig.phase, chuck.rig.sign),
            (3, 0., 1.)
        );
        assert_eq!(chuck.range, [-360., 360.]);
        assert_eq!(chuck.jaws_down(0., 0.), [-60., 60.]);
        assert_eq!(chuck.jaws_down(30., 0.), [60., -60.]);
        assert_eq!(chuck.jaws_down(-100., 0.), [-60., -180.]);
        assert_eq!(chuck.jaws_down(150., 0.), [180., 60.]);
        // Already with no jaw up: nothing to turn to.
        assert!(chuck.jaws_down(60., 0.).is_empty());
        assert!(chuck.jaws_down(-180., 0.).is_empty());
        // Inside A's range, at machine A = angle + A's work offset.
        chuck.range = [-180., 180.];
        assert_eq!(chuck.jaws_down(170., 0.), [180., 60.]);
        assert_eq!(chuck.jaws_down(170., 10.), [60.]);
        assert_eq!(chuck.jaws_down(10., 150.), [-60.]);
        // The other way round, Direction Negative, and with jaw 1 at 30°.
        chuck.range = [-360., 360.];
        (chuck.rig.sign, chuck.rig.phase) = (-1., 30.);
        assert_eq!(chuck.jaws_down(0., 0.), [-30., 90.]);
        chuck.rig.jaws = 4;
        (chuck.rig.sign, chuck.rig.phase) = (1., 0.);
        assert_eq!(chuck.jaws_down(10., 0.), [45., -45.]);
    }

    #[test]
    fn every_move_is_checked_on_both_sides_of_the_machines_travel_and_a_range() {
        // Machine::new's travel: X 600, Y 400, Z 300; A -360° to 360°.
        let (doc, mesh, sim) = job("-150", "90");
        // The job as generated is inside every limit.
        let said = found(&doc, &mesh, sim.clone());
        assert!(said.is_empty(), "{said:?}");
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
        // 80 mm out of the jaws, A swings only the Ø40 bar under the tool:
        // a retract over its 20 mm needs 5 mm under the top of the Z travel.
        let (mut doc, mesh, sim) = job("-150", "90");
        let swing = crate::indexed::tests::height(&doc, &mesh, 1, &sim.plan, 0.);
        assert_eq!((swing.what, swing.radius), ("the stock reaches", 20.));
        for (centre, room) in [(-25. + 0.01, Some("4.99")), (-25. - 0.01, None)] {
            doc.setups[0]
                .machine
                .p
                .insert("a_centre_z".into(), centre.to_string());
            doc.sync_indexed();
            // Its operation is refused when generated (15 mm of clearance
            // over the bar is above the Z travel): the room is said anyway.
            assert!(prepare(&doc, &mesh, 1, 0).is_err());
            let said = found(&doc, &mesh, sim.clone());
            let room_said = format!(
                "Flats, Contour: A turns all within 20 mm of its centreline under the tool (the stock reaches that far at any angle), which at machine Z {} reaches machine Z -4.99: 4.99 mm under the top of the machine's 300 mm Z travel (Z 0), less than the 5 mm a retract over it needs.",
                mm(centre)
            );
            match room {
                Some(_) => assert!(said.iter().any(|s| s.starts_with(&room_said)), "{said:?}"),
                None => assert!(
                    said.iter().all(|s| !s.contains("A turns all within")),
                    "{said:?}"
                ),
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
        // The tool is 20 mm from the jaws, which A swings 75.66 mm out:
        // under it A swings only the bar, which leaves no room either.
        let sim = prepare(&doc, &mesh, 1, 0).unwrap();
        let said = found(&doc, &mesh, sim);
        assert!(
            said[0].starts_with("Flats, Contour: A turns all within 40 mm of its centreline under the tool (the stock reaches that far at any angle), which at machine Z -40 reaches machine Z 0: 0 mm under the top of the machine's 55 mm Z travel (Z 0), less than the 5 mm a retract over it needs."),
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
            refused.contains("at machine Z 15 with the tool over X")
                && refused.contains("(the A centreline at Z -40 + 40 mm, as far as the stock reaches from it at any angle, + 15 mm clearance)"),
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
        // Setup 1 holds the bar in the chuck at A0: its moves' checks are
        // the 3-axis ones, unchanged; a cut parting the bar would be said.
        let (doc, mesh, sim) = job("-150", "90");
        assert!(problems(&doc, &mesh, 0, &[(0, Ok(sim))]).is_empty());
    }

    /// The 4-angle job the demo used to be: the part 20 mm out of the
    /// chuck's jaws in a Ø40 bar `len` mm long, 20 mm of it in the jaws, a
    /// contour at A 90, 180, 270 and 0° whose passes across the bar's ends go
    /// `bottom` mm over the part's bottom, 10 mm under the A axis; and what
    /// the rotary checks say of setup 2. Ready to post: the tool numbered and
    /// the part's file given.
    fn four_angles_on(len: f32, bottom: &str) -> (Document, Mesh, Vec<String>) {
        let mesh = prism(30.);
        let mut op = contour(&cutter("12"));
        op.p.insert("bottom".into(), bottom.into());
        let stickout = (len - 20.).to_string();
        let mut doc = bar_job(&mesh, 20., &stickout, "-150", "90, 180, 270, 0", op);
        doc.setups[0]
            .p
            .insert("stock_length".into(), len.to_string());
        doc.tools[0].p.insert("number".into(), "1".into());
        doc.file = Some(crate::model::ModelFile {
            name: "bar.stl".into(),
            path: "bar.stl".into(),
            format: "STL".into(),
            bytes: 0,
        });
        doc.sync_stock_sources();
        doc.sync_turns(Some(&mesh));
        let sims = crate::run::simulated_for_tests(&doc, &mesh, 1);
        let said = problems(&doc, &mesh, 1, &sims)
            .into_iter()
            .map(|(_, m)| m)
            .collect();
        (doc, mesh, said)
    }
    /// `four_angles_on` the 120 mm bar.
    fn four_angles(bottom: &str) -> (Document, Mesh, Vec<String>) {
        four_angles_on(120., bottom)
    }
    /// What the rotary checks say of the contour to the A axis, which parts
    /// the bar at A 270° (LIO-208): the part, with what the angle before left
    /// round it.
    const PARTED: &str = "Flats at A 270°, Contour: the cuts leave nothing of the Ø40 mm bar on the A axis from X-52 to X-40, and nothing else joins the 40 × 30 × 20 mm piece of it (17520 mm³) from X-40 to X-10 to the bar the chuck holds, so it comes loose, to be thrown or to break the tool. Leave a web of the stock that holds it (the stock is checked in 1 mm cells: a thinner web counts as cut through, and a loose piece over 2 mm long or 2 mm³ is not taken for a chip); a part-off is not an operation yet.";

    #[test]
    fn a_contour_to_the_axis_that_cuts_the_part_off_the_bar_is_refused_where_it_does() {
        // Each angle's passes across the bar's ends go to the A axis: after
        // A 270° nothing joins the part to the bar on the axis, between the
        // Ø12 cutter's edges at X-52.06 and X-40.06 (to the 1 mm cells).
        // Said once, at the angle that parts it.
        let (_, _, said) = four_angles("10");
        assert_eq!(said, vec![PARTED.to_string()]);
        // On a 250 mm bar the part and the 63 mm of bar beyond it, out to
        // its free end.
        let (_, _, said) = four_angles_on(250., "10");
        assert_eq!(said.len(), 1, "{said:?}");
        assert!(
            said[0].starts_with("Flats at A 270°, Contour: the cuts leave nothing of the Ø40 mm bar on the A axis from X-75 to X-63, and nothing else joins the 68 × 40 × 40 mm piece of it (79878 mm³) from X-68 to X0"),
            "{said:?}"
        );
    }

    #[test]
    fn a_contour_to_the_parts_bottom_leaves_a_fin_at_the_bars_end_that_is_refused() {
        // To the part's bottom, 10 mm under the axis, as the demo job once
        // did: already at A 90° the round corner of the passes at the bar's
        // free end leaves a fin 3 × 3 mm across, 20 mm tall, on less than a
        // millimetre of the bar. Under three voxels thick, the first check
        // took it for a chip (LIO-211).
        let (_, _, said) = four_angles("0");
        assert_eq!(said.len(), 1, "{said:?}");
        assert!(
            said[0].starts_with("Flats at A 90°, Contour: the cuts leave nothing of the Ø40 mm bar on the A axis from X-10 to X-3, and nothing else joins the 20 × 3 × 3 mm piece of it (60 mm³) from X-3 to X0 to the bar the chuck holds"),
            "{said:?}"
        );
    }

    /// What the part-off check says of setup 2 of a Ø40 bar `len` mm long,
    /// 20 mm of it in the jaws, at A 90, 180, 270 and 0°, its contour's moves
    /// replaced by a Ø`d` flat end mill's feeds `passes`: (A, from, to) in
    /// the setup's frame, where the A axis is at Y0 Z20 and the bar's free
    /// end at X `len` / 2, A turning between them. At A 90° the setup's -Y
    /// is the bar's +Z at A0, its +Z the bar's +Y.
    fn carved(len: f32, d: f32, passes: &[(f32, [f32; 3], [f32; 3])]) -> Vec<String> {
        let mesh = prism(30.);
        let stickout = (len - 20.).to_string();
        let mut doc = bar_job(
            &mesh,
            20.,
            &stickout,
            "-150",
            "90, 180, 270, 0",
            contour(&cutter("12")),
        );
        doc.setups[0]
            .p
            .insert("stock_length".into(), len.to_string());
        doc.tools[0].p.insert("diameter".into(), d.to_string());
        doc.sync_stock_sources();
        doc.sync_turns(Some(&mesh));
        let mut sim = prepare(&doc, &mesh, 1, 0).unwrap();
        (sim.plan.radius, sim.plan.flute_length) = (d / 2., 60.);
        let (mut moves, mut t, mut at) = (vec![], 0., None);
        let mut step = |from, to, motion, a| {
            moves.push(Move {
                from,
                to,
                motion,
                cutting_pass: motion == Motion::Feed,
                feed: 1000.,
                start: t,
                end: t + 1.,
                a: Some(a),
            });
            t += 1.;
        };
        for &(a, from, to) in passes {
            if at.is_some_and(|b| b != a) {
                let over = [from[0], from[1], 100.];
                step(over, over, Motion::Index, a);
            }
            at = Some(a);
            step(from, to, Motion::Feed, a);
        }
        (sim.plan.moves, sim.plan.duration) = (moves, t);
        parted(&doc, &mesh, 1, &[(0, Ok(sim))])
            .into_iter()
            .map(|(_, m)| m)
            .collect()
    }
    /// The passes that cut a stick `l` × `t` × `t` mm free at the free end
    /// `e` of the bar, along the A axis, `t` mm wide across Y (from half a
    /// millimetre off the axis, so that the 1 mm cells see it `t` across)
    /// and 10 to 10 + `t` mm over the axis, with a Ø`t` cutter: at A0 down to
    /// its bottom on either side, down to its top over it and through the
    /// bar beyond its end towards the jaws; at A 90° through the bar under
    /// it.
    fn stick(e: f32, l: f32, t: f32) -> Vec<(f32, [f32; 3], [f32; 3])> {
        let (r, c, z) = (t / 2., (t / 2.).fract(), 30.);
        let (from, to) = (e - l - 2. * r, e + 2. * r);
        let mut v = vec![];
        let mut y = r + r;
        while y - r < 21. {
            for side in [c - y, c + y] {
                v.push((0., [from, side, z], [to, side, z]));
            }
            y += 1.8 * r;
        }
        v.push((0., [from, c, z + t], [to, c, z + t]));
        v.push((0., [e - l - r, -26., z - t], [e - l - r, 26., z - t]));
        v.push((90., [e - l, -(10. - r), -1.], [to, -(10. - r), -1.]));
        v
    }
    /// The passes that cut a disc `t` mm thick off the free end `e` of the
    /// bar: a Ø6 cutter across the bar beside it to half a millimetre past
    /// the axis at A 90 and 270°.
    fn disc(e: f32, t: f32) -> Vec<(f32, [f32; 3], [f32; 3])> {
        let x = e - t - 3.;
        [90., 270.]
            .map(|a| (a, [x, -26., 19.5], [x, 26., 19.5]))
            .to_vec()
    }

    #[test]
    fn a_stick_or_a_disc_cut_free_is_refused_on_a_120_and_a_250_mm_bar() {
        // A 30 × 3 × 3 mm stick on the 120 mm bar, a 60 × 6 × 6 mm one on
        // the 250 mm bar (its simulation's voxels 2.6 mm), and the bar's
        // full Ø40 end, 3 and 7 mm thick: none three voxels thick everywhere
        // on the simulation's grid, all passed as chips before (LIO-211).
        for (len, d, passes, piece, angle) in [
            (
                120.,
                3.,
                stick(60., 30., 3.),
                "the 30 × 3 × 3 mm piece of it (270 mm³) from X-30 to X0",
                90,
            ),
            (
                250.,
                6.,
                stick(125., 60., 6.),
                "the 60 × 6 × 6 mm piece of it (2160 mm³) from X-60 to X0",
                90,
            ),
            (
                120.,
                6.,
                disc(60., 3.),
                "the 40 × 40 × 3 mm piece of it (3792 mm³) from X-3 to X0",
                270,
            ),
            (
                250.,
                6.,
                disc(125., 7.),
                "the 40 × 40 × 7 mm piece of it (8848 mm³) from X-7 to X0",
                270,
            ),
        ] {
            let said = carved(len, d, &passes);
            assert_eq!(said.len(), 1, "{len}: {said:?}");
            let s = &said[0];
            assert!(
                s.starts_with(&format!("Flats at A {angle}°, Contour: ")),
                "{s}"
            );
            assert!(s.contains(piece), "{len}: {s}");
            assert!(
                s.contains("so it comes loose, to be thrown or to break the tool"),
                "{s}"
            );
        }
        // Without the last pass that frees each, nothing is loose, and the
        // webs left (the stick's base, the disc's lower half) hold.
        for (len, d, mut passes) in [
            (120., 3., stick(60., 30., 3.)),
            (250., 6., stick(125., 60., 6.)),
            (120., 6., disc(60., 3.)),
            (250., 6., disc(125., 7.)),
        ] {
            passes.pop();
            assert_eq!(carved(len, d, &passes), Vec::<String>::new(), "{len}");
        }
    }

    #[test]
    fn a_web_too_thin_to_hold_is_refused_with_its_size_and_a_sound_one_passes() {
        // 0.8 mm over the axis at all four angles leaves the part on a
        // 1.6 mm square neck: it was seen, and passed. It is refused, on the
        // 120 and the 250 mm bar, as soon as the cuts leave it under 3 mm
        // across, with the square web the cut's force needs.
        for len in [120., 250.] {
            let (_, _, said) = four_angles_on(len, "10.8");
            assert_eq!(said.len(), 1, "{len}: {said:?}");
            let s = &said[0];
            assert!(
                s.starts_with("Flats at A 270°, Contour: the web the cuts leave at X"),
                "{s}"
            );
            assert!(s.contains("2 mm across at its thinnest: a web under 3 mm across does not hold what is beyond it, and to hold the cut's 168 N"), "{s}");
            assert!(s.contains("a square one about 13."), "{s}");
        }
        // 2 mm over it, a 4 mm square web: thick enough to be one, too thin
        // to hold the cut on the part out of it: refused with the stresses.
        let (_, _, said) = four_angles("12");
        assert_eq!(
            said,
            vec!["Flats at A 270°, Contour: the web the cuts leave at X-48.5, between the bar the chuck holds and the 48.5 mm of it out to X0, is 92 mm² and 4.49 mm across at its thinnest: the cut's 168 N 48.5 mm out bends it at 215 N/mm² and its 4085 N·mm about A twist it at 42 N/mm², 227 N/mm² together, against the 70 N/mm² the stock (no material is chosen: taken as copper, the least of the metals) yields at (×0.3, under the ×2 wanted). It needs to be about 13.35 mm square. Leave a thicker web or take a lighter cut (the stock is checked in 1 mm cells); a part-off is not an operation yet.".to_string()]
        );
        // 7 mm over it, a 14 mm web (`on_the_bar`), holds on both bars.
        for len in [120., 250.] {
            let (_, _, said) = four_angles_on(len, "17");
            assert!(said.is_empty(), "{len}: {said:?}");
        }
    }

    #[test]
    fn held_is_inside_the_jaws_grip_not_anything_behind_their_front() {
        // A core of the bar inside the jaws, cut free of the shell they grip
        // round it (an annulus round it and a slab across its end), is not
        // held: the jaws close on the shell, not on it. Only cells are
        // cleared here; no cutter reaches inside the jaws.
        let mesh = prism(30.);
        let doc = bar_job(
            &mesh,
            20.,
            "100",
            "-150",
            "90, 180, 270, 0",
            contour(&cutter("12")),
        );
        let sim = prepare(&doc, &mesh, 1, 0).unwrap();
        let stock = doc.setup_stock(0, Some(&mesh)).unwrap();
        let rig = doc.chuck_rig(&doc.setups[0], stock).unwrap();
        let at = doc.at_angle(1, 0., &mesh).unwrap();
        let v0 = sim.initial_volume.unwrap();
        let mut v = crate::volume::Volume::finer(v0.bounds, v0.frame, CELL).rounded(v0.bar_x);
        let grid = Gripped::new(&v, at.frame, &rig);
        let whole = grid.held(&v);
        let core = |i: usize| grid.r[i] < 5. && grid.x[i] < rig.front - 4.;
        for i in 0..grid.x.len() {
            let (r, x) = (grid.r[i], grid.x[i]);
            let slab = r < 8. && (rig.front - 4. ..rig.front - 2.).contains(&x);
            if (5. ..8.).contains(&r) && x < rig.front - 2. || slab {
                v.occupied[i] = 0;
            }
        }
        let held = grid.held(&v);
        let cores: Vec<usize> = (0..grid.x.len())
            .filter(|&i| core(i) && v.occupied[i] != 0)
            .collect();
        assert!(!cores.is_empty());
        assert!(cores.iter().all(|&i| whole[i] && !held[i]));
        // It is a piece that came loose.
        let pieces = grid.loose(&v, &whole, &held);
        assert_eq!(pieces.len(), 1);
        assert_eq!(pieces[0].voxels.len(), cores.len());
        // The rest of the bar out of the jaws is still held.
        assert!(
            (0..grid.x.len())
                .filter(|&i| grid.x[i] > rig.front && v.occupied[i] != 0)
                .all(|i| held[i])
        );
    }

    #[test]
    fn a_loose_bit_up_to_two_cells_is_a_chip_and_three_is_a_piece() {
        let mesh = prism(30.);
        let doc = bar_job(
            &mesh,
            20.,
            "100",
            "-150",
            "90, 180, 270, 0",
            contour(&cutter("12")),
        );
        let sim = prepare(&doc, &mesh, 1, 0).unwrap();
        let stock = doc.setup_stock(0, Some(&mesh)).unwrap();
        let rig = doc.chuck_rig(&doc.setups[0], stock).unwrap();
        let at = doc.at_angle(1, 0., &mesh).unwrap();
        let v0 = sim.initial_volume.unwrap();
        let fresh = crate::volume::Volume::finer(v0.bounds, v0.frame, CELL).rounded(v0.bar_x);
        let grid = Gripped::new(&fresh, at.frame, &rig);
        let whole = grid.held(&fresh);
        // A bit of `n` cells in a row along X on the axis, the stock round
        // it, a cell thick, cleared away.
        let n = fresh.n;
        let mid = [n[0] - 20, n[1] / 2, n[2] / 2];
        for (len, loose) in [(1, false), (2, false), (3, true)] {
            let mut v = fresh.clone();
            for dz in -1i32..=1 {
                for dy in -1i32..=1 {
                    for dx in -1..=len {
                        let p = [
                            (mid[0] as i32 + dx) as usize,
                            (mid[1] as i32 + dy) as usize,
                            (mid[2] as i32 + dz) as usize,
                        ];
                        let inside = dy == 0 && dz == 0 && (0..len).contains(&dx);
                        if !inside {
                            let i = v.index(p);
                            v.occupied[i] = 0;
                        }
                    }
                }
            }
            let pieces = grid.loose(&v, &whole, &grid.held(&v));
            assert_eq!(!pieces.is_empty(), loose, "{len} cells");
        }
    }

    #[test]
    fn the_export_the_plan_check_and_the_run_refuse_a_cut_that_parts_the_bar() {
        for (bottom, parts) in [("10", true), ("17", false)] {
            let (mut doc, mesh, _) = four_angles(bottom);
            let posted = crate::post::post_job(&doc, &mesh);
            match parts {
                true => {
                    let refused = posted.setups[1].result.as_ref().unwrap_err();
                    assert!(refused.contains(&PARTED.to_string()), "{refused:?}");
                }
                false => assert!(
                    posted.setups[1].result.is_ok(),
                    "{:?}",
                    posted.setups[1].result
                ),
            }
            crate::proposal::tests::aluminium(&mut doc);
            doc.active = doc.setups[0].id.clone();
            let report = crate::analysis::report(&doc, &mesh).unwrap();
            let check = crate::plan_check::check_built(&doc, &mesh, &report);
            let said = check.problems.iter().filter(|p| p.message == PARTED);
            assert_eq!(said.count(), usize::from(parts), "{:?}", check.problems);
            // A run's admission on a UC100 too, once: the machine with the
            // CNC4030's UC100 profile.
            let dir = tempfile::tempdir().unwrap();
            let uc100 = crate::control::rotary_machine(dir.path()).unwrap();
            for k in ["controller", "uc100_profile"] {
                let v = get(&uc100.p, k).to_string();
                doc.setups[0].machine.p.insert(k.into(), v);
            }
            doc.sync_indexed();
            let ready = crate::run::ready(&doc, &mesh, 1, 0);
            let said = ready.problems.iter().filter(|m| m.as_str() == PARTED);
            assert_eq!(said.count(), usize::from(parts), "{:?}", ready.problems);
        }
    }

    #[test]
    fn a_cut_through_in_the_first_setup_in_the_chuck_is_refused_too() {
        use crate::indexed::tests::on_the_bar;
        // Setup 1, at A0 with no A, contours the part down to the bar's
        // bottom: its passes across the bar's ends cut it through.
        let mesh = prism(30.);
        let mut doc = bar_job(&mesh, 20., "100", "-150", "90", on_the_bar(&cutter("12")));
        let mut through = contour(&doc.tools[0]);
        through.p.insert("bottom".into(), "-10".into());
        doc.setups[0].operations = vec![through];
        doc.sync_stock_sources();
        doc.sync_turns(Some(&mesh));
        let sims = crate::run::simulated_for_tests(&doc, &mesh, 0);
        let said: Vec<String> = problems(&doc, &mesh, 0, &sims)
            .into_iter()
            .map(|(_, m)| m)
            .collect();
        let name = doc.setups[0].name();
        assert_eq!(said.len(), 1, "{said:?}");
        assert!(
            said[0].starts_with(&format!("{name} at A 0°, Contour: the cuts leave nothing of the Ø40 mm bar on the A axis from X-52 to X-40, and nothing else joins the 40 × 30 × 20 mm piece of it (23040 mm³) from X-40 to X-10")),
            "{said:?}"
        );
        doc.tools[0].p.insert("number".into(), "1".into());
        doc.file = Some(crate::model::ModelFile {
            name: "bar.stl".into(),
            path: "bar.stl".into(),
            format: "STL".into(),
            bytes: 0,
        });
        let posted = crate::post::post_job(&doc, &mesh);
        let refused = posted.setups[0].result.as_ref().unwrap_err();
        assert!(refused.contains(&said[0]), "{refused:?}");
        // The same cut in a 3-axis setup, the stock in a vise: not this
        // check's, unchanged.
        doc.setups.truncate(1);
        doc.setups[0].p.insert("holding".into(), "Vise".into());
        let sims = crate::run::simulated_for_tests(&doc, &mesh, 0);
        assert!(sims.iter().all(|(_, r)| r.is_ok()));
        assert!(problems(&doc, &mesh, 0, &sims).is_empty());
    }
}
