//! The connection to a UC100 machine controller: the driver's `Controller`
//! for one machine of the project, real (USB) or simulated.
use crate::model::{Fields, Machine, get, post_setting};
use std::collections::{BTreeMap, VecDeque};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};
use uc100::config::MachineConfig;
use uc100::controller::{Block, Command, Controller, ProbeResult, Snapshot, State};
use uc100::diagnostics::{INPUT_PINS, InputTracker};
use uc100::gcode::{Action as Do, Located, Spindle};
use uc100::protocol::Status;

/// Whether a UC100 machine connects to the simulator. Unset means the
/// simulator, except on macOS, where the real machine is connected.
pub fn simulated(m: &Machine) -> bool {
    match get(&m.p, "uc100_connection") {
        "USB" => false,
        "Simulated" => true,
        _ => !cfg!(target_os = "macos"),
    }
}

/// The Connection value shown for a machine, with the default applied.
pub fn connection_label(m: &Machine) -> &'static str {
    if simulated(m) { "Simulated" } else { "USB" }
}

/// Loads a machine's UC100 profile and checks it with the driver.
pub fn profile(m: &Machine) -> Result<MachineConfig, String> {
    let path = get(&m.p, "uc100_profile").trim();
    if path.is_empty() {
        return Err("Choose the machine's UC100 profile file.".into());
    }
    MachineConfig::load(Path::new(path)).map_err(|e| e.to_string())
}

/// The spindle speed a UC100 machine's profile caps the machine's rpm to, as a
/// run checks: its maximum at full PWM. None on any other machine, or when the
/// profile has no spindle PWM output.
pub fn spindle_cap(m: &Machine) -> Result<Option<f32>, String> {
    if get(&m.p, "controller") != "UC100" {
        return Ok(None);
    }
    let out = profile(m)?.outputs;
    Ok(out
        .spindle_pwm
        .is_some()
        .then_some(out.spindle_max_rpm as f32))
}

/// The coolant choices machine `m` has outputs for, in the order Flood,
/// Mist, Air, Off: on a UC100 only what its profile can switch, as a run
/// checks. Off is always one.
pub fn coolants(m: &Machine) -> Result<Vec<&'static str>, String> {
    // (mist, flood) outputs: LinuxCNC writes M7 and M8 on any machine.
    let outputs = if get(&m.p, "controller") == "UC100" {
        let out = profile(m)?.outputs;
        (out.mist.is_some(), out.flood.is_some())
    } else {
        (true, true)
    };
    let air = crate::post::PostSettings::for_machine(m)
        .ok()
        .and_then(|s| s.air);
    Ok([
        ("Flood", Some((false, true))),
        ("Mist", Some((true, false))),
        ("Air", air),
        ("Off", Some((false, false))),
    ]
    .into_iter()
    .filter(|(_, uses)| {
        uses.is_some_and(|(mist, flood)| (!mist || outputs.0) && (!flood || outputs.1))
    })
    .map(|(name, _)| name)
    .collect())
}

/// The coolant a new operation on machine `m` starts with, the first it has
/// outputs for: Flood, or on a UC100 Mist without a flood output and Off
/// with neither. Flood when the profile cannot be read: Run says why.
pub fn default_coolant(m: &Machine) -> &'static str {
    coolants(m).map_or("Flood", |c| c[0])
}

/// The owner's machine, the Jingyan CNC4030-800W on its UC100, for a
/// workspace whose library has no machine. Its profile, the driver's JY5800
/// profile, is written to `dir` unless a file is there already: Setup may
/// have edited it.
pub fn owners_machine(dir: &Path) -> std::io::Result<Machine> {
    let path = dir.join("jy5800.toml");
    if !path.exists() {
        std::fs::create_dir_all(dir)?;
        std::fs::write(&path, include_str!("../assets/jy5800.toml"))?;
    }
    let mut m = Machine::new(3);
    // Travel from the profile's soft limits, the feed from its X/Y speed and
    // the RPM from its (uncalibrated) spindle maximum.
    for (key, value) in [
        ("name", "Jingyan CNC4030"),
        ("controller", "UC100"),
        ("x", "390"),
        ("y", "280"),
        ("z", "55"),
        ("rpm", "24000"),
        ("feed", "1000"),
    ] {
        m.p.insert(key.into(), value.into());
    }
    m.p.insert("uc100_profile".into(), path.display().to_string());
    Ok(m)
}

/// A UC100 machine's probe settings (Machine library): mm and mm/min.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Probe {
    /// The touch plate's thickness, for Probe Z; None until it is set.
    pub plate: Option<f64>,
    /// The probe tip's diameter, for Probe corner and Probe centre; None
    /// until it is set.
    pub tip: Option<f64>,
    pub feed: f64,
    /// How far down a probe goes at most.
    pub distance: f64,
    /// How far the tool lifts after contact.
    pub retract: f64,
}

/// The machine's probe settings, or what is wrong with them. The plate and
/// the tip are measured: left empty, they are not set.
pub fn probe_settings(p: &Fields) -> Result<Probe, Vec<String>> {
    let mut e = vec![];
    let mut value = |key, what: &str, lo: f64, hi: f64, unit: &str| {
        let text = post_setting(p, key).trim();
        match text.parse::<f64>() {
            Ok(v) if (lo..=hi).contains(&v) => Some(v),
            _ if text.is_empty() && matches!(key, "probe_plate" | "probe_tip") => None,
            _ => {
                e.push(format!("{what} must be from {lo} to {hi} {unit}."));
                None
            }
        }
    };
    let probe = Probe {
        plate: value("probe_plate", "Touch plate thickness", 0., 100., "mm"),
        tip: value("probe_tip", "Probe tip diameter", 0., 50., "mm"),
        feed: value("probe_feed", "Probe feed", 1., 1000., "mm/min").unwrap_or(0.),
        distance: value("probe_distance", "Maximum probe distance", 0.1, 200., "mm").unwrap_or(0.),
        retract: value("probe_retract", "Retract after the probe", 0., 100., "mm").unwrap_or(0.),
    };
    if e.is_empty() { Ok(probe) } else { Err(e) }
}

/// What a touch-plate probe straight down sets once the plate is touched.
/// The plate lies on a surface; its top is the surface + its thickness.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Touch {
    /// The surface is the stock top, at this work Z (`Connection::stock_top`).
    WorkZ(f64),
    /// Record the surface's work Z as the tool-change reference.
    Reference,
    /// The surface is the reference, at this work Z.
    FromReference(f64),
}

/// A probe move sent and not reported yet.
struct Probing {
    touch: Touch,
    probe: Probe,
    /// Machine Z it probes down to.
    target: f64,
    /// The driver's program outcome and probe result counts before it
    /// started: the next of each are its. Every probe ends with an outcome,
    /// also a refused or stopped one.
    seq: (u64, u64),
}

/// The work Z to give machine Z `now` so that machine Z `trip` is at work Z
/// `at`: the machine stops a little past the contact.
pub fn work_z(at: f64, trip: f64, now: f64) -> f64 {
    at + now - trip
}

/// The simulator's block: a box on the middle of the table, its top halfway
/// down Z, for trying the probe (min and max corners, machine mm): `size` X
/// and Y (a setup's stock), or up to 100 × 100 mm. None without soft limits
/// on X, Y and Z to place it by.
pub fn sim_block(cfg: &MachineConfig, size: Option<[f32; 3]>) -> Option<([f64; 3], [f64; 3])> {
    let mut lo = [0.; 3];
    let mut hi = [0.; 3];
    for a in 0..3 {
        let x = cfg.axes.get(a)?;
        let (min, max) = (x.soft_min?, x.soft_max?);
        if a < 2 {
            let half = size.map_or(((max - min) / 4.).min(50.), |s| s[a] as f64 / 2.);
            let mid = (min + max) / 2.;
            (lo[a], hi[a]) = (mid - half, mid + half);
        } else {
            (lo[a], hi[a]) = (min, (min + max) / 2.);
        }
    }
    Some((lo, hi))
}

type Vec6 = [f64; 6];

/// A face a probe touches for a stock-based work zero, in work coordinates
/// (mm from the work zero).
#[derive(Debug, Clone)]
struct Face {
    name: &'static str,
    axis: usize,
    /// 1 when the probe moves toward + to touch it (the -X and -Y sides),
    /// -1 toward − (the +X and +Y sides, the top).
    dir: f64,
    /// Where the face is on its axis.
    at: f64,
    /// The spot it is touched in (`post::touch_spots`).
    lo: [f64; 3],
    hi: [f64; 3],
}

/// One command of a probing routine.
#[derive(Debug, Clone)]
enum Step {
    /// Rapid moves, then a probe move that must touch nothing.
    Go(Vec<Vec6>, Option<Vec6>),
    /// A probe move that must touch face `.0`.
    Touch(usize, Vec6),
}

/// Probe corner, Probe edge or Probe centre, by the work zero's
/// references (`post::references`): the top, then the sides, each touched
/// at its spot (`post::touch_spots`), one driver command at a time. The
/// work zero is set where the setup's datum is from the faces touched.
struct Edges {
    refs: [crate::post::At; 2],
    /// The work zero is on the stock's bottom, `size[2]` below the top.
    bottom: bool,
    /// The work zero is Model origin, found from the front-left corner.
    model: bool,
    faces: Vec<Face>,
    /// The setup's stock (mm).
    size: [f64; 3],
    /// How far a face may be from where it is expected (mm): how high above
    /// the top the probe travels, how far outside a side it goes down, and
    /// how far past where the side should be it probes.
    clear: f64,
    radius: f64,
    feed: f64,
    /// How far the first touch of the top goes down at most.
    distance: f64,
    /// The bottom and the top of Z (machine).
    z: (f64, f64),
    /// Where the probe was when it started (machine).
    start: Vec6,
    /// Each face's touches: where the probe touched it (machine), as the
    /// driver reports it.
    touches: Vec<Vec<Vec6>>,
    /// Where the last touch began: backing off returns there.
    from: Vec6,
    steps: VecDeque<Step>,
    /// The command under way and the counts before it (`Connection::seq`).
    sent: Option<(Step, (u64, u64))>,
}

