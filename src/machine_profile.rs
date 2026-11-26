//! A machine's UC100 profile, its probe and fixture-pin settings, and the
//! coolants and spindle cap it allows: model-level data read from a machine's
//! fields and its profile file. It sits below the app (LIO-729): `control`
//! re-exports it for the app.
use crate::model::{Fields, Machine, get, post_setting};
use std::collections::BTreeMap;
use std::path::Path;
use uc100::config::MachineConfig;

/// The least room between the top of Z's travel and the largest part on A
/// for a safe retract over it (mm, LIO-191).
pub const RETRACT_ROOM: f64 = 5.;

/// The machine field that says how the centreline was found: empty while it
/// is assumed, `MEASURED` once probed and saved, or why it went stale.
pub const CENTRE_MARK: &str = "a_centre_measured";
/// The machine field value for a centreline probed and saved (`CENTRE_MARK`).
pub const MEASURED: &str = "measured";

/// Loads a machine's UC100 profile and checks it with the driver.
pub fn profile(m: &Machine) -> Result<MachineConfig, String> {
    let path = get(&m.p, "uc100_profile").trim();
    if path.is_empty() {
        return Err("Choose the machine's UC100 profile file.".into());
    }
    MachineConfig::load(Path::new(path)).map_err(|e| e.to_string())
}

/// A UC100 machine's X, Y and Z travel are its profile's soft limits, given
/// once (LIO-344): their spans go into machine `id` in the library and in
/// each setup's copy, which a run checks against them (`travel_into`).
/// Whether any changed.
pub fn travel_from_profile(
    doc: &mut crate::model::Document,
    id: &str,
    cfg: &MachineConfig,
) -> bool {
    let setups = doc.setups.iter_mut().map(|s| &mut s.machine);
    let mut changed = false;
    for m in doc.machines.iter_mut().chain(setups) {
        changed |= travel_into(m, id, cfg);
    }
    changed
}

/// Machine `m`, when it is the UC100 machine `id`, given its profile's soft
/// limit spans as X, Y and Z travel, marked Measured where they change. An
/// axis without both limits keeps its travel. Whether any changed.
pub fn travel_into(m: &mut Machine, id: &str, cfg: &MachineConfig) -> bool {
    if m.id != id || get(&m.p, "controller") != "UC100" {
        return false;
    }
    let mut changed = false;
    for (k, key) in ["x", "y", "z"].into_iter().enumerate() {
        let Some(span) = cfg
            .axes
            .get(k)
            .and_then(|a| Some(a.soft_max? - a.soft_min?))
        else {
            continue;
        };
        if (crate::post_check::machine_mm(m, key) as f64 - span).abs() <= 0.01 {
            continue;
        }
        let v = if get(&m.p, "units") == "inch" {
            span / 25.4
        } else {
            span
        };
        m.p.insert(key.into(), v.to_string());
        m.p.insert(format!("{key}_source"), crate::envelope::MEASURED.into());
        changed = true;
    }
    changed
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
    pub(crate) static CAP_READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
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
    crate::envelope::jingyan(&mut m);
    Ok(m)
}

/// The A axis added to the owner's JY5800 profile for the machine with the
/// rotary unit (LIO-184): the profile's own text, `[axes.a]` after it. The
/// pins are the driver's free step/dir pair 8 and 9 and the rest is the
/// driver's sample A axis: ASSUMED, none of it wired or measured yet.
pub(crate) const ROTARY_AXIS: &str = "
[axes.a]
# ASSUMED (LIO-184), NOT WIRED OR MEASURED: pins 8 and 9 are the driver's free
# step/dir pair, 44.444 steps per degree is its sample A axis (16000 per turn)
# and the speeds are its sample's. Measure all of them on the rotary unit
# (LIO-186) and correct them. The soft limits are the machine's rotary range
# in AXIS CAM, -180 to 180 degrees.
steps_per_unit = 44.444
max_velocity = 3600
acceleration = 360
step = { pin = 8, active_low = true }
dir = { pin = 9, active_low = false }
reversed = false
soft_min = -180.0
soft_max = 180.0
";

/// The owner's UC100 profile with the rotary unit's A axis: `jy5800.toml`
/// and `[axes.a]`, added the way the Setup form adds it (LIO-183): the
/// pinout and steps-per-unit checks of `[commissioning]` were made on X, Y
/// and Z, so they are unticked, and Reset stays refused until A's pins and
/// steps per degree are checked in the form.
pub fn rotary_profile() -> String {
    let mut d = crate::profile_draft::Draft::from_text(
        String::new(),
        "jy5800-rotary.toml".into(),
        include_str!("../assets/jy5800.toml").into(),
    )
    .expect("the shipped profile is TOML");
    d.edit(crate::profile_draft::Edit::AddAxis("a".into()));
    d.to_text()
}

/// `rotary_profile` before it unticked the checks: a copy left unedited on
/// disk is replaced, since it would let A move on assumed values.
fn rotary_profile_ungated() -> String {
    format!("{}{ROTARY_AXIS}", include_str!("../assets/jy5800.toml"))
}

/// The Jingyan CNC4030 with a rotary unit on the table (LIO-184): the owner's
/// machine as 4-axis "Indexed (3+1)", A about X, on its own copy of the JY5800
/// profile with `[axes.a]`, `jy5800-rotary.toml` in `dir` (kept if it is
/// there). The rotary geometry is the assumed default of `ROTARY_GEOMETRY`.
/// The owner's 3-axis machine stays what it is.
pub fn rotary_machine(dir: &Path) -> std::io::Result<Machine> {
    let path = dir.join("jy5800-rotary.toml");
    if std::fs::read_to_string(&path).is_ok_and(|t| t == rotary_profile_ungated()) {
        std::fs::remove_file(&path)?;
    }
    if !path.exists() {
        std::fs::create_dir_all(dir)?;
        std::fs::write(&path, rotary_profile())?;
    }
    let mut m = Machine::new(4);
    for (key, value) in [
        ("name", "Jingyan CNC4030 + rotary"),
        ("controller", "UC100"),
        ("x", "390"),
        ("y", "280"),
        ("z", "55"),
        ("rpm", "24000"),
        ("feed", "1000"),
    ] {
        m.p.insert(key.into(), value.into());
    }
    // The pivot is the centreline Machine::new left at its own guess.
    for (key, from) in [("pivot_y", "a_centre_y"), ("pivot_z", "a_centre_z")] {
        let value = get(&m.p, from).to_string();
        m.p.insert(key.into(), value);
    }
    m.p.insert("uc100_profile".into(), path.display().to_string());
    crate::envelope::jingyan(&mut m);
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
    /// The probe's length from the spindle nose to its tip (LIO-207), for
    /// the checks' estimate of where a tool of another length puts the
    /// spindle on an A centreline probed with it; None until it is set.
    pub length: Option<f64>,
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
            _ if text.is_empty() && matches!(key, "probe_plate" | "probe_tip" | "probe_length") => {
                None
            }
            _ => {
                e.push(format!("{what} must be from {lo} to {hi} {unit}."));
                None
            }
        }
    };
    let probe = Probe {
        plate: value("probe_plate", "Touch plate thickness", 0., 100., "mm"),
        tip: value("probe_tip", "Probe tip diameter", 0., 50., "mm"),
        length: value("probe_length", "Probe length", 0., 300., "mm"),
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
