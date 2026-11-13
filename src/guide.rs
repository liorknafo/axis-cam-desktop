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
/// The second touch of an axis the Zero step halves (LIO-451): where the
/// edge finder touches the + face (work coordinates, `contact`), whether
/// what it touches is round there, and what the DRO reads there after
/// the − face was set to 0: the width between the touches plus the
/// finder's diameter, a check on the blank, not a number to type.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Far {
    pub at: [f32; 3],
    pub round: bool,
    pub reads: f32,
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
    /// diameter `finder` and the DRO's compensation off. An axis whose zero
    /// is on a face touches that face (`approach`: −1 the − face from
    /// outside it, +1 the + face) and is set there to `touch`, the face
    /// less or plus the finder's radius. An axis whose zero is at the
    /// middle is halved (LIO-451): it touches its − face (`approach` −1) and
    /// is set to 0 there, then touches its + face (`far`), where the DRO
    /// reads about the stock's width plus the finder's diameter, and is set
    /// there to `touch`, half of that reading, which puts 0 midway whatever
    /// the blank's real size. Z touches the top, set to `touch[2]`. `at` is
    /// where each of X and Y is first touched (work coordinates,
    /// `contact`), which `touch` is set from: on the box's face at the
    /// top's height when the face reaches it, lower where earlier setups
    /// cut the top of the face back, and on the outermost material above
    /// the holding's jaws when that face is only in them; `round` whether
    /// what it touches is round there (turned), so that the touch is true
    /// only on its centre line, where it stands out farthest.
    Zero {
        corner: [f32; 3],
        approach: [i8; 2],
        touch: [f32; 3],
        finder: f32,
        at: [[f32; 3]; 2],
        round: [bool; 2],
        far: [Option<Far>; 2],
    },
    /// Set the gear to `rpm` for the tool `tool` (an id). With `top`, a new
    /// tool: put it in, touch the stock's top with it and set Z to `top`
    /// (the top before the operation, work Z); without, the tool is in
    /// already and only its gear changes. With `comp`, enter `diameter` in
    /// the DRO and set X and Y to `comp` (0 off: centres).
    Tool {
        op: usize,
        tool: String,
        rpm: f32,
        diameter: f32,
        comp: Option<[i8; 2]>,
        top: Option<f32>,
    },
    /// A hand tap `tool` (an id) for the tapping `op`: put it in, the
    /// spindle off and free to turn; no gear (LIO-410). With `top`, Z is
    /// set on the stock's top with the tap's tip, as for a new tool, so
    /// that each hole's lift to a safe Z is a DRO reading (LIO-449).
    TapIn {
        op: usize,
        tool: String,
        top: Option<f32>,
    },
    /// Same tool and gear: enter its `diameter` in the DRO and set the
    /// compensation sides `comp`, not set before or changed.
    Comp {
        op: usize,
        comp: [i8; 2],
        diameter: f32,
    },
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
    /// taking off `remove`; the user plans the passes. A new tool picks up Z
    /// on the top first, set to `top`, as on a Tool step.
    Face {
        op: usize,
        tool: String,
        rpm: f32,
        diameter: f32,
        top: Option<f32>,
        remove: f32,
        to: f32,
    },
    /// A squaring Box (one closed X or Y side, the wall, and a slab off it
    /// one row wide): cut the stock's `side` (−1 or +1) of `axis` to `to`
    /// (the wall as cut, with its allowance), which the DRO reads as `shown`
    /// (the edge with compensation, else the centre), taking off `remove`,
    /// down to `down_to` (the floor as cut). With the DRO's compensation on,
    /// enter `diameter` and set `axis` to its side in `comp` (toward the
    /// wall; the other axis 0); all 0 with it off. A new tool picks up Z on
    /// the top first, set to `top`, as on a Tool step.
    Square {
        op: usize,
        tool: String,
        rpm: f32,
        diameter: f32,
        comp: [i8; 2],
        top: Option<f32>,
        axis: usize,
        side: i8,
        to: f32,
        shown: f32,
        remove: f32,
        down_to: f32,
    },
    /// The holes of a Drill in the order drilled: X and Y (centres) and the
    /// Z each is drilled to; and, by hole, the Z its cut starts at: its
    /// mouth, where a tap's turns are counted from. A centre drill's Z is
    /// its pilot's tip. By hole, whether a twist drill starts it on a flat
    /// face, with nothing before it to centre it (`uncentred`). With a
    /// `pattern` the DRO leads to (LIO-399), each is reached when X and Y
    /// read 0, and `numbers` are the DRO's hole numbers in the order
    /// drilled: an arc's mirrored holes are taken one axis a move, out of
    /// the DRO's order round the circle (LIO-406). Without, 1, 2, 3…
    Holes {
        op: usize,
        holes: Vec<[f32; 3]>,
        mouths: Vec<f32>,
        uncentred: Vec<bool>,
        pattern: Option<Pattern>,
        numbers: Vec<usize>,
    },
    /// What to type into the DRO for the holes of `op`, which are all one
    /// `pattern`; `same` when the operation before used the same one (a
    /// centre drill before its drill): start it again at hole 1.
    Pattern {
        op: usize,
        pattern: Pattern,
        same: bool,
    },
    /// One pass of a Boring (1-based `index` of `of`): the boring head set
    /// to `size` (Ø mm), from `before` (what the last pass left, or None
    /// before the first: set it to `size`), turning out `dial` mm on the
    /// diameter a division; then each hole of `holes` (X and Y centres and
    /// the Z it is bored to; `mouths` the Z its cut starts at) bored and
    /// left with the spindle stopped, or fed back out when `feed_out`.
    Bore {
        op: usize,
        index: usize,
        of: usize,
        size: f32,
        before: Option<f32>,
        dial: f32,
        feed_out: bool,
        holes: Vec<[f32; 3]>,
        mouths: Vec<f32>,
    },
    /// One walk of a Round corners (1-based `index` of `of`), round corner
    /// `corner` of `corners` (1-based), at `place` on the part, of radius
    /// `radius` about `centre` (work X, Y), `offset` outside the round (0:
    /// on it, the last walk there), at work Z `z`, level `level` of
    /// `levels`. `targets` are its DRO numbers in order, tool centres (the
    /// DRO's compensation off): the way to the start clear of the stock,
    /// the backlash taken up, down, in, the steps, out and up; `points` the
    /// walk's readings from the first on the round's start to its last; it
    /// leaves at most `scallop` proud of the round (or of the line `offset`
    /// out).
    Corner {
        op: usize,
        index: usize,
        of: usize,
        corner: usize,
        corners: usize,
        place: &'static str,
        radius: f32,
        centre: [f32; 2],
        offset: f32,
        z: f32,
        level: usize,
        levels: usize,
        scallop: f32,
        points: Vec<[f32; 2]>,
        targets: Vec<Target>,
        never: Limits,
        turns_per_minute: f32,
    },
    /// A full-width capsule pass, the exact plan's one-axis centre readings.
    Slot {
        op: usize,
        index: usize,
        of: usize,
        z: f32,
        top: f32,
        a: [f32; 2],
        b: [f32; 2],
        targets: Vec<Target>,
        entry_clear: bool,
    },
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
/// The Z, from `zero`, each hole's cut starts at, in `holes`' order: the
/// top of its first cutting move.
fn mouths(plan: &Plan, zero: [f32; 3]) -> Vec<f32> {
    mouths_of(&plan.moves, zero)
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
/// `turns` to a tenth, rounded down, a hair over a whole tenth kept.
fn tenths_down(turns: f32) -> f32 {
    (turns * 10. + 1e-3).floor() / 10.
}
/// Whether the mouth of the hole Ø`d` at `at` (stock frame), drilled from
/// as high as `top` down to `bottom`, is opened in `before` so that a twist
/// drill finds its centre: its axis is open, or the material on it starts
/// lower than all round the hole's rim (a centre drill's or a countersink's
/// cone, a pilot hole). A flat face, a flat-bottomed pocket the hole starts
/// in, a slope, a step beside it, or a rim with no material somewhere (the
/// stock's edge) is not. Exact: the cuts' sweeps, every 0.02 mm down.
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
    // The rim's lowest material: a cone round the axis keeps all of it
    // higher than the axis; a step or a slope keeps some no higher.
    let mut rim = f32::INFINITY;
    for k in 0..8 {
        let a = k as f32 * std::f32::consts::TAU / 8.;
        match first(at[0] + r * a.cos(), at[1] + r * a.sin()) {
            Some(z) => rim = rim.min(z),
            None => return false,
        }
    }
    axis < rim - STEP
}
fn holes(plan: &Plan, zero: [f32; 3]) -> Vec<[f32; 3]> {
    holes_of(&plan.moves, zero)
}
/// Within this, mm, two holes share an X or a Y: the move between them is
/// along one axis.
const SAME_AXIS: f32 = 0.005;
/// Whether the move between `a` and `b` is along one axis.
pub(crate) fn one_axis(a: [f32; 2], b: [f32; 2]) -> bool {
    (0..2).any(|k| (a[k] - b[k]).abs() <= SAME_AXIS)
}
/// The order to drill an arc's holes `at` (centres in the DRO's order, round
/// the circle), as indices of `at`: from hole 1, next the hole left that
/// shares this one's Y (a move along X), else its X, the nearer of two,
/// else the nearest left. Used only when it has more one-axis moves than
/// going round (LIO-406).
// ponytail: the DRO is taken to count counter-clockwise from its first
// hole; if the owner's counts the other way the numbers mirror, and every
// hole is still drilled once.
fn drill_order(at: &[[f32; 2]]) -> Vec<usize> {
    let moves = |o: &[usize]| {
        (o.windows(2))
            .filter(|w| one_axis(at[w[0]], at[w[1]]))
            .count()
    };
    let round: Vec<usize> = (0..at.len()).collect();
    let mut order = vec![0];
    let mut left: Vec<usize> = (1..at.len()).collect();
    while let Some(&here) = order.last()
        && !left.is_empty()
    {
        let p = at[here];
        let near = |j: usize| (at[j][0] - p[0]).hypot(at[j][1] - p[1]);
        let rank = |j: usize| match (at[j][1] - p[1]).abs() <= SAME_AXIS {
            true => 0,
            false if (at[j][0] - p[0]).abs() <= SAME_AXIS => 1,
            false => 2,
        };
        let pick = (0..left.len())
            .min_by(|&a, &b| {
                let (a, b) = (left[a], left[b]);
                rank(a).cmp(&rank(b)).then(near(a).total_cmp(&near(b)))
            })
            .unwrap_or(0);
        order.push(left.remove(pick));
    }
    if moves(&order) > moves(&round) {
        order
    } else {
        round
    }
}
/// `v` in `order`.
fn reordered<T: Clone>(v: &[T], order: &[usize]) -> Vec<T> {
    order.iter().filter_map(|&j| v.get(j).cloned()).collect()
}
/// A pattern of holes a DRO leads to by itself (LIO-399): its matrix holes
/// (a grid) or its arc holes. Work coordinates; mm, and degrees from +X
/// counter-clockwise; rounded to 0.01.
#[derive(Debug, Clone, PartialEq)]
pub enum Pattern {
    /// `columns` holes a row and `rows` rows: `first` the first hole,
    /// `pitch` along a row and between the rows, the rows at `angle`. The
    /// order: row by row from the first hole, back and forth, each row the
    /// opposite way from the one before, so that square to the axes every
    /// move to the next hole is along one axis (LIO-405).
    Grid {
        first: [f32; 2],
        columns: usize,
        rows: usize,
        pitch: [f32; 2],
        angle: f32,
    },
    /// `count` holes on the circle Ø`diameter` round `centre`, the first at
    /// angle `first`, each one `step` on counter-clockwise; `full` all the
    /// way round. The DRO's order: from the first, counter-clockwise.
    Arc {
        centre: [f32; 2],
        diameter: f32,
        count: usize,
        first: f32,
        step: f32,
        full: bool,
    },
}
/// How far a hole may be off its place in a pattern, mm: the centres come
/// from the part's mesh.
const ON_PATTERN: f32 = 0.02;
fn hundredths(v: f32) -> f32 {
    (v * 100.).round() / 100. + 0.
}
/// How far a hole may be from where the numbers as shown put it, mm.
const SHOWN_OFF: f32 = 0.05;
/// The pattern of `holes` (centres and depths, work coordinates) the DRO
/// leads to (`pattern`), when all of them go to the same Z to 0.01.
// ponytail: the DRO's order of the holes is not known (row by row each the
// same way, or back and forth), so holes of unequal depth go one by one,
// lest the DRO's hole N get another hole's depth (LIO-403). Lift it when the
// owner's DRO order is known.
fn level_pattern(holes: &[[f32; 3]], grids: bool, arcs: bool) -> Option<(Pattern, Vec<usize>)> {
    let z = hundredths(holes.first()?[2]);
    if holes.iter().any(|h| hundredths(h[2]) != z) {
        return None;
    }
    let xy: Vec<[f32; 2]> = holes.iter().map(|h| [h[0], h[1]]).collect();
    pattern(&xy, grids, arcs)
}
/// Where the DRO puts each hole of `p`, in its order, from its numbers as
/// shown: each hole a pitch, or an angle between holes, on from the last.
fn places(p: &Pattern) -> Vec<[f32; 2]> {
    match *p {
        Pattern::Grid {
            first,
            columns,
            rows,
            pitch,
            angle,
        } => {
            let (s, c) = angle.to_radians().sin_cos();
            (0..rows)
                .flat_map(|r| {
                    (0..columns).map(move |k| {
                        let k = if r % 2 == 1 { columns - 1 - k } else { k };
                        let (u, v) = (k as f32 * pitch[0], r as f32 * pitch[1]);
                        [first[0] + u * c - v * s, first[1] + u * s + v * c]
                    })
                })
                .collect()
        }
        Pattern::Arc {
            centre,
            diameter,
            count,
            first,
            step,
            ..
        } => (0..count)
            .map(|j| {
                let t = (first + j as f32 * step).to_radians();
                [
                    centre[0] + diameter / 2. * t.cos(),
                    centre[1] + diameter / 2. * t.sin(),
                ]
            })
            .collect(),
    }
}
/// The one pattern the hole centres `holes` (work coordinates) make all
/// together, of those the DRO has (`grids`, `arcs`), with the holes'
/// indices in the DRO's order; None for fewer than three holes, or when the
/// numbers as shown put a hole more than 0.05 mm off (LIO-402). Four holes
/// on a square are a grid and a circle: the grid when square to the axes,
/// else the circle, a bolt circle of four (LIO-402).
pub fn pattern(holes: &[[f32; 2]], grids: bool, arcs: bool) -> Option<(Pattern, Vec<usize>)> {
    if holes.len() < 3 {
        return None;
    }
    let shown = |found: Option<(Pattern, Vec<usize>)>| {
        found.filter(|(p, order)| {
            (places(p).iter().zip(order))
                .all(|(q, &h)| (q[0] - holes[h][0]).hypot(q[1] - holes[h][1]) <= SHOWN_OFF)
        })
    };
    let as_grid = if grids { shown(grid(holes)) } else { None };
    let as_arc = if arcs { shown(arc(holes)) } else { None };
    match (as_grid, as_arc) {
        (
            Some((Pattern::Grid { angle, .. }, _)),
            Some(round @ (Pattern::Arc { full: true, .. }, _)),
        ) if angle != 0. => Some(round),
        (Some(found), _) => Some(found),
        (None, found) => found,
    }
}
/// `holes` as one grid: its rows along the direction of a pair of holes,
/// the longest pairs first, as their direction is the truest.
// ponytail: every pair's direction is tried, cubic in the holes; a Drill
// has tens. Bin the directions first if hundreds of holes get slow.
fn grid(holes: &[[f32; 2]]) -> Option<(Pattern, Vec<usize>)> {
    let n = holes.len();
    let d = |&(a, b): &(usize, usize)| (holes[b][0] - holes[a][0]).hypot(holes[b][1] - holes[a][1]);
    let mut pairs: Vec<(usize, usize)> = (0..n)
        .flat_map(|a| (a + 1..n).map(move |b| (a, b)))
        .collect();
    pairs.sort_by(|x, y| d(y).total_cmp(&d(x)));
    let mut tried: Vec<f32> = vec![];
    pairs.into_iter().find_map(|(a, b)| {
        let (dx, dy) = (holes[b][0] - holes[a][0], holes[b][1] - holes[a][1]);
        let turn = dy.atan2(dx).rem_euclid(std::f32::consts::PI);
        let near = |t: &f32| {
            let off = (t - turn).abs();
            off.min(std::f32::consts::PI - off) < 1e-3
        };
        if tried.iter().any(near) {
            return None;
        }
        tried.push(turn);
        grid_along(holes, [turn.cos(), turn.sin()])
    })
}
/// The ranks of `values` with those within 0.2 mm of the one before as one,
/// and how many ranks.
fn ranks(values: &[f32]) -> (Vec<usize>, usize) {
    let mut by: Vec<usize> = (0..values.len()).collect();
    by.sort_by(|a, b| values[*a].total_cmp(&values[*b]));
    let mut rank = vec![0; values.len()];
    let mut r = 0;
    for w in 1..by.len() {
        if values[by[w]] - values[by[w - 1]] > 10. * ON_PATTERN {
            r += 1;
        }
        rank[by[w]] = r;
    }
    (rank, r + 1)
}
/// `holes` as a grid with its rows roughly along the unit vector `u`.
fn grid_along(holes: &[[f32; 2]], u: [f32; 2]) -> Option<(Pattern, Vec<usize>)> {
    let n = holes.len();
    let dot = |p: [f32; 2], q: [f32; 2]| p[0] * q[0] + p[1] * q[1];
    let along: Vec<f32> = holes.iter().map(|p| dot(*p, u)).collect();
    let across: Vec<f32> = holes.iter().map(|p| dot(*p, [-u[1], u[0]])).collect();
    let (mut i, mut c) = ranks(&along);
    let (mut k, mut r) = ranks(&across);
    if c * r != n {
        return None;
    }
    let mut seen = vec![false; n];
    for h in 0..n {
        let at = k[h] * c + i[h];
        if std::mem::replace(&mut seen[at], true) {
            return None;
        }
    }
    // A full grid: the least-squares pitch on each index alone.
    let fit = |idx: &[usize]| -> [f32; 2] {
        let m = idx.iter().sum::<usize>() as f32 / n as f32;
        let ss: f32 = idx.iter().map(|&j| (j as f32 - m).powi(2)).sum();
        if ss == 0. {
            return [0.; 2];
        }
        std::array::from_fn(|a| {
            let pm = holes.iter().map(|p| p[a]).sum::<f32>() / n as f32;
            (idx.iter().zip(holes))
                .map(|(&j, p)| (j as f32 - m) * (p[a] - pm))
                .sum::<f32>()
                / ss
        })
    };
    let (mut pa, mut pb) = (fit(&i), fit(&k));
    // Rows along the pitch nearer X; a column alone is a row.
    let len = |v: [f32; 2]| v[0].hypot(v[1]);
    if c == 1 || (r > 1 && pb[0].abs() / len(pb) > pa[0].abs() / len(pa)) {
        (i, k, c, r, pa, pb) = (k, i, r, c, pb, pa);
    }
    // The rows run toward +X, or +Y when upright within the 0.1° a mesh's
    // centres can turn them (their angle over −89.9° and up to 90.1°); the
    // rows after the first go counter-clockwise of them, square to them.
    let turned = pa[1].atan2(pa[0]).to_degrees();
    if !(-89.9..=90.1).contains(&turned) {
        pa = pa.map(|v| -v);
        i.iter_mut().for_each(|j| *j = c - 1 - *j);
    }
    let ua = pa.map(|v| v / len(pa));
    let ub = [-ua[1], ua[0]];
    let mut across = dot(pb, ub);
    if across < 0. {
        across = -across;
        k.iter_mut().for_each(|j| *j = r - 1 - *j);
    }
    let pb = ub.map(|v| v * across);
    let mean = |idx: &[usize]| idx.iter().sum::<usize>() as f32 / n as f32;
    let (mi, mk) = (mean(&i), mean(&k));
    let first: [f32; 2] = std::array::from_fn(|a| {
        holes.iter().map(|p| p[a]).sum::<f32>() / n as f32 - mi * pa[a] - mk * pb[a]
    });
    let off = |h: usize| {
        let at: [f32; 2] =
            std::array::from_fn(|a| first[a] + i[h] as f32 * pa[a] + k[h] as f32 * pb[a]);
        (at[0] - holes[h][0]).hypot(at[1] - holes[h][1])
    };
    if (0..n).any(|h| off(h) > ON_PATTERN) {
        return None;
    }
    // Back and forth: the odd rows the other way (LIO-405).
    // ponytail: no setting for the DRO's order; add one if the owner's DRO
    // goes each row from the same end.
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by_key(|&h| (k[h], if k[h] % 2 == 1 { c - 1 - i[h] } else { i[h] }));
    // Square to the axes within 0.02° reads 0 (LIO-402).
    let turned = pa[1].atan2(pa[0]).to_degrees();
    let angle = if turned.abs() <= 0.02 {
        0.
    } else {
        hundredths(turned)
    };
    // A lone line along Y is one hole a row, the rows up it: a DRO whose
    // matrix holes take no angle does it so (LIO-402).
    let pattern = if r == 1 && (turned - 90.).abs() <= 0.02 {
        Pattern::Grid {
            first: first.map(hundredths),
            columns: 1,
            rows: c,
            pitch: [0., hundredths(len(pa))],
            angle: 0.,
        }
    } else {
        Pattern::Grid {
            first: first.map(hundredths),
            columns: c,
            rows: r,
            pitch: [len(pa), across].map(hundredths),
            angle,
        }
    };
    Some((pattern, order))
}
/// `holes` as one arc: on the circle that fits them best, at even angles,
/// all the way round or along part of it (at least four holes, as any three
/// lie on some circle (LIO-402), and 10° from the first to the last, not a
/// line).
fn arc(holes: &[[f32; 2]]) -> Option<(Pattern, Vec<usize>)> {
    let n = holes.len();
    // The algebraic (Kåsa) fit, about the holes' middle.
    let m: [f64; 2] =
        std::array::from_fn(|a| holes.iter().map(|p| f64::from(p[a])).sum::<f64>() / n as f64);
    let (mut uu, mut uv, mut vv, mut r1, mut r2) = (0., 0., 0., 0., 0.);
    for p in holes {
        let (u, v) = (f64::from(p[0]) - m[0], f64::from(p[1]) - m[1]);
        (uu, uv, vv) = (uu + u * u, uv + u * v, vv + v * v);
        r1 += u * (u * u + v * v) / 2.;
        r2 += v * (u * u + v * v) / 2.;
    }
    let det = uu * vv - uv * uv;
    if det.abs() < 1e-9 * (uu * vv).max(1e-12) {
        return None;
    }
    let (cu, cv) = ((r1 * vv - r2 * uv) / det, (r2 * uu - r1 * uv) / det);
    let radius = (cu * cu + cv * cv + (uu + vv) / n as f64).sqrt() as f32;
    let centre = [(m[0] + cu) as f32, (m[1] + cv) as f32];
    // Each hole's angle, 0 to 360; one under 0 by no more than a hole may
    // be off its place is taken as 0.
    let hair = (ON_PATTERN / radius.max(ON_PATTERN)).to_degrees();
    let angle = |p: [f32; 2]| {
        let a = (p[1] - centre[1]).atan2(p[0] - centre[0]).to_degrees();
        if a < -hair { a + 360. } else { a.max(0.) }
    };
    let mut by: Vec<usize> = (0..n).collect();
    by.sort_by(|a, b| angle(holes[*a]).total_cmp(&angle(holes[*b])));
    let gap = |j: usize| {
        let next = angle(holes[by[(j + 1) % n]]);
        (next - angle(holes[by[j]])).rem_euclid(360.)
    };
    let widest = (0..n).max_by(|a, b| gap(*a).total_cmp(&gap(*b)))?;
    // All the way round, from the hole at the least angle; else from the
    // hole after the widest gap.
    let tries = [
        (true, 0, 360. / n as f32),
        (
            false,
            (widest + 1) % n,
            (360. - gap(widest)) / (n - 1) as f32,
        ),
    ];
    tries.into_iter().find_map(|(full, start, step)| {
        if !full && (n < 4 || step * ((n - 1) as f32) < 10.) {
            return None;
        }
        let order: Vec<usize> = (0..n).map(|j| by[(start + j) % n]).collect();
        let a0 = angle(holes[order[0]]);
        let first = (order.iter().enumerate())
            .map(|(j, &h)| (angle(holes[h]) - a0).rem_euclid(360.) + a0 - j as f32 * step)
            .sum::<f32>()
            / n as f32;
        let fits = order.iter().enumerate().all(|(j, &h)| {
            let t = (first + j as f32 * step).to_radians();
            let at = [centre[0] + radius * t.cos(), centre[1] + radius * t.sin()];
            (at[0] - holes[h][0]).hypot(at[1] - holes[h][1]) <= ON_PATTERN
        });
        fits.then(|| {
            let pattern = Pattern::Arc {
                centre: centre.map(hundredths),
                diameter: hundredths(2. * radius),
                count: n,
                first: hundredths(first).rem_euclid(360.),
                step: hundredths(step),
                full,
            };
            (pattern, order)
        })
    })
}
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
fn contact(
    before: &Volume,
    frame: crate::volume::Frame,
    stock: Bounds,
    k: usize,
    plus: bool,
    above: f32,
) -> Option<([f32; 3], bool)> {
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
    if before.cuts.is_empty() || height.is_nan() || height <= 2. * IN {
        return Some((at, false));
    }
    let rest = crate::rest::RestStock::new(before, frame);
    // Only the cuts that reach the stock, by cell.
    let removed = rest.removed_in(stock);
    let inward = if plus { -1. } else { 1. };
    let (width, depth) = (stock.max[o] - stock.min[o], stock.max[k] - stock.min[k]);
    let across = |i: usize| stock.min[o] + (i as f32 + 0.5) / ACROSS as f32 * width;
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
        let gap = 1.5 * width / ACROSS as f32;
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
    // The heights tried, top first, none in the jaws: the top, then a
    // whole number of millimetres under it.
    let pitch = (height / ROWS).ceil().max(1.);
    let low = (stock.min[2] + IN).max(above);
    let rows: Vec<f32> = std::iter::once(stock.max[2] - IN)
        .chain((1..).map(|n| stock.max[2] - n as f32 * pitch))
        .take_while(|z| *z >= low)
        .collect();
    for (n, &z) in rows.iter().enumerate() {
        let found = solid(IN, z);
        if found.is_empty() {
            continue;
        }
        let round;
        if n == 0 {
            (at[o], round) = met(&found, &|a| has(IN, a, z));
            return Some((at, round));
        }
        let lower = z - UNDER;
        let below = if lower >= low {
            solid(IN, lower)
        } else {
            vec![]
        };
        let (z, found) = if below.is_empty() {
            (z, found)
        } else {
            (lower, below)
        };
        (at[o], round) = met(&found, &|a| has(IN, a, z));
        at[2] = z;
        return Some((at, round));
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
        let round;
        (at[o], round) = met(&lines, &|a| has(nearest, a, z));
        let (mut out, mut inside) = ((nearest - STEP).max(0.), nearest);
        while inside - out > 0.005 {
            let mid = (out + inside) / 2.;
            if has(mid, at[o], z) {
                inside = mid;
            } else {
                out = mid;
            }
        }
        at[k] = edge + inward * (inside * 100.).round() / 100.;
        if n > 0 {
            at[2] = z;
        }
        return Some((at, round));
    }
    None
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
    let dro = get(&machine.p, "dro_comp") == "Yes";
    // The DRO's own hole functions (LIO-399).
    // Yes unless set to No, so a pattern needs nothing set up.
    let grids = get(&machine.p, "dro_matrix") != "No";
    let arcs = get(&machine.p, "dro_arc") != "No";
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
    // A zero at the middle is found by halving between both faces.
    let halve = found.0.map(|a| a == At::Mid);
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
    // Where axis `k` is touched from its + side when `plus`: clear above
    // the jaws, else just above them.
    let touch = |k: usize, plus: bool| {
        let near = |clear: f32| {
            let above = jaws.map_or(f32::NEG_INFINITY, |top| top + clear);
            contact(&before, frame, stock, k, plus, above)
        };
        near(FINDER_CLEAR)
            .or_else(|| near(FINDER_LEAST))
            .ok_or(NOT_CLEAR)
    };
    let mut touched = [([0.; 3], false); 2];
    let mut far: [Option<Far>; 2] = [None; 2];
    for k in 0..2 {
        touched[k] = touch(k, approach[k] > 0)?;
        if halve[k] {
            let (at, round) = touch(k, true)?;
            far[k] = Some(Far {
                at: std::array::from_fn(|i| at[i] - zero[i]),
                round,
                // Set to 0 with the finder's centre a radius outside the −
                // face, read with it a radius outside the + face.
                reads: at[k] - touched[k].0[k] + finder,
            });
        }
    }
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
            // Set to the face it touches, less or plus the finder's radius;
            // a halved axis at its + face, to where the finder's centre is
            // there: half the reading when the touches are either side of
            // the zero alike.
            touch: std::array::from_fn(|k| match (approach.get(k), far.get(k)) {
                (_, Some(Some(f))) => f.at[k] + finder / 2.,
                (Some(1), _) => touched[k].0[k] + finder / 2. - zero[k],
                (Some(_), _) => touched[k].0[k] - finder / 2. - zero[k],
                (None, _) => stock.max[2] - zero[2],
            }),
            finder,
            at: std::array::from_fn(|k| std::array::from_fn(|i| touched[k].0[i] - zero[i])),
            round: touched.map(|(_, round)| round),
            far,
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
    // The hole pattern of the operation before, and of this one.
    let mut next_pattern: Option<Pattern> = None;
    for ((oi, plan), after) in done.into_iter().zip(afters) {
        let last_pattern = next_pattern.take();
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
        // Where a new tool picks up Z: the highest top left. A tool already
        // in keeps its Z (LIO-368).
        let top = changed.then_some(left.max[2] - zero[2]);
        // The screen that sets the tool, gear and DRO for this operation:
        // all of them for a new tool; with the same tool only what changes,
        // a gear with the DRO when it changes too, else the DRO alone (its
        // diameter and sides; nothing with its compensation off), or none.
        let tap = op.tapping();
        // The DRO's compensation this tool and gear leave on, when on.
        let comp_on = matches!(set, Some((t, s, Some(c))) if t == tool && s == rpm && c != [0; 2]);
        let mut sides = |comp: [i8; 2], steps: &mut Vec<Step>| {
            // A tap is turned by hand: its own screen, no gear (LIO-410);
            // a new one picks up Z for the lifts between holes (LIO-449).
            if tap {
                steps.push(Step::TapIn {
                    op: oi,
                    tool: tool.into(),
                    top,
                });
                set = Some((tool, rpm, Some(comp)));
                return;
            }
            let tool_step = |comp: Option<[i8; 2]>| Step::Tool {
                op: oi,
                tool: tool.into(),
                rpm,
                diameter,
                comp,
                top,
            };
            match set {
                Some((t, s, c)) if t == tool => {
                    let dro = c != Some(comp) && comp != [0; 2];
                    if s != rpm {
                        steps.push(tool_step(dro.then_some(comp)));
                    } else if dro {
                        steps.push(Step::Comp {
                            op: oi,
                            comp,
                            diameter,
                        });
                    }
                }
                _ => steps.push(tool_step(Some(comp))),
            }
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
            Strategy::Drill if op.boring() => {
                sides([0; 2], &mut steps);
                // Each pass's moves: from its stop to the next pass's.
                let passes: Vec<(usize, f32)> = (plan.stops.iter())
                    .filter_map(|s| Some((s.at, s.size?)))
                    .collect();
                let head = doc.tools.iter().find(|t| t.id == op.tool);
                let dial = head.and_then(|t| num(&t.p, "dial")).unwrap_or(0.);
                let feed_out = head
                    .is_some_and(|t| get(&t.p, "bore_retract") == crate::model::BORE_RETRACTS[1]);
                count = passes.len();
                for (k, &(at, size)) in passes.iter().enumerate() {
                    let end = passes.get(k + 1).map_or(plan.moves.len(), |p| p.0);
                    let moves = &plan.moves[at..end];
                    steps.push(Step::Bore {
                        op: oi,
                        index: k + 1,
                        of: passes.len(),
                        size,
                        before: k.checked_sub(1).map(|j| passes[j].1),
                        dial,
                        feed_out,
                        holes: holes_of(moves, zero),
                        mouths: mouths_of(moves, zero),
                    });
                }
                steps.push(Step::Check { op: oi });
            }
            Strategy::Drill => {
                sides([0; 2], &mut steps);
                let mut holes = holes(&plan, zero);
                count = holes.len();
                let mouths = mouths(&plan, zero);
                let centre =
                    (doc.tools.iter().find(|t| t.id == op.tool)).and_then(|t| t.centre_drill());
                // A centre drill is set to its pilot's tip, the plan's point
                // being the pilot's top.
                if let Some(c) = centre {
                    holes.iter_mut().for_each(|h| h[2] -= c.pilot_length);
                }
                let uncentred: Vec<bool> = holes
                    .iter()
                    .map(|h| {
                        crate::centre::wants_centre(op, &doc.tools, machine)
                            && !opened(
                                earlier,
                                frame,
                                [h[0] + zero[0], h[1] + zero[1]],
                                diameter,
                                left.max[2],
                                h[2] + zero[2],
                            )
                    })
                    .collect();
                // ponytail: one pattern an operation, all its holes or none;
                // several would need the holes split into patterns first.
                let found = level_pattern(&holes, grids, arcs);
                let mut numbers: Vec<usize> = (1..=holes.len()).collect();
                let (holes, mouths, uncentred, pattern) = match found {
                    Some((pattern, round)) => {
                        steps.push(Step::Pattern {
                            op: oi,
                            pattern: pattern.clone(),
                            same: last_pattern.as_ref() == Some(&pattern),
                        });
                        // An arc's mirrored holes one axis a move (LIO-406).
                        let drilled = match pattern {
                            Pattern::Arc { .. } => {
                                let at: Vec<[f32; 2]> =
                                    round.iter().map(|&j| [holes[j][0], holes[j][1]]).collect();
                                drill_order(&at)
                            }
                            Pattern::Grid { .. } => (0..round.len()).collect(),
                        };
                        numbers = drilled.iter().map(|j| j + 1).collect();
                        let order: Vec<usize> = drilled.iter().map(|&j| round[j]).collect();
                        (
                            reordered(&holes, &order),
                            reordered(&mouths, &order),
                            reordered(&uncentred, &order),
                            Some(pattern),
                        )
                    }
                    None => (holes, mouths, uncentred, None),
                };
                next_pattern = pattern.clone();
                steps.push(Step::Holes {
                    op: oi,
                    holes,
                    mouths,
                    uncentred,
                    pattern,
                    numbers,
                });
                steps.push(Step::Check { op: oi });
            }
            Strategy::Slot => {
                sides([0; 2], &mut steps);
                if comp_on {
                    steps.push(Step::Comp {
                        op: oi,
                        comp: [0; 2],
                        diameter,
                    });
                }
                let boxes = doc
                    .holding_boxes(setup, held)
                    .map_err(|e| format!("{}: workholding cannot be checked: {e}", op.name()))?;
                let gap = crate::holding::gap(setup).unwrap_or_default();
                if let Some(hit) =
                    crate::holding::hits(&plan, &boxes, gap + crate::post::blend(machine)).first()
                {
                    return Err(crate::holding::message(op.name(), hit, gap, zero));
                }
                let slot = crate::slot::Slot::read(op, zero)?;
                let pass_ends: Vec<usize> = plan
                    .moves
                    .iter()
                    .enumerate()
                    .filter(|(_, m)| m.cutting_pass)
                    .map(|(i, _)| i)
                    .collect();
                count = pass_ends.len();
                let mut start = 0;
                for (i, &end) in pass_ends.iter().enumerate() {
                    let stop = if i + 1 == pass_ends.len() {
                        plan.moves.len()
                    } else {
                        (end + 2).min(plan.moves.len())
                    };
                    let mut targets = vec![];
                    if i == 0 {
                        // First put Z above the stock, independent of wherever
                        // the operator left the axes before starting the plan.
                        targets.push(Target {
                            order: 1,
                            axis: 2,
                            centre: plan.moves[0].from[2] - zero[2],
                            shown: plan.moves[0].from[2] - zero[2],
                            edge: false,
                            feed: false,
                            lock: [false; 3],
                        });
                    }
                    for m in plan.moves[start..stop]
                        .iter()
                        .filter(|m| m.motion != Motion::Dwell)
                    {
                        let axis = moved(m)?;
                        let v = m.to[axis] - zero[axis];
                        targets.push(Target {
                            order: (targets.len() + 1) as u8,
                            axis,
                            centre: v,
                            shown: v,
                            edge: false,
                            feed: m.motion == Motion::Feed,
                            lock: std::array::from_fn(|k| m.cutting_pass && k != axis),
                        });
                    }
                    steps.push(Step::Slot {
                        op: oi,
                        index: i + 1,
                        of: count,
                        z: plan.moves[end].to[2] - zero[2],
                        top: plan.rest_top.unwrap_or(stock.max[2]) - zero[2],
                        a: [slot.a[0] - zero[0], slot.a[1] - zero[1]],
                        b: [slot.b[0] - zero[0], slot.b[1] - zero[1]],
                        targets,
                        entry_clear: get(
                            &doc.tools
                                .iter()
                                .find(|t| t.id == op.tool)
                                .ok_or("Slot tool missing")?
                                .p,
                            "center_cutting",
                        ) != "Yes",
                    });
                    start = stop;
                }
                steps.push(Step::Check { op: oi });
            }
            Strategy::Corner => {
                // Tool centres, as the walk is worked out: the compensation
                // off, said so when the operation before left it on.
                sides([0; 2], &mut steps);
                if comp_on {
                    steps.push(Step::Comp {
                        op: oi,
                        comp: [0; 2],
                        diameter,
                    });
                }
                // Walks that reach into the vise or the clamps are no steps
                // to follow: said, as the planning page says them (LIO-443).
                // Nor are they where the workholding is not one that can be
                // checked: a vise too small for the stock, a deleted holder.
                let boxes = doc.holding_boxes(setup, held).map_err(|e| {
                    format!(
                        "{}: the workholding cannot be checked against the walks: {e}",
                        op.name()
                    )
                })?;
                let gap = crate::holding::gap(setup).unwrap_or_default();
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
                // The walks as the plan made them, each with its corner.
                let walks = &plan.corner_walks;
                if walks.is_empty() {
                    return Err(format!(
                        "{}: its walks are not known. {}",
                        op.name(),
                        crate::corner::BROKEN
                    ));
                }
                count = walks.len();
                for (k, w) in walks.iter().enumerate() {
                    let at = (k + 1, walks.len());
                    let (Some(c), Some(moves)) =
                        (corners.get(w.corner), plan.moves.get(w.moves.clone()))
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
                    let (mut lo, mut hi) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
                    for m in moves.iter().filter(|m| m.motion == Motion::Feed) {
                        for p in [m.from, m.to] {
                            for k in 0..3 {
                                (lo[k], hi[k]) = (lo[k].min(p[k]), hi[k].max(p[k]));
                            }
                        }
                    }
                    let legs: Vec<&Move> =
                        moves.iter().filter(|m| m.motion != Motion::Dwell).collect();
                    let mut targets = vec![];
                    for m in &legs {
                        // A move of no number, or along two axes, is no DRO
                        // row: refused as the walk's, which the plan checks
                        // report (LIO-447).
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
                        let down =
                            axis == 2 && m.to[2] < m.from[2] && (m.to[2] - lo[2]).abs() < 1e-4;
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
                    // Each walk's screen stands alone: Z up above the stock,
                    // then X and Y where it goes down, said even when the
                    // walk before left the tool there already (LIO-447:
                    // walk 9 of the adapter plate went down with no X).
                    let down = targets.iter().position(|t| t.axis == 2 && t.lock[2]);
                    if let (Some(down), Some(start)) = (down, legs.first().map(|m| m.from)) {
                        let said: Vec<usize> = (targets[..down].iter()).map(|t| t.axis).collect();
                        let first_z = said.iter().position(|&k| k == 2);
                        let mut ahead = vec![];
                        for axis in [2, 0, 1] {
                            // Z must come before X and Y are set; X and Y
                            // before the tool goes down.
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
                        return Err(crate::corner::broken(
                            op.name(),
                            (k + 1, walks.len()),
                            c.place(),
                            &why,
                        ));
                    }
                    let feed = moves.iter().find(|m| m.motion == Motion::Feed);
                    steps.push(Step::Corner {
                        op: oi,
                        index: k + 1,
                        of: walks.len(),
                        corner: w.corner + 1,
                        corners: corners.len(),
                        place: c.place(),
                        radius: c.r,
                        centre: [c.centre[0] - zero[0], c.centre[1] - zero[1]],
                        offset,
                        z: w.z,
                        level: w.level,
                        levels: w.levels,
                        scallop: w.stair.scallop as f32,
                        points,
                        targets,
                        never: Limits {
                            x: [lo[0] - r - zero[0], hi[0] + r - zero[0]],
                            y: [lo[1] - r - zero[1], hi[1] + r - zero[1]],
                            z: lo[2] - zero[2],
                        },
                        turns_per_minute: feed.map_or(0., |m| {
                            manual::turns(machine, moved(m).unwrap_or(0), m.feed)
                        }),
                    });
                }
                steps.push(Step::Check { op: oi });
            }
            other => return Err(format!("{} does not run on a manual mill.", other.name())),
        }
        sections.push(Section {
            // A Thread as a Thread (LIO-419).
            name: crate::threading::shown_name(op, &doc.tools),
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

#[cfg(test)]
pub(crate) mod tests {
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
                Step::TapIn { .. } => "TapIn",
                Step::Comp { .. } => "Comp",
                Step::Pass { .. } => "Pass",
                Step::Corner { .. } => "Corner",
                Step::Slot { .. } => "Slot",
                Step::Face { .. } => "Face",
                Step::Square { .. } => "Square",
                Step::Holes { .. } => "Holes",
                Step::Pattern { .. } => "Pattern",
                Step::Bore { .. } => "Bore",
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
    /// `cols` × `rows` holes from `first`, `pitch` apart, the rows at
    /// `angle`° from +X, row by row back and forth.
    pub(crate) fn grid_at(
        first: [f32; 2],
        cols: usize,
        rows: usize,
        pitch: [f32; 2],
        angle: f32,
    ) -> Vec<[f32; 2]> {
        places(&Pattern::Grid {
            first,
            columns: cols,
            rows,
            pitch,
            angle,
        })
    }
    /// `n` holes on the circle Ø`d` round `centre`, from `first`° each
    /// `step`° on counter-clockwise.
    pub(crate) fn circle_at(
        centre: [f32; 2],
        d: f32,
        n: usize,
        first: f32,
        step: f32,
    ) -> Vec<[f32; 2]> {
        places(&Pattern::Arc {
            centre,
            diameter: d,
            count: n,
            first,
            step,
            full: false,
        })
    }
    /// The reported three-setup sequence, without depending on a user file.
    pub(crate) fn squared_job() -> (Document, Mesh) {
        let bounds = Bounds {
            min: [0.; 3],
            max: [134., 88., 20.],
        };
        let tris = crate::viewport::box_triangles(bounds, [0.7; 3])
            .into_iter()
            .map(|(t, _)| t)
            .collect();
        let mesh = crate::mesh::normalize(tris).unwrap();
        let mut doc = Document::default();
        let machine = manual::defaults();
        let mut tool = Tool::new("Square mill", "Flat end mill", "10");
        for (k, v) in [
            ("length", "25"),
            ("stickout", "35"),
            ("holder", "32"),
            ("flutes", "3"),
            ("machines", &machine.id),
        ] {
            tool.p.insert(k.into(), v.into());
        }
        let mut holder = crate::holding::starters()
            .into_iter()
            .find(|h| h.kind() == "Vise")
            .unwrap();
        holder.p.insert("machines".into(), machine.id.clone());
        holder.p.insert("jaw_width".into(), "100".into());
        doc.workholding = vec![holder.clone()];
        doc.machines = vec![machine.clone()];
        for (si, rx) in ["90", "-90", "0"].into_iter().enumerate() {
            let mut setup = crate::model::Setup::new(machine.clone(), "G54");
            for (k, v) in [
                ("rx", rx),
                ("origin", "Stock top center"),
                ("holding", "Vise"),
                ("holder", &holder.id),
                ("vise_axis", "Y"),
                ("vise_grip", if si == 2 { "8" } else { "30" }),
                ("stock_left", "3"),
                ("stock_right", "3"),
                ("stock_front", "0"),
                ("stock_back", "0"),
                ("stock_top", "3"),
                ("stock_bottom", "3"),
            ] {
                setup.p.insert(k.into(), v.into());
            }
            if si > 0 {
                setup
                    .p
                    .insert("stock_source".into(), doc.setups[si - 1].id.clone());
            }
            let block = if si < 2 {
                [-70., 70., -10., 10., 0., -3.]
            } else {
                [67., 90., -44., 44., 0., -20.]
            };
            setup.operations = vec![boxed(
                "Square edge",
                &tool.id,
                block,
                &[
                    ("allowance", "0"),
                    ("stepover", "40"),
                    ("stepdown", "0.5"),
                    ("rpm", "1600"),
                    ("feed", "120"),
                ],
            )];
            doc.setups.push(setup);
        }
        doc.tools = vec![tool];
        doc.active = doc.setups[2].id.clone();
        (doc, mesh)
    }
    /// Check actual continuous sweeps on both finished planes, through the
    /// full blank thickness and length: no ledge within the 8 mm jaw band.
    fn squared_acceptance(doc: &Document, mesh: &Mesh) {
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
            let Step::Hold { stock, .. } = guide.steps[0] else {
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
    fn squared_edges_regrip_on_finished_sides() {
        let (doc, mesh) = squared_job();
        squared_acceptance(&doc, &mesh);
    }
    #[test]
    fn partially_squared_edge_still_rejects_a_stepped_jaw_contact() {
        let (mut doc, mesh) = squared_job();
        for (k, v) in [("y_min", "-5"), ("y_max", "5")] {
            doc.setups[0].operations[0].p.insert(k.into(), v.into());
        }
        let w = simulate(&doc, &mesh, 2, "empty").unwrap();
        let s = &doc.setups[2];
        let f = crate::volume::Frame::new(s, &mesh);
        let bands = crate::holding::Bands::new(&w.before, f, simulation::part_box(s, &mesh));
        let held = crate::holding::held_by(&doc, s, w.done[0].1.stock, Some(&bands));
        let issues = crate::holding::holdable(
            2,
            &crate::holding::layout(s).unwrap(),
            doc.holder(get(&s.p, "holder")).unwrap(),
            held,
            Some(&crate::rest::Removed::new(&w.before, f)),
            None,
            &|_, _| String::new(),
        );
        assert!(
            issues
                .iter()
                .any(|s| s.contains("removed the material the jaws grip")),
            "{issues:?}"
        );
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
    #[test]
    fn grids_and_arcs_of_holes_are_found_to_the_hundredth_in_the_dros_order() {
        // Out of order and up to 0.002 off, as a mesh's centres are.
        let jumble = |v: &[[f32; 2]]| {
            let mut v = v.to_vec();
            v.reverse();
            v.rotate_left(1);
            for (j, p) in v.iter_mut().enumerate() {
                p[0] += 0.002 * ((j % 3) as f32 - 1.);
                p[1] -= 0.002 * (j % 2) as f32;
            }
            v
        };
        let found = |holes: &[[f32; 2]], grids: bool, arcs: bool| {
            let given = jumble(holes);
            let (p, order) = pattern(&given, grids, arcs)?;
            // In the DRO's order, the holes as made.
            for (a, b) in reordered(&given, &order).iter().zip(holes) {
                assert!((a[0] - b[0]).hypot(a[1] - b[1]) < 0.02, "{a:?} {b:?}");
            }
            Some(p)
        };
        let grid = |first, columns, rows, pitch, angle| {
            Some(Pattern::Grid {
                first,
                columns,
                rows,
                pitch,
                angle,
            })
        };
        // A 3 × 4 grid, and turned 30° round its first hole.
        for angle in [0., 30.] {
            let holes = grid_at([10., 20.], 4, 3, [15., 10.], angle);
            let want = grid([10., 20.], 4, 3, [15., 10.], angle);
            assert_eq!(found(&holes, true, true), want);
            assert_eq!(found(&holes, false, true), None);
        }
        // A line of four along Y: 1 hole a row × 4 rows at 0° (LIO-402);
        // along X: 4 × 1; slanted: a row at its angle.
        let up = grid_at([5., 5.], 1, 4, [0., 12.5], 0.);
        assert_eq!(
            found(&up, true, false),
            grid([5., 5.], 1, 4, [0., 12.5], 0.)
        );
        let along = grid_at([5., 5.], 4, 1, [12.5, 0.], 0.);
        assert_eq!(
            found(&along, true, false),
            grid([5., 5.], 4, 1, [12.5, 0.], 0.)
        );
        let slant = grid_at([5., 5.], 4, 1, [12.5, 0.], 60.);
        assert_eq!(
            found(&slant, true, false),
            grid([5., 5.], 4, 1, [12.5, 0.], 60.)
        );
        // Square to the axes within 0.02° reads 0.00° (LIO-402).
        let hair = grid_at([10., 20.], 4, 3, [15., 10.], 0.015);
        assert_eq!(
            found(&hair, true, true),
            grid([10., 20.], 4, 3, [15., 10.], 0.)
        );
        // Back and forth: one axis between holes one after the other
        // (LIO-405). The jumble above checks the order as made.
        let order = grid_at([0., 0.], 4, 3, [10., 10.], 0.);
        for w in order.windows(2) {
            let moved = (0..2).filter(|&k| (w[0][k] - w[1][k]).abs() > 1e-3).count();
            assert_eq!(moved, 1, "{w:?}");
        }
        assert_eq!(
            (order[3], order[4], order[7], order[8]),
            ([30., 0.], [30., 10.], [0., 10.], [0., 20.])
        );
        // A bolt circle of six, and an arc of four over 90°.
        let bolts = circle_at([50., 40.], 80., 6, 15., 60.);
        let arc = |centre, diameter, count, first, step, full| {
            Some(Pattern::Arc {
                centre,
                diameter,
                count,
                first,
                step,
                full,
            })
        };
        assert_eq!(
            found(&bolts, true, true),
            arc([50., 40.], 80., 6, 15., 60., true)
        );
        assert_eq!(found(&bolts, true, false), None);
        // Part of a circle fits the noise less tightly than the whole.
        let part = circle_at([-20., 10.], 50., 4, 10., 30.);
        let Some(Pattern::Arc {
            centre,
            diameter,
            count: 4,
            first,
            step,
            full: false,
        }) = found(&part, false, true)
        else {
            panic!()
        };
        let near = |a: f32, b: f32| (a - b).abs() <= 0.03;
        let all = [centre[0], centre[1], diameter, first, step];
        for (got, want) in all.into_iter().zip([-20., 10., 50., 10., 30.]) {
            assert!(near(got, want), "{all:?}");
        }
        // A square of four: a grid when the DRO has grids, else an arc.
        let square = grid_at([0., 0.], 2, 2, [40., 40.], 0.);
        assert_eq!(
            found(&square, true, true),
            grid([0., 0.], 2, 2, [40., 40.], 0.)
        );
        // As an arc it starts at the corner at 45°, the back-right one.
        let round = circle_at([20., 20.], 40. * std::f32::consts::SQRT_2, 4, 45., 90.);
        assert!(round.iter().all(|p| {
            square
                .iter()
                .any(|q| (p[0] - q[0]).hypot(p[1] - q[1]) < 1e-3)
        }));
        assert_eq!(
            found(&round, false, true),
            arc([20., 20.], 56.57, 4, 45., 90., true)
        );
        // No pattern: a ragged set, two holes, one hole; a grid one hole
        // short; three in a line unevenly.
        let ragged = [[0., 0.], [10., 0.], [25., 3.], [7., 19.]];
        let short = &grid_at([0., 0.], 3, 3, [10., 10.], 0.)[..8];
        // Three holes in an isosceles triangle: on a circle, but three
        // prove no arc short of all the way round (LIO-402). A pitch that is
        // no whole hundredth adds up: 31 holes over 100 mm end 0.1 off.
        let isosceles = [[0., 0.], [20., 0.], [10., 5.]];
        let thirds = grid_at([0., 0.], 31, 1, [100. / 30., 0.], 0.);
        for holes in [
            &ragged[..],
            &ragged[..2],
            &ragged[..1],
            short,
            &[[0., 0.], [10., 0.], [25., 0.]][..],
            &isosceles[..],
            &thirds[..],
        ] {
            assert_eq!(found(holes, true, true), None, "{holes:?}");
        }
        // Three all the way round are an arc.
        let three = circle_at([0., 0.], 40., 3, 90., 120.);
        assert_eq!(
            found(&three, true, true),
            arc([0., 0.], 40., 3, 90., 120., true)
        );
        // Four on a circle at 0°, 90°, 180°, 270°: a bolt circle of four,
        // not a grid turned 45° (LIO-402); a grid when the DRO has no arcs.
        let four = circle_at([0., 0.], 50., 4, 0., 90.);
        assert_eq!(
            found(&four, true, true),
            arc([0., 0.], 50., 4, 0., 90., true)
        );
        // (In the grid's order, not the circle's.)
        assert!(matches!(
            pattern(&four, true, false),
            Some((Pattern::Grid { angle, .. }, _)) if (angle.abs() - 45.).abs() < 0.05
        ));
    }
    #[test]
    fn mirrored_arc_holes_are_taken_one_axis_a_move() {
        let order =
            |at: &[[f32; 2]]| -> Vec<usize> { drill_order(at).iter().map(|j| j + 1).collect() };
        let moves = |at: &[[f32; 2]]| {
            let o = drill_order(at);
            (o.windows(2))
                .filter(|w| one_axis(at[w[0]], at[w[1]]))
                .count()
        };
        // Six on Ø60 from 30°: 30, 150, 210, 330, then 270, 90.
        let six = circle_at([0., 0.], 60., 6, 30., 60.);
        assert_eq!(order(&six), [1, 3, 4, 6, 5, 2]);
        assert_eq!(moves(&six), 4);
        // Four at 0°, 90°, 180°, 270°: across, then the other pair.
        let four = circle_at([0., 0.], 50., 4, 0., 90.);
        assert_eq!(order(&four), [1, 3, 2, 4]);
        assert_eq!(moves(&four), 2);
        // Four from 45° (one axis a move already) and five from 17° (no
        // mirrored pair) go round as before.
        assert_eq!(order(&circle_at([0., 0.], 50., 4, 45., 90.)), [1, 2, 3, 4]);
        assert_eq!(
            order(&circle_at([0., 0.], 50., 5, 17., 72.)),
            [1, 2, 3, 4, 5]
        );
        // Part of a circle with a mirrored pair (60° and 120°): one after
        // the other.
        let part = circle_at([0., 0.], 50., 4, 60., 30.);
        let o = order(&part);
        let (a, b) = (
            o.iter().position(|&n| n == 1),
            o.iter().position(|&n| n == 3),
        );
        assert_eq!(b.unwrap().abs_diff(a.unwrap()), 1, "{o:?}");
    }
    #[test]
    fn a_bolt_circle_is_drilled_with_the_dros_numbers_beside_the_order() {
        let (doc, mesh) = pattern_job(&plate_circle(), false, true, false);
        let guide = build(&doc, &mesh, 0).unwrap();
        let Some(Step::Holes { holes, numbers, .. }) =
            guide.steps.iter().find(|s| matches!(s, Step::Holes { .. }))
        else {
            panic!()
        };
        assert_eq!(numbers, &[1, 3, 4, 6, 5, 2]);
        // Each hole is the DRO's hole of its number, round from 30°.
        for (h, n) in holes.iter().zip(numbers) {
            let t = (30. + 60. * (*n as f32 - 1.)).to_radians();
            assert!(
                (h[0] - 30. * t.cos()).hypot(h[1] - 30. * t.sin()) < 0.05,
                "{n} {h:?}"
            );
        }
    }
    #[test]
    fn a_pattern_is_used_only_when_every_hole_goes_to_the_same_depth() {
        let at = |z: &[f32]| -> Vec<[f32; 3]> {
            (grid_at([0., 0.], 3, 2, [10., 10.], 0.).iter().zip(z))
                .map(|(p, z)| [p[0], p[1], *z])
                .collect()
        };
        let level = at(&[-12.; 6]);
        assert!(level_pattern(&level, true, true).is_some());
        // One hole shallower: the DRO's hole N might be another depth (LIO-403).
        let shallow = at(&[-12., -12., -8., -12., -12., -12.]);
        assert_eq!(level_pattern(&shallow, true, true), None);
        // Within the hundredth shown is the same depth.
        let close = at(&[-12., -12.001, -12., -12., -12.002, -12.]);
        assert!(level_pattern(&close, true, true).is_some());
    }
    /// A 120 × 90 × 10 plate with Ø6 holes through at `at` (its middle at
    /// 0), drilled Ø6 on a manual mill whose DRO has matrix holes (`grids`)
    /// and arc holes (`arcs`); with `centred` a centre drilling of the same
    /// holes before it.
    pub(crate) fn pattern_job(
        at: &[[f32; 2]],
        grids: bool,
        arcs: bool,
        centred: bool,
    ) -> (Document, Mesh) {
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
        let mut machine = manual::defaults();
        for (k, on) in [("dro_matrix", grids), ("dro_arc", arcs)] {
            machine
                .p
                .insert(k.into(), if on { "Yes" } else { "No" }.into());
        }
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
        grid_at([-30., -15.], 4, 3, [20., 15.], 0.)
    }
    /// The plate's bolt circle: six Ø6 holes on Ø60, the first at 30°.
    pub(crate) fn plate_circle() -> Vec<[f32; 2]> {
        circle_at([0., 0.], 60., 6, 30., 60.)
    }
    #[test]
    fn a_drilled_grid_has_a_pattern_screen_and_its_holes_in_the_dros_order() {
        let (doc, mesh) = pattern_job(&plate_grid(), true, false, false);
        let guide = build(&doc, &mesh, 0).unwrap();
        let section = guide.sections[1].steps.clone();
        assert_eq!(
            kinds(&guide, section.clone()),
            ["Tool", "Pattern", "Holes", "Check"]
        );
        let Step::Pattern {
            pattern:
                Pattern::Grid {
                    first,
                    columns,
                    rows,
                    pitch,
                    angle,
                },
            same: false,
            ..
        } = guide.steps[section.start + 1]
        else {
            panic!("{:?}", guide.steps[section.start + 1])
        };
        assert_eq!((columns, rows, pitch, angle), (4, 3, [20., 15.], 0.));
        let Step::Holes { holes, pattern, .. } = &guide.steps[section.start + 2] else {
            panic!()
        };
        assert!(pattern.is_some());
        // Row by row from the first hole, back and forth (LIO-405).
        assert_eq!(holes.len(), 12);
        for (j, h) in holes.iter().enumerate() {
            let col = if (j / 4) % 2 == 1 { 3 - j % 4 } else { j % 4 };
            let want = [first[0] + col as f32 * 20., first[1] + (j / 4) as f32 * 15.];
            assert!((h[0] - want[0]).hypot(h[1] - want[1]) < 0.02, "{j} {h:?}");
        }
        // Without the DRO's matrix holes: hole by hole, as before.
        let (doc, mesh) = pattern_job(&plate_grid(), false, true, false);
        let guide = build(&doc, &mesh, 0).unwrap();
        let section = guide.sections[1].steps.clone();
        assert_eq!(kinds(&guide, section.clone()), ["Tool", "Holes", "Check"]);
        assert!(matches!(
            guide.steps[section.start + 1],
            Step::Holes { pattern: None, .. }
        ));
    }
    #[test]
    fn a_drill_after_its_centre_drill_on_the_same_circle_says_the_pattern_is_the_same() {
        let (doc, mesh) = pattern_job(&plate_circle(), false, true, true);
        let guide = build(&doc, &mesh, 0).unwrap();
        let patterns: Vec<(&Pattern, bool)> = (guide.steps.iter())
            .filter_map(|s| match s {
                Step::Pattern { pattern, same, .. } => Some((pattern, *same)),
                _ => None,
            })
            .collect();
        assert_eq!(patterns.len(), 2, "{:?}", guide.steps);
        assert_eq!((patterns[0].1, patterns[1].1), (false, true));
        let Pattern::Arc {
            diameter,
            count,
            first,
            step,
            full,
            ..
        } = patterns[1].0
        else {
            panic!()
        };
        assert_eq!(
            (*diameter, *count, *first, *step, *full),
            (60., 6, 30., 60., true)
        );
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
        // The 15 mm step is one row on each of its two levels (LIO-366); the
        // Face's tool is still in, so only the DRO is set (LIO-368).
        assert_eq!(of(2), ["Comp", "Pass", "Pass", "Check"]);
        // The four holes on a 96 x 56 rectangle are a 2 x 2 grid, which the
        // DRO's matrix holes (on by default, LIO-399) take: a Pattern first.
        assert_eq!(of(3), ["Tool", "Pattern", "Holes", "Check"]);
        assert_eq!(of(4), ["HandOver"]);
        let counts: Vec<usize> = guide.sections.iter().map(|s| s.count).collect();
        assert_eq!(counts, [0, 0, 2, 4, 0]);
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
    #[test]
    fn steps_run_hold_zero_then_each_operation_and_a_face_is_one_step() {
        let (doc, mesh) = job("Yes");
        let guide = build(&doc, &mesh, 0).unwrap();
        assert_eq!(
            kinds(&guide, 0..guide.steps.len()),
            // The holes are a 2 x 2 grid: a Pattern before them (LIO-399).
            [
                "Hold", "Zero", "Face", "Comp", "Pass", "Pass", "Check", "Tool", "Pattern",
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
                // The 124 × 120 block's left and front faces, at their
                // middles, on the top.
                at: [[0., 60., 0.], [62., 0., 0.]],
                round: [false, false],
                // A corner: no axis is halved.
                far: [None; 2],
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
        close(top.unwrap(), 0.);
        assert_eq!(guide.steps[11], Step::HandOver { next: Some(1) });
        // The Step's floor is closed: not a squaring Box. The Face's tool
        // is still in: only the DRO's diameter and sides (LIO-368).
        assert_eq!(
            guide.steps[3],
            Step::Comp {
                op: 1,
                comp: [1, 1],
                diameter: 25.
            }
        );
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
        // The Face's mill on the untouched top; after the 2 mm face, the
        // Holes' drill touches a top 2 mm down. The Step keeps the Face's
        // mill and its Z (LIO-368).
        let tops: Vec<(usize, Option<f32>)> = (guide.steps.iter())
            .filter_map(|s| match s {
                Step::Tool { op, top, .. } | Step::Face { op, top, .. } => Some((*op, *top)),
                _ => None,
            })
            .collect();
        assert_eq!(tops.len(), 2, "{tops:?}");
        for ((op, top), want) in tops.into_iter().zip([(0, 0.), (2, -2.)]) {
            assert_eq!(op, want.0);
            assert!((top.unwrap() - want.1).abs() < 0.01, "{op} {top:?}");
        }
        // A square after the face with its tool still in picks up nothing.
        let (mut doc, mesh) = job("Yes");
        let mill = doc.tools[3].id.clone();
        let b = [123., 500., -500., 500., 0., -30.];
        let square = boxed("Square", &mill, b, &[("allowance", "0")]);
        doc.setups[0].operations.insert(1, square);
        let guide = build(&doc, &mesh, 0).unwrap();
        let Step::Square { top, .. } = &guide.steps[3] else {
            panic!("{:?}", guide.steps[3])
        };
        assert_eq!(*top, None);
    }
    #[test]
    fn a_tool_already_in_gets_only_what_changes() {
        let (mut doc, mesh) = job("Yes");
        let mill = doc.tools[3].id.clone();
        // A deeper step after the Step: same tool, gear and sides.
        let b = [-500., 500., -500., 15.5, 0., -14.];
        let deeper = boxed("Deeper", &mill, b, &[("stepdown", "5")]);
        doc.setups[0].operations.insert(2, deeper);
        let guide = build(&doc, &mesh, 0).unwrap();
        let first = |g: &Guide, i: usize| g.steps[g.sections[i].steps.start].clone();
        // After the Face: only the DRO's diameter and sides.
        assert_eq!(
            first(&guide, 2),
            Step::Comp {
                op: 1,
                comp: [1, 1],
                diameter: 25.
            }
        );
        // Nothing changes: the section starts with its first pass.
        assert!(
            matches!(
                first(&guide, 3),
                Step::Pass {
                    op: 2,
                    index: 1,
                    ..
                }
            ),
            "{:?}",
            first(&guide, 3)
        );
        // A change of tool: put it in, pick up Z, the DRO.
        assert!(matches!(
            first(&guide, 4),
            Step::Tool {
                op: 3,
                top: Some(_),
                comp: Some([0, 0]),
                ..
            }
        ));
        // Another gear alone: the gear and nothing else.
        doc.setups[0].operations[2]
            .p
            .insert("rpm".into(), "400".into());
        let guide = build(&doc, &mesh, 0).unwrap();
        assert!(matches!(
            first(&guide, 3),
            Step::Tool {
                op: 2,
                top: None,
                comp: None,
                ..
            }
        ));
    }
    /// The shaft turned on the lathe (`proposal::tests::turned`), Ø20 over
    /// its top 20 mm and Ø30 under it; the manual mill, setup 2, then
    /// drills its Ø4 hole from the model origin on its axis with T8, and
    /// the CNC drills it again: the job the planner builds of it.
    pub(crate) fn turned_then_drilled_by_hand() -> (Document, Mesh) {
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
    pub(crate) fn short_part_in_soft_jaws() -> (
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
    #[test]
    fn with_only_2_mm_of_ø20_over_the_jaws_the_zero_touches_it_and_with_none_it_is_refused() {
        // The CNC grips the Ø30 foot 28 mm.
        let (doc, mesh, report, plan) = stepped_in_soft_jaws(28., 28.);
        let (check, job) = crate::plan_check::check(&doc, &mesh, &report, &plan);
        assert!(check.passed, "{:?}", check.problems);
        let mut job = job.unwrap();
        // The manual mill grips 28 mm of the 30 too.
        job.setups[1].p.insert("round_grip".into(), "28".into());
        let guide = build(&job, &mesh, 1).unwrap();
        let Step::Zero { touch, at, .. } = &guide.steps[1] else {
            panic!("{:?}", guide.steps[1])
        };
        // The Ø20 stands 2 mm over the jaws' top at 28: the finder comes
        // in over them to it, at the top, and X/Y are set to −15 for it.
        for k in 0..2 {
            close(at[k][k], -10.);
            close(at[k][2], touch[2]);
            close(touch[k], -15.);
        }
        // Gripped to 29.6, nothing stands 0.5 mm clear of the jaws.
        job.setups[1].p.insert("round_grip".into(), "29.6".into());
        assert_eq!(build(&job, &mesh, 1).unwrap_err(), NOT_CLEAR);
    }
    #[test]
    fn a_short_part_in_soft_jaws_is_touched_on_its_ø20_above_them_not_its_ø30_inside_them() {
        let (doc, mesh, report, plan) = short_part_in_soft_jaws();
        let (check, job) = crate::plan_check::check(&doc, &mesh, &report, &plan);
        assert!(check.passed, "{:?}", check.problems);
        // The manual mill grips 8 mm of it too.
        let mut job = job.unwrap();
        job.setups[1].p.insert("round_grip".into(), "8".into());
        let guide = build(&job, &mesh, 1).unwrap();
        let Step::Zero {
            touch, at, round, ..
        } = &guide.steps[1]
        else {
            panic!("{:?}", guide.steps[1])
        };
        // The Ø30 stands only 2 mm over the jaws: the finder touches the
        // Ø20 at the top, 10 off the axis, and X/Y are set to −15 for it.
        for k in 0..2 {
            close(at[k][k], -10.);
            close(at[k][1 - k], 0.);
            close(at[k][2], touch[2]);
            close(touch[k], -15.);
        }
        assert_eq!(*round, [true, true]);
    }
    #[test]
    fn after_the_lathe_the_mills_zero_touches_the_shafts_foot_under_its_shoulder() {
        let (job, mesh) = turned_then_drilled_by_hand();
        let guide = build(&job, &mesh, 1).unwrap();
        let Step::Zero {
            touch, at, round, ..
        } = &guide.steps[1]
        else {
            panic!("{:?}", guide.steps[1])
        };
        // The left and front of the Ø30, 15 off the axis, less the Ø10
        // finder's radius: −20, not the −15 the Ø20 top would need.
        close(touch[0], -20.);
        close(touch[1], -20.);
        for k in 0..2 {
            // On the Ø30's face, on its centre line, 3 mm under the
            // shoulder that is 20 under the top.
            close(at[k][k], -15.);
            close(at[k][1 - k], 0.);
            close(touch[2] - at[k][2], 23.);
        }
        assert_eq!(*round, [true, true]);
        // With the zero on the top's centre, each axis is halved across
        // the Ø30 (LIO-451): both sides on the centre line, as low, round
        // both; the DRO reads 30 + Ø10 on the + side and is set to half.
        let mut job = job;
        (job.setups[1].p).insert("origin".into(), "Stock top center".into());
        let guide = build(&job, &mesh, 1).unwrap();
        let Step::Zero {
            touch,
            at,
            round,
            far,
            ..
        } = &guide.steps[1]
        else {
            panic!("{:?}", guide.steps[1])
        };
        assert_eq!(*round, [true, true]);
        for k in 0..2 {
            let f = far[k].unwrap();
            assert!(f.round);
            close(at[k][k], -15.);
            close(f.at[k], 15.);
            close(f.at[1 - k], 0.);
            close(f.at[2], at[k][2]);
            close(f.reads, 40.);
            close(touch[k], 20.);
        }
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
        // 124 × 120 × 47 stock: the back-right corner, the centre, the
        // middle of the front edge, the bottom. A middle is halved
        // (LIO-451): set at the + face to half of what the DRO reads there,
        // the width plus the finder's diameter.
        for (origin, corner, approach, touch, reads) in [
            (
                "Stock top back-right",
                [0., 0., 0.],
                [1, 1],
                [3., 3., 0.],
                [None, None],
            ),
            (
                "Stock top center",
                [0., 0., 0.],
                [-1, -1],
                [65., 63., 0.],
                [Some(130.), Some(126.)],
            ),
            (
                "Stock top front",
                [0., 0., 0.],
                [-1, -1],
                [65., -3., 0.],
                [Some(130.), None],
            ),
            (
                "Stock bottom front-left",
                [0., 0., 47.],
                [-1, -1],
                [-3., -3., 47.],
                [None, None],
            ),
        ] {
            doc.setups[0].p.insert("origin".into(), origin.into());
            let guide = build(&doc, &mesh, 0).unwrap();
            let Step::Zero {
                corner: c,
                approach: a,
                touch: t,
                finder,
                far,
                ..
            } = &guide.steps[1]
            else {
                panic!()
            };
            assert_eq!((*a, *finder), (approach, 6.), "{origin}");
            for k in 0..3 {
                close(c[k], corner[k]);
                close(t[k], touch[k]);
            }
            for k in 0..2 {
                match (far[k], reads[k]) {
                    (Some(f), Some(r)) => close(f.reads, r),
                    (None, None) => {}
                    other => panic!("{origin} axis {k}: {other:?}"),
                }
            }
        }
        doc.setups[0]
            .machine
            .p
            .insert("edge_finder".into(), "0".into());
        let error = build(&doc, &mesh, 0).unwrap_err();
        assert!(error.contains("edge finder"), "{error}");
    }
    /// `squared_job`'s manual setup twice (LIO-451): the first cuts the
    /// Box `cut` (x_min, x_max, y_min, y_max, z_top, z_bottom, work) into
    /// a fresh block, the second, with the zero at the top's centre, only
    /// faces 0.5 off its other end, so that its Zero touches what the
    /// first left.
    pub(crate) fn after_a_cut(cut: [f32; 6]) -> (Document, Mesh) {
        let (mut doc, mesh) = squared_job();
        let mut first = doc.setups[2].clone();
        first.id = "earlier".into();
        for (k, v) in [
            ("stock_source", "fresh"),
            ("stock_top", "0"),
            ("stock_bottom", "0"),
            ("vise_grip", "6"),
            ("name", "Earlier cut"),
            ("origin", "Stock top center"),
        ] {
            first.p.insert(k.into(), v.into());
        }
        let fields = [
            ("allowance", "0"),
            ("stepover", "40"),
            ("stepdown", "2"),
            ("rpm", "1000"),
            ("feed", "100"),
        ];
        first.operations = vec![boxed("Earlier cut", &doc.tools[0].id, cut, &fields)];
        let mut later = first.duplicate("G54");
        later.p.insert("name".into(), "Zero after it".into());
        // Outside the part's 134, on the side away from the cut.
        let side = if cut[0] > 0. {
            [-80., -67.]
        } else {
            [67., 80.]
        };
        later.operations = vec![boxed(
            "Face",
            &doc.tools[0].id,
            [side[0], side[1], -50., 50., 0., -0.5],
            &fields,
        )];
        doc.active = later.id.clone();
        doc.setups = vec![first, later];
        (doc, mesh)
    }
    #[test]
    fn a_halved_axis_is_never_touched_in_a_notch_an_earlier_setup_cut() {
        // A notch 20 wide and 12 deep in the middle of the +X face's top;
        // then also a slot 1.2 wide beside it, narrower than the pitch the
        // face is sampled at, where the material's middle falls.
        for slot in [false, true] {
            let (mut doc, mesh) = after_a_cut([67., 80., -10., 10., 0., -12.]);
            if slot {
                let mut tool = doc.tools[0].clone();
                tool.id = "slot-mill".into();
                tool.p.insert("diameter".into(), "1.2".into());
                doc.setups[0].operations.push(boxed(
                    "Slot",
                    &tool.id,
                    [67., 80., 26.088522, 27.288522, 0., -12.],
                    &[
                        ("allowance", "0"),
                        ("stepover", "40"),
                        ("stepdown", "1"),
                        ("rpm", "1000"),
                        ("feed", "40"),
                    ],
                ));
                doc.tools.push(tool);
            }
            let walked = simulate(&doc, &mesh, 1, "").unwrap();
            let guide = build(&doc, &mesh, 1).unwrap();
            let Step::Zero { far, touch, .. } = &guide.steps[1] else {
                panic!("{:?}", guide.steps[1])
            };
            let f = far[0].unwrap();
            // On the +X face, at the top, beside the notch, where there is
            // material just inside it.
            close(f.at[0], 70.);
            close(f.at[2], 0.);
            assert!(f.at[1].abs() > 10., "{f:?}");
            let frame = crate::volume::Frame::new(&doc.setups[1], &mesh);
            let rest = crate::rest::RestStock::new(&walked.before, frame);
            let removed = rest.removed_in(walked.done[0].1.stock);
            let mut p: [f32; 3] = std::array::from_fn(|k| f.at[k] + guide.zero[k]);
            (p[0], p[2]) = (p[0] - 0.2, p[2] - 0.2);
            assert!(
                rest.in_stock(p) && !removed.at(p),
                "slot {slot}: {f:?} {p:?}"
            );
            if slot {
                assert!(!(26.08..=27.29).contains(&f.at[1]), "{f:?}");
            }
            // Both faces whole: half the reading.
            close(f.reads, 150.);
            close(touch[0], 75.);
        }
    }
    #[test]
    fn halved_touches_not_either_side_of_the_zero_alike_say_how_far_from_half() {
        // The +X face cut back 3 mm for 16 mm down, into the jaws: the
        // finder touches it at 67, the −X face at −70; the DRO reads 147
        // and is set to 72, half less 1.5. Mirrored on −X, half plus 1.5.
        for (cut, near, far_x, set) in [
            ([67., 80., -50., 50., 0., -16.], -70., 67., 72.),
            ([-80., -67., -50., 50., 0., -16.], -67., 70., 75.),
        ] {
            let (doc, mesh) = after_a_cut(cut);
            let guide = build(&doc, &mesh, 1).unwrap();
            let zero = &guide.steps[1];
            let Step::Zero { at, far, touch, .. } = zero else {
                panic!("{zero:?}")
            };
            let f = far[0].unwrap();
            close(at[0][0], near);
            close(f.at[0], far_x);
            close(f.reads, 147.);
            close(touch[0], set);
            // The DRO reads 0 at the zero with the faces where touched.
            close(dro_at_zero(zero, 0, [near, far_x]), 0.);
        }
    }
    /// What a DRO shows at the work zero after the Zero step's touches on
    /// axis `k` (X or Y) of `zero`, for a blank whose faces are where
    /// `faces` (work coordinates) say, which may differ from the
    /// drawing's: the edge finder's centre stands a radius outside each
    /// face touched, the axis is set as the step says, and the reading
    /// at the work zero is what the DRO then shows there.
    fn dro_at_zero(zero: &Step, k: usize, faces: [f32; 2]) -> f32 {
        let Step::Zero {
            approach,
            touch,
            finder,
            far,
            ..
        } = zero
        else {
            panic!("{zero:?}")
        };
        let r = finder / 2.;
        // The DRO reads `position - offset`; each set picks the offset.
        // Halved: 0 on the − face, then the + face set to `touch`, which
        // replaces it.
        let offset = match far[k] {
            Some(_) => (faces[1] + r) - touch[k],
            None if approach[k] > 0 => (faces[1] + r) - touch[k],
            None => (faces[0] - r) - touch[k],
        };
        -offset
    }
    #[test]
    fn a_centre_zero_is_found_by_halving_between_both_faces() {
        let (mut doc, mesh) = job("Yes");
        doc.setups[0].operations.drain(..2);
        doc.setups[0]
            .p
            .insert("origin".into(), "Stock top center".into());
        let guide = build(&doc, &mesh, 0).unwrap();
        let zero = &guide.steps[1];
        let Step::Zero {
            approach,
            touch,
            finder,
            at,
            far,
            ..
        } = zero
        else {
            panic!("{zero:?}")
        };
        assert_eq!((*approach, *finder), ([-1, -1], 10.));
        // The 124 × 120 block: the − faces at −62 and −60, the + faces at
        // +62 and +60, each touched at the middle of the other axis, on
        // the top.
        let far = far.map(Option::unwrap);
        for (k, half) in [(0, 62.), (1, 60.)] {
            close(at[k][k], -half);
            close(far[k].at[k], half);
            close(at[k][1 - k], 0.);
            close(far[k].at[1 - k], 0.);
            close(at[k][2], 0.);
            close(far[k].at[2], 0.);
            // The DRO reads the width plus the Ø10 finder there: a check.
            close(far[k].reads, 2. * half + 10.);
            // Set to half the reading: the finder's radius cancels.
            close(touch[k], far[k].reads / 2.);
        }
        close(touch[2], 0.);
        // On the drawing's blank, the DRO reads 0 at the work zero.
        for k in 0..2 {
            let half = [62., 60.][k];
            close(dro_at_zero(zero, k, [-half, half]), 0.);
        }
        // A sawn blank 1 mm wider on X, 0.6 mm narrower on Y than drawn,
        // its middle on the drawing's: halving, with ½ taking half of what
        // the DRO really reads, still puts 0 on its middle, where one touch
        // on the − face set to the drawing's half (the step before
        // LIO-451) is off by half the error.
        for (k, faces, error) in [(0, [-62.5, 62.5], 0.5), (1, [-59.7, 59.7], -0.3)] {
            let read = (faces[1] + 5.) - (faces[0] - 5.);
            let mut halved = zero.clone();
            let mut one_face = zero.clone();
            if let Step::Zero { touch, .. } = &mut halved {
                touch[k] = read / 2.;
            }
            if let Step::Zero { touch, far, .. } = &mut one_face {
                touch[k] = -[62., 60.][k] - 5.;
                far[k] = None;
            }
            close(dro_at_zero(&halved, k, faces), 0.);
            close(dro_at_zero(&one_face, k, faces), error);
        }
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
            // it runs its passes, the same tool still in (LIO-368), after
            // the DRO's diameter and sides when its compensation is on.
            let want: &[&str] = match dro {
                "Yes" => &["Comp", "Pass", "Pass", "Check"],
                _ => &["Pass", "Pass", "Check"],
            };
            assert_eq!(kinds(&guide, guide.sections[3].steps.clone()), want);
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
            assert_eq!((*index, *of), (n, 2));
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
                // plan leaves where it starts. Its one row (Y 2.5, LIO-366)
                // is off the plan's start, so here every axis moves.
                let extra: Vec<(usize, f32)> =
                    got.iter().copied().filter(|g| !legs.contains(g)).collect();
                assert!(extra.iter().all(|e| *e == (e.0, start[e.0])), "{extra:?}");
                assert_eq!(got.len(), legs.len() + extra.len());
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
        assert_eq!(n, 2);
        assert!(moves.all(|m| !m.cutting_pass));
        // Conventional with M3: one row on each level, its wall on +Y, so it
        // feeds along +X with the uncut material on its left.
        let rows: Vec<&Move> = plan.moves.iter().filter(|m| m.cutting_pass).collect();
        assert!(rows.iter().all(|m| m.to[0] > m.from[0]));
        assert!(simulation::uncut_sides(&rows).is_empty());
    }
    #[test]
    fn x_and_y_read_the_tool_edge_on_the_comp_side_with_the_dros_compensation() {
        let (doc, mesh) = job("Yes");
        let guide = build(&doc, &mesh, 0).unwrap();
        // Open on both X ends (+), closed on +Y (+).
        assert!(matches!(guide.steps[3], Step::Comp { comp: [1, 1], .. }));
        assert!(matches!(
            guide.steps[7],
            Step::Tool {
                comp: Some([0, 0]),
                ..
            }
        ));
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
        let Step::Pass { never, cut, .. } = &guide.steps[5] else {
            panic!()
        };
        close(never.y[1], 15.);
        close(never.z, -11.5);
        close(cut.max[1], 15.);
        close(cut.min[2], -11.5);
        // Without it, every readout is the centre.
        let (doc, mesh) = job("No");
        let guide = build(&doc, &mesh, 0).unwrap();
        // The Face's tool is still in and there are no sides to set: the
        // Step starts with its first pass (LIO-368).
        assert!(matches!(guide.steps[3], Step::Pass { op: 1, .. }));
        for (_, t) in targets(&guide) {
            assert!(!t.edge && t.shown == t.centre, "{t:?}");
        }
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
    fn the_holes_are_centres_and_depths_in_work_coordinates() {
        let (doc, mesh) = job("Yes");
        let guide = build(&doc, &mesh, 0).unwrap();
        // After the Tool and the holes' Pattern (LIO-399).
        let Step::Holes { op, holes, .. } = &guide.steps[9] else {
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
    /// The job's Step as a Ø10 pocket in the stock in front of the part
    /// (X 20..100, Y 2..18, 5 deep): closed all round, so it plunges.
    pub(crate) fn pocket_the_step(doc: &mut Document) {
        let mut mill = Tool::new("End mill 10", "Flat end mill", "10");
        let machine = doc.setups[0].machine.id.clone();
        for (k, v) in [("length", "30"), ("stickout", "40"), ("machines", &machine)] {
            mill.p.insert(k.into(), v.into());
        }
        let b = [20., 100., 2., 18., 0., -5.];
        doc.setups[0].operations[1] = boxed("Pocket", &mill.id, b, &[("stepdown", "5")]);
        doc.tools.push(mill);
    }
    #[test]
    fn a_closed_pocket_plunges_once_per_level_and_feeds_across_to_its_rows() {
        let (mut doc, mesh) = job("Yes");
        pocket_the_step(&mut doc);
        let guide = build(&doc, &mesh, 0).unwrap();
        let passes: Vec<&Vec<Target>> = (guide.steps.iter())
            .filter_map(|s| match s {
                Step::Pass { op: 1, targets, .. } => Some(targets),
                _ => None,
            })
            .collect();
        // Two rows (Y 7.5 and 12.5) on one level.
        assert_eq!(passes.len(), 2);
        for (i, targets) in passes.iter().enumerate() {
            let (cut, setting) = targets.split_last().unwrap();
            assert!(cut.feed && cut.axis == 0);
            let plunges = setting.iter().filter(|t| t.axis == 2 && t.feed).count();
            let across = setting.iter().filter(|t| t.axis == 1 && t.feed).count();
            // The first row plunges; the second feeds across.
            assert_eq!((plunges, across), [(1, 0), (0, 1)][i], "{i} {targets:?}");
        }
        // Feeds along +X with the uncut material on +Y on the left.
        let plan = &guide.done[1].plan;
        let rows: Vec<&Move> = plan.moves.iter().filter(|m| m.cutting_pass).collect();
        assert!(rows.iter().all(|m| m.to[0] > m.from[0]));
        assert!(simulation::uncut_sides(&rows).iter().all(|s| *s > 0.));
    }
    #[test]
    fn a_step_closed_at_both_ends_goes_down_outside_its_open_front_and_feeds_across() {
        let (mut doc, mesh) = job("Yes");
        close_the_step(&mut doc);
        let guide = build(&doc, &mesh, 0).unwrap();
        let passes: Vec<&Vec<Target>> = (guide.steps.iter())
            .filter_map(|s| match s {
                Step::Pass { op: 1, targets, .. } => Some(targets),
                _ => None,
            })
            .collect();
        // One row (Y 2.5) on each of two levels.
        assert_eq!(passes.len(), 2);
        for targets in &passes {
            let (cut, setting) = targets.split_last().unwrap();
            assert!(cut.feed && cut.axis == 0);
            // No plunge: down in front of the stock, then the first target
            // that cuts is a feed across into the row (LIO-367).
            assert!(
                !setting.iter().any(|t| t.axis == 2 && t.feed),
                "{targets:?}"
            );
            let first = setting.iter().find(|t| t.feed).unwrap();
            assert_eq!(first.axis, 1);
            close(first.centre, 2.5);
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
                comp: [1, -1],
                diameter: 25.
            }
        );
        // The back step's row reads its wall less the allowance: set on its
        // first pass, its one row on each level (LIO-366).
        let y = (guide.steps.iter())
            .filter_map(|s| match s {
                Step::Pass { op: 2, targets, .. } => Some(targets),
                _ => None,
            })
            .flatten()
            .rfind(|t| t.axis == 1)
            .unwrap();
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
        // One step on for the holes' Pattern since LIO-399.
        doc.guide.insert(id.clone(), 8);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("job.axis.json");
        crate::storage::save(&path, &doc).unwrap();
        let mut doc = crate::storage::load(&path).unwrap();
        assert_eq!(doc.guide.get(&id), Some(&8));
        assert_eq!(place(&doc, &guide), 8);
        // Without the Step the setup has 8 steps: the place is the last.
        doc.setups[0].operations.remove(1);
        let guide = build(&doc, &mesh, 0).unwrap();
        assert_eq!(guide.steps.len(), 8);
        assert_eq!(place(&doc, &guide), 7);
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
    pub(crate) fn die_sample() -> (Document, Mesh) {
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