impl Edges {
    /// For a setup's work zero `origin` found from the faces of `refs`
    /// on its `stock` at `datum` (plan mm), the probe with a tip of `tip`
    /// mm starting at `start`.
    #[allow(clippy::too_many_arguments)]
    fn new(
        origin: &str,
        refs: [crate::post::At; 2],
        stock: crate::mesh::Bounds,
        datum: [f32; 3],
        probe: &Probe,
        tip: f64,
        start: Vec6,
        z: (f64, f64),
    ) -> Self {
        let rel = |v: f32, a: usize| (v - datum[a]) as f64;
        let faces: Vec<Face> = crate::post::probe_faces(stock, refs)
            .into_iter()
            .map(|(name, axis, far, spot)| Face {
                name,
                axis,
                dir: if far { -1. } else { 1. },
                at: rel(
                    if far {
                        stock.max[axis]
                    } else {
                        stock.min[axis]
                    },
                    axis,
                ),
                lo: std::array::from_fn(|a| rel(spot.min[a], a)),
                hi: std::array::from_fn(|a| rel(spot.max[a], a)),
            })
            .collect();
        Self {
            refs,
            bottom: crate::post::stock_zero(origin).is_some_and(|(_, top)| !top),
            model: crate::post::stock_zero(origin).is_none(),
            touches: vec![vec![]; faces.len()],
            faces,
            size: stock.dimensions().map(f64::from),
            // 10 mm, a quarter of the shorter top side when less.
            clear: crate::post::corner_area(stock) as f64,
            radius: tip / 2.,
            feed: probe.feed,
            distance: probe.distance,
            z,
            start,
            from: start,
            steps: VecDeque::new(),
            sent: None,
        }
    }

    /// Where face `f` is on its axis from a touch at `p`: a side is a tip
    /// radius ahead of the tip's centre; the top is where the tip's bottom is.
    fn contact(&self, f: &Face, p: &Vec6) -> f64 {
        p[f.axis] + if f.axis < 2 { f.dir * self.radius } else { 0. }
    }

    /// The work zero (machine) as far as the touches tell, and the axes they
    /// fix: one whose faces are all touched. Until then X and Y come from
    /// where the probe started, as if over the middle of the top's spot.
    fn zero(&self) -> ([f64; 3], [bool; 3]) {
        let (mut w, mut fixed) = ([0.; 3], [false; 3]);
        for k in 0..3 {
            let on: Vec<Option<f64>> = (self.faces.iter().zip(&self.touches))
                .filter(|(f, _)| f.axis == k)
                .map(|(f, t)| Some(self.contact(f, t.last()?) - f.at))
                .collect();
            let found: Vec<f64> = on.iter().flatten().copied().collect();
            fixed[k] = found.len() == on.len();
            let top = &self.faces[0];
            w[k] = if found.is_empty() {
                self.start[k] - (top.lo[k] + top.hi[k]) / 2.
            } else {
                found.iter().sum::<f64>() / found.len() as f64
            };
        }
        (w, fixed)
    }

    /// The face to touch next: the first not touched yet, then, once, one
    /// touched outside its spot now that the touches fix where that is.
    fn next(&self) -> Option<usize> {
        let (w, fixed) = self.zero();
        let off = |i: usize| {
            let (f, p) = (&self.faces[i], &self.touches[i][0]);
            (0..2).any(|a| {
                a != f.axis
                    && fixed[a]
                    && !(f.lo[a] - 1e-3..=f.hi[a] + 1e-3).contains(&(p[a] - w[a]))
            })
        };
        let n = self.faces.len();
        (0..n)
            .find(|&i| self.touches[i].is_empty())
            .or_else(|| (0..n).find(|&i| self.touches[i].len() == 1 && off(i)))
    }

    /// The commands that touch face `i`: back to where the last touch began,
    /// up to the clearance above the top, over, down outside a side (a probe
    /// move that must touch nothing) and in toward it; or down onto the top.
    fn plan(&self, i: usize) -> VecDeque<Step> {
        if self.touches[0].is_empty() {
            // The first touch: straight down from where the probe is.
            let mut to = self.start;
            to[2] = (to[2] - self.distance).max(self.z.0);
            return [Step::Touch(0, to)].into();
        }
        let (w, fixed) = self.zero();
        let f = &self.faces[i];
        let mid = |a: usize| (f.lo[a] + f.hi[a]) / 2.;
        let top = self.top(w);
        let high = (top + self.clear).min(self.z.1);
        let mut up = self.from;
        up[2] = high;
        let mut over = up;
        for a in 0..2 {
            over[a] = if fixed[a] {
                w[a] + mid(a)
            } else {
                self.start[a]
            };
        }
        let k = f.axis;
        if k == 2 {
            let mut to = over;
            to[2] = (top - self.clear).max(self.z.0);
            return [
                Step::Go(vec![self.from, up, over], None),
                Step::Touch(i, to),
            ]
            .into();
        }
        let side = w[k] + f.at;
        over[k] = side - f.dir * (self.clear + self.radius);
        // A ball tip touches at its equator, a radius above its bottom.
        let mut down = over;
        down[2] = w[2] + (mid(2) - self.radius).max(f.lo[2]);
        let mut to = down;
        to[k] = side + f.dir * self.clear;
        [
            Step::Go(vec![self.from, up, over], Some(down)),
            Step::Touch(i, to),
        ]
        .into()
    }

    /// The machine Z of the stock top, given the work zero `w` (machine).
    fn top(&self, w: [f64; 3]) -> f64 {
        w[2] + self.faces[0].at
    }

    /// Where the tip should be found, for a message: "its centre", "the
    /// front-left corner", "the middle of the front edge".
    fn spot(&self) -> String {
        probe_spot(self.refs)
    }

    fn name(&self) -> &'static str {
        probe_name(self.refs)
    }

    /// What was found, once every face is touched at its spot.
    fn found(&self) -> String {
        use crate::post::At;
        let (w, _) = self.zero();
        let count: usize = self.touches.iter().map(Vec::len).sum();
        let spot = match self.spot().as_str() {
            "its centre" => "the centre".to_string(),
            s => s.to_string(),
        };
        let at = format!("at machine X {:.3} Y {:.3} Z {:.3}", w[0], w[1], w[2]);
        let mut s = if self.model {
            format!(
                "Work zero set on Model origin {at}, set off from {spot} of the stock top, from {count} touches."
            )
        } else if self.bottom {
            format!(
                "Work zero set on the stock bottom under {spot} of the stock top, {:.3} mm below it, {at}, from {count} touches.",
                self.faces[0].at
            )
        } else {
            format!("Work zero set on {spot} of the stock top {at}, from {count} touches.")
        };
        // Both sides of an axis touched: the stock measured across it.
        let across = |k: usize| {
            let mut c = (self.faces.iter().zip(&self.touches))
                .filter(|(f, _)| f.axis == k)
                .map(|(f, t)| self.contact(f, t.last().unwrap()));
            let (a, b) = (c.next().unwrap(), c.next().unwrap());
            (a - b).abs()
        };
        match self.refs {
            [At::Mid, At::Mid] => {
                s += &format!(
                    " Measured {:.3} × {:.3} mm across; the setup's stock is {:.3} × {:.3} mm.",
                    across(0),
                    across(1),
                    self.size[0],
                    self.size[1]
                )
            }
            [At::Mid, _] | [_, At::Mid] => {
                let k = usize::from(self.refs[1] == At::Mid);
                s += &format!(
                    " Measured {:.3} mm across {}; the setup's stock is {:.3} mm.",
                    across(k),
                    ["X", "Y"][k],
                    self.size[k]
                );
            }
            _ => {}
        }
        s
    }
}

/// The probing routine that finds a work zero from the faces of `refs`
/// (`post::references`): Probe centre, Probe edge or Probe corner.
pub fn probe_name(refs: [crate::post::At; 2]) -> &'static str {
    use crate::post::At::Mid;
    match refs {
        [Mid, Mid] => "Probe centre",
        [Mid, _] | [_, Mid] => "Probe edge",
        _ => "Probe corner",
    }
}

/// Where on the stock top the probing routine for `refs` starts:
/// "its centre", "the middle of the front edge", "the front-left corner".
pub fn probe_spot(refs: [crate::post::At; 2]) -> String {
    use crate::post::At::{Max, Mid};
    match refs {
        [Mid, Mid] => "its centre".into(),
        [Mid, _] | [_, Mid] => {
            let k = usize::from(refs[0] == Mid);
            format!(
                "the middle of the {} edge",
                crate::post::ENDS[k][usize::from(refs[k] == Max)]
            )
        }
        _ => format!("the {} corner", crate::post::corner_name(refs)),
    }
}

/// Rapid moves through `points` from `at`, leaving out any that go nowhere.
fn moves_from(mut at: Vec6, points: &[Vec6]) -> Vec<Do> {
    let mut out = vec![];
    for &p in points {
        if p != at {
            out.push(Do::Move {
                target: p,
                feed: None,
            });
            at = p;
        }
    }
    out
}

/// A job of `actions` in machine coordinates, numbered from line 1.
fn job(name: &str, actions: Vec<Do>) -> Command {
    let actions = (actions.into_iter().enumerate())
        .map(|(i, action)| Located {
            line: i as u32 + 1,
            action,
        })
        .collect();
    Command::Job {
        name: name.into(),
        actions: Arc::new(actions),
    }
}

/// How often a held jog is sent again: the driver ends a jog that is not
/// refreshed within 400 ms, so a lost release cannot run an axis away.
const JOG_RESEND: Duration = Duration::from_millis(100);
/// The driver's own limit: it ends a jog not sent again within this.
const JOG_KEEPALIVE: Duration = Duration::from_millis(400);
/// A held jog that has not started by then was refused by the driver.
const JOG_START: Duration = Duration::from_millis(400);

