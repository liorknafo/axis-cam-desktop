//! Running a setup on the connected UC100 without G-code: its checked
//! programs, one per tool run; each lowered to the driver's actions when
//! Cycle Start is pressed; the checks before Cycle Start; progress and the
//! manual tool change between tool runs.
use crate::control::Connection;
use crate::mesh::Mesh;
use crate::model::Document;
use crate::post::{Block, Checked, Kind, OFFSETS, PostSettings, clearance_plane, extents};
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
    /// Make this tool run (index) the next one.
    Next(usize),
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
    /// The driver's outcome count before it started: the next outcome is its.
    seq: u64,
    /// The work offset it was lowered with.
    pub offset: [f64; 6],
    /// What Cycle Start recorded over (`xy`, `zero`, `last`, `touched`), put
    /// back when the driver refuses the job: nothing ran then (#168).
    before: Before,
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
            done: vec![],
            running: None,
            xy: None,
            zero: None,
            reference: None,
            last: None,
            touched: false,
            outcome: None,
        }
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
                self.checked_as(Arc::new(Err(vec![REMOVED.into()])));
                None
            }
        }
    }

    /// The checked programs, once the check passed.
    pub fn checked(&self) -> Option<&[Checked]> {
        self.programs.as_deref()?.as_deref().ok()
    }

    /// A check's answer.
    pub fn checked_as(&mut self, programs: Arc<Result<Vec<Checked>, Vec<String>>>) {
        let count = programs.as_ref().as_ref().map_or(0, Vec::len);
        if self.done.len() != count {
            self.done = vec![false; count];
            self.next = 0;
        }
        self.programs = Some(programs);
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
            self.done[r.index] = true;
            self.next = r.index + 1;
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
        if !c.idle() {
            e.push("The machine is not Idle.".into());
        }
        let unset: Vec<&str> = (0..3)
            .filter(|&a| !s.referenced[a])
            .map(|a| AXES[a])
            .collect();
        if !unset.is_empty() {
            e.push(format!(
                "{} {} no machine position (every E-stop clears it), so the soft limits are off. {} under Position, then Cycle Start.",
                unset.join(", "),
                if unset.len() == 1 { "has" } else { "have" },
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
                e.push(format!("The UC100 profile sets no soft limits on {}, so AXIS CAM cannot check that the job fits the machine.", AXES[a]));
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
        if limits.iter().all(Option::is_some) && unset.is_empty() {
            let span = extents(&p.blocks);
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
        e.extend(output_problems(&c.config.outputs, &p.ops));
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
            && (turning == Spindle::Cw) != p.ops[0].clockwise
        {
            e.push(format!(
                "The spindle is running {}, but tool run {} starts it {}. Stop it first (Stop under Spindle): the driver E-stops a reversal.",
                way(turning == Spindle::Cw),
                self.next + 1,
                way(p.ops[0].clockwise)
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
                "Tool run {} uses T{} {}: touch off its Z first (Set Z from reference, or Zero Z).",
                self.next + 1,
                p.ops[0].tool,
                p.ops[0].tool_name
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
        let actions = lower(p, s.work_offset, s.machine_pos);
        let tool = p.ops[0].tool_id.clone();
        let zero = (p.origin.clone(), p.datum);
        let name = format!("{} · tool run {} of {count}", self.name, self.next + 1);
        self.running = Some(Running {
            index: self.next,
            seq: s.program_outcome.as_ref().map_or(0, |o| o.seq),
            offset: s.work_offset,
            before: Before {
                xy: self.xy,
                zero: self.zero.clone(),
                last: self.last.clone(),
                touched: self.touched,
            },
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
        let p = self.checked()?.get(self.running.as_ref()?.index)?;
        let block = (line.unwrap_or(1).max(1) as usize - 1).min(p.blocks.len() - 1);
        let op = p.blocks[..=block].iter().rev().find_map(op_of).unwrap_or(0);
        Some((p, op, block))
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
    if si >= doc.setups.len() {
        return Err(vec![REMOVED.into()]);
    }
    if doc.file.is_none() {
        return Err(vec![NO_PART.into()]);
    }
    crate::post::checked(
        doc,
        mesh,
        si,
        &simulated(doc, mesh, si),
        PostSettings::for_uc100,
    )
}

const NO_PART: &str = "Import a part before running a setup. The example project's paths are illustrative and cannot be machined.";
const REMOVED: &str = "This setup was removed from the project.";

/// The `sequence` results of setup `si`'s enabled operations, from a
/// simulation of the job up to the end of that setup.
fn simulated(
    doc: &Document,
    mesh: &Mesh,
    si: usize,
) -> Vec<(usize, Result<crate::simulation::Simulation, String>)> {
    let last = doc.setups[si].operations.iter().rposition(|o| o.enabled);
    let mut sims = vec![];
    if let Some(oi) = last {
        crate::simulation::sequence(doc, mesh, Some((si, oi)), |s, oi, r| {
            if s == si {
                sims.push((oi, r))
            }
        });
    }
    sims
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
    Ready {
        key,
        setup: si,
        generated: sims
            .iter()
            .filter(|(_, r)| r.is_ok())
            .map(|(oi, _)| *oi)
            .collect(),
        held: crate::post::held_walked(doc, mesh, si, &sims).unwrap_or_default(),
        problems: problems.err().unwrap_or_default(),
        zero: crate::post::zero_checked(doc, mesh, si, &sims),
    }
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
pub fn lower(p: &Checked, offset: [f64; 6], start: [f64; 6]) -> Vec<Located> {
    let plane = clearance_plane(&p.blocks);
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
    for (i, block) in p.blocks.iter().enumerate() {
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
    out
}

/// A move to `to`, unless the tool is there already.
fn go(at: &mut [f64; 6], to: [f64; 6], feed: Option<f64>, actions: &mut Vec<Do>) {
    if to != *at {
        actions.push(Do::Move { target: to, feed });
        *at = to;
    }
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
    if !(crate::control::pins_settings(&setup.machine.p).ok().flatten())
        .is_some_and(|own| own.same_pins(pins))
    {
        e.push("This setup's snapshot of the machine has other fixture pins than the Machine library: use Assign to current setup there, then check the setup again.".into());
    }
    let Some(at) = pins.reference else {
        e.push("The pins' place is not recorded: Record the pins' place under Position first.".into());
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
        let located = lower(&p, [0.; 6], [0.; 6]);
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
        let actions = lower(&p, offset, start);
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
            done: vec![false],
            running: Some(Running {
                index: 0,
                seq: 0,
                offset: [0.; 6],
                before: Before {
                    xy: None,
                    zero: None,
                    last: None,
                    touched: false,
                },
            }),
            xy: None,
            zero: None,
            reference: None,
            last: None,
            touched: false,
            outcome: None,
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
