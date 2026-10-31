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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    /// The line each of its blocks moves the tool tip along (`paths`).
    paths: Vec<Option<[[f32; 3]; 2]>>,
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

/// The `Mark` of each of `blocks`, the operations' clearances `clear` (by
/// index in the program; None for one without steps): a step starts at the
/// first feed move after the tool, between two feed moves, rose to the
/// clearance (`plan_steps`), a retract to the clearance plane rising too.
fn step_marks(blocks: &[Block], clear: &[Option<f32>]) -> Vec<Option<Mark>> {
    let n = clear.len();
    let (mut count, mut fed, mut high) = (vec![0; n], vec![false; n], vec![false; n]);
    let mut last = None;
    (blocks.iter())
        .map(|b| {
            let op = match b {
                Block::Retract | Block::Stop | Block::End => last,
                _ => op_of(b),
            }
            .filter(|&o| o < n)?;
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
                    let c = clear[op].unwrap_or(f32::INFINITY);
                    high[op] |= fed[op] && from[2].max(to[2]) >= c - AT_CLEARANCE;
                }
                Block::Retract => high[op] |= fed[op],
                _ => {}
            }
            Some((op, count[op], high[op]))
        })
        .collect()
}

/// The run's record of its tool runs before a Cycle Start.
#[derive(Clone)]
struct Before {
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
            stopped: None,
            completed: false,
            project: doc.title.clone(),
            journal: None,
            resumed: None,
            journal_note: None,
        }
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
        // Where a run of this setup stopped: the start after it, while the
        // program of its operation is the one it stopped in.
        if let Some(j) = self.journal.take().filter(|_| self.checked().is_some()) {
            let same = self
                .op_steps(j.op)
                .is_some_and(|s| s.fingerprint == j.fingerprint);
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
            let up = self.checked().and_then(|c| c.get(r.index)).and_then(|p| {
                let plane = clearance_plane(p.from(r.from).ok()?);
                Some(match p.settings.machine_home {
                    true => 0.,
                    false => plane as f64 - p.datum[2] as f64 + r.offset[2],
                })
            });
            message = format!("Stopped after {}", self.step_label(op, s));
            self.stopped = Some(Stopped {
                op,
                last: Some(s),
                up,
                why: "Stop after step".into(),
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
    pub fn stop_due(&mut self, s: &uc100::controller::Snapshot) -> bool {
        if !self.stop_after || self.stopping.is_some() || s.state != Some(State::Run) {
            return false;
        }
        let Some((_, _, block)) = self.progress(s.current_line) else {
            return false;
        };
        let Some(step) = self.step_end(block) else {
            return false;
        };
        self.stopping = Some(step);
        true
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

    /// The Pi stopped the running tool run after a step, at job line
    /// `line`: the step it ends, for when its outcome comes.
    pub fn stopped_on_pi(&mut self, line: u32) {
        if self.stopping.is_none()
            && let Some((_, _, block)) = self.progress(Some(line))
        {
            self.stopping = self.step_end(block);
        }
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
        if c.pi_down() {
            return vec!["The link to the Pi is not answering: wait for it to reconnect.".into()];
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
                    if c.home_switches() { "Home again" } else { "Set machine zero again (or Keep the last position)" }
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
                Some(_) if !self.zeroed => e.push(format!(
                    "This run starts at {}, after operations taken as cut: set the work zero on the stock they left in the Work zero tab, or press Keep this work zero there if it is still the one they were cut with.",
                    first.name
                )),
                Some(_) => {}
            }
        }
        if !c.idle() {
            e.push("The machine is not Idle.".into());
        }
        // No machine position on X, Y or Z, or no soft limits in the profile,
        // does not stop Cycle Start (`limits_unchecked`). On A it does: the
        // driver runs nothing while A has soft limits and no position, and a
        // program never moves an A without one.
        let a_unset: Vec<&str> = (3..6)
            .filter(|&a| c.axis(a) && !s.referenced[a])
            .map(|a| AXES[a])
            .collect();
        if !a_unset.is_empty() {
            e.push(format!(
                "{} {} no machine position (every E-stop clears it), so the soft limits are off. {} in the Machine tab, then Cycle Start.",
                a_unset.join(", "),
                if a_unset.len() == 1 { "has" } else { "have" },
                if c.home_switches() { "Home again" } else { "Set machine zero again" }
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
        let mut actions = lower(p, self.from, s.work_offset, s.machine_pos).ok()?;
        // At a step, it waits over the step's entry with the spindle on,
        // after the rapids there, until Resume feeds in (LIO-275).
        if let Some((oi, n)) = self.at_step()
            && let Some(line) = (p.from(self.from).ok()?.iter())
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
        // Where the tool tip is, in plan coordinates: the first retract's start.
        let tip =
            std::array::from_fn(|k| (s.machine_pos[k] - s.work_offset[k]) as f32 + p.datum[k]);
        let blocks = p.from(self.from).ok()?;
        let paths = paths(blocks, tip);
        // An operation of one step ends it too, once risen to its clearance
        // (LIO-311).
        let clear: Vec<Option<f32>> = (p.indexes.iter())
            .map(|&oi| self.op_steps(oi).map(|s| s.clear))
            .collect();
        let marks = step_marks(blocks, &clear);
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
    /// before taken as cut. Its work zero is set on the stock they leave
    /// (`partly`), and Cycle Start waits for it (`zeroed`).
    pub fn partway(&self) -> bool {
        // An operation taken as cut before the start (`Skip::Cut`) is one.
        let start =
            (self.checked()).and_then(|c| c.get(self.next)?.indexes.get(self.from).copied());
        let cut = |&(oi, how): &(usize, Skip)| how == Skip::Cut && start.is_some_and(|s| oi < s);
        // So are steps taken as cut before it.
        let steps = |(oi, cut): &(usize, Vec<usize>)| {
            !cut.is_empty() && start.is_some_and(|s| *oi < s || *oi == s && cut.contains(&0))
        };
        let early = (self.next, self.from) != (0, 0)
            || self.skips.iter().any(cut)
            || self.cut_steps.iter().any(steps);
        self.checked().is_some() && (self.at_step().is_some() || !self.started() && early)
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
        let before = programs.get(self.next)?.partly.get(self.from)?;
        if self.at_step().is_none() {
            return Some(Cow::Borrowed(before));
        }
        // Inside an operation the check does not follow the stock, but it
        // knows it before the operation and after it (before the next one
        // sent): a spot neither cut is as it was, a surface the same in both
        // is that surface. Cut steps only take material away.
        let after = (programs.iter().flat_map(|p| &p.partly))
            .skip_while(|p| !std::ptr::eq(*p, before))
            .nth(1);
        let unknown = "stock the steps before the start cut, which the check does not follow";
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
                            (Err(e), _) => Err(e.clone()),
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
                let plan = &sim.plan;
                for m in (steps.iter())
                    .filter_map(|&s| found.steps.get(s))
                    .flat_map(|s| &plan.moves[s.moves.clone()])
                    .filter(|m| m.motion == crate::simulation::Motion::Feed)
                {
                    if let Some(frame) = sim.frame_at(m.a) {
                        let (r, flute) = (plan.radius, plan.flute_length);
                        volume.cut(m.from, m.to, r, flute, plan.shape, frame, sim.source);
                    }
                }
                sim.initial_volume = Some(volume);
            }
            // Steps taken as cut are in the stock already: the stock before
            // this operation and those before it has them cut, so the live
            // carve starts from it (LIO-312). ponytail: the top view only;
            // an indexed setup's moves (at an A) stay out of it.
            let before = Arc::make_mut(&mut left.before);
            let moves = (steps.iter().filter_map(|&s| found.steps.get(s)))
                .flat_map(|s| &sim.plan.moves[s.moves.clone()])
                .filter(|m| m.a.is_none());
            for m in moves {
                for b in before.iter_mut().filter(|b| b.op <= *oi) {
                    b.stock
                        .cut_shape(m.from, m.to, sim.plan.radius, sim.plan.shape);
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
}

/// The steps of one operation of a setup, as the run's check found them
/// (`op_steps`).
#[derive(Debug, Clone, PartialEq)]
pub struct OpSteps {
    /// The operation's index in the setup.
    pub op: usize,
    /// Its clearance: the highest plan Z it moves at.
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
    /// else passes.
    pub fn kind(&self) -> (&'static str, &'static str) {
        if self.hole {
            ("Hole", "Holes")
        } else if self.steps.iter().all(|s| s.z.is_some()) {
            ("Level", "Levels")
        } else {
            ("Pass", "Passes")
        }
    }

    /// Step `s`'s label: "Level 14", "Hole 3", "Pass 2".
    pub fn label(&self, s: usize) -> String {
        format!("{} {}", self.kind().0, s + 1)
    }

    /// Steps `range`'s label: "Levels 5–9", or one step's.
    pub fn labels(&self, range: std::ops::RangeInclusive<usize>) -> String {
        match range.end() - range.start() {
            0 => self.label(*range.start()),
            _ => format!(
                "{} {}–{}",
                self.kind().1,
                range.start() + 1,
                range.end() + 1
            ),
        }
    }
}

/// How far below its clearance the tool may be and still be at it, mm.
const AT_CLEARANCE: f32 = 0.01;

/// The steps of `plan` (`Step`): split where the tool, between two feed
/// moves, is at the plan's clearance, its highest Z, which must be above the
/// stock top. Empty for a plan that turns A, a lathe's, or one with no feed
/// move; one step for a plan that never goes back up there between feeds.
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
    let clear = (moves.iter())
        .map(|m| m.from[2].max(m.to[2]))
        .fold(f32::NEG_INFINITY, f32::max);
    if clear <= plan.stock.max[2] {
        return vec![];
    }
    // Between two feeds, the first move that starts at the clearance; a
    // stretch of rapids that only rises to it starts the step after them.
    let mut starts = vec![0];
    let mut fed = None;
    for i in 0..=last {
        if feed(i) {
            if let Some(f) = fed
                && i > f + 1
                && let Some(s) = (f + 1..=i).find(|&j| moves[j].from[2] >= clear - AT_CLEARANCE)
            {
                starts.push(s);
            }
            fed = Some(i);
        }
    }
    let ends = starts.iter().skip(1).copied().chain([moves.len()]);
    (starts.iter().zip(ends))
        .map(|(&a, b)| {
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
            let clear = (plan.moves.iter())
                .map(|m| m.from[2].max(m.to[2]))
                .fold(f32::NEG_INFINITY, f32::max);
            Some(OpSteps {
                op: *oi,
                clear,
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
/// a program enters as it enters an operation. Its stops follow their moves.
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
        for i in step.moves.clone() {
            new[i] = Some(out.moves.len());
            out.moves.push(plan.moves[i].clone());
            out.keep_air
                .push(plan.keep_air.get(i).copied().unwrap_or(false));
        }
    }
    out.stops = (plan.stops.iter())
        .filter_map(|s| {
            Some(Stop {
                at: new.get(s.at).copied().flatten()?,
                ..s.clone()
            })
        })
        .collect();
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

const NO_PART: &str = "Import a part before running a setup. The example project's paths are illustrative and cannot be machined.";
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
/// first shows the setup, `viewport::OP_PICTURE` px.
pub fn op_pictures(
    left: &Left,
    frame: crate::volume::Frame,
    jaws: &[crate::mesh::Bounds],
) -> Vec<(usize, iced::widget::image::Handle)> {
    let holding: Vec<_> = jaws
        .iter()
        .flat_map(|b| crate::viewport::box_triangles(*b, crate::viewport::JAWS))
        .collect();
    let picture = |volume: &crate::volume::Volume| {
        let surface =
            crate::stock_surface::Snapshot::of(volume, crate::stock_surface::Detail::Draft)
                .build()
                .ok()?;
        let mut triangles: Vec<_> =
            crate::viewport::stock_triangles(volume, &surface, frame).collect();
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
                .map(|(oi, volume)| (*oi, scope.spawn(|| picture(volume))))
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
    (0..3)
        .filter(|&a| !c.snapshot.referenced[a])
        .map(|a| AXES[a])
        .collect()
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
    let unset: Vec<&str> = unset_axes(c)
        .into_iter()
        .filter(|n| !unlimited.iter().any(|&a| AXES[a] == *n))
        .collect();
    if !unset.is_empty() {
        lines.push(format!(
            "{} {} no machine position (every E-stop clears it), so Cycle Start runs with the soft limits off and does not check that the job fits the machine's travel. Make sure the work zero is on the stock (touch it off again after an E-stop, as the drives may lose steps) and watch the machine. {} in the Machine tab to turn them on.",
            unset.join(", "),
            if unset.len() == 1 { "has" } else { "have" },
            if c.home_switches() { "Home" } else { "Set machine zero" }
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
    if crate::manual::is_hand(&doc.setups[si].machine) {
        return Ok(());
    }
    let programs = crate::post::checked(doc, mesh, si, sims, PostSettings::for_uc100)?;
    let config = crate::control::profile(&doc.setups[si].machine).map_err(|e| vec![e])?;
    let e: Vec<String> = programs
        .iter()
        .flat_map(|p| output_problems(&config.outputs, &p.ops))
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
/// is not recorded, X and Y are not referenced from pin 1 since the last
/// E-stop, or the work offset is not the one the pins give. None for any
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
    if !c.pin_referenced {
        e.push("X and Y are not referenced from pin 1 since the machine was connected or E-stopped: Reference X Y from pin 1 in the Machine tab first.".into());
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

#[cfg(test)]
mod tests {
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
        let (doc, mesh) = crate::guide::tests::job("Yes");
        // The manual mill's tools have no numbers: it makes no programs, so
        // its holding is not checked, and that is said.
        let sims = simulated(&doc, &mesh, 0);
        let errors = crate::post::held_walked(&doc, &mesh, 0, &sims).unwrap_err();
        let line = crate::post::not_checked(&errors);
        let why = errors[0].trim_end_matches('.');
        assert_eq!(line, format!("Holding not checked: {why}."));
        assert!(
            line.contains("tool number") || line.contains("number"),
            "{line}"
        );
        let held: Vec<(Option<usize>, String)> = (crate::post::held(&doc, &mesh, 0, &sims))
            .into_iter()
            .map(|h| (h.operation, h.message))
            .collect();
        assert_eq!(held, [(None, line.clone())]);
        // The marks do not call it a refused vise: it is one of the problems.
        let ready = ready(&doc, &mesh, 0, 1);
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

    /// One facing tool run: down to Z5, across at F600, a 1 s dwell.
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
            stopped: None,
            completed: false,
            project: String::new(),
            journal: None,
            resumed: None,
            journal_note: None,
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
            on: None,
            marks: vec![],
        });
        assert!(!run.skip(4, Some(Skip::Cut)) && run.skips.is_empty());
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
        let clear: Vec<Option<f32>> = (p.indexes.iter()).map(|&oi| Some(of(oi).clear)).collect();
        let marks = step_marks(&p.blocks, &clear);
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
