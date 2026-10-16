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

/// The spindle cap of each profile file read so far, by path and the file's
/// modification time: a view asks for it on every redraw, and the file is
/// read and parsed again only when it has changed.
static CAPS: std::sync::Mutex<BTreeMap<std::path::PathBuf, (std::time::SystemTime, Capped)>> =
    std::sync::Mutex::new(BTreeMap::new());
type Capped = Result<Option<f32>, String>;
#[cfg(test)]
thread_local! {
    /// How many times this thread has read a profile for its cap.
    static CAP_READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The spindle speed a UC100 machine's profile caps the machine's rpm to, as a
/// run checks: its maximum at full PWM. None on any other machine, or when the
/// profile has no spindle PWM output. The profile is read once per path and
/// modification time (`CAPS`): this is called from view code.
pub fn spindle_cap(m: &Machine) -> Result<Option<f32>, String> {
    if get(&m.p, "controller") != "UC100" {
        return Ok(None);
    }
    let read = |m: &Machine| {
        #[cfg(test)]
        CAP_READS.with(|n| n.set(n.get() + 1));
        let out = profile(m)?.outputs;
        Ok(out
            .spindle_pwm
            .is_some()
            .then_some(out.spindle_max_rpm as f32))
    };
    let path = Path::new(get(&m.p, "uc100_profile").trim());
    // A profile whose time cannot be read (missing, no path) is not kept: it
    // is read, and refused, as before.
    let Some(modified) = std::fs::metadata(path)
        .and_then(|f| f.modified())
        .ok()
        .filter(|_| !path.as_os_str().is_empty())
    else {
        return read(m);
    };
    let mut caps = CAPS.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((time, cap)) = caps.get(path)
        && *time == modified
    {
        return cap.clone();
    }
    let cap = read(m);
    caps.insert(path.to_path_buf(), (modified, cap.clone()));
    cap
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

/// A machine's fixture pins (Machine library): dowel pins pressed into a
/// plate on the table, which a part drops over by holes in it, so its place
/// and turn are known without probing it (mm). The plate is square to the
/// machine's X and Y.
#[derive(Debug, Clone, PartialEq)]
pub struct Pins {
    /// Each pin's X Y on the plate as drilled, pin 1 (the reference pin)
    /// first.
    pub at: Vec<[f64; 2]>,
    pub diameter: f64,
    /// How far a pin stands above the plate.
    pub height: f64,
    /// Pin 1's machine X Y, recorded on the Control page (Record the pins'
    /// place); None until then. Never typed.
    pub reference: Option<[f64; 2]>,
}

impl Pins {
    /// Where pin `i` stands from pin 1.
    pub fn offset(&self, i: usize) -> [f64; 2] {
        [self.at[i][0] - self.at[0][0], self.at[i][1] - self.at[0][1]]
    }

    /// The same pins, wherever the place of pin 1 was recorded.
    pub fn same_pins(&self, other: &Pins) -> bool {
        self.at == other.at && self.diameter == other.diameter && self.height == other.height
    }
}

/// The machine's fixture pins, None when it has none, or what is wrong with
/// them. The diameter and the height are measured: they have no default.
pub fn pins_settings(p: &Fields) -> Result<Option<Pins>, Vec<String>> {
    let text = get(p, "pins").trim();
    if text.is_empty() {
        return Ok(None);
    }
    let mut e = vec![];
    let at: Option<Vec<[f64; 2]>> = (text.split(';'))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|pin| {
            let v: Vec<f64> = (pin.split([' ', ',', '\t']))
                .filter(|s| !s.is_empty())
                .map(|s| s.parse::<f64>().ok().filter(|v| v.is_finite()))
                .collect::<Option<_>>()?;
            (v.len() == 2).then(|| [v[0], v[1]])
        })
        .collect();
    let at = at.unwrap_or_else(|| {
        e.push(
            "Fixture pins must be X Y pairs separated by semicolons, such as \"0 0; 100 0\"."
                .into(),
        );
        vec![]
    });
    if !at.is_empty() && at.len() < 2 {
        e.push(
            "Fixture pins need at least two pins: one places the part, the second its turn.".into(),
        );
    }
    let mut value = |key, what: &str, lo: f64, hi: f64| {
        let text = get(p, key).trim();
        match text.parse::<f64>() {
            Ok(v) if (lo..=hi).contains(&v) => v,
            _ if text.is_empty() => {
                e.push(format!("{what} is not set: measure the pins and set it."));
                0.
            }
            _ => {
                e.push(format!("{what} must be from {lo} to {hi} mm."));
                0.
            }
        }
    };
    let diameter = value("pin_diameter", "Pin diameter", 1., 50.);
    let height = value("pin_height", "Pin height", 1., 100.);
    for i in 0..at.len() {
        for j in i + 1..at.len() {
            let d = (at[i][0] - at[j][0]).hypot(at[i][1] - at[j][1]);
            if diameter > 0. && d < diameter {
                e.push(format!(
                    "Pins {} and {} are closer than their diameter.",
                    i + 1,
                    j + 1
                ));
            }
        }
    }
    let reference = match ["pin_ref_x", "pin_ref_y"].map(|k| get(p, k).trim()) {
        ["", ""] => None,
        [x, y] => {
            match (x.parse::<f64>(), y.parse::<f64>()) {
                (Ok(x), Ok(y)) if x.is_finite() && y.is_finite() => Some([x, y]),
                _ => {
                    e.push("The pins' recorded place does not read: record it again on the Control page.".into());
                    None
                }
            }
        }
    };
    if e.is_empty() {
        Ok(Some(Pins {
            at,
            diameter,
            height,
            reference,
        }))
    } else {
        Err(e)
    }
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
    /// The setup's work zero (`post::ORIGINS`), for a message.
    origin: String,
    /// `refs` are not the zero's own spot: the zero is placed from the
    /// faces touched by the setup's stock size.
    moved: bool,
    faces: Vec<Face>,
    /// The setup's stock (mm).
    size: [f64; 3],
    /// How far a face may be from where it is expected (mm): how far outside
    /// a side the probe goes down, how far past where the side should be it
    /// probes, and how high above the top, or the tallest jaw or clamp, it
    /// travels.
    clear: f64,
    /// The setup's jaws or clamps from the work zero (mm): their names,
    /// min and max corners.
    holds: Vec<(String, [f64; 3], [f64; 3])>,
    /// How far above the stock top the probe travels between faces (mm):
    /// `clear`, or more when one of `holds` stands higher than the top
    /// (`post::probe_height`, #163).
    rise: f64,
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
    /// on its `stock` at `datum` (plan mm), held by `holds`, the probe with
    /// a tip of `tip` mm starting at `start`.
    #[allow(clippy::too_many_arguments)]
    fn new(
        origin: &str,
        refs: [crate::post::At; 2],
        stock: crate::mesh::Bounds,
        datum: [f32; 3],
        holds: &[crate::holding::HoldBox],
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
        let corner = |b: [f32; 3]| std::array::from_fn(|a| rel(b[a], a));
        Self {
            refs,
            bottom: crate::post::stock_zero(origin).is_some_and(|(_, top)| !top),
            model: crate::post::stock_zero(origin).is_none(),
            origin: origin.into(),
            moved: crate::post::stock_zero(origin).is_some_and(|(own, _)| own != refs),
            touches: vec![vec![]; faces.len()],
            faces,
            size: stock.dimensions().map(f64::from),
            // 10 mm, a quarter of the shorter top side when less.
            clear: crate::post::corner_area(stock) as f64,
            rise: (crate::post::probe_height(stock, holds) - stock.max[2]) as f64,
            holds: holds
                .iter()
                .map(|h| (h.label.clone(), corner(h.bounds.min), corner(h.bounds.max)))
                .collect(),
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
        let high = self.high(w);
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

    /// The machine Z the probe travels at between faces, given the work
    /// zero `w` (machine): `rise` above the stock top, or the top of Z.
    fn high(&self, w: [f64; 3]) -> f64 {
        (self.top(w) + self.rise).min(self.z.1)
    }

    /// Why the traverse `moves` to face `i` (`plan`) must not be made: the
    /// top of Z holds it so low that its level moves would pass within a
    /// millimetre over one of `holds`, the tip's radius allowed for
    /// sideways. At `rise` above the top the traverse clears every one by
    /// `clear`; only the top of Z can bound it lower (#163).
    fn blocked(&self, i: usize, moves: &[Vec6]) -> Option<String> {
        let (w, _) = self.zero();
        let r = self.radius;
        for pair in moves.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            if (a[2] - b[2]).abs() > 1e-9 {
                continue;
            }
            for (label, lo, hi) in &self.holds {
                let top = w[2] + hi[2];
                let (lo, hi) = (
                    [w[0] + lo[0] - r, w[1] + lo[1] - r],
                    [w[0] + hi[0] + r, w[1] + hi[1] + r],
                );
                if top + 1. > a[2]
                    && crate::post::segment_crosses([a[0], a[1]], [b[0], b[1]], lo, hi)
                {
                    return Some(format!(
                        "The probe cannot pass over the {label}, whose top is at machine Z {top:.3}, on its way to the {}: Z ends at {:.3}. Touch the work zero off by hand.",
                        self.faces[i].name, self.z.1
                    ));
                }
            }
        }
        None
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
        } else if self.moved {
            format!(
                "Work zero set on {} {at}, placed from {spot} of the stock top by the setup's {:.3} × {:.3} × {:.3} mm stock, from {count} touches: a stock of another size moves it by the difference.",
                self.origin, self.size[0], self.size[1], self.size[2]
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

/// How far a corner probe steps along each side from where it starts
/// before it probes the other side (mm): it starts outside the corner, less
/// than this off both sides.
pub const CORNER_REACH: f64 = 15.;

/// A feature the probe finds without a setup, from where the operator put
/// it, setting work zero there in the active work system, as a milling
/// machine's probing screen does. The tip's radius is allowed for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Feature {
    /// Straight down onto a surface: Z0 there.
    Top,
    /// Along axis `.0` (X or Y), toward + when `.1`, onto a side: that
    /// axis's zero on it. The tip starts beside the side, below its top.
    Edge(usize, bool),
    /// An outside corner (`post::At::Min` or `Max` along X and Y): X0 Y0
    /// on it. The tip starts outside it, below the top, less than
    /// `CORNER_REACH` off both sides.
    Corner([crate::post::At; 2]),
    /// The centre of a bore or pocket: X0 Y0 there. The tip starts inside
    /// it, below its top.
    Bore,
    /// The centre of a boss or pin: X0 Y0 there. The tip starts over it,
    /// above its top; the side is less than the maximum probe distance from
    /// there.
    Boss,
}

impl Feature {
    pub const EDGES: [Feature; 4] = [
        Feature::Edge(0, true),
        Feature::Edge(0, false),
        Feature::Edge(1, true),
        Feature::Edge(1, false),
    ];
    pub const CORNERS: [Feature; 4] = {
        use crate::post::At::{Max, Min};
        [
            Feature::Corner([Min, Max]),
            Feature::Corner([Max, Max]),
            Feature::Corner([Min, Min]),
            Feature::Corner([Max, Min]),
        ]
    };

    /// The button's words: "Z top", "X+", "Front-left", "Bore centre".
    pub fn label(self) -> String {
        match self {
            Feature::Top => "Z top".into(),
            Feature::Edge(k, plus) => format!("{}{}", ["X", "Y"][k], if plus { "+" } else { "−" }),
            Feature::Corner(c) => {
                let s = crate::post::corner_name(c);
                s[..1].to_uppercase() + &s[1..]
            }
            Feature::Bore => "Bore centre".into(),
            Feature::Boss => "Boss centre".into(),
        }
    }

    /// The axes it moves and sets.
    fn axes(self) -> &'static [usize] {
        match self {
            Feature::Top => &[2],
            Feature::Edge(0, _) => &[0],
            Feature::Edge(..) => &[1],
            Feature::Corner(_) | Feature::Bore | Feature::Boss => &[0, 1],
        }
    }

    /// The axes it moves: those it sets, and Z over a boss.
    fn moves(self) -> &'static [usize] {
        match self {
            Feature::Boss => &[0, 1, 2],
            f => f.axes(),
        }
    }

    /// Where to put the tip before pressing it, and what it does.
    pub fn how(self) -> String {
        match self {
            Feature::Top => "Jog the tip over the surface and press Z top: it probes straight down and sets Z0 where the tip's bottom touches. A 3D probe is not as long as the cutter: touch the cutter off on Z0 after it, with Probe Z or Set Z.".into(),
            Feature::Edge(k, plus) => {
                let side = crate::post::ENDS[k][usize::from(!plus)];
                format!(
                    "Jog the tip beside the {side} side of the part, below its top, and press {}: it probes toward {} and sets {}0 on the side.",
                    self.label(),
                    if plus { "+" } else { "−" },
                    ["X", "Y"][k]
                )
            }
            Feature::Corner(c) => format!(
                "Jog the tip outside the {} corner, below the top, less than {CORNER_REACH:.0} mm off both sides, and press {}: it steps along each side, probes the other and sets X0 Y0 on the corner.",
                crate::post::corner_name(c),
                self.label()
            ),
            Feature::Bore => "Jog the tip into the bore or pocket, below its top, and press Bore centre: it probes −X, +X at the middle found, −Y and +Y, sets X0 Y0 at the centre and stays there.".into(),
            Feature::Boss => format!(
                "Jog the tip over the middle of the boss or pin, above its top, and press Boss centre: it touches the top, goes out by the maximum probe distance and probes in from −X and +X {BOSS_DEPTH:.0} mm below the top, then from −Y and +Y at the middle found, sets X0 Y0 at the centre and returns over it."
            ),
        }
    }
}

/// How far below its top Boss centre probes a boss's sides (mm).
pub const BOSS_DEPTH: f64 = 3.;

/// The sides a boss is probed on, in order: the axis and the way the probe
/// moves to touch it (in from −X, +X, −Y and +Y).
const BOSS_SIDES: [(usize, f64); 4] = [(0, 1.), (0, -1.), (1, 1.), (1, -1.)];

/// Where to put the tip for Pin centre, with its button for `{}`.
const PIN_HOW: &str = "Take the part off the pins, jog the tip over the middle of pin 1, a few mm above its top, and press {} again.";

/// What a `Routine` is for.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Purpose {
    /// Set work zero on the feature, on the axes it sets.
    Work,
    /// Record the pins' place: pin 1, then pin 2 with pin 1's centre found.
    Record(Option<[f64; 2]>),
    /// Declare machine X and Y so that pin 1's centre is where it was
    /// recorded.
    Reference,
}

/// One driver command of a `Routine`.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Leg {
    /// A rapid move.
    Rapid(Vec6),
    /// A probe move that must touch nothing.
    Guard(Vec6),
    /// A probe move that must touch.
    Touch(Vec6),
}

