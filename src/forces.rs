//! Whether a setup's vise or clamps hold the stock against the cutting
//! forces of its operations. Each operation is replayed on the stock it
//! starts from, the earlier setups' cuts included (`bites`); what it removes
//! over each stretch (a stretch too light for the stock's voxels to register
//! counts as the most that can hide between them) gives the cutting force
//! there, the material's specific cutting force over the cutting speed, at
//! most what the spindle drives (`pushes`). That force, sideways and upward
//! at the height it acts, is set against what the jaws' or clamps' friction
//! and leverage resist, the jaws over the contact the earlier cuts left them
//! above where the stock stands (`check`). A simple model, a peak per
//! operation: every assumed number is named below, and a library entry's
//! clamping force (`force`) and a machine's spindle power (`power`) can be
//! typed in.
use crate::holding::{Gripped, Holder, Layout, Side, clip, grid};
use crate::material::Material;
use crate::mesh::Bounds;
use crate::model::{Document, Machine, get, num};
use crate::rest::Removed;
use crate::simulation::{Motion, Simulation};
use crate::volume::Volume;
use std::f32::consts::PI;

/// Friction between the stock and a jaw, the table or a clamp's toe: dry
/// steel on metal, on the low side.
pub const FRICTION: f32 = 0.2;
/// A vise's clamping force when its library entry gives none, N: a 100 mm
/// vise tightened by hand.
pub const VISE_FORCE: f32 = 5000.;
/// A clamp's force on the stock when its library entry gives none, N: an M6
/// to M8 toe clamp tightened by hand, less what its heel takes.
pub const CLAMP_FORCE: f32 = 2000.;
/// A spindle's power when its machine gives none, W: the 800 W spindle of a
/// Jingyan CNC4030.
pub const SPINDLE_POWER: f32 = 800.;
/// The peak force on the stock over the mean tangential cutting force: the
/// teeth coming round, and the radial force beside the tangential.
pub const PEAK: f32 = 1.5;
/// The upward pull of a right-hand helix (about 30°) over the tangential
/// force.
pub const LIFT: f32 = 0.5;
/// The least margin, what the holding resists over what the cut applies,
/// that is enough.
pub const SAFETY: f32 = 2.;
/// Specific cutting force kc1.1 (N/mm² at a 1 mm chip) and its exponent mc
/// by material group, rounded from Sandvik's tables. "Other", and a job
/// without a material, count as steel.
pub const KC: [(&str, f32, f32); 7] = [
    ("Aluminium", 700., 0.25),
    ("Steel", 1700., 0.25),
    ("Stainless steel", 2000., 0.21),
    ("Brass and copper", 700., 0.25),
    ("Plastic", 60., 0.25),
    ("Wood and composite", 20., 0.25),
    ("Other", 1700., 0.25),
];

