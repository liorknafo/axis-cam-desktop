//! The step guide of a setup on a manual mill or a manual lathe: what to do
//! at the hand-fed machine, screen by screen, with the numbers its DRO (the
//! mill) or its dials (the lathe, which has no DRO) should read. It is made
//! from the setup's simulated plans, so every number is a move the app
//! checked. Every coordinate a step shows is a work coordinate (the stock
//! frame less the work zero); `Guide::zero` converts the kept plans.
use crate::{
    cam::{box_block, box_cut, open_sides},
    holding::{Gripped, Layout},
    lathe::{self, Slide},
    manual,
    mesh::{Bounds, Mesh},
    model::{Document, Machine, Strategy, get, num},
    post::At,
    simulation::{self, Motion, Move, POINT_RISE, Plan},
    turn,
    volume::Volume,
};
use std::ops::Range;

/// One DRO number to move to, in the order the axes are moved.
#[derive(Debug, Clone, PartialEq)]
pub struct Target {
    /// 1, 2, 3… within the pass.
    pub order: u8,
    /// 0 X, 1 Y, 2 Z.
    pub axis: usize,
    /// The tool centre at the end of the move.
    pub centre: f32,
    /// What the DRO reads there: the tool edge on X and Y with the DRO's
    /// compensation on, else the centre.
    pub shown: f32,
    /// `shown` is the tool edge.
    pub edge: bool,
    /// The move there cuts: a feed (a plunge, a feed across into the next
    /// row, the pass), not a rapid.
    pub feed: bool,
    /// The axes to lock: on the cut, the two that do not move; on a move
    /// that sets an axis for the last time before the cut, that axis.
    pub lock: [bool; 3],
}
/// The lowest Z and the extreme X and Y the tool's edge reaches in an
/// operation's feeds: the numbers not to pass.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Limits {
    pub x: [f32; 2],
    pub y: [f32; 2],
    pub z: f32,
}
/// A vise as the Hold step draws it: its jaws close across axis `across`
/// (0 X, 1 Y) and grip `grip` mm of the stock up from `Gripped::bottom`;
/// its jaw plates are `jaw_height` tall and `jaw_thickness` thick.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Vise {
    pub across: usize,
    pub grip: f32,
    pub jaw_height: f32,
    pub jaw_thickness: f32,
}
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// The stock box the setup starts from, what the vise or clamps hold,
    /// and the vise when it is one.
    Hold {
        stock: Bounds,
        holding: Gripped,
        vise: Option<Vise>,
    },
    /// The work zero, `corner` on the DRO, picked up with an edge finder of
    /// diameter `finder` and the DRO's compensation off: X and Y each touch
    /// one face (`approach`: −1 the − face from outside it, +1 the + face; a
    /// centre zero touches the − face) and are set there to `touch`, the
    /// face less or plus the finder's radius; Z touches the top, set to
    /// `touch[2]`.
    Zero {
        corner: [f32; 3],
        approach: [i8; 2],
        touch: [f32; 3],
        finder: f32,
    },
    /// Put in the tool `tool` (an id) at `rpm`, touch the stock's top with
    /// it and set Z to `top` (the top before the operation, work Z); with
    /// the DRO's compensation on, enter `diameter` and set X and Y to
    /// `comp` (0 off: centres).
    Tool {
        op: usize,
        tool: String,
        rpm: f32,
        diameter: f32,
        comp: [i8; 2],
        top: f32,
    },
    /// Same tool: only the compensation sides change.
    Comp { op: usize, comp: [i8; 2] },
    /// One pass of a Box, 1-based `index` of `of`. `cut` is what it sweeps
    /// out of the stock, `earlier` what the operation's passes before it did.
    Pass {
        op: usize,
        index: usize,
        of: usize,
        targets: Vec<Target>,
        cut: Bounds,
        earlier: Vec<Bounds>,
        never: Limits,
        turns_per_minute: f32,
    },
    /// A facing Box (open on all four sides): face the top down to `to`,
    /// taking off `remove`; the user plans the passes. The tool picks up Z
    /// on the top first, set to `top`, as on a Tool step.
    Face {
        op: usize,
        tool: String,
        rpm: f32,
        diameter: f32,
        top: f32,
        remove: f32,
        to: f32,
    },
    /// A squaring Box (one closed X or Y side, the wall, and a slab off it
    /// one row wide): cut the stock's `side` (−1 or +1) of `axis` to `to`
    /// (the wall as cut, with its allowance), which the DRO reads as `shown`
    /// (the edge with compensation, else the centre), taking off `remove`,
    /// down to `down_to` (the floor as cut). With the DRO's compensation on,
    /// enter `diameter` and set `axis` to its side in `comp` (toward the
    /// wall; the other axis 0); all 0 with it off. The tool picks up Z on
    /// the top first, set to `top`, as on a Tool step.
    Square {
        op: usize,
        tool: String,
        rpm: f32,
        diameter: f32,
        comp: [i8; 2],
        top: f32,
        axis: usize,
        side: i8,
        to: f32,
        shown: f32,
        remove: f32,
        down_to: f32,
    },
    /// The holes of a Drill in the order drilled: X and Y (centres) and the
    /// Z each is drilled to.
    Holes { op: usize, holes: Vec<[f32; 3]> },
    /// The stock after the operation.
    Check { op: usize },
    /// The stock leaves for setup `next` (none for the last setup).
    HandOver { next: Option<usize> },
    /// A step at the manual lathe.
    Lathe(LatheStep),
}
/// A number set on one of the lathe's dials: `lathe::dial` of `distance`.
#[derive(Debug, Clone, PartialEq)]
pub struct Dial {
    pub slide: Slide,
    /// How far the slide goes from its zero, mm: for the cross slide the
    /// amount off the diameter (onto it, boring) from the touch on it; for
    /// the carriage the travel toward the chuck from its zero on the end;
    /// for the top slide and the quill the travel along them.
    pub distance: f32,
    /// Whole turns and the reading, or why there is no number:
    /// `lathe::CHECK_THE_DIAL` while the cross-slide dial is not known, never
    /// a guessed reading.
    pub reading: Result<(u32, f32), String>,
}
impl Dial {
    pub fn new(machine: &Machine, slide: Slide, distance: f32) -> Self {
        Self {
            slide,
            distance,
            reading: lathe::dial(machine, slide, distance),
        }
    }
    /// No number until the dial check is answered.
    pub fn unknown(&self) -> bool {
        self.reading
            .as_ref()
            .is_err_and(|e| e == lathe::CHECK_THE_DIAL)
    }
    /// The distance the shown reading sets (`distance` to the dial's
    /// division): what the slide is really at once it is set.
    fn set(&self, machine: &Machine) -> Option<f32> {
        let (turns, reading) = *self.reading.as_ref().ok()?;
        let key = match self.slide {
            Slide::Cross => "cross_turn",
            Slide::Top => "top_turn",
            Slide::Carriage => "carriage_turn",
            Slide::Quill => "quill_turn",
        };
        let amount = turns as f32 * num(&machine.p, key)? + reading;
        Some(match (self.slide, lathe::cross_reads(machine)) {
            (Slide::Cross, Some("Radius")) => 2. * amount,
            _ => amount,
        })
    }
    /// "1 turn + 2.00", or why there is no number.
    pub fn text(&self) -> String {
        match &self.reading {
            Ok((0, r)) => format!("{r:.2}"),
            Ok((1, r)) => format!("1 turn + {r:.2}"),
            Ok((t, r)) => format!("{t} turns + {r:.2}"),
            Err(e) => e.clone(),
        }
    }
}
/// A diameter the tool touches to zero the cross-slide dial on it.
#[derive(Debug, Clone, PartialEq)]
pub struct Contact {
    /// The diameter there as planned, mm.
    pub planned: f32,
    /// What the user measured it at (the bar, at the Zero step), when this
    /// is that diameter.
    pub measured: Option<f32>,
    /// A hole, touched from inside.
    pub inside: bool,
}
impl Contact {
    /// The diameter the cross-slide readings are measured from.
    pub fn diameter(&self) -> f32 {
        self.measured.unwrap_or(self.planned)
    }
}
/// The key of a measurement in `Document::measured`: the setup's id and the
/// step's (`zero` for the bar at the Zero step, an operation's id for the
/// Measure before its last pass).
pub fn measure_key(setup: &str, step: &str) -> String {
    format!("{setup}/{step}")
}
/// The top slide's angle from the cross slide for a thread of
/// `thread_angle`: 29.5° for 60° (half of it less ½°, so the tool cuts on
/// its leading flank and only shaves the other), 27.5° for 55° (spec §6).
pub fn top_angle(thread_angle: f32) -> f32 {
    if (thread_angle - 55.).abs() < 1e-3 {
        27.5
    } else {
        thread_angle / 2. - 0.5
    }
}
/// How far the top slide, swung to `angle`° from the cross slide, travels to
/// feed the tool `infeed` mm in toward the axis: moving `s` along it moves the
/// tool `s·cos(angle)` in and `s·sin(angle)` toward the chuck, so
/// `s = infeed / cos(angle)`.
pub fn top_travel(infeed: f32, angle: f32) -> f32 {
    infeed / angle.to_radians().cos()
}
/// A step of a lathe setup's guide. Lengths in mm, diameters as diameters,
/// Z in lathe work coordinates (0 at the free end as the setup starts,
/// negative toward the chuck).
#[derive(Debug, Clone, PartialEq)]
pub enum LatheStep {
    /// Only while the machine's `cross_reads` is empty: turn a light skin, go
    /// in `divisions` on the cross dial (a reading of `reading`), cut
    /// `length` long and measure: the diameter went down by `reading` (the
    /// dial reads the diameter) or by twice it (the radius).
    DialCheck {
        divisions: u32,
        reading: f32,
        length: f32,
    },
    /// The bar in the chuck `holder` (an id) with `jaws` jaws: `grip` in the
    /// jaws, `stickout` from the jaws to the free end; `diameter` where the
    /// jaws close, `length` of the stock, and `reach`, how far from the end
    /// the deepest cut goes.
    Hold {
        holder: String,
        jaws: u32,
        grip: f32,
        stickout: f32,
        diameter: f32,
        length: f32,
        reach: f32,
    },
    /// With `tool`: touch the end → zero the carriage dial; measure the bar
    /// (`planned` Ø; `measured` under `key` once typed); touch it → zero the
    /// cross dial.
    Zero {
        tool: String,
        key: String,
        planned: f32,
        measured: Option<f32>,
    },
    /// Before an operation's cuts, with `tool` at `rpm`: with `top`, swing
    /// the top slide to that angle (a thread) and zero its dial on the
    /// touch; when `carriage`, touch the end (at `end`) → zero the carriage
    /// dial (a new tool, or the slides moved since the last zero); with
    /// `cross`, touch that diameter → zero the cross dial.
    Touch {
        op: usize,
        tool: String,
        rpm: f32,
        carriage: bool,
        end: f32,
        cross: Option<Contact>,
        top: Option<f32>,
    },
    /// One screen: lock the carriage, touch the end, the top slide (along
    /// the bed) in `face_off` on its dial, face across to `to_diameter`;
    /// then, the tool on the new end at `new_end`: zero the carriage dial
    /// there. Every later carriage reading is from the new end.
    Face {
        op: usize,
        tool: String,
        rpm: f32,
        face_off: f32,
        to_diameter: f32,
        top: Dial,
        new_end: f32,
    },
    /// Pass `index` of `of` of an Outside (or Bore, `bore`) to `diameter`:
    /// the cross dial in (out, boring) to `cross`, power feed at `feed`
    /// mm/rev toward the chuck, off at `feed_off` (1 mm before), the last
    /// mm by hand to `carriage`.
    Pass {
        op: usize,
        index: usize,
        of: usize,
        bore: bool,
        diameter: f32,
        rpm: f32,
        feed: f32,
        cross: Dial,
        carriage: Dial,
        feed_off: Option<Dial>,
    },
    /// Before the last pass: measure the diameter (`expect` as planned, the
    /// last pass to `target`); typed under `key`, it sets the last pass's
    /// cross reading: what is left (measured − target, boring the other
    /// way) all on that pass.
    Measure {
        op: usize,
        key: String,
        expect: f32,
        target: f32,
        bore: bool,
        measured: Option<f32>,
    },
    /// The tailstock: centre drill, then the `diameter` drill `depth` deep
    /// from where it touches the end plus `point`, on the quill dial
    /// (`quill`, zeroed on the touch), pecking every `peck`.
    TailDrill {
        op: usize,
        tool: String,
        rpm: f32,
        diameter: f32,
        depth: f32,
        point: f32,
        peck: f32,
        quill: Dial,
    },
    /// A 45° chamfer of leg `size` on `edge`: the carriage at `carriage`
    /// (on a shoulder; the end edges are touched), locked, the top slide in
    /// `top` from the touch on the corner.
    Chamfer {
        op: usize,
        tool: String,
        rpm: f32,
        edge: String,
        size: f32,
        top: Dial,
        carriage: Option<Dial>,
    },
    /// A groove (or a part off, `part_off`) with a `width` blade: the
    /// carriage locked with the tool's left side at each of `carriages`, the
    /// cross dial in to `cross` (the `bottom` Ø; to the centre when parting
    /// off, None).
    Plunge {
        op: usize,
        tool: String,
        rpm: f32,
        width: f32,
        part_off: bool,
        bottom: f32,
        cross: Option<Dial>,
        carriages: Vec<Dial>,
    },
    /// Thread pass `index` of `of` at `pitch` on the gearbox, the top slide
    /// at `angle`°: the cross slide back to its zero, the top slide in to
    /// `top` (`infeed` toward the axis from the touch), the half-nuts closed
    /// to `carriage`, then back out and reverse.
    ThreadPass {
        op: usize,
        index: usize,
        of: usize,
        rpm: f32,
        pitch: f32,
        angle: f32,
        infeed: f32,
        top: Dial,
        carriage: Dial,
    },
}
impl LatheStep {
    /// The step's name, as a screen's title.
    pub fn title(&self) -> &'static str {
        match self {
            LatheStep::DialCheck { .. } => "Dial check",
            LatheStep::Hold { .. } => "Hold",
            LatheStep::Zero { .. } => "Zero",
            LatheStep::Touch { .. } => "Touch",
            LatheStep::Face { .. } => "Face",
            LatheStep::Pass { .. } => "Pass",
            LatheStep::Measure { .. } => "Measure",
            LatheStep::TailDrill { .. } => "Drill",
            LatheStep::Chamfer { .. } => "Chamfer",
            LatheStep::Plunge { part_off: true, .. } => "Part off",
            LatheStep::Plunge { .. } => "Groove",
            LatheStep::ThreadPass { .. } => "Thread pass",
        }
    }
}
/// A stretch of the steps: "Set up", one per operation, "Hand over".
#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    pub name: String,
    pub op: Option<usize>,
    pub steps: Range<usize>,
    /// Passes or holes; 0 for the others.
    pub count: usize,
    pub minutes: f32,
}
/// What an operation of the setup did, for the pictures.
#[derive(Debug, Clone)]
pub struct Done {
    pub op: usize,
    /// In the stock frame; its `stock` is the box the setup starts from.
    pub plan: Plan,
    /// The stock after it.
    pub after: Volume,
}
#[derive(Debug, Clone)]
pub struct Guide {
    pub setup: usize,
    pub steps: Vec<Step>,
    pub sections: Vec<Section>,
    pub minutes: f32,
    /// The work zero in the stock frame: a work coordinate is a stock one less it.
    pub zero: [f32; 3],
    /// The stock before the setup's first operation.
    pub before: Volume,
    /// The enabled operations in order.
    pub done: Vec<Done>,
}
/// What a Box is on the hand-fed mill (`cam::open_sides`).
#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    Face,
    /// The closed side, the wall, is on `axis`, on its − side when `low`.
    Square {
        axis: usize,
        low: bool,
    },
    Other,
}
/// A Box with its sides `open` that cuts `lo`..`hi` (`cam::box_cut`) with a
/// tool of radius `r`: facing when all four X and Y sides are open;
/// squaring when exactly one is closed (the wall; its ends along the wall
/// are then open) and the slab off the wall is at most `r` wide, so one row
/// per level takes it, whatever its floor; otherwise passes.
fn kind(open: [[bool; 2]; 3], lo: [f32; 3], hi: [f32; 3], r: f32) -> Kind {
    let closed: Vec<(usize, bool)> = (0..2)
        .flat_map(|k| [(k, true), (k, false)])
        .filter(|&(k, low)| !open[k][usize::from(!low)])
        .collect();
    match closed[..] {
        [] => Kind::Face,
        [(axis, low)] if hi[axis] - lo[axis] <= r + crate::cam::ON => Kind::Square { axis, low },
        _ => Kind::Other,
    }
}
/// The DRO's compensation side on X and Y: toward the box's closed side,
/// + when both or neither are closed.
fn comp_sides(open: [[bool; 2]; 3]) -> [i8; 2] {
    std::array::from_fn(|k| if open[k] == [false, true] { -1 } else { 1 })
}
/// The one axis a manual move changes.
fn moved(m: &Move) -> Result<usize, String> {
    match (0..3).filter(|&k| m.from[k] != m.to[k]).collect::<Vec<_>>()[..] {
        [k] => Ok(k),
        _ => Err("A move on the manual mill changes more than one axis.".into()),
    }
}
/// The targets of `legs` (axis, stock-frame end, a feed), the last of them
/// the cut, as the DRO shows them.
fn targets(legs: &[(usize, f32, bool)], zero: [f32; 3], comp: [i8; 2], r: f32) -> Vec<Target> {
    let last = legs.len() - 1;
    let cut = legs[last].0;
    (legs.iter().enumerate())
        .map(|(i, &(axis, at, feed))| {
            let centre = at - zero[axis];
            let side = comp.get(axis).copied().unwrap_or(0);
            let lock = std::array::from_fn(|k| match i == last {
                true => k != axis,
                false => k == axis && k != cut && legs[i + 1..].iter().all(|l| l.0 != k),
            });
            Target {
                order: u8::try_from(i + 1).unwrap_or(u8::MAX),
                axis,
                centre,
                shown: centre + f32::from(side) * r,
                edge: side != 0,
                feed,
                lock,
            }
        })
        .collect()
}
/// One Pass step per cut of a Box's `plan`.
fn passes(
    op: usize,
    plan: &Plan,
    zero: [f32; 3],
    comp: [i8; 2],
    machine: &Machine,
) -> Result<Vec<Step>, String> {
    let (r, s) = (plan.radius, plan.stock);
    let work = |b: Bounds| b.translated(zero.map(|v| -v));
    let (mut lo, mut hi) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
    for m in plan.moves.iter().filter(|m| m.motion == Motion::Feed) {
        for p in [m.from, m.to] {
            for k in 0..3 {
                (lo[k], hi[k]) = (lo[k].min(p[k]), hi[k].max(p[k]));
            }
        }
    }
    let never = Limits {
        x: [lo[0] - r - zero[0], hi[0] + r - zero[0]],
        y: [lo[1] - r - zero[1], hi[1] + r - zero[1]],
        z: lo[2] - zero[2],
    };
    let cuts: Vec<&Move> = plan.moves.iter().filter(|m| m.cutting_pass).collect();
    let top = plan.rest_top.unwrap_or(s.max[2]);
    let (mut steps, mut earlier, mut legs) = (vec![], vec![], vec![]);
    for m in plan.moves.iter().filter(|m| m.motion != Motion::Dwell) {
        let axis = moved(m)?;
        legs.push((axis, m.to[axis], m.motion == Motion::Feed));
        if !m.cutting_pass {
            continue;
        }
        if steps.is_empty() {
            // Where the tool is before the first pass is not known: the axes
            // the plan does not move from its start are set before the plunge.
            let before = &legs[..legs.len() - 1];
            let missing: Vec<(usize, f32, bool)> = (0..3)
                .filter(|&k| before.iter().all(|l| l.0 != k))
                .map(|k| (k, m.from[k], false))
                .collect();
            let at = before
                .iter()
                .rposition(|l| l.0 == 2)
                .unwrap_or(before.len());
            legs.splice(at..at, missing);
        }
        let targets = targets(&legs, zero, comp, r);
        legs.clear();
        // From the level above (or the top of the material) down to this one.
        let above = cuts
            .iter()
            .map(|c| c.to[2])
            .filter(|z| *z > m.to[2])
            .fold(top, f32::min);
        let cut = Bounds {
            min: std::array::from_fn(|k| match k {
                2 => m.to[2],
                _ => m.from[k].min(m.to[k]) - r,
            }),
            max: std::array::from_fn(|k| match k {
                2 => above,
                _ => m.from[k].max(m.to[k]) + r,
            }),
        };
        let cut = work(Bounds {
            min: std::array::from_fn(|k| cut.min[k].max(s.min[k])),
            max: std::array::from_fn(|k| cut.max[k].min(s.max[k])),
        });
        steps.push(Step::Pass {
            op,
            index: steps.len() + 1,
            of: cuts.len(),
            targets,
            cut,
            earlier: earlier.clone(),
            never,
            turns_per_minute: manual::turns(machine, axis, m.feed),
        });
        earlier.push(cut);
    }
    Ok(steps)
}
/// The holes of a Drill's `plan` in the order drilled: centre and depth.
fn holes(plan: &Plan, zero: [f32; 3]) -> Vec<[f32; 3]> {
    let mut holes: Vec<[f32; 3]> = vec![];
    for m in plan.moves.iter().filter(|m| m.cutting_pass) {
        let at: [f32; 3] = std::array::from_fn(|k| m.to[k] - zero[k]);
        match holes.last_mut() {
            Some(h) if h[0] == at[0] && h[1] == at[1] => h[2] = h[2].min(at[2]),
            _ => holes.push(at),
        }
    }
    holes
}
/// A setup's enabled operations as simulated: (operation index, plan) in
/// order, the stock after each, and the stock before the first.
struct Walked {
    done: Vec<(usize, Plan)>,
    afters: Vec<Volume>,
    before: Volume,
}
/// The operations of setup `si` simulated in order up to its last enabled
/// one; `empty` when it has none.
fn simulate(doc: &Document, mesh: &Mesh, si: usize, empty: &str) -> Result<Walked, String> {
    let setup = doc.setups.get(si).ok_or("Choose a setup.")?;
    let last = (setup.operations.iter())
        .rposition(|o| o.enabled)
        .ok_or(empty)?;
    let (mut done, mut afters, mut before, mut error) = (vec![], vec![], None, None);
    let mut end = None;
    simulation::sequence_with(
        doc,
        mesh,
        Some((si, last)),
        |s, oi, result| match result {
            Ok(mut sim) if s == si => {
                if before.is_none() {
                    before = sim.initial_volume.clone();
                }
                if oi == last {
                    // The walk stops before playing its last operation.
                    done.push((oi, sim.plan.clone()));
                    sim.seek(1.);
                    end = sim.volume;
                } else {
                    done.push((oi, sim.plan));
                }
            }
            Err(e) if s == si && error.is_none() => error = Some(e),
            _ => {}
        },
        |s, _, volume| {
            if s == si {
                afters.push(volume.clone());
            }
        },
    );
    if let Some(error) = error {
        return Err(error);
    }
    afters.extend(end);
    let before = before.ok_or("The stock before the setup is not known.")?;
    if done.is_empty() || afters.len() != done.len() {
        return Err("The setup's operations did not all simulate.".into());
    }
    Ok(Walked {
        done,
        afters,
        before,
    })
}
/// The guide of setup `si`, a setup on a manual mill or a manual lathe
/// (`lathe_guide`): the document's operations simulated in order up to the
/// end of it.
pub fn build(doc: &Document, mesh: &Mesh, si: usize) -> Result<Guide, String> {
    let setup = doc.setups.get(si).ok_or("Choose a setup.")?;
    let machine = &setup.machine;
    if lathe::is_lathe(machine) {
        let walked = simulate(
            doc,
            mesh,
            si,
            "Add a Turn or Drill operation to see the steps.",
        )?;
        return lathe_guide(doc, mesh, si, walked);
    }
    if !manual::is_manual(machine) {
        return Err("Steps are for a setup on a manual mill or lathe.".into());
    }
    let Walked {
        done,
        afters,
        before,
    } = simulate(
        doc,
        mesh,
        si,
        "Add a Box or Drill operation to see the steps.",
    )?;
    let stock = done[0].1.stock;
    let zero = crate::post::datum(setup, stock, mesh)?;
    let work = |b: Bounds| b.translated(zero.map(|v| -v));
    let dro = get(&machine.p, "dro_comp") == "Yes";
    let tool_time = num(&machine.p, "tool_time").unwrap_or(0.);

    // Set up.
    let frame = crate::volume::Frame::new(setup, mesh);
    let bands = (!before.cuts.is_empty())
        .then(|| crate::holding::Bands::new(&before, frame, simulation::part_box(setup, mesh)));
    let held = crate::holding::held_by(doc, setup, stock, bands.as_ref());
    let vise = match crate::holding::layout(setup) {
        Ok(crate::holding::Layout::Vise { axis_y, grip, .. }) => {
            (doc.holder(get(&setup.p, "holder"))).map(|h| Vise {
                across: usize::from(axis_y),
                grip,
                jaw_height: num(&h.p, "jaw_height").unwrap_or(grip).max(grip),
                jaw_thickness: num(&h.p, "jaw_thickness").unwrap_or(0.),
            })
        }
        _ => None,
    };
    let origin = get(&setup.p, "origin");
    let found = crate::post::references(origin)
        .zip(crate::post::touch_point(origin, stock))
        .ok_or("Choose a work zero for the setup.")?;
    let finder = num(&machine.p, "edge_finder")
        .filter(|d| *d > 0.)
        .ok_or("Enter the edge finder's diameter on the manual mill.")?;
    let approach = found.0.map(|a| if a == At::Max { 1 } else { -1 });
    let mut steps = vec![
        Step::Hold {
            stock: work(stock),
            holding: Gripped {
                stock: work(held.stock),
                band: work(held.band),
                bottom: held.bottom - zero[2],
                girth: held.girth,
            },
            vise,
        },
        Step::Zero {
            corner: std::array::from_fn(|k| found.1[k] - zero[k]),
            approach,
            touch: std::array::from_fn(|k| match approach.get(k) {
                Some(1) => stock.max[k] + finder / 2. - zero[k],
                Some(_) => stock.min[k] - finder / 2. - zero[k],
                None => stock.max[2] - zero[2],
            }),
            finder,
        },
    ];
    let mut sections = vec![Section {
        name: "Set up".into(),
        op: None,
        steps: 0..2,
        count: 0,
        minutes: 0.,
    }];

    // One section per operation.
    let mut last_tool: Option<&str> = None;
    // The tool, rpm and compensation sides the last operation left set.
    let mut set: Option<(&str, f32, Option<[i8; 2]>)> = None;
    let mut done_ops = vec![];
    for ((oi, plan), after) in done.into_iter().zip(afters) {
        let op = &setup.operations[oi];
        let tool = op.tool.as_str();
        let rpm = num(&op.p, "rpm").unwrap_or(0.);
        let (r, first) = (plan.radius, steps.len());
        let diameter = 2. * r;
        let changed = last_tool.replace(tool) != Some(tool);
        let minutes = plan.duration / 60. + if changed { tool_time } else { 0. };
        let mut count = 0;
        // The stock before this operation.
        let earlier = done_ops.last().map_or(&before, |d: &Done| &d.after);
        let left = simulation::leftover_box(earlier, setup, mesh);
        // Where a new tool picks up Z: the highest top left.
        let top = left.max[2] - zero[2];
        let mut sides = |comp: [i8; 2], steps: &mut Vec<Step>| {
            let new = (set.as_ref())
                .is_some_and(|&(t, s, c)| t == tool && s == rpm && c.is_some_and(|c| c != comp));
            steps.push(match new {
                true => Step::Comp { op: oi, comp },
                false => Step::Tool {
                    op: oi,
                    tool: tool.into(),
                    rpm,
                    diameter,
                    comp,
                    top,
                },
            });
            set = Some((tool, rpm, Some(comp)));
        };
        match op.strategy {
            Strategy::Box => {
                let (low, high) = box_block(op, zero)?;
                let open = open_sides(low, high, &plan.stock);
                let allowance = crate::cam::number(op, "allowance")?;
                let (lo, hi) = box_cut(low, high, &plan.stock, allowance);
                let comp = if dro { comp_sides(open) } else { [0; 2] };
                match kind(open, lo, hi, r) {
                    Kind::Face => {
                        steps.push(Step::Face {
                            op: oi,
                            tool: tool.into(),
                            rpm,
                            diameter,
                            top,
                            remove: plan.rest_top.unwrap_or(stock.max[2]) - plan.target,
                            to: plan.target - zero[2],
                        });
                        set = Some((tool, rpm, None));
                    }
                    Kind::Square { axis, low } => {
                        let side: i8 = if low { 1 } else { -1 };
                        // The cut nearest the wall: its centre, and its edge on the wall.
                        let centres = (plan.moves.iter())
                            .filter(|m| m.motion == Motion::Feed)
                            .flat_map(|m| [m.from[axis], m.to[axis]]);
                        let centre = match low {
                            true => centres.fold(f32::INFINITY, f32::min),
                            false => centres.fold(f32::NEG_INFINITY, f32::max),
                        };
                        let wall = centre - f32::from(side) * r;
                        steps.push(Step::Square {
                            op: oi,
                            tool: tool.into(),
                            rpm,
                            diameter,
                            comp: std::array::from_fn(|k| if k == axis { comp[k] } else { 0 }),
                            top,
                            axis,
                            side,
                            to: wall - zero[axis],
                            shown: (if dro { wall } else { centre }) - zero[axis],
                            remove: if low {
                                left.max[axis] - wall
                            } else {
                                wall - left.min[axis]
                            },
                            down_to: plan.target - zero[2],
                        });
                        set = Some((tool, rpm, None));
                    }
                    Kind::Other => {
                        sides(comp, &mut steps);
                        let passes = passes(oi, &plan, zero, comp, machine)?;
                        count = passes.len();
                        steps.extend(passes);
                        steps.push(Step::Check { op: oi });
                    }
                }
            }
            Strategy::Drill => {
                sides([0; 2], &mut steps);
                let holes = holes(&plan, zero);
                count = holes.len();
                steps.push(Step::Holes { op: oi, holes });
                steps.push(Step::Check { op: oi });
            }
            other => return Err(format!("{} does not run on a manual mill.", other.name())),
        }
        sections.push(Section {
            name: op.name().into(),
            op: Some(oi),
            steps: first..steps.len(),
            count,
            minutes,
        });
        done_ops.push(Done {
            op: oi,
            plan,
            after,
        });
    }
    let n = steps.len();
    steps.push(Step::HandOver {
        next: (si + 1 < doc.setups.len()).then_some(si + 1),
    });
    sections.push(Section {
        name: "Hand over".into(),
        op: None,
        steps: n..n + 1,
        count: 0,
        minutes: 0.,
    });
    Ok(Guide {
        setup: si,
        minutes: sections.iter().map(|s| s.minutes).sum(),
        steps,
        sections,
        zero,
        before,
        done: done_ops,
    })
}
/// `v` to the thousandth of a mm.
fn mm(v: f32) -> f32 {
    (v * 1000.).round() / 1000.
}
/// Where the lathe's carriage dial is zeroed, as the guide goes.
struct Zeroes<'a> {
    machine: &'a Machine,
    stock: Bounds,
    /// The setup-frame Z of the free end as it is now.
    end: f32,
    /// The tool the carriage dial was last zeroed with, and the setup-frame
    /// Z it was zeroed at; None once the slides moved since (a thread swings
    /// the top slide, a chamfer feeds it in).
    carriage: Option<(String, f32)>,
}
impl Zeroes<'_> {
    /// The Touch step before operation `op` with `tool` at `rpm`, when it
    /// has anything to zero: the carriage when `carriage` is asked and it was
    /// zeroed with another tool or not since the slides moved (or the top
    /// slide swings to `top`), the cross dial on `cross`. The setup-frame Z
    /// the carriage readings are then from.
    fn touch(
        &mut self,
        steps: &mut Vec<Step>,
        (op, tool, rpm): (usize, &str, f32),
        carriage: bool,
        cross: Option<Contact>,
        top: Option<f32>,
    ) -> f32 {
        let again =
            carriage && (top.is_some() || self.carriage.as_ref().is_none_or(|(t, _)| t != tool));
        if again {
            self.carriage = Some((tool.into(), self.end));
        }
        if again || cross.is_some() || top.is_some() {
            steps.push(Step::Lathe(LatheStep::Touch {
                op,
                tool: tool.into(),
                rpm,
                carriage: again,
                end: mm(turn::work_z(self.stock, self.end)),
                cross,
                top,
            }));
        }
        self.carriage.as_ref().map_or(self.end, |c| c.1)
    }
    fn dial(&self, slide: Slide, distance: f32) -> Dial {
        Dial::new(self.machine, slide, mm(distance))
    }
}
/// The guide of setup `si`, a setup on the manual lathe: Set up (the dial
/// check while the cross-slide dial is not known, Hold, Zero), one section
/// per operation, Hand over. Every number is a dial reading
/// (`lathe::dial`): the cross slide from a touch on the diameter, the
/// carriage from its zero on the end (the new end after a Face), the top
/// slide for Face, Chamfer and Thread infeed, the quill for drills.
fn lathe_guide(doc: &Document, mesh: &Mesh, si: usize, walked: Walked) -> Result<Guide, String> {
    let Walked {
        done,
        afters,
        before,
    } = walked;
    let setup = &doc.setups[si];
    let machine = &setup.machine;
    let stock = done[0].1.stock;
    let zero = crate::post::datum(setup, stock, mesh)?;
    let frame = crate::volume::Frame::new(setup, mesh);
    let end = turn::end(stock);
    let tool_time = num(&machine.p, "tool_time").unwrap_or(0.);
    let material =
        |v: &Volume| turn::Material::of(stock, false, Some(&crate::rest::coaxial(v, frame)));
    let tool_of = |id: &str| doc.tools.iter().find(|t| t.id == id);
    let mut steps = vec![];

    // Set up.
    if lathe::cross_reads(machine).is_none() {
        let reading = num(&machine.p, "cross_turn")
            .zip(num(&machine.p, "cross_div"))
            .filter(|(_, d)| *d > 0.)
            .map_or(0., |(t, d)| 10. * t / d);
        steps.push(Step::Lathe(LatheStep::DialCheck {
            divisions: 10,
            reading: mm(reading),
            length: 30.,
        }));
    }
    let layout = crate::holding::layout(setup)?;
    let Layout::Chuck {
        holder,
        grip,
        stickout,
    } = &layout
    else {
        return Err("Hold the bar in a chuck: choose Chuck on the setup.".into());
    };
    let chuck = (doc.holder(holder)).ok_or("Choose a chuck from the Workholding library.")?;
    let first = material(&before);
    let (bar, _) = (first.girth(stock.min[2], stock.max[2])).ok_or("No stock is left to turn.")?;
    let bar = mm(bar);
    let coaxial = crate::rest::coaxial(&before, frame);
    let held = crate::holding::girth(setup, si == 0, &layout, stock, Some(&coaxial));
    let deepest = (done.iter())
        .flat_map(|(_, p)| p.moves.iter().filter(|m| m.motion == Motion::Feed))
        .map(|m| m.to[2])
        .fold(end, f32::min);
    steps.push(Step::Lathe(LatheStep::Hold {
        holder: holder.clone(),
        jaws: num(&chuck.p, "jaws").unwrap_or(3.) as u32,
        grip: *grip,
        stickout: *stickout,
        diameter: mm(held.map_or(bar, |g| g.grip)),
        length: mm(stock.max[2] - stock.min[2]),
        reach: mm(end - deepest),
    }));
    let zero_key = measure_key(&setup.id, "zero");
    let typed = |key: &str| (doc.measured.get(key).copied()).filter(|v| v.is_finite() && *v > 0.);
    let bar_measured = typed(&zero_key);
    let mut zeroes = Zeroes {
        machine,
        stock,
        end,
        carriage: None,
    };
    let first_turn = (done.iter())
        .map(|(oi, _)| &setup.operations[*oi])
        .find(|o| o.strategy == Strategy::Turn);
    if let Some(op) = first_turn {
        steps.push(Step::Lathe(LatheStep::Zero {
            tool: op.tool.clone(),
            key: zero_key,
            planned: bar,
            measured: bar_measured,
        }));
        zeroes.carriage = Some((op.tool.clone(), end));
    }
    // A touch on the bar as it came is a touch on what was measured there.
    let contact = |planned: f32, inside: bool| Contact {
        planned,
        measured: bar_measured.filter(|_| !inside && (planned - bar).abs() < 1e-3),
        inside,
    };
    let mut sections = vec![Section {
        name: "Set up".into(),
        op: None,
        steps: 0..steps.len(),
        count: 0,
        minutes: 0.,
    }];

    // One section per operation.
    let mut last_tool: Option<String> = None;
    let mut done_ops: Vec<Done> = vec![];
    for ((oi, plan), after) in done.into_iter().zip(afters) {
        let op = &setup.operations[oi];
        let tool = op.tool.as_str();
        let rpm = num(&op.p, "rpm").unwrap_or(0.);
        let changed = last_tool.replace(tool.into()).as_deref() != Some(tool);
        let minutes = plan.duration / 60. + if changed { tool_time } else { 0. };
        let begin = steps.len();
        let mat = material(done_ops.last().map_or(&before, |d| &d.after));
        let feeds: Vec<&Move> = (plan.moves.iter())
            .filter(|m| m.motion == Motion::Feed)
            .collect();
        let along: Vec<&Move> = (feeds.iter().copied())
            .filter(|m| (m.from[0] - m.to[0]).abs() < 1e-6)
            .collect();
        let across: Vec<&Move> = (feeds.iter().copied())
            .filter(|m| (m.from[2] - m.to[2]).abs() < 1e-6)
            .collect();
        let kind = match op.strategy {
            Strategy::Drill => "Drill",
            _ => get(&op.p, "turn_kind"),
        };
        let missing = || format!("{} has no cut to guide.", op.name());
        // The widest outside, or the narrowest hole, from `z` to the end.
        let outer = |z: f32| {
            (mat.sections(z, end).iter())
                .map(|s| s.outer)
                .fold(0., f32::max)
        };
        let hole = |z: f32| {
            (mat.sections(z, end).iter())
                .map(|s| s.inner)
                .fold(f32::INFINITY, f32::min)
        };

        let count = match kind {
            "Face" => {
                let z = plan.target;
                let face_off = num(&op.p, "face_off").unwrap_or(0.);
                steps.push(Step::Lathe(LatheStep::Face {
                    op: oi,
                    tool: tool.into(),
                    rpm,
                    face_off,
                    to_diameter: num(&op.p, "to_diameter").unwrap_or(0.),
                    top: zeroes.dial(Slide::Top, face_off),
                    new_end: mm(turn::work_z(stock, z)),
                }));
                // The tool is on the new end: the carriage is zeroed there.
                zeroes.end = z;
                zeroes.carriage = Some((tool.into(), z));
                1
            }
            "Outside" | "Bore" => {
                let bore = kind == "Bore";
                let z0 = along.first().ok_or_else(missing)?.to[2];
                let planned = mm(2. * if bore { hole(z0) } else { outer(z0) });
                let touch = contact(planned, bore);
                let from = touch.diameter();
                let zc = zeroes.touch(&mut steps, (oi, tool, rpm), true, Some(touch), None);
                let stop = zc - z0;
                let key = measure_key(&setup.id, &op.id);
                let measured = typed(&key);
                let n = along.len();
                let mut last: Option<(f32, Dial)> = None;
                for (i, m) in along.iter().enumerate() {
                    let d = mm(2. * m.to[0]);
                    let off = |to: f32| if bore { to - from } else { from - to };
                    let mut cross = zeroes.dial(Slide::Cross, off(d));
                    if i + 1 == n {
                        let expect = last.as_ref().map_or(from, |l| l.0);
                        steps.push(Step::Lathe(LatheStep::Measure {
                            op: oi,
                            key: key.clone(),
                            expect,
                            target: d,
                            bore,
                            measured,
                        }));
                        if let Some(was) = measured {
                            // From where the dial is now, all of what is left.
                            let at = last
                                .as_ref()
                                .map_or(0., |l| l.1.set(machine).unwrap_or(l.1.distance));
                            let left = if bore { d - was } else { was - d };
                            let past = off(was) < -1e-3;
                            cross = if left < -1e-3 {
                                Dial {
                                    reading: Err(format!("Ø{was:.2} is past Ø{d:.2} already.")),
                                    ..cross
                                }
                            } else if past {
                                Dial {
                                    reading: Err(format!(
                                        "Ø{was:.2} is not what was cut from Ø{from:.2}: measure again."
                                    )),
                                    ..cross
                                }
                            } else {
                                zeroes.dial(Slide::Cross, at + left.max(0.))
                            };
                        }
                    }
                    steps.push(Step::Lathe(LatheStep::Pass {
                        op: oi,
                        index: i + 1,
                        of: n,
                        bore,
                        diameter: d,
                        rpm,
                        feed: if rpm > 0. { mm(m.feed / rpm) } else { 0. },
                        cross: cross.clone(),
                        carriage: zeroes.dial(Slide::Carriage, stop),
                        feed_off: (stop > 1. + 1e-3)
                            .then(|| zeroes.dial(Slide::Carriage, stop - 1.)),
                    }));
                    last = Some((d, cross));
                }
                n
            }
            "Drill" => {
                feeds.first().ok_or_else(missing)?;
                let diameter = tool_of(tool)
                    .and_then(|t| num(&t.p, "diameter"))
                    .unwrap_or(0.);
                // Where the drill touches: the nearest material to the
                // tailstock narrower than the drill (as `turn` plans it).
                let touch = (mat.sections(stock.min[2], end).iter())
                    .filter(|s| s.inner < diameter / 2. - 1e-3)
                    .map(|s| s.z[1])
                    .fold(f32::NEG_INFINITY, f32::max);
                let quill = touch - plan.target;
                let point = mm(diameter / 2. * POINT_RISE);
                steps.push(Step::Lathe(LatheStep::TailDrill {
                    op: oi,
                    tool: tool.into(),
                    rpm,
                    diameter,
                    depth: mm(quill - point),
                    point,
                    peck: diameter,
                    quill: zeroes.dial(Slide::Quill, quill),
                }));
                1
            }
            "Chamfer" => {
                let cut = feeds.first().ok_or_else(missing)?;
                let edge = get(&op.p, "edge");
                let size = num(&op.p, "size").unwrap_or(0.);
                let shoulder = edge == "Shoulder";
                let zc = zeroes.touch(&mut steps, (oi, tool, rpm), shoulder, None, None);
                steps.push(Step::Lathe(LatheStep::Chamfer {
                    op: oi,
                    tool: tool.into(),
                    rpm,
                    edge: edge.into(),
                    size,
                    top: zeroes.dial(Slide::Top, size),
                    carriage: shoulder.then(|| zeroes.dial(Slide::Carriage, zc - cut.from[2])),
                }));
                // The top slide was fed in: the carriage zero no longer holds.
                zeroes.carriage = None;
                1
            }
            "Groove" | "Part off" => {
                let part_off = kind == "Part off";
                let first = across.first().ok_or_else(missing)?;
                let width = tool_of(tool).and_then(|t| num(&t.p, "width")).unwrap_or(0.);
                let low = across.iter().map(|m| m.to[2]).fold(f32::INFINITY, f32::min);
                let high = across.iter().map(|m| m.to[2]).fold(low, f32::max) + width;
                let planned = mm(2.
                    * (mat.sections(low, high).iter())
                        .map(|s| s.outer)
                        .fold(0., f32::max));
                let touch = (!part_off).then(|| contact(planned, false));
                let from = touch.as_ref().map_or(planned, Contact::diameter);
                let zc = zeroes.touch(&mut steps, (oi, tool, rpm), true, touch, None);
                let bottom = if part_off { 0. } else { mm(2. * first.to[0]) };
                steps.push(Step::Lathe(LatheStep::Plunge {
                    op: oi,
                    tool: tool.into(),
                    rpm,
                    width,
                    part_off,
                    bottom,
                    cross: (!part_off).then(|| zeroes.dial(Slide::Cross, from - bottom)),
                    carriages: (across.iter())
                        .map(|m| zeroes.dial(Slide::Carriage, zc - m.to[2]))
                        .collect(),
                }));
                if part_off {
                    // What is left in the chuck ends at the blade's left side.
                    zeroes.end = low;
                }
                across.len()
            }
            "Thread" => {
                let z0 = along.first().ok_or_else(missing)?.to[2];
                let inside = get(&op.p, "thread_side") == "Inside";
                let pitch = num(&op.p, "pitch").unwrap_or(0.);
                let angle = top_angle(
                    tool_of(tool)
                        .and_then(|t| num(&t.p, "thread_angle"))
                        .unwrap_or(60.),
                );
                let planned = mm(2. * if inside { hole(z0) } else { outer(z0) });
                let touch = contact(planned, inside);
                let r0 = touch.diameter() / 2.;
                let zc = zeroes.touch(&mut steps, (oi, tool, rpm), true, Some(touch), Some(angle));
                let n = along.len();
                for (i, m) in along.iter().enumerate() {
                    // In toward the axis from the touch (out, inside).
                    let infeed = mm(if inside { m.to[0] - r0 } else { r0 - m.to[0] }.max(0.));
                    steps.push(Step::Lathe(LatheStep::ThreadPass {
                        op: oi,
                        index: i + 1,
                        of: n,
                        rpm,
                        pitch,
                        angle,
                        infeed,
                        top: zeroes.dial(Slide::Top, top_travel(infeed, angle)),
                        carriage: zeroes.dial(Slide::Carriage, zc - z0),
                    }));
                }
                // The top slide is swung and fed in: the carriage zero no
                // longer holds.
                zeroes.carriage = None;
                n
            }
            other => return Err(format!("\"{other}\" does not run on the manual lathe.")),
        };
        sections.push(Section {
            name: op.name().into(),
            op: Some(oi),
            steps: begin..steps.len(),
            count,
            minutes,
        });
        done_ops.push(Done {
            op: oi,
            plan,
            after,
        });
    }
    let n = steps.len();
    steps.push(Step::HandOver {
        next: (si + 1 < doc.setups.len()).then_some(si + 1),
    });
    sections.push(Section {
        name: "Hand over".into(),
        op: None,
        steps: n..n + 1,
        count: 0,
        minutes: 0.,
    });
    Ok(Guide {
        setup: si,
        minutes: sections.iter().map(|s| s.minutes).sum(),
        steps,
        sections,
        zero,
        before,
        done: done_ops,
    })
}
/// The lathe `guide` worked out again for `doc` from its own simulation,
/// which neither a measurement nor the dial check's answer changes: the
/// cross-slide numbers and the measure steps as they are now, at once.
pub fn again(doc: &Document, mesh: &Mesh, guide: &Guide) -> Result<Guide, String> {
    let setup = doc.setups.get(guide.setup).ok_or("Choose a setup.")?;
    if !lathe::is_lathe(&setup.machine) {
        return build(doc, mesh, guide.setup);
    }
    let walked = Walked {
        done: (guide.done.iter())
            .map(|d| (d.op, d.plan.clone()))
            .collect(),
        afters: guide.done.iter().map(|d| d.after.clone()).collect(),
        before: guide.before.clone(),
    };
    lathe_guide(doc, mesh, guide.setup, walked)
}
/// The step the guide was left at (`Document::guide`), within its steps.
pub fn place(doc: &Document, guide: &Guide) -> usize {
    (doc.setups.get(guide.setup))
        .and_then(|s| doc.guide.get(&s.id))
        .map_or(0, |&at| at.min(guide.steps.len().saturating_sub(1)))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::model::{BOX_FIELDS, Operation, Tool};

    /// A Box named `name` cutting `b` (x_min, x_max, y_min, y_max, z_top,
    /// z_bottom, work frame) with the tool `tool`.
    pub(crate) fn boxed(name: &str, tool: &str, b: [f32; 6], fields: &[(&str, &str)]) -> Operation {
        let mut op = Operation::new(Strategy::Box, tool.into());
        op.p.insert("name".into(), name.into());
        for (k, v) in BOX_FIELDS.iter().zip(b) {
            op.p.insert((*k).into(), v.to_string());
        }
        for (k, v) in fields {
            op.p.insert((*k).into(), (*v).into());
        }
        op
    }
    /// `post::fixture::job` with the manual mill's setup in front, the CNC
    /// setup after it. The manual setup has 20 mm of stock in front of and
    /// behind the part, its zero on the stock top's front-left corner, and
    /// three operations: Face (2 mm off the whole top), Step (open on −Y, its
    /// wall at Y 15.5 with 0.5 left, down to −12 in levels of at most 5)
    /// with a Ø25 end mill, and Holes (a Ø4 drill in the top's four Ø6.6
    /// through holes).
    pub(crate) fn job(dro: &str) -> (Document, Mesh) {
        let (mut doc, mesh) = crate::post::fixture::job();
        let mut machine = manual::defaults();
        machine.p.insert("dro_comp".into(), dro.into());
        let mut mill = Tool::new("End mill 25", "Flat end mill", "25");
        let mut drill = Tool::new("Drill 4", "Drill", "4");
        for t in [&mut mill, &mut drill] {
            for (k, v) in [
                ("length", "60"),
                ("stickout", "65"),
                ("machines", &machine.id),
            ] {
                t.p.insert(k.into(), v.into());
            }
        }
        let mut holes = Operation::new(Strategy::Drill, drill.id.clone());
        holes.surfaces = doc.setups[0].operations[3].surfaces.clone();
        for (k, v) in [
            ("name", "Holes"),
            ("rpm", "1600"),
            ("feed", "100"),
            ("hole_diameter", "6.6"),
        ] {
            holes.p.insert(k.into(), v.into());
        }
        let ops = vec![
            boxed(
                "Face",
                &mill.id,
                [-500., 500., -500., 500., 0., -2.],
                &[("allowance", "0")],
            ),
            boxed(
                "Step",
                &mill.id,
                [-500., 500., -500., 15.5, 0., -12.],
                &[("stepdown", "5")],
            ),
            holes,
        ];
        doc.machines.push(machine);
        doc.tools.extend([mill, drill]);
        doc.add_manual_setup().unwrap();
        let setup = &mut doc.setups[0];
        for (k, v) in [
            ("origin", "Stock top front-left"),
            ("stock_front", "20"),
            ("stock_back", "20"),
        ] {
            setup.p.insert(k.into(), v.into());
        }
        setup.operations = ops;
        (doc, mesh)
    }
    fn kinds(guide: &Guide, range: Range<usize>) -> Vec<&'static str> {
        guide.steps[range]
            .iter()
            .map(|s| match s {
                Step::Hold { .. } => "Hold",
                Step::Zero { .. } => "Zero",
                Step::Tool { .. } => "Tool",
                Step::Comp { .. } => "Comp",
                Step::Pass { .. } => "Pass",
                Step::Face { .. } => "Face",
                Step::Square { .. } => "Square",
                Step::Holes { .. } => "Holes",
                Step::Check { .. } => "Check",
                Step::HandOver { .. } => "HandOver",
                Step::Lathe(l) => l.title(),
            })
            .collect()
    }
    fn close(a: f32, b: f32) {
        assert!((a - b).abs() < 1e-3, "{a} != {b}");
    }
    /// The Pass steps' targets with their pass's number.
    fn targets(guide: &Guide) -> Vec<(usize, &Target)> {
        (guide.steps.iter())
            .filter_map(|s| match s {
                Step::Pass { index, targets, .. } => Some((*index, targets)),
                _ => None,
            })
            .flat_map(|(i, t)| t.iter().map(move |t| (i, t)))
            .collect()
    }
    #[test]
    fn one_section_per_operation_between_set_up_and_hand_over() {
        let (doc, mesh) = job("Yes");
        let guide = build(&doc, &mesh, 0).unwrap();
        let names: Vec<&str> = guide.sections.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["Set up", "Face", "Step", "Holes", "Hand over"]);
        let ops: Vec<Option<usize>> = guide.sections.iter().map(|s| s.op).collect();
        assert_eq!(ops, [None, Some(0), Some(1), Some(2), None]);
        // The ranges cover every step once, in order.
        let mut at = 0;
        for s in &guide.sections {
            assert_eq!(s.steps.start, at);
            assert!(s.steps.end > at);
            at = s.steps.end;
        }
        assert_eq!(at, guide.steps.len());
        let of = |i: usize| kinds(&guide, guide.sections[i].steps.clone());
        assert_eq!(of(0), ["Hold", "Zero"]);
        assert_eq!(of(1), ["Face"]);
        assert_eq!(of(2), ["Tool", "Pass", "Pass", "Pass", "Pass", "Check"]);
        assert_eq!(of(3), ["Tool", "Holes", "Check"]);
        assert_eq!(of(4), ["HandOver"]);
        let counts: Vec<usize> = guide.sections.iter().map(|s| s.count).collect();
        assert_eq!(counts, [0, 0, 4, 4, 0]);
        // The minutes: the operations' plans and two tool changes of 3 minutes.
        let plans: f32 = guide.done.iter().map(|d| d.plan.duration / 60.).sum();
        close(guide.minutes, plans + 6.);
        let sum: f32 = guide.sections.iter().map(|s| s.minutes).sum();
        close(sum, guide.minutes);
        close(
            guide.sections[1].minutes,
            guide.done[0].plan.duration / 60. + 3.,
        );
        close(guide.sections[2].minutes, guide.done[1].plan.duration / 60.);
        assert_eq!(manual::tool_changes(&doc.setups[0]), 2);
        // What the pictures need: the stock before, and after each operation.
        assert_eq!(
            guide.done.iter().map(|d| d.op).collect::<Vec<_>>(),
            [0, 1, 2]
        );
        assert!(guide.before.cuts.is_empty());
        let removed: Vec<f32> = guide
            .done
            .iter()
            .map(|d| d.after.removed_volume())
            .collect();
        assert!(
            removed[0] > 0. && removed.windows(2).all(|w| w[1] > w[0]),
            "{removed:?}"
        );
    }
    #[test]
    fn steps_run_hold_zero_then_each_operation_and_a_face_is_one_step() {
        let (doc, mesh) = job("Yes");
        let guide = build(&doc, &mesh, 0).unwrap();
        assert_eq!(
            kinds(&guide, 0..guide.steps.len()),
            [
                "Hold", "Zero", "Face", "Tool", "Pass", "Pass", "Pass", "Pass", "Check", "Tool",
                "Holes", "Check", "HandOver"
            ]
        );
        let Step::Hold {
            stock,
            holding,
            vise,
        } = &guide.steps[0]
        else {
            panic!()
        };
        assert_eq!(*vise, None);
        // In work coordinates: the zero on the stock top's front-left corner.
        assert_eq!((stock.min[0], stock.min[1], stock.max[2]), (0., 0., 0.));
        close(stock.max[1], 120.);
        assert_eq!(holding.stock, *stock);
        // The Ø10 edge finder touches the left and front faces from outside:
        // its centre is 5 mm off them when the DRO is set.
        assert_eq!(
            guide.steps[1],
            Step::Zero {
                corner: [0.; 3],
                approach: [-1, -1],
                touch: [-5., -5., 0.],
                finder: 10.,
            }
        );
        let Step::Face {
            op,
            tool,
            rpm,
            diameter,
            top,
            remove,
            to,
        } = &guide.steps[2]
        else {
            panic!("{:?}", guide.steps[2])
        };
        assert_eq!(
            (*op, tool, *rpm, *diameter),
            (0, &doc.tools[3].id, 640., 25.)
        );
        close(*to, -2.);
        close(*remove, 2.);
        // The first tool picks up Z on the untouched top.
        close(*top, 0.);
        assert_eq!(guide.steps[12], Step::HandOver { next: Some(1) });
        // The Step's floor is closed: not a squaring Box.
        assert!(matches!(guide.steps[3], Step::Tool { op: 1, .. }));
    }
    /// The job in the manual mill's vise, its jaws on the Y sides.
    pub(crate) fn in_the_vise(doc: &mut Document) {
        doc.adopt_holders();
        let mill = doc.setups[0].machine.id.clone();
        for h in &mut doc.workholding {
            h.p.insert("machines".into(), mill.clone());
        }
        doc.setups[0].p.insert("holding".into(), "Vise".into());
        doc.setups[0].p.insert("vise_axis".into(), "Y".into());
        doc.settle_holding(0);
    }
    #[test]
    fn the_hold_step_has_the_vise_its_grip_and_jaw_height() {
        let (mut doc, mesh) = job("Yes");
        in_the_vise(&mut doc);
        let guide = build(&doc, &mesh, 0).unwrap();
        let Step::Hold {
            stock,
            holding,
            vise: Some(vise),
        } = &guide.steps[0]
        else {
            panic!("{:?}", guide.steps[0])
        };
        let setup = &doc.setups[0];
        let holder = doc.holder(get(&setup.p, "holder")).unwrap();
        assert_eq!(vise.across, 1);
        assert_eq!(Some(vise.grip), num(&setup.p, "vise_grip"));
        assert_eq!(Some(vise.jaw_height), num(&holder.p, "jaw_height"));
        assert_eq!(Some(vise.jaw_thickness), num(&holder.p, "jaw_thickness"));
        assert!(vise.grip > 0. && vise.grip < stock.max[2] - stock.min[2]);
        assert_eq!(holding.bottom, stock.min[2]);
    }
    #[test]
    fn every_new_tool_picks_up_z_on_the_top_left_before_its_operation() {
        let (doc, mesh) = job("Yes");
        let guide = build(&doc, &mesh, 0).unwrap();
        // After the 2 mm face, the Step's mill and the Holes' drill touch a
        // top 2 mm down.
        let tops: Vec<(usize, f32)> = (guide.steps.iter())
            .filter_map(|s| match s {
                Step::Tool { op, top, .. } | Step::Face { op, top, .. } => Some((*op, *top)),
                _ => None,
            })
            .collect();
        assert_eq!(tops.len(), 3, "{tops:?}");
        for ((op, top), want) in tops.into_iter().zip([(0, 0.), (1, -2.), (2, -2.)]) {
            assert_eq!(op, want.0);
            assert!((top - want.1).abs() < 0.01, "{op} {top}");
        }
        // A square after the face picks up on it too.
        let (mut doc, mesh) = job("Yes");
        let mill = doc.tools[3].id.clone();
        let b = [123., 500., -500., 500., 0., -30.];
        let square = boxed("Square", &mill, b, &[("allowance", "0")]);
        doc.setups[0].operations.insert(1, square);
        let guide = build(&doc, &mesh, 0).unwrap();
        let Step::Square { top, .. } = &guide.steps[3] else {
            panic!("{:?}", guide.steps[3])
        };
        assert!((top + 2.).abs() < 0.01, "{top}");
    }
    #[test]
    fn the_zero_takes_the_edge_finders_radius_off_each_face_it_touches() {
        let (mut doc, mesh) = job("Yes");
        // Only the holes, which do not depend on the zero.
        doc.setups[0].operations.drain(..2);
        doc.setups[0]
            .machine
            .p
            .insert("edge_finder".into(), "6".into());
        // 124 × 120 × 47 stock: the back-right corner, the centre, the bottom.
        for (origin, corner, approach, touch) in [
            ("Stock top back-right", [0., 0., 0.], [1, 1], [3., 3., 0.]),
            ("Stock top center", [0., 0., 0.], [-1, -1], [-65., -63., 0.]),
            (
                "Stock bottom front-left",
                [0., 0., 47.],
                [-1, -1],
                [-3., -3., 47.],
            ),
        ] {
            doc.setups[0].p.insert("origin".into(), origin.into());
            let guide = build(&doc, &mesh, 0).unwrap();
            let Step::Zero {
                corner: c,
                approach: a,
                touch: t,
                finder,
            } = &guide.steps[1]
            else {
                panic!()
            };
            assert_eq!((*a, *finder), (approach, 6.), "{origin}");
            for k in 0..3 {
                close(c[k], corner[k]);
                close(t[k], touch[k]);
            }
        }
        doc.setups[0]
            .machine
            .p
            .insert("edge_finder".into(), "0".into());
        let error = build(&doc, &mesh, 0).unwrap_err();
        assert!(error.contains("edge finder"), "{error}");
    }
    #[test]
    fn a_squaring_box_is_one_square_step_cut_to_the_wall() {
        let (width, height) = (124., 47.);
        // The +X side, 1 mm off as cut: down to a closed floor at −30 with
        // and without compensation, with 0.5 left on the wall and the floor,
        // and down through the stock's bottom.
        for (dro, readout, x_min, z_bottom, allowance, down_to) in [
            ("Yes", 0., 123., -30., "0", -30.),
            ("No", 12.5, 123., -30., "0", -30.),
            ("Yes", 0., 122.5, -30., "0.5", -29.5),
            ("Yes", 0., 123., -100., "0", -height),
        ] {
            let (mut doc, mesh) = job(dro);
            let mill = doc.tools[3].id.clone();
            let b = [x_min, 500., -500., 500., 0., z_bottom];
            let square = boxed("Square", &mill, b, &[("allowance", allowance)]);
            doc.setups[0].operations.insert(1, square);
            let guide = build(&doc, &mesh, 0).unwrap();
            let Step::Hold { stock, .. } = &guide.steps[0] else {
                panic!()
            };
            close(stock.max[0], width);
            close(stock.min[2], -height);
            assert_eq!(kinds(&guide, guide.sections[2].steps.clone()), ["Square"]);
            let Step::Square {
                op,
                comp,
                axis,
                side,
                to,
                shown,
                remove,
                down_to: down,
                diameter,
                ..
            } = &guide.steps[3]
            else {
                panic!("{:?}", guide.steps[3])
            };
            assert_eq!((*op, *axis, *side, *diameter), (1, 0, 1, 25.));
            // The DRO's compensation reads the edge on the wall's side: the
            // wall is on −X of the tool, cutting the stock's +X side.
            let want = if dro == "Yes" { [-1, 0] } else { [0, 0] };
            assert_eq!(*comp, want);
            close(*to, 123.);
            close(*shown, 123. + readout);
            close(*remove, 1.);
            close(*down, down_to);
            // The plan's cut nearest the wall has its centre a radius off it,
            // on the floor.
            let plan = &guide.done[1].plan;
            let near = (plan.moves.iter())
                .filter(|m| m.cutting_pass)
                .map(|m| m.to[0] - guide.zero[0])
                .fold(f32::INFINITY, f32::min);
            close(near, 123. + 12.5);
            close(plan.target - guide.zero[2], down_to);
            // The 15.5 mm front step, wider than the radius, is not squaring:
            // its section starts with its tool and runs its passes.
            assert_eq!(
                kinds(&guide, guide.sections[3].steps.clone()),
                ["Tool", "Pass", "Pass", "Pass", "Pass", "Check"]
            );
        }
    }
    #[test]
    fn targets_replay_the_plans_moves_in_order() {
        let (doc, mesh) = job("Yes");
        let guide = build(&doc, &mesh, 0).unwrap();
        let (zero, plan) = (guide.zero, &guide.done[1].plan);
        let work = |p: [f32; 3]| -> [f32; 3] { std::array::from_fn(|k| p[k] - zero[k]) };
        let start = work(plan.moves[0].from);
        let mut moves = plan.moves.iter().filter(|m| m.motion != Motion::Dwell);
        let (mut at, mut n) = (start, 0);
        for step in &guide.steps {
            let Step::Pass {
                targets, index, of, ..
            } = step
            else {
                continue;
            };
            n += 1;
            assert_eq!((*index, *of), (n, 4));
            // The plan's moves since the last cut, this cut last.
            let mut legs = vec![];
            for m in moves.by_ref() {
                let k = moved(m).unwrap();
                legs.push((k, work(m.to)[k]));
                if m.cutting_pass {
                    break;
                }
            }
            let got: Vec<(usize, f32)> = targets.iter().map(|t| (t.axis, t.centre)).collect();
            if n == 1 {
                // The first pass also sets, before the plunge, each axis the
                // plan leaves where it starts (here Y on the stock's front).
                let extra: Vec<(usize, f32)> =
                    got.iter().copied().filter(|g| !legs.contains(g)).collect();
                assert_eq!(extra, [(1, start[1])]);
                assert_eq!(got.len(), legs.len() + 1);
                let plunge = got.iter().rposition(|g| g.0 == 2).unwrap();
                assert_eq!(got[plunge - 1], extra[0]);
                for k in 0..3 {
                    assert!(got[..got.len() - 1].iter().any(|g| g.0 == k));
                }
            } else {
                assert_eq!(got, legs);
            }
            // Applied in order from where the last pass left the tool, they
            // reach the cut's end; their numbers run 1, 2, 3…
            for (i, t) in targets.iter().enumerate() {
                assert_eq!(usize::from(t.order), i + 1);
                at[t.axis] = t.centre;
            }
            let cut = plan
                .moves
                .iter()
                .filter(|m| m.cutting_pass)
                .nth(n - 1)
                .unwrap();
            assert_eq!(at, work(cut.to));
            // The cut is along X with Y and Z locked; Y and Z are locked
            // where they are set for the last time.
            let (cut, setting) = targets.split_last().unwrap();
            assert_eq!((cut.axis, cut.lock), (0, [false, true, true]));
            for (i, t) in setting.iter().enumerate() {
                let again = setting[i + 1..].iter().any(|u| u.axis == t.axis);
                let want = t.axis != 0 && !again;
                assert_eq!(
                    t.lock,
                    std::array::from_fn(|k| k == t.axis && want),
                    "{t:?}"
                );
            }
        }
        assert_eq!(n, 4);
        assert!(moves.all(|m| !m.cutting_pass));
        // Conventional with M3: the rows step toward the wall on +Y, so they
        // feed along +X with the uncut material on their left.
        let rows: Vec<&Move> = plan.moves.iter().filter(|m| m.cutting_pass).collect();
        assert!(rows.iter().all(|m| m.to[0] > m.from[0]));
        let sides = simulation::uncut_sides(&rows);
        assert!(
            !sides.is_empty() && sides.iter().all(|s| *s > 0.),
            "{sides:?}"
        );
    }
    #[test]
    fn x_and_y_read_the_tool_edge_on_the_comp_side_with_the_dros_compensation() {
        let (doc, mesh) = job("Yes");
        let guide = build(&doc, &mesh, 0).unwrap();
        // Open on both X ends (+), closed on +Y (+).
        assert!(matches!(guide.steps[3], Step::Tool { comp: [1, 1], .. }));
        assert!(matches!(guide.steps[9], Step::Tool { comp: [0, 0], .. }));
        let all = targets(&guide);
        for (_, t) in &all {
            match t.axis {
                2 => assert!(!t.edge && t.shown == t.centre),
                _ => {
                    assert!(t.edge);
                    close(t.shown, t.centre + 12.5);
                }
            }
        }
        // The last row's edge stops on the wall less the stock to leave.
        let (_, y) = all.iter().rev().find(|(_, t)| t.axis == 1).unwrap();
        close(y.shown, 15.);
        close(y.centre, 2.5);
        // Never past the wall's allowance, the floor's, nor below the floor.
        let Step::Pass { never, cut, .. } = &guide.steps[7] else {
            panic!()
        };
        close(never.y[1], 15.);
        close(never.z, -11.5);
        close(cut.max[1], 15.);
        close(cut.min[2], -11.5);
        // Without it, every readout is the centre.
        let (doc, mesh) = job("No");
        let guide = build(&doc, &mesh, 0).unwrap();
        assert!(matches!(guide.steps[3], Step::Tool { comp: [0, 0], .. }));
        for (_, t) in targets(&guide) {
            assert!(!t.edge && t.shown == t.centre, "{t:?}");
        }
    }
    #[test]
    fn the_holes_are_centres_and_depths_in_work_coordinates() {
        let (doc, mesh) = job("Yes");
        let guide = build(&doc, &mesh, 0).unwrap();
        let Step::Holes { op, holes } = &guide.steps[10] else {
            panic!()
        };
        assert_eq!((*op, holes.len()), (2, 4));
        let plan = &guide.done[2].plan;
        for h in holes {
            let at: Vec<&Move> = (plan.moves.iter())
                .filter(|m| {
                    m.cutting_pass
                        && m.to[0] - guide.zero[0] == h[0]
                        && m.to[1] - guide.zero[1] == h[1]
                })
                .collect();
            assert!(!at.is_empty());
            let deepest = at.iter().map(|m| m.to[2]).fold(f32::INFINITY, f32::min);
            assert_eq!(h[2], deepest - guide.zero[2]);
            assert!(
                h[2] < -2. && (0. ..124.).contains(&h[0]) && (20. ..100.).contains(&h[1]),
                "{h:?}"
            );
        }
    }
    /// The job's Step closed at both X ends (X 20..100): its passes start
    /// inside the stock.
    pub(crate) fn close_the_step(doc: &mut Document) {
        let mill = doc.tools[3].id.clone();
        doc.setups[0].operations[1] = boxed(
            "Step",
            &mill,
            [20., 100., -500., 15.5, 0., -12.],
            &[("stepdown", "5")],
        );
    }
    #[test]
    fn a_closed_step_plunges_once_per_level_and_feeds_across_to_its_rows() {
        let (mut doc, mesh) = job("Yes");
        close_the_step(&mut doc);
        let guide = build(&doc, &mesh, 0).unwrap();
        let passes: Vec<&Vec<Target>> = (guide.steps.iter())
            .filter_map(|s| match s {
                Step::Pass { op: 1, targets, .. } => Some(targets),
                _ => None,
            })
            .collect();
        // Two rows (Y 0 and 2.5) on two levels.
        assert_eq!(passes.len(), 4);
        for (i, targets) in passes.iter().enumerate() {
            let (cut, setting) = targets.split_last().unwrap();
            assert!(cut.feed && cut.axis == 0);
            let plunges = setting.iter().filter(|t| t.axis == 2 && t.feed).count();
            let across = setting.iter().filter(|t| t.axis == 1 && t.feed).count();
            // The first row of a level plunges; the second feeds across.
            assert_eq!(
                (plunges, across),
                [(1, 0), (0, 1)][i % 2],
                "{i} {targets:?}"
            );
            if i % 2 == 1 {
                // Off the wall 0.2 mm on −Y, back along X, across into the row.
                let axes: Vec<(usize, bool)> = setting.iter().map(|t| (t.axis, t.feed)).collect();
                assert_eq!(axes, [(1, false), (0, false), (1, true)]);
                close(setting[0].centre, -0.2);
                close(setting[2].centre, 2.5);
            }
        }
        // Feeds along +X with the wall on +Y on the left: conventional.
        let plan = &guide.done[1].plan;
        let rows: Vec<&Move> = plan.moves.iter().filter(|m| m.cutting_pass).collect();
        assert!(rows.iter().all(|m| m.to[0] > m.from[0]));
        assert!(simulation::uncut_sides(&rows).iter().all(|s| *s > 0.));
    }
    #[test]
    fn a_change_of_sides_alone_is_a_comp_step() {
        let (mut doc, mesh) = job("Yes");
        let mill = doc.tools[3].id.clone();
        // Behind the part: closed on −Y.
        let back = boxed(
            "Back step",
            &mill,
            [-500., 500., 104.5, 500., 0., -12.],
            &[("stepdown", "5")],
        );
        doc.setups[0].operations.insert(2, back);
        let guide = build(&doc, &mesh, 0).unwrap();
        let of = |i: usize| kinds(&guide, guide.sections[i].steps.clone());
        assert_eq!(guide.sections[3].name, "Back step");
        assert_eq!(of(3)[0], "Comp");
        assert_eq!(
            guide.steps[guide.sections[3].steps.start],
            Step::Comp {
                op: 2,
                comp: [1, -1]
            }
        );
        // The back step's last row reads its wall less the allowance.
        let pass = |s: &&Step| matches!(s, Step::Pass { op: 2, .. });
        let Some(Step::Pass { targets, .. }) = guide.steps.iter().rfind(pass) else {
            panic!()
        };
        let y = targets.iter().rev().find(|t| t.axis == 1).unwrap();
        close(y.shown, 105.);
        close(y.centre, 117.5);
        // A different speed with the same tool is a Tool step.
        doc.setups[0].operations[2]
            .p
            .insert("rpm".into(), "400".into());
        let guide = build(&doc, &mesh, 0).unwrap();
        assert_eq!(kinds(&guide, guide.sections[3].steps.clone())[0], "Tool");
    }
    #[test]
    fn a_pass_shows_the_handwheel_turns_for_its_feed() {
        let (doc, mesh) = job("Yes");
        let guide = build(&doc, &mesh, 0).unwrap();
        assert_eq!(manual::turns(&doc.setups[0].machine, 0, 120.), 24.);
        for step in &guide.steps {
            if let Step::Pass {
                turns_per_minute, ..
            } = step
            {
                assert_eq!(*turns_per_minute, 24.);
            }
        }
    }
    #[test]
    fn the_place_is_saved_with_the_project_and_kept_within_the_steps() {
        let (mut doc, mesh) = job("Yes");
        let guide = build(&doc, &mesh, 0).unwrap();
        assert_eq!(place(&doc, &guide), 0);
        let id = doc.setups[0].id.clone();
        doc.guide.insert(id.clone(), 7);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("job.axis.json");
        crate::storage::save(&path, &doc).unwrap();
        let mut doc = crate::storage::load(&path).unwrap();
        assert_eq!(doc.guide.get(&id), Some(&7));
        assert_eq!(place(&doc, &guide), 7);
        // Without the Step the setup has 7 steps: the place is the last.
        doc.setups[0].operations.remove(1);
        let guide = build(&doc, &mesh, 0).unwrap();
        assert_eq!(guide.steps.len(), 7);
        assert_eq!(place(&doc, &guide), 6);
        // A project saved without places loads with none.
        let mut json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        json.as_object_mut().unwrap().remove("guide");
        let old: Document = serde_json::from_value(json).unwrap();
        assert!(old.guide.is_empty());
    }
    #[test]
    fn faces_squares_and_comp_sides_follow_the_open_sides() {
        let all = [[true; 2]; 3];
        let (lo, hi, r) = ([0., 0., -10.], [124., 120., 0.], 12.5);
        assert_eq!(kind(all, lo, hi, r), Kind::Face);
        // A wall on −X with a 1 mm slab off it: squaring, floor open or closed.
        let mut wall = all;
        wall[0][0] = false;
        let slab = [123., 0., -10.];
        assert_eq!(kind(wall, slab, hi, r), Kind::Square { axis: 0, low: true });
        assert_eq!(comp_sides(wall), [-1, 1]);
        wall[2][0] = false;
        assert_eq!(kind(wall, slab, hi, r), Kind::Square { axis: 0, low: true });
        // As wide as the radius still is; wider is not.
        assert_eq!(
            kind(wall, [111.5, 0., -10.], hi, r),
            Kind::Square { axis: 0, low: true }
        );
        assert_eq!(kind(wall, [111.4, 0., -10.], hi, r), Kind::Other);
        // The front step: 15 mm off its +Y wall, wider than the radius.
        let mut step = all;
        (step[1][1], step[2][0]) = (false, false);
        let front = [124., 15., 0.];
        assert_eq!(
            (kind(step, lo, front, r), comp_sides(step)),
            (Kind::Other, [1, 1])
        );
        // Two closed sides are never squaring, however thin.
        let mut slot = all;
        slot[1] = [false; 2];
        let thin = ([0., 50., -10.], [124., 60., 0.]);
        assert_eq!(
            (kind(slot, thin.0, thin.1, r), comp_sides(slot)),
            (Kind::Other, [1, 1])
        );
    }
    #[test]
    fn only_a_manual_setup_with_an_enabled_operation_has_steps() {
        let (mut doc, mesh) = job("Yes");
        assert!(build(&doc, &mesh, 1).is_err());
        for op in &mut doc.setups[0].operations {
            op.enabled = false;
        }
        assert!(build(&doc, &mesh, 0).is_err());
    }

    // ---- The lathe ----

    use crate::turn::tests::{add, job as lathe_job, tube};

    /// The mockup's part: a Ø35 sleeve (bore Ø11) 61 long, its end 1 mm in
    /// from the bar's end, in a Ø50.2 × 100 bar on the 3-jaw; the cross
    /// dial reads `reads` ("" = not known yet). Face 1, Outside Ø40 to the
    /// shoulder at −65 (2 a pass, 0.2 finish), Drill Ø10 30 deep, Groove 3
    /// wide to Ø36 at −46, Thread M40 × 1.5 to −40, Part off at −62.
    pub(crate) fn sample(reads: &str) -> (Document, Mesh) {
        let (mut doc, mesh) = lathe_job(50.2, 100., tube(5.5, 17.5, 0., 61.), -62.);
        doc.setups[0]
            .machine
            .p
            .insert("cross_reads".into(), reads.into());
        add(&mut doc, "Face", &[("face_off", "1")]);
        add(
            &mut doc,
            "Outside",
            &[
                ("diameter", "40"),
                ("z_end", "-65"),
                ("depth", "2"),
                ("finish", "0.2"),
                ("allowance", "0"),
            ],
        );
        add(&mut doc, "Drill", &[("depth", "30")]);
        add(
            &mut doc,
            "Groove",
            &[("z", "-46"), ("width", "3"), ("diameter", "36")],
        );
        add(
            &mut doc,
            "Thread",
            &[
                ("thread_side", "Outside"),
                ("diameter", "40"),
                ("pitch", "1.5"),
                ("z_end", "-40"),
            ],
        );
        add(&mut doc, "Part off", &[("z", "-62")]);
        (doc, mesh)
    }
    fn lathe_steps(guide: &Guide) -> Vec<&LatheStep> {
        (guide.steps.iter())
            .filter_map(|s| match s {
                Step::Lathe(l) => Some(l),
                _ => None,
            })
            .collect()
    }
    /// Every dial number of the guide.
    fn dials(guide: &Guide) -> Vec<&Dial> {
        let mut out = vec![];
        for l in lathe_steps(guide) {
            match l {
                LatheStep::Face { top, .. } | LatheStep::TailDrill { quill: top, .. } => {
                    out.push(top)
                }
                LatheStep::Pass {
                    cross,
                    carriage,
                    feed_off,
                    ..
                } => out.extend(
                    [Some(cross), Some(carriage), feed_off.as_ref()]
                        .into_iter()
                        .flatten(),
                ),
                LatheStep::Chamfer { top, carriage, .. } => {
                    out.extend([Some(top), carriage.as_ref()].into_iter().flatten())
                }
                LatheStep::Plunge {
                    cross, carriages, ..
                } => out.extend(cross.iter().chain(carriages)),
                LatheStep::ThreadPass { top, carriage, .. } => out.extend([top, carriage]),
                _ => {}
            }
        }
        out
    }
    fn reads(dial: &Dial, turns: u32, reading: f32) {
        let (t, r) = dial
            .reading
            .clone()
            .unwrap_or_else(|e| panic!("{dial:?}: {e}"));
        assert!(
            t == turns && (r - reading).abs() < 1e-4,
            "{dial:?} is not {turns} turns + {reading}"
        );
    }
    fn passes(guide: &Guide, op: usize) -> Vec<(f32, &Dial, &Dial, Option<&Dial>)> {
        (lathe_steps(guide).into_iter())
            .filter_map(|l| match l {
                LatheStep::Pass {
                    op: o,
                    diameter,
                    cross,
                    carriage,
                    feed_off,
                    ..
                } if *o == op => Some((*diameter, cross, carriage, feed_off.as_ref())),
                _ => None,
            })
            .collect()
    }
    fn set_measured(doc: &mut Document, step: &str, value: f32) {
        let key = measure_key(&doc.setups[0].id.clone(), step);
        doc.measured.insert(key, value);
    }

    #[test]
    fn the_sample_parts_steps_run_set_up_each_operation_and_hand_over() {
        let (doc, mesh) = sample("Diameter");
        let guide = build(&doc, &mesh, 0).unwrap();
        let names: Vec<&str> = guide.sections.iter().map(|s| s.name.as_str()).collect();
        let ops: Vec<String> = (doc.setups[0].operations.iter())
            .map(|o| o.name().to_string())
            .collect();
        assert_eq!(names[0], "Set up");
        assert_eq!(
            names[1..7],
            ops.iter().map(|s| s.as_str()).collect::<Vec<_>>()[..]
        );
        assert_eq!(names[7], "Hand over");
        let of = |i: usize| kinds(&guide, guide.sections[i].steps.clone());
        // The dial is known: no dial check.
        assert_eq!(of(0), ["Hold", "Zero"]);
        assert_eq!(of(1), ["Face"]);
        // The turning tool faced and is zeroed on the new end: only the
        // cross dial is touched again, on the bar.
        assert_eq!(
            of(2),
            [
                "Touch", "Pass", "Pass", "Pass", "Pass", "Pass", "Measure", "Pass"
            ]
        );
        assert_eq!(of(3), ["Drill"]);
        assert_eq!(of(4), ["Touch", "Groove"]);
        assert_eq!(of(5)[..2], ["Touch", "Thread pass"]);
        assert_eq!(of(5).len(), 9);
        assert_eq!(of(6), ["Touch", "Part off"]);
        assert_eq!(of(7), ["HandOver"]);
        let counts: Vec<usize> = guide.sections.iter().map(|s| s.count).collect();
        assert_eq!(counts, [0, 1, 6, 1, 1, 8, 1, 0]);
        // The ranges cover every step once, in order.
        let mut at = 0;
        for s in &guide.sections {
            assert_eq!(s.steps.start, at);
            at = s.steps.end;
        }
        assert_eq!(at, guide.steps.len());
        let plans: f32 = guide.done.iter().map(|d| d.plan.duration / 60.).sum();
        assert!(guide.minutes > plans && guide.minutes.is_finite());

        // Hold: the 3-jaw, the setup's grip and stickout, the cuts 65 deep.
        let setup = &doc.setups[0];
        let Step::Lathe(LatheStep::Hold {
            holder,
            jaws,
            grip,
            stickout,
            diameter,
            length,
            reach,
        }) = &guide.steps[0]
        else {
            panic!("{:?}", guide.steps[0])
        };
        assert_eq!(holder, get(&setup.p, "holder"));
        assert_eq!(*jaws, 3);
        assert_eq!(Some(*grip), num(&setup.p, "grip"));
        assert_eq!(Some(*stickout), num(&setup.p, "stickout"));
        close(*diameter, 50.2);
        close(*length, 100.);
        close(*reach, 65.);
        // Zero: the first turning tool, the bar to measure.
        let turning = &setup.operations[0].tool;
        assert_eq!(
            guide.steps[1],
            Step::Lathe(LatheStep::Zero {
                tool: turning.clone(),
                key: measure_key(&setup.id, "zero"),
                planned: 50.2,
                measured: None,
            })
        );
        // Face: 1.00 on the top slide, then the carriage zeroed on the new end.
        let Step::Lathe(LatheStep::Face { top, new_end, .. }) = &guide.steps[2] else {
            panic!()
        };
        reads(top, 0, 1.);
        close(*new_end, -1.);
        // The Outside touches the bar to zero the cross dial; the carriage
        // keeps the zero the Face made.
        let Step::Lathe(LatheStep::Touch {
            carriage, cross, ..
        }) = &guide.steps[3]
        else {
            panic!()
        };
        assert!(!carriage);
        assert_eq!(
            cross,
            &Some(Contact {
                planned: 50.2,
                measured: None,
                inside: false
            })
        );
        assert_eq!(guide.steps.last(), Some(&Step::HandOver { next: None }));
    }
    #[test]
    fn the_outside_passes_read_off_the_touched_diameter_and_stop_from_the_new_end() {
        let (doc, mesh) = sample("Diameter");
        let guide = build(&doc, &mesh, 0).unwrap();
        let passes = passes(&guide, 1);
        // The plan's passes, in order, 2.00 off the Ø each, the finish last.
        let plan = &guide.done[1].plan;
        let planned: Vec<f32> = (plan.moves.iter())
            .filter(|m| m.motion == Motion::Feed)
            .map(|m| mm(2. * m.to[0]))
            .collect();
        let shown: Vec<f32> = passes.iter().map(|p| p.0).collect();
        assert_eq!(shown, planned);
        assert_eq!(shown, [48.2, 46.2, 44.2, 42.2, 40.2, 40.]);
        // Off the Ø50.20 touched: 2, 4, 6 (1 turn + 2.00 on a 4-a-turn
        // dial), 8, 10, 10.2.
        for ((d, cross, ..), (turns, reading)) in
            passes
                .iter()
                .zip([(0, 2.), (1, 0.), (1, 2.), (2, 0.), (2, 2.), (2, 2.2)])
        {
            close(cross.distance, 50.2 - d);
            reads(cross, turns, reading);
        }
        // Faced 1 mm, the carriage was zeroed on the new end: the shoulder
        // at −65 is 64 from it, 3 turns + 4.0 on a 20-a-turn dial; the power
        // feed goes off 1 mm before, at 3 turns + 3.0.
        for (_, _, carriage, feed_off) in &passes {
            close(carriage.distance, 64.);
            reads(carriage, 3, 4.);
            let off = feed_off.unwrap();
            close(off.distance, 63.);
            reads(off, 3, 3.);
        }
        // Without the Face the carriage is zeroed on the bar's end: 65.
        let (mut doc, mesh) = sample("Diameter");
        doc.setups[0].operations.remove(0);
        let guide = build(&doc, &mesh, 0).unwrap();
        for (_, _, carriage, feed_off) in passes_of(&guide, 0) {
            reads(carriage, 3, 5.);
            reads(feed_off.unwrap(), 3, 4.);
        }
        // The groove and the part off are from the new end too.
        let (doc, mesh) = sample("Diameter");
        let guide = build(&doc, &mesh, 0).unwrap();
        let plunges: Vec<&LatheStep> = (lathe_steps(&guide).into_iter())
            .filter(|l| matches!(l, LatheStep::Plunge { .. }))
            .collect();
        let LatheStep::Plunge {
            cross: Some(cross),
            carriages,
            bottom,
            width,
            ..
        } = plunges[0]
        else {
            panic!()
        };
        // The grooving tool's left side at −46: 45 from the new end.
        assert_eq!((carriages.len(), *width), (1, 3.));
        close(carriages[0].distance, 45.);
        reads(&carriages[0], 2, 5.);
        // From the touch on the turned Ø40 to Ø36: 4 off, 1 turn.
        close(*bottom, 36.);
        close(cross.distance, 4.);
        reads(cross, 1, 0.);
        // The blade's left side at −65 (−62 less its 3): 64 from the new
        // end; to the centre, no cross number.
        let LatheStep::Plunge {
            part_off: true,
            cross: None,
            carriages,
            ..
        } = plunges[1]
        else {
            panic!("{:?}", plunges[1])
        };
        close(carriages[0].distance, 64.);
        reads(&carriages[0], 3, 4.);
    }
    fn passes_of(guide: &Guide, op: usize) -> Vec<(f32, &Dial, &Dial, Option<&Dial>)> {
        passes(guide, op)
    }
    #[test]
    fn a_new_tool_or_moved_slides_zero_the_carriage_again_on_the_end() {
        let (doc, mesh) = sample("Diameter");
        let guide = build(&doc, &mesh, 0).unwrap();
        type Seen = (usize, bool, f32, Option<f32>, Option<f32>);
        let touches: Vec<Seen> = (lathe_steps(&guide).into_iter())
            .filter_map(|l| match l {
                LatheStep::Touch {
                    op,
                    carriage,
                    end,
                    cross,
                    top,
                    ..
                } => Some((
                    *op,
                    *carriage,
                    *end,
                    cross.as_ref().map(|c| c.planned),
                    *top,
                )),
                _ => None,
            })
            .collect();
        assert_eq!(
            touches,
            [
                // The Outside: the turning tool is zeroed already.
                (1, false, -1., Some(50.2), None),
                // The grooving tool: a new tool touches the new end.
                (3, true, -1., Some(40.), None),
                // The threading tool, the top slide swung to 29.5° first.
                (4, true, -1., Some(40.), Some(29.5)),
                // The blade: a new tool; to the centre, no cross touch.
                (5, true, -1., None, None),
            ]
        );
        // The same tool after a thread, with the top slide swung, touches
        // the end again.
        let (mut doc, mesh) = sample("Diameter");
        let thread_tool = doc.setups[0].operations[4].tool.clone();
        doc.setups[0].operations.truncate(5);
        add(
            &mut doc,
            "Thread",
            &[
                ("thread_side", "Outside"),
                ("diameter", "40"),
                ("pitch", "1.5"),
                ("z_end", "-30"),
            ],
        );
        assert_eq!(doc.setups[0].operations[5].tool, thread_tool);
        let guide = build(&doc, &mesh, 0).unwrap();
        let second = guide.sections[6].steps.start;
        assert!(matches!(
            guide.steps[second],
            Step::Lathe(LatheStep::Touch {
                op: 5,
                carriage: true,
                ..
            })
        ));
    }
    #[test]
    fn every_dial_number_is_lathe_dial_of_its_distance() {
        for reads in ["Diameter", "Radius"] {
            let (doc, mesh) = sample(reads);
            let guide = build(&doc, &mesh, 0).unwrap();
            let machine = &doc.setups[0].machine;
            let all = dials(&guide);
            assert!(all.len() > 30, "{}", all.len());
            for d in all {
                assert_eq!(
                    d.reading,
                    lathe::dial(machine, d.slide, d.distance),
                    "{d:?}"
                );
                assert!(d.reading.is_ok(), "{d:?}");
            }
        }
    }
    #[test]
    fn a_radius_dial_reads_half_what_a_diameter_dial_reads_for_the_same_pass() {
        let (doc, mesh) = sample("Diameter");
        let diameter = build(&doc, &mesh, 0).unwrap();
        let (doc, mesh) = sample("Radius");
        let radius = build(&doc, &mesh, 0).unwrap();
        let (a, b) = (passes(&diameter, 1), passes(&radius, 1));
        assert_eq!(a.len(), b.len());
        for (p, q) in a.iter().zip(&b) {
            // The same 6.00 off the Ø: 1 turn + 2.00, or 0 turns + 3.00.
            assert_eq!(p.1.distance, q.1.distance);
        }
        reads(a[2].1, 1, 2.);
        reads(b[2].1, 0, 3.);
        assert_ne!(a[2].1.reading, b[2].1.reading);
        // The carriage does not depend on it.
        assert_eq!(a[2].2, b[2].2);
    }
    #[test]
    fn the_dial_check_comes_first_only_while_the_cross_dial_is_not_known() {
        let (doc, mesh) = sample("");
        let guide = build(&doc, &mesh, 0).unwrap();
        assert_eq!(
            kinds(&guide, guide.sections[0].steps.clone()),
            ["Dial check", "Hold", "Zero"]
        );
        // 10 divisions of 0.04: 0.40 on the dial.
        assert_eq!(
            guide.steps[0],
            Step::Lathe(LatheStep::DialCheck {
                divisions: 10,
                reading: 0.4,
                length: 30.,
            })
        );
        // Every cross-slide number waits for it; never a guess. The others
        // are numbers.
        let all = dials(&guide);
        let cross: Vec<&&Dial> = all.iter().filter(|d| d.slide == Slide::Cross).collect();
        assert_eq!(cross.len(), 7);
        for d in &cross {
            assert!(d.unknown(), "{d:?}");
            assert_eq!(d.reading, Err(lathe::CHECK_THE_DIAL.into()));
            assert_eq!(d.text(), lathe::CHECK_THE_DIAL);
        }
        for d in all.iter().filter(|d| d.slide != Slide::Cross) {
            assert!(d.reading.is_ok(), "{d:?}");
        }
        // A measurement does not make a number of it either.
        let (mut doc, mesh) = sample("");
        let id = doc.setups[0].operations[1].id.clone();
        set_measured(&mut doc, &id, 40.3);
        let guide = build(&doc, &mesh, 0).unwrap();
        assert!(passes(&guide, 1).iter().all(|p| p.1.unknown()));
        // Answered: no check, every cross number shown.
        for answer in ["Diameter", "Radius"] {
            let (doc, mesh) = sample(answer);
            let guide = build(&doc, &mesh, 0).unwrap();
            assert!(!kinds(&guide, 0..guide.steps.len()).contains(&"Dial check"));
            assert!(dials(&guide).iter().all(|d| d.reading.is_ok()));
        }
    }
    #[test]
    fn the_measured_diameter_puts_what_is_left_on_the_last_pass() {
        let (mut doc, mesh) = sample("Diameter");
        let id = doc.setups[0].operations[1].id.clone();
        let key = measure_key(&doc.setups[0].id, &id);
        let guide = build(&doc, &mesh, 0).unwrap();
        let measure = |guide: &Guide| {
            (lathe_steps(guide).into_iter())
                .find_map(|l| match l {
                    LatheStep::Measure {
                        key,
                        expect,
                        target,
                        measured,
                        bore,
                        ..
                    } => Some((key.clone(), *expect, *target, *measured, *bore)),
                    _ => None,
                })
                .unwrap()
        };
        // Before the last pass: Ø40.20 expected, Ø40.00 to come.
        assert_eq!(measure(&guide), (key.clone(), 40.2, 40., None, false));
        reads(passes(&guide, 1)[5].1, 2, 2.2);
        // It sprang: Ø40.32 after the pass at 10.00. All of the 0.32 left
        // goes on the last pass: 10.32, 2 turns + 2.32.
        doc.measured.insert(key.clone(), 40.32);
        let guide = build(&doc, &mesh, 0).unwrap();
        assert_eq!(measure(&guide).3, Some(40.32));
        let last = passes(&guide, 1)[5].1;
        close(last.distance, 10.32);
        reads(last, 2, 2.32);
        // The passes before it do not change.
        reads(passes(&guide, 1)[4].1, 2, 2.);
        // With a radius dial: 5.16 on the dial, 1 turn + 1.16.
        doc.setups[0]
            .machine
            .p
            .insert("cross_reads".into(), "Radius".into());
        let guide = build(&doc, &mesh, 0).unwrap();
        reads(passes(&guide, 1)[5].1, 1, 1.16);
        doc.setups[0]
            .machine
            .p
            .insert("cross_reads".into(), "Diameter".into());
        // Under size already, or bigger than the bar: no number.
        for (was, says) in [(39.9, "past"), (50.5, "measure again")] {
            doc.measured.insert(key.clone(), was);
            let guide = build(&doc, &mesh, 0).unwrap();
            let last = passes(&guide, 1)[5].1;
            assert!(
                last.reading.as_ref().unwrap_err().contains(says),
                "{last:?}"
            );
        }
        // The bar measured at the Zero step is what the Outside touches:
        // Ø50.28, so 2.08 to Ø48.20.
        doc.measured.remove(&key);
        set_measured(&mut doc, "zero", 50.28);
        let guide = build(&doc, &mesh, 0).unwrap();
        assert!(matches!(
            guide.steps[1],
            Step::Lathe(LatheStep::Zero {
                measured: Some(m),
                ..
            }) if m == 50.28
        ));
        let first = passes(&guide, 1)[0].1;
        close(first.distance, 2.08);
        reads(first, 0, 2.08);
        // The groove touches the turned Ø40, not the bar.
        let plunge = (lathe_steps(&guide).into_iter())
            .find_map(|l| match l {
                LatheStep::Plunge { cross: Some(c), .. } => Some(c),
                _ => None,
            })
            .unwrap();
        close(plunge.distance, 4.);
    }
    #[test]
    fn a_bore_reads_out_from_the_hole_and_its_measure_the_other_way() {
        let (mut doc, mesh) = lathe_job(50.2, 100., tube(10.5, 17.5, 0., 61.), -62.);
        doc.setups[0]
            .machine
            .p
            .insert("cross_reads".into(), "Diameter".into());
        for t in &mut doc.tools {
            if get(&t.p, "type") == "Boring bar" {
                t.p.insert("min_bore".into(), "8".into());
            }
        }
        add(&mut doc, "Drill", &[("depth", "30")]);
        add(
            &mut doc,
            "Bore",
            &[
                ("diameter", "20"),
                ("z_end", "-20"),
                ("depth", "2"),
                ("finish", "0.2"),
                ("allowance", "0"),
            ],
        );
        let guide = build(&doc, &mesh, 0).unwrap();
        // The Zero step uses the first Turn's tool, the boring bar.
        let of = |g: &Guide, i: usize| kinds(g, g.sections[i].steps.clone());
        assert_eq!(of(&guide, 0), ["Hold", "Zero"]);
        assert_eq!(of(&guide, 1), ["Drill"]);
        assert_eq!(of(&guide, 2)[0], "Touch");
        // The boring bar zeroed the carriage at the Zero step: only the
        // hole is touched.
        let Step::Lathe(LatheStep::Touch {
            carriage: false,
            cross: Some(cross),
            ..
        }) = &guide.steps[guide.sections[2].steps.start]
        else {
            panic!("{:?}", guide.steps[guide.sections[2].steps.start])
        };
        assert_eq!(
            (cross.planned, cross.inside, cross.measured),
            (10., true, None)
        );
        let bore = passes(&guide, 1);
        let shown: Vec<f32> = bore.iter().map(|p| p.0).collect();
        assert_eq!(shown, [11.96, 13.92, 15.88, 17.84, 19.8, 20.]);
        for (d, cross, carriage, _) in &bore {
            close(cross.distance, d - 10.);
            reads(carriage, 1, 0.);
        }
        reads(bore[5].1, 2, 2.);
        // Measured Ø19.88 after 9.80: the 0.12 left on the last pass.
        let id = doc.setups[0].operations[1].id.clone();
        set_measured(&mut doc, &id, 19.88);
        let guide = build(&doc, &mesh, 0).unwrap();
        let last = passes(&guide, 1)[5].1;
        close(last.distance, 9.92);
        reads(last, 2, 1.92);
        assert!(
            lathe_steps(&guide)
                .iter()
                .any(|l| matches!(l, LatheStep::Pass { bore: true, .. }))
        );
    }
    #[test]
    fn the_tailstock_drill_reads_its_depth_and_point_on_the_quill() {
        let (doc, mesh) = sample("Diameter");
        let guide = build(&doc, &mesh, 0).unwrap();
        let Some(LatheStep::TailDrill {
            diameter,
            depth,
            point,
            peck,
            quill,
            ..
        }) = lathe_steps(&guide)
            .into_iter()
            .find(|l| matches!(l, LatheStep::TailDrill { .. }))
        else {
            panic!()
        };
        // 30 deep plus the Ø10 drill's 3.00 point, from where it touches
        // the faced end: 33.00 on a 2.5-a-turn quill, 13 turns + 0.50.
        assert_eq!((*diameter, *peck), (10., 10.));
        close(*depth, 30.);
        close(*point, 3.004);
        close(quill.distance, 33.004);
        reads(quill, 13, 0.5);
        let plan = &guide.done[2].plan;
        let feeds: Vec<&Move> = (plan.moves.iter())
            .filter(|m| m.motion == Motion::Feed)
            .collect();
        // The plan starts its feed 1 mm in front of the end it touches.
        close(
            quill.distance,
            mm(feeds[0].from[2] - crate::turn::CLEAR - plan.target),
        );
    }
    #[test]
    fn a_thread_feeds_the_top_slide_at_29_5_degrees_by_the_infeed_over_its_cosine() {
        // Moving the top slide s at 29.5° from the cross slide moves the
        // tool s·cos 29.5° toward the axis: the 0.915 thread depth of M40 ×
        // 1.5 is 1.0513 along the top slide.
        close(top_travel(0.915, 29.5), 1.0513);
        close(top_angle(60.), 29.5);
        close(top_angle(55.), 27.5);
        let (doc, mesh) = sample("Diameter");
        let guide = build(&doc, &mesh, 0).unwrap();
        let threads: Vec<(f32, f32, &Dial, &Dial)> = (lathe_steps(&guide).into_iter())
            .filter_map(|l| match l {
                LatheStep::ThreadPass {
                    infeed,
                    angle,
                    top,
                    carriage,
                    pitch,
                    ..
                } => {
                    assert_eq!(*pitch, 1.5);
                    Some((*infeed, *angle, top, carriage))
                }
                _ => None,
            })
            .collect();
        assert_eq!(threads.len(), 8);
        let plan = &guide.done[4].plan;
        let feeds: Vec<&Move> = (plan.moves.iter())
            .filter(|m| m.motion == Motion::Feed)
            .collect();
        for ((infeed, angle, top, carriage), m) in threads.iter().zip(&feeds) {
            // The infeed is the plan's, in from the touch on the Ø40 blank.
            close(*infeed, mm(20. - m.to[0]));
            assert_eq!(*angle, 29.5);
            close(top.distance, mm(top_travel(*infeed, 29.5)));
            // The half-nuts stay closed to the thread's end at −40: 39 from
            // the faced end.
            close(carriage.distance, 39.);
            reads(carriage, 1, 19.);
        }
        // Pass 3: 0.560 in, 0.643 along the slide, 0.64 on its dial of
        // 0.02s; the last 0.915 in, 1.051, 1.06.
        close(threads[2].0, 0.56);
        reads(threads[2].2, 0, 0.64);
        close(threads[7].0, 0.915);
        reads(threads[7].2, 0, 1.06);
    }
    #[test]
    fn a_chamfer_feeds_the_top_slide_and_a_shoulder_one_reads_the_carriage() {
        let (mut doc, mesh) = sample("Diameter");
        doc.setups[0].operations.truncate(2);
        add(&mut doc, "Chamfer", &[("edge", "Shoulder"), ("size", "1")]);
        add(
            &mut doc,
            "Chamfer",
            &[("edge", "End outside"), ("size", "0.5")],
        );
        let guide = build(&doc, &mesh, 0).unwrap();
        let of = |i: usize| kinds(&guide, guide.sections[i].steps.clone());
        // The turning tool, zeroed on the faced end: no touch first.
        assert_eq!(of(3), ["Chamfer"]);
        let Step::Lathe(LatheStep::Chamfer {
            top,
            carriage: Some(carriage),
            ..
        }) = &guide.steps[guide.sections[3].steps.start]
        else {
            panic!()
        };
        reads(top, 0, 1.);
        // The shoulder at −65: 64 from the faced end.
        close(carriage.distance, 64.);
        // The top slide was fed in: the next chamfer's carriage zero no
        // longer holds, but an end chamfer needs none.
        assert_eq!(of(4), ["Chamfer"]);
        let Step::Lathe(LatheStep::Chamfer {
            top,
            carriage: None,
            ..
        }) = &guide.steps[guide.sections[4].steps.start]
        else {
            panic!()
        };
        reads(top, 0, 0.5);
    }
    #[test]
    fn the_measurements_and_the_place_survive_save_and_load() {
        let (mut doc, mesh) = sample("");
        let id = doc.setups[0].id.clone();
        let op = doc.setups[0].operations[1].id.clone();
        set_measured(&mut doc, "zero", 50.28);
        set_measured(&mut doc, &op, 40.32);
        doc.guide.insert(id.clone(), 5);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shaft.axis.json");
        crate::storage::save(&path, &doc).unwrap();
        let loaded = crate::storage::load(&path).unwrap();
        assert_eq!(loaded.measured, doc.measured);
        assert_eq!(loaded.measured.get(&measure_key(&id, "zero")), Some(&50.28));
        let guide = build(&loaded, &mesh, 0).unwrap();
        assert_eq!(place(&loaded, &guide), 5);
        assert_eq!(build(&doc, &mesh, 0).unwrap().steps, guide.steps);
        // A project saved without measurements loads with none.
        let mut json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        json.as_object_mut().unwrap().remove("measured");
        let old: Document = serde_json::from_value(json).unwrap();
        assert!(old.measured.is_empty());
    }
    #[test]
    fn a_lathe_setup_needs_an_operation_and_a_chuck() {
        let (mut doc, mesh) = sample("Diameter");
        doc.setups[0].p.insert("holding".into(), "None".into());
        let error = build(&doc, &mesh, 0).unwrap_err();
        assert!(error.contains("chuck"), "{error}");
        let (mut doc, mesh) = sample("Diameter");
        doc.setups[0].operations.clear();
        let error = build(&doc, &mesh, 0).unwrap_err();
        assert!(error.contains("Turn"), "{error}");
    }
}