/// A `Feature` probed from where the tip was put (machine mm).
struct Routine {
    feature: Feature,
    purpose: Purpose,
    start: Vec6,
    radius: f64,
    feed: f64,
    /// How far each touch goes at most.
    distance: f64,
    /// How far out from where it started a boss's sides are probed from.
    reach: f64,
    /// How far below a boss's top its sides are probed.
    depth: f64,
    /// The moves back over the feature (`last`) are made as legs: the
    /// routine ends once they are, for a purpose that acts from there.
    backed: bool,
    /// Where each touch was made, as the driver reports it.
    touches: Vec<Vec6>,
    legs: VecDeque<Leg>,
    /// The command under way and the counts before it (`Connection::seq`).
    sent: Option<(Leg, (u64, u64))>,
}

impl Routine {
    fn new(feature: Feature, probe: &Probe, start: Vec6) -> Self {
        Self {
            feature,
            purpose: Purpose::Work,
            start,
            radius: probe.tip.unwrap_or_default() / 2.,
            feed: probe.feed,
            distance: probe.distance,
            reach: probe.distance,
            depth: BOSS_DEPTH,
            backed: false,
            touches: vec![],
            legs: VecDeque::new(),
            sent: None,
        }
    }

    /// Pin centre: Boss centre on one of `pins`, going out past the pin's
    /// radius, the tip's and as much again (the tip lands on the pin's top
    /// less than both radii off its centre), and probing the sides
    /// min(3 mm, half the pin's height) below its top.
    fn pin(pins: &Pins, probe: &Probe, start: Vec6, purpose: Purpose) -> Self {
        let mut r = Self::new(Feature::Boss, probe, start);
        r.purpose = purpose;
        r.reach = 2. * (pins.diameter / 2. + r.radius) + 1.;
        r.depth = (pins.height / 2.).min(BOSS_DEPTH);
        r
    }

    /// Where to put the tip before pressing its button, and what it does.
    fn how(&self) -> String {
        match self.purpose {
            Purpose::Work => self.feature.how(),
            Purpose::Record(_) => PIN_HOW.replace("{}", "Record the pins' place"),
            Purpose::Reference => PIN_HOW.replace("{}", "Reference X Y from pin 1"),
        }
    }

    /// The probe settings it runs with.
    fn probe(&self) -> Probe {
        Probe {
            plate: None,
            tip: Some(self.radius * 2.),
            feed: self.feed,
            distance: self.distance,
            retract: 0.,
        }
    }

    /// `p` moved `d` along axis `k`.
    fn by(p: Vec6, k: usize, d: f64) -> Vec6 {
        let mut q = p;
        q[k] += d;
        q
    }

    /// Which way the corner probe moves along X and Y to reach the part.
    fn ways(c: [crate::post::At; 2]) -> [f64; 2] {
        c.map(|a| if a == crate::post::At::Min { 1. } else { -1. })
    }

    /// The bore's centre along X, from its first two touches; a boss's
    /// from the two after its top.
    fn mid_x(&self) -> f64 {
        let k = usize::from(self.feature == Feature::Boss);
        (self.touches[k][0] + self.touches[k + 1][0]) / 2.
    }

    /// Where a boss's side `n` (from 1, `BOSS_SIDES`) is probed toward, at
    /// the height the sides are probed at: the middle where the tip started,
    /// along X at the middle found for the Y sides.
    fn boss_mid(&self, n: usize) -> Vec6 {
        let mut c = self.start;
        if n > 2 {
            c[0] = self.mid_x();
        }
        c[2] = self.touches[0][2] - self.depth;
        c
    }

    /// Where the probe goes down beside a boss's side `n`, `reach` out
    /// from `boss_mid`, and above it at the height it started at.
    fn boss_out(&self, n: usize) -> [Vec6; 2] {
        let (k, way) = BOSS_SIDES[n - 1];
        let low = Self::by(self.boss_mid(n), k, -way * self.reach);
        let mut high = low;
        high[2] = self.start[2];
        [low, high]
    }

    /// The legs to the next touch, from the last one; None once every
    /// touch is made.
    fn plan(&self) -> Option<Vec<Leg>> {
        use Leg::{Guard, Rapid, Touch};
        let (s, d, n) = (self.start, self.distance, self.touches.len());
        Some(match (self.feature, n) {
            (Feature::Top, 0) => vec![Touch(Self::by(s, 2, -d))],
            (Feature::Edge(k, plus), 0) => vec![Touch(Self::by(s, k, if plus { d } else { -d }))],
            (Feature::Corner(c), 0) => {
                let w = Self::ways(c);
                let along = Self::by(s, 1, w[1] * CORNER_REACH);
                vec![Guard(along), Touch(Self::by(along, 0, w[0] * d))]
            }
            (Feature::Corner(c), 1) => {
                let w = Self::ways(c);
                let along = Self::by(s, 0, w[0] * CORNER_REACH);
                vec![
                    Rapid(Self::by(s, 1, w[1] * CORNER_REACH)),
                    Rapid(s),
                    Guard(along),
                    Touch(Self::by(along, 1, w[1] * d)),
                ]
            }
            (Feature::Bore, 0) => vec![Touch(Self::by(s, 0, -d))],
            (Feature::Bore, 1) => vec![Rapid(s), Touch(Self::by(s, 0, d))],
            (Feature::Bore, 2 | 3) => {
                let mut mid = s;
                mid[0] = self.mid_x();
                vec![
                    Rapid(mid),
                    Touch(Self::by(mid, 1, if n == 2 { -d } else { d })),
                ]
            }
            (Feature::Boss, 0) => vec![Touch(Self::by(s, 2, -d))],
            // Up off the top or out from the side before, over beside the
            // next side, down (a probe move: a boss wider than expected, or
            // anything beside it, stops it), and in toward the middle: it
            // touches before reaching it.
            (Feature::Boss, 1..=4) => {
                let mut legs = if n == 1 {
                    vec![Rapid(s)]
                } else {
                    self.boss_out(n - 1).map(Rapid).to_vec()
                };
                let [low, high] = self.boss_out(n);
                legs.extend([Rapid(high), Guard(low), Touch(self.boss_mid(n))]);
                legs
            }
            _ => return None,
        })
    }

    /// Where the routine leaves the tip once every touch is made: back where
    /// it started, or on the bore's centre.
    fn last(&self) -> Vec<Vec6> {
        match self.feature {
            Feature::Corner(c) => {
                let w = Self::ways(c);
                vec![Self::by(self.start, 0, w[0] * CORNER_REACH), self.start]
            }
            Feature::Bore => {
                let z = self.zero();
                let mut at = self.start;
                (at[0], at[1]) = (z[0], z[1]);
                vec![at]
            }
            Feature::Boss => {
                let z = self.zero();
                let mut over = self.start;
                (over[0], over[1]) = (z[0], z[1]);
                let mut out = self.boss_out(4).to_vec();
                out.push(over);
                out
            }
            Feature::Top => vec![self.start],
            Feature::Edge(..) => vec![self.start],
        }
    }

    /// The feature's machine position on the axes it sets (`Feature::axes`),
    /// once every touch is made: a side is a tip radius ahead of the tip's
    /// centre, a surface at the tip's bottom, a bore's centre midway between
    /// opposite touches.
    fn zero(&self) -> [f64; 3] {
        let (t, r) = (&self.touches, self.radius);
        let mut w = [self.start[0], self.start[1], self.start[2]];
        match self.feature {
            Feature::Top => w[2] = t[0][2],
            Feature::Edge(k, plus) => w[k] = t[0][k] + if plus { r } else { -r },
            Feature::Corner(c) => {
                let way = Self::ways(c);
                w[0] = t[0][0] + way[0] * r;
                w[1] = t[1][1] + way[1] * r;
            }
            Feature::Bore => {
                w[0] = self.mid_x();
                w[1] = (t[2][1] + t[3][1]) / 2.;
            }
            Feature::Boss => {
                w[0] = self.mid_x();
                w[1] = (t[3][1] + t[4][1]) / 2.;
                w[2] = t[0][2];
            }
        }
        w
    }

    /// A boss's size across axis `k` from its touches `a` and `b` on
    /// either side: the tip's centres less its diameter.
    fn boss_across(&self, a: usize, b: usize, k: usize) -> f64 {
        (self.touches[b][k] - self.touches[a][k]).abs() - 2. * self.radius
    }