/// What an operation cuts over a stretch of its path (`bites`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bite {
    /// Stock removed, mm³/s.
    pub rate: f32,
    /// Feed along the path, mm/min.
    pub feed: f32,
    /// Where the cut's force acts, mm high in the setup's frame: halfway up
    /// the flutes below the stock's top.
    pub z: f32,
}
/// The bites of `sim`'s operation: its feed moves replayed on the stock it
/// starts from (`Simulation::initial_volume`), cut in pieces at most two
/// tool diameters long, and the stock removed over each run of them that
/// long (a rapid or a dwell ends a run). A run removes at most a slot as
/// wide as the cutter and as deep as its flutes reach below the stock's
/// top: a pass lighter than the stock's voxels can take a whole layer of
/// them. A run whose cutter touches material (comes within a voxel of the
/// centre of a voxel that has some, `Volume::touches`) but sweeps none of
/// the voxels' centres, as a skim thinner than a voxel can, is never taken
/// as free: it counts as removing a skin a voxel thick along the run, as
/// wide as the cutter or as tall as its depth in the stock, whichever is
/// more (the most that can hide between the centres: a surface an earlier
/// cut left lies anywhere up to a voxel above the highest centre it left,
/// the original face half a voxel above), and at most the slot (exactly
/// the slot for such a skim). A run in the air, whose cutter comes within
/// a voxel of no such centre (over what the setup's earlier operations
/// already cut away, or beside the stock), removes nothing. Empty without that stock, and
/// for drilling, which pushes the stock down into its holding and twists
/// it (not modelled), but not sideways or up.
/// ponytail: a replay more than the simulation makes; keep what
/// `Simulation::seek` removes per move if that time ever shows.
pub fn bites(sim: &Simulation) -> Vec<Bite> {
    let (Some(mut volume), Some(frame)) = (sim.initial_volume.clone(), sim.frame) else {
        return vec![];
    };
    if sim.plan.strategy == crate::model::Strategy::Drill {
        return vec![];
    }
    let plan = &sim.plan;
    let cell: f32 = volume.cell.iter().product();
    // A voxel's longest side, mm.
    let thick = volume.cell.into_iter().fold(0., f32::max);
    let (top, reach) = (plan.stock.max[2], 4. * plan.radius);
    // Whether the cutter, from `from` to `to`, grown by a whole voxel all
    // round, sweeps the centre of a voxel that has material: it touched
    // some. Half a voxel reaches the original stock faces, which lie half a
    // voxel above the centres, but not a floor or a wall an earlier cut
    // left, which can lie a whole voxel past the last centre it kept.
    let touches = |from: [f32; 3], to: [f32; 3], volume: &Volume| {
        let lower = |p: [f32; 3]| [p[0], p[1], p[2] - thick];
        volume.touches(
            lower(from),
            lower(to),
            plan.radius + thick,
            plan.flute_length + 2. * thick,
            plan.shape,
            frame,
        )
    };
    // Each run's voxels removed, length (mm), minutes, lowest tip and
    // whether it touched material.
    let fresh = (0, 0., 0., f32::INFINITY, false);
    let (mut runs, mut run) = (vec![], fresh);
    for m in &plan.moves {
        if m.motion != Motion::Feed || m.feed <= 0. {
            runs.push(std::mem::replace(&mut run, fresh));
            continue;
        }
        let length = (0..3)
            .map(|k| (m.to[k] - m.from[k]).powi(2))
            .sum::<f32>()
            .sqrt();
        let pieces = (length / reach).ceil().max(1.);
        let at = |t: f32| std::array::from_fn(|k| m.from[k] + (m.to[k] - m.from[k]) * t);
        for i in 0..pieces as usize {
            let (from, to): ([f32; 3], [f32; 3]) =
                (at(i as f32 / pieces), at((i + 1) as f32 / pieces));
            let before = volume.remaining;
            volume.cut(
                from,
                to,
                plan.radius,
                plan.flute_length,
                plan.shape,
                frame,
                sim.source,
            );
            run.0 += before - volume.remaining;
            run.1 += length / pieces;
            run.2 += length / pieces / m.feed;
            run.3 = run.3.min(from[2]).min(to[2]);
            if run.0 == 0 && !run.4 {
                run.4 = touches(from, to, &volume);
            }
            if run.1 >= 0.999 * reach {
                runs.push(std::mem::replace(&mut run, fresh));
            }
        }
    }
    runs.push(run);
    runs.into_iter()
        .filter(|&(removed, _, minutes, _, touched)| (removed > 0 || touched) && minutes > 0.)
        .map(|(removed, length, minutes, low, _)| {
            let depth = (top - low).clamp(0., plan.flute_length);
            let slot = 2. * plan.radius * depth * length;
            // What swept no voxel's centre is a skin at most a voxel thick
            // along the run, as wide as the cutter or as tall as its depth,
            // whichever is more: so much is taken.
            let removed = if removed > 0 {
                removed as f32 * cell
            } else {
                thick * (2. * plan.radius).max(depth) * length
            };
            Bite {
                rate: removed.min(slot) / (minutes * 60.),
                feed: length / minutes,
                z: low + depth / 2.,
            }
        })
        .filter(|b| b.rate > 0.)
        .collect()
}
/// A peak force of a cut on the stock, N: sideways (whichever way) and
/// upward, and the height it acts at (mm, the setup's frame).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Push {
    pub side: f32,
    pub up: f32,
    pub z: f32,
}
/// kc1.1 and mc of `material`'s group (`KC`), "Other"'s without one.
fn kc(material: Option<&Material>) -> (f32, f32) {
    let group = material.map_or("Other", |m| get(&m.p, "group"));
    let (_, kc, mc) = KC
        .iter()
        .find(|row| row.0 == group)
        .unwrap_or(&KC[KC.len() - 1]);
    (*kc, *mc)
}
/// The pushes of `bites` taken by a Ø`diameter` tool with `flutes` at `rpm`
/// in `material` (None when the project has none) on `machine`. Each bite's
/// mean tangential force is kc × rate ÷ cutting speed, kc being kc1.1 ×
/// h^-mc at a slot's mean chip h, 2/π of the feed per tooth. It is at most
/// what the spindle's torque drives at the tool's edge, its power at its top
/// speed (a spindle keeps its torque below that speed, not its power).
/// `PEAK` times it pushes sideways, `LIFT` times it up.
pub fn pushes(
    bites: &[Bite],
    diameter: f32,
    flutes: f32,
    rpm: f32,
    material: Option<&Material>,
    machine: &Machine,
) -> Vec<Push> {
    let power = num(&machine.p, "power")
        .filter(|p| *p > 0.)
        .unwrap_or(SPINDLE_POWER);
    let top = num(&machine.p, "rpm").filter(|n| *n > 0.).unwrap_or(rpm);
    // N: torque (N·m) at the edge of a Ø`diameter` mm tool.
    let most = 60_000. * power / (PI * diameter * top);
    // mm/s
    let speed = PI * diameter * rpm / 60.;
    let (kc, mc) = kc(material);
    bites
        .iter()
        .map(|b| {
            let chip = 2. / PI * b.feed / (rpm * flutes);
            let force = (kc * chip.powf(-mc) * b.rate / speed).min(most);
            Push {
                side: PEAK * force,
                up: LIFT * force,
                z: b.z,
            }
        })
        .collect()
}
/// How the stock would give way.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Way {
    /// Slide along the jaws or lift out of them (a vise), or slide on the
    /// table (clamps).
    Slip,
    /// Lift off the table against the clamps.
    Lift,
    /// Turn out of the jaws over their top edge (a vise: None), or over the
    /// stock's bottom edge on that side (clamps).
    Tip(Option<Side>),
}
/// An operation's least margin (`check`): what its vise or clamps resist
/// over what its cut applies, the way the stock would give and the push.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Margin {
    pub factor: f32,
    pub way: Way,
    pub push: Push,
    /// How high the push acts above the jaws' contact or the table, mm, for
    /// a turn.
    pub lever: f32,
    /// What resists: N, or N·mm for a turn.
    pub resisted: f32,
    /// A vise's contact with the stock: its height and its length along the
    /// jaws, mm (the least of the two jaws').
    pub contact: Option<[f32; 2]>,
}
/// What a vise or clamps resist.
enum Hold {
    Vise {
        /// Both jaws' friction, N.
        friction: f32,
        /// What the contact resists a turn with, N·mm.
        couple: f32,
        /// The contact's top, mm.
        edge: f32,
        contact: [f32; 2],
    },
    Clamps {
        /// Each clamp's force, N.
        force: f32,
        /// The centre of each clamp's bite on the stock.
        at: Vec<[f32; 2]>,
        stock: Bounds,
    },
}
/// What `layout`, with `holder`, resists on `held` given what earlier
/// setups `removed` (None for setup 1). A vise's jaws touch the stock where
/// it has material on their faces (`holding::holdable`'s faces, 12 × 8
/// points over each jaw's width on the band and the grip above where the
/// stock stands, `Gripped::bottom`, as the jaws are placed): both jaws
/// together resist sliding with twice the friction of the clamping force,
/// and a turn with the clamping force times the contact's height (the
/// stock wedged between them), or the friction over half its length when
/// that is less (a turn in the jaws' plane about the contact's middle).
/// Clamps hold down with their force at their bites' centres. None without
/// workholding, when it cannot be modelled (`holding::boxes`) or when a jaw
/// touches no material, which `holding::holdable` refuses.
fn hold(
    layout: &Layout,
    holder: &Holder,
    held: Gripped,
    removed: Option<&Removed>,
) -> Option<Hold> {
    let placed = crate::holding::boxes(layout, holder, held).ok()?;
    let force = |typical| {
        num(&holder.p, "force")
            .filter(|f| *f > 0.)
            .unwrap_or(typical)
    };
    match layout {
        Layout::None => None,
        Layout::Vise { axis_y, grip, .. } => {
            let (across, along) = if *axis_y { (1, 0) } else { (0, 1) };
            let (band, stock) = (held.band, held.stock);
            let height = (held.bottom, (held.bottom + grip).min(stock.max[2]));
            let (mut tall, mut long, mut edge) = (f32::INFINITY, f32::INFINITY, f32::INFINITY);
            for jaw in &placed {
                let b = jaw.bounds;
                let face = if b.min[across] >= band.max[across] {
                    band.max[across] - crate::holding::INSIDE
                } else {
                    band.min[across] + crate::holding::INSIDE
                };
                let width = clip(
                    (b.min[along], b.max[along]),
                    band.min[along],
                    band.max[along],
                );
                let touching: Vec<[f32; 3]> =
                    grid([along, 2, across], width, height, face, (12, 8))?
                        .into_iter()
                        .filter(|p| removed.is_none_or(|r| !r.at(*p)))
                        .collect();
                let span = |k: usize| {
                    let (lo, hi) = touching
                        .iter()
                        .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), p| {
                            (lo.min(p[k]), hi.max(p[k]))
                        });
                    (lo, hi)
                };
                if touching.is_empty() {
                    return None;
                }
                let ((bottom, top), (start, end)) = (span(2), span(along));
                tall = tall.min(top - bottom);
                long = long.min(end - start);
                edge = edge.min(top);
            }
            let force = force(VISE_FORCE);
            Some(Hold::Vise {
                friction: 2. * FRICTION * force,
                couple: force * tall.min(FRICTION * long / 2.),
                edge,
                contact: [tall, long],
            })
        }
        Layout::Clamps { .. } => {
            let stock = held.stock;
            let at = placed
                .iter()
                .map(|c| {
                    let b = c.bounds;
                    let x = clip((b.min[0], b.max[0]), stock.min[0], stock.max[0]);
                    let y = clip((b.min[1], b.max[1]), stock.min[1], stock.max[1]);
                    [(x.0 + x.1) / 2., (y.0 + y.1) / 2.]
                })
                .collect();
            Some(Hold::Clamps {
                force: force(CLAMP_FORCE),
                at,
                stock,
            })
        }
    }
}
/// The least margin of `pushes` against `layout` with `holder` holding
/// `held`, given what earlier setups `removed` (`hold`). A vise: its
/// friction against the push in the jaws' plane (sideways, taken along the
/// jaws, and up), and its couple against the push times its height above
/// the contact's top. Clamps: friction under the toes and on the table
/// (what the push lifts off the clamps' force taken from the table's)
/// against the push sideways; their force against the push up; and, over
/// each bottom edge of the stock, each clamp's force times its distance from
/// that edge against the push times its height above the table. None
/// without pushes or when `hold` is None.
pub fn check(
    layout: &Layout,
    holder: &Holder,
    held: Gripped,
    removed: Option<&Removed>,
    pushes: &[Push],
) -> Option<Margin> {
    let hold = hold(layout, holder, held, removed)?;
    let contact = match &hold {
        Hold::Vise { contact, .. } => Some(*contact),
        Hold::Clamps { .. } => None,
    };
    let mut least: Option<Margin> = None;
    for &push in pushes {
        // (resisted, applied, way, lever)
        let tries: Vec<(f32, f32, Way, f32)> = match &hold {
            Hold::Vise {
                friction,
                couple,
                edge,
                ..
            } => {
                let lever = (push.z - edge).max(0.);
                vec![
                    (*friction, push.side.hypot(push.up), Way::Slip, 0.),
                    (*couple, push.side * lever, Way::Tip(None), lever),
                ]
            }
            Hold::Clamps { force, at, stock } => {
                let down = force * at.len() as f32;
                let lever = (push.z - stock.min[2]).max(0.);
                let mut tries = vec![
                    (
                        FRICTION * (2. * down - push.up).max(0.),
                        push.side,
                        Way::Slip,
                        0.,
                    ),
                    (down, push.up, Way::Lift, 0.),
                ];
                for side in Side::ALL {
                    let held: f32 = at
                        .iter()
                        .map(|c| {
                            force
                                * match side {
                                    Side::MinusX => c[0] - stock.min[0],
                                    Side::PlusX => stock.max[0] - c[0],
                                    Side::MinusY => c[1] - stock.min[1],
                                    Side::PlusY => stock.max[1] - c[1],
                                }
                                .max(0.)
                        })
                        .sum();
                    tries.push((held, push.side * lever, Way::Tip(Some(side)), lever));
                }
                tries
            }
        };
        for (resisted, applied, way, lever) in tries {
            let factor = if applied > 0. {
                resisted / applied
            } else {
                f32::INFINITY
            };
            if least.is_none_or(|m| factor < m.factor) {
                least = Some(Margin {
                    factor,
                    way,
                    push,
                    lever,
                    resisted,
                    contact,
                });
            }
        }
    }
    least
}
/// The margin of each operation of setup `si` of `doc` whose `bites` are
/// given (its index in the setup, its bites) against the setup's vise or
/// clamps holding `held`, given what earlier setups `removed` (`check`):
/// None for an operation that pushes nothing sideways or up (`bites`) or
/// has no tool, flutes or spindle speed. Empty when the setup models no vise or clamps.
pub fn margins<'a>(
    doc: &Document,
    si: usize,
    bites: impl IntoIterator<Item = (usize, &'a [Bite])>,
    held: Gripped,
    removed: Option<&Removed>,
) -> Vec<(usize, Option<Margin>)> {
    let Some(setup) = doc.setups.get(si) else {
        return vec![];
    };
    let (Ok(layout), Some(holder)) = (
        crate::holding::layout(setup),
        doc.holder(get(&setup.p, "holder")),
    ) else {
        return vec![];
    };
    if layout == Layout::None {
        return vec![];
    }
    let positive = |p, key| num(p, key).filter(|v| *v > 0.);
    bites
        .into_iter()
        .map(|(oi, bites)| {
            let margin = setup.operations.get(oi).and_then(|op| {
                let tool = doc.tools.iter().find(|t| t.id == op.tool)?;
                let pushes = pushes(
                    bites,
                    positive(&tool.p, "diameter")?,
                    positive(&tool.p, "flutes")?,
                    positive(&op.p, "rpm")?,
                    doc.selected_material(),
                    &setup.machine,
                );
                check(&layout, holder, held, removed, &pushes)
            });
            (oi, margin)
        })
        .collect()
}
/// A margin as said: ×1.4, ×2, ×12. Rounded down, so that one under
/// `SAFETY` never reads as it.
fn times(factor: f32) -> String {
    let tenths = match factor {
        f if f >= 10. => f.floor(),
        f => (f * 10.).floor() / 10.,
    };
    format!("×{tenths}")
}
/// What the setup's holding section says of operation `name`'s `margin`.
pub fn line(name: &str, margin: Option<&Margin>) -> String {
    let Some(m) = margin else {
        return format!("{name}: no cutting force sideways or up to hold against.");
    };
    let way = match m.way {
        Way::Slip if m.contact.is_some() => "sliding or lifting in the jaws",
        Way::Slip => "sliding",
        Way::Lift => "lifting",
        Way::Tip(_) => "tipping",
    };
    if m.factor >= SAFETY {
        format!("{name}: holds {} against {way}.", times(m.factor))
    } else {
        format!(
            "{name}: only {} against {way}; {} is wanted.",
            times(m.factor),
            times(SAFETY)
        )
    }
}
/// The issue of operation `name`'s `margin` when it is under `SAFETY`: how
/// the stock would give way to what, and what to change.
pub fn issue(name: &str, m: &Margin) -> Option<String> {
    if m.factor >= SAFETY {
        return None;
    }
    let short = format!("{}, under the {} wanted", times(m.factor), times(SAFETY));
    let lighter = "take a smaller stepdown or stepover or a lower feed";
    let (side, up) = (m.push.side.round(), m.push.up.round());
    let lever = m.lever.round();
    Some(match (m.way, m.contact) {
        (Way::Slip, Some(_)) => format!(
            "{name}: the cut could move the stock in the vise: about {side} N against the jaws' {} N of friction ({short}). Set the vise's clamping force in the library if it clamps harder, {lighter}, or choose another holder.",
            m.resisted.round()
        ),
        (Way::Tip(_), Some([tall, long])) => format!(
            "{name}: the cut could tip the stock out of the vise: about {side} N, {lever} mm above the top of the jaws' contact, which is {tall:.1} mm high and {long:.0} mm long ({short}). Grip deeper, {lighter}, or choose another holder."
        ),
        (Way::Slip, None) => format!(
            "{name}: the cut could slide the stock under the clamps: about {side} N against {} N of friction ({short}). Add a clamp, {lighter}, or choose another holder.",
            m.resisted.round()
        ),
        (Way::Lift, _) => format!(
            "{name}: the cut could lift the stock against the clamps: about {up} N up against their {} N ({short}). Add a clamp, {lighter}, or choose another holder.",
            m.resisted.round()
        ),
        (Way::Tip(edge), None) => {
            let (edge, far) = match edge {
                Some(Side::MinusX) => ("-X", "+X"),
                Some(Side::PlusX) => ("+X", "-X"),
                Some(Side::MinusY) => ("-Y", "+Y"),
                _ => ("+Y", "-Y"),
            };
            format!(
                "{name}: the cut could tip the stock over its {edge} bottom edge: about {side} N, {lever} mm above the table ({short}). Add a clamp on {far}, {lighter}, or choose another holder."
            )
        }
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::holding::{Bands, holdable, starters};
    use crate::model::Setup;
    use crate::simulation::{CutterShape, Motion};
    use crate::volume::Frame;
    /// 100 × 60 × 40 mm, its bottom at Z0.
    const STOCK: Bounds = Bounds {
        min: [-50., -30., 0.],
        max: [50., 30., 40.],
    };
    /// The frame of a setup 1 in which `STOCK` sits as given.
    fn frame() -> Frame {
        let mesh =
            crate::mesh::normalize(vec![[[0., 0., 0.], [20., 0., 0.], [0., 20., 10.]]]).unwrap();
        Frame::new(&Setup::new(Machine::new(3), "G54"), &mesh)
    }
    fn starter(kind: &str) -> Holder {
        starters().into_iter().find(|h| h.kind() == kind).unwrap()
    }
    #[test]
    fn a_slot_pushes_with_the_stock_it_removes_up_to_what_the_spindle_drives() {
        // A Ø10 slot 4 mm deep along X through the whole stock at 600 mm/min.
        let mut plan =
            crate::post::fixture::plan([-60., 0., 36.], &[(Motion::Feed, [60., 0., 36.], 600.)]);
        (plan.stock, plan.radius, plan.flute_length) = (STOCK, 5., 20.);
        let mut sim = Simulation::new(plan);
        (sim.initial_volume, sim.frame) = (Some(Volume::new(STOCK, frame())), Some(frame()));
        let bites = sim.bites();
        // 10 × 4 mm² at 10 mm/s, halfway up the 4 mm the flutes are in.
        let full = bites.iter().map(|b| b.rate).fold(0., f32::max);
        assert!((full - 400.).abs() < 4., "{bites:?}");
        assert!(
            bites
                .iter()
                .all(|b| (b.feed - 600.).abs() < 0.1 && b.z == 38.)
        );
        // In aluminium with 3 flutes at 8000 rpm: kc 1971 N/mm², 188 N.
        let aluminium = crate::material::starters().remove(0);
        let mut machine = Machine::new(3);
        machine.p.insert("power".into(), "5000".into());
        let full = [Bite {
            rate: 400.,
            feed: 600.,
            z: 38.,
        }];
        let push = pushes(&full, 10., 3., 8000., Some(&aluminium), &machine)[0];
        assert!(
            (push.side - 282.).abs() < 1. && (push.up - 94.).abs() < 1.,
            "{push:?}"
        );
        // An 800 W spindle at up to 12000 rpm drives at most 127 N at its edge,
        // in aluminium as in steel, what a job without a material counts as.
        machine.p.insert("power".into(), "800".into());
        for material in [Some(&aluminium), None] {
            let push = pushes(&full, 10., 3., 8000., material, &machine)[0];
            assert!((push.side - 191.).abs() < 1., "{push:?}");
        }
    }
    #[test]
    fn a_skim_thinner_than_a_voxel_still_pushes_and_a_shallow_grip_refuses_it() {
        // A Ø10 skim 0.3 mm deep along X across the whole stock at 1000
        // mm/min: its tip stays above the centres of the top voxels (about 1
        // mm tall), so it sweeps none of them.
        let skim = |y: f32| {
            let mut plan = crate::post::fixture::plan(
                [-60., y, 39.7],
                &[(Motion::Feed, [60., y, 39.7], 1000.)],
            );
            (plan.stock, plan.radius, plan.flute_length) = (STOCK, 5., 20.);
            let mut sim = Simulation::new(plan);
            (sim.initial_volume, sim.frame) = (Some(Volume::new(STOCK, frame())), Some(frame()));
            sim.bites().to_vec()
        };
        let bites = skim(0.);
        // Taken as the slot it cuts: 10 × 0.3 mm² at 16.7 mm/s, its force
        // 0.15 mm under the top.
        assert!(
            !bites.is_empty(),
            "a skim thinner than a voxel removes nothing"
        );
        assert!(
            bites.iter().all(|b| (b.rate - 50.).abs() < 0.5
                && (b.feed - 1000.).abs() < 0.1
                && (b.z - 39.85).abs() < 0.01),
            "{bites:?}"
        );
        // The same move beside the stock, in the air, cuts nothing.
        assert!(skim(40.).is_empty());
        // In steel (no material) with 3 flutes at 8000 rpm: 50 N, 75 N sideways.
        let push = pushes(&bites, 10., 3., 8000., None, &Machine::new(3));
        assert!((push[0].side - 75.4).abs() < 0.5, "{push:?}");
        // The starter vise gripping 1 mm of the stock on ±Y holds it ×1.6
        // against tipping: refused.
        let layout = Layout::Vise {
            holder: String::new(),
            axis_y: true,
            offset: 0.,
            grip: 1.,
            gap: 2.,
        };
        let m = check(&layout, &starter("Vise"), STOCK.into(), None, &push).unwrap();
        assert_eq!(
            line("Facing", Some(&m)),
            "Facing: only ×1.6 against tipping; ×2 is wanted."
        );
        assert_eq!(
            issue("Facing", &m).unwrap(),
            "Facing: the cut could tip the stock out of the vise: about 75 N, 39 mm above the top of the jaws' contact, which is 1.0 mm high and 98 mm long (×1.6, under the ×2 wanted). Grip deeper, take a smaller stepdown or stepover or a lower feed, or choose another holder."
        );
    }
    #[test]
    fn a_skim_under_the_floor_an_earlier_cut_left_still_pushes() {
        // A 300 × 60 × 40 mm stock: its voxels are 3.125 mm long and 3.08 mm
        // tall, their centres in rows 3.08 mm apart.
        const LONG: Bounds = Bounds {
            min: [-150., -30., 0.],
            max: [150., 30., 40.],
        };
        let f = frame();
        let mut before = Volume::new(LONG, f);
        let c = before.cell[2];
        assert!((before.cell[0] - 3.125).abs() < 0.001 && (c - 40. / 13.).abs() < 0.001);
        // An earlier operation faced the whole top to a floor 0.8 voxel
        // above the highest row of centres it left (34.77 mm; the row at
        // 32.31): the material from that row's cube (33.85 mm) up to the
        // floor lies in the voxels the facing emptied.
        let row = before.point([0, 0, 10], 0.5)[2];
        let floor = row + 0.8 * c;
        before.cut(
            [-200., 0., floor],
            [200., 0., floor],
            50.,
            10.,
            CutterShape::Flat,
            f,
            (0, 0),
        );
        assert_eq!(before.occupied[before.index([48, 10, 11])], 0);
        assert_eq!(before.occupied[before.index([48, 10, 10])], 1);
        // This operation faces in 1.5 mm layers from the box's top with a
        // Ø10 cutter at 1000 mm/min: its passes at 38.5, 37 and 35.5 are in
        // the air over the floor, and the one at 34 takes the 0.77 mm under
        // it, sweeping no centre.
        let mut moves = vec![];
        for layer in 1..=4 {
            let z = 40. - 1.5 * layer as f32;
            moves.push((Motion::Rapid, [-160., 0., 50.], 6000.));
            moves.push((Motion::Rapid, [-160., 0., z], 6000.));
            moves.push((Motion::Feed, [160., 0., z], 1000.));
        }
        let mut plan = crate::post::fixture::plan([-160., 0., 50.], &moves);
        (plan.stock, plan.radius, plan.flute_length) = (LONG, 5., 20.);
        let mut sim = Simulation::new(plan);
        (sim.initial_volume, sim.frame) = (Some(before), Some(f));
        let bites = sim.bites();
        assert!(
            !bites.is_empty(),
            "a skim under the floor an earlier cut left removes nothing"
        );
        // Only the pass at 34 bites, taken as a skin a voxel thick and as
        // wide as the cutter: 3.125 × 10 mm² at 16.7 mm/s (less than the 6
        // mm slot below the box's top), its force 3 mm above its tip.
        assert!(
            bites
                .iter()
                .all(|b| (b.rate - 520.8).abs() < 0.5 && (b.z - 37.).abs() < 0.01),
            "{bites:?}"
        );
        // In steel with 3 flutes at 8000 rpm on a 5 kW spindle: 524 N, 786 N
        // sideways. The starter vise gripping 3 mm holds that ×0.5 against
        // tipping: refused.
        let mut machine = Machine::new(3);
        machine.p.insert("power".into(), "5000".into());
        let push = pushes(bites, 10., 3., 8000., None, &machine);
        assert!((push[0].side - 785.6).abs() < 0.5, "{push:?}");
        let layout = Layout::Vise {
            holder: String::new(),
            axis_y: true,
            offset: 0.,
            grip: 3.,
            gap: 2.,
        };
        let m = check(&layout, &starter("Vise"), LONG.into(), None, &push).unwrap();
        assert_eq!(m.way, Way::Tip(None));
        assert_eq!(
            line("Finish", Some(&m)),
            "Finish: only ×0.5 against tipping; ×2 is wanted."
        );
        assert_eq!(
            issue("Finish", &m).unwrap(),
            "Finish: the cut could tip the stock out of the vise: about 786 N, 34 mm above the top of the jaws' contact, which is 2.9 mm high and 98 mm long (×0.5, under the ×2 wanted). Grip deeper, take a smaller stepdown or stepover or a lower feed, or choose another holder."
        );
    }
    #[test]
    fn the_jaws_contact_is_measured_from_where_the_stock_stands() {
        // Setup 1 faces the whole top with a Ø10 ball end mill, 2 mm deep
        // (tips at 38), a pass every 4 mm, which the box keeps; setup 2,
        // turned over, rests on the faced surface 2 mm above the box's
        // bottom, and the starter vise grips 8 mm from there.
        let mesh =
            crate::mesh::normalize(vec![[[0., 0., 0.], [20., 0., 0.], [0., 20., 10.]]]).unwrap();
        let f = frame();
        let mut flipped = Setup::new(Machine::new(3), "G54");
        flipped.p.insert("rx".into(), "180".into());
        let flipped = Frame::new(&flipped, &mesh);
        let mut v = Volume::new(STOCK, f);
        for i in 0..16 {
            let y = -30. + 4. * i as f32;
            v.cut(
                [-60., y, 38.],
                [60., y, 38.],
                5.,
                10.,
                CutterShape::Ball,
                f,
                (0, 0),
            );
        }
        let part = f.transfer_bounds(
            Bounds {
                min: [-40., -20., 3.],
                max: [40., 20., 37.],
            },
            flipped,
        );
        let left = crate::rest::RestStock::new(&v, flipped).left(part);
        let layout = Layout::Vise {
            holder: String::new(),
            axis_y: true,
            offset: 0.,
            grip: 8.,
            gap: 2.,
        };
        let vise = starter("Vise");
        let held =
            crate::holding::gripped(&layout, &vise, left, Some(&Bands::new(&v, flipped, part)));
        assert!((held.bottom - (left.min[2] + 2.)).abs() < 0.001, "{held:?}");
        let removed = Removed::new(&v, flipped);
        let nameless = |_, _| String::new();
        assert!(holdable(1, &layout, &vise, held, Some(&removed), &nameless).is_empty());
        // 500 N sideways 1 mm under the top: the contact is the 8 mm the
        // jaws reach above the faced surface (less the sampling's insets),
        // its top 10 mm above the box's bottom, 29 mm under the push.
        let push = [Push {
            side: 500.,
            up: 0.,
            z: left.max[2] - 1.,
        }];
        let m = check(&layout, &vise, held, Some(&removed), &push).unwrap();
        let [tall, long] = m.contact.unwrap();
        assert!(
            (tall - 7.84).abs() < 0.01 && (long - 98.).abs() < 0.1,
            "{m:?}"
        );
        assert!((m.lever - 29.08).abs() < 0.01, "{m:?}");
        // Without the earlier cuts the jaws' contact starts at the box's bottom.
        let whole = check(&layout, &vise, left.into(), None, &push).unwrap();
        assert!((whole.lever - 31.08).abs() < 0.01, "{whole:?}");
    }
    #[test]
    fn earlier_cuts_that_lower_the_jaws_contact_leave_a_later_setup_unheld() {
        let (vise, f) = (starter("Vise"), frame());
        let layout = Layout::Vise {
            holder: String::new(),
            axis_y: true,
            offset: 0.,
            grip: 8.,
            gap: 2.,
        };
        // 500 N sideways 1 mm under the top of the stock.
        let push = [Push {
            side: 500.,
            up: 0.,
            z: 39.,
        }];
        // Gripped 8 mm on the whole stock it holds, ×2.5 against tipping.
        let whole = check(&layout, &vise, STOCK.into(), None, &push).unwrap();
        assert_eq!(whole.way, Way::Tip(None));
        assert!((whole.factor - 2.52).abs() < 0.01, "{whole:?}");
        assert_eq!(issue("Face", &whole), None);
        // An earlier outside contour down to 5.5 mm above the bottom, over
        // the ±Y faces the jaws close on: they still find the most of the
        // material they grip, but only the lower 4.5 mm of their contact.
        let mut v = Volume::new(STOCK, f);
        for z in [5.5, 10.5, 15.5, 20.5, 25.5, 30.5, 35.5] {
            for y in [-30., 30.] {
                v.cut(
                    [-60., y, z],
                    [60., y, z],
                    5.,
                    5.,
                    CutterShape::Flat,
                    f,
                    (0, 2),
                );
            }
        }
        let part = Bounds {
            min: [-40., -20., 3.],
            max: [40., 20., 37.],
        };
        let held = crate::holding::gripped(&layout, &vise, STOCK, Some(&Bands::new(&v, f, part)));
        let removed = Removed::new(&v, f);
        let nameless = |_, _| String::new();
        assert!(holdable(1, &layout, &vise, held, Some(&removed), &nameless).is_empty());
        let cut = check(&layout, &vise, held, Some(&removed), &push).unwrap();
        assert!((cut.factor - 1.3).abs() < 0.01, "{cut:?}");
        assert_eq!(
            issue("Face", &cut).unwrap(),
            "Face: the cut could tip the stock out of the vise: about 500 N, 34 mm above the top of the jaws' contact, which is 4.5 mm high and 98 mm long (×1.3, under the ×2 wanted). Grip deeper, take a smaller stepdown or stepover or a lower feed, or choose another holder."
        );
    }
    #[test]
    fn one_clamp_lets_the_stock_tip_over_its_own_edge_and_a_second_across_holds_it() {
        let clamp = starter("Clamp");
        let clamps = |at: &[Side]| Layout::Clamps {
            holder: String::new(),
            clamps: at.iter().map(|side| (*side, 0.)).collect(),
            gap: 2.,
        };
        let push = [Push {
            side: 300.,
            up: 150.,
            z: 39.,
        }];
        // Its bite's centre 6 mm in from -X: 2000 N × 6 mm against 300 N × 39 mm.
        let one = check(&clamps(&[Side::MinusX]), &clamp, STOCK.into(), None, &push).unwrap();
        assert_eq!(one.way, Way::Tip(Some(Side::MinusX)));
        assert!((one.factor - 1.02).abs() < 0.01, "{one:?}");
        assert!(
            issue("Pocket", &one)
                .unwrap()
                .starts_with("Pocket: the cut could tip the stock over its -X bottom edge: about 300 N, 39 mm above the table (×1, under the ×2 wanted). Add a clamp on +X,")
        );
        let two = clamps(&[Side::MinusX, Side::PlusX]);
        // Its friction is then the least: 0.2 × (2 × 4000 N - 150 N) against 300 N.
        let two = check(&two, &clamp, STOCK.into(), None, &push).unwrap();
        assert_eq!(
            line("Pocket", Some(&two)),
            "Pocket: holds ×5.2 against sliding."
        );
    }
}