/// What the Control page asks of the connected machine.
#[derive(Debug, Clone)]
pub enum Action {
    /// Home every axis in the profile's order.
    Home,
    /// Declare the current position machine zero: X, Y and Z, or one axis.
    MachineZero(Option<usize>),
    /// Declare every axis at the machine position it shows, which after
    /// connecting is where the machine was left.
    KeepPosition,
    /// A jog button or key went down: one step, or a continuous jog for as
    /// long as it is held (axis, negative).
    JogPress(usize, bool),
    /// A jog button or key went up, the pointer left the button, or the
    /// window lost the focus.
    JogRelease,
    /// The step size in mm; None jogs continuously.
    JogStep(Option<f64>),
    /// Jog speed, % of each axis's maximum velocity.
    JogSpeed(f32),
    /// Make the current position work zero of the active coordinate
    /// system: one axis, or every axis.
    WorkZero(Option<usize>),
    /// Select G54 (0) … G59 (5).
    Wcs(usize),
    /// The RPM field.
    Rpm(String),
    Spindle(Spindle),
    Flood(bool),
    Mist(bool),
    /// Feed override, %.
    FeedOverride(f32),
    /// Ignore the limit switches, to drive off one that tripped.
    OverrideLimits(bool),
    /// The MDI field.
    MdiText(String),
    /// Run the MDI field's line.
    MdiSend,
    /// Force an output pin high or low for a wiring test, or release it.
    TestOutput(u8, Option<bool>),
    /// Calibration: the axis, the commanded travel field, the measured
    /// travel field, and the move.
    CalAxis(usize),
    CalDistance(String),
    CalMeasured(String),
    CalMove,
    /// Probe Z down onto the touch plate on the stock top: work Z there is
    /// the plate's thickness.
    ProbeZ,
    /// Probe corner, Probe edge or Probe centre: the X, Y and Z work zero
    /// of the setup picked in the Run panel, from its stock.
    ProbeEdges,
    /// Sets the work position where the tool tip is to that of the setup
    /// picked in the Run panel's touch-off point (`Connection::touch_off`):
    /// the tip is on the stock top there. Like Probe Z, it waits while the
    /// setup's check is out of date (`Connection::run_program`).
    TouchOff,
    /// Simulated: rapid the tool tip onto the work zero of the setup picked
    /// in the Run panel on the simulator's block (`Connection::sim_zero`).
    /// Waits as TouchOff does.
    ToSimZero,
}

/// Steps-per-mm calibration: one commanded move, then the travel measured
/// at the machine.
pub struct Calibration {
    pub axis: usize,
    /// Commanded travel, mm.
    pub distance: String,
    /// Measured travel, mm.
    pub measured: String,
    /// The axis's step counter before the move.
    start: Option<i32>,
}

/// The input pins' changes since connecting, as seen at each tick.
#[derive(Default)]
pub struct InputLog {
    tracker: InputTracker,
    /// The last edges, newest last: seconds since connecting, pin, HIGH now.
    pub edges: VecDeque<(f64, u8, bool)>,
}

impl InputLog {
    fn observe(&mut self, since: Duration, st: &Status) {
        for (pin, high) in self.tracker.observe(st) {
            self.edges.push_back((since.as_secs_f64(), pin, high));
            if self.edges.len() > 20 {
                self.edges.pop_front();
            }
        }
    }

    /// How often a pin changed since connecting.
    pub fn changes(&self, pin: u8) -> u32 {
        INPUT_PINS
            .iter()
            .position(|&p| p == pin)
            .map_or(0, |i| self.tracker.transitions[i])
    }
}

/// The continuous jog held down.
struct Held {
    axis: usize,
    negative: bool,
    sent: Instant,
    /// The driver has reported Jog since the press.
    started: bool,
    /// The driver's log count at the press. It logs a jog it ends at a
    /// soft limit, so a line since means this jog is over.
    log_seq: u64,
}

/// A running driver controller for one machine.
pub struct Connection {
    controller: Controller,
    pub machine_id: String,
    pub simulated: bool,
    /// The profile the controller was started with; the snapshot has no config.
    pub config: MachineConfig,
    /// The driver's state at the last tick.
    pub snapshot: Snapshot,
    /// Machine positions still to declare (axis, value), sent one per tick:
    /// the driver drops a second `SetMachine` until it has re-read the position.
    declare: VecDeque<(usize, f64)>,
    held: Option<Held>,
    pub jog_step: Option<f64>,
    /// Jog speed, % of each axis's maximum velocity.
    pub jog_speed: f32,
    /// Feed override, %.
    pub feed_override: f32,
    pub rpm: String,
    pub mdi: String,
    /// The setup being run, tool run by tool run.
    pub run: Option<crate::run::Run>,
    opened: Instant,
    pub inputs: InputLog,
    /// Output pins forced for a wiring test (pin, high). The driver releases
    /// them all on E-stop.
    pub forced: BTreeMap<u8, bool>,
    pub calibration: Calibration,
    /// The machine's probe settings, or what is wrong with them.
    pub probe: Result<Probe, Vec<String>>,
    probing: Option<Probing>,
    edges: Option<Edges>,
    /// How the last probe ended (ok, what it did).
    pub probed: Option<(bool, String)>,
    /// The simulator's block (min and max corners, machine mm).
    pub block: Option<([f64; 3], [f64; 3])>,
    /// Keeps macOS from throttling AXIS CAM in the background, which would
    /// starve the controller's motion buffer.
    _nap: uc100::macos::ActivityToken,
}

impl Connection {
    /// Starts the controller for `m`. The driver connects and leaves the
    /// machine in E-stop. `state_dir` holds its saved offsets and last
    /// position: one file per machine, and a separate one for the simulator
    /// so a simulated session never overwrites the real machine's.
    pub fn open(m: &Machine, state_dir: &Path) -> Result<Self, String> {
        let mut cfg = profile(m)?;
        let simulated = simulated(m);
        cfg.controller.simulated = simulated;
        let suffix = if simulated { "-simulated" } else { "" };
        let file = state_dir.join(format!("{}{suffix}.toml", m.id));
        let controller = Controller::start(cfg.clone(), Some(file));
        let block = sim_block(&cfg, None).filter(|_| simulated);
        if let Some((min, max)) = block {
            controller.send(Command::SimBlock(Some(Block { min, max })));
        }
        Ok(Self {
            _nap: uc100::macos::prevent_app_nap("AXIS CAM machine control"),
            controller,
            config: cfg,
            machine_id: m.id.clone(),
            simulated,
            snapshot: Snapshot::default(),
            declare: VecDeque::new(),
            held: None,
            jog_step: None,
            jog_speed: 50.,
            feed_override: 100.,
            rpm: String::new(),
            mdi: String::new(),
            run: None,
            opened: Instant::now(),
            inputs: InputLog::default(),
            forced: BTreeMap::new(),
            calibration: Calibration {
                axis: 0,
                distance: "100".into(),
                measured: String::new(),
                start: None,
            },
            probe: probe_settings(&m.p),
            probing: None,
            edges: None,
            probed: None,
            block,
        })
    }

    /// The simulator's block takes the X/Y size of the stock of the setup
    /// to run, once it is checked, so that Probe centre finds its sides.
    pub fn fit_sim_block(&mut self) {
        let size = (self.run.as_ref())
            .and_then(|r| r.checked()?.first())
            .map(|p| p.stock.dimensions());
        if let (true, Some(size)) = (self.simulated, size)
            && let Some((min, max)) = sim_block(&self.config, Some(size))
        {
            self.block = Some((min, max));
            self.send(Command::SimBlock(Some(Block { min, max })));
        }
    }

    /// Simulated: where point `at` of the setup to run (plan mm) is on the
    /// simulator's block, which stands for its stock (machine mm). None until
    /// the setup is checked.
    fn on_block(&self, at: impl FnOnce(&crate::post::Checked) -> [f32; 3]) -> Option<[f64; 3]> {
        let (lo, hi) = self.block.filter(|_| self.simulated)?;
        let p = self.run.as_ref()?.checked()?.first()?;
        let (s, d) = (p.stock, at(p));
        Some([
            lo[0] + (d[0] - s.min[0]) as f64,
            lo[1] + (d[1] - s.min[1]) as f64,
            hi[2] + (d[2] - s.max[2]) as f64,
        ])
    }

    /// Simulated: where the work zero of the setup to run is on the
    /// simulator's block (`on_block`).
    pub fn sim_zero(&self) -> Option<[f64; 3]> {
        self.on_block(|p| p.datum)
    }

    /// Simulated: where the setup to run is touched off on the simulator's
    /// block (`touch_off`), or its work zero when that is not on the stock
    /// top (`on_block`).
    pub fn sim_touch(&self) -> Option<[f64; 3]> {
        let at = self.touch_off().map(|(at, _)| at);
        self.on_block(|p| at.unwrap_or(p.datum))
    }

    /// Rapids the tool tip up to the top of Z, over where the setup is
    /// touched off on the simulator's block and down onto it (`sim_touch`),
    /// once X, Y and Z have a machine position.
    fn to_sim_zero(&mut self) -> Option<()> {
        let to = self
            .sim_touch()
            .filter(|_| self.idle() && self.unset_limited_axes().is_empty())?;
        let now = self.snapshot.machine_pos;
        let mut up = now;
        up[2] = self.config.axes.get(2)?.soft_max?;
        let mut over = up;
        over[..2].copy_from_slice(&to[..2]);
        let mut down = over;
        down[2] = to[2];
        self.send(job("To the work zero", moves_from(now, &[up, over, down])));
        Some(())
    }

    /// The driver's program outcome and probe result counts now: the next
    /// of each belong to the command sent next.
    fn seq(&self) -> (u64, u64) {
        let s = &self.snapshot;
        (
            s.program_outcome.as_ref().map_or(0, |o| o.seq),
            s.probe_result.as_ref().map_or(0, |r| r.seq),
        )
    }

    /// How the command sent after counts `seq` ended, once the machine has
    /// stopped: ok, the driver's message, and its probe result if it made one.
    fn ended(&self, seq: (u64, u64)) -> Option<(bool, String, Option<ProbeResult>)> {
        let s = &self.snapshot;
        let o = s.program_outcome.as_ref().filter(|o| o.seq > seq.0)?;
        if matches!(s.state, Some(State::Run | State::Hold | State::Stopping)) {
            return None;
        }
        let r = s.probe_result.as_ref().filter(|r| o.ok && r.seq > seq.1);
        Some((o.ok, o.message.clone(), r.cloned()))
    }

    pub fn send(&self, command: Command) {
        self.controller.send(command)
    }

    pub fn refresh(&mut self) {
        self.snapshot = self.controller.snapshot();
        self.observe();
    }