    /// What was found and set, once every touch is made.
    fn found(&self) -> String {
        let w = self.zero();
        let at = |axes: &[usize]| {
            axes.iter()
                .map(|&a| format!("{} {:.3}", ["X", "Y", "Z"][a], w[a]))
                .collect::<Vec<_>>()
                .join(" ")
        };
        match self.feature {
            Feature::Top => format!("Z0 set on the surface at machine {}.", at(&[2])),
            Feature::Edge(k, plus) => format!(
                "{}0 set on the {} side at machine {}.",
                ["X", "Y"][k],
                crate::post::ENDS[k][usize::from(!plus)],
                at(&[k])
            ),
            Feature::Corner(c) => format!(
                "X0 Y0 set on the {} corner at machine {}.",
                crate::post::corner_name(c),
                at(&[0, 1])
            ),
            Feature::Bore => {
                let t = &self.touches;
                let across =
                    |a: usize, b: usize, k: usize| (t[b][k] - t[a][k]).abs() + 2. * self.radius;
                format!(
                    "X0 Y0 set on the bore's centre at machine {}: {:.3} mm across Y through it, {:.3} mm across X where the tip started.",
                    at(&[0, 1]),
                    across(2, 3, 1),
                    across(0, 1, 0)
                )
            }
            Feature::Boss => format!(
                "X0 Y0 set on the boss's centre at machine {}: {:.3} mm across Y through it, {:.3} mm across X where the tip started, its top at machine Z {:.3}.",
                at(&[0, 1]),
                self.boss_across(3, 4, 1),
                self.boss_across(1, 2, 0),
                w[2]
            ),
        }
    }
}

/// How square the plate is, from pin 1 and pin 2 found at machine X Y
/// `first` and `second`: whether pin 2 is within 0.05 mm of the line the
/// plate puts it on, and what is said of it.
pub fn squareness(pins: &Pins, first: [f64; 2], second: [f64; 2]) -> (bool, String) {
    let d = pins.offset(1);
    let length = d[0].hypot(d[1]);
    let u = [d[0] / length, d[1] / length];
    let f = [second[0] - first[0], second[1] - first[1]];
    let off = u[0] * f[1] - u[1] * f[0];
    let along = u[0] * f[0] + u[1] * f[1] - length;
    let across = if u[0].abs() >= u[1].abs() {
        "in Y"
    } else {
        "in X"
    };
    let said = format!(
        "Pin 2 is {:.3} mm off the line {across} over {length:.1} mm (the plate is turned {:.3}° to the machine) and {:+.3} mm along it.",
        off.abs(),
        off.atan2(length).to_degrees().abs(),
        along
    );
    if off.abs() > 0.05 {
        (
            false,
            format!(
                "{said} That is over 0.05 mm: square the plate to X and Y, or the pins are off by up to that much in every setup on them."
            ),
        )
    } else {
        (true, said)
    }
}

/// Where the simulator's pin 1 stands before its place is recorded: the
/// pins centred between the front of the table and the default block
/// (`sim_block`), or between its left end and the block when they spread
/// further along Y, so that the block is not over them.
fn sim_pins(cfg: &MachineConfig, pins: &Pins) -> [f64; 2] {
    let Some((lo, hi)) = sim_block(cfg, None) else {
        return [0., 0.];
    };
    let ends = |a: usize| {
        let v = (0..pins.at.len()).map(|i| pins.offset(i)[a]);
        (
            v.clone().fold(f64::INFINITY, f64::min),
            v.fold(f64::NEG_INFINITY, f64::max),
        )
    };
    let spread = |a: usize| ends(a).1 - ends(a).0;
    let mut c = [(lo[0] + hi[0]) / 2., (lo[1] + hi[1]) / 2.];
    let k = usize::from(spread(0) >= spread(1));
    let end = cfg.axes.get(k).and_then(|x| x.soft_min).unwrap_or(lo[k]);
    c[k] = (end + lo[k]) / 2.;
    [0, 1].map(|a| c[a] - (ends(a).0 + ends(a).1) / 2.)
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
    /// Halve an axis's work position where the tool is, as a milling
    /// machine's DRO does: zeroed on one side of a part and moved to the
    /// other, X0 is then midway.
    WorkHalf(usize),
    /// An axis's Set field (`Connection::presets`).
    WorkText(usize, String),
    /// Make the current position the axis's Set field value in the active
    /// coordinate system.
    WorkSet(usize),
    /// Rapid X and Y to work X0 Y0 at the height the tool is at.
    ToWorkXY,
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
    /// The stock-top spot Probe corner, Probe edge or Probe centre finds the
    /// setup's work zero from (`post::Checked::probe_from`): None for the
    /// zero's own.
    ProbeFrom(Option<[crate::post::At; 2]>),
    /// Probe a feature where the tip is and set work zero on it, without a
    /// setup (`Feature`).
    ProbeFeature(Feature),
    /// Sets the work position where the tool tip is to that of the setup
    /// picked in the Run panel's touch-off point (`Connection::touch_off`):
    /// the tip is on the stock top there. Like Probe Z, it waits while the
    /// setup's check is out of date (`Connection::run_program`).
    TouchOff,
    /// Simulated: rapid the tool tip onto the work zero of the setup picked
    /// in the Run panel on the simulator's block (`Connection::sim_zero`).
    /// Waits as TouchOff does.
    ToSimZero,
    /// Record the pins' place: Pin centre on pin 1 where the tip is, which
    /// becomes the reference (`Pins::reference`), then on pin 2 where the
    /// plate puts it, reporting how square the plate is.
    RecordPins,
    /// Reference X Y from pin 1: Pin centre on pin 1 where the tip is, then
    /// declare machine X and Y so that its centre is where it was recorded.
    ReferencePins,
    /// Set the X and Y work offset of the active work system to the one the
    /// pins give the setup picked in the Run panel (`Connection::pin_offset`),
    /// at the current position. Waits as TouchOff does.
    PinZero,
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
    /// The stock-top spot picked to probe the setup's work zero from
    /// (`Action::ProbeFrom`).
    pub probe_spot: Option<[crate::post::At; 2]>,
    routine: Option<Routine>,
    /// Each axis's Set field (`Action::WorkSet`).
    pub presets: [String; 6],
    /// How the last probe ended (ok, what it did).
    pub probed: Option<(bool, String)>,
    /// The simulator's block (min and max corners, machine mm).
    pub block: Option<([f64; 3], [f64; 3])>,
    /// The machine's fixture pins, or what is wrong with them: from the
    /// Machine library at connecting, then as saved and recorded.
    pub pins: Result<Option<Pins>, Vec<String>>,
    /// X and Y were referenced from pin 1 (or its place recorded) since
    /// connecting: cleared by every E-stop and by declaring X or Y another
    /// way.
    pub pin_referenced: bool,
    /// Pin 1's machine X Y just recorded, for the app to keep in the
    /// machine's profile (`take`n there).
    pub recorded: Option<[f64; 2]>,
    /// How the last Record the pins' place or Reference X Y from pin 1
    /// ended (ok, what it did).
    pub pin_said: Option<(bool, String)>,
    /// Reference X Y from pin 1 waits for X and Y to be declared where they
    /// are before it probes.
    pin_start: bool,
    /// The pin 1 declarations are in `declare`: once sent, X and Y are
    /// referenced from it and this is said.
    referencing: Option<String>,
    /// Simulated: pin 1's machine X Y. Declaring X or Y moves it in machine
    /// coordinates the other way, as it stays put on the table.
    sim_pins: Option<[f64; 2]>,
    /// Simulated: the blocks last sent to the driver.
    sim_sent: Vec<Block>,
    /// Keeps macOS from throttling AXIS CAM in the background, which would
    /// starve the controller's motion buffer.
    _nap: uc100::macos::ActivityToken,
}

