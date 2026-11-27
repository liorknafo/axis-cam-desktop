//! The step guide of a setup on a manual mill or a manual lathe. On the mill
//! it is a short sheet (LIO-688): where the work zero is on the stock, then
//! one page per operation listing what it cuts, each hole, box, slot or
//! corner with its place, size and depth; the machinist works out the
//! moves. On the lathe it goes screen by screen, with the numbers its dials
//! (it has no DRO) should read. It is made from the setup's simulated
//! plans, so every number is one the app checked. Every coordinate a step
//! shows is a work coordinate (the stock frame less the work zero);
//! `Guide::zero` converts the kept plans.
use crate::{
    cam::{box_block, box_cut, open_sides},
    holding::{Gripped, Layout},
    lathe::{self, Slide},
    manual,
    mesh::{Bounds, Mesh},
    model::{Document, Machine, Operation, Strategy, Tool, get, num},
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
/// A vise as the Zero page draws it: its jaws close across axis `across`
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
    /// Where the work zero is (LIO-688): the stock box the setup starts
    /// from, in work coordinates, so that X0 Y0 Z0 is where it sits on it;
    /// what the vise or clamps hold, the vise when it is one, and the
    /// clamps over the stock (labelled, work coordinates).
    Zero {
        stock: Bounds,
        holding: Gripped,
        vise: Option<Vise>,
        clamps: Vec<(String, Bounds)>,
    },
    /// One operation on one page (LIO-688): its tool and spindle speed
    /// (none for a hand tap) and the features it cuts, numbered as the
    /// plan picture numbers them.
    Sheet {
        op: usize,
        tool: String,
        rpm: Option<f32>,
        features: Features,
    },
    /// The stock leaves for setup `next` (none for the last setup).
    HandOver { next: Option<usize> },
    /// A step at the manual lathe.
    Lathe(LatheStep),
}
/// How deep a hole goes (LIO-688).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Depth {
    /// Through the stock; the tool's tip at work Z `tip`.
    Through { tip: f32 },
    /// `deep` mm below its mouth: a twist drill's to its full diameter, a
    /// tap's to its full thread, a bore's to its floor, a centre drill's
    /// or a cut too shallow for its point to its tip; the tool's tip at
    /// work Z `tip`.
    Blind { deep: f32, tip: f32 },
}
/// A hole of a Drill, a Tap or a Boring: its centre (work X, Y), the work
/// Z of its mouth, how deep it goes, and whether a twist drill starts it on
/// a flat face with nothing before it to centre it (`uncentred`, a plan
/// check).
#[derive(Debug, Clone, PartialEq)]
pub struct Hole {
    pub at: [f32; 2],
    pub mouth: f32,
    pub depth: Depth,
    pub uncentred: bool,
}
/// A Round corners' corner: where it is on the part, the centre of its
/// round (work X, Y), its radius, and the work Z it goes from and to.
#[derive(Debug, Clone, PartialEq)]
pub struct Rounded {
    pub place: &'static str,
    pub centre: [f32; 2],
    pub radius: f32,
    pub top: f32,
    pub bottom: f32,
}
/// What an operation's page lists (LIO-688). Work coordinates throughout.
#[derive(Debug, Clone, PartialEq)]
pub enum Features {
    /// Holes of one size: Ø `diameter` (a Boring's finished size), or a
    /// tap's `thread`, "M8×1.25".
    Holes {
        diameter: f32,
        thread: Option<String>,
        holes: Vec<Hole>,
    },
    /// A Box as it cuts: `lo`..`hi` on X and Y within the stock, from the
    /// top it meets (`hi[2]`) down to its floor (`lo[2]`), or `through`.
    Box {
        lo: [f32; 3],
        hi: [f32; 3],
        through: bool,
    },
    /// A Slot: the centres of its round ends `a` and `b`, its width as cut,
    /// the top it starts in and its floor, or `through`.
    Slot {
        a: [f32; 2],
        b: [f32; 2],
        width: f32,
        top: f32,
        floor: f32,
        through: bool,
    },
    /// Round corners' corners, in the order cut.
    Corners(Vec<Rounded>),
}
impl Features {
    /// How many features the page numbers.
    pub fn count(&self) -> usize {
        match self {
            Features::Holes { holes, .. } => holes.len(),
            Features::Box { .. } | Features::Slot { .. } => 1,
            Features::Corners(c) => c.len(),
        }
    }
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
    /// Whole turns and how far past them the handle is wound (in the dial's
    /// numbers), or why there is no number: `lathe::CHECK_THE_DIAL` while
    /// the cross-slide dial is not known, never a guessed reading.
    pub reading: Result<(u32, f32), String>,
    /// Wound out, away from the work (a bore's cross slide, an inside
    /// thread's top slide): the collar counts down from its zero.
    pub out: bool,
    /// What the collar reads at its index once set (`lathe::reads`): the
    /// reading winding in, `turn − reading` winding out.
    pub reads: Option<f32>,
}
impl Dial {
    /// Wound in from its zero.
    pub fn new(machine: &Machine, slide: Slide, distance: f32) -> Self {
        Self::wound(machine, slide, distance, false)
    }
    /// Wound `out` (or in) from its zero.
    pub fn wound(machine: &Machine, slide: Slide, distance: f32, out: bool) -> Self {
        let reading = lathe::dial(machine, slide, distance);
        let reads =
            (reading.as_ref().ok()).and_then(|(_, r)| lathe::reads(machine, slide, *r, out));
        Self {
            slide,
            distance,
            reading,
            out,
            reads,
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
    /// "1 turn + 2.00", "2 turns out, reads 2.20", or why there is no
    /// number.
    pub fn text(&self) -> String {
        match (&self.reading, self.out) {
            (Ok((t, _)), true) => {
                let turns = if *t == 1 { "turn" } else { "turns" };
                format!("{t} {turns} out, reads {:.2}", self.reads.unwrap_or(0.))
            }
            (Ok((0, r)), false) => format!("{r:.2}"),
            (Ok((1, r)), false) => format!("1 turn + {r:.2}"),
            (Ok((t, r)), false) => format!("{t} turns + {r:.2}"),
            (Err(e), _) => e.clone(),
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
    /// With `tool` (the first turning tool, which cuts outside): touch the
    /// end → zero the carriage dial; when `measure`, measure the bar
    /// (`planned` Ø; `measured` under `key` once typed); when `cross`, touch
    /// it → zero the cross dial. A setup with only inside tools (a boring
    /// bar) neither touches nor measures the bar's outside: its sections
    /// touch the hole (LIO-203).
    Zero {
        tool: String,
        key: String,
        planned: f32,
        measured: Option<f32>,
        cross: bool,
        measure: bool,
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
    /// there. Every later carriage reading is from the new end. The top
    /// slide is set along the bed (90° from the cross slide); `swung` is the angle from the cross
    /// slide a thread before left it at, to set back.
    Face {
        op: usize,
        tool: String,
        rpm: f32,
        face_off: f32,
        to_diameter: f32,
        top: Dial,
        new_end: f32,
        swung: Option<f32>,
    },
    /// Pass `index` of `of` of an Outside (or Bore, `bore`) to `diameter`:
    /// the cross dial in (out, boring) to `cross`, power feed at `feed`
    /// mm/rev toward the chuck, off at `feed_off` (1 mm before), the last
    /// mm by hand to `carriage`; `to_jaws` from where it stops to the jaws,
    /// what the carriage stop guards under power feed.
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
        to_jaws: f32,
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
    /// Tapping on the centre from the tailstock, by hand with the spindle
    /// off: the `tap` (a thread size, `pitch` mm) `depth` of full thread
    /// deep plus its `lead`, from where it touches the hole's mouth, on the
    /// quill dial (`quill`, zeroed on the touch); `turns` of the tap in all.
    TailTap {
        op: usize,
        tool: String,
        tap: String,
        pitch: f32,
        depth: f32,
        lead: f32,
        /// To a tenth, rounded down: a turn more would go past the hole.
        turns: f32,
        quill: Dial,
    },
    /// A thread cut with a die, by hand with the spindle off: the `die` of
    /// thread `pitch` and outside diameter `od`, in its holder, pushed
    /// square on to the bar's `diameter` by the tailstock quill, `length`
    /// along it: `turns` of the holder.
    DieThread {
        op: usize,
        tool: String,
        die: String,
        pitch: f32,
        diameter: f32,
        od: f32,
        length: f32,
        /// To a tenth, rounded down: a turn more would go past the end.
        turns: f32,
    },
    /// A 45° chamfer of leg `size` on `edge` with a tool that has a 45°
    /// edge: the carriage at `carriage` (on a shoulder: the tool's point
    /// `size` short of it; the end edges are touched), locked; the 45° edge
    /// brought to the corner, then the top slide, set along the bed (90°
    /// from the cross slide), in `top` (`size`) toward the chuck, so both legs are `size`.
    Chamfer {
        op: usize,
        tool: String,
        rpm: f32,
        edge: String,
        size: f32,
        top: Dial,
        carriage: Option<Dial>,
        /// The angle from the cross slide a thread before left the top
        /// slide at, to set back along the bed (90° from the cross slide).
        swung: Option<f32>,
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
    /// to `carriage`, then back out and reverse. The last pass is set to the
    /// division below its travel, never deeper than planned (`infeed` is
    /// what it feeds); then try the nut, and a spring pass.
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
    /// Before a single-point thread's first pass, set once: the gearbox to
    /// `pitch`, the gear to `rpm`, the top slide to `angle`° from the cross
    /// slide, the tool on centre and square, the half-nuts closed and left
    /// closed to the end (LIO-415).
    ThreadSetUp {
        op: usize,
        rpm: f32,
        pitch: f32,
        angle: f32,
    },
    /// The end of a thread: try it with its mating part, `what` ("M16×2
    /// nut", "M12×1.75 screw"); `spring` for a single-point thread, which
    /// takes spring passes when it is tight (LIO-413).
    Try {
        op: usize,
        what: String,
        spring: bool,
    },
}
/// A thread's name: the standard size of `major` × `pitch` ("M16×2"), else
/// "M16.5×1.5".
/// The top slide's travel for each thread pass of `infeeds`, worked out once
/// for the Steps and the Passes tab (LIO-437): the last set to the slide's
/// division below its travel, never deeper than planned; the passes before
/// it never past that, so the readings never go back.
pub fn thread_travels(machine: &Machine, infeeds: &[f32], angle: f32) -> Vec<f32> {
    let deepest = infeeds.last().map_or(0., |d| {
        lathe::floor(machine, Slide::Top, mm(top_travel(*d, angle)))
    });
    let n = infeeds.len();
    (infeeds.iter().enumerate())
        .map(|(i, d)| {
            if i + 1 == n {
                deepest
            } else {
                mm(top_travel(*d, angle)).min(deepest)
            }
        })
        .collect()
}
pub fn thread_label(major: f32, pitch: f32) -> String {
    (crate::thread::SPECS.iter())
        .find(|s| (s.major - major).abs() < 0.05 && (s.pitch - pitch).abs() < 1e-3)
        .map_or_else(
            || format!("M{}×{}", trim(major), trim(pitch)),
            |s| s.name.to_string(),
        )
}
/// A number without trailing zeros.
fn trim(v: f32) -> String {
    let s = format!("{v:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
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
            LatheStep::TailTap { .. } => "Tap",
            LatheStep::DieThread { .. } => "Die",
            LatheStep::Chamfer { .. } => "Chamfer",
            LatheStep::Plunge { part_off: true, .. } => "Part off",
            LatheStep::Plunge { .. } => "Groove",
            LatheStep::ThreadPass { .. } => "Thread pass",
            LatheStep::ThreadSetUp { .. } => "Set up the thread",
            LatheStep::Try { .. } => "Try",
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
/// Whether a Box with its sides `open` that cuts `lo`..`hi`
/// (`cam::box_cut`) with a tool of radius `r` is cut in passes: not a
/// facing (all four X and Y sides open), nor a squaring (exactly one
/// closed, the wall, and the slab off it at most `r` wide, one row a level
/// whatever its floor).
fn in_passes(open: [[bool; 2]; 3], lo: [f32; 3], hi: [f32; 3], r: f32) -> bool {
    let closed: Vec<(usize, bool)> = (0..2)
        .flat_map(|k| [(k, true), (k, false)])
        .filter(|&(k, low)| !open[k][usize::from(!low)])
        .collect();
    match closed[..] {
        [] => false,
        [(axis, _)] => hi[axis] - lo[axis] > r + crate::cam::ON,
        _ => true,
    }
}
/// The one axis a manual move changes.
fn moved(m: &Move) -> Result<usize, String> {
    match (0..3).filter(|&k| m.from[k] != m.to[k]).collect::<Vec<_>>()[..] {
        [k] => Ok(k),
        _ => Err("A move on the manual mill changes more than one axis.".into()),
    }
}
/// `mouths` of the holes `moves` cut.
fn mouths_of(moves: &[Move], zero: [f32; 3]) -> Vec<f32> {
    let mut out: Vec<([f32; 2], f32)> = vec![];
    for m in moves.iter().filter(|m| m.cutting_pass) {
        let at = [m.to[0] - zero[0], m.to[1] - zero[1]];
        let top = m.from[2].max(m.to[2]) - zero[2];
        match out.last_mut() {
            Some((xy, z)) if *xy == at => *z = z.max(top),
            _ => out.push((at, top)),
        }
    }
    out.into_iter().map(|(_, z)| z).collect()
}
/// Whether the mouth of the hole Ø`d` at `at` (stock frame), drilled from
/// as high as `top` down to `bottom`, is opened in `before` so that a twist
/// drill finds its centre: its axis is open, or the material on it starts
/// lower than all round a ring about it within the hole's rim (a centre
/// drill's or a countersink's cone, a pilot hole). A flat face, a
/// flat-bottomed pocket the hole starts in, a slope, a step beside it, or
/// a rim with no material somewhere (the stock's edge) is not. Exact: the
/// cuts' sweeps, every 0.02 mm down.
pub(crate) fn opened(
    before: &Volume,
    frame: crate::volume::Frame,
    at: [f32; 2],
    d: f32,
    top: f32,
    bottom: f32,
) -> bool {
    const STEP: f32 = 0.02;
    let r = d / 2.;
    let near = Bounds {
        min: [at[0] - r, at[1] - r, bottom],
        max: [at[0] + r, at[1] + r, top],
    };
    let rest = crate::rest::RestStock::new(before, frame);
    let removed = rest.removed_in(near);
    let solid = |p: [f32; 3]| rest.in_stock(p) && !removed.at(p);
    // The highest Z with material at (x, y), from `top` down to `bottom`.
    let first = |x: f32, y: f32| {
        let n = ((top - bottom) / STEP).ceil().max(0.) as usize;
        (0..=n)
            .map(|i| top - i as f32 * STEP)
            .find(|&z| solid([x, y, z]))
    };
    let Some(axis) = first(at[0], at[1]) else {
        return true;
    };
    // A ring round the axis whose lowest material is still higher than
    // the axis: a cone round it keeps all of its rim higher; a step or a
    // slope keeps some no higher. Rings inside the rim count too: two
    // holes closer than their size (a slot's ends) each find the other's
    // pilot on their own rim, though each is centred in its own cone
    // (LIO-536).
    let ring = |rho: f32| {
        let mut low = f32::INFINITY;
        for k in 0..8 {
            let a = k as f32 * std::f32::consts::TAU / 8.;
            low = low.min(first(at[0] + rho * a.cos(), at[1] + rho * a.sin())?);
        }
        Some(low)
    };
    (1..=RINGS)
        .rev()
        .map(|k| r * k as f32 / RINGS as f32)
        .any(|rho| ring(rho).is_some_and(|rim| axis < rim - STEP))
}
/// How many rings round a hole's axis `opened` looks at, evenly out to its
/// rim.
const RINGS: usize = 4;
/// `holes` of the holes `moves` cut.
fn holes_of(moves: &[Move], zero: [f32; 3]) -> Vec<[f32; 3]> {
    let mut holes: Vec<[f32; 3]> = vec![];
    for m in moves.iter().filter(|m| m.cutting_pass) {
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
pub struct Walked {
    pub done: Vec<(usize, Plan)>,
    afters: Vec<Volume>,
    pub before: Volume,
}
/// The operations of setup `si` simulated in order up to its last enabled
/// one; `empty` when it has none.
pub fn simulate(doc: &Document, mesh: &Mesh, si: usize, empty: &str) -> Result<Walked, String> {
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
            // Named by its operation, for the Steps page to open (LIO-374).
            Err(e) if s == si && error.is_none() => {
                let op = &setup.operations[oi];
                // A threading's missing tool, named (LIO-412).
                let e = match crate::threading::needs(op) {
                    Some(n) => e.replace("Choose a tool from the library.", &format!("needs {n}.")),
                    None => e,
                };
                error = Some(format!("{}: {e}", op.name()))
            }
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
/// How far above the top of the holding's jaws an edge finder touches the
/// stock, mm: room for its tip to come in over them; no closer than
/// `FINDER_LEAST` when nothing stands that far above them.
const FINDER_CLEAR: f32 = 2.;
const FINDER_LEAST: f32 = 0.5;
/// Why a manual mill's setup has no Zero: nothing of its stock stands clear
/// of the jaws for the edge finder to touch.
pub const NOT_CLEAR: &str = "Nothing of the stock stands clear of the jaws for the edge finder to touch on X and Y: grip less of it, or hold it by another part, so that X and Y can be picked up above the jaws.";
/// How a manual mill's setup with no Zero starts its reason when its clamps
/// are over every spot of a face the edge finder must touch (LIO-458).
pub const UNDER_CLAMPS: &str = "The edge finder has no spot clear of the clamps";
/// How far an edge finder keeps from a clamp, mm, beyond its radius, and
/// how far out from a face it comes down before it is fed in to touch
/// (LIO-458).
const CLAMP_CLEAR: f32 = 2.;
const FINDER_OUT: f32 = 3.;
/// How far in from where a face's material ends a touch moved clear of the
/// clamps keeps, mm: a deburred corner's chamfer (LIO-458).
const EDGE_IN: f32 = 0.5;
/// Where `contact` has the edge finder touch a face: the spot, whether what
/// it touches is round there, and, when every spot of the face's material
/// is under a clamp, those clamps (indexes into the clamps it was given):
/// `at` is then where it would touch without them (LIO-458).
#[derive(Debug, Clone, PartialEq)]
pub struct Touch {
    pub at: [f32; 3],
    round: bool,
    pub under: Vec<usize>,
}
/// Where across a face (the other of X and Y) an edge finder of radius `r`
/// keeps its centre out of for each clamp of `clamps` (holding boxes, in
/// the setup's frame) when it touches the face on axis `k` at `face` (`out`
/// +1 when its outside is the + side, −1 the − side) at height `z`
/// (LIO-458): the clamp's index and the open interval, for each clamp that
/// reaches above `z` over where the finder comes down `FINDER_OUT` out from
/// where it touches and is fed in; `CLAMP_CLEAR` round it. The clamps are
/// the probe check's (`post::in_the_way`), so that the two agree.
pub fn keep_off(
    clamps: &[crate::holding::HoldBox],
    k: usize,
    out: f32,
    face: f32,
    z: f32,
    r: f32,
) -> Vec<(usize, f32, f32)> {
    let o = 1 - k;
    let far = face + out * (2. * r + FINDER_OUT);
    let (lo, hi) = (face.min(far) - CLAMP_CLEAR, face.max(far) + CLAMP_CLEAR);
    (clamps.iter().enumerate())
        .filter(|(_, c)| c.bounds.max[2] > z && c.bounds.min[k] < hi && c.bounds.max[k] > lo)
        .map(|(i, c)| {
            let b = &c.bounds;
            (i, b.min[o] - r - CLAMP_CLEAR, b.max[o] + r + CLAMP_CLEAR)
        })
        .collect()
}
/// Where an edge finder touches the stock (the box of what the earlier
/// setups left, `before`, in the setup's `frame`) coming in along axis `k`
/// (X or Y) from its + side when `plus`, else from its − side, and whether
/// what it touches is round there. The finder touches no lower than
/// `above` (the holding's jaws are below it), at the middle of the material
/// it meets, on the box's face when that face has material above `above`:
/// at the top when the face reaches the top, else `UNDER` mm under the
/// highest the face reaches (on that height when the face is no taller or
/// `UNDER` lower is too low), in whole millimetres under the top. A stepped
/// or turned stock, a shaft whose top is narrower than its foot, is touched
/// on its widest part, which is what sets the box's face. When the box's
/// face is only in the jaws, the finder touches the outermost material it
/// can reach, at the highest height it has any, on the line where it stands
/// out farthest, found to 0.01 mm. Round when the material is met over less
/// than half the stock's width: a turned diameter, met along a line. With
/// no earlier cuts, the top's middle on the box's face; None when nothing
/// of the stock stands above `above`.
///
/// A finder of radius `r` touches clear of `clamps` (`keep_off`,
/// LIO-458): a spot under one moves along the face to the nearest stretch
/// of its material clear of them all, to its middle (the one first along
/// the face when two are as near and as wide). Where they cover all of a
/// height's material the heights under it are tried; with none clear, the
/// first height's spot without them, and the clamps over it (`Touch`).
#[allow(clippy::too_many_arguments)]
pub fn contact(
    before: &Volume,
    frame: crate::volume::Frame,
    stock: Bounds,
    k: usize,
    plus: bool,
    above: f32,
    clamps: &[crate::holding::HoldBox],
    r: f32,
) -> Option<Touch> {
    // Inside the face, as far as a turned bar's chord is wide enough to
    // find; samples across it, and rows down it a whole millimetre apart
    // (more for a tall stock); steps in from the face where it is not.
    const IN: f32 = 0.2;
    const ACROSS: usize = 61;
    const ROWS: f32 = 200.;
    const UNDER: f32 = 2.;
    const STEP: f32 = 0.5;
    let o = 1 - k;
    let edge = if plus { stock.max[k] } else { stock.min[k] };
    let mut at = [0.; 3];
    at[k] = edge;
    at[o] = (stock.min[o] + stock.max[o]) / 2.;
    at[2] = stock.max[2];
    let height = stock.max[2] - stock.min[2];
    let inward = if plus { -1. } else { 1. };
    let (width, depth) = (stock.max[o] - stock.min[o], stock.max[k] - stock.min[k]);
    let across = |i: usize| stock.min[o] + (i as f32 + 0.5) / ACROSS as f32 * width;
    let gap = 1.5 * width / ACROSS as f32;
    // Where across the face the clamps keep the finder's centre out of,
    // for a touch `d` in from the box's face at height `z`.
    let offs = |d: f32, z: f32| keep_off(clamps, k, -inward, edge + inward * d, z, r);
    // The spot `at` (across the face) a touch `d` in at height `z` moves to
    // clear of the clamps: itself when clear, else the middle of the
    // nearest stretch clear of them of an unbroken run of `found` (samples
    // with material, which `solid` says of any point across), the run's
    // ends found to 0.01 mm and kept `EDGE_IN` in from, the clamps'
    // keep-outs taken exactly, not at the samples; of two as near (to
    // 0.01 mm), the wider, then the first along the face. Where a
    // stretch's middle has no material, its sample nearest it. Err with
    // the clamps over all of `found`.
    let clear = |at: f32, found: &[f32], solid: &dyn Fn(f32) -> bool, d: f32, z: f32| {
        let offs = offs(d, z);
        if !offs.iter().any(|&(_, a0, a1)| a0 < at && at < a1) {
            return Ok(at);
        }
        let mut runs: Vec<(f32, f32)> = vec![];
        for &a in found {
            match runs.last_mut() {
                Some(run) if a - run.1 <= gap => run.1 = a,
                _ => runs.push((a, a)),
            }
        }
        // Where the material ends between `inside` (material) and
        // `outside` (the next sample out, or the face's end).
        let end = |mut inside: f32, mut outside: f32| {
            if solid(outside) {
                return outside;
            }
            while (inside - outside).abs() > 0.005 {
                let mid = (inside + outside) / 2.;
                if solid(mid) {
                    inside = mid;
                } else {
                    outside = mid;
                }
            }
            inside
        };
        let pitch = width / ACROSS as f32;
        for run in &mut runs {
            run.0 = end(run.0, (run.0 - pitch).max(stock.min[o])) + EDGE_IN;
            run.1 = end(run.1, (run.1 + pitch).min(stock.max[o])) - EDGE_IN;
        }
        runs.retain(|(l, h)| l <= h);
        let mut stretches: Vec<(f32, f32)> = vec![];
        for &run in &runs {
            let mut parts = vec![run];
            for &(_, a0, a1) in &offs {
                parts = (parts.into_iter())
                    .flat_map(|(l, h)| [(l, h.min(a0)), (l.max(a1), h)])
                    .filter(|(l, h)| l <= h)
                    .collect();
            }
            stretches.extend(parts);
        }
        // In hundredths of a millimetre: how far from `at`, less wide.
        let rank = |&(l, h): &(f32, f32)| {
            let off = (l - at).max(at - h).max(0.);
            (
                (off * 100.).round() as i64,
                -((h - l) * 100.).round() as i64,
            )
        };
        stretches.sort_by_key(rank);
        for &(l, h) in &stretches {
            let mid = (l + h) / 2.;
            if solid(mid) {
                return Ok(mid);
            }
            let near = (found.iter().copied())
                .filter(|a| (l..=h).contains(a))
                .min_by(|a, b| (a - mid).abs().total_cmp(&(b - mid).abs()));
            if let Some(a) = near {
                return Ok(a);
            }
        }
        let over: Vec<usize> = (offs.iter())
            .filter(|&&(_, a0, a1)| runs.iter().any(|&(l, h)| a0 < h && a1 > l))
            .map(|&(i, _, _)| i)
            .collect();
        Err(if over.is_empty() {
            (offs.iter())
                .filter(|&&(_, a0, a1)| a0 < at && at < a1)
                .map(|&(i, _, _)| i)
                .collect()
        } else {
            over
        })
    };
    // An uncut or thin stock is touched at its face's middle, where a box
    // and a bar standing up alike stand out farthest; under a clamp, its
    // material is looked for as a cut one's is (a thin one's at half its
    // height).
    let thin = height <= 2. * IN;
    let middle_clear = !(offs(0., at[2]).iter()).any(|&(_, a0, a1)| a0 < at[o] && at[o] < a1);
    if (before.cuts.is_empty() || thin) && middle_clear {
        return Some(Touch {
            at,
            round: false,
            under: vec![],
        });
    }
    if height.is_nan() {
        let found: Vec<f32> = (0..ACROSS).map(across).collect();
        return Some(match clear(at[o], &found, &|_| true, 0., at[2]) {
            Ok(a) => {
                at[o] = a;
                Touch {
                    at,
                    round: false,
                    under: vec![],
                }
            }
            Err(over) => Touch {
                at,
                round: false,
                under: over,
            },
        });
    }
    let rest = crate::rest::RestStock::new(before, frame);
    // Only the cuts that reach the stock, by cell.
    let removed = rest.removed_in(stock);
    // Material `d` in from the box's face, at `a` across it, at height `z`.
    let has = |d: f32, a: f32, z: f32| {
        let mut p = [0.; 3];
        (p[k], p[o], p[2]) = (edge + inward * d, a, z);
        rest.in_stock(p) && !removed.at(p)
    };
    // The samples across the face with material `d` in at height `z`.
    let solid = |d: f32, z: f32| -> Vec<f32> {
        (0..ACROSS).map(across).filter(|&a| has(d, a, z)).collect()
    };
    // Where the finder touches what it meets (`found`, samples across the
    // face with material, which `solid` says of any point across): the
    // middle of the widest unbroken run of it (the one nearer the middle
    // when two are as wide), so never in a gap an earlier cut left in the
    // face; when a cut narrower than the samples' pitch lies at that
    // middle, the run's sample nearest it (LIO-451). Round when all it
    // meets spans less than half the stock's width.
    let met = |found: &[f32], solid: &dyn Fn(f32) -> bool| {
        let middle = (stock.min[o] + stock.max[o]) / 2.;
        let mut best = (found[0], found[0]);
        let mut run = best;
        for &a in &found[1..] {
            if a - run.1 > gap {
                run = (a, a);
            } else {
                run.1 = a;
            }
            let wider = run.1 - run.0 > best.1 - best.0 + gap / 3.;
            let as_wide = (run.1 - run.0 - (best.1 - best.0)).abs() <= gap / 3.;
            let nearer =
                ((run.0 + run.1) / 2. - middle).abs() < ((best.0 + best.1) / 2. - middle).abs();
            if wider || (as_wide && nearer) {
                best = run;
            }
        }
        let (first, last) = (found[0], found[found.len() - 1]);
        let mut at = (best.0 + best.1) / 2.;
        if !solid(at) {
            let run = found.iter().filter(|a| (best.0..=best.1).contains(*a));
            at = run
                .copied()
                .min_by(|a, b| (a - at).abs().total_cmp(&(b - at).abs()))
                .unwrap_or(at);
        }
        (at, last - first < width / 2.)
    };
    // How far in from the box's face the material is at `a` across it and
    // height `z`, to 0.01 mm: between `out` (none, or the face) and
    // `inside` (material), so that a face an earlier cut skimmed back by
    // less than the samples' depth is touched where it now is (LIO-451).
    let face = |mut out: f32, mut inside: f32, a: f32, z: f32| {
        while inside - out > 0.005 {
            let mid = (out + inside) / 2.;
            if has(mid, a, z) {
                inside = mid;
            } else {
                out = mid;
            }
        }
        (inside * 100.).round() / 100.
    };
    // The heights tried, top first, none in the jaws: the top, then a
    // whole number of millimetres under it.
    let pitch = (height / ROWS).ceil().max(1.);
    let low = (stock.min[2] + IN).max(above);
    let rows: Vec<f32> = if thin {
        vec![(stock.min[2] + stock.max[2]) / 2.]
    } else {
        std::iter::once(stock.max[2] - IN)
            .chain((1..).map(|n| stock.max[2] - n as f32 * pitch))
            .take_while(|z| *z >= low)
            .collect()
    };
    // The touch at `a` across, `d` in, at height `z` (shown at `shown`),
    // moved clear of the clamps (`clear`); what reads as a round is moved
    // only where its face is as far out as at `a`, to 0.01 mm (a flat
    // wall), else touched on its centre line or not at all. Err with the
    // spot without them and the clamps over it.
    let place = |a: f32, round: bool, found: &[f32], d: f32, z: f32, shown: f32| {
        let solid = |a: f32| has(d, a, z);
        let out = (d - STEP).max(0.);
        let moved = match clear(a, found, &solid, d, z) {
            Ok(b) if round && (face(out, d, b, z) - face(out, d, a, z)).abs() > 0.01 => {
                Err((offs(d, z).into_iter())
                    .filter(|&(_, a0, a1)| a0 < a && a < a1)
                    .map(|(i, _, _)| i)
                    .collect())
            }
            moved => moved,
        };
        let spot = |a: f32| {
            let mut p = [0.; 3];
            (p[o], p[2]) = (a, shown);
            p[k] = edge + inward * face((d - STEP).max(0.), d, a, z);
            p
        };
        match moved {
            Ok(b) => Ok(Touch {
                at: spot(b),
                round,
                under: vec![],
            }),
            Err(over) => Err(Touch {
                at: spot(a),
                round,
                under: over,
            }),
        }
    };
    // The first height's touch all under the clamps, when no height has
    // one clear of them.
    let mut covered: Option<Touch> = None;
    for (n, &z) in rows.iter().enumerate() {
        let found = solid(IN, z);
        if found.is_empty() {
            continue;
        }
        let (z, found) = if n == 0 {
            (z, found)
        } else {
            let lower = z - UNDER;
            let below = if lower >= low {
                solid(IN, lower)
            } else {
                vec![]
            };
            if below.is_empty() {
                (z, found)
            } else {
                (lower, below)
            }
        };
        let (a, round) = met(&found, &|a| has(IN, a, z));
        let shown = if n == 0 { at[2] } else { z };
        match place(a, round, &found, IN, z, shown) {
            Ok(touch) => return Some(touch),
            Err(touch) => {
                covered.get_or_insert(touch);
            }
        }
    }
    if covered.is_some() {
        return covered;
    }
    // The box's face is only in the jaws: the outermost material above
    // them, stepped in to, then found between the steps.
    for (n, &z) in rows.iter().enumerate() {
        let first = (0..ACROSS).filter_map(|i| {
            let a = across(i);
            let steps = (depth / STEP) as usize;
            (0..=steps)
                .map(|s| IN + s as f32 * STEP)
                .find(|&d| has(d, a, z))
                .map(|d| (d, a))
        });
        let first: Vec<(f32, f32)> = first.collect();
        let Some(nearest) = first.iter().map(|(d, _)| *d).reduce(f32::min) else {
            continue;
        };
        let lines: Vec<f32> = (first.iter())
            .filter(|(d, _)| *d <= nearest)
            .map(|(_, a)| *a)
            .collect();
        let (a, round) = met(&lines, &|a| has(nearest, a, z));
        let shown = if n > 0 { z } else { at[2] };
        match place(a, round, &lines, nearest, z, shown) {
            Ok(touch) => return Some(touch),
            Err(touch) => {
                covered.get_or_insert(touch);
            }
        }
    }
    covered
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
        "Add a Box, Slot or Drill operation to see the steps.",
    )?;
    let stock = done[0].1.stock;
    let zero = crate::post::datum(setup, stock, mesh)?;
    let work = |b: Bounds| b.translated(zero.map(|v| -v));
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
    let found = crate::post::references(get(&setup.p, "origin"))
        .ok_or("Choose a work zero for the setup.")?;
    let finder = num(&machine.p, "edge_finder")
        .filter(|d| *d > 0.)
        .ok_or("Enter the edge finder's diameter on the manual mill.")?;
    let approach = found.map(|a| if a == At::Max { 1 } else { -1 });
    // A zero at the middle is found by halving between both faces.
    let halve = found.map(|a| a == At::Mid);
    // The finder touches clear above what the holding's jaws reach: a
    // vise's grip above where the stock stands, soft jaws' or a collet's
    // above the bar's bottom (a V-block holds a bar from below).
    let jaws = match crate::holding::layout(setup) {
        Ok(crate::holding::Layout::Vise { grip, .. }) => Some(held.bottom + grip),
        Ok(crate::holding::Layout::Round(r)) if r.kind != crate::round_hold::Kind::VBlock => {
            Some(stock.min[2] + r.grip)
        }
        _ => None,
    };
    // The clamps over the stock, which the finder touches clear of: the
    // probe check's (LIO-458). A vise's jaws are below where it touches.
    // Clamps that cannot be laid out or placed give no Zero to follow.
    let clamps = match crate::holding::layout(setup) {
        Ok(Layout::Clamps { .. }) => doc.holding_boxes(setup, held)?,
        Err(why) if get(&setup.p, "holding") == "Clamps" => return Err(why),
        _ => vec![],
    };
    // Whether the edge finder reaches the face axis `k` is found on, its +
    // face when `plus`: clear above the jaws, else just above them; with
    // the clamps over all of that face, named, when they are.
    let touch = |k: usize, plus: bool| {
        let near = |clear: f32| {
            let above = jaws.map_or(f32::NEG_INFINITY, |top| top + clear);
            contact(&before, frame, stock, k, plus, above, &clamps, finder / 2.)
        };
        let t = near(FINDER_CLEAR)
            .or_else(|| near(FINDER_LEAST))
            .ok_or(NOT_CLEAR)?;
        let names: Vec<&str> = t.under.iter().map(|&i| clamps[i].label.as_str()).collect();
        Ok::<_, &str>((!names.is_empty()).then(|| crate::post::listed(&names)))
    };
    // A face under clamps: "the front face (the clamp on -Y (offset 0))".
    let face = |k: usize, plus: bool, names: &str| {
        let side = [["left", "right"], ["front", "back"]][k][usize::from(plus)];
        format!("the {side} face ({names})")
    };
    // The page says where the zero is, not how to find it (LIO-688), but a
    // zero the edge finder cannot reach is still refused: a face it must
    // touch above the jaws and clear of the clamps; a middle needs one of
    // its two faces (LIO-451, LIO-458).
    for k in 0..2 {
        let under = touch(k, approach[k] > 0)?;
        if !halve[k] {
            if let Some(names) = under {
                return Err(format!(
                    "{UNDER_CLAMPS} on {}. Move the clamp along its side, or choose a work zero on faces the clamps leave clear.",
                    face(k, approach[k] > 0, &names)
                ));
            }
            continue;
        }
        if let (Some(minus), Some(plus)) = (under, touch(k, true)?) {
            return Err(format!(
                "{UNDER_CLAMPS} on {} or {}. Move a clamp along its side so that the finder can touch one of them, or choose a work zero on faces the clamps leave clear.",
                face(k, false, &minus),
                face(k, true, &plus)
            ));
        }
    }
    let mut steps = vec![Step::Zero {
        stock: work(stock),
        holding: Gripped {
            stock: work(held.stock),
            band: work(held.band),
            bottom: held.bottom - zero[2],
            girth: held.girth,
        },
        vise,
        clamps: (clamps.iter())
            .map(|c| (c.label.clone(), work(c.bounds)))
            .collect(),
    }];
    let mut sections = vec![Section {
        name: "Set up".into(),
        op: None,
        steps: 0..1,
        count: 0,
        minutes: 0.,
    }];

    // One page per operation.
    let mut last_tool: Option<&str> = None;
    let mut done_ops = vec![];
    for ((oi, plan), after) in done.into_iter().zip(afters) {
        let op = &setup.operations[oi];
        // No operation cuts into the parallels the stock stands on, or past
        // their height into the table under them (LIO-537), as the plan
        // check and the export refuse it.
        let gap = crate::holding::gap(setup).unwrap_or_default();
        if let Some(why) = crate::parallels::refusal(
            setup,
            held.stock,
            &plan,
            gap,
            crate::post::blend(machine),
            op.name(),
            zero,
        ) {
            return Err(why);
        }
        let tool = op.tool.as_str();
        let rpm = num(&op.p, "rpm").unwrap_or(0.);
        let r = plan.radius;
        let changed = last_tool.replace(tool) != Some(tool);
        let minutes = plan.duration / 60. + if changed { tool_time } else { 0. };
        // The stock before this operation.
        let earlier = done_ops.last().map_or(&before, |d: &Done| &d.after);
        let left = simulation::leftover_box(earlier, setup, mesh);
        let features = match op.strategy {
            Strategy::Box => {
                let (low, high) = box_block(op, zero)?;
                let open = open_sides(low, high, &plan.stock);
                let allowance = crate::cam::number(op, "allowance")?;
                let (lo, hi) = box_cut(low, high, &plan.stock, allowance);
                // The hand-fed mill moves one axis at a time: a Box cut in
                // passes with a move along two is no box to cut by hand.
                if in_passes(open, lo, hi, r) {
                    for m in plan.moves.iter().filter(|m| m.motion != Motion::Dwell) {
                        moved(m)?;
                    }
                }
                // On X and Y within the stock before it, from the top it
                // meets there.
                Features::Box {
                    lo: std::array::from_fn(|k| {
                        let v = if k == 2 { lo[2] } else { lo[k].max(left.min[k]) };
                        v - zero[k]
                    }),
                    hi: std::array::from_fn(|k| {
                        let v = if k == 2 { hi[2].min(left.max[2]) } else { hi[k].min(left.max[k]) };
                        v - zero[k]
                    }),
                    through: open[2][0],
                }
            }
            Strategy::Drill if op.boring() => {
                // The head's body is far wider than the bore (LIO-563): set
                // down beside a clamp it hits it. Refused as a slot's are.
                // Only the head's ways down and up at each hole count.
                let boxes = doc
                    .holding_boxes(setup, held)
                    .map_err(|e| format!("{}: workholding cannot be checked: {e}", op.name()))?;
                let at_holes: Vec<_> = (plan.moves.iter())
                    .filter(|m| (m.to[0] - m.from[0]).hypot(m.to[1] - m.from[1]) < 1e-4)
                    .map(|m| (m.from, m.to))
                    .collect();
                let parts = crate::holding::plan_parts(&plan);
                let reach = gap + crate::post::blend(machine);
                if let Some(hit) =
                    crate::holding::hits_parts(&at_holes, &parts, &boxes, reach).first()
                {
                    return Err(format!(
                        "{} Hold the part so that the {} stands clear of the boring head, or bore with a bar that reaches farther below its body.",
                        crate::holding::message(op.name(), hit, gap, zero),
                        hit.label
                    ));
                }
                // The finished size, the last pass's, and the holes as it
                // bores them: no passes or head settings (LIO-688).
                let (at, size) = (plan.stops.iter())
                    .filter_map(|s| Some((s.at, s.size?)))
                    .last()
                    .unwrap_or((0, 2. * r));
                let moves = &plan.moves[at.min(plan.moves.len())..];
                let holes = holes_of(moves, zero);
                let uncentred = vec![false; holes.len()];
                Features::Holes {
                    diameter: size,
                    thread: None,
                    holes: hole_rows(doc, op, &plan, &holes, &mouths_of(moves, zero), &uncentred),
                }
            }
            Strategy::Drill => {
                let mut holes = holes_of(&plan.moves, zero);
                let mouths = mouths_of(&plan.moves, zero);
                let tool = doc.tools.iter().find(|t| t.id == op.tool);
                // A centre drill goes down to its pilot's tip, the plan's
                // point being the pilot's top.
                if let Some(c) = tool.and_then(|t| t.centre_drill()) {
                    holes.iter_mut().for_each(|h| h[2] -= c.pilot_length);
                }
                // A twist drill started on a flat face walks off: a plan
                // check (`plan_check`), not said on the page.
                let uncentred: Vec<bool> = holes
                    .iter()
                    .map(|h| {
                        crate::centre::wants_centre(op, &doc.tools, machine)
                            && !opened(
                                earlier,
                                frame,
                                [h[0] + zero[0], h[1] + zero[1]],
                                2. * r,
                                left.max[2],
                                h[2] + zero[2],
                            )
                    })
                    .collect();
                let thread = (op.tapping())
                    .then(|| tool.and_then(|t| t.tap_spec()))
                    .flatten();
                Features::Holes {
                    diameter: 2. * r,
                    thread: thread.map(|s| s.name.to_string()),
                    holes: hole_rows(doc, op, &plan, &holes, &mouths, &uncentred),
                }
            }
            Strategy::Slot => {
                let boxes = doc
                    .holding_boxes(setup, held)
                    .map_err(|e| format!("{}: workholding cannot be checked: {e}", op.name()))?;
                if let Some(hit) =
                    crate::holding::hits(&plan, &boxes, gap + crate::post::blend(machine)).first()
                {
                    return Err(crate::holding::message(op.name(), hit, gap, zero));
                }
                let slot = crate::slot::Slot::read(op, zero)?;
                doc.tools
                    .iter()
                    .find(|t| t.id == op.tool)
                    .ok_or("Slot tool missing")?;
                if plan.slot_passes.is_empty() {
                    return Err(format!("{}: the Slot's passes are missing.", op.name()));
                }
                // As cut: its own width less the stock to leave, its floor
                // with the stock left on it, a through slot's the stock's
                // bottom (`slot::generate`); from the top of the material
                // it starts in.
                let through = get(&op.p, "through") == "Yes";
                let floor = if through {
                    slot.bottom
                } else {
                    slot.bottom + crate::cam::number(op, "axial_allowance").unwrap_or(0.)
                };
                let allowance = crate::cam::number(op, "allowance").unwrap_or(0.);
                Features::Slot {
                    a: [slot.a[0] - zero[0], slot.a[1] - zero[1]],
                    b: [slot.b[0] - zero[0], slot.b[1] - zero[1]],
                    width: slot.width - 2. * allowance,
                    top: plan.rest_top.unwrap_or(slot.top) - zero[2],
                    floor: floor - zero[2],
                    through,
                }
            }
            Strategy::Corner => {
                // Walks that reach into the vise or the clamps are refused,
                // as the planning page says them (LIO-443), and so are
                // they where the workholding cannot be checked.
                let boxes = doc.holding_boxes(setup, held).map_err(|e| {
                    format!(
                        "{}: the workholding cannot be checked against the walks: {e}",
                        op.name()
                    )
                })?;
                let reach = gap + crate::post::blend(machine);
                if let Some(hit) = crate::holding::hits(&plan, &boxes, reach).first() {
                    return Err(format!(
                        "{} Hold the part so that the corners to round stand clear of the vise's jaws or the clamps.",
                        crate::holding::message(op.name(), hit, gap, zero)
                    ));
                }
                let corners = crate::corner::picked(setup, mesh, op)?;
                #[cfg(test)]
                let plan = tests::faulted(op, &plan);
                walks_checked(op, &plan, &corners, zero)?;
                Features::Corners(
                    (corners.iter())
                        .map(|c| Rounded {
                            place: c.place(),
                            centre: [c.centre[0] - zero[0], c.centre[1] - zero[1]],
                            radius: c.r,
                            top: c.top - zero[2],
                            bottom: c.bottom - zero[2],
                        })
                        .collect(),
                )
            }
            other => return Err(format!("{} does not run on a manual mill.", other.name())),
        };
        let n = steps.len();
        sections.push(Section {
            // A Thread as a Thread (LIO-419).
            name: crate::threading::shown_name(op, &doc.tools),
            op: Some(oi),
            steps: n..n + 1,
            count: features.count(),
            minutes,
        });
        steps.push(Step::Sheet {
            op: oi,
            tool: tool.into(),
            // A tap is turned by hand, the spindle off (LIO-410).
            rpm: (!op.tapping()).then_some(rpm),
            features,
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
/// The holes of a drilling, tapping or boring `op` (its `plan`): `holes`
/// their centres and the work Z the tool's tip goes to, `mouths` the work Z
/// each cut starts at, `uncentred` by hole (LIO-688). How deep each goes is
/// what its Check measured (LIO-450): through when the operation breaks
/// out (a cone or a fixture pin's hole never does); else from the mouth,
/// which a cut starts Top over (`rest::start_over`), a fixture pin's on the
/// face itself (`cam::pin_holes`), down to where the hole is its full size:
/// a twist drill's point above its tip, a tap's lead above its tip; a cone,
/// a flat-bottomed bore or a cut too shallow for its point, to its tip.
fn hole_rows(
    doc: &Document,
    op: &Operation,
    plan: &Plan,
    holes: &[[f32; 3]],
    mouths: &[f32],
    uncentred: &[bool],
) -> Vec<Hole> {
    let pins = op.drills_pin_holes();
    let air = if pins {
        0.
    } else {
        crate::cam::number(op, "top").unwrap_or(0.).max(0.)
    };
    let tool = doc.tools.iter().find(|t| t.id == op.tool);
    let cone = tool.is_some_and(|t| {
        ["Chamfer mill", crate::model::CENTRE_DRILL].contains(&get(&t.p, "type"))
    });
    let through = matches!(crate::cam::breakthrough(op), Ok((true, _))) && !cone && !pins;
    let full = if cone {
        0.
    } else if op.tapping() {
        tool.and_then(|t| Some(crate::cam::tap_lead(t) * t.tap_spec()?.pitch))
            .unwrap_or(0.)
    } else if plan.shape == simulation::CutterShape::Drill {
        plan.radius * POINT_RISE
    } else {
        0.
    };
    (holes.iter().zip(mouths).zip(uncentred))
        .map(|((h, m), u)| {
            let (mouth, tip) = (m - air, h[2]);
            let deep = match mouth - tip - full {
                d if d >= 0.005 => d,
                _ => mouth - tip,
            };
            Hole {
                at: [h[0], h[1]],
                mouth,
                depth: match through {
                    true => Depth::Through { tip },
                    false => Depth::Blind { deep, tip },
                },
                uncentred: *u,
            }
        })
        .collect()
}
/// Whether a Round corners' walks, as `plan` made them round `corners`,
/// can be followed on the hand-fed mill (LIO-447): each move a number along
/// one axis, with X and Y set where it goes down and the walk's one way
/// (`corner::walk_problem`). The page lists the corners only (LIO-688),
/// but a walk the plan broke is still refused, and the plan checks say so.
fn walks_checked(
    op: &Operation,
    plan: &Plan,
    corners: &[crate::corner::Corner],
    zero: [f32; 3],
) -> Result<(), String> {
    let walks = &plan.corner_walks;
    if walks.is_empty() {
        return Err(format!(
            "{}: its walks are not known. {}",
            op.name(),
            crate::corner::BROKEN
        ));
    }
    for (k, w) in walks.iter().enumerate() {
        let at = (k + 1, walks.len());
        let (Some(c), Some(moves)) = (corners.get(w.corner), plan.moves.get(w.moves.clone()))
        else {
            return Err(format!(
                "{}: walk {} of {}: its corner or its moves are not the plan's. {}",
                op.name(),
                at.0,
                at.1,
                crate::corner::BROKEN
            ));
        };
        let offset = crate::corner::shown_offset(w, c, plan.radius, zero);
        let lowest = (moves.iter())
            .filter(|m| m.motion == Motion::Feed)
            .flat_map(|m| [m.from[2], m.to[2]])
            .fold(f32::INFINITY, f32::min);
        let legs: Vec<&Move> = moves.iter().filter(|m| m.motion != Motion::Dwell).collect();
        let mut targets = vec![];
        for m in &legs {
            // A move of no number, or along two axes, is no DRO row.
            let axis = (m.from.iter().chain(&m.to).all(|v| v.is_finite()))
                .then(|| moved(m).ok())
                .flatten()
                .ok_or_else(|| {
                    crate::corner::broken(
                        op.name(),
                        at,
                        c.place(),
                        "a move of it is not a number or goes along two axes at once",
                    )
                })?;
            let centre = m.to[axis] - zero[axis];
            let down = axis == 2 && m.to[2] < m.from[2] && (m.to[2] - lowest).abs() < 1e-4;
            targets.push(Target {
                order: 0,
                axis,
                centre,
                shown: centre,
                edge: false,
                feed: m.motion == Motion::Feed,
                // The quill locked at the level for the walk.
                lock: [false, false, down],
            });
        }
        // Each walk stands alone: Z up above the stock, then X and Y where
        // it goes down, even when the walk before left the tool there
        // already (LIO-447: walk 9 of the adapter plate went down with no
        // X).
        let down = targets.iter().position(|t| t.axis == 2 && t.lock[2]);
        if let (Some(down), Some(start)) = (down, legs.first().map(|m| m.from)) {
            let said: Vec<usize> = (targets[..down].iter()).map(|t| t.axis).collect();
            let first_z = said.iter().position(|&k| k == 2);
            let mut ahead = vec![];
            for axis in [2, 0, 1] {
                // Z must come before X and Y are set; X and Y before the
                // tool goes down.
                let needed = if axis == 2 {
                    first_z.is_none_or(|i| said[..i].iter().any(|&k| k < 2))
                } else {
                    !said.contains(&axis)
                };
                if needed {
                    let centre = start[axis] - zero[axis];
                    ahead.push(Target {
                        order: 0,
                        axis,
                        centre,
                        shown: centre,
                        edge: false,
                        feed: false,
                        lock: [false; 3],
                    });
                }
            }
            targets.splice(0..0, ahead);
        }
        for (i, t) in targets.iter_mut().enumerate() {
            t.order = u8::try_from(i + 1).unwrap_or(u8::MAX);
        }
        let points: Vec<[f32; 2]> = (w.stair.points.iter())
            .map(|p| [p[0] as f32, p[1] as f32])
            .collect();
        let s = plan.stock;
        if let Some(why) = crate::corner::walk_problem(
            &targets,
            &points,
            w.z,
            (
                [s.min[0] - zero[0], s.min[1] - zero[1]],
                [s.max[0] - zero[0], s.max[1] - zero[1]],
                s.max[2] - zero[2],
            ),
            &crate::corner::Round {
                centre: [c.centre[0] - zero[0], c.centre[1] - zero[1]],
                radius: c.r,
                quarter: c.quarter,
                tool: plan.radius,
                offset,
                scallop: w.stair.scallop as f32,
            },
        ) {
            return Err(crate::corner::broken(op.name(), at, c.place(), &why));
        }
    }
    Ok(())
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
    /// A dial wound `out` (or in) by `distance`.
    fn wound(&self, slide: Slide, distance: f32, out: bool) -> Dial {
        Dial::wound(self.machine, slide, mm(distance), out)
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
    // The Zero is made with the first turning tool, which cuts outside;
    // with none, with the first Turn's tool, touching the bar's outside only
    // when that tool cuts outside.
    let type_of = |o: &Operation| tool_of(&o.tool).map_or("", |t| get(&t.p, "type"));
    // A die is turned by hand: it touches nothing.
    let turns: Vec<&Operation> = (done.iter())
        .map(|(oi, _)| &setup.operations[*oi])
        .filter(|o| o.strategy == Strategy::Turn && type_of(o) != "Die")
        .collect();
    let inside = |o: &Operation| {
        type_of(o) == "Boring bar"
            || match get(&o.p, "turn_kind") {
                "Bore" => true,
                "Thread" => get(&o.p, "thread_side") == "Inside",
                "Chamfer" => get(&o.p, "edge") == "End inside",
                _ => false,
            }
    };
    let zero_op = (turns.iter())
        .find(|o| type_of(o) == "Turning tool")
        .or(turns.first());
    if let Some(op) = zero_op {
        steps.push(Step::Lathe(LatheStep::Zero {
            tool: op.tool.clone(),
            key: zero_key,
            planned: bar,
            measured: bar_measured,
            cross: !inside(op),
            // The measured bar is what the outside's touches read from.
            measure: turns.iter().any(|o| !inside(o)),
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
    // The top slide's angle from the cross slide while a thread left it
    // swung; None while it is along the bed.
    let mut swung: Option<f32> = None;
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
                    swung: swung.take(),
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
                    let mut cross = zeroes.wound(Slide::Cross, off(d), bore);
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
                                    reads: None,
                                    ..cross
                                }
                            } else if past {
                                Dial {
                                    reading: Err(format!(
                                        "Ø{was:.2} is not what was cut from Ø{from:.2}: measure again."
                                    )),
                                    reads: None,
                                    ..cross
                                }
                            } else {
                                zeroes.wound(Slide::Cross, at + left.max(0.), bore)
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
                        to_jaws: mm(turn::work_z(stock, m.to[2]) + stickout),
                    }));
                    last = Some((d, cross));
                }
                n
            }
            "Drill" if op.tapping() => {
                let first = feeds.first().ok_or_else(missing)?;
                let spec = tool_of(tool)
                    .and_then(Tool::tap_spec)
                    .ok_or("Choose a tap, with its thread size, for Tapping.")?;
                // Where the tap touches the hole's mouth: its plan starts
                // 1 mm in front of it.
                let quill = first.from[2] - turn::CLEAR - plan.target;
                let lead = mm(tool_of(tool).map_or(0., crate::cam::tap_lead) * spec.pitch);
                steps.push(Step::Lathe(LatheStep::TailTap {
                    op: oi,
                    tool: tool.into(),
                    tap: spec.name.into(),
                    pitch: mm(spec.pitch),
                    depth: mm(quill - lead),
                    lead,
                    turns: tenths_down(quill / spec.pitch),
                    quill: zeroes.dial(Slide::Quill, quill),
                }));
                steps.push(Step::Lathe(LatheStep::Try {
                    op: oi,
                    what: format!("{} screw", spec.name),
                    spring: false,
                }));
                1
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
                    swung: swung.take(),
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
            "Thread" if tool_of(tool).and_then(lathe::die_spec).is_some() => {
                let z0 = along.first().ok_or_else(missing)?.to[2];
                let spec = tool_of(tool)
                    .and_then(lathe::die_spec)
                    .ok_or_else(missing)?;
                let length = mm(end - z0);
                steps.push(Step::Lathe(LatheStep::DieThread {
                    op: oi,
                    tool: tool.into(),
                    die: spec.name.into(),
                    pitch: mm(spec.pitch),
                    diameter: num(&op.p, "diameter").unwrap_or(spec.major),
                    od: tool_of(tool).and_then(|t| num(&t.p, "od")).unwrap_or(0.),
                    length,
                    turns: tenths_down(length / spec.pitch),
                }));
                steps.push(Step::Lathe(LatheStep::Try {
                    op: oi,
                    what: format!("{} nut", spec.name),
                    spring: false,
                }));
                1
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
                steps.push(Step::Lathe(LatheStep::ThreadSetUp {
                    op: oi,
                    rpm,
                    pitch,
                    angle,
                }));
                let n = along.len();
                // In toward the axis from the touch (out, inside), as
                // planned.
                let infeeds: Vec<f32> = (along.iter())
                    .map(|m| mm(if inside { m.to[0] - r0 } else { r0 - m.to[0] }.max(0.)))
                    .collect();
                let travels = thread_travels(machine, &infeeds, angle);
                let deepest = travels.last().copied().unwrap_or(0.);
                for (i, infeed) in infeeds.iter().enumerate() {
                    let travel = travels[i];
                    let top = zeroes.wound(Slide::Top, travel, inside);
                    // What the top slide set there feeds in.
                    let infeed = if i + 1 == n {
                        mm(deepest * angle.to_radians().cos())
                    } else {
                        *infeed
                    };
                    steps.push(Step::Lathe(LatheStep::ThreadPass {
                        op: oi,
                        index: i + 1,
                        of: n,
                        rpm,
                        pitch,
                        angle,
                        infeed,
                        top,
                        carriage: zeroes.dial(Slide::Carriage, zc - z0),
                    }));
                }
                let label = thread_label(num(&op.p, "diameter").unwrap_or(0.), pitch);
                let mate = if inside { "screw" } else { "nut" };
                steps.push(Step::Lathe(LatheStep::Try {
                    op: oi,
                    what: format!("{label} {mate}"),
                    spring: true,
                }));
                // The top slide is swung and fed in: the carriage zero no
                // longer holds.
                zeroes.carriage = None;
                swung = Some(angle);
                n
            }
            other => return Err(format!("\"{other}\" does not run on the manual lathe.")),
        };
        // A thread's section is named for it (LIO-415).
        let name = if kind == "Thread" {
            let pitch = num(&op.p, "pitch").unwrap_or(0.);
            match tool_of(tool).and_then(lathe::die_spec) {
                Some(spec) => format!("Thread {}", spec.name),
                None => format!(
                    "Thread {}",
                    thread_label(num(&op.p, "diameter").unwrap_or(0.), pitch)
                ),
            }
        } else {
            crate::threading::shown_name(op, &doc.tools)
        };
        sections.push(Section {
            name,
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

#[cfg(any(test, feature = "test-support"))]
#[cfg_attr(not(test), allow(dead_code, unused_imports))]
pub mod tests {
    use super::*;
    use crate::model::BOX_FIELDS;

    /// Damage done to the plan a Round corners' Steps are said from, by
    /// operation id: for tests that the Steps refuse a broken walk and the
    /// plan checks say so (LIO-447). Keyed by id, so tests running at once
    /// touch only their own operation.
    pub(crate) static CORNER_FAULTS: std::sync::Mutex<Vec<(String, Fault)>> =
        std::sync::Mutex::new(vec![]);
    /// Damage done to a plan (`CORNER_FAULTS`).
    pub(crate) type Fault = fn(&mut Plan);
    /// `plan` with the damage registered for `op`, if any.
    pub(crate) fn faulted(op: &Operation, plan: &Plan) -> Plan {
        let mut plan = plan.clone();
        for (id, fault) in CORNER_FAULTS.lock().unwrap().iter() {
            if *id == op.id {
                fault(&mut plan);
            }
        }
        plan
    }

    /// A Box named `name` cutting `b` (x_min, x_max, y_min, y_max, z_top,
    /// z_bottom, work frame) with the tool `tool`.
    pub fn boxed(name: &str, tool: &str, b: [f32; 6], fields: &[(&str, &str)]) -> Operation {
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
    pub fn job(dro: &str) -> (Document, Mesh) {
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
                Step::Zero { .. } => "Zero",
                Step::Sheet { .. } => "Sheet",
                Step::HandOver { .. } => "HandOver",
                Step::Lathe(l) => l.title(),
            })
            .collect()
    }
    pub fn close(a: f32, b: f32) {
        assert!((a - b).abs() < 1e-3, "{a} != {b}");
    }
    /// What operation `op`'s page lists.
    pub(crate) fn features(guide: &Guide, op: usize) -> &Features {
        (guide.steps.iter())
            .find_map(|s| match s {
                Step::Sheet {
                    op: o, features, ..
                } if *o == op => Some(features),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no page for operation {op}: {:?}", guide.steps))
    }
    /// The holes operation `op`'s page lists.
    pub(crate) fn holes_of_op(guide: &Guide, op: usize) -> &[Hole] {
        match features(guide, op) {
            Features::Holes { holes, .. } => holes,
            f => panic!("{f:?}"),
        }
    }
    /// Check actual continuous sweeps on both finished planes, through the
    /// full blank thickness and length: no ledge within the 8 mm jaw band.
    pub fn squared_acceptance(doc: &Document, mesh: &Mesh) {
        for (si, expected) in [[140., 20., 94.], [140., 20., 91.], [140., 88., 20.]]
            .into_iter()
            .enumerate()
        {
            let walked = simulate(doc, mesh, si, "empty").unwrap();
            let plan = &walked.done[0].1;
            let setup = &doc.setups[si];
            let frame = crate::volume::Frame::new(setup, mesh);
            let part = simulation::part_box(setup, mesh);
            let bands = crate::holding::Bands::new(&walked.before, frame, part);
            let held = crate::holding::held_by(doc, setup, plan.stock, Some(&bands));
            let removed = crate::rest::Removed::new(&walked.before, frame);
            let issues = crate::holding::holdable(
                si,
                &crate::holding::layout(setup).unwrap(),
                doc.holder(get(&setup.p, "holder")).unwrap(),
                held,
                Some(&removed),
                None,
                &|_, _| String::new(),
            );
            assert!(issues.is_empty(), "{issues:?}");
            let guide = build(doc, mesh, si).unwrap();
            let Step::Zero { stock, .. } = guide.steps[0] else {
                panic!()
            };
            for (v, want) in stock.dimensions().into_iter().zip(expected) {
                assert!((v - want).abs() < 0.005, "setup {}: {stock:?}", si + 1);
            }
            eprintln!(
                "setup {} remaining envelope {:?}; jaw band {:?}; holding accepted",
                si + 1,
                stock.dimensions(),
                held.band.dimensions()
            );
            if si == 2 {
                let b = plan.stock;
                let mut samples = 0;
                for i in 0..=140 {
                    for j in 0..=80 {
                        let x = b.min[0] + b.dimensions()[0] * i as f32 / 140.;
                        let z = part.min[2] + part.dimensions()[2] * j as f32 / 80.;
                        for (face, sign) in [(part.min[1], -1.), (part.max[1], 1.)] {
                            assert!(
                                removed.at([x, face + sign * 0.02, z]),
                                "uncut ledge at {x}, {face}, {z}"
                            );
                            assert!(
                                !removed.at([x, face - sign * 0.02, z]),
                                "gouged contact at {x}, {face}, {z}"
                            );
                            samples += 2;
                        }
                    }
                }
                eprintln!(
                    "{samples} continuous-sweep point checks, 1 mm along X and 0.25 mm through thickness; ±0.02 mm of both finished sides"
                );
            }
        }
    }
    #[test]
    #[ignore = "private saved-project acceptance: AXIS_REGRIP_FIXTURE"]
    fn saved_squared_edge_regrip() {
        let dir = std::path::PathBuf::from(std::env::var_os("AXIS_REGRIP_FIXTURE").unwrap());
        let mut doc = crate::storage::load(&dir.join("adapter-plate.axis.json")).unwrap();
        doc.file.as_mut().unwrap().path = dir.join("adapter-plate.step");
        let mesh = crate::mesh::load_file(doc.file.as_ref().unwrap()).unwrap();
        for expanded in [false, true] {
            let mut trial = doc.clone();
            if expanded {
                for setup in &mut trial.setups[..2] {
                    for (k, v) in [
                        ("x_min", "-71"),
                        ("x_max", "71"),
                        ("y_min", "-11"),
                        ("y_max", "11"),
                    ] {
                        setup.operations[0].p.insert(k.into(), v.into());
                    }
                }
            }
            eprintln!(
                "PRIVATE ACCEPTANCE: {}",
                if expanded {
                    "expanded X ±71 / Y ±11 bounds"
                } else {
                    "original X ±70 / Y ±10 bounds"
                }
            );
            squared_acceptance(&trial, &mesh);
        }
    }
    /// A 120 × 90 × 10 plate with Ø6 holes through at `at` (its middle at
    /// 0), drilled Ø6 on a manual mill; with `centred` a centre drilling of
    /// the same holes before it.
    pub(crate) fn holes_job(at: &[[f32; 2]], centred: bool) -> (Document, Mesh) {
        use crate::cam::boring::tests::{drill, drilling, job};
        let outer = [
            (-60., -45., 0.),
            (60., -45., 0.),
            (60., 45., 0.),
            (-60., 45., 0.),
        ];
        let circles: Vec<[(f64, f64, f64); 2]> = (at.iter())
            .map(|p| [f64::from(p[0]), f64::from(p[1])])
            .map(|[x, y]| [(x + 3., y, 1.), (x - 3., y, 1.)])
            .collect();
        let mut loops: Vec<&[(f64, f64, f64)]> = vec![&outer[..]];
        loops.extend(circles.iter().map(|c| &c[..]));
        let mesh = crate::cam::arcs::tests::slab(&loops, 10., 0., true, true);
        let mesh = crate::mesh::normalize(mesh).unwrap();
        let machine = manual::defaults();
        let mut d = drill("6");
        d.p.insert("machines".into(), machine.id.clone());
        let mut op = drilling(&mesh, &d, "Drilling");
        for (k, v) in [("name", "Holes"), ("rpm", "1000"), ("feed", "100")] {
            op.p.insert(k.into(), v.into());
        }
        let (mut doc, _) = job(&mesh, machine, vec![(d, op)]);
        if centred {
            let mut c = Tool::new("Centre drill A2.5×6.3", crate::model::CENTRE_DRILL, "6.3");
            c.set_type(crate::model::CENTRE_DRILL);
            c.p.insert("stickout".into(), "40".into());
            doc.tools.push(c);
            let (ops, _) = crate::centre::needed(&doc, 0, None);
            crate::centre::insert(&mut doc, 0, ops);
        }
        (doc, mesh)
    }
    /// The plate's 3 × 4 grid: Ø6 holes 20 apart in X, 15 in Y.
    pub(crate) fn plate_grid() -> Vec<[f32; 2]> {
        (0..3)
            .flat_map(|r| (0..4).map(move |c| [-30. + 20. * c as f32, -15. + 15. * r as f32]))
            .collect()
    }
    /// The plate's bolt circle: six Ø6 holes on Ø60, the first at 30°.
    pub(crate) fn plate_circle() -> Vec<[f32; 2]> {
        (0..6)
            .map(|k| (30. + 60. * k as f32).to_radians())
            .map(|a| [30. * a.cos(), 30. * a.sin()])
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
        // The zero, then a page an operation (LIO-688).
        assert_eq!(of(0), ["Zero"]);
        for k in 1..4 {
            assert_eq!(of(k), ["Sheet"]);
        }
        assert_eq!(of(4), ["HandOver"]);
        // What each page lists: a box each, four holes.
        let counts: Vec<usize> = guide.sections.iter().map(|s| s.count).collect();
        assert_eq!(counts, [0, 1, 1, 4, 0]);
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
    fn a_mill_too_short_for_the_moves_is_refused_on_every_route() {
        let (mut doc, mesh) = job("Yes");
        let routes = |doc: &Document| {
            let mut all = vec![];
            simulation::sequence(doc, &mesh, None, |si, _, r| {
                if si == 0 {
                    all.push(r.err())
                }
            });
            // The Work page's generation, Generate all and the Steps page.
            (
                simulation::prepare(doc, &mesh, 0, 0).err(),
                all,
                build(doc, &mesh, 0).err(),
            )
        };
        (doc.setups[0].machine.p).insert("x".into(), "100".into());
        let (work, all, steps) = routes(&doc);
        let says = |e: &Option<String>| {
            e.as_ref()
                .is_some_and(|e| e.contains("The manual mill travels 100 mm in X"))
        };
        assert!(says(&work), "{work:?}");
        assert!(all.len() == 3 && all.iter().all(says), "{all:?}");
        assert!(says(&steps), "{steps:?}");
        (doc.setups[0].machine.p).insert("x".into(), "700".into());
        let (work, all, steps) = routes(&doc);
        assert!(work.is_none() && all.iter().all(Option::is_none) && steps.is_none());
    }
    #[test]
    fn a_manual_setups_holding_is_checked_without_tool_numbers() {
        // No tool has a T number: the manual mill changes them by hand, so
        // its holding is still walked; the CNC setup's is not (LIO-176).
        let (mut doc, mesh) = job("Yes");
        in_the_vise(&mut doc);
        for t in &mut doc.tools {
            t.p.remove("number");
        }
        let mut results: Vec<Vec<_>> = vec![vec![]; doc.setups.len()];
        simulation::sequence(&doc, &mesh, None, |si, oi, r| results[si].push((oi, r)));
        let held = |si: usize| crate::post::held(&doc, &mesh, si, &results[si]);
        let unchecked =
            |si: usize| (held(si).iter()).any(|h| h.message.starts_with("Holding not checked"));
        assert!(!unchecked(0), "{:?}", held(0));
        assert!(unchecked(1), "{:?}", held(1));
    }
    /// LIO-688: the zero page holds the stock as it starts, where the zero
    /// is on it; each operation's page its tool, its speed and what it
    /// cuts, in work coordinates.
    #[test]
    fn steps_run_the_zero_then_a_page_an_operation() {
        let (doc, mesh) = job("Yes");
        let guide = build(&doc, &mesh, 0).unwrap();
        assert_eq!(
            kinds(&guide, 0..guide.steps.len()),
            ["Zero", "Sheet", "Sheet", "Sheet", "HandOver"]
        );
        let Step::Zero {
            stock,
            holding,
            vise,
            clamps,
        } = &guide.steps[0]
        else {
            panic!()
        };
        assert_eq!((*vise, clamps.len()), (None, 0));
        // In work coordinates: the zero on the stock top's front-left corner.
        assert_eq!((stock.min[0], stock.min[1], stock.max[2]), (0., 0., 0.));
        close(stock.max[1], 120.);
        assert_eq!(holding.stock, *stock);
        let Step::Sheet {
            op,
            tool,
            rpm,
            features: Features::Box { lo, hi, through },
        } = &guide.steps[1]
        else {
            panic!("{:?}", guide.steps[1])
        };
        assert_eq!((*op, tool, *rpm, *through), (0, &doc.tools[3].id, Some(640.), false));
        close(lo[2], -2.);
        close(hi[2], 0.);
        for k in 0..2 {
            close(lo[k], stock.min[k]);
            close(hi[k], stock.max[k]);
        }
        let holes = holes_of_op(&guide, 2);
        assert_eq!(holes.len(), 4);
        // On the Face's floor.
        for h in holes {
            close(h.mouth, -2.);
        }
        assert_eq!(guide.steps[4], Step::HandOver { next: Some(1) });
    }
    /// The job in the manual mill's vise, its jaws on the Y sides.
    pub fn in_the_vise(doc: &mut Document) {
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
    fn the_zero_step_has_the_vise_its_grip_and_jaw_height() {
        let (mut doc, mesh) = job("Yes");
        in_the_vise(&mut doc);
        let guide = build(&doc, &mesh, 0).unwrap();
        let Step::Zero {
            stock,
            holding,
            vise: Some(vise),
            ..
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
    /// The shaft turned on the lathe (`proposal::tests::turned`), Ø20 over
    /// its top 20 mm and Ø30 under it; the manual mill, setup 2, then
    /// drills its Ø4 hole from the model origin on its axis with T8, and
    /// the CNC drills it again: the job the planner builds of it.
    pub fn turned_then_drilled_by_hand() -> (Document, Mesh) {
        let (doc, mesh, report, plan) = lathe_then_by_hand(None, |_| {}, |_, _| {});
        let job = crate::proposal::build(&doc, &mesh, &report, &plan).unwrap();
        (job, mesh)
    }
    /// `proposal::tests::with_lathe` turned about Z from `section` (radius,
    /// height pairs; None: its own shaft), with a manual mill in the library
    /// and T8, a Ø4 drill, on it; `edit` changes the document before its
    /// report is made. The plan is `proposal::tests::turned`, of the report,
    /// with a copy of its CNC setup put second on the manual mill, drilling
    /// with T8 at 1600 rpm and 150 mm/min; `plan` then changes it.
    pub(crate) fn lathe_then_by_hand(
        section: Option<&[(f32, f32)]>,
        edit: impl FnOnce(&mut Document),
        plan: impl FnOnce(&mut crate::proposal::Proposal, &crate::analysis::PartReport),
    ) -> (
        Document,
        Mesh,
        crate::analysis::PartReport,
        crate::proposal::Proposal,
    ) {
        use crate::proposal::tests::{turned, with_lathe};
        let (mut doc, mut mesh, _) = with_lathe();
        if let Some(section) = section {
            mesh = crate::mesh::normalize(crate::cam::tests::revolved(section)).unwrap();
        }
        let mill = crate::manual::defaults();
        let mut drill = crate::model::Tool::new("Drill 4 hand", "Drill", "4");
        for (k, v) in [
            ("length", "60"),
            ("stickout", "65"),
            ("flutes", "2"),
            ("number", "8"),
            ("machines", mill.id.as_str()),
        ] {
            drill.p.insert(k.into(), v.into());
        }
        doc.tools
            .push(crate::proposal::tests::hand_centre_drill(&mill));
        doc.machines.push(mill);
        doc.tools.push(drill);
        edit(&mut doc);
        let report = crate::analysis::report(&doc, &mesh).unwrap();
        let mut planned = turned(&report);
        let mut hand = planned.setups[1].clone();
        hand.machine = (report.machines.iter())
            .find(|m| m.kind == "Manual mill")
            .map(|m| m.number);
        hand.name = "Drill by hand".into();
        let op = &mut hand.operations[0];
        (op.tool, op.rpm, op.feed) = (8, 1600., 150.);
        planned.setups.insert(1, hand);
        plan(&mut planned, &report);
        (doc, mesh, report, planned)
    }
    /// Holds setup 1 of `doc`, and so the setups that share its holder, in
    /// the starter soft jaws bored upright to `bore`, `height` tall,
    /// gripping `grip` of the stock, on its CNC and the manual mills.
    pub(crate) fn in_soft_jaws(doc: &mut Document, bore: &str, height: &str, grip: &str) {
        let mut jaws = crate::round_hold::starters()
            .into_iter()
            .find(|h| h.kind() == crate::round_hold::SOFT_JAWS)
            .unwrap();
        jaws.p.insert("bore".into(), bore.into());
        jaws.p.insert("jaw_height".into(), height.into());
        jaws.p
            .insert("name".into(), format!("Soft jaws bored Ø{bore}"));
        // On the CNC and on the hand mills.
        let machines: Vec<&str> = std::iter::once(doc.setups[0].machine.id.as_str())
            .chain(
                (doc.machines.iter())
                    .filter(|m| manual::is_manual(m))
                    .map(|m| m.id.as_str()),
            )
            .collect();
        jaws.p.insert("machines".into(), machines.join(","));
        let p = &mut doc.setups[0].p;
        for (k, v) in [
            ("holding", crate::round_hold::SOFT_JAWS),
            ("holder", &jaws.id),
            ("jaw_bore", "Z"),
            ("round_grip", grip),
            ("round_offset", "0"),
        ] {
            p.insert(k.into(), v.into());
        }
        doc.workholding.push(jaws);
    }
    /// A short stepped part, Ø30 for its bottom 10 mm and Ø20 for the 20
    /// above, held by its Ø30 in soft jaws 8 mm tall gripping 8
    /// (`stepped_in_soft_jaws`).
    pub fn short_part_in_soft_jaws() -> (
        Document,
        Mesh,
        crate::analysis::PartReport,
        crate::proposal::Proposal,
    ) {
        stepped_in_soft_jaws(10., 8.)
    }
    /// A part 30 mm tall, Ø30 for its bottom `foot` mm and Ø20 above, a Ø4
    /// hole 10 deep in its top, turned from a Ø34 × 70 bar
    /// (`lathe_then_by_hand`: faced 2, Ø30 to −35, Ø20 down to the foot,
    /// parted off at −32), then held by its Ø30 in soft jaws bored Ø30,
    /// `grip` tall and gripping `grip`, on the manual mill and the CNC.
    pub(crate) fn stepped_in_soft_jaws(
        foot: f32,
        grip: f32,
    ) -> (
        Document,
        Mesh,
        crate::analysis::PartReport,
        crate::proposal::Proposal,
    ) {
        let section = [
            (0., 0.),
            (15., 0.),
            (15., foot),
            (10., foot),
            (10., 30.),
            (2., 30.),
            (2., 20.),
            (0., 20.),
        ];
        let grip = grip.to_string();
        lathe_then_by_hand(
            Some(&section),
            |doc| in_soft_jaws(doc, "30", &grip, &grip),
            |plan, _| {
                let lathe = &mut plan.setups[0];
                lathe.bar.as_mut().unwrap().length = 70.;
                lathe.operations[1].turn.as_mut().unwrap().z_end = Some(-35.);
                lathe.operations[2].turn.as_mut().unwrap().z_end = Some(foot - 32.);
                lathe.operations[3].turn.as_mut().unwrap().z = Some(-32.);
            },
        )
    }
    /// `job`'s manual setup held down by the starter toe clamp, made
    /// `width` wide, at `clamps` (a clamp layout) instead of its vise
    /// (LIO-458).
    pub fn on_clamps(doc: &mut Document, clamps: &str, width: &str) {
        let mut holder = (crate::holding::starters().into_iter())
            .find(|h| h.kind() == "Clamp")
            .unwrap();
        holder
            .p
            .insert("machines".into(), doc.setups[0].machine.id.clone());
        holder.p.insert("width".into(), width.into());
        for (k, v) in [
            ("holding", "Clamps"),
            ("holder", holder.id.as_str()),
            ("clamps", clamps),
        ] {
            doc.setups[0].p.insert(k.into(), v.into());
        }
        doc.workholding.push(holder);
    }
    /// LIO-688: the zero page says where the zero is, not how to find it,
    /// but a zero the edge finder cannot reach is still refused, as the
    /// plan check says (LIO-451, LIO-458): no finder, nothing clear above
    /// soft jaws, a face it must touch all under a clamp, a middle with both
    /// faces under clamps. One face of a middle under a clamp is not.
    #[test]
    fn a_zero_the_edge_finder_cannot_reach_is_refused() {
        let (mut doc, mesh) = job("Yes");
        (doc.setups[0].machine.p).insert("edge_finder".into(), "0".into());
        let error = build(&doc, &mesh, 0).unwrap_err();
        assert!(error.contains("edge finder"), "{error}");
        // The Ø20 over a Ø30 gripped 28 of its 30 in soft jaws: 2 mm of it
        // stands over them; gripped 29.6, nothing stands 0.5 mm clear.
        let (doc, mesh, report, plan) = stepped_in_soft_jaws(28., 28.);
        let (check, job2) = crate::plan_check::check(&doc, &mesh, &report, &plan);
        assert!(check.passed, "{:?}", check.problems);
        let mut job2 = job2.unwrap();
        job2.setups[1].p.insert("round_grip".into(), "28".into());
        assert!(build(&job2, &mesh, 1).is_ok());
        job2.setups[1].p.insert("round_grip".into(), "29.6".into());
        assert_eq!(build(&job2, &mesh, 1).unwrap_err(), NOT_CLEAR);
        // A strap over the whole front face.
        let centred = |clamps: &str| {
            let (mut doc, mesh) = job("No");
            on_clamps(&mut doc, clamps, "140");
            doc.setups[0].operations.drain(..2);
            (doc.setups[0].p).insert("origin".into(), "Stock top center".into());
            (doc, mesh)
        };
        // The zero at the top's centre: Y from the back face alone.
        let (doc, mesh) = centred("-Y:0");
        let guide = build(&doc, &mesh, 0).unwrap();
        let Step::Zero { clamps, .. } = &guide.steps[0] else {
            panic!("{:?}", guide.steps[0])
        };
        assert_eq!(clamps[0].0, "clamp on -Y (offset 0)");
        // The front and the back both under straps: Y cannot be found.
        let (doc, mesh) = centred("-Y:0;+Y:0");
        assert_eq!(
            build(&doc, &mesh, 0).unwrap_err(),
            format!(
                "{UNDER_CLAMPS} on the front face (clamp on -Y (offset 0)) or the back face (clamp on +Y (offset 0)). Move a clamp along its side so that the finder can touch one of them, or choose a work zero on faces the clamps leave clear."
            )
        );
        // A zero on the front-left corner must touch the front: refused,
        // and the plan check says why.
        let (mut doc, mesh) = job("No");
        on_clamps(&mut doc, "-Y:0", "140");
        let why = build(&doc, &mesh, 0).unwrap_err();
        assert!(
            why.starts_with(&format!(
                "{UNDER_CLAMPS} on the front face (clamp on -Y (offset 0))."
            )),
            "{why}"
        );
        doc.adopt_materials();
        doc.material_id = doc.materials[0].id.clone();
        let report = crate::analysis::report(&doc, &mesh).unwrap();
        let check = crate::plan_check::check_built(&doc, &mesh, &report);
        assert!(
            (check.problems.iter())
                .any(|p| p.setup == 1 && p.stage == "workholding" && p.message == why),
            "{:?}",
            check.problems
        );
        // A 20 wide clamp on the front, clear of the corner: given.
        let (mut doc, mesh) = job("No");
        on_clamps(&mut doc, "-Y:0", "20");
        assert!(build(&doc, &mesh, 0).is_ok());
    }
    #[test]
    fn clamps_that_cannot_be_placed_give_no_zero() {
        // A clamp beyond its side: no touch is shown as if it were not
        // there; the holding's own refusal is.
        let (mut doc, mesh) = job("No");
        on_clamps(&mut doc, "-Y:0;+Y:100", "20");
        let why = build(&doc, &mesh, 0).unwrap_err();
        assert!(why.contains("beyond its"), "{why}");
        // Nor clamps whose layout cannot be read: a gap below 0, a
        // position that is not a number.
        for (key, value) in [("holding_gap", "-1"), ("clamps", "-Y:0;+Y:bad")] {
            let (mut doc, mesh) = job("No");
            on_clamps(&mut doc, "-Y:0;+Y:0", "20");
            doc.setups[0].p.insert(key.into(), value.into());
            let why = build(&doc, &mesh, 0).unwrap_err();
            assert!(!why.is_empty() && !why.starts_with(UNDER_CLAMPS), "{why}");
        }
    }
    #[test]
    }
    #[test]
    fn a_step_beside_a_hole_or_a_flat_face_does_not_centre_it_and_a_cone_does() {
        use crate::simulation::CutterShape;
        let f = crate::volume::Frame::identity();
        let b = Bounds {
            min: [0., 0., 0.],
            max: [40., 40., 10.],
        };
        let fresh = Volume::new(b, f);
        assert!(!opened(&fresh, f, [30., 20.], 6., 10., 0.));
        // A step: the stock up to X 20 cut down to Z 5. A hole at X 17.5
        // has its axis on the step's floor and part of its rim on the
        // step's wall, higher.
        let mut v = fresh.clone();
        for y in (0..=40).step_by(4) {
            let y = y as f32;
            v.cut(
                [-10., y, 5.],
                [15., y, 5.],
                5.,
                20.,
                CutterShape::Flat,
                f,
                (0, 0),
            );
        }
        assert!(!opened(&v, f, [17.5, 20.], 6., 10., 0.));
        // A centre drill's cone opened to Ø6 at the top centres it.
        let c = crate::model::CentreDrill {
            pilot: 2.5,
            pilot_length: 3.1,
            body: 6.3,
        };
        let point = 10. - c.cone_depth(6.);
        v.cut(
            [30., 20., 12.],
            [30., 20., point],
            3.15,
            10.,
            c.shape(),
            f,
            (0, 0),
        );
        assert!(opened(&v, f, [30., 20.], 6., 10., 0.));
    }
    #[test]
    fn two_spots_closer_than_the_drill_each_centre_their_own_hole() {
        // A slot's ends, Ø8 and 5 apart, each centre drilled (LIO-536):
        // each end's rim reaches into the other's pilot, as deep as its
        // own axis, and each is still centred in its own cone.
        let f = crate::volume::Frame::identity();
        let b = Bounds {
            min: [0., 0., -20.],
            max: [40., 40., 0.],
        };
        let mut v = Volume::new(b, f);
        let c = crate::model::CentreDrill {
            pilot: 2.5,
            pilot_length: 3.1,
            body: 6.3,
        };
        let point = -c.cone_depth(8.);
        let spot = |v: &mut Volume, x: f32| {
            v.cut(
                [x, 20., 5.],
                [x, 20., point],
                3.15,
                10.,
                c.shape(),
                f,
                (0, 0),
            )
        };
        spot(&mut v, 15.);
        // One end spotted: the other, on the flat beside it, is not.
        assert!(opened(&v, f, [15., 20.], 8., 0., -20.));
        assert!(!opened(&v, f, [20., 20.], 8., 0., -20.));
        spot(&mut v, 20.);
        assert!(opened(&v, f, [15., 20.], 8., 0., -20.));
        assert!(opened(&v, f, [20., 20.], 8., 0., -20.));
    }
    #[test]
    fn the_holes_are_centres_and_depths_in_work_coordinates() {
        let (doc, mesh) = job("Yes");
        let guide = build(&doc, &mesh, 0).unwrap();
        let holes = holes_of_op(&guide, 2);
        assert_eq!(holes.len(), 4);
        let plan = &guide.done[2].plan;
        for h in holes {
            let at: Vec<&Move> = (plan.moves.iter())
                .filter(|m| {
                    m.cutting_pass
                        && m.to[0] - guide.zero[0] == h.at[0]
                        && m.to[1] - guide.zero[1] == h.at[1]
                })
                .collect();
            assert!(!at.is_empty());
            let deepest = at.iter().map(|m| m.to[2]).fold(f32::INFINITY, f32::min);
            let (Depth::Through { tip } | Depth::Blind { tip, .. }) = h.depth;
            assert_eq!(tip, deepest - guide.zero[2]);
            assert!(
                tip < -2. && (0. ..124.).contains(&h.at[0]) && (20. ..100.).contains(&h.at[1]),
                "{h:?}"
            );
        }
    }
    #[test]
    fn the_place_is_saved_with_the_project_and_kept_within_the_steps() {
        let (mut doc, mesh) = job("Yes");
        let guide = build(&doc, &mesh, 0).unwrap();
        assert_eq!(place(&doc, &guide), 0);
        let id = doc.setups[0].id.clone();
        // The hand-over (LIO-688: the zero, a page an operation).
        doc.guide.insert(id.clone(), 4);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("job.axis.json");
        crate::storage::save(&path, &doc).unwrap();
        let mut doc = crate::storage::load(&path).unwrap();
        assert_eq!(doc.guide.get(&id), Some(&4));
        assert_eq!(place(&doc, &guide), 4);
        // Without the Step the setup has 4 steps: the place is the last.
        doc.setups[0].operations.remove(1);
        let guide = build(&doc, &mesh, 0).unwrap();
        assert_eq!(guide.steps.len(), 4);
        assert_eq!(place(&doc, &guide), 3);
        // A project saved without places loads with none.
        let mut json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        json.as_object_mut().unwrap().remove("guide");
        let old: Document = serde_json::from_value(json).unwrap();
        assert!(old.guide.is_empty());
    }
    #[test]
    fn only_a_box_neither_facing_nor_squaring_is_cut_in_passes() {
        let all = [[true; 2]; 3];
        let (lo, hi, r) = ([0., 0., -10.], [124., 120., 0.], 12.5);
        assert!(!in_passes(all, lo, hi, r));
        // A wall on −X with a 1 mm slab off it: squaring, floor open or closed.
        let mut wall = all;
        wall[0][0] = false;
        let slab = [123., 0., -10.];
        assert!(!in_passes(wall, slab, hi, r));
        wall[2][0] = false;
        assert!(!in_passes(wall, slab, hi, r));
        // As wide as the radius still is; wider is not.
        assert!(!in_passes(wall, [111.5, 0., -10.], hi, r));
        assert!(in_passes(wall, [111.4, 0., -10.], hi, r));
        // Two closed sides are never squaring, however thin.
        let mut slot = all;
        slot[1] = [false; 2];
        assert!(in_passes(slot, [0., 50., -10.], [124., 60., 0.], r));
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
    pub fn sample(reads: &str) -> (Document, Mesh) {
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
                LatheStep::Face { top, .. }
                | LatheStep::TailDrill { quill: top, .. }
                | LatheStep::TailTap { quill: top, .. } => out.push(top),
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
    pub(crate) fn reads(dial: &Dial, turns: u32, reading: f32) {
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
        // A thread's section is named for its thread (LIO-415).
        let ops: Vec<String> = (doc.setups[0].operations.iter())
            .map(|o| match o.name() {
                "Turn · Thread" => "Thread M40×1.5".to_string(),
                n => n.to_string(),
            })
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
        // Set up once before the passes, tried after them (LIO-415, LIO-413).
        assert_eq!(of(5)[..3], ["Touch", "Set up the thread", "Thread pass"]);
        assert_eq!((of(5).len(), of(5).last().copied()), (11, Some("Try")));
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
                cross: true,
                measure: true,
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
        // The shoulder at −65 is 5 from the jaws, 70 out of them.
        let setup = &doc.setups[0];
        assert_eq!(num(&setup.p, "stickout"), Some(70.));
        for l in lathe_steps(&guide) {
            if let LatheStep::Pass { op: 1, to_jaws, .. } = l {
                close(*to_jaws, 5.);
            }
        }
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
                // Nothing on the outside part winds out: each reads what
                // it is wound.
                assert!(!d.out, "{d:?}");
                assert_eq!(d.reads, d.reading.as_ref().ok().map(|r| r.1), "{d:?}");
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
        // No turning tool: the Zero step uses the first Turn's tool, the
        // boring bar, which touches the end and neither touches nor measures
        // the bar's outside, as nothing reads it (LIO-203; it was measured).
        let of = |g: &Guide, i: usize| kinds(g, g.sections[i].steps.clone());
        assert_eq!(of(&guide, 0), ["Hold", "Zero"]);
        let bar_tool = doc.setups[0].operations[1].tool.clone();
        assert!(matches!(
            &guide.steps[1],
            Step::Lathe(LatheStep::Zero { tool, cross: false, measure: false, planned, .. })
                if *tool == bar_tool && (*planned - 50.2).abs() < 1e-4
        ));
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
            assert!(cross.out && !carriage.out, "{cross:?}");
            reads(carriage, 1, 0.);
        }
        // Wound out from the zero on the Ø10 hole the collar counts down:
        // pass 5 is 9.80 out on a 4-a-turn diameter dial, 2 turns and 1.80
        // more, so the collar reads 4 − 1.80 = 2.20.
        close(bore[4].1.distance, 9.8);
        reads(bore[4].1, 2, 1.8);
        close(bore[4].1.reads.unwrap(), 2.2);
        assert_eq!(bore[4].1.text(), "2 turns out, reads 2.20");
        // Pass 1: 1.96 out reads 2.04; the last, 10.00 out, reads 2.00.
        assert_eq!(bore[0].1.text(), "0 turns out, reads 2.04");
        reads(bore[5].1, 2, 2.);
        close(bore[5].1.reads.unwrap(), 2.);
        // Measured Ø19.88 after 9.80: the 0.12 left on the last pass.
        let id = doc.setups[0].operations[1].id.clone();
        set_measured(&mut doc, &id, 19.88);
        let guide = build(&doc, &mesh, 0).unwrap();
        let last = passes(&guide, 1)[5].1;
        close(last.distance, 9.92);
        reads(last, 2, 1.92);
        assert_eq!(last.text(), "2 turns out, reads 2.08");
        assert!(
            lathe_steps(&guide)
                .iter()
                .any(|l| matches!(l, LatheStep::Pass { bore: true, .. }))
        );
        // An inside thread feeds the top slide out, away from the axis: its
        // collar counts down too.
        doc.measured.clear();
        doc.setups[0].operations[1]
            .p
            .insert("diameter".into(), "20.4".into());
        add(
            &mut doc,
            "Thread",
            &[
                ("thread_side", "Inside"),
                ("diameter", "22"),
                ("pitch", "1.5"),
                ("z_end", "-15"),
            ],
        );
        let guide = build(&doc, &mesh, 0).unwrap();
        let tops: Vec<&Dial> = (lathe_steps(&guide).into_iter())
            .filter_map(|l| match l {
                LatheStep::ThreadPass { top, carriage, .. } => {
                    assert!(!carriage.out);
                    Some(top)
                }
                _ => None,
            })
            .collect();
        assert!(!tops.is_empty());
        let machine = &doc.setups[0].machine;
        for top in tops {
            assert!(top.out, "{top:?}");
            let (_, r) = top.reading.clone().unwrap();
            assert_eq!(top.reads, lathe::reads(machine, Slide::Top, r, true));
            close(top.reads.unwrap(), (2. - r) % 2.);
        }
    }
    #[test]
    fn the_zero_is_made_with_the_turning_tool_even_when_a_bore_comes_first() {
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
        add(&mut doc, "Bore", &[("diameter", "20"), ("z_end", "-20")]);
        add(&mut doc, "Outside", &[("diameter", "40"), ("z_end", "-30")]);
        let guide = build(&doc, &mesh, 0).unwrap();
        let turning = doc.setups[0].operations[2].tool.clone();
        // The turning tool touches the end and the bar's outside.
        assert!(matches!(
            &guide.steps[1],
            Step::Lathe(LatheStep::Zero { tool, cross: true, .. }) if *tool == turning
        ));
        // The boring bar is a new tool: it touches the end again, and the
        // hole for its cross dial.
        assert!(matches!(
            &guide.steps[guide.sections[2].steps.start],
            Step::Lathe(LatheStep::Touch {
                carriage: true,
                cross: Some(Contact { inside: true, .. }),
                ..
            })
        ));
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
    fn a_tapping_reads_its_depth_and_lead_on_the_quill_and_its_turns() {
        use crate::turn::tests::{add_tap, m12_job, m12_lead};
        let (mut doc, mesh, tap) = m12_job();
        doc.setups[0]
            .machine
            .p
            .insert("cross_reads".into(), "Diameter".into());
        add(&mut doc, "Drill", &[("depth", "16")]);
        add_tap(&mut doc, &tap, "10");
        let guide = build(&doc, &mesh, 0).unwrap();
        let of = |i: usize| kinds(&guide, guide.sections[i].steps.clone());
        // No turning tool, so no Zero; the tap is its own section.
        assert_eq!(of(0), ["Hold"]);
        // The tap ends tried with a screw (LIO-413).
        assert_eq!(
            [of(1), of(2), of(3)],
            [vec!["Drill"], vec!["Tap", "Try"], vec!["HandOver"]]
        );
        assert_eq!(guide.sections[2].count, 1);
        assert!(guide.sections[2].minutes > 1.);
        let Some(LatheStep::TailTap {
            tap: name,
            pitch,
            depth,
            lead,
            turns,
            quill,
            ..
        }) = lathe_steps(&guide)
            .into_iter()
            .find(|l| matches!(l, LatheStep::TailTap { .. }))
        else {
            panic!()
        };
        // 10 of thread and the tap's lead from the hole's mouth, on the
        // 2.5-a-turn quill, and that much of the 1.75 pitch in turns of
        // the tap.
        let want = 10. + m12_lead(&doc, &tap);
        assert_eq!(name, "M12×1.75");
        close(*pitch, 1.75);
        close(*depth, 10.);
        close(*lead, want - 10.);
        // Rounded down: not a turn past the quill's depth.
        assert_eq!(*turns, (want / 1.75 * 10. + 1e-3).floor() / 10.);
        assert!(*turns * 1.75 <= want + 1e-3);
        close(quill.distance, want);
        let (t, r) = lathe::dial(&doc.setups[0].machine, Slide::Quill, want).unwrap();
        reads(quill, t, r);
        assert_eq!(quill.slide, Slide::Quill);
        let plan = &guide.done[1].plan;
        let first = plan
            .moves
            .iter()
            .find(|m| m.motion == Motion::Feed)
            .unwrap();
        close(
            quill.distance,
            mm(first.from[2] - crate::turn::CLEAR - plan.target),
        );
        assert!(dials(&guide).contains(&quill));
    }
    /// A Ø20 bar with a Ø16 Outside turn to -30 and an M16 × 2 Thread to -20
    /// cut with the job's die (M16 × 2, outside diameter 20).
    pub fn die_sample() -> (Document, Mesh) {
        let (mut doc, mesh) = lathe_job(20., 100., tube(0., 8., 0., 50.), -60.);
        doc.setups[0]
            .machine
            .p
            .insert("cross_reads".into(), "Diameter".into());
        add(
            &mut doc,
            "Outside",
            &[("diameter", "16"), ("z_end", "-30"), ("finish", "0.2")],
        );
        add(
            &mut doc,
            "Thread",
            &[("diameter", "16"), ("pitch", "2"), ("z_end", "-20")],
        );
        let die = doc.tools.iter_mut().find(|t| get(&t.p, "type") == "Die");
        let die = die.unwrap();
        die.p.insert("thread".into(), "M16×2".into());
        let id = die.id.clone();
        doc.setups[0].operations[1].tool = id;
        (doc, mesh)
    }
    #[test]
    fn a_die_is_one_screen_of_turns_not_thread_passes_and_touches_nothing() {
        let (doc, mesh) = die_sample();
        let guide = build(&doc, &mesh, 0).unwrap();
        let of = |i: usize| kinds(&guide, guide.sections[i].steps.clone());
        // The Zero is the turning tool's; the die has no Touch, no Thread pass.
        assert_eq!(of(0), ["Hold", "Zero"]);
        // Then tried with the nut (LIO-413).
        assert_eq!(of(2), ["Die", "Try"]);
        assert!(
            !lathe_steps(&guide)
                .iter()
                .any(|l| matches!(l, LatheStep::ThreadPass { .. }))
        );
        let turning = doc.setups[0].operations[0].tool.clone();
        assert!(matches!(
            &guide.steps[1],
            Step::Lathe(LatheStep::Zero { tool, .. }) if *tool == turning
        ));
        assert_eq!(guide.sections[2].count, 1);
        let Some(LatheStep::DieThread {
            die,
            pitch,
            diameter,
            od,
            length,
            turns,
            ..
        }) = lathe_steps(&guide)
            .into_iter()
            .find(|l| matches!(l, LatheStep::DieThread { .. }))
        else {
            panic!()
        };
        // From the end to Z -20 at a 2 mm pitch: 10 turns of the holder.
        assert_eq!(die, "M16×2");
        close(*pitch, 2.);
        close(*diameter, 16.);
        close(*od, 20.);
        close(*length, 20.);
        assert_eq!(*turns, 10.);
        // The same thread with the threading tool has its passes.
        let mut tool = doc.clone();
        let threading = tool
            .tools
            .iter()
            .find(|t| get(&t.p, "type") == "Threading tool")
            .unwrap()
            .id
            .clone();
        tool.setups[0].operations[1].tool = threading;
        let guide = build(&tool, &mesh, 0).unwrap();
        assert!(
            lathe_steps(&guide)
                .iter()
                .any(|l| matches!(l, LatheStep::ThreadPass { .. }))
        );
        assert!(
            !lathe_steps(&guide)
                .iter()
                .any(|l| matches!(l, LatheStep::DieThread { .. }))
        );
    }
    #[test]
    fn a_die_alone_has_no_zero() {
        let (mut doc, mesh) = die_sample();
        doc.setups[0].operations.remove(0);
        // The bar is already Ø20, the die's thread Ø16 is not ≥ it: a Ø16 bar.
        doc.setups[0].p.insert("stock_diameter".into(), "16".into());
        let guide = build(&doc, &mesh, 0).unwrap();
        assert_eq!(kinds(&guide, guide.sections[0].steps.clone()), ["Hold"]);
        assert_eq!(
            kinds(&guide, guide.sections[1].steps.clone()),
            ["Die", "Try"]
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
        let n = threads.len();
        for (k, ((infeed, angle, top, carriage), m)) in threads.iter().zip(&feeds).enumerate() {
            assert_eq!(*angle, 29.5);
            if k + 1 < n {
                // The infeed is the plan's, in from the touch on the Ø40
                // blank.
                close(*infeed, mm(20. - m.to[0]));
                close(top.distance, mm(top_travel(*infeed, 29.5)));
            }
            // The half-nuts stay closed to the thread's end at −40: 39 from
            // the faced end.
            close(carriage.distance, 39.);
            reads(carriage, 1, 19.);
        }
        // Pass 3: 0.560 in, 0.643 along the slide, 0.64 on its dial of
        // 0.02s.
        close(threads[2].0, 0.56);
        reads(threads[2].2, 0, 0.64);
        // The last: 0.915 in is 1.0513 along the slide. Rounded to the
        // nearest 0.02 it would be 1.06, 0.007 deeper than the thread; it is
        // set at 1.04, 0.905 in, and the nut and a spring pass finish it.
        close(threads[7].2.distance, 1.04);
        reads(threads[7].2, 0, 1.04);
        close(threads[7].0, 0.905);
        assert!(threads[7].0 <= 0.915 + 1e-6);
        // The readings never go back from one pass to the next.
        let set: Vec<f32> = threads
            .iter()
            .map(|t| t.2.reading.clone().unwrap().1)
            .collect();
        assert!(set.windows(2).all(|w| w[1] >= w[0] - 1e-6), "{set:?}");
        assert_eq!(
            set.iter().map(|r| format!("{r:.2}")).collect::<Vec<_>>(),
            [
                "0.38", "0.52", "0.64", "0.74", "0.84", "0.92", "0.98", "1.04"
            ]
        );
    }
    #[test]
    fn a_thread_never_goes_past_its_last_reading() {
        // In 30 passes the last few are less than a division apart: none is
        // set past the last, which is set below its travel, and the
        // readings never go back.
        let (mut doc, mesh) = sample("Diameter");
        doc.setups[0].operations[4]
            .p
            .insert("passes".into(), "30".into());
        let guide = build(&doc, &mesh, 0).unwrap();
        let set: Vec<f32> = (lathe_steps(&guide).into_iter())
            .filter_map(|l| match l {
                LatheStep::ThreadPass { top, .. } => Some(top.reading.clone().unwrap().1),
                _ => None,
            })
            .collect();
        assert_eq!(set.len(), 30);
        close(*set.last().unwrap(), 1.04);
        assert!(set.iter().all(|r| *r <= 1.04 + 1e-6), "{set:?}");
        assert!(set.windows(2).all(|w| w[1] >= w[0] - 1e-6), "{set:?}");
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
        // The top slide, along the bed, feeds the 45° edge the leg: 1.00.
        assert_eq!(top.slide, Slide::Top);
        close(top.distance, 1.);
        reads(top, 0, 1.);
        // The shoulder at −65 is 64 from the faced end: the point stops 1
        // short of it, at 63, for the top slide to feed it the last 1.
        close(carriage.distance, 63.);
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
    fn after_a_thread_the_top_slide_is_set_back_along_the_bed() {
        let (doc, mesh) = sample("Diameter");
        let guide = build(&doc, &mesh, 0).unwrap();
        // The Face comes first: the top slide is along the bed already.
        assert!(matches!(
            guide.steps[2],
            Step::Lathe(LatheStep::Face { swung: None, .. })
        ));
        // A chamfer and a face after the thread set it back from 29.5°;
        // the face after the chamfer finds it along the bed.
        let (mut doc, mesh) = sample("Diameter");
        doc.setups[0].operations.truncate(5);
        add(
            &mut doc,
            "Chamfer",
            &[("edge", "End outside"), ("size", "0.5")],
        );
        add(
            &mut doc,
            "Face",
            &[("face_off", "0.5"), ("to_diameter", "36")],
        );
        let guide = build(&doc, &mesh, 0).unwrap();
        let swung: Vec<Option<f32>> = (lathe_steps(&guide).into_iter())
            .filter_map(|l| match l {
                LatheStep::Face { swung, .. } | LatheStep::Chamfer { swung, .. } => Some(*swung),
                _ => None,
            })
            .collect();
        assert_eq!(swung, [None, Some(29.5), None]);
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