    /// Acts on the snapshot `refresh` read.
    fn observe(&mut self) {
        if let Some(run) = &mut self.run {
            run.refresh(self.snapshot.program_outcome.as_ref());
        }
        if let Some(st) = &self.snapshot.status {
            self.inputs.observe(self.opened.elapsed(), st);
        }
        if self.snapshot.state == Some(State::EStop) {
            self.forced.clear();
        }
        // A probe ends with its outcome, once the machine has stopped.
        if let Some((_, message, result)) = self.probing.as_ref().and_then(|p| self.ended(p.seq)) {
            let p = self.probing.take().unwrap();
            match result {
                Some(r) => self.probe_done(p, r.tripped.then_some(r.position[2])),
                None => self.probed = Some((false, message)),
            }
        }
        self.edges_step();
        // Any state but Idle (E-stop, a move) drops what is left, so nothing
        // is declared later by surprise.
        if self.idle() {
            if let Some((axis, value)) = self.declare.pop_front() {
                self.send(Command::SetMachine { axis, value });
            }
        } else {
            self.declare.clear();
        }
        let jogging = self.snapshot.state == Some(State::Jog);
        if let Some(h) = &mut self.held {
            if jogging {
                h.started = true;
                // Sent again only while this is still the jog the press
                // started: the driver has logged nothing since (it logs a
                // jog it ends at a soft limit) and its keepalive cannot
                // have lapsed. The driver ends a jog itself and goes Idle a
                // few of its loops later, with Jog in the snapshots between;
                // a resend from one of those would reach it Idle and start
                // a new jog, which must take a new press.
                let same = h.log_seq == self.snapshot.log_seq && h.sent.elapsed() <= JOG_KEEPALIVE;
                if same && h.sent.elapsed() >= JOG_RESEND {
                    h.sent = Instant::now();
                    self.controller.send(Command::JogStart {
                        axis: h.axis,
                        negative: h.negative,
                        speed: self.jog_speed as f64 / 100.,
                    });
                }
            } else if h.started || h.sent.elapsed() > JOG_START {
                // The jog ended without a release (a soft limit, E-stop, the
                // driver's timeout) or never started: it needs a new press.
                self.held = None;
            }
        }
    }

    /// Does what the page asks, if the machine can; otherwise nothing.
    pub fn act(&mut self, action: Action) {
        match action {
            Action::Home if self.idle() && self.home_switches() => self.send(Command::Home(vec![])),
            Action::MachineZero(axis) if self.idle() => {
                let axes = axis.map_or(0..3, |a| a..a + 1);
                let zero: Vec<_> = axes
                    .filter(|&a| self.axis(a) && self.switchless(a))
                    .map(|a| (a, 0.))
                    .collect();
                self.declare.extend(zero);
            }
            Action::KeepPosition if self.idle() => {
                let here: Vec<_> = (0..6)
                    .filter(|&a| self.axis(a) && self.switchless(a))
                    .map(|a| (a, self.snapshot.machine_pos[a]))
                    .collect();
                self.declare.extend(here);
            }
            Action::JogPress(axis, negative) if self.manual() && self.axis(axis) => {
                match self.jog_step {
                    Some(step) => self.send(Command::JogStep {
                        axis,
                        distance: if negative { -step } else { step },
                        feed: self.jog_feed(axis),
                    }),
                    None if self.held.is_none() => {
                        self.send(Command::JogStart {
                            axis,
                            negative,
                            speed: self.jog_speed as f64 / 100.,
                        });
                        self.held = Some(Held {
                            axis,
                            negative,
                            sent: Instant::now(),
                            started: false,
                            log_seq: self.snapshot.log_seq,
                        });
                    }
                    None => {}
                }
            }
            Action::JogRelease => {
                if self.held.take().is_some() {
                    self.send(Command::JogStop);
                }
            }
            Action::JogStep(step) => self.jog_step = step,
            Action::JogSpeed(percent) => self.jog_speed = percent.clamp(1., 100.),
            Action::WorkZero(axis) if self.manual() => {
                if let (Some(run), None | Some(2)) = (&mut self.run, axis) {
                    run.touched = true;
                }
                let axes = axis.map_or(0..6, |a| a..a + 1);
                for a in axes.filter(|&a| self.axis(a)) {
                    self.send(Command::SetWork { axis: a, value: 0. });
                }
            }
            Action::Wcs(i) if self.idle() && i < 6 => {
                self.send(Command::Mdi(format!("G{}", 54 + i)))
            }
            Action::Rpm(text) => self.rpm = text,
            Action::Spindle(Spindle::Off) => self.send(Command::Spindle {
                state: Spindle::Off,
                rpm: 0.,
            }),
            Action::Spindle(state) if self.spindle_ok(state) => {
                if let Some(rpm) = self.rpm_value() {
                    self.send(Command::Spindle { state, rpm });
                }
            }
            Action::Flood(flood) if self.config.outputs.flood.is_some() => {
                self.send(Command::Coolant {
                    flood,
                    mist: self.snapshot.mist,
                })
            }
            Action::Mist(mist) if self.config.outputs.mist.is_some() => {
                self.send(Command::Coolant {
                    flood: self.snapshot.flood,
                    mist,
                })
            }
            Action::FeedOverride(percent) => {
                self.feed_override = percent.clamp(10., 200.);
                self.send(Command::FeedOverride(self.feed_override as f64 / 100.));
            }
            Action::OverrideLimits(on) if self.limit_inputs() => {
                self.send(Command::OverrideLimits(on))
            }
            Action::MdiText(text) => self.mdi = text,
            // A line the machine cannot take now stays in the field.
            Action::MdiSend if self.manual() && !self.mdi.trim().is_empty() => {
                let line = std::mem::take(&mut self.mdi);
                self.send(Command::Mdi(line.trim().into()));
            }
            Action::TestOutput(pin, None) => {
                self.forced.remove(&pin);
                self.send(Command::TestOutput { pin, high: None });
            }
            Action::CalAxis(axis) if self.axis(axis) => {
                self.calibration.axis = axis;
                self.calibration.start = None;
            }
            Action::CalDistance(text) => self.calibration.distance = text,
            Action::CalMeasured(text) => self.calibration.measured = text,
            Action::CalMove => {
                let axis = self.calibration.axis;
                if let (true, Some(distance), Some(st)) =
                    (self.idle(), self.cal_distance(), &self.snapshot.status)
                {
                    self.calibration.start = Some(st.position[axis]);
                    self.send(Command::JogStep {
                        axis,
                        distance,
                        feed: self.jog_feed(axis),
                    });
                }
            }
            Action::ProbeZ => {
                if let Ok(top) = self.stock_top() {
                    self.start_probe(Touch::WorkZ(top));
                }
            }
            Action::ProbeEdges => self.start_edges(),
            // Both act on the checked setup, which an edit may have changed:
            // like the probes, they wait while its check is out of date (#39).
            Action::TouchOff if self.manual() && self.run_program().is_ok() => {
                if let Some((_, work)) = self.touch_off() {
                    for (a, value) in work.into_iter().enumerate() {
                        self.send(Command::SetWork { axis: a, value });
                    }
                    if let Some(run) = &mut self.run {
                        run.touched = true;
                    }
                }
            }
            Action::ToSimZero if self.run_program().is_ok() => {
                self.to_sim_zero();
            }
            Action::TestOutput(pin, Some(high)) if self.can_force(pin) => {
                self.forced.insert(pin, high);
                self.send(Command::TestOutput {
                    pin,
                    high: Some(high),
                });
            }
            _ => {}
        }
    }

    /// Does what the Run panel asks, if the machine can; otherwise nothing.
    /// Picking and checking a setup is the app's: it needs the part.
    pub fn run_act(&mut self, action: crate::run::Action, doc: &crate::model::Document) {
        use crate::run::Action as R;
        let Some(mut run) = self.run.take() else {
            return;
        };
        let state = self.snapshot.state.clone();
        match action {
            R::Next(i) if run.running.is_none() && i < run.done.len() => run.next = i,
            R::CycleStart => {
                if let Some(job) = run.cycle_start(self, doc) {
                    self.send(job);
                }
            }
            R::Hold if state == Some(State::Run) => self.send(Command::FeedHold),
            R::Resume if matches!(state, Some(State::Hold | State::Paused(_))) => {
                self.send(Command::Resume)
            }
            R::Stop if crate::run::busy(state.as_ref()) => self.send(Command::Stop),
            R::RecordReference if self.manual() => run.reference = Some(self.snapshot.work_pos[2]),
            R::ZFromReference if self.manual() => {
                if let Some(z) = run.reference {
                    self.send(Command::SetWork { axis: 2, value: z });
                    run.touched = true;
                }
            }
            R::ProbeReference => self.start_probe(Touch::Reference),
            R::ProbeFromReference => {
                if let Some(z) = run.reference {
                    self.start_probe(Touch::FromReference(z));
                }
            }
            _ => {}
        }
        self.run = Some(run);
    }

    /// Why a probe moving `axes` cannot start now; None when it can.
    fn probe_ready(&self, axes: &[usize]) -> Option<String> {
        let s = &self.snapshot;
        let Some(pin) = self.config.inputs.probe else {
            return Some(
                "The UC100 profile has no probe input: add it under Inputs in Controller configuration.".into(),
            );
        };
        if let Err(e) = &self.probe {
            return Some(format!("{} Set it in the Machine library.", e.join(" ")));
        }
        if !self.idle() {
            return Some("The machine is not Idle.".into());
        }
        let unset: Vec<&str> = (axes.iter())
            .filter(|&&a| !s.referenced[a])
            .map(|&a| ["X", "Y", "Z"][a])
            .collect();
        if !unset.is_empty() {
            return Some(format!(
                "{} {} no machine position (every E-stop clears it): {} first.",
                unset.join(", "),
                if unset.len() == 1 { "has" } else { "have" },
                if self.home_switches() {
                    "home"
                } else {
                    "set machine zero"
                }
            ));
        }
        if s.status.as_ref().and_then(|st| st.input_active(pin)) == Some(true) {
            return Some("The probe input is active: lift the tool or probe off what it touches, or check the probe's clip and wiring.".into());
        }
        None
    }

    /// The work Z of the stock top a touch plate lies on for Probe Z: that
    /// of the setup picked in the Run panel (`touch_off`), 0 without one.
    pub fn stock_top(&self) -> Result<f64, String> {
        if self.run.is_none() {
            return Ok(0.);
        }
        self.run_program()?;
        self.touch_off()
            .map(|(_, work)| work[2])
            .ok_or_else(|| format!("{} Probe Z cannot set it.", crate::post::STOCK_TOP_CUT))
    }

    /// Why Probe Z cannot start now; None when it can.
    pub fn probe_z_blocked(&self) -> Option<String> {
        self.probe_blocked().or_else(|| self.stock_top().err())
    }

