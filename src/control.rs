//! The connection to a UC100 machine controller: the driver's `Controller`
//! for one machine of the project, real (USB) or simulated.
use crate::model::{Machine, get};
use std::collections::VecDeque;
use std::path::Path;
use std::time::{Duration, Instant};
use uc100::config::MachineConfig;
use uc100::controller::{Command, Controller, Snapshot, State};
use uc100::gcode::Spindle;

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

/// How often a held jog is sent again: the driver ends a jog that is not
/// refreshed within 400 ms, so a lost release cannot run an axis away.
const JOG_RESEND: Duration = Duration::from_millis(100);
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
}

/// The continuous jog held down.
struct Held {
    axis: usize,
    negative: bool,
    sent: Instant,
    /// The driver has reported Jog since the press.
    started: bool,
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
            held: None,
            jog_step: None,
            jog_speed: 50.,
            feed_override: 100.,
            rpm: String::new(),
            mdi: String::new(),
            run: None,
        })
    }

    pub fn send(&self, command: Command) {
        self.controller.send(command)
    }

    pub fn refresh(&mut self) {
        self.snapshot = self.controller.snapshot();
        if let Some(run) = &mut self.run {
            run.refresh(self.snapshot.program_outcome.as_ref());
        }
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
                if h.sent.elapsed() >= JOG_RESEND {
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
            _ => {}
        }
        self.run = Some(run);
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
