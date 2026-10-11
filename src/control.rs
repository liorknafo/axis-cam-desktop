//! The connection to a UC100 machine controller: the driver's `Controller`
//! for one machine of the project, real (USB) or simulated.
use crate::model::{Machine, get};
use std::collections::VecDeque;
use std::path::Path;
use uc100::config::MachineConfig;
use uc100::controller::{Command, Controller, Snapshot, State};

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
        Ok(Self {
            _nap: uc100::macos::prevent_app_nap("AXIS CAM machine control"),
            controller: Controller::start(cfg.clone(), Some(file)),
            config: cfg,
            machine_id: m.id.clone(),
            simulated,
            snapshot: Snapshot::default(),
            declare: VecDeque::new(),
        })
    }

    pub fn send(&self, command: Command) {
        self.controller.send(command)
    }

    pub fn refresh(&mut self) {
        self.snapshot = self.controller.snapshot();
        // Any state but Idle (E-stop, a move) drops what is left, so nothing
        // is declared later by surprise.
        if self.idle() {
            if let Some((axis, value)) = self.declare.pop_front() {
                self.send(Command::SetMachine { axis, value });
            }
        } else {
            self.declare.clear();
        }
    }

    /// Does what the page asks, if the machine can; otherwise nothing.
    pub fn act(&mut self, action: Action) {
        match action {
            Action::Home if self.idle() && self.home_switches() => self.send(Command::Home(vec![])),
            Action::MachineZero(axis) if self.idle() => {
                let axes = axis.map_or(0..3, |a| a..a + 1);
                for a in axes.filter(|&a| self.config.axis_enabled(a)) {
                    self.declare.push_back((a, 0.0));
                }
            }
            Action::KeepPosition if self.idle() => {
                for a in (0..6).filter(|&a| self.config.axis_enabled(a)) {
                    self.declare.push_back((a, self.snapshot.machine_pos[a]));
                }
            }
            _ => {}
        }
    }

    pub fn idle(&self) -> bool {
        self.snapshot.state == Some(State::Idle)
    }

    /// Jog, work zero, MDI and the spindle are allowed: Idle, or paused in a program.
    pub fn manual(&self) -> bool {
        matches!(self.snapshot.state, Some(State::Idle | State::Paused(_)))
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

#[cfg(test)]
mod tests {
    use super::*;

    fn uc100(profile: &str, connection: &str) -> Machine {
        let mut m = Machine::new(3);
        m.p.insert("controller".into(), "UC100".into());
        m.p.insert("uc100_profile".into(), profile.into());
        m.p.insert("uc100_connection".into(), connection.into());
        m
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