    /// Why a touch-plate probe cannot start now; None when it can.
    pub fn probe_blocked(&self) -> Option<String> {
        self.probe_ready(&[2]).or_else(|| match &self.probe {
            Ok(Probe { plate: None, .. }) => Some(
                "Touch plate thickness is not set: measure the plate and set it in the Machine library."
                    .into(),
            ),
            _ => None,
        })
    }

    /// A probe move or probing routine is under way: planning is locked
    /// (`App::locked`).
    pub fn probing(&self) -> bool {
        self.probing.is_some() || self.edges.is_some()
    }

    /// Probe corner or Probe centre is under way.
    pub fn edging(&self) -> bool {
        self.edges.is_some()
    }

    /// Probes straight down, up to the maximum distance or the bottom of Z.
    fn start_probe(&mut self, touch: Touch) {
        let (None, Ok(probe)) = (self.probe_blocked(), self.probe.clone()) else {
            return;
        };
        let mut target = self.snapshot.machine_pos;
        target[2] -= probe.distance;
        if let Some(lo) = self.config.axes.get(2).and_then(|z| z.soft_min) {
            target[2] = target[2].max(lo);
        }
        self.probing = Some(Probing {
            touch,
            probe,
            target: target[2],
            seq: self.seq(),
        });
        self.probed = None;
        // G38.3: a miss is not an error; it is reported as one here.
        self.send(Command::Probe {
            target,
            feed: probe.feed,
            must_trip: false,
        });
    }

    /// A probe ended, Idle: on contact at machine Z `trip`, sets what it is
    /// for and lifts the tool.
    fn probe_done(&mut self, p: Probing, trip: Option<f64>) {
        let Some(trip) = trip else {
            self.probed = Some((
                false,
                format!(
                    "No contact down to machine Z {:.3}. Check the plate and the probe's clip, jog closer and probe again.",
                    p.target
                ),
            ));
            return;
        };
        let s = &self.snapshot;
        let (now, plate) = (s.machine_pos[2], p.probe.plate.unwrap_or_default());
        let message = match p.touch {
            Touch::Reference => {
                // The surface under the plate, at today's work offset.
                let surface = trip - now + s.work_pos[2] - plate;
                if let Some(run) = &mut self.run {
                    run.reference = Some(surface);
                }
                format!("Reference surface recorded: Z {surface:.3} under the {plate} mm plate.")
            }
            Touch::WorkZ(at) | Touch::FromReference(at) => {
                self.send(Command::SetWork {
                    axis: 2,
                    value: work_z(at + plate, trip, now),
                });
                if let Some(run) = &mut self.run {
                    run.touched = true;
                }
                format!(
                    "Work Z set: the plate's top is Z {:.3}, the surface under it Z {at:.3}.",
                    at + plate
                )
            }
        };
        let top = self.config.axes.get(2).and_then(|z| z.soft_max);
        let up = top.map_or(p.probe.retract, |top| p.probe.retract.min(top - now));
        if up > 0. {
            let feed = self.config.axes.get(2).map_or(0., |z| z.max_velocity);
            self.send(Command::JogStep {
                axis: 2,
                distance: up,
                feed,
            });
        }
        self.probed = Some((true, format!("Contact at machine Z {trip:.3}. {message}")));
    }

    /// The first program of the setup picked in the Run panel, once it is
    /// checked for the project as it is, or why there is none.
    pub fn run_program(&self) -> Result<&crate::post::Checked, String> {
        let Some(run) = &self.run else {
            return Err("Pick a setup in the Run panel.".into());
        };
        let p = match run.programs.as_deref() {
            None => return Err("Checking the setup…".into()),
            Some(Ok(p)) if !p.is_empty() => &p[0],
            Some(_) => return Err("The setup cannot run: see the Run panel.".into()),
        };
        if run.stale {
            return Err(
                "The project changed since the setup was checked: check it again in the Run panel."
                    .into(),
            );
        }
        Ok(p)
    }

    /// The faces a probe touches to find the work zero of the setup to run
    /// (`post::Checked::probe`) and its first program, or why no probe
    /// finds it.
    pub fn edges_setup(&self) -> Result<([crate::post::At; 2], &crate::post::Checked), String> {
        if self.run.is_none() {
            return Err("Probe corner, Probe edge and Probe centre set the work zero of the setup picked in the Run panel, from its stock.".into());
        }
        let p = self.run_program()?;
        match &p.probe {
            Ok(refs) => Ok((*refs, p)),
            Err(why) => Err(why.clone()),
        }
    }

    /// Where the operator touches off the setup to run, in plan mm, and the
    /// work position that is there (`post::touch_point` less the datum).
    /// None when it is not on the stock top: a Model origin whose stock top
    /// an earlier setup cut.
    pub fn touch_off(&self) -> Option<([f32; 3], [f64; 3])> {
        let p = self.run.as_ref()?.checked()?.first()?;
        if p.located.is_some_and(|(_, top)| top.is_none()) {
            return None;
        }
        let at = crate::post::touch_point(&p.origin, p.stock)?;
        Some((at, std::array::from_fn(|a| (at[a] - p.datum[a]) as f64)))
    }

    /// Why Probe corner or Probe centre cannot start now; None when it can.
    pub fn edges_blocked(&self) -> Option<String> {
        let (_, p) = match self.edges_setup() {
            Ok(x) => x,
            Err(why) => return Some(why),
        };
        if let Some(why) = self.probe_ready(&[0, 1, 2]) {
            return Some(why);
        }
        if let Ok(Probe { tip: None, .. }) = self.probe {
            return Some("Probe tip diameter is not set: set it in the Machine library.".into());
        }
        let active = 54 + self.snapshot.active_wcs;
        if p.offset != format!("G{active}") {
            return Some(format!(
                "This setup uses {}, but the active work system is G{active}: select {} first.",
                p.offset, p.offset
            ));
        }
        None
    }

    /// Starts Probe corner or Probe centre from where the probe is.
    fn start_edges(&mut self) {
        let (None, Ok(probe), Ok((refs, p))) =
            (self.edges_blocked(), self.probe.clone(), self.edges_setup())
        else {
            return;
        };
        let z = self.config.axes.get(2);
        let z = (
            z.and_then(|z| z.soft_min).unwrap_or(f64::NEG_INFINITY),
            z.and_then(|z| z.soft_max).unwrap_or(f64::INFINITY),
        );
        let start = self.snapshot.machine_pos;
        let tip = probe.tip.unwrap_or_default();
        self.edges = Some(Edges::new(
            &p.origin, refs, p.stock, p.datum, &probe, tip, start, z,
        ));
        self.probed = None;
        self.edges_step();
    }

    /// Follows the probing routine: once a command has ended, checks how and
    /// sends the next, until every face is touched at its spot.
    fn edges_step(&mut self) {
        let Some(mut e) = self.edges.take() else {
            return;
        };
        if let Some((step, seq)) = e.sent.clone() {
            let Some((ok, message, result)) = self.ended(seq) else {
                self.edges = Some(e);
                return;
            };
            let tripped = result.as_ref().filter(|r| r.tripped);
            let failed = match &step {
                _ if !ok => Some(message),
                Step::Go(_, Some(_)) if tripped.is_some() => {
                    let f = e.steps.front().map_or(0, |s| match s {
                        Step::Touch(i, _) => *i,
                        _ => 0,
                    });
                    let p = tripped.unwrap().position;
                    Some(format!(
                        "The probe touched something going down beside the {} at machine X {:.3} Y {:.3} Z {:.3}, where the stock should not be: start over the stock top nearer {}.",
                        e.faces[f].name,
                        p[0],
                        p[1],
                        p[2],
                        e.spot()
                    ))
                }
                Step::Touch(i, to) => match tripped {
                    Some(r) => {
                        e.touches[*i].push(r.position);
                        None
                    }
                    None if e.touches[0].is_empty() => Some(format!(
                        "No contact down to machine Z {:.3}: jog the probe over the stock top, closer, and probe again.",
                        to[2]
                    )),
                    None => {
                        let k = e.faces[*i].axis;
                        Some(format!(
                            "No contact with the {} up to machine {} {:.3}: check the probe and the setup's stock, and start over the stock top nearer {}.",
                            e.faces[*i].name,
                            ["X", "Y", "Z"][k],
                            to[k],
                            e.spot()
                        ))
                    }
                },
                _ => None,
            };
            if let Some(why) = failed {
                self.probed = Some((false, why));
                return;
            }
            e.sent = None;
        }
        if e.steps.is_empty() {
            match e.next() {
                Some(i) => e.steps = e.plan(i),
                None => return self.edges_done(e),
            }
        }
        let step = e.steps.pop_front().unwrap();
        let seq = self.seq();
        match &step {
            Step::Go(moves, guard) => {
                let mut actions = moves_from(self.snapshot.machine_pos, moves);
                if let Some(target) = guard {
                    actions.push(Do::Probe {
                        target: *target,
                        feed: e.feed / 60.,
                        must_trip: false,
                    });
                }
                self.send(job(e.name(), actions));
            }
            Step::Touch(_, target) => {
                e.from = self.snapshot.machine_pos;
                // G38.3: a miss is not an error; it is reported as one here.
                self.send(Command::Probe {
                    target: *target,
                    feed: e.feed,
                    must_trip: false,
                });
            }
        }
        e.sent = Some((step, seq));
        self.edges = Some(e);
    }

    /// Every face is touched at its spot, Idle: sets the X, Y and Z work zero
    /// and backs the probe off and up.
    fn edges_done(&mut self, e: Edges) {
        let (w, _) = e.zero();
        let now = self.snapshot.machine_pos;
        for a in 0..3 {
            self.send(Command::SetWork {
                axis: a,
                value: now[a] - w[a],
            });
        }
        if let Some(run) = &mut self.run {
            run.touched = true;
        }
        let mut up = e.from;
        up[2] = (e.top(w) + e.clear).min(e.z.1);
        self.send(job(e.name(), moves_from(now, &[e.from, up])));
        self.probed = Some((true, e.found()));
    }

    /// An axis of the profile. The driver panics on an index of 6 or more,
    /// so nothing else is ever sent.
    pub fn axis(&self, a: usize) -> bool {
        a < 6 && self.config.axis_enabled(a)
    }

