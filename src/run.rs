//! Running a setup on the connected UC100 without G-code: its checked
//! programs, one per tool run; each lowered to the driver's actions when
//! Cycle Start is pressed; the checks before Cycle Start; progress and the
//! manual tool change between tool runs.
use crate::control::Connection;
use crate::mesh::Mesh;
use crate::model::Document;
use crate::post::{Block, Checked, Kind, OFFSETS, Partly, PostSettings, clearance_plane, extents};
use crate::simulation::{CutterShape, StockGrid};
use std::sync::Arc;
use uc100::controller::{Command, State};
use uc100::gcode::{Action as Do, Located, Spindle};

const AXES: [&str; 6] = crate::control::AXIS_NAMES;

/// What the Run panel asks for.
#[derive(Debug, Clone)]
pub enum Action {
    /// Run this setup (index): check it and start from its first tool run.
    Setup(usize),
    /// Check the setup again after the project changed, keeping the progress.
    Check,
    /// Make this tool run (index) the next one, from its first operation.
    Next(usize),
    /// Make operation `k` (index in its program) of tool run `i` the one
    /// Cycle Start begins with: those before it are taken as cut.
    From(usize, usize),
    /// The work zero is right for the stock a run that starts partway
    /// leaves (`Run::partway`): the one those operations were cut with, or
    /// one set by hand on a surface they left.
    KeepZero,
    CycleStart,
    Hold,
    Resume,
    Stop,
    /// Keep the work Z of the tool tip now: the reference surface the next
    /// tool is touched off on.
    RecordReference,
    /// Make the tool tip's height now the reference's work Z.
    ZFromReference,
    /// Record reference and Set Z from reference with the touch plate on
    /// the reference surface: probes down onto it.
    ProbeReference,
    ProbeFromReference,
    /// For a tool run that turns A: the tool touches the A centreline's Z
    /// reference with the touch plate, which sets its work Z on the
    /// centreline (LIO-207, `Connection::a_reference`).
    ProbeCentreReference,
    /// Set Off the coolant the profile has no output for (`coolant_off`),
    /// then check again.
    CoolantOff,
    /// Skip operation `oi` (index in the setup) in this run as `Skip` says,
    /// or (None) skip it no more: the setup is checked again (`Run::skip`).
    Skip(usize, Option<Skip>),
    /// Open the skips of operation `oi` (index in the setup) on its row of
    /// the Operations list, or close them (None).
    More(Option<usize>),
    /// Unfold the steps of operation `oi` (index in the setup) under its
    /// row, or fold them (None).
    Steps(Option<usize>),
    /// Pick step `s` of operation `oi`: with Shift held, the steps from the
    /// one picked before to it (`Run::pick`).
    PickStep(usize, usize),
    /// Pick no step.
    ClearSteps,
    /// Take the steps picked as cut, or (false) skip none of that
    /// operation's steps: the setup is checked again (`Run::skip_steps`).
    SkipSteps(bool),
    /// Start at the first step picked: the steps and operations before it
    /// are taken as cut (`Run::start_step`).
    StartStep,
    /// The tool in the spindle is the one the steps before a start at a
    /// step were cut with, its Z unchanged since (`Run::tool_kept`).
    KeepToolZ,
    /// Arm Stop after step while a tool run runs, or disarm it
    /// (`Run::stop_after`).
    StopAfterStep,
}

/// How a run skips an operation of its setup (`Run::skips`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Skip {
    /// It is not sent, and the operations after it take its stock as cut,
    /// as those before a Start here are (`post::checked_to_start`).
    Cut,
    /// It is not sent, and its material stays: the setup is checked without
    /// it, as if it were disabled.
    Left,
}

/// The tool run the machine is cutting.
pub struct Running {
    pub index: usize,
    /// The operation (index in its program) it started at.
    pub from: usize,
    /// The driver's outcome count before it started: the next outcome is its.
    seq: u64,
    /// The work offset it was lowered with.
    pub offset: [f64; 6],
    /// What Cycle Start recorded over (`xy`, `zero`, `last`, `touched`), put
    /// back when the driver refuses the job: nothing ran then (#168).
    before: Before,
    /// The line each of its blocks moves the tool tip along (`paths`),
    /// from where the tool tip was as it started (plan coordinates).
    paths: Vec<Option<[[f32; 3]; 2]>>,
    tip: [f32; 3],
    /// The machine position it started from.
    start: [f64; 6],
    /// `program_print` of what it sent.
    print: u64,
    /// `op_print` of each operation it took as already cut (`Skip::Cut`).
    cut_ops: Vec<(usize, u64)>,
    /// The block the tool tip is on and how far along it (`track`).
    on: Option<(usize, f32)>,
    /// The step of each of its blocks (`step_marks`).
    marks: Vec<Option<Mark>>,
}

/// Where a block of a running program is in the steps of its operation:
/// the operation (index in the program), the step of it as sent (counted
/// in the program, the steps taken as cut not sent), and whether the tool
/// has risen to the operation's clearance since that step's last feed
/// move: its cutting done.
type Mark = (usize, usize, bool);

/// What `step_marks` goes by of one operation of a program.
#[derive(Debug, Clone, PartialEq)]
struct MarkOp {
    /// Its clearance (`OpSteps::clear`).
    clear: f32,
    /// A drilling one's holes as the program sends them, each as how many
    /// feed moves it has (`Step::feeds`): its steps are these, whatever the
    /// tool does in a hole between them, a deep drilling's pecks or a
    /// helical milling's rings (LIO-549, LIO-555). None for any other.
    holes: Option<Vec<usize>>,
}

/// The `Mark` of each of `blocks`, of the operations `ops` (by index in
/// the program; None for one without steps): a step starts at the first
/// feed move after the tool, between two feed moves, rose to the clearance
/// (`plan_steps`), a retract to the clearance plane rising too; but a
/// drilling operation's feed move that does not start one of its holes is
/// in the step of the hole it is in, which is not done until the tool
/// rises out of the hole for the next (LIO-549, LIO-555).
fn step_marks(blocks: &[Block], ops: &[Option<MarkOp>]) -> Vec<Option<Mark>> {
    let n = ops.len();
    let op_at = |b: &Block, last: Option<usize>| {
        match b {
            Block::Retract | Block::Stop | Block::End => last,
            _ => op_of(b),
        }
        .filter(|&o| o < n)
    };
    // Whether the next feed move after each block stays in a drilling
    // operation's hole: one that does not start a hole, counted by its feed
    // moves.
    let mut in_hole_ahead = vec![false; blocks.len()];
    {
        // Each drilling operation's feed moves its holes start at.
        let starts: Vec<Option<Vec<usize>>> = (ops.iter())
            .map(|o| {
                let holes = o.as_ref()?.holes.as_ref()?;
                Some(
                    (holes.iter())
                        .scan(0, |at, &feeds| {
                            let start = *at;
                            *at += feeds;
                            Some(start)
                        })
                        .collect(),
                )
            })
            .collect();
        let mut feeds = vec![0; n];
        let mut in_hole = vec![false; blocks.len()];
        for (i, b) in blocks.iter().enumerate() {
            if let Block::Move {
                op,
                kind: Kind::Feed,
                ..
            } = b
                && *op < n
            {
                in_hole[i] = starts[*op]
                    .as_ref()
                    .is_some_and(|s| s.binary_search(&feeds[*op]).is_err());
                feeds[*op] += 1;
            }
        }
        let mut ahead = false;
        for (i, b) in blocks.iter().enumerate().rev() {
            in_hole_ahead[i] = ahead;
            if matches!(
                b,
                Block::Move {
                    kind: Kind::Feed,
                    ..
                }
            ) {
                ahead = in_hole[i];
            }
        }
    }
    let (mut count, mut fed, mut high) = (vec![0; n], vec![false; n], vec![false; n]);
    let mut last = None;
    (blocks.iter().enumerate())
        .map(|(i, b)| {
            let op = op_at(b, last)?;
            match b {
                Block::Move {
                    kind: Kind::Feed, ..
                } => {
                    if fed[op] && high[op] {
                        count[op] += 1;
                    }
                    (fed[op], high[op], last) = (true, false, Some(op));
                }
                Block::Move { from, to, .. } => {
                    let c = ops[op].as_ref().map_or(f32::INFINITY, |o| o.clear);
                    high[op] |=
                        fed[op] && !in_hole_ahead[i] && from[2].max(to[2]) >= c - AT_CLEARANCE;
                }
                Block::Retract => high[op] |= fed[op] && !in_hole_ahead[i],
                _ => {}
            }
            Some((op, count[op], high[op]))
        })
        .collect()
}

/// The run's record of its tool runs before a Cycle Start.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Before {
    xy: Option<(usize, [f64; 2])>,
    zero: Option<(String, [f32; 3])>,
    last: Option<(String, f64)>,
    touched: bool,
}

/// The driver's outcome of a job it refused before anything moved: "<name>:
/// not started, <why>" (the machine not idle, Override limits on, a move
/// outside the soft limits).
pub const NOT_STARTED: &str = "not started";

/// What the Run panel says of a setup whose check waits (`stale` and not
/// checked): for the machine to stop (#167), or for the edits to pause.
pub const CHECK_WAITS: &str = "The setup is checked once the machine stops and the edits pause.";

/// Why Cycle Start waits while a link to the Pi that was live is not
/// answering (`Connection::pi_lost`).
pub const PI_LOST: &str = "The link to the Pi is not answering: wait for it to reconnect.";

/// Why Cycle Start waits while this computer connects to the Pi and the
/// link was never live yet: nothing was lost (LIO-519).
pub const PI_CONNECTING: &str = "Connecting to the Pi: wait for it.";

/// What the Run panel says while a Cycle Start press waits for the Pi to
/// apply what this computer sent before it (`Connection::cycle_pending`).
pub const PI_APPLYING: &str =
    "Cycle Start waits for the Pi to apply what this computer sent before it…";

/// The same on USB and the simulator: the controller takes a command
/// within milliseconds.
pub const CONTROLLER_TAKING: &str =
    "Cycle Start waits for the controller to take what was sent before it…";

/// Why Cycle Start waits while machine positions are declared axis by axis
/// (`Connection::declaring_machine`).
pub const DECLARING: &str = "The machine position is being set, axis by axis: wait for it.";

/// Why a run that starts partway cannot touch off or probe its work zero
/// when the check did not work out the stock its start leaves.
pub const PARTWAY_UNKNOWN: &str =
    "The check did not work out the stock the start leaves: check the setup again.";

/// What cut the stock a run started partway starts from, when operations
/// before its start did (`Run::cut_by`).
const CUT_BY_OPERATIONS: &str = "the operations before the start";

/// What steps taken as cut in the start's operation, the setup's last
/// operation sent, cut, said as `Partly::cut`: the check knows its stock
/// only before that operation, none after it (`Run::partly`).
pub(crate) const STEPS_UNFOLLOWED: &str =
    "stock cut by the steps taken as cut, which the check does not follow";

/// A setup being run on the connected machine, tool run by tool run.
pub struct Run {
    pub setup: usize,
    pub(crate) setup_id: String,
    pub name: String,
    /// Its checked programs, or why it cannot run; None while checking.
    pub programs: Option<Arc<Result<Vec<Checked>, Vec<String>>>>,
    /// Which check's answer is awaited.
    pub epoch: u64,
    /// The project changed since the check.
    pub stale: bool,
    /// The tool run Cycle Start starts.
    pub next: usize,
    /// The operation of it (index in its program) Cycle Start starts at:
    /// those before it are taken as cut.
    pub from: usize,
    /// The work zero was set for the stock the start leaves since it was
    /// chosen, or kept (`Action::KeepZero`): what a run that starts partway
    /// waits for (`partway`).
    pub zeroed: bool,
    pub done: Vec<bool>,
    pub running: Option<Running>,
    /// The work system and X/Y work offset when the first tool run started:
    /// tool changes keep X/Y zero.
    xy: Option<(usize, [f64; 2])>,
    /// The setup's work zero when the first tool run started (`Checked::origin`
    /// and `datum`): what the machine's work offset was touched off for.
    zero: Option<(String, [f32; 3])>,
    /// The reference surface's work Z.
    pub reference: Option<f64>,
    /// A's work position when the reference was recorded, on a machine
    /// with A: a surface that turns with A is touched off at that angle
    /// again (`reference_a_problem`).
    pub reference_a: Option<f64>,
    /// The tool of the last tool run started and the Z work offset then.
    last: Option<(String, f64)>,
    /// The tool last touched off on the A centreline's Z reference, how
    /// much longer than the probe the centreline was probed with it is
    /// (mm, LIO-207), and the reference's machine Z it was measured from:
    /// the only relation of a tool's tip to the centreline that a tool run
    /// turning A runs with (`rotary_problems`), while that reference is the
    /// one kept with the centreline.
    pub length: Option<(String, f64, f64)>,
    /// Z was touched off on the Control page since the last tool run started.
    pub touched: bool,
    /// How the last tool run ended (ok, the driver's message).
    pub outcome: Option<(bool, String)>,
    /// The stock after each of the setup's operations, by its index in the
    /// setup, pictured for the Tool runs panel (`op_pictures`): empty until
    /// worked out after the check.
    pub pictures: Vec<(usize, iced::widget::image::Handle)>,
    /// The stock the check left, until it is pictured (`App::picture_run`).
    pub left: Option<Arc<Left>>,
    /// The stock before each operation of the setup, from the check
    /// (`Left::before`): where the live carve starts.
    pub before: Arc<Vec<OpStock>>,
    /// The stock as the machine's tool has cut it during this run's tool
    /// runs (`carve`), from the stock before the operation the first started
    /// at. None until a tool run moves, and again once the start is changed.
    pub carving: Option<Carving>,
    /// How long its tool runs take (`Times`), worked out with the check:
    /// None while checking, or when the setup cannot run.
    pub times: Option<Arc<Times>>,
    /// The operations it skips (index in the setup), each how (`skip`).
    pub skips: Vec<(usize, Skip)>,
    /// The tool runs as the last check without a skip found them: the
    /// Operations list's rows, skipped ones too.
    pub layout: Option<Arc<Result<Vec<Checked>, Vec<String>>>>,
    /// The operations whose material cannot be left (`Skip::Left`), each
    /// with why (`Left::beneath`).
    pub beneath: Vec<(usize, String)>,
    /// What was done and where Cycle Start starts, by operation (index in
    /// the setup), when a skip asked for the check: kept over its answer.
    keep: Option<(Vec<usize>, Option<usize>)>,
    /// The operation (index in the setup) whose row shows its skips
    /// (`Action::More`).
    pub menu: Option<usize>,
    /// The steps of the setup's operations, from the last check (`Left::steps`).
    pub steps: Vec<OpSteps>,
    /// The steps it takes as cut, by operation (index in the setup): not
    /// sent, the steps after them re-entered from the clearance (`cut_plan`).
    pub cut_steps: Vec<(usize, Vec<usize>)>,
    /// The step Cycle Start starts at, of an operation (index in the setup),
    /// after 0: the steps before it are taken as cut, and its operation is
    /// the start (`next`, `from`).
    pub start_step: Option<(usize, usize)>,
    /// The operation whose steps are unfolded under its row.
    pub open: Option<usize>,
    /// The steps picked, of an operation: the first and last picked.
    pub picked: Option<(usize, usize, usize)>,
    /// The tool's Z was kept for a start at a step (`Action::KeepToolZ`).
    pub tool_kept: bool,
    /// The setup is to be checked again, once the machine stands still: a
    /// start at a step was set from where a tool run stopped.
    pub recheck: bool,
    /// Stop after step is armed: the running tool run stops once the step
    /// it cuts ends, the tool back at its operation's clearance.
    pub stop_after: bool,
    /// The step the driver was told to stop after (Stop sent at its end):
    /// its operation (index in the setup) and step.
    stopping: Option<(usize, usize)>,
    /// The Pi made that stop with no computer in control (this one lost
    /// for its link-lost time), and lifted Z itself.
    unattended: bool,
    /// Where the last tool run stopped short, until the connection keeps it
    /// (`Connection::observe`): in the journal, and as the next start.
    pub stopped: Option<Stopped>,
    /// A tool run ended done since the connection last looked: the journal
    /// of where one stopped is over.
    pub completed: bool,
    /// The project's title, for the journal.
    project: String,
    /// The journal of where a run of this setup stopped, read when it was
    /// picked: a start there once the check finds its program unchanged
    /// (`checked_as`).
    pub journal: Option<Journal>,
    /// The journal the start Cycle Start starts at came from, and what
    /// the check found of one that no longer holds.
    pub resumed: Option<Journal>,
    pub journal_note: Option<String>,
    /// The tool run on the Pi this run is to follow once checked, AXIS
    /// CAM having been closed since it was sent (`following`).
    pub follow: Option<Follow>,
    /// Whether it follows it: the name it was sent as, or why not
    /// (`follow_again`), until the connection takes it.
    pub followed: Option<Result<String, String>>,
    /// What `following` found of the operations taken as cut, for
    /// `journal_note` once it follows.
    cut_note: Option<String>,
    /// The check without its skips, for the Operations list's rows, failed:
    /// the rows are made from its programs once checked
    /// (`layout_with_skips`).
    pub rows_from_skips: bool,
}

/// What a tool run sent to the Pi was sent from (`Run::follow_record`),
/// kept on disk with it: AXIS CAM opened again builds the run anew from it
/// and follows the tool run, while the check finds the program sent
/// (`program`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Follow {
    /// The setup's id, and its name and its project's as they were.
    pub setup: String,
    #[serde(default)]
    pub setup_name: String,
    #[serde(default)]
    pub project: String,
    pub skips: Vec<(usize, Skip)>,
    pub cut_steps: Vec<(usize, Vec<usize>)>,
    pub start_step: Option<(usize, usize)>,
    /// The tool run (`Running::index`) and the operation it started at.
    pub index: usize,
    pub from: usize,
    /// Each tool run's, `program_print` of its whole program (no actions)
    /// once done before it: done again only while it is unchanged.
    pub done: Vec<Option<u64>>,
    /// The work offset it was sent with, and where the tool tip was then.
    pub offset: [f64; 6],
    pub tip: [f32; 3],
    /// The machine position it was sent from.
    pub start: [f64; 6],
    /// `program_print` of what it sends: its moves, its spindle, coolant
    /// and tools, and its operations' cutters.
    pub program: u64,
    /// `op_print` of each operation taken as already cut (`Skip::Cut`).
    #[serde(default)]
    pub cut_ops: Vec<(usize, u64)>,
    /// The run's record before and after its Cycle Start.
    pub before: Before,
    pub xy: Option<(usize, [f64; 2])>,
    pub zero: Option<(String, [f32; 3])>,
    pub last: Option<(String, f64)>,
    /// The name it was sent as, which the Pi's Stop after step names.
    pub name: String,
}

/// The Operations list's rows when the setup does not check without its
/// skips (an operation whose material is left made one it cannot send):
/// the operations `programs` sends and those of setup `si` that `skips`
/// skips, in the setup's order, a tool run ending where the tool changes
/// or a sent tool run ends. So a skipped operation joins the tool run of
/// the one beside it with its tool, and one of another tool between two
/// sent together splits them, as the check without skips does. What a
/// skipped one would send is not known, only its name and tool.
pub fn layout_with_skips(
    doc: &Document,
    si: usize,
    programs: &[Checked],
    skips: &[(usize, Skip)],
) -> Vec<Checked> {
    let (Some(setup), Some(first)) = (doc.setups.get(si), programs.first()) else {
        return programs.to_vec();
    };
    // Each operation: its index in the setup, what it runs, and the tool
    // run that sends it (None: skipped).
    let mut all: Vec<(usize, crate::post::OpState, Option<usize>)> = (programs.iter().enumerate())
        .flat_map(|(i, p)| {
            (p.indexes.iter().zip(&p.ops)).map(move |(&oi, o)| (oi, o.clone(), Some(i)))
        })
        .collect();
    for &(oi, _) in skips {
        let Some(op) = setup.operations.get(oi) else {
            continue;
        };
        if all.iter().any(|a| a.0 == oi) {
            continue;
        }
        let tool = doc.tools.iter().find(|t| t.id == op.tool);
        let state = crate::post::OpState {
            name: op.name().into(),
            strategy: op.strategy.name(),
            tool: tool.and_then(|t| t.number()).unwrap_or(0),
            tool_id: op.tool.clone(),
            tool_name: tool.map_or_else(String::new, |t| t.name().into()),
            diameter: tool
                .and_then(|t| crate::num(&t.p, "diameter"))
                .unwrap_or(0.),
            stickout: 0.,
            rpm: crate::num(&op.p, "rpm").unwrap_or(0.),
            clockwise: true,
            coolant: (false, false),
        };
        all.push((oi, state, None));
    }
    all.sort_by_key(|a| a.0);
    let mut groups: Vec<Vec<(usize, crate::post::OpState, Option<usize>)>> = vec![];
    for item in all {
        let apart = groups.last().and_then(|g| g.last()).is_none_or(|last| {
            last.1.tool_id != item.1.tool_id
                || matches!((last.2, item.2), (Some(a), Some(b)) if a != b)
        });
        if apart {
            groups.push(vec![item]);
        } else {
            groups.last_mut().expect("a group").push(item);
        }
    }
    (groups.into_iter())
        .map(|g| {
            // The tool run that sends its operations, as it is when it is
            // all of them; else a row of their own.
            let mut row = g
                .iter()
                .find_map(|a| a.2)
                .map_or(first, |i| &programs[i])
                .clone();
            let indexes: Vec<usize> = g.iter().map(|a| a.0).collect();
            if row.indexes != indexes {
                row.blocks.clear();
                (row.indexes, row.ops) = (indexes, g.into_iter().map(|a| a.1).collect());
            }
            row
        })
        .collect()
}

/// FNV-1a over operation `oi` of setup `si` and its tool as the project
/// saves them: what an operation taken as cut was (`Follow::cut_ops`).
fn op_print(doc: &Document, si: usize, oi: usize) -> Option<u64> {
    let op = doc.setups.get(si)?.operations.get(oi)?;
    let tool = doc.tools.iter().find(|t| t.id == op.tool);
    let text = serde_json::to_string(&(op, tool)).ok()?;
    let mut h = 0xcbf29ce484222325u64;
    for b in text.bytes() {
        h = (h ^ u64::from(b)).wrapping_mul(0x100000001b3);
    }
    Some(h)
}

/// `op_print` of each operation of setup `si` taken as cut in `skips`.
fn cut_prints(doc: &Document, si: usize, skips: &[(usize, Skip)]) -> Vec<(usize, u64)> {
    (skips.iter())
        .filter(|s| s.1 == Skip::Cut)
        .filter_map(|&(oi, _)| Some((oi, op_print(doc, si, oi)?)))
        .collect()
}

/// Why AXIS CAM opened again does not follow the tool run on the Pi
/// (`Run::follow_again`).
fn not_followed(why: &str) -> String {
    format!(
        "The tool run on the Pi is not followed: {why}. The machine carries on; Hold, Stop and E-stop still act on it."
    )
}

/// FNV-1a over what tool run `p` sends from operation `from` on
/// (`actions`: its moves, spindle, coolant, tool changes and pauses), its
/// blocks, and its operations with their tools and cutters (`OpState`):
/// the program sent, stable across builds (`Follow::program`). None when
/// it cannot start there.
pub fn program_print(p: &Checked, from: usize, actions: &[Located]) -> Option<u64> {
    struct Fnv(u64);
    impl std::fmt::Write for Fnv {
        fn write_str(&mut self, s: &str) -> std::fmt::Result {
            for b in s.bytes() {
                self.0 = (self.0 ^ u64::from(b)).wrapping_mul(0x100000001b3);
            }
            Ok(())
        }
    }
    use std::fmt::Write;
    let mut h = Fnv(0xcbf29ce484222325);
    write!(
        h,
        "{from} {:?} {:?} {:?} {:?} {} {};",
        p.indexes, p.ops, p.datum, p.settings, p.offset, p.origin
    )
    .ok()?;
    for b in p.from(from).ok()? {
        write!(h, "{b:?};").ok()?;
    }
    for a in actions {
        write!(h, "{a:?};").ok()?;
    }
    Some(h.0)
}

/// Where a tool run stopped short (LIO-275): after a step (Stop after
/// step), or in one (an E-stop, the connection lost, the app closed),
/// whose step then counts as not done.
#[derive(Debug, Clone, PartialEq)]
pub struct Stopped {
    /// The operation it was in (index in the setup), and its last step
    /// done; None for none.
    pub op: usize,
    pub last: Option<usize>,
    /// After a step: the machine Z of the clearance plane, where the tool
    /// is lifted to once the machine stops.
    pub up: Option<f64>,
    /// Why: "Stop after step", the driver's message.
    pub why: String,
}

/// The journal of where a run stopped (LIO-275), kept on disk beside the
/// machine's saved position (`Connection::journal`): it survives closing
/// the app, and holds only while the operation's program is the one its
/// fingerprint is of (`OpSteps::fingerprint`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Journal {
    pub project: String,
    /// The setup's id and name.
    pub setup: String,
    pub setup_name: String,
    /// The operation it stopped in (index in the setup), its name, how
    /// many steps it has and its last step done (None: none).
    pub op: usize,
    pub op_name: String,
    pub steps: usize,
    pub step: Option<usize>,
    /// The work system and work offset the steps were cut with.
    pub wcs: usize,
    pub offset: [f64; 6],
    /// The tool's id and label.
    pub tool: String,
    pub tool_label: String,
    /// When, in seconds since 1970.
    pub time: u64,
    pub fingerprint: u64,
    /// Why it stopped.
    pub why: String,
}

impl Journal {
    pub fn read(path: &std::path::Path) -> Option<Self> {
        serde_json::from_slice(&std::fs::read(path).ok()?).ok()
    }

    pub fn write(&self, path: &std::path::Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let data = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        crate::storage::write_atomic(path, &data)
    }

    /// How long ago it stopped, at `now` (seconds since 1970): "12 min
    /// ago", "3 h ago", "2 days ago".
    pub fn ago(&self, now: u64) -> String {
        let s = now.saturating_sub(self.time);
        match s {
            0..120 => "just now".into(),
            120..7200 => format!("{} min ago", s / 60),
            7200..172800 => format!("{} h ago", s / 3600),
            _ => format!("{} days ago", s / 86400),
        }
    }
}

/// Seconds since 1970, now.
pub fn now() -> u64 {
    (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)).map_or(0, |d| d.as_secs())
}

/// The stock before an operation of the setup being run, seen from above,
/// and the shape of its cutter.
#[derive(Debug, Clone)]
pub struct OpStock {
    /// The operation's index in the setup.
    pub op: usize,
    pub stock: StockGrid,
    pub shape: CutterShape,
    /// The stock as the simulation holds it (`Simulation::initial_volume`),
    /// its cutter's flute length, the frame its moves are in
    /// (`Simulation::frame`) and the frame the part is posed in at each A
    /// angle (`Simulation::turns`, empty for a 3-axis setup): the live carve
    /// cuts this, at each move's angle, and the 3D view draws its surface.
    pub volume: Option<crate::volume::Volume>,
    pub flute: f32,
    pub frame: Option<crate::volume::Frame>,
    pub turns: Vec<(f32, crate::volume::Frame)>,
}

/// The stock a run's tool runs have cut, as the machine's tool moved: each
/// tick, the moves of the program it has passed since the last tick whole,
/// the move it is on up to the tool's tip, and the tool where it is.
#[derive(Debug, Clone)]
pub struct Carving {
    /// Which carve this is, for its surface (`RunMeshKey`): none other has
    /// the same, while each starts its versions from 0.
    pub serial: u64,
    pub stock: StockGrid,
    /// The stock as a volume, with every cut made in it, cut at each move's
    /// A in the frame the part is posed in there (in `frame` without A):
    /// what its surface is meshed from (`surface`). None where the check
    /// had none. For a tool run that turns A, the top-view `stock` is not
    /// cut, being the first angle's.
    pub volume: Option<crate::volume::Volume>,
    /// The tool run turns A.
    turning: bool,
    frame: Option<crate::volume::Frame>,
    turns: Vec<(f32, crate::volume::Frame)>,
    /// The surface of `volume` as last meshed (`App::refresh_run_mesh`),
    /// with the version and detail it was meshed at: drawn until the next
    /// is meshed, as the simulation's is.
    pub surface: Option<(
        u64,
        crate::stock_surface::Detail,
        Arc<crate::stock_surface::SurfaceMesh>,
    )>,
    /// The block of the running blocks at the last sample: None between
    /// tool runs, so that a jog then does not cut.
    last: Option<usize>,
    /// Where the last sample's cut ended: its block, the point on its move
    /// it was cut up to and the tool's tip. A sample that finds the tool
    /// there again (held, paused, a snapshot repeated) cuts nothing, and
    /// one further along the same move cuts only the rest of it.
    reached: Option<(usize, [f32; 3], [f32; 3])>,
    /// Counts the cuts, for the 3D view's redraw.
    pub version: u64,
}

impl Run {
    pub fn new(doc: &Document, setup: usize, epoch: u64) -> Self {
        let s = &doc.setups[setup];
        Self {
            setup,
            setup_id: s.id.clone(),
            name: s.name().to_string(),
            programs: None,
            epoch,
            stale: false,
            next: 0,
            from: 0,
            zeroed: false,
            done: vec![],
            running: None,
            xy: None,
            zero: None,
            reference: None,
            reference_a: None,
            last: None,
            length: None,
            touched: false,
            outcome: None,
            pictures: vec![],
            left: None,
            before: Arc::new(vec![]),
            carving: None,
            times: None,
            skips: vec![],
            layout: None,
            beneath: vec![],
            keep: None,
            menu: None,
            steps: vec![],
            cut_steps: vec![],
            start_step: None,
            open: None,
            picked: None,
            tool_kept: false,
            recheck: false,
            stop_after: false,
            stopping: None,
            unattended: false,
            stopped: None,
            completed: false,
            project: doc.title.clone(),
            journal: None,
            resumed: None,
            journal_note: None,
            follow: None,
            followed: None,
            cut_note: None,
            rows_from_skips: false,
        }
    }

    /// The run of `follow`'s setup (index `setup`) as it was when its tool
    /// run was sent to the Pi: its skips, steps taken as cut and start at a
    /// step, so its check finds the program sent again. Once checked, it
    /// follows that tool run (`follow_again`).
    pub fn following(doc: &Document, setup: usize, epoch: u64, follow: Follow) -> Self {
        let mut r = Self::new(doc, setup, epoch);
        // An operation taken as cut that has changed since: said, since
        // what was cut is the operation as it was.
        let now = cut_prints(doc, setup, &follow.skips);
        let changed: Vec<&str> = (follow.cut_ops.iter())
            .filter(|c| !now.contains(c))
            .filter_map(|&(oi, _)| Some(doc.setups[setup].operations.get(oi)?.name()))
            .collect();
        if !changed.is_empty() {
            r.cut_note = Some(format!(
                "Skipped as already cut, {} changed since: what is cut is the operation as it was. Don't skip it to cut it as it is now.",
                changed.join(", ")
            ));
        }
        r.skips = follow.skips.clone();
        r.cut_steps = follow.cut_steps.clone();
        r.start_step = follow.start_step;
        r.follow = Some(follow);
        r
    }

    /// What the tool run Cycle Start just started (`cycle_start`) was sent
    /// from, sent as `name`: kept on disk with it, so AXIS CAM opened again
    /// follows it (`following`).
    pub fn follow_record(&self, name: &str) -> Option<Follow> {
        let r = self.running.as_ref()?;
        let programs = self.checked()?;
        Some(Follow {
            setup: self.setup_id.clone(),
            setup_name: self.name.clone(),
            project: self.project.clone(),
            skips: self.skips.clone(),
            cut_steps: self.cut_steps.clone(),
            start_step: self.start_step,
            index: r.index,
            from: r.from,
            done: (programs.iter().zip(&self.done))
                .map(|(p, &d)| d.then(|| program_print(p, 0, &[])).flatten())
                .collect(),
            offset: r.offset,
            tip: r.tip,
            start: r.start,
            program: r.print,
            cut_ops: r.cut_ops.clone(),
            before: r.before.clone(),
            xy: self.xy,
            zero: self.zero.clone(),
            last: self.last.clone(),
            name: name.to_owned(),
        })
    }

    /// Follows the tool run `f` was kept for, once the check answered: the
    /// tool runs done before it are done again, and it runs, as after its
    /// Cycle Start, while the check finds the program sent. The name it was
    /// sent as, or why it is not followed.
    fn follow_again(&mut self, f: Follow) -> Result<String, String> {
        let why = not_followed;
        let programs = match self.programs.as_deref() {
            Some(Ok(p)) => p,
            Some(Err(e)) => {
                return Err(why(&format!(
                    "the setup does not check now ({})",
                    e.join(" ")
                )));
            }
            None => return Err(why("the setup was not checked")),
        };
        // As Cycle Start sent it: at the step it started at, if any.
        let sent = |p: &Checked| {
            let step = f
                .start_step
                .filter(|(oi, _)| p.indexes.get(f.from) == Some(oi));
            let actions = self.job_actions(p, f.from, f.offset, f.start, step)?;
            program_print(p, f.from, &actions)
        };
        let p = (programs.get(f.index))
            .filter(|p| f.done.len() == programs.len() && sent(p) == Some(f.program))
            .ok_or_else(|| why("its setup's program has changed since it was sent"))?;
        // A tool run done before it is done again only while unchanged.
        let done: Vec<bool> = (programs.iter().zip(&f.done))
            .map(|(p, d)| d.is_some() && *d == program_print(p, 0, &[]))
            .collect();
        let changed: Vec<String> = (f.done.iter().zip(&done).enumerate())
            .filter(|(_, (was, now))| was.is_some() && !**now)
            .map(|(i, _)| format!("{}", i + 1))
            .collect();
        let blocks = p.from(f.from).map_err(|e| why(&e.join(" ")))?;
        let marks = self.marks_of(p, blocks);
        let running = Running {
            index: f.index,
            from: f.from,
            // Its end comes by its number (`Connection::run_outcome`).
            seq: 0,
            offset: f.offset,
            before: f.before,
            paths: paths(blocks, f.tip),
            tip: f.tip,
            start: f.start,
            print: f.program,
            cut_ops: f.cut_ops.clone(),
            on: None,
            marks,
        };
        let mut notes: Vec<String> = self.cut_note.take().into_iter().collect();
        if !changed.is_empty() {
            notes.push(format!(
                "Tool run {} was done, but its program has changed since, so it is not shown done: run it again if it is to be cut as it is now.",
                changed.join(", ")
            ));
        }
        if !notes.is_empty() {
            self.journal_note = Some(notes.join(" "));
        }
        (self.done, self.next, self.from) = (done, f.index, f.from);
        (self.xy, self.zero, self.last) = (f.xy, f.zero, f.last);
        (self.running, self.outcome, self.journal) = (Some(running), None, None);
        Ok(f.name)
    }

    /// The step of each block of tool run `p` from its operation `blocks`
    /// start at (`step_marks`).
    fn marks_of(&self, p: &Checked, blocks: &[Block]) -> Vec<Option<Mark>> {
        // An operation of one step ends it too, once risen to its clearance
        // (LIO-311).
        let ops: Vec<Option<MarkOp>> = (p.indexes.iter())
            .map(|&oi| Some(self.op_steps(oi)?.mark_op(&self.cut_of(oi))))
            .collect();
        step_marks(blocks, &ops)
    }

    /// Cuts the stock the tool has gone through since the last sample, while
    /// a tool run runs (`live`): every move of the program from the one it
    /// was on (from the first block at the tool run's first sample) up to
    /// the one it is on now, that one up to the tool's tip, and the tool
    /// where it is. So a tick that comes late still cuts every pass. The
    /// move it is on is the one nearest the tip up to the driver's line,
    /// which runs ahead of the tool by its motion buffer. The first sample
    /// of a run's first tool run starts from the stock before the operation
    /// it started at.
    pub fn carve(&mut self, s: &uc100::controller::Snapshot) {
        let Some(live) = self.live(s) else {
            if let Some(c) = &mut self.carving {
                (c.last, c.reached) = (None, None);
            }
            return;
        };
        let Some((_, _, line)) = self.progress(s.current_line) else {
            return;
        };
        if !self.start_carving() {
            return;
        }
        let last = self.carving.as_ref().and_then(|c| c.last);
        let now = self.nearest(last.unwrap_or(0), line, live.tip);
        self.sweep(last, Some((now, live.tip)));
        if let Some(c) = &mut self.carving {
            c.last = Some(now);
        }
    }

    /// The move of the running blocks `from..=to` nearest `tip`: the first
    /// within a micron, else the nearest. `to` when none is a move.
    fn nearest(&self, from: usize, to: usize, tip: [f32; 3]) -> usize {
        let Some(blocks) = self.running_blocks() else {
            return to;
        };
        let mut best = (to, f32::INFINITY);
        for (i, block) in blocks.iter().enumerate().take(to + 1).skip(from) {
            let Block::Move { from: a, to: b, .. } = block else {
                continue;
            };
            let d: [f32; 3] = std::array::from_fn(|k| b[k] - a[k]);
            let length = d.iter().map(|v| v * v).sum::<f32>();
            let t = if length < 1e-12 {
                0.
            } else {
                ((0..3).map(|k| (tip[k] - a[k]) * d[k]).sum::<f32>() / length).clamp(0., 1.)
            };
            let gap = (0..3)
                .map(|k| (a[k] + t * d[k] - tip[k]).powi(2))
                .sum::<f32>();
            if gap < 1e-6 {
                return i;
            }
            if gap < best.1 {
                best = (i, gap);
            }
        }
        best.0
    }

    /// Starts the carve, unless it has: from the stock before the
    /// operation the running tool run started at. Whether it is under way.
    fn start_carving(&mut self) -> bool {
        if self.carving.is_some() {
            return true;
        }
        let first = (self.running.as_ref())
            .and_then(|r| self.checked()?.get(r.index)?.indexes.get(r.from).copied());
        let Some(b) = self.before.iter().find(|b| Some(b.op) == first) else {
            return false;
        };
        // The stock seen from above is the first angle's: a tool run that
        // turns A carves the volume instead, at each move's angle.
        let turning = (self.running_blocks()).is_some_and(|b| !a_angles(b).is_empty());
        if turning && (b.volume.is_none() || b.turns.is_empty()) {
            return false;
        }
        static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        self.carving = Some(Carving {
            serial: SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            stock: b.stock.clone(),
            volume: b.volume.clone().filter(|_| turning || b.frame.is_some()),
            turning,
            frame: b.frame,
            turns: b.turns.clone(),
            surface: None,
            last: None,
            reached: None,
            version: 0,
        });
        true
    }

    /// Cuts the rest of the running tool run's program from the last
    /// sample (all of it without one): it ended, done, before a tick saw
    /// its last moves.
    fn finish_carve(&mut self) {
        if self.start_carving() {
            let last = self.carving.as_ref().and_then(|c| c.last);
            self.sweep(last, None);
        }
    }

    /// Cuts the moves of the running blocks from block `from` (None: the
    /// first) up to `to` (block and tip; None: to the end), each with its
    /// operation's cutter: those before `to`'s block whole, that one up to
    /// the tip's place on it, and the cutter at the tip.
    fn sweep(&mut self, from: Option<usize>, to: Option<(usize, [f32; 3])>) {
        let (Some(blocks), Some(p)) = (
            self.running_blocks(),
            self.running
                .as_ref()
                .and_then(|r| self.checked()?.get(r.index)),
        ) else {
            return;
        };
        let ends = |b: usize| match blocks.get(b) {
            Some(Block::Move {
                op, from, to, a, ..
            }) => Some((*op, *from, *to, *a)),
            _ => None,
        };
        let now = to.map_or(blocks.len(), |(b, _)| b);
        // Cut on the same move as the last sample: from where that cut ended.
        let reached = (self.carving.as_ref())
            .and_then(|c| c.reached)
            .filter(|r| from == Some(now) && r.0 == now);
        if let (Some((_, tip)), Some((_, _, was))) = (to, reached)
            && tip == was
        {
            return;
        }
        let mut sweeps: Vec<_> = (from.unwrap_or(0)..now).filter_map(ends).collect();
        let mut end = None;
        if let (Some((_, tip)), Some((op, a, b, angle))) = (to, ends(now)) {
            // The tip's place on the move: the share of it done.
            let d: [f32; 3] = std::array::from_fn(|k| b[k] - a[k]);
            let length = d.iter().map(|v| v * v).sum::<f32>();
            let t = if length < 1e-12 {
                1.
            } else {
                ((0..3).map(|k| (tip[k] - a[k]) * d[k]).sum::<f32>() / length).clamp(0., 1.)
            };
            let on: [f32; 3] = std::array::from_fn(|k| a[k] + t * d[k]);
            sweeps.push((op, reached.map_or(a, |r| r.1), on, angle));
            sweeps.push((op, tip, tip, angle));
            end = Some((now, on, tip));
        }
        let cutter = |op: usize| {
            let before = (self.before.iter()).find(|b| Some(b.op) == p.indexes.get(op).copied());
            let radius = p.ops.get(op).map_or(0., |o| o.diameter / 2.);
            let flute = before.map_or(0., |b| b.flute);
            (radius, before.map_or(CutterShape::Flat, |b| b.shape), flute)
        };
        let si = u16::try_from(self.setup).unwrap_or(u16::MAX);
        let sweeps: Vec<_> = (sweeps.into_iter())
            .map(|(op, a, b, angle)| {
                let oi = p.indexes.get(op).copied().unwrap_or(op);
                let source = (si, u16::try_from(oi).unwrap_or(u16::MAX));
                (a, b, cutter(op), angle, source)
            })
            .collect();
        if let Some(c) = &mut self.carving {
            c.reached = end;
            for (a, b, (radius, shape, flute), angle, source) in sweeps {
                if let Some(volume) = &mut c.volume {
                    let frame = if c.turning {
                        (c.turns.iter())
                            .find(|t| angle.is_some_and(|x| (x - t.0).abs() < 1e-4))
                            .map(|t| t.1)
                    } else {
                        c.frame
                    };
                    let Some(frame) = frame else { continue };
                    volume.cut(a, b, radius, flute, shape, frame, source);
                }
                if !c.turning {
                    c.stock.cut_shape(a, b, radius, shape);
                }
                c.version += 1;
            }
        }
    }

    /// The stock as this run's tool runs have cut it, once one has moved
    /// (`carve`), with its version.
    pub fn stock(&self) -> Option<(&StockGrid, u64)> {
        let c = self.carving.as_ref().filter(|c| !c.turning)?;
        Some((&c.stock, c.version))
    }
    /// The stock as this run's tool runs that turn A have cut it, as a
    /// volume (`Carving::volume`), with its version.
    pub fn carved_volume(&self) -> Option<(&crate::volume::Volume, u64)> {
        let c = self.carving.as_ref().filter(|c| c.turning)?;
        Some((c.volume.as_ref()?, c.version))
    }
    /// The stock as this run's tool runs have cut it, as a volume, and the
    /// surface it was last meshed to (`Carving::surface`), once it has been.
    pub fn carved_surface(
        &self,
    ) -> Option<(&crate::volume::Volume, &crate::stock_surface::SurfaceMesh)> {
        let c = self.carving.as_ref()?;
        Some((c.volume.as_ref()?, c.surface.as_ref()?.2.as_ref()))
    }

    /// A tool run has started: X/Y work zero is kept from then on.
    pub fn started(&self) -> bool {
        self.xy.is_some()
    }

    /// Keeps up with the project after an edit: the setup's index, since
    /// setups may have been moved. None once the setup was removed: the run
    /// then has nothing to check, and says so instead of its programs.
    pub fn follow(&mut self, doc: &Document) -> Option<usize> {
        match doc.setups.iter().position(|s| s.id == self.setup_id) {
            Some(i) => {
                self.setup = i;
                Some(i)
            }
            None => {
                // Out of range, so that no setup the project has is shown as picked.
                self.setup = usize::MAX;
                self.checked_as(Arc::new(Err(vec![REMOVED.into()])), None, None);
                None
            }
        }
    }

    /// The checked programs, once the check passed.
    pub fn checked(&self) -> Option<&[Checked]> {
        self.programs.as_deref()?.as_deref().ok()
    }

    /// A check's answer, the stock it left to picture and how long its tool
    /// runs take.
    pub fn checked_as(
        &mut self,
        programs: Arc<Result<Vec<Checked>, Vec<String>>>,
        left: Option<Arc<Left>>,
        times: Option<Arc<Times>>,
    ) {
        // A start at a step, or steps taken as cut, that the check refuses:
        // dropped, and the setup checked again from that operation's start,
        // so that it never stays without a start to pick (LIO-311).
        if let Err(why) = programs.as_ref()
            && (self.start_step.is_some() || !self.cut_steps.is_empty())
            && why.first().is_none_or(|w| w != REMOVED)
        {
            self.journal_note = Some(format!(
                "The start at a step or the steps taken as cut could not be checked, so they are dropped and the operation starts at its first step: {} Pick another start in the Operations list if you like.",
                why.join(" ")
            ));
            (self.start_step, self.resumed, self.picked) = (None, None, None);
            self.cut_steps.clear();
            (self.programs, self.recheck) = (None, true);
            // Checked otherwise, it is not the program sent.
            if self.follow.take().is_some() {
                self.followed = Some(Err(not_followed(&format!(
                    "the setup does not check now ({})",
                    why.join(" ")
                ))));
            }
            return;
        }
        // Refused, the progress stays kept for the skip's undoing (`skip`).
        let keep = programs
            .as_ref()
            .as_ref()
            .ok()
            .and_then(|_| self.keep.take());
        if let (Some((done, start)), Ok(new)) = (keep, programs.as_ref()) {
            // A skip's check: what was done stays done, and Cycle Start starts
            // where it did, or at the first operation after it still sent.
            self.done = (new.iter())
                .map(|p| p.indexes.iter().all(|oi| done.contains(oi)))
                .collect();
            (self.next, self.from) = start
                .and_then(|start| {
                    (new.iter().enumerate()).find_map(|(i, p)| {
                        let k = p.indexes.iter().position(|&oi| oi >= start)?;
                        Some((i, k))
                    })
                })
                .unwrap_or((new.len(), 0));
        } else {
            // The start stays on its operation only when its tool run still
            // has the same operations; the work zero is set for it again.
            let ops = |p: Option<&Checked>| p.map(|p| (p.indexes.clone(), p.ops.clone()));
            let before = ops(self.checked().and_then(|c| c.get(self.next)));
            let count = programs.as_ref().as_ref().map_or(0, Vec::len);
            if self.done.len() != count {
                self.done = vec![false; count];
                self.next = 0;
            }
            let after = ops(programs
                .as_ref()
                .as_ref()
                .ok()
                .and_then(|c| c.get(self.next)));
            if before.is_none() || before != after {
                self.from = 0;
            }
        }
        if self.skips.is_empty() {
            self.layout = Some(programs.clone());
        }
        self.beneath = left.as_ref().map_or_else(Vec::new, |l| l.beneath.clone());
        self.steps = left.as_ref().map_or_else(Vec::new, |l| l.steps.clone());
        // A start at a step the check did not keep (a step of an operation
        // gone, or no longer in steps) is no start.
        if let Some((oi, n)) = self.start_step
            && self.op_steps(oi).is_none_or(|s| n >= s.steps.len())
            && !self.steps.is_empty()
        {
            self.start_step = None;
        }
        (self.zeroed, self.tool_kept, self.recheck) = (false, false, false);
        self.programs = Some(programs);
        self.before = left
            .as_ref()
            .map_or_else(Default::default, |l| l.before.clone());
        // A check without stock (the smoke run's) is not pictured.
        self.left =
            left.filter(|l| self.checked().is_some() && (!l.after.is_empty() || l.last.is_some()));
        self.times = times.filter(|_| self.checked().is_some());
        self.pictures.clear();
        match self.follow.take() {
            Some(f) => self.followed = Some(self.follow_again(f)),
            None => self.take_journal(),
        }
    }

    /// Where a run of this setup stopped (`journal`), once it is checked:
    /// the start after it, while the program of its operation is the one it
    /// stopped in.
    fn take_journal(&mut self) {
        if self.checked().is_none() {
            return;
        }
        if let Some(j) = self.journal.take() {
            // Its steps as they split then: a build that splits them
            // otherwise (a deep drilling's pecks into its holes, LIO-549)
            // numbers them otherwise.
            let same = self
                .op_steps(j.op)
                .is_some_and(|s| s.fingerprint == j.fingerprint && s.steps.len() == j.steps);
            if same && j.setup == self.setup_id {
                self.resume_after(j.op, j.step);
                self.resumed = Some(j);
            } else {
                self.journal_note = Some(format!(
                    "This setup last stopped in {} ({}), but its program has changed since, so where it stopped is not picked: pick the start in the Operations list.",
                    j.op_name, j.why
                ));
            }
        }
    }

    /// Where a run of this setup stopped, found while it is open (the Pi
    /// stopped a tool run this computer did not follow): its start is the
    /// step after, now or once it is checked.
    pub fn adopt_journal(&mut self, j: Journal) {
        if self.running.is_none() && j.setup == self.setup_id {
            self.journal = Some(j);
            self.take_journal();
        }
    }

    /// The journal of each stop Stop after step on the Pi can make in the
    /// running tool run (`stop_ranges`, one range per step), the machine at
    /// work system `wcs` and work `offset`: kept as it is sent, for when
    /// the Pi stops it with this computer not following (closed and opened
    /// again).
    pub fn pi_stops(
        &self,
        wcs: usize,
        offset: [f64; 6],
    ) -> Vec<(std::ops::RangeInclusive<u32>, Journal)> {
        let n = self.running_blocks().map_or(0, |b| b.len());
        let mut ends: Vec<(std::ops::RangeInclusive<u32>, (usize, usize))> = vec![];
        for block in 0..n {
            let Some(step) = self.step_end(block) else {
                continue;
            };
            // Job line n is block n - 1 (`progress`).
            let line = block as u32 + 1;
            match ends.last_mut() {
                Some((r, s)) if *s == step && *r.end() + 1 == line => *r = *r.start()..=line,
                _ => ends.push((line..=line, step)),
            }
        }
        (ends.into_iter())
            .filter_map(|(r, (op, s))| {
                let st = Stopped {
                    op,
                    last: Some(s),
                    up: None,
                    why: "Stop after step".into(),
                };
                Some((r, self.journal_of(&st, wcs, offset)?))
            })
            .collect()
    }

    /// The journal of the running tool run done, the machine at work
    /// system `wcs` and work `offset`: the next tool run's first operation,
    /// nothing of it done, so the start is there. None when it is the
    /// setup's last. Kept as `pi_stops` is.
    pub fn pi_done(&self, wcs: usize, offset: [f64; 6]) -> Option<Journal> {
        let r = self.running.as_ref()?;
        let op = *self.checked()?.get(r.index + 1)?.indexes.first()?;
        let st = Stopped {
            op,
            last: None,
            up: None,
            why: String::new(),
        };
        self.journal_of(&st, wcs, offset)
    }

    /// The journal of where this run stopped (`stopped`), the machine at
    /// work system `wcs` and work `offset`.
    pub fn journal_of(&self, st: &Stopped, wcs: usize, offset: [f64; 6]) -> Option<Journal> {
        let (p, k) = (self.checked()?.iter())
            .find_map(|p| Some((p, p.indexes.iter().position(|&o| o == st.op)?)))?;
        let o = &p.ops[k];
        let steps = self.op_steps(st.op);
        Some(Journal {
            project: self.project.clone(),
            setup: self.setup_id.clone(),
            setup_name: self.name.clone(),
            op: st.op,
            op_name: o.name.clone(),
            steps: steps.map_or(0, |s| s.steps.len()),
            step: st.last,
            wcs,
            offset,
            tool: o.tool_id.clone(),
            tool_label: crate::model::tool_label(o.tool, &o.tool_name),
            time: now(),
            fingerprint: steps.map_or(0, |s| s.fingerprint),
            why: st.why.clone(),
        })
    }

    /// The picture of the stock operation `op` (its index in the setup)
    /// leaves, once worked out.
    pub fn picture(&self, op: usize) -> Option<&iced::widget::image::Handle> {
        self.pictures.iter().find(|(i, _)| *i == op).map(|(_, p)| p)
    }

    /// Keeps the tool tip's work Z as the reference surface, and on a
    /// machine with A where A is (`reference_a`).
    pub fn record_reference(&mut self, c: &Connection, z: f64) {
        let s = &c.snapshot;
        self.reference = Some(z);
        self.reference_a = (c.axis(3) && s.referenced[3]).then_some(s.work_pos[3]);
    }

    /// Why Set Z from reference (or its probe) cannot be made now: A is not
    /// at the angle the reference was recorded at, so a reference surface
    /// that turns with A (the stock, the chuck) is not where it was. None
    /// when it can, or nothing was recorded of A.
    pub fn reference_a_problem(&self, c: &Connection) -> Option<String> {
        let ra = self.reference_a?;
        let s = &c.snapshot;
        (!s.referenced[3] || (s.work_pos[3] - ra).abs() > A_AT).then(|| {
            format!(
                "Turn A to {ra:.3}° first, where the reference surface was recorded: a surface that turns with A is touched off at the same angle."
            )
        })
    }

    /// The next tool run, when its tool is not the one the last tool run
    /// used: the operator changes the tool and touches off its Z first.
    pub fn tool_change(&self) -> Option<&Checked> {
        let p = self.checked()?.get(self.next)?;
        let (tool, _) = self.last.as_ref()?;
        (self.running.is_none() && *tool != p.ops[0].tool_id).then_some(p)
    }

    /// Notes how the running tool run ended, once the driver reports it. A
    /// job the driver refused ("not started": sent while it re-anchored its
    /// position after a work zero, say) never ran, so the start is undone:
    /// the work zero is not kept from it, and the first tool run's work-zero
    /// steps stay (#168).
    pub fn refresh(&mut self, outcome: Option<&uc100::controller::ProgramOutcome>) {
        let (Some(r), Some(o)) = (&self.running, outcome) else {
            return;
        };
        if o.seq <= r.seq {
            return;
        }
        let stopping = self.stopping.take();
        let unattended = std::mem::take(&mut self.unattended);
        self.stop_after = false;
        let mut message = o.message.clone();
        if o.ok {
            self.finish_carve();
            let Some(r) = &self.running else {
                return;
            };
            self.done[r.index] = true;
            self.next = r.index + 1;
            self.from = 0;
            self.completed = true;
        } else if o.message.contains(NOT_STARTED) {
            let Before {
                xy,
                zero,
                last,
                touched,
            } = r.before.clone();
            (self.xy, self.zero, self.last, self.touched) = (xy, zero, last, touched);
        } else if let Some((op, s)) = stopping {
            // Stopped after a step, the tool at its clearance: lifted to the
            // clearance plane once the machine stands still.
            let up = self.step_lift();
            message = format!("Stopped after {}", self.step_label(op, s));
            if unattended {
                message += ": no computer controlled the machine for a minute, so the Pi stopped there, lifted Z and turned the spindle off. Cycle Start goes on from the next step.";
            }
            self.stopped = Some(Stopped {
                op,
                last: Some(s),
                up,
                why: match unattended {
                    true => "Stop after step: no computer in control for a minute".into(),
                    false => "Stop after step".into(),
                },
            });
        } else if o.message.contains("E-stop") || o.message.contains("E-STOP") {
            // In a step: it counts as not done.
            self.stopped = self.stopped_in("an E-stop");
        }
        self.outcome = Some((o.ok, message));
        self.running = None;
    }

    /// Where the running tool run is, stopped now in a step that then
    /// counts as not done (`why`): the operation and step of the block the
    /// tool tip is on (the first until it is found), the last step done of
    /// it being the one before, or that one if its cutting was done.
    pub fn stopped_in(&self, why: &str) -> Option<Stopped> {
        let r = self.running.as_ref()?;
        let p = self.checked()?.get(r.index)?;
        let block = r.on.map_or(0, |o| o.0);
        let (op, s, done) = match r.marks.get(block).copied().flatten() {
            Some(mark) => match self.mark_step(p, mark) {
                Some(step) => step,
                None => (*p.indexes.get(mark.0)?, 0, false),
            },
            None => (*p.indexes.get(r.from)?, 0, false),
        };
        let steps = self.op_steps(op).map_or(0, |o| o.steps.len());
        let last = if done && steps > 1 {
            Some(s)
        } else {
            s.checked_sub(1)
        };
        Some(Stopped {
            op,
            last: last.filter(|_| steps > 1),
            up: None,
            why: why.into(),
        })
    }

    /// Makes the start after step `last` of operation `oi` (index in the
    /// setup; None: before its first) the one Cycle Start starts at, as
    /// after a stop there (`Stopped`, `Journal`), and unfolds its steps: the
    /// next step, the operation's start, or the next operation sent after
    /// its last. A start at a step is checked first (`recheck`).
    pub fn resume_after(&mut self, oi: usize, last: Option<usize>) {
        let Some(programs) = self.checked() else {
            return;
        };
        let count = self.op_steps(oi).map_or(0, |s| s.steps.len());
        let next = last.map_or(0, |l| l + 1);
        let sent = |o: usize| {
            (programs.iter().enumerate()).find_map(|(i, p)| {
                let k = p.indexes.iter().position(|&x| x == o)?;
                Some((i, k))
            })
        };
        if next > 0 && next < count {
            // Nothing is started until the check answers with its program.
            if self.start_at_step(oi, next) {
                (self.recheck, self.programs) = (true, None);
            }
        } else {
            let after = if next == 0 {
                sent(oi)
            } else {
                (programs.iter().flat_map(|p| &p.indexes))
                    .find(|&&o| o > oi)
                    .and_then(|&o| sent(o))
            };
            match after {
                Some((i, k)) => self.start_at(i, k),
                None => (self.next, self.from) = (programs.len(), 0),
            }
        }
        self.open = (count > 1).then_some(oi);
    }

    /// Whether Stop after step stops the running tool run now (`stop_after`
    /// armed, the machine running, `s` its snapshot), and then which step
    /// it ends: the driver's line is past the last feed move of a step, on
    /// a move that starts at the operation's clearance (across, or the way
    /// down to the next step, all rapids in the air), so that its planner,
    /// which the stop slows down from, is there; the tool, behind it by the
    /// motion buffer, comes up to it and stops.
    /// Paused at a program's pause (M0) that comes after a step's end, as a
    /// Boring's to set the head for the next pass does after the pass's
    /// last visit, it stops there too, the step done, the pause not yet
    /// answered (LIO-558).
    pub fn stop_due(&mut self, s: &uc100::controller::Snapshot) -> bool {
        let paused = matches!(s.state, Some(State::Paused(_)));
        if !self.stop_after || self.stopping.is_some() || !(paused || s.state == Some(State::Run)) {
            return false;
        }
        let Some((_, _, block)) = self.progress(s.current_line) else {
            return false;
        };
        let block = match paused {
            true => match self.pause_after(block) {
                Some(b) => b,
                None => return false,
            },
            false => block,
        };
        let Some(step) = self.step_end(block) else {
            return false;
        };
        self.stopping = Some(step);
        true
    }

    /// The pause (M0) block the machine is paused at, its driver's line on
    /// block `block`: the line stays the last move's, the rise before the
    /// pause, so the pause is the first after it, with no feed move between.
    fn pause_after(&self, block: usize) -> Option<usize> {
        let blocks = self.running_blocks()?;
        for (b, at) in blocks.iter().enumerate().skip(block) {
            match at {
                Block::Pause { .. } => return Some(b),
                Block::Move {
                    kind: Kind::Feed, ..
                } => return None,
                _ => {}
            }
        }
        None
    }

    /// Stop after step on the Pi (`uc100_remote`): the running tool run's
    /// job lines where `stop_due` would stop it, runs of lines next to each
    /// other as one range. The Pi stops the first time it sees one, with or
    /// without this computer.
    pub fn stop_ranges(&self) -> Vec<std::ops::RangeInclusive<u32>> {
        let n = self.running_blocks().map_or(0, |b| b.len());
        let mut out: Vec<std::ops::RangeInclusive<u32>> = vec![];
        for block in (0..n).filter(|&b| self.step_end(b).is_some()) {
            // Job line n is block n - 1 (`progress`).
            let line = block as u32 + 1;
            match out.last_mut() {
                Some(r) if *r.end() + 1 == line => *r = *r.start()..=line,
                _ => out.push(line..=line),
            }
        }
        out
    }

    /// The machine Z the tool is lifted to after Stop after step stops
    /// the running tool run: its clearance plane (machine home's 0).
    pub fn step_lift(&self) -> Option<f64> {
        let r = self.running.as_ref()?;
        let p = self.checked()?.get(r.index)?;
        let plane = clearance_plane(p.from(r.from).ok()?);
        Some(match p.settings.machine_home {
            true => 0.,
            false => plane as f64 - p.datum[2] as f64 + r.offset[2],
        })
    }

    /// The Pi stopped the running tool run after a step, at job line
    /// `line`: the step it ends, for when its outcome comes. `unattended`:
    /// with no computer in control, after this one was lost.
    pub fn stopped_on_pi(&mut self, line: u32, unattended: bool) {
        if self.stopping.is_none()
            && let Some((_, _, block)) = self.progress(Some(line))
        {
            self.stopping = self.step_end(block);
        }
        self.unattended |= unattended && self.stopping.is_some();
    }

    /// The tool run just started never reached the machine (`why`): it is
    /// put back as it was, as one the driver refused (`NOT_STARTED`).
    pub fn not_started(&mut self, why: String) {
        let Some(r) = self.running.take() else {
            return;
        };
        let Before {
            xy,
            zero,
            last,
            touched,
        } = r.before;
        (self.xy, self.zero, self.last, self.touched) = (xy, zero, last, touched);
        (self.stopping, self.stop_after) = (None, false);
        self.outcome = Some((false, why));
    }

    /// A Cycle Start press that waited for the machine was dropped, nothing
    /// sent (`Connection::cycle_pending`): `why` says so where a tool run's
    /// outcome is said.
    pub fn press_dropped(&mut self, why: String) {
        if self.running.is_none() {
            self.outcome = Some((false, why));
        }
    }

    /// The step (operation index in the setup, step) Stop after step stops
    /// after when the driver's line is on block `block` of the running tool
    /// run, if it stops there.
    fn step_end(&self, block: usize) -> Option<(usize, usize)> {
        let r = self.running.as_ref()?;
        let blocks = self.running_blocks()?;
        let mark = r.marks.get(block).copied().flatten()?;
        let p = self.checked().and_then(|c| c.get(r.index))?;
        // The end of an operation is a step's end too (LIO-311): on the way
        // into the next operation sent, before its first feed, the one before
        // it, risen to its clearance after its last feed, did its last step.
        // Its last move may be that rise, which is not yet at the clearance.
        let ended = || {
            for (m, b) in r.marks[..block].iter().zip(&blocks[..block]).rev() {
                let m = (*m)?;
                if m.0 != mark.0 {
                    return (m.0 + 1 == mark.0 && m.2).then_some(m.0);
                }
                if matches!(
                    b,
                    Block::Move {
                        kind: Kind::Feed,
                        ..
                    }
                ) {
                    return None;
                }
            }
            None
        };
        let step = match mark.2 {
            true => self.mark_step(p, mark),
            false => ended().and_then(|k| {
                let oi = *p.indexes.get(k)?;
                let cut = self.cut_of(oi);
                let n = self.op_steps(oi)?.steps.len();
                Some((oi, (0..n).rev().find(|s| !cut.contains(s))?, true))
            }),
        };
        let (op, step, _) = step?;
        // The driver's line is where its planner is, which the stop slows
        // down from: at the clearance, up there or on its way down into the
        // next step, in the air over the stock. A rise to it is not yet there.
        let clear = self.op_steps(op).map_or(f32::INFINITY, |o| o.clear);
        let there = match &blocks[block] {
            Block::Move { kind, from, .. } => {
                *kind != Kind::Feed && from[2] >= clear - AT_CLEARANCE
            }
            Block::Descend { .. } => false,
            _ => true,
        };
        there.then_some((op, step))
    }

    /// Why Cycle Start cannot start the next tool run now; empty when it can.
    pub fn problems(&self, c: &Connection, doc: &Document) -> Vec<String> {
        let Some(setup) = doc.setups.get(self.setup).filter(|s| s.id == self.setup_id) else {
            return vec![REMOVED.into()];
        };
        let programs = match self.programs.as_deref() {
            None if self.stale => return vec![CHECK_WAITS.into()],
            None => return vec!["Checking the setup…".into()],
            Some(Err(problems)) => return problems.clone(),
            Some(Ok(programs)) => programs,
        };
        if self.running.is_some() {
            return vec!["A tool run is running.".into()];
        }
        if c.press_waits() {
            return vec![match c.pi {
                Some(_) => PI_APPLYING.into(),
                None => CONTROLLER_TAKING.into(),
            }];
        }
        if c.pi_down() {
            // Never live since it connected: nothing to reconnect yet
            // (LIO-519).
            if let Some(why) = c.pi_refused() {
                return vec![format!("The Pi refused this computer: {why}")];
            }
            return vec![if c.pi_lost() {
                PI_LOST.into()
            } else {
                PI_CONNECTING.into()
            }];
        }
        if c.watching() {
            return vec!["Another computer controls the machine: Take control first.".into()];
        }
        let Some(p) = programs.get(self.next) else {
            return vec!["Every tool run of this setup has run. Pick one to run it again.".into()];
        };
        let s = &c.snapshot;
        let mut e = vec![];
        if self.stale {
            e.push("The project changed since the setup was checked. Check it again.".into());
        }
        let from = self.from.min(p.ops.len() - 1);
        let blocks = match p.from(from) {
            Ok(blocks) => Some(blocks),
            Err(why) => {
                e.extend(why);
                None
            }
        };
        let first = &p.ops[from];
        let unset = unset_axes(c);
        if let Some((oi, n)) = self.at_step() {
            let step = self.step_label(oi, n);
            if !self.zeroed {
                e.push(format!(
                    "This run starts {} at {step}, the steps before it taken as cut: keep the work zero they were cut with (Keep it, under Start at a step), or set it on the stock they left.",
                    first.name
                ));
            }
            if !unset.is_empty() {
                e.push(format!(
                    "{} {} no machine position, so the work zero the steps before {step} were cut with is not where it was: {} in the Machine tab first.",
                    unset.join(", "),
                    if unset.len() == 1 { "has" } else { "have" },
                    c.place(&unset_ids(c), true, true)
                ));
            }
            if !self.tool_kept && !self.touched {
                e.push(format!(
                    "Confirm {}'s Z for the start at {step}: Same tool, keep Z if it was not taken out since, or Set Z from reference.",
                    crate::model::tool_label(first.tool, &first.tool_name)
                ));
            }
        } else if self.partway() {
            match self.partly() {
                None => e.push(format!("The check did not work out the stock operation {}, {}, starts from: pick its first operation, or check the setup again.", from + 1, first.name)),
                Some(_) if !self.zeroed => {
                    let oi = p.indexes.get(from).copied();
                    let steps: Vec<usize> = oi.map_or_else(Vec::new, |oi| self.cut_of(oi));
                    e.push(match (oi, &steps[..]) {
                        (Some(oi), [first_step, ..]) => format!(
                            "This run starts at {}, with {} of it taken as cut: set the work zero on the stock they left in the Work zero tab, or press Keep this work zero there if it is still the one they were cut with.",
                            first.name,
                            if steps.len() == 1 {
                                self.step_label(oi, *first_step)
                            } else {
                                format!("{} of its steps", steps.len())
                            }
                        ),
                        _ => format!(
                            "This run starts at {}, after operations taken as cut: set the work zero on the stock they left in the Work zero tab, or press Keep this work zero there if it is still the one they were cut with.",
                            first.name
                        ),
                    });
                }
                Some(_) => {}
            }
        }
        if !c.idle() {
            e.push("The machine is not Idle.".into());
        }
        if c.declaring_machine() {
            e.push(DECLARING.into());
        }
        // No machine position on X, Y or Z, or no soft limits in the profile,
        // does not stop Cycle Start (`limits_unchecked`). On A it does: the
        // driver runs nothing while A has soft limits and no position, and a
        // program never moves an A without one.
        let a_ids: Vec<usize> = (3..6).filter(|&a| c.axis(a) && !s.referenced[a]).collect();
        let a_unset: Vec<&str> = a_ids.iter().map(|&a| AXES[a]).collect();
        if !a_unset.is_empty() {
            e.push(format!(
                "{} {} no machine position (every E-stop clears it), so the soft limits are off. {} in the Machine tab, then Cycle Start.",
                a_unset.join(", "),
                if a_unset.len() == 1 { "has" } else { "have" },
                c.place(&a_ids, true, false)
            ));
        }
        if s.limits_override {
            e.push("Turn off Override limit switches first.".into());
        }
        if setup.machine.id != c.machine_id {
            e.push(format!(
                "This setup uses the machine {}, not the connected one.",
                setup.machine.name()
            ));
        }
        let limits: Vec<Option<(f64, f64)>> = (0..3)
            .map(|a| {
                let x = c.config.axes.get(a)?;
                Some((x.soft_min?, x.soft_max?))
            })
            .collect();
        for (a, limit) in limits.iter().enumerate() {
            let Some((lo, hi)) = *limit else {
                continue;
            };
            let travel =
                crate::post_check::machine_mm(&setup.machine, &AXES[a].to_lowercase()) as f64;
            if (travel - (hi - lo)).abs() > 0.01 {
                e.push(format!(
                    "The machine's {} travel in AXIS CAM is {travel:.3} mm, but the UC100 profile's soft limits are {lo:.3} to {hi:.3} mm ({:.3} mm). Make them agree.",
                    AXES[a],
                    hi - lo
                ));
            }
        }
        e.extend(a_problems(&setup.machine, &c.config));
        // A retract to machine home goes to machine Z 0, which the checks
        // take as the top of Z.
        if p.settings.machine_home
            && let Some(top) = limits[2].map(|(_, hi)| hi)
            && top.abs() > 1e-6
        {
            e.push(format!(
                "This setup retracts to machine home, machine Z 0, which the checks take as the top of Z's travel, but the UC100 profile's Z soft limits end at {top:.3} mm. Make Z's soft maximum 0."
            ));
        }
        if let Some(blocks) = blocks {
            e.extend(rotary_problems(
                c,
                doc,
                setup,
                p,
                blocks,
                self.length.as_ref(),
            ));
        }
        match OFFSETS.iter().position(|o| *o == p.offset) {
            Some(i) if i < 6 => {
                if i != s.active_wcs {
                    e.push(format!(
                        "This setup uses {} but the machine's active work system is G{}. Select {} under Work zero.",
                        p.offset,
                        54 + s.active_wcs,
                        p.offset
                    ));
                }
            }
            _ => e.push(format!(
                "{} is not a UC100 work system (G54 to G59).",
                p.offset
            )),
        }
        e.extend(pin_problems(c, setup, p));
        // The program header names the sides by the setup's snapshot of the
        // machine, the Control page by the connected one (LIO-457).
        let own = crate::orient::Orient::of(&setup.machine);
        if own != c.orient {
            e.push(format!(
                "This setup's snapshot of the machine has +X pointing {} but the Machine library has {}: the program header and the Control page would name the stock's sides differently. Use Assign to current setup there, then check the setup again.",
                own.label().to_lowercase(),
                c.orient.label().to_lowercase()
            ));
        }
        if let Some(blocks) = blocks
            && limits.iter().all(Option::is_some)
            && unset.is_empty()
        {
            let span = extents(blocks);
            for (a, limit) in limits.iter().enumerate() {
                let (lo, hi) = limit.unwrap();
                let at = |v: f32| v as f64 - p.datum[a] as f64 + s.work_offset[a];
                let (from, to) = (at(span.min[a]), at(span.max[a]));
                if from < lo - 1e-6 || to > hi + 1e-6 {
                    e.push(format!(
                        "At this work zero the tool would reach machine {} {from:.3} to {to:.3}, outside the soft limits {lo:.3} to {hi:.3}. Put the work zero on the stock: see Work zero on the stock below.",
                        AXES[a]
                    ));
                }
            }
        }
        if let Some(blocks) = blocks
            && unset.is_empty()
        {
            e.extend(way_in_problems(p, blocks, s.work_offset, s.machine_pos));
        }
        e.extend(output_problems(&c.config.outputs, &p.ops[from..]));
        // A spindle left running by hand the other way: the driver E-stops a
        // reversal, so it must be stopped first. Running the tool run's way
        // it stays, as the program sets its rpm.
        let way = |clockwise: bool| {
            if clockwise {
                "clockwise"
            } else {
                "counterclockwise"
            }
        };
        if let Some(turning @ (Spindle::Cw | Spindle::Ccw)) = s.spindle
            && (turning == Spindle::Cw) != first.clockwise
        {
            e.push(format!(
                "The spindle is running {}, but tool run {} starts it {}. Stop it first (Stop under Spindle): the driver E-stops a reversal.",
                way(turning == Spindle::Cw),
                self.next + 1,
                way(first.clockwise)
            ));
        }
        if let Some((wcs, xy)) = self.xy
            && (wcs != s.active_wcs || (0..2).any(|a| (s.work_offset[a] - xy[a]).abs() > 1e-6))
        {
            e.push("X/Y work zero changed since the first tool run started. Tool changes keep X/Y zero: put it back, or pick the setup again to start over.".into());
        }
        if let Some((origin, datum)) = &self.zero {
            let moved: Vec<String> = (0..3)
                .filter(|&a| (p.datum[a] - datum[a]).abs() > 1e-3)
                .map(|a| format!("{} {:.3}", AXES[a], p.datum[a] - datum[a]))
                .collect();
            if !moved.is_empty() {
                let what = if p.origin == *origin {
                    format!("{origin} moved {} mm in the project", moved.join(" "))
                } else {
                    format!(
                        "{origin} then, {} now, {} mm apart",
                        p.origin,
                        moved.join(" ")
                    )
                };
                e.push(format!(
                    "The setup's work zero moved since the first tool run started: {what}. The machine's work zero was touched off for the old one, so this tool run would cut that far from where the first one did. Put the setup back as it was, or pick it again to start over and touch off the new work zero."
                ));
            }
        }
        if let Some((tool, z)) = &self.last
            && *tool != p.ops[0].tool_id
            && !self.touched
            && (s.work_offset[2] - z).abs() < 1e-6
        {
            e.push(format!(
                "Tool run {} uses {}: touch off its Z first (Set Z from reference, or Zero Z).",
                self.next + 1,
                crate::model::tool_label(p.ops[0].tool, &p.ops[0].tool_name)
            ));
        }
        e
    }

    /// The enabled operations of the setup (indices) asking for a coolant the
    /// UC100 profile has no output for, in any tool run: what Set coolant to
    /// Off changes.
    pub fn coolant_off(&self, c: &Connection, doc: &Document) -> Vec<usize> {
        let Some(setup) = doc.setups.get(self.setup).filter(|s| s.id == self.setup_id) else {
            return vec![];
        };
        let air = PostSettings::for_uc100(&setup.machine)
            .ok()
            .and_then(|s| s.air);
        let out = &c.config.outputs;
        (0..setup.operations.len())
            .filter(|&i| {
                let o = &setup.operations[i];
                o.enabled
                    && crate::post::coolant(o, air).is_some_and(|(mist, flood)| {
                        mist && out.mist.is_none() || flood && out.flood.is_none()
                    })
            })
            .collect()
    }

    /// Starts the next tool run when nothing stands in the way: lowers it with
    /// the live work offset from where the machine is.
    pub fn cycle_start(&mut self, c: &Connection, doc: &Document) -> Option<Command> {
        if !self.problems(c, doc).is_empty() {
            return None;
        }
        let programs = self.checked()?;
        let (count, p) = (programs.len(), &programs[self.next]);
        let s = &c.snapshot;
        let actions =
            self.job_actions(p, self.from, s.work_offset, s.machine_pos, self.at_step())?;
        let print = program_print(p, self.from, &actions)?;
        let cut_ops = cut_prints(doc, self.setup, &self.skips);
        // Where the tool tip is, in plan coordinates: the first retract's start.
        let tip =
            std::array::from_fn(|k| (s.machine_pos[k] - s.work_offset[k]) as f32 + p.datum[k]);
        let blocks = p.from(self.from).ok()?;
        let paths = paths(blocks, tip);
        let marks = self.marks_of(p, blocks);
        let tool = p.ops[0].tool_id.clone();
        let zero = (p.origin.clone(), p.datum);
        let mut name = format!("{} · tool run {} of {count}", self.name, self.next + 1);
        if self.from > 0 {
            name += &format!(" from {}", p.ops[self.from].name);
        }
        if let Some((oi, n)) = self.at_step() {
            name += &format!(" at {}", self.step_label(oi, n));
        }
        self.running = Some(Running {
            index: self.next,
            from: self.from,
            seq: s.program_outcome.as_ref().map_or(0, |o| o.seq),
            offset: s.work_offset,
            before: Before {
                xy: self.xy,
                zero: self.zero.clone(),
                last: self.last.clone(),
                touched: self.touched,
            },
            paths,
            tip,
            start: s.machine_pos,
            print,
            cut_ops,
            on: None,
            marks,
        });
        self.xy
            .get_or_insert((s.active_wcs, [s.work_offset[0], s.work_offset[1]]));
        self.zero.get_or_insert(zero);
        self.last = Some((tool, s.work_offset[2]));
        self.touched = false;
        self.outcome = None;
        Some(Command::Job {
            name,
            actions: Arc::new(actions),
        })
    }

    /// What Cycle Start sends of tool run `p` from its operation `from`,
    /// the machine at `start` with work offset `offset`, starting at step
    /// `step` (`at_step`).
    fn job_actions(
        &self,
        p: &Checked,
        from: usize,
        offset: [f64; 6],
        start: [f64; 6],
        step: Option<(usize, usize)>,
    ) -> Option<Vec<Located>> {
        let mut actions = lower(p, from, offset, start).ok()?;
        // At a step, it waits over the step's entry with the spindle on,
        // after the rapids there, until Resume feeds in (LIO-275).
        if let Some((oi, n)) = step
            && let Some(line) = (p.from(from).ok()?.iter())
                .position(|b| matches!(b, Block::Descend { .. }))
                .map(|i| i as u32 + 1)
        {
            let at = actions
                .iter()
                .rposition(|l| l.line <= line)
                .map_or(0, |i| i + 1);
            let reason = format!(
                "Over the entry of {}: Resume feeds in.",
                self.step_label(oi, n)
            );
            actions.insert(
                at,
                Located {
                    line,
                    action: Do::Pause { reason },
                },
            );
        }
        Some(actions)
    }

    /// The running tool run's program and where in it the machine is: the
    /// operation (index in the program) and the block being cut.
    pub fn progress(&self, line: Option<u32>) -> Option<(&Checked, usize, usize)> {
        let r = self.running.as_ref()?;
        let p = self.checked()?.get(r.index)?;
        let blocks = self.running_blocks()?;
        let block = (line.unwrap_or(1).max(1) as usize - 1).min(blocks.len() - 1);
        let op = blocks[..=block]
            .iter()
            .rev()
            .find_map(op_of)
            .unwrap_or(r.from);
        Some((p, op, block))
    }

    /// The blocks the running tool run was lowered from: its program from
    /// the operation it started at.
    pub fn running_blocks(&self) -> Option<&[Block]> {
        let r = self.running.as_ref()?;
        self.checked()?.get(r.index)?.from(r.from).ok()
    }

    /// The run starts partway through the setup before any of its tool runs
    /// started: at a later tool run or operation than the first, those
    /// before taken as cut, or with steps of its operation taken as cut.
    /// Its work zero is set on the stock they leave (`partly`), and Cycle
    /// Start waits for it (`zeroed`).
    pub fn partway(&self) -> bool {
        // An operation taken as cut before the start (`Skip::Cut`) is one.
        let start =
            (self.checked()).and_then(|c| c.get(self.next)?.indexes.get(self.from).copied());
        let cut = |&(oi, how): &(usize, Skip)| how == Skip::Cut && start.is_some_and(|s| oi < s);
        // So are steps taken as cut before it, and any of its own: taken as
        // cut, their material is gone, wherever the work zero is touched
        // off (LIO-568).
        let steps =
            |(oi, cut): &(usize, Vec<usize>)| !cut.is_empty() && start.is_some_and(|s| *oi <= s);
        let early = (self.next, self.from) != (0, 0)
            || self.skips.iter().any(cut)
            || self.cut_steps.iter().any(steps);
        self.checked().is_some() && (self.at_step().is_some() || !self.started() && early)
    }

    /// Where the operator touches off its setup, in plan mm, and the work
    /// position that is there (`post::touch_point` less the datum). None
    /// when it is not on the stock top: a Model origin whose stock top an
    /// earlier setup cut. A run that starts partway touches off on the
    /// surface the operations before it leave there, when the check knows
    /// it (`partly`), and is None when not (`partway_blocked`).
    pub fn touch_off(&self) -> Option<([f32; 3], [f64; 3])> {
        let p = self.checked()?.first()?;
        if p.located.is_some_and(|(_, top)| top.is_none()) {
            return None;
        }
        let mut at = crate::post::touch_point(&p.origin, p.stock)?;
        if self.partway() {
            at[2] = self.partly()?.surface?;
        }
        Some((at, std::array::from_fn(|a| (at[a] - p.datum[a]) as f64)))
    }

    /// Why Zero Z on the stock top and Probe Z cannot set the work Z of a
    /// run that starts partway (`partway`): the check cannot tell the
    /// surface the operations before its start leave where the stock top
    /// is touched off. None when it can, or the run starts at the setup's
    /// first operation.
    pub fn partway_blocked(&self) -> Option<String> {
        if !self.partway() {
            return None;
        }
        let Some(partly) = self.partly() else {
            return Some(PARTWAY_UNKNOWN.into());
        };
        if partly.surface.is_some() {
            return None;
        }
        let by = self.cut_by();
        Some(if partly.cut.is_empty() {
            format!(
                "The check cannot tell the surface {by} leave where the stock top is touched off."
            )
        } else if partly.cut == [STEPS_UNFOLLOWED] {
            // Not a part of the stock: said as the reason it is (LIO-579).
            "The check does not follow the stock cut by the steps taken as cut, so it cannot tell the surface they left where the stock top is touched off.".into()
        } else {
            format!(
                "{}{} cut the stock's {}, and the check cannot tell the surface they left where the stock top is touched off.",
                by[..1].to_uppercase(),
                &by[1..],
                partly.cut.join(", ")
            )
        })
    }

    /// What cut the stock a run that starts partway (`partway`) starts
    /// from, as its messages say it: "the operations before the start", or
    /// "the steps taken as cut" when it is partway by steps of its own
    /// operation alone (LIO-568).
    pub fn cut_by(&self) -> &'static str {
        let start =
            (self.checked()).and_then(|c| c.get(self.next)?.indexes.get(self.from).copied());
        let ops_before = (self.next, self.from) != (0, 0)
            || (self.skips.iter()).any(|s| s.1 == Skip::Cut && start.is_some_and(|o| s.0 < o))
            || (self.cut_steps.iter()).any(|c| start.is_some_and(|o| c.0 < o));
        if ops_before {
            CUT_BY_OPERATIONS
        } else {
            "the steps taken as cut"
        }
    }

    /// What a run that starts partway (`partway`) takes as cut before its
    /// start, in the setup's order: each operation by name ("Face"), sent
    /// before it or skipped (`Skip::Cut`), and the steps of the start's own
    /// operation ("Face pass 2", "Face passes 1–2", "Face, 3 of its
    /// passes").
    pub fn taken_as_cut(&self) -> Vec<String> {
        let Some(programs) = self.checked() else {
            return vec![];
        };
        let Some(p) = programs.get(self.next) else {
            return vec![];
        };
        let start = p.indexes.get(self.from).copied();
        // The names of operations not sent are the layout's (`layout`).
        let name = |oi: usize| -> String {
            let all = (self.layout.as_deref()).and_then(|l| l.as_deref().ok());
            (all.into_iter().chain([programs]).flatten())
                .find_map(|p| Some(p.ops[p.indexes.iter().position(|&o| o == oi)?].name.clone()))
                .unwrap_or_else(|| format!("operation {}", oi + 1))
        };
        let mut ops: Vec<usize> = (programs[..self.next].iter())
            .flat_map(|p| p.indexes.iter().copied())
            .chain(p.indexes[..self.from.min(p.indexes.len())].iter().copied())
            .chain(
                (self.skips.iter())
                    .filter(|s| s.1 == Skip::Cut && start.is_some_and(|o| s.0 < o))
                    .map(|s| s.0),
            )
            .collect();
        ops.sort_unstable();
        ops.dedup();
        let mut said: Vec<String> = ops.into_iter().map(name).collect();
        if let Some(oi) = start {
            let cut = self.cut_of(oi);
            if let (Some(&first), Some(&last)) = (cut.first(), cut.last()) {
                let (op, steps) = (name(oi), self.op_steps(oi));
                said.push(match steps {
                    Some(s) if last - first + 1 == cut.len() => {
                        format!("{op} {}", s.labels(first..=last).to_lowercase())
                    }
                    _ => {
                        let kind = steps.map_or("steps", |s| s.kind().1).to_lowercase();
                        format!("{op}, {} of its {kind}", cut.len())
                    }
                });
            }
        }
        said
    }

    /// The start at a step Cycle Start starts at (`start_step`), while its
    /// operation is the start: the operation (index in the setup) and step.
    pub fn at_step(&self) -> Option<(usize, usize)> {
        let (oi, n) = self.start_step?;
        let p = self.checked()?.get(self.next)?;
        (self.running.is_none() && p.indexes.get(self.from) == Some(&oi)).then_some((oi, n))
    }

    /// The steps of operation `oi` (index in the setup), from the check.
    pub fn op_steps(&self, oi: usize) -> Option<&OpSteps> {
        self.steps.iter().find(|s| s.op == oi)
    }

    /// Step `n` of operation `oi` as a sentence says it: "level 14".
    pub fn step_label(&self, oi: usize, n: usize) -> String {
        (self.op_steps(oi))
            .map_or(format!("step {}", n + 1), |s| s.label(n))
            .to_lowercase()
    }

    /// The steps of operation `oi` the check takes as cut: those skipped,
    /// and those before a start at one of its steps.
    pub fn cut_of(&self, oi: usize) -> Vec<usize> {
        let mut cut: Vec<usize> = (self.cut_steps.iter())
            .filter(|c| c.0 == oi)
            .flat_map(|c| c.1.iter().copied())
            .collect();
        if let Some((o, n)) = self.start_step
            && o == oi
        {
            cut.extend(0..n);
        }
        cut.sort_unstable();
        cut.dedup();
        cut
    }

    /// Every operation's steps the check takes as cut (`cut_of`).
    pub fn all_cut(&self) -> Vec<(usize, Vec<usize>)> {
        let mut ops: Vec<usize> = self.cut_steps.iter().map(|c| c.0).collect();
        ops.extend(self.start_step.map(|s| s.0));
        ops.sort_unstable();
        ops.dedup();
        (ops.into_iter())
            .map(|oi| (oi, self.cut_of(oi)))
            .filter(|c| !c.1.is_empty())
            .collect()
    }

    /// What was done and where Cycle Start starts, by operation (index in
    /// the setup), for a check that keeps them (`keep`).
    fn progress_kept(&self, programs: &[Checked]) -> (Vec<usize>, Option<usize>) {
        let done: Vec<usize> = (programs.iter().zip(&self.done))
            .filter(|(_, d)| **d)
            .flat_map(|(p, _)| p.indexes.iter().copied())
            .collect();
        let start = (programs.get(self.next)).and_then(|p| p.indexes.get(self.from).copied());
        (done, start)
    }

    /// Picks step `s` of operation `oi`, or with `range` the steps from the
    /// one picked first to it, while the machine stands in a manual state.
    pub fn pick(&mut self, oi: usize, s: usize, range: bool) {
        if self.op_steps(oi).is_none_or(|o| s >= o.steps.len()) {
            return;
        }
        self.picked = match self.picked {
            Some((o, first, _)) if range && o == oi => Some((oi, first.min(s), first.max(s))),
            _ => Some((oi, s, s)),
        };
    }

    /// Takes the steps picked as cut (`cut`), or skips none of their
    /// operation's steps (not `cut`), while no tool run runs: not every step
    /// of the operation, and not one of a tool run done in this run.
    /// Whether it changed, and the setup is to be checked again, keeping
    /// what was done and where Cycle Start starts (`keep`).
    pub fn skip_steps(&mut self, cut: bool) -> bool {
        let Some((oi, first, last)) = self.picked else {
            return false;
        };
        let Some(programs) = self.checked().filter(|_| self.running.is_none()) else {
            return false;
        };
        let keep = self.progress_kept(programs);
        let count = self.op_steps(oi).map_or(0, |s| s.steps.len());
        let mut now: Vec<usize> = (self.cut_steps.iter())
            .find(|c| c.0 == oi)
            .map_or_else(Vec::new, |c| c.1.clone());
        let before = now.clone();
        if cut {
            now.extend(first..=last);
            now.sort_unstable();
            now.dedup();
        } else {
            now.clear();
        }
        let started = match self.start_step {
            Some((o, n)) if o == oi => n,
            _ => 0,
        };
        let all = (0..count).all(|s| now.contains(&s) || s < started);
        if now == before || keep.0.contains(&oi) || cut && all {
            return false;
        }
        self.cut_steps.retain(|c| c.0 != oi);
        if !now.is_empty() {
            self.cut_steps.push((oi, now));
        }
        (self.keep, self.picked, self.carving) = (Some(keep), None, None);
        true
    }

    /// Makes the first step picked the one Cycle Start starts at, while no
    /// tool run runs: the steps before it and the operations before its
    /// operation are taken as cut, as before a Start here. Whether it
    /// changed, and the setup is to be checked again (`keep`).
    pub fn start_step(&mut self) -> bool {
        let Some((oi, n, _)) = self.picked else {
            return false;
        };
        self.start_at_step(oi, n)
    }

    /// `start_step` at step `n` of operation `oi` (index in the setup).
    pub fn start_at_step(&mut self, oi: usize, n: usize) -> bool {
        let Some(programs) = self.checked().filter(|_| self.running.is_none()) else {
            return false;
        };
        let (done, _) = self.progress_kept(programs);
        let start = (n > 0).then_some((oi, n));
        if self.start_step == start && self.at_step() == start && start.is_some() {
            return false;
        }
        // The steps it starts after are taken as cut by the start, not
        // skipped: a skip of them is dropped.
        for c in &mut self.cut_steps {
            if c.0 == oi {
                c.1.retain(|&s| s >= n);
            }
        }
        self.cut_steps.retain(|c| !c.1.is_empty());
        // Done in this run, its tool run is cut again from that step.
        let done: Vec<usize> = done.into_iter().filter(|&o| o < oi).collect();
        self.start_step = start;
        (self.keep, self.picked, self.carving) = (Some((done, Some(oi))), None, None);
        (self.zeroed, self.tool_kept) = (false, false);
        true
    }

    /// What a run that starts partway finds of the work zero (`partway`,
    /// `Checked::partly`). None when it does not start partway, or the
    /// check did not work it out.
    pub fn partly(&self) -> Option<std::borrow::Cow<'_, Partly>> {
        use std::borrow::Cow;
        if !self.partway() {
            return None;
        }
        let programs = self.checked()?;
        let p = programs.get(self.next)?;
        let before = p.partly.get(self.from)?;
        // Steps of its operation taken as cut, before a start at a step or
        // skipped, have cut the stock it starts from (LIO-568).
        let steps_cut = (p.indexes.get(self.from)).is_some_and(|&oi| !self.cut_of(oi).is_empty());
        if self.at_step().is_none() && !steps_cut {
            return Some(Cow::Borrowed(before));
        }
        // Inside an operation the check does not follow the stock, but it
        // knows it before the operation and after it (before the next one
        // sent): a spot neither cut is as it was, a surface the same in both
        // is that surface. Cut steps only take material away.
        let after = (programs.iter().flat_map(|p| &p.partly))
            .skip_while(|p| !std::ptr::eq(*p, before))
            .nth(1);
        let unknown = STEPS_UNFOLLOWED;
        // The stock after the operation says what the operations before
        // the next one cut; partway by steps of its own alone, they are
        // the steps taken as cut (LIO-568).
        let by_steps = self.cut_by() != CUT_BY_OPERATIONS;
        let said = |e: &String| match e.strip_prefix("The operations before it ") {
            Some(rest) if by_steps => format!("The steps taken as cut {rest}"),
            _ => e.clone(),
        };
        Some(Cow::Owned(match after {
            Some(after) => Partly {
                cut: after.cut.clone(),
                surface: before.surface.filter(|s| after.surface == Some(*s)),
                top_cut: after.top_cut,
                probe_from: (after.probe_from.iter())
                    .map(|(r, ok)| {
                        let was = before.probe_from.iter().find(|(b, _)| b == r);
                        let ok = match (ok, was) {
                            (Ok(()), Some((_, Ok(())))) => Ok(()),
                            (Err(e), _) => Err(said(e)),
                            (_, Some((_, Err(e)))) => Err(e.clone()),
                            _ => Err(format!("The check cannot follow the {unknown}.")),
                        };
                        (*r, ok)
                    })
                    .collect(),
            },
            None => Partly {
                cut: vec![unknown.into()],
                surface: None,
                top_cut: true,
                probe_from: (before.probe_from.iter())
                    .map(|(r, _)| (*r, Err(format!("The check cannot follow the {unknown}."))))
                    .collect(),
            },
        }))
    }

    /// Skips operation `oi` (index in the setup) as `how` says, or skips it
    /// no more (None), while no tool run runs: one that has not been cut in
    /// this run, whose material may be left (`beneath`), and not the last one
    /// sent. Whether it changed, and the setup is to be checked again: what
    /// was done and where Cycle Start starts are kept over that (`keep`).
    /// A skip whose check refused the setup can be undone: what was done is
    /// then the one kept from before it.
    pub fn skip(&mut self, oi: usize, how: Option<Skip>) -> bool {
        self.menu = None;
        if self.running.is_some() || self.programs.is_none() {
            return false;
        }
        let now = self.skips.iter().find(|s| s.0 == oi).map(|s| s.1);
        let (keep, sent) = match self.checked() {
            Some(programs) => {
                let done: Vec<usize> = (programs.iter().zip(&self.done))
                    .filter(|(_, d)| **d)
                    .flat_map(|(p, _)| p.indexes.iter().copied())
                    .collect();
                let mut start =
                    (programs.get(self.next)).and_then(|p| p.indexes.get(self.from).copied());
                // Undone, it is the start again when the start moved past
                // it to the next operation sent, or there is none.
                let between =
                    |s: usize| (programs.iter().flat_map(|p| &p.indexes)).any(|&o| o > oi && o < s);
                if how.is_none() && start.is_none_or(|s| oi < s && !between(s)) {
                    start = Some(oi);
                }
                let sent = programs.iter().map(|p| p.ops.len()).sum::<usize>();
                ((done, start), sent)
            }
            // Refused: only undone.
            None if how.is_none() => (self.keep.clone().unwrap_or_default(), 0),
            None => return false,
        };
        let refused = match how {
            _ if now == how || keep.0.contains(&oi) => true,
            Some(_) if now.is_none() && sent <= 1 => true,
            Some(Skip::Left) => self.beneath.iter().any(|b| b.0 == oi),
            _ => false,
        };
        if refused {
            return false;
        }
        self.keep = Some(keep);
        self.skips.retain(|s| s.0 != oi);
        if let Some(how) = how {
            self.skips.push((oi, how));
        }
        self.carving = None;
        true
    }

    /// Skips nothing any more, without asking for a check: the project
    /// changed, and the setup is checked again anyway.
    pub fn unskip(&mut self) {
        (self.skips, self.keep, self.menu) = (vec![], None, None);
        (self.cut_steps, self.start_step, self.picked) = (vec![], None, None);
    }

    /// Makes operation `k` of tool run `i` the one Cycle Start starts at:
    /// the work zero is set for the stock it starts from again.
    pub fn start_at(&mut self, i: usize, k: usize) {
        let ops = self
            .checked()
            .and_then(|c| c.get(i))
            .map_or(0, |p| p.ops.len());
        if self.running.is_none() && k < ops {
            // A start at a step is checked again without it.
            if self.start_step.take().is_some()
                && let Some(programs) = self.checked()
            {
                let (done, _) = self.progress_kept(programs);
                let start = programs[i].indexes.get(k).copied();
                (self.keep, self.recheck) = (Some((done, start)), true);
            }
            (self.next, self.from) = (i, k);
            self.zeroed = false;
            // The stock is the one this start finds again.
            self.carving = None;
        }
    }
}

/// Where an operation of a tool run stands, for the Tool runs panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Done,
    /// The machine is cutting it.
    Cutting,
    /// Cycle Start starts it: the operation chosen in the tool run marked
    /// next, its first unless another was chosen.
    Next,
    /// Before the start: taken as cut, and not sent.
    Skipped,
    ToCome,
}

impl Run {
    /// Where each operation of tool run `i` stands, the machine being at
    /// program line `line`: while it runs, those before the one it cuts are
    /// done, those before the one it started at skipped. Otherwise the next
    /// tool run's are skipped before its start (done if it was), its start
    /// is next and the rest to come; another's are done once it is, and
    /// skipped when it comes before the next.
    pub fn stages(&self, i: usize, line: Option<u32>) -> Vec<Stage> {
        let Some(p) = self.checked().and_then(|c| c.get(i)) else {
            return vec![];
        };
        let running = self.running.as_ref().filter(|r| r.index == i);
        let cutting = self
            .progress(line)
            .filter(|_| running.is_some())
            .map(|(_, op, _)| op);
        let from = running.map_or(self.from, |r| r.from);
        let done = self.done.get(i) == Some(&true);
        (0..p.ops.len())
            .map(|k| match cutting {
                Some(_) if k < from => Stage::Skipped,
                Some(op) if k < op => Stage::Done,
                Some(op) if k == op => Stage::Cutting,
                Some(_) => Stage::ToCome,
                // A tool run picked again after it was done is cut again.
                None if i == self.next && k < from && done => Stage::Done,
                None if i == self.next && k < from => Stage::Skipped,
                None if i == self.next && k == from => Stage::Next,
                None if i == self.next => Stage::ToCome,
                None if done => Stage::Done,
                None if i < self.next => Stage::Skipped,
                None => Stage::ToCome,
            })
            .collect()
    }
}

/// Where a step of an operation stands, for the Operations list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepStage {
    Done,
    Cutting,
    /// Cycle Start starts at it.
    Next,
    /// Skipped: taken as cut, not sent (`Run::cut_steps`).
    Cut,
    /// Before the start at a step: taken as cut, not sent.
    Before,
    ToCome,
}

impl Run {
    /// The step the running tool run is in, of the block the tool tip is
    /// on (the driver's line until it is found): its operation (index in
    /// the setup), the step, and whether its cutting is done.
    pub fn step_at(&self, line: Option<u32>) -> Option<(usize, usize, bool)> {
        let r = self.running.as_ref()?;
        let p = self.checked()?.get(r.index)?;
        let block = match r.on {
            Some((b, _)) => b,
            None => self.progress(line)?.2,
        };
        self.mark_step(p, (*r.marks.get(block)?)?)
    }

    /// A `Mark` of program `p`'s as a step of its operation: the operation
    /// (index in the setup), the step (index in its steps), and whether its
    /// cutting is done.
    fn mark_step(&self, p: &Checked, (op, sent, done): Mark) -> Option<(usize, usize, bool)> {
        let oi = *p.indexes.get(op)?;
        let n = self.op_steps(oi)?.steps.len();
        let cut = self.cut_of(oi);
        let s = (0..n).filter(|s| !cut.contains(s)).nth(sent)?;
        Some((oi, s, done))
    }

    /// Where each step of operation `oi` (index in the setup) stands, the
    /// operation's own stage being `stage` and the machine at program line
    /// `line`.
    pub fn step_stages(&self, oi: usize, stage: Stage, line: Option<u32>) -> Vec<StepStage> {
        let Some(os) = self.op_steps(oi) else {
            return vec![];
        };
        let skipped: Vec<usize> = (self.cut_steps.iter())
            .filter(|c| c.0 == oi)
            .flat_map(|c| c.1.iter().copied())
            .collect();
        let start = self.start_step.filter(|s| s.0 == oi).map(|s| s.1);
        let now = self.step_at(line).filter(|n| n.0 == oi);
        let first =
            (0..os.steps.len()).find(|s| !skipped.contains(s) && start.is_none_or(|n| *s >= n));
        (0..os.steps.len())
            .map(|s| match stage {
                _ if skipped.contains(&s) => StepStage::Cut,
                _ if start.is_some_and(|n| s < n) => StepStage::Before,
                Stage::Done => StepStage::Done,
                Stage::Skipped => StepStage::Before,
                Stage::Cutting => match now {
                    Some((_, c, done)) if s < c || s == c && done => StepStage::Done,
                    Some((_, c, _)) if s == c => StepStage::Cutting,
                    _ => StepStage::ToCome,
                },
                Stage::Next if first == Some(s) => StepStage::Next,
                Stage::Next | Stage::ToCome => StepStage::ToCome,
            })
            .collect()
    }
}

/// The machine's tool while a tool run cuts, for the 3D view.
pub struct Live {
    pub setup: usize,
    /// The tool tip in the setup's plan coordinates (mm).
    pub tip: [f32; 3],
    pub radius: f32,
    pub stickout: f32,
}

impl Run {
    /// Where the machine's tool is in the running tool run's plan: machine
    /// position − the work offset it was lowered with + datum.
    pub fn live(&self, s: &uc100::controller::Snapshot) -> Option<Live> {
        let r = self.running.as_ref()?;
        let (p, op, _) = self.progress(s.current_line)?;
        Some(Live {
            setup: self.setup,
            tip: std::array::from_fn(|k| {
                (s.machine_pos[k] - r.offset[k] + p.datum[k] as f64) as f32
            }),
            radius: p.ops[op].diameter / 2.,
            stickout: p.ops[op].stickout,
        })
    }
}

/// Why a machine with an A axis cannot run on this UC100 profile: the
/// profile has no `[axes.a]` or no soft limits on it, or its A range is not
/// the machine's rotary range in AXIS CAM (degrees), as the XYZ travel must
/// be. Empty for a machine without an A axis (LIO-184).
fn a_problems(
    machine: &crate::model::Machine,
    config: &uc100::config::MachineConfig,
) -> Vec<String> {
    if !machine.has_a() {
        return vec![];
    }
    let Some(a) = config.axes.get(3) else {
        return vec![format!(
            "{} has an A axis in AXIS CAM, but the UC100 profile has no [axes.a]. Add the A axis to the profile.",
            machine.name()
        )];
    };
    let Some((lo, hi)) = a.soft_min.zip(a.soft_max) else {
        return vec!["The UC100 profile sets no soft limits on A, so AXIS CAM cannot check that the job fits the machine.".into()];
    };
    let (min, max) = (
        crate::model::num(&machine.p, "rotary_min"),
        crate::model::num(&machine.p, "rotary_max"),
    );
    let Some((min, max)) = min.zip(max) else {
        return vec!["The machine's rotary range in AXIS CAM is not a number.".into()];
    };
    // Both ends: an angle, unlike a travel, has a place as well as a length.
    if (min as f64 - lo).abs() > 0.01 || (max as f64 - hi).abs() > 0.01 {
        return vec![format!(
            "The machine's A range in AXIS CAM is {min:.3} to {max:.3} degrees, but the UC100 profile's soft limits are {lo:.3} to {hi:.3} degrees. Make them agree."
        )];
    }
    vec![]
}

/// How far the work zero's Y and Z may be off the measured A centreline
/// for an indexed setup to run (mm): a feature cut at A 180° lands twice
/// as far off as the one cut at A 0°, 0.04 mm at most, under the 0.05 mm
/// over which the Control page's alignment checks warn (`ALIGN_LIMIT`).
pub const ON_CENTRELINE: f64 = 0.02;

/// How near its angle A must be for a setup that does not turn A (at A0)
/// and for a touch-off on a reference that turns with A (degrees): about
/// a step of the rotary unit's A.
pub const A_AT: f64 = 0.01;

/// The A angles (degrees, as commanded) program `blocks` turns A to or
/// makes moves at, each once, in order; empty for one that never commands
/// A.
pub fn a_angles(blocks: &[Block]) -> Vec<f32> {
    let mut v: Vec<f32> = vec![];
    for b in blocks {
        let a = match b {
            Block::Index { a, .. } => *a,
            Block::Move { a: Some(a), .. } => *a,
            _ => continue,
        };
        if !v.iter().any(|b| (a - b).abs() < 1e-4) {
            v.push(a);
        }
    }
    v
}

/// The A angles of operation `op` (index in the program) of `blocks`, in
/// the order it is cut at them: those of its moves.
pub fn op_angles(blocks: &[Block], op: usize) -> Vec<f32> {
    let mut v: Vec<f32> = vec![];
    for b in blocks {
        if let Block::Move {
            op: o, a: Some(a), ..
        } = b
            && *o == op
            && !v.iter().any(|b| (a - b).abs() < 1e-4)
        {
            v.push(*a);
        }
    }
    v
}

/// The Y and Z work offsets that put program `p`'s work zero on the A
/// centreline of `setup`'s machine for the probe it was probed with: the
/// plan's centreline (`Turn::axis`) on the machine's (`RotaryUnit`).
/// None without the rotary geometry or the setup's turn.
pub fn on_centre(doc: &Document, setup: &crate::model::Setup, p: &Checked) -> Option<[f64; 2]> {
    let (u, turn) = (doc.rotary_unit(&setup.machine)?, setup.turn?);
    Some([
        u.centre_y as f64 + p.datum[1] as f64 - turn.axis[0] as f64,
        u.centre_z as f64 + p.datum[2] as f64 - turn.axis[1] as f64,
    ])
}

/// What stands in the way of running program `blocks` of `setup` on the
/// machine with A (LIO-192). A program that turns A (an indexed setup's)
/// needs the rotary calibration current, the measured A centreline the
/// Control page keeps neither assumed nor gone stale; the work zero where
/// the setup has the A centreline, within `ON_CENTRELINE`; and every A it
/// turns to inside A's soft limits at A's work offset. One that does not
/// turn A cuts at A0, where the setup's work zero was found: A must be
/// there. Nothing for a machine without A.
///
/// The Z sent for a cut at work Z `w` is `w` + the Z work offset; with no
/// tool length offset on the UC100, the tip is then the tool's length
/// under it. The centreline's machine Z (`RotaryUnit::centre_z`) is where
/// the probe's tip was on it, so the offset `on_centre` gives is right for
/// a tool as long as the probe only. A tool `d` mm longer needs it `d` mm
/// higher, or every cut is `d` deeper and a feature cut at A 180° lands
/// `2d` off (LIO-207). `d` is known only from the tool's touch-off on the
/// Z reference the probe touched with the centreline (`length`): without
/// one for this tool run's tool, it is refused.
fn rotary_problems(
    c: &Connection,
    doc: &Document,
    setup: &crate::model::Setup,
    p: &Checked,
    blocks: &[Block],
    length: Option<&(String, f64, f64)>,
) -> Vec<String> {
    let m = &setup.machine;
    if !m.has_a() || !c.axis(3) {
        return vec![];
    }
    let s = &c.snapshot;
    let wcs = 54 + s.active_wcs;
    let angles = a_angles(blocks);
    if angles.is_empty() {
        return if s.referenced[3] && s.work_pos[3].abs() > A_AT {
            vec![format!(
                "This setup cuts at A0 and does not turn A, but A is at {:.3}° in G{wcs}. Turn A to A0 first (G0 A0 by MDI).",
                s.work_pos[3]
            )]
        } else {
            vec![]
        };
    }
    let mut e = vec![];
    let unit = c.rotary.as_ref().and(doc.rotary_unit(m));
    let marks = [
        c.rotary.as_ref().map_or("", |r| r.mark.as_str()),
        doc.centre_mark(m),
    ];
    match marks.iter().find(|k| **k != crate::control::MEASURED) {
        _ if unit.is_none() => e.push(
            "The machine's rotary unit geometry is not recorded, so where A turns the stock is not known: fill it in in the Machine library, then probe the A centreline under Control → Rotary A.".into(),
        ),
        None => {}
        Some(&"") => e.push(
            "The A centreline is assumed, not measured: an indexed setup runs only about a measured one. Probe it under Control → Rotary A (Probe the centreline, then Save the centreline).".into(),
        ),
        Some(why) => e.push(format!(
            "The measured A centreline went {why}: probe it again under Control → Rotary A before running an indexed setup."
        )),
    }
    // The work zero where the setup has the A centreline, for this tool:
    // the probe's, its Z as much higher as the tool is longer.
    let tool = &p.ops[0];
    let measured = unit.is_some() && marks.iter().all(|k| *k == crate::control::MEASURED);
    let longer = match (unit.and_then(|u| u.reference), length) {
        _ if !measured => None,
        (None, _) => {
            e.push(crate::control::NO_CENTRE_REFERENCE.into());
            None
        }
        (Some(at), Some((id, d, from)))
            if *id == tool.tool_id && (*from - at[2] as f64).abs() < 1e-6 =>
        {
            Some(*d)
        }
        (Some(at), _) => {
            e.push(format!(
                "T{} {} is not touched off on the A centreline's Z reference, so where its tip is on the centreline is not known: the centreline was probed with the probe, and a tool longer or shorter than it cuts that much deeper or shallower. With T{} in the spindle, jog over the reference (machine X {:.3} Y {:.3}), put the touch plate on it and press Touch off on the A reference.",
                tool.tool, tool.tool_name, tool.tool, at[0], at[1]
            ));
            None
        }
    };
    match (on_centre(doc, setup, p), longer) {
        (None, _) if unit.is_some() => e.push(
            "Where the A centreline is in this setup is not known: check the first setup's stock."
                .into(),
        ),
        (Some([y, z]), Some(d)) if s.referenced[1] && s.referenced[2] => {
            let want = [y, z + d];
            let off = (s.work_offset[1] - want[0]).hypot(s.work_offset[2] - want[1]);
            if off > ON_CENTRELINE {
                e.push(format!(
                    "The work zero is {off:.3} mm off the A centreline (within {ON_CENTRELINE} mm is needed): its Y and Z offsets in {} are {:.3} and {:.3}, but this setup's work zero on the measured centreline (machine Y {:.3} Z {:.3} for the probe; T{}'s tip is {:.3} mm under the probe's) is at Y {:.3} Z {:.3}. Touch the tool off on the A reference again, or set them there (G10 L2 P{} Y{:.3} Z{:.3} by MDI).",
                    p.offset,
                    s.work_offset[1],
                    s.work_offset[2],
                    unit.map_or(0., |u| u.centre_y),
                    unit.map_or(0., |u| u.centre_z),
                    tool.tool,
                    d,
                    want[0],
                    want[1],
                    s.active_wcs + 1,
                    want[0],
                    want[1],
                ));
            }
        }
        _ => {}
    }
    if s.referenced[3]
        && let Some((lo, hi)) = c
            .config
            .axes
            .get(3)
            .and_then(|x| x.soft_min.zip(x.soft_max))
    {
        let out: Vec<String> = angles
            .iter()
            .map(|a| *a as f64 + s.work_offset[3])
            .filter(|a| *a < lo - 1e-6 || *a > hi + 1e-6)
            .map(|a| format!("{a:.3}°"))
            .collect();
        if !out.is_empty() {
            e.push(format!(
                "At A's work offset in G{wcs} ({:.3}°) A would turn to machine A {}, outside its soft limits {lo:.3} to {hi:.3}°. Zero A where Find A zero on a flat puts it (Control → Rotary A).",
                s.work_offset[3],
                out.join(", ")
            ));
        }
    }
    e
}

/// The operation a block belongs to, if it names one.
fn op_of(b: &Block) -> Option<usize> {
    match b {
        Block::Start { op, .. }
        | Block::Run { op, .. }
        | Block::Approach { op, .. }
        | Block::Descend { op, .. }
        | Block::Move { op, .. }
        | Block::Pause { op, .. } => Some(*op),
        _ => None,
    }
}

/// The checked programs of setup `si` for a UC100 run, one per tool run,
/// from a simulation of the job up to the end of that setup.
pub fn programs(doc: &Document, mesh: &Mesh, si: usize) -> Result<Vec<Checked>, Vec<String>> {
    check(doc, mesh, si).0
}

/// `programs`, and the stock that simulation leaves after each operation
/// of the setup, for its pictures (`op_pictures`).
pub fn check(doc: &Document, mesh: &Mesh, si: usize) -> (Result<Vec<Checked>, Vec<String>>, Left) {
    check_skipping(doc, mesh, si, &[])
}

/// `check` of a run that `skips` operations (`Run::skips`): those whose
/// material is left are disabled for it, those taken as cut simulated but
/// not sent.
pub fn check_skipping(
    doc: &Document,
    mesh: &Mesh,
    si: usize,
    skips: &[(usize, Skip)],
) -> (Result<Vec<Checked>, Vec<String>>, Left) {
    check_steps(doc, mesh, si, skips, &[])
}

/// `check_skipping` of a run that also takes steps of its operations as
/// cut (`Run::all_cut`): their simulations' plans lose those steps
/// (`cut_plan`) after the steps were found (`Left::steps`), so that the
/// programs, their checks and every start re-enter the step after a cut
/// one from the clearance, as a Start here enters an operation.
pub fn check_steps(
    doc: &Document,
    mesh: &Mesh,
    si: usize,
    skips: &[(usize, Skip)],
    cut_steps: &[(usize, Vec<usize>)],
) -> (Result<Vec<Checked>, Vec<String>>, Left) {
    if si >= doc.setups.len() {
        return (Err(vec![REMOVED.into()]), Left::default());
    }
    if doc.file.is_none() {
        return (Err(vec![NO_PART.into()]), Left::default());
    }
    let of =
        |how: Skip| -> Vec<usize> { (skips.iter()).filter(|s| s.1 == how).map(|s| s.0).collect() };
    let (cut, left_out) = (of(Skip::Cut), of(Skip::Left));
    let mut doc = std::borrow::Cow::Borrowed(doc);
    for &oi in &left_out {
        if let Some(o) = doc.to_mut().setups[si].operations.get_mut(oi) {
            o.enabled = false;
        }
    }
    let (mut sims, mut left) = simulated_with(&doc, mesh, si);
    left.steps = op_steps(&doc, si, &sims);
    for (oi, steps) in cut_steps {
        let Some(found) = left.steps.iter().find(|s| s.op == *oi) else {
            continue;
        };
        if let Some((_, Ok(sim))) = sims.iter_mut().find(|(o, _)| o == oi)
            && !steps.is_empty()
            && steps.len() < found.steps.len()
        {
            // Taken as cut, they cut the stock the operation starts from:
            // the checks of the steps sent see that material gone (LIO-311).
            if let Some(mut volume) = sim.initial_volume.take() {
                // The work zero was set on the stock before them: they were
                // cut with it, and a start keeps it (LIO-568).
                sim.before_steps.get_or_insert_with(|| volume.clone());
                let plan = &sim.plan;
                for m in (steps.iter())
                    .filter_map(|&s| found.steps.get(s))
                    .flat_map(|s| &plan.moves[s.moves.clone()])
                    .filter(|m| m.motion == crate::simulation::Motion::Feed)
                {
                    if let Some(frame) = sim.frame_at(m.a) {
                        let (r, flute) = (plan.cut_radius(), plan.flute_length);
                        volume.cut(m.from, m.to, r, flute, plan.shape, frame, sim.source);
                    }
                }
                sim.initial_volume = Some(volume);
            }
            // Steps taken as cut are in the stock already: the stock before
            // this operation and those before it has them cut, so the live
            // carve starts from it (LIO-312), in the top view and in the 3D
            // view's volume (LIO-568). ponytail: the top view leaves out an
            // indexed setup's moves (at an A).
            let before = Arc::make_mut(&mut left.before);
            let plan = &sim.plan;
            let (r, flute) = (plan.cut_radius(), plan.flute_length);
            let moves = (steps.iter().filter_map(|&s| found.steps.get(s)))
                .flat_map(|s| &plan.moves[s.moves.clone()]);
            for m in moves {
                for b in before.iter_mut().filter(|b| b.op <= *oi) {
                    if m.a.is_none() {
                        b.stock.cut_shape(m.from, m.to, r, plan.shape);
                    }
                    if m.motion == crate::simulation::Motion::Feed
                        && let (Some(volume), Some(frame)) = (&mut b.volume, sim.frame_at(m.a))
                    {
                        volume.cut(m.from, m.to, r, flute, plan.shape, frame, sim.source);
                    }
                }
            }
            sim.plan = cut_plan(&sim.plan, &found.steps, steps);
        }
    }
    let programs =
        crate::post::checked_to_start(&doc, mesh, si, &sims, PostSettings::for_uc100, &cut);
    left.beneath = beneath(&doc, si, &sims, &cut);
    (programs, left)
}

/// A step of an operation (LIO-275): a stretch of its plan from the tool at
/// its clearance to the tool at its clearance again, a Z level, a pass, a
/// hole or a band, which a run can skip, start at or stop after.
#[derive(Debug, Clone, PartialEq)]
pub struct Step {
    /// Its plan moves: the first starts with the tool at the clearance (the
    /// operation's first move, or the first after it rose there).
    pub moves: std::ops::Range<usize>,
    /// The plan Z its level feed moves cut at, when they are all at one.
    pub z: Option<f32>,
    /// Its seconds in the plan, at the programmed feeds.
    pub seconds: f32,
    /// How many of its moves feed: the running program's marks count a
    /// drilling operation's holes by them (`step_marks`).
    pub feeds: usize,
    /// The pass it is in (from 0), of a plan cut in passes with a stop
    /// before each to set the tool to its size, a Boring's (`Stop::size`):
    /// each pass goes round every hole, so its steps are its visits of them
    /// (LIO-558). None for any other plan, or one of a single pass.
    pub pass: Option<usize>,
}

/// The steps of one operation of a setup, as the run's check found them
/// (`op_steps`).
#[derive(Debug, Clone, PartialEq)]
pub struct OpSteps {
    /// The operation's index in the setup.
    pub op: usize,
    /// Its clearance (`step_clearance`).
    pub clear: f32,
    /// A drilling cycle: each step is a hole.
    pub hole: bool,
    pub steps: Vec<Step>,
    /// FNV-1a of its plan's moves and its tool (`fingerprint`): a journal of
    /// where a run stopped holds for these steps only (`Journal`).
    pub fingerprint: u64,
}

impl OpSteps {
    /// What its steps are called: holes, levels when each cuts at one Z,
    /// else passes. A plan cut in passes (`Step::pass`) is named by them,
    /// each a visit of its holes (`label`).
    pub fn kind(&self) -> (&'static str, &'static str) {
        if self.passes() > 0 {
            ("Pass", "Passes")
        } else if self.hole {
            ("Hole", "Holes")
        } else if self.steps.iter().all(|s| s.z.is_some()) {
            ("Level", "Levels")
        } else {
            ("Pass", "Passes")
        }
    }

    /// What the running program's marks go by of it, its steps `cut` not
    /// sent (`step_marks`).
    fn mark_op(&self, cut: &[usize]) -> MarkOp {
        MarkOp {
            clear: self.clear,
            holes: self.hole.then(|| {
                (self.steps.iter().enumerate())
                    .filter(|(s, _)| !cut.contains(s))
                    .map(|(_, s)| s.feeds)
                    .collect()
            }),
        }
    }

    /// How many passes its plan is cut in (`Step::pass`): 0 for one not
    /// cut in passes.
    pub fn passes(&self) -> usize {
        (self.steps.iter())
            .filter_map(|s| s.pass)
            .max()
            .map_or(0, |p| p + 1)
    }

    /// Step `s` of a plan cut in passes as (its pass, its hole in the pass,
    /// the pass's holes), from 0: each pass visits the holes in the same
    /// order, so hole 2 of a pass is hole 2 of every other.
    fn in_pass(&self, s: usize) -> Option<(usize, usize, usize)> {
        let pass = self.steps.get(s)?.pass?;
        let first = (self.steps.iter()).position(|t| t.pass == Some(pass))?;
        let holes = (self.steps.iter()).filter(|t| t.pass == Some(pass)).count();
        Some((pass, s - first, holes))
    }

    /// Whether each of its passes is a single hole's: then a step is a pass.
    fn one_hole(&self) -> bool {
        self.steps.len() == self.passes()
    }

    /// How many steps it has, as its row says: "5 levels", "6 holes"; a
    /// plan cut in passes counts its passes and holes, "3 passes · 2 holes",
    /// not its visits (LIO-558), or "3 passes" of one hole.
    pub fn count(&self) -> String {
        let (n, kind) = (self.steps.len(), self.kind().1.to_lowercase());
        match self.passes() {
            0 => format!("{n} {kind}"),
            p if self.one_hole() => format!("{p} {kind}"),
            p => {
                let holes = self.in_pass(0).map_or(0, |h| h.2);
                let noun = if holes == 1 { "hole" } else { "holes" };
                format!("{p} {kind} · {holes} {noun}")
            }
        }
    }

    /// Step `s`'s label: "Level 14", "Hole 3", "Pass 2"; a visit of a hole
    /// of a plan cut in passes says both, "Pass 2 · Hole 1" (LIO-558).
    pub fn label(&self, s: usize) -> String {
        match self.in_pass(s) {
            Some((p, _, _)) if self.one_hole() => format!("Pass {}", p + 1),
            Some((p, h, _)) => format!("Pass {} · Hole {}", p + 1, h + 1),
            None => format!("{} {}", self.kind().0, s + 1),
        }
    }

    /// Steps `range`'s label: "Levels 5–9", or one step's. Of a plan cut in
    /// passes: "Pass 2 · Holes 1–3" within a pass, "Passes 1–2" for whole
    /// passes, else from the first's label to the last's.
    /// Step `s`'s label with how many there are, as the run line says
    /// where the job is: "Level 3 of 9"; a visit of a plan cut in passes
    /// counts each, "Pass 1 of 3 · Hole 2 of 2", never "of 6" visits
    /// (LIO-558).
    pub fn label_of(&self, s: usize) -> String {
        match self.in_pass(s) {
            Some((p, _, _)) if self.one_hole() => format!("Pass {} of {}", p + 1, self.passes()),
            Some((p, h, n)) => format!(
                "Pass {} of {} · Hole {} of {n}",
                p + 1,
                self.passes(),
                h + 1
            ),
            None => format!("{} of {}", self.label(s), self.steps.len()),
        }
    }

    /// The lines a folded run of steps `range` shows as (`ui::step_list`):
    /// itself, but a plan cut in passes splits one that starts or ends
    /// partway through a pass and runs into another into the rest of the
    /// pass it starts in, the whole passes after it and the start of the
    /// pass it ends in, so that a line reads "Passes 2–10" or "Pass 11 ·
    /// Holes 1–3", never "Pass 1 · Hole 2 – Pass 11 · Hole 1" (LIO-558).
    pub fn folds(
        &self,
        range: std::ops::RangeInclusive<usize>,
    ) -> Vec<std::ops::RangeInclusive<usize>> {
        let (a, b) = (*range.start(), *range.end());
        let (Some((pa, ha, na)), Some((pb, hb, nb))) = (self.in_pass(a), self.in_pass(b)) else {
            return vec![range];
        };
        if pa == pb || (ha == 0 && hb + 1 == nb) {
            return vec![range];
        }
        let mut out = vec![];
        // The rest of the pass it starts in.
        let mut whole = a;
        if ha > 0 {
            whole = a + (na - ha);
            out.push(a..=whole - 1);
        }
        // The whole passes, then the start of the pass it ends in.
        let tail = (hb + 1 < nb).then_some(b - hb);
        let last = tail.map_or(b, |t| t - 1);
        if whole <= last {
            out.push(whole..=last);
        }
        out.extend(tail.map(|t| t..=b));
        out
    }

    pub fn labels(&self, range: std::ops::RangeInclusive<usize>) -> String {
        let (a, b) = (*range.start(), *range.end());
        if a == b {
            return self.label(a);
        }
        let (Some((pa, ha, _)), Some((pb, hb, nb))) = (self.in_pass(a), self.in_pass(b)) else {
            return format!("{} {}–{}", self.kind().1, a + 1, b + 1);
        };
        let whole = ha == 0 && hb + 1 == nb;
        if whole && pa == pb {
            format!("Pass {}", pa + 1)
        } else if whole {
            format!("Passes {}–{}", pa + 1, pb + 1)
        } else if pa == pb {
            format!("Pass {} · Holes {}–{}", pa + 1, ha + 1, hb + 1)
        } else {
            format!("{} – {}", self.label(a), self.label(b))
        }
    }
}

/// How far below its clearance the tool may be and still be at it, mm.
const AT_CLEARANCE: f32 = 0.01;

/// The steps of `plan` (`Step`): split where the tool, between two feed
/// moves, is at the plan's clearance (`step_clearance`), which must be above
/// the stock top. Empty for a plan that turns A, a lathe's, or one with no feed
/// move; one step for a plan that never goes back up there between feeds.
/// A drilling plan's steps are its holes (`Plan::holes`): it splits only
/// where one starts, so a deep drilling's pecks and a helical milling's
/// rings, which rise to the clearance too when it is at the retract height,
/// are in the step of their hole (LIO-549, LIO-555). A plan cut in passes,
/// a Boring's, goes round its holes each pass: a step is one visit, and
/// says its pass (`Step::pass`, LIO-558).
pub fn plan_steps(plan: &crate::simulation::Plan) -> Vec<Step> {
    use crate::simulation::Motion;
    let moves = &plan.moves;
    let feed = |i: usize| moves[i].motion == Motion::Feed;
    let Some(last) = (0..moves.len()).rposition(feed) else {
        return vec![];
    };
    if plan.is_lathe() || moves.iter().any(|m| m.a.is_some()) {
        return vec![];
    }
    let clear = step_clearance(plan);
    if clear <= plan.stock.max[2] {
        return vec![];
    }
    // Between two feeds, the first move that starts at the clearance; a
    // stretch of rapids that only rises to it starts the step after them.
    // A drilling plan's, only where a hole starts between them.
    let holes = &plan.holes;
    let hole_between = |f: usize, i: usize| {
        let h = holes.partition_point(|&h| h <= f);
        holes.get(h).is_some_and(|&h| h <= i)
    };
    let mut starts = vec![0];
    let mut fed: Option<usize> = None;
    for i in 0..=last {
        if feed(i) {
            if let Some(f) = fed
                && i > f + 1
                && (holes.is_empty() || hole_between(f, i))
                && let Some(s) = (f + 1..=i).find(|&j| moves[j].from[2] >= clear - AT_CLEARANCE)
            {
                starts.push(s);
            }
            fed = Some(i);
        }
    }
    // Where each pass is set, of a plan cut in more than one (`Step::pass`).
    let mut sets: Vec<usize> = (plan.stops.iter())
        .filter(|s| s.size.is_some())
        .map(|s| s.at)
        .collect();
    sets.sort_unstable();
    let ends = starts.iter().skip(1).copied().chain([moves.len()]);
    (starts.iter().zip(ends))
        .map(|(&a, b)| {
            // Set before its first feed move, the stop after the retract
            // that starts it.
            let first_feed = (a..b).find(|&i| feed(i)).unwrap_or(a);
            let pass = (sets.len() > 1).then(|| {
                sets.partition_point(|&at| at <= first_feed)
                    .saturating_sub(1)
            });
            let level: Vec<f32> = (moves[a..b].iter())
                .filter(|m| m.motion == Motion::Feed && (m.from[2] - m.to[2]).abs() < 1e-4)
                .map(|m| m.to[2])
                .collect();
            let z = level
                .first()
                .copied()
                .filter(|z| level.iter().all(|v| (v - z).abs() < 0.005));
            Step {
                moves: a..b,
                z,
                seconds: moves[a..b].iter().map(|m| m.end - m.start).sum(),
                feeds: moves[a..b]
                    .iter()
                    .filter(|m| m.motion == Motion::Feed)
                    .count(),
                pass,
            }
        })
        .collect()
}

/// The clearance a plan's steps start at (`plan_steps`): the highest Z it
/// moves at from its first feed move on. Its way in may come from higher, as
/// a chuck-held plan does from over what its jaws swing
/// (`indexed::enter_held`), but it never rises there again, so steps by that
/// height would be one: a bore at a time is the clearance it retracts to
/// (LIO-555). The highest Z of all for a plan without a feed move. Air
/// trimming's links between levels rise to it too (`air::relink`, LIO-565).
pub(crate) fn step_clearance(plan: &crate::simulation::Plan) -> f32 {
    use crate::simulation::Motion;
    let moves = &plan.moves;
    let first = (moves.iter())
        .position(|m| m.motion == Motion::Feed)
        .unwrap_or(0);
    (moves[first..].iter())
        .map(|m| m.from[2].max(m.to[2]))
        .fold(f32::NEG_INFINITY, f32::max)
}

/// The steps of setup `si`'s operations that generated, from the check's
/// simulations `sims` (`plan_steps`).
fn op_steps(doc: &Document, si: usize, sims: &Sims) -> Vec<OpSteps> {
    (sims.iter())
        .filter_map(|(oi, r)| {
            let plan = &r.as_ref().ok()?.plan;
            let op = doc.setups[si].operations.get(*oi)?;
            Some(OpSteps {
                op: *oi,
                clear: step_clearance(plan),
                hole: op.strategy == crate::model::Strategy::Drill,
                steps: plan_steps(plan),
                fingerprint: fingerprint(plan, &op.tool),
            })
        })
        .collect()
}

/// FNV-1a over every bit of `plan`'s moves and the id of `tool`: stable
/// across builds, unlike `std`'s hasher, for a journal kept on disk.
pub fn fingerprint(plan: &crate::simulation::Plan, tool: &str) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    let mut eat = |w: u64| h = (h ^ w).wrapping_mul(0x100000001b3);
    for m in &plan.moves {
        for v in m.from.iter().chain(&m.to).chain([&m.feed]) {
            eat(u64::from(v.to_bits()));
        }
        eat(m.motion as u64);
    }
    for b in tool.bytes() {
        eat(u64::from(b));
    }
    h
}

/// `plan` without its `steps` `cut` (indexes in `steps`, not all of them):
/// the moves of the others, in order. Where cut steps were, a rapid across
/// at the clearance joins the step before them, where it rose to it, to
/// where the one after them starts, as the plan moves between two of its
/// steps; cut first, the plan starts where the first kept step does, which
/// a program enters as it enters an operation. Its stops follow their moves,
/// but for a pass's setting cut after the plan has cut (`Step::pass`), and a
/// drilling plan's kept steps are its holes (`Plan::holes`).
pub fn cut_plan(
    plan: &crate::simulation::Plan,
    steps: &[Step],
    cut: &[usize],
) -> crate::simulation::Plan {
    use crate::simulation::{Motion, Move, Stop};
    let mut out = crate::simulation::Plan {
        moves: vec![],
        stops: vec![],
        keep_air: vec![],
        corner_walks: vec![],
        slot_passes: vec![],
        holes: vec![],
        ..plan.clone()
    };
    let mut new = vec![None; plan.moves.len()];
    let mut gap = false;
    for (s, step) in steps.iter().enumerate() {
        if cut.contains(&s) {
            gap = true;
            continue;
        }
        let first = &plan.moves[step.moves.start];
        if let Some(last) = out.moves.last().filter(|_| gap).cloned() {
            out.keep_air.push(false);
            out.moves.push(Move {
                from: last.to,
                to: first.from,
                motion: Motion::Rapid,
                cutting_pass: false,
                feed: first.feed,
                start: last.end,
                end: last.end,
                a: None,
            });
        }
        gap = false;
        if !plan.holes.is_empty() {
            out.holes.push(out.moves.len());
        }
        for i in step.moves.clone() {
            new[i] = Some(out.moves.len());
            out.moves.push(plan.moves[i].clone());
            out.keep_air
                .push(plan.keep_air.get(i).copied().unwrap_or(false));
        }
    }
    let kept = |s: usize| !cut.contains(&s);
    // A pass's setting of the tool (`Step::pass`) cut with the pass's first
    // visit goes before its first visit sent, when the plan cut at an
    // earlier pass's setting before it: the visit skipped alone, the holes
    // after it are not cut at that pass's size yet (LIO-558). Cut before
    // anything sent, as a start at a step is, the tool is taken as set for
    // the pass under way.
    let moved = |s: &Stop| -> Option<usize> {
        s.size?;
        let c = steps.iter().position(|t| t.moves.contains(&s.at))?;
        let pass = steps[c].pass?;
        let t = (c + 1..steps.len()).find(|&t| kept(t) && steps[t].pass == Some(pass))?;
        (0..t).any(kept).then_some(())?;
        new[steps[t].moves.start]
    };
    out.stops = (plan.stops.iter())
        .filter_map(|s| {
            Some(Stop {
                at: new.get(s.at).copied().flatten().or_else(|| moved(s))?,
                ..s.clone()
            })
        })
        .collect();
    out.stops.sort_by_key(|s| s.at);
    out
}

/// This setup's cuts by the 5 mm square of the grid their removal box
/// reaches (`rest::removal_box`), each square's in the order they were
/// made: which of them first removed a point (`rest::FirstCut`) looks at a
/// few.
struct Squares<'a> {
    lo: [f32; 2],
    n: [usize; 2],
    first: Vec<crate::rest::FirstCut<'a>>,
}

const SQUARE: f32 = 5.;

impl<'a> Squares<'a> {
    fn new(cuts: &[&'a crate::stock_surface::Cut], frame: crate::volume::Frame) -> Self {
        let boxes: Vec<_> = (cuts.iter())
            .map(|c| crate::rest::removal_box(c, frame))
            .collect();
        let lo = [0, 1].map(|k| boxes.iter().map(|b| b.min[k]).fold(f32::INFINITY, f32::min));
        let hi = [0, 1].map(|k| {
            boxes
                .iter()
                .map(|b| b.max[k])
                .fold(f32::NEG_INFINITY, f32::max)
        });
        if boxes.is_empty() {
            return Self {
                lo,
                n: [0; 2],
                first: vec![],
            };
        }
        // ponytail: 400 squares a side at most, the last one then reaching
        // to the far end; wider squares if that is ever slow.
        let n = [0, 1].map(|k| (((hi[k] - lo[k]) / SQUARE) as usize + 1).min(400));
        let mut lists = vec![vec![]; n[0] * n[1]];
        let at = |v: f32, k: usize| (((v - lo[k]) / SQUARE).max(0.) as usize).min(n[k] - 1);
        for (c, b) in cuts.iter().zip(&boxes) {
            for y in at(b.min[1], 1)..=at(b.max[1], 1) {
                for x in at(b.min[0], 0)..=at(b.max[0], 0) {
                    lists[y * n[0] + x].push(*c);
                }
            }
        }
        Self {
            lo,
            n,
            first: (lists.into_iter())
                .map(|l| crate::rest::FirstCut::new(l, frame))
                .collect(),
        }
    }

    fn by(&self, p: [f32; 3]) -> Option<(u16, u16)> {
        let at = |k: usize| {
            let i = ((p[k] - self.lo[k]) / SQUARE).floor();
            (i >= 0. && self.n[k] > 0).then(|| (i as usize).min(self.n[k] - 1))
        };
        let (x, y) = (at(0)?, at(1)?);
        self.first.get(y * self.n[0] + x)?.by(p)
    }
}

/// The operations of setup `si` (index in it) whose material a run cannot
/// leave (`Skip::Left`), each with why: a later one of the setup, not taken
/// as cut (`cut`), cuts beneath it. Its feed moves would then cut what it
/// leaves at a feed and depth made for less, which the checks see only of
/// rapids. From the check's simulations `sims`: at points 0.05 mm over the
/// tool tip, every millimetre along each later operation's feed moves, where
/// the setup's stock had material, the operation whose cut first removed it.
fn beneath(doc: &Document, si: usize, sims: &Sims, cut: &[usize]) -> Vec<(usize, String)> {
    use crate::simulation::{Motion, Simulation};
    let ok: Vec<(usize, &Simulation)> = (sims.iter())
        .filter_map(|(oi, r)| Some((*oi, r.as_ref().ok()?)))
        .collect();
    let (Some((_, first)), Some((_, last))) = (ok.first(), ok.last()) else {
        return vec![];
    };
    let (Some(start), Some(volume)) = (&first.initial_volume, &last.initial_volume) else {
        return vec![];
    };
    let own: Vec<&crate::stock_surface::Cut> = (volume.cuts.iter())
        .filter(|c| c.source.0 == first.source.0)
        .collect();
    let mut squares = std::collections::HashMap::new();
    let mut out: Vec<(usize, String)> = vec![];
    for &(oi, sim) in ok.iter().skip(1).filter(|(oi, _)| !cut.contains(oi)) {
        for m in sim.plan.moves.iter().filter(|m| m.motion == Motion::Feed) {
            let Some(frame) = sim.frame_at(m.a) else {
                continue;
            };
            let squares =
                (squares.entry(m.a.map(f32::to_bits))).or_insert_with(|| Squares::new(&own, frame));
            let length = (0..3)
                .map(|k| (m.to[k] - m.from[k]).powi(2))
                .sum::<f32>()
                .sqrt();
            let steps = length.ceil().max(1.) as usize;
            for j in 0..=steps {
                let t = j as f32 / steps as f32;
                let mut p: [f32; 3] =
                    std::array::from_fn(|k| m.from[k] + t * (m.to[k] - m.from[k]));
                p[2] += 0.05;
                if !start.filled(p, frame) {
                    continue;
                }
                if let Some((s, earlier)) = squares.by(p)
                    && s == sim.source.0
                    && earlier < sim.source.1
                    && !out.iter().any(|b| b.0 == earlier as usize)
                {
                    let name = doc.setups[si].operations.get(oi).map_or("", |o| o.name());
                    out.push((
                        earlier as usize,
                        format!("{name} cuts beneath it: its moves would cut the material left at a feed made for less."),
                    ));
                }
            }
        }
    }
    out
}

pub(crate) const NO_PART: &str = "Import a part before running a setup. The example project's paths are illustrative and cannot be machined.";
pub const REMOVED: &str = "This setup was removed from the project.";

/// The `sequence` results of setup `si`'s enabled operations, from a
/// simulation of the job up to the end of that setup.
fn simulated(doc: &Document, mesh: &Mesh, si: usize) -> Sims {
    simulated_with(doc, mesh, si).0
}

/// `simulated`, for tests elsewhere that check a setup as export does.
#[cfg(test)]
pub(crate) fn simulated_for_tests(doc: &Document, mesh: &Mesh, si: usize) -> Sims {
    simulated(doc, mesh, si)
}

/// A setup's `sequence` results, by its operations' index in the setup.
type Sims = Vec<(usize, Result<crate::simulation::Simulation, String>)>;

/// `simulated`, and the stock it leaves after each operation (`Left`).
fn simulated_with(doc: &Document, mesh: &Mesh, si: usize) -> (Sims, Left) {
    let last = doc.setups[si].operations.iter().rposition(|o| o.enabled);
    let mut sims = vec![];
    let mut after = vec![];
    if let Some(oi) = last {
        crate::simulation::sequence_with(
            doc,
            mesh,
            Some((si, oi)),
            |s, oi, r| {
                if s == si {
                    sims.push((oi, r))
                }
            },
            |s, oi, v| {
                if s == si {
                    after.push((oi, v.clone()))
                }
            },
        );
    }
    // The walk stops at the last operation, before playing it to its end.
    let last = sims
        .last()
        .filter(|(oi, _)| Some(*oi) == last)
        .and_then(|(oi, r)| Some((*oi, r.as_ref().ok()?.clone())));
    let before = (sims.iter())
        .filter_map(|(oi, r)| {
            let sim = r.as_ref().ok()?;
            let stock = match (&sim.initial_volume, sim.frame) {
                (Some(v), Some(frame)) => StockGrid::from_volume(sim.plan.stock, v, frame),
                _ => sim.initial_stock.clone(),
            };
            Some(OpStock {
                op: *oi,
                stock,
                shape: sim.plan.shape,
                volume: sim.initial_volume.clone(),
                flute: sim.plan.flute_length,
                frame: sim.frame,
                turns: sim.turns.clone(),
            })
        })
        .collect();
    (
        sims,
        Left {
            after,
            last,
            before: Arc::new(before),
            beneath: vec![],
            steps: vec![],
        },
    )
}

/// The stock a run's check leaves after each operation of its setup, for
/// their pictures (`op_pictures`).
#[derive(Debug, Clone, Default)]
pub struct Left {
    /// The stock each operation but the last leaves, by its index in the
    /// setup: none from an operation that could not be generated on.
    pub after: Vec<(usize, crate::volume::Volume)>,
    /// The last operation's simulation, which `op_pictures` plays to its end:
    /// the check does not need the stock it leaves.
    pub last: Option<(usize, crate::simulation::Simulation)>,
    /// The stock before each operation that could be generated on, seen
    /// from above (`Run::before`).
    pub before: Arc<Vec<OpStock>>,
    /// The operations whose material a run cannot leave, each with why
    /// (`beneath`).
    pub beneath: Vec<(usize, String)>,
    /// The steps of each operation that generated (`op_steps`), as found
    /// before any was taken as cut.
    pub steps: Vec<OpSteps>,
}

/// How many stocks `op_pictures` meshes at once: each mesher also runs
/// threads of its own.
const PICTURING_THREADS: usize = 4;

/// Pictures of the stock each operation leaves (`left`), by its index in
/// the setup: meshed at Draft detail and seen in the setup's `frame` with the
/// boxes `jaws` (its jaws, clamps or pins, in its frame in mm) as the 3D view
/// first shows the setup, `viewport::OP_PICTURE` px. What each operation
/// cut is in its own colour (`viewport::stock_triangles`); `si` is the
/// setup's index in the document.
pub fn op_pictures(
    left: &Left,
    si: usize,
    frame: crate::volume::Frame,
    jaws: &[crate::mesh::Bounds],
) -> Vec<(usize, iced::widget::image::Handle)> {
    let holding: Vec<_> = jaws
        .iter()
        .flat_map(|b| crate::viewport::box_triangles(*b, crate::viewport::JAWS))
        .collect();
    let picture = |oi: usize, volume: &crate::volume::Volume| {
        let surface =
            crate::stock_surface::Snapshot::of(volume, crate::stock_surface::Detail::Draft)
                .build()
                .ok()?;
        let since = (volume.cuts.iter()).position(|c| c.source == (si as u16, oi as u16));
        let mut triangles: Vec<_> =
            crate::viewport::stock_triangles(volume, &surface, frame, since).collect();
        triangles.extend(holding.iter().cloned());
        crate::viewport::stock_picture(&triangles, crate::viewport::OP_PICTURE)
    };
    let mut end = None;
    if let Some((oi, sim)) = &left.last {
        let mut sim = sim.clone();
        sim.seek(1.);
        end = sim.volume.map(|v| (*oi, v));
    }
    let stocks: Vec<&(usize, crate::volume::Volume)> = left.after.iter().chain(&end).collect();
    // A few stocks are meshed at once: most of a mesh's time is the field of
    // its cuts, built on one thread.
    let mut pictures = vec![];
    for chunk in stocks.chunks(PICTURING_THREADS) {
        std::thread::scope(|scope| {
            let drawn: Vec<_> = chunk
                .iter()
                .map(|(oi, volume)| (*oi, scope.spawn(|| picture(*oi, volume))))
                .collect();
            for (oi, h) in drawn {
                if let Ok(Some(p)) = h.join() {
                    pictures.push((oi, p));
                }
            }
        });
    }
    pictures
}

/// What the checks before a run find for one setup, for the browser's marks
/// and the simulation panel (`ui::op_mark`), worked out for the job whose
/// fingerprint is `key` (`App::holding_key`).
#[derive(Debug, Clone, Default)]
pub struct Ready {
    pub key: u64,
    pub setup: usize,
    /// The operations (index in the setup) that generated.
    pub generated: Vec<usize>,
    /// The workholding problems of its programs, each with its operation.
    pub held: Vec<crate::post::Held>,
    /// Why it cannot run: what a run refuses before Cycle Start without the
    /// connected machine (`plan_problems`), export's refusal for a machine
    /// that is not a UC100. Empty when it can.
    pub problems: Vec<String>,
    /// Whether its work zero is on the stock it starts from and how a probe
    /// finds it (`post::zero_checked`). None when the setup cannot be walked.
    pub zero: Option<crate::post::ZeroCheck>,
}

/// `Ready` of setup `si`: `plan_problems` (`post::checked` for a machine that
/// is not a UC100), and what of its workholding they refuse, operation by
/// operation (`post::held_walked`).
pub(crate) fn ready(doc: &Document, mesh: &Mesh, si: usize, key: u64) -> Ready {
    let sims = simulated(doc, mesh, si);
    let problems = if doc.file.is_none() {
        Err(vec![NO_PART.into()])
    } else if crate::model::get(&doc.setups[si].machine.p, "controller") == "UC100" {
        plan_problems(doc, mesh, si, &sims)
    } else {
        crate::post::checked(doc, mesh, si, &sims, PostSettings::from_machine).map(|_| ())
    };
    // A setup that cannot be walked: its holding was not checked. Said as a
    // problem, not as a refusal of its jaws or clamps.
    let mut problems = problems.err().unwrap_or_default();
    // An indexed setup's moves on the machine: its travel, Z reach and A;
    // and a cut parting the bar in the chuck. Each said once: the export's
    // check says them too.
    for (_, message) in crate::rotary_check::problems(doc, mesh, si, &sims) {
        if !problems.contains(&message) {
            problems.push(message);
        }
    }
    let held = match crate::post::held_walked(doc, mesh, si, &sims) {
        Ok(held) => held,
        Err(errors) => {
            problems.push(crate::post::not_checked(&errors));
            vec![]
        }
    };
    Ready {
        key,
        setup: si,
        generated: sims
            .iter()
            .filter(|(_, r)| r.is_ok())
            .map(|(oi, _)| *oi)
            .collect(),
        held,
        problems,
        zero: crate::post::zero_checked(doc, mesh, si, &sims),
    }
}

/// X, Y and Z without a machine position (every E-stop clears it).
fn unset_axes(c: &Connection) -> Vec<&'static str> {
    unset_ids(c).into_iter().map(|a| AXES[a]).collect()
}

/// X, Y and Z without a machine position, by index.
fn unset_ids(c: &Connection) -> Vec<usize> {
    (0..3).filter(|&a| !c.snapshot.referenced[a]).collect()
}

/// What Cycle Start does not check while the profile sets no soft limits on
/// X, Y or Z, or one of them has no machine position: nothing stops the
/// machine at the end of its travel, and the job is not checked against it.
/// It runs anyway, as a tool run moves only in work coordinates (its
/// retracts go to the clearance plane, not machine Z0). None once every
/// axis has both soft limits and a machine position.
pub fn limits_unchecked(c: &Connection) -> Option<String> {
    let unlimited = c.unlimited_axes();
    let mut lines = vec![];
    if !unlimited.is_empty() {
        let names = crate::ui::axis_names(&unlimited);
        lines.push(format!(
            "The UC100 profile sets no soft limits on {names}, so nothing stops the machine at the end of its travel and Cycle Start does not check that the job fits it. Make sure the work zero is on the stock and watch the machine. Add soft_min and soft_max for {names} to the profile to turn them on."
        ));
    }
    // An axis without soft limits is not waiting for its position.
    let unset_i: Vec<usize> = (unset_ids(c).into_iter())
        .filter(|a| !unlimited.contains(a))
        .collect();
    let unset: Vec<&str> = unset_i.iter().map(|&a| AXES[a]).collect();
    if !unset.is_empty() {
        lines.push(format!(
            "{} {} no machine position (every E-stop clears it), so Cycle Start runs with the soft limits off and does not check that the job fits the machine's travel. Make sure the work zero is on the stock (touch it off again after an E-stop, as the drives may lose steps) and watch the machine. {} in the Machine tab to turn them on.",
            unset.join(", "),
            if unset.len() == 1 { "has" } else { "have" },
            c.place(&unset_i, false, false)
        ));
    }
    (!lines.is_empty()).then(|| lines.join(" "))
}

/// What the UC100 profile's `out`puts refuse in a tool run's `ops`: a
/// spindle speed above its maximum, a counterclockwise spindle without a
/// direction output, and coolant it has no output for.
pub fn output_problems(out: &uc100::config::Outputs, ops: &[crate::post::OpState]) -> Vec<String> {
    let mut e = vec![];
    for o in ops {
        if out.spindle_pwm.is_some() && o.rpm as f64 > out.spindle_max_rpm {
            e.push(format!(
                "{}: S{} is above the UC100 profile's {:.0} rpm.",
                o.name,
                o.rpm.round(),
                out.spindle_max_rpm
            ));
        }
        if !o.clockwise && out.spindle_dir.is_none() && out.spindle_ccw.is_none() {
            e.push(format!("{}: the spindle turns counterclockwise, but the UC100 profile has no spindle direction output.", o.name));
        }
        for (on, pin, what) in [
            (o.coolant.1, out.flood, "flood"),
            (o.coolant.0, out.mist, "mist"),
        ] {
            if on && pin.is_none() {
                e.push(format!("{}: uses {what} coolant, but the UC100 profile has no {what} output. Set its coolant to Off, or add the output to the profile.", o.name));
            }
        }
    }
    e
}

/// How the planner checks setup `si` of a UC100 machine, from the job's
/// simulations `sims`: as a run checks it before Cycle Start, without the
/// checks that need the connected machine (its work zero and position).
pub(crate) fn plan_problems(
    doc: &Document,
    mesh: &Mesh,
    si: usize,
    sims: &[(usize, Result<crate::simulation::Simulation, String>)],
) -> Result<(), Vec<String>> {
    plan_held(doc, mesh, si, sims).map_err(|e| e.into_iter().map(|h| h.message).collect())
}
/// `plan_problems`, each problem with the operation it is about.
pub(crate) fn plan_held(
    doc: &Document,
    mesh: &Mesh,
    si: usize,
    sims: &[(usize, Result<crate::simulation::Simulation, String>)],
) -> Result<(), Vec<crate::post::Held>> {
    use crate::post::Held;
    if crate::manual::is_hand(&doc.setups[si].machine) {
        return Ok(());
    }
    let programs =
        crate::post::checked_held(doc, mesh, si, sims, PostSettings::for_uc100, false, &[])?;
    let config = crate::control::profile(&doc.setups[si].machine).map_err(|e| {
        vec![Held {
            operation: None,
            message: e,
        }]
    })?;
    let e: Vec<Held> = (programs.iter())
        .flat_map(|p| p.ops.iter().zip(&p.indexes))
        .flat_map(|(op, oi)| {
            output_problems(&config.outputs, std::slice::from_ref(op))
                .into_iter()
                .map(|message| Held {
                    operation: Some(*oi),
                    message,
                })
        })
        .collect();
    if e.is_empty() { Ok(()) } else { Err(e) }
}

/// The driver actions of a tool run's `program`: plan points become machine
/// coordinates with the live work `offset` (machine = plan − datum +
/// offset), feeds mm/min become mm/s. `start` is the machine position when
/// Cycle Start is pressed: the first retract moves Z alone there, and A, B
/// and C stay where they are. Each action's line is its block's index + 1.
///
/// A retract goes to machine Z 0, the top of Z, when the program's settings
/// retract to machine home (`G53 G0 Z0`, a machine with A whose Safe
/// retract says so), else to the setup clearance plane; it never moves Z
/// down: a tool above that height stays there (LIO-212). The approach after
/// it moves X and Y alone at the height the tool is at, as export's `G0 X Y`
/// does, A turns over the first point and the tool comes down onto it: from
/// wherever the machine is, the way in goes up, across, turns A and comes
/// down, each move alone, as `way_in_problems` checks it.
///
/// On a setup in the chuck, when that way is refused but another is not,
/// the run leaves where the machine is by it (`way_out`, LIO-213): after
/// the retract, A turns alone first so that no jaw stands up, or the tool
/// moves along X alone first to the first point's X before Y, or both.
///
/// An indexed program's A angles become machine A the same way (machine A =
/// A + A's work offset). A turns only in its index blocks (`Block::Index`),
/// a rapid of A alone at A's speed with X, Y and Z still, made where the
/// program has put the tool: above all that A swings, which the check
/// proved (LIO-193). A move that would turn A while X, Y or Z moves is
/// refused, and so is any action list that would (`combined`): a run never
/// moves A together with X, Y or Z (LIO-192).
///
/// `k` is the operation (index in the program) it starts at: from a later
/// one than the first it is lowered from that operation's own program
/// (`Checked::from`), with its safe start and nothing of the operations
/// before it, each action's line its block's index there + 1.
pub fn lower(
    p: &Checked,
    k: usize,
    offset: [f64; 6],
    start: [f64; 6],
) -> Result<Vec<Located>, Vec<String>> {
    let blocks = p.from(k)?;
    let leave = way_out(p, blocks, offset, start).unwrap_or_default();
    lower_blocks(p, blocks, blocks.len(), offset, start, leave)
}

/// A setup's tool runs worked out on the computer for the shop tablet
/// (LIO-475), which plans nothing: each tool run as Cycle Start sends it
/// from its first operation, and from each later one (`starts`, LIO-476),
/// lowered here once in work coordinates, with where Stop after step stops. The tablet file carries them as
/// `runs.json` (`bundle::pack`).
///
/// The tablet makes the job's machine coordinates from the live work
/// offset `o` and where the machine is, `at`, move by move: each axis of a
/// `Move` target is `target[k] + o[k]`, or `at[k]` (where the tool is) when
/// null; a `lift` move's Z is the larger of `lift` (in machine Z, `+ o[2]`
/// unless `lift.machine`) and `at[2]`. `at` is then the target. That is
/// `lower` exactly, but for a move to where the tool already is, which the
/// tablet may drop. A tool run with a `start` (a setup in the chuck) is
/// `lower` only from there, and starts from nowhere else.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TabletSetup {
    pub setup_id: String,
    pub name: String,
    /// The machine profile's file name in the tablet file's `machine/`.
    pub machine_profile: Option<String>,
    /// The work offset its zero is set in ("G54").
    pub work_offset: String,
    /// Why it cannot run, as the computer's Run panel says; no tool runs then.
    pub problems: Vec<String>,
    pub tool_runs: Vec<TabletRun>,
}

/// One tool run of a `TabletSetup`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TabletRun {
    /// The job's name, as the computer names it: "Setup 1 · tool run 2 of 3".
    pub name: String,
    pub tool: TabletTool,
    /// Its tool is not the previous tool run's: the operator changes the
    /// tool and touches off its Z before it (`Run::tool_change`).
    pub tool_change: bool,
    pub operations: Vec<TabletOperation>,
    /// Where a retract lifts the tool, and Stop after step after its stop
    /// (`Run::step_lift`): a work Z, or a machine Z when `machine` (a
    /// machine that retracts to machine Z 0).
    pub lift: Lift,
    /// On a setup in the chuck, where the machine must stand for Cycle
    /// Start (LIO-482); null elsewhere, where it starts from anywhere.
    pub start: Option<TabletStart>,
    pub actions: Vec<TabletLine>,
    /// Its operations' steps, in order.
    pub steps: Vec<TabletStep>,
    /// Its starts at each of its later operations (LIO-476), in order: the
    /// operations before taken as cut, as a Start here on the Run panel
    /// starts there (`Checked::starts`). A file from before them has none.
    #[serde(default)]
    pub starts: Vec<TabletOpStart>,
    /// The job line its way out starts on: the spindle and coolant off,
    /// the retract, the end (`way_out_line`). A tablet job that skips its
    /// last steps ends with the lines from there (LIO-567).
    #[serde(default)]
    pub way_out: Option<u32>,
    /// Its starts at each step of its operations after the step's first
    /// (LIO-567), in order: as a start at a step on the Run panel, the
    /// steps before it taken as cut, each checked so (`check_steps`). The
    /// tablet starts at a step with one, and joins them to skip steps. A
    /// file from before them has none.
    #[serde(default)]
    pub step_starts: Vec<TabletStepStart>,
    /// For a later tool run, started at its first operation with the tool
    /// runs before taken as cut: its work zero on the stock they left
    /// (LIO-548). Null for the first tool run, and in a file from before.
    #[serde(default)]
    pub zero: Option<TabletZero>,
}

/// A tool run started at one of its operations after the first, as Cycle
/// Start sends it from there: its own job (its lines numbered from 1), the
/// steps from there on, and the lift; or why it cannot start there.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TabletOpStart {
    /// The operation it starts at: its index in the setup.
    pub operation: usize,
    /// Why it cannot start there, as the Run panel says; its job is empty then.
    pub problems: Vec<String>,
    pub lift: Lift,
    pub actions: Vec<TabletLine>,
    pub steps: Vec<TabletStep>,
    /// The job line of its way down onto its first point (LIO-567): the
    /// lines before it take the tool across in the air from wherever it
    /// stands, which is where the tablet's Stop after step stops a job
    /// that goes on here after a skipped step.
    #[serde(default)]
    pub entry: Option<u32>,
    /// As `TabletRun::way_out`.
    #[serde(default)]
    pub way_out: Option<u32>,
    /// Its work zero on the stock the operations before it left (LIO-548);
    /// null when it cannot start there, and in a file from before.
    #[serde(default)]
    pub zero: Option<TabletZero>,
}

/// A tool run started at a step of one of its operations after the step's
/// first (LIO-567), as Cycle Start sends it from a start at a step: the
/// setup checked with that operation's steps before it taken as cut
/// (`check_steps`), its own job (its lines numbered from 1), its steps from
/// there on, and the lift; or why it cannot start there.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TabletStepStart {
    /// The operation it starts at (its index in the setup) and the step.
    pub operation: usize,
    pub step: usize,
    /// Why it cannot start there, as the Run panel says; its job is empty then.
    pub problems: Vec<String>,
    pub lift: Lift,
    /// Without the wait over the step's entry (`wait`). With `rest`, only
    /// its lines before the rest.
    pub actions: Vec<TabletLine>,
    /// Its job goes on as its tool run's own does (`TabletRun::actions`)
    /// from line `rest[0]` on, each of those lines its line `rest[0]` is
    /// `rest[1]` here, after `actions`: the same blocks from the step on,
    /// carried once, not once per step (`share`). None: `actions` is all.
    #[serde(default)]
    pub rest: Option<[u32; 2]>,
    pub steps: Vec<TabletStep>,
    /// As `TabletOpStart::entry`.
    pub entry: Option<u32>,
    /// Started here by the operator, the job waits over the step's entry
    /// with the spindle on until Resume feeds in, as `Run::job_actions`
    /// makes it: a pause with this reason after the last action on or
    /// before line `entry`. Not when the tablet's job goes on here after a
    /// skipped step.
    pub wait: String,
    /// As `TabletRun::way_out`.
    pub way_out: Option<u32>,
    /// Its work zero on the stock the steps before it, and the operations
    /// before its operation, left (LIO-579), as the Work zero tab finds it
    /// for the Run panel's start at that step (`Run::partly` inside an
    /// operation); null when it cannot start there, and in a file from
    /// before.
    #[serde(default)]
    pub zero: Option<TabletZero>,
}

/// The work zero of a start partway through a setup (`Run::partway`) on
/// the stock the operations before it left, as the Work zero tab guides
/// it: X and Y stay those they were cut with, and the tool tip touched off
/// at the stock top's touch-off spot sets Z (`Run::touch_off`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TabletZero {
    /// The work Z the touch-off at `spot` sets: the height of the surface
    /// they left there. Null when the check cannot tell it (`blocked`).
    pub set_z: Option<f64>,
    /// Why the touch-off cannot set Z, as the Work zero tab says
    /// (`Run::partway_blocked`); null when it can.
    pub blocked: Option<String>,
    /// What of the stock the work zero is found on they cut ("top"): empty
    /// when they left it as the setup's first operation finds it
    /// (`Partly::cut`).
    pub cut: Vec<String>,
    /// They cut below the stock top somewhere over it: a touch plate laid
    /// on the top may not lie at a height the check knows.
    pub top_cut: bool,
    /// The stock top's work Z before the setup's first operation.
    pub top: f64,
    /// Where on the stock top the tool tip touches off, in the computer's
    /// words for the machine's orientation: "the front-left corner", "the
    /// centre". Null when the work zero is not on the stock.
    pub spot: Option<String>,
    /// The stock-top spots a probe finds the work zero from on the stock
    /// they left, each with why it cannot (`Partly::probe_from`).
    pub probe_from: Vec<TabletSpot>,
}

/// A stock-top spot a probe starts from: X and Y each on the stock's −
/// side, its middle or its + side.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TabletSpot {
    pub x: crate::post::At,
    pub y: crate::post::At,
    /// Why a probe cannot find the work zero from there; null when it can.
    pub problem: Option<String>,
}

/// Where a tool run on a setup in the chuck starts from: the tool at the
/// lift (`TabletRun::lift`) over the first point it comes down onto, A at
/// the angle it cuts that point at. Work X, Y and A, made machine ones as
/// a move's target is. From there its way in is straight down, which the
/// computer checked clear of the jaws (`run::way_out`); from anywhere else
/// only the computer can check it, so the tablet refuses Cycle Start
/// until X, Y, Z and A are there.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TabletStart {
    pub x: f64,
    pub y: f64,
    pub a: f64,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TabletTool {
    pub number: u32,
    pub name: String,
    /// As the computer shows it: "T1 Flat 10".
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TabletOperation {
    /// Its index in the setup.
    pub index: usize,
    pub name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Lift {
    pub z: f64,
    pub machine: bool,
}

/// An action of the job and its job line: the driver's `Located`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TabletLine {
    pub line: u32,
    #[serde(flatten)]
    pub action: TabletAction,
}

/// The driver's `Action`, a move's target in work coordinates
/// (`TabletSetup`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum TabletAction {
    Move {
        /// X, Y, Z, A, B, C; null: where the tool is.
        target: [Option<f64>; 6],
        /// mm/s; null: rapid.
        feed: Option<f64>,
        /// A retract: Z rises to the lift height, unless higher already.
        lift: bool,
    },
    Dwell {
        seconds: f64,
    },
    Spindle {
        state: Spindle,
        rpm: f64,
    },
    Coolant {
        flood: bool,
        mist: bool,
    },
    Pause {
        reason: String,
    },
    End,
}

/// A step of an operation (`Step`) in a tool run's job.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TabletStep {
    /// Its operation's index in the setup.
    pub operation: usize,
    /// Its index among the operation's steps.
    pub step: usize,
    /// "Level 3", "Hole 2", "Pass 1".
    pub label: String,
    /// The first and last job lines it is cut on: the job is in this step
    /// while its line is in there.
    pub lines: Option<[u32; 2]>,
    /// The job lines Stop after step stops on at its end, runs of lines
    /// as first and last (`Run::stop_ranges`): the first time the job's
    /// line is in one, the tool is back at its clearance.
    pub end_lines: Vec<[u32; 2]>,
}

/// The tool runs of setup `si` for the tablet: those Cycle Start sends
/// after the Run panel's check, or why it refuses them.
/// `mesh` is the part, or why it could not be loaded.
pub fn tablet_runs(doc: &Document, mesh: Result<&Mesh, &str>, si: usize) -> TabletSetup {
    let setup = &doc.setups[si];
    let profile = crate::model::get(&setup.machine.p, "uc100_profile");
    let mut out = TabletSetup {
        setup_id: setup.id.clone(),
        name: setup.name().to_string(),
        machine_profile: (profile.rsplit(['/', '\\']).next())
            .filter(|n| !n.trim().is_empty())
            .map(|n| n.trim().to_string()),
        work_offset: String::new(),
        problems: vec![],
        tool_runs: vec![],
    };
    let mesh = match mesh {
        Ok(m) => m,
        Err(why) => {
            out.problems = vec![why.to_string()];
            return out;
        }
    };
    let (programs, left) = check(doc, mesh, si);
    let left = Arc::new(left);
    let mut run = Run::new(doc, si, 0);
    run.checked_as(Arc::new(programs), Some(left.clone()), None);
    let all = run.programs.clone().expect("checked");
    // Each start's work zero as the Run panel finds it started there.
    let mut started = Run::new(doc, si, 0);
    started.checked_as(all.clone(), Some(left), None);
    let orient = crate::orient::Orient::of(&setup.machine);
    let mut zero = |i: usize, k: usize| {
        started.start_at(i, k);
        tablet_zero(&started, orient)
    };
    let programs = match all.as_ref() {
        Ok(p) => p,
        Err(why) => {
            out.problems = why.clone();
            return out;
        }
    };
    out.work_offset = programs.first().map_or(String::new(), |p| p.offset.clone());
    let count = programs.len();
    for (i, p) in programs.iter().enumerate() {
        let start = match p.chuck {
            None => None,
            Some(_) => match tablet_start(doc, mesh, si, p) {
                Ok(s) => Some(s),
                Err(why) => {
                    (out.problems, out.tool_runs) = (why, vec![]);
                    return out;
                }
            },
        };
        let (actions, steps, lift) = match tablet_job_from(&mut run, p, i, 0) {
            Ok(job) => job,
            Err(why) => {
                (out.problems, out.tool_runs) = (why, vec![]);
                return out;
            }
        };
        // A setup in the chuck starts only over the first point of its tool
        // run, the one way in past the jaws that is checked (`tablet_start`).
        let starts = (1..p.ops.len())
            .map(|k| {
                let job = match p.chuck {
                    Some(_) => Err(vec![CHUCK_STARTS.to_string()]),
                    None => tablet_job_from(&mut run, p, i, k),
                };
                let (actions, steps, at_lift, problems, zero) = match job {
                    Ok((a, s, l)) => (a, s, l, vec![], zero(i, k)),
                    Err(why) => (vec![], vec![], lift, why, None),
                };
                let blocks = p.from(k).ok().filter(|_| problems.is_empty());
                TabletOpStart {
                    operation: p.indexes[k],
                    problems,
                    lift: at_lift,
                    actions,
                    steps,
                    entry: blocks.and_then(entry_line),
                    way_out: blocks.and_then(way_out_line),
                    zero,
                }
            })
            .collect();
        let step_starts = tablet_step_starts(doc, mesh, si, &run, p, lift, &actions);
        let o = &p.ops[0];
        out.tool_runs.push(TabletRun {
            name: format!("{} · tool run {} of {count}", run.name, i + 1),
            tool: TabletTool {
                number: o.tool,
                name: o.tool_name.clone(),
                label: crate::model::tool_label(o.tool, &o.tool_name),
            },
            tool_change: i > 0 && programs[i - 1].ops[0].tool_id != o.tool_id,
            operations: (p.indexes.iter().zip(&p.ops))
                .map(|(&index, o)| TabletOperation {
                    index,
                    name: o.name.clone(),
                })
                .collect(),
            lift,
            start,
            actions,
            steps,
            starts,
            way_out: way_out_line(&p.blocks),
            step_starts,
            zero: if i > 0 { zero(i, 0) } else { None },
        });
    }
    out
}

/// The work zero of `run`, started partway (`Run::partway`), on the stock
/// the operations before its start left (`TabletZero`), on a machine of
/// orientation `orient`; None when it does not start partway.
fn tablet_zero(run: &Run, orient: crate::orient::Orient) -> Option<TabletZero> {
    if !run.partway() {
        return None;
    }
    let p = run.checked()?.first()?;
    let partly = run.partly();
    let blocked = run.partway_blocked();
    let set_z = run.touch_off().map(|(_, work)| work[2]);
    Some(TabletZero {
        set_z,
        // Not on the stock top, the touch-off cannot set Z either, as
        // Probe Z says (`Connection::stock_top`).
        blocked: blocked.or_else(|| {
            set_z
                .is_none()
                .then(|| crate::post::STOCK_TOP_CUT.to_string())
        }),
        cut: partly.as_ref().map_or_else(Vec::new, |x| x.cut.clone()),
        top_cut: partly.as_ref().is_some_and(|x| x.top_cut),
        top: (p.stock.max[2] - p.datum[2]) as f64,
        spot: (crate::post::references(&p.origin))
            .map(|refs| crate::control::touch_spot(orient, refs)),
        probe_from: (partly.as_ref())
            .map_or(&[][..], |x| &x.probe_from[..])
            .iter()
            .map(|([x, y], ok)| TabletSpot {
                x: *x,
                y: *y,
                problem: ok.clone().err(),
            })
            .collect(),
    })
}

/// The job line of the first way down onto a point of `blocks` (its first
/// `Block::Descend`): the lines before it take the tool across in the air.
fn entry_line(blocks: &[Block]) -> Option<u32> {
    (blocks.iter())
        .position(|b| matches!(b, Block::Descend { .. }))
        .map(|i| i as u32 + 1)
}

/// The job line a program's way out starts on: its last `Block::Stop`, the
/// spindle and coolant off, after which it only retracts, turns A back to
/// 0 and ends (`post::blocks`).
fn way_out_line(blocks: &[Block]) -> Option<u32> {
    (blocks.iter())
        .rposition(|b| matches!(b, Block::Stop))
        .map(|i| i as u32 + 1)
}

/// The tablet's starts of tool run `p` of setup `si` at each step of its
/// operations after the step's first (LIO-567), as the Run panel's start at
/// a step: the setup checked again with the operation's steps before it
/// taken as cut (`check_steps`), the program from that operation lowered
/// as `tablet_job_from` lowers it, its steps from the start on, and where
/// it waits (`Run::job_actions`); or why it cannot start there. `base` is
/// the run of the setup's own check, whose steps they are; `lift` the tool
/// run's, said with a refused start; `own` its actions, which each start
/// shares the rest of its job with (`share`).
fn tablet_step_starts(
    doc: &Document,
    mesh: &Mesh,
    si: usize,
    base: &Run,
    p: &Checked,
    lift: Lift,
    own: &[TabletLine],
) -> Vec<TabletStepStart> {
    let mut out = vec![];
    for &oi in &p.indexes {
        let Some(os) = base.op_steps(oi) else {
            continue;
        };
        for n in 1..os.steps.len() {
            let mut start = TabletStepStart {
                operation: oi,
                step: n,
                problems: vec![],
                lift,
                actions: vec![],
                rest: None,
                steps: vec![],
                entry: None,
                wait: format!(
                    "Over the entry of {}: Resume feeds in.",
                    base.step_label(oi, n)
                ),
                way_out: None,
                zero: None,
            };
            match tablet_step_job(doc, mesh, si, p.chuck.is_some(), oi, n) {
                Ok((mut actions, steps, lift, entry, way_out, zero)) => {
                    start.rest = share(&mut actions, own);
                    (start.actions, start.steps, start.lift) = (actions, steps, lift);
                    (start.entry, start.way_out, start.zero) = (entry, way_out, zero);
                }
                Err(why) => start.problems = why,
            }
            out.push(start);
        }
    }
    out
}

/// The lines a start's `actions` ends with that its tool run's `own` end
/// with too, but for a line number shifted by the same for each: cut off
/// `actions`, and said as `TabletStepStart::rest`. Whole lines only, at
/// least one; and only when the tool run's lines from there, renumbered,
/// are exactly the lines cut off (`rest_of`). None: nothing cut.
fn share(actions: &mut Vec<TabletLine>, own: &[TabletLine]) -> Option<[u32; 2]> {
    let (mut i, mut j) = (actions.len(), own.len());
    let mut shift = None;
    while i > 0 && j > 0 {
        let (a, b) = (&actions[i - 1], &own[j - 1]);
        let d = a.line as i64 - b.line as i64;
        if a.action != b.action || shift.is_some_and(|s| s != d) {
            break;
        }
        (shift, i, j) = (Some(d), i - 1, j - 1);
    }
    // From a line's first action in both.
    let split = |v: &[TabletLine], k: usize| k > 0 && k < v.len() && v[k - 1].line == v[k].line;
    while i < actions.len() && (split(actions, i) || split(own, j)) {
        (i, j) = (i + 1, j + 1);
    }
    if i >= actions.len() || j >= own.len() || (i > 0 && actions[i - 1].line >= actions[i].line) {
        return None;
    }
    let rest = [own[j].line, actions[i].line];
    (rest_of(own, rest) == actions[i..]).then(|| {
        actions.truncate(i);
        rest
    })
}

/// The lines a `TabletStepStart::rest` stands for: its tool run's `own`
/// from line `rest[0]` on, line `rest[0]` numbered `rest[1]`.
fn rest_of(own: &[TabletLine], rest: [u32; 2]) -> Vec<TabletLine> {
    (own.iter())
        .filter(|l| l.line >= rest[0])
        .map(|l| TabletLine {
            line: l.line - rest[0] + rest[1],
            action: l.action.clone(),
        })
        .collect()
}

/// The job of a start at step `n` of operation `oi` of setup `si` for the
/// tablet (`tablet_step_starts`): its actions, its steps from there on, its
/// lift, entry and way out, and its work zero on the stock the steps before
/// it left (`tablet_zero`); or why it cannot start there. Not in the chuck
/// (`chuck`), whose way in is checked only over a tool run's first point.
#[allow(clippy::type_complexity)]
fn tablet_step_job(
    doc: &Document,
    mesh: &Mesh,
    si: usize,
    chuck: bool,
    oi: usize,
    n: usize,
) -> Result<
    (
        Vec<TabletLine>,
        Vec<TabletStep>,
        Lift,
        Option<u32>,
        Option<u32>,
        Option<TabletZero>,
    ),
    Vec<String>,
> {
    if chuck {
        return Err(vec![CHUCK_STARTS.into()]);
    }
    let (programs, left) = check_steps(doc, mesh, si, &[], &[(oi, (0..n).collect())]);
    let left = Arc::new(left);
    let mut run = Run::new(doc, si, 0);
    // The steps before it taken as cut by the start, as `start_at_step`
    // makes it: the running program's marks count the steps sent after them.
    run.start_step = Some((oi, n));
    run.checked_as(Arc::new(Ok(programs?)), Some(left.clone()), None);
    if run.start_step != Some((oi, n)) {
        return Err(vec![format!(
            "The check did not find {} of this operation again, so the tablet cannot start there. Check the setup again on the computer.",
            run.step_label(oi, n)
        )]);
    }
    let all = run.programs.clone().expect("checked");
    let programs = all.as_ref().as_ref().map_err(Clone::clone)?;
    let Some((i, p, k)) = (programs.iter().enumerate())
        .find_map(|(i, p)| Some((i, p, p.indexes.iter().position(|&o| o == oi)?)))
    else {
        return Err(vec![
            "The check sends this operation in no tool run, so the tablet cannot start at its steps. Check the setup again on the computer.".into(),
        ]);
    };
    let (actions, mut steps, lift) = tablet_job_from(&mut run, p, i, k)?;
    // Those before it are the start's, cut already.
    steps.retain(|t| t.operation != oi || t.step >= n);
    let blocks = p.from(k)?;
    // Its work zero as the Work zero tab finds it for the Run panel's start
    // at that step, on a run of its own: as `start_at_step` leaves it, the
    // check puts Cycle Start at the step's operation (`keep`), so the start
    // is the run's (`at_step`). One it would not be at is said as none.
    let mut started = Run::new(doc, si, 0);
    started.start_step = Some((oi, n));
    started.keep = Some((vec![], Some(oi)));
    started.checked_as(all.clone(), Some(left), None);
    let orient = crate::orient::Orient::of(&doc.setups[si].machine);
    let zero = (started.at_step() == Some((oi, n)))
        .then(|| tablet_zero(&started, orient))
        .flatten();
    Ok((
        actions,
        steps,
        lift,
        entry_line(blocks),
        way_out_line(blocks),
        zero,
    ))
}

/// Why a tablet's tool run on a setup in the chuck does not start at a later
/// operation.
const CHUCK_STARTS: &str = "The stock is held in the chuck: the computer checked the way in past its jaws only over the tool run's first point, so it starts there and nowhere else.";

/// Program `p` (tool run `i` of `run`'s setup) as the tablet sends it from
/// its operation `k`: its actions in work coordinates (`TabletSetup`), its
/// steps from there with the job lines each is cut on and ends on, and the
/// lift; why not when it cannot start there (`Checked::from`). Leaves `run`
/// running it from there, at work offset 0.
fn tablet_job_from(
    run: &mut Run,
    p: &Checked,
    i: usize,
    k: usize,
) -> Result<(Vec<TabletLine>, Vec<TabletStep>, Lift), Vec<String>> {
    let blocks = p.from(k)?;
    // From nowhere (NaN), so that no move is dropped as already there,
    // and each axis kept where the tool is comes out NaN.
    let lowered = lower_blocks(
        p,
        blocks,
        blocks.len(),
        [0.; 6],
        [f64::NAN; 6],
        Leave::default(),
    )?;
    let actions = (lowered.into_iter())
        .map(|l| {
            let block = &blocks[l.line as usize - 1];
            let action = match l.action {
                Do::Move { target, feed } => {
                    // Where the tool is: all of a retract's (its Z the
                    // lift), an approach's Z and A, an index's X, Y and Z;
                    // those are where an earlier move left them, which
                    // `lower` takes from the machine.
                    let kept: &[usize] = match block {
                        Block::Retract => &[0, 1, 2, 3, 4, 5],
                        Block::Approach { .. } => &[2, 3, 4, 5],
                        Block::Index { .. } => &[0, 1, 2, 4, 5],
                        _ => &[],
                    };
                    TabletAction::Move {
                        target: std::array::from_fn(|k| {
                            Some(target[k]).filter(|v| !v.is_nan() && !kept.contains(&k))
                        }),
                        feed,
                        lift: matches!(block, Block::Retract),
                    }
                }
                Do::Dwell { seconds } => TabletAction::Dwell { seconds },
                Do::Spindle { state, rpm } => TabletAction::Spindle { state, rpm },
                Do::Coolant { flood, mist } => TabletAction::Coolant { flood, mist },
                Do::Pause { reason } => TabletAction::Pause { reason },
                Do::End => TabletAction::End,
                Do::Probe { .. } => unreachable!("a run lowers no probe"),
            };
            TabletLine {
                line: l.line,
                action,
            }
        })
        .collect();
    // As running from operation `k`, at work offset 0: what `stop_ranges`
    // and `step_lift` read.
    run.running = Some(Running {
        index: i,
        from: k,
        seq: 0,
        offset: [0.; 6],
        before: Before {
            xy: None,
            zero: None,
            last: None,
            touched: false,
        },
        paths: vec![],
        tip: [0.; 3],
        start: [0.; 6],
        print: 0,
        cut_ops: vec![],
        on: None,
        marks: run.marks_of(p, blocks),
    });
    let mut steps: Vec<TabletStep> = (p.indexes[k..].iter())
        .filter_map(|&oi| run.op_steps(oi))
        .flat_map(|os| {
            (0..os.steps.len()).map(|s| TabletStep {
                operation: os.op,
                step: s,
                label: os.label(s),
                lines: None,
                end_lines: vec![],
            })
        })
        .collect();
    let marks = run.running.as_ref().map_or(&[][..], |r| &r.marks[..]);
    fn of(steps: &mut [TabletStep], at: Option<(usize, usize)>) -> Option<&mut TabletStep> {
        let (oi, s) = at?;
        steps.iter_mut().find(|t| (t.operation, t.step) == (oi, s))
    }
    for b in 0..blocks.len() {
        let line = b as u32 + 1;
        let on = marks.get(b).copied().flatten();
        let on = on
            .and_then(|m| run.mark_step(p, m))
            .map(|(oi, s, _)| (oi, s));
        if let Some(t) = of(&mut steps, on) {
            let span = t.lines.get_or_insert([line, line]);
            span[1] = line;
        }
        if let Some(t) = of(&mut steps, run.step_end(b)) {
            match t.end_lines.last_mut() {
                Some(r) if r[1] + 1 == line => r[1] = line,
                _ => t.end_lines.push([line, line]),
            }
        }
    }
    let lift = Lift {
        z: run.step_lift().expect("running"),
        machine: p.settings.machine_home,
    };
    Ok((actions, steps, lift))
}

/// The `TabletStart` of program `p` of setup `si`, in the chuck: over its
/// first point at the height of its first retract, A at the last index
/// before it comes down (A 0 without one), checked as Cycle Start checks
/// the way in from there (`way_out`) at the setup's own work zero on the
/// machine; why not when that way is refused.
///
/// ponytail: checked at the work zero the setup puts on the machine
/// (`AtAngle::machine_zero`), which only a machine that retracts to
/// machine Z 0 reads (where that is over the part). The computer's live
/// checks of the work zero (`rotary_problems`) are the tablet's to make
/// before it runs (LIO-476).
fn tablet_start(
    doc: &Document,
    mesh: &Mesh,
    si: usize,
    p: &Checked,
) -> Result<TabletStart, Vec<String>> {
    let blocks = &p.blocks;
    let end = (blocks.iter())
        .position(|b| matches!(b, Block::Descend { .. }))
        .map_or(blocks.len(), |i| i + 1);
    let over = blocks[..end].iter().find_map(|b| match b {
        Block::Approach { xy, .. } => Some(*xy),
        _ => None,
    });
    let a = blocks[..end].iter().rev().find_map(|b| match b {
        Block::Index { a, .. } => Some(*a as f64),
        _ => None,
    });
    let Some(xy) = over else {
        return Err(vec![
            "The tool run has no first point to start over. Check the setup again.".into(),
        ]);
    };
    // The work offset that puts the work zero where the setup has it on
    // the machine; without it, any: only machine Z 0 depends on it.
    let offset = match doc.at_angle(si, 0., mesh).ok().and_then(|at| Some((at, at.machine_zero?))) {
        Some((at, zero)) => std::array::from_fn(|k| match k {
            0..3 => (zero[k] - at.zero[k] + p.datum[k]) as f64,
            _ => 0.,
        }),
        None if p.settings.machine_home => return Err(vec![
            "Where the machine's Z 0 is over the stock is not known, so the way in past the chuck's jaws cannot be checked for the tablet: record the rotary unit's geometry in the Machine library, or run this setup from the computer.".into(),
        ]),
        None => [0.; 6],
    };
    let work = |k: usize| xy[k] as f64 - p.datum[k] as f64;
    let at = TabletStart {
        x: work(0),
        y: work(1),
        a: a.unwrap_or(0.),
    };
    let top = match p.settings.machine_home {
        true => 0.,
        false => clearance_plane(blocks) as f64 - p.datum[2] as f64 + offset[2],
    };
    let start = [
        at.x + offset[0],
        at.y + offset[1],
        top,
        at.a + offset[3],
        0.,
        0.,
    ];
    match way_out(p, blocks, offset, start) {
        Ok(leave) if leave == Leave::default() => Ok(at),
        _ => Err(vec![format!(
            "From over its first point (X {:.3} Y {:.3}) at the retract height with A at {:.3}°, the way down onto it is not clear of the chuck, so the tablet has nowhere to start it from. Run this setup from the computer.",
            at.x, at.y, at.a
        )]),
    }
}

/// How a run leaves where the machine is before its approach (LIO-213),
/// at the height of its first retract: A turned first, alone, to machine A
/// `a`; and X first, alone, to the first point's X, before Y.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Leave {
    a: Option<f64>,
    x_first: bool,
}

/// `lower` of program `p`'s `blocks` (its own, or from one of its
/// operations on) up to block `end`, leaving where the machine is as
/// `leave` says.
fn lower_blocks(
    p: &Checked,
    blocks: &[Block],
    end: usize,
    offset: [f64; 6],
    start: [f64; 6],
    leave: Leave,
) -> Result<Vec<Located>, Vec<String>> {
    let plane = clearance_plane(blocks);
    // X, Y and Z of plan point `q`; A, B and C where the tool is (`at`).
    let machine = |q: [f32; 3], at: [f64; 6]| -> [f64; 6] {
        let mut m = at;
        for k in 0..3 {
            m[k] = q[k] as f64 - p.datum[k] as f64 + offset[k];
        }
        m
    };
    // Machine Z 0 is the top of Z's travel.
    let high = match p.settings.machine_home {
        true => 0.,
        false => machine([0., 0., plane], start)[2],
    };
    let mut out = vec![];
    let mut at = start;
    let mut left = false;
    let (mut spindle, mut coolant) = (false, (false, false));
    for (i, block) in blocks[..end].iter().enumerate() {
        let line = i as u32 + 1;
        let mut actions = vec![];
        match block {
            Block::Stop => {
                if coolant != (false, false) {
                    actions.push(Do::Coolant {
                        flood: false,
                        mist: false,
                    });
                    coolant = (false, false);
                }
                if spindle {
                    actions.push(Do::Spindle {
                        state: Spindle::Off,
                        rpm: 0.,
                    });
                    spindle = false;
                }
            }
            Block::Retract => {
                let mut to = at;
                to[2] = high.max(at[2]);
                go(&mut at, to, None, &mut actions);
            }
            Block::Start { .. } => {}
            Block::Run {
                op,
                spindle: s,
                coolant: set,
            } => {
                let o = &p.ops[*op];
                if *s {
                    actions.push(Do::Spindle {
                        state: if o.clockwise {
                            Spindle::Cw
                        } else {
                            Spindle::Ccw
                        },
                        rpm: o.rpm as f64,
                    });
                    spindle = true;
                }
                if *set {
                    actions.push(Do::Coolant {
                        flood: o.coolant.1,
                        mist: o.coolant.0,
                    });
                    coolant = o.coolant;
                }
            }
            Block::Wait { seconds } | Block::Standstill { seconds } => actions.push(Do::Dwell {
                seconds: *seconds as f64,
            }),
            // The run waits, saying it, until Resume: the spindle stopped.
            Block::Pause { message, .. } => actions.push(Do::Pause {
                reason: message.clone(),
            }),
            // X and Y alone, at the retract's height; the first after A
            // and X alone when the way out says so.
            Block::Approach { xy, .. } => {
                let mut to = machine([xy[0], xy[1], 0.], at);
                to[2] = at[2];
                if !left {
                    left = true;
                    if let Some(a) = leave.a {
                        let mut turned = at;
                        turned[3] = a;
                        go(&mut at, turned, None, &mut actions);
                    }
                    if leave.x_first {
                        let mut along = at;
                        along[0] = to[0];
                        go(&mut at, along, None, &mut actions);
                    }
                    to[3] = at[3];
                }
                go(&mut at, to, None, &mut actions)
            }
            Block::Descend { xy, z, .. } => {
                let to = machine([xy[0], xy[1], *z], at);
                go(&mut at, to, None, &mut actions)
            }
            Block::Move {
                kind: Kind::Dwell,
                seconds,
                ..
            } => actions.push(Do::Dwell {
                seconds: *seconds as f64,
            }),
            Block::Move {
                kind, to, feed, a, ..
            } => {
                let feed = (*kind == Kind::Feed).then(|| *feed as f64 / 60.);
                let mut target = machine(*to, at);
                if let Some(a) = a {
                    target[3] = *a as f64 + offset[3];
                }
                if (target[3] - at[3]).abs() > A_SAME {
                    return Err(vec![format!(
                        "Block {line} would turn A from {:.3}° to {:.3}° while X, Y or Z moves. A run turns A only by itself, in an index move.",
                        at[3], target[3]
                    )]);
                }
                target[3] = at[3];
                go(&mut at, target, feed, &mut actions)
            }
            // A alone, at A's rapid: X, Y and Z stay where the tool is.
            Block::Index { a, .. } => {
                let mut to = at;
                to[3] = *a as f64 + offset[3];
                go(&mut at, to, None, &mut actions)
            }
            Block::End => actions.push(Do::End),
        }
        out.extend(actions.into_iter().map(|action| Located { line, action }));
    }
    if let Some(line) = combined(&out, start) {
        return Err(vec![format!(
            "Block {line} would move A together with X, Y or Z. A run turns A only by itself, in an index move."
        )]);
    }
    Ok(out)
}

/// Why Cycle Start cannot take the tool from where the machine is, `start`,
/// into program `p` at the first operation of its `blocks`, with the live
/// work `offset`, on a setup in the chuck (`Checked::chuck`, LIO-212): no
/// way out of there (`way_out`) passes the checks.
fn way_in_problems(
    p: &Checked,
    blocks: &[Block],
    offset: [f64; 6],
    start: [f64; 6],
) -> Vec<String> {
    way_out(p, blocks, offset, start).err().unwrap_or_default()
}

/// How a run takes the tool from where the machine is, `start`, into
/// program `p` at the first operation of its `blocks`, with the live work
/// `offset`: its way in as `lower` makes it (up to the retract height,
/// across, A turning and down onto the first point), each move checked at
/// the A it is made at (`rotary_check::Chuck::way_in`). Wherever the
/// machine stands, at any X, Z and A, after a Stop or before a later
/// operation alike. Refused, another way is tried (LIO-213), checked the
/// same: A turned first, at the retract height over where the tool is, to
/// the nearest angle each way at which no jaw stands up (the shorter turn
/// first, inside A's range), the turn checked against all it sweeps; the
/// tool moved along X first, at the retract height, to the first point's X
/// before it crosses in Y; and the two together. The first accepted, or
/// the usual way's problems and how to jog clear. The usual way, with
/// nothing to check, for a setup not in the chuck: its clearance plane,
/// checked over all its clamping, is clear from wherever the tool comes to
/// it.
fn way_out(
    p: &Checked,
    blocks: &[Block],
    offset: [f64; 6],
    start: [f64; 6],
) -> Result<Leave, Vec<String>> {
    let Some(chuck) = &p.chuck else {
        return Ok(Leave::default());
    };
    let end = blocks
        .iter()
        .position(|b| matches!(b, Block::Descend { .. }))
        .map_or(blocks.len(), |i| i + 1);
    let op = blocks[..end].iter().find_map(|b| match b {
        Block::Start { op, .. } => Some(*op),
        _ => None,
    });
    // Machine X, Y, Z and A in the setup's frame, A as commanded.
    let frame = |m: [f64; 6]| -> ([f32; 3], f32) {
        (
            std::array::from_fn(|k| (m[k] - offset[k] + p.datum[k] as f64) as f32),
            (m[3] - offset[3]) as f32,
        )
    };
    let check = |leave: Leave| -> Result<Vec<String>, Vec<String>> {
        let actions = lower_blocks(p, blocks, end, offset, start, leave)?;
        let mut at = start;
        let mut moves = vec![];
        for l in &actions {
            if let Do::Move { target, .. } = l.action {
                let ((from, a), (to, b)) = (frame(at), frame(target));
                moves.push((from, to, a, b));
                at = target;
            }
        }
        Ok(chuck.way_in(op.unwrap_or(0), &moves, p.datum))
    };
    let mut refused = check(Leave::default())?;
    if refused.is_empty() {
        return Ok(Leave::default());
    }
    let turns = chuck.jaws_down(frame(start).1, offset[3] as f32);
    let ways = std::iter::once(None)
        .chain(turns.iter().map(|a| Some(*a as f64 + offset[3])))
        .flat_map(|a| [(a, false), (a, true)])
        .skip(1)
        .map(|(a, x_first)| Leave { a, x_first });
    for leave in ways {
        if check(leave).is_ok_and(|e| e.is_empty()) {
            return Ok(leave);
        }
    }
    refused.push(jog(p, blocks, offset, start, turns.first().copied()));
    Err(refused)
}

/// How to jog clear when no way out of where the machine is, `start`, is
/// accepted: up to the height of the first retract, A to `a` (where no jaw
/// stands up) when there is one, then over the first point of `blocks`.
fn jog(p: &Checked, blocks: &[Block], offset: [f64; 6], start: [f64; 6], a: Option<f32>) -> String {
    let top = match p.settings.machine_home {
        true => 0.,
        false => clearance_plane(blocks) as f64 - p.datum[2] as f64 + offset[2],
    };
    let over = blocks.iter().find_map(|b| match b {
        Block::Approach { xy, .. } => {
            Some([0, 1].map(|k| xy[k] as f64 - p.datum[k] as f64 + offset[k]))
        }
        _ => None,
    });
    let turn = a.map_or(String::new(), |a| {
        format!("A to machine A {:.3}° (no jaw up), ", a as f64 + offset[3])
    });
    let across = over.map_or("over the stock".into(), |[x, y]| {
        format!("X to machine X {x:.3} and then Y to Y {y:.3}, over the first cut")
    });
    format!(
        "No other way out is clear either: neither turning A first so that no jaw stands up, nor moving along X first at the top. Jog the tool up to machine Z {:.3}, {turn}{across}, watching the chuck, then press Cycle Start.",
        top.max(start[2])
    )
}

/// Within this of each other, two A positions are the same (degrees): far
/// below A's step, so a move that keeps A computes it a hair off at most.
const A_SAME: f64 = 1e-6;

/// The line of the first move of `actions` that turns A while X, Y or Z
/// moves, from `start`; None when A always turns alone.
pub fn combined(actions: &[Located], start: [f64; 6]) -> Option<u32> {
    let mut at = start;
    for l in actions {
        if let Do::Move { target, .. } = l.action {
            let turns = (target[3] - at[3]).abs() > A_SAME;
            let moves = (0..3).any(|k| (target[k] - at[k]).abs() > 1e-9);
            if turns && moves {
                return Some(l.line);
            }
            at = target;
        }
    }
    None
}

/// A move to `to`, unless the tool is there already.
fn go(at: &mut [f64; 6], to: [f64; 6], feed: Option<f64>, actions: &mut Vec<Do>) {
    if to != *at {
        actions.push(Do::Move { target: to, feed });
        *at = to;
    }
}

/// How long the machine takes over a checked setup's tool runs: each
/// program lowered as Cycle Start lowers it and played through the
/// driver's own planner (`uc100::planner`), with the profile's axis speeds,
/// accelerations and corner deviation, its spindle spin-up and the
/// program's dwells, at 100 % feed. Manual tool changes between tool runs
/// and holds are the operator's, and not counted.
#[derive(Debug, Default)]
pub struct Times {
    /// By tool run, then by the operation it starts at (`Checked::from`):
    /// the seconds of each block of that program; None when it cannot
    /// start there.
    programs: Vec<Vec<Option<Vec<f32>>>>,
}

/// How many moves the driver keeps planned ahead (its
/// `MAX_PLANNED_BLOCKS`): the look-ahead the estimate plans with.
const LOOK_AHEAD: usize = 400;
/// The time step the estimate samples the planner at, s.
const SAMPLE: f64 = 0.01;

impl Times {
    /// The times of a check's answer, when it passed.
    pub fn checked(
        programs: &Result<Vec<Checked>, Vec<String>>,
        config: &uc100::config::MachineConfig,
    ) -> Option<Arc<Self>> {
        Some(Arc::new(Self::of(programs.as_ref().ok()?, config)))
    }

    pub fn of(programs: &[Checked], config: &uc100::config::MachineConfig) -> Self {
        Times {
            programs: (programs.iter())
                .map(|p| {
                    (0..p.ops.len())
                        .map(|k| block_seconds(p, k, config))
                        .collect()
                })
                .collect(),
        }
    }

    /// The seconds of each block of tool run `i`'s program from its
    /// operation `k` on.
    pub fn blocks(&self, i: usize, k: usize) -> Option<&[f32]> {
        self.programs.get(i)?.get(k)?.as_deref()
    }

    /// The seconds of each operation of tool run `p` (index `i`), cut from
    /// its first.
    pub fn ops(&self, i: usize, p: &Checked) -> Option<Vec<f32>> {
        Some(op_seconds(&p.blocks, self.blocks(i, 0)?, p.ops.len()))
    }

    /// The seconds of tool run `i`, cut from operation `k` on.
    pub fn run(&self, i: usize, k: usize) -> Option<f32> {
        Some(self.blocks(i, k)?.iter().sum())
    }
}

/// The seconds of each of `ops` operations, from the seconds of each of
/// `blocks`: a block that names no operation (stop, retract, dwell, end)
/// counts to the next that does, the last ones to the last operation.
pub fn op_seconds(blocks: &[Block], seconds: &[f32], ops: usize) -> Vec<f32> {
    let mut out = vec![0.; ops];
    for (o, s) in owners(blocks, ops).into_iter().zip(seconds) {
        if let Some(o) = o {
            out[o] += s;
        }
    }
    out
}

/// The operation each of `blocks` counts to (`op_seconds`).
fn owners(blocks: &[Block], ops: usize) -> Vec<Option<usize>> {
    let mut owner = vec![None; blocks.len()];
    let mut next = ops.checked_sub(1);
    for (j, b) in blocks.iter().enumerate().rev() {
        next = op_of(b).or(next);
        owner[j] = next.filter(|&o| o < ops);
    }
    owner
}

/// The seconds of `blocks` at feed override `feed` (1 is 100 %): their
/// feed moves take 1 / `feed` as long, their rapids, dwells and spin-ups
/// as long. Feeds the axes cap stay a little faster than this.
fn at_override(blocks: &[Block], seconds: &[f32], feed: f32) -> Vec<f32> {
    (blocks.iter().zip(seconds))
        .map(|(b, &s)| match b {
            Block::Move {
                kind: Kind::Feed, ..
            } => s / feed,
            _ => s,
        })
        .collect()
}

/// How near its move the tool tip is taken to be on it, mm: within the
/// corners the planner rounds and a coarse axis's step.
const ON_MOVE: f32 = 0.1;
/// How many blocks past the driver's line the tool may be.
const LINE_LAG: usize = 8;

/// The straight line each of `blocks` moves the tool tip along, in plan
/// coordinates, as `lower` lowers them from the tool tip at `start`: None
/// for a block that does not move it.
fn paths(blocks: &[Block], start: [f32; 3]) -> Vec<Option<[[f32; 3]; 2]>> {
    let plane = clearance_plane(blocks);
    let mut at = Some(start);
    (blocks.iter())
        .map(|b| {
            let to = match b {
                Block::Retract => at.map(|a| [a[0], a[1], plane]),
                Block::Approach { xy, .. } => Some([xy[0], xy[1], plane]),
                Block::Descend { xy, z, .. } => Some([xy[0], xy[1], *z]),
                Block::Move {
                    kind: Kind::Feed | Kind::Rapid,
                    to,
                    ..
                } => Some(*to),
                _ => None,
            };
            let path = to.and_then(|to| Some([at?, to]));
            at = to.or(at);
            path
        })
        .collect()
}

/// The share of `path` behind `tip`, when the tip is on it (`ON_MOVE`).
fn along([from, to]: [[f32; 3]; 2], tip: [f32; 3]) -> Option<f32> {
    let d: [f32; 3] = std::array::from_fn(|a| to[a] - from[a]);
    let length = d.iter().map(|x| x * x).sum::<f32>();
    let t = if length > 0. {
        ((0..3).map(|a| (tip[a] - from[a]) * d[a]).sum::<f32>() / length).clamp(0., 1.)
    } else {
        1.
    };
    let gap = (0..3)
        .map(|a| (from[a] + t * d[a] - tip[a]).powi(2))
        .sum::<f32>();
    (gap <= ON_MOVE * ON_MOVE).then_some(t)
}

/// How long the run has left, in seconds, at the feed override now
/// (`Run::left`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimeLeft {
    /// The operation being cut (index in its program): left, and all of it.
    pub op: Option<(usize, f32, f32)>,
    /// The tool run being cut, or the one Cycle Start starts from its start.
    pub run: f32,
    /// That tool run and every one after it.
    pub setup: f32,
}

impl Run {
    /// The estimated seconds of each operation of tool run `i` at 100 %
    /// feed: cut from its first, but for the tool run being cut or next when
    /// it starts at a later operation, from there; that operation then has
    /// the safe start too.
    pub fn op_times(&self, i: usize) -> Option<Vec<f32>> {
        let times = self.times.as_ref()?;
        let p = self.checked()?.get(i)?;
        let mut ops = times.ops(i, p)?;
        let (at, k) = self
            .running
            .as_ref()
            .map_or((self.next, self.from), |r| (r.index, r.from));
        if at == i
            && k > 0
            && let (Ok(blocks), Some(seconds)) = (p.from(k), times.blocks(i, k))
        {
            let from = op_seconds(blocks, seconds, p.ops.len());
            ops[k..].copy_from_slice(&from[k..]);
        }
        Some(ops)
    }

    /// The estimated seconds of all the setup's tool runs, each cut from its
    /// first operation, at 100 % feed.
    pub fn setup_time(&self) -> Option<f32> {
        let times = self.times.as_ref()?;
        (0..self.checked()?.len()).map(|i| times.run(i, 0)).sum()
    }

    /// How long the run has left at the feed override now, the machine as
    /// `s` reports it: while a tool run cuts, the blocks after the one it is
    /// on and what is left of that one (how far along its move the tool
    /// is); otherwise the tool run Cycle Start starts, from the operation it
    /// starts at. The tool runs after it count from their first operation.
    /// None until the check is timed.
    pub fn left(&self, s: &uc100::controller::Snapshot) -> Option<TimeLeft> {
        let times = self.times.as_ref()?;
        let programs = self.checked()?;
        let feed = (s.feed_override as f32).max(0.01);
        let (i, k) = self
            .running
            .as_ref()
            .map_or((self.next, self.from), |r| (r.index, r.from));
        let later: f32 = (i + 1..programs.len())
            .map(|j| {
                let seconds = times.blocks(j, 0)?;
                Some(
                    at_override(&programs[j].blocks, seconds, feed)
                        .iter()
                        .sum::<f32>(),
                )
            })
            .sum::<Option<f32>>()?;
        let Some(p) = programs.get(i) else {
            return Some(TimeLeft {
                op: None,
                run: 0.,
                setup: later,
            });
        };
        let blocks = p.from(k).ok()?;
        let seconds = at_override(blocks, times.blocks(i, k)?, feed);
        let Some((_, op, _)) = self
            .progress(s.current_line)
            .filter(|_| self.running.is_some())
        else {
            let run = seconds.iter().sum();
            return Some(TimeLeft {
                op: None,
                run,
                setup: run + later,
            });
        };
        // What is left of the block the tool tip is on (`track`): the share
        // of its move past the tip. Until the tip is found, all of it. The
        // operation is the one the panel shows cut (`progress`): until the
        // tip reaches it, all of it is left.
        let owner = owners(blocks, p.ops.len());
        let (block, rest) = (self.running.as_ref())
            .and_then(|r| r.on)
            .map_or((0, 1.), |(j, t)| (j, 1. - t));
        let ahead = |j: usize| {
            if j > block {
                seconds[j]
            } else if j == block {
                seconds[j] * rest
            } else {
                0.
            }
        };
        let of_op = |j: &usize| owner[*j] == Some(op);
        let op_left = (0..blocks.len()).filter(of_op).map(ahead).sum();
        let op_all = (0..blocks.len()).filter(of_op).map(|j| seconds[j]).sum();
        let run: f32 = (0..blocks.len()).map(ahead).sum();
        Some(TimeLeft {
            op: Some((op, op_left, op_all)),
            run,
            setup: run + later,
        })
    }
}

impl Run {
    /// Follows the tool tip along the running tool run's blocks, the
    /// machine as `s` reports it: from the block it was on, the first on
    /// whose move the tip is, up to a few past the driver's line
    /// (`LINE_LAG`). It stays on its block only while the tip goes on along
    /// it: moves that go back over the same line (a retract and the plunge
    /// after it, a pocket's passes) are told apart by their order. Where the
    /// tip is on none, it stays.
    pub fn track(&mut self, s: &uc100::controller::Snapshot) {
        let (Some(live), Some((_, _, line))) = (self.live(s), self.progress(s.current_line)) else {
            return;
        };
        let Some(r) = self.running.as_mut() else {
            return;
        };
        let (from, t0) = r.on.unwrap_or((0, 0.));
        let end = (line + LINE_LAG + 1).min(r.paths.len());
        r.on = (from..end)
            .find_map(|j| {
                let t = along(r.paths[j]?, live.tip)?;
                (j > from || t >= t0 - 1e-4).then_some((j, t))
            })
            .or(r.on);
    }
}

/// A time as a clock shows it: "m:ss", or "h:mm:ss" from an hour on.
pub fn clock(seconds: f32) -> String {
    let s = seconds.max(0.).round() as u64;
    match (s / 3600, s / 60 % 60, s % 60) {
        (0, m, s) => format!("{m}:{s:02}"),
        (h, m, s) => format!("{h}:{m:02}:{s:02}"),
    }
}

/// A step's time in the Operations list: `clock`, but "<1 s" under a
/// second, where the clock would say "0:00" (LIO-398).
pub fn step_clock(seconds: f32) -> String {
    if seconds < 1. {
        "<1 s".into()
    } else {
        clock(seconds)
    }
}

/// The seconds the driver takes over each block of tool run `p`'s program
/// from operation `k` on, or None when it cannot start there: its moves
/// planned `LOOK_AHEAD` ahead, as the driver feeds them, waiting for the
/// motion to stop at each dwell and spindle start. It starts at the
/// clearance plane over the program's first point.
fn block_seconds(p: &Checked, k: usize, config: &uc100::config::MachineConfig) -> Option<Vec<f32>> {
    use uc100::planner::{Limits, Planner};
    let blocks = p.from(k).ok()?;
    let plane = clearance_plane(blocks);
    let first = blocks.iter().find_map(|b| match b {
        Block::Approach { xy, .. } | Block::Descend { xy, .. } => Some(*xy),
        Block::Move { from, .. } => Some([from[0], from[1]]),
        _ => None,
    })?;
    let mut start = [0.; 6];
    for (a, v) in [first[0], first[1], plane].into_iter().enumerate() {
        start[a] = v as f64 - p.datum[a] as f64;
    }
    let actions = lower(p, k, [0.; 6], start).ok()?;
    let mut limits = Limits {
        max_velocity: [0.; 6],
        max_acceleration: [0.; 6],
        junction_deviation: config.motion.junction_deviation,
    };
    for a in 0..6 {
        if let Some(axis) = config.axes.get(a) {
            limits.max_velocity[a] = axis.max_velocity / 60.;
            limits.max_acceleration[a] = axis.acceleration;
        }
    }
    let mut planner = Planner::new(limits, start);
    let mut seconds = vec![0f64; blocks.len()];
    let at = |line: u32| (line.max(1) as usize - 1).min(blocks.len() - 1);
    let mut actions = actions.into_iter().peekable();
    let mut spindle = Spindle::Off;
    loop {
        while planner.queued() < LOOK_AHEAD
            && let Some(Located {
                line,
                action: Do::Move { target, feed },
            }) = actions.peek()
        {
            planner.push_line(*target, *feed, *line);
            actions.next();
        }
        if !planner.is_idle() {
            let line = planner.current_tag();
            planner.sample(SAMPLE);
            seconds[at(line)] += SAMPLE;
            continue;
        }
        let Some(Located { line, action }) = actions.next() else {
            break;
        };
        match action {
            Do::Dwell { seconds: s } => seconds[at(line)] += s,
            Do::Spindle { state, .. } => {
                if state != Spindle::Off && state != spindle {
                    seconds[at(line)] += config.outputs.spindle_spinup_s;
                }
                spindle = state;
            }
            Do::End => break,
            _ => {}
        }
    }
    Some(seconds.into_iter().map(|s| s as f32).collect())
}

/// Whether a job is cutting, held or paused: Cycle Start and the tool
/// change wait, Hold, Resume and Stop apply.
pub fn busy(state: Option<&State>) -> bool {
    matches!(
        state,
        Some(State::Run | State::Hold | State::Stopping | State::Paused(_))
    )
}

/// Why a setup that sits on the pins (`Checked::pins`) cannot start on
/// `c` yet: the connected machine's pins are not the setup's, their place
/// is not recorded, X and Y are not referenced from pin 1 (or homed) since
/// the last E-stop, or the work offset is not the one the pins give. None for any
/// other setup.
fn pin_problems(c: &Connection, setup: &crate::model::Setup, p: &Checked) -> Vec<String> {
    let Some(on) = &p.pins else {
        return vec![];
    };
    let pins = match &c.pins {
        Ok(Some(pins)) => pins,
        _ => {
            return vec![
                "This setup sits on the pins, but the connected machine has no fixture pins in the Machine library.".into(),
            ];
        }
    };
    let mut e = vec![];
    if !(crate::control::pins_settings(&setup.machine.p)
        .ok()
        .flatten())
    .is_some_and(|own| own.same_pins(pins))
    {
        e.push("This setup's snapshot of the machine has other fixture pins than the Machine library: use Assign to current setup there, then check the setup again.".into());
    }
    let Some(at) = pins.reference else {
        e.push(
            "The pins' place is not recorded: Record the pins' place in the Machine tab first."
                .into(),
        );
        return e;
    };
    // Home for X and Y with a home switch, the pin for those without
    // (LIO-574).
    if let Some(why) = c.pins_unheld() {
        e.push(why);
        return e;
    }
    let want = [0, 1].map(|a| at[a] - on.pin1[a] as f64);
    let s = &c.snapshot;
    if (0..2).any(|a| (s.work_offset[a] - want[a]).abs() > 1e-3) {
        e.push(format!(
            "The X/Y work offset of {} is not the one the pins give, machine X {:.3} Y {:.3}: press Set X Y from the pins in the Work zero tab.",
            p.offset, want[0], want[1]
        ));
    }
    e
}

/// The fixture block's Face and Drill (two tools, levels and holes) on a
/// UC100 machine with the profile at `profile`: a project for the tablet's
/// tool runs (LIO-475). It runs on the driver's template and on the JY5800
/// (LIO-540): with no coolant, as neither has a coolant output, and with
/// the drill's clearance 5 mm, so its 44 mm holes fit the JY5800's 55 mm
/// of Z (from 15 mm the drill would need 59). Its pecks retract to 2 mm,
/// below the clearance: a peck that rose to the clearance would read as the
/// start of a new hole, each hole's pecks as steps of their own.
#[cfg(test)]
pub(crate) fn tablet_fixture(profile: &std::path::Path) -> (Document, Mesh) {
    let (mut doc, mesh) = crate::post::fixture::job();
    let setup = &mut doc.setups[0];
    // Face and pocket with the flat end mill, then the drill: a tool run of
    // two operations (a start at the second, LIO-476) and a tool change.
    setup.operations = [0, 1, 3].map(|i| setup.operations[i].clone()).to_vec();
    for op in &mut setup.operations {
        op.p.insert("coolant".into(), "Off".into());
        if op.strategy == crate::model::Strategy::Drill {
            op.p.insert("clearance".into(), "5".into());
            op.p.insert("retract".into(), "2".into());
        }
    }
    // Each axis's travel is the profile's, where its soft limits give one
    // (the JY5800's Z is 55 mm), so Cycle Start finds they agree.
    let config = std::fs::read_to_string(profile)
        .map_err(|e| e.to_string())
        .and_then(|t| uc100::config::MachineConfig::from_toml(&t).map_err(|e| e.to_string()))
        .unwrap_or_else(|e| panic!("{}: {e}", profile.display()));
    let travel = |a: usize, default: f64| {
        let axis = config.axes.get(a);
        match axis.and_then(|x| Some(x.soft_max? - x.soft_min?)) {
            Some(t) => t.to_string(),
            None => default.to_string(),
        }
    };
    let profile = profile.display().to_string();
    let m = &mut doc.setups[0].machine;
    for (k, v) in [
        ("controller", "UC100".to_string()),
        ("uc100_profile", profile),
        ("x", travel(0, 390.)),
        ("y", travel(1, 280.)),
        ("z", travel(2, 80.)),
    ] {
        m.p.insert(k.into(), v);
    }
    doc.machines = vec![doc.setups[0].machine.clone()];
    (doc, mesh)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::post::{OpState, blocks, fixture};
    use crate::simulation::{Motion, Plan};
    use uc100::config::MachineConfig;

    fn settings() -> PostSettings {
        PostSettings {
            tool_table: false,
            machine_home: false,
            blend: 0.,
            dwell: 2.,
            air: None,
        }
    }

    #[test]
    fn a_setup_that_cannot_be_walked_says_its_holding_was_not_checked() {
        let (mut doc, mesh) = crate::guide::tests::job("Yes");
        // The CNC setup's tools have no numbers: it makes no programs, so
        // its holding is not checked, and that is said. (The manual mill's
        // tools need none, LIO-176: its holding is checked without them.)
        for t in &mut doc.tools {
            t.p.remove("number");
        }
        let sims = simulated(&doc, &mesh, 0);
        assert!(crate::post::held_walked(&doc, &mesh, 0, &sims).is_ok());
        let sims = simulated(&doc, &mesh, 1);
        let errors = crate::post::held_walked(&doc, &mesh, 1, &sims).unwrap_err();
        let line = crate::post::not_checked(&errors);
        let why = errors[0].trim_end_matches('.');
        assert_eq!(line, format!("Holding not checked: {why}."));
        assert!(
            line.contains("tool number") || line.contains("number"),
            "{line}"
        );
        let held: Vec<(Option<usize>, String)> = (crate::post::held(&doc, &mesh, 1, &sims))
            .into_iter()
            .map(|h| (h.operation, h.message))
            .collect();
        assert_eq!(held, [(None, line.clone())]);
        // The marks do not call it a refused vise: it is one of the problems.
        let ready = ready(&doc, &mesh, 1, 1);
        assert!(ready.held.is_empty(), "{:?}", ready.held);
        assert!(ready.problems.contains(&line), "{:?}", ready.problems);
        assert_eq!(
            crate::post::not_checked(&[]),
            "Holding not checked: its programs cannot be made."
        );
    }

    /// One tool run of `ops` and their `plans`, checked.
    fn checked(ops: Vec<OpState>, plans: &[&Plan]) -> Checked {
        let blocks = blocks(&ops, plans, &settings());
        Checked {
            blocks,
            indexes: (0..ops.len()).collect(),
            ops,
            stock: plans[0].stock,
            datum: [0., 0., 10.],
            settings: settings(),
            offset: "G54".into(),
            origin: "Stock top center".into(),
            part: Some((1, 1, 1)),
            located: None,
            probe: Ok([crate::post::At::Mid; 2]),
            probe_from: vec![],
            holds: vec![],
            pins: None,
            starts: vec![],
            partly: vec![],
            chuck: None,
        }
    }

    /// What the tablet makes of tool run `r` with the live work offset `o`,
    /// from where the machine is, `at` (`TabletSetup`).
    pub(crate) fn tablet_job(r: &TabletRun, o: [f64; 6], at: [f64; 6]) -> Vec<Located> {
        tablet_job_of(&r.actions, r.lift, o, at)
    }

    /// `tablet_job` of `actions` lifted to `lift`: a tool run's, or one of
    /// its `starts`.
    pub(crate) fn tablet_job_of(
        actions: &[TabletLine],
        lift: Lift,
        o: [f64; 6],
        mut at: [f64; 6],
    ) -> Vec<Located> {
        let lift = lift.z + if lift.machine { 0. } else { o[2] };
        let mut out = vec![];
        for l in actions {
            let action = match &l.action {
                TabletAction::Move {
                    target,
                    feed,
                    lift: up,
                } => {
                    let mut to: [f64; 6] =
                        std::array::from_fn(|k| target[k].map_or(at[k], |v| v + o[k]));
                    if *up {
                        to[2] = lift.max(at[2]);
                    }
                    if to == at {
                        continue;
                    }
                    at = to;
                    Do::Move {
                        target: to,
                        feed: *feed,
                    }
                }
                TabletAction::Dwell { seconds } => Do::Dwell { seconds: *seconds },
                TabletAction::Spindle { state, rpm } => Do::Spindle {
                    state: *state,
                    rpm: *rpm,
                },
                TabletAction::Coolant { flood, mist } => Do::Coolant {
                    flood: *flood,
                    mist: *mist,
                },
                TabletAction::Pause { reason } => Do::Pause {
                    reason: reason.clone(),
                },
                TabletAction::End => Do::End,
            };
            out.push(Located {
                line: l.line,
                action,
            });
        }
        out
    }

    /// One facing tool run: down to Z5, across at F600, a 1 s dwell.
    /// The tool runs a tablet file carries (LIO-475), read back from it:
    /// made into machine coordinates as `TabletSetup` tells the tablet to,
    /// with a work offset and from where the machine stands, each is the
    /// job Cycle Start sends (`lower`). With its tool, the tool change
    /// before it, the lift and its operations' steps.
    #[test]
    fn the_tablet_file_carries_the_tool_runs_cycle_start_sends() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("template.toml");
        std::fs::write(&profile, uc100::config::TEMPLATE).unwrap();
        let (doc, mesh) = tablet_fixture(&profile);
        let zip = crate::bundle::pack(&doc).unwrap();
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(zip)).unwrap();
        let runs: crate::bundle::Runs =
            serde_json::from_reader(zip.by_name(crate::bundle::RUNS).unwrap()).unwrap();
        assert_eq!(runs.format, 1);
        let [s] = &runs.setups[..] else {
            panic!("{:?}", runs.setups.len())
        };
        assert!(s.problems.is_empty(), "{:?}", s.problems);
        assert_eq!(s.machine_profile.as_deref(), Some("template.toml"));
        assert_eq!(s.work_offset, "G54");
        let tools: Vec<_> = (s.tool_runs.iter())
            .map(|r| (r.tool.number, r.tool_change, r.name.as_str()))
            .collect();
        assert_eq!(
            tools,
            [
                (1, false, "Top · tool run 1 of 2"),
                (3, true, "Top · tool run 2 of 2")
            ]
        );
        assert_eq!(s.tool_runs[1].tool.label, "T3 Drill 4");
        let programs = programs(&doc, &mesh, 0).unwrap();
        // Cycle Start takes each tool run on the driver's template and on the
        // JY5800 (LIO-540): no coolant, as neither has a coolant output; the
        // machine's travel is the profile's where its soft limits give one;
        // and each run spans no more of an axis than they allow, so some
        // work zero fits it (the drill's 44 mm holes and 5 mm clearance in
        // the JY5800's 55 mm of Z).
        for (name, toml) in [
            ("template.toml", uc100::config::TEMPLATE),
            ("jy5800.toml", include_str!("../assets/jy5800.toml")),
        ] {
            let path = dir.path().join(name);
            std::fs::write(&path, toml).unwrap();
            let (doc, mesh) = tablet_fixture(&path);
            let config = MachineConfig::from_toml(toml).unwrap();
            let machine = &doc.setups[0].machine;
            let runs = super::programs(&doc, &mesh, 0).unwrap();
            let mut soft = 0;
            for (a, axis_name) in AXES.iter().enumerate().take(3) {
                let axis = config.axes.get(a).unwrap();
                let (Some(lo), Some(hi)) = (axis.soft_min, axis.soft_max) else {
                    continue;
                };
                soft += 1;
                let travel =
                    crate::post_check::machine_mm(machine, &axis_name.to_lowercase()) as f64;
                assert!((travel - (hi - lo)).abs() < 0.01, "{name} {axis_name}");
                for p in &runs {
                    let span = extents(&p.blocks);
                    let reach = (span.max[a] - span.min[a]) as f64;
                    assert!(
                        reach <= hi - lo,
                        "{name} {} {}: {reach} > {}",
                        p.indexes[0],
                        axis_name,
                        hi - lo
                    );
                }
            }
            assert_eq!(soft, if name == "jy5800.toml" { 3 } else { 0 });
            for p in &runs {
                assert_eq!(
                    output_problems(&config.outputs, &p.ops),
                    Vec::<String>::new()
                );
            }
        }
        let offset = [120., 90., -35., 0., 0., 0.];
        for (r, p) in s.tool_runs.iter().zip(&programs) {
            // Not in the chuck: it starts from anywhere.
            assert!(!r.lift.machine && r.start.is_none());
            let plane = clearance_plane(&p.blocks) as f64 - p.datum[2] as f64;
            assert!((r.lift.z - plane).abs() < 1e-9, "{} {plane}", r.lift.z);
            // Below the lift and above it (a tool change leaves Z high).
            for start in [[-5., -7., -30., 0., 0., 0.], [3., 4., -1., 0., 0., 0.]] {
                assert_eq!(
                    tablet_job(r, offset, start),
                    lower(p, 0, offset, start).unwrap(),
                    "{}",
                    r.name
                );
            }
            let ops: Vec<usize> = r.operations.iter().map(|o| o.index).collect();
            assert_eq!(ops, p.indexes);
            // Each step is cut on its own lines, in order, and ends where
            // Stop after step stops: after its first line.
            assert!(!r.steps.is_empty(), "{}", r.name);
            let mut last = 0;
            for t in &r.steps {
                let [a, b] = t.lines.unwrap_or_else(|| panic!("{} {}", r.name, t.label));
                assert!(a > last && b >= a, "{} {:?}", t.label, t.lines);
                last = b;
                for [x, y] in &t.end_lines {
                    assert!(
                        *x > a && y >= x,
                        "{} {:?} {:?}",
                        t.label,
                        t.lines,
                        t.end_lines
                    );
                }
            }
        }
        let holes = &s.tool_runs[1].steps;
        assert!(holes.len() > 1 && holes.iter().all(|t| t.label.starts_with("Hole")));
        assert!(holes.iter().all(|t| !t.end_lines.is_empty()), "{holes:?}");
        // One step for each hole, every peck of it in its lines (LIO-540:
        // pecks that retract to the clearance would start steps of their
        // own). A block's line is its index + 1.
        let mut places: Vec<([i64; 2], usize)> = vec![];
        for (i, b) in programs[1].blocks.iter().enumerate() {
            let Block::Move {
                kind: Kind::Feed,
                to,
                ..
            } = b
            else {
                continue;
            };
            let place = [to[0], to[1]].map(|v| (v * 100.).round() as i64);
            let step = (holes.iter())
                .position(|t| {
                    t.lines
                        .is_some_and(|[a, b]| (a..=b).contains(&(i as u32 + 1)))
                })
                .unwrap_or_else(|| panic!("line {} is in no hole", i + 1));
            match places.iter().find(|(p, _)| *p == place) {
                Some((_, s)) => assert_eq!(*s, step, "{place:?}"),
                None => places.push((place, step)),
            }
        }
        let steps: Vec<usize> = places.iter().map(|(_, s)| *s).collect();
        assert_eq!(steps, (0..holes.len()).collect::<Vec<_>>(), "{places:?}");
        assert_eq!(holes.len(), 5);
        // Each tool run starts at each of its later operations as the Run
        // panel's Start here does (LIO-476): `lower` from there, the steps
        // from there on, their lines its own.
        for (r, p) in s.tool_runs.iter().zip(&programs) {
            assert_eq!(r.starts.len(), p.ops.len() - 1, "{}", r.name);
            for (k, st) in (1..).zip(&r.starts) {
                assert!(st.problems.is_empty(), "{} {:?}", r.name, st.problems);
                assert_eq!(st.operation, p.indexes[k]);
                assert!(!st.lift.machine);
                for start in [[-5., -7., -30., 0., 0., 0.], [3., 4., -1., 0., 0., 0.]] {
                    assert_eq!(
                        tablet_job_of(&st.actions, st.lift, offset, start),
                        lower(p, k, offset, start).unwrap(),
                        "{} from {k}",
                        r.name
                    );
                }
                let ops: Vec<usize> = st.steps.iter().map(|t| t.operation).collect();
                assert!(ops.iter().all(|o| p.indexes[k..].contains(o)), "{ops:?}");
                let mut last = 0;
                for t in &st.steps {
                    let [a, b] = t.lines.unwrap_or_else(|| panic!("{} {}", r.name, t.label));
                    assert!(a > last && b >= a, "{} {:?}", t.label, t.lines);
                    last = b;
                }
            }
        }
    }

    /// The tablet file's starts at steps (LIO-567): one at each step of each
    /// operation after its first, each the Run panel's start at that step.
    /// Made into machine coordinates with the wait over the step's entry put
    /// after the last action on or before its `entry`, as the tablet puts
    /// it, it is the job Cycle Start sends at that step (`job_actions`),
    /// from the setup checked with the steps before it taken as cut
    /// (`check_steps`). Its steps start at that one, numbered as the tool
    /// run's, and each ends where Stop after step stops; its lines before
    /// `entry` take the tool across in the air, and from `way_out` on it only
    /// stops the spindle, retracts and ends, as do the tool run's and each
    /// start at an operation.
    #[test]
    fn a_tablet_start_at_each_step_sends_what_a_start_at_a_step_sends() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("template.toml");
        std::fs::write(&profile, uc100::config::TEMPLATE).unwrap();
        let (doc, mesh) = tablet_fixture(&profile);
        let s = tablet_runs(&doc, Ok(&mesh), 0);
        assert!(s.problems.is_empty(), "{:?}", s.problems);
        let offset = [120., 90., -35., 0., 0., 0.];
        // Only stops the spindle (and coolant), lifts and ends from there.
        let out = |actions: &[TabletLine], from: Option<u32>| {
            let from = from.expect("a way out");
            let kinds: Vec<&str> = (actions.iter().filter(|l| l.line >= from))
                .map(|l| match &l.action {
                    TabletAction::Spindle {
                        state: Spindle::Off,
                        ..
                    } => "off",
                    TabletAction::Coolant {
                        flood: false,
                        mist: false,
                    } => "off",
                    TabletAction::Move { lift: true, .. } => "lift",
                    TabletAction::End => "end",
                    a => panic!("{a:?} in the way out"),
                })
                .filter(|k| *k != "off")
                .collect();
            assert_eq!(kinds, ["lift", "end"]);
        };
        let mut count = 0;
        for r in &s.tool_runs {
            out(&r.actions, r.way_out);
            for st in &r.starts {
                out(&st.actions, st.way_out);
                assert!(st.entry.is_some_and(|e| e > 1), "{:?}", st.entry);
            }
            for o in &r.operations {
                let steps = r.steps.iter().filter(|t| t.operation == o.index).count();
                let mine: Vec<usize> = (r.step_starts.iter())
                    .filter(|t| t.operation == o.index)
                    .map(|t| t.step)
                    .collect();
                assert_eq!(
                    mine,
                    (1..steps.max(1)).collect::<Vec<_>>(),
                    "{} {}",
                    r.name,
                    o.name
                );
            }
            for st in &r.step_starts {
                let (oi, n) = (st.operation, st.step);
                // The Run panel's start at that step; one its check refuses
                // is refused on the tablet, saying the same.
                let mut run = Run::new(&doc, 0, 0);
                run.start_step = Some((oi, n));
                let (programs, left) = check_steps(&doc, &mesh, 0, &[], &[(oi, (0..n).collect())]);
                if let Err(why) = &programs {
                    assert_eq!(&st.problems, why, "{} {oi} {n}", r.name);
                    assert!(st.actions.is_empty() && st.entry.is_none());
                    continue;
                }
                assert!(
                    st.problems.is_empty(),
                    "{} {oi} {n}: {:?}",
                    r.name,
                    st.problems
                );
                count += 1;
                run.checked_as(Arc::new(programs), Some(Arc::new(left)), None);
                assert_eq!(run.start_step, Some((oi, n)));
                let p = (run.checked().unwrap().iter())
                    .find(|p| p.indexes.contains(&oi))
                    .unwrap()
                    .clone();
                let k = p.indexes.iter().position(|&o| o == oi).unwrap();
                let label = run.step_label(oi, n);
                assert_eq!(
                    st.wait,
                    format!("Over the entry of {label}: Resume feeds in.")
                );
                let entry = st.entry.unwrap();
                // Its lines, and the rest its tool run's own from there,
                // carried once (`share`): all of it from its step's moves on.
                let rest = st
                    .rest
                    .unwrap_or_else(|| panic!("{} at {label}: nothing shared", r.name));
                let mut actions = st.actions.clone();
                actions.extend(rest_of(&r.actions, rest));
                assert!(st.actions.iter().all(|l| l.line < rest[1]));
                let at = (actions.iter())
                    .rposition(|l| l.line <= entry)
                    .map_or(0, |i| i + 1);
                let mut waits = actions.clone();
                waits.insert(
                    at,
                    TabletLine {
                        line: entry,
                        action: TabletAction::Pause {
                            reason: st.wait.clone(),
                        },
                    },
                );
                for start in [[-5., -7., -30., 0., 0., 0.], [3., 4., -1., 0., 0., 0.]] {
                    assert_eq!(
                        tablet_job_of(&waits, st.lift, offset, start),
                        run.job_actions(&p, k, offset, start, Some((oi, n)))
                            .unwrap(),
                        "{} at {label}",
                        r.name
                    );
                }
                // Across in the air before the entry: no feed.
                assert!(
                    actions
                        .iter()
                        .filter(|l| l.line < entry)
                        .all(|l| !matches!(l.action, TabletAction::Move { feed: Some(_), .. }))
                );
                out(&actions, st.way_out);
                let first = st.steps.first().unwrap();
                assert_eq!((first.operation, first.step), (oi, n));
                // In order, the first's lines from its way in on, as a start
                // at an operation's.
                let mut last = 0;
                for t in &st.steps {
                    assert_eq!(t.label, run.op_steps(t.operation).unwrap().label(t.step));
                    let [a, b] = t.lines.unwrap_or_else(|| panic!("{label} {}", t.label));
                    assert!(a > last && b >= a, "{label} {} {:?}", t.label, t.lines);
                    last = b;
                    assert!(!t.end_lines.is_empty(), "{label} {}", t.label);
                }
                let want: Vec<(usize, usize)> = (r.steps.iter())
                    .skip_while(|t| (t.operation, t.step) != (oi, n))
                    .map(|t| (t.operation, t.step))
                    .collect();
                let got: Vec<(usize, usize)> =
                    st.steps.iter().map(|t| (t.operation, t.step)).collect();
                assert_eq!(got, want);
            }
        }
        // Pocket's later levels and holes 2 to 5 start, and Face's pass 2:
        // the check takes pass 1 as cut without refusing the work zero it
        // was cut with (LIO-568). None is refused.
        assert!(count >= 9, "{count}");
        let refused: Vec<_> = (s.tool_runs.iter().flat_map(|r| &r.step_starts))
            .filter(|st| !st.problems.is_empty())
            .map(|st| (st.operation, st.step, &st.problems))
            .collect();
        assert!(refused.is_empty(), "{refused:?}");
        assert!((s.tool_runs.iter().flat_map(|r| &r.step_starts)).any(|st| (
            st.operation,
            st.step
        ) == (0, 1)
            && st.entry.is_some()));
    }

    /// Each start partway through the setup carries its work zero on the
    /// stock the operations before it left (LIO-548), as the Work zero tab
    /// finds it for a run started there: the Z the touch-off sets, or why
    /// it cannot, what they cut, where the tool tip touches off and the
    /// spots a probe can find the zero from. The setup's own start has none.
    #[test]
    fn the_tablet_file_carries_each_partway_starts_work_zero() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("template.toml");
        std::fs::write(&profile, uc100::config::TEMPLATE).unwrap();
        let (doc, mesh) = tablet_fixture(&profile);
        let s = tablet_runs(&doc, Ok(&mesh), 0);
        assert!(s.problems.is_empty(), "{:?}", s.problems);
        let (programs, left) = check(&doc, &mesh, 0);
        let mut run = Run::new(&doc, 0, 0);
        run.checked_as(Arc::new(programs), Some(Arc::new(left)), None);
        let p = run.checked().unwrap()[0].clone();
        let top = (p.stock.max[2] - p.datum[2]) as f64;
        assert!(s.tool_runs[0].zero.is_none());
        let mut zeros = vec![];
        for (i, r) in s.tool_runs.iter().enumerate() {
            let starts: Vec<_> = (r.starts.iter())
                .map(|st| (p_index(&run, i, st.operation), &st.zero))
                .collect();
            for (k, zero) in [(0, &r.zero)]
                .into_iter()
                .chain(starts)
                .skip(usize::from(i == 0))
            {
                run.start_at(i, k);
                assert!(run.partway(), "{i} {k}");
                let zero = zero.as_ref().unwrap_or_else(|| panic!("{i} {k}"));
                let partly = run.partly().unwrap();
                assert_eq!(zero.set_z, run.touch_off().map(|(_, w)| w[2]), "{i} {k}");
                assert_eq!(zero.blocked, run.partway_blocked(), "{i} {k}");
                assert_eq!(zero.cut, partly.cut, "{i} {k}");
                assert_eq!(zero.top_cut, partly.top_cut, "{i} {k}");
                assert_eq!(zero.top, top);
                assert_eq!(zero.probe_from.len(), partly.probe_from.len());
                for (t, ([x, y], ok)) in zero.probe_from.iter().zip(&partly.probe_from) {
                    assert_eq!((t.x, t.y, t.problem.clone()), (*x, *y, ok.clone().err()));
                }
                zeros.push((i, k, zero.clone()));
            }
        }
        // The face cut the top 4 mm down where the work zero (the stock
        // top's centre) is touched off: the pocket's start touches off on
        // that floor. The drill's touches off on the pocket's floor, cut
        // there by the flat end mill, which the check knows exactly. Every
        // stock-top spot a probe would start from was cut.
        let [(0, 1, pocket), (1, 0, drill)] = &zeros[..] else {
            panic!("{zeros:?}")
        };
        for z in [pocket, drill] {
            assert_eq!(z.blocked, None);
            assert_eq!(z.spot.as_deref(), Some("the centre"));
            assert_eq!(z.top, 0.);
            assert!(
                z.top_cut && z.cut.first().is_some_and(|c| c == "top face"),
                "{z:?}"
            );
            assert_eq!(z.probe_from.len(), 9);
            assert!(z.probe_from.iter().all(|s| s.problem.is_some()), "{z:?}");
        }
        assert_eq!(pocket.set_z, Some(-4.));
        assert!((drill.set_z.unwrap() + 15.8).abs() < 1e-5, "{drill:?}");
        // As runs.json says it.
        let json = serde_json::to_value(&s.tool_runs[1].zero).unwrap();
        assert_eq!(json["spot"], "the centre");
        assert_eq!(json["probeFrom"][0]["x"], "mid");
        assert!(json["setZ"].is_number() && json["topCut"] == true, "{json}");
    }

    /// Each start at a step carries its work zero on the stock the steps
    /// before it, and the operations before its operation, left (LIO-579),
    /// as the Work zero tab finds it for the Run panel's start at that step
    /// (`start_at_step`, the setup checked again with those steps taken as
    /// cut): the Z the touch-off sets, or why it cannot, what they cut,
    /// where the tool tip touches off and the spots a probe can find the
    /// zero from. A start the check refuses has none, and so does a file
    /// from before.
    #[test]
    fn the_tablet_file_carries_each_step_starts_work_zero() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("template.toml");
        std::fs::write(&profile, uc100::config::TEMPLATE).unwrap();
        let (doc, mesh) = tablet_fixture(&profile);
        let s = tablet_runs(&doc, Ok(&mesh), 0);
        assert!(s.problems.is_empty(), "{:?}", s.problems);
        let (programs, left) = check(&doc, &mesh, 0);
        let (whole, left) = (Arc::new(programs), Arc::new(left));
        let mut zeros = vec![];
        for r in &s.tool_runs {
            for st in &r.step_starts {
                let (oi, n) = (st.operation, st.step);
                if !st.problems.is_empty() {
                    assert_eq!(st.zero, None, "{oi} {n}");
                    continue;
                }
                // The Run panel: the setup checked, the step picked as
                // the start, and the setup checked again so.
                let mut run = Run::new(&doc, 0, 0);
                run.checked_as(whole.clone(), Some(left.clone()), None);
                assert!(run.start_at_step(oi, n), "{oi} {n}");
                let (programs, cut) = check_steps(&doc, &mesh, 0, &[], &[(oi, (0..n).collect())]);
                run.checked_as(Arc::new(programs), Some(Arc::new(cut)), None);
                assert_eq!(run.at_step(), Some((oi, n)));
                assert!(run.partway(), "{oi} {n}");
                let zero = st.zero.as_ref().unwrap_or_else(|| panic!("{oi} {n}"));
                let p = run.checked().unwrap()[0].clone();
                let partly = run.partly().unwrap();
                assert_eq!(zero.set_z, run.touch_off().map(|(_, w)| w[2]), "{oi} {n}");
                assert_eq!(zero.blocked, run.partway_blocked(), "{oi} {n}");
                assert_eq!(zero.cut, partly.cut, "{oi} {n}");
                assert_eq!(zero.top_cut, partly.top_cut, "{oi} {n}");
                assert_eq!(zero.top, (p.stock.max[2] - p.datum[2]) as f64);
                assert_eq!(zero.probe_from.len(), partly.probe_from.len());
                for (t, ([x, y], ok)) in zero.probe_from.iter().zip(&partly.probe_from) {
                    assert_eq!((t.x, t.y, t.problem.clone()), (*x, *y, ok.clone().err()));
                }
                zeros.push((oi, n, zero.clone()));
            }
        }
        // Pocket's levels and Drilling's holes after the first: each is
        // touched off at the stock top's centre, on the floor the face left
        // and the pocket cut. Knowing the stock inside an operation only
        // before and after it, the check may not tell that floor (LIO-582).
        assert!(zeros.len() >= 9, "{}", zeros.len());
        for (oi, n, z) in &zeros {
            assert_eq!(z.spot.as_deref(), Some("the centre"), "{oi} {n}");
            assert!(z.top_cut, "{oi} {n} {z:?}");
            // Drilling is the last operation: the check has no stock after
            // it to tell its holes by, and says so in a sentence.
            if *oi == 2 {
                assert_eq!(z.cut, [STEPS_UNFOLLOWED], "{n}");
                assert_eq!(
                    z.blocked.as_deref(),
                    Some(
                        "The check does not follow the stock cut by the steps taken as cut, so it cannot tell the surface they left where the stock top is touched off."
                    ),
                    "{n}"
                );
            } else {
                assert!(
                    z.cut.first().is_some_and(|c| c == "top face"),
                    "{oi} {n} {z:?}"
                );
            }
        }
        assert!(zeros.iter().any(|z| z.0 == 2), "{zeros:?}");
        // As runs.json says it.
        let first = s.tool_runs[0]
            .step_starts
            .iter()
            .find(|t| t.problems.is_empty());
        let json = serde_json::to_value(first.unwrap()).unwrap();
        assert!(
            json["zero"]["topCut"] == true && json["zero"].get("setZ").is_some(),
            "{json}"
        );
        // A file from before reads with none.
        let mut json = serde_json::to_value(&s.tool_runs[1].step_starts[0]).unwrap();
        json.as_object_mut().unwrap().remove("zero");
        let old: TabletStepStart = serde_json::from_value(json).unwrap();
        assert_eq!(old.zero, None);
    }

    /// Where the check cannot tell the surface the operations before a
    /// start left at the touch-off spot, the start's work zero has no Z to
    /// set, and says why as the Work zero tab does; not partway, none.
    #[test]
    fn a_tablet_start_partway_whose_surface_the_check_cannot_tell_has_no_z_to_set() {
        let (doc, _) = fixture::job();
        let plan = fixture::plan([0., 0., 15.], &[(Motion::Feed, [10., 0., 5.], 600.)]);
        let mut p = checked(vec![fixture::facing_op(); 2], &[&plan, &plan]);
        let cut = Partly {
            cut: vec!["top face".into()],
            surface: None,
            top_cut: true,
            probe_from: vec![],
        };
        p.partly = vec![cut.clone(), cut];
        let mut run = Run::new(&doc, 0, 0);
        run.checked_as(Arc::new(Ok(vec![p])), None, None);
        let orient = crate::orient::Orient::Right;
        assert_eq!(tablet_zero(&run, orient), None);
        run.start_at(0, 1);
        let zero = tablet_zero(&run, orient).unwrap();
        assert_eq!(zero.set_z, None);
        assert_eq!(
            zero.blocked.as_deref(),
            Some(
                "The operations before the start cut the stock's top face, and the check cannot tell the surface they left where the stock top is touched off."
            )
        );
        assert_eq!(zero.blocked, run.partway_blocked());
        assert_eq!(zero.spot.as_deref(), Some("the centre"));
    }

    /// The index in tool run `i`'s program of its operation `oi` (an index
    /// in the setup).
    fn p_index(run: &Run, i: usize, oi: usize) -> usize {
        let p = &run.checked().unwrap()[i];
        p.indexes.iter().position(|&o| o == oi).unwrap()
    }

    /// A tablet file from before the starts (LIO-476) still reads: its tool
    /// runs have none.
    #[test]
    fn a_tablet_run_without_starts_reads_with_none() {
        let json = r#"{"name":"Top · tool run 1 of 1","tool":{"number":1,"name":"Flat 10","label":"T1 Flat 10"},"toolChange":false,"operations":[{"index":0,"name":"Face"}],"lift":{"z":15.0,"machine":false},"start":null,"actions":[],"steps":[]}"#;
        let r: TabletRun = serde_json::from_str(json).unwrap();
        assert!(r.starts.is_empty());
        // Nor a work zero for a start partway (LIO-548).
        assert!(r.zero.is_none());
        let start = r#"{"operation":1,"problems":[],"lift":{"z":15.0,"machine":false},"actions":[],"steps":[]}"#;
        let st: TabletOpStart = serde_json::from_str(start).unwrap();
        assert!(st.zero.is_none());
    }

    /// A tablet start at a second and a third operation (LIO-476), their
    /// indexes in the setup not their places in the program, from machine
    /// home and not: each sends what the Run panel's Start here sends from
    /// there (a pause of the program kept), its lift is `step_lift`'s, and
    /// its steps are its own, Stop after step stopping only on a move that
    /// starts at the clearance, never on the way down into a step. A start
    /// the check refused stays refused.
    #[test]
    fn a_tablet_start_at_a_later_operation_sends_what_start_here_sends() {
        let (doc, _) = fixture::job();
        let mut plan = fixture::plan(
            [0., 0., 15.],
            &[
                (Motion::Rapid, [0., 0., 5.], 6000.),
                (Motion::Feed, [10., 0., 5.], 600.),
                (Motion::Rapid, [10., 0., 15.], 6000.),
                (Motion::Rapid, [0., 0., 15.], 6000.),
                (Motion::Rapid, [0., 0., 3.], 6000.),
                (Motion::Feed, [10., 0., 3.], 600.),
                (Motion::Rapid, [10., 0., 15.], 6000.),
            ],
        );
        plan.stops.push(crate::simulation::Stop {
            at: 0,
            pause: Some("Set the head before cutting".into()),
            size: None,
        });
        let ops = vec![fixture::facing_op(); 3];
        let mut p = checked(ops, &[&plan, &plan, &plan]);
        p.indexes = vec![0, 2, 4];
        for home in [false, true] {
            p.settings.machine_home = home;
            p.starts = (1..3)
                .map(|k| {
                    let mut b = blocks(&p.ops[k..], &vec![&plan; 3 - k], &p.settings);
                    for block in &mut b {
                        match block {
                            Block::Start { op, .. }
                            | Block::Run { op, .. }
                            | Block::Approach { op, .. }
                            | Block::Descend { op, .. }
                            | Block::Move { op, .. }
                            | Block::Pause { op, .. }
                            | Block::Index { op, .. } => *op += k,
                            _ => {}
                        }
                    }
                    Ok(b)
                })
                .collect();
            let mut run = Run::new(&doc, 0, 0);
            run.programs = Some(Arc::new(Ok(vec![p.clone()])));
            run.steps = (p.indexes.iter())
                .map(|&oi| OpSteps {
                    op: oi,
                    clear: 15.,
                    hole: false,
                    steps: plan_steps(&plan),
                    fingerprint: 0,
                })
                .collect();
            for k in 1..3 {
                let (actions, steps, lift) = tablet_job_from(&mut run, &p, 0, k).unwrap();
                assert_eq!(steps.len(), (3 - k) * 2);
                let ops: Vec<usize> = steps.iter().map(|s| s.operation).collect();
                let want: Vec<usize> = p.indexes[k..].iter().flat_map(|&oi| [oi, oi]).collect();
                assert_eq!(ops, want);
                assert!(actions.iter().any(|l| matches!(&l.action,
                    TabletAction::Pause { reason } if reason == "Set the head before cutting")));
                let offset = [100., 80., -30., 7., 2., 3.];
                for at in [
                    [3., 4., -50., 7., 2., 3.],
                    [100., 80., 0., 7., 2., 3.],
                    [100., 80., -25., 7., 2., 3.],
                ] {
                    let sent = run.job_actions(&p, k, offset, at, None).unwrap();
                    assert_eq!(
                        tablet_job_of(&actions, lift, offset, at),
                        sent,
                        "{home} {k}"
                    );
                }
                run.running.as_mut().unwrap().offset = offset;
                let z = lift.z + if home { 0. } else { offset[2] };
                assert_eq!(run.step_lift().unwrap(), z);
                let mut lines: Vec<u32> = (steps.iter().flat_map(|s| &s.end_lines))
                    .flat_map(|r| r[0]..=r[1])
                    .collect();
                lines.sort_unstable();
                let ranges: Vec<u32> = run.stop_ranges().iter().flat_map(|r| r.clone()).collect();
                assert_eq!(lines, ranges);
                assert!(steps.iter().all(|s| !s.end_lines.is_empty()));
                for s in &steps {
                    for line in s.end_lines.iter().flat_map(|r| r[0]..=r[1]) {
                        assert_eq!(run.step_end(line as usize - 1), Some((s.operation, s.step)));
                        match &p.from(k).unwrap()[line as usize - 1] {
                            Block::Move { kind, from, .. } => {
                                assert_ne!(*kind, Kind::Feed);
                                assert!(from[2] >= 15. - AT_CLEARANCE);
                            }
                            Block::Descend { .. } => panic!("a stop on the way down into a step"),
                            _ => {}
                        }
                    }
                }
            }
            p.starts[0] = Err(vec!["The way in is not clear.".into()]);
            assert_eq!(
                tablet_job_from(&mut run, &p, 0, 1).unwrap_err(),
                vec!["The way in is not clear."]
            );
            assert!(tablet_job_from(&mut run, &p, 0, 3).is_err());
        }
    }

    fn program() -> Checked {
        let plan = fixture::plan(
            [0., 0., 15.],
            &[
                (Motion::Rapid, [0., 0., 5.], 6000.),
                (Motion::Feed, [10., 0., 5.], 600.),
                (Motion::Dwell, [10., 0., 5.], 1.),
                (Motion::Rapid, [10., 0., 15.], 6000.),
            ],
        );
        checked(vec![fixture::facing_op()], &[&plan])
    }

    #[test]
    fn the_estimate_moves_each_block_as_the_drivers_planner_does() {
        // X and Z of the template: 2000 and 1000 mm/min, 300 and 200 mm/s²,
        // with a 0.5 s spindle spin-up.
        let mut config = uc100::config::MachineConfig::from_toml(uc100::config::TEMPLATE).unwrap();
        config.outputs.spindle_spinup_s = 0.5;
        let p = program();
        let times = Times::of(std::slice::from_ref(&p), &config);
        let seconds = times.blocks(0, 0).unwrap();
        assert_eq!(seconds.len(), p.blocks.len());
        let of = |want: &dyn Fn(&Block) -> bool| -> f32 {
            (p.blocks.iter().zip(seconds))
                .filter(|(b, _)| want(b))
                .map(|(_, s)| s)
                .sum()
        };
        // 10 mm at F600 (10 mm/s), from and to a stop: 1 s and v/a at 300 mm/s².
        let feed = of(&|b| {
            matches!(
                b,
                Block::Move {
                    kind: Kind::Feed,
                    ..
                }
            )
        });
        assert!((feed - (1. + 10. / 300.)).abs() < 0.02, "{feed}");
        // The start dwell (2 s) after the spin-up, and the program's 1 s dwell.
        let wait = of(&|b| matches!(b, Block::Wait { .. }));
        let run = of(&|b| matches!(b, Block::Run { .. }));
        assert!(
            (wait - 2.).abs() < 1e-6 && (run - 0.5).abs() < 1e-6,
            "{wait} {run}"
        );
        let dwell = of(&|b| {
            matches!(
                b,
                Block::Move {
                    kind: Kind::Dwell,
                    ..
                }
            )
        });
        assert!((dwell - 1.).abs() < 1e-6, "{dwell}");
        // Z down 10 and up 10 mm at 1000 mm/min, 200 mm/s²: 0.6 + 1/12 s each.
        let rapids = of(&|b| {
            matches!(
                b,
                Block::Move {
                    kind: Kind::Rapid,
                    ..
                }
            )
        });
        let one = 10. / (1000. / 60.) + (1000. / 60.) / 200.;
        assert!((rapids - 2. * one).abs() < 0.03, "{rapids}");
        // One operation: all of it, the stops and retracts too.
        let ops = times.ops(0, &p).unwrap();
        let total: f32 = seconds.iter().sum();
        assert_eq!(ops.len(), 1);
        assert!((ops[0] - total).abs() < 1e-4 && times.run(0, 0) == Some(total));
    }

    #[test]
    fn a_time_reads_as_a_clock() {
        assert_eq!(
            [0., 9.4, 59.6, 61., 3599., 3600., 3725.].map(clock),
            [
                "0:00", "0:09", "1:00", "1:01", "59:59", "1:00:00", "1:02:05"
            ]
        );
        // A step under a second says so (LIO-398).
        assert_eq!(
            [0.4, 0.99, 1., 9.4].map(step_clock),
            ["<1 s", "<1 s", "0:01", "0:09"]
        );
    }

    #[test]
    fn a_block_of_no_operation_counts_to_the_next_operation_and_the_end_to_the_last() {
        let mv = |op| Block::Move {
            op,
            kind: Kind::Feed,
            from: [0.; 3],
            to: [1., 0., 0.],
            feed: 60.,
            seconds: 0.,
            a: None,
        };
        let blocks = [
            Block::Stop,
            Block::Retract,
            mv(0),
            Block::Stop,
            Block::Standstill { seconds: 2. },
            mv(1),
            Block::Stop,
            Block::Retract,
            Block::End,
        ];
        let seconds = [1., 2., 4., 8., 16., 32., 64., 128., 256.];
        assert_eq!(
            op_seconds(&blocks, &seconds, 2),
            [1. + 2. + 4., 8. + 16. + 32. + 64. + 128. + 256.]
        );
    }

    #[test]
    fn the_fixture_jobs_tool_runs_take_at_least_their_feed_time() {
        let (mut doc, mesh) = fixture::job();
        let m = &mut doc.setups[0].machine;
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("template.toml");
        std::fs::write(&profile, uc100::config::TEMPLATE).unwrap();
        m.p.insert("controller".into(), "UC100".into());
        m.p.insert("uc100_profile".into(), profile.display().to_string());
        let programs = programs(&doc, &mesh, 0).unwrap();
        let config = uc100::config::MachineConfig::from_toml(uc100::config::TEMPLATE).unwrap();
        let started = std::time::Instant::now();
        let times = Times::of(&programs, &config);
        eprintln!("estimated in {:?}", started.elapsed());
        for (i, p) in programs.iter().enumerate() {
            let feed: f32 = (p.blocks.iter())
                .map(|b| match b {
                    Block::Move {
                        kind: Kind::Feed,
                        from,
                        to,
                        feed,
                        ..
                    } => {
                        let d = (0..3).map(|a| (to[a] - from[a]).powi(2)).sum::<f32>();
                        d.sqrt() / feed.min(2000.) * 60.
                    }
                    _ => 0.,
                })
                .sum();
            let run = times.run(i, 0).unwrap();
            eprintln!(
                "tool run {i}: {} blocks, feed {feed:.1} s, estimate {run:.1} s",
                p.blocks.len()
            );
            assert!(run >= feed, "{run} {feed}");
            for k in 1..p.ops.len() {
                assert!(times.run(i, k).unwrap() < run);
            }
        }
    }

    #[test]
    fn a_spindle_reversal_lowers_to_off_a_dwell_and_the_reversed_start() {
        // Two facing passes of one tool, the second turning the spindle the
        // other way: between them the coolant and the spindle go off, the
        // spindle gets the start dwell to stand still, then it starts the
        // other way with the coolant and the start dwell follows (#160).
        let pass = |from: [f32; 2], to: [f32; 2]| {
            fixture::plan(
                [from[0], from[1], 15.],
                &[
                    (Motion::Rapid, [from[0], from[1], 5.], 6000.),
                    (Motion::Feed, [to[0], to[1], 5.], 600.),
                    (Motion::Rapid, [to[0], to[1], 15.], 6000.),
                ],
            )
        };
        let (first, second) = (pass([0., 0.], [10., 0.]), pass([10., 10.], [0., 10.]));
        let ops = vec![
            fixture::facing_op(),
            OpState {
                name: "Face back".into(),
                clockwise: false,
                ..fixture::facing_op()
            },
        ];
        let p = checked(ops, &[&first, &second]);
        let located = lower(&p, 0, [0.; 6], [0.; 6]).unwrap();
        let actions: Vec<Do> = located.iter().map(|l| l.action.clone()).collect();
        let off = Do::Spindle {
            state: Spindle::Off,
            rpm: 0.,
        };
        let at = actions.iter().position(|a| *a == off).unwrap();
        assert_eq!(
            actions[at - 1..at + 5],
            [
                Do::Coolant {
                    flood: false,
                    mist: false,
                },
                off.clone(),
                Do::Dwell { seconds: 2. },
                Do::Spindle {
                    state: Spindle::Ccw,
                    rpm: 8000.,
                },
                Do::Coolant {
                    flood: true,
                    mist: false,
                },
                Do::Dwell { seconds: 2. },
            ],
            "{actions:?}"
        );
        // The run still ends with the spindle off, and the standstill dwell
        // belongs to its block.
        assert_eq!(actions.iter().filter(|a| **a == off).count(), 2);
        assert!(matches!(
            p.blocks[located[at + 1].line as usize - 1],
            Block::Standstill { seconds } if seconds == 2.
        ));
    }

    #[test]
    fn a_tool_run_lowers_to_machine_coordinates_at_the_work_offset() {
        let p = program();
        // Work zero (the stock top centre, plan Z10) at machine X100 Y50 Z-30.
        let offset = [100., 50., -30., 0., 0., 0.];
        // Under the clearance plane: the retract lifts Z alone to it.
        let start = [7., 8., -27., 0., 0., 45.];
        let actions = lower(&p, 0, offset, start).unwrap();
        // Plan times are f32: a dwell is its seconds to a few µs.
        let found: Vec<Do> = actions
            .iter()
            .map(|l| match l.action {
                Do::Dwell { seconds } => Do::Dwell {
                    seconds: (seconds * 1e3).round() / 1e3,
                },
                ref a => a.clone(),
            })
            .collect();
        let mv = |x: f64, y: f64, z: f64, feed| Do::Move {
            target: [x, y, z, 0., 0., 45.],
            feed,
        };
        // The clearance plane is plan Z15: work Z5, machine Z-25.
        let expected = vec![
            mv(7., 8., -25., None),
            Do::Spindle {
                state: Spindle::Cw,
                rpm: 8000.,
            },
            Do::Coolant {
                flood: true,
                mist: false,
            },
            Do::Dwell { seconds: 2. },
            mv(100., 50., -25., None),
            mv(100., 50., -35., None),
            mv(110., 50., -35., Some(10.)),
            Do::Dwell { seconds: 1. },
            mv(110., 50., -25., None),
            Do::Coolant {
                flood: false,
                mist: false,
            },
            Do::Spindle {
                state: Spindle::Off,
                rpm: 0.,
            },
            Do::End,
        ];
        assert_eq!(found, expected);
        // Every action carries its block's number; blocks run in order.
        for l in &actions {
            assert!(l.line >= 1 && l.line as usize <= p.blocks.len());
        }
        assert!(actions.windows(2).all(|w| w[0].line <= w[1].line));
        let feed = actions.iter().find(|l| l.action == expected[6]).unwrap();
        assert!(matches!(
            p.blocks[feed.line as usize - 1],
            Block::Move {
                kind: Kind::Feed,
                ..
            }
        ));
    }

    /// A retract never moves Z down (LIO-212): from above the clearance
    /// plane (machine Z-25 here) the tool stays at its height, crosses
    /// there and comes down to the first point; from below it, it goes up
    /// to the plane as before. The end's retract, after the cut, goes up.
    /// Retracting to machine home, it goes to machine Z 0, the top of Z,
    /// and stays there from a start at it.
    #[test]
    fn a_retract_below_the_tool_does_not_move_it_down() {
        let p = program();
        let offset = [100., 50., -30., 0., 0., 0.];
        let moves = |p: &Checked, start: [f64; 6]| -> Vec<[f64; 6]> {
            (lower(p, 0, offset, start).unwrap().iter())
                .filter_map(|l| match l.action {
                    Do::Move { target, .. } => Some(target),
                    _ => None,
                })
                .collect()
        };
        let high = moves(&p, [7., 8., -2., 0., 0., 0.]);
        assert_eq!(
            high[..3],
            [
                [100., 50., -2., 0., 0., 0.],
                [100., 50., -25., 0., 0., 0.],
                [100., 50., -35., 0., 0., 0.],
            ]
        );
        assert_eq!(high.last().unwrap(), &[110., 50., -25., 0., 0., 0.]);
        let low = moves(&p, [7., 8., -40., 0., 0., 0.]);
        assert_eq!(
            low[..2],
            [[7., 8., -25., 0., 0., 0.], [100., 50., -25., 0., 0., 0.]]
        );
        let mut home = program();
        home.settings.machine_home = true;
        let up = moves(&home, [7., 8., -2., 0., 0., 0.]);
        assert_eq!(
            up[..3],
            [
                [7., 8., 0., 0., 0., 0.],
                [100., 50., 0., 0., 0., 0.],
                [100., 50., -25., 0., 0., 0.],
            ]
        );
        let top = moves(&home, [7., 8., 0., 0., 0., 0.]);
        assert_eq!(top[..2], up[1..3]);
        assert_eq!(top.last().unwrap(), &[110., 50., 0., 0., 0., 0.]);
    }

    /// A facing tool run made at A90 then A180 as an indexed setup's
    /// operation is: from the swing height (plan Z40), turning A there.
    fn indexed_program() -> Checked {
        let moves = [
            (Motion::Index, [-15., -15., 40.], 3600., 90.),
            (Motion::Rapid, [-15., -15., 11.], 6000., 90.),
            (Motion::Feed, [15., -15., 9.], 1000., 90.),
            (Motion::Rapid, [15., -15., 40.], 6000., 90.),
            (Motion::Index, [15., -15., 40.], 3600., 180.),
            (Motion::Rapid, [15., -15., 11.], 6000., 180.),
            (Motion::Feed, [-15., -15., 9.], 1000., 180.),
            (Motion::Rapid, [-15., -15., 40.], 6000., 180.),
        ];
        let steps: Vec<_> = moves.iter().map(|m| (m.0, m.1, m.2)).collect();
        let mut plan = fixture::plan([-15., -15., 40.], &steps);
        for (m, step) in plan.moves.iter_mut().zip(&moves) {
            m.a = Some(step.3);
        }
        checked(vec![fixture::facing_op()], &[&plan])
    }

    #[test]
    fn an_indexed_tool_run_turns_a_alone_at_its_work_offset() {
        let p = indexed_program();
        assert_eq!(a_angles(&p.blocks), [90., 180., 0.]);
        assert_eq!(op_angles(&p.blocks, 0), [90., 180.]);
        // A's work offset is 5°: machine A = A + 5. A starts at machine 30°.
        let offset = [100., 50., -30., 5., 0., 0.];
        let start = [7., 8., -2., 30., 0., 0.];
        let actions = lower(&p, 0, offset, start).unwrap();
        assert_eq!(combined(&actions, start), None);
        let moves: Vec<[f64; 6]> = actions
            .iter()
            .filter_map(|l| match l.action {
                Do::Move { target, .. } => Some(target),
                _ => None,
            })
            .collect();
        // The first retract moves Z alone, to the clearance plane (plan Z40,
        // machine Z0), and leaves A where it is.
        assert_eq!(moves[0], [7., 8., 0., 30., 0., 0.]);
        // Each turn: X, Y and Z where the move before left them, A alone.
        let mut turns = vec![];
        for w in moves.windows(2) {
            if w[1][3] != w[0][3] {
                assert_eq!(w[1][..3], w[0][..3], "{w:?}");
                assert!(
                    actions.iter().any(|l| l.action
                        == Do::Move {
                            target: w[1],
                            feed: None
                        }),
                    "a turn is a rapid"
                );
                turns.push(w[1][3]);
            }
        }
        assert_eq!(turns, [95., 185., 5.]);
        // Then across over the first cut at that height, A still where it
        // was: A turns only with the tool over the operation (LIO-209).
        assert_eq!((moves[1][2], moves[1][3]), (0., 30.));
        assert_eq!(moves[2][..3], moves[1][..3]);
        // Every move between two turns is made at the A of the first.
        assert!(moves[2..].iter().all(|m| [95., 185., 5.].contains(&m[3])));
        // The turn to A90 is the index block after the tool's retract and
        // its approach, and the second is made at the swing height, plan
        // Z40: machine Z0.
        let second = actions
            .iter()
            .find(|l| matches!(l.action, Do::Move { target, .. } if target[3] == 185.))
            .unwrap();
        assert!(matches!(
            p.blocks[second.line as usize - 1],
            Block::Index { a, .. } if a == 180.
        ));
        assert!(matches!(second.action, Do::Move { target, .. } if (target[2] - 0.).abs() < 1e-4));
        // A program whose move turns A as it goes is refused, not run.
        let mut bad = p.clone();
        let i = (bad.blocks.iter())
            .position(|b| matches!(b, Block::Move { a: Some(a), .. } if *a == 180.))
            .unwrap();
        if let Block::Move { a, .. } = &mut bad.blocks[i] {
            *a = Some(200.);
        }
        let why = lower(&bad, 0, offset, start).unwrap_err();
        assert!(why[0].contains("would turn A"), "{why:?}");
        // An action list that does it is caught too.
        let both = [Located {
            line: 3,
            action: Do::Move {
                target: [1., 0., 0., 10., 0., 0.],
                feed: None,
            },
        }];
        assert_eq!(combined(&both, [0.; 6]), Some(3));
        // A 3-axis tool run never commands A: it stays where it is.
        let flat = lower(&program(), 0, offset, start).unwrap();
        assert!(flat.iter().all(|l| match l.action {
            Do::Move { target, .. } => target[3] == 30.,
            _ => true,
        }));
        assert!(a_angles(&program().blocks).is_empty());
    }

    #[test]
    fn a_way_out_turns_a_then_moves_x_alone_before_the_approach() {
        // LIO-213: A first, then X, each alone at the retract height, then Y
        // over the first point and down; never two axes together, and only
        // before the first approach.
        let p = program();
        let start = [-20., -30., -40., 15., 0., 0.];
        let leave = Leave {
            a: Some(60.),
            x_first: true,
        };
        let blocks = &p.blocks;
        let actions = lower_blocks(&p, blocks, blocks.len(), [0.; 6], start, leave).unwrap();
        let moves: Vec<[f64; 6]> = (actions.iter())
            .filter_map(|l| match l.action {
                Do::Move { target, .. } => Some(target),
                _ => None,
            })
            .collect();
        let top = moves[0][2];
        assert_eq!(moves[0], [-20., -30., top, 15., 0., 0.]);
        assert_eq!(moves[1], [-20., -30., top, 60., 0., 0.]);
        assert_eq!(moves[2], [0., -30., top, 60., 0., 0.]);
        assert_eq!(moves[3], [0., 0., top, 60., 0., 0.]);
        assert!(moves[3..].iter().all(|m| m[3] == 60.));
        assert_eq!(combined(&actions, start), None);
        // Without a way out the approach moves X and Y together.
        let usual = lower_blocks(&p, blocks, blocks.len(), [0.; 6], start, Leave::default());
        let usual = usual.unwrap();
        let second = (usual.iter())
            .filter_map(|l| match l.action {
                Do::Move { target, .. } => Some(target),
                _ => None,
            })
            .nth(1);
        assert_eq!(second, Some([0., 0., top, 15., 0., 0.]));
        // A program without a chuck has nothing to leave: the usual way.
        assert_eq!(way_out(&p, blocks, [0.; 6], start), Ok(Leave::default()));
    }

    #[test]
    fn the_running_block_names_its_operation() {
        let p = program();
        let mut run = Run {
            setup: 0,
            setup_id: String::new(),
            name: "Top".into(),
            programs: Some(Arc::new(Ok(vec![p.clone()]))),
            epoch: 0,
            stale: false,
            next: 0,
            from: 0,
            zeroed: false,
            done: vec![false],
            running: Some(Running {
                index: 0,
                from: 0,
                seq: 0,
                offset: [0.; 6],
                before: Before {
                    xy: None,
                    zero: None,
                    last: None,
                    touched: false,
                },
                paths: vec![],
                tip: [0.; 3],
                start: [0.; 6],
                print: 0,
                cut_ops: vec![],
                on: None,
                marks: vec![],
            }),
            xy: None,
            zero: None,
            reference: None,
            reference_a: None,
            last: None,
            length: None,
            touched: false,
            outcome: None,
            pictures: vec![],
            left: None,
            before: Default::default(),
            times: None,
            carving: None,
            skips: vec![],
            layout: None,
            beneath: vec![],
            keep: None,
            menu: None,
            steps: vec![],
            cut_steps: vec![],
            start_step: None,
            open: None,
            picked: None,
            tool_kept: false,
            recheck: false,
            stop_after: false,
            stopping: None,
            unattended: false,
            stopped: None,
            completed: false,
            project: String::new(),
            journal: None,
            resumed: None,
            journal_note: None,
            follow: None,
            followed: None,
            cut_note: None,
            rows_from_skips: false,
        };
        let (_, op, block) = run.progress(Some(3)).unwrap();
        assert_eq!((op, block), (0, 2));
        // The outcome after the start ends the tool run and moves on.
        let o = |seq, ok| uc100::controller::ProgramOutcome {
            seq,
            ok,
            message: "m".into(),
        };
        run.refresh(Some(&o(0, true)));
        assert!(run.running.is_some());
        run.refresh(Some(&o(1, true)));
        assert!(run.running.is_none() && run.done[0] && run.next == 1);
    }

    /// A skip's check keeps, by operation, what was done and where Cycle
    /// Start starts; one taken as cut before the start asks for the work
    /// zero on the stock it left (`partway`); an operation of a tool run done
    /// in this run is not skipped, nor is the material of one a later one
    /// cuts beneath left (`beneath`), nor anything while a tool run runs;
    /// and a skip whose check refused the setup is undone with the progress
    /// from before it.
    #[test]
    fn a_skip_keeps_what_was_done_and_where_cycle_start_starts() {
        let (doc, _) = fixture::job();
        let mut run = Run::new(&doc, 0, 0);
        let p = program();
        let answer = |run: &mut Run, runs: &[&[usize]]| {
            let programs = (runs.iter())
                .map(|r| Checked {
                    indexes: r.to_vec(),
                    ops: vec![p.ops[0].clone(); r.len()],
                    ..p.clone()
                })
                .collect();
            run.checked_as(Arc::new(Ok(programs)), None, None);
        };
        let whole: &[&[usize]] = &[&[0, 1, 2], &[3], &[4]];
        answer(&mut run, whole);
        // The first operation taken as cut, before the start.
        assert!(!run.partway());
        assert!(run.skip(0, Some(Skip::Cut)));
        assert_eq!(run.skips, [(0, Skip::Cut)]);
        answer(&mut run, &[&[1, 2], &[3], &[4]]);
        assert_eq!((run.next, run.from), (0, 0));
        assert!(run.partway());
        // Its material left instead: the stock is the setup's.
        assert!(run.skip(0, Some(Skip::Left)));
        answer(&mut run, &[&[1, 2], &[3], &[4]]);
        assert!(!run.partway());
        assert!(run.skip(0, None) && run.skips.is_empty());
        answer(&mut run, whole);
        assert_eq!((run.next, run.from, run.partway()), (0, 0, false));
        // The rows are the tool runs without a skip.
        assert_eq!(run.layout.as_deref().unwrap().as_ref().unwrap().len(), 3);
        // Tool run 1 done: its operations stay, the next is skipped.
        (run.done[0], run.next) = (true, 1);
        assert!(!run.skip(1, Some(Skip::Cut)));
        run.beneath = vec![(4, "Drilling cuts beneath it".into())];
        assert!(!run.skip(4, Some(Skip::Left)));
        assert!(run.skip(3, Some(Skip::Cut)));
        answer(&mut run, &[&[0, 1, 2], &[4]]);
        assert_eq!(
            (run.done.clone(), run.next, run.from),
            (vec![true, false], 1, 0)
        );
        assert_eq!(run.layout.as_deref().unwrap().as_ref().unwrap().len(), 3);
        // Undone, it is next again.
        assert!(run.skip(3, None));
        answer(&mut run, whole);
        assert_eq!((run.done.clone(), run.next), (vec![true, false, false], 1));
        // Both later tool runs skipped: there is nothing next.
        assert!(run.skip(3, Some(Skip::Cut)));
        answer(&mut run, &[&[0, 1, 2], &[4]]);
        assert!(run.skip(4, Some(Skip::Cut)));
        answer(&mut run, &[&[0, 1, 2]]);
        assert_eq!((run.done.clone(), run.next), (vec![true], 1));
        for oi in [4, 3] {
            assert!(run.skip(oi, None));
        }
        answer(&mut run, whole);
        assert_eq!((run.done.clone(), run.next), (vec![true, false, false], 1));
        // A refused check: the skip is undone, what was done kept.
        assert!(run.skip(3, Some(Skip::Left)));
        run.checked_as(Arc::new(Err(vec!["refused".into()])), None, None);
        assert!(!run.skip(4, Some(Skip::Cut)));
        assert!(run.skip(3, None));
        answer(&mut run, whole);
        assert_eq!((run.done.clone(), run.next), (vec![true, false, false], 1));
        // Not while a tool run runs.
        run.running = Some(Running {
            index: 1,
            from: 0,
            seq: 0,
            offset: [0.; 6],
            before: Before {
                xy: None,
                zero: None,
                last: None,
                touched: false,
            },
            paths: vec![],
            tip: [0.; 3],
            start: [0.; 6],
            print: 0,
            cut_ops: vec![],
            on: None,
            marks: vec![],
        });
        assert!(!run.skip(4, Some(Skip::Cut)) && run.skips.is_empty());
    }

    /// A deep drilling's steps are its holes, each with all its pecks
    /// (LIO-549): the tablet fixture's Drilling with its pecks retracting to
    /// its clearance, both 5 mm above the stock, as axis-shop's copy of it
    /// had (its holes go 44 mm deep and that profile has 55 mm of Z): 5 holes
    /// in 3 mm pecks that each rise to the clearance. It has 5 steps,
    /// "Hole 1" to "Hole 5", not a "Hole" a peck. The running program's marks
    /// count them as the plan does and call a hole done only once the tool
    /// leaves it, so Stop after
    /// step stops between holes, never between two pecks; the tablet file
    /// says the same.
    #[test]
    fn a_deep_drillings_pecks_are_in_the_step_of_their_hole() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("template.toml");
        std::fs::write(&profile, uc100::config::TEMPLATE).unwrap();
        let (mut doc, mesh) = tablet_fixture(&profile);
        let oi = (doc.setups[0].operations.iter())
            .position(|o| o.strategy == crate::model::Strategy::Drill)
            .unwrap();
        let op = &mut doc.setups[0].operations[oi];
        // The fixture's own pecks retract to 2 mm, below its 5 mm clearance
        // (LIO-540); a project may retract them to the clearance itself.
        op.p.insert("clearance".into(), "5".into());
        op.p.insert("retract".into(), "5".into());
        let op = &doc.setups[0].operations[oi];
        assert_eq!(crate::get(&op.p, "cycle"), "Deep drilling");
        assert_eq!(crate::num(&op.p, "peck"), Some(3.));
        let sims = simulated(&doc, &mesh, 0);
        let steps = op_steps(&doc, 0, &sims);
        let drill = steps.iter().find(|s| s.op == oi).unwrap();
        let plan = &sims
            .iter()
            .find(|s| s.0 == oi)
            .unwrap()
            .1
            .as_ref()
            .unwrap()
            .plan;
        let feeds: Vec<usize> = (0..plan.moves.len())
            .filter(|&i| plan.moves[i].motion == Motion::Feed)
            .collect();
        let xy = |p: [f32; 3]| [0, 1].map(|k| (p[k] * 100.).round() as i64);
        let mut holes: Vec<_> = feeds.iter().map(|&i| xy(plan.moves[i].to)).collect();
        holes.dedup();
        assert_eq!(holes.len(), 5);
        // Pecked, each peck up to the clearance before the next: steps by
        // the clearance alone would be a "Hole" a peck, "Hole 1" to
        // "Hole 59".
        let mut by_clearance = plan.clone();
        by_clearance.holes.clear();
        assert_eq!(plan_steps(&by_clearance).len(), 59);
        assert_eq!(drill.steps.len(), 5);
        let labels: Vec<String> = (0..5).map(|s| drill.label(s)).collect();
        assert_eq!(labels, ["Hole 1", "Hole 2", "Hole 3", "Hole 4", "Hole 5"]);
        assert_eq!(drill.labels(0..=4), "Holes 1–5");
        assert_eq!(drill.labels(1..=1), "Hole 2");
        assert_eq!(drill.label_of(1), "Hole 2 of 5");
        // Each step all the pecks of one hole.
        for (s, step) in drill.steps.iter().enumerate() {
            let at: Vec<_> = (feeds.iter())
                .filter(|i| step.moves.contains(i))
                .map(|&i| xy(plan.moves[i].to))
                .collect();
            assert!(at.len() > 1 && at.iter().all(|p| *p == holes[s]), "{s}");
        }
        // The program: its marks count the holes, and none is done while
        // the tool is up between two of its pecks.
        let programs =
            crate::post::checked_to_start(&doc, &mesh, 0, &sims, PostSettings::for_uc100, &[])
                .unwrap();
        let (r, p) = (programs.iter().enumerate())
            .find(|(_, p)| p.indexes.contains(&oi))
            .unwrap();
        let k = p.indexes.iter().position(|&o| o == oi).unwrap();
        let mark_ops: Vec<Option<MarkOp>> = (p.indexes.iter())
            .map(|&o| steps.iter().find(|s| s.op == o).map(|s| s.mark_op(&[])))
            .collect();
        let marks = step_marks(&p.blocks, &mark_ops);
        let is_feed = |b: &Block| {
            matches!(
                b,
                Block::Move {
                    op,
                    kind: Kind::Feed,
                    ..
                } if *op == k
            )
        };
        let fed: Vec<usize> = (0..p.blocks.len())
            .filter(|&b| is_feed(&p.blocks[b]))
            .collect();
        assert_eq!(fed.len(), feeds.len());
        let of_feed: Vec<usize> = fed.iter().map(|&b| marks[b].unwrap().1).collect();
        assert_eq!(of_feed.first(), Some(&0));
        assert_eq!(of_feed.last(), Some(&4));
        assert!(of_feed.windows(2).all(|w| w[1] == w[0] || w[1] == w[0] + 1));
        for (b, m) in marks.iter().enumerate() {
            let Some((o, s, true)) = *m else { continue };
            if o != k {
                continue;
            }
            if let Some(&next) = fed.iter().find(|&&f| f > b) {
                assert_eq!(
                    marks[next].unwrap().1,
                    s + 1,
                    "block {b} said hole {s} done"
                );
            }
        }
        // The tablet file: the same 5 holes, each ending where Stop after
        // step stops, after its last peck.
        let zip = crate::bundle::pack(&doc).unwrap();
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(zip)).unwrap();
        let runs: crate::bundle::Runs =
            serde_json::from_reader(zip.by_name(crate::bundle::RUNS).unwrap()).unwrap();
        let tablet: Vec<_> = (runs.setups[0].tool_runs[r].steps.iter())
            .filter(|t| t.operation == oi)
            .collect();
        let labels: Vec<&str> = tablet.iter().map(|t| t.label.as_str()).collect();
        assert_eq!(labels, ["Hole 1", "Hole 2", "Hole 3", "Hole 4", "Hole 5"]);
        for (s, t) in tablet.iter().enumerate() {
            let last = fed.iter().rfind(|&&b| marks[b].unwrap().1 == s).unwrap();
            let last = *last as u32 + 1;
            assert!(!t.end_lines.is_empty(), "{}", t.label);
            assert!(
                t.end_lines.iter().all(|[x, _]| *x > last),
                "{} ends {:?}, its last peck on line {last}",
                t.label,
                t.end_lines
            );
        }
    }

    /// A Boring's steps are its visits of its holes, a pass at a time, and
    /// are called so (LIO-558): two Ø32 bores 70 mm apart, drilled Ø12,
    /// milled to Ø31.4 and bored to Ø32 in 3 passes (Ø31.65 and Ø31.9 at
    /// 0.15 a pass on the radius, then the size). The head goes round both
    /// bores each pass, rising to the clearance between them, so its 6 steps
    /// are "Pass 1 · Hole 1" to "Pass 3 · Hole 2", not "Hole 1" to "Hole 6",
    /// hole 2 the same bore in every pass, and its row counts "3 passes · 2
    /// holes". The running program's marks count a visit a step, each pass's
    /// pause to set the head between its first visit and the visit before,
    /// and the tablet file names them the same. A start at "Pass 2 · Hole 2"
    /// sends the rest with the third pass's pause only: the head is set for
    /// the second, which took hole 1; "Pass 2 · Hole 1" skipped alone, the
    /// second pass's pause goes before "Pass 2 · Hole 2" instead, the head
    /// still set for the first. A Boring of one bore reads "Pass 1" to
    /// "Pass 3".
    #[test]
    fn a_borings_steps_are_named_by_pass_and_hole() {
        use crate::cam::boring::tests::{
            drill, drilling, end_mill, head, job, on_uc100, ring_triangles,
        };
        use crate::model::{BORE_RETRACTS, BORING, HELICAL_MILLING, Machine};
        let dir = tempfile::tempdir().unwrap();
        // `n` bores, each drilled, milled and bored (operation 2).
        let bored = |n: usize| {
            let tris: Vec<[[f32; 3]; 3]> = (0..n)
                .flat_map(|k| {
                    let x = 70. * k as f32;
                    (ring_triangles(32., 60., 28.).into_iter())
                        .map(move |t| t.map(|p| [p[0] + x, p[1], p[2]]))
                })
                .collect();
            let mesh = crate::mesh::normalize(tris).unwrap();
            let t = drill("12");
            let mut d = drilling(&mesh, &t, "Deep drilling");
            d.p.insert("peck".into(), "12".into());
            d.p.insert("breakthrough".into(), "4.5".into());
            let m = end_mill("10");
            let mut h = drilling(&mesh, &m, HELICAL_MILLING);
            for (k, v) in [("bore_allowance", "0.6"), ("rpm", "8000"), ("feed", "800")] {
                h.p.insert(k.into(), v.into());
            }
            let b = head(BORE_RETRACTS[0]);
            let mut o = drilling(&mesh, &b, BORING);
            o.p.insert("hole_diameter".into(), "32".into());
            o.p.insert("bore_cut".into(), "0.15".into());
            let (doc, all) = job(&mesh, Machine::new(3), vec![(t, d), (m, h), (b, o)]);
            for r in &all {
                assert!(r.is_ok(), "{:?}", r.as_ref().err());
            }
            (on_uc100(doc, dir.path()), mesh)
        };
        let oi = 2;
        let (doc, mesh) = bored(2);
        let sims = simulated(&doc, &mesh, 0);
        let steps = op_steps(&doc, 0, &sims);
        let bore = steps.iter().find(|s| s.op == oi).unwrap();
        let plan = &(sims.iter().find(|s| s.0 == oi).unwrap().1.as_ref())
            .unwrap()
            .plan;
        let sets: Vec<usize> = (plan.stops.iter())
            .filter(|s| s.size.is_some())
            .map(|s| s.at)
            .collect();
        assert_eq!(sets.len(), 3, "{:?}", plan.stops);
        assert_eq!(
            (plan.holes.len(), bore.steps.len(), bore.passes()),
            (6, 6, 3)
        );
        let labels: Vec<String> = (0..6).map(|s| bore.label(s)).collect();
        let named = [
            "Pass 1 · Hole 1",
            "Pass 1 · Hole 2",
            "Pass 2 · Hole 1",
            "Pass 2 · Hole 2",
            "Pass 3 · Hole 1",
            "Pass 3 · Hole 2",
        ];
        assert_eq!(labels, named);
        assert_eq!(bore.kind(), ("Pass", "Passes"));
        assert_eq!(bore.count(), "3 passes · 2 holes");
        // The run line counts passes and holes, not visits.
        assert_eq!(bore.label_of(3), "Pass 2 of 3 · Hole 2 of 2");
        assert_eq!(bore.labels(0..=5), "Passes 1–3");
        assert_eq!(bore.labels(2..=3), "Pass 2");
        assert_eq!(bore.labels(2..=5), "Passes 2–3");
        assert_eq!(bore.labels(1..=2), "Pass 1 · Hole 2 – Pass 2 · Hole 1");
        assert_eq!(bore.labels(0..=4), "Pass 1 · Hole 1 – Pass 3 · Hole 1");
        // Folded in the step list, a run is split where passes start.
        assert_eq!(bore.folds(1..=4), [1..=1, 2..=3, 4..=4]);
        assert_eq!(bore.folds(1..=5), [1..=1, 2..=5]);
        assert_eq!(bore.folds(0..=4), [0..=3, 4..=4]);
        assert_eq!(
            bore.folds(0..=5),
            vec![0..=5; 1],
            "whole passes are one line"
        );
        assert_eq!(
            bore.folds(2..=3),
            vec![2..=3; 1],
            "one whole pass is one line"
        );
        // Each step one visit, in to the bore's top and one cut down it;
        // hole 1 and hole 2 are the same two bores in every pass, and each
        // pass is set before its first visit.
        let xy = |p: [f32; 3]| [0, 1].map(|k| (p[k] * 100.).round() as i64);
        let per = bore.steps[0].feeds;
        let at: Vec<[i64; 2]> = (bore.steps.iter())
            .map(|st| {
                assert_eq!(st.feeds, per, "{st:?}");
                let cuts: Vec<usize> = (st.moves.clone())
                    .filter(|&i| plan.moves[i].motion == Motion::Feed)
                    .filter(|&i| plan.moves[i].cutting_pass)
                    .collect();
                assert_eq!(cuts.len(), 1, "{st:?}");
                xy(plan.moves[cuts[0]].to)
            })
            .collect();
        assert_ne!(at[0], at[1]);
        assert_eq!((&at[2..4], &at[4..6]), (&at[0..2], &at[0..2]));
        for (p, &set) in sets.iter().enumerate() {
            let first = &bore.steps[2 * p];
            let feed = (first.moves.clone())
                .find(|&i| plan.moves[i].motion == Motion::Feed)
                .unwrap();
            assert!(first.moves.start <= set && set <= feed, "pass {p}");
        }
        // Three passes of three holes: a range within a pass says its holes.
        let mut three = bore.clone();
        three.steps = (0..6)
            .map(|s| Step {
                pass: Some(s / 3),
                ..bore.steps[0].clone()
            })
            .collect();
        assert_eq!(three.labels(0..=1), "Pass 1 · Holes 1–2");
        assert_eq!(three.labels(4..=5), "Pass 2 · Holes 2–3");
        assert_eq!(three.labels(3..=5), "Pass 2");
        assert_eq!(three.count(), "2 passes · 3 holes");
        assert_eq!(three.folds(1..=4), [1..=2, 3..=4]);
        assert_eq!(three.labels(1..=2), "Pass 1 · Holes 2–3");
        // The program: a visit a step, done only once the head leaves it,
        // each pause between the visit before and its pass's first.
        let programs =
            crate::post::checked_to_start(&doc, &mesh, 0, &sims, PostSettings::for_uc100, &[])
                .unwrap();
        let fed_of = |p: &Checked| -> (usize, Vec<usize>, Vec<usize>) {
            let k = p.indexes.iter().position(|&o| o == oi).unwrap();
            let fed = (0..p.blocks.len())
                .filter(
                    |&b| matches!(p.blocks[b], Block::Move { op, kind: Kind::Feed, .. } if op == k),
                )
                .collect();
            let pauses = (0..p.blocks.len())
                .filter(|&b| matches!(p.blocks[b], Block::Pause { op, .. } if op == k))
                .collect();
            (k, fed, pauses)
        };
        let p = programs.iter().find(|p| p.indexes.contains(&oi)).unwrap();
        let (k, fed, pauses) = fed_of(p);
        let mark_ops: Vec<Option<MarkOp>> = (p.indexes.iter())
            .map(|&o| steps.iter().find(|s| s.op == o).map(|s| s.mark_op(&[])))
            .collect();
        let marks = step_marks(&p.blocks, &mark_ops);
        // A visit's feed moves, their first block and their last.
        let visits = |fed: &[usize]| -> Vec<(usize, usize)> {
            assert_eq!(fed.len() % per, 0);
            (fed.chunks(per)).map(|v| (v[0], v[per - 1])).collect()
        };
        let of_feed: Vec<usize> = fed.iter().map(|&b| marks[b].unwrap().1).collect();
        let by_visit: Vec<usize> = (0..6).flat_map(|s| vec![s; per]).collect();
        assert_eq!(of_feed, by_visit);
        let v = visits(&fed);
        assert_eq!(pauses.len(), 3);
        assert!(pauses[0] < v[0].0);
        for q in 1..3 {
            assert!(
                v[2 * q - 1].1 < pauses[q] && pauses[q] < v[2 * q].0,
                "pass {q}"
            );
        }
        for (b, m) in marks.iter().enumerate() {
            let Some((o, s, true)) = *m else { continue };
            if o != k {
                continue;
            }
            if let Some(&next) = fed.iter().find(|&&f| f > b) {
                assert_eq!(
                    marks[next].unwrap().1,
                    s + 1,
                    "block {b} said visit {s} done"
                );
            }
        }
        // The tablet file names them the same.
        let tablet = tablet_runs(&doc, Ok(&mesh), 0);
        assert!(tablet.problems.is_empty(), "{:?}", tablet.problems);
        let said: Vec<&str> = (tablet.tool_runs.iter())
            .flat_map(|r| &r.steps)
            .filter(|t| t.operation == oi)
            .map(|t| t.label.as_str())
            .collect();
        assert_eq!(said, named);
        // Started at "Pass 2 · Hole 2": that visit and the third pass, set
        // once, before it.
        let (started, _) = check_steps(&doc, &mesh, 0, &[], &[(oi, vec![0, 1, 2])]);
        let started = started.unwrap();
        let p = started.iter().find(|p| p.indexes.contains(&oi)).unwrap();
        let (_, fed, pauses) = fed_of(p);
        let v = visits(&fed);
        assert_eq!((v.len(), pauses.len()), (3, 1));
        assert!(v[0].1 < pauses[0] && pauses[0] < v[1].0);
        let Block::Pause { message, .. } = &p.blocks[pauses[0]] else {
            unreachable!()
        };
        assert!(message.contains("pass 3 of 3"), "{message}");
        // "Pass 2 · Hole 1" skipped alone: the head is still set for the
        // first pass after "Pass 1 · Hole 2", so the program still pauses to
        // set it for the second, before "Pass 2 · Hole 2", the spindle
        // stopped.
        let (skipping, _) = check_steps(&doc, &mesh, 0, &[], &[(oi, vec![2])]);
        let skipping = skipping.unwrap();
        let p = skipping.iter().find(|p| p.indexes.contains(&oi)).unwrap();
        let (_, fed, pauses) = fed_of(p);
        let v = visits(&fed);
        assert_eq!((v.len(), pauses.len()), (5, 3));
        assert!(v[1].1 < pauses[1] && pauses[1] < v[2].0);
        let said = |b: usize| match &p.blocks[b] {
            Block::Pause { message, .. } => message.clone(),
            _ => unreachable!(),
        };
        assert!(
            said(pauses[1]).contains("pass 2 of 3"),
            "{}",
            said(pauses[1])
        );
        assert!(
            said(pauses[2]).contains("pass 3 of 3"),
            "{}",
            said(pauses[2])
        );
        let actions = lower(p, 0, [100., 100., -100., 0., 0., 0.], [0.; 6]).unwrap();
        let mut on = false;
        let mut paused = 0;
        for l in &actions {
            match &l.action {
                uc100::gcode::Action::Spindle { state, .. } => {
                    on = *state != uc100::gcode::Spindle::Off
                }
                uc100::gcode::Action::Pause { reason } => {
                    assert!(!on, "paused with the spindle on: {reason}");
                    paused += 1;
                }
                _ => {}
            }
        }
        assert_eq!(paused, 3);
        // One bore: a step is a pass.
        let (doc, mesh) = bored(1);
        let sims = simulated(&doc, &mesh, 0);
        let one = (op_steps(&doc, 0, &sims).into_iter())
            .find(|s| s.op == oi)
            .unwrap();
        let labels: Vec<String> = (0..one.steps.len()).map(|s| one.label(s)).collect();
        assert_eq!(labels, ["Pass 1", "Pass 2", "Pass 3"]);
        assert_eq!(one.count(), "3 passes");
        assert_eq!(one.label_of(1), "Pass 2 of 3");
        assert_eq!(one.labels(1..=2), "Passes 2–3");
    }

    /// A helical-milled bore is one step, all its rings (LIO-555): the
    /// tablet fixture's Drilling as Helical milling with a Ø1.5 end mill in
    /// its 5 holes, its clearance at its retract height, 5 mm, alone in the
    /// setup so its bores start at the stock top. Each bore is milled in
    /// rings, and the tool rises out of the bore to its approach height
    /// between two, 5 mm over the top, which is the clearance: the steps by the
    /// clearance alone would be a "Hole" a ring. It has 5 steps, "Hole 1" to
    /// "Hole 5", each every ring of one bore. Skipped bores leave the others
    /// whole; the running program's marks count the bores as the plan does
    /// and call one done only once the tool leaves it for the next, so Stop
    /// after step stops between bores, never between two rings; the tablet
    /// file says the same.
    #[test]
    fn a_helical_milled_bores_rings_are_in_the_step_of_their_bore() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("template.toml");
        std::fs::write(&profile, uc100::config::TEMPLATE).unwrap();
        let (mut doc, mesh) = tablet_fixture(&profile);
        let mut mill = crate::model::Tool::new("Flat 1.5", "Flat end mill", "1.5");
        for (k, v) in [("length", "60"), ("stickout", "65"), ("number", "4")] {
            mill.p.insert(k.into(), v.into());
        }
        let oi = (doc.setups[0].operations.iter())
            .position(|o| o.strategy == crate::model::Strategy::Drill)
            .unwrap();
        // The Face and Pocket before it would leave its bores' tops 4 mm
        // lower, their approach height under the clearance.
        for (o, op) in doc.setups[0].operations.iter_mut().enumerate() {
            op.enabled = o == oi;
        }
        let op = &mut doc.setups[0].operations[oi];
        op.tool = mill.id.clone();
        op.p.insert("cycle".into(), crate::model::HELICAL_MILLING.into());
        op.p.insert("retract".into(), "5".into());
        op.p.insert("clearance".into(), "5".into());
        doc.tools.push(mill);
        let sims = simulated(&doc, &mesh, 0);
        let steps = op_steps(&doc, 0, &sims);
        let bores = steps.iter().find(|s| s.op == oi).unwrap();
        let plan = &(sims.iter().find(|s| s.0 == oi).unwrap().1.as_ref())
            .unwrap()
            .plan;
        let feeds: Vec<usize> = (0..plan.moves.len())
            .filter(|&i| plan.moves[i].motion == Motion::Feed)
            .collect();
        assert_eq!(plan.holes.len(), 5);
        // Each bore in 5 rings (Ø1.5 at 40% stepover), each up to the
        // clearance before the next: by the clearance alone, "Hole 1" to
        // "Hole 25".
        let mut by_clearance = plan.clone();
        by_clearance.holes.clear();
        assert_eq!(plan_steps(&by_clearance).len(), 25);
        assert_eq!(bores.steps.len(), 5);
        let labels: Vec<String> = (0..5).map(|s| bores.label(s)).collect();
        assert_eq!(labels, ["Hole 1", "Hole 2", "Hole 3", "Hole 4", "Hole 5"]);
        assert_eq!(bores.labels(0..=4), "Holes 1–5");
        // Each step every ring of one bore: its feeds round the centre its
        // last one ends at, risen to the clearance between two rings, and
        // each bore another, apart from the others.
        let clear = bores.clear;
        let mut discs: Vec<([f32; 3], f32)> = vec![];
        for (s, step) in bores.steps.iter().enumerate() {
            let fed: Vec<usize> = feeds
                .iter()
                .copied()
                .filter(|i| step.moves.contains(i))
                .collect();
            assert_eq!(fed.len(), step.feeds, "{s}");
            let c = plan.moves[*fed.last().unwrap()].to;
            let reach = (fed.iter())
                .map(|&i| (plan.moves[i].to[0] - c[0]).hypot(plan.moves[i].to[1] - c[1]))
                .fold(0., f32::max);
            let rises = (fed[0]..*fed.last().unwrap())
                .filter(|&i| plan.moves[i].motion == Motion::Rapid)
                .filter(|&i| plan.moves[i].from[2] < clear - AT_CLEARANCE)
                .filter(|&i| plan.moves[i].to[2] >= clear - AT_CLEARANCE)
                .count();
            assert_eq!(rises, 4, "bore {s} rises between its 5 rings");
            for (o, r) in &discs {
                let apart = (o[0] - c[0]).hypot(o[1] - c[1]);
                assert!(apart > r + reach + 2. * plan.radius, "bore {s}");
            }
            discs.push((c, reach));
        }
        // Skipped (or before a start at a step), bores go whole: the plan
        // without bores 1 and 3 is bores 2, 4 and 5, every ring of each.
        let cut = [0, 2];
        let less = cut_plan(plan, &bores.steps, &cut);
        let kept: Vec<&Step> = (bores.steps.iter().enumerate())
            .filter(|(s, _)| !cut.contains(s))
            .map(|(_, s)| s)
            .collect();
        let again = plan_steps(&less);
        assert_eq!(again.len(), 3);
        for (a, k) in again.iter().zip(&kept) {
            assert_eq!(a.feeds, k.feeds);
        }
        // The programs: their marks count the bores, the sent ones of a run
        // that skips two too, and none is done while the tool is up between
        // two of its rings.
        let (skipping, _) = check_steps(&doc, &mesh, 0, &[], &[(oi, cut.to_vec())]);
        let skipping = skipping.unwrap();
        let programs =
            crate::post::checked_to_start(&doc, &mesh, 0, &sims, PostSettings::for_uc100, &[])
                .unwrap();
        let mut whole = (vec![], vec![]);
        for (programs, cut, sent) in [
            (&skipping, &cut[..], kept.clone()),
            (&programs, &[][..], bores.steps.iter().collect()),
        ] {
            let p = programs.iter().find(|p| p.indexes.contains(&oi)).unwrap();
            let k = p.indexes.iter().position(|&o| o == oi).unwrap();
            let mark_ops: Vec<Option<MarkOp>> = (p.indexes.iter())
                .map(|&o| {
                    let cut = if o == oi { cut } else { &[] };
                    steps.iter().find(|s| s.op == o).map(|s| s.mark_op(cut))
                })
                .collect();
            let marks = step_marks(&p.blocks, &mark_ops);
            let fed: Vec<usize> = (0..p.blocks.len())
                .filter(|&b| {
                    matches!(
                        p.blocks[b],
                        Block::Move {
                            op,
                            kind: Kind::Feed,
                            ..
                        } if op == k
                    )
                })
                .collect();
            let of_feed: Vec<usize> = fed.iter().map(|&b| marks[b].unwrap().1).collect();
            for (s, step) in sent.iter().enumerate() {
                assert_eq!(
                    of_feed.iter().filter(|&&m| m == s).count(),
                    step.feeds,
                    "bore {s} sent of cut {cut:?}"
                );
            }
            assert!(of_feed.windows(2).all(|w| w[1] == w[0] || w[1] == w[0] + 1));
            for (b, m) in marks.iter().enumerate() {
                let Some((o, s, true)) = *m else { continue };
                if o != k {
                    continue;
                }
                if let Some(&next) = fed.iter().find(|&&f| f > b) {
                    assert_eq!(
                        marks[next].unwrap().1,
                        s + 1,
                        "block {b} said bore {s} done, cut {cut:?}"
                    );
                }
            }
            whole = (fed, marks);
        }
        // The tablet file: the same 5 bores, each ending where Stop after
        // step stops, after its last ring.
        let (fed, marks) = whole;
        let r = programs
            .iter()
            .position(|p| p.indexes.contains(&oi))
            .unwrap();
        let zip = crate::bundle::pack(&doc).unwrap();
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(zip)).unwrap();
        let runs: crate::bundle::Runs =
            serde_json::from_reader(zip.by_name(crate::bundle::RUNS).unwrap()).unwrap();
        let tablet: Vec<_> = (runs.setups[0].tool_runs[r].steps.iter())
            .filter(|t| t.operation == oi)
            .collect();
        let labels: Vec<&str> = tablet.iter().map(|t| t.label.as_str()).collect();
        assert_eq!(labels, ["Hole 1", "Hole 2", "Hole 3", "Hole 4", "Hole 5"]);
        for (s, t) in tablet.iter().enumerate() {
            let last = fed.iter().rfind(|&&b| marks[b].unwrap().1 == s).unwrap();
            let last = *last as u32 + 1;
            assert!(!t.end_lines.is_empty(), "{}", t.label);
            assert!(
                t.end_lines.iter().all(|[x, _]| *x > last),
                "{} ends {:?}, its last ring on line {last}",
                t.label,
                t.end_lines
            );
        }
    }

    /// A chuck-held operation's bores are a step each too (LIO-555): a
    /// Helical milling of the threaded plate's holes with a Ø1.5 end mill,
    /// its clearance at its retract height, on a bar in the chuck. Its way in
    /// comes down from over what the jaws swing (`indexed::enter_held`),
    /// far above the clearance it retracts to between bores; steps by its
    /// highest Z would be one, all its bores in it. They start at the
    /// clearance it rises to after its first cut, "Hole 1" to "Hole n" on
    /// the computer and in the tablet file.
    #[test]
    fn a_chuck_held_operations_bores_are_a_step_each() {
        use crate::model::{HELICAL_MILLING, Tool};
        let dir = tempfile::tempdir().unwrap();
        let mesh = crate::mesh::load(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/threaded-plate.3mf"),
        )
        .unwrap();
        let mut tool = Tool::new("Flat 1.5", "Flat end mill", "1.5");
        for (k, v) in [("length", "60"), ("stickout", "65"), ("number", "1")] {
            tool.p.insert(k.into(), v.into());
        }
        let mut op = crate::cam::boring::tests::drilling(&mesh, &tool, HELICAL_MILLING);
        for (k, v) in [
            ("through", "No"),
            ("clearance", "5"),
            ("retract", "5"),
            ("coolant", "Off"),
            ("feed", "30"),
            ("rpm", "8000"),
        ] {
            op.p.insert(k.into(), v.into());
        }
        let mut doc = crate::indexed::tests::bar_job(&mesh, 20., "100", "-150", "90", op.clone());
        op.tool = tool.id.clone();
        doc.tools = vec![tool];
        doc.setups[0].operations = vec![op];
        doc.setups.truncate(1);
        let setup = &mut doc.setups[0];
        setup.p.insert("stock_diameter".into(), "44".into());
        let tz = 22. - mesh.dimensions[2] / 2. - 4. / mesh.display_scale();
        setup.p.insert("tz".into(), tz.to_string());
        let profile = dir.path().join("template.toml");
        // With an A axis, and as fast as the plans' 6000 mm/min rapids.
        let fast = (uc100::config::TEMPLATE)
            .replace("max_velocity = 2000.0", "max_velocity = 12000.0")
            .replace("max_velocity = 1000.0", "max_velocity = 6000.0");
        let config = format!(
            "{fast}\n[axes.a]\nsteps_per_unit = 10.0\nmax_velocity = 360.0\nacceleration = 100.0\nsoft_min = -360.0\nsoft_max = 360.0\nstep = {{ pin = 8 }}\ndir = {{ pin = 9 }}\n"
        );
        std::fs::write(&profile, config).unwrap();
        for (k, v) in [("z", "300"), ("controller", "UC100")] {
            setup.machine.p.insert(k.into(), v.into());
        }
        let profile_path = profile.display().to_string();
        setup.machine.p.insert("uc100_profile".into(), profile_path);
        doc.machines = vec![doc.setups[0].machine.clone()];
        doc.active = doc.setups[0].id.clone();
        doc.file = Some(crate::model::ModelFile {
            name: "threaded-plate.3mf".into(),
            path: std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("examples/threaded-plate.3mf"),
            format: "3mf".into(),
            bytes: 0,
            thickness: None,
        });
        doc.sample_model = false;
        doc.sync_stock_sources();
        doc.sync_turns(Some(&mesh));
        let sims = simulated(&doc, &mesh, 0);
        let plan = &sims[0].1.as_ref().unwrap().plan;
        let n = plan.holes.len();
        assert!(n > 1, "{n}");
        // It comes in from far above where it retracts to.
        let first = (plan.moves.iter())
            .position(|m| m.motion == Motion::Feed)
            .unwrap();
        let clear = step_clearance(plan);
        assert!(
            plan.moves[0].from[2] > clear + 10.,
            "{} {clear}",
            plan.moves[0].from[2]
        );
        assert!(
            plan.moves[first..]
                .iter()
                .any(|m| m.to[2] >= clear - AT_CLEARANCE)
        );
        let steps = op_steps(&doc, 0, &sims);
        assert_eq!(steps[0].steps.len(), n);
        let want: Vec<String> = (1..=n).map(|h| format!("Hole {h}")).collect();
        let labels: Vec<String> = (0..n).map(|s| steps[0].label(s)).collect();
        assert_eq!(labels, want);
        // The tablet file: a step a bore, each ending somewhere to stop.
        let tablet = tablet_runs(&doc, Ok(&mesh), 0);
        assert!(tablet.problems.is_empty(), "{:?}", tablet.problems);
        let tablet: Vec<_> = tablet.tool_runs.iter().flat_map(|r| &r.steps).collect();
        let labels: Vec<&str> = tablet.iter().map(|t| t.label.as_str()).collect();
        assert_eq!(labels, want);
        assert!(tablet.iter().all(|t| !t.end_lines.is_empty()));
    }

    /// A chuck-held Contour's levels are a step each (LIO-565): the plain
    /// setup on the bar in the chuck, its roughing Contour alone, comes in
    /// from over what the jaws swing (`indexed::enter_held`) and retracts to
    /// its own clearance between levels. Air trimming replaces some of those
    /// retracts by links of its own, which rose to the swing height, the
    /// plan's highest rapid: by the highest Z from the first cut on
    /// (`step_clearance`), only those split it, Z38, Z36 and Z34 in one step
    /// called "Pass 1". The links go no higher than its own clearance now:
    /// every level starts at it, "Level 1" to "Level 7" at Z38 to Z27; a
    /// skipped level leaves the others whole; the running program's marks
    /// count the levels as the plan does, and the tablet file says them.
    #[test]
    fn a_chuck_held_contours_levels_are_a_step_each() {
        let dir = tempfile::tempdir().unwrap();
        let (mut doc, mesh) = crate::indexed::tests::chuck_setup_1_job("90, 180");
        doc.setups.truncate(1);
        doc.setups[0].operations.truncate(1);
        let profile = dir.path().join("template.toml");
        // With an A axis, and as fast as the plans' 6000 mm/min rapids.
        let fast = (uc100::config::TEMPLATE)
            .replace("max_velocity = 2000.0", "max_velocity = 12000.0")
            .replace("max_velocity = 1000.0", "max_velocity = 6000.0");
        let config = format!(
            "{fast}\n[axes.a]\nsteps_per_unit = 10.0\nmax_velocity = 360.0\nacceleration = 100.0\nsoft_min = -360.0\nsoft_max = 360.0\nstep = {{ pin = 8 }}\ndir = {{ pin = 9 }}\n"
        );
        std::fs::write(&profile, config).unwrap();
        let machine = &mut doc.setups[0].machine;
        for (k, v) in [("z", "300"), ("controller", "UC100")] {
            machine.p.insert(k.into(), v.into());
        }
        machine
            .p
            .insert("uc100_profile".into(), profile.display().to_string());
        doc.machines = vec![doc.setups[0].machine.clone()];
        doc.active = doc.setups[0].id.clone();
        doc.sync_stock_sources();
        doc.sync_turns(Some(&mesh));
        let sims = simulated(&doc, &mesh, 0);
        let sim = sims[0].1.as_ref().unwrap();
        let plan = &sim.plan;
        let first = (plan.moves.iter())
            .position(|m| m.motion == Motion::Feed)
            .unwrap();
        let clear = step_clearance(plan);
        // It comes in from far above its clearance, and air trimming linked
        // some of its levels; no move from its first cut on goes back up
        // there.
        let entry = plan.moves[0].from[2];
        assert!(entry > clear + 10., "{entry} {clear}");
        let air = sim.air.as_ref().unwrap();
        assert!(air.stretches > 0, "{air:?}");
        assert!(plan.moves[first..].iter().all(|m| m.to[2] < entry - 10.));
        // Its levels, from the top, each cut flat.
        let mut levels: Vec<f32> = (plan.moves.iter())
            .filter(|m| m.motion == Motion::Feed && (m.from[2] - m.to[2]).abs() < 1e-4)
            .map(|m| m.to[2])
            .collect();
        levels.sort_by(|a, b| b.total_cmp(a));
        levels.dedup_by(|a, b| (*a - *b).abs() < 0.005);
        assert_eq!(levels, [38., 36., 34., 32., 30., 28., 27.]);
        let steps = op_steps(&doc, 0, &sims);
        let contour = &steps[0];
        assert_eq!(contour.kind().0, "Level");
        let zs: Vec<Option<f32>> = contour.steps.iter().map(|s| s.z).collect();
        assert_eq!(zs, levels.iter().map(|&z| Some(z)).collect::<Vec<_>>());
        let want: Vec<String> = (1..=7).map(|l| format!("Level {l}")).collect();
        let labels: Vec<String> = (0..7).map(|s| contour.label(s)).collect();
        assert_eq!(labels, want);
        for w in contour.steps.windows(2) {
            assert!(plan.moves[w[1].moves.start].from[2] >= clear - AT_CLEARANCE);
        }
        // Skipped (or before a start at a step), a level goes whole: the
        // plan without Z36 is the other six, each a step at its own height.
        let less = cut_plan(plan, &contour.steps, &[1]);
        let kept: Vec<Option<f32>> = (plan_steps(&less).iter()).map(|s| s.z).collect();
        let mut others = zs.clone();
        others.remove(1);
        assert_eq!(kept, others);
        // The running program's marks count its levels as the plan does,
        // each done only when the tool has left it for the next.
        let programs =
            crate::post::checked_to_start(&doc, &mesh, 0, &sims, PostSettings::for_uc100, &[])
                .unwrap();
        let p = programs.iter().find(|p| p.indexes.contains(&0)).unwrap();
        let k = p.indexes.iter().position(|&o| o == 0).unwrap();
        let mark_ops: Vec<Option<MarkOp>> = (p.indexes.iter())
            .map(|&o| steps.iter().find(|s| s.op == o).map(|s| s.mark_op(&[])))
            .collect();
        let marks = step_marks(&p.blocks, &mark_ops);
        let fed: Vec<usize> = (0..p.blocks.len())
            .filter(|&b| {
                matches!(
                    p.blocks[b],
                    Block::Move {
                        op,
                        kind: Kind::Feed,
                        ..
                    } if op == k
                )
            })
            .collect();
        let of_feed: Vec<usize> = fed.iter().map(|&b| marks[b].unwrap().1).collect();
        for (s, step) in contour.steps.iter().enumerate() {
            let sent = of_feed.iter().filter(|&&m| m == s).count();
            assert_eq!(sent, step.feeds, "level {s}");
        }
        assert!(of_feed.windows(2).all(|w| w[1] == w[0] || w[1] == w[0] + 1));
        for (b, m) in marks.iter().enumerate() {
            let Some((o, s, true)) = *m else { continue };
            if o != k {
                continue;
            }
            if let Some(&next) = fed.iter().find(|&&f| f > b) {
                assert_eq!(marks[next].unwrap().1, s + 1, "block {b} said level {s} done");
            }
        }
        // The tablet file: a step a level, each ending somewhere to stop.
        let tablet = tablet_runs(&doc, Ok(&mesh), 0);
        assert!(tablet.problems.is_empty(), "{:?}", tablet.problems);
        let tablet: Vec<_> = tablet.tool_runs.iter().flat_map(|r| &r.steps).collect();
        let labels: Vec<&str> = tablet.iter().map(|t| t.label.as_str()).collect();
        assert_eq!(labels, want);
        assert!(tablet.iter().all(|t| !t.end_lines.is_empty()));
    }

    /// The fixture job's Face, Pocket, Contour and Drill on a UC100 (the
    /// template profile in `dir`): its Parallel and Waterline off, to be quick.
    fn four_ops(dir: &std::path::Path) -> (Document, Mesh) {
        let (mut doc, mesh) = fixture::job();
        let profile = dir.join("template.toml");
        std::fs::write(&profile, uc100::config::TEMPLATE).unwrap();
        let m = &mut doc.setups[0].machine;
        m.p.insert("controller".into(), "UC100".into());
        m.p.insert("uc100_profile".into(), profile.display().to_string());
        for o in &mut doc.setups[0].operations[4..] {
            o.enabled = false;
        }
        (doc, mesh)
    }

    /// Real programs split into steps where the tool, between two feeds, is
    /// back at the operation's clearance (LIO-275): the Face's two Z levels,
    /// the Contour's levels, the Drill's holes, the Pocket's passes. Each
    /// step starts at the clearance and they cover the plan; a plan without
    /// steps taken as cut re-enters each kept one from the clearance and
    /// splits into those same steps; the running program's marks count them
    /// as the plan does.
    #[test]
    fn real_programs_split_into_levels_passes_and_holes() {
        let dir = tempfile::tempdir().unwrap();
        let (doc, mesh) = four_ops(dir.path());
        let sims = simulated(&doc, &mesh, 0);
        let steps = op_steps(&doc, 0, &sims);
        let of = |oi: usize| steps.iter().find(|s| s.op == oi).unwrap();
        let plan = |oi: usize| {
            &sims
                .iter()
                .find(|s| s.0 == oi)
                .unwrap()
                .1
                .as_ref()
                .unwrap()
                .plan
        };
        let (face, pocket, contour, drill) = (of(0), of(1), of(2), of(3));
        assert_eq!((face.steps.len(), face.kind().0), (2, "Level"));
        assert!(face.steps[0].z.unwrap() > face.steps[1].z.unwrap());
        assert!(contour.steps.len() > 1, "{:?}", contour.steps);
        assert_eq!(drill.kind().0, "Hole");
        // One hole a step, its pecks in it.
        let p = plan(3);
        let mut holes: Vec<[i64; 2]> = (p.moves.iter())
            .filter(|m| m.motion == crate::simulation::Motion::Feed)
            .map(|m| [0, 1].map(|k| (m.to[k] * 100.).round() as i64))
            .collect();
        holes.dedup();
        assert_eq!(drill.steps.len(), holes.len());
        assert!(pocket.steps.len() > 1, "{:?}", pocket.steps.len());
        assert_eq!(face.labels(0..=1), "Levels 1–2");
        for os in [face, pocket, contour, drill] {
            let p = plan(os.op);
            assert_eq!(os.steps[0].moves.start, 0);
            assert_eq!(os.steps.last().unwrap().moves.end, p.moves.len());
            for w in os.steps.windows(2) {
                assert_eq!(w[0].moves.end, w[1].moves.start);
                assert!(p.moves[w[1].moves.start].from[2] >= os.clear - AT_CLEARANCE);
            }
            // Cut: the first two, then the second alone.
            for cut in [vec![0, 1], vec![1]] {
                if cut.len() >= os.steps.len() {
                    continue;
                }
                let less = cut_plan(p, &os.steps, &cut);
                let kept: Vec<&Step> = (0..os.steps.len())
                    .filter(|s| !cut.contains(s))
                    .map(|s| &os.steps[s])
                    .collect();
                let again = plan_steps(&less);
                assert_eq!(again.len(), kept.len(), "{:?} cut {cut:?}", p.strategy);
                assert!(less.moves[0].from[2] >= os.clear - AT_CLEARANCE);
                for w in less.moves.windows(2) {
                    assert_eq!(w[0].to, w[1].from, "{:?} cut {cut:?}", p.strategy);
                }
            }
        }
        // The whole program's marks count each operation's steps.
        let programs =
            crate::post::checked_to_start(&doc, &mesh, 0, &sims, PostSettings::for_uc100, &[])
                .unwrap();
        let p = &programs[0];
        let mark_ops: Vec<Option<MarkOp>> = (p.indexes.iter())
            .map(|&oi| Some(of(oi).mark_op(&[])))
            .collect();
        let marks = step_marks(&p.blocks, &mark_ops);
        for (k, &oi) in p.indexes.iter().enumerate() {
            let last = marks.iter().flatten().rfind(|m| m.0 == k).unwrap();
            assert_eq!(last.1 + 1, of(oi).steps.len(), "operation {oi}");
            assert!(
                last.2,
                "the last step of operation {oi} rose to the clearance"
            );
        }
    }

    /// A start at a step re-enters as a Start here does (LIO-275): the
    /// Contour from its third level is sent from there, its program from
    /// the operation the tool change's safe start (a retract, the spindle,
    /// across at the clearance plane, down onto the level's entry at the
    /// operation's clearance), lowered as a rapid at the clearance first;
    /// Cycle Start then waits over the entry. A skip of steps leaves them
    /// out, the steps after re-entered across at the clearance.
    #[test]
    fn a_start_at_a_step_reenters_from_the_clearance() {
        let dir = tempfile::tempdir().unwrap();
        let (doc, mesh) = four_ops(dir.path());
        let (whole, left) = check_steps(&doc, &mesh, 0, &[], &[]);
        let whole = whole.unwrap();
        let contour = left.steps.iter().find(|s| s.op == 2).unwrap().clone();
        let (programs, _) = check_steps(&doc, &mesh, 0, &[], &[(2, vec![0, 1])]);
        let programs = programs.unwrap();
        let p = &programs[0];
        let k = p.indexes.iter().position(|&oi| oi == 2).unwrap();
        let blocks = p.from(k).unwrap();
        let entry = {
            let sims = simulated(&doc, &mesh, 0);
            let plan = &sims
                .iter()
                .find(|s| s.0 == 2)
                .unwrap()
                .1
                .as_ref()
                .unwrap()
                .plan;
            plan.moves[contour.steps[2].moves.start].from
        };
        let head: Vec<&Block> = blocks.iter().take(8).collect();
        assert!(matches!(head[0], Block::Stop) && matches!(head[1], Block::Retract));
        let approach = blocks.iter().find_map(|b| match b {
            Block::Approach { xy, .. } => Some(*xy),
            _ => None,
        });
        let descend = blocks.iter().find_map(|b| match b {
            Block::Descend { xy, z, .. } => Some((*xy, *z)),
            _ => None,
        });
        assert_eq!(approach, Some([entry[0], entry[1]]));
        assert_eq!(descend, Some(([entry[0], entry[1]], entry[2])));
        assert!(entry[2] >= contour.clear - AT_CLEARANCE);
        // Its first moves: Z alone up to the clearance plane, across there
        // over the entry, then down to it (nothing when the plane is the
        // operation's clearance).
        let plane = clearance_plane(blocks);
        let start = [300., 200., -100., 0., 0., 0.];
        let moves: Vec<[f64; 6]> = (lower(p, k, [0.; 6], start).unwrap().iter())
            .filter_map(|l| match l.action {
                Do::Move { target, .. } => Some(target),
                _ => None,
            })
            .collect();
        let at = |q: [f32; 3]| -> [f64; 3] { std::array::from_fn(|i| (q[i] - p.datum[i]) as f64) };
        let top = (plane - p.datum[2]) as f64;
        assert_eq!(moves[0], [300., 200., top, 0., 0., 0.]);
        assert_eq!(moves[1][..3], [at(entry)[0], at(entry)[1], top]);
        assert!(top >= at(entry)[2] - 1e-4);
        if top > at(entry)[2] + 1e-4 {
            assert_eq!(moves[2][..3], at(entry));
        }
        // Fewer blocks than from the Contour's start, the levels before cut.
        let full = &whole[0];
        assert!(blocks.len() < full.from(k).unwrap().len());
        // The run starts there: Cycle Start's start, and the pause over the
        // entry it lowers with.
        let mut run = Run::new(&doc, 0, 0);
        run.checked_as(
            Arc::new(Ok(whole.clone())),
            Some(Arc::new(left.clone())),
            None,
        );
        run.pick(2, 2, false);
        assert!(run.start_step());
        assert_eq!(run.all_cut(), [(2, vec![0, 1])]);
        run.checked_as(
            Arc::new(Ok(programs.clone())),
            Some(Arc::new(left.clone())),
            None,
        );
        assert_eq!((run.next, run.from, run.at_step()), (0, k, Some((2, 2))));
        assert!(run.partway());
        assert_eq!(run.step_label(2, 2), contour.label(2).to_lowercase());
        // A skip of the Contour's second level: the third follows the first,
        // across at the clearance.
        let (skipped, _) = check_steps(&doc, &mesh, 0, &[], &[(2, vec![1])]);
        let skipped = skipped.unwrap();
        let ops = |p: &Checked| {
            p.blocks
                .iter()
                .filter(|b| matches!(b, Block::Move { op, .. } if *op == k))
                .count()
        };
        assert!(ops(&skipped[0]) < ops(full));
    }

    /// Steps taken as cut are in the stock a run's live carve starts from
    /// (LIO-312): the stock before their operation, and before those ahead
    /// of it, has them cut; the stock after their operation is as it was.
    #[test]
    fn steps_taken_as_cut_are_in_the_stock_before() {
        let dir = tempfile::tempdir().unwrap();
        let (doc, mesh) = four_ops(dir.path());
        let (_, whole) = check_steps(&doc, &mesh, 0, &[], &[]);
        let (_, cut) = check_steps(&doc, &mesh, 0, &[], &[(2, vec![0, 1])]);
        let left = |l: &Left, oi: usize| -> f32 {
            let b = l.before.iter().find(|b| b.op == oi).unwrap();
            b.stock.heights.iter().sum()
        };
        for oi in 0..=2 {
            assert!(left(&cut, oi) < left(&whole, oi), "before operation {oi}");
        }
        assert_eq!(left(&cut, 3), left(&whole, 3));
    }

    /// A start at Face's Pass 2 on the tablet's fixture (LIO-568), its work
    /// zero Stock top center: Pass 1, taken as cut, faced the stock top, but
    /// it was cut with the work zero the start keeps, so the start passes,
    /// the zero found and probed as for the whole run. Pass 1 is still in
    /// the stock the run starts from, the top view's and the 3D view's, and
    /// a work zero set on the stock now finds the top cut, whether the run
    /// starts at Pass 2 or skips Pass 1.
    #[test]
    fn a_start_at_a_later_pass_of_the_first_operation_keeps_its_work_zero() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("template.toml");
        std::fs::write(&profile, uc100::config::TEMPLATE).unwrap();
        let (doc, mesh) = tablet_fixture(&profile);
        assert_eq!(
            crate::model::get(&doc.setups[0].p, "origin"),
            "Stock top center"
        );
        let (whole, left) = check_steps(&doc, &mesh, 0, &[], &[]);
        let whole = whole.unwrap();
        let face_steps = left.steps.iter().find(|s| s.op == 0).unwrap().clone();
        assert!(face_steps.steps.len() >= 2, "{:?}", face_steps.steps.len());
        let (cut, cut_left) = check_steps(&doc, &mesh, 0, &[], &[(0, vec![0])]);
        let cut = cut.unwrap_or_else(|e| panic!("{e:?}"));
        let (p, w) = (&cut[0], &whole[0]);
        assert_eq!(p.indexes.first(), Some(&0));
        assert!(p.from(0).is_ok());
        assert_eq!(p.datum, w.datum);
        assert_eq!(p.probe, w.probe);
        assert_eq!(p.probe_from, w.probe_from);
        assert!(p.probe.is_ok(), "{:?}", p.probe);
        // Pass 1 is cut in the stock before Face, and Face sends less.
        let before = |l: &Left| -> f32 {
            let b = l.before.iter().find(|b| b.op == 0).unwrap();
            b.stock.heights.iter().sum()
        };
        assert!(before(&cut_left) < before(&left));
        // In the 3D view's volume too: the live carve starts with Pass 1 cut.
        let removed = |l: &Left| -> f32 {
            let b = l.before.iter().find(|b| b.op == 0).unwrap();
            b.volume.as_ref().unwrap().removed_volume()
        };
        assert!(
            removed(&cut_left) > removed(&left) + 1000.,
            "{}",
            removed(&cut_left)
        );
        let moves = |c: &Checked| {
            (c.blocks.iter())
                .filter(|b| matches!(b, Block::Move { op: 0, .. }))
                .count()
        };
        assert!(moves(p) < moves(w));
        // A run that keeps the work zero passes; one set on the stock now
        // finds the top cut where it is touched off, whether the run starts
        // at Pass 2, skips Pass 1, or starts at Pass 1 with Pass 2, already
        // cut, skipped: not the stock top it was set on.
        let partly = |s: usize, skip: bool| {
            let mut run = Run::new(&doc, 0, 0);
            run.checked_as(
                Arc::new(Ok(whole.clone())),
                Some(Arc::new(left.clone())),
                None,
            );
            run.pick(0, s, false);
            assert!(if skip {
                run.skip_steps(true)
            } else {
                run.start_step()
            });
            let cut_steps = vec![if skip { s } else { s - 1 }];
            assert_eq!(run.all_cut(), [(0, cut_steps.clone())]);
            let (cut, cut_left) = check_steps(&doc, &mesh, 0, &[], &[(0, cut_steps)]);
            run.checked_as(Arc::new(cut), Some(Arc::new(cut_left)), None);
            assert!(run.partway());
            let partly = run.partly().unwrap().into_owned();
            (partly, run)
        };
        let (skipped, first) = partly(0, true);
        let (started, second) = partly(1, false);
        let (later, run) = partly(1, true);
        // The Work zero tab says what each takes as cut.
        let face = doc.setups[0].operations[0].name();
        let pass = |n: usize| vec![format!("{face} {}", face_steps.label(n).to_lowercase())];
        assert_eq!(first.taken_as_cut(), pass(0));
        assert_eq!(second.taken_as_cut(), pass(0));
        assert_eq!(run.taken_as_cut(), pass(1));
        assert_eq!(skipped, started);
        assert_eq!(later, started);
        assert!(skipped.top_cut && !skipped.cut.is_empty(), "{skipped:?}");
        assert_eq!(skipped.surface, None);
        let centre = (skipped.probe_from.iter()).find(|(r, _)| *r == p.probe.clone().unwrap());
        assert!(centre.is_some_and(|(_, ok)| ok.is_err()), "{centre:?}");
        // Zero Z and Probe Z say why they cannot set Z, naming the steps.
        for r in [&first, &second, &run] {
            let why = r.partway_blocked().unwrap();
            assert!(
                why.starts_with("The steps taken as cut cut the stock's "),
                "{why}"
            );
            assert_eq!(r.touch_off(), None);
            assert_eq!(r.cut_by(), "the steps taken as cut");
            // Nor do the probe spots' reasons speak of operations.
            for (_, ok) in &r.partly().unwrap().probe_from {
                let why = ok.as_ref().unwrap_err();
                assert!(!why.contains("operations"), "{why}");
            }
        }
        // Starting at Pass 1, Cycle Start waits for the work zero, kept or
        // set again.
        assert!(!run.zeroed);
        assert_eq!((run.next, run.from, run.at_step()), (0, 0, None));
    }

    /// The journal of where a run stopped (LIO-275) is written and read
    /// back whole; picked with its setup, it makes the step after the one
    /// done the start once its operation's program is the one it stopped
    /// in, and unfolds its steps; changed, it says so and starts nothing.
    #[test]
    fn a_journal_resumes_its_unchanged_program_only() {
        let (doc, _) = fixture::job();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("uc100").join("m-journal.json");
        let steps = |fingerprint| {
            let step = Step {
                moves: 0..1,
                z: Some(5.),
                seconds: 1.,
                feeds: 1,
                pass: None,
            };
            Left {
                steps: vec![OpSteps {
                    op: 0,
                    clear: 15.,
                    hole: false,
                    steps: vec![step; 3],
                    fingerprint,
                }],
                ..Default::default()
            }
        };
        let fresh = |left: &Left| {
            let mut run = Run::new(&doc, 0, 0);
            run.checked_as(
                Arc::new(Ok(vec![program()])),
                Some(Arc::new(left.clone())),
                None,
            );
            run
        };
        let run = fresh(&steps(7));
        let stopped = Stopped {
            op: 0,
            last: Some(0),
            up: None,
            why: "Stop after step".into(),
        };
        let journal = run
            .journal_of(&stopped, 0, [1., 2., 3., 0., 0., 0.])
            .unwrap();
        assert_eq!((journal.steps, journal.fingerprint), (3, 7));
        journal.write(&path).unwrap();
        let read = Journal::read(&path).unwrap();
        assert_eq!(read, journal);
        // Unchanged: level 2 is the start, checked again first.
        let mut run = Run::new(&doc, 0, 0);
        run.journal = Some(read.clone());
        run.checked_as(
            Arc::new(Ok(vec![program()])),
            Some(Arc::new(steps(7))),
            None,
        );
        assert_eq!(run.start_step, Some((0, 1)));
        assert!(run.recheck && run.programs.is_none() && run.resumed.is_some());
        assert_eq!(run.open, Some(0));
        // The answer keeps it.
        run.checked_as(
            Arc::new(Ok(vec![program()])),
            Some(Arc::new(steps(7))),
            None,
        );
        assert_eq!(run.at_step(), Some((0, 1)));
        // Refused, it is dropped and the setup checked again from the
        // operation's start, which the setup can then run (LIO-311).
        let why = "Face: a rapid descends to X1 Y2 Z-3 through material.";
        run.checked_as(Arc::new(Err(vec![why.into()])), None, None);
        assert_eq!((run.start_step, run.recheck), (None, true));
        assert!(run.programs.is_none() && run.resumed.is_none());
        assert!(run.journal_note.as_ref().unwrap().contains(why));
        run.checked_as(
            Arc::new(Ok(vec![program()])),
            Some(Arc::new(steps(7))),
            None,
        );
        assert!(run.checked().is_some());
        assert_eq!((run.next, run.from, run.at_step()), (0, 0, None));
        // Its steps split otherwise since, as a deep drilling's pecks did
        // before they were put in their holes (LIO-549): step 37 of 59 is
        // no step now, so nothing is picked either.
        let mut run = Run::new(&doc, 0, 0);
        run.journal = Some(Journal {
            steps: 59,
            step: Some(36),
            ..read.clone()
        });
        run.checked_as(
            Arc::new(Ok(vec![program()])),
            Some(Arc::new(steps(7))),
            None,
        );
        assert_eq!((run.start_step, run.recheck), (None, false));
        assert!(run.journal_note.as_ref().unwrap().contains("changed"));
        // Changed: nothing is picked, and that is said.
        let mut run = Run::new(&doc, 0, 0);
        run.journal = Some(read);
        run.checked_as(
            Arc::new(Ok(vec![program()])),
            Some(Arc::new(steps(8))),
            None,
        );
        assert_eq!((run.start_step, run.recheck), (None, false));
        assert!(run.journal_note.as_ref().unwrap().contains("changed"));
        assert_eq!(read_ago(), "3 h ago");
    }

    fn read_ago() -> String {
        let j = Journal {
            project: String::new(),
            setup: String::new(),
            setup_name: String::new(),
            op: 0,
            op_name: String::new(),
            steps: 0,
            step: None,
            wcs: 0,
            offset: [0.; 6],
            tool: String::new(),
            tool_label: String::new(),
            time: 1000,
            fingerprint: 0,
            why: String::new(),
        };
        j.ago(1000 + 3 * 3600 + 60)
    }

    #[test]
    fn a_removed_setup_leaves_no_setup_of_the_project_picked() {
        let (mut doc, _) = fixture::flipped_job();
        let mut run = Run::new(&doc, 1, 0);
        assert_eq!(run.follow(&doc), Some(1));
        // Another setup moves up to the removed one's index.
        doc.setups.remove(1);
        doc.setups.push(doc.setups[0].duplicate(&doc.next_offset()));
        assert_eq!(doc.setups.len(), 2);
        assert_eq!(run.follow(&doc), None);
        assert!((0..doc.setups.len()).all(|si| run.setup != si));
        assert_eq!(
            run.programs.as_deref().and_then(|p| p.as_ref().err()),
            Some(&vec![REMOVED.to_string()])
        );
    }

    fn rotary(range: Option<(&str, &str)>) -> crate::model::Machine {
        let dir = tempfile::tempdir().unwrap();
        let mut m = crate::control::rotary_machine(dir.path()).unwrap();
        if let Some((min, max)) = range {
            m.p.insert("rotary_min".into(), min.into());
            m.p.insert("rotary_max".into(), max.into());
        }
        m
    }

    #[test]
    fn a_range_that_matches_the_profiles_a_soft_limits_is_accepted() {
        let config = MachineConfig::from_toml(&crate::control::rotary_profile()).unwrap();
        assert_eq!(a_problems(&rotary(None), &config), Vec::<String>::new());
        // The same width somewhere else does not agree.
        let shifted = rotary(Some(("0", "360")));
        assert_eq!(a_problems(&shifted, &config).len(), 1);
    }

    #[test]
    fn a_range_that_differs_from_the_profiles_a_soft_limits_is_refused() {
        let config = MachineConfig::from_toml(&crate::control::rotary_profile()).unwrap();
        let problems = a_problems(&rotary(Some(("-90", "90"))), &config);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("A range in AXIS CAM is -90.000 to 90.000 degrees"));
        assert!(problems[0].contains("soft limits are -180.000 to 180.000 degrees"));
        // Soft limits on A are required, as on XYZ.
        let mut open = config.clone();
        open.axes.a.as_mut().unwrap().soft_max = None;
        let problems = a_problems(&rotary(None), &open);
        assert!(
            problems[0].contains("sets no soft limits on A"),
            "{problems:?}"
        );
    }

    #[test]
    fn a_machine_with_an_a_axis_on_a_profile_without_one_is_refused() {
        let config = MachineConfig::from_toml(include_str!("../assets/jy5800.toml")).unwrap();
        let problems = a_problems(&rotary(None), &config);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("has no [axes.a]"));
    }

    #[test]
    fn a_machine_without_an_a_axis_is_not_asked_about_a() {
        let dir = tempfile::tempdir().unwrap();
        let owners = crate::control::owners_machine(dir.path()).unwrap();
        for toml in [
            include_str!("../assets/jy5800.toml").to_string(),
            crate::control::rotary_profile(),
        ] {
            let config = MachineConfig::from_toml(&toml).unwrap();
            assert!(a_problems(&owners, &config).is_empty());
        }
    }
}
