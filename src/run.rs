//! Running a setup on the connected UC100 without G-code: its checked
//! programs, one per tool run; each lowered to the driver's actions when
//! Cycle Start is pressed; the checks before Cycle Start; progress and the
//! manual tool change between tool runs.
use crate::air::step_clearance;
use crate::control::Connection;
use crate::mesh::Mesh;
use crate::model::Document;
use crate::post::{Block, Checked, Kind, OFFSETS, Partly, PostSettings, clearance_plane, extents};
// What a setup's UC100 checks refuse is core (LIO-729): `post` holds it.
pub(crate) use crate::post::{output_problems, plan_problems};
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
    /// The part this setup's runs cut, or with operations or steps of it
    /// taken as cut, this session is still the one held
    /// (`Run::stock_unconfirmed`), and nothing else has cut it since: that
    /// stays the stock's. Refused after a disconnect while the Pi shows a
    /// tool run this computer did not send (`Run::away`, LIO-595).
    SamePart,
    /// The part this setup's runs cut, or with operations or steps of it
    /// taken as cut (LIO-606), this session is still the one held, but
    /// after a disconnect (`Run::away`) something else may have cut it
    /// meanwhile: what cut it is unknown (`Run::note_history_unknown`)
    /// until New stock loaded (LIO-595).
    OtherRunsMayHaveCut,
    /// New stock is held: what this setup's runs cut, and what was taken as
    /// cut, this session is forgotten (`Run::stock_unconfirmed`).
    NewStock,
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

/// What a run took as cut (`Run::declared`): its operations skipped as cut
/// and its steps skipped, by operation (index in the setup).
pub type Declared = (Vec<(usize, Skip)>, Vec<(usize, Vec<usize>)>);

/// What the operator took as cut in a setup's run (`Run::declared`), kept
/// once its run is dropped by the session (`App::run_declared`) and on disk
/// with what runs cut (`CutHistory::declared`), so that it survives closing
/// AXIS CAM, and given back when the setup is picked again, until New
/// stock loaded (LIO-583 review, LIO-611).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct KeptDeclared {
    /// The setup's id.
    pub setup: String,
    /// Its operations skipped as cut (`Skip::Cut`), by index in the setup.
    pub skips: Vec<(usize, Skip)>,
    /// Its steps taken as cut, by operation (index in the setup).
    pub steps: Vec<(usize, Vec<usize>)>,
    /// Some of what was taken as cut was dropped since, by a project edit
    /// say (`Run::declared_lost`).
    pub lost: bool,
    /// The print of the plan, its steps and its cutter
    /// (`OpSteps::declared_print`) of each operation `skips` and `steps`
    /// name, as they were taken as cut; None
    /// when not known. They hold only for a check whose prints are these
    /// (`Run::declared_from`).
    #[serde(default)]
    pub prints: Vec<(usize, Option<u64>)>,
}

/// The operations of a setup marked Already cut, each with the print it
/// holds for (`KeptDeclared::prints`; `Run::earlier_taken`, LIO-603).
pub type Marks = Vec<(usize, Option<u64>)>;

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
    /// What Cycle Start recorded over (`xy`, `zero`, `last`, `touched_for`), put
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
    /// A file from before LIO-619 kept a `touched` flag: no touch-off is
    /// known from it, so another is made.
    #[serde(default)]
    touched_for: Option<TouchedFor>,
}

/// Z touched off on the Control page (`Run::touched_for`): for which tool,
/// and the Z work offset it left. That Z is the tool's, so it holds for no
/// other (LIO-619).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TouchedFor {
    /// The tool's id (`tool_id`): the next tool run's when it was touched
    /// off; while the setup is checked again (after an edit, a skip or a
    /// new start), the one the check before found next (`Run::check_again`),
    /// which the Tool change tab showed. None when no tool run was checked
    /// then: it holds for no tool.
    tool: Option<String>,
    /// Made before the setup's first check answered: tool run 1's tool
    /// once that check starts the run there (`checked_as`).
    #[serde(default)]
    first: bool,
    /// The Z work offset it left, once the machine shows what was sent
    /// (`Run::touch_shown`); None until then.
    z: Option<f64>,
    /// A Z set from the reference or by probe was pressed since, and is
    /// not shown applied (`Run::hold_touch`): the offset may still be this
    /// tool's, so it still holds for no other, but it no longer counts as
    /// touched off for this one (LIO-627 review).
    #[serde(default)]
    held: bool,
    /// It holds for the start picked: another start picked or the setup
    /// checked again (`Run::check_again`) forgets it (`Run::forget_touch`,
    /// LIO-618), though the
    /// offset stays its tool's Z. A file from before LIO-618 has none: it
    /// holds no more.
    #[serde(default)]
    for_start: bool,
    /// The machine's active work system it was made in (`Snapshot::
    /// active_wcs`): it holds only for a setup in that one (`touched`),
    /// one pressed before the setup's first check answered too (LIO-643).
    /// None from a file before LIO-643.
    #[serde(default)]
    wcs: Option<usize>,
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

/// What steps taken as cut in the start's operation cut, said as
/// `Partly::cut`, when the check did not carve them into the stock
/// (`Partly::steps`, LIO-582) and has none after their operation, its
/// setup's last; or carved other steps, the start having moved since
/// (`Run::partly`).
pub(crate) const STEPS_UNFOLLOWED: &str =
    "stock cut by the steps taken as cut, which the check does not follow";

/// What operations after a start's, or steps of them, taken as cut cut,
/// said as `Partly::cut`: the check follows the stock a start touches off
/// on only through the operations before it and its own (`Run::partly`,
/// LIO-582).
pub(crate) const LATER_UNFOLLOWED: &str = "stock the operations or steps after the start, taken as cut, cut, which the check does not follow";

/// Why a probe cannot find a work zero from a spot of a stock
/// `LATER_UNFOLLOWED` cut.
pub(crate) const LATER_UNFOLLOWED_WHY: &str = "Operations or steps after the start are taken as cut, and the check does not follow the stock they cut.";

/// What this setup's runs cut earlier, said as `Partly::cut`,
/// while the operator has not said whether the same part is still held
/// (`Run::stock_unconfirmed`).
pub(crate) const STOCK_UNCONFIRMED: &str = "stock earlier runs cut, or with operations or steps of it taken as cut, which may or may not still be held";

/// Why a probe cannot find a work zero from a spot while
/// `Run::stock_unconfirmed`.
const STOCK_UNCONFIRMED_WHY: &str = "Earlier runs cut the stock, or operations or steps of it were taken as cut: say under Work zero whether it is the same part or new stock.";

/// Why Zero Z on the stock top and Probe Z cannot set Z while
/// `Run::stock_unconfirmed`.
const STOCK_UNCONFIRMED_BLOCKED: &str = "Earlier runs cut the stock, or operations or steps of it were taken as cut: press Same part if that part is still held, or New stock loaded if not, under Work zero.";

/// Why Zero Z on the stock top and Probe Z cannot set Z while
/// `Run::stock_unconfirmed` after a disconnect (`Run::away`, LIO-595).
pub(crate) const STOCK_AWAY_BLOCKED: &str = "Earlier runs cut the stock, or operations or steps of it were taken as cut, and the machine was disconnected or AXIS CAM closed since: say under Work zero whether the part is still held and whether anything else ran on the machine meanwhile. The answer covers every setup.";

/// What runs this computer did not follow may have cut, said as
/// `Partly::cut` (`HISTORY_UNKNOWN` in `Run::cut_in_run`): another
/// session's, or anything that ran on the machine while it was
/// disconnected (LIO-586 review, LIO-595).
pub(crate) const HISTORY_UNFOLLOWED: &str = "stock runs this computer did not follow may have cut";

/// Why a probe cannot find a work zero from a spot of a stock
/// `HISTORY_UNFOLLOWED` cut.
pub(crate) const HISTORY_UNFOLLOWED_WHY: &str = "Runs this computer did not follow may have cut the stock, and the check does not know what they cut here.";

/// Why Zero Z on the stock top and Probe Z cannot set Z where
/// `HISTORY_UNFOLLOWED` cut (`Run::partway_blocked`).
pub(crate) const HISTORY_UNFOLLOWED_BLOCKED: &str = "Runs this computer did not follow may have cut the stock, so the check cannot tell the surface where the stock top is touched off.";

/// What was taken as cut before the project changed, said as `Partly::cut`
/// (`Run::declared_lost`, LIO-583 review).
pub(crate) const DECLARED_LOST: &str = "stock taken as cut before the project changed, some of which the check may no longer take as cut";

/// Why a probe cannot find a work zero from a spot while
/// `Run::declared_lost`.
pub(crate) const DECLARED_LOST_WHY: &str = "Operations or steps of this setup were taken as cut before the project changed, and the check may not take all of them as cut now.";

/// Why Zero Z on the stock top and Probe Z cannot set Z while
/// `Run::declared_lost`.
const DECLARED_LOST_BLOCKED: &str = "Operations or steps of this setup were taken as cut before the project changed, and the check may not take all of them as cut now, so it cannot tell the surface where the stock top is touched off.";

/// What other setups' runs cut this session, said as `Partly::cut`, when
/// the check does not simulate it before this setup as it was cut
/// (`Run::earlier_unfollowed`).
pub(crate) const EARLIER_UNFOLLOWED: &str =
    "stock another setup's run cut earlier, which the check does not simulate as it was cut";

/// The way out of another setup's cut on the tablet (LIO-635): the
/// computer's New stock loaded, then a new sync.
macro_rules! new_stock_on_the_computer {
    () => {
        " If new stock is held, press New stock loaded on the computer (Control page, Work zero tab, the machine connected and a setup of this project picked in the run bar), then Sync projects here, or save the project for the tablet again."
    };
}

/// How one of this project's setups cut the part in a way the check does
/// not simulate before this setup (`Run::earlier_unfollowed_how`): what a
/// probe and Zero Z on the stock top say of a stock `EARLIER_UNFOLLOWED`
/// cut, the setup named by its place in the plan, so the operator need
/// not look for a change that is not there (LIO-599). Another project's
/// setups are said by `Run::earlier_elsewhere`, and ones whose project is
/// not known by `EARLIER_ELSEWHERE_BLOCKED` (LIO-607, LIO-635). The tablet
/// says the same (axis-shop job.ts).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EarlierUnfollowed {
    /// Its setup comes before this one in the plan (its index), and an
    /// operation it cut is disabled, changed or deleted since.
    Changed(usize),
    /// Its setup comes before this one in the plan (its index), and its
    /// runs cut an operation before one that comes ahead of it in the
    /// setup, with that one's material still there (LIO-599 review).
    OutOfOrder(usize),
    /// Its setup comes after this one in the plan (its index): cut first,
    /// or moved after it since.
    After(usize),
    /// Its setup was this project's and is deleted since (`Run::earlier_own`).
    Deleted,
    /// The check gave no plan to tell which.
    Unknown,
}

impl EarlierUnfollowed {
    fn said(self) -> String {
        match self {
            Self::Changed(i) => format!(
                "Setup {}'s run cut this part with an operation disabled, changed or deleted since",
                i + 1
            ),
            Self::OutOfOrder(i) => format!(
                "Setup {}'s run cut its operations out of the plan's order",
                i + 1
            ),
            Self::After(i) => format!(
                "Setup {} comes after this one but its run cut this part first",
                i + 1
            ),
            Self::Deleted => "A setup deleted since cut this part".into(),
            Self::Unknown => {
                "Another setup's run cut this part earlier in a way the check does not follow"
                    .into()
            }
        }
    }

    /// What the Work zero tab says of it, with New stock loaded.
    pub fn said_whole(self) -> String {
        format!("{}.", self.said())
    }

    /// Why a probe cannot find the work zero of the setup to run at its own
    /// spot (`Connection::cut_probe_blocked`).
    pub fn probe_blocked(self) -> String {
        format!(
            "{}, so the check does not know the stock where a probe finds this work zero.",
            self.said()
        )
    }

    /// Why a probe cannot find a work zero from a spot of the stock it cut.
    pub fn why(self) -> String {
        format!(
            "{}, so the check does not know what it cut here.",
            self.said()
        )
    }

    /// Why Zero Z on the stock top and Probe Z cannot set Z on the stock it
    /// cut (`Run::partway_blocked`).
    pub fn blocked(self) -> String {
        format!(
            "{}, so the check cannot tell the surface where the stock top is touched off. If new stock is held, press New stock loaded under Work zero.",
            self.said()
        )
    }

    /// `said` as the tablet file says it (`FOR_TABLET`, LIO-590): cut on
    /// the computer before the file was made.
    pub(crate) fn said_on_tablet(self) -> String {
        const BEFORE: &str = "before this tablet file was made";
        match self {
            Self::Changed(i) => format!(
                "Setup {}'s run on the computer cut this part {BEFORE}, with an operation disabled, changed or deleted since",
                i + 1
            ),
            Self::OutOfOrder(i) => format!(
                "Setup {}'s run on the computer cut its operations out of the plan's order, {BEFORE}",
                i + 1
            ),
            Self::After(i) => format!(
                "Setup {} comes after this one but its run on the computer cut this part first, {BEFORE}",
                i + 1
            ),
            Self::Deleted => {
                format!("A setup deleted since cut this part on the computer {BEFORE}")
            }
            Self::Unknown => format!(
                "Another setup's run on the computer cut this part {BEFORE}, in a way the plan does not follow"
            ),
        }
    }

    /// `blocked` as the tablet file says it, with the way out on the
    /// computer (LIO-635).
    pub fn tablet_blocked(self) -> String {
        format!(
            "{}, so the computer cannot tell the surface where the stock top is touched off.{}",
            self.said_on_tablet(),
            new_stock_on_the_computer!()
        )
    }

    /// `why` as the tablet file says it, with the way out on the computer.
    pub fn tablet_why(self) -> String {
        format!(
            "{}, so the computer does not know what it cut here.{}",
            self.said_on_tablet(),
            new_stock_on_the_computer!()
        )
    }
}

/// Other projects whose runs cut, each its title and the names of the
/// operations that cut (`Run::earlier_elsewhere_projects`, LIO-607).
type ProjectsCut<'a> = Vec<(&'a str, Vec<&'a str>)>;

/// Why a probe cannot find a work zero from a spot of a stock
/// `EARLIER_UNFOLLOWED` cut by setups this project does not have alone
/// (`Run::earlier_elsewhere`, LIO-635).
const EARLIER_ELSEWHERE_WHY: &str = "This computer's runs cut stock for a setup this project does not have, another project's or one deleted since, and the check cannot tell whether that stock is the one held.";

/// Why Zero Z on the stock top and Probe Z cannot set Z where
/// `EARLIER_UNFOLLOWED` cut by setups this project does not have alone
/// (`Run::earlier_elsewhere`, LIO-635).
const EARLIER_ELSEWHERE_BLOCKED: &str = "This computer's runs cut stock for a setup this project does not have, another project's or one deleted since, and the check cannot tell whether that stock is the one held, so it cannot tell the surface where the stock top is touched off. If new stock is held, press New stock loaded under Work zero.";

/// Why a probe cannot find a work zero from a spot of a stock
/// `EARLIER_UNFOLLOWED` cut, where another setup's run cut an operation only
/// in part (`Run::earlier_partway`, LIO-601).
pub(crate) const EARLIER_PARTWAY_WHY: &str = "Another setup's run left an operation of this part unfinished earlier (its run stopped partway through it, or where it stopped is not known), and the check takes that operation as cut whole, so it does not know what is left here.";

/// Why Zero Z on the stock top and Probe Z cannot set Z where another
/// setup's run cut an operation only in part (`Run::earlier_partway`,
/// LIO-601).
pub(crate) const EARLIER_PARTWAY_BLOCKED: &str = "Another setup's run left an operation of this part unfinished earlier (its run stopped partway through it, or where it stopped is not known), and the check takes that operation as cut whole, so it cannot tell the surface where the stock top is touched off. Run that operation to its end first, in a run of its setup. If new stock is held, press New stock loaded under Work zero.";

/// The computer's reasons for this session's cuts in the tablet file's
/// words (LIO-590): the tablet has no Same part or New stock loaded, nor
/// the computer's questions after a disconnect (LIO-595), and "this
/// session" and "this computer" are the computer's, before the file was
/// made.
const FOR_TABLET: [(&str, &str); 9] = [
    (
        STOCK_UNCONFIRMED_BLOCKED,
        "This setup's stock was cut, or taken as cut, on the computer before this tablet file was made, and the tablet cannot tell whether that part is still held, so it cannot tell the surface where the stock top is touched off.",
    ),
    (
        STOCK_UNCONFIRMED_WHY,
        "This setup's stock was cut, or taken as cut, on the computer before this tablet file was made, and the tablet cannot tell whether that part is still held.",
    ),
    (
        STOCK_AWAY_BLOCKED,
        "Runs on the computer cut the stock before this tablet file was made, and the machine was disconnected from it or AXIS CAM closed since: the tablet cannot tell whether that part is still held, or what else ran on the machine, so it cannot tell the surface where the stock top is touched off.",
    ),
    (
        HISTORY_UNFOLLOWED_BLOCKED,
        "Runs the computer did not follow may have cut the stock before this tablet file was made, so it cannot tell the surface where the stock top is touched off.",
    ),
    (
        HISTORY_UNFOLLOWED_WHY,
        "Runs the computer did not follow may have cut the stock before this tablet file was made, and it does not know what they cut here.",
    ),
    (
        EARLIER_ELSEWHERE_BLOCKED,
        concat!(
            "Runs on the computer cut stock for a setup this project does not have, another project's or one deleted since, before this tablet file was made, and the tablet cannot tell whether that stock is the one held, so it cannot tell the surface where the stock top is touched off.",
            new_stock_on_the_computer!()
        ),
    ),
    (
        EARLIER_ELSEWHERE_WHY,
        concat!(
            "Runs on the computer cut stock for a setup this project does not have, another project's or one deleted since, before this tablet file was made, and the tablet cannot tell whether that stock is the one held.",
            new_stock_on_the_computer!()
        ),
    ),
    (
        EARLIER_PARTWAY_BLOCKED,
        concat!(
            "Another setup's run on the computer left an operation of this part unfinished before this tablet file was made (its run stopped partway through it, or where it stopped is not known), and the plan takes that operation as cut whole, so the computer cannot tell the surface where the stock top is touched off. Run that operation to its end first, in a run of its setup on the computer, then Sync projects here, or save the project for the tablet again.",
            new_stock_on_the_computer!()
        ),
    ),
    (
        EARLIER_PARTWAY_WHY,
        concat!(
            "Another setup's run on the computer left an operation of this part unfinished before this tablet file was made (its run stopped partway through it, or where it stopped is not known), and the plan takes that operation as cut whole, so it does not know what is left here.",
            new_stock_on_the_computer!()
        ),
    ),
];

/// `why`, from the Run panel, as the tablet file says it (`FOR_TABLET`).
fn for_tablet(why: String) -> String {
    (FOR_TABLET.iter())
        .find(|(computer, _)| *computer == why)
        .map_or(why, |(_, tablet)| tablet.to_string())
}

/// What each setup's runs cut, by setup id (`App::run_cuts`).
pub type SessionCuts = [(String, Vec<CutInRun>)];

/// What the tablet file is made with (`App::session_cuts`, LIO-590): what
/// each setup's runs cut (`App::run_cuts`, with the run picked now),
/// whether what runs cut was lost (`App::others_unknown`, LIO-594), which
/// makes a setup with none kept cut by runs no session here knows of, and
/// what the operator took as cut in each setup's run (`App::run_declared`,
/// LIO-583), the run picked now's included: a later setup takes an
/// operation marked Already cut as cut while it is so (LIO-603); and whose
/// each setup of `cuts` is (`App::cut_owners`), so that another project's
/// cuts are said as that project's, named (LIO-638).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Session {
    pub cuts: Vec<(String, Vec<CutInRun>)>,
    pub lost: bool,
    pub declared: Vec<KeptDeclared>,
    pub owners: Vec<CutOwner>,
}

impl Run {
    /// Given `session` as the tablet file's work zeros go by it (LIO-590):
    /// as a pick of the setup on the Run panel (`picked_in`); and with its
    /// operations or steps taken as cut on the computer (`declared`), whose
    /// material may be gone though nothing here says where, its stock is
    /// not the setup's own and the part held is not known to be that one
    /// (`stock_unconfirmed`), as on the Run panel once picked again
    /// (LIO-583), which the tablet cannot answer: no Z from the stock top.
    /// Whose the setups not `doc`'s are, as the Run panel's pick has it
    /// (`give_elsewhere`, LIO-638).
    pub fn given(&mut self, session: &Session, doc: &Document) {
        self.picked_in(&session.cuts, session.lost, &session.declared);
        give_elsewhere(&session.owners, doc, self);
        if session.declared.iter().any(|k| k.setup == self.setup_id) {
            if self.cut_in_run.is_empty() {
                self.note_history_unknown();
            }
            self.stock_unconfirmed = true;
        }
    }
}

/// What a tool run of a run cut (`Run::cut_in_run`): the operation (index in
/// the setup), all of it (None) or its steps before the one given, and the
/// print of the plan and cutter it cut (`OpSteps::cut_print`). Steps before
/// one are a bound: the last of them may be cut only in part, after a stop
/// in it, and `MAY_HAVE_CUT_ALL` is anywhere up to all of it. Last, the
/// print of the plan it was last cut whole with (None steps): the stock has
/// at least that cut, whatever else cut it, so a later setup's programs,
/// made for it cut so, find no more material than they were made for
/// (`Run::earlier_not_whole`, LIO-602).
pub type CutInRun = (usize, Option<usize>, Option<u64>, Option<u64>);

/// `CutInRun`'s steps of an operation a tool run that did not finish ran,
/// with nothing to tell how far: stopped where the run cannot place it, or
/// dropped while it ran (`Run::cuts_so_far`). Any of it may be cut, all of
/// it or only some (LIO-601): its own run takes it as all (`Run::cut_beyond`
/// as for None), a later setup's as not cut whole (`Run::earlier_partway`).
pub const MAY_HAVE_CUT_ALL: Option<usize> = Some(usize::MAX);

/// Notes in `cuts` (`Run::cut_in_run`) that operation `oi` was cut: all of
/// it (None) or its steps before `steps`, from the plan and cutter `print`.
pub(crate) fn note_cut_in(
    cuts: &mut Vec<CutInRun>,
    oi: usize,
    steps: Option<usize>,
    print: Option<u64>,
) {
    note_cut_whole(cuts, oi, steps, print, print.filter(|_| steps.is_none()));
}

/// `note_cut_in`, with the plan it was certainly cut whole with, `whole`,
/// given: a record's own, merged into another (`Run::cuts_if_started`),
/// keeps what it says (LIO-602 review).
fn note_cut_whole(
    cuts: &mut Vec<CutInRun>,
    oi: usize,
    steps: Option<usize>,
    print: Option<u64>,
    whole: Option<u64>,
) {
    match cuts.iter_mut().find(|c| c.0 == oi) {
        Some(c) => {
            c.1 = match (c.1, steps) {
                (Some(a), Some(b)) => Some(a.max(b)),
                _ => None,
            };
            // Cut from two plans: neither is all it cut.
            c.2 = c.2.filter(|&p| print == Some(p));
            // More cuts after one whole take only more away (LIO-602).
            c.3 = whole.or(c.3);
        }
        None => cuts.push((oi, steps, print, whole)),
    }
}

/// `CutHistory::cuts` as kept on disk: each cut as `CutInRun`, or as it was
/// kept before the plan last cut whole was (LIO-602), which counts as not
/// known: before LIO-601 a tool run stopped where it was not placed was
/// noted as whole too.
fn kept_cuts<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Vec<(String, Vec<CutInRun>)>, D::Error> {
    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum Kept {
        Now(usize, Option<usize>, Option<u64>, Option<u64>),
        Before(usize, Option<usize>, Option<u64>),
    }
    let kept: Vec<(String, Vec<Kept>)> = serde::Deserialize::deserialize(d)?;
    Ok((kept.into_iter())
        .map(|(id, cuts)| {
            let cuts = (cuts.into_iter())
                .map(|c| match c {
                    Kept::Now(oi, steps, print, whole) => (oi, steps, print, whole),
                    Kept::Before(oi, steps, print) => (oi, steps, print, None),
                })
                .collect();
            (id, cuts)
        })
        .collect())
}

/// Takes into `into`, a setup's record of what runs cut here, `theirs`,
/// what another AXIS CAM on the same folder kept for it, `base` what was
/// kept when this one last read or wrote it. Unchanged here since, theirs
/// is the newer: taken as it is. Changed by both, an operation one holds
/// cut whole and the other in part may be either (`MAY_HAVE_CUT_ALL`):
/// whole is no longer the safe side, partway no longer the only one, since
/// a later setup trusts a whole one (LIO-601 review).
pub(crate) fn take_kept_cuts(
    into: &mut Vec<CutInRun>,
    base: Option<&Vec<CutInRun>>,
    theirs: &[CutInRun],
) {
    if base.is_some_and(|b| b == into) {
        *into = theirs.to_vec();
        return;
    }
    for &(oi, steps, print, whole) in theirs {
        match into.iter_mut().find(|c| c.0 == oi) {
            Some(c) => {
                c.1 = match (c.1, steps) {
                    (None, None) => None,
                    (Some(a), Some(b)) => Some(a.max(b)),
                    _ => MAY_HAVE_CUT_ALL,
                };
                c.2 = c.2.filter(|&p| print == Some(p));
                // Which plan cut it whole last is known only where both
                // agree (LIO-602).
                c.3 = c.3.filter(|&w| whole == Some(w));
            }
            None => into.push((oi, steps, print, whole)),
        }
    }
}

/// Notes in a record of what runs cut (`Run::cut_in_run`, `App::run_cuts`)
/// that something else may have cut the part since (`HISTORY_UNKNOWN`,
/// LIO-596). An empty record says nothing was cut and stays so. True when
/// it was not already unknown.
pub(crate) fn note_unknown(cuts: &mut Vec<CutInRun>) -> bool {
    !cuts.is_empty() && note_unknown_before(cuts)
}

/// As `note_unknown`, for what was cut before a tool run that runs or ran
/// (`Run::before_follow`, `App::run_pending`), empty or not: that tool run
/// cuts the part, and its outcome starts from this (LIO-596 review).
pub(crate) fn note_unknown_before(cuts: &mut Vec<CutInRun>) -> bool {
    if cuts.iter().any(|c| c.0 == HISTORY_UNKNOWN) {
        return false;
    }
    note_cut_in(cuts, HISTORY_UNKNOWN, None, None);
    true
}

/// What follows from operations of earlier setups not cut
/// (`Run::earlier_uncut_ways`): for Zero Z and Probe Z, and for Cycle Start
/// (LIO-602).
const TOUCH_OFF_UNKNOWN: &str =
    ", so it cannot tell the surface where the stock top is touched off.";
const RAPIDS_THROUGH: &str =
    ", so this setup's programs, made for {them} cut, could send rapids through the material left.";

/// What this run's own tool runs cut, said as `Partly::cut`, when the
/// check's stock before the start does not have it: one runs, or one at or
/// after the start was cut (`Run::partly`, LIO-582 review).
const RUN_UNFOLLOWED: &str =
    "stock this run's tool runs cut since, which the check does not follow";

/// Why a probe cannot find a work zero from a spot of a stock
/// `RUN_UNFOLLOWED` cut.
const RUN_UNFOLLOWED_WHY: &str = "This run's tool runs have cut the stock since, and the check does not follow what they cut here.";

/// Why Zero Z on the stock top and Probe Z cannot set Z where
/// `RUN_UNFOLLOWED` cut (`Run::partway_blocked`).
const RUN_UNFOLLOWED_BLOCKED: &str = "This run's tool runs have cut the stock since, and the check does not follow what they cut, so it cannot tell the surface where the stock top is touched off.";

/// What a start at a step whose stock's surface the check cannot tell
/// (`Run::partway_blocked`) does for its work zero: kept, or Z set by hand
/// (LIO-586 review).
pub(crate) const STEP_ZERO_BY_HAND: &str = "keep the work zero they were cut with (Keep it, under Start at a step), or set Z by hand on a surface whose height you know (Set under Work zero)";

/// Why Zero Z on the stock top and Probe Z cannot set Z for a run from the
/// setup's start (`Run::inherited_unknown`).
const INHERITED_UNKNOWN: &str = "Earlier setups may have cut the stock under the stock top where it is touched off, so the check cannot tell its surface there.";

/// Why Zero Z on the stock top and Probe Z cannot set Z where
/// `LATER_UNFOLLOWED` cut (`Run::partway_blocked`).
const LATER_UNFOLLOWED_BLOCKED: &str = "Operations or steps after the start are taken as cut, and the check does not follow the stock they cut, so it cannot tell the surface where the stock top is touched off.";

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
    /// What this run's tool runs have cut, by operation (index in the
    /// setup): all of it (None), or its steps before the one given, and
    /// the print of the plan and cutter it cut (`OpSteps::cut_print`; None when
    /// the check gave none, or it cut more than one). Kept over the starts
    /// picked, the edits and the checks, which take what is before a start
    /// as cut but know nothing of what was cut at or after it, nor of an
    /// operation since disabled or changed (`Run::partly`, LIO-582 review).
    pub cut_in_run: Vec<CutInRun>,
    /// `cut_in_run` came from an earlier pick of this setup, this session
    /// or before AXIS CAM was last closed (`CutHistory`, LIO-594), or
    /// tells of runs whose cuts are not known (`HISTORY_UNKNOWN`), or what
    /// was taken as cut came from an earlier pick, this session or before
    /// AXIS CAM was last closed (`declared`, LIO-583 review, LIO-611), and the
    /// operator has not said whether the same part is still held
    /// (`Action::SamePart`, `Action::NewStock`): nothing sets Z or probes
    /// until then (`partly`, LIO-582 review).
    pub stock_unconfirmed: bool,
    /// `cut_in_run`, or what was taken as cut (`declared`, LIO-606), was
    /// kept across a disconnect from the machine, while which anything may
    /// have run on it (LIO-595): the operator is asked too whether
    /// something else ran meanwhile (`Action::SamePart`,
    /// `Action::OtherRunsMayHaveCut`). Cleared once answered.
    pub away: Option<Away>,
    /// Operations or steps were taken as cut (`declared`) and dropped since:
    /// by a project edit (`unskip`), or a check that refused them
    /// (`checked_as`). The check no longer takes their material as cut, so
    /// it cannot tell the surface the stock top is touched off on: nothing
    /// sets Z or probes, and Cycle Start waits for the work zero, until New
    /// stock loaded (`new_stock`, LIO-583 review). Taking operations or
    /// steps as cut again keeps it, as what was dropped may not be what is
    /// taken as cut again: the words say "may not all be taken as cut now"
    /// (LIO-621).
    pub declared_lost: bool,
    /// Its skips came with a tool run it follows (`following`) that was
    /// sent before a project edit the session noted (`App::run_declared`
    /// lost): they say what was sent, by indexes the edit may have changed,
    /// so the session keeps none of them as taken as cut (`declared`), only
    /// that something was (`declared_lost`). An edit or New stock loaded,
    /// which drop them, ends it (LIO-583 review).
    pub declared_stale: bool,
    /// What was taken as cut (`skips`, `cut_steps`) was given back from what
    /// the session kept (`App::run_declared`), from before AXIS CAM was
    /// last closed too (`CutHistory::declared`), with the print of each
    /// operation it names as it was taken as cut (`KeptDeclared::prints`).
    /// The first check that answers holds it only while its prints are the
    /// same: else the project, the part or AXIS CAM changed since, and it is
    /// dropped as a project edit drops it (`declared_lost`, LIO-611).
    pub declared_from: Option<Vec<(usize, Option<u64>)>>,
    /// What the other setups cut this session (`App::run_cuts`), by setup
    /// id: in the part whether those setups come before it, were deleted or
    /// moved after it since. The check's stock has it only while it
    /// simulates each such operation, before this setup, as it was cut,
    /// and whole: it has no stock for one cut partway (`earlier_unfollowed`,
    /// LIO-582 review, LIO-601).
    pub earlier_cuts: Vec<(String, Vec<CutInRun>)>,
    /// The setups `earlier_cuts` was given for: one no longer in it was
    /// forgotten (New stock loaded).
    pub earlier_given: Vec<String>,
    /// Of the setups of `earlier_cuts`, those of another project, whose
    /// (`CutOwner`, LIO-607): their cuts are said as that project's.
    pub earlier_elsewhere: Vec<CutOwner>,
    /// The ids of the project's setups, as `follow` last found them: a cut
    /// in `earlier_cuts` by none of them, whose owner is not known, may be
    /// another project's, or a deleted setup's (`earlier_not_here`,
    /// LIO-635).
    pub(crate) project_setups: Vec<String>,
    /// Of the setups of `earlier_cuts`, those kept as this project's though
    /// it no longer has them: deleted since (`CutOwner`, LIO-607), not
    /// another project's.
    pub(crate) earlier_own: Vec<String>,
    /// The operations each other setup's run has marked Already cut now
    /// (`Skip::Cut`, `App::run_declared`), by setup id, each with the print
    /// of its plan, steps and cutter as it was marked
    /// (`KeptDeclared::prints`): taken as cut by `earlier_uncut` while
    /// marked so and that plan's, never kept with what was cut, for a mark
    /// can be undone (LIO-603 review).
    pub earlier_taken: Vec<(String, Marks)>,
    /// A run following on the Pi the tool run its run ran when dropped
    /// (`App::run_pending`): what was cut before that tool run. Kept only
    /// once the follow holds (`follow_again`) and until this computer starts
    /// a tool run, so that only that tool run's outcome starts from this
    /// instead of `cut_in_run`'s bound of it (LIO-582 review).
    pub before_follow: Option<Vec<CutInRun>>,
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
    /// Z touched off on the Control page since the last tool run started
    /// (`z_touched`): it holds for its tool only (`touched`, LIO-619), and
    /// until another start is picked or the setup checked again (LIO-618).
    touched_for: Option<TouchedFor>,
    /// A check of the setup has answered since it was picked.
    answered: bool,
    /// The setup's work system the last check that answered found
    /// (`Run::wcs`), kept while it is checked again (LIO-643).
    known_wcs: Option<usize>,
    /// The machine's active work system the reference was recorded in: a
    /// check that finds the setup in another forgets it (LIO-643).
    reference_wcs: Option<usize>,
    /// While the setup is checked again: the tool of the tool run the check
    /// before found next (`check_again`), what a touch-off is for meanwhile.
    rechecked_tool: Option<String>,
    /// How many times a touch-off was forgotten for the start
    /// (`forget_touch`): a Z set pressed before it and shown after counts
    /// for no start since (`Connection::z_set_check`, LIO-618).
    starts: u64,
    /// How the last tool run ended (ok, the driver's message).
    pub outcome: Option<(bool, String)>,
    /// The stock after each of the setup's operations, by its index in the
    /// setup, pictured for the Tool runs panel (`op_pictures`): empty until
    /// worked out after the check.
    pub pictures: Vec<(usize, iced::widget::image::Handle)>,
    /// The stock the check left, until it is pictured (`App::picture_run`).
    pub left: Option<Arc<Left>>,
    /// The operations of the setups before it the check simulated
    /// (`Left::earlier`), kept once `left` is pictured (`earlier_unfollowed`,
    /// LIO-601 review).
    simulated_earlier: Vec<EarlierOp>,
    /// The plan's setups by id, in order, as checked (`Left::plan`), kept
    /// with `simulated_earlier`: where a setup whose cut the check does not
    /// follow is in the plan (`earlier_unfollowed_how`, LIO-599).
    plan_setups: Vec<String>,
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
    /// The start the operator picked (Start here, Next on a tool run, Start
    /// at a step: `pick_start`, `start_step`): its operation (index in the
    /// setup) and step (0: the operation's start). Not one the run came to
    /// by itself: the first operation sent, past those skipped, or where a
    /// run stopped. New stock loaded keeps only such a start (`new_stock`,
    /// LIO-633).
    pub start_picked: Option<(usize, usize)>,
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
    /// `journal` is one this session wrote, unchanged since (`read_own`):
    /// what its runs cut is in `cut_in_run`. Else another session's.
    pub journal_own: bool,
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
    /// The last check's answer that passed, kept while the next check runs
    /// (`programs` is None then) and over a refused one: what the next
    /// check that passes compares the start's tool run with (`checked_as`,
    /// LIO-636).
    last_passed: Option<Arc<Result<Vec<Checked>, Vec<String>>>>,
    /// Its operations by id (`Left::ops`; empty when it gave none).
    last_passed_ops: Vec<String>,
    /// A check moved a start partway into a tool run back to its first
    /// operation, its operations having changed: the operation that was
    /// picked (`op_key`, name), until a start is picked or Cycle Start
    /// sends a tool run (`start_moved`).
    start_moved: Option<(String, String)>,
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
        Some(Self::read_own(path)?.0)
    }

    /// `read`, and whether it is as this session wrote it (`read_own`).
    pub fn read_own(path: &std::path::Path) -> Option<(Self, bool)> {
        let (data, own) = read_own(path)?;
        Some((serde_json::from_slice(&data).ok()?, own))
    }

    pub fn write(&self, path: &std::path::Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let data = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        crate::storage::write_atomic(path, &data)
    }

    /// `write`, as this session's (`write_own`): read back unchanged, it
    /// is this session's (`read_own`).
    pub fn write_own(&self, path: &std::path::Path) -> Result<(), String> {
        let data = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        write_own(path, &data)
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

/// What each setup's runs cut, kept on disk beside the machines' journals
/// (`uc100/cuts.json`, `App::run_cuts`), so that it survives closing AXIS
/// CAM (LIO-594): a setup picked again in a new session takes it as this
/// session's, and asks whether the same part is still held
/// (`Run::stock_unconfirmed`). Written as it changes: a tool run as it
/// starts counts as cutting all it runs (`Run::cuts_so_far`), so a session
/// that ends mid-run, AXIS CAM killed say, has kept no less than it cut.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CutHistory {
    /// By setup id, what its runs cut (`Run::cut_in_run`). Kept empty for
    /// a setup whose runs cut nothing since New stock loaded only while
    /// `others_unknown`.
    #[serde(deserialize_with = "kept_cuts")]
    pub cuts: Vec<(String, Vec<CutInRun>)>,
    /// The journals of where runs stopped as the sessions that kept `cuts`
    /// wrote them (`KeptJournal`).
    #[serde(default)]
    pub journals: Vec<KeptJournal>,
    /// What runs cut was lost (this file could not be read): a setup with
    /// no entry in `cuts` was cut by runs no session here knows of
    /// (`HISTORY_UNKNOWN`), until New stock loaded for it.
    #[serde(default)]
    pub others_unknown: bool,
    /// The tool runs this computer sent a Pi (`control::sent_tool_runs`),
    /// the latest `SENT_KEPT`: each kept as cut here before it was sent,
    /// none ran while it was away (LIO-595), whichever session sent it.
    #[serde(default)]
    pub sent: Vec<u64>,
    /// Whose each setup in `cuts` is (`CutOwner`), so that another
    /// project's cuts are said to be so (LIO-607). A setup with none, kept
    /// by an AXIS CAM before it, is said as any setup the check does not
    /// follow.
    #[serde(default)]
    pub owners: Vec<CutOwner>,
    /// What the operator took as cut in each setup's run, by setup
    /// (`App::run_declared`), as the session kept it: absent from a file
    /// written before it was kept, which loads with none (LIO-611).
    #[serde(default)]
    pub declared: Vec<KeptDeclared>,
    /// The Pi's start and count of control takes as last heard
    /// (`App::pi_control_seen`, LIO-597): opened again, a new start or a
    /// take this computer did not make since means something else may
    /// have cut the part while AXIS CAM was closed, by a jog or a plain job
    /// that leaves no tool run. None from an AXIS CAM before it, or before
    /// any Pi was heard.
    #[serde(default)]
    pub pi_control: Option<PiControl>,
}

/// The Pi's start and count of control takes (`CutHistory::pi_control`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PiControl {
    pub boot: u64,
    pub takes: u64,
}

/// The project and names of a setup whose runs cut (`CutHistory::owners`),
/// as they were when its cuts were last kept (LIO-607).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CutOwner {
    pub setup: String,
    /// The project's identity (`Document::project_id`) and title.
    pub project: String,
    pub title: String,
    /// The names of the setup's operations, by index.
    pub ops: Vec<String>,
}

impl CutOwner {
    /// Setup `si` of `doc`'s.
    pub fn of(doc: &Document, si: usize) -> Self {
        let s = &doc.setups[si];
        Self {
            setup: s.id.clone(),
            project: doc.project_id.clone(),
            title: doc.title.clone(),
            ops: s.operations.iter().map(|o| o.name().to_string()).collect(),
        }
    }
}

/// Gives `run` whose the setups of its `earlier_cuts` that are another
/// project's are (`owners`, `App::cut_owners`; `Run::earlier_elsewhere`),
/// so that their cuts are said as that project's (LIO-607). One `doc` has,
/// or one deleted from it (`Run::earlier_own`), is this project's; one
/// with no owner kept, of a project not known (`Run::earlier_not_here`,
/// LIO-635). Only what is said goes by it: what the check follows does not.
pub fn give_elsewhere(owners: &[CutOwner], doc: &Document, run: &mut Run) {
    let gone = (run.earlier_cuts.iter())
        .filter(|(id, _)| !doc.setups.iter().any(|s| s.id == *id))
        .filter_map(|(id, _)| owners.iter().find(|o| o.setup == *id));
    let (own, elsewhere): (Vec<&CutOwner>, Vec<&CutOwner>) =
        gone.partition(|o| o.project == doc.project_id);
    run.earlier_elsewhere = elsewhere.into_iter().cloned().collect();
    // Deleted from it since: not another project's (LIO-635).
    run.earlier_own = own.into_iter().map(|o| o.setup.clone()).collect();
}

/// How many of the tool runs sent a Pi `CutHistory::sent` keeps.
pub const SENT_KEPT: usize = 1000;

/// A journal (`Journal`) as the session that kept what its run cut
/// (`CutHistory::cuts`) wrote it: the setup's, in the machine's journal
/// file named `file` (`Connection::journal_name`), its very text. Read back
/// as it is, it tells of runs that history has; any other journal there,
/// or none where this one was, of runs it may not have (another session's
/// that kept none, an older AXIS CAM's), whose cuts are not known
/// (`App::journal_known`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct KeptJournal {
    pub setup: String,
    pub file: String,
    pub text: String,
}

impl CutHistory {
    /// The history kept at `path`: Ok(None) when there is none yet, Err
    /// when there is one that cannot be read.
    pub fn read(path: &std::path::Path) -> Result<Option<Self>, String> {
        match std::fs::read(path) {
            Ok(data) => serde_json::from_slice(&data)
                .map(Some)
                .map_err(|e| e.to_string()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    pub fn write(&self, path: &std::path::Path) -> Result<(), String> {
        #[cfg(test)]
        if FAIL_CUT_WRITES.with(|f| f.get()) {
            return Err("a write refused for the test".into());
        }
        let data = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        crate::storage::write_atomic(path, &data)
    }
}

#[cfg(test)]
thread_local! {
    /// Tests: `CutHistory::write` fails.
    pub(crate) static FAIL_CUT_WRITES: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Seconds since 1970, now.
pub fn now() -> u64 {
    (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)).map_or(0, |d| d.as_secs())
}

/// One version of a file, as its metadata tells it from any other: on
/// Unix its device and inode and the times it was modified and changed.
/// The change time is the system's, set on every write and every time
/// set, so a copy put back with its modified time (`cp -p`) is not the
/// version it was copied from. None where it cannot be told (not Unix):
/// no file is this session's there.
#[derive(Debug, Clone, PartialEq)]
struct Version {
    dev: u64,
    ino: u64,
    modified: (i64, i64),
    changed: (i64, i64),
}

fn version(m: &std::fs::Metadata) -> Option<Version> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Some(Version {
            dev: m.dev(),
            ino: m.ino(),
            modified: (m.mtime(), m.mtime_nsec()),
            changed: (m.ctime(), m.ctime_nsec()),
        })
    }
    #[cfg(not(unix))]
    {
        let _ = m;
        None
    }
}

thread_local! {
    /// The versions of the files this session wrote, on the thread that
    /// runs the app (`write_own`): a journal read back as one of them is this
    /// session's; any other, whatever it holds or when, another session's,
    /// which alone knew what its runs cut (`Run::take_journal`, LIO-586
    /// review).
    static OWN_FILES: std::cell::RefCell<Vec<(std::path::PathBuf, Version)>> =
        const { std::cell::RefCell::new(vec![]) };
}

/// Reads `path` to the end, and the version of the very file read,
/// taken from it once read, so that one changed meanwhile is not the one
/// it was before.
fn read_version(path: &std::path::Path) -> Option<(Vec<u8>, Option<Version>)> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).ok()?;
    let mut data = vec![];
    file.read_to_end(&mut data).ok()?;
    Some((data, file.metadata().ok().as_ref().and_then(version)))
}

/// Tests: what another process does to the file `write_own` just renamed
/// into place, at each point it looks at it again (0: before it takes its
/// version from the file it made; 1: before it reads it back).
#[cfg(test)]
type Meanwhile = Box<dyn Fn(usize, &std::path::Path)>;

#[cfg(test)]
thread_local! {
    static MEANWHILE: std::cell::RefCell<Option<Meanwhile>> = std::cell::RefCell::new(None);
}

/// Writes `data` to `path` as `storage::write_atomic` does (a new file,
/// renamed there), and notes the version this session wrote (`read_own`)
/// only when nothing else wrote it since (LIO-586 review): the file it
/// made, renamed there, still has the modified time it had once written
/// (a rename leaves it; any write changes it), and the file read back at
/// `path` is that very version, its inode and both times, and holds what
/// was written. One put there or written meanwhile, by another session
/// say, is not noted.
pub fn write_own(path: &std::path::Path, data: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let parent = path.parent().ok_or("Missing parent directory")?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let tmp = parent.join(format!(".axis-{}.tmp", uuid::Uuid::new_v4()));
    let written = (|| {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
        let made = f.metadata()?;
        std::fs::rename(&tmp, path)?;
        #[cfg(test)]
        MEANWHILE.with(|m| m.borrow().as_ref().map(|m| m(0, path)));
        let placed = f.metadata()?;
        Ok((version(&made), version(&placed)))
    })()
    .map_err(|e: std::io::Error| e.to_string());
    let (made, placed) = match written {
        Ok(v) => v,
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            return Err(e);
        }
    };
    #[cfg(test)]
    MEANWHILE.with(|m| m.borrow().as_ref().map(|m| m(1, path)));
    if let (Some(made), Some(placed), Some((read, Some(v)))) = (made, placed, read_version(path))
        && (placed.dev, placed.ino, placed.modified) == (made.dev, made.ino, made.modified)
        && v == placed
        && read == data
    {
        OWN_FILES.with(|o| o.borrow_mut().push((path.to_path_buf(), v)));
    }
    Ok(())
}

/// Reads `path`, and whether what was read is a version this session
/// wrote (`write_own`): the version of the very file read, taken from it
/// once read, so that one changed meanwhile is not.
pub fn read_own(path: &std::path::Path) -> Option<(Vec<u8>, bool)> {
    let (data, read) = read_version(path)?;
    let own = read
        .is_some_and(|v| OWN_FILES.with(|o| o.borrow().iter().any(|(p, w)| p == path && *w == v)));
    Some((data, own))
}

/// The file at `path` is taken as this session's no more (`read_own`):
/// a journal New stock loaded could not remove, which is then not known
/// (`App::journal_known`, LIO-634 review).
pub fn disown(path: &std::path::Path) {
    OWN_FILES.with(|o| o.borrow_mut().retain(|(p, _)| p != path));
}

/// Tests: AXIS CAM closed, so the files it wrote are another session's to
/// the one opened next.
#[cfg(test)]
pub(crate) fn forget_own_files() {
    OWN_FILES.with(|o| o.borrow_mut().clear());
}

/// What `Run::cut_in_run` holds for cuts of the setup no session here
/// knows (`Run::note_history_unknown`): an operation after every start,
/// which no check simulates, so that no start goes by the check's stock
/// until New stock loaded, whatever the operations are by then.
pub(crate) const HISTORY_UNKNOWN: usize = usize::MAX;

/// A setup's cuts kept across a disconnect from the machine (`Run::away`,
/// `App::run_away`, LIO-595).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Away {
    /// The tool runs the Pi had, going or ended, when the machine was first
    /// disconnected since, with those this computer sent since: one it
    /// shows that is neither ran while this computer was away
    /// (`control::Connection::same_part_refused`). None when it was not a Pi.
    pub seen: Option<Vec<u64>>,
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
            cut_in_run: vec![],
            stock_unconfirmed: false,
            away: None,
            declared_lost: false,
            declared_stale: false,
            declared_from: None,
            earlier_cuts: vec![],
            earlier_given: vec![],
            earlier_elsewhere: vec![],
            project_setups: doc.setups.iter().map(|s| s.id.clone()).collect(),
            earlier_own: vec![],
            earlier_taken: vec![],
            before_follow: None,
            running: None,
            xy: None,
            zero: None,
            reference: None,
            reference_a: None,
            last: None,
            length: None,
            touched_for: None,
            answered: false,
            known_wcs: None,
            reference_wcs: None,
            rechecked_tool: None,
            starts: 0,
            outcome: None,
            pictures: vec![],
            left: None,
            simulated_earlier: vec![],
            plan_setups: vec![],
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
            start_picked: None,
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
            journal_own: false,
            follow: None,
            followed: None,
            cut_note: None,
            rows_from_skips: false,
            last_passed: None,
            last_passed_ops: vec![],
            start_moved: None,
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
        // What was cut before the tool run it follows is for that tool
        // run's outcome only: kept once the follow holds (LIO-582 review).
        let before = self.before_follow.take();
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
        self.before_follow = before;
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
        self.project_setups = doc.setups.iter().map(|s| s.id.clone()).collect();
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

    /// The tool (`tool_id`) of tool run `i`, as checked.
    fn tool_of(&self, i: usize) -> Option<String> {
        Some(self.checked()?.get(i)?.ops[0].tool_id.clone())
    }

    /// Z was touched off on the Control page since the last tool run
    /// started, for the next tool run's tool: a touch-off holds for its
    /// tool only (LIO-619), and for the start picked then (LIO-618).
    pub fn touched(&self) -> bool {
        let next = self.tool_of(self.next);
        // Made in another work system than the setup's, it set that one's
        // Z (LIO-643).
        let wcs = self.wcs();
        (self.touched_for.as_ref()).is_some_and(|t| {
            t.for_start
                && !t.held
                && next.is_some()
                && t.tool == next
                && t.wcs.is_none_or(|w| wcs.is_none_or(|s| s == w))
        })
    }

    /// The touch-off just counted (`z_touched_for`) was made in the
    /// machine's active work system `wcs` (LIO-643).
    pub fn touched_in(&mut self, wcs: usize) {
        if let Some(t) = &mut self.touched_for {
            t.wcs = Some(wcs);
        }
    }

    /// The tool a touch-off made before the setup's first check answered
    /// is for, once it has: tool run 1's, when the run starts there; else
    /// none known (LIO-619).
    pub fn first_touch_tool(&self) -> Option<String> {
        (self.next == 0 && self.from == 0 && self.at_step().is_none())
            .then(|| self.tool_of(0))
            .flatten()
    }

    /// No check of the setup has answered since it was picked: a touch-off
    /// made now is for the tool the first one finds (`first_touch_tool`).
    pub fn unanswered(&self) -> bool {
        !self.answered
    }

    /// The tool a touch-off now is for (`z_touched`).
    pub fn next_tool(&self) -> Option<String> {
        (self.tool_of(self.next)).or_else(|| self.rechecked_tool.clone())
    }

    /// Z is touched off for the next tool run's tool: Set Z from
    /// reference, Zero Z, Set Z, a probe or a touch-off.
    pub fn z_touched(&mut self) {
        self.z_touched_for(self.next_tool());
    }

    /// A Z set for the next tool is on its way: the touch-off before it no
    /// longer counts for its tool until the machine shows the new one, but
    /// still says whose Z the offset is for the others (LIO-627).
    pub fn hold_touch(&mut self) {
        if let Some(t) = &mut self.touched_for {
            t.held = true;
        }
    }

    /// The setup is checked again, as check `epoch`, keeping its progress:
    /// a touch-off meanwhile is for the tool the check before found next.
    pub fn check_again(&mut self, epoch: u64) {
        self.keep_next_tool();
        // A Z set before is forgotten as the tool's Z kept is: a skip's
        // check may move the start to another tool's tool run, and an edit
        // number them otherwise (LIO-618 review). One set while it checks,
        // after the start was picked, holds for the tool it was for; one
        // set before the first check is tool run 1's (`checked_as`).
        if self.answered {
            self.forget_touch();
        }
        (self.epoch, self.programs, self.stale, self.recheck) = (epoch, None, false, false);
    }

    /// Keeps the next tool run's tool for touch-offs while the setup is
    /// checked again (`rechecked_tool`).
    fn keep_next_tool(&mut self) {
        if let Some(tool) = self.tool_of(self.next) {
            self.rechecked_tool = Some(tool);
        }
    }

    /// Z is touched off for `tool` (None: not known, `TouchedFor::first`).
    pub fn z_touched_for(&mut self, tool: Option<String>) {
        let first = tool.is_none() && !self.answered;
        self.touched_for = Some(TouchedFor {
            tool,
            first,
            z: None,
            held: false,
            for_start: true,
            wcs: None,
        });
    }

    /// A Z set before is forgotten for the start (`touched`), as the tool's
    /// Z kept is: another start picked, or the setup checked again, which
    /// may move it (a skip) or number the tool runs otherwise (an edit,
    /// LIO-618). Whose Z the offset is stays known, so that another tool
    /// next is touched off first (LIO-619).
    pub fn forget_touch(&mut self) {
        self.starts += 1;
        if let Some(t) = &mut self.touched_for {
            t.for_start = false;
        }
    }

    /// At a tool change, Z was set since otherwise than by a Z set on the
    /// Control page (MDI, the tablet, LIO-630): the Z work offset is
    /// neither the one the last tool run started with nor the one the last
    /// Z set here left, so whose Z it is is not known, and the new tool
    /// waits for its own, as on the tablet (LIO-648). None while a Z set
    /// is on its way: Cycle Start waits for the machine to show it.
    pub fn z_elsewhere(&self, c: &Connection) -> bool {
        if self.tool_change().is_none() {
            return false;
        }
        let offset = c.snapshot.work_offset[2];
        let unchanged = |z: f64| (offset - z).abs() < 1e-6;
        (self.last.as_ref()).is_some_and(|(_, z)| !unchanged(*z))
            && !c.z_set_waits(self)
            && !(self.touched_for.as_ref()).is_some_and(|t| t.z.is_none_or(unchanged))
    }

    /// The start a Z set is pressed for (`forget_touch`): another picked
    /// since, or the setup checked again, it counts for none (LIO-618).
    pub fn starts(&self) -> u64 {
        self.starts
    }

    /// The Z work offset the last touch-off left, once shown; none while
    /// a Z set pressed since waits (`hold_touch`).
    #[cfg(test)]
    pub fn touched_z(&self) -> Option<f64> {
        self.touched_for.as_ref().filter(|t| !t.held)?.z
    }

    /// The machine shows everything sent, with Z work offset `z`: what a
    /// touch-off not shown yet left.
    pub fn touch_shown(&mut self, z: f64) {
        if let Some(t) = &mut self.touched_for {
            t.z.get_or_insert(z);
        }
    }

    /// Whether what is taken as cut (`skips`, `cut_steps`) was taken as cut
    /// from the plans and cutters a check found (`now`): each operation it
    /// names has the print `from` gives it (`declared_from`, LIO-611).
    fn declared_holds(&self, from: &[(usize, Option<u64>)], now: &[OpSteps]) -> bool {
        let named = (self.skips.iter())
            .filter(|s| s.1 == Skip::Cut)
            .map(|s| s.0)
            .chain(self.cut_steps.iter().map(|s| s.0));
        named.into_iter().all(|oi| {
            let was = from.iter().find(|p| p.0 == oi).and_then(|p| p.1);
            let is = now.iter().find(|s| s.op == oi).map(OpSteps::declared_print);
            was.is_some() && is == was
        })
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
        // A check that had no part to check against, not loaded yet after
        // AXIS CAM opened say, says nothing of what was given back as taken
        // as cut: kept as it is, on disk too, for the check once the part is
        // in, nothing set from it meanwhile (`declared_from`, LIO-611 review).
        let no_part = |w: &String| w == PART_NOT_LOADED || w == NO_PART;
        let waits_for_part = self.declared_from.is_some()
            && (programs.as_ref().as_ref()).is_err_and(|why| why.first().is_some_and(no_part));
        if let Err(why) = programs.as_ref()
            && (self.start_step.is_some() || !self.cut_steps.is_empty())
            && why.first().is_none_or(|w| w != REMOVED)
            && !waits_for_part
        {
            self.journal_note = Some(format!(
                "The start at a step or the steps taken as cut could not be checked, so they are dropped and the operation starts at its first step: {} Pick another start in the Operations list if you like.",
                why.join(" ")
            ));
            (self.start_step, self.resumed, self.picked) = (None, None, None);
            // Its steps taken as cut go: their material is not the check's.
            self.declared_lost |= !self.cut_steps.is_empty();
            self.cut_steps.clear();
            self.keep_next_tool();
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
        // What was taken as cut, given back from before, holds only for the
        // plans and cutters it was taken as cut from: changed or not known
        // since, the project, the part or AXIS CAM changed, and it is
        // dropped as a project edit drops it (LIO-611).
        if programs.is_ok()
            && let Some(from) = self.declared_from.take()
            && !self.declared_holds(&from, left.as_ref().map_or(&[][..], |l| &l.steps[..]))
        {
            self.declared_lost = true;
            if self.follow.is_some() {
                // Followed as sent (`follow_again`), none kept as taken as cut.
                self.declared_stale = true;
            } else {
                self.skips.retain(|s| s.1 != Skip::Cut);
                self.cut_steps.clear();
                self.keep_next_tool();
                (self.programs, self.recheck) = (None, true);
                return;
            }
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
            // The start is the skip's now.
            self.start_moved = None;
        } else if let Ok(new) = programs.as_ref() {
            // The start stays on its operation only when its tool run still
            // has the same operations; the work zero is set for it again.
            // They are compared with the last check that passed: Check again
            // drops `programs` before it asks, and a refused check in
            // between leaves the start and the progress as they were
            // (LIO-636). Coolant set to Off leaves them the same operations,
            // and an edit elsewhere that numbers them otherwise (Duplicate,
            // Remove, Move up of an operation before them) too: they are
            // known by id, where both checks gave them (LIO-636 review).
            let ids_now = left.as_ref().map_or(&[][..], |l| &l.ops[..]);
            let by_id = !ids_now.is_empty() && !self.last_passed_ops.is_empty();
            let ops = |p: Option<&Checked>, ids: &[String]| {
                p.map(|p| {
                    let ops = (p.ops.iter()).map(|o| crate::post::OpState {
                        coolant: (false, false),
                        ..o.clone()
                    });
                    let ids = match by_id {
                        true => ids,
                        false => &[][..],
                    };
                    let keys = (p.indexes.iter()).map(|&oi| op_key(ids, oi));
                    (keys.collect::<Vec<_>>(), ops.collect::<Vec<_>>())
                })
            };
            let last = (self.last_passed.as_deref()).and_then(|a| a.as_deref().ok());
            let was = last.and_then(|c| c.get(self.next));
            let before = ops(was, &self.last_passed_ops);
            if self.done.len() != new.len() {
                self.done = vec![false; new.len()];
                self.next = 0;
            }
            let now = new.get(self.next);
            let after = ops(now, ids_now);
            if before.is_none() || before != after {
                // A start partway that moves says so (LIO-636). One moved
                // before stays moved from the operation picked then.
                if let Some(was) = was.filter(|_| self.from > 0) {
                    self.start_moved = (was.indexes.get(self.from))
                        .zip(was.ops.get(self.from))
                        .map(|(&oi, o)| (op_key(&self.last_passed_ops, oi), o.name.clone()));
                }
                self.from = 0;
            }
        }
        if self.skips.is_empty() {
            self.layout = Some(programs.clone());
        }
        self.beneath = left.as_ref().map_or_else(Vec::new, |l| l.beneath.clone());
        self.steps = left.as_ref().map_or_else(Vec::new, |l| l.steps.clone());
        self.simulated_earlier = left.as_ref().map_or_else(Vec::new, |l| l.earlier.clone());
        self.plan_setups = left.as_ref().map_or_else(Vec::new, |l| l.plan.clone());
        // A start at a step the check did not keep (a step of an operation
        // gone, or no longer in steps) is no start.
        if let Some((oi, n)) = self.start_step
            && self.op_steps(oi).is_none_or(|s| n >= s.steps.len())
            && !self.steps.is_empty()
        {
            self.start_step = None;
        }
        (self.zeroed, self.tool_kept, self.recheck) = (false, false, false);
        if programs.is_ok() {
            self.last_passed = Some(programs.clone());
            self.last_passed_ops = left.as_ref().map_or_else(Vec::new, |l| l.ops.clone());
        }
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
        let start = self.first_touch_tool();
        if let Some(t) = &mut self.touched_for
            && std::mem::take(&mut t.first)
        {
            t.tool = start;
        }
        (self.answered, self.rechecked_tool) = (true, None);
        // The setup's work system, kept while it is checked again; a
        // reference recorded in another is not the setup's (LIO-643).
        if let Some(w) = self.checked_wcs() {
            self.known_wcs = Some(w);
        }
        if let (Some(w), Some(r)) = (self.wcs(), self.reference_wcs)
            && w != r
        {
            (self.reference, self.reference_a, self.reference_wcs) = (None, None, None);
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
            // Another session's, it tells of runs of this setup whose cuts
            // only that session knew, whether or not where it stopped is
            // picked. This session's own is in what its runs cut
            // (`App::run_cuts`) (LIO-586 review).
            if j.setup == self.setup_id && !std::mem::take(&mut self.journal_own) {
                self.note_history_unknown();
            }
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

    /// Notes that runs of this setup no session here followed cut the
    /// stock (`HISTORY_UNKNOWN`): another session's journal, or a tool run
    /// it sent, followed (LIO-586 review). Such a journal says where its
    /// run last stopped, not what it cut before (all of the setup, cut
    /// again from an earlier step, or by operations deleted since), so no
    /// start sets Z from the stock the check carves until New stock
    /// loaded: the work zero is kept, or Z set by hand.
    pub fn note_history_unknown(&mut self) {
        note_cut_in(&mut self.cut_in_run, HISTORY_UNKNOWN, None, None);
    }

    /// What this setup's runs cut is unknown (`note_history_unknown`), or
    /// what another setup's did that this one has (`earlier_cuts`, LIO-596).
    pub fn history_unknown(&self) -> bool {
        (self.cut_in_run.iter()).any(|c| c.0 == HISTORY_UNKNOWN)
            || (self.earlier_cuts.iter())
                .any(|(_, cuts)| cuts.iter().any(|c| c.0 == HISTORY_UNKNOWN))
    }

    /// Something else may have cut the part: the Pi showed a tool run this
    /// computer did not send, or another computer in control (LIO-596), or
    /// the operator said other runs may have cut it after a disconnect
    /// (`Action::OtherRunsMayHaveCut`, LIO-595). What this setup's runs
    /// cut, a tool run of its running included (`cuts_so_far`), what the
    /// other setups' did (`earlier_cuts`), and what was cut before a tool
    /// run it follows (`before_follow`) are unknown from now on
    /// (`HISTORY_UNKNOWN`), until New stock loaded. True when any of them
    /// was not already.
    pub fn note_cut_elsewhere(&mut self) -> bool {
        // A tool run running cuts, its check in or not; what was taken as
        // cut, skipped, stepped over or started after (`partway`), may be
        // cut deeper since too (LIO-596 review).
        let (skips, steps) = self.declared();
        let declared = !skips.is_empty() || steps.iter().any(|(_, s)| !s.is_empty());
        let own = (self.running.is_some()
            || !self.cuts_so_far().is_empty()
            || declared
            || self.declared_lost
            || self.partway()
            // A start at a later operation or step, while its check is
            // pending (`partway` needs it).
            || (((self.next, self.from) != (0, 0) || self.start_step.is_some())
                && !self.started())
            || !self.all_cut().is_empty())
            && !(self.cut_in_run.iter()).any(|c| c.0 == HISTORY_UNKNOWN);
        if own {
            self.note_history_unknown();
        }
        let mut marked = own;
        for (_, cuts) in &mut self.earlier_cuts {
            marked |= note_unknown(cuts);
        }
        if let Some(cuts) = &mut self.before_follow {
            marked |= note_unknown_before(cuts);
        }
        marked
    }

    /// This computer saw this setup's runs cut it (`cut_in_run`), not
    /// counting that runs it did not follow may have
    /// (`history_unknown`).
    pub fn cut_by_runs(&self) -> bool {
        self.cut_in_run.iter().any(|c| c.0 != HISTORY_UNKNOWN)
    }

    /// Where a run of this setup stopped, found while it is open (the Pi
    /// stopped a tool run this computer did not follow): its start is the
    /// step after, now or once it is checked.
    /// `own`: its stops kept as this session wrote them (`read_own`).
    pub fn adopt_journal(&mut self, j: Journal, own: bool) {
        if self.running.is_none() && j.setup == self.setup_id {
            self.journal = Some(j);
            self.journal_own = own;
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
        self.reference_wcs = Some(s.active_wcs);
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

    /// The setup's work system, as the machine numbers them
    /// (`Snapshot::active_wcs`: G54 is 0), once the setup is checked; None
    /// before, or for one the machine does not have.
    /// While it is checked again, the one the check before found; None
    /// before the first check answers.
    pub fn wcs(&self) -> Option<usize> {
        self.checked_wcs().or(self.known_wcs)
    }

    /// The setup's work system its checked programs give.
    fn checked_wcs(&self) -> Option<usize> {
        let p = self.checked()?.first()?;
        OFFSETS
            .iter()
            .position(|o| *o == p.offset)
            .filter(|&i| i < 6)
    }

    /// The machine's active work system the reference was recorded in.
    pub fn reference_wcs(&self) -> Option<usize> {
        self.reference_wcs
    }

    /// The machine's active work system is the setup's, or the setup's is
    /// not known yet (its first check): only then does a reference
    /// recorded hold for the setup (LIO-643).
    pub fn in_wcs(&self, c: &Connection) -> bool {
        self.wcs().is_none_or(|w| w == c.snapshot.active_wcs)
    }

    /// Why the run's own buttons that touch its tool off, set its work zero
    /// or record its reference cannot be pressed now: the machine's active
    /// work system is not the setup's, so they would set another's Z, and
    /// the setup's would stay the old tool's (LIO-643). None when it is the
    /// setup's, or the setup's is not known yet.
    pub fn wcs_problem(&self, c: &Connection) -> Option<String> {
        crate::control::wcs_mismatch(OFFSETS[self.wcs()?], c.snapshot.active_wcs)
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
    /// steps stay (#168). `at`: the machine position the snapshot with the
    /// outcome gives, where the tool stopped (`reach`).
    pub fn refresh(
        &mut self,
        outcome: Option<&uc100::controller::ProgramOutcome>,
        at: Option<&[f64; 6]>,
    ) {
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
        // What it cut, for the starts after it (`cut_in_run`): its
        // operations from where it started, up to a stop.
        let ran: Vec<usize> = (self.checked().and_then(|c| c.get(r.index)))
            .map(|p| p.indexes[r.from.min(p.indexes.len())..].to_vec())
            .unwrap_or_default();
        // How much of operation `o` it cut, when the tool may have stopped
        // anywhere up to all of it: whole if it was followed past it.
        let passed = self.followed_past();
        let bound = |o: usize| match passed.contains(&o) {
            true => None,
            false => MAY_HAVE_CUT_ALL,
        };
        // The operations before `op` cut `before`: whole once the tool went
        // on past them, or up to all of them when it may not have.
        let up_to = |op: usize, steps: usize, before: &dyn Fn(usize) -> Option<usize>| {
            (ran.iter().take_while(|&&o| o != op))
                .map(|&o| (o, before(o)))
                .chain([(op, Some(steps))])
                .collect::<Vec<(usize, Option<usize>)>>()
        };
        let cut: Vec<(usize, Option<usize>)> = if o.ok {
            ran.iter().map(|&o| (o, None)).collect()
        } else if o.message.contains(NOT_STARTED) {
            vec![]
        } else if let Some((op, s)) = stopping {
            // Stopped after its last step sent, the operation is cut whole
            // (LIO-601 review): those after it were taken as cut before. Not
            // so where an E-stop ended it as it stopped, the tool maybe
            // still in that step's last feed (LIO-601 review).
            let cut = self.cut_of(op);
            let steps = self.op_steps(op).map_or(0, |o| o.steps.len());
            let e_stop = o.message.contains("E-stop") || o.message.contains("E-STOP");
            match !e_stop && (s + 1..steps).all(|t| cut.contains(&t)) {
                true => (ran.iter().take_while(|&&o| o != op))
                    .chain([&op])
                    .map(|&o| (o, None))
                    .collect(),
                // Ended by an E-stop, those before it are whole only where
                // the tool was followed past them (LIO-601 review).
                false if e_stop => up_to(op, s + 1, &bound),
                false => up_to(op, s + 1, &|_| None),
            }
        } else if let Some(far) = self.reach(at).and_then(|b| self.stopped_on(b, "")) {
            // In a step: it may be cut in part, up to the furthest step the
            // tool may have got to (`reach`), not just the one it was last
            // followed on, which may lag (LIO-586 review). The tool may not
            // have got that far, so those before it are known whole only
            // once it was followed past them (LIO-601).
            up_to(far.op, far.last.map_or(1, |l| l + 2), &bound)
        } else {
            // Where it stopped is not known: all it ran may be cut, or only
            // some of it, but for what it was followed past (LIO-601).
            ran.iter().map(|&o| (o, bound(o))).collect()
        };

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
                touched_for,
            } = r.before.clone();
            (self.xy, self.zero, self.last, self.touched_for) = (xy, zero, last, touched_for);
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
        // Followed again, what was cut before it is known: its outcome says
        // the rest (`before_follow`). One never started cut nothing more,
        // and keeps the bound.
        if let Some(before) = self.before_follow.take()
            && !o.message.contains(NOT_STARTED)
        {
            // Unknown stays unknown: something else may have cut the part
            // since, which no outcome of this computer's tells (LIO-596
            // review).
            let unknown = self.cut_in_run.iter().any(|c| c.0 == HISTORY_UNKNOWN);
            self.cut_in_run = before;
            if unknown {
                self.note_history_unknown();
            }
        }
        for (op, steps) in cut {
            self.note_cut(op, steps);
        }
        // Started, it took what is before it as cut (LIO-603).
        let started = (self.running.as_ref()).map(|r| (r.index, r.from));
        if let Some((index, from)) = started.filter(|_| !o.message.contains(NOT_STARTED)) {
            let cuts = std::mem::take(&mut self.cut_in_run);
            self.cut_in_run = self.taken_with(cuts, index, from);
        }
        self.outcome = Some((o.ok, message));
        self.running = None;
    }

    /// Where the running tool run is, stopped now in a step that then
    /// counts as not done (`why`): the operation and step of the block the
    /// tool tip is on (the first until it is found), the last step done of
    /// it being the one before, or that one if its cutting was done.
    pub fn stopped_in(&self, why: &str) -> Option<Stopped> {
        let block = self.running.as_ref()?.on.map_or(0, |o| o.0);
        self.stopped_on(block, why)
    }

    /// The furthest block of the running tool run the tool may be on,
    /// stopped at machine position `at`: the last, from the one it was
    /// followed to (`track`), whose move the tip is on. What it was followed
    /// to may lag: snapshots the app missed while busy, or the one with the
    /// outcome, which comes before `track` and gives no line once the
    /// program ended. So the tip's place bounds what it cut (LIO-586
    /// review). `at` must be where the tool stopped, not where it went since
    /// (None: not known, as on the Pi). None when it is not known, the tip
    /// is on none of them, or that one is in no step (`mark_step`): where
    /// it got is not known.
    fn reach(&self, at: Option<&[f64; 6]>) -> Option<usize> {
        let (r, at) = (self.running.as_ref()?, at?);
        let p = self.checked()?.get(r.index)?;
        let tip = std::array::from_fn(|k| (at[k] - r.offset[k] + p.datum[k] as f64) as f32);
        let from = r.on.map_or(0, |o| o.0);
        let b = (from..r.paths.len())
            .rev()
            .find(|&j| r.paths[j].is_some_and(|m| along(m, tip).is_some()))?;
        self.mark_step(p, r.marks.get(b).copied().flatten()?)?;
        Some(b)
    }

    /// The operations (index in the setup) of the running tool run the tool
    /// was followed past: those it sends before the operation of the block
    /// it was last followed on (`track`). Where it was followed lags where
    /// it is, so it got at least that far: they are cut whole (LIO-601
    /// review). None until a block in a step is followed.
    fn followed_past(&self) -> Vec<usize> {
        let Some(r) = &self.running else {
            return vec![];
        };
        let Some(p) = self.checked().and_then(|c| c.get(r.index)) else {
            return vec![];
        };
        let on = (r.on.and_then(|(b, _)| r.marks.get(b).copied().flatten())).map_or(0, |m| m.0);
        p.indexes[r.from.min(on).min(p.indexes.len())..on.min(p.indexes.len())].to_vec()
    }

    /// `stopped_in` with the tool on block `block` of the running tool run.
    fn stopped_on(&self, block: usize, why: &str) -> Option<Stopped> {
        let r = self.running.as_ref()?;
        let p = self.checked()?.get(r.index)?;
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
                self.keep_next_tool();
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
            touched_for,
        } = r.before;
        (self.xy, self.zero, self.last, self.touched_for) = (xy, zero, last, touched_for);
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
        if let Some(why) = self.earlier_not_whole(doc) {
            e.push(why);
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
        // A Z set on its way counts: Cycle Start waits for the machine to
        // show it, and then asks again if it did not take
        // (`Connection::z_set_waits`, LIO-627); as the work zero too, for
        // one that sets it (LIO-628).
        let touched = self.touched() || c.z_set_waits(self);
        let zeroed = self.zeroed || c.z_set_zeroes(self);
        // One that did not take: Z is as it was, maybe another tool's,
        // until the tool is touched off again (LIO-627 review).
        if let Some(why) = c.z_failed(self) {
            e.push(why.to_string());
        }
        if let Some((oi, n)) = self.at_step() {
            let step = self.step_label(oi, n);
            if !zeroed {
                // Where the check cannot tell the stock they left, it is
                // set by hand, and the reason said (LIO-586 review).
                e.push(match self.partway_blocked() {
                    Some(why) => format!(
                        "This run starts {} at {step}, the steps before it taken as cut: {STEP_ZERO_BY_HAND}. {why}",
                        first.name
                    ),
                    None => format!(
                        "This run starts {} at {step}, the steps before it taken as cut: keep the work zero they were cut with (Keep it, under Start at a step), or set it on the stock they left.",
                        first.name
                    ),
                });
            }
            if !unset.is_empty() {
                e.push(format!(
                    "{} {} no machine position, so the work zero the steps before {step} were cut with is not where it was: {} in the Machine tab first.",
                    unset.join(", "),
                    if unset.len() == 1 { "has" } else { "have" },
                    c.place(&unset_ids(c), true, true)
                ));
            }
            if !self.tool_kept && !touched {
                // At a tool change the tool in the spindle is another one:
                // not Same tool, keep Z (LIO-652).
                e.push(format!(
                    "Confirm {}'s Z for the start at {step}: {}.",
                    crate::model::tool_label(first.tool, &first.tool_name),
                    if self.tool_change().is_some() {
                        "Set Z from reference"
                    } else {
                        "Same tool, keep Z if it was not taken out since, or Set Z from reference"
                    }
                ));
            }
        } else if self.asks_zero() {
            // Operations or steps after the start, or of its operation, taken
            // as cut wait for the work zero too once a tool run of this run
            // has started: each new check of them asks again (LIO-583 review).
            let later = self.later_cut();
            match self.partly() {
                None if !later => e.push(format!("The check did not work out the stock operation {}, {}, starts from: pick its first operation, or check the setup again.", from + 1, first.name)),
                _ if zeroed => {}
                _ => {
                    let oi = p.indexes.get(from).copied();
                    let steps: Vec<usize> = oi.map_or_else(Vec::new, |oi| self.cut_of(oi));
                    let after: Vec<String> = (self.cut_named().into_iter())
                        .filter(|c| oi.is_some_and(|o| c.0 > o))
                        .map(|c| c.1)
                        .collect();
                    e.push(match (oi, &steps[..]) {
                        // Dropped by an edit: only a hand, or new stock,
                        // knows the surface (LIO-583 review).
                        _ if self.declared_lost && !later => format!(
                            "This run starts at {}, and operations or steps taken as cut before the project changed may not all be taken as cut now, so the check cannot tell the surface they left where the work zero is touched off: in the Work zero tab, set Z by hand on a surface whose height you know and press Keep this work zero, or press New stock loaded if the stock is new.",
                            first.name
                        ),
                        // Nothing sets Z on the stock they left but a hand
                        // that knows its height, and Keep this work zero
                        // says so (`later_cut`, LIO-583).
                        _ if later => format!(
                            "This run starts at {}, with {} after it taken as cut, and the check cannot tell the surface they left where the work zero is touched off: in the Work zero tab, set Z by hand on a surface whose height you know and press Keep this work zero, or press it if the work zero is still the one they were cut with.",
                            first.name,
                            after.join(", ")
                        ),
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
        // Z touched off for another tool since (another tool run picked,
        // or its tool changed by an edit, LIO-619): that tool's Z, while
        // the offset is the one it left.
        let tool = &p.ops[0].tool_id;
        let unchanged = |z: f64| (s.work_offset[2] - z).abs() < 1e-6;
        let other = (self.touched_for.as_ref())
            .filter(|t| t.tool.as_ref() != Some(tool))
            .is_some_and(|t| t.z.is_none_or(unchanged));
        let change = (self.last.as_ref()).is_some_and(|(t, z)| t != tool && unchanged(*z));
        // At a tool change, a touch-off forgotten since (another start
        // picked, or the setup checked again, LIO-618) is not this start's
        // while the offset is the one it left: it waits for the new tool's
        // Z, as the Tool change tab shows.
        let forgotten = (self.last.as_ref()).is_some_and(|(t, _)| t != tool)
            && (self.touched_for.as_ref())
                .is_some_and(|t| !t.for_start && t.z.is_none_or(unchanged));
        if self.z_elsewhere(c) || !touched && (other || change || forgotten) {
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
        // A tool run this computer starts is not the one a follow was for.
        self.before_follow = None;
        self.running = Some(Running {
            index: self.next,
            from: self.from,
            seq: s.program_outcome.as_ref().map_or(0, |o| o.seq),
            offset: s.work_offset,
            before: Before {
                xy: self.xy,
                zero: self.zero.clone(),
                last: self.last.clone(),
                touched_for: self.touched_for.clone(),
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
        self.touched_for = None;
        (self.outcome, self.start_moved) = (None, None);
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

    /// Operations after the start's operation, or steps of them, are taken
    /// as cut (`Skip::Cut`, `cut_steps`): the stock they cut, which may be
    /// where the work zero is touched off or probed, is in no stock the
    /// check found for the start, partway or not, so neither sets it there
    /// (`touch_off`, `partway_blocked`, `Connection::probe_spots`; LIO-582).
    pub fn later_cut(&self) -> bool {
        let Some(start) =
            (self.checked()).and_then(|c| c.get(self.next)?.indexes.get(self.from).copied())
        else {
            return false;
        };
        (self.skips.iter()).any(|s| s.1 == Skip::Cut && s.0 > start)
            || (self.cut_steps.iter()).any(|c| !c.1.is_empty() && c.0 > start)
    }

    /// The check cannot tell the stock top the setup's first operation
    /// starts from where the work zero is touched off (`Checked::partly`
    /// of it, `post::spot_surface`): an earlier setup's cut may reach under
    /// it there, a bore from the other side, say, which the work-zero
    /// check's sample points can miss (LIO-582 review). For a run from the
    /// setup's start; a partway one has its own (`partly`).
    fn inherited_unknown(&self) -> bool {
        (self.checked().and_then(|c| c.first()?.partly.first()))
            .is_some_and(|first| first.surface.is_none())
    }

    /// Earlier setups cut into the stock top the setup's first operation
    /// starts from, somewhere over the stock (`Partly::top_cut` of it): a
    /// touch plate laid on the top may lie in what they cut (LIO-582
    /// review). For a run from the setup's start.
    pub fn inherited_top_cut(&self) -> bool {
        (self.checked().and_then(|c| c.first()?.partly.first())).is_some_and(|first| first.top_cut)
    }

    /// Notes that a tool run of this run cut operation `oi` (index in the
    /// setup): all of it (None) or its steps before `steps`, with the plan
    /// the check has of it now (`cut_in_run`).
    fn note_cut(&mut self, oi: usize, steps: Option<usize>) {
        let print = self.op_steps(oi).map(|s| s.cut_print);
        note_cut_in(&mut self.cut_in_run, oi, steps, print);
    }

    /// What this run's tool runs have cut so far (`cut_in_run`), a tool
    /// run that runs now taken as cutting every operation it runs up to all
    /// of it (`MAY_HAVE_CUT_ALL`) but those it was followed past
    /// (`followed_past`), as far as it may get: what is kept when
    /// the run is dropped while one runs, the machine disconnected say,
    /// which no outcome then tells (LIO-582 review, LIO-601).
    ///
    /// With them, what a tool run that started took as cut, before its
    /// start (`taken_with`): a later setup goes by an earlier one's
    /// operations as cut only while its runs cut them or took them as cut
    /// (`earlier_uncut`, LIO-603).
    pub fn cuts_so_far(&self) -> Vec<CutInRun> {
        match &self.running {
            Some(r) => self.cuts_with(r.index, r.from, &self.followed_past()),
            None => self.cut_in_run.clone(),
        }
    }

    /// `cuts_so_far` once Cycle Start starts the next tool run
    /// (`next`, `from`): what is kept on disk before it is sent (LIO-594
    /// review).
    pub fn cuts_if_started(&self) -> Vec<CutInRun> {
        let mut cuts = self.cuts_so_far();
        let started = self.cuts_with(self.next, self.from, &[]);
        for (oi, steps, print, whole) in started {
            note_cut_whole(&mut cuts, oi, steps, print, whole);
        }
        cuts
    }

    /// `cut_in_run` and tool run `index` from its operation `from` on, up
    /// to all of each (`MAY_HAVE_CUT_ALL`), but those it was followed
    /// `passed`, whole (LIO-601), with what it takes as cut (`taken_with`).
    fn cuts_with(&self, index: usize, from: usize, passed: &[usize]) -> Vec<CutInRun> {
        let mut cuts = self.cut_in_run.clone();
        if let Some(p) = self.checked().and_then(|c| c.get(index)) {
            for &oi in &p.indexes[from.min(p.indexes.len())..] {
                let print = self.op_steps(oi).map(|s| s.cut_print);
                let steps = match passed.contains(&oi) {
                    true => None,
                    false => MAY_HAVE_CUT_ALL,
                };
                note_cut_in(&mut cuts, oi, steps, print);
            }
        }
        self.taken_with(cuts, index, from)
    }

    /// `cuts` with the operations tool run `index`, started from its
    /// operation `from`, took as cut whole (`note_taken`): those of the
    /// tool runs before it and of its own before that one. Not those
    /// skipped as already cut: a skip can be undone after, so a later setup
    /// reads them as they are marked now (`Run::earlier_taken`, LIO-603
    /// review).
    fn taken_with(&self, mut cuts: Vec<CutInRun>, index: usize, from: usize) -> Vec<CutInRun> {
        let programs = self.checked().unwrap_or(&[]);
        let before = programs
            .iter()
            .take(index)
            .flat_map(|p| p.indexes.iter().copied());
        let own = (programs.get(index).into_iter())
            .flat_map(|p| p.indexes[..from.min(p.indexes.len())].iter().copied());
        self.note_taken(&mut cuts, before.chain(own).collect());
        cuts
    }

    /// Notes in `cuts` the operations `taken` as cut whole, from the plan
    /// the check has of each now: one that has no note of what was cut of
    /// it. One cut in part stays so: taken as cut, its stock is not known
    /// (LIO-601). A later setup's check takes an earlier setup's operations
    /// as cut only while that setup's runs cut them, or took them as cut
    /// (`Run::earlier_uncut`, LIO-603).
    ///
    /// Each goes in at its place in the setup's order, ahead of operations
    /// after it: taken as cut before them, not cut after them, which
    /// `cut_out_of_order` would otherwise read in the record (LIO-599
    /// review).
    fn note_taken(&self, cuts: &mut Vec<CutInRun>, taken: Vec<usize>) {
        for oi in taken {
            if !cuts.iter().any(|c| c.0 == oi) {
                let print = self.op_steps(oi).map(|s| s.cut_print);
                let at = cuts.iter().position(|c| c.0 > oi).unwrap_or(cuts.len());
                cuts.insert(at, (oi, None, print, print));
            }
        }
    }

    /// This run's tool runs cut more than the check's stock before the start
    /// has (`cut_in_run`): an operation after the start's, the start's own
    /// whole, or steps of it at or after the step it starts at; or one the
    /// check does not cut as it was cut, disabled or left since, or its plan
    /// changed (its fingerprint), so its stock lacks what was cut.
    fn cut_beyond(&self, start: usize) -> bool {
        let step = self.at_step().map_or(0, |(_, n)| n);
        let simulated = |op: usize| {
            (self.checked().into_iter().flatten()).any(|p| p.indexes.contains(&op))
                || (self.skips.iter()).any(|s| s.0 == op && s.1 == Skip::Cut)
        };
        (self.cut_in_run.iter()).any(|&(op, steps, print, _)| {
            op > start
                || op == start && steps.is_none_or(|m| m > step)
                || !simulated(op)
                || print.is_none()
                || self.op_steps(op).map(|s| s.cut_print) != print
        })
    }

    /// Given what the setups' runs cut (`App::run_cuts`) as a pick of the
    /// setup on the Run panel takes it: its own setup's cuts, until the
    /// operator says whether the same part is still held
    /// (`stock_unconfirmed`), and the others' (`earlier_cuts`). With what
    /// runs cut `lost` (LIO-594) and none kept for it, it was cut by runs
    /// no session here knows of (`note_lost_cuts`). With the other setups'
    /// operations marked Already cut, `declared` (`earlier_taken`, LIO-603).
    pub fn picked_in(&mut self, cuts: &SessionCuts, lost: bool, declared: &[KeptDeclared]) {
        let own = cuts
            .iter()
            .find(|(id, _)| *id == self.setup_id)
            .map(|(_, c)| c.clone());
        let own = own.or_else(|| lost.then(|| vec![(HISTORY_UNKNOWN, None, None, None)]));
        if let Some(own) = own.filter(|c| !c.is_empty()) {
            self.cut_in_run = own;
            self.stock_unconfirmed = true;
        }
        crate::give_earlier_cuts(cuts, declared, self);
    }

    /// Another setup's run cut an operation this session (`earlier_cuts`)
    /// that the check does not simulate before this setup as it was cut:
    /// disabled, left, its plan or cutter changed, or its setup deleted,
    /// moved after this one, or after it and cut first (`Left::earlier`).
    /// Its stock lacks what was cut (LIO-582 review).
    ///
    /// So is one an earlier setup's runs neither cut nor took as cut, though
    /// they cut others of this part: its material left, or never run
    /// (`earlier_uncut`, LIO-603). Or one cut not known whole: stopped
    /// partway, or where it stopped not known (`earlier_unfinished`): the
    /// check simulates it whole, so its stock lacks what is still there
    /// (LIO-601).
    pub fn earlier_unfollowed(&self) -> bool {
        self.earlier_unsimulated() || !self.earlier_uncut().is_empty() || self.earlier_unfinished()
    }

    /// The operations of the setups before this one, as the Run panel names
    /// them, that the check simulates as cut but whose setup's runs cut
    /// others of this part (`earlier_cuts`) and neither cut nor took them
    /// as cut (`cuts_so_far`), nor are they marked Already cut there now,
    /// its runs' cuts known (no `HISTORY_UNKNOWN`),
    /// as planned now (`earlier_taken`): their material left
    /// (`Skip::Left`), or never run. What is there is not known, so nothing
    /// sets Z or probes (LIO-603). A setup with nothing cut, no entry or an
    /// empty one (New stock loaded), is taken as cut whole, as before
    /// (LIO-582).
    pub fn earlier_uncut(&self) -> Vec<&str> {
        (self.earlier_uncut_ops().iter())
            .map(|e| e.name.as_str())
            .collect()
    }

    /// `earlier_uncut`'s operations, as `Left::earlier` has them.
    fn earlier_uncut_ops(&self) -> Vec<&EarlierOp> {
        let earlier = &self.simulated_earlier[..];
        (earlier.iter())
            .filter(|e| {
                let marked = (self.earlier_taken.iter()).any(|(id, t)| {
                    *id == e.setup && t.iter().any(|&(o, p)| o == e.op && p == Some(e.marked))
                });
                // Cut by runs no session here knows of (`HISTORY_UNKNOWN`):
                // not known to be uncut either, which `unfollowed_earlier`
                // says as it is (LIO-603 review).
                !marked
                    && (self.earlier_cuts.iter()).any(|(id, cuts)| {
                        *id == e.setup
                            && !cuts.is_empty()
                            && !cuts.iter().any(|c| c.0 == e.op || c.0 == HISTORY_UNKNOWN)
                    })
            })
            .collect()
    }

    /// How many setups `earlier_uncut`'s operations are of.
    fn earlier_uncut_setups(&self) -> usize {
        let mut setups: Vec<&str> = (self.earlier_uncut_ops().iter())
            .map(|e| e.setup.as_str())
            .collect();
        setups.sort_unstable();
        setups.dedup();
        setups.len()
    }

    /// The operations `earlier_uncut` names, said as the reason the check
    /// does not know the stock, when they are all of `earlier_unfollowed`
    /// (LIO-603).
    ///
    /// With other cuts the check does not follow too, this project's
    /// (those after a left operation cut a plan the check does not
    /// simulate, the stock under them higher, say), that is said after
    /// them; another project's, or a setup's this project does not have,
    /// are said as they are without (LIO-607, LIO-635).
    ///
    /// Said so only where those other cuts are all of the setups' own
    /// operations after the first not cut: else what else the check does
    /// not follow is not known to be that, and the general reason is said
    /// instead (`earlier_unfollowed_how`, LIO-603 review, LIO-599).
    fn earlier_uncut_said(&self) -> Option<String> {
        let uncut = self.earlier_uncut();
        if uncut.is_empty() || self.elsewhere_unfollowed() || self.earlier_not_here() {
            return None;
        }
        let also = self.unfollowed_earlier().next().is_some();
        if also && !self.unfollowed_after_uncut() {
            return None;
        }
        let [were, them] = if uncut.len() == 1 {
            ["was", "it"]
        } else {
            ["were", "them"]
        };
        let whose = match (uncut.len(), self.earlier_uncut_setups()) {
            (1, _) => "its setup's",
            (_, 1) => "their setup's",
            _ => "their setups'",
        };
        let list = uncut.join(", ");
        Some(match also {
            false => format!(
                "{list} {were} not cut, though {whose} runs cut this part, and the check takes {them} as cut"
            ),
            true => format!(
                "{list} {were} not cut, though {whose} runs cut this part: the check takes {them} as cut, and the operations cut after {them} were cut with {its} material still there, which it does not follow either",
                its = if uncut.len() == 1 { "its" } else { "their" }
            ),
        })
    }

    /// Why Zero Z and Probe Z cannot set Z where `earlier_uncut_said`, with
    /// the ways out that work. Where the operations after them were cut
    /// with their material still there (`unfollowed_earlier`), cutting
    /// them now, or marking them Already cut, leaves the check simulating
    /// those after them from a stock they were not cut from: only disabling
    /// them, which the check then simulates them without, or New stock
    /// loaded, clears it (LIO-603 review).
    fn earlier_uncut_blocked(&self) -> Option<String> {
        self.earlier_uncut_ways(false, TOUCH_OFF_UNKNOWN)
    }

    /// `earlier_uncut_blocked` in the tablet file's words (LIO-603
    /// review): the ways out are the computer's, then a new sync. `so` is
    /// what follows from them: `TOUCH_OFF_UNKNOWN`, or `RAPIDS_THROUGH`.
    fn earlier_uncut_ways(&self, tablet: bool, so: &str) -> Option<String> {
        let said = self.earlier_uncut_said()?;
        let also = self.unfollowed_earlier().next().is_some();
        let one = self.earlier_uncut().len() == 1;
        let [they, them, were, are] = match one {
            true => ["it", "it", "was", "is"],
            false => ["they", "them", "were", "are"],
        };
        let so = so.replace("{them}", them);
        let its = match (one, self.earlier_uncut_setups()) {
            (true, _) => "its setup",
            (_, 1) => "their setup",
            _ => "their setups",
        };
        if tablet {
            let then = "then Sync projects here, or save the project for the tablet again.";
            return Some(match also {
                false => format!(
                    "{said}{so} On the computer, cut {them} first (pick {its}), mark {them} Already cut there if {they} {were} cut before, or disable {them} if {they} {are} not to be cut; {then}{}",
                    new_stock_on_the_computer!()
                ),
                true => format!(
                    "{said}{so} If {they} {are} not to be cut, disable {them} in {its} on the computer, which then follows what was cut; {then}{}",
                    new_stock_on_the_computer!()
                ),
            });
        }
        Some(match also {
            false => format!(
                "{said}{so} Cut {them} first: pick {its}. If {they} {were} cut before, mark {them} Already cut there; if {they} {are} not to be cut, disable {them}. If new stock is held, press New stock loaded under Work zero."
            ),
            true => format!(
                "{said}{so} If {they} {are} not to be cut, disable {them} in {its}: the check then follows what was cut. If new stock is held, press New stock loaded under Work zero."
            ),
        })
    }

    /// Every cut of `unfollowed_earlier` is of a setup `earlier_uncut`
    /// names an operation of, after the first it names there: cut with
    /// that one's material still there (LIO-603 review).
    fn unfollowed_after_uncut(&self) -> bool {
        let earlier = &self.simulated_earlier[..];
        let uncut = self.earlier_uncut_ops();
        (self.earlier_cuts.iter()).all(|(id, cuts)| {
            let first = (uncut.iter())
                .filter(|e| e.setup == *id)
                .map(|e| e.op)
                .min();
            cuts.iter().all(|&(op, _, print, _)| {
                let followed =
                    (earlier.iter()).any(|e| e.setup == *id && e.op == op && Some(e.cut) == print);
                followed || first.is_some_and(|f| op > f)
            })
        })
    }

    /// Why Zero Z and Probe Z cannot set Z, as the tablet file says it
    /// (`TabletZero::blocked`): `partway_blocked` in its words, the ways
    /// out of an operation not cut the computer's (LIO-603 review).
    fn tablet_blocked(&self) -> Option<String> {
        let blocked = self.partway_blocked()?;
        match self.earlier_uncut_blocked() {
            Some(computer) if computer == blocked => {
                self.earlier_uncut_ways(true, TOUCH_OFF_UNKNOWN)
            }
            _ => Some(self.for_tablet(blocked)),
        }
    }

    /// `why`, from the Run panel, as the tablet file says it: another
    /// project's cuts as that project's, named (`earlier_cause`, LIO-638),
    /// this project's setup's cut not followed by its case (LIO-599), else
    /// as `FOR_TABLET` has it. Each matched exactly: only the words change,
    /// never whether it is refused.
    fn for_tablet(&self, why: String) -> String {
        if self.elsewhere_unfollowed()
            && let Some(cause) = self.earlier_cause(true)
        {
            if why == self.earlier_unfollowed_blocked() {
                return format!(
                    "{cause}, so the computer cannot tell the surface where the stock top is touched off.{}",
                    new_stock_on_the_computer!()
                );
            }
            if why == self.earlier_unfollowed_why() {
                return format!(
                    "{cause}, so the computer does not know what they cut here.{}",
                    new_stock_on_the_computer!()
                );
            }
        }
        match self.earlier_unfollowed_how() {
            Some(h) if why == h.blocked() => h.tablet_blocked(),
            Some(h) if why == h.why() => h.tablet_why(),
            _ => for_tablet(why),
        }
    }

    /// `earlier_unfollowed` for an operation the check does not simulate
    /// before this setup as it was cut (`simulated_earlier`).
    fn earlier_unsimulated(&self) -> bool {
        self.unfollowed_earlier().next().is_some()
    }

    /// The setups of `earlier_cuts` whose cuts the check does not simulate
    /// before this setup as they were cut (`earlier_unsimulated`), with
    /// those cuts.
    fn unfollowed_earlier(&self) -> impl Iterator<Item = &(String, Vec<CutInRun>)> {
        let earlier = &self.simulated_earlier;
        (self.earlier_cuts.iter()).filter(move |(id, cuts)| {
            cuts.iter().any(|&(op, _, print, _)| {
                !(earlier.iter()).any(|e| e.setup == *id && e.op == op && Some(e.cut) == print)
            })
        })
    }

    /// Every cut that makes `earlier_unfollowed` is by a setup this project
    /// does not have (`project_setups`) nor is known to have had
    /// (`earlier_own`): another project's, the computer keeping one record
    /// for all of them (`App::run_cuts`), or one deleted since. Where whose
    /// is known to be another project's, that is said instead
    /// (`earlier_elsewhere`, LIO-607, in the tablet file too, LIO-638); with
    /// no owner kept, it is said so, with New stock loaded on the computer
    /// as the tablet's way out (LIO-635).
    pub fn earlier_not_here(&self) -> bool {
        let mut ids = self.unfollowed_earlier().map(|(id, _)| id).peekable();
        ids.peek().is_some()
            && ids.all(|id| !self.project_setups.contains(id) && !self.earlier_own.contains(id))
    }

    /// How this project's setups that `earlier_unfollowed` finds cut the
    /// part (LIO-599), the first in the plan the check had (`plan_setups`),
    /// the plan's own before a deleted one. A setup whose cuts the check
    /// does not follow are all after an operation of its own it left
    /// (`earlier_uncut`) is not named: that is said of it (LIO-603), and
    /// nothing of it is disabled or changed (LIO-599 review). None when only
    /// another project's did (`earlier_elsewhere`), or setups whose project
    /// is not known (`earlier_not_here`), each said so instead.
    pub fn earlier_unfollowed_how(&self) -> Option<EarlierUnfollowed> {
        let plan = &self.plan_setups;
        let own: Vec<&(String, Vec<CutInRun>)> = (self.unfollowed_earlier())
            .filter(|(id, _)| !self.earlier_elsewhere.iter().any(|o| o.setup == *id))
            .collect();
        // None of this project's, as far as is known (LIO-635).
        if own.is_empty() || self.earlier_not_here() {
            return None;
        }
        if plan.is_empty() {
            return Some(EarlierUnfollowed::Unknown);
        }
        let named: Vec<&(String, Vec<CutInRun>)> = (own.iter().copied())
            .filter(|(id, cuts)| !self.after_own_uncut(id, cuts))
            .collect();
        let me = plan.iter().position(|s| *s == self.setup_id);
        let placed = (named.iter())
            .filter_map(|(id, cuts)| Some((plan.iter().position(|s| s == id)?, cuts)))
            .min_by_key(|(at, _)| *at);
        Some(match placed {
            Some((at, _)) if me.is_some_and(|me| at > me) => EarlierUnfollowed::After(at),
            Some((at, cuts)) if self.cut_out_of_order(&plan[at], cuts) => {
                EarlierUnfollowed::OutOfOrder(at)
            }
            Some((at, _)) => EarlierUnfollowed::Changed(at),
            None if named.iter().any(|(id, _)| self.earlier_own.contains(id)) => {
                EarlierUnfollowed::Deleted
            }
            None => EarlierUnfollowed::Unknown,
        })
    }

    /// The cuts of setup `id`, `cuts`, the check does not follow are all of
    /// operations after the first of its own `earlier_uncut` names: cut
    /// with that one's material still there, as LIO-603 says.
    fn after_own_uncut(&self, id: &str, cuts: &[CutInRun]) -> bool {
        let earlier = &self.simulated_earlier;
        let first = (self.earlier_uncut_ops().iter())
            .filter(|e| e.setup == id)
            .map(|e| e.op)
            .min();
        first.is_some_and(|first| {
            cuts.iter().all(|&(op, _, print, _)| {
                op > first
                    || (earlier.iter()).any(|e| e.setup == id && e.op == op && Some(e.cut) == print)
            })
        })
    }

    /// Setup `id`'s runs cut an operation the check does not follow
    /// before one that comes ahead of it in the setup: `cuts` keeps each
    /// operation where it was first cut (`note_cut_in`).
    fn cut_out_of_order(&self, id: &str, cuts: &[CutInRun]) -> bool {
        let earlier = &self.simulated_earlier;
        (cuts.iter().enumerate()).any(|(i, &(op, _, print, _))| {
            let followed =
                (earlier.iter()).any(|e| e.setup == id && e.op == op && Some(e.cut) == print);
            !followed && op != HISTORY_UNKNOWN && cuts[i + 1..].iter().any(|c| c.0 < op)
        })
    }

    /// What another project's runs cut that the check does not follow
    /// (`earlier_unfollowed`, `earlier_elsewhere`), said (LIO-607): None
    /// when none did. With whether a setup of this project's, or one whose
    /// project is not known, cut what it does not follow too. In the
    /// tablet file's words when `tablet` (LIO-638): those runs were the
    /// computer's, before the file was made.
    fn earlier_elsewhere_said(&self, tablet: bool) -> Option<(String, bool)> {
        let (projects, own) = self.earlier_elsewhere_projects()?;
        let which = (projects.iter())
            .map(|(title, ops)| match (title.trim(), ops.is_empty()) {
                ("", true) => "one with no name".to_string(),
                ("", false) => format!("one with no name: {}", crate::post::listed(ops)),
                (t, true) => format!("\"{t}\""),
                (t, false) => format!("\"{t}\": {}", crate::post::listed(ops)),
            })
            .collect::<Vec<_>>()
            .join("; ");
        let whose = match projects.len() {
            1 => "another project's runs",
            _ => "other projects' runs",
        };
        Some((
            match tablet {
                false => format!(
                    "{whose} cut stock on this machine earlier ({which}), which this project's check does not follow"
                ),
                true => format!(
                    "{whose} on the computer cut stock on the machine before this tablet file was made ({which}), which this project's plan does not follow"
                ),
            },
            own,
        ))
    }

    /// The other projects whose runs cut what the check does not follow
    /// (`earlier_unfollowed`, `earlier_elsewhere`), each its title and the
    /// names of the operations that cut, and whether a setup of this
    /// project's, or one whose project is not known, cut what it does not
    /// follow too (LIO-607). None when no other project's did.
    fn earlier_elsewhere_projects(&self) -> Option<(ProjectsCut<'_>, bool)> {
        let mut projects: Vec<(&str, &str, Vec<&str>)> = vec![];
        let mut own = false;
        for (id, cuts) in self.unfollowed_earlier() {
            let Some(o) = self.earlier_elsewhere.iter().find(|o| o.setup == *id) else {
                own = true;
                continue;
            };
            let i = match projects.iter().position(|p| p.0 == o.project) {
                Some(i) => i,
                None => {
                    projects.push((&o.project, &o.title, vec![]));
                    projects.len() - 1
                }
            };
            for &(op, _, _, _) in cuts {
                if let Some(name) = o.ops.get(op).filter(|n| !n.is_empty())
                    && !projects[i].2.contains(&name.as_str())
                {
                    projects[i].2.push(name);
                }
            }
        }
        if projects.is_empty() {
            return None;
        }
        let projects = projects.into_iter().map(|(_, t, ops)| (t, ops)).collect();
        Some((projects, own))
    }

    /// Why the stock is not the check's where another setup's or another
    /// project's run cut it (`earlier_unfollowed`), the start of a sentence:
    /// this project's setup by its case (`earlier_unfollowed_how`,
    /// LIO-599), another project's by its name (LIO-607), or both. None
    /// when only setups whose project is not known did (`earlier_not_here`).
    /// In the tablet file's words when `tablet` (LIO-638).
    fn earlier_cause(&self, tablet: bool) -> Option<String> {
        let how = self.earlier_unfollowed_how();
        let said = self.earlier_elsewhere_said(tablet);
        Some(match (how, said, tablet) {
            (Some(how), Some((said, _)), false) => format!("{}, and {said}", how.said()),
            // The tablet's case says "before this tablet file was made"
            // itself (LIO-638 review).
            (Some(how), Some((said, _)), true) => format!(
                "{}, and {}",
                how.said_on_tablet(),
                said.replacen(" before this tablet file was made", " too", 1)
            ),
            (None, Some((said, true)), false) => {
                format!("{}, and {said}", EarlierUnfollowed::Unknown.said())
            }
            // One sentence, "does not follow" once (LIO-638 review).
            (None, Some((said, true)), true) => format!("Another setup's run and {said}"),
            (None, Some((said, false)), _) => format!("{}{}", said[..1].to_uppercase(), &said[1..]),
            (Some(how), None, false) => how.said(),
            (Some(how), None, true) => how.said_on_tablet(),
            (None, None, _) => return None,
        })
    }

    /// Another project's runs cut what the check does not follow, said by
    /// `earlier_cause` whole rather than by a case's own sentences.
    fn elsewhere_unfollowed(&self) -> bool {
        self.earlier_elsewhere_projects().is_some()
    }

    /// Why Zero Z on the stock top and Probe Z cannot set Z where
    /// `EARLIER_UNFOLLOWED` cut (`partway_blocked`).
    fn earlier_unfollowed_blocked(&self) -> String {
        match (self.earlier_cause(false), self.elsewhere_unfollowed()) {
            (Some(cause), true) => format!(
                "{cause}, so the check cannot tell the surface where the stock top is touched off. If new stock is held, press New stock loaded under Work zero."
            ),
            (Some(_), false) => (self.earlier_unfollowed_how())
                .unwrap_or(EarlierUnfollowed::Unknown)
                .blocked(),
            // Whose not known, but none of this project's: the tablet
            // file's case with no owner kept (LIO-635).
            (None, _) => EARLIER_ELSEWHERE_BLOCKED.into(),
        }
    }

    /// What the Work zero tab says of the cuts the check does not follow
    /// (`earlier_unfollowed`), with New stock loaded: short, for why they
    /// set no Z is said with the steps above it (`partway_blocked`); this
    /// project's by its case (LIO-599), another project's named (LIO-607).
    pub fn earlier_unfollowed_note(&self) -> String {
        // Only operations not cut: named (LIO-603).
        if let Some(said) = self.earlier_uncut_said() {
            return format!("{said}.");
        }
        let how = self.earlier_unfollowed_how();
        let Some((projects, own)) = self.earlier_elsewhere_projects() else {
            return match how {
                Some(how) => how.said_whole(),
                // Whose not known, but none of this project's (LIO-635).
                None => "This computer's runs cut stock for a setup this project does not have: another project's, or one deleted since.".into(),
            };
        };
        let mine = how.unwrap_or(EarlierUnfollowed::Unknown).said();
        let titles: Vec<String> = (projects.iter())
            .map(|(t, _)| match t.trim() {
                "" => "one with no name".to_string(),
                t => format!("\"{t}\""),
            })
            .collect();
        let titles = crate::post::listed(&titles);
        match (own, projects.len()) {
            (false, 1) => format!("Another project's runs ({titles}) may have cut this stock."),
            (false, _) => format!("Other projects' runs ({titles}) may have cut this stock."),
            (true, 1) => {
                format!("{mine}, and another project's runs ({titles}) may have cut it too.")
            }
            (true, _) => {
                format!("{mine}, and other projects' runs ({titles}) may have cut it too.")
            }
        }
    }

    /// Why a probe cannot find a work zero from a spot of a stock
    /// `EARLIER_UNFOLLOWED` cut.
    fn earlier_unfollowed_why(&self) -> String {
        if self.earlier_partway() {
            return EARLIER_PARTWAY_WHY.into();
        }
        if let Some(said) = self.earlier_uncut_said() {
            return format!("{said}, so it does not know what is left here.");
        }
        match (self.earlier_cause(false), self.elsewhere_unfollowed()) {
            (Some(cause), true) => format!("{cause}."),
            (Some(_), false) => (self.earlier_unfollowed_how())
                .unwrap_or(EarlierUnfollowed::Unknown)
                .why(),
            (None, _) => EARLIER_ELSEWHERE_WHY.into(),
        }
    }

    /// Why a probe cannot find this work zero where `EARLIER_UNFOLLOWED`
    /// cut (`Connection::cut_probe_blocked`).
    pub fn earlier_unfollowed_probe(&self) -> String {
        if self.earlier_partway() {
            return "Another setup's run left an operation of this part unfinished earlier (its run stopped partway through it, or where it stopped is not known), and the check takes that operation as cut whole where a probe finds this work zero. Run that operation to its end first, in a run of its setup.".into();
        }
        if let Some(said) = self.earlier_uncut_said() {
            return format!("{said}, where a probe finds this work zero.");
        }
        match (self.earlier_cause(false), self.elsewhere_unfollowed()) {
            (Some(cause), true) => format!(
                "{cause}, so the check cannot tell the stock where a probe finds this work zero."
            ),
            (Some(_), false) => (self.earlier_unfollowed_how())
                .unwrap_or(EarlierUnfollowed::Unknown)
                .probe_blocked(),
            // Whose not known, but none of this project's (LIO-635).
            (None, _) => "This computer's runs cut stock for a setup this project does not have, another project's or one deleted since: if that is the part held, they may have cut it where a probe finds this work zero. If new stock is held, press New stock loaded under Work zero.".into(),
        }
    }

    /// Another setup's runs cut an operation not known to be cut whole
    /// (`earlier_cuts` with steps): stopped in it, or where it stopped not
    /// known (`MAY_HAVE_CUT_ALL`). Even steps before one past its last are
    /// no proof: a stop in its last step is noted so too (`refresh`). One
    /// they left or never ran is `earlier_uncut`'s (LIO-603).
    fn earlier_unfinished(&self) -> bool {
        !self.earlier_unfinished_ops().is_empty()
    }

    /// The operations `earlier_unfinished` is about: each setup's id and
    /// the operation's index in it.
    fn earlier_unfinished_ops(&self) -> Vec<(&str, usize)> {
        (self.earlier_cuts.iter())
            .flat_map(|(id, cuts)| {
                (cuts.iter().filter(|c| c.1.is_some())).map(move |c| (id.as_str(), c.0))
            })
            .collect()
    }

    /// The operations `earlier_unfinished` is about, as "setup / operation",
    /// for the operator to finish (LIO-601 review).
    pub fn earlier_partway_named(&self, doc: &Document) -> Vec<String> {
        (self.earlier_unfinished_ops().into_iter())
            .filter_map(|(id, oi)| {
                let setup = doc.setups.iter().find(|s| s.id == *id)?;
                Some(format!(
                    "{} / {}",
                    setup.name(),
                    setup.operations.get(oi)?.name()
                ))
            })
            .collect()
    }

    /// `earlier_unfollowed` only because an earlier setup's run cut an
    /// operation the check simulates before this setup, but maybe not to
    /// its end (`earlier_unfinished`): the check takes it as cut whole, so
    /// where what was not cut would have cut, its stock is lower than the
    /// part's, and finishing that operation is what clears it (LIO-601).
    /// Its reasons say so.
    pub fn earlier_partway(&self) -> bool {
        self.earlier_unfinished() && !self.earlier_unsimulated() && self.earlier_uncut().is_empty()
    }

    /// Why Cycle Start waits because the check cuts, before this setup, an
    /// operation another setup's runs cut earlier (`earlier_cuts`) but not
    /// certainly whole with the plan and cutter it has now (`Left::earlier`,
    /// `CutInRun`): stopped partway, stopped where it is not known, or cut
    /// from another plan. The programs are made for the stock the check
    /// leaves, their rapids and links just clear of it: over what the real
    /// cut left, they could go through material (LIO-602). None when none
    /// is: one the check leaves out (disabled, or its setup deleted or
    /// moved after this one) leaves the check more stock than the part has,
    /// which no rapid runs into, and so do cuts after one certainly whole.
    /// Nor where an earlier setup's runs cut this part but neither cut nor
    /// took as cut an operation it simulates (`earlier_uncut`, LIO-603):
    /// its material left, the programs made for it gone.
    pub fn earlier_not_whole(&self, doc: &Document) -> Option<String> {
        self.earlier_not_whole_for(doc, false)
    }

    /// `earlier_not_whole`, said for the tablet file (`tablet`), which has no
    /// New stock loaded and was made after those runs (`for_tablet`).
    fn earlier_not_whole_for(&self, doc: &Document, tablet: bool) -> Option<String> {
        enum Why {
            Partway,
            NotKnown,
            Changed,
        }
        let uncut = self.earlier_uncut();
        if !uncut.is_empty() {
            if let Some(why) = self.earlier_uncut_ways(tablet, RAPIDS_THROUGH) {
                return Some(why);
            }
            // Said otherwise for the touch-off, with another project's runs
            // or ones the check does not follow (`earlier_uncut_said`
            // None): still made for them cut (LIO-602 review).
            let [they, them, were] = match uncut.len() {
                1 => ["it", "it", "was"],
                _ => ["they", "them", "were"],
            };
            let its = match (uncut.len(), self.earlier_uncut_setups()) {
                (1, _) => "its setup",
                (_, 1) => "their setup",
                _ => "their setups",
            };
            let said = format!(
                "{} {were} not cut, though the runs of {its} cut this part, and this setup's programs are made for {them} cut: their rapids could go through the material left.",
                uncut.join(", "),
            );
            return Some(match tablet {
                false => format!(
                    "{said} Cut {them} first (pick {its}), mark {them} Already cut there if {they} {were} cut before, or disable {them}. If new stock is held, press New stock loaded under Work zero."
                ),
                true => format!(
                    "{said} On the computer, cut {them} first (pick {its}), mark {them} Already cut there if {they} {were} cut before, or disable {them}; then Sync projects here, or save the project for the tablet again.{}",
                    new_stock_on_the_computer!()
                ),
            });
        }
        let earlier = &self.simulated_earlier;
        let (id, op, why) = (self.earlier_cuts.iter()).find_map(|(id, cuts)| {
            cuts.iter().find_map(|&(op, steps, print, whole)| {
                let p = &earlier.iter().find(|e| e.setup == *id && e.op == op)?.cut;
                let why = match (print == Some(*p), steps) {
                    _ if whole == Some(*p) => return None,
                    (true, MAY_HAVE_CUT_ALL) => Why::NotKnown,
                    (true, Some(_)) => Why::Partway,
                    // Kept before the plan cut whole was (`kept_cuts`).
                    (true, None) => Why::NotKnown,
                    (false, _) => Why::Changed,
                };
                Some((id, op, why))
            })
        })?;
        let setup = doc.setups.iter().find(|s| s.id == *id)?;
        let name = setup.name();
        let op_name = (setup.operations.get(op)).map_or("an operation", |o| o.name());
        let (run, then) = match tablet {
            false => (format!("{name}'s run"), ""),
            true => (
                format!("{name}'s run on the computer"),
                " before this tablet file was made",
            ),
        };
        // The ways out: on the computer, for the tablet, then a new sync;
        // the tablet's own runs never reach the computer (LIO-602 review).
        let first = |what: &str| match tablet {
            false => format!(
                "{what} first, in {name}'s run. If new stock is held, press New stock loaded under Work zero."
            ),
            true => format!(
                "On the computer, {}{} first, in {name}'s run, then Sync projects here, or save the project for the tablet again.{}",
                what[..1].to_lowercase(),
                &what[1..],
                new_stock_on_the_computer!()
            ),
        };
        Some(match why {
            Why::Partway => format!(
                "{run} stopped partway through {op_name}{then}, and this setup's programs are made for the stock with {op_name} cut whole: their rapids could go through the material it left. {}",
                match tablet {
                    false => format!(
                        "Finish it first: pick {name}, which resumes it where it stopped. If new stock is held, press New stock loaded under Work zero."
                    ),
                    true => first(&format!("Finish {op_name}")),
                }
            ),
            Why::NotKnown => format!(
                "{run} of {op_name} stopped where it is not known{then}, so it may have cut only part of it, and this setup's programs are made for the stock with {op_name} cut whole: their rapids could go through what it left. {}",
                first(&format!("Cut {op_name} whole"))
            ),
            Why::Changed => format!(
                "{name}'s {op_name} changed since {}cut it{then}, and this setup's programs are made for the stock its plan leaves now: their rapids could go through material that cut left. {}",
                match tablet {
                    false => "its run ".to_string(),
                    true => "its run on the computer ".to_string(),
                },
                first(&format!("Cut {op_name} whole again"))
            ),
        })
    }

    /// The stock the start begins from is not the setup's own: the run
    /// starts partway (`partway`), or a tool run of it has started, which
    /// cut it, or has cut (`cut_in_run`); or whether the part runs cut is
    /// still held is not said yet (`stock_unconfirmed`), its own runs' or
    /// only other setups' (`earlier_cuts`) kept across a disconnect
    /// (LIO-596 review). Zero Z on the stock top, Probe Z and the probe
    /// spots go by the stock the check finds before the start (`partly`)
    /// then (LIO-582 review). Whether Cycle Start waits for the work zero
    /// is `asks_zero`'s, not this (LIO-583).
    pub fn on_cut_stock(&self) -> bool {
        self.checked().is_some()
            && (self.partway()
                || self.started()
                || !self.cut_in_run.is_empty()
                || self.earlier_unfollowed()
                || self.stock_unconfirmed)
    }

    /// The run starts partway through the setup before any of its tool runs
    /// started: at a later tool run or operation than the first, those
    /// before taken as cut, with steps of its operation taken as cut, or
    /// with operations or steps after its operation taken as cut
    /// (`later_cut`). Its work zero is set on the stock they leave
    /// (`partly`), and Cycle Start waits for it (`zeroed`).
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
        // So are operations or steps after it taken as cut, from the setup's
        // very start too: they were cut on this setup with a work zero the
        // stock top they may have cut away no longer finds, so Cycle Start
        // waits for it, kept or set by hand (LIO-583).
        let early = (self.next, self.from) != (0, 0)
            || self.skips.iter().any(cut)
            || self.cut_steps.iter().any(steps)
            || self.later_cut()
            || self.declared_lost;
        self.checked().is_some() && (self.at_step().is_some() || !self.started() && early)
    }

    /// What the operator took as cut in this run: the operations skipped as
    /// cut (`Skip::Cut`) and the steps skipped (`cut_steps`), kept by the
    /// session when the run is dropped and given back when its setup is
    /// picked again, until New stock loaded (`App::run_declared`, LIO-583
    /// review). Empty when nothing is.
    pub fn declared(&self) -> Declared {
        if self.declared_stale {
            return (vec![], vec![]);
        }
        let skips: Vec<(usize, Skip)> = (self.skips.iter())
            .filter(|s| s.1 == Skip::Cut)
            .copied()
            .collect();
        (skips, self.cut_steps.clone())
    }

    /// The print of the plan and cutter of each operation `declared` names
    /// (`KeptDeclared::prints`): as given back, while no check has held it
    /// since (`declared_from`), else the last check's (`OpSteps::declared_print`);
    /// None when not known (LIO-611).
    pub fn declared_prints(&self, (skips, steps): &Declared) -> Vec<(usize, Option<u64>)> {
        let mut ops: Vec<usize> = (skips.iter().map(|s| s.0))
            .chain(steps.iter().map(|s| s.0))
            .collect();
        ops.sort_unstable();
        ops.dedup();
        (ops.into_iter())
            .map(|oi| {
                let print = match &self.declared_from {
                    Some(from) => from.iter().find(|p| p.0 == oi).and_then(|p| p.1),
                    None => self.op_steps(oi).map(OpSteps::declared_print),
                };
                (oi, print)
            })
            .collect()
    }

    /// New stock loaded: what this setup's runs and the other setups' cut
    /// this session is forgotten, and so is what the operator took as cut
    /// (`declared`), the setup to be checked again without it (`recheck`).
    /// A start the operator picked stays (`start_picked`). One the run
    /// came to by itself goes back, with no tool run done, to the start
    /// the operator picked before it, or else to the setup's first
    /// operation sent: past operations skipped as cut (LIO-633), where a
    /// run stopped, this session's or a journal's, or after tool runs done.
    /// Kept there, what comes before it would be taken as cut on stock that
    /// is new, and Zero Z on the stock top would set Z on its floor
    /// (LIO-634). Where a run of the setup stopped is over whatever the
    /// start (`journal`, `resumed`; the file, `Connection::run_act`).
    /// Whether it went back.
    pub fn new_stock(&mut self) -> bool {
        (self.cut_in_run, self.earlier_cuts) = (vec![], vec![]);
        self.earlier_taken.clear();
        (self.stock_unconfirmed, self.declared_lost) = (false, false);
        (self.declared_stale, self.declared_from) = (false, None);
        let (skips, steps) = self.declared();
        // Where a run of it stopped is over, whatever the start (LIO-634
        // review): a journal waiting for the check to take it
        // (`take_journal`), and the one the start came from.
        let waits = (self.journal.take()).is_some_and(|j| j.setup == self.setup_id);
        (self.journal_own, self.resumed) = (false, None);
        let mut to = None;
        let back = match self.checked() {
            Some(programs) => {
                let (done, start) = self.progress_kept(programs);
                // The start, its operation and step; None once all is done.
                let at = start.map(|s| (s, self.at_step().map_or(0, |(_, n)| n)));
                // The operator's pick before it, which the run went on from.
                to = (self.start_picked).filter(|&p| at.is_none_or(|a| p <= a));
                at.is_none_or(|a| self.start_picked != Some(a))
                    && (to.is_some()
                        || !done.is_empty()
                        // No start left: the skips moved it past the end, or
                        // a stop after the last step did.
                        || at.is_none()
                        || at.is_some_and(|(s, n)| {
                            (self.next, self.from) != (0, 0)
                                || n > 0
                                || skips.iter().any(|&(oi, _)| oi < s)
                        }))
            }
            // Pressed while the check answers (a start at a step where a
            // run stopped is checked again, say): which operation the start
            // is cannot be told without its programs, so it goes back to the
            // setup's first operation unless it is the operator's start at
            // a step. A start the operator picked otherwise goes back too
            // then, which sets no Z on a floor that new stock does not have.
            // A journal waiting for the check was a start too: the check
            // would have put the start there.
            None => {
                waits
                    || match self.start_step {
                        Some(at) => self.start_picked != Some(at),
                        None => {
                            (self.next, self.from) != (0, 0)
                                || self.done.contains(&true)
                                || !skips.is_empty()
                        }
                    }
            }
        };
        // No tool run is done on the new stock, whatever the start: one run
        // to its end on the old, the operator's pick before it kept, is
        // sent again after it (LIO-634 review).
        self.done.iter_mut().for_each(|d| *d = false);
        if back {
            // A pick at a step is that step again; one not gone back to is
            // the operator's start no more (LIO-634 review).
            self.start_step = to.filter(|p| p.1 > 0);
            self.start_picked = to;
            // Nothing sets Z from programs checked for the old start until
            // the check answers.
            ((self.next, self.from), self.programs) = ((0, 0), None);
            // The first operation sent from there once nothing is cut.
            self.keep = Some((vec![], Some(to.map_or(0, |p| p.0))));
        } else if skips.is_empty() && steps.is_empty() {
            return false;
        } else if let Some(programs) = self.checked() {
            self.keep = Some((vec![], self.progress_kept(programs).1));
        }
        self.skips.retain(|s| s.1 != Skip::Cut);
        self.cut_steps.clear();
        (self.recheck, self.picked, self.carving) = (true, None, None);
        back
    }

    /// Cycle Start waits for the work zero to be set or kept for the stock
    /// the run declares (`zeroed`): it starts partway (`partway`), or
    /// operations or steps after its start's operation are taken as cut
    /// (`later_cut`), or, once a tool run of it has started, steps of its
    /// start's operation are, a later step taken as cut after a Stop, say.
    /// Each new check asks again (LIO-583 review).
    pub fn asks_zero(&self) -> bool {
        let start =
            (self.checked()).and_then(|c| c.get(self.next)?.indexes.get(self.from).copied());
        self.checked().is_some()
            && (self.partway()
                || self.later_cut()
                || self.declared_lost
                || self.started() && start.is_some_and(|oi| !self.cut_of(oi).is_empty()))
    }

    /// Where the operator touches off its setup, in plan mm, and the work
    /// position that is there (`post::touch_point` less the datum). None
    /// when it is not on the stock top: a Model origin whose stock top an
    /// earlier setup cut. A run that starts partway touches off on the
    /// surface the operations before it leave there, when the check knows
    /// it (`partly`), and is None when not (`partway_blocked`).
    pub fn touch_off(&self) -> Option<([f32; 3], [f64; 3])> {
        if self.later_cut() || self.declared_lost {
            return None;
        }
        let p = self.checked()?.first()?;
        if p.located.is_some_and(|(_, top)| top.is_none()) {
            return None;
        }
        let mut at = crate::post::touch_point(&p.origin, p.stock)?;
        if self.on_cut_stock() {
            at[2] = self.partly()?.surface?;
        } else if self.inherited_unknown() {
            return None;
        }
        Some((at, std::array::from_fn(|a| (at[a] - p.datum[a]) as f64)))
    }

    /// Why Zero Z on the stock top and Probe Z cannot set the work Z of a
    /// run that starts partway (`partway`): the check cannot tell the
    /// surface the operations before its start leave where the stock top
    /// is touched off. None when it can, or the run starts at the setup's
    /// first operation.
    pub fn partway_blocked(&self) -> Option<String> {
        if self.later_cut() {
            return Some(LATER_UNFOLLOWED_BLOCKED.into());
        }
        if self.declared_lost {
            return Some(DECLARED_LOST_BLOCKED.into());
        }
        if !self.on_cut_stock() {
            return self.inherited_unknown().then(|| INHERITED_UNKNOWN.into());
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
        } else if partly.cut == [LATER_UNFOLLOWED] {
            // Not a part of the stock either (LIO-582).
            LATER_UNFOLLOWED_BLOCKED.into()
        } else if partly.cut == [RUN_UNFOLLOWED] {
            RUN_UNFOLLOWED_BLOCKED.into()
        } else if partly.cut == [STOCK_UNCONFIRMED] && self.away.is_some() {
            STOCK_AWAY_BLOCKED.into()
        } else if partly.cut == [STOCK_UNCONFIRMED] {
            STOCK_UNCONFIRMED_BLOCKED.into()
        } else if partly.cut == [HISTORY_UNFOLLOWED] {
            HISTORY_UNFOLLOWED_BLOCKED.into()
        } else if partly.cut == [EARLIER_UNFOLLOWED] && self.earlier_partway() {
            EARLIER_PARTWAY_BLOCKED.into()
        } else if partly.cut == [EARLIER_UNFOLLOWED] {
            if let Some(blocked) = self.earlier_uncut_blocked() {
                return Some(blocked);
            }
            self.earlier_unfollowed_blocked()
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

    /// What a run that starts partway (`partway`) takes as cut, in the
    /// setup's order: each operation by name ("Face"), sent before its start
    /// or skipped (`Skip::Cut`, before the start or after it), and the steps
    /// taken as cut of the start's own operation and those after it ("Face
    /// pass 2", "Face passes 1–2", "Face, 3 of its passes"; LIO-583).
    pub fn taken_as_cut(&self) -> Vec<String> {
        self.cut_named().into_iter().map(|(_, said)| said).collect()
    }

    /// `taken_as_cut`, each with its operation (index in the setup).
    fn cut_named(&self) -> Vec<(usize, String)> {
        let Some(programs) = self.checked() else {
            return vec![];
        };
        let Some(p) = programs.get(self.next) else {
            return vec![];
        };
        let Some(start) = p.indexes.get(self.from).copied() else {
            return vec![];
        };
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
                    .filter(|s| s.1 == Skip::Cut)
                    .map(|s| s.0),
            )
            .collect();
        ops.sort_unstable();
        ops.dedup();
        let mut said: Vec<(usize, String)> = ops.iter().map(|&oi| (oi, name(oi))).collect();
        // The steps taken as cut of the start's operation and of those after
        // it (LIO-583); those before it are taken as cut whole.
        for (oi, cut) in self.all_cut() {
            if oi < start || ops.contains(&oi) {
                continue;
            }
            let (Some(&first), Some(&last)) = (cut.first(), cut.last()) else {
                continue;
            };
            let (op, steps) = (name(oi), self.op_steps(oi));
            let words = match steps {
                Some(s) if last - first + 1 == cut.len() => {
                    format!("{op} {}", s.labels(first..=last).to_lowercase())
                }
                _ => {
                    let kind = steps.map_or("steps", |s| s.kind().1).to_lowercase();
                    format!("{op}, {} of its {kind}", cut.len())
                }
            };
            said.push((oi, words));
        }
        said.sort_by_key(|s| s.0);
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
        let changed = self.start_at_step(oi, n);
        // The operator's own start, which New stock loaded keeps (LIO-633).
        if self.checked().is_some() && self.running.is_none() {
            self.start_picked = Some((oi, n));
        }
        changed
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
        // The steps it starts after are taken as cut by the start too; a
        // skip of one is kept all the same, the operator's word that it was
        // cut, which a start picked elsewhere later must not forget
        // (LIO-583 review). `step_stages` shows it as before the start.
        // Done in this run, its tool run is cut again from that step.
        let done: Vec<usize> = done.into_iter().filter(|&o| o < oi).collect();
        self.start_step = start;
        (self.keep, self.picked, self.carving) = (Some((done, Some(oi))), None, None);
        (self.zeroed, self.tool_kept, self.start_moved) = (false, false, None);
        // A Z set before is asked for again, as the tool's Z kept is
        // (LIO-618).
        self.forget_touch();
        true
    }

    /// What the Run panel says of a start a check moved back to its tool
    /// run's first operation (`checked_as`, LIO-636): where it was picked,
    /// and where Cycle Start starts now. Nothing while the start is at the
    /// operation picked, its tool run's first now.
    pub fn start_moved(&self) -> Option<String> {
        let (key, picked) = self.start_moved.as_ref()?;
        let p = self.checked()?.get(self.next).filter(|_| self.from == 0)?;
        let first = (p.indexes.first()).map(|&oi| op_key(&self.last_passed_ops, oi));
        (first.as_ref() != Some(key)).then(|| {
            format!(
                "The start was at {picked}, but the operations of its tool run have changed since, so Cycle Start now starts at {}, the first operation of tool run {}. Pick the start again in the Operations list if the operations before it are cut.",
                p.ops[0].name,
                self.next + 1
            )
        })
    }

    /// What a run that starts partway finds of the work zero (`partway`,
    /// `Checked::partly`). None when it does not start partway, or the
    /// check did not work it out.
    pub fn partly(&self) -> Option<std::borrow::Cow<'_, Partly>> {
        use std::borrow::Cow;
        if !self.on_cut_stock() {
            return None;
        }
        let programs = self.checked()?;
        let p = programs.get(self.next)?;
        let before = p.partly.get(self.from)?;
        let start = p.indexes.get(self.from).copied();
        // Whether the part its runs cut is still held is not said: nothing
        // sets Z or probes (LIO-582 review).
        if self.stock_unconfirmed {
            return Some(Cow::Owned(Partly {
                cut: vec![STOCK_UNCONFIRMED.into()],
                surface: None,
                top_cut: true,
                probe_from: (before.probe_from.iter())
                    .map(|(r, _)| (*r, Err(STOCK_UNCONFIRMED_WHY.into())))
                    .collect(),
                steps: vec![],
            }));
        }
        // Runs this computer did not follow may have cut it: another
        // session's, anything that ran while it was disconnected, or that
        // the Pi showed since: this setup's or another's record (LIO-586
        // review, LIO-595, LIO-596).
        if self.history_unknown() {
            return Some(Cow::Owned(Partly {
                cut: vec![HISTORY_UNFOLLOWED.into()],
                surface: None,
                top_cut: true,
                probe_from: (before.probe_from.iter())
                    .map(|(r, _)| (*r, Err(HISTORY_UNFOLLOWED_WHY.into())))
                    .collect(),
                steps: vec![],
            }));
        }
        // Another setup's run cut an operation the check does not simulate
        // so before this setup (LIO-582 review).
        if self.earlier_unfollowed() {
            return Some(Cow::Owned(Partly {
                cut: vec![EARLIER_UNFOLLOWED.into()],
                surface: None,
                top_cut: true,
                probe_from: (before.probe_from.iter())
                    .map(|(r, _)| (*r, Err(self.earlier_unfollowed_why())))
                    .collect(),
                steps: vec![],
            }));
        }
        // The tool runs this run has cut are the stock's: the check's stock
        // before the start has them when they are all before it, and not
        // while one runs or when one cut at or after it, a start picked
        // again before what was cut (LIO-582 review).
        if self.running.is_some() || start.is_some_and(|s| self.cut_beyond(s)) {
            return Some(Cow::Owned(Partly {
                cut: vec![RUN_UNFOLLOWED.into()],
                surface: None,
                top_cut: true,
                probe_from: (before.probe_from.iter())
                    .map(|(r, _)| (*r, Err(RUN_UNFOLLOWED_WHY.into())))
                    .collect(),
                steps: vec![],
            }));
        }
        // What was taken as cut and dropped since is in no stock the check
        // found (LIO-583 review).
        if self.declared_lost {
            return Some(Cow::Owned(Partly {
                cut: vec![DECLARED_LOST.into()],
                surface: None,
                top_cut: true,
                probe_from: (before.probe_from.iter())
                    .map(|(r, _)| (*r, Err(DECLARED_LOST_WHY.into())))
                    .collect(),
                steps: vec![],
            }));
        }
        // Operations after the start's, or steps of them, taken as cut have
        // cut the stock it starts from too, which no stock the check found
        // for it has (`post::partly`): it cannot tell the surface (LIO-582).
        if self.later_cut() {
            return Some(Cow::Owned(Partly {
                cut: vec![LATER_UNFOLLOWED.into()],
                surface: None,
                top_cut: true,
                probe_from: (before.probe_from.iter())
                    .map(|(r, _)| (*r, Err(LATER_UNFOLLOWED_WHY.into())))
                    .collect(),
                steps: vec![],
            }));
        }
        // Steps of its operation taken as cut, before a start at a step or
        // skipped, have cut the stock it starts from (LIO-568).
        let cut = start.map_or_else(Vec::new, |oi| self.cut_of(oi));
        // Partway by steps of its own alone, what cut the stock is the steps
        // taken as cut, and the probe spots' reasons say so (LIO-568).
        let by_steps = self.cut_by() != CUT_BY_OPERATIONS;
        let said = |e: &String| match e.strip_prefix("The operations before it ") {
            Some(rest) if by_steps => format!("The steps taken as cut {rest}"),
            _ => e.clone(),
        };
        // The check carved those very steps into it, or none were: it is the
        // stock the check found (LIO-582).
        if before.steps == cut && (!cut.is_empty() || self.at_step().is_none()) {
            if !by_steps {
                return Some(Cow::Borrowed(before));
            }
            let mut found = before.clone();
            for (_, ok) in &mut found.probe_from {
                if let Err(e) = ok {
                    *e = said(e);
                }
            }
            return Some(Cow::Owned(found));
        }
        // Inside an operation the check does not follow the stock, but it
        // knows it before the operation and after it (before the next one
        // sent): a spot neither cut is as it was, a surface the same in both
        // is that surface. Cut steps only take material away. A stock the
        // check carved other steps into is neither: the start is not the
        // one checked yet.
        let after = (programs.iter().flat_map(|p| &p.partly))
            .skip_while(|p| !std::ptr::eq(*p, before))
            .nth(1)
            .filter(|_| before.steps.is_empty());
        let unknown = STEPS_UNFOLLOWED;
        // The stock after the operation says what the operations before
        // the next one cut; partway by steps of its own alone, they are
        // the steps taken as cut (`said`, LIO-568).
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
                steps: vec![],
            },
            None => Partly {
                cut: vec![unknown.into()],
                surface: None,
                top_cut: true,
                probe_from: (before.probe_from.iter())
                    .map(|(r, _)| (*r, Err(format!("The check cannot follow the {unknown}."))))
                    .collect(),
                steps: vec![],
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
        // What was taken as cut is still cut in the stock held (LIO-583
        // review).
        let (cut, steps) = self.declared();
        self.declared_lost |= !cut.is_empty() || !steps.is_empty();
        (self.declared_stale, self.declared_from) = (false, None);
        (self.skips, self.keep, self.menu) = (vec![], None, None);
        (self.cut_steps, self.start_step, self.picked) = (vec![], None, None);
        // The other setups' marks name their operations by indexes the
        // edit may have changed, and their plans may cut otherwise now: no
        // longer taken as cut (`lose_declared`, LIO-603 review).
        self.earlier_taken.clear();
        // The edit may number the operations otherwise.
        self.start_picked = None;
    }

    /// Start here, or Next on a tool run: `start_at`, the start then the
    /// operator's own (`start_picked`), which New stock loaded keeps
    /// (LIO-633).
    pub fn pick_start(&mut self, i: usize, k: usize) {
        self.start_at(i, k);
        if self.running.is_none()
            && (self.next, self.from) == (i, k)
            && let Some(oi) = (self.checked()).and_then(|c| c.get(i)?.indexes.get(k).copied())
        {
            self.start_picked = Some((oi, 0));
        }
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
            (self.zeroed, self.start_moved) = (false, None);
            // A Z set before is forgotten: Cycle Start waits for this
            // start's, at a tool change or a start at a step (LIO-618).
            self.forget_touch();
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
                // Before a start at a step, skipped or not (LIO-583 review).
                _ if start.is_some_and(|n| s < n) => StepStage::Before,
                _ if skipped.contains(&s) => StepStage::Cut,
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
                // Which they are, for the work zero of a start at a step
                // (`Run::partly`, LIO-582).
                sim.steps_cut = (steps.iter().copied())
                    .filter(|&s| s < found.steps.len())
                    .collect();
                sim.steps_cut.sort_unstable();
                sim.steps_cut.dedup();
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
            // The tool went down their columns in the run that cut them: a
            // later step's air link may come down one as far as it went
            // (`air::relink`), which the export's check proves by that
            // (LIO-565).
            sim.went = (steps.iter().filter_map(|&s| found.steps.get(s)))
                .flat_map(|s| &sim.plan.moves[s.moves.clone()])
                .filter(|m| m.a.is_none())
                .flat_map(|m| [m.from, m.to])
                .collect();
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
    /// `fingerprint` with what the cutter takes away along the moves: its
    /// shape and sizes, and the A of each move (`cut_print`). What a run
    /// cut is the check's only while this is the same (`Run::cut_in_run`,
    /// LIO-582 review). Kept on disk with what runs cut and what was taken
    /// as cut (`CutHistory`, LIO-594, LIO-611).
    pub cut_print: u64,
}

impl OpSteps {
    /// `cut_print` with how many steps the plan is split into: what was
    /// taken as cut by step number holds only while both are the same, as
    /// a journal holds only for as many steps (`Run::take_journal`), so that
    /// an AXIS CAM that splits a plan otherwise never takes other cuts by
    /// the old numbers (`KeptDeclared::prints`, LIO-611 review).
    pub fn declared_print(&self) -> u64 {
        let mut h = self.cut_print;
        for b in (self.steps.len() as u64).to_le_bytes() {
            h = (h ^ u64::from(b)).wrapping_mul(0x100000001b3);
        }
        h
    }

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
                cut_print: cut_print(plan, &op.tool),
            })
        })
        .collect()
}

/// `fingerprint` of `plan` and `tool` with what the cutter takes away along
/// the moves: its shape, radius (and form), flute length, the reach below
/// its tip and its neck, a lathe plan's rings, and the A each move is at.
/// A tool saved at another size, or an operation posed at another angle,
/// cuts other stock along the same moves (LIO-582 review). Kept on disk
/// (`CutHistory`), its words for the cutter (`Debug`) may still change
/// between builds: an AXIS CAM updated since then finds another print, and
/// what was cut or taken as cut no longer counts as the check's, the safe
/// side (LIO-611).
fn cut_print(plan: &crate::simulation::Plan, tool: &str) -> u64 {
    let mut h = fingerprint(plan, tool);
    let mut eat = |w: u64| h = (h ^ w).wrapping_mul(0x100000001b3);
    let cutter = format!(
        "{:?} {:?} {:?} {:?} {:?} {:?} {:?}",
        plan.shape, plan.radius, plan.form, plan.flute_length, plan.below, plan.neck, plan.rings
    );
    for b in cutter.bytes() {
        eat(u64::from(b));
    }
    for m in &plan.moves {
        eat(m.a.map_or(u64::MAX, |a| u64::from(a.to_bits())));
    }
    h
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

/// Why a run's check refused while the project's part is not loaded yet,
/// after AXIS CAM opened say (`App::check_run`).
pub(crate) const PART_NOT_LOADED: &str = "Load the imported part before running a setup.";

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
    let mut earlier = vec![];
    if let Some(oi) = last {
        crate::simulation::sequence_with(
            doc,
            mesh,
            Some((si, oi)),
            |s, oi, r| {
                if s == si {
                    sims.push((oi, r))
                } else if let (Ok(sim), Some(setup)) = (&r, doc.setups.get(s))
                    && let Some(op) = setup.operations.get(oi)
                {
                    let cut = cut_print(&sim.plan, &op.tool);
                    let steps = OpSteps {
                        op: oi,
                        clear: 0.,
                        hole: false,
                        steps: plan_steps(&sim.plan),
                        fingerprint: 0,
                        cut_print: cut,
                    };
                    earlier.push(EarlierOp {
                        setup: setup.id.clone(),
                        op: oi,
                        cut,
                        marked: steps.declared_print(),
                        name: format!("{} ({})", op.name(), setup.name()),
                    });
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
            earlier,
            plan: doc.setups.iter().map(|s| s.id.clone()).collect(),
            ops: (doc.setups[si].operations.iter())
                .map(|o| o.id.clone())
                .collect(),
        },
    )
}

/// Operation `oi` (index in the setup) of a check whose operations are
/// `ids` (`Left::ops`): by its id, or by its index when the check gave none.
fn op_key(ids: &[String], oi: usize) -> String {
    ids.get(oi).cloned().unwrap_or_else(|| format!("#{oi}"))
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
    /// Each operation of the setups before it the check simulated: the
    /// setup's id, the operation's index in it, and its plan and cutter's
    /// print (`OpSteps::cut_print`). What earlier setups' runs cut this
    /// session is the stock's only while this has it (`Run::earlier_cuts`,
    /// LIO-582 review), and an operation of a setup whose runs cut is only
    /// while they cut it or took it as cut (`Run::earlier_uncut`, LIO-603).
    pub earlier: Vec<EarlierOp>,
    /// The plan's setups by id, in order, as checked: where a setup whose
    /// cut this does not have is now (`Run::earlier_unfollowed_how`).
    pub plan: Vec<String>,
    /// The setup's operations by id, in order, as checked: what a start
    /// follows over an edit that numbers them otherwise (`Run::checked_as`,
    /// LIO-636 review).
    pub ops: Vec<String>,
}

/// An operation of a setup before a run's the check simulated (`Left::earlier`).
#[derive(Debug, Clone, PartialEq)]
pub struct EarlierOp {
    /// Its setup's id.
    pub setup: String,
    /// Its index in the setup.
    pub op: usize,
    /// The print of its plan and cutter (`OpSteps::cut_print`).
    pub cut: u64,
    /// That with how many steps the plan is split into
    /// (`OpSteps::declared_print`): a mark of it Already cut holds only
    /// for this (`KeptDeclared::prints`, `Run::earlier_taken`).
    pub marked: u64,
    /// Its name and its setup's, as the Run panel says them.
    pub name: String,
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
    /// The setup's work zero, one of `post::ORIGINS` (`Checked::origin`),
    /// and where it is in the plan (`Checked::datum`): the tablet records
    /// them at the first tool run's start and refuses a later Cycle Start
    /// once the work zero moved in the project since, as the Run panel
    /// does (`Run::zero`, LIO-617). Null when it cannot run, and in a file
    /// from before.
    #[serde(default)]
    pub origin: Option<String>,
    #[serde(default)]
    pub datum: Option<[f32; 3]>,
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
    /// (LIO-548). For the first tool run, the setup's start, its work zero
    /// on the stock earlier setups left, only where the computer refuses
    /// the touch-off on the stock top or a probe spot there, an earlier
    /// setup's cut maybe reaching under them (`tablet_start_zero`,
    /// LIO-584); null otherwise, and in a file from before.
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
/// `mesh` is the part, or why it could not be loaded. Their work zeros go
/// by what the setups' runs cut this session, `session`, as the Run panel's
/// do with the setup picked (`Run::picked_in`, LIO-590).
pub fn tablet_runs(
    doc: &Document,
    mesh: Result<&Mesh, &str>,
    si: usize,
    session: &Session,
) -> TabletSetup {
    let setup = &doc.setups[si];
    let profile = crate::model::get(&setup.machine.p, "uc100_profile");
    let mut out = TabletSetup {
        setup_id: setup.id.clone(),
        name: setup.name().to_string(),
        machine_profile: (profile.rsplit(['/', '\\']).next())
            .filter(|n| !n.trim().is_empty())
            .map(|n| n.trim().to_string()),
        work_offset: String::new(),
        origin: None,
        datum: None,
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
    // Each start's work zero as the Run panel finds it started there, the
    // setup picked with this session's cuts (LIO-590); the jobs are the
    // plan's alone.
    let mut started = Run::new(doc, si, 0);
    started.given(session, doc);
    started.checked_as(all.clone(), Some(left), None);
    // Its programs are made for an earlier setup's operation cut whole that
    // the computer's runs did not certainly cut so: the tablet's Cycle
    // Start refuses them as the Run panel's waits (LIO-602).
    let uncut = started.earlier_not_whole_for(doc, true);
    let orient = crate::orient::Orient::of(&setup.machine);
    // The setup's start's, on the stock earlier setups left (LIO-584).
    let inherited = crate::model::get(&setup.p, "stock_source") != "fresh";
    let mut start_zero = tablet_start_zero(&started, orient, inherited);
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
    if let Some(why) = uncut {
        out.problems = vec![why];
        return out;
    }
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
        let step_starts = tablet_step_starts(doc, mesh, si, session, &run, p, lift, &actions);
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
            zero: if i > 0 { zero(i, 0) } else { start_zero.take() },
        });
    }
    // One work zero for the whole setup: each tool run's `Checked` has it.
    (out.origin, out.datum) = programs
        .first()
        .map_or((None, None), |p| (Some(p.origin.clone()), Some(p.datum)));
    out
}

/// The work zero of `run`, started partway (`Run::partway`), on the stock
/// the operations before its start left (`TabletZero`), on a machine of
/// orientation `orient`; None when it does not start partway.
fn tablet_zero(run: &Run, orient: crate::orient::Orient) -> Option<TabletZero> {
    if !run.partway() {
        return None;
    }
    let partly = run.partly();
    zero_said(
        run,
        orient,
        partly.as_ref().map_or_else(Vec::new, |x| x.cut.clone()),
        partly.as_ref().is_some_and(|x| x.top_cut),
        partly.as_ref().map_or(&[][..], |x| &x.probe_from[..]),
    )
}

/// The work zero of `run` from the setup's start (not `Run::partway`) on
/// the stock earlier setups left, `inherited` saying the setup's stock is
/// theirs (its `stock_source`), when the computer refuses the touch-off
/// on the stock top there (`Run::touch_off` None: `INHERITED_UNKNOWN`, or
/// `STOCK_TOP_CUT`) or a probe spot (`Checked::probe_from`, as
/// `Connection::probe_spots` has them then), an earlier setup's cut maybe
/// reaching under them (LIO-584); or this session's cuts the check does
/// not have make the Run panel refuse them (`Run::on_cut_stock`, LIO-590).
/// None otherwise: on fresh stock, as no earlier setup of the plan cut it
/// (its spots refused for a jaw at most), and for a zero not on the stock,
/// which no touch-off on its top finds; but for this session's cuts.
fn tablet_start_zero(
    run: &Run,
    orient: crate::orient::Orient,
    inherited: bool,
) -> Option<TabletZero> {
    if run.partway() {
        return None;
    }
    let p = run.checked()?.first()?;
    // This session's cuts the check does not have, of this setup's runs or
    // another's (`Run::on_cut_stock`): the Run panel's Work zero goes by
    // them from the setup's start too, on fresh stock or not, and with a
    // zero not on the stock top (A axis at stock end), whose touch-off it
    // refuses as well (LIO-590 review).
    if run.on_cut_stock() {
        let partly = run.partly();
        return zero_said(
            run,
            orient,
            partly.as_ref().map_or_else(Vec::new, |x| x.cut.clone()),
            partly.as_ref().is_none_or(|x| x.top_cut),
            partly.as_ref().map_or(&[][..], |x| &x.probe_from[..]),
        );
    }
    crate::post::references(&p.origin)?;
    if !inherited {
        return None;
    }
    let refused = run.touch_off().is_none() || p.probe_from.iter().any(|(_, ok)| ok.is_err());
    if !refused {
        return None;
    }
    // Earlier setups' cuts below the stock top over it, which a touch plate
    // laid on it may rest over (`Partly::top_cut` of the setup's first
    // operation, from them alone).
    let top_cut = p.partly.first().is_some_and(|x| x.top_cut);
    zero_said(run, orient, vec![], top_cut, &p.probe_from)
}

/// `TabletZero` of `run` as its Work zero tab says it: the work Z its
/// touch-off sets, or why none (`Run::partway_blocked`), with `cut`,
/// `top_cut` and the probe spots `spots` of the stock it starts from.
fn zero_said(
    run: &Run,
    orient: crate::orient::Orient,
    cut: Vec<String>,
    top_cut: bool,
    spots: &[([crate::post::At; 2], Result<(), String>)],
) -> Option<TabletZero> {
    let p = run.checked()?.first()?;
    let blocked = run.tablet_blocked();
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
        cut,
        top_cut,
        top: (p.stock.max[2] - p.datum[2]) as f64,
        spot: (crate::post::references(&p.origin))
            .map(|refs| crate::control::touch_spot(orient, refs)),
        probe_from: (spots.iter())
            .map(|([x, y], ok)| TabletSpot {
                x: *x,
                y: *y,
                problem: ok.clone().err().map(|why| run.for_tablet(why)),
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
#[allow(clippy::too_many_arguments)]
fn tablet_step_starts(
    doc: &Document,
    mesh: &Mesh,
    si: usize,
    session: &Session,
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
            match tablet_step_job(doc, mesh, si, session, p.chuck.is_some(), oi, n) {
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
    session: &Session,
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
    started.given(session, doc);
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
            touched_for: None,
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

    /// The tablet file carries the setup's work zero (LIO-617), the one
    /// the Run panel records at the first tool run's start (`Run::zero`):
    /// its name and where it is in the plan, the same for every tool run.
    /// Moved in the project, the file says so, as the check that refuses a
    /// later tool run there finds it. A setup that cannot run has none,
    /// and a file from before reads as none.
    #[test]
    fn the_tablet_file_carries_the_setups_work_zero() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("template.toml");
        std::fs::write(&profile, uc100::config::TEMPLATE).unwrap();
        let (mut doc, mesh) = tablet_fixture(&profile);
        let file = |doc: &Document| {
            let zip = crate::bundle::pack(doc, &Session::default()).unwrap();
            let mut zip = zip::ZipArchive::new(std::io::Cursor::new(zip)).unwrap();
            let mut runs: crate::bundle::Runs =
                serde_json::from_reader(zip.by_name(crate::bundle::RUNS).unwrap()).unwrap();
            runs.setups.remove(0)
        };
        let s = file(&doc);
        assert!(s.problems.is_empty(), "{:?}", s.problems);
        let all = programs(&doc, &mesh, 0).unwrap();
        assert_eq!(all.len(), 2);
        for p in &all {
            assert_eq!(s.origin.as_deref(), Some(p.origin.as_str()));
            assert_eq!(s.datum, Some(p.datum));
        }
        assert_eq!(s.origin.as_deref(), Some("Stock top center"));
        // Moved to the stock's front-left corner: X and Y move, Z stays.
        (doc.setups[0].p).insert("origin".into(), "Stock top front-left".into());
        let moved = file(&doc);
        assert!(moved.problems.is_empty(), "{:?}", moved.problems);
        let p = &programs(&doc, &mesh, 0).unwrap()[0];
        assert_eq!(moved.origin.as_deref(), Some("Stock top front-left"));
        assert_eq!(moved.datum, Some(p.datum));
        let (old, new) = (s.datum.unwrap(), moved.datum.unwrap());
        assert!(
            (0..2).all(|a| (old[a] - new[a]).abs() > 1e-3),
            "{old:?} {new:?}"
        );
        assert!((old[2] - new[2]).abs() < 1e-6, "{old:?} {new:?}");
        // As JSON, the computer's names; none in a setup that cannot run.
        let json = serde_json::to_value(&moved).unwrap();
        assert_eq!(json["origin"], "Stock top front-left");
        assert_eq!(json["datum"].as_array().map(Vec::len), Some(3));
        let none = tablet_runs(&doc, Err("no part"), 0, &Session::default());
        assert_eq!((none.origin, none.datum), (None, None));
        let mut json = serde_json::to_value(&s).unwrap();
        let o = json.as_object_mut().unwrap();
        o.remove("origin");
        o.remove("datum");
        let before: TabletSetup = serde_json::from_value(json).unwrap();
        assert_eq!((before.origin, before.datum), (None, None));
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
        let zip = crate::bundle::pack(&doc, &Session::default()).unwrap();
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
        let s = tablet_runs(&doc, Ok(&mesh), 0, &Session::default());
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
        let s = tablet_runs(&doc, Ok(&mesh), 0, &Session::default());
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
        let s = tablet_runs(&doc, Ok(&mesh), 0, &Session::default());
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
        // Face's pass 2, Pocket's levels and Drilling's holes after the
        // first: each is touched off at the stock top's centre, on what the
        // steps before it left there, as the check carved them (LIO-582).
        assert!(zeros.len() >= 9, "{}", zeros.len());
        for (oi, n, z) in &zeros {
            assert_eq!(z.spot.as_deref(), Some("the centre"), "{oi} {n}");
            assert!(
                z.top_cut && z.cut.first().is_some_and(|c| c == "top face"),
                "{oi} {n} {z:?}"
            );
            assert_eq!(z.set_z.is_some(), z.blocked.is_none(), "{oi} {n} {z:?}");
        }
        // Below the stock top by as much as the floor is (`TabletZero::top`
        // is the top's work Z).
        let below = |z: &TabletZero| z.set_z.map(|s| (z.top - s) as f32);
        let top = (whole.as_ref().as_ref().unwrap())[0].stock.max[2];
        let floor = |oi: usize, n: usize| {
            let s = left.steps.iter().find(|s| s.op == oi).unwrap();
            top - s.steps[n].z.unwrap()
        };
        let near = |a: Option<f32>, b: f32| a.is_some_and(|a| (a - b).abs() < 1e-3);
        // Face's pass 2 on pass 1's floor.
        let face = &zeros.iter().find(|z| (z.0, z.1) == (0, 1)).unwrap().2;
        assert!(near(below(face), floor(0, 0)), "{face:?}");
        // Drilling's holes on the pocket's floor, which no hole drilled
        // before cuts there: the floor of its last level.
        let pocket = left.steps.iter().find(|s| s.op == 1).unwrap().steps.len();
        let holes: Vec<_> = zeros.iter().filter(|z| z.0 == 2).collect();
        assert!(holes.len() >= 4, "{}", holes.len());
        for (_, n, z) in holes {
            assert!(near(below(z), floor(1, pocket - 1)), "{n} {z:?}");
        }
        // Pocket's levels have none: within the end mill's reach of the
        // centre's spot the pocket leaves stock standing beside the level's
        // floor, which the tool touched off there may rest on.
        for (_, n, z) in zeros.iter().filter(|z| z.0 == 1) {
            assert_eq!(z.set_z, None, "{n}");
            assert!(
                z.blocked
                    .as_deref()
                    .is_some_and(|b| b.contains("cannot tell"))
            );
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
            steps: vec![],
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
                    cut_print: 0,
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
            cut_in_run: vec![],
            stock_unconfirmed: false,
            away: None,
            declared_lost: false,
            declared_stale: false,
            declared_from: None,
            earlier_cuts: vec![],
            earlier_given: vec![],
            earlier_elsewhere: vec![],
            project_setups: vec![],
            earlier_own: vec![],
            earlier_taken: vec![],
            before_follow: None,
            running: Some(Running {
                index: 0,
                from: 0,
                seq: 0,
                offset: [0.; 6],
                before: Before {
                    xy: None,
                    zero: None,
                    last: None,
                    touched_for: None,
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
            touched_for: None,
            answered: false,
            known_wcs: None,
            reference_wcs: None,
            rechecked_tool: None,
            starts: 0,
            outcome: None,
            pictures: vec![],
            left: None,
            simulated_earlier: vec![],
            plan_setups: vec![],
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
            start_picked: None,
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
            journal_own: false,
            follow: None,
            followed: None,
            cut_note: None,
            rows_from_skips: false,
            last_passed: None,
            last_passed_ops: vec![],
            start_moved: None,
        };
        let (_, op, block) = run.progress(Some(3)).unwrap();
        assert_eq!((op, block), (0, 2));
        // The outcome after the start ends the tool run and moves on.
        let o = |seq, ok| uc100::controller::ProgramOutcome {
            seq,
            ok,
            message: "m".into(),
        };
        run.refresh(Some(&o(0, true)), None);
        assert!(run.running.is_some());
        run.refresh(Some(&o(1, true)), None);
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
                touched_for: None,
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
        let zip = crate::bundle::pack(&doc, &Session::default()).unwrap();
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
        let tablet = tablet_runs(&doc, Ok(&mesh), 0, &Session::default());
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
        let zip = crate::bundle::pack(&doc, &Session::default()).unwrap();
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
        // Its run cut this session, its zero not on the stock top (A axis at
        // stock end): the Run panel refuses the touch-off until it is said
        // whether the same part is held, and so does the tablet file, which
        // carries the refusal though no stock-top spot (LIO-590 review).
        assert!(crate::indexed::ORIGINS.contains(&crate::model::get(&doc.setups[0].p, "origin")));
        let (c, l) = check(&doc, &mesh, 0);
        let mut cut = Run::new(&doc, 0, 0);
        cut.checked_as(Arc::new(c), Some(Arc::new(l)), None);
        cut.note_cut(0, None);
        let session = vec![(doc.setups[0].id.clone(), cut.cut_in_run.clone())];
        let mut picked = Run::new(&doc, 0, 0);
        picked.picked_in(&session, false, &[]);
        let (c, l) = check(&doc, &mesh, 0);
        picked.checked_as(Arc::new(c), Some(Arc::new(l)), None);
        assert!(picked.touch_off().is_none());
        assert_eq!(
            picked.partway_blocked().as_deref(),
            Some(STOCK_UNCONFIRMED_BLOCKED)
        );
        let session = Session {
            cuts: session,
            ..Session::default()
        };
        let zero = tablet_runs(&doc, Ok(&mesh), 0, &session).tool_runs[0]
            .zero
            .clone();
        let zero = zero.expect("the setup's start carries the refusal");
        assert_eq!((zero.set_z, zero.spot), (None, None));
        assert_eq!(
            zero.blocked,
            Some(for_tablet(STOCK_UNCONFIRMED_BLOCKED.into()))
        );
        assert!(
            zero.blocked
                .as_ref()
                .is_some_and(|b| !b.contains("Same part"))
        );
        assert_eq!(
            tablet_runs(&doc, Ok(&mesh), 0, &Session::default()).tool_runs[0].zero,
            None
        );
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
        let tablet = tablet_runs(&doc, Ok(&mesh), 0, &Session::default());
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
        // Started at each level, its levels before taken as cut, and with
        // each level skipped, the program passes the export's check: a
        // level's air link comes down the column the levels before cleared,
        // where the tool went in the run that cut them.
        for s in 0..7 {
            let start: Vec<usize> = (0..s).collect();
            for cut in [start, vec![s]] {
                if cut.is_empty() {
                    continue;
                }
                let (checked, _) = check_steps(&doc, &mesh, 0, &[], &[(0, cut.clone())]);
                assert!(checked.is_ok(), "{cut:?}: {:?}", checked.err());
            }
        }
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
                assert_eq!(
                    marks[next].unwrap().1,
                    s + 1,
                    "block {b} said level {s} done"
                );
            }
        }
        // The tablet file: a step a level, each ending somewhere to stop.
        // Its start at each level after the first says plainly why it is
        // not there: in the chuck, the tablet starts a tool run only at its
        // first point, the way in past the jaws it checked (LIO-476,
        // LIO-567).
        let tablet = tablet_runs(&doc, Ok(&mesh), 0, &Session::default());
        assert!(tablet.problems.is_empty(), "{:?}", tablet.problems);
        let starts: Vec<_> = (tablet.tool_runs.iter())
            .flat_map(|r| &r.step_starts)
            .filter(|st| st.operation == 0)
            .collect();
        let at: Vec<usize> = starts.iter().map(|st| st.step).collect();
        assert_eq!(at, (1..7).collect::<Vec<_>>());
        for st in &starts {
            assert_eq!(st.problems, [CHUCK_STARTS], "Level {}", st.step + 1);
        }
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
    /// starts at Pass 2 or skips Pass 1, and touches off on the floor the
    /// pass taken as cut left (LIO-582).
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
        assert!(skipped.top_cut && !skipped.cut.is_empty(), "{skipped:?}");
        let centre = (skipped.probe_from.iter()).find(|(r, _)| *r == p.probe.clone().unwrap());
        assert!(centre.is_some_and(|(_, ok)| ok.is_err()), "{centre:?}");
        // What cut it is named as the steps taken as cut, in the probe
        // spots' reasons too, not operations (LIO-568).
        for r in [&first, &second, &run] {
            assert_eq!(r.cut_by(), "the steps taken as cut");
            for (_, ok) in &r.partly().unwrap().probe_from {
                let why = ok.as_ref().unwrap_err();
                assert!(!why.contains("operations"), "{why}");
            }
        }
        // The touch-off sets Z on the floor the pass taken as cut left
        // (LIO-582): Pass 1's, or Pass 2's when it is the one taken as cut.
        let floor = |n: usize| face_steps.steps[n].z.unwrap();
        assert!(floor(1) < floor(0) && floor(0) < p.stock.max[2] - 0.1);
        assert_eq!(skipped.surface, Some(floor(0)));
        assert_eq!(later.surface, Some(floor(1)));
        assert_eq!((&later.cut, later.top_cut), (&started.cut, started.top_cut));
        let set_z = |r: &Run| r.touch_off().map(|(_, w)| w[2]);
        let want = |n: usize| Some((floor(n) - p.datum[2]) as f64);
        assert_eq!((set_z(&first), set_z(&second)), (want(0), want(0)));
        assert_eq!((set_z(&run), run.partway_blocked()), (want(1), None));
        // A check that carved other steps than the run takes as cut is not
        // the run's yet (its start changed since): no Z from it.
        let (_, mut stale) = partly(1, true);
        let (carved, carved_left) = check_steps(&doc, &mesh, 0, &[], &[(0, vec![0])]);
        stale.checked_as(Arc::new(carved), Some(Arc::new(carved_left)), None);
        assert_eq!(stale.all_cut(), [(0, vec![1])]);
        let unknown = stale.partly().unwrap();
        assert_eq!((unknown.surface, unknown.top_cut), (None, true));
        assert_eq!(set_z(&stale), None);
        assert_eq!(unknown.cut, [STEPS_UNFOLLOWED]);
        assert_eq!(
            stale.partway_blocked().as_deref(),
            Some(
                "The check does not follow the stock cut by the steps taken as cut, so it cannot tell the surface they left where the stock top is touched off."
            )
        );
        // Starting at Pass 1, Cycle Start waits for the work zero, kept or
        // set again.
        assert!(!run.zeroed);
        assert_eq!((run.next, run.from, run.at_step()), (0, 0, None));
    }

    /// Steps of an operation after the start's taken as cut (LIO-582
    /// review): Pocket's level 1 cut the centre Face's pass 2 would touch
    /// off on, below pass 1's floor, which no stock the check found for the
    /// start has. The touch-off sets no Z and the probe finds no spot,
    /// saying why, whether the run starts at a step or at an operation.
    #[test]
    fn a_start_with_a_later_operations_steps_taken_as_cut_sets_no_z() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("template.toml");
        std::fs::write(&profile, uc100::config::TEMPLATE).unwrap();
        let (doc, mesh) = tablet_fixture(&profile);
        let (whole, left) = check_steps(&doc, &mesh, 0, &[], &[]);
        let (whole, left) = (Arc::new(whole), Arc::new(left));
        let recheck = |run: &mut Run| {
            let (c, l) = check_steps(&doc, &mesh, 0, &[], &run.all_cut());
            run.checked_as(Arc::new(c), Some(Arc::new(l)), None);
        };
        let said = "Operations or steps after the start are taken as cut, and the check does not follow the stock they cut, so it cannot tell the surface where the stock top is touched off.";
        let blocked = |run: &Run| {
            assert!(run.partway());
            let partly = run.partly().unwrap();
            assert_eq!(partly.cut, [LATER_UNFOLLOWED]);
            assert_eq!((partly.surface, partly.top_cut), (None, true));
            assert!(!partly.probe_from.is_empty());
            assert!(
                (partly.probe_from.iter())
                    .all(|(_, ok)| ok.as_ref().is_err_and(|e| e == LATER_UNFOLLOWED_WHY))
            );
            assert_eq!(run.touch_off(), None);
            assert_eq!(run.partway_blocked().as_deref(), Some(said));
        };
        // At Face's pass 2, Pocket's level 1 taken as cut.
        let mut run = Run::new(&doc, 0, 0);
        run.checked_as(whole.clone(), Some(left.clone()), None);
        run.pick(1, 0, false);
        assert!(run.skip_steps(true));
        recheck(&mut run);
        run.pick(0, 1, false);
        assert!(run.start_step());
        recheck(&mut run);
        assert_eq!(run.all_cut(), [(0, vec![0]), (1, vec![0])]);
        assert_eq!(run.at_step(), Some((0, 1)));
        blocked(&run);
        // At Pocket, Drilling's hole 1 taken as cut.
        let mut run = Run::new(&doc, 0, 0);
        run.checked_as(whole.clone(), Some(left.clone()), None);
        run.start_at(0, 1);
        run.pick(2, 0, false);
        assert!(run.skip_steps(true));
        recheck(&mut run);
        assert_eq!(
            (run.next, run.from, run.all_cut()),
            (0, 1, vec![(2, vec![0])])
        );
        blocked(&run);
        // From the setup's very start, Face's pass 1, with Pocket's level 1
        // taken as cut, or the whole Pocket skipped as cut: partway, so
        // Cycle Start waits for the work zero, kept or set by hand, and the
        // touch-off sets no Z and Zero Z says why (LIO-582 review, LIO-583).
        let at_start = |run: &Run, cut: &str| {
            assert_eq!((run.next, run.from, run.at_step()), (0, 0, None));
            assert!(run.partway() && run.later_cut());
            assert_eq!(run.touch_off(), None);
            assert_eq!(run.partway_blocked().as_deref(), Some(said));
            assert_eq!(run.taken_as_cut(), [cut]);
            blocked(run);
        };
        let mut run = Run::new(&doc, 0, 0);
        run.checked_as(whole.clone(), Some(left.clone()), None);
        run.pick(1, 0, false);
        assert!(run.skip_steps(true));
        recheck(&mut run);
        let pocket = run.op_steps(1).unwrap().label(0).to_lowercase();
        at_start(&run, &format!("Pocket {pocket}"));
        let mut run = Run::new(&doc, 0, 0);
        run.checked_as(whole.clone(), Some(left.clone()), None);
        assert!(run.skip(1, Some(Skip::Cut)));
        let (c, l) = check_steps(&doc, &mesh, 0, &run.skips, &run.all_cut());
        run.checked_as(Arc::new(c), Some(Arc::new(l)), None);
        at_start(&run, "Pocket");
        // With nothing after it taken as cut, the stock top as ever, and
        // the setup's own start.
        assert!(run.skip(1, None));
        let (c, l) = check_steps(&doc, &mesh, 0, &run.skips, &run.all_cut());
        run.checked_as(Arc::new(c), Some(Arc::new(l)), None);
        assert!(!run.later_cut() && !run.partway() && run.touch_off().is_some());
        assert_eq!(run.partway_blocked(), None);
        assert!(run.taken_as_cut().is_empty());
        // Pocket's level 1 taken as cut, a start at its level 2, then Face's
        // Start here: the skip outlives the start at a step, so the run from
        // the setup's start still has Pocket's level 1 taken as cut and
        // waits, setting no Z (LIO-583 review).
        let mut run = Run::new(&doc, 0, 0);
        run.checked_as(whole.clone(), Some(left.clone()), None);
        run.pick(1, 0, false);
        assert!(run.skip_steps(true));
        recheck(&mut run);
        assert!(run.start_at_step(1, 1));
        recheck(&mut run);
        assert_eq!(
            (run.at_step(), run.all_cut()),
            (Some((1, 1)), vec![(1, vec![0])])
        );
        run.start_at(0, 0);
        recheck(&mut run);
        assert_eq!((run.start_step, run.all_cut()), (None, vec![(1, vec![0])]));
        assert!(run.asks_zero() && !run.zeroed);
        at_start(&run, &format!("Pocket {pocket}"));
        // At Face's pass 2 alone, it has pass 1's floor.
        let mut run = Run::new(&doc, 0, 0);
        run.checked_as(whole, Some(left), None);
        run.pick(0, 1, false);
        assert!(run.start_step());
        recheck(&mut run);
        assert!(run.touch_off().is_some() && run.partway_blocked().is_none());
    }

    /// A ring 24 mm across and 10 high round a 16 mm bore, written as an
    /// STL in `dir` and loaded, with a Ø10 and a Ø2 flat end mill (LIO-582
    /// review): the part a bore through the stock, or opened from the other
    /// side, is cut in.
    fn ring(dir: &std::path::Path) -> (std::path::PathBuf, Mesh, [crate::model::Tool; 2]) {
        let tris =
            crate::cam::tests::revolved(&[(8., 0.), (12., 0.), (12., 10.), (8., 10.), (8., 0.)]);
        let path = dir.join("ring.stl");
        let mut stl = String::from("solid ring\n");
        for tri in &tris {
            stl.push_str("facet normal 0 0 0\nouter loop\n");
            for p in tri {
                stl.push_str(&format!("vertex {} {} {}\n", p[0], p[1], p[2]));
            }
            stl.push_str("endloop\nendfacet\n");
        }
        stl.push_str("endsolid ring\n");
        std::fs::write(&path, stl).unwrap();
        let mesh = crate::mesh::load(&path).unwrap();
        let tool = |name: &str, d: &str, n: &str| {
            let mut t = crate::model::Tool::new(name, "Flat end mill", d);
            for (k, v) in [("length", "40"), ("stickout", "45"), ("number", n)] {
                t.p.insert(k.into(), v.into());
            }
            t
        };
        (
            path,
            mesh,
            [tool("Mill 10", "10", "1"), tool("Mill 2", "2", "2")],
        )
    }

    /// The ring's (`ring`) faces at the top of setup `setup`'s stock, up
    /// (`up`) or down: what its Drill and Face operations cut.
    fn ring_top(mesh: &Mesh, setup: &crate::model::Setup, up: bool) -> crate::surface::Selection {
        let placement = crate::stock::Placement::new(mesh, Some(setup));
        let z = placement.bounds.max[2];
        let ids: Vec<usize> = (mesh.surfaces.iter().enumerate())
            .filter(|(_, s)| {
                (if up {
                    s.normal[2] > 0.99
                } else {
                    s.normal[2] < -0.99
                }) && (placement.transform(s.center)[2] - z).abs() < 0.01
            })
            .map(|(i, _)| i)
            .collect();
        assert!(!ids.is_empty());
        crate::surface::Selection {
            mesh_fingerprint: mesh.fingerprint.clone(),
            ids,
        }
    }

    /// The ring's bore helically milled through with the Ø10 end mill,
    /// breaking through by `breakthrough` mm.
    fn ring_bore(
        tool: &crate::model::Tool,
        top: crate::surface::Selection,
        breakthrough: &str,
    ) -> crate::model::Operation {
        let mut bore = crate::model::Operation::new(crate::model::Strategy::Drill, tool.id.clone());
        bore.geometry.clear();
        bore.surfaces = Some(top);
        for (k, v) in [
            ("cycle", crate::model::HELICAL_MILLING),
            ("through", "Yes"),
            ("breakthrough", breakthrough),
            ("coolant", "Off"),
            ("stepdown", "2"),
        ] {
            bore.p.insert(k.into(), v.into());
        }
        bore
    }

    /// The ring's top faced with `tool` over the stock's silhouette.
    fn ring_face(
        tool: &crate::model::Tool,
        top: crate::surface::Selection,
    ) -> crate::model::Operation {
        let mut face = crate::model::Operation::new(crate::model::Strategy::Face, tool.id.clone());
        face.geometry.clear();
        face.surfaces = Some(top);
        for (k, v) in [
            ("boundary", "Stock silhouette"),
            ("containment", "Tool outside boundary"),
            ("allowance", "0"),
            ("coolant", "Off"),
        ] {
            face.p.insert(k.into(), v.into());
        }
        face
    }

    /// A ring setup (`ring`) on the tablet fixture's machine, its stock
    /// flush with the part but for `top` and `bottom` mm, its work zero
    /// Stock top center.
    fn ring_setup(doc: &Document, offset: &str, top: &str, bottom: &str) -> crate::model::Setup {
        let mut setup = crate::model::Setup::new(doc.setups[0].machine.clone(), offset);
        for (k, _) in crate::stock::OFFSETS {
            setup.p.insert(k.into(), "0".into());
        }
        setup.p.insert("stock_top".into(), top.into());
        setup.p.insert("stock_bottom".into(), bottom.into());
        setup.p.insert("origin".into(), "Stock top center".into());
        setup
    }

    /// Setup 1 faces 10 mm off the fixture block's stock top in 1 mm
    /// levels; setup 2, held the same way, pockets, contours and drills it
    /// (LIO-602). Setup 2's programs are made from a simulation
    /// that cuts setup 1's face whole: stopped after its first level, 9 mm
    /// of the stock top is still there, and setup 2's rapids, which clear
    /// the check's stock, go through it. So setup 2's Cycle Start waits
    /// while setup 1's runs did not certainly cut the face whole with the
    /// plan it has now: stopped partway, stopped where it is not known, or
    /// cut from another plan. Not once a tool run of it ends done, nor for a
    /// face the check leaves out (disabled since), which no rapid runs into.
    #[test]
    fn a_later_setup_waits_for_an_earlier_operation_cut_partway_or_changed() {
        let (mut doc, mesh) = crate::post::fixture::job();
        // On the UC100 the Run panel drives.
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("template.toml");
        std::fs::write(&profile, uc100::config::TEMPLATE).unwrap();
        let machine = tablet_fixture(&profile).0.setups[0].machine.clone();
        doc.machines = vec![machine.clone()];
        let mut first = doc.setups[0].clone();
        first.machine = machine;
        first.p.insert("stock_top".into(), "10".into());
        let mut second = crate::model::Setup::new(first.machine.clone(), "G55");
        second.p = first.p.clone();
        second.p.insert("name".into(), "Top again".into());
        // Its Pocket, Contour and Drill.
        second.operations = first.operations[1..4].to_vec();
        first.operations.truncate(1);
        first.operations[0].p.insert("stepdown".into(), "1".into());
        doc.setups = vec![first, second];
        doc.sync_stock_sources();
        // Setup 1's face, its levels its steps.
        let sims = simulated(&doc, &mesh, 0);
        let steps = op_steps(&doc, 0, &sims);
        let face_steps = steps.iter().find(|s| s.op == 0).unwrap();
        let levels = &face_steps.steps;
        assert_eq!(levels.len(), 10);
        let face = sims[0].1.as_ref().unwrap();
        // The stock it leaves stopped after its first level.
        let mut real = face.initial_volume.clone().unwrap();
        let plan = &face.plan;
        for m in (plan.moves[levels[0].moves.clone()].iter())
            .filter(|m| m.motion == crate::simulation::Motion::Feed)
        {
            let frame = face.frame_at(m.a).unwrap();
            let (r, flute) = (plan.cut_radius(), plan.flute_length);
            real.cut(m.from, m.to, r, flute, plan.shape, frame, face.source);
        }
        // Setup 2's programs are made for the face cut whole.
        let (programs, left) = check(&doc, &mesh, 1);
        assert!(programs.is_ok(), "{:?}", programs.err());
        let checked = (Arc::new(programs), Arc::new(left));
        let mut through = 0;
        for (oi, sim) in simulated(&doc, &mesh, 1) {
            let sim = sim.unwrap();
            // Each rapid against the stock as it is then: the check's, and
            // the real one, both cut by the feeds before it.
            let mut checked = sim.initial_volume.clone().unwrap();
            let plan = &sim.plan;
            let (r, flute) = (plan.cut_radius(), plan.flute_length);
            for m in &plan.moves {
                let frame = sim.frame_at(m.a).unwrap();
                match m.motion {
                    crate::simulation::Motion::Feed => {
                        for v in [&mut checked, &mut real] {
                            v.cut(m.from, m.to, r, flute, plan.shape, frame, sim.source);
                        }
                    }
                    crate::simulation::Motion::Rapid => {
                        // Its rapids clear the stock the check has.
                        assert!(
                            !checked.touches(m.from, m.to, r, flute, plan.shape, frame),
                            "{oi} {m:?}"
                        );
                        if real.touches(m.from, m.to, r, flute, plan.shape, frame) {
                            through += 1;
                        }
                    }
                    _ => {}
                }
            }
        }
        // Over the face stopped after a level, they go through the stock.
        assert!(through > 10, "{through}");
        // Setup 2's run, given what setup 1's run cut this session.
        type Checks = (Arc<Result<Vec<Checked>, Vec<String>>>, Arc<Left>);
        let setup_2 = |doc: &Document, checked: &Checks, cut: CutInRun| {
            let mut run = Run::new(doc, 1, 0);
            crate::give_earlier_cuts(&[(doc.setups[0].id.clone(), vec![cut])], &[], &mut run);
            run.checked_as(checked.0.clone(), Some(checked.1.clone()), None);
            assert!(run.checked().is_some(), "{:?}", run.programs);
            // Its stock pictured, as `App::picture_run` takes it (LIO-602
            // review): what the check simulated before it is still known.
            assert!(run.left.take().is_some());
            run
        };
        let print = Some(face_steps.cut_print);
        let wait = |cut: CutInRun| setup_2(&doc, &checked, cut).earlier_not_whole(&doc);
        // What setup 1's runs cut, as its tool runs' outcomes note it.
        let (c, l) = check(&doc, &mesh, 0);
        let first: Checks = (Arc::new(c), Arc::new(l));
        let noted = |notes: &[Option<usize>]| {
            let mut one = Run::new(&doc, 0, 0);
            one.checked_as(first.0.clone(), Some(first.1.clone()), None);
            for &cut in notes {
                one.note_cut(0, cut);
            }
            assert_eq!(one.cut_in_run.len(), 1);
            one.cut_in_run[0]
        };
        // Cut whole (a tool run that ended done, or stopped after the last
        // step, LIO-601): nothing waits.
        assert_eq!(wait(noted(&[None])), None);
        // Stopped after level 1, or in a level, its last too (a bound):
        // Cycle Start waits, until a resume cuts the rest.
        for n in [1, levels.len()] {
            let why = wait(noted(&[Some(n)])).unwrap();
            assert!(
                why.starts_with("Top's run stopped partway through Facing,")
                    && why.contains("Finish it first: pick Top"),
                "{why}"
            );
            assert_eq!(wait(noted(&[Some(n), None])), None);
        }
        // Stopped where it is not known, or dropped while it ran: all of it
        // may be cut, or only some (`MAY_HAVE_CUT_ALL`).
        let why = wait(noted(&[MAY_HAVE_CUT_ALL])).unwrap();
        assert!(
            why.starts_with("Top's run of Facing stopped where it is not known")
                && why.contains("Cut Facing whole first, in Top's run"),
            "{why}"
        );
        // Cut whole before, then stopped partway: at least the whole cut.
        assert_eq!(wait(noted(&[None, Some(1)])), None);
        // Cut whole from another plan or cutter, or from two: it waits,
        // until it is cut whole again with the plan it has now.
        let other = print.map(|p| p ^ 1);
        for cut in [(0, None, other, other), (0, None, None, None)] {
            let why = wait(cut).unwrap();
            assert!(
                why.starts_with("Top's Facing changed since its run cut it")
                    && why.contains("Cut Facing whole again first, in Top's run"),
                "{why}"
            );
        }
        let mut cuts = vec![(0, None, other, other)];
        note_cut_in(&mut cuts, 0, None, print);
        assert_eq!(cuts, [(0, None, None, print)]);
        assert_eq!(wait(cuts[0]), None);
        // What is kept on disk from before this was noted is not known whole.
        let kept: CutHistory = serde_json::from_str(
            r#"{"cuts":[["a",[[0,null,7],[1,2,null]]],["b",[[0,null,7,7]]]]}"#,
        )
        .unwrap();
        assert_eq!(
            kept.cuts,
            [
                (
                    "a".to_string(),
                    vec![(0, None, Some(7), None), (1, Some(2), None, None)]
                ),
                ("b".to_string(), vec![(0, None, Some(7), Some(7))]),
            ]
        );
        let back: CutHistory =
            serde_json::from_str(&serde_json::to_string(&kept).unwrap()).unwrap();
        assert_eq!(back, kept);
        // Such a cut kept as the next tool run starts (`cuts_if_started`)
        // is still not known whole: that tool run may stop partway too.
        let mut again = Run::new(&doc, 0, 0);
        again.checked_as(first.0.clone(), Some(first.1.clone()), None);
        again.cut_in_run = vec![(0, None, print, None)];
        let kept = again.cuts_if_started();
        assert_eq!(kept[0].3, None, "{kept:?}");
        assert!(wait(kept[0]).is_some());
        // The tablet file refuses setup 2 as the Run panel waits, in its
        // own words, and lists it again once the face is cut whole.
        let tablet = |cut: CutInRun| {
            let session = Session {
                cuts: vec![(doc.setups[0].id.clone(), vec![cut])],
                lost: false,
                declared: vec![],
                owners: vec![],
            };
            tablet_runs(&doc, Ok(&mesh), 1, &session)
        };
        let refused = tablet(noted(&[Some(1)]));
        assert!(refused.tool_runs.is_empty());
        assert_eq!(
            refused.problems,
            [format!(
                "Top's run on the computer stopped partway through Facing before this tablet file was made, and this setup's programs are made for the stock with Facing cut whole: their rapids could go through the material it left. On the computer, finish Facing first, in Top's run, then Sync projects here, or save the project for the tablet again.{}",
                new_stock_on_the_computer!()
            )]
        );
        let listed = tablet(noted(&[Some(1), None]));
        assert!(listed.problems.is_empty() && !listed.tool_runs.is_empty());
        // Disabled since: the check leaves more stock than the part has,
        // which no rapid runs into (Zero Z waits for it, LIO-582 review).
        let mut disabled = doc.clone();
        disabled.setups[0].operations[0].enabled = false;
        let (c, l) = check(&disabled, &mesh, 1);
        let run = setup_2(
            &disabled,
            &(Arc::new(c), Arc::new(l)),
            (0, Some(1), print, None),
        );
        assert!(run.earlier_unfollowed());
        assert_eq!(run.earlier_not_whole(&disabled), None);
    }

    /// A bore milled through the stock, then a face with another tool (LIO-582
    /// review): a start at the face, in the second tool run, finds no floor
    /// at the stock top's centre, where the bore went through, so neither
    /// the Run panel nor the tablet file sets a Z there.
    #[test]
    fn a_start_after_a_bore_through_the_stock_has_no_floor_to_touch_off_on() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("template.toml");
        std::fs::write(&profile, uc100::config::TEMPLATE).unwrap();
        let (mut doc, _) = tablet_fixture(&profile);
        let (path, mesh, [mill, small]) = ring(dir.path());
        let mut setup = ring_setup(&doc, "G54", "1", "0");
        let top = ring_top(&mesh, &setup, true);
        setup.operations = vec![ring_bore(&mill, top.clone(), "1"), ring_face(&small, top)];
        doc.active = setup.id.clone();
        doc.setups = vec![setup];
        doc.tools = vec![mill, small];
        if let Some(f) = &mut doc.file {
            (f.path, f.name, f.format) = (path, "ring.stl".into(), "stl".into());
        }
        let (programs, left) = check(&doc, &mesh, 0);
        let programs = programs.unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(programs.len(), 2);
        let mut run = Run::new(&doc, 0, 0);
        run.checked_as(Arc::new(Ok(programs)), Some(Arc::new(left)), None);
        run.start_at(1, 0);
        assert!(run.partway());
        let partly = run.partly().unwrap();
        assert!(partly.top_cut && !partly.cut.is_empty(), "{partly:?}");
        assert_eq!(partly.surface, None);
        assert_eq!(run.touch_off(), None);
        assert!(run.partway_blocked().is_some());
        let s = tablet_runs(&doc, Ok(&mesh), 0, &Session::default());
        assert!(s.problems.is_empty(), "{:?}", s.problems);
        let zero = s.tool_runs[1].zero.as_ref().unwrap();
        assert_eq!(zero.set_z, None);
        assert_eq!(zero.blocked, run.partway_blocked());
    }

    /// A 200 mm disk 10 high with a Ø1 blind bore 2 mm deep at its centre,
    /// its stock 2.02 mm wider to +X, so the bore lies 1 mm off the stock's
    /// centre and between the points its centre's probe spot is sampled at,
    /// on the tablet fixture's machine (in `dir`): the bore milled with a
    /// Ø0.5 end mill, then the top faced with a Ø10, in two tool runs; work
    /// zero Stock top center (LIO-582 review).
    pub(crate) fn small_bore_job(dir: &std::path::Path) -> (Document, Mesh) {
        let profile = dir.join("template.toml");
        std::fs::write(&profile, uc100::config::TEMPLATE).unwrap();
        let (mut doc, _) = tablet_fixture(&profile);
        let tris = crate::cam::tests::revolved(&[
            (0., 0.),
            (100., 0.),
            (100., 10.),
            (0.5, 10.),
            (0.5, 8.),
            (0., 8.),
            (0., 0.),
        ]);
        let path = dir.join("disk.stl");
        let mut stl = String::from("solid disk\n");
        for tri in &tris {
            stl.push_str("facet normal 0 0 0\nouter loop\n");
            for p in tri {
                stl.push_str(&format!("vertex {} {} {}\n", p[0], p[1], p[2]));
            }
            stl.push_str("endloop\nendfacet\n");
        }
        stl.push_str("endsolid disk\n");
        std::fs::write(&path, stl).unwrap();
        let mesh = crate::mesh::load(&path).unwrap();
        let tool = |name: &str, d: &str, n: &str| {
            let mut t = crate::model::Tool::new(name, "Flat end mill", d);
            for (k, v) in [("length", "40"), ("stickout", "45"), ("number", n)] {
                t.p.insert(k.into(), v.into());
            }
            t
        };
        let (small, large) = (tool("Mill 0.5", "0.5", "1"), tool("Mill 10", "10", "2"));
        let mut setup = ring_setup(&doc, "G54", "1", "0");
        setup.p.insert("stock_right".into(), "2.02".into());
        let top = ring_top(&mesh, &setup, true);
        let mut bore = ring_bore(&small, top.clone(), "0");
        for (k, v) in [("through", "No"), ("allowance", "0"), ("stepdown", "1")] {
            bore.p.insert(k.into(), v.into());
        }
        let mut face = ring_face(&large, top);
        face.p.insert("stepover".into(), "90".into());
        setup.operations = vec![bore, face];
        doc.active = setup.id.clone();
        doc.setups = vec![setup];
        doc.tools = vec![small, large];
        if let Some(f) = &mut doc.file {
            (f.path, f.name, f.format) = (path, "disk.stl".into(), "stl".into());
        }
        (doc, mesh)
    }

    /// A Ø1 blind bore 2 mm deep, 1 mm off the centre of a 200 mm disk's
    /// stock, then a face with a Ø10 end mill (LIO-582 review): a start at
    /// the face, the next tool run, finds the centre's probe spot not
    /// proven uncut, though no point it is sampled at lies in the bore, so
    /// neither the Run panel nor the tablet file offers it.
    #[test]
    fn a_start_after_a_small_bore_offers_no_probe_spot_over_it() {
        let dir = tempfile::tempdir().unwrap();
        let (doc, mesh) = small_bore_job(dir.path());
        let (programs, left) = check(&doc, &mesh, 0);
        let programs = programs.unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(programs.len(), 2);
        let mut run = Run::new(&doc, 0, 0);
        run.checked_as(Arc::new(Ok(programs)), Some(Arc::new(left)), None);
        run.start_at(1, 0);
        assert!(run.partway());
        let partly = run.partly().unwrap();
        let centre = |spots: &[([crate::post::At; 2], Result<(), String>)]| {
            let (_, ok) = (spots.iter())
                .find(|(r, _)| *r == [crate::post::At::Mid; 2])
                .unwrap();
            ok.clone()
        };
        let why = centre(&partly.probe_from).unwrap_err();
        assert!(
            why.contains("may have cut the stock's top face at its centre"),
            "{why}"
        );
        // Not every spot: the corners, far from the bore, stay.
        assert!((partly.probe_from.iter()).any(|(_, ok)| ok.is_ok()));
        let s = tablet_runs(&doc, Ok(&mesh), 0, &Session::default());
        assert!(s.problems.is_empty(), "{:?}", s.problems);
        let zero = s.tool_runs[1].zero.as_ref().unwrap();
        let spot = (zero.probe_from.iter())
            .find(|t| (t.x, t.y) == (crate::post::At::Mid, crate::post::At::Mid))
            .unwrap();
        assert_eq!(spot.problem.as_deref(), Some(why.as_str()));
    }

    /// The small bore's job split in two setups, the second's stock the
    /// first's (`small_bore_job`, LIO-584): the first bores, the second
    /// faces. The tablet file's first tool run of the second carries why
    /// its touch-off cannot set Z, and the probe spots, as the Run panel
    /// has them from its start; the first setup's, on fresh stock, and the
    /// second's when nothing was bored under the spot, carry none.
    #[test]
    fn the_tablet_file_says_a_setup_start_cannot_touch_off_over_an_earlier_setups_cut() {
        use crate::post::At;
        let dir = tempfile::tempdir().unwrap();
        let (doc, mesh) = small_bore_job(dir.path());
        let mut split = doc.clone();
        let mut second = split.setups[0].duplicate("G55");
        second.operations.remove(0);
        split.setups[0].operations.truncate(1);
        split.setups.push(second);
        split.sync_stock_sources();
        // As the Run panel has it from the second setup's start.
        let (programs, left) = check(&split, &mesh, 1);
        let mut run = Run::new(&split, 1, 0);
        run.checked_as(Arc::new(programs), Some(Arc::new(left)), None);
        assert!(!run.partway() && run.touch_off().is_none());
        let why = run.partway_blocked().unwrap();
        assert_eq!(why, INHERITED_UNKNOWN);
        let p = run.checked().unwrap()[0].clone();
        let s = tablet_runs(&split, Ok(&mesh), 1, &Session::default());
        assert!(s.problems.is_empty(), "{:?}", s.problems);
        let zero = s.tool_runs[0].zero.as_ref().unwrap();
        assert_eq!(
            (zero.set_z, zero.blocked.as_deref(), zero.cut.is_empty()),
            (None, Some(why.as_str()), true)
        );
        assert_eq!(zero.top, (p.stock.max[2] - p.datum[2]) as f64);
        assert_eq!(zero.spot.as_deref(), Some("the centre"));
        assert_eq!(zero.probe_from.len(), p.probe_from.len());
        for (t, ([x, y], ok)) in zero.probe_from.iter().zip(&p.probe_from) {
            assert_eq!((t.x, t.y, t.problem.clone()), (*x, *y, ok.clone().err()));
        }
        let centre = (zero.probe_from.iter()).find(|t| (t.x, t.y) == (At::Mid, At::Mid));
        assert!(centre.is_some_and(|t| t.problem.is_some()), "{zero:?}");
        assert!(zero.probe_from.iter().any(|t| t.problem.is_none()));
        // The first setup, on fresh stock: none.
        let s = tablet_runs(&split, Ok(&mesh), 0, &Session::default());
        assert!(s.problems.is_empty(), "{:?}", s.problems);
        assert!(s.tool_runs.iter().all(|r| r.zero.is_none()));
        // Bored nowhere, the second touches off as ever: none either.
        let mut plain = split.clone();
        plain.setups[0].operations.clear();
        let (programs, left) = check(&plain, &mesh, 1);
        let mut run = Run::new(&plain, 1, 0);
        run.checked_as(Arc::new(programs), Some(Arc::new(left)), None);
        assert!(run.touch_off().is_some());
        let s = tablet_runs(&plain, Ok(&mesh), 1, &Session::default());
        assert!(s.problems.is_empty(), "{:?}", s.problems);
        assert_eq!(s.tool_runs[0].zero, None);
    }

    /// A bore milled from one side leaving a 0.5 mm roof, then the stock
    /// turned over and the roof faced away (LIO-582 review): the face opens
    /// the bore from above, so a start at the face's level 2 or at the next
    /// tool run finds no stock under the floor the face leaves at the
    /// centre, and neither the Run panel nor the tablet file sets a Z there.
    #[test]
    fn a_start_after_opening_a_bore_an_earlier_setup_cut_has_no_floor() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("template.toml");
        std::fs::write(&profile, uc100::config::TEMPLATE).unwrap();
        let (mut doc, _) = tablet_fixture(&profile);
        let (path, mesh, [mill, small]) = ring(dir.path());
        let mut first = ring_setup(&doc, "G54", "1", "1");
        let top = ring_top(&mesh, &first, true);
        first.operations = vec![ring_bore(&mill, top, "0.5")];
        let mut flipped = ring_setup(&doc, "G55", "1", "1");
        flipped.p.insert("rx".into(), "180".into());
        let top = ring_top(&mesh, &flipped, false);
        let mut opening = ring_face(&mill, top.clone());
        opening.p.insert("stepdown".into(), "0.6".into());
        flipped.operations = vec![opening, ring_face(&small, top)];
        doc.active = flipped.id.clone();
        doc.setups = vec![first, flipped];
        doc.tools = vec![mill, small];
        if let Some(f) = &mut doc.file {
            (f.path, f.name, f.format) = (path, "ring.stl".into(), "stl".into());
        }
        let (programs, left) = check(&doc, &mesh, 1);
        let programs = programs.unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(programs.len(), 2);
        let (whole, left) = (Arc::new(Ok(programs)), Arc::new(left));
        // The face opens it at its second level: there is one to start at.
        let face = left.steps.iter().find(|s| s.op == 0).unwrap();
        assert!(face.steps.len() >= 2, "{}", face.steps.len());
        let blocked = |run: &Run| {
            assert!(run.partway());
            assert_eq!(run.partly().unwrap().surface, None);
            assert_eq!(run.touch_off(), None);
            run.partway_blocked().unwrap()
        };
        let mut run = Run::new(&doc, 1, 0);
        run.checked_as(whole.clone(), Some(left.clone()), None);
        run.start_at(1, 0);
        let at_op = blocked(&run);
        let mut run = Run::new(&doc, 1, 0);
        run.checked_as(whole, Some(left), None);
        assert!(run.start_at_step(0, 1));
        let (c, l) = check_steps(&doc, &mesh, 1, &[], &[(0, vec![0])]);
        run.checked_as(Arc::new(c), Some(Arc::new(l)), None);
        assert_eq!(run.at_step(), Some((0, 1)));
        let at_step = blocked(&run);
        let s = tablet_runs(&doc, Ok(&mesh), 1, &Session::default());
        assert!(s.problems.is_empty(), "{:?}", s.problems);
        let zero = s.tool_runs[1].zero.as_ref().unwrap();
        assert_eq!((zero.set_z, zero.blocked.as_ref()), (None, Some(&at_op)));
        let step = (s.tool_runs[0].step_starts.iter())
            .find(|t| (t.operation, t.step) == (0, 1))
            .unwrap();
        let zero = step.zero.as_ref().unwrap();
        assert_eq!((zero.set_z, zero.blocked.as_ref()), (None, Some(&at_step)));
    }

    /// A Z set before another start is picked (LIO-618): the end mill's
    /// tool run was picked again after it ran, and Z set from the reference
    /// left the offset as it was; the drill's tool run picked then waits
    /// for the drill's Z. A start at a step asks for its tool's Z again
    /// after another step is picked, as for the tool's Z kept, though the
    /// tool is the same; so does a check asked for, which a skip moves the
    /// start with, but not one set while it runs. The offset stays the
    /// touched-off tool's (LIO-619): another
    /// tool next is touched off first while it is unchanged.
    #[test]
    fn a_z_set_before_another_start_is_picked_is_asked_again() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("template.toml");
        std::fs::write(&profile, uc100::config::TEMPLATE).unwrap();
        let (doc, mesh) = tablet_fixture(&profile);
        let mut m = doc.setups[0].machine.clone();
        m.p.insert("uc100_connection".into(), "Simulated".into());
        let c = crate::control::Connection::open(&m, dir.path()).unwrap();
        let offset = c.snapshot.work_offset[2];
        let (whole, left) = check_steps(&doc, &mesh, 0, &[], &[]);
        let (whole, left) = (Arc::new(whole), Arc::new(left));
        let fresh = || {
            let mut run = Run::new(&doc, 0, 0);
            run.checked_as(whole.clone(), Some(left.clone()), None);
            run
        };
        let mut run = fresh();
        let tools: Vec<String> = (run.checked().unwrap().iter())
            .map(|p| p.ops[0].tool_id.clone())
            .collect();
        assert_eq!(tools.len(), 2);
        assert_ne!(tools[0], tools[1]);
        let touch = "touch off its Z first";
        let says =
            |run: &Run, what: &str| (run.problems(&c, &doc).iter()).any(|p| p.contains(what));
        let set_z = |run: &mut Run| {
            run.z_touched();
            run.touch_shown(offset);
        };
        // The end mill ran last, at this Z offset.
        run.last = Some((tools[0].clone(), offset));
        run.start_at(1, 0);
        assert!(says(&run, touch), "{:?}", run.problems(&c, &doc));
        set_z(&mut run);
        assert!(run.touched() && !says(&run, touch));
        // Its tool run picked again, Z set with the end mill, then the
        // drill's: Cycle Start waits for the drill's Z.
        run.start_at(0, 0);
        set_z(&mut run);
        assert!(run.touched() && !says(&run, touch));
        run.start_at(1, 0);
        assert!(!run.touched());
        assert!(says(&run, touch), "{:?}", run.problems(&c, &doc));
        // The end mill's tool run picked again: Z is the end mill's still,
        // but the start was picked since it was set.
        run.start_at(0, 0);
        assert!(!run.touched());
        // A start at a step: its tool's Z set, then another step picked.
        let pocket = (run.op_steps(1).map(|s| s.steps.len())).unwrap_or(0);
        assert!(pocket >= 3, "{pocket}");
        let confirm = "Z for the start at";
        for n in [1, 2] {
            assert!(run.start_at_step(1, n));
            run.check_again(0);
            let (p, l) = check_steps(&doc, &mesh, 0, &[], &[(1, (0..n).collect())]);
            run.checked_as(Arc::new(p), Some(Arc::new(l)), None);
            assert_eq!(run.at_step(), Some((1, n)));
            assert!(!run.touched());
            assert!(says(&run, confirm), "{n}: {:?}", run.problems(&c, &doc));
            set_z(&mut run);
            assert!(!says(&run, confirm), "{n}: {:?}", run.problems(&c, &doc));
        }
        // The end mill's tool run stopped partway, Z set again with the end
        // mill, then the rest of its operations skipped as cut: the skip's
        // check moves the start to the drill's tool run, which waits for
        // the drill's Z (LIO-618 review).
        let mut run = fresh();
        run.last = Some((tools[0].clone(), offset));
        set_z(&mut run);
        assert!(!says(&run, touch), "{:?}", run.problems(&c, &doc));
        for oi in [0, 1] {
            assert!(run.skip(oi, Some(Skip::Cut)), "{oi}");
            run.check_again(0);
            let (p, l) = check_steps(&doc, &mesh, 0, &run.skips.clone(), &[]);
            // Z set again while the last skip's check runs: for the end
            // mill, the tool next before it (LIO-619).
            if oi == 1 {
                set_z(&mut run);
            }
            run.checked_as(Arc::new(p), Some(Arc::new(l)), None);
        }
        let next = &run.checked().unwrap()[run.next];
        assert_eq!(next.ops[0].tool_id, tools[1]);
        assert!(!run.touched());
        assert!(says(&run, touch), "{:?}", run.problems(&c, &doc));
        // A check after a project edit forgets it too: it may number the
        // tool runs otherwise.
        let mut run = fresh();
        set_z(&mut run);
        assert!(run.touched());
        run.check_again(0);
        assert!(!run.touched());
        run.checked_as(whole.clone(), Some(left.clone()), None);
        assert!(!run.touched());
        // Set while the setup is checked again, after the start was picked,
        // it holds for the tool next then, still next (LIO-619).
        run.check_again(0);
        set_z(&mut run);
        run.checked_as(whole.clone(), Some(left.clone()), None);
        assert!(run.touched() && !says(&run, touch));
        run.check_again(0);
        run.checked_as(whole.clone(), Some(left.clone()), None);
        assert!(!run.touched());
        // At a tool change, the drill touched off (the offset its own, not
        // the end mill's), then the setup checked again: the touch-off is
        // not this start's, so it waits for the drill's Z, as the Tool
        // change tab shows the touch-off not done.
        let mut change = fresh();
        change.last = Some((tools[0].clone(), offset - 5.));
        change.start_at(1, 0);
        // The offset moved since the end mill's tool run, by no Z set
        // here: whose Z it is is not known, so it waits (LIO-630).
        assert!(says(&change, touch), "{:?}", change.problems(&c, &doc));
        set_z(&mut change);
        assert!(change.touched() && !says(&change, touch));
        change.check_again(0);
        change.checked_as(whole.clone(), Some(left.clone()), None);
        assert_eq!(change.next, 1);
        assert!(!change.touched());
        assert!(says(&change, touch), "{:?}", change.problems(&c, &doc));
        // Forgotten, the offset is still the end mill's: the drill's tool
        // run picked next waits for the drill's Z, though no tool run ran
        // (LIO-619).
        run.start_at(1, 0);
        assert!(says(&run, touch), "{:?}", run.problems(&c, &doc));
    }

    /// A Z set with one tool moves the Z work offset off the one the last
    /// tool run started with (LIO-630): T1, T2, T1 ran; tool run 1 picked
    /// again and Set on Z (or Zero Z, a touch-off, a probe) made with T1
    /// in; tool run 2 picked then waits for T2's Z, though the offset is
    /// no longer the last tool run's. The other way round, Z set with T2
    /// for tool run 2, then tool run 1 picked, waits for T1's. Z set with
    /// the new tool for its own start lets Cycle Start go. Z set otherwise
    /// (MDI, the tablet) is no tool's known Z: at a tool change it waits.
    #[test]
    fn a_z_set_with_one_tool_is_not_another_tools_after_the_offset_moved() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("template.toml");
        std::fs::write(&profile, uc100::config::TEMPLATE).unwrap();
        let (doc, mesh) = tablet_fixture(&profile);
        let mut m = doc.setups[0].machine.clone();
        m.p.insert("uc100_connection".into(), "Simulated".into());
        let c = crate::control::Connection::open(&m, dir.path()).unwrap();
        // The Z work offset the machine shows, which the Z set leaves; the
        // last tool run started 5 mm off it.
        let offset = c.snapshot.work_offset[2];
        let (whole, left) = check_steps(&doc, &mesh, 0, &[], &[]);
        let (whole, left) = (Arc::new(whole), Arc::new(left));
        let fresh = |last: usize| {
            let mut run = Run::new(&doc, 0, 0);
            run.checked_as(whole.clone(), Some(left.clone()), None);
            let tool = run.tool_of(last).unwrap();
            run.last = Some((tool, offset - 5.));
            run
        };
        let probe = fresh(0);
        let programs = probe.checked().unwrap();
        assert_eq!(programs.len(), 2);
        assert_ne!(programs[0].ops[0].tool_id, programs[1].ops[0].tool_id);
        let label = |i: usize| {
            let op = &programs[i].ops[0];
            crate::model::tool_label(op.tool, &op.tool_name)
        };
        let wait = |i: usize| {
            format!(
                "Tool run {} uses {}: touch off its Z first",
                i + 1,
                label(i)
            )
        };
        let touch = "touch off its Z first";
        let problems = |run: &Run| run.problems(&c, &doc);
        let says = |run: &Run, what: &str| problems(run).iter().any(|p| p.contains(what));
        let set_z = |run: &mut Run| {
            run.z_touched();
            run.touch_shown(offset);
        };
        // The last tool run's tool was either: T1 (T1, T2, T1 ran) or T2.
        for last in [0, 1] {
            // Tool run 1 picked again, Z set with T1 in: it holds there.
            let mut run = fresh(last);
            run.start_at(0, 0);
            set_z(&mut run);
            assert!(run.touched(), "{last}");
            assert!(!says(&run, touch), "{last}: {:?}", problems(&run));
            // Tool run 2 picked: T2 would cut with T1's Z.
            run.start_at(1, 0);
            assert!(!run.touched());
            assert!(says(&run, &wait(1)), "{last}: {:?}", problems(&run));
            // T2's Z set for it: Cycle Start goes.
            set_z(&mut run);
            assert!(run.touched());
            assert!(!says(&run, touch), "{last}: {:?}", problems(&run));
            // The other way round: tool run 1 picked waits for T1's Z.
            run.start_at(0, 0);
            assert!(says(&run, &wait(0)), "{last}: {:?}", problems(&run));
            set_z(&mut run);
            assert!(!says(&run, touch), "{last}: {:?}", problems(&run));
            // Tool run 2 picked first, Z set with T2, then tool run 1.
            let mut run = fresh(last);
            run.start_at(1, 0);
            set_z(&mut run);
            assert!(!says(&run, touch), "{last}: {:?}", problems(&run));
            run.start_at(0, 0);
            assert!(says(&run, &wait(0)), "{last}: {:?}", problems(&run));
        }
        // Z set otherwise since (MDI, the tablet): the offset neither the
        // last tool run's nor the one the last Z set here left, so whose
        // Z it is is not known; at a tool change the new tool waits for
        // its own (LIO-630, as the tablet's LIO-648).
        let mut moved = crate::control::Connection::open(&m, dir.path()).unwrap();
        moved.snapshot.work_offset[2] = offset + 1.;
        let says_at = |run: &Run, c: &Connection, what: &str| {
            (run.problems(c, &doc).iter()).any(|p| p.contains(what))
        };
        let mut run = fresh(0);
        run.start_at(1, 0);
        assert!(run.z_elsewhere(&moved));
        assert!(
            says_at(&run, &moved, &wait(1)),
            "{:?}",
            run.problems(&moved, &doc)
        );
        // T2's Z set, then moved otherwise: it waits again; back to the
        // one T2's left, it goes.
        set_z(&mut run);
        assert!(run.touched() && !says_at(&run, &c, touch));
        assert!(
            says_at(&run, &moved, &wait(1)),
            "{:?}",
            run.problems(&moved, &doc)
        );
        // The same tool next: no tool change, nothing asked.
        let mut run = fresh(1);
        run.start_at(1, 0);
        assert!(!run.z_elsewhere(&moved));
        assert!(
            !says_at(&run, &moved, touch),
            "{:?}",
            run.problems(&moved, &doc)
        );
    }

    /// What this run's tool runs cut (`cut_in_run`, LIO-582 review): Face
    /// done and the Pocket stopped after its level 2. The start at level 3,
    /// where the stop leaves Cycle Start, goes by the stock the check
    /// carves; one at level 2 or the Pocket's start, before what was cut,
    /// and the Face's, give no Z; the Drilling's, after it all, does, until
    /// the Face is disabled or its plan changed.
    #[test]
    fn a_start_before_what_the_run_cut_sets_no_z() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("template.toml");
        std::fs::write(&profile, uc100::config::TEMPLATE).unwrap();
        let (doc, mesh) = tablet_fixture(&profile);
        let (whole, left) = check_steps(&doc, &mesh, 0, &[], &[]);
        let (whole, left) = (Arc::new(whole), Arc::new(left));
        let at = |oi: usize, n: usize| {
            let mut run = Run::new(&doc, 0, 0);
            run.checked_as(whole.clone(), Some(left.clone()), None);
            run.note_cut(0, None);
            run.note_cut(1, Some(2));
            run.note_cut(1, Some(1));
            let print = |oi: usize| run.op_steps(oi).map(|s| s.cut_print);
            assert_eq!(
                run.cut_in_run,
                [(0, None, print(0), print(0)), (1, Some(2), print(1), None)]
            );
            assert!(print(0).is_some() && print(1).is_some());
            if n > 0 {
                assert!(run.start_at_step(oi, n));
                let (c, l) = check_steps(&doc, &mesh, 0, &[], &[(oi, (0..n).collect())]);
                run.checked_as(Arc::new(c), Some(Arc::new(l)), None);
                assert_eq!(run.at_step(), Some((oi, n)));
            } else {
                let i = (run.checked().unwrap().iter())
                    .position(|p| p.indexes.contains(&oi))
                    .unwrap();
                let k = run.checked().unwrap()[i]
                    .indexes
                    .iter()
                    .position(|&o| o == oi);
                run.start_at(i, k.unwrap());
            }
            run
        };
        let cut = |run: &Run| run.partly().is_some_and(|p| p.cut == [RUN_UNFOLLOWED]);
        // The Pocket's levels have no Z on this fixture anyway (stock
        // beside the floor): what matters is it is the check's stock.
        assert_eq!(at(1, 2).partly().map(|p| p.steps.clone()), Some(vec![0, 1]));
        assert!(cut(&at(1, 1)) && at(1, 1).touch_off().is_none());
        assert!(cut(&at(1, 0)) && cut(&at(0, 0)));
        assert!(!cut(&at(2, 0)) && at(2, 0).touch_off().is_some());
        // The Face, cut, then disabled or changed, and the setup checked
        // again: its stock lacks what the Face cut, so the Drilling's start
        // gives no Z either (LIO-582 review).
        let edited = |edit: &dyn Fn(&mut crate::model::Operation)| {
            let mut run = at(2, 0);
            let mut doc = doc.clone();
            edit(&mut doc.setups[0].operations[0]);
            let (c, l) = check_steps(&doc, &mesh, 0, &[], &[]);
            run.checked_as(Arc::new(c), Some(Arc::new(l)), None);
            assert_eq!(
                run.checked().unwrap().iter().flat_map(|p| &p.indexes).max(),
                Some(&2)
            );
            run
        };
        let off = edited(&|o| o.enabled = false);
        assert!(cut(&off) && off.touch_off().is_none());
        let deeper = edited(&|o| {
            let z = crate::model::get(&o.p, "stepdown")
                .parse::<f32>()
                .unwrap_or(2.);
            o.p.insert("stepdown".into(), (z / 2.).to_string());
        });
        assert!(cut(&deeper) && deeper.touch_off().is_none());
        assert!(!cut(&edited(&|_| {})));
    }

    /// A start at a step of a tool run this run cut to its end (LIO-586):
    /// the tablet fixture's Face and Pocket ran to their end (the driver's
    /// outcome, through `refresh`), then Face pass 2 is picked to cut again.
    /// The check takes only pass 1 as cut, but pass 2 cut the touch-off spot
    /// down to its own floor too: Set Z of pass 1's floor there would set
    /// the work zero as deep too low as pass 2 cuts. So nothing sets Z from
    /// the check's stock, and Zero Z on the stock top and the probe spots
    /// say why, as at the tool run's own start. Before any tool run, and
    /// after one stopped after pass 1, the same start sets pass 1's floor.
    #[test]
    fn a_step_start_in_a_tool_run_cut_to_its_end_sets_no_z() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("template.toml");
        std::fs::write(&profile, uc100::config::TEMPLATE).unwrap();
        let (doc, mesh) = tablet_fixture(&profile);
        let (whole, left) = check_steps(&doc, &mesh, 0, &[], &[]);
        let (whole, left) = (Arc::new(whole), Arc::new(left));
        // Face's passes go down: pass 2's floor is below pass 1's.
        let face = left.steps.iter().find(|s| s.op == 0).unwrap();
        assert!(face.steps.len() >= 2, "{}", face.steps.len());
        let (pass_1, pass_2) = (face.steps[0].z.unwrap(), face.steps[1].z.unwrap());
        assert!(pass_2 < pass_1 - 0.5, "{pass_1} {pass_2}");
        assert_eq!(whole.as_ref().as_ref().unwrap()[0].indexes, [0, 1]);
        // Tool run 1 (Face, Pocket) sent from its start, at work offset 0,
        // and its outcome: ended ok, stopped after a step of the Face
        // (`stopping`), or stopped in a step (`message`), the tool last
        // followed on block `on` and standing at machine position `at`.
        let p = whole.as_ref().as_ref().unwrap()[0].clone();
        let blocks = p.from(0).unwrap();
        // Tool run 1 running, the tool last followed on block `on`.
        let running = |on: Option<usize>| {
            let mut run = Run::new(&doc, 0, 0);
            run.checked_as(whole.clone(), Some(left.clone()), None);
            let marks = run.marks_of(&p, blocks);
            run.running = Some(Running {
                index: 0,
                from: 0,
                seq: 0,
                offset: [0.; 6],
                before: Before {
                    xy: None,
                    zero: None,
                    last: None,
                    touched_for: None,
                },
                paths: paths(blocks, [0.; 3]),
                tip: [0.; 3],
                start: [0.; 6],
                print: 0,
                cut_ops: vec![],
                on: on.map(|b| (b, 0.5)),
                marks,
            });
            run
        };
        let ran_at = |ok: bool,
                      stopping: Option<(usize, usize)>,
                      message: &str,
                      on: Option<usize>,
                      at: Option<[f64; 6]>| {
            let mut run = running(on);
            run.stopping = stopping;
            run.refresh(
                Some(&uc100::controller::ProgramOutcome {
                    seq: 1,
                    ok,
                    message: message.into(),
                }),
                at.as_ref(),
            );
            assert!(run.running.is_none());
            run
        };
        let ran = |ok: bool, stopping| ran_at(ok, stopping, "m", None, None);
        // Face pass 2 picked as the start, and the setup checked again
        // with pass 1 taken as cut.
        let at_pass_2 = |mut run: Run| {
            assert!(run.start_at_step(0, 1));
            let (c, l) = check_steps(&doc, &mesh, 0, &[], &[(0, vec![0])]);
            run.checked_as(Arc::new(c), Some(Arc::new(l)), None);
            assert_eq!((run.next, run.from, run.at_step()), (0, 0, Some((0, 1))));
            assert!(run.partway() && run.on_cut_stock());
            run
        };
        let near = |a: f32, b: f32| (a - b).abs() < 1e-3;
        let on_pass_1 = |run: &Run| {
            let partly = run.partly().unwrap();
            assert!(partly.cut != [RUN_UNFOLLOWED], "{:?}", partly.cut);
            assert!(
                partly.surface.is_some_and(|z| near(z, pass_1)),
                "{partly:?}"
            );
            let (at, _) = run.touch_off().unwrap();
            assert!(near(at[2], pass_1), "{at:?}");
            assert_eq!(run.partway_blocked(), None);
        };
        // Before any tool run: pass 1's floor.
        let mut fresh = Run::new(&doc, 0, 0);
        fresh.checked_as(whole.clone(), Some(left.clone()), None);
        on_pass_1(&at_pass_2(fresh));
        // Stopped after pass 1: pass 2 never cut, pass 1's floor still.
        let stopped = ran(false, Some((0, 0)));
        assert_eq!(stopped.cut_in_run.len(), 1);
        assert_eq!(stopped.cut_in_run[0].1, Some(1));
        on_pass_1(&at_pass_2(stopped));
        // Cut to its end, or stopped after pass 2: pass 2 cut there, which
        // the check's stock before pass 2 does not have.
        let done = ran(true, None);
        assert!(done.done[0] && done.next == 1);
        assert_eq!(
            done.cut_in_run
                .iter()
                .map(|c| (c.0, c.1))
                .collect::<Vec<_>>(),
            [(0, None), (1, None)]
        );
        let refused = |run: Run| {
            let partly = run.partly().unwrap();
            assert_eq!(partly.cut, [RUN_UNFOLLOWED]);
            assert_eq!(partly.surface, None);
            assert!(!partly.probe_from.is_empty());
            assert!(
                (partly.probe_from.iter())
                    .all(|(_, ok)| ok.as_ref().is_err_and(|e| e == RUN_UNFOLLOWED_WHY))
            );
            assert_eq!(run.touch_off(), None);
            assert_eq!(
                run.partway_blocked().as_deref(),
                Some(RUN_UNFOLLOWED_BLOCKED)
            );
            // As at the tool run's own start, picked again.
            let mut start = run;
            start.start_at(0, 0);
            start.checked_as(whole.clone(), Some(left.clone()), None);
            assert_eq!((start.next, start.from, start.at_step()), (0, 0, None));
            assert_eq!(start.touch_off(), None);
            assert_eq!(
                start.partway_blocked().as_deref(),
                Some(RUN_UNFOLLOWED_BLOCKED)
            );
        };
        refused(at_pass_2(done));
        refused(at_pass_2(ran(false, Some((0, 1)))));
        // Stopped in a step (an E-stop), the tool last followed early in
        // pass 1: the app may have missed the snapshots that followed it on,
        // and the one with the outcome has no line (LIO-586 review). What
        // it cut goes by where the tool stands.
        let mut plain = Run::new(&doc, 0, 0);
        plain.checked_as(whole.clone(), Some(left.clone()), None);
        let marks = plain.marks_of(&p, blocks);
        let path = paths(blocks, [0.; 3]);
        let in_pass = |s: usize| -> Vec<usize> {
            (0..blocks.len())
                .filter(|&j| {
                    matches!(
                        blocks[j],
                        Block::Move {
                            kind: Kind::Feed,
                            ..
                        }
                    ) && path[j].is_some()
                        && marks[j].and_then(|m| plain.mark_step(&p, m)) == Some((0, s, false))
                })
                .collect()
        };
        let (one, two) = (in_pass(0), in_pass(1));
        assert!(one.len() > 2 && two.len() > 2, "{one:?} {two:?}");
        // The machine position at the middle of block `j`'s move.
        let mid = |j: usize| -> [f64; 6] {
            let [a, b] = path[j].unwrap();
            std::array::from_fn(|k| match k {
                0..3 => ((a[k] + b[k]) / 2. - p.datum[k]) as f64,
                _ => 0.,
            })
        };
        let e_stop = |at| ran_at(false, None, "E-stop", Some(one[0]), Some(at));
        let cuts = |run: &Run| {
            run.cut_in_run
                .iter()
                .map(|c| (c.0, c.1))
                .collect::<Vec<_>>()
        };
        // Standing in pass 1: pass 2 not cut, pass 1's floor.
        let in_1 = e_stop(mid(one[one.len() / 2]));
        assert_eq!(cuts(&in_1), [(0, Some(1))]);
        on_pass_1(&at_pass_2(in_1));
        // Standing in pass 2: cut up to there, so no Z.
        let in_2 = e_stop(mid(two[two.len() / 2]));
        assert_eq!(cuts(&in_2), [(0, Some(2))]);
        refused(at_pass_2(in_2));
        // On no move, or where it stopped not known (on the Pi): all it
        // ran may be cut, or only some of it (LIO-601).
        let lost = e_stop([1e3; 6]);
        let may = MAY_HAVE_CUT_ALL;
        assert_eq!(cuts(&lost), [(0, may), (1, may)]);
        refused(at_pass_2(lost));
        let unknown = ran_at(false, None, "E-stop", Some(one[0]), None);
        assert_eq!(cuts(&unknown), [(0, may), (1, may)]);
        refused(at_pass_2(unknown));
        // Stopped after the Face's last pass, or after the Pocket's last
        // step: each cut whole (LIO-601 review); but not where an E-stop
        // ended it as it stopped.
        let last = face.steps.len() - 1;
        assert_eq!(cuts(&ran(false, Some((0, last)))), [(0, None)]);
        let e_stopped = ran_at(false, Some((0, last)), "E-stop", None, None);
        assert_eq!(cuts(&e_stopped), [(0, Some(last + 1))]);
        let pocket_steps = left.steps.iter().find(|s| s.op == 1).unwrap().steps.len();
        let at_end = ran(false, Some((1, pocket_steps - 1)));
        assert_eq!(cuts(&at_end), [(0, None), (1, None)]);
        // Followed into the Pocket (LIO-601 review): the Face was cut whole,
        // stopped where the run cannot tell (on the Pi), or dropped while
        // the Pocket ran (`cuts_so_far`); only the Pocket may be cut in
        // part. Its run finished, all is cut whole, so a later setup goes
        // by it again (`earlier_unfinished`).
        let pocket = (0..blocks.len()).find(|&j| marks[j].is_some_and(|m| m.0 == 1));
        let pocket = pocket.unwrap();
        // Stop after step at the Pocket's end, ended by an E-stop: the Face
        // is whole only once it was followed past (LIO-601 review).
        let at = Some((1, pocket_steps - 1));
        let unfollowed = ran_at(false, at, "E-stop", None, None);
        assert_eq!(cuts(&unfollowed)[0], (0, may));
        let followed = ran_at(false, at, "E-stop", Some(pocket), None);
        assert_eq!(cuts(&followed)[0], (0, None));
        let in_pocket = ran_at(false, None, "E-stop", Some(pocket), None);
        assert_eq!(cuts(&in_pocket), [(0, None), (1, may)]);
        let dropped = running(Some(pocket)).cuts_so_far();
        let dropped: Vec<_> = dropped.iter().map(|c| (c.0, c.1)).collect();
        assert_eq!(dropped, [(0, None), (1, may)]);
        // Standing in the Pocket, the Face too.
        let far = (0..blocks.len()).rev().find(|&j| {
            path[j].is_some()
                && marks[j]
                    .and_then(|m| plain.mark_step(&p, m))
                    .is_some_and(|m| m.0 == 1)
        });
        let in_pocket = ran_at(false, None, "E-stop", Some(pocket), Some(mid(far.unwrap())));
        assert_eq!(cuts(&in_pocket)[0], (0, None));
        let mut later = Run::new(&doc, 0, 0);
        later.earlier_cuts = vec![("setup 1".into(), in_pocket.cut_in_run.clone())];
        assert!(later.earlier_unfinished());
        let mut finished = in_pocket;
        finished.note_cut(1, None);
        assert_eq!(cuts(&finished), [(0, None), (1, None)]);
        later.earlier_cuts = vec![("setup 1".into(), finished.cut_in_run.clone())];
        assert!(!later.earlier_unfinished());
    }

    /// A journal another session wrote, picked up again (LIO-586 review):
    /// it says where its run last stopped, not what it cut before (all of
    /// the setup, then Face cut again from pass 1 and stopped after it,
    /// say), which only that session knew. So no start sets Z from the
    /// check's stock (`HISTORY_UNKNOWN`): not Face pass 2, whatever stopped
    /// it (Stop after pass 1, an E-stop in pass 2, the Face's tool run done
    /// on the Pi); not when its program has changed since, so that where it
    /// stopped is not picked; not with an operation left out of the check
    /// as it is picked up and enabled again, nor after operations deleted
    /// and added since. A run that follows a tool run another session sent
    /// knows no more. The journal this session wrote is in what its runs
    /// cut (`App::run_cuts`): that record stands.
    #[test]
    fn a_journal_picked_again_keeps_what_its_run_may_have_cut() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("template.toml");
        std::fs::write(&profile, uc100::config::TEMPLATE).unwrap();
        let (doc, mesh) = tablet_fixture(&profile);
        let (whole, left) = check_steps(&doc, &mesh, 0, &[], &[]);
        let (whole, left) = (Arc::new(whole), Arc::new(left));
        let pass_1 = (left.steps.iter().find(|s| s.op == 0).unwrap().steps[0].z).unwrap();
        let journal = |op: usize, last: Option<usize>, why: &str| {
            let mut run = Run::new(&doc, 0, 0);
            run.checked_as(whole.clone(), Some(left.clone()), None);
            let st = Stopped {
                op,
                last,
                up: None,
                why: why.into(),
            };
            run.journal_of(&st, 0, [0.; 6]).unwrap()
        };
        // Picked with the setup, this session's (`own`) or another's, with
        // what this session's runs cut.
        let picked = |j: Journal, cuts: Vec<CutInRun>, own: bool| {
            let mut run = Run::new(&doc, 0, 0);
            (run.journal, run.journal_own, run.cut_in_run) = (Some(j), own, cuts);
            run.checked_as(whole.clone(), Some(left.clone()), None);
            run
        };
        let at_step = |mut run: Run, oi: usize, n: usize| {
            if run.start_step != Some((oi, n)) {
                assert!(run.start_at_step(oi, n));
            }
            let (c, l) = check_steps(&doc, &mesh, 0, &[], &[(oi, (0..n).collect())]);
            run.checked_as(Arc::new(c), Some(Arc::new(l)), None);
            assert_eq!(run.at_step(), Some((oi, n)));
            run
        };
        let unknown = |run: &Run| run.cut_in_run.iter().any(|c| c.0 == HISTORY_UNKNOWN);
        let refused = |run: &Run| {
            assert!(unknown(run), "{:?}", run.cut_in_run);
            // Said as runs this computer did not follow (LIO-595).
            assert_eq!(run.partly().unwrap().cut, [HISTORY_UNFOLLOWED]);
            assert_eq!(run.touch_off(), None);
            assert_eq!(
                run.partway_blocked().as_deref(),
                Some(HISTORY_UNFOLLOWED_BLOCKED)
            );
            // Keep this work zero stays.
            assert!(run.partway() && !run.zeroed);
        };
        let on_pass_1 = |run: &Run| {
            let (at, _) = run.touch_off().unwrap();
            assert!((at[2] - pass_1).abs() < 1e-3, "{at:?} {pass_1}");
        };
        // Without a journal, Face pass 2 touches off on pass 1's floor and
        // Drilling hole 2 on the pocket's: it is the journal that tells.
        let fresh = || {
            let mut run = Run::new(&doc, 0, 0);
            run.checked_as(whole.clone(), Some(left.clone()), None);
            run
        };
        on_pass_1(&at_step(fresh(), 0, 1));
        assert!(at_step(fresh(), 2, 1).touch_off().is_some());
        for j in [
            journal(0, Some(0), "Stop after step"),
            journal(0, Some(0), "an E-stop"),
            journal(2, None, ""),
        ] {
            let run = picked(j, vec![], false);
            assert!(run.resumed.is_some());
            refused(&at_step(run, 0, 1));
        }
        // Its program changed since (another feed, say): where it stopped
        // is not picked, but what it may have cut is still taken.
        let mut changed = journal(0, Some(0), "Stop after step");
        changed.fingerprint ^= 1;
        let run = picked(changed, vec![], false);
        assert!(run.resumed.is_none() && run.journal_note.is_some());
        refused(&at_step(run, 0, 1));
        // The Drilling left out of the check as it is picked up (disabled),
        // or deleted with the Pocket before and added again: its hole 2
        // sets no Z either.
        let mut off = doc.clone();
        off.setups[0].operations[2].enabled = false;
        let mut face_only = doc.clone();
        face_only.setups[0].operations.truncate(1);
        for edited in [off, face_only] {
            let (c, l) = check_steps(&edited, &mesh, 0, &[], &[]);
            let mut run = Run::new(&edited, 0, 0);
            run.journal = Some(journal(0, Some(0), "Stop after step"));
            run.checked_as(Arc::new(c), Some(Arc::new(l)), None);
            run.checked_as(whole.clone(), Some(left.clone()), None);
            refused(&at_step(run, 2, 1));
        }
        // A run following another session's tool run: the same.
        let mut follows = fresh();
        follows.note_history_unknown();
        refused(&at_step(follows, 0, 1));
        // This session's journal, with what its runs cut (Face pass 1,
        // given as `App::run_cuts` does): that record stands.
        let mut known = fresh();
        known.note_cut(0, Some(1));
        let session = known.cut_in_run.clone();
        let run = picked(
            journal(0, Some(0), "Stop after step"),
            session.clone(),
            true,
        );
        assert!(run.resumed.is_some());
        assert_eq!(run.cut_in_run, session);
        on_pass_1(&at_step(run, 0, 1));
    }

    /// A file is this session's only as it wrote it (`write_own`, LIO-586
    /// review): written again since, the same bytes or not, by another
    /// session say, it is not; nor with its modified time put back as it
    /// was (a copy put back with `cp -p`); nor one it never wrote, nor once
    /// AXIS CAM is opened again.
    #[cfg(unix)]
    #[test]
    fn a_file_is_this_sessions_only_as_it_wrote_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("uc100").join("m-journal.json");
        let own = |path: &std::path::Path| read_own(path).map(|(_, own)| own);
        assert_eq!(own(&path), None);
        crate::storage::write_atomic(&path, b"{}").unwrap();
        assert_eq!(own(&path), Some(false));
        write_own(&path, b"{}").unwrap();
        assert_eq!(own(&path), Some(true));
        let was = std::fs::metadata(&path).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&path, "{}").unwrap();
        assert_eq!(own(&path), Some(false));
        let file = std::fs::File::options().write(true).open(&path).unwrap();
        file.set_modified(was).unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), was);
        assert_eq!(own(&path), Some(false));
        write_own(&path, b"{}").unwrap();
        assert_eq!(own(&path), Some(true));
        forget_own_files();
        assert_eq!(own(&path), Some(false));
        // Another process writes the same bytes in place just after it is
        // renamed there, or just before it is read back: not noted.
        for at in [0, 1] {
            MEANWHILE.with(|m| {
                *m.borrow_mut() = Some(Box::new(move |i, p| {
                    if i == at {
                        std::thread::sleep(std::time::Duration::from_millis(20));
                        std::fs::write(p, "{}").unwrap();
                    }
                }))
            });
            write_own(&path, b"{}").unwrap();
            MEANWHILE.with(|m| *m.borrow_mut() = None);
            assert_eq!(own(&path), Some(false), "{at}");
        }
        write_own(&path, b"{}").unwrap();
        assert_eq!(own(&path), Some(true));
    }

    /// A plan's moves cut with another cutter, or at another A, cut other
    /// stock: the journal's fingerprint is the same, the cut print is not
    /// (LIO-582 review: a drill saved at Ø1 after cutting at Ø8).
    #[test]
    fn the_cut_print_follows_the_cutter_and_a() {
        use crate::simulation::CutterShape;
        let plan = fixture::plan([0., 0., 15.], &[(Motion::Feed, [0., 0., -5.], 100.)]);
        let same = |p: &crate::simulation::Plan| {
            (
                fingerprint(p, "t1") == fingerprint(&plan, "t1"),
                cut_print(p, "t1") == cut_print(&plan, "t1"),
            )
        };
        assert_eq!(same(&plan.clone()), (true, true));
        let mut smaller = plan.clone();
        smaller.radius /= 8.;
        assert_eq!(same(&smaller), (true, false));
        let mut drill = plan.clone();
        drill.shape = CutterShape::Drill;
        assert_eq!(same(&drill), (true, false));
        let mut shorter = plan.clone();
        shorter.flute_length /= 2.;
        assert_eq!(same(&shorter), (true, false));
        let mut turned = plan.clone();
        turned.moves[0].a = Some(90.);
        assert_eq!(same(&turned), (true, false));
        assert_ne!(cut_print(&plan, "t2"), cut_print(&plan, "t1"));
    }

    /// A Ø8 hole drilled 3 mm from the stock's centre, cut in this run, then
    /// the drill saved at Ø1 and the setup checked again (LIO-582 review):
    /// its moves and tool are the same, so the journal's fingerprint is, but
    /// the check's Ø1 hole is not the one cut, and the start at the face
    /// that follows gives no Z.
    #[test]
    fn a_start_after_a_cut_whose_tool_changed_since_sets_no_z() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("template.toml");
        std::fs::write(&profile, uc100::config::TEMPLATE).unwrap();
        let (mut doc, _) = tablet_fixture(&profile);
        let (path, mesh, [mut drill, small]) = ring(dir.path());
        drill.p.insert("type".into(), "Drill".into());
        drill.p.insert("diameter".into(), "8".into());
        let mut setup = ring_setup(&doc, "G54", "1", "0");
        setup.p.insert("stock_right".into(), "6".into());
        let top = ring_top(&mesh, &setup, true);
        let mut bore = ring_bore(&drill, top.clone(), "4");
        bore.p.insert("cycle".into(), "Drilling".into());
        setup.operations = vec![bore, ring_face(&small, top)];
        doc.active = setup.id.clone();
        doc.setups = vec![setup];
        doc.tools = vec![drill, small];
        if let Some(f) = &mut doc.file {
            (f.path, f.name, f.format) = (path, "ring.stl".into(), "stl".into());
        }
        let check_at_face = |doc: &Document, run: &mut Run| {
            let (c, l) = check(doc, &mesh, 0);
            assert!(c.is_ok(), "{c:?}");
            run.checked_as(Arc::new(c), Some(Arc::new(l)), None);
            run.start_at(1, 0);
        };
        let mut run = Run::new(&doc, 0, 0);
        check_at_face(&doc, &mut run);
        run.note_cut(0, None);
        let print = run.op_steps(0).unwrap().fingerprint;
        assert!(run.partly().is_some_and(|p| p.cut != [RUN_UNFOLLOWED]));
        doc.tools[0].p.insert("diameter".into(), "1".into());
        check_at_face(&doc, &mut run);
        assert_eq!(run.op_steps(0).unwrap().fingerprint, print);
        assert!(run.partly().is_some_and(|p| p.cut == [RUN_UNFOLLOWED]));
        assert!(run.touch_off().is_none());
        let why = run.partway_blocked().unwrap();
        assert!(
            why.starts_with("This run's tool runs have cut the stock since"),
            "{why}"
        );
    }

    /// LIO-607: another project's setups' cuts, kept with whose they are
    /// (`CutOwner`), are said as that project's, its name and the
    /// operations cut; a setup deleted from this project as any setup the
    /// check does not follow; one kept with no owner (an older AXIS CAM's)
    /// as maybe another project's (LIO-635).
    #[test]
    fn another_projects_cuts_are_said_as_its() {
        let doc = crate::post::fixture::job().0;
        let mut other = doc.clone();
        other.project_id = crate::id();
        other.title = "Bracket".into();
        for s in &mut other.setups {
            s.id = crate::id();
        }
        let mut third = other.clone();
        third.project_id = crate::id();
        third.title = "Lid".into();
        third.setups[0].id = crate::id();
        let (a, b) = (CutOwner::of(&other, 0), CutOwner::of(&third, 0));
        let (face, pocket) = (a.ops[0].clone(), a.ops[1].clone());
        // Of this project, but deleted since.
        let mut gone = CutOwner::of(&doc, 0);
        gone.setup = crate::id();
        let owners = [a.clone(), b.clone(), gone.clone()];
        let note = |cuts: &[(&str, &[usize])]| {
            let mut run = Run::new(&doc, 0, 0);
            run.earlier_cuts = (cuts.iter())
                .map(|(id, ops)| {
                    (
                        id.to_string(),
                        ops.iter().map(|&o| (o, None, None, None)).collect(),
                    )
                })
                .collect();
            give_elsewhere(&owners, &doc, &mut run);
            (run.earlier_unfollowed_why(), run.earlier_unfollowed_note())
        };
        // Why, said with the steps, and the note with New stock loaded.
        assert_eq!(
            note(&[(&a.setup, &[0, 1, 0, HISTORY_UNKNOWN])]),
            (
                format!(
                    "Another project's runs cut stock on this machine earlier (\"Bracket\": {face} and {pocket}), which this project's check does not follow."
                ),
                "Another project's runs (\"Bracket\") may have cut this stock.".into()
            )
        );
        assert_eq!(
            note(&[(&a.setup, &[1]), (&b.setup, &[0])]),
            (
                format!(
                    "Other projects' runs cut stock on this machine earlier (\"Bracket\": {pocket}; \"Lid\": {face}), which this project's check does not follow."
                ),
                "Other projects' runs (\"Bracket\" and \"Lid\") may have cut this stock.".into()
            )
        );
        // Not checked, the run has no plan to tell which way: no cause named
        // (LIO-599).
        let generic = (
            EarlierUnfollowed::Unknown.why(),
            "Another setup's run cut this part earlier in a way the check does not follow."
                .to_string(),
        );
        assert_eq!(note(&[(&gone.setup, &[0])]), generic);
        // Kept with no owner, and not this project's: maybe another
        // project's, said so (LIO-635).
        assert_eq!(
            note(&[("kept-with-no-owner", &[0])]),
            (
                EARLIER_ELSEWHERE_WHY.to_string(),
                "This computer's runs cut stock for a setup this project does not have: another project's, or one deleted since.".to_string(),
            )
        );
        assert_eq!(
            note(&[(&gone.setup, &[0]), (&a.setup, &[0])]),
            (
                format!(
                    "Another setup's run cut this part earlier in a way the check does not follow, and another project's runs cut stock on this machine earlier (\"Bracket\": {face}), which this project's check does not follow."
                ),
                "Another setup's run cut this part earlier in a way the check does not follow, and another project's runs (\"Bracket\") may have cut it too.".into()
            )
        );
        // A project with no name.
        let mut nameless = a.clone();
        (nameless.setup, nameless.title) = (crate::id(), " ".into());
        let owners = [nameless.clone()];
        let mut run = Run::new(&doc, 0, 0);
        run.earlier_cuts = vec![(nameless.setup.clone(), vec![(0, None, None, None)])];
        give_elsewhere(&owners, &doc, &mut run);
        assert_eq!(
            run.earlier_unfollowed_note(),
            "Another project's runs (one with no name) may have cut this stock."
        );
        // A history kept before owners were kept still reads.
        let old =
            r#"{"cuts":[["s",[[0,null,null]]]],"journals":[],"others_unknown":false,"sent":[]}"#;
        let h: CutHistory = serde_json::from_str(old).unwrap();
        assert!(h.owners.is_empty() && h.cuts.len() == 1);
    }

    /// Setup 1 bores the ring from below, leaving a 0.5 mm roof, cut in a
    /// run this session; setup 2, flipped, faces the roof away (LIO-582
    /// review). While setup 1's bore is checked as it was cut, setup 2's
    /// start at its Face's level 2 sees it; disabled since, or its drill
    /// changed, setup 2's check no longer has it, and no start of setup 2
    /// touches off or probes. New stock loaded forgets it, here and in the
    /// session's record.
    #[test]
    fn an_earlier_setups_cut_disabled_since_blocks_a_later_setups_touch_off() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("template.toml");
        std::fs::write(&profile, uc100::config::TEMPLATE).unwrap();
        let (mut doc, _) = tablet_fixture(&profile);
        let (path, mesh, [mill, small]) = ring(dir.path());
        let mut first = ring_setup(&doc, "G54", "1", "1");
        let top = ring_top(&mesh, &first, true);
        first.operations = vec![ring_bore(&mill, top, "0.5")];
        let mut flipped = ring_setup(&doc, "G55", "1", "1");
        flipped.p.insert("rx".into(), "180".into());
        let top = ring_top(&mesh, &flipped, false);
        let mut opening = ring_face(&mill, top.clone());
        opening.p.insert("stepdown".into(), "0.6".into());
        flipped.operations = vec![opening, ring_face(&small, top)];
        doc.active = flipped.id.clone();
        doc.setups = vec![first, flipped];
        doc.tools = vec![mill, small];
        if let Some(f) = &mut doc.file {
            (f.path, f.name, f.format) = (path, "ring.stl".into(), "stl".into());
        }
        // Setup 1's run cut its bore: its print as the check has it now.
        let (c, l) = check(&doc, &mesh, 0);
        let mut one = Run::new(&doc, 0, 0);
        one.checked_as(Arc::new(c), Some(Arc::new(l)), None);
        one.note_cut(0, None);
        let session = vec![(doc.setups[0].id.clone(), one.cut_in_run.clone())];
        let at_level_2_of = |doc: &Document, session: &[(String, Vec<CutInRun>)]| {
            // The flipped setup, wherever it is now.
            let si = (doc.setups.iter()).position(|s| s.p.get("rx").is_some_and(|r| r == "180"));
            let si = si.unwrap();
            let mut run = Run::new(doc, si, 0);
            crate::give_earlier_cuts(session, &[], &mut run);
            let (c, l) = check(doc, &mesh, si);
            run.checked_as(Arc::new(c), Some(Arc::new(l)), None);
            let level_2 = run.op_steps(0).is_some_and(|s| s.steps.len() >= 2);
            assert!(level_2 && run.start_at_step(0, 1));
            let (c, l) = check_steps(doc, &mesh, si, &[], &[(0, vec![0])]);
            run.checked_as(Arc::new(c), Some(Arc::new(l)), None);
            assert_eq!(run.at_step(), Some((0, 1)));
            run
        };
        let at_level_2 = |doc: &Document| at_level_2_of(doc, &session);
        let unfollowed = |run: &Run| run.partly().is_some_and(|p| p.cut == [EARLIER_UNFOLLOWED]);
        // As it was cut: setup 2's check has the bore (and blocks the
        // floor over it, as before).
        let run = at_level_2(&doc);
        assert_eq!(run.earlier_cuts, session);
        assert!(!run.earlier_unfollowed() && !unfollowed(&run));
        assert!(run.touch_off().is_none());
        // Disabled since, drilled with a smaller tool, its setup deleted
        // or moved after the flipped one: not the stock's.
        let mut off = doc.clone();
        off.setups[0].operations[0].enabled = false;
        let mut smaller = doc.clone();
        smaller.tools[0].p.insert("diameter".into(), "4".into());
        let mut deleted = doc.clone();
        deleted.remove_setup(0).unwrap();
        let mut moved = doc.clone();
        moved.move_setup(0, 1).unwrap();
        assert!(moved.setups[1].id == doc.setups[0].id);
        // Each said by its case (LIO-599); a setup deleted since as one this
        // project does not have (LIO-635).
        let cases = [
            (off, Some(EarlierUnfollowed::Changed(0))),
            (smaller, Some(EarlierUnfollowed::Changed(0))),
            (deleted, None),
            (moved, Some(EarlierUnfollowed::After(1))),
        ];
        for (edited, how) in cases {
            let mut run = at_level_2(&edited);
            assert_eq!(run.earlier_unfollowed_how(), how);
            assert!(run.earlier_unfollowed() && unfollowed(&run));
            assert!(run.touch_off().is_none());
            let why = run.partway_blocked().unwrap();
            // A setup deleted since is one this project does not have, as
            // another project's is (LIO-635).
            let gone = !(edited.setups.iter()).any(|s| s.id == doc.setups[0].id);
            assert_eq!(run.earlier_not_here(), gone);
            assert_eq!(
                why,
                match how {
                    None => EARLIER_ELSEWHERE_BLOCKED.to_string(),
                    Some(how) => how.blocked(),
                },
            );
            let probes = &run.partly().unwrap().probe_from;
            let said = how.map_or(EARLIER_ELSEWHERE_WHY.to_string(), |h| h.why());
            assert!(
                probes.iter().all(|(_, w)| *w == Err(said.clone())),
                "{probes:?}"
            );
            // From setup 2's start too.
            run.start_at(0, 0);
            assert!(run.on_cut_stock() && unfollowed(&run) && run.touch_off().is_none());
            // New stock loaded: forgotten, in the session's record too.
            run.earlier_cuts.clear();
            let mut kept = session.clone();
            crate::keep_run_cuts(&mut kept, &run, false);
            assert!(kept.is_empty(), "{kept:?}");
            assert!(!run.earlier_unfollowed());
        }
        // Something else may have cut the part since (LIO-596): setup 1's
        // record, as setup 2 has it, is unknown, and so is the level setup
        // 2's start at level 2 takes as cut (LIO-596 review).
        let mut run = at_level_2(&doc);
        assert!(run.note_cut_elsewhere(), "{:?}", run.earlier_cuts);
        assert!(!run.note_cut_elsewhere(), "noted once");
        assert_eq!(run.cut_in_run, [(HISTORY_UNKNOWN, None, None, None)]);
        let unknown = |run: &Run| run.partly().is_some_and(|p| p.cut == [HISTORY_UNFOLLOWED]);
        let said = |run: &Run| {
            let why = run.partway_blocked().unwrap();
            assert!(
                why.starts_with("Runs this computer did not follow"),
                "{why}"
            );
        };
        assert!(run.history_unknown() && unknown(&run) && run.touch_off().is_none());
        said(&run);
        // So is it in the session's record (`App::run_cuts`): a run of
        // setup 2 made anew has it so, from its start too.
        let mut marked = session.clone();
        assert!(note_unknown(&mut marked[0].1) && !note_unknown(&mut marked[0].1));
        let mut run = at_level_2_of(&doc, &marked);
        assert!(run.history_unknown() && unknown(&run) && run.touch_off().is_none());
        said(&run);
        run.start_at(0, 0);
        assert!(run.on_cut_stock() && unknown(&run) && run.touch_off().is_none());
        // An empty record says nothing was cut, and stays so.
        assert!(!note_unknown(&mut vec![]));
        // Kept across a disconnect, setup 1's record is asked about when
        // setup 2 is picked, though setup 2's runs cut nothing: the answer
        // is said of the part (LIO-596).
        let away = vec![(
            doc.setups[0].id.clone(),
            Away {
                seen: Some(vec![7]),
            },
        )];
        let mut run = Run::new(&doc, 1, 0);
        crate::give_earlier_cuts(&session, &[], &mut run);
        let (c, l) = check(&doc, &mesh, 1);
        run.checked_as(Arc::new(c), Some(Arc::new(l)), None);
        // Before the disconnect, its start touches off on the stock the
        // check carves with setup 1's cuts.
        assert!(run.touch_off().is_some() && run.partway_blocked().is_none());
        crate::give_away(&away, &mut run);
        assert!(run.stock_unconfirmed);
        assert_eq!(
            run.away,
            Some(Away {
                seen: Some(vec![7])
            })
        );
        // Until answered, nothing sets Z from the stock the check carves:
        // the part may not be the one held, or cut since (Fable review).
        assert!(run.touch_off().is_none());
        assert_eq!(run.partway_blocked().as_deref(), Some(STOCK_AWAY_BLOCKED));
        // Same part, nothing else ran: as before.
        (run.stock_unconfirmed, run.away) = (false, None);
        assert!(run.touch_off().is_some() && run.partway_blocked().is_none());
        // Nothing of setup 1's kept: nothing to ask.
        let mut run = Run::new(&doc, 1, 0);
        crate::give_earlier_cuts(&[], &[], &mut run);
        crate::give_away(&away, &mut run);
        assert!(!run.stock_unconfirmed && run.away.is_none());
        // Setup 1 deleted since: still asked for, as put back its cuts are
        // followed again (LIO-596 review).
        let mut deleted = doc.clone();
        deleted.remove_setup(0).unwrap();
        let si = (deleted.setups.iter()).position(|s| s.p.get("rx").is_some_and(|r| r == "180"));
        let mut run = Run::new(&deleted, si.unwrap(), 0);
        crate::give_earlier_cuts(&session, &[], &mut run);
        crate::give_away(&away, &mut run);
        assert!(run.stock_unconfirmed && run.away.is_some());
        // Another project's setup kept so (`earlier_elsewhere`): its cuts
        // set no Z until New stock loaded whatever is answered, and are said
        // as that project's (LIO-607), so nothing is asked for them.
        let mut run = Run::new(&deleted, si.unwrap(), 0);
        crate::give_earlier_cuts(&session, &[], &mut run);
        let mut owner = CutOwner::of(&doc, 0);
        owner.project = "another project".into();
        run.earlier_elsewhere = vec![owner];
        crate::give_away(&away, &mut run);
        assert!(!run.stock_unconfirmed && run.away.is_none());
    }

    /// Setup 1 bores the ring from below in a run this session; setup 2,
    /// flipped, faces the roof left over the bore (LIO-590). Disabled since,
    /// the plan alone has no bore, and a tablet file made from it alone let
    /// setup 2's start touch off on the stock top, in the bore. Made with
    /// what this session cut, as Save for tablet makes it, each of its work
    /// zeros refuses Z and every probe spot, in the tablet's words, as the
    /// Run panel refuses them. A cut the plan follows changes nothing for
    /// the setups after it; a setup's own runs' cut refuses its zeros until
    /// it is known whether that part is held, which the tablet cannot say;
    /// and a setup the cut came after, on fresh stock, is refused too, as on
    /// the Run panel.
    #[test]
    fn the_tablet_file_refuses_a_touch_off_over_a_session_cut_the_plan_no_longer_follows() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("template.toml");
        std::fs::write(&profile, uc100::config::TEMPLATE).unwrap();
        let (mut doc, _) = tablet_fixture(&profile);
        let (path, mesh, [mill, small]) = ring(dir.path());
        let mut first = ring_setup(&doc, "G54", "1", "1");
        let top = ring_top(&mesh, &first, true);
        first.operations = vec![ring_bore(&mill, top, "0.5")];
        let mut flipped = ring_setup(&doc, "G55", "1", "1");
        flipped.p.insert("rx".into(), "180".into());
        let top = ring_top(&mesh, &flipped, false);
        let mut opening = ring_face(&mill, top.clone());
        opening.p.insert("stepdown".into(), "0.6".into());
        flipped.operations = vec![opening, ring_face(&small, top)];
        doc.active = flipped.id.clone();
        doc.setups = vec![first, flipped];
        doc.tools = vec![mill, small];
        if let Some(f) = &mut doc.file {
            (f.path, f.name, f.format) = (path, "ring.stl".into(), "stl".into());
        }
        let cut_by = |doc: &Document, si: usize| {
            let (c, l) = check(doc, &mesh, si);
            let mut run = Run::new(doc, si, 0);
            run.checked_as(Arc::new(c), Some(Arc::new(l)), None);
            run.note_cut(0, None);
            vec![(doc.setups[si].id.clone(), run.cut_in_run.clone())]
        };
        let session = cut_by(&doc, 0);
        let setup_in = |doc: &Document, si: usize, session: &Session| {
            let runs = crate::bundle::runs(doc, Ok(&mesh), session);
            let s = (runs.setups.into_iter()).find(|s| s.setup_id == doc.setups[si].id);
            let s = s.unwrap();
            assert!(s.problems.is_empty(), "{:?}", s.problems);
            s
        };
        let setup = |doc: &Document, si: usize, cuts: &SessionCuts| {
            let session = Session {
                cuts: cuts.to_vec(),
                ..Session::default()
            };
            setup_in(doc, si, &session)
        };
        // Every work zero the file carries, the setup's start's first.
        let zeros = |s: &TabletSetup| -> Vec<Option<TabletZero>> {
            (s.tool_runs.iter())
                .flat_map(|r| {
                    let starts = r.starts.iter().map(|t| t.zero.clone());
                    let steps = r.step_starts.iter().map(|t| t.zero.clone());
                    std::iter::once(r.zero.clone()).chain(starts).chain(steps)
                })
                .collect()
        };
        // No tablet text names what only the computer has, but as the
        // computer's way out of another setup's cut (LIO-635).
        for (computer, said) in FOR_TABLET {
            assert_eq!(for_tablet(computer.into()), said);
            let said = said.replace(new_stock_on_the_computer!(), "");
            for only in ["Same part", "New stock", "under Work zero", "this computer"] {
                assert!(!said.contains(only), "{said}");
            }
        }
        let tablet = |computer: &str| for_tablet(computer.into());
        // All refused, in the tablet's words for the Run panel's `blocked`
        // and `why`, the setup's start's included.
        // Every start carries a zero: one with none leaves the tablet's
        // Zero Z on there (LIO-590 review).
        let refused_as = |s: &TabletSetup, blocked: &str, why: &str| {
            let all = zeros(s);
            assert!(
                all.iter().all(Option::is_some),
                "a start carries no zero: {all:?}"
            );
            for z in all.into_iter().flatten() {
                assert_eq!(z.set_z, None);
                assert_eq!(z.blocked.as_deref(), Some(blocked));
                assert!(!z.probe_from.is_empty());
                for spot in &z.probe_from {
                    assert_eq!(spot.problem.as_deref(), Some(why));
                }
            }
        };
        let refused = |s: &TabletSetup, blocked: &str, why: &str| {
            refused_as(s, &tablet(blocked), &tablet(why))
        };
        // This project's setup's cut not followed, by its case (LIO-599),
        // in no words only the computer has.
        let refused_by = |s: &TabletSetup, how: EarlierUnfollowed| {
            let (blocked, why) = (how.tablet_blocked(), how.tablet_why());
            for said in [&blocked, &why] {
                let said = said.replace(new_stock_on_the_computer!(), "");
                for only in ["Same part", "New stock", "under Work zero", "this computer"] {
                    assert!(!said.contains(only), "{said}");
                }
            }
            refused_as(s, &blocked, &why)
        };
        // The bore disabled since: the Run panel refuses setup 2's start.
        let mut off = doc.clone();
        off.setups[0].operations[0].enabled = false;
        let mut run = Run::new(&off, 1, 0);
        run.picked_in(&session, false, &[]);
        let (c, l) = check(&off, &mesh, 1);
        run.checked_as(Arc::new(c), Some(Arc::new(l)), None);
        assert!(run.on_cut_stock() && run.touch_off().is_none());
        assert_eq!(
            run.partway_blocked(),
            Some(EarlierUnfollowed::Changed(0).blocked())
        );
        // From the plan alone, Zero Z was on there.
        assert_eq!(setup(&off, 1, &[]).tool_runs[0].zero, None);
        // With this session's cuts, it is off, and so is every start's:
        // the face's tool run's and its levels'.
        let two = setup(&off, 1, &session);
        assert_eq!(two.tool_runs.len(), 2);
        assert!(!two.tool_runs[0].step_starts.is_empty());
        refused_by(&two, EarlierUnfollowed::Changed(0));
        assert_eq!(
            EarlierUnfollowed::Changed(0).tablet_blocked(),
            format!(
                "Setup 1's run on the computer cut this part before this tablet file was made, with an operation disabled, changed or deleted since, so the computer cannot tell the surface where the stock top is touched off.{}",
                new_stock_on_the_computer!()
            )
        );
        // Deleted since: refused too, as a setup this project does not
        // have, as another project's is (LIO-635).
        let mut deleted = doc.clone();
        deleted.remove_setup(0).unwrap();
        refused(
            &setup(&deleted, 0, &session),
            EARLIER_ELSEWHERE_BLOCKED,
            EARLIER_ELSEWHERE_WHY,
        );
        // The bore as the plan has it: setup 2's is the plan's alone. Setup
        // 1's own run cut it: whether that part is still held, the tablet
        // cannot say.
        assert_eq!(setup(&doc, 1, &session), setup(&doc, 1, &[]));
        refused(
            &setup(&doc, 0, &session),
            STOCK_UNCONFIRMED_BLOCKED,
            STOCK_UNCONFIRMED_WHY,
        );
        // Setup 2's own run cut its opening face: whether that part is
        // still held, the tablet cannot say.
        let own = cut_by(&doc, 1);
        let (cut, plan) = (setup(&doc, 1, &own), setup(&doc, 1, &[]));
        refused(&cut, STOCK_UNCONFIRMED_BLOCKED, STOCK_UNCONFIRMED_WHY);
        // Its jobs are the plan's still: only the zeros go by the session.
        for (a, b) in cut.tool_runs.iter().zip(&plan.tool_runs) {
            assert_eq!(
                (&a.actions, &a.steps, a.lift),
                (&b.actions, &b.steps, b.lift)
            );
            for (x, y) in a.step_starts.iter().zip(&b.step_starts) {
                assert_eq!(
                    (&x.actions, &x.steps, x.rest),
                    (&y.actions, &y.steps, y.rest)
                );
            }
        }
        // Setup 1, on fresh stock, before the setup that cut: as the Run
        // panel, which does not simulate it before setup 1.
        assert_eq!(crate::model::get(&doc.setups[0].p, "stock_source"), "fresh");
        // Said as setup 2 cut first, nothing edited (LIO-599).
        refused_by(&setup(&doc, 0, &own), EarlierUnfollowed::After(1));
        assert_eq!(
            EarlierUnfollowed::After(1).tablet_blocked(),
            format!(
                "Setup 2 comes after this one but its run on the computer cut this part first, before this tablet file was made, so the computer cannot tell the surface where the stock top is touched off.{}",
                new_stock_on_the_computer!()
            )
        );
        // What runs cut was lost (LIO-594): a setup with none kept was cut
        // by runs no session here knows of, as the Run panel's pick has it
        // (`note_lost_cuts`); one kept as having cut nothing was not.
        let lost = |cuts: Vec<(String, Vec<CutInRun>)>| Session {
            cuts,
            lost: true,
            ..Session::default()
        };
        let mut run = Run::new(&doc, 1, 0);
        run.picked_in(&[], true, &[]);
        let (c, l) = check(&doc, &mesh, 1);
        run.checked_as(Arc::new(c), Some(Arc::new(l)), None);
        assert!(run.history_unknown() && run.touch_off().is_none());
        refused(
            &setup_in(&doc, 1, &lost(vec![])),
            STOCK_UNCONFIRMED_BLOCKED,
            STOCK_UNCONFIRMED_WHY,
        );
        let kept_none = lost(vec![(doc.setups[1].id.clone(), vec![])]);
        assert_eq!(setup_in(&doc, 1, &kept_none), setup(&doc, 1, &[]));
        // Setup 2's opening face taken as cut on the computer, nothing cut
        // there (LIO-583): picked again, the Run panel refuses every start
        // until it is said whether that part is held, and so does the file.
        let mut run = Run::new(&doc, 1, 0);
        (run.skips, run.stock_unconfirmed) = (vec![(0, Skip::Cut)], true);
        let (c, l) = check_skipping(&doc, &mesh, 1, &run.skips);
        run.checked_as(Arc::new(c), Some(Arc::new(l)), None);
        assert!(run.on_cut_stock() && run.touch_off().is_none());
        run.start_at(0, 0);
        assert!(run.touch_off().is_none());
        let declared = Session {
            declared: vec![KeptDeclared {
                setup: doc.setups[1].id.clone(),
                skips: vec![(0, Skip::Cut)],
                steps: vec![],
                lost: false,
                prints: vec![],
            }],
            ..Session::default()
        };
        refused(
            &setup_in(&doc, 1, &declared),
            STOCK_UNCONFIRMED_BLOCKED,
            STOCK_UNCONFIRMED_WHY,
        );
        // Another setup's: setup 2's own stock is the plan's.
        let other = Session {
            declared: vec![KeptDeclared {
                setup: doc.setups[0].id.clone(),
                skips: vec![(0, Skip::Cut)],
                steps: vec![],
                lost: false,
                prints: vec![],
            }],
            ..Session::default()
        };
        assert_eq!(setup_in(&doc, 1, &other), setup(&doc, 1, &[]));
    }

    /// LIO-638: project A ("Bracket") bores the ring in a run; project B,
    /// the same part and plan, is opened and saved for the tablet with what
    /// the computer's runs cut and whose they are (`Session::owners`). Every
    /// work zero of B's file says the bore as A's, named, in the tablet's
    /// words, with the way out on the computer (LIO-635), and nothing
    /// disabled, deleted or moved; and refuses Z and every probe spot
    /// exactly as with the owners not known. A's own file is the same with
    /// them or not.
    #[test]
    fn a_tablet_file_says_another_projects_cuts_as_its() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("template.toml");
        std::fs::write(&profile, uc100::config::TEMPLATE).unwrap();
        let (mut a, _) = tablet_fixture(&profile);
        let (path, mesh, [mill, small]) = ring(dir.path());
        let mut first = ring_setup(&a, "G54", "1", "1");
        let top = ring_top(&mesh, &first, true);
        first.operations = vec![ring_bore(&mill, top, "0.5")];
        let mut flipped = ring_setup(&a, "G55", "1", "1");
        flipped.p.insert("rx".into(), "180".into());
        let top = ring_top(&mesh, &flipped, false);
        flipped.operations = vec![ring_face(&mill, top.clone()), ring_face(&small, top)];
        a.active = flipped.id.clone();
        a.setups = vec![first, flipped];
        a.tools = vec![mill, small];
        if let Some(f) = &mut a.file {
            (f.path, f.name, f.format) = (path, "ring.stl".into(), "stl".into());
        }
        a.title = "Bracket".into();
        let mut b = a.clone();
        (b.project_id, b.title) = (crate::id(), "Lid".into());
        for s in &mut b.setups {
            s.id = crate::id();
        }
        let bore = a.setups[0].operations[0].name().to_string();
        let (c, l) = check(&a, &mesh, 0);
        let mut run = Run::new(&a, 0, 0);
        run.checked_as(Arc::new(c), Some(Arc::new(l)), None);
        run.note_cut(0, None);
        let cuts = vec![(a.setups[0].id.clone(), run.cut_in_run.clone())];
        let with = Session {
            cuts: cuts.clone(),
            owners: vec![CutOwner::of(&a, 0)],
            ..Session::default()
        };
        let without = Session {
            cuts,
            ..Session::default()
        };
        let setup_in = |doc: &Document, si: usize, session: &Session| {
            let runs = crate::bundle::runs(doc, Ok(&mesh), session);
            let s = (runs.setups.into_iter()).find(|s| s.setup_id == doc.setups[si].id);
            let s = s.unwrap();
            assert!(s.problems.is_empty(), "{:?}", s.problems);
            s
        };
        let zeros = |s: &TabletSetup| -> Vec<TabletZero> {
            let all: Vec<Option<TabletZero>> = (s.tool_runs.iter())
                .flat_map(|r| {
                    let starts = r.starts.iter().map(|t| t.zero.clone());
                    let steps = r.step_starts.iter().map(|t| t.zero.clone());
                    std::iter::once(r.zero.clone()).chain(starts).chain(steps)
                })
                .collect();
            assert!(
                !all.is_empty() && all.iter().all(Option::is_some),
                "{all:?}"
            );
            all.into_iter().flatten().collect()
        };
        let way_out = new_stock_on_the_computer!();
        // B's two setups, with the owners and without: refused alike, said
        // as `blocked` and `why` say.
        let said = |session: &Session, plain: &Session, blocked: &str, why: &str| {
            for si in 0..2 {
                let said = zeros(&setup_in(&b, si, session));
                let generic = zeros(&setup_in(&b, si, plain));
                assert_eq!(said.len(), generic.len());
                for (z, g) in said.into_iter().zip(generic) {
                    // Refused as before: no Z, every spot refused, the
                    // stock said the same.
                    assert_eq!((z.set_z, g.set_z), (None, None));
                    assert_eq!(
                        (&z.cut, z.top_cut, z.top, &z.spot),
                        (&g.cut, g.top_cut, g.top, &g.spot)
                    );
                    assert!(g.blocked.is_some());
                    assert_eq!(z.blocked.as_deref(), Some(blocked));
                    assert_eq!(z.probe_from.len(), g.probe_from.len());
                    assert!(!z.probe_from.is_empty());
                    for (spot, gs) in z.probe_from.iter().zip(&g.probe_from) {
                        assert_eq!((&spot.x, &spot.y), (&gs.x, &gs.y));
                        assert!(gs.problem.is_some());
                        assert_eq!(spot.problem.as_deref(), Some(why));
                    }
                }
            }
            for text in [blocked, why] {
                for only in [
                    "Same part",
                    "under Work zero",
                    "this computer",
                    "disabled",
                    "deleted",
                    "moved",
                ] {
                    assert!(!text.contains(only), "{only:?} in {text}");
                }
                assert!(text.contains("\"Bracket\""), "{text}");
                assert!(text.ends_with(way_out), "{text}");
                assert_eq!(text.matches("does not follow").count(), 1, "{text}");
            }
        };
        let cause = format!(
            "another project's runs on the computer cut stock on the machine before this tablet file was made (\"Bracket\": {bore}), which this project's plan does not follow"
        );
        let capital = format!("A{}", &cause[1..]);
        said(
            &with,
            &without,
            &format!(
                "{capital}, so the computer cannot tell the surface where the stock top is touched off.{way_out}"
            ),
            &format!("{capital}, so the computer does not know what they cut here.{way_out}"),
        );
        // With a cut kept with no owner too: one sentence.
        let (mut both, mut plain) = (with.clone(), without.clone());
        let mut unknown = both.cuts[0].clone();
        unknown.0 = "kept-with-no-owner".into();
        both.cuts.push(unknown.clone());
        plain.cuts.push(unknown);
        said(
            &both,
            &plain,
            &format!(
                "Another setup's run and {cause}, so the computer cannot tell the surface where the stock top is touched off.{way_out}"
            ),
            &format!(
                "Another setup's run and {cause}, so the computer does not know what they cut here.{way_out}"
            ),
        );
        // With B's own setup 1's bore cut too, disabled since: setup 2's
        // zeros say setup 1's case, then A's cuts, in one sentence.
        let mut off = b.clone();
        off.setups[0].operations[0].enabled = false;
        let mut mixed = with.clone();
        let mut mine = mixed.cuts[0].clone();
        mine.0 = b.setups[0].id.clone();
        mixed.cuts.insert(0, mine);
        let also = format!(
            "{}, and another project's runs on the computer cut stock on the machine too (\"Bracket\": {bore}), which this project's plan does not follow, so the computer cannot tell the surface where the stock top is touched off.{way_out}",
            EarlierUnfollowed::Changed(0).said_on_tablet()
        );
        assert!(also.starts_with("Setup 1's run on the computer"), "{also}");
        for z in zeros(&setup_in(&off, 1, &mixed)) {
            assert_eq!(z.set_z, None);
            assert_eq!(z.blocked.as_deref(), Some(&also[..]));
            assert!(!z.probe_from.is_empty());
            for spot in &z.probe_from {
                let why = spot.problem.as_deref().unwrap();
                assert!(why.contains("\"Bracket\""), "{why}");
            }
        }
        // A setup of B's own, deleted since, kept with its owner: said as
        // B's deleted setup, not as maybe another project's (LIO-599).
        let mut gone = CutOwner::of(&b, 0);
        gone.setup = crate::id();
        let deleted = Session {
            cuts: vec![(gone.setup.clone(), with.cuts[0].1.clone())],
            owners: vec![gone],
            ..Session::default()
        };
        for z in zeros(&setup_in(&b, 1, &deleted)) {
            assert_eq!(z.set_z, None);
            assert_eq!(z.blocked, Some(EarlierUnfollowed::Deleted.tablet_blocked()));
            assert!(!z.probe_from.is_empty());
            for spot in &z.probe_from {
                assert_eq!(spot.problem, Some(EarlierUnfollowed::Deleted.tablet_why()));
            }
        }
        // A's own file: its setups' cuts are its own, said as before.
        for si in 0..2 {
            assert_eq!(setup_in(&a, si, &with), setup_in(&a, si, &without));
        }
    }

    /// LIO-635: the computer keeps one record of what runs cut for all its
    /// projects, so a cut by another project's setup, or one deleted since,
    /// reaches every project it shares: a setup this project does not have.
    /// The Run panel and the tablet file refuse every start over it, as
    /// before, and say so: the cut may be another project's, and New stock
    /// loaded on the computer clears it, then a new sync. A cut by a setup
    /// of the project the plan no longer follows says it may be one too.
    #[test]
    fn another_projects_cut_is_said_as_one_with_the_way_out() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("template.toml");
        std::fs::write(&profile, uc100::config::TEMPLATE).unwrap();
        let (mut doc, _) = tablet_fixture(&profile);
        let (path, mesh, [mill, small]) = ring(dir.path());
        let mut first = ring_setup(&doc, "G54", "1", "1");
        let top = ring_top(&mesh, &first, true);
        first.operations = vec![ring_bore(&mill, top, "0.5")];
        let mut flipped = ring_setup(&doc, "G55", "1", "1");
        flipped.p.insert("rx".into(), "180".into());
        let top = ring_top(&mesh, &flipped, false);
        flipped.operations = vec![ring_face(&mill, top.clone()), ring_face(&small, top)];
        doc.active = flipped.id.clone();
        doc.setups = vec![first, flipped];
        doc.tools = vec![mill, small];
        if let Some(f) = &mut doc.file {
            (f.path, f.name, f.format) = (path, "ring.stl".into(), "stl".into());
        }
        // Another project's setup bored its part: an id this one lacks.
        let (c, l) = check(&doc, &mesh, 0);
        let mut run = Run::new(&doc, 0, 0);
        run.checked_as(Arc::new(c), Some(Arc::new(l)), None);
        run.note_cut(0, None);
        let elsewhere = Session {
            cuts: vec![(crate::id(), run.cut_in_run.clone())],
            ..Session::default()
        };
        let zeros = |s: &TabletSetup| -> Vec<Option<TabletZero>> {
            (s.tool_runs.iter())
                .flat_map(|r| {
                    let starts = r.starts.iter().map(|t| t.zero.clone());
                    let steps = r.step_starts.iter().map(|t| t.zero.clone());
                    std::iter::once(r.zero.clone()).chain(starts).chain(steps)
                })
                .collect()
        };
        let runs = crate::bundle::runs(&doc, Ok(&mesh), &elsewhere);
        assert_eq!(runs.setups.len(), 2);
        for s in &runs.setups {
            assert!(s.problems.is_empty(), "{:?}", s.problems);
            let all = zeros(s);
            assert!(all.iter().all(Option::is_some), "{all:?}");
            for z in all.into_iter().flatten() {
                assert_eq!(z.set_z, None);
                let blocked = z.blocked.unwrap();
                for said in [
                    "another project",
                    "New stock loaded on the computer",
                    "Sync projects",
                ] {
                    assert!(blocked.contains(said), "{said:?} not in {blocked:?}");
                }
                assert!(!z.probe_from.is_empty());
                for spot in &z.probe_from {
                    let why = spot.problem.as_deref().unwrap();
                    assert!(why.contains("another project"), "{why}");
                    assert!(why.contains("New stock loaded on the computer"), "{why}");
                }
            }
        }
        // The Run panel says it in its own words: New stock loaded is there.
        for si in 0..2 {
            let mut run = Run::new(&doc, si, 0);
            run.given(&elsewhere, &doc);
            let (c, l) = check(&doc, &mesh, si);
            run.checked_as(Arc::new(c), Some(Arc::new(l)), None);
            assert!(run.on_cut_stock() && run.touch_off().is_none());
            let why = run.partway_blocked().unwrap();
            assert!(why.contains("another project"), "{why}");
            assert!(
                why.contains("press New stock loaded under Work zero"),
                "{why}"
            );
            run.earlier_cuts.clear();
            assert!(!run.on_cut_stock());
        }
        // The plan's own setup 1's bore, disabled since, cut before setup 2:
        // said as setup 1's, its operation disabled, changed or deleted, not
        // as maybe another project's (LIO-599), with the same way out on the
        // tablet.
        let mut own = elsewhere.clone();
        own.cuts[0].0 = doc.setups[0].id.clone();
        let mut off = doc.clone();
        off.setups[0].operations[0].enabled = false;
        let runs = crate::bundle::runs(&off, Ok(&mesh), &own);
        let two = (runs.setups.iter()).find(|s| s.setup_id == doc.setups[1].id);
        for z in zeros(two.unwrap()).into_iter().flatten() {
            let blocked = z.blocked.unwrap();
            assert!(blocked.starts_with("Setup 1's run"), "{blocked}");
            assert!(blocked.contains("operation disabled"), "{blocked}");
            assert!(!blocked.contains("another project"), "{blocked}");
            assert!(
                blocked.contains("New stock loaded on the computer"),
                "{blocked}"
            );
        }
    }

    /// Setup 1 faces the ring's stock top 2 mm down with the Ø10 mill, but
    /// for 1 mm the Ø2 one then faces off; setup 2, held the same way,
    /// bores it with each, its work zero on the stock top setup 1 leaves
    /// (LIO-603). Its check takes setup 1's operations as cut. Once setup
    /// 1's runs cut some of them this session, that holds only for those
    /// they cut or took as cut: one whose material a run left
    /// (`Skip::Left`), or that none ran, refuses setup 2's Zero Z, Probe Z
    /// and probe spots, named.
    #[test]
    fn an_earlier_setups_operation_left_or_never_run_blocks_a_later_setups_touch_off() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("template.toml");
        std::fs::write(&profile, uc100::config::TEMPLATE).unwrap();
        let (mut doc, _) = tablet_fixture(&profile);
        let (path, mesh, [mill, small]) = ring(dir.path());
        let mut first = ring_setup(&doc, "G54", "2", "0");
        first.p.insert("name".into(), "Top".into());
        let top = ring_top(&mesh, &first, true);
        let mut face = ring_face(&mill, top.clone());
        face.p.insert("name".into(), "Face".into());
        face.p.insert("bottom".into(), "1".into());
        let mut finish = ring_face(&small, top);
        finish.p.insert("name".into(), "Finish face".into());
        first.operations = vec![face, finish];
        let mut second = ring_setup(&doc, "G55", "2", "0");
        let top = ring_top(&mesh, &second, true);
        second.operations = vec![
            ring_bore(&mill, top.clone(), "1"),
            ring_bore(&small, top, "1"),
        ];
        doc.active = second.id.clone();
        doc.setups = vec![first, second];
        doc.tools = vec![mill, small];
        if let Some(f) = &mut doc.file {
            (f.path, f.name, f.format) = (path, "ring.stl".into(), "stl".into());
        }
        // Setup 1's run, skipping as `skips` says, checked so.
        let one = |skips: &[(usize, Skip)]| {
            let mut one = Run::new(&doc, 0, 0);
            let (c, l) = check(&doc, &mesh, 0);
            one.checked_as(Arc::new(c), Some(Arc::new(l)), None);
            for &(oi, how) in skips {
                assert!(one.skip(oi, Some(how)), "{oi} {how:?}");
            }
            let (c, l) = check_steps(&doc, &mesh, 0, &one.skips, &[]);
            one.checked_as(Arc::new(c), Some(Arc::new(l)), None);
            assert!(one.checked().is_some(), "{:?}", one.programs);
            one
        };
        // Its tool run `index` sent from operation `from` and ended: cut.
        let ran = |one: &mut Run, index: usize, from: usize| {
            let cut_ops = cut_prints(&doc, 0, &one.skips);
            one.running = Some(Running {
                index,
                from,
                seq: 0,
                offset: [0.; 6],
                before: Before {
                    xy: None,
                    zero: None,
                    last: None,
                    touched_for: None,
                },
                paths: vec![],
                tip: [0.; 3],
                start: [0.; 6],
                print: 0,
                cut_ops,
                on: None,
                marks: vec![],
            });
            one.refresh(
                Some(&uc100::controller::ProgramOutcome {
                    seq: 1,
                    ok: true,
                    message: "m".into(),
                }),
                None,
            );
            assert!(one.running.is_none());
        };
        // Setup 1's run dropped: what the session keeps of it
        // (`App::run_cuts`, `App::run_declared`).
        let drop = |one: &Run| {
            let (mut session, mut declared) = (vec![], vec![]);
            crate::keep_run_cuts(&mut session, one, false);
            crate::keep_declared(&mut declared, one);
            (session, declared)
        };
        // Setup 2's run once setup 1's was dropped.
        let setup_2 = |one: &Run| {
            let (session, declared) = drop(one);
            let mut run = Run::new(&doc, 1, 0);
            crate::give_earlier_cuts(&session, &declared, &mut run);
            let (c, l) = check(&doc, &mesh, 1);
            run.checked_as(Arc::new(c), Some(Arc::new(l)), None);
            assert!(run.checked().is_some(), "{:?}", run.programs);
            run
        };
        let stock_top = {
            let one = one(&[]);
            assert_eq!(one.checked().unwrap().len(), 2);
            one.touch_off().unwrap().0[2]
        };
        let followed = |run: &Run| {
            assert!(run.earlier_uncut().is_empty() && !run.earlier_unfollowed());
            let (at, _) = run.touch_off().unwrap();
            assert!(
                (at[2] - (stock_top - 2.)).abs() < 1e-3,
                "{at:?} {stock_top}"
            );
            assert!(run.partway_blocked().is_none());
        };
        let refused_also = |mut run: Run, uncut: &[&str], also: bool| {
            // The check alone takes them as cut (LIO-582): the faced top.
            assert_eq!(run.unfollowed_earlier().next().is_some(), also);
            assert_eq!(run.earlier_uncut(), uncut);
            assert!(run.earlier_unfollowed());
            for (next, from) in [(0, 0), (1, 0)] {
                run.start_at(next, from);
                assert!(run.on_cut_stock());
                assert!(run.touch_off().is_none(), "{:?}", run.touch_off());
                let why = run.partway_blocked().unwrap();
                let list = uncut.join(", ");
                assert!(why.starts_with(&format!("{list} ")), "{why}");
                assert!(
                    why.contains("New stock loaded") && why.contains("disable"),
                    "{why}"
                );
                // Only what clears it is offered (LIO-603 review).
                assert_eq!(why.contains("Already cut"), !also, "{why}");
                assert_eq!(
                    why.contains("Cut it first") || why.contains("Cut them first"),
                    !also,
                    "{why}"
                );
                let partly = run.partly().unwrap();
                assert_eq!(partly.cut, [EARLIER_UNFOLLOWED]);
                assert!(partly.surface.is_none() && partly.top_cut);
                assert!(!partly.probe_from.is_empty());
                for (_, ok) in &partly.probe_from {
                    let why = ok.as_ref().unwrap_err();
                    assert!(why.starts_with(&format!("{list} ")), "{why}");
                }
                assert!(run.earlier_unfollowed_probe().starts_with(&list));
                assert!(run.earlier_unfollowed_note().starts_with(&list));
                let other = why.contains("material still there, which it does not follow either");
                assert_eq!(other, also, "{why}");
            }
        };
        let refused = |run: Run, uncut: &[&str]| refused_also(run, uncut, false);
        // Nothing cut this session: setup 1 is taken as cut whole (LIO-582).
        followed(&setup_2(&one(&[])));
        // Both tool runs cut.
        let mut all = one(&[]);
        ran(&mut all, 0, 0);
        ran(&mut all, 1, 0);
        followed(&setup_2(&all));
        // The finish's material left, the face cut (the issue's example):
        // the stock top is 1 mm higher than setup 2's check has it.
        let mut left = one(&[(1, Skip::Left)]);
        assert_eq!(left.checked().unwrap().len(), 1);
        ran(&mut left, 0, 0);
        assert_eq!(
            left.cuts_so_far().iter().map(|c| c.0).collect::<Vec<_>>(),
            [0]
        );
        refused(setup_2(&left), &["Finish face (Top)"]);
        // The face's left, the finish cut from the stock top: the finish
        // cut otherwise than setup 2's check has it, after the face, which
        // is said after the face, named (LIO-603 review).
        let mut face_left = one(&[(0, Skip::Left)]);
        assert_eq!(face_left.checked().unwrap().len(), 1);
        ran(&mut face_left, 0, 0);
        refused_also(setup_2(&face_left), &["Face (Top)"], true);
        // The face cut, then its plan changed (another stepover), the
        // finish never run: the face's cut is not followed, but not for
        // the finish's material, so that is not said, nor disabling the
        // finish as the way out (LIO-603 review).
        let mut changed = doc.clone();
        changed.setups[0].operations[0]
            .p
            .insert("stepover".into(), "20".into());
        let mut session = vec![];
        let mut face_cut = one(&[]);
        ran(&mut face_cut, 0, 0);
        crate::keep_run_cuts(&mut session, &face_cut, false);
        let mut run = Run::new(&changed, 1, 0);
        crate::give_earlier_cuts(&session, &[], &mut run);
        let (c, l) = check(&changed, &mesh, 1);
        run.checked_as(Arc::new(c), Some(Arc::new(l)), None);
        assert_eq!(run.earlier_uncut(), ["Finish face (Top)"]);
        assert!(run.touch_off().is_none());
        let why = run.partway_blocked().unwrap();
        // Said by its case: setup 1's face changed since (LIO-599).
        assert_eq!(why, EarlierUnfollowed::Changed(0).blocked());
        assert!(
            !why.contains("disable it in") && !why.contains("material still there"),
            "{why}"
        );
        // Following that: the face disabled, setup 2 touches off on the
        // stock top the finish faced 2 mm down, as the tool finds it.
        let mut off = doc.clone();
        off.setups[0].operations[0].enabled = false;
        let mut session = vec![];
        crate::keep_run_cuts(&mut session, &face_left, false);
        let mut run = Run::new(&off, 1, 0);
        crate::give_earlier_cuts(&session, &[], &mut run);
        let (c, l) = check(&off, &mesh, 1);
        run.checked_as(Arc::new(c), Some(Arc::new(l)), None);
        assert!(!run.earlier_unfollowed(), "{:?}", run.partway_blocked());
        let (at, _) = run.touch_off().unwrap();
        assert!(
            (at[2] - (stock_top - 2.)).abs() < 1e-3,
            "{at:?} {stock_top}"
        );
        assert!(run.partway_blocked().is_none());
        // The face cut, the finish never run: the same.
        let mut stopped = one(&[]);
        ran(&mut stopped, 0, 0);
        refused(setup_2(&stopped), &["Finish face (Top)"]);
        // Cycle Start waits for it, its programs made for it cut, the
        // number said as it is (LIO-602); so it does with another
        // project's cuts on the record too, which the touch-off says
        // instead (LIO-602 review).
        let run = setup_2(&stopped);
        let why = run.earlier_not_whole(&doc).unwrap();
        assert!(
            why.starts_with("Finish face (Top) was not cut")
                && why.contains("made for it cut, could send rapids through"),
            "{why}"
        );
        let mut foreign = setup_2(&stopped);
        foreign.earlier_cuts.push((
            "another project's setup".into(),
            vec![(0, None, Some(1), Some(1))],
        ));
        assert_eq!(foreign.earlier_uncut(), ["Finish face (Top)"]);
        assert!(foreign.earlier_uncut_said().is_none());
        let why = foreign.earlier_not_whole(&doc).unwrap();
        assert!(
            why.starts_with("Finish face (Top) was not cut")
                && why.contains("made for it cut: their rapids could go through"),
            "{why}"
        );
        // What setup 1's runs cut lost (`note_lost_cuts`, LIO-594), alone
        // or beside the face's cut (another session's journal): cut by runs
        // no session here knows of, so nothing is called not cut, and Z is
        // still refused, as runs not followed (LIO-596, LIO-603 review).
        let (cuts, _) = drop(&stopped);
        let face = cuts[0].1.clone();
        let unknown = (HISTORY_UNKNOWN, None, None, None);
        for lost in [vec![unknown], [face, vec![unknown]].concat()] {
            let session = vec![(doc.setups[0].id.clone(), lost)];
            let mut run = Run::new(&doc, 1, 0);
            crate::give_earlier_cuts(&session, &[], &mut run);
            let (c, l) = check(&doc, &mesh, 1);
            run.checked_as(Arc::new(c), Some(Arc::new(l)), None);
            assert!(run.earlier_uncut().is_empty() && run.earlier_unfollowed());
            for (next, from) in [(0, 0), (1, 0)] {
                run.start_at(next, from);
                assert!(run.touch_off().is_none());
                // Said as runs not followed (LIO-596), every setup's.
                let why = run.partway_blocked().unwrap();
                assert_eq!(why, HISTORY_UNFOLLOWED_BLOCKED);
                assert!(!why.contains("not cut"), "{why}");
                assert!(!run.earlier_unfollowed_note().contains("not cut"));
                assert!(!run.earlier_unfollowed_probe().contains("not cut"));
            }
        }
        // The tablet file too (`tablet_runs`, LIO-590): its programs, made
        // for the finish cut, could rapid through its material, so setup 2
        // has no tool runs there, the finish named (LIO-602); a full run's
        // start sets Z.
        let tablet = |one: &Run| {
            let (cuts, declared) = drop(one);
            let session = Session {
                cuts,
                lost: false,
                declared,
                owners: vec![],
            };
            tablet_runs(&doc, Ok(&mesh), 1, &session)
        };
        let tablet_zero = |one: &Run| tablet(one).tool_runs[0].zero.clone();
        let tablet_refused = |one: &Run| {
            let s = tablet(one);
            assert!(s.tool_runs.is_empty(), "{:?}", s.problems);
            assert_eq!(s.problems.len(), 1);
            let why = s.problems[0].clone();
            assert!(why.contains("could send rapids through"), "{why}");
            why
        };
        let blocked = tablet_refused(&stopped);
        assert!(
            blocked.starts_with("Finish face (Top) was not cut"),
            "{blocked}"
        );
        // In the tablet's words: the ways out are the computer's, then a
        // new sync; nothing only the computer has (LIO-603 review).
        let for_the_tablet = |blocked: &str| {
            assert!(
                blocked.contains("on the computer") || blocked.contains("On the computer"),
                "{blocked}"
            );
            assert!(blocked.contains("Sync projects here"), "{blocked}");
            let said = blocked.replace(new_stock_on_the_computer!(), "");
            for only in ["Same part", "New stock", "under Work zero", "this computer"] {
                assert!(!said.contains(only), "{blocked}");
            }
        };
        for_the_tablet(&blocked);
        assert!(blocked.contains("Already cut"), "{blocked}");
        // The face left: only disabling it, on the computer.
        let blocked = tablet_refused(&face_left);
        assert!(blocked.starts_with("Face (Top) was not cut"), "{blocked}");
        for_the_tablet(&blocked);
        assert!(
            !blocked.contains("Already cut") && blocked.contains("disable it"),
            "{blocked}"
        );
        let full = tablet_zero(&all);
        assert!(
            full.as_ref()
                .is_none_or(|z| z.blocked.is_none() && z.set_z.is_some()),
            "{full:?}"
        );
        // The finish marked Already cut after the face ran: taken as cut.
        let mut marked = one(&[]);
        ran(&mut marked, 0, 0);
        assert!(marked.skip(1, Some(Skip::Cut)));
        followed(&setup_2(&marked));
        // Unmarked again before it was dropped: not.
        assert!(marked.skip(1, None));
        refused(setup_2(&marked), &["Finish face (Top)"]);
        // Marked before the face ran, then unmarked: not either (LIO-603
        // review). Setup 1 itself then starts at the finish on the faced
        // stock, which its run cut.
        let marked_ran = || {
            let mut run = one(&[(1, Skip::Cut)]);
            assert_eq!(run.checked().unwrap().len(), 1);
            ran(&mut run, 0, 0);
            run
        };
        let mut early = marked_ran();
        assert_eq!(
            early.cut_in_run.iter().map(|c| c.0).collect::<Vec<_>>(),
            [0]
        );
        followed(&setup_2(&early));
        // Unmarked, or lost to a project edit, in another AXIS CAM on the
        // same folder, with setup 2's run live here: the file held the mark
        // when this one read it (`base`), and holds it no more, or lost
        // (`theirs`). Taken in as `App::sync_cut_history` does
        // (`take_in_declared`, then `retake_marks`), the finish is not cut
        // again, on the Run panel and in the tablet file, and setup 2's
        // work zero is set again (LIO-603 review).
        let (cuts, declared) = drop(&early);
        let lost = {
            let mut lost = declared.clone();
            crate::lose_declared(&mut lost);
            lost
        };
        for theirs in [vec![], lost] {
            let mut run = setup_2(&early);
            assert!(run.earlier_uncut().is_empty());
            let mut mine = declared.clone();
            crate::take_in_declared(&declared, &theirs, &mut mine, Some(&mut run));
            assert!(
                mine.iter().all(|k| k.skips.is_empty() && k.lost),
                "{mine:?}"
            );
            assert!(crate::retake_marks(&mine, &mut run));
            refused(run, &["Finish face (Top)"]);
            let session = Session {
                cuts: cuts.clone(),
                lost: false,
                declared: mine,
                owners: vec![],
            };
            // The tablet file has no tool runs for it then (LIO-602).
            let s = tablet_runs(&doc, Ok(&mesh), 1, &session);
            assert!(s.tool_runs.is_empty());
            let blocked = s.problems.concat();
            assert!(
                blocked.starts_with("Finish face (Top) was not cut"),
                "{blocked}"
            );
        }
        // Unchanged there: kept, and setup 2 still follows.
        let mut run = setup_2(&early);
        let mut mine = declared.clone();
        assert!(!crate::take_in_declared(
            &declared,
            &declared,
            &mut mine,
            Some(&mut run)
        ));
        assert_eq!(mine, declared);
        assert!(!crate::retake_marks(&mine, &mut run));
        assert!(run.earlier_uncut().is_empty());
        // The tablet file goes by the mark too.
        let marked_zero = tablet_zero(&early);
        assert!(
            marked_zero
                .as_ref()
                .is_none_or(|z| z.blocked.is_none() && z.set_z.is_some()),
            "{marked_zero:?}"
        );
        let unmark = |run: &mut Run| {
            assert!(run.skip(1, None));
            let (c, l) = check_steps(&doc, &mesh, 0, &run.skips, &[]);
            run.checked_as(Arc::new(c), Some(Arc::new(l)), None);
        };
        let late = marked_ran();
        unmark(&mut early);
        refused(setup_2(&early), &["Finish face (Top)"]);
        let finish_start = |run: &mut Run| {
            run.start_at(1, 0);
            assert_eq!(run.checked().unwrap()[1].indexes, [1]);
            let (at, _) = run.touch_off().unwrap();
            assert!(
                (at[2] - (stock_top - 1.)).abs() < 1e-3,
                "{at:?} {stock_top}"
            );
            assert!(run.partway_blocked().is_none());
        };
        finish_start(&mut early);
        // The same with setup 2 picked in between: setup 1 picked again
        // gets its cuts and its mark back (`App::picked`), unmarked then.
        let (session, declared) = drop(&late);
        followed(&setup_2(&late));
        let mut again = Run::new(&doc, 0, 0);
        again.cut_in_run = session[0].1.clone();
        again.skips = declared[0].skips.clone();
        let (c, l) = check_steps(&doc, &mesh, 0, &again.skips, &[]);
        again.checked_as(Arc::new(c), Some(Arc::new(l)), None);
        assert_eq!(
            again.cuts_so_far().iter().map(|c| c.0).collect::<Vec<_>>(),
            [0]
        );
        unmark(&mut again);
        refused(setup_2(&again), &["Finish face (Top)"]);
        finish_start(&mut again);
        // A start at the finish's tool run takes the face as cut, and
        // keeps it so once the run is dropped: ahead of the finish, in the
        // setup's order, not as cut after it (LIO-599 review).
        let mut started = one(&[]);
        started.start_at(1, 0);
        ran(&mut started, 1, 0);
        let kept = started.cuts_so_far();
        assert_eq!(
            kept.iter().map(|c| (c.0, c.1)).collect::<Vec<_>>(),
            [(0, None), (1, None)]
        );
        followed(&setup_2(&started));
        // So does the face skipped as already cut, while it is: read as
        // marked now, not kept with what was cut.
        let mut skipped = one(&[(0, Skip::Cut)]);
        assert_eq!(skipped.checked().unwrap().len(), 1);
        ran(&mut skipped, 0, 0);
        assert!(skipped.cut_in_run.iter().all(|c| c.0 != 0));
        followed(&setup_2(&skipped));
        // Lost to a project edit since (`lose_declared`): not.
        let (session, mut declared) = drop(&skipped);
        crate::lose_declared(&mut declared);
        let mut run = Run::new(&doc, 1, 0);
        crate::give_earlier_cuts(&session, &declared, &mut run);
        let (c, l) = check(&doc, &mesh, 1);
        run.checked_as(Arc::new(c), Some(Arc::new(l)), None);
        refused(run, &["Face (Top)"]);
        // Setup 2 picked, its run live, and the project edited (LIO-603
        // review): the marks it was given name operations by indexes the
        // edit may change, and plans it may change, so they no longer count
        // (`Run::unskip`, as `App::run_follows_project` does).
        let edited_live = |doc: &Document, edit: &dyn Fn(&mut Document)| {
            let mut one = Run::new(doc, 0, 0);
            let (c, l) = check(doc, &mesh, 0);
            one.checked_as(Arc::new(c), Some(Arc::new(l)), None);
            assert!(one.skip(1, Some(Skip::Cut)));
            let (c, l) = check_steps(doc, &mesh, 0, &one.skips, &[]);
            one.checked_as(Arc::new(c), Some(Arc::new(l)), None);
            ran(&mut one, 0, 0);
            let (mut session, mut declared) = (vec![], vec![]);
            crate::keep_run_cuts(&mut session, &one, false);
            crate::keep_declared(&mut declared, &one);
            let mut run = Run::new(doc, 1, 0);
            crate::give_earlier_cuts(&session, &declared, &mut run);
            let (c, l) = check(doc, &mesh, 1);
            run.checked_as(Arc::new(c), Some(Arc::new(l)), None);
            let before = run.earlier_uncut().len();
            let mut doc = doc.clone();
            edit(&mut doc);
            run.unskip();
            let (c, l) = check(&doc, &mesh, 1);
            run.checked_as(Arc::new(c), Some(Arc::new(l)), None);
            assert!(run.checked().is_some(), "{:?}", run.programs);
            (before, run)
        };
        // Setup 1 with a middle face, marked Already cut, and deleted once
        // setup 2 is picked: the finish moves to its index.
        let mut three = doc.clone();
        let mut middle = three.setups[0].operations[1].clone();
        middle.id = crate::model::id();
        middle.p.insert("name".into(), "Middle face".into());
        middle.p.insert("bottom".into(), "0.5".into());
        three.setups[0].operations.insert(1, middle);
        let (before, run) = edited_live(&three, &|d: &mut Document| {
            d.setups[0].operations.remove(1);
        });
        assert_eq!(before, 1);
        refused(run, &["Finish face (Top)"]);
        // The finish marked Already cut, and its plan changed once setup 2
        // is picked: no longer taken as cut.
        let (before, run) = edited_live(&doc, &|d: &mut Document| {
            d.setups[0].operations[1]
                .p
                .insert("stepover".into(), "20".into());
        });
        assert_eq!(before, 0);
        refused(run, &["Finish face (Top)"]);
        // The finish disabled since: the check leaves its material too,
        // and follows.
        let mut off = doc.clone();
        off.setups[0].operations[1].enabled = false;
        let mut run = Run::new(&off, 1, 0);
        let mut session = vec![];
        crate::keep_run_cuts(&mut session, &stopped, false);
        crate::give_earlier_cuts(&session, &[], &mut run);
        let (c, l) = check(&off, &mesh, 1);
        run.checked_as(Arc::new(c), Some(Arc::new(l)), None);
        assert!(run.earlier_uncut().is_empty() && !run.earlier_unfollowed());
        assert!(run.touch_off().is_some() && run.partway_blocked().is_none());
        // Nothing edited, but the finish cut with the face left, then the
        // face: said as cut out of the plan's order, not as a change
        // (LIO-599 review).
        let mut swapped = one(&[(0, Skip::Left)]);
        ran(&mut swapped, 0, 0);
        assert!(swapped.skip(0, None));
        let (c, l) = check_steps(&doc, &mesh, 0, &swapped.skips, &[]);
        swapped.checked_as(Arc::new(c), Some(Arc::new(l)), None);
        swapped.start_at(0, 0);
        ran(&mut swapped, 0, 0);
        let run = setup_2(&swapped);
        assert!(run.earlier_uncut().is_empty());
        assert_eq!(
            run.earlier_unfollowed_how(),
            Some(EarlierUnfollowed::OutOfOrder(0))
        );
        assert_eq!(
            run.partway_blocked().as_deref(),
            Some(
                "Setup 1's run cut its operations out of the plan's order, so the check cannot tell the surface where the stock top is touched off. If new stock is held, press New stock loaded under Work zero."
            )
        );
        // Setup 1 started at its Finish, which takes the Face as cut: not
        // out of the plan's order. Followed as it is; with the Finish
        // changed or disabled since, said as that (LIO-599 review).
        let mut at_finish = one(&[]);
        at_finish.start_at(1, 0);
        ran(&mut at_finish, 1, 0);
        let mut session = vec![];
        crate::keep_run_cuts(&mut session, &at_finish, false);
        let ops: Vec<usize> = session[0].1.iter().map(|c| c.0).collect();
        assert_eq!(ops, [0, 1], "taken as cut ahead of what was cut");
        let mut stepover = doc.clone();
        stepover.setups[0].operations[1]
            .p
            .insert("stepover".into(), "20".into());
        let mut finish_off = doc.clone();
        finish_off.setups[0].operations[1].enabled = false;
        for (edited, how) in [
            (&doc, None),
            (&stepover, Some(EarlierUnfollowed::Changed(0))),
            (&finish_off, Some(EarlierUnfollowed::Changed(0))),
        ] {
            let mut run = Run::new(edited, 1, 0);
            crate::give_earlier_cuts(&session, &[], &mut run);
            let (c, l) = check(edited, &mesh, 1);
            run.checked_as(Arc::new(c), Some(Arc::new(l)), None);
            assert_eq!(run.earlier_unfollowed_how(), how);
            if let Some(how) = how {
                assert_eq!(run.partway_blocked(), Some(how.blocked()));
            }
        }
        // A third setup after setup 2, holding the ring the same way.
        let mut doc3 = doc.clone();
        let mut third = ring_setup(&doc, "G56", "2", "0");
        third.p.insert("name".into(), "Third".into());
        third.p.insert("origin".into(), "Model origin".into());
        let top3 = ring_top(&mesh, &third, true);
        third.operations = vec![ring_bore(&doc.tools[0], top3, "1")];
        doc3.setups.push(third);
        type Cuts = Vec<(String, Vec<CutInRun>)>;
        let run3 = |si: usize, session: &Cuts, skips: &[(usize, Skip)]| {
            let mut r = Run::new(&doc3, si, 0);
            crate::give_earlier_cuts(session, &[], &mut r);
            let (c, l) = check(&doc3, &mesh, si);
            r.checked_as(Arc::new(c), Some(Arc::new(l)), None);
            for &(oi, how) in skips {
                assert!(r.skip(oi, Some(how)));
            }
            let (c, l) = check_steps(&doc3, &mesh, si, &r.skips, &[]);
            r.checked_as(Arc::new(c), Some(Arc::new(l)), None);
            assert!(r.checked().is_some(), "{:?}", r.programs);
            r
        };
        // Setup 1's face left and its finish cut, then setup 3 cut;
        // setup 2 picked, nothing edited. Setup 1's cut is the face's to
        // explain, not a change: setup 3 cut first is said, whichever order
        // the cuts are kept in (LIO-599 review).
        let mut kept: Cuts = vec![];
        let mut a = run3(0, &kept, &[(0, Skip::Left)]);
        ran(&mut a, 0, 0);
        crate::keep_run_cuts(&mut kept, &a, false);
        let mut c = run3(2, &kept, &[]);
        ran(&mut c, 0, 0);
        crate::keep_run_cuts(&mut kept, &c, false);
        let mut reversed = kept.clone();
        reversed.reverse();
        for cuts in [&kept, &reversed] {
            let run = run3(1, cuts, &[]);
            assert_eq!(run.earlier_uncut(), ["Face (Top)"]);
            assert_eq!(
                run.earlier_unfollowed_how(),
                Some(EarlierUnfollowed::After(2))
            );
            let why = run.partway_blocked().unwrap();
            assert!(why.starts_with("Setup 3 comes after this one"), "{why}");
        }
        // Setups 3 then 2 cut, setup 1 picked: the first of them in the
        // plan is named, whichever was cut first.
        let mut kept: Cuts = vec![];
        let mut c = run3(2, &kept, &[]);
        ran(&mut c, 0, 0);
        crate::keep_run_cuts(&mut kept, &c, false);
        let mut b = run3(1, &kept, &[]);
        ran(&mut b, 0, 0);
        ran(&mut b, 1, 0);
        crate::keep_run_cuts(&mut kept, &b, false);
        assert_eq!(
            run3(0, &kept, &[]).earlier_unfollowed_how(),
            Some(EarlierUnfollowed::After(1))
        );
    }

    /// Setup 1 faces 2 mm off the ring's stock top in four levels; setup 2,
    /// held the same way, bores it, its work zero on the stock top that
    /// setup 1 leaves (LIO-601). Faced whole this session, setup 2 touches
    /// off on the faced top. Stopped after its first level, the top is
    /// still 1.5 mm higher than that: setup 2's check, which simulates the
    /// face whole, cannot tell it, so neither Zero Z nor Probe Z sets Z,
    /// from setup 2's start or partway, its probe spots are refused, and
    /// the tablet file's zeros too; so where the face may have stopped
    /// anywhere. Said to finish the face only when that is all there is.
    #[test]
    fn an_earlier_setups_cut_stopped_partway_blocks_a_later_setups_touch_off() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("template.toml");
        std::fs::write(&profile, uc100::config::TEMPLATE).unwrap();
        let (mut doc, _) = tablet_fixture(&profile);
        let (path, mesh, [mill, small]) = ring(dir.path());
        let mut first = ring_setup(&doc, "G54", "2", "0");
        let mut face = ring_face(&mill, ring_top(&mesh, &first, true));
        face.p.insert("stepdown".into(), "0.5".into());
        first.operations = vec![face];
        let mut second = ring_setup(&doc, "G55", "2", "0");
        let top = ring_top(&mesh, &second, true);
        second.operations = vec![
            ring_bore(&mill, top.clone(), "1"),
            ring_bore(&small, top, "1"),
        ];
        doc.active = second.id.clone();
        doc.setups = vec![first, second];
        doc.tools = vec![mill, small];
        if let Some(f) = &mut doc.file {
            (f.path, f.name, f.format) = (path, "ring.stl".into(), "stl".into());
        }
        // Setup 1's run: its face's levels are its steps.
        let (c, l) = check(&doc, &mesh, 0);
        let mut one = Run::new(&doc, 0, 0);
        one.checked_as(Arc::new(c), Some(Arc::new(l)), None);
        let levels = one.op_steps(0).map_or(0, |s| s.steps.len());
        assert_eq!(levels, 4);
        let stock_top = one.touch_off().unwrap().0[2];
        // Setup 2 after setup 1's run cut its face (`cut`), from its start
        // or at its second bore.
        let session = |cut: Option<usize>| {
            let mut one = Run::new(&doc, 0, 0);
            let (c, l) = check(&doc, &mesh, 0);
            one.checked_as(Arc::new(c), Some(Arc::new(l)), None);
            one.note_cut(0, cut);
            vec![(doc.setups[0].id.clone(), one.cut_in_run.clone())]
        };
        let setup_2 = |cut: Option<usize>, partway: bool| {
            let session = session(cut);
            let mut run = Run::new(&doc, 1, 0);
            crate::give_earlier_cuts(&session, &[], &mut run);
            let (c, l) = check(&doc, &mesh, 1);
            run.checked_as(Arc::new(c), Some(Arc::new(l)), None);
            assert!(run.checked().is_some(), "{:?}", run.programs);
            if partway {
                let programs = run.checked().unwrap();
                let at = (programs.iter().enumerate())
                    .find_map(|(i, p)| Some((i, p.indexes.iter().position(|&o| o == 1)?)));
                let (i, k) = at.unwrap();
                run.start_at(i, k);
                assert!(run.partway());
            }
            run
        };
        // Faced whole: setup 2's stock top is the faced one, 2 mm down.
        let whole = setup_2(None, false);
        assert!(!whole.earlier_unfollowed() && !whole.on_cut_stock());
        let (at, _) = whole.touch_off().unwrap();
        assert!(
            (at[2] - (stock_top - 2.)).abs() < 1e-3,
            "{at:?} {stock_top}"
        );
        assert!(whole.partway_blocked().is_none());
        // Stopped after its first level, then after its third; stopped in
        // its last, the tool may not have got round it (`Run::refresh`);
        // stopped where the run cannot tell, or dropped while it ran
        // (`Run::cuts_so_far`), any of it may be cut.
        for cut in [Some(1), Some(3), Some(levels), MAY_HAVE_CUT_ALL] {
            for partway in [false, true] {
                let run = setup_2(cut, partway);
                assert!(run.earlier_unfollowed(), "{cut:?}");
                assert!(run.on_cut_stock(), "{cut:?}");
                assert!(run.touch_off().is_none(), "{cut:?} {:?}", run.touch_off());
                let why = run.partway_blocked().unwrap();
                assert!(
                    why.starts_with(
                        "Another setup's run left an operation of this part unfinished"
                    ),
                    "{why}"
                );
                let partly = run.partly().unwrap();
                assert_eq!(partly.cut, [EARLIER_UNFOLLOWED]);
                assert!(partly.surface.is_none() && partly.top_cut);
                assert!(!partly.probe_from.is_empty());
                for (_, ok) in &partly.probe_from {
                    assert_eq!(ok.as_ref().unwrap_err(), EARLIER_PARTWAY_WHY);
                }
            }
            // The tablet file made with this session's cuts (LIO-590)
            // has no tool runs for setup 2: made for the face cut whole,
            // their rapids could go through what it left, so it refuses
            // them as the Run panel's Cycle Start waits, in its own words
            // (LIO-602).
            let cuts = Session {
                cuts: session(cut),
                ..Session::default()
            };
            let runs = tablet_runs(&doc, Ok(&mesh), 1, &cuts);
            assert!(runs.tool_runs.is_empty());
            let why = runs.problems.concat();
            let how = match cut {
                MAY_HAVE_CUT_ALL => "stopped where it is not known",
                _ => "stopped partway through",
            };
            assert!(
                why.contains(how)
                    && why.contains("their rapids could go through")
                    && why.contains("On the computer,"),
                "{why}"
            );
        }
        // Its plan changed since as well: the reason is that, which
        // finishing the face does not clear.
        let mut changed = session(Some(1));
        changed[0].1[0].2 = changed[0].1[0].2.map(|p| p ^ 1);
        let mut run = Run::new(&doc, 1, 0);
        crate::give_earlier_cuts(&changed, &[], &mut run);
        let (c, l) = check(&doc, &mesh, 1);
        run.checked_as(Arc::new(c), Some(Arc::new(l)), None);
        assert!(run.earlier_unfollowed() && !run.earlier_partway());
        assert!(run.touch_off().is_none());
        // Said by its case (LIO-599).
        assert_eq!(
            run.partway_blocked(),
            Some(EarlierUnfollowed::Changed(0).blocked())
        );
        // Setup 2's run stopped partway, and setup 1 picked: the check of
        // setup 1 has nothing of setup 2, which is the reason.
        let mut run = Run::new(&doc, 1, 0);
        crate::give_earlier_cuts(&[], &[], &mut run);
        let (c, l) = check(&doc, &mesh, 1);
        run.checked_as(Arc::new(c), Some(Arc::new(l)), None);
        run.note_cut(0, Some(1));
        let later = vec![(doc.setups[1].id.clone(), run.cut_in_run.clone())];
        let mut run = Run::new(&doc, 0, 0);
        crate::give_earlier_cuts(&later, &[], &mut run);
        let (c, l) = check(&doc, &mesh, 0);
        run.checked_as(Arc::new(c), Some(Arc::new(l)), None);
        assert!(run.earlier_unfollowed() && !run.earlier_partway());
        assert!(run.touch_off().is_none());
        // Said as setup 2 cut first (LIO-599).
        assert_eq!(
            run.partway_blocked(),
            Some(EarlierUnfollowed::After(1).blocked())
        );
        // Faced whole, the tablet file's start touches off as the plan has it.
        let whole = Session {
            cuts: session(None),
            ..Session::default()
        };
        let runs = tablet_runs(&doc, Ok(&mesh), 1, &whole);
        assert_eq!(runs, tablet_runs(&doc, Ok(&mesh), 1, &Session::default()));
        // Cut whole later this session (resumed): followed again.
        let mut one = Run::new(&doc, 0, 0);
        let (c, l) = check(&doc, &mesh, 0);
        one.checked_as(Arc::new(c), Some(Arc::new(l)), None);
        one.note_cut(0, Some(1));
        one.note_cut(0, None);
        assert_eq!(one.cut_in_run[0].1, None);
    }

    /// What another AXIS CAM on the same folder kept (LIO-601 review): taken
    /// as it is where this one changed nothing since it last read it; where
    /// both changed, an operation one holds whole and the other in part may
    /// be either, so neither: up to all of it.
    #[test]
    fn another_axis_cams_cuts_never_make_a_partway_one_whole() {
        let base = vec![(0, None, Some(1), Some(1))];
        let theirs = [(0, Some(2), Some(1), None)];
        let mut unchanged = base.clone();
        take_kept_cuts(&mut unchanged, Some(&base), &theirs);
        assert_eq!(unchanged, theirs);
        // Neither is whole then, so no plan is known cut whole (LIO-602).
        let mut both = vec![(0, None, Some(1), Some(1)), (1, None, Some(2), Some(2))];
        take_kept_cuts(&mut both, Some(&base), &theirs);
        assert_eq!(
            both,
            [
                (0, MAY_HAVE_CUT_ALL, Some(1), None),
                (1, None, Some(2), Some(2))
            ]
        );
        let mut partway = vec![(0, Some(1), Some(1), None)];
        take_kept_cuts(&mut partway, None, &[(0, None, Some(1), Some(1))]);
        assert_eq!(partway, [(0, MAY_HAVE_CUT_ALL, Some(1), None)]);
        let mut deeper = vec![(0, Some(1), Some(1), None)];
        take_kept_cuts(&mut deeper, None, &theirs);
        assert_eq!(deeper, theirs);
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
                    cut_print: fingerprint,
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

    /// LIO-611: a cuts.json written before what was taken as cut was kept
    /// with it loads, with none taken as cut; one written now keeps it.
    #[test]
    fn an_older_cut_history_loads_with_nothing_taken_as_cut() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cuts.json");
        let older =
            r#"{"cuts":[["s1",[[0,null,7]]]],"journals":[],"others_unknown":false,"sent":[3]}"#;
        std::fs::write(&path, older).unwrap();
        let h = CutHistory::read(&path).unwrap().unwrap();
        assert_eq!(h.cuts, [("s1".to_string(), vec![(0, None, Some(7), None)])]);
        assert!(h.declared.is_empty());
        let kept = KeptDeclared {
            setup: "s1".into(),
            skips: vec![(2, Skip::Cut)],
            steps: vec![(1, vec![0, 3])],
            lost: false,
            prints: vec![(1, Some(11)), (2, None)],
        };
        let h = CutHistory {
            declared: vec![kept],
            ..h
        };
        h.write(&path).unwrap();
        assert_eq!(CutHistory::read(&path).unwrap(), Some(h));
    }
    mod chamfer_tests {
        use crate::chamfer::tests::*;

        #[test]
        fn a_chamfer_milling_posts_for_linuxcnc_and_runs_on_the_uc100() {
            let (mut doc, mesh) = crate::post::fixture::job();
            let mut spot = mill("90", "0");
            spot.p.insert("number".into(), "9".into());
            let op = chamfer(
                &spot,
                &[
                    ("chamfer_source", "Line"),
                    ("from_x", "-5"),
                    ("to_x", "5"),
                    ("line_z", "0"),
                    ("chamfer_size", "Tip depth"),
                    ("tip_z", "-1"),
                ],
            );
            doc.tools.push(spot);
            doc.setups[0].operations = vec![op];
            // LinuxCNC: down to the tip, then the level pass 1 mm off the line.
            let job = crate::post::post_job(&doc, &mesh);
            assert!(job.errors.is_empty(), "{:?}", job.errors);
            let programs = job.setups[0].result.as_ref().unwrap();
            let text = &programs[0].text;
            assert!(text.contains("G1 Z-1.000"), "{text}");
            assert!(
                text.contains("X-5.000 Y1.000\nZ5.000\nG1 Z-1.000"),
                "{text}"
            );
            assert!(text.contains("G1 Z-1.000 F75.0\nX5.000 F300.0"), "{text}");
            // The UC100: its run checks pass and the tool run has the pass.
            let dir = tempfile::tempdir().unwrap();
            let profile = dir.path().join("template.toml");
            std::fs::write(&profile, uc100::config::TEMPLATE).unwrap();
            // The template profile has no coolant outputs.
            doc.setups[0].operations[0]
                .p
                .insert("coolant".into(), "Off".into());
            let m = &mut doc.setups[0].machine;
            m.p.insert("controller".into(), "UC100".into());
            m.p.insert("uc100_profile".into(), profile.display().to_string());
            let job = crate::post::post_job(&doc, &mesh);
            assert!(
                matches!(job.setups[0].run, Some(Ok(()))),
                "{:?}",
                job.setups[0].run
            );
            let runs = crate::run::programs(&doc, &mesh, 0).unwrap();
            let level = (runs.iter().flat_map(|r| &r.blocks)).any(|b| {
                matches!(b, crate::post::Block::Move { kind: crate::post::Kind::Feed, from, to, .. }
                    if (from[2] - to[2]).abs() < 1e-6 && (to[0] - from[0]).abs() > 9.99)
            });
            assert!(level);
        }
    }
    mod envelope_tests {
        use crate::envelope::tests::*;
        use crate::envelope::*;
        use crate::mesh::Mesh;
        use crate::model::Document;
        use crate::model::HOLDER_LENGTH;
        use crate::model::Machine;
        use crate::simulation::Simulation;

        fn simulated(doc: &Document, mesh: &Mesh) -> Vec<(usize, Result<Simulation, String>)> {
            crate::run::simulated_for_tests(doc, mesh, 0)
        }
        #[test]
        fn a_machine_without_the_figures_is_not_checked_and_a_bad_one_is_said() {
            let (doc, mesh) = drilled(machine("", "", ""), Some("25"));
            assert!(problems(&doc, &mesh, 0, &simulated(&doc, &mesh)).is_empty());
            assert!(!Machine::new(3).p.contains_key("nose_to_table"));
            assert!(issues(&machine("150", "120", "390")).is_empty());
            assert!(issues(&machine("", "", "")).is_empty());
            assert_eq!(
                issues(&machine("0", "120", "390")),
                ["Spindle nose to table at Z top must be greater than zero."]
            );
            assert_eq!(
                issues(&machine("150", "abc", "390")),
                ["Gantry clearance height must be greater than zero."]
            );
            assert_eq!(
                issues(&machine("150", "120", "300")),
                [
                    "Gantry clearance width cannot be narrower than the 390 mm X travel the spindle crosses it over."
                ]
            );
            let mut m = machine("150", "120", "390");
            m.p.insert("nose_to_table_source".into(), "guess".into());
            assert_eq!(
                issues(&m),
                ["Spindle nose to table at Z top: choose ASSUMED or Measured."]
            );
        }
        #[test]
        fn the_stack_up_is_the_vise_the_stock_and_the_tool_up_to_the_nose() {
            // The fixture's stock is 98 mm along Y; the vise holds it by 8 mm.
            let (doc, mesh) = drilled(machine("1000", "1000", "1000"), Some("25"));
            let stack = Stack::of(&doc, &mesh, 0).unwrap();
            let height = doc.setup_stock(0, Some(&mesh)).unwrap().dimensions()[2];
            assert_eq!(stack.base, 25.);
            assert!(!stack.base_assumed);
            assert_eq!(stack.stock_top, 25. + height);
            assert_eq!(stack.top, stack.stock_top);
            // A frame Z is the stock's underside at the base, rising with it.
            let stock = doc.setup_stock(0, Some(&mesh)).unwrap();
            assert_eq!(stack.height(stock.min[2]), Some(25.));
            assert_eq!(stack.height(stock.max[2]), Some(25. + height));
            // Roomy: the job passes, and a library vise that gives no height
            // is taken as the typical one, saying so.
            assert!(problems(&doc, &mesh, 0, &simulated(&doc, &mesh)).is_empty());
            let (doc, mesh) = drilled(machine("1000", "1000", "1000"), None);
            let stack = Stack::of(&doc, &mesh, 0).unwrap();
            assert_eq!((stack.base, stack.base_assumed), (VISE_BASE, true));
            assert!(stack.what.contains("taken"), "{}", stack.what);
            // Clamps and nothing stand on the table.
            let mut none = doc.clone();
            none.setups[0].p.insert("holding".into(), "None".into());
            let stack = Stack::of(&none, &mesh, 0).unwrap();
            assert_eq!((stack.base, stack.base_assumed), (0., false));
            assert!(
                stack.what.starts_with("on the table, the stock's "),
                "{}",
                stack.what
            );
        }
        #[test]
        fn a_part_taller_than_the_gantry_is_refused_naming_the_numbers() {
            let (doc, mesh) = drilled(machine("1000", "50", "1000"), Some("25"));
            let stack = Stack::of(&doc, &mesh, 0).unwrap();
            let found = problems(&doc, &mesh, 0, &simulated(&doc, &mesh));
            assert_eq!(found.len(), 1, "{found:?}");
            assert_eq!(found[0].0, None);
            let top = crate::envelope::mm(stack.top);
            assert!(
                found[0].1.contains(&format!(
                    "the part doesn't fit under the gantry: needs {top} mm, has 50 mm."
                )),
                "{}",
                found[0].1
            );
            // The same part under a gantry with the room passes, and one marked
            // ASSUMED says so in the refusal.
            let tall = stack.top + 1.;
            let (roomy, _) = drilled(machine("1000", &tall.to_string(), "1000"), Some("25"));
            assert!(problems(&roomy, &mesh, 0, &simulated(&roomy, &mesh)).is_empty());
            let (mut doc, mesh) = drilled(machine("1000", "50", "1000"), Some("25"));
            doc.setups[0]
                .machine
                .p
                .insert("gantry_height_source".into(), ASSUMED.into());
            let found = problems(&doc, &mesh, 0, &simulated(&doc, &mesh));
            assert!(
                found[0].1.contains("has 50 mm (ASSUMED, not measured"),
                "{}",
                found[0].1
            );
        }
        #[test]
        fn a_part_wider_than_the_gantry_is_refused() {
            let (doc, mesh) = drilled(machine("1000", "1000", "10"), Some("25"));
            let found = problems(&doc, &mesh, 0, &simulated(&doc, &mesh));
            assert_eq!(found.len(), 1, "{found:?}");
            assert!(
                found[0].1.contains("doesn't fit under the gantry: needs ")
                    && found[0].1.contains(" mm across X, has 10 mm"),
                "{}",
                found[0].1
            );
        }
        /// LIO-457: the gantry's width spans the axis across the table: Y on a
        /// machine whose X runs front to back.
        #[test]
        fn the_gantry_width_is_across_the_table_whichever_axis_that_is() {
            use crate::orient::{KEY, Orient};
            // X 500 long, Y 200 across, the gantry 250 wide.
            let mut m = machine("150", "120", "250");
            m.p.insert("x".into(), "500".into());
            m.p.insert("y".into(), "200".into());
            for (o, issue) in [
                ("Right", Some("the 500 mm X travel")),
                ("Left", Some("the 500 mm X travel")),
                ("Away from you", None),
                ("Toward you", None),
            ] {
                m.p.insert(KEY.into(), o.into());
                let found = issues(&m);
                match issue {
                    Some(words) => {
                        assert!(found.iter().any(|e| e.contains(words)), "{o}: {found:?}")
                    }
                    None => assert!(found.is_empty(), "{o}: {found:?}"),
                }
            }
            m.p.insert("x".into(), "200".into());
            m.p.insert("y".into(), "500".into());
            m.p.insert(KEY.into(), "Away from you".into());
            assert!(
                issues(&m).iter().any(|e| e.contains("the 500 mm Y travel")),
                "{:?}",
                issues(&m)
            );
            // A part that fits across one way and not the other: its reach
            // across X and across Y, and a gantry between the two.
            // The vise's jaws along X: wider across X than across Y.
            let (mut doc, mesh) = drilled(machine("1000", "1000", "1000"), Some("25"));
            doc.setups[0].p.insert("vise_axis".into(), "X".into());
            let reach = |o: Orient| {
                let mut d = doc.clone();
                d.setups[0].machine.p.insert(KEY.into(), o.label().into());
                let [lo, hi] = Stack::of(&d, &mesh, 0).unwrap().across.unwrap();
                hi - lo
            };
            let (wx, wy) = (reach(Orient::Right), reach(Orient::Away));
            assert!((wx - wy).abs() > 2., "{wx} {wy}");
            assert_eq!(reach(Orient::Left), wx);
            assert_eq!(reach(Orient::Toward), wy);
            let limit = format!("{}", (wx + wy) / 2.);
            for o in [Orient::Right, Orient::Left, Orient::Away, Orient::Toward] {
                let (mut d, mesh) = drilled(machine("1000", "1000", &limit), Some("25"));
                d.setups[0].p.insert("vise_axis".into(), "X".into());
                d.setups[0].machine.p.insert(KEY.into(), o.label().into());
                let found = problems(&d, &mesh, 0, &simulated(&d, &mesh));
                let wide = if o.across() == 0 { wx > wy } else { wy > wx };
                let axis = ["X", "Y"][o.across()];
                assert_eq!(
                    found
                        .iter()
                        .any(|(_, e)| e.contains(&format!(" mm across {axis}, has "))),
                    wide,
                    "{o:?}: {found:?}"
                );
                assert_eq!(found.len(), usize::from(wide), "{o:?}: {found:?}");
            }
        }
        #[test]
        fn a_tool_that_reaches_the_nose_is_refused_with_its_stick_out_and_holder() {
            let (mut doc, mesh) = drilled(machine("1000", "1000", "1000"), Some("25"));
            let sims = simulated(&doc, &mesh);
            let op = &doc.setups[0].operations[sims[0].0];
            let (stickout, id) = (
                sims[0].1.as_ref().unwrap().plan.tool_length,
                op.tool.clone(),
            );
            let tool = doc.tools.iter_mut().find(|t| t.id == id).unwrap();
            tool.p.insert("holder_length".into(), "20".into());
            let stack = Stack::of(&doc, &mesh, 0).unwrap();
            let sim = sims[0].1.as_ref().unwrap();
            let high = sim
                .plan
                .moves
                .iter()
                .flat_map(|m| [m.from[2], m.to[2]])
                .filter_map(|z| stack.height(z))
                .fold(stack.stock_top, f32::max);
            let needs = high + stickout + 20.;
            // A hair of room passes; a hair short is refused with the sum.
            doc.setups[0]
                .machine
                .p
                .insert("nose_to_table".into(), (needs + 0.1).to_string());
            assert!(problems(&doc, &mesh, 0, &sims).is_empty());
            doc.setups[0]
                .machine
                .p
                .insert("nose_to_table".into(), (needs - 0.1).to_string());
            let found = problems(&doc, &mesh, 0, &sims);
            assert_eq!(found.len(), 1, "{found:?}");
            assert_eq!(found[0].0, Some(sims[0].0));
            let said = &found[0].1;
            assert!(
                said.contains(&format!(
                    "the tool doesn't fit under the spindle: needs {} mm, has {} mm from the table",
                    crate::envelope::mm(needs),
                    crate::envelope::mm(needs - 0.1)
                )),
                "{said}"
            );
            assert!(
                said.contains(&format!(
                    "{} mm stick-out and 20 mm holder",
                    crate::envelope::mm(stickout)
                )),
                "{said}"
            );
            assert!(!said.contains("is taken"), "{said}");
            // A tool that gives no holder length is taken as the short default.
            doc.tools
                .iter_mut()
                .find(|t| t.id == id)
                .unwrap()
                .p
                .remove("holder_length");
            let found = problems(&doc, &mesh, 0, &sims);
            assert!(
                found.is_empty() || found[0].1.contains("The holder's 10 mm length is taken"),
                "{found:?}"
            );
        }
        #[test]
        fn a_tool_too_short_to_reach_down_is_refused_only_for_a_measured_nose_height() {
            let (mut doc, mesh) = drilled(machine("150", "1000", "1000"), Some("25"));
            // A long Z travel, so that the span of the job is inside it.
            doc.setups[0].machine.p.insert("z".into(), "100".into());
            let sims = simulated(&doc, &mesh);
            // The drilling goes down through the stock to near the vise.
            let stack = Stack::of(&doc, &mesh, 0).unwrap();
            let sim = sims[0].1.as_ref().unwrap();
            let tool = sim.plan.tool_length + HOLDER_LENGTH;
            let low = sim
                .plan
                .moves
                .iter()
                .flat_map(|m| [m.from[2], m.to[2]])
                .filter_map(|z| stack.height(z))
                .fold(f32::INFINITY, f32::min);
            let id = doc.setups[0].operations[sims[0].0].tool.clone();
            doc.tools
                .iter_mut()
                .find(|t| t.id == id)
                .unwrap()
                .p
                .remove("holder_length");
            // The tip's lowest reach is `low` + 1 mm too high.
            let nose = low + 1. + tool + 100.;
            doc.setups[0]
                .machine
                .p
                .insert("nose_to_table".into(), nose.to_string());
            let found = problems(&doc, &mesh, 0, &sims);
            assert!(found.is_empty(), "{found:?}");
            doc.setups[0]
                .machine
                .p
                .insert("nose_to_table_source".into(), ASSUMED.into());
            assert!(problems(&doc, &mesh, 0, &sims).is_empty());
            doc.setups[0]
                .machine
                .p
                .insert("nose_to_table_source".into(), MEASURED.into());
            let found = problems(&doc, &mesh, 0, &sims);
            assert_eq!(found.len(), 1, "{found:?}");
            assert!(
                found[0]
                    .1
                    .contains("the tool can't reach down: the cut goes to"),
                "{}",
                found[0].1
            );
            assert!(found[0].1.contains("100 mm Z travel"), "{}", found[0].1);
            // Lower by 2 mm: it reaches.
            doc.setups[0]
                .machine
                .p
                .insert("nose_to_table".into(), (nose - 2.).to_string());
            assert!(problems(&doc, &mesh, 0, &sims).is_empty());
        }
        #[test]
        fn export_run_and_the_plan_check_see_the_refusal() {
            let (doc, mesh) = drilled(machine("1000", "50", "1000"), Some("25"));
            let said = |messages: &[String]| {
                messages
                    .iter()
                    .filter(|m| m.contains("doesn't fit under the gantry"))
                    .count()
            };
            let posted = crate::post::post_job(&doc, &mesh);
            assert_eq!(said(posted.setups[0].result.as_ref().unwrap_err()), 1);
            assert_eq!(said(&crate::run::ready(&doc, &mesh, 0, 0).problems), 1);
            // The same job on a gantry with the room is ready.
            let (doc, mesh) = drilled(machine("1000", "1000", "1000"), Some("25"));
            assert_eq!(said(&crate::run::ready(&doc, &mesh, 0, 0).problems), 0);
            let posted = crate::post::post_job(&doc, &mesh);
            let errors = posted.setups[0].result.clone().err().unwrap_or_default();
            assert_eq!(said(&errors), 0);
        }
    }
    mod indexed_tests {
        use crate::indexed::tests::*;

        #[test]
        fn a_turn_of_a_below_the_swing_height_is_refused() {
            use crate::model::POST_RETRACT;
            // Setup 3 indexes to A90 as setup 2 left it: its plan has no index
            // move, so the program turns A to A90 where its retract leaves the
            // tool, since the A the machine is at when it starts is not known.
            let (mut doc, mesh) = posting_bar_job(20., "100", "90");
            let mut third = doc.setups[1].clone();
            third.id = crate::model::id();
            doc.setups.push(third);
            doc.sync_stock_sources();
            doc.sync_turns(Some(&mesh));
            // Setup 3 starts at the angle setup 2 left, and comes down from
            // above what A swings under the tool as an index move does
            // (LIO-205): its program turns A at machine Z home over the first
            // cut, and is exported. At the clearance plane, now over the bar
            // and not the jaws (LIO-209), a program that starts with the tool
            // wherever it was is refused, as on a 3-axis setup: it could arrive
            // over the jaws below their reach.
            for (mode, exported) in POST_RETRACT.iter().zip([true, false]) {
                doc.setups[0]
                    .machine
                    .p
                    .insert("post_retract".into(), (*mode).into());
                doc.sync_indexed();
                let posted = crate::post::post_job(&doc, &mesh);
                for k in [1, 2] {
                    let result = &posted.setups[k].result;
                    match exported {
                        true => assert!(result.is_ok(), "{mode}: {result:?}"),
                        false => assert!(
                            result.as_ref().is_err_and(|e| e
                                .iter()
                                .any(|e| e.contains("is not above the chuck jaw"))),
                            "{mode}: {result:?}"
                        ),
                    }
                }
            }
            // Its plans checked once the operation's clearance is raised to
            // 40 mm put the swing above where its program turns A, the
            // clearance plane: that turn is refused. At machine Z home it is not.
            let sims = crate::run::simulated_for_tests(&doc, &mesh, 2);
            let mut raised = doc.clone();
            (raised.setups[2].operations[0].p).insert("clearance".into(), "40".into());
            let below = "A turns to A 90° with the tool tip at Z";
            for (mode, refused) in POST_RETRACT.iter().zip([false, true]) {
                raised.setups[0]
                    .machine
                    .p
                    .insert("post_retract".into(), (*mode).into());
                raised.sync_indexed();
                let found = crate::post::checked(
                    &raised,
                    &mesh,
                    2,
                    &sims,
                    crate::post::PostSettings::from_machine,
                )
                .err()
                .unwrap_or_default();
                assert_eq!(
                    found
                        .iter()
                        .any(|e| e.contains(below)
                            && e.contains("where nothing A swings reaches it")),
                    refused,
                    "{mode}: {found:?}"
                );
            }
        }
        #[test]
        fn a_plain_setup_in_the_chuck_comes_down_from_above_the_jaws() {
            use crate::simulation::{Motion, prepare};
            let (doc, mesh) = chuck_setup_1_job("90, 180");
            // The kernel starts at the stock's -X -Y corner, in the jaws, at its
            // clearance plane, below what they reach.
            let mut fresh = doc.setups[0].clone();
            fresh.p.insert("stock_source".into(), "fresh".into());
            let op = &doc.setups[0].operations[0];
            let kernel = crate::simulation::generate(&fresh, op, &doc.tools[0], &mesh).unwrap();
            let stock = doc.setup_stock(0, Some(&mesh)).unwrap();
            let rig = doc.chuck_rig(&doc.setups[0], stock).unwrap();
            let corner = kernel.moves[0].from;
            assert!(
                corner[0] < rig.front && (corner[0] - stock.min[0]).abs() < 1.,
                "{corner:?}"
            );
            // Both operations instead come down over their first cut from the
            // swing height of setup 1's own clamping, with no A, clear of the
            // jaws (LIO-206).
            let swing = doc.swing(0, &mesh, 15.).unwrap();
            let all = doc.swing(1, &mesh, 15.).unwrap();
            assert_eq!((swing.what, swing.z), (all.what, all.z));
            assert!(corner[2] < swing.z - 30., "{corner:?} {swing:?}");
            let gap = crate::holding::gap(&doc.setups[0]).unwrap();
            for oi in 0..2 {
                let plan = prepare(&doc, &mesh, 0, oi).unwrap().plan;
                let first = (plan.moves.iter())
                    .position(|m| m.motion == Motion::Feed)
                    .unwrap();
                let (down, over) = (&plan.moves[..first], plan.moves[first].from);
                assert_eq!(down.len(), 1, "{down:?}");
                assert_eq!(down[0].motion, Motion::Rapid);
                assert!(close(down[0].from, [over[0], over[1], swing.z]), "{down:?}");
                assert!(close(down[0].to, over));
                assert!(plan.moves.iter().all(|m| m.a.is_none()));
                let tool = (plan.radius, plan.tool_length, plan.holder_radius);
                assert!(
                    rig.hits_at(0., &[(down[0].from, over)], tool, gap)
                        .is_empty()
                );
                assert!((plan.duration - plan.moves.last().unwrap().end).abs() < 1e-3);
            }
            // Its workholding check finds nothing, and the job exports: setup
            // 1 at A0 with no A word, and the indexed setup after it.
            let sims = crate::run::simulated_for_tests(&doc, &mesh, 0);
            let held = crate::post::held(&doc, &mesh, 0, &sims);
            assert!(held.is_empty(), "{held:?}");
            let posted = crate::post::post_job(&doc, &mesh);
            for k in [0, 1] {
                assert!(
                    posted.setups[k].result.is_ok(),
                    "{k}: {:?}",
                    posted.setups[k].result
                );
            }
            let program = &posted.setups[0].result.as_ref().unwrap()[0].text;
            let code = program.lines().filter(|l| !l.starts_with('('));
            assert!(code.clone().all(|l| !l.contains('A')), "{program}");
            // Both headers say the chuck was checked, not a vise (LIO-210).
            for k in [0, 1] {
                let program = &posted.setups[k].result.as_ref().unwrap()[0].text;
                assert!(
                    program.contains("(Not checked: the table, the machine body and tool-change motion. The chuck, its jaws, the rotary head,")
                        && !program.contains("Vise jaws"),
                    "{program}"
                );
            }
        }
    }
    mod rotary_check_tests {
        use crate::indexed::tests::bar_job;
        use crate::indexed::tests::contour;
        use crate::indexed::tests::cutter;
        use crate::indexed::tests::prism;
        use crate::mesh::Mesh;
        use crate::model::Document;
        use crate::model::get;
        use crate::rotary_check::tests::*;
        use crate::rotary_check::*;

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
                thickness: None,
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
                let uc100 = crate::machine_profile::rotary_machine(dir.path()).unwrap();
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
                thickness: None,
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
}