    /// An axis without a home switch: declaring its machine position
    /// references it. The driver references an axis with a switch only by
    /// homing, and a declaration takes that away.
    pub fn switchless(&self, a: usize) -> bool {
        self.config.axes.get(a).is_some_and(|x| x.home.is_none())
    }

    /// The continuous jog held down (axis, negative).
    pub fn held(&self) -> Option<(usize, bool)> {
        self.held.as_ref().map(|h| (h.axis, h.negative))
    }

    /// The step jog feed of an axis at the jog speed, mm/min.
    pub fn jog_feed(&self, axis: usize) -> f64 {
        self.config
            .axes
            .get(axis)
            .map_or(0., |a| a.max_velocity * self.jog_speed as f64 / 100.)
    }

    /// The RPM field, when it is a speed the profile's spindle can run.
    pub fn rpm_value(&self) -> Option<f64> {
        let max = self.config.outputs.spindle_max_rpm;
        self.rpm
            .trim()
            .parse()
            .ok()
            .filter(|&rpm: &f64| rpm >= 1. && rpm <= max)
    }

    /// The profile has a way to run the spindle CCW. Without one the driver
    /// E-stops on CCW, so it is never offered.
    pub fn reversible(&self) -> bool {
        let o = &self.config.outputs;
        o.spindle_dir.is_some() || o.spindle_ccw.is_some()
    }

    /// The spindle may start this way now: never CCW without a direction
    /// output, and never the other way while it turns (both E-stop).
    pub fn spindle_ok(&self, state: Spindle) -> bool {
        let turning = self.snapshot.spindle.unwrap_or(Spindle::Off);
        self.manual()
            && (state != Spindle::Ccw || self.reversible())
            && (turning == Spindle::Off || turning == state)
    }

    /// Some axis has a limit switch, so there is something to override.
    pub fn limit_inputs(&self) -> bool {
        (0..6)
            .filter_map(|a| self.config.axes.get(a))
            .any(|a| a.limit_pos.is_some() || a.limit_neg.is_some())
    }

    /// The commanded calibration travel, when it is a move to make.
    pub fn cal_distance(&self) -> Option<f64> {
        let d: f64 = self.calibration.distance.trim().parse().ok()?;
        (d.is_finite() && d != 0. && self.axis(self.calibration.axis)).then_some(d)
    }

    /// Step pulses commanded on the calibration axis since its move began.
    pub fn cal_pulses(&self) -> Option<i32> {
        let st = self.snapshot.status.as_ref()?;
        Some(st.position[self.calibration.axis] - self.calibration.start?)
    }

    /// Steps per mm from the pulses and the measured travel, once both exist.
    pub fn calibrated(&self) -> Option<Result<f64, String>> {
        let measured: f64 = self.calibration.measured.trim().parse().ok()?;
        let pulses = self.cal_pulses()?;
        Some(
            uc100::diagnostics::measured_steps_per_unit(pulses, measured)
                .map_err(|e| e.to_string()),
        )
    }

    /// An output pin may be forced: while Idle, and never one the firmware
    /// drives (step, direction, charge pump, PWM, spindle direction).
    pub fn can_force(&self, pin: u8) -> bool {
        self.idle() && uc100::protocol::is_output_pin(pin) && !self.config.firmware_output_pin(pin)
    }

    /// Nothing moves: the profile may be saved and the machine reconnected.
    pub fn stopped(&self) -> bool {
        matches!(
            self.snapshot.state,
            None | Some(State::EStop | State::Idle | State::Disconnected)
        )
    }

    /// Idle, and no probing routine is between two of its moves.
    pub fn idle(&self) -> bool {
        self.snapshot.state == Some(State::Idle) && self.edges.is_none()
    }

    /// Jog, work zero, MDI and the spindle are allowed: Idle, or paused in a program.
    pub fn manual(&self) -> bool {
        matches!(self.snapshot.state, Some(State::Idle | State::Paused(_))) && self.edges.is_none()
    }

    /// Every enabled axis has a home switch, so Home can run.
    pub fn home_switches(&self) -> bool {
        (0..6)
            .filter_map(|a| self.config.axes.get(a))
            .all(|a| a.home.is_some())
    }

    pub fn soft_limits_configured(&self) -> bool {
        self.config.soft_limits_configured()
    }

    /// Axes with a soft limit and no machine position yet. The driver applies
    /// the soft limits only once this is empty.
    pub fn unset_limited_axes(&self) -> Vec<usize> {
        (0..6)
            .filter(|&a| {
                self.config
                    .axes
                    .get(a)
                    .is_some_and(|x| x.soft_min.is_some())
                    && !self.snapshot.referenced[a]
            })
            .collect()
    }

    /// Disables the machine and stops the controller thread.
    pub fn close(self) {
        let Connection { controller, .. } = self;
        controller.send(Command::Disconnect);
        controller.shutdown();
    }
}

/// A widget operation that answers whether any text field has the keyboard
/// focus: the arrow keys jog only while nobody is typing.
pub fn typing() -> impl iced::advanced::widget::Operation<bool> {
    use iced::advanced::widget::{
        Id, Operation,
        operation::{Focusable, Outcome},
    };
    struct Typing(bool);
    impl Operation<bool> for Typing {
        fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<bool>)) {
            operate(self)
        }
        fn focusable(&mut self, _: Option<&Id>, _: iced::Rectangle, state: &mut dyn Focusable) {
            self.0 |= state.is_focused()
        }
        fn finish(&self) -> Outcome<bool> {
            Outcome::Some(self.0)
        }
    }
    Typing(false)
}