/// Why a probe whose program ended with the machine in `state`, not Idle
/// or paused, set nothing: the driver takes SetWork in those two only.
fn stopped_after(state: &Option<State>) -> String {
    match state {
        Some(State::EStop) => {
            "The machine was E-stopped as the probe ended, so nothing is set: probe again.".into()
        }
        Some(s) => format!(
            "The machine is {} after the probe ended, so nothing is set: probe again.",
            s.label()
        ),
        None => {
            "The machine is not connected after the probe ended, so nothing is set: probe again."
                .into()
        }
    }
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
        let pins = pins_settings(&m.p);
        let sim_pins = match (&pins, block) {
            (Ok(Some(pins)), Some(_)) => {
                Some(pins.reference.unwrap_or_else(|| sim_pins(&cfg, pins)))
            }
            _ => None,
        };
        let mut c = Self {
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
            probe_spot: None,
            routine: None,
            presets: Default::default(),
            probed: None,
            block,
            pins,
            pin_referenced: false,
            recorded: None,
            pin_said: None,
            pin_start: false,
            referencing: None,
            sim_pins,
            sim_sent: vec![],
        };
        c.send_sim_blocks();
        Ok(c)
    }

    /// Simulated: the blocks the probe touches. The stock's block, unless
    /// the part is off the pins: while a pin is probed, and for a setup that
    /// sits on them until X and Y are referenced from pin 1 (it is put on
    /// them after); and a block of each pin's size from the bottom of Z, pin
    /// 1 at `sim_pins`.
    fn sim_world(&self) -> Vec<Block> {
        let mut out = vec![];
        let pin_probe = self
            .routine
            .as_ref()
            .is_some_and(|r| r.purpose != Purpose::Work);
        let on_pins = (self.run.as_ref())
            .and_then(|r| r.checked()?.first())
            .is_some_and(|p| p.pins.is_some());
        let off = pin_probe || (on_pins && !self.pin_referenced);
        if let Some((min, max)) = self.block.filter(|_| !off) {
            out.push(Block { min, max });
        }
        if let (Some(at), Ok(Some(pins)), Some((lo, _))) = (self.sim_pins, &self.pins, self.block) {
            let r = pins.diameter / 2.;
            for i in 0..pins.at.len() {
                let d = pins.offset(i);
                let c = [at[0] + d[0], at[1] + d[1]];
                out.push(Block {
                    min: [c[0] - r, c[1] - r, lo[2]],
                    max: [c[0] + r, c[1] + r, lo[2] + pins.height],
                });
            }
        }
        out
    }

    /// Simulated: pin 1's machine X Y, where the simulator's pins stand.
    pub fn sim_pin1(&self) -> Option<[f64; 2]> {
        self.sim_pins.filter(|_| self.simulated)
    }

    /// Simulated: sends the blocks when they changed since last sent.
    fn send_sim_blocks(&mut self) {
        if !self.simulated {
            return;
        }
        let world = self.sim_world();
        if world != self.sim_sent {
            self.send(Command::SimBlocks(world.clone()));
            self.sim_sent = world;
        }
    }

    /// The simulator's block takes the X/Y size of the stock of the setup
    /// to run, once it is checked, so that Probe centre finds its sides.
    /// A setup that sits on the pins has its block where the pins place it:
    /// pin 1 under the spot of the stock it goes into.
    pub fn fit_sim_block(&mut self) {
        let p = (self.run.as_ref()).and_then(|r| r.checked()?.first());
        let size = p.map(|p| p.stock.dimensions());
        if let (true, Some(size)) = (self.simulated, size)
            && let Some((mut min, mut max)) = sim_block(&self.config, Some(size))
        {
            if let (Some(p), Some(at)) = (p, self.sim_pins)
                && let Some(on) = &p.pins
            {
                let pin1 = on.places[0];
                for a in 0..2 {
                    min[a] = at[a] + (p.stock.min[a] - pin1[a]) as f64;
                    max[a] = at[a] + (p.stock.max[a] - pin1[a]) as f64;
                }
            }
            self.block = Some((min, max));
            self.send_sim_blocks();
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
    fn move_to_sim_zero(&mut self) -> Option<()> {
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
        match &s.state {
            Some(State::Run | State::Hold | State::Stopping) => return None,
            Some(State::Idle | State::Paused(_)) => {}
            // An E-stop (or a disconnect, a jog) just after the outcome: the
            // driver leaves an ok outcome in place, having no program to
            // abort, and drops SetWork and the retract. Nothing is set (#166).
            other if o.ok => return Some((false, stopped_after(other), None)),
            _ => {}
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
            self.pin_referenced = false;
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
        self.routine_step();
        // Any state but Idle (E-stop, a move) drops what is left, so nothing
        // is declared later by surprise.
        if self.idle() {
            if let Some((axis, value)) = self.declare.pop_front() {
                self.send(Command::SetMachine { axis, value });
                // The pins stay put on the table: in the new machine
                // coordinates they are where the axis moved from.
                if let Some(mut at) = self.sim_pins.filter(|_| axis < 2) {
                    at[axis] += value - self.snapshot.machine_pos[axis];
                    self.sim_pins = Some(at);
                    self.fit_sim_block();
                }
                if self.declare.is_empty()
                    && let Some(said) = self.referencing.take()
                {
                    self.pin_referenced = true;
                    self.pin_said = Some((true, said));
                }
            }
        } else {
            self.declare.clear();
            if self.referencing.take().is_some() {
                self.pin_said = Some((
                    false,
                    "The machine stopped before X and Y were declared from pin 1: reference them again.".into(),
                ));
            }
        }
        // Reference X Y from pin 1 probes once X and Y have a position: the
        // driver reports it a loop or two after the declarations.
        if self.pin_start && self.declare.is_empty() {
            if !self.idle() {
                self.pin_start = false;
                self.pin_said = Some((
                    false,
                    "The machine stopped before X and Y were declared where they show: reference them again.".into(),
                ));
            } else if (0..2).all(|a| self.snapshot.referenced[a]) {
                self.pin_start = false;
                self.start_pins(Purpose::Reference);
            }
        }
        self.send_sim_blocks();
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
                self.declared(&zero);
            }
            Action::KeepPosition if self.idle() => {
                let here: Vec<_> = (0..6)
                    .filter(|&a| self.axis(a) && self.switchless(a))
                    .map(|a| (a, self.snapshot.machine_pos[a]))
                    .collect();
                self.declared(&here);
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
            Action::WorkHalf(a) if self.manual() && self.axis(a) => {
                if let (Some(run), 2) = (&mut self.run, a) {
                    run.touched = true;
                }
                self.send(Command::SetWork {
                    axis: a,
                    value: self.snapshot.work_pos[a] / 2.,
                });
            }
            Action::WorkText(a, text) if a < 6 => self.presets[a] = text,
            Action::WorkSet(a) if self.manual() && self.axis(a) => {
                if let Some(value) = self.preset(a) {
                    if let (Some(run), 2) = (&mut self.run, a) {
                        run.touched = true;
                    }
                    self.send(Command::SetWork { axis: a, value });
                }
            }
            Action::ToWorkXY if self.to_work_xy().is_ok() => {
                let now = self.snapshot.machine_pos;
                let mut to = now;
                for (v, work) in to.iter_mut().zip(self.snapshot.work_pos).take(2) {
                    *v -= work;
                }
                self.send(job("To X0 Y0", moves_from(now, &[to])));
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
            Action::ProbeFrom(spot) if !self.probing() => self.probe_spot = spot,
            Action::ProbeFeature(f) => self.start_routine(f),
            // Both act on the checked setup, which an edit may have changed:
            // like the probes, they wait while its check is out of date (#39).
            Action::TouchOff if self.manual() && self.run_program().is_ok() => {
                if let Some((_, work)) = self.touch_off() {
                    // On the pins, the pins set X and Y: the touch-off is Z's.
                    let pins = self.run_program().is_ok_and(|p| p.pins.is_some());
                    for (a, value) in work.into_iter().enumerate() {
                        if pins && a < 2 {
                            continue;
                        }
                        self.send(Command::SetWork { axis: a, value });
                    }
                    if let Some(run) = &mut self.run {
                        run.touched = true;
                    }
                }
            }
            Action::ToSimZero if self.run_program().is_ok() => {
                self.move_to_sim_zero();
            }
            Action::RecordPins if self.record_blocked().is_none() => {
                self.start_pins(Purpose::Record(None))
            }
            Action::ReferencePins if self.reference_blocked().is_none() => {
                self.pin_said = None;
                if (0..2).all(|a| self.snapshot.referenced[a]) {
                    self.start_pins(Purpose::Reference);
                } else {
                    // Short moves from where the tip is: X and Y are declared
                    // where they show first, as the driver probes only an
                    // axis with a position.
                    let at = self.snapshot.machine_pos;
                    self.declare.extend([(0, at[0]), (1, at[1])]);
                    self.pin_start = true;
                }
            }
            Action::PinZero if self.manual() && self.run_program().is_ok() => {
                if let Ok(offset) = self.pin_offset() {
                    for (a, &o) in offset.iter().enumerate() {
                        self.send(Command::SetWork {
                            axis: a,
                            value: self.snapshot.machine_pos[a] - o,
                        });
                    }
                }
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
    pub fn probe_ready(&self, axes: &[usize]) -> Option<String> {
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
        self.probing.is_some() || self.edges.is_some() || self.routine.is_some() || self.pin_start
    }

    /// The feature being probed (`Feature`), if one is.
    pub fn feature(&self) -> Option<Feature> {
        (self.routine.as_ref())
            .filter(|r| r.purpose == Purpose::Work)
            .map(|r| r.feature)
    }

    /// Record the pins' place (true) or Reference X Y from pin 1 (false)
    /// is under way, if one is.
    pub fn pinning(&self) -> Option<bool> {
        match self.routine.as_ref().map(|r| r.purpose) {
            Some(Purpose::Record(_)) => Some(true),
            Some(Purpose::Reference) => Some(false),
            _ => self.pin_start.then_some(false),
        }
    }

    /// Probe corner or Probe centre is under way.
    pub fn edging(&self) -> bool {
        self.edges.is_some()
    }

    /// The setup picked in the Run panel cannot be changed now: a tool run
    /// is under way, or a probe is setting that setup's work zero (#165).
    pub fn setup_locked(&self) -> bool {
        self.run.as_ref().is_some_and(|r| r.running.is_some()) || self.probing()
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
        // The driver takes SetWork only Idle or paused (#166).
        if !self.manual() {
            self.probed = Some((false, stopped_after(&self.snapshot.state)));
            return;
        }
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
        // A spot picked that a new check of the setup finds blocked is
        // left for the zero's own, as the grid shows it.
        if let Some(refs) = self.probe_spot
            && p.probe_from.iter().any(|(r, ok)| *r == refs && ok.is_ok())
        {
            return Ok((refs, p));
        }
        match &p.probe {
            Ok(refs) => Ok((*refs, p)),
            Err(why) if p.probe_from.iter().any(|(_, ok)| ok.is_ok()) => Err(format!(
                "{why} Pick a spot under Probe from that a probe can reach."
            )),
            Err(why) => Err(why.clone()),
        }
    }

    /// An axis's Set field, when it is a number.
    pub fn preset(&self, a: usize) -> Option<f64> {
        let v: f64 = self.presets.get(a)?.trim().parse().ok()?;
        v.is_finite().then_some(v)
    }

    /// Why Go to X0 Y0 cannot move now.
    pub fn to_work_xy(&self) -> Result<(), String> {
        if !self.idle() {
            return Err("The machine is not Idle.".into());
        }
        if !self.unset_limited_axes().is_empty() || !(0..2).all(|a| self.snapshot.referenced[a]) {
            return Err(format!(
                "X and Y need a machine position first: {}.",
                if self.home_switches() {
                    "home"
                } else {
                    "set machine zero"
                }
            ));
        }
        Ok(())
    }

    /// Why probing feature `f` cannot start now; None when it can.
    pub fn feature_blocked(&self, f: Feature) -> Option<String> {
        self.probe_ready(f.moves())
            .or_else(|| match (&self.probe, f) {
                (
                    Ok(Probe { tip: None, .. }),
                    Feature::Edge(..) | Feature::Corner(_) | Feature::Bore | Feature::Boss,
                ) => Some("Probe tip diameter is not set: set it in the Machine library.".into()),
                _ => None,
            })
    }

    /// Declares the machine positions `axes` (axis, value). X or Y declared
    /// another way than from pin 1 is no longer referenced from it.
    fn declared(&mut self, axes: &[(usize, f64)]) {
        if axes.iter().any(|&(a, _)| a < 2) {
            self.pin_referenced = false;
        }
        self.declare.extend(axes.iter().copied());
    }

    /// The machine's pins, or why there are none to probe.
    fn pins_set(&self) -> Result<&Pins, String> {
        match &self.pins {
            Ok(Some(pins)) => Ok(pins),
            Ok(None) => Err("This machine has no fixture pins: add them under Fixture pins in the Machine library.".into()),
            Err(e) => Err(format!("{} Set them in the Machine library.", e.join(" "))),
        }
    }

    /// Why Pin centre cannot probe pin 1 now (on axes `axes`); None when
    /// it can.
    fn pin_blocked(&self, axes: &[usize]) -> Option<String> {
        self.pins_set()
            .err()
            .or_else(|| self.probe_ready(axes))
            .or_else(|| match &self.probe {
                Ok(Probe { tip: None, .. }) => {
                    Some("Probe tip diameter is not set: set it in the Machine library.".into())
                }
                _ => None,
            })
    }

    /// Why Record the pins' place cannot start now; None when it can. X,
    /// Y and Z need a machine position: the place it records is in them.
    pub fn record_blocked(&self) -> Option<String> {
        self.pin_blocked(&[0, 1, 2])
    }

    /// Why Reference X Y from pin 1 cannot start now; None when it can.
    /// Z needs a machine position; X and Y need none, but the pins' place
    /// must be recorded.
    pub fn reference_blocked(&self) -> Option<String> {
        self.pin_blocked(&[2]).or_else(|| match self.pins_set() {
            Ok(Pins { reference: None, .. }) => Some(
                "The pins' place is not recorded yet: set machine zero, then Record the pins' place."
                    .into(),
            ),
            _ => None,
        })
    }

    /// How a feature probe for `purpose` ended: in `probed`, or for the pins
    /// in `pin_said`.
    fn said(&mut self, purpose: Purpose, ok: bool, message: String) {
        match purpose {
            Purpose::Work => self.probed = Some((ok, message)),
            _ => self.pin_said = Some((ok, message)),
        }
    }

    /// Starts Pin centre on pin 1 where the tip is, for `purpose`.
    fn start_pins(&mut self, purpose: Purpose) {
        let (Ok(pins), Ok(probe)) = (self.pins_set(), self.probe.clone()) else {
            return;
        };
        self.routine = Some(Routine::pin(
            pins,
            &probe,
            self.snapshot.machine_pos,
            purpose,
        ));
        self.pin_said = None;
        self.send_sim_blocks();
        self.routine_step();
    }

    /// The X and Y work offset the pins give the setup picked in the Run
    /// panel: pin 1's recorded machine place less where pin 1 stands in the
    /// setup's work coordinates. Or why there is none.
    pub fn pin_offset(&self) -> Result<[f64; 2], String> {
        let p = self.run_program()?;
        let Some(on) = &p.pins else {
            return Err("This setup does not sit on the pins.".into());
        };
        let pins = self.pins_set()?;
        let Some(at) = pins.reference else {
            return Err(
                "The pins' place is not recorded yet: Record the pins' place under Position."
                    .into(),
            );
        };
        Ok([0, 1].map(|a| at[a] - on.pin1[a] as f64))
    }

    /// `p` held inside the soft limits of X, Y and Z, which the driver
    /// refuses a move past.
    fn inside(&self, mut p: Vec6) -> Vec6 {
        for (a, v) in p.iter_mut().enumerate().take(3) {
            if let Some(x) = self.config.axes.get(a) {
                if let Some(lo) = x.soft_min {
                    *v = v.max(lo);
                }
                if let Some(hi) = x.soft_max {
                    *v = v.min(hi);
                }
            }
        }
        p
    }

    /// Starts probing feature `f` from where the tip is.
    fn start_routine(&mut self, f: Feature) {
        let (None, Ok(probe)) = (self.feature_blocked(f), self.probe.clone()) else {
            return;
        };
        self.routine = Some(Routine::new(f, &probe, self.snapshot.machine_pos));
        self.probed = None;
        self.routine_step();
    }

    /// Follows a feature probe: once a command has ended, checks how and
    /// sends the next, until every touch is made.
    fn routine_step(&mut self) {
        let Some(mut r) = self.routine.take() else {
            return;
        };
        if let Some((leg, seq)) = r.sent {
            let Some((ok, message, result)) = self.ended(seq) else {
                self.routine = Some(r);
                return;
            };
            let tripped = result.filter(|r| r.tripped).map(|r| r.position);
            let failed = match (leg, tripped) {
                _ if !ok => Some(message),
                (Leg::Guard(_), Some(p)) => Some(format!(
                    "The probe touched something at machine X {:.3} Y {:.3} Z {:.3} {}. {}",
                    p[0],
                    p[1],
                    p[2],
                    if r.feature == Feature::Boss {
                        "going down beside the boss: it is wider than the probe goes out, or something stands beside it"
                    } else {
                        "stepping along the side before it probes the other"
                    },
                    r.how()
                )),
                (Leg::Touch(_), Some(p)) => {
                    r.touches.push(p);
                    None
                }
                (Leg::Touch(to), None) => Some(format!(
                    "No contact up to machine X {:.3} Y {:.3} Z {:.3}. {}",
                    to[0],
                    to[1],
                    to[2],
                    r.how()
                )),
                _ => None,
            };
            if let Some(why) = failed {
                self.said(r.purpose, false, why);
                return;
            }
            r.sent = None;
        }
        loop {
            if r.legs.is_empty() {
                match r.plan() {
                    Some(legs) => r.legs = legs.into(),
                    // The pins' place is recorded and X and Y declared
                    // from where the tip ends up, back over the pin and
                    // stopped there.
                    None if r.purpose != Purpose::Work && !r.backed => {
                        r.backed = true;
                        r.legs = r.last().into_iter().map(Leg::Rapid).collect();
                        continue;
                    }
                    None => return self.routine_done(r),
                }
            }
            let now = self.snapshot.machine_pos;
            let leg = match r.legs.pop_front().unwrap() {
                Leg::Rapid(to) => Leg::Rapid(self.inside(to)),
                Leg::Guard(to) => Leg::Guard(self.inside(to)),
                Leg::Touch(to) => Leg::Touch(self.inside(to)),
            };
            let name = format!("Probe {}", r.feature.label());
            let seq = self.seq();
            match leg {
                // A rapid that goes nowhere is no command: the driver would
                // report no outcome for it.
                Leg::Rapid(to) if (0..6).all(|a| (to[a] - now[a]).abs() < 1e-9) => continue,
                Leg::Rapid(to) => self.send(job(&name, moves_from(now, &[to]))),
                Leg::Guard(to) => self.send(job(
                    &name,
                    vec![Do::Probe {
                        target: to,
                        feed: r.feed / 60.,
                        must_trip: false,
                    }],
                )),
                // G38.3: a miss is not an error; it is reported as one here.
                Leg::Touch(to) => self.send(Command::Probe {
                    target: to,
                    feed: r.feed,
                    must_trip: false,
                }),
            }
            r.sent = Some((leg, seq));
            self.routine = Some(r);
            return;
        }
    }

    /// Every touch of a feature probe is made, Idle: sets work zero on the
    /// feature and moves the tip back (`Routine::last`).
    fn routine_done(&mut self, r: Routine) {
        // The driver takes SetWork only Idle or paused (#166).
        if !self.manual() {
            self.said(r.purpose, false, stopped_after(&self.snapshot.state));
            return;
        }
        let w = r.zero();
        // A pin routine is back over the pin already (`Routine::backed`).
        let now = self.snapshot.machine_pos;
        match r.purpose {
            Purpose::Work => {
                let back: Vec<Vec6> = r.last().into_iter().map(|p| self.inside(p)).collect();
                for &a in r.feature.axes() {
                    self.send(Command::SetWork {
                        axis: a,
                        value: now[a] - w[a],
                    });
                }
                if let (Some(run), Feature::Top) = (&mut self.run, r.feature) {
                    run.touched = true;
                }
                self.probed = Some((true, r.found()));
                let moves = moves_from(now, &back);
                if !moves.is_empty() {
                    self.send(job(&format!("Probe {}", r.feature.label()), moves));
                }
            }
            Purpose::Record(None) => {
                // Pin 1 found: on to pin 2 where the plate puts it, over the
                // top of Z, down to the height the tip started at over pin 1.
                let Ok(pins) = self.pins_set() else {
                    return;
                };
                let d = pins.offset(1);
                let mut next = r.start;
                (next[0], next[1]) = (w[0] + d[0], w[1] + d[1]);
                let mut second =
                    Routine::pin(pins, &r.probe(), next, Purpose::Record(Some([w[0], w[1]])));
                let top = self.config.axes.get(2).and_then(|z| z.soft_max);
                let high = |mut p: Vec6| {
                    p[2] = top.unwrap_or(p[2]).max(p[2]);
                    p
                };
                second.legs = [high(now), high(next), next]
                    .map(|p| Leg::Rapid(self.inside(p)))
                    .into();
                self.routine = Some(second);
                self.routine_step();
            }
            Purpose::Record(Some(first)) => {
                let Ok(pins) = self.pins_set() else {
                    return;
                };
                let (ok, said) = squareness(pins, first, [w[0], w[1]]);
                if let Ok(Some(pins)) = &mut self.pins {
                    pins.reference = Some(first);
                }
                self.recorded = Some(first);
                self.pin_referenced = true;
                self.pin_said = Some((
                    ok,
                    format!(
                        "Pins' place recorded: pin 1 at machine X {:.3} Y {:.3}. {said}",
                        first[0], first[1]
                    ),
                ));
            }
            Purpose::Reference => {
                let Ok(&Pins {
                    reference: Some(at),
                    ..
                }) = self.pins_set()
                else {
                    return;
                };
                let by = [at[0] - w[0], at[1] - w[1]];
                // Stopped over the pin: X and Y are declared moved by `by`
                // from here, one a tick (`declare`).
                self.declare
                    .extend([(0, now[0] + by[0]), (1, now[1] + by[1])]);
                self.referencing = Some(format!(
                    "X and Y referenced from pin 1: it was found at machine X {:.3} Y {:.3}, {:.3} mm and {:.3} mm from its recorded place X {:.3} Y {:.3}, so X and Y are declared moved by {:+.3} and {:+.3} mm. Every saved work offset, the soft limits and the pins' place hold again.",
                    w[0], w[1], -by[0], -by[1], at[0], at[1], by[0], by[1]
                ));
            }
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
            &p.origin, refs, p.stock, p.datum, &p.holds, &probe, tip, start, z,
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
            let Some(i) = e.next() else {
                return self.edges_done(e);
            };
            e.steps = e.plan(i);
            // The top of Z would hold the traverse down onto a jaw or clamp
            // under its way (#163).
            if let Some(Step::Go(moves, _)) = e.steps.front()
                && let Some(why) = e.blocked(i, moves)
            {
                self.probed = Some((false, why));
                return;
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
        // The driver takes SetWork only Idle or paused (#166).
        if !self.manual() {
            self.probed = Some((false, stopped_after(&self.snapshot.state)));
            return;
        }
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
        up[2] = e.high(w);
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
        self.snapshot.state == Some(State::Idle) && self.edges.is_none() && self.routine.is_none()
    }

    /// Jog, work zero, MDI and the spindle are allowed: Idle, or paused in a program.
    pub fn manual(&self) -> bool {
        matches!(self.snapshot.state, Some(State::Idle | State::Paused(_)))
            && self.edges.is_none()
            && self.routine.is_none()
    }

    /// Every enabled axis has a home switch, so Home can run.
    pub fn home_switches(&self) -> bool {
        (0..6)
            .filter_map(|a| self.config.axes.get(a))
            .all(|a| a.home.is_some())
    }

    /// X, Y and Z all have both soft limits, as `MachineView::envelope`
    /// wants: the driver's `soft_limits_configured` is true with one axis
    /// limited, while the others run into their ends (#164).
    pub fn soft_limits_configured(&self) -> bool {
        self.unlimited_axes().is_empty()
    }

    /// X, Y and Z axes without both soft limits in the profile: nothing
    /// stops them at the end of their travel, position or not.
    pub fn unlimited_axes(&self) -> Vec<usize> {
        (0..3)
            .filter(|&a| {
                self.config
                    .axes
                    .get(a)
                    .is_none_or(|x| x.soft_min.is_none() || x.soft_max.is_none())
            })
            .collect()
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

/// A widget operation that gives the keyboard focus to the form's first text
/// field, for a Tab with nothing focused: the first in the inspector
/// (`ui::INSPECTOR_ID`) when the page has one, else the first form field of the
/// page (one with a `help::FieldTag`), else the first text field there is.
pub fn focus_first() -> impl iced::advanced::widget::Operation {
    use crate::help::FieldTag;
    use iced::advanced::widget::{
        Id, Operation,
        operation::{self, Focusable, Outcome, Scrollable},
    };
    /// Counts the text fields, noting the first in the inspector and the
    /// first in a form field.
    #[derive(Default)]
    struct Scan {
        count: usize,
        inspector: Option<usize>,
        field: Option<usize>,
        /// The inspector or a field was just announced: its content follows.
        enter_inspector: bool,
        enter_field: bool,
        in_inspector: bool,
        in_field: bool,
    }
    impl Operation<Option<usize>> for Scan {
        fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<Option<usize>>)) {
            let was = (self.in_inspector, self.in_field);
            self.in_inspector |= std::mem::take(&mut self.enter_inspector);
            self.in_field |= std::mem::take(&mut self.enter_field);
            operate(self);
            (self.in_inspector, self.in_field) = was;
        }
        fn scrollable(
            &mut self,
            id: Option<&Id>,
            _: iced::Rectangle,
            _: iced::Rectangle,
            _: iced::Vector,
            _: &mut dyn Scrollable,
        ) {
            self.enter_inspector = id == Some(&Id::new(crate::ui::INSPECTOR_ID));
        }
        fn custom(&mut self, _: Option<&Id>, _: iced::Rectangle, state: &mut dyn std::any::Any) {
            self.enter_field = state.is::<FieldTag>();
        }
        fn focusable(&mut self, _: Option<&Id>, _: iced::Rectangle, _: &mut dyn Focusable) {
            if self.in_inspector {
                self.inspector.get_or_insert(self.count);
            }
            if self.in_field {
                self.field.get_or_insert(self.count);
            }
            self.count += 1;
        }
        fn finish(&self) -> Outcome<Option<usize>> {
            Outcome::Some(
                self.inspector
                    .or(self.field)
                    .or((self.count > 0).then_some(0)),
            )
        }
    }
    /// Focuses the text field at the index, counting the fields passed.
    struct FocusAt(Option<usize>, usize);
    impl Operation for FocusAt {
        fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
            operate(self)
        }
        fn focusable(&mut self, _: Option<&Id>, _: iced::Rectangle, state: &mut dyn Focusable) {
            if self.0 == Some(self.1) {
                state.focus()
            }
            self.1 += 1
        }
    }
    operation::then(Scan::default(), |target| FocusAt(target, 0))
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

    fn capped(profile: &Path, rpm: u32) {
        let text =
            format!("{LIMITED}[outputs]\nspindle_pwm = {{ pin = 16 }}\nspindle_max_rpm = {rpm}\n");
        std::fs::write(profile, text).unwrap();
    }

    #[test]
    fn the_spindle_cap_is_read_once_per_profile_file_and_modification_time() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("capped.toml");
        capped(&profile, 12000);
        let m = uc100(&profile.display().to_string(), "Simulated");
        let reads = || CAP_READS.with(|n| n.get());
        let before = reads();
        // A view asks on every redraw, and the NC page once per operation.
        for _ in 0..5 {
            assert_eq!(spindle_cap(&m), Ok(Some(12000.)));
        }
        assert_eq!(reads() - before, 1);
        // Saved with another maximum: read again, once.
        capped(&profile, 9000);
        let file = std::fs::File::options().write(true).open(&profile).unwrap();
        file.set_modified(std::time::SystemTime::now() + Duration::from_secs(5))
            .unwrap();
        for _ in 0..3 {
            assert_eq!(spindle_cap(&m), Ok(Some(9000.)));
        }
        assert_eq!(reads() - before, 2);
        // Another path is another entry.
        let other = dir.path().join("other.toml");
        capped(&other, 6000);
        let n = uc100(&other.display().to_string(), "Simulated");
        assert_eq!(spindle_cap(&n), Ok(Some(6000.)));
        assert_eq!(spindle_cap(&m), Ok(Some(9000.)));
        assert_eq!(reads() - before, 3);
    }

    #[test]
    fn a_profile_that_is_missing_is_refused_not_kept() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("gone.toml");
        let m = uc100(&profile.display().to_string(), "Simulated");
        assert!(spindle_cap(&m).is_err());
        // Written afterwards, it is used at once.
        capped(&profile, 8000);
        assert_eq!(spindle_cap(&m), Ok(Some(8000.)));
        // And a machine that is not a UC100 reads no file.
        let mut other = Machine::new(3);
        other
            .p
            .insert("uc100_profile".into(), "/no/such/file.toml".into());
        let before = CAP_READS.with(|n| n.get());
        assert_eq!(spindle_cap(&other), Ok(None));
        assert_eq!(CAP_READS.with(|n| n.get()), before);
    }

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
    fn touch_box(e: Edges, lo: [f64; 3], hi: [f64; 3]) -> Edges {
        touch_box_round(e, lo, hi, &[]).unwrap()
    }

    /// `touch_box` with `clamps` (machine min and max corners) about the
    /// box: a level traverse whose tip, `e.radius` round, would run into
    /// one at or below its height fails, and the routine's own refusal of
    /// a traverse (`Edges::blocked`) is returned.
    fn touch_box_round(
        mut e: Edges,
        lo: [f64; 3],
        hi: [f64; 3],
        clamps: &[([f64; 3], [f64; 3])],
    ) -> Result<Edges, String> {
        let mut at = e.start;
        while let Some(i) = e.next() {
            let steps = e.plan(i);
            if let Some(Step::Go(moves, _)) = steps.front()
                && let Some(why) = e.blocked(i, moves)
            {
                return Err(why);
            }
            for step in steps {
                match step {
                    Step::Go(moves, guard) => {
                        for pair in moves.windows(2) {
                            let (a, b) = (pair[0], pair[1]);
                            if (a[2] - b[2]).abs() > 1e-9 {
                                continue;
                            }
                            for (lo, hi) in clamps {
                                let r = e.radius;
                                let hit = hi[2] >= a[2]
                                    && crate::post::segment_crosses(
                                        [a[0], a[1]],
                                        [b[0], b[1]],
                                        [lo[0] - r, lo[1] - r],
                                        [hi[0] + r, hi[1] + r],
                                    );
                                assert!(
                                    !hit,
                                    "the traverse at Z {} from {a:?} to {b:?} runs into the clamp at {lo:?}…{hi:?}",
                                    a[2]
                                );
                            }
                        }
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
        Ok(e)
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
            &[],
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
            &[],
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
            &[],
            &probe,
            4.,
            start,
            z,
        );
        let e = touch_box(e, lo, hi);
        assert_eq!(e.touches[0].len(), 2);
        check(&e, [100., 50., -30.]);
    }

    /// Every stock-based work zero, probed from each of the nine spots of
    /// the stock top, is set where the program has it: from its own spot as
    /// before, from another by the stock's size.
    #[test]
    fn every_zero_is_found_from_every_spot() {
        use crate::post::{At, ORIGINS, probe_faces, stock_zero};
        let stock = crate::mesh::Bounds {
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
        // Machine = plan + this: the stock at machine X 100…224, Y 50…134,
        // Z -70…-30.
        let shift = [100., 50., -70.];
        let (lo, hi) = ([100., 50., -70.], [224., 134., -30.]);
        let along = |k: usize, at: At| match at {
            At::Min => stock.min[k],
            At::Mid => (stock.min[k] + stock.max[k]) / 2.,
            At::Max => stock.max[k],
        };
        let spots = [At::Min, At::Mid, At::Max];
        for origin in ORIGINS.iter().filter(|o| stock_zero(o).is_some()) {
            let (xy, top) = stock_zero(origin).unwrap();
            let datum = [
                along(0, xy[0]),
                along(1, xy[1]),
                if top { stock.max[2] } else { stock.min[2] },
            ];
            let zero: [f64; 3] = std::array::from_fn(|a| datum[a] as f64 + shift[a]);
            for refs in spots
                .iter()
                .flat_map(|&x| spots.iter().map(move |&y| [x, y]))
            {
                let spot = probe_faces(stock, refs)[0].3;
                let start = [
                    (spot.min[0] + spot.max[0]) as f64 / 2. + shift[0],
                    (spot.min[1] + spot.max[1]) as f64 / 2. + shift[1],
                    -25.,
                    0.,
                    0.,
                    0.,
                ];
                let e = Edges::new(
                    origin,
                    refs,
                    stock,
                    datum,
                    &[],
                    &probe,
                    4.,
                    start,
                    (-80., 0.),
                );
                let e = touch_box(e, lo, hi);
                let (w, fixed) = e.zero();
                assert_eq!(fixed, [true; 3], "{origin} from {refs:?}");
                assert!(
                    (0..3).all(|a| (w[a] - zero[a]).abs() < 1e-9),
                    "{origin} from {refs:?}: {w:?}, expected {zero:?}"
                );
                let own = refs == xy;
                assert_eq!(e.found().contains("placed from"), !own, "{}", e.found());
            }
        }
    }

    /// Runs a feature probe against the material `solid` says is there
    /// (machine mm): a guarded move must meet nothing, a touch stops where
    /// the tip, `r.radius` round, first meets material (its bottom going
    /// down, its equator going sideways), to 1e-5 mm. Returns where the
    /// routine leaves the tip.
    fn walk_routine(r: &mut Routine, solid: &dyn Fn([f64; 3]) -> bool) -> Vec6 {
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
        let rad = r.radius;
        let lead = move |from: Vec6, to: Vec6| {
            let k = (0..3)
                .find(|&k| (to[k] - from[k]).abs() > 1e-12)
                .unwrap_or(2);
            let dir = (to[k] - from[k]).signum();
            move |p: Vec6| {
                let mut q = [p[0], p[1], p[2]];
                if k < 2 {
                    (q[k], q[2]) = (q[k] + dir * rad, q[2] + rad);
                }
                q
            }
        };
        let mut at = r.start;
        while let Some(legs) = r.plan() {
            for leg in legs {
                match leg {
                    Leg::Rapid(to) => {
                        assert!(
                            first(at, to, &lead(at, to)).is_none(),
                            "a rapid from {at:?} to {to:?} hits"
                        );
                        at = to;
                    }
                    Leg::Guard(to) => {
                        let hit = first(at, to, &lead(at, to));
                        assert!(
                            hit.is_none(),
                            "the guarded move to {to:?} touched at {hit:?}"
                        );
                        at = to;
                    }
                    Leg::Touch(to) => {
                        at = first(at, to, &lead(at, to))
                            .unwrap_or_else(|| panic!("no contact up to {to:?}"));
                        r.touches.push(at);
                    }
                }
            }
        }
        for to in r.last() {
            assert!(
                first(at, to, &lead(at, to)).is_none(),
                "backing off to {to:?} hits"
            );
            at = to;
        }
        at
    }

    #[test]
    fn each_feature_is_found_from_where_the_tip_is() {
        use crate::post::At::{Max, Min};
        let probe = Probe {
            plate: None,
            tip: Some(4.),
            feed: 100.,
            distance: 30.,
            retract: 5.,
        };
        // A block at machine X 100…224, Y 50…134, Z -70…-30.
        let (lo, hi) = ([100., 50., -70.], [224., 134., -30.]);
        let block = |p: [f64; 3]| (0..3).all(|a| p[a] >= lo[a] && p[a] <= hi[a]);
        let found = |f: Feature, start: [f64; 3], solid: &dyn Fn([f64; 3]) -> bool| {
            let mut r = Routine::new(f, &probe, [start[0], start[1], start[2], 0., 0., 0.]);
            let end = walk_routine(&mut r, solid);
            (r.zero(), end, r.found())
        };
        let near = |a: f64, b: f64| (a - b).abs() < 1e-3;
        let (w, end, _) = found(Feature::Top, [150., 90., -20.], &block);
        assert!(near(w[2], -30.) && near(end[2], -20.), "{w:?} {end:?}");
        for (f, start, k, at) in [
            (Feature::Edge(0, true), [90., 90., -35.], 0, 100.),
            (Feature::Edge(0, false), [234., 90., -35.], 0, 224.),
            (Feature::Edge(1, true), [150., 40., -35.], 1, 50.),
            (Feature::Edge(1, false), [150., 144., -35.], 1, 134.),
        ] {
            let (w, end, s) = found(f, start, &block);
            assert!(near(w[k], at), "{f:?}: {w:?}");
            assert!(
                (0..3).all(|a| near(end[a], start[a])),
                "{f:?} ends at {end:?}"
            );
            assert!(s.contains(&format!("{}0 set on the", ["X", "Y"][k])), "{s}");
        }
        for (c, start, at) in [
            ([Min, Min], [95., 45., -35.], [100., 50.]),
            ([Max, Min], [229., 45., -35.], [224., 50.]),
            ([Min, Max], [95., 139., -35.], [100., 134.]),
            ([Max, Max], [229., 139., -35.], [224., 134.]),
        ] {
            let (w, end, s) = found(Feature::Corner(c), start, &block);
            assert!(near(w[0], at[0]) && near(w[1], at[1]), "{c:?}: {w:?}");
            assert!(
                (0..3).all(|a| near(end[a], start[a])),
                "{c:?} ends at {end:?}"
            );
            assert!(s.contains(&crate::post::corner_name(c)), "{s}");
        }
        // A 30 mm bore at X160 Y92, the tip started 5 mm left of its centre
        // and 3 mm behind it: it ends on the centre.
        let bored =
            |p: [f64; 3]| block(p) && ((p[0] - 160.).powi(2) + (p[1] - 92.).powi(2)).sqrt() >= 15.;
        let (w, end, s) = found(Feature::Bore, [155., 95., -40.], &bored);
        assert!(near(w[0], 160.) && near(w[1], 92.), "{w:?}");
        assert!(
            near(end[0], 160.) && near(end[1], 92.) && near(end[2], -40.),
            "{end:?}"
        );
        assert!(s.contains("30.000 mm across Y"), "{s}");
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
                let zero = datum(setup, stock, &mesh).unwrap();
                let found = zero_check(setup, stock, zero, &first, &[]);
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
                let e = Edges::new(origin, refs, stock, zero, &[], &probe, 4., start, z);
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

    /// A toe clamp on the +X −Y corner of the stock top, 20 mm proud, lies
    /// under the diagonal Probe centre crosses from the +X side to the −Y
    /// side, away from every column the check looks down. Given the
    /// setup's clamps, the routine travels 10 mm above the clamp, which is
    /// the top of Z; one 35 mm proud, whose top is above the top of Z,
    /// stops it before that traverse (#163).
    #[test]
    fn a_clamp_bar_on_the_stock_top_is_flown_over_or_refused() {
        use crate::{holding::HoldBox, mesh::Bounds, post::At};
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
        // The stock at machine X 100…224, Y 50…134, Z -70…-30, the probe
        // started 3 mm right of its centre and 2 mm in front, 15 mm above.
        let (lo, hi) = ([100., 50., -70.], [224., 134., -30.]);
        let start = [165., 90., -15., 0., 0., 0.];
        let clamp = |proud: f32| HoldBox {
            label: "clamp on +X (offset 0)".into(),
            bounds: Bounds {
                min: [94., 0., 40.],
                max: [124., 24., 40. + proud],
            },
        };
        // The same clamp in machine coordinates, for the harness.
        let at = |c: &HoldBox| {
            (
                [194., 50., -30.],
                [224., 74., -30. + f64::from(c.bounds.max[2] - 40.)],
            )
        };
        let edges = |holds: &[HoldBox]| {
            Edges::new(
                "Stock top center",
                [At::Mid; 2],
                stock,
                [62., 42., 40.],
                holds,
                &probe,
                4.,
                start,
                z,
            )
        };
        let c = clamp(20.);
        assert_eq!(
            crate::post::probe_height(stock, std::slice::from_ref(&c)),
            70.
        );
        let e = edges(std::slice::from_ref(&c));
        assert_eq!((e.clear, e.rise), (10., 30.));
        let e = touch_box_round(e, lo, hi, &[at(&c)]).unwrap();
        let (w, fixed) = e.zero();
        assert_eq!(fixed, [true; 3]);
        assert!(
            (0..3).all(|a| (w[a] - [162., 92., -30.][a]).abs() < 1e-9),
            "{w:?}"
        );
        // 10 mm over the clamp's top at machine Z -10: the top of Z.
        assert_eq!(e.high(w), 0.);
        assert_eq!(e.touches.iter().map(Vec::len).collect::<Vec<_>>(), [1; 5]);
        // 35 mm proud, its top at machine Z 5, above the top of Z: refused
        // before the traverse from the +X side to the -Y side.
        let c = clamp(35.);
        let why = match touch_box_round(edges(std::slice::from_ref(&c)), lo, hi, &[at(&c)]) {
            Err(why) => why,
            Ok(_) => panic!("the traverse over the clamp was not refused"),
        };
        assert!(
            why.contains(
                "cannot pass over the clamp on +X (offset 0), whose top is at machine Z 5.000, on its way to the -Y side"
            ) && why.contains("Z ends at 0.000"),
            "{why}"
        );
        // Without the clamps the routine travels 10 mm above the stock top,
        // through the 20 mm clamp: the harness fails on that traverse.
        let c = clamp(20.);
        let hit = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            touch_box_round(edges(&[]), lo, hi, &[at(&c)])
        }));
        assert!(hit.is_err());
    }

    /// `ended` (#166): the outcome of the command after `seq` once the
    /// machine has stopped. An ok outcome counts only Idle or paused, when
    /// the driver takes the SetWork that follows; E-stopped or disconnected
    /// as the probe ended, it is not ok and says so. A failed outcome keeps
    /// the driver's words.
    #[test]
    fn ended_takes_an_ok_outcome_only_idle_or_paused() {
        use uc100::controller::ProgramOutcome;
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("limited.toml");
        std::fs::write(&profile, LIMITED).unwrap();
        let mut m = Machine::new(3);
        m.p.insert("controller".into(), "UC100".into());
        m.p.insert("uc100_profile".into(), profile.display().to_string());
        m.p.insert("uc100_connection".into(), "Simulated".into());
        let mut c = Connection::open(&m, dir.path()).unwrap();
        c.snapshot.program_outcome = Some(ProgramOutcome {
            seq: 1,
            ok: true,
            message: "Probe done".into(),
        });
        c.snapshot.probe_result = Some(ProbeResult {
            seq: 1,
            tripped: true,
            position: [0.; 6],
        });
        let ended = |c: &mut Connection, state: Option<State>| {
            c.snapshot.state = state;
            c.ended((0, 0))
        };
        let (ok, message, result) = ended(&mut c, Some(State::Idle)).unwrap();
        assert!(ok && message == "Probe done" && result.is_some_and(|r| r.tripped));
        assert!(ended(&mut c, Some(State::Paused("M0".into()))).unwrap().0);
        assert_eq!(ended(&mut c, Some(State::Run)), None);
        assert_eq!(ended(&mut c, Some(State::Hold)), None);
        assert_eq!(ended(&mut c, Some(State::Stopping)), None);
        let (ok, message, result) = ended(&mut c, Some(State::EStop)).unwrap();
        assert!(!ok && result.is_none(), "{message}");
        assert_eq!(
            message,
            "The machine was E-stopped as the probe ended, so nothing is set: probe again."
        );
        let (ok, message, _) = ended(&mut c, Some(State::Jog)).unwrap();
        assert!(
            !ok && message.starts_with("The machine is Jog after"),
            "{message}"
        );
        let (ok, message, _) = ended(&mut c, None).unwrap();
        assert!(!ok && message.contains("not connected"), "{message}");
        // The driver's own words when the probe failed.
        c.snapshot.program_outcome = Some(ProgramOutcome {
            seq: 1,
            ok: false,
            message: "Program aborted by E-stop: E-stop pressed".into(),
        });
        let (ok, message, result) = ended(&mut c, Some(State::EStop)).unwrap();
        assert!(!ok && result.is_none());
        assert_eq!(message, "Program aborted by E-stop: E-stop pressed");
        // An outcome from before the command is not its.
        assert_eq!(ended(&mut c, Some(State::Idle)).map(|e| e.0), Some(false));
        assert_eq!(c.ended((1, 1)), None);
    }

    /// LIMITED's travel on fast axes, with a probe input: the simulator's
    /// block is X 145…245, Y 90…190, its top at machine Z -40.
    const PROBED: &str = r#"
[controller]
step_rate = "100kHz"
[motion]
junction_deviation = 0.02
arc_tolerance = 0.002
buffer_ms = 400
default_feed = 500
[axes.x]
steps_per_unit = 20.0
max_velocity = 60000.0
acceleration = 20000.0
step = { pin = 2 }
dir = { pin = 3 }
soft_min = 0.0
soft_max = 390.0
[axes.y]
steps_per_unit = 20.0
max_velocity = 60000.0
acceleration = 20000.0
step = { pin = 4 }
dir = { pin = 5 }
soft_min = 0.0
soft_max = 280.0
[axes.z]
steps_per_unit = 20.0
max_velocity = 30000.0
acceleration = 20000.0
step = { pin = 6 }
dir = { pin = 7 }
soft_min = -80.0
soft_max = 0.0
[inputs]
probe = { pin = 13, active_low = true }
"#;

    /// Probe Z onto the simulator's block, then an E-stop in the window
    /// between the probe's end and the tick that reads it (#166): the
    /// driver, with no program left to abort, keeps the probe's ok outcome
    /// and would drop the SetWork; the tick reports the E-stop, sets
    /// nothing and marks nothing touched off.
    #[test]
    fn a_probe_that_ended_into_an_e_stop_sets_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("probed.toml");
        std::fs::write(&profile, PROBED).unwrap();
        let mut m = Machine::new(3);
        m.p.insert("controller".into(), "UC100".into());
        m.p.insert("uc100_profile".into(), profile.display().to_string());
        m.p.insert("uc100_connection".into(), "Simulated".into());
        m.p.insert("probe_plate".into(), "10".into());
        m.p.insert("probe_feed".into(), "1000".into());
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
        let (_, top) = c.block.expect("the simulator's block");
        assert_eq!(top[2], -40.);
        // 10 mm above the block's top.
        c.send(Command::Mdi("G53 G0 X195 Y140 Z-30".into()));
        until(&mut c, 5.0, "over the block", |s| {
            s.state == Some(State::Idle) && (s.machine_pos[2] - -30.).abs() < 1e-3
        });
        assert_eq!(c.probe_blocked(), None);
        c.act(Action::ProbeZ);
        let seq = c.probing.as_ref().expect("probing").seq;
        // The driver's own snapshot, not read by a tick.
        let raw = |c: &Connection, what: &str, ok: &dyn Fn(&Snapshot) -> bool| {
            let cap = Instant::now() + Duration::from_secs(60);
            loop {
                let s = c.controller.snapshot();
                if ok(&s) {
                    return s;
                }
                assert!(
                    Instant::now() < cap,
                    "timed out waiting for {what}: {:?} {:?}",
                    s.state,
                    s.log
                );
                std::thread::sleep(Duration::from_millis(5));
            }
        };
        let s = raw(&c, "the probe to end", &|s| {
            s.state == Some(State::Idle)
                && (s.program_outcome.as_ref()).is_some_and(|o| o.seq > seq.0 && o.ok)
                && (s.probe_result.as_ref()).is_some_and(|r| r.seq > seq.1 && r.tripped)
        });
        let touched = s.probe_result.unwrap().position[2];
        assert!((touched - top[2]).abs() < 0.1, "{touched}");
        // E-stop before the tick.
        c.send(Command::EStop);
        raw(&c, "E-stop", &|s| s.state == Some(State::EStop));
        c.refresh();
        assert!(!c.probing());
        let (ok, message) = c.probed.clone().expect("the probe ended");
        assert!(!ok, "{message}");
        assert_eq!(
            message,
            "The machine was E-stopped as the probe ended, so nothing is set: probe again."
        );
        // Nothing was set: Reset, and the work offset is as it was.
        c.send(Command::Reset);
        until(&mut c, 3.0, "Idle after Reset", |s| {
            s.state == Some(State::Idle)
        });
        assert!(
            c.snapshot.work_offset[2].abs() < 1e-9,
            "{:?}",
            c.snapshot.work_offset
        );
        assert_eq!(c.probed.map(|p| p.0), Some(false));
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

    fn pinned(extra: &[(&str, &str)]) -> Fields {
        let mut m = Machine::new(3);
        for (k, v) in [
            ("pins", "0 0; 100 0"),
            ("pin_diameter", "6"),
            ("pin_height", "8"),
        ]
        .iter()
        .chain(extra)
        {
            m.p.insert(k.to_string(), v.to_string());
        }
        m.p
    }

    #[test]
    fn pin_settings_read_the_plate_and_refuse_what_does_not_place_a_part() {
        assert_eq!(pins_settings(&Machine::new(3).p), Ok(None));
        let pins = pins_settings(&pinned(&[("pins", " 10 5 ; 110,5; 10 85 ")]))
            .unwrap()
            .unwrap();
        assert_eq!(pins.at, vec![[10., 5.], [110., 5.], [10., 85.]]);
        assert_eq!((pins.diameter, pins.height, pins.reference), (6., 8., None));
        assert_eq!(pins.offset(2), [0., 80.]);
        let recorded = pins_settings(&pinned(&[("pin_ref_x", "120.5"), ("pin_ref_y", "88")]));
        assert_eq!(recorded.unwrap().unwrap().reference, Some([120.5, 88.]));
        for (fields, said) in [
            (vec![("pins", "0 0")], "at least two pins"),
            (vec![("pins", "0 0; 100")], "X Y pairs"),
            (vec![("pins", "0 0; 4 0")], "closer than their diameter"),
            (vec![("pin_diameter", "")], "Pin diameter is not set"),
            (
                vec![("pin_height", "0")],
                "Pin height must be from 1 to 100 mm.",
            ),
            (
                vec![("pin_ref_x", "1"), ("pin_ref_y", "y")],
                "record it again",
            ),
        ] {
            let e = pins_settings(&pinned(&fields)).unwrap_err();
            assert!(e.iter().any(|e| e.contains(said)), "{fields:?}: {e:?}");
        }
        // A machine's own issues say what is wrong with them.
        let mut m = Machine::new(3);
        m.p = pinned(&[("pins", "0 0")]);
        assert!(m.issues().iter().any(|e| e.contains("at least two pins")));
    }

    /// Boss centre and Pin centre touch the top, then each side at its
    /// equator below it from outside, find the centre whatever the tip's
    /// radius and where it started over the boss, and leave the tip over it.
    #[test]
    fn a_boss_or_pin_is_found_from_over_it_with_the_tips_radius() {
        let probe = Probe {
            plate: None,
            tip: Some(4.),
            feed: 100.,
            distance: 20.,
            retract: 5.,
        };
        let near = |a: f64, b: f64| (a - b).abs() < 1e-3;
        // A Ø6 pin, 8 mm tall, centred at X150 Y90 on a plate at Z-60.
        let pin = |p: [f64; 3]| {
            (p[2] <= -60.)
                || (p[2] <= -52. && ((p[0] - 150.).powi(2) + (p[1] - 90.).powi(2)).sqrt() <= 3.)
        };
        let pins = pins_settings(&pinned(&[])).unwrap().unwrap();
        for start in [[150., 90., -48.], [151.5, 89., -49.], [148.2, 91.7, -50.]] {
            let s = [start[0], start[1], start[2], 0., 0., 0.];
            let mut r = Routine::pin(&pins, &probe, s, Purpose::Work);
            // Out past both radii and as much again, the sides 3 mm down.
            assert!(
                near(r.reach, 11.) && near(r.depth, 3.),
                "{} {}",
                r.reach,
                r.depth
            );
            let end = walk_routine(&mut r, &pin);
            let w = r.zero();
            assert!(
                near(w[0], 150.) && near(w[1], 90.) && near(w[2], -52.),
                "{start:?}: {w:?}"
            );
            assert!(
                near(end[0], 150.) && near(end[1], 90.) && near(end[2], start[2]),
                "{end:?}"
            );
            // The sides were touched at the tip's equator, below the top.
            for t in &r.touches[1..] {
                assert!(near(t[2] + 2., -53.), "{t:?}");
            }
            // Across Y through the centre; across X where the tip started,
            // a chord when that was off the centre in Y.
            let s = r.found();
            let chord = 2. * (9. - (start[1] - 90f64).powi(2)).sqrt();
            assert!(s.contains("6.000 mm across Y"), "{s}");
            assert!(s.contains(&format!("{chord:.3} mm across X")), "{s}");
        }
        // A short pin is probed at half its height.
        let short = pins_settings(&pinned(&[("pin_height", "4")]))
            .unwrap()
            .unwrap();
        let s = [150., 90., -50., 0., 0., 0.];
        assert!(near(
            Routine::pin(&short, &probe, s, Purpose::Work).depth,
            2.
        ));
        // Boss centre, without pins, goes out by the maximum probe
        // distance: a Ø30 boss.
        let boss = |p: [f64; 3]| {
            (p[2] <= -60.)
                || (p[2] <= -40. && ((p[0] - 150.).powi(2) + (p[1] - 90.).powi(2)).sqrt() <= 15.)
        };
        let mut r = Routine::new(Feature::Boss, &probe, [152., 88., -35., 0., 0., 0.]);
        walk_routine(&mut r, &boss);
        let w = r.zero();
        assert!(near(w[0], 150.) && near(w[1], 90.), "{w:?}");
        assert!(r.found().contains("30.000 mm across Y"), "{}", r.found());
    }

    #[test]
    fn squareness_measures_pin_2_off_the_line_from_pin_1() {
        let pins = pins_settings(&pinned(&[])).unwrap().unwrap();
        let (ok, said) = squareness(&pins, [100., 50.], [200.02, 50.04]);
        assert!(ok, "{said}");
        assert!(
            said.starts_with("Pin 2 is 0.040 mm off the line in Y over 100.0 mm"),
            "{said}"
        );
        assert!(said.contains("+0.020 mm along it"), "{said}");
        let (ok, said) = squareness(&pins, [100., 50.], [200., 49.93]);
        assert!(
            !ok && said.contains("0.070 mm off") && said.contains("square the plate"),
            "{said}"
        );
        // Pins along Y are off the line in X.
        let along_y = pins_settings(&pinned(&[("pins", "0 0; 0 80")]))
            .unwrap()
            .unwrap();
        let (_, said) = squareness(&along_y, [0., 0.], [0.03, 80.]);
        assert!(
            said.contains("0.030 mm off the line in X over 80.0 mm"),
            "{said}"
        );
    }

    /// Opens the simulated `PROBED` machine with the pins of `pinned`,
    /// Idle with machine zero set where it stands.
    fn pinned_machine(dir: &Path) -> Connection {
        let profile = dir.join("probed.toml");
        std::fs::write(&profile, PROBED).unwrap();
        let mut m = Machine::new(3);
        m.p = pinned(&[
            ("probe_tip", "2"),
            ("probe_feed", "1000"),
            ("probe_distance", "20"),
        ]);
        m.p.insert("controller".into(), "UC100".into());
        m.p.insert("uc100_profile".into(), profile.display().to_string());
        m.p.insert("uc100_connection".into(), "Simulated".into());
        let mut c = Connection::open(&m, dir).unwrap();
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
        c
    }

    /// Runs `c` until a pin routine has ended and what it declares is sent.
    fn pinned_end(c: &mut Connection, what: &str) -> (bool, String) {
        let cap = Instant::now() + Duration::from_secs(120);
        loop {
            c.refresh();
            if c.pinning().is_none()
                && c.referencing.is_none()
                && c.declare.is_empty()
                && c.snapshot.state == Some(State::Idle)
                && let Some(said) = c.pin_said.clone()
            {
                return said;
            }
            assert!(
                Instant::now() < cap,
                "timed out waiting for {what}: {:?} {:?}",
                c.snapshot.state,
                c.snapshot.log
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Rapids `c` to machine `at` and waits there.
    fn rapid_to(c: &mut Connection, at: [f64; 3]) {
        c.send(Command::Mdi(format!(
            "G53 G0 X{:.3} Y{:.3} Z{:.3}",
            at[0], at[1], at[2]
        )));
        until(c, 10.0, "the rapid", |s| {
            s.state == Some(State::Idle) && (0..3).all(|a| (s.machine_pos[a] - at[a]).abs() < 1e-3)
        });
    }

    /// On the simulator, Record the pins' place finds pin 1 and pin 2 and
    /// keeps pin 1's place; after an E-stop and a machine zero 1.3 mm and
    /// -0.7 mm off, Reference X Y from pin 1 declares X and Y back by
    /// exactly that, also from X and Y without a machine position.
    #[test]
    fn the_pins_place_is_recorded_and_references_x_and_y_again() {
        let dir = tempfile::tempdir().unwrap();
        let mut c = pinned_machine(dir.path());
        assert!(!c.pin_referenced);
        assert_eq!(
            c.reference_blocked().as_deref(),
            Some(
                "The pins' place is not recorded yet: set machine zero, then Record the pins' place."
            )
        );
        // The simulator's pins stand in front of its block, pin 1 at X145
        // Y45, 8 mm tall on the bottom of Z.
        let pin1 = c.sim_pins.expect("simulated pins");
        assert_eq!(pin1, [145., 45.]);
        let (lo, _) = c.block.unwrap();
        assert!(pin1[1] + 3. < lo[1], "{lo:?}");
        rapid_to(&mut c, [145.6, 44.5, -69.]);
        assert_eq!(c.record_blocked(), None);
        c.act(Action::RecordPins);
        assert_eq!(c.pinning(), Some(true));
        let (ok, said) = pinned_end(&mut c, "Record the pins' place");
        assert!(ok, "{said}");
        let at = c.recorded.take().expect("pin 1's place to keep");
        assert!((0..2).all(|a| (at[a] - pin1[a]).abs() < 0.01), "{at:?}");
        assert!(
            said.contains("Pin 2 is 0.00") && said.contains("off the line in Y over 100.0 mm"),
            "{said}"
        );
        assert!(c.pin_referenced);
        let reference = match &c.pins {
            Ok(Some(p)) => p.reference.unwrap(),
            other => panic!("{other:?}"),
        };
        assert_eq!(reference, at);
        // It ends over pin 2.
        let s = c.snapshot.machine_pos;
        assert!(
            (s[0] - 245.).abs() < 0.01 && (s[1] - 45.).abs() < 0.01,
            "{s:?}"
        );

        // An E-stop clears the reference; a rough machine zero is off by
        // X +1.3 Y -0.7: the pins stay put on the table, so in the new
        // machine coordinates they are that far off.
        c.send(Command::EStop);
        until(&mut c, 3.0, "E-stop", |s| s.state == Some(State::EStop));
        assert!(!c.pin_referenced);
        c.send(Command::Reset);
        until(&mut c, 3.0, "Idle after Reset", |s| {
            s.state == Some(State::Idle)
        });
        let now = c.snapshot.machine_pos;
        c.declared(&[(0, now[0] + 1.3), (1, now[1] - 0.7), (2, now[2])]);
        until(&mut c, 3.0, "X, Y and Z declared", |s| {
            s.referenced[..3].iter().all(|r| *r)
        });
        let off = c.sim_pins.unwrap();
        assert!((off[0] - (at[0] + 1.3)).abs() < 0.01 && (off[1] - (at[1] - 0.7)).abs() < 0.01);
        rapid_to(&mut c, [off[0] - 0.8, off[1] + 0.3, -69.]);
        assert_eq!(c.reference_blocked(), None);
        c.act(Action::ReferencePins);
        let (ok, said) = pinned_end(&mut c, "Reference X Y from pin 1");
        assert!(ok, "{said}");
        assert!(
            said.contains("declared moved by -1.30") && said.contains("+0.70"),
            "{said}"
        );
        assert!(c.pin_referenced);
        let back = c.sim_pins.unwrap();
        assert!(
            (0..2).all(|a| (back[a] - at[a]).abs() < 0.01),
            "{back:?} {at:?}"
        );
        // It ends over pin 1, which is where it was recorded, once the
        // driver has taken both declarations.
        until(&mut c, 3.0, "over pin 1 where it was recorded", |s| {
            (0..2).all(|a| (s.machine_pos[a] - at[a]).abs() < 0.01)
        });

        // After another E-stop, X and Y need no machine position: Z does.
        c.send(Command::EStop);
        until(&mut c, 3.0, "E-stop", |s| s.state == Some(State::EStop));
        c.send(Command::Reset);
        until(&mut c, 3.0, "Idle after Reset", |s| {
            s.state == Some(State::Idle)
        });
        assert!(
            c.reference_blocked()
                .unwrap()
                .contains("Z has no machine position")
        );
        let z = c.snapshot.machine_pos[2];
        c.declared(&[(2, z)]);
        until(&mut c, 3.0, "Z declared", |s| s.referenced[2]);
        assert!(!c.snapshot.referenced[0]);
        assert_eq!(c.reference_blocked(), None);
        c.act(Action::ReferencePins);
        let (ok, said) = pinned_end(&mut c, "Reference X Y from pin 1 unset");
        assert!(ok, "{said}");
        assert!(c.pin_referenced && c.snapshot.referenced[..3].iter().all(|r| *r));
        let back = c.sim_pins.unwrap();
        assert!(
            (0..2).all(|a| (back[a] - at[a]).abs() < 0.01),
            "{back:?} {at:?}"
        );
        // Declaring X another way is not from the pin.
        c.act(Action::MachineZero(Some(0)));
        assert!(!c.pin_referenced);
    }
}
