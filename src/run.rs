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

const AXES: [&str; 3] = ["X", "Y", "Z"];

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
    /// Set Off the coolant the profile has no output for (`coolant_off`),
    /// then check again.
    CoolantOff,
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
    setup_id: String,
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
    /// The tool of the last tool run started and the Z work offset then.
    last: Option<(String, f64)>,
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
}

/// The stock before an operation of the setup being run, seen from above,
/// and the shape of its cutter.
#[derive(Debug, Clone)]
pub struct OpStock {
    /// The operation's index in the setup.
    pub op: usize,
    pub stock: StockGrid,
    pub shape: CutterShape,
}

/// The stock a run's tool runs have cut, as the machine's tool moved: each
/// tick, the moves of the program it has passed since the last tick whole,
/// the move it is on up to the tool's tip, and the tool where it is.
#[derive(Debug, Clone)]
pub struct Carving {
    pub stock: StockGrid,
    /// The block of the running blocks at the last sample: None between
    /// tool runs, so that a jog then does not cut.
    last: Option<usize>,
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
            last: None,
            touched: false,
            outcome: None,
            pictures: vec![],
            left: None,
            before: Arc::new(vec![]),
            carving: None,
            times: None,
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
                c.last = None;
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
        self.carving = Some(Carving {
            stock: b.stock.clone(),
            last: None,
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
            Some(Block::Move { op, from, to, .. }) => Some((*op, *from, *to)),
            _ => None,
        };
        let now = to.map_or(blocks.len(), |(b, _)| b);
        let mut sweeps: Vec<_> = (from.unwrap_or(0)..now).filter_map(ends).collect();
        if let (Some((_, tip)), Some((op, a, b))) = (to, ends(now)) {
            // The tip's place on the move: the share of it done.
            let d: [f32; 3] = std::array::from_fn(|k| b[k] - a[k]);
            let length = d.iter().map(|v| v * v).sum::<f32>();
            let t = if length < 1e-12 {
                1.
            } else {
                ((0..3).map(|k| (tip[k] - a[k]) * d[k]).sum::<f32>() / length).clamp(0., 1.)
            };
            sweeps.push((op, a, std::array::from_fn(|k| a[k] + t * d[k])));
            sweeps.push((op, tip, tip));
        }
        let cutter = |op: usize| {
            let shape = (self.before.iter())
                .find(|b| Some(b.op) == p.indexes.get(op).copied())
                .map_or(CutterShape::Flat, |b| b.shape);
            (p.ops.get(op).map_or(0., |o| o.diameter / 2.), shape)
        };
        let sweeps: Vec<_> = (sweeps.into_iter())
            .map(|(op, a, b)| (a, b, cutter(op)))
            .collect();
        if let Some(c) = &mut self.carving {
            for (a, b, (radius, shape)) in sweeps {
                c.stock.cut_shape(a, b, radius, shape);
                c.version += 1;
            }
        }
    }

    /// The stock as this run's tool runs have cut it, once one has moved
    /// (`carve`), with its version.
    pub fn stock(&self) -> Option<(&StockGrid, u64)> {
        let c = self.carving.as_ref()?;
        Some((&c.stock, c.version))
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
        // The start stays on its operation only when its tool run still has
        // the same operations; the work zero is set for it again.
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
        self.zeroed = false;
        self.programs = Some(programs);
        self.before = left
            .as_ref()
            .map_or_else(Default::default, |l| l.before.clone());
        self.left = left.filter(|_| self.checked().is_some());
        self.times = times.filter(|_| self.checked().is_some());
        self.pictures.clear();
    }