/// A widget operation that selects all of the focused text field, so typing
/// replaces its value: a Tab into a field ends with it.
pub fn select_focused() -> impl iced::advanced::widget::Operation {
    use iced::advanced::widget::{
        Id, Operation,
        operation::{self, Focusable, TextInput, focusable::count},
    };
    // The focused field's index, and the fields passed. A text field is
    // offered as a text input first, then as focusable.
    struct SelectAt(Option<usize>, usize);
    impl Operation for SelectAt {
        fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
            operate(self)
        }
        fn text_input(&mut self, _: Option<&Id>, _: iced::Rectangle, state: &mut dyn TextInput) {
            if self.0 == Some(self.1) {
                state.select_all()
            }
        }
        fn focusable(&mut self, _: Option<&Id>, _: iced::Rectangle, _: &mut dyn Focusable) {
            self.1 += 1
        }
    }
    operation::then(count(), |count| SelectAt(count.focused, 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Like the JY5800: X, Y, Z with soft limits and no switches.
    const LIMITED: &str = r#"
[controller]
step_rate = "100kHz"
[axes.x]
steps_per_unit = 200.0
max_velocity = 2000.0
acceleration = 300.0
step = { pin = 2 }
dir = { pin = 3 }
soft_min = 0.0
soft_max = 390.0
[axes.y]
steps_per_unit = 200.0
max_velocity = 2000.0
acceleration = 300.0
step = { pin = 4 }
dir = { pin = 5 }
soft_min = 0.0
soft_max = 280.0
[axes.z]
steps_per_unit = 400.0
max_velocity = 1000.0
acceleration = 200.0
step = { pin = 6 }
dir = { pin = 7 }
soft_min = -55.0
soft_max = 0.0
"#;

    /// Refreshes `c` until `ok` holds of its snapshot; fails once the
    /// driver has looped for `secs` (4 ms a loop, later under load), or
    /// ten times that on the clock.
    fn until(c: &mut Connection, secs: f64, what: &str, ok: impl Fn(&Snapshot) -> bool) {
        let loops = (secs * 250.) as u64;
        let cap = Instant::now() + Duration::from_secs_f64(secs * 10.).max(Duration::from_secs(60));
        let (mut seen, mut last) = (0, None);
        loop {
            c.refresh();
            if ok(&c.snapshot) {
                return;
            }
            if let Some(timer) = c.snapshot.status.map(|st| st.timer) {
                seen += u64::from(timer.wrapping_sub(last.unwrap_or(timer)));
                last = Some(timer);
            }
            assert!(
                seen <= loops && Instant::now() < cap,
                "timed out waiting for {what}: {:?} {:?}",
                c.snapshot.state,
                c.snapshot.log
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// The driver ends a jog itself at a soft limit and goes Idle a few of
    /// its loops later; until then its snapshot still says Jog, with the
    /// limit logged. A tick that read such a snapshot and acts on it once
    /// the driver is Idle must not send the jog again: the driver would
    /// start a new one. A new press jogs again.
    #[test]
    fn a_jog_the_driver_ended_is_not_restarted_by_the_keepalive() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("limited.toml");
        std::fs::write(&profile, LIMITED).unwrap();
        let mut m = Machine::new(3);
        m.p.insert("controller".into(), "UC100".into());
        m.p.insert("uc100_profile".into(), profile.display().to_string());
        m.p.insert("uc100_connection".into(), "Simulated".into());
        let mut c = Connection::open(&m, dir.path()).unwrap();
        until(&mut c, 3.0, "E-stop after connecting", |s| {
            s.state == Some(State::EStop)
        });
        c.send(Command::Reset);
        until(&mut c, 3.0, "Idle after Reset", |s| {
            s.state == Some(State::Idle)
        });
        c.act(Action::MachineZero(None));
        until(&mut c, 3.0, "X, Y and Z set", |s| {
            s.referenced[..3].iter().all(|r| *r)
        });
        c.send(Command::Mdi("G0 X3".into()));
        until(&mut c, 5.0, "X at 3", |s| {
            s.state == Some(State::Idle) && (s.machine_pos[0] - 3.).abs() < 1e-3
        });
        let hits = |s: &Snapshot| s.log.iter().filter(|l| *l == "Soft limit reached").count();
        c.act(Action::JogPress(0, true));
        let pressed = c.held.as_ref().expect("held").log_seq;
        until(&mut c, 3.0, "the jog to end at the limit", |s| {
            s.state == Some(State::Idle) && hits(s) == 1
        });
        assert_eq!(c.held(), None);
        assert!(
            c.snapshot.machine_pos[0].abs() < 0.05,
            "{:?}",
            c.snapshot.machine_pos
        );
        // The snapshot the driver published while it stopped the jog, read
        // by a tick with the key still held and the resend due.
        c.snapshot.state = Some(State::Jog);
        c.held = Some(Held {
            axis: 0,
            negative: true,
            sent: Instant::now() - JOG_RESEND,
            started: true,
            log_seq: pressed,
        });
        c.observe();
        // Nothing starts over the driver's next 75 loops (300 ms).
        let (seq, from) = (
            c.snapshot.log_seq,
            c.snapshot.status.map_or(0, |st| st.timer),
        );
        until(&mut c, 3.0, "75 driver loops", |s| {
            assert_eq!(s.state, Some(State::Idle), "{:?}", s.log);
            assert_eq!(s.log_seq, seq, "the jog was sent again: {:?}", s.log);
            s.status.is_some_and(|st| st.timer.wrapping_sub(from) >= 75)
        });
        assert_eq!((c.held(), hits(&c.snapshot)), (None, 1));
        c.act(Action::JogPress(0, false));
        until(&mut c, 3.0, "jogging +X", |s| {
            s.state == Some(State::Jog) && s.machine_pos[0] > 0.5
        });
        c.act(Action::JogRelease);
        until(&mut c, 3.0, "Idle after the release", |s| {
            s.state == Some(State::Idle)
        });
    }

    #[test]
    fn a_tab_into_a_field_selects_its_value() {
        use iced::advanced::widget::operation::{
            Focusable, Operation, Outcome, TextInput, focusable,
        };
        #[derive(Default)]
        struct Field {
            focused: bool,
            selected: bool,
        }
        impl Focusable for Field {
            fn is_focused(&self) -> bool {
                self.focused
            }
            // Like iced's text field, which puts the cursor at the end.
            fn focus(&mut self) {
                (self.focused, self.selected) = (true, false)
            }
            fn unfocus(&mut self) {
                (self.focused, self.selected) = (false, false)
            }
        }
        impl TextInput for Field {
            fn text(&self) -> &str {
                "5"
            }
            fn move_cursor_to_front(&mut self) {}
            fn move_cursor_to_end(&mut self) {}
            fn move_cursor_to(&mut self, _: usize) {}
            fn select_all(&mut self) {
                self.selected = true
            }
            fn select_range(&mut self, _: usize, _: usize) {}
        }
        // An operation and those it chains to, over the fields offered in
        // order the way a text field offers itself.
        fn run(mut op: Box<dyn Operation>, fields: &mut [Field]) {
            loop {
                op.traverse(&mut |op| {
                    for f in fields.iter_mut() {
                        op.text_input(None, iced::Rectangle::default(), f);
                        op.focusable(None, iced::Rectangle::default(), f);
                    }
                });
                match op.finish() {
                    Outcome::Chain(next) => op = next,
                    _ => return,
                }
            }
        }
        let state = |f: &[Field]| {
            f.iter()
                .map(|f| (f.focused, f.selected))
                .collect::<Vec<_>>()
        };
        let mut fields: Vec<Field> = (0..3).map(|_| Field::default()).collect();
        fields[0].focused = true;
        run(Box::new(focusable::focus_next()), &mut fields);
        run(Box::new(select_focused()), &mut fields);
        assert_eq!(
            state(&fields),
            [(false, false), (true, true), (false, false)]
        );
        run(Box::new(focusable::focus_previous()), &mut fields);
        run(Box::new(select_focused()), &mut fields);
        assert_eq!(
            state(&fields),
            [(true, true), (false, false), (false, false)]
        );
    }

    fn uc100(profile: &str, connection: &str) -> Machine {
        let mut m = Machine::new(3);
        m.p.insert("controller".into(), "UC100".into());
        m.p.insert("uc100_profile".into(), profile.into());
        m.p.insert("uc100_connection".into(), connection.into());
        m
    }

    #[test]
    fn the_owners_machine_runs_on_the_profile_it_writes_and_keeps_an_edited_one() {
        let dir = tempfile::tempdir().unwrap();
        let m = owners_machine(&dir.path().join("AXIS CAM Studio")).unwrap();
        assert!(m.issues().is_empty(), "{:?}", m.issues());
        assert!(crate::post::PostSettings::for_uc100(&m).is_ok());
        // The library's travel is the span of the profile's soft limits.
        let config = profile(&m).unwrap();
        for (a, key) in ["x", "y", "z"].into_iter().enumerate() {
            let axis = config.axes.get(a).unwrap();
            let span = axis.soft_max.unwrap() - axis.soft_min.unwrap();
            assert_eq!(span.to_string(), get(&m.p, key));
        }
        assert!(simulated(&m) != cfg!(target_os = "macos"));
        let path = dir.path().join("AXIS CAM Studio").join("jy5800.toml");
        std::fs::write(&path, "edited in Setup").unwrap();
        owners_machine(&dir.path().join("AXIS CAM Studio")).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "edited in Setup");
    }

    #[test]
    fn the_input_log_keeps_the_last_edges() {
        let mut log = InputLog::default();
        let mut st = Status {
            inputs: 0x1f,
            ..Default::default()
        };
        log.observe(Duration::ZERO, &st);
        assert!(log.edges.is_empty());
        st.inputs = 0x1e;
        log.observe(Duration::from_millis(1500), &st);
        assert_eq!(log.edges.back(), Some(&(1.5, 10, false)));
        assert_eq!((log.changes(10), log.changes(15)), (1, 0));
        for _ in 0..30 {
            st.inputs ^= 1;
            log.observe(Duration::from_secs(2), &st);
        }
        assert_eq!(log.edges.len(), 20);
        assert_eq!(log.changes(10), 31);
    }

    #[test]
    fn typing_finds_a_focused_text_field() {
        use iced::advanced::widget::{
            Operation,
            operation::{Focusable, Outcome},
        };
        struct Field(bool);
        impl Focusable for Field {
            fn is_focused(&self) -> bool {
                self.0
            }
            fn focus(&mut self) {}
            fn unfocus(&mut self) {}
        }
        let answer = |fields: &mut [Field]| {
            let mut op = typing();
            for f in fields {
                op.focusable(None, iced::Rectangle::default(), f);
            }
            matches!(op.finish(), Outcome::Some(true))
        };
        assert!(!answer(&mut [Field(false), Field(false)]));
        assert!(answer(&mut [Field(false), Field(true)]));
    }

    #[test]
    fn probe_settings_leave_the_plate_and_tip_to_measure_and_need_sane_values() {
        let mut m = Machine::new(3);
        let p = probe_settings(&m.p).unwrap();
        assert_eq!((p.plate, p.tip), (None, None));
        m.p.insert("probe_plate".into(), "-1".into());
        m.p.insert("probe_tip".into(), "x".into());
        assert_eq!(
            probe_settings(&m.p).unwrap_err(),
            vec![
                "Touch plate thickness must be from 0 to 100 mm.".to_string(),
                "Probe tip diameter must be from 0 to 50 mm.".to_string()
            ]
        );
        m.p.insert("probe_plate".into(), "15.2".into());
        m.p.insert("probe_tip".into(), "2".into());
        let p = probe_settings(&m.p).unwrap();
        assert_eq!(
            (p.plate, p.tip, p.feed, p.distance, p.retract),
            (Some(15.2), Some(2.), 100., 20., 5.)
        );
        // Machines saved before the probe settings get the same defaults.
        for k in ["probe_feed", "probe_distance", "probe_retract"] {
            m.p.remove(k);
        }
        assert_eq!(probe_settings(&m.p), Ok(p));
        for (k, v) in [
            ("probe_feed", "0"),
            ("probe_distance", "NaN"),
            ("probe_retract", "-1"),
        ] {
            m.p.insert(k.into(), v.into());
        }
        assert_eq!(probe_settings(&m.p).unwrap_err().len(), 3);
    }

    #[test]
    fn work_z_puts_the_contact_where_it_belongs() {
        // Contact at machine Z -40, stopped 0.05 mm past it: the plate's top
        // (15 mm) at the contact makes the stop work Z 14.95.
        let z = work_z(15., -40., -40.05);
        assert!((z - 14.95).abs() < 1e-9);
        // The work offset that follows puts the contact at work Z 15.
        let offset = -40.05 - z;
        assert!((-40. - offset - 15.).abs() < 1e-9);
        assert_eq!(work_z(-3.5, 10., 10.), -3.5);
    }

    #[test]
    fn the_simulated_block_sits_on_the_middle_of_the_table() {
        let mut cfg = MachineConfig::from_toml(uc100::config::TEMPLATE).unwrap();
        for (a, (lo, hi)) in [(0., 390.), (0., 80.), (-55., 0.)].into_iter().enumerate() {
            let x = cfg.axes.get_mut(a).unwrap();
            (x.soft_min, x.soft_max) = (Some(lo), Some(hi));
        }
        let (lo, hi) = sim_block(&cfg, None).unwrap();
        assert_eq!((lo, hi), ([145., 20., -55.], [245., 60., -27.5]));
        // A setup's stock gives it its X/Y size.
        let (lo, hi) = sim_block(&cfg, Some([124., 30., 47.])).unwrap();
        assert_eq!((lo, hi), ([133., 25., -55.], [257., 55., -27.5]));
        cfg.axes.get_mut(1).unwrap().soft_min = None;
        assert_eq!(sim_block(&cfg, None), None);
    }

    /// Runs a probing routine against a box at machine `lo`…`hi`: each touch
    /// stops where the tip, `e.radius` round, meets a face.
    fn touch_box(mut e: Edges, lo: [f64; 3], hi: [f64; 3]) -> Edges {
        let mut at = e.start;
        while let Some(i) = e.next() {
            for step in e.plan(i) {
                match step {
                    Step::Go(moves, guard) => {
                        if let Some(down) = guard {
                            let clear = (0..2).any(|a| down[a] < lo[a] || down[a] > hi[a]);
                            assert!(clear, "going down onto the box at {down:?}");
                        }
                        at = guard.or(moves.last().copied()).unwrap();
                    }
                    Step::Touch(i, to) => {
                        let f = &e.faces[i];
                        let k = f.axis;
                        let mut p = to;
                        p[k] = match (k, f.dir > 0.) {
                            (2, _) => hi[2],
                            (_, true) => lo[k] - e.radius,
                            (_, false) => hi[k] + e.radius,
                        };
                        // It moves toward the face and reaches it before its target.
                        assert!((p[k] - at[k]) * f.dir > 0. && (to[k] - p[k]) * f.dir > 0.);
                        e.from = at;
                        e.touches[i].push(p);
                        at = p;
                    }
                }
            }
        }
        e
    }

    #[test]
    fn probing_touches_each_face_at_its_spot_and_finds_the_zero() {
        use crate::{mesh::Bounds, post::At};
        let stock = Bounds {
            min: [0.; 3],
            max: [124., 84., 40.],
        };
        let probe = Probe {
            plate: None,
            tip: Some(4.),
            feed: 100.,
            distance: 20.,
            retract: 5.,
        };
        let z = (-80., 0.);
        // The stock at machine X 100…224, Y 50…134, Z -70…-30.
        let (lo, hi) = ([100., 50., -70.], [224., 134., -30.]);
        let check = |e: &Edges, zero: [f64; 3]| {
            let (w, fixed) = e.zero();
            assert_eq!(fixed, [true; 3]);
            assert!((0..3).all(|a| (w[a] - zero[a]).abs() < 1e-9), "{w:?}");
            for (f, t) in e.faces.iter().zip(&e.touches) {
                let p = t.last().unwrap();
                for a in (0..3).filter(|&a| a != f.axis) {
                    let v = p[a] - w[a] + if a == 2 { e.radius } else { 0. };
                    assert!(
                        (f.lo[a] - 1e-6..=f.hi[a] + 1e-6).contains(&v),
                        "{} touched at {a} {v}, outside {}…{}",
                        f.name,
                        f.lo[a],
                        f.hi[a]
                    );
                }
            }
        };
        // Corner: started 2 mm in from both edges, inside the top's spot (1
        // to 11 mm), but the -X side's spot is 4.2 to 12.6 mm from the
        // corner: it is touched again there once -Y has fixed Y.
        let start = [102., 52., -25., 0., 0., 0.];
        let e = Edges::new(
            "Stock top front-left",
            [At::Min; 2],
            stock,
            [0., 0., 40.],
            &probe,
            4.,
            start,
            z,
        );
        let e = touch_box(e, lo, hi);
        assert_eq!(
            e.touches.iter().map(Vec::len).collect::<Vec<_>>(),
            [1, 2, 1]
        );
        check(&e, [100., 50., -30.]);
        assert!(e.found().contains("front-left corner of the stock top at machine X 100.000 Y 50.000 Z -30.000, from 4 touches"), "{}", e.found());
        // Centre: started 3 mm off it, inside the top's spot: once each.
        let start = [165., 90., -15., 0., 0., 0.];
        let e = Edges::new(
            "Stock top center",
            [At::Mid; 2],
            stock,
            [62., 42., 40.],
            &probe,
            4.,
            start,
            z,
        );
        let e = touch_box(e, lo, hi);
        assert_eq!(e.touches.iter().map(Vec::len).collect::<Vec<_>>(), [1; 5]);
        check(&e, [162., 92., -30.]);
        assert!(
            e.found().contains("Measured 124.000 × 84.000 mm across"),
            "{}",
            e.found()
        );
        // Off the top's spot (15 mm from the corner): the top is touched
        // again on it.
        let start = [115., 65., -25., 0., 0., 0.];
        let e = Edges::new(
            "Stock top front-left",
            [At::Min; 2],
            stock,
            [0., 0., 40.],
            &probe,
            4.,
            start,
            z,
        );
        let e = touch_box(e, lo, hi);
        assert_eq!(e.touches[0].len(), 2);
        check(&e, [100., 50., -30.]);
    }

    /// Runs a probing routine against the material `solid` says is there
    /// (machine coordinates being the setup's, mm): a guarded descent must
    /// meet nothing, and each touch stops where the tip, `e.radius` round,
    /// first meets material on its way (its bottom going down, its equator
    /// going sideways), to 1e-5 mm.
    fn touch_material(mut e: Edges, solid: &dyn Fn([f64; 3]) -> bool) -> Edges {
        let first = |from: Vec6, to: Vec6, lead: &dyn Fn(Vec6) -> [f64; 3]| {
            let at = |t: f64| -> Vec6 { std::array::from_fn(|a| from[a] + (to[a] - from[a]) * t) };
            let steps = 4000;
            let k = (1..=steps).find(|&k| solid(lead(at(k as f64 / steps as f64))))?;
            let (mut lo, mut hi) = ((k - 1) as f64 / steps as f64, k as f64 / steps as f64);
            while (hi - lo) * 400. > 1e-5 {
                let mid = (lo + hi) / 2.;
                if solid(lead(at(mid))) {
                    hi = mid
                } else {
                    lo = mid
                }
            }
            Some(at(hi))
        };
        let mut at = e.start;
        while let Some(i) = e.next() {
            for step in e.plan(i) {
                match step {
                    Step::Go(moves, guard) => {
                        if let Some(down) = guard {
                            let from = *moves.last().unwrap();
                            let landed = first(from, down, &|p| [p[0], p[1], p[2]]);
                            assert!(landed.is_none(), "going down onto the stock at {landed:?}");
                        }
                        at = guard.or(moves.last().copied()).unwrap();
                    }
                    Step::Touch(i, to) => {
                        let (k, dir, r) = (e.faces[i].axis, e.faces[i].dir, e.radius);
                        let lead = |p: Vec6| {
                            let mut q = [p[0], p[1], p[2]];
                            if k < 2 {
                                (q[k], q[2]) = (q[k] + dir * r, q[2] + r);
                            }
                            q
                        };
                        let p = first(at, to, &lead)
                            .unwrap_or_else(|| panic!("missed the {}", e.faces[i].name));
                        e.from = at;
                        e.touches[i].push(p);
                        at = p;
                    }
                }
            }
        }
        e
    }

    /// For every work zero, on the raw stock of setup 1, and on the stock a
    /// setup turned about Z and one flipped over start from after setup 1
    /// faced and pocketed it: the probe, run against that material (the
    /// original block less what earlier setups can have cut), sets the work
    /// zero where the program has it (`post::datum`) within 0.01 mm, unless
    /// the check refuses the zero because a spot it is found from was cut.
    #[test]
    fn the_probe_sets_the_programs_zero_on_every_stock_state() {
        use crate::post::{ORIGINS, datum, fixture, probe_faces, zero_check};
        let probe = Probe {
            plate: None,
            tip: Some(4.),
            feed: 100.,
            distance: 30.,
            retract: 5.,
        };
        let raw = fixture::flipped_after(2);
        let mut turned = fixture::flipped_after(2);
        turned.0.setups[1].p.insert("rx".into(), "0".into());
        turned.0.setups[1].p.insert("rz".into(), "90".into());
        // Faced on the top, as setup 1 is.
        turned.0.setups[1].operations[0].surfaces =
            turned.0.setups[0].operations[0].surfaces.clone();
        let mut refused = vec![];
        for (state, (mut doc, mesh), si) in [
            ("raw", raw.clone(), 0),
            ("turned", turned, 1),
            ("flipped", raw, 1),
        ] {
            doc.sync_stock_sources();
            for origin in ORIGINS {
                doc.setups[si].p.insert("origin".into(), origin.into());
                let mut first = None;
                crate::simulation::sequence(&doc, &mesh, None, |s, _, r| {
                    if s == si && first.is_none() {
                        first = Some(r);
                    }
                });
                let first = first.unwrap().unwrap_or_else(|e| panic!("{state}: {e}"));
                let (setup, stock) = (&doc.setups[si], first.plan.stock);
                let found = zero_check(setup, stock, &first, &[]);
                if !found.refused.is_empty() {
                    refused.push(format!("{state} {origin}"));
                    continue;
                }
                let refs = found.probe.unwrap();
                let (volume, frame) =
                    (first.initial_volume.as_ref().unwrap(), first.frame.unwrap());
                let block = volume.transformed_bounds(frame);
                let removed = crate::rest::Removed::new(volume, frame);
                let solid = |p: [f64; 3]| {
                    let q = p.map(|v| v as f32);
                    (0..3).all(|a| q[a] >= block.min[a] && q[a] <= block.max[a]) && !removed.at(q)
                };
                // Started 5 mm above the middle of the top's spot.
                let spot = probe_faces(stock, refs)[0].3;
                let start = [
                    ((spot.min[0] + spot.max[0]) / 2.) as f64,
                    ((spot.min[1] + spot.max[1]) / 2.) as f64,
                    (stock.max[2] + 5.) as f64,
                    0.,
                    0.,
                    0.,
                ];
                let zero = datum(setup, stock, &mesh).unwrap();
                let z = (stock.min[2] as f64 - 100., stock.max[2] as f64 + 100.);
                let e = Edges::new(origin, refs, stock, zero, &probe, 4., start, z);
                let (w, fixed) = touch_material(e, &solid).zero();
                assert_eq!(fixed, [true; 3], "{state} {origin}");
                assert!(
                    (0..3).all(|a| (w[a] - zero[a] as f64).abs() < 0.01),
                    "{state} {origin}: the probe set {w:?}, the program has {zero:?}"
                );
            }
        }
        // Setup 1's pocket is in the middle of its top: the top's centre
        // where a centre zero is touched off once turned, the bottom under
        // it once flipped over.
        assert_eq!(
            refused,
            [
                "turned Stock top center",
                "turned Stock bottom center",
                "flipped Stock bottom center"
            ]
        );
    }

    #[test]
    fn connection_defaults_to_the_simulator_except_on_macos() {
        assert!(simulated(&uc100("", "Simulated")));
        assert!(!simulated(&uc100("", "USB")));
        assert_eq!(simulated(&uc100("", "")), !cfg!(target_os = "macos"));
        assert_eq!(connection_label(&uc100("", "Simulated")), "Simulated");
        assert_eq!(connection_label(&uc100("", "USB")), "USB");
    }

    #[test]
    fn the_profile_is_loaded_and_checked_with_the_driver() {
        let dir = tempfile::tempdir().unwrap();
        assert!(profile(&uc100("", "")).unwrap_err().contains("Choose"));
        let good = dir.path().join("machine.toml");
        std::fs::write(&good, uc100::config::TEMPLATE).unwrap();
        assert!(profile(&uc100(good.to_str().unwrap(), "")).is_ok());
        let bad = dir.path().join("bad.toml");
        std::fs::write(&bad, "[controller]\nnonsense = 1\n").unwrap();
        let e = profile(&uc100(bad.to_str().unwrap(), "")).unwrap_err();
        assert!(e.contains("invalid config"), "{e}");
        let missing = dir.path().join("missing.toml");
        let e = profile(&uc100(missing.to_str().unwrap(), "")).unwrap_err();
        assert!(e.contains("cannot read"), "{e}");
    }
}