    /// The picture of the stock operation `op` (its index in the setup)
    /// leaves, once worked out.
    pub fn picture(&self, op: usize) -> Option<&iced::widget::image::Handle> {
        self.pictures.iter().find(|(i, _)| *i == op).map(|(_, p)| p)
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
        if o.ok {
            self.finish_carve();
            let Some(r) = &self.running else {
                return;
            };
            self.done[r.index] = true;
            self.next = r.index + 1;
            self.from = 0;
        } else if o.message.contains(NOT_STARTED) {
            let Before {
                xy,
                zero,
                last,
                touched,
            } = r.before.clone();
            (self.xy, self.zero, self.last, self.touched) = (xy, zero, last, touched);
        }
        self.outcome = Some((o.ok, o.message.clone()));
        self.running = None;
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
        if self.partway() {
            match self.partly() {
                None => e.push(format!("The check did not work out the stock operation {}, {}, starts from: pick its first operation, or check the setup again.", from + 1, first.name)),
                Some(_) if !self.zeroed => e.push(format!(
                    "This run starts at {}, after operations taken as cut: set the work zero on the stock they left under Work zero on the stock below, or press Keep this work zero there if it is still the one they were cut with.",
                    first.name
                )),
                Some(_) => {}
            }
        }
        if !c.idle() {
            e.push("The machine is not Idle.".into());
        }
        // No machine position, or no soft limits in the profile, does not
        // stop Cycle Start (`limits_unchecked`).
        let unset = unset_axes(c);
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
        let actions = lower(p, self.from, s.work_offset, s.machine_pos).ok()?;
        // Where the tool tip is, in plan coordinates: the first retract's start.
        let tip =
            std::array::from_fn(|k| (s.machine_pos[k] - s.work_offset[k]) as f32 + p.datum[k]);
        let paths = paths(p.from(self.from).ok()?, tip);
        let tool = p.ops[0].tool_id.clone();
        let zero = (p.origin.clone(), p.datum);
        let mut name = format!("{} · tool run {} of {count}", self.name, self.next + 1);
        if self.from > 0 {
            name += &format!(" from {}", p.ops[self.from].name);
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
        !self.started() && self.checked().is_some() && (self.next, self.from) != (0, 0)
    }

    /// What a run that starts partway finds of the work zero (`partway`,
    /// `Checked::partly`). None when it does not start partway, or the
    /// check did not work it out.
    pub fn partly(&self) -> Option<&Partly> {
        if !self.partway() {
            return None;
        }
        self.checked()?.get(self.next)?.partly.get(self.from)
    }

    /// Makes operation `k` of tool run `i` the one Cycle Start starts at:
    /// the work zero is set for the stock it starts from again.
    pub fn start_at(&mut self, i: usize, k: usize) {
        let ops = self
            .checked()
            .and_then(|c| c.get(i))
            .map_or(0, |p| p.ops.len());
        if self.running.is_none() && k < ops {
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

/// The operation a block belongs to, if it names one.
fn op_of(b: &Block) -> Option<usize> {
    match b {
        Block::Start { op, .. }
        | Block::Run { op, .. }
        | Block::Approach { op, .. }
        | Block::Descend { op, .. }
        | Block::Move { op, .. } => Some(*op),
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
    if si >= doc.setups.len() {
        return (Err(vec![REMOVED.into()]), Left::default());
    }
    if doc.file.is_none() {
        return (Err(vec![NO_PART.into()]), Left::default());
    }
    let (sims, left) = simulated_with(doc, mesh, si);
    let programs = crate::post::checked_to_start(doc, mesh, si, &sims, PostSettings::for_uc100);
    (programs, left)
}

const NO_PART: &str = "Import a part before running a setup. The example project's paths are illustrative and cannot be machined.";
const REMOVED: &str = "This setup was removed from the project.";

/// The `sequence` results of setup `si`'s enabled operations, from a
/// simulation of the job up to the end of that setup.
fn simulated(doc: &Document, mesh: &Mesh, si: usize) -> Sims {
    simulated_with(doc, mesh, si).0
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
            })
        })
        .collect();
    (
        sims,
        Left {
            after,
            last,
            before: Arc::new(before),
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
            "{} {} no machine position (every E-stop clears it), so Cycle Start runs with the soft limits off and does not check that the job fits the machine's travel. Make sure the work zero is on the stock (touch it off again after an E-stop, as the drives may lose steps) and watch the machine. {} under Position to turn them on.",
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
    let plane = clearance_plane(blocks);
    let machine = |q: [f32; 3]| -> [f64; 6] {
        let mut m = start;
        for k in 0..3 {
            m[k] = q[k] as f64 - p.datum[k] as f64 + offset[k];
        }
        m
    };
    let high = machine([0., 0., plane])[2];
    let mut out = vec![];
    let mut at = start;
    let (mut spindle, mut coolant) = (false, (false, false));
    for (i, block) in blocks.iter().enumerate() {
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
                to[2] = high;
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
            Block::Approach { xy, .. } => {
                go(&mut at, machine([xy[0], xy[1], plane]), None, &mut actions)
            }
            Block::Descend { xy, z, .. } => {
                go(&mut at, machine([xy[0], xy[1], *z]), None, &mut actions)
            }
            Block::Move {
                kind: Kind::Dwell,
                seconds,
                ..
            } => actions.push(Do::Dwell {
                seconds: *seconds as f64,
            }),
            Block::Move { kind, to, feed, .. } => {
                let feed = (*kind == Kind::Feed).then(|| *feed as f64 / 60.);
                go(&mut at, machine(*to), feed, &mut actions)
            }
            Block::End => actions.push(Do::End),
        }
        out.extend(actions.into_iter().map(|action| Located { line, action }));
    }
    Ok(out)
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
            "The pins' place is not recorded: Record the pins' place under Position first.".into(),
        );
        return e;
    };
    if !c.pin_referenced {
        e.push("X and Y are not referenced from pin 1 since the machine was connected or E-stopped: Reference X Y from pin 1 under Position first.".into());
        return e;
    }
    let want = [0, 1].map(|a| at[a] - on.pin1[a] as f64);
    let s = &c.snapshot;
    if (0..2).any(|a| (s.work_offset[a] - want[a]).abs() > 1e-3) {
        e.push(format!(
            "The X/Y work offset of {} is not the one the pins give, machine X {:.3} Y {:.3}: press Set X Y from the pins under Work zero on the stock.",
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
        let start = [7., 8., -2., 0., 0., 45.];
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
            }),
            xy: None,
            zero: None,
            reference: None,
            last: None,
            touched: false,
            outcome: None,
            pictures: vec![],
            left: None,
            before: Default::default(),
            times: None,
            carving: None,
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
}
