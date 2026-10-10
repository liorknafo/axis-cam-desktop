# UC100 machine control, part 1 (AXIS CAM side) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** AXIS CAM connects to a UC100 (real over USB, or the driver's simulator), shows the machine's live state on a new Control page, and offers Reset and an always-reachable E-stop.

**Architecture:** AXIS CAM depends on the `uc100` crate (git, pinned commit, no GUI). A UC100 machine profile names the driver's TOML profile and a Connection (USB/Simulated). A new `src/control.rs` owns one driver `Controller`; the app copies its snapshot on the existing 33 ms tick, and the Control page draws from that copy. E-stop is a top-bar button on every page while connected and the Esc key.

**Tech Stack:** Rust 2024, Iced 0.14, `uc100` crate from `github.com/liorknafo/UC100-driver` (`default-features = false`), rfd 0.15, tempfile (tests).

**Spec:** `docs/superpowers/specs/2026-10-11-uc100-foundation-design.md` (§2–§4 are this plan).

**Repository and branch:** `C:\Users\liork\.t3\worktrees\axis-cam-desktop\t3-1a348eb5`, branch `t3/uc100-controller-integration`.

**Two phases.** Tasks 1–2 use driver commit `cac67e22412468d24aa3c2f8757b2a16726a08e8` (optional GUI, no simulator yet) and can start now. Tasks 3–5 need the simulator: stop after Task 2, report, and wait for the orchestrator's message with the driver commit to pin.

## Global Constraints

- Never connect to real hardware from tests. Every connected test uses `uc100_connection = "Simulated"`.
- The `uc100` dependency is always `default-features = false` (no egui in AXIS CAM).
- No GitHub token in any file; cargo fetches the private repo through git (`[net] git-fetch-with-cli = true`).
- Field keys: `controller` value `"UC100"`, `uc100_profile` (path), `uc100_connection` (`"USB"` | `"Simulated"`; empty means Simulated, except USB on macOS).
- State files: `<workspace dir>/uc100/<machine id>.toml` (USB) and `<machine id>-simulated.toml` (simulator), where `<workspace dir>` is `App::state_path.parent()`.
- Existing behaviour unchanged: LinuxCNC machines, G-code export, every existing test.
- Run `cargo test` before each commit; all tests pass. Commit after each task, messages in the repo's style (a plain sentence, e.g. "The vise closes on the material in its grip band"), ending with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus

- A UC100 machine whose profile file was moved or deleted after saving: Connect must show the driver's "cannot read …" message, not panic (pinned in Task 3, `connect_reports_a_missing_profile_and_needs_a_uc100_machine`).
- Esc pressed with no machine connected must do nothing (pinned in Task 3, same test).
- Closing the app while connected must stop the machine (pinned in Task 3, `closing_the_app_disconnects_the_machine`).
- A simulated session must not overwrite the real machine's saved offsets (pinned in Task 3, `connects_to_the_simulated_uc100_resets_and_e_stops`, state-file assertions).
- Editing the connected machine's profile while connected must not be possible (pinned in Task 4, render test reaches the locked wizard state; the lock is in the view code).

---

### Task 1: The `uc100` dependency and the profile helpers

**Files:**
- Modify: `Cargo.toml` (`[dependencies]`)
- Create: `.cargo/config.toml`
- Create: `src/control.rs`
- Modify: `src/main.rs:1-36` (module list: add `mod control;` in alphabetical order, after `mod choice_help;`)

**Interfaces:**
- Produces:
  - `control::simulated(m: &Machine) -> bool`
  - `control::connection_label(m: &Machine) -> &'static str` (`"Simulated"` or `"USB"`)
  - `control::profile(m: &Machine) -> Result<uc100::config::MachineConfig, String>`

- [ ] **Step 1: Add the dependency and the cargo setting**

In `Cargo.toml` `[dependencies]`, add after `fidget`:

```toml
uc100 = { git = "https://github.com/liorknafo/UC100-driver.git", rev = "cac67e22412468d24aa3c2f8757b2a16726a08e8", default-features = false }
```

Create `.cargo/config.toml`:

```toml
# The UC100 driver is a private GitHub repository: fetch it with git, which
# already has the login, rather than cargo's built-in client.
[net]
git-fetch-with-cli = true
```

Run: `cargo build`
Expected: builds; `cargo tree -e normal | grep -ciE "egui|eframe"` prints `0`.

- [ ] **Step 2: Write the failing tests**

Create `src/control.rs`:

```rust
//! The connection to a UC100 machine controller: the driver's `Controller`
//! for one machine of the project, real (USB) or simulated.
use crate::model::{Machine, get};
use std::path::Path;
use uc100::config::MachineConfig;

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
```

Add `mod control;` to `src/main.rs` after `mod choice_help;`.

- [ ] **Step 3: Run them to see them fail**

Run: `cargo test control::`
Expected: compile errors, `cannot find function 'simulated'` / `'profile'`.

- [ ] **Step 4: Implement the helpers**

In `src/control.rs`, above the tests:

```rust
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
```

- [ ] **Step 5: Run the tests**

Run: `cargo test control::` then `cargo test`
Expected: both new tests pass; the whole suite passes. (`dead_code` warnings for these functions are expected until Task 2 uses them; do not add `allow`s.)

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock .cargo/config.toml src/control.rs src/main.rs
git commit -m "AXIS CAM reads UC100 machine profiles with the UC100 driver" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: UC100 machines in the machine wizard

**Files:**
- Modify: `src/model.rs:260-284` (`Machine::issues`)
- Modify: `src/ui.rs:2387-2400` (wizard step 3, "Controller")
- Modify: `src/main.rs` (`Message` enum near `SaveMachine`; `update` arms)
- Modify: `src/choice_help.rs:~376` (body match) and its tests (~line 704)
- Test: `src/smoke.rs` (`mod tests`)

**Interfaces:**
- Consumes: `control::{connection_label, profile}` (Task 1).
- Produces: `Message::BrowseUc100Profile`, `Message::Uc100ProfilePicked(Option<std::path::PathBuf>)`.

- [ ] **Step 1: Write the failing tests**

In `src/smoke.rs` `mod tests`, add:

```rust
    #[test]
    fn a_uc100_machine_needs_its_profile_file() {
        let mut m = Machine::new(3);
        m.p.insert("controller".into(), "UC100".into());
        assert!(m.issues().iter().any(|e| e.contains("UC100 profile")));
        m.p.insert("uc100_profile".into(), "C:/machines/jy5800.toml".into());
        assert!(m.issues().is_empty());
        assert!(Machine::new(3).issues().is_empty());
    }

    #[test]
    fn the_wizard_shows_the_uc100_fields_and_takes_a_picked_profile() {
        let dir = tempfile::tempdir().unwrap();
        let mut a = App::from_workspace(dir.path().join("workspace.json"), None);
        let _ = a.update(Message::NewMachine);
        let _ = a.update(Message::WizardStep(3));
        let _ = a.update(Message::Edit(Target::Machine, "controller", "UC100".into()));
        let _view = a.view();
        let profile = dir.path().join("jy5800.toml");
        std::fs::write(&profile, uc100::config::TEMPLATE).unwrap();
        let _ = a.update(Message::Uc100ProfilePicked(Some(profile.clone())));
        assert_eq!(get(&a.machine_draft.p, "uc100_profile"), profile.display().to_string());
        let _view = a.view();
        // Cancelling the file dialog keeps the chosen profile.
        let _ = a.update(Message::Uc100ProfilePicked(None));
        assert_eq!(get(&a.machine_draft.p, "uc100_profile"), profile.display().to_string());
    }
```

In `src/choice_help.rs` tests, next to the existing `controller` assertions (~line 704), add:

```rust
        assert!(
            selected("controller", "Controller", "UC100", Target::Machine, &p)
                .body
                .contains("Control page")
        );
        assert!(
            selected("uc100_connection", "Connection", "Simulated", Target::Machine, &p)
                .body
                .contains("no hardware")
        );
        assert!(
            selected("uc100_connection", "Connection", "USB", Target::Machine, &p)
                .body
                .contains("E-stop")
        );
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test a_uc100_machine_needs_its_profile_file the_wizard_shows_the_uc100_fields` (two runs, one per name) and `cargo test choice_help`
Expected: compile error for `Message::Uc100ProfilePicked`; once it compiles, the issue and help assertions fail.

- [ ] **Step 3: Require the profile in `Machine::issues`**

In `src/model.rs`, in `Machine::issues`, before the final `e`:

```rust
        if get(&self.p, "controller") == "UC100" && get(&self.p, "uc100_profile").trim().is_empty() {
            e.push("Choose the machine's UC100 profile file.".into())
        }
```

- [ ] **Step 4: Add the messages and their handlers**

In `src/main.rs` `enum Message`, after `DeleteMachine(String),`:

```rust
    /// Opens a file dialog for the machine draft's UC100 profile.
    BrowseUc100Profile,
    /// The chosen UC100 profile; None when the dialog was cancelled.
    Uc100ProfilePicked(Option<PathBuf>),
```

In `update`, next to `Message::WizardStep(s) => …`:

```rust
            Message::BrowseUc100Profile => {
                return Task::perform(
                    async {
                        rfd::AsyncFileDialog::new()
                            .set_title("Choose the UC100 profile")
                            .add_filter("UC100 profile", &["toml"])
                            .pick_file()
                            .await
                            .map(|f| f.path().to_path_buf())
                    },
                    Message::Uc100ProfilePicked,
                );
            }
            Message::Uc100ProfilePicked(path) => {
                if let Some(path) = path {
                    return self.update(Message::Edit(
                        Target::Machine,
                        "uc100_profile",
                        path.display().to_string(),
                    ));
                }
            }
```

- [ ] **Step 5: Show the UC100 fields in wizard step 3**

In `src/ui.rs` `fn wizard`, step `3`:

1. In the controller options, add `"UC100"` after `"LinuxCNC"`.
2. Change that step's note to: `"LinuxCNC has a 3-axis postprocessor. AXIS CAM drives a UC100 machine itself. Other controllers are saved for planning; exporting G-code for them is refused."`
3. After the `if get(p, "controller") == "LinuxCNC" { … }` block, add:

```rust
            if get(p, "controller") == "UC100" {
                // ponytail: reads the profile on each redraw of this step; cache it if that ever shows.
                let check: Element<_> = match crate::control::profile(m) {
                    Ok(_) => muted("UC100 profile checked.").into(),
                    Err(e) => warnings(vec![e]),
                };
                form = form
                    .push(bold("UC100", 15))
                    .push(
                        row![
                            field(p, "uc100_profile", "UC100 profile file", t),
                            btn("Browse…", Message::BrowseUc100Profile, false)
                        ]
                        .spacing(8)
                        .align_y(Alignment::End),
                    )
                    .push(choice_value(
                        p,
                        crate::control::connection_label(m),
                        "uc100_connection",
                        "Connection",
                        &["USB", "Simulated"],
                        t,
                    ))
                    .push(check)
                    .push(note("AXIS CAM drives this machine itself from the Control page. The profile is the UC100 driver's machine file: pins, steps per mm, speeds, limits and homing."));
            }
```

- [ ] **Step 6: Explain the choices**

In `src/choice_help.rs`, in the `body = match key { … }` block, before the generic `"controller" => format!(…)` arm:

```rust
            "controller" if value == "UC100" => "AXIS CAM drives this machine itself through the UC100 driver: connect, reset and E-stop it on the Control page. G-code export is not offered for it.".to_string(),
            "uc100_connection" if value == "Simulated" => "Connect to a simulated UC100 inside AXIS CAM: the driver's controller, planner and checks run, but no hardware is opened and nothing moves. Its saved offsets and position are kept apart from the real machine's.".to_string(),
            "uc100_connection" => "Connect to the real UC100 over USB (macOS). The machine starts in E-stop; press Reset on the Control page to enable it.".to_string(),
```

- [ ] **Step 7: Run the tests**

Run: `cargo test`
Expected: all pass, including the three new ones.

- [ ] **Step 8: Commit, then stop and report**

```bash
git add src/model.rs src/ui.rs src/main.rs src/choice_help.rs src/smoke.rs
git commit -m "A machine can use the UC100 controller with its driver profile" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

Stop here. Report Tasks 1–2 and wait for the orchestrator's message with the driver commit for Task 3.

---

### Task 3: Connect, Reset, E-stop, Disconnect (needs the driver simulator)

**Files:**
- Modify: `Cargo.toml` (`uc100` rev)
- Modify: `src/control.rs` (add `Connection`)
- Modify: `src/main.rs` (`App` fields and constructor, `Message`, `update`, `Tick`, `subscription`, `request_close`, helpers)
- Test: `src/smoke.rs`

**Interfaces:**
- Consumes: driver `uc100::controller::{Command, Controller, Snapshot}`, `uc100::macos::{prevent_app_nap, ActivityToken}`, `uc100::config::ControllerConfig::simulated` (driver Tasks 2–3).
- Produces:
  - `control::Connection { pub machine_id: String, pub simulated: bool, pub snapshot: Snapshot, .. }` with `open(m: &Machine, state_dir: &Path) -> Result<Connection, String>`, `send(&self, Command)`, `refresh(&mut self)`, `close(self)`.
  - `App::control: Option<control::Connection>`, `App::control_machine: Option<String>`.
  - `App::control_target(&self) -> Option<&Machine>`, `App::disconnect_machine(&mut self)`.
  - `Message::{ControlMachine(String), Connect, Disconnect, MachineReset, MachineEStop}`.

- [ ] **Step 1: Pin the driver with the simulator**

In `Cargo.toml`, set the `uc100` `rev` to the full commit hash from the orchestrator's message. Run `cargo build`. Expected: builds; `uc100::config::ControllerConfig` has a `simulated` field.

- [ ] **Step 2: Write the failing tests**

In `src/smoke.rs` `mod tests`:

```rust
    /// An app with one simulated UC100 machine whose profile is the driver's template.
    fn uc100_app(dir: &std::path::Path) -> App {
        let mut a = App::from_workspace(dir.join("workspace.json"), None);
        let profile = dir.join("uc100.toml");
        std::fs::write(&profile, uc100::config::TEMPLATE).unwrap();
        let mut m = Machine::new(3);
        m.p.insert("name".into(), "JY5800".into());
        m.p.insert("controller".into(), "UC100".into());
        m.p.insert("uc100_profile".into(), profile.display().to_string());
        m.p.insert("uc100_connection".into(), "Simulated".into());
        a.doc.machines.push(m);
        a
    }

    /// Ticks until the machine snapshot satisfies `ok`; fails after `secs`.
    fn tick_until(a: &mut App, secs: f64, what: &str, ok: impl Fn(&uc100::controller::Snapshot) -> bool) {
        let end = std::time::Instant::now() + std::time::Duration::from_secs_f64(secs);
        loop {
            let _ = a.update(Message::Tick);
            let s = &a.control.as_ref().expect("connected").snapshot;
            if ok(s) {
                return;
            }
            assert!(std::time::Instant::now() < end, "timed out waiting for {what}: {:?} {:?}", s.state, s.log);
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }

    #[test]
    fn connects_to_the_simulated_uc100_resets_and_e_stops() {
        use uc100::controller::State;
        let dir = tempfile::tempdir().unwrap();
        let mut a = uc100_app(dir.path());
        let _ = a.update(Message::Connect);
        assert!(a.control.as_ref().is_some_and(|c| c.simulated));
        tick_until(&mut a, 3.0, "E-stop after connecting", |s| s.state == Some(State::EStop));
        let identity = a.control.as_ref().unwrap().snapshot.identity.clone().unwrap();
        assert_eq!(identity.hardware_id.as_deref(), Some("SIMULATED"));
        let _ = a.update(Message::MachineReset);
        tick_until(&mut a, 3.0, "Idle after Reset", |s| s.state == Some(State::Idle));
        let _ = a.update(Message::MachineEStop);
        tick_until(&mut a, 2.0, "E-stop", |s| s.state == Some(State::EStop));
        let _ = a.update(Message::Disconnect);
        assert!(a.control.is_none());
        // The simulator keeps its own saved state; the real machine's file is untouched.
        let id = a.doc.machines[0].id.clone();
        let state = dir.path().join("uc100");
        assert!(state.join(format!("{id}-simulated.toml")).exists());
        assert!(!state.join(format!("{id}.toml")).exists());
    }

    #[test]
    fn connect_reports_a_missing_profile_and_needs_a_uc100_machine() {
        let dir = tempfile::tempdir().unwrap();
        let mut empty = App::from_workspace(dir.path().join("empty.json"), None);
        let _ = empty.update(Message::Connect);
        assert!(empty.control.is_none() && empty.status.contains("UC100 machine"));
        // Esc without a connection does nothing.
        let _ = empty.update(Message::MachineEStop);
        assert!(empty.control.is_none());
        let mut a = uc100_app(dir.path());
        std::fs::remove_file(dir.path().join("uc100.toml")).unwrap();
        let _ = a.update(Message::Connect);
        assert!(a.control.is_none());
        assert!(a.status.contains("cannot read"), "{}", a.status);
    }

    #[test]
    fn closing_the_app_disconnects_the_machine() {
        use uc100::controller::State;
        let dir = tempfile::tempdir().unwrap();
        let mut a = uc100_app(dir.path());
        let _ = a.update(Message::Connect);
        tick_until(&mut a, 3.0, "E-stop after connecting", |s| s.state == Some(State::EStop));
        let _ = a.request_close(iced::window::Id::unique());
        assert!(a.control.is_none());
    }
```

- [ ] **Step 3: Run them to see them fail**

Run: `cargo test connects_to_the_simulated_uc100`
Expected: compile errors (`no variant Connect`, `no field control`).

- [ ] **Step 4: Add `Connection` to `src/control.rs`**

Add the imports `use uc100::controller::{Command, Controller, Snapshot};` and, after `profile`:

```rust
/// A running driver controller for one machine.
pub struct Connection {
    controller: Controller,
    pub machine_id: String,
    pub simulated: bool,
    /// The driver's state at the last tick.
    pub snapshot: Snapshot,
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
            controller: Controller::start(cfg, Some(file)),
            machine_id: m.id.clone(),
            simulated,
            snapshot: Snapshot::default(),
        })
    }

    pub fn send(&self, command: Command) {
        self.controller.send(command)
    }

    pub fn refresh(&mut self) {
        self.snapshot = self.controller.snapshot();
    }

    /// Disables the machine and stops the controller thread.
    pub fn close(self) {
        let Connection { controller, .. } = self;
        controller.send(Command::Disconnect);
        controller.shutdown();
    }
}
```

- [ ] **Step 5: App state, messages and handlers**

In `src/main.rs`:

1. `pub struct App`: add

```rust
    /// The machine controller, while connected.
    pub control: Option<control::Connection>,
    /// The UC100 machine chosen on the Control page (machine id).
    pub control_machine: Option<String>,
```

and initialise both to `None` in the `Self { … }` of `from_workspace`.

2. `enum Message`: after `Uc100ProfilePicked(Option<PathBuf>),` add

```rust
    /// Chooses the UC100 machine for the Control page (machine id).
    ControlMachine(String),
    Connect,
    Disconnect,
    MachineReset,
    MachineEStop,
```

3. In `impl App`, next to `request_close`:

```rust
    /// The machine the Control page connects to: the chosen UC100 machine, or the first one.
    pub fn control_target(&self) -> Option<&Machine> {
        let uc100 = |m: &&Machine| get(&m.p, "controller") == "UC100";
        self.doc
            .machines
            .iter()
            .filter(uc100)
            .find(|m| self.control_machine.as_ref() == Some(&m.id))
            .or_else(|| self.doc.machines.iter().find(uc100))
    }

    /// Disables the machine and stops its controller, if connected.
    pub fn disconnect_machine(&mut self) {
        if let Some(c) = self.control.take() {
            c.close();
            self.status = "Disconnected from the machine.".into();
        }
    }
```

4. `update`, new arms next to the Task 2 ones:

```rust
            Message::ControlMachine(id) => {
                if self.control.is_none() {
                    self.control_machine = Some(id);
                }
            }
            Message::Connect => {
                if self.control.is_some() {
                    return Task::none();
                }
                let Some(m) = self.control_target().cloned() else {
                    self.status = "Add a UC100 machine in the Machine library first.".into();
                    return Task::none();
                };
                let dir = self.state_path.parent().unwrap().join("uc100");
                match control::Connection::open(&m, &dir) {
                    Ok(c) => {
                        self.status = format!("Connecting to {}…", m.name());
                        self.control = Some(c);
                    }
                    Err(e) => self.status = format!("Cannot connect to {}: {e}", m.name()),
                }
            }
            Message::Disconnect => self.disconnect_machine(),
            Message::MachineReset => {
                if let Some(c) = &self.control {
                    c.send(uc100::controller::Command::Reset);
                }
            }
            Message::MachineEStop => {
                if let Some(c) = &self.control {
                    c.send(uc100::controller::Command::EStop);
                }
            }
```

5. `Message::Tick => {` — make the first statement:

```rust
                if let Some(c) = &mut self.control {
                    c.refresh();
                }
```

6. `fn subscription`: the interval condition becomes `if self.playing || self.help_animating || self.control.is_some()`, and add this arm to the `listen_with` match before `_ => None`:

```rust
                // E-stop from any page, even while typing; ignored when not connected.
                iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                    key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape),
                    ..
                }) => Some(Message::MachineEStop),
```

7. `fn request_close`: make `self.disconnect_machine();` its first statement.

- [ ] **Step 6: Run the tests**

Run: `cargo test connects_to_the_simulated_uc100 connect_reports_a_missing_profile closing_the_app_disconnects` (one name per run) then `cargo test`
Expected: all pass. The connected tests run in real time (a few seconds each).

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock src/control.rs src/main.rs src/smoke.rs
git commit -m "AXIS CAM connects to a UC100, resets it and E-stops it" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: The Control page and the top-bar E-stop

**Files:**
- Modify: `src/main.rs:45-56` (`enum Page`: add `Control`)
- Modify: `src/ui.rs` (top bar ~line 285, page match ~line 314, wizard UC100 block from Task 2, new `fn control` and `fn estop`)
- Test: `src/smoke.rs`

**Interfaces:**
- Consumes: Task 3's `App::{control, control_target}`, `control::Connection::{snapshot, simulated, machine_id}`, `Message::{ControlMachine, Connect, Disconnect, MachineReset, MachineEStop}`.

- [ ] **Step 1: Write the failing test**

In `src/smoke.rs` `mod tests`, add `Page::Control` to the page list of `first_launch_and_empty_project_roundtrip`, and add:

```rust
    #[test]
    fn the_control_page_renders_in_every_state() {
        use uc100::controller::State;
        let dir = tempfile::tempdir().unwrap();
        let mut empty = App::from_workspace(dir.path().join("empty.json"), None);
        empty.page = Page::Control;
        let _view = empty.view();
        let mut a = uc100_app(dir.path());
        a.page = Page::Control;
        let _view = a.view();
        let _ = a.update(Message::Connect);
        let _view = a.view();
        tick_until(&mut a, 3.0, "E-stop after connecting", |s| s.state == Some(State::EStop));
        let _view = a.view();
        let _ = a.update(Message::MachineReset);
        tick_until(&mut a, 3.0, "Idle after Reset", |s| s.state == Some(State::Idle));
        for page in [Page::Home, Page::Machines, Page::Control] {
            a.page = page;
            let _view = a.view();
        }
        // The connected machine's UC100 fields are locked in the wizard.
        let id = a.doc.machines[0].id.clone();
        let _ = a.update(Message::EditMachine(id));
        let _ = a.update(Message::WizardStep(3));
        let _view = a.view();
        let _ = a.update(Message::Disconnect);
    }
```

- [ ] **Step 2: Run it to see it fail**

Run: `cargo test the_control_page_renders_in_every_state`
Expected: compile error, `no variant named Control`.

- [ ] **Step 3: Add the page, the button and the E-stop**

1. `src/main.rs` `enum Page`: add `Control,` after `Machines,`.
2. `src/ui.rs` top bar: after the `"Machine library"` button add `btn("Control", Message::Page(Page::Control), app.page == Page::Control),`. Then, where the top-bar `row![…]` is built, append the E-stop while connected. Bind the row first and push conditionally:

```rust
    let mut top_row = row![ /* the existing buttons, unchanged, plus Control */ ]
        .spacing(5)
        .align_y(Alignment::Center);
    if app.control.is_some() {
        top_row = top_row.push(estop());
    }
```

and use `top_row` where the `row![…]` expression was.
3. `src/ui.rs` page match: add `Page::Control => control(app),`.
4. `src/ui.rs`, new functions after `fn wizard`:

```rust
/// The red E-stop: in the top bar on every page while connected, and on the Control page.
fn estop<'a>() -> Element<'a, Message> {
    button(bold("E-STOP", 14).color(Color::WHITE))
        .on_press(Message::MachineEStop)
        .padding([8, 16])
        .style(|_, status| button::Style {
            background: Some(
                if matches!(status, button::Status::Hovered | button::Status::Pressed) {
                    Color::from_rgb8(160, 20, 20)
                } else {
                    Color::from_rgb8(200, 30, 30)
                }
                .into(),
            ),
            text_color: Color::WHITE,
            border: Border {
                radius: 4.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .into()
}

/// The Control page: the UC100 machine, its connection and its live state.
fn control(app: &App) -> Element<'_, Message> {
    use uc100::{controller::State, gcode::Spindle};
    let target = app.control_target();
    let mut left = column![bold("Machine", 18)].spacing(10).width(300);
    let machines: Vec<&Machine> = app
        .doc
        .machines
        .iter()
        .filter(|m| get(&m.p, "controller") == "UC100")
        .collect();
    if machines.is_empty() {
        left = left
            .push(note("No UC100 machine yet. In the Machine library, set a machine's controller to UC100 and choose its UC100 profile file."))
            .push(btn("Machine library", Message::Page(Page::Machines), false));
    }
    for m in &machines {
        left = left.push(btn(
            m.name(),
            Message::ControlMachine(m.id.clone()),
            target.is_some_and(|t| t.id == m.id),
        ));
    }
    if let Some(m) = target {
        left = left.push(muted(format!("Connection: {}", crate::control::connection_label(m))));
        // The profile is read only while disconnected, not on every 33 ms tick.
        left = match &app.control {
            Some(_) => left.push(btn("Disconnect", Message::Disconnect, false)),
            None => match crate::control::profile(m) {
                Ok(_) => left.push(primary("Connect", Message::Connect)),
                Err(e) => left.push(warnings(vec![e])),
            },
        };
    }
    let left = container(left).padding(16).style(panel_style);
    let Some(c) = &app.control else {
        return row![left, container(muted("Not connected.")).padding(16).width(Fill)]
            .spacing(12)
            .padding(12)
            .into();
    };
    let s = &c.snapshot;
    let mut head = row![bold(s.state.as_ref().map_or("Connecting…", State::label), 28)]
        .spacing(12)
        .align_y(Alignment::Center);
    if c.simulated {
        head = head.push(
            container(bold("SIMULATED", 12).color(Color::WHITE))
                .padding([4, 8])
                .style(|_| container::Style {
                    background: Some(Color::from_rgb8(196, 120, 20).into()),
                    border: Border {
                        radius: 4.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }),
        );
    }
    let mut main = column![head].spacing(12).width(Fill);
    if s.state == Some(State::EStop) {
        if let Some(reason) = &s.estop_reason {
            main = main.push(text(reason.clone()).size(13));
        }
    }
    let mut buttons = row![].spacing(8).align_y(Alignment::Center);
    if s.state == Some(State::EStop) {
        buttons = buttons.push(primary("Reset", Message::MachineReset));
    }
    main = main.push(buttons.push(estop()));
    if let Some(id) = &s.identity {
        let v = |o: &Option<String>| o.clone().unwrap_or_else(|| "?".into());
        main = main.push(muted(format!(
            "UC100 firmware {} · hardware {} · bootloader {}",
            v(&id.firmware),
            v(&id.hardware_id),
            v(&id.bootloader)
        )));
    }
    let mono = |v: String| text(v).size(14).font(Font::MONOSPACE).width(120);
    let mut table = column![row![
        bold("Axis", 12).width(60),
        bold("Machine", 12).width(120),
        bold("Work", 12).width(120),
        bold("Homed", 12)
    ]]
    .spacing(4);
    for i in (0..6).filter(|&i| s.axes_enabled[i]) {
        table = table.push(row![
            text(["X", "Y", "Z", "A", "B", "C"][i]).size(14).width(60),
            mono(format!("{:.3}", s.machine_pos[i])),
            mono(format!("{:.3}", s.work_pos[i])),
            text(if s.referenced[i] { "Yes" } else { "No" }).size(14),
        ]);
    }
    main = main.push(table);
    let spindle = match s.spindle {
        Some(Spindle::Cw) => "CW",
        Some(Spindle::Ccw) => "CCW",
        _ => "off",
    };
    let on = |b: bool| if b { "on" } else { "off" };
    main = main.push(muted(format!(
        "Spindle {spindle} · {:.0} rpm · flood {} · mist {} · feed {:.0} mm/min · override {:.0}%",
        s.rpm,
        on(s.flood),
        on(s.mist),
        s.feed_rate,
        s.feed_override * 100.0
    )));
    // ponytail: the last 30 log lines, newest last; a scrollable log when someone needs older lines.
    let log = s
        .log
        .iter()
        .skip(s.log.len().saturating_sub(30))
        .fold(column![bold("Log", 14)].spacing(2), |col, line| {
            col.push(text(line.clone()).size(11).font(Font::MONOSPACE))
        });
    row![
        left,
        container(main).padding(16).width(Fill).style(panel_style),
        container(log).padding(16).width(420).style(panel_style)
    ]
    .spacing(12)
    .padding(12)
    .into()
}
```

5. In the wizard's UC100 block from Task 2, lock the fields of the connected machine: wrap the profile row and the Connection choice so that, when `app.control.as_ref().is_some_and(|c| c.machine_id == m.id)`, the form instead pushes `muted(format!("Profile: {}", get(p, "uc100_profile")))`, `muted(format!("Connection: {}", crate::control::connection_label(m)))` and `note("Connected on the Control page. Disconnect there to change the profile or the connection.")`.

If `Machine`, `get`, `Font`, `Border` or `container` are not yet in scope in `src/ui.rs`, they come from the existing imports (`model::*`, `iced::{…}`); add nothing else.

- [ ] **Step 4: Run the tests**

Run: `cargo test the_control_page_renders_in_every_state`, then `cargo test`
Expected: all pass.

- [ ] **Step 5: Look at it**

Run the app (`cargo run`), create a machine with controller UC100, choose `C:\Users\liork\projects\UC100-driver\configs\jy5800-investigation.toml` as its profile and Connection "Simulated", open Control, Connect, Reset, then E-stop with the top-bar button and with Esc. Check the SIMULATED badge, the position table (X, Y, Z) and the log. Take a screenshot for the report if your harness can.

- [ ] **Step 6: Commit**

```bash
git add src/main.rs src/ui.rs src/smoke.rs
git commit -m "The Control page shows the machine's state; E-stop is on every page while connected" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 5: Documentation

**Files:**
- Modify: `README.md` (new section before "## LinuxCNC G-code export"; Code map)
- Modify: `docs/VALIDATION.md` (new section after "## LinuxCNC post")

- [ ] **Step 1: README**

Add before "## LinuxCNC G-code export":

```markdown
## Machine control (UC100)

AXIS CAM can drive a CNCdrive UC100 itself, through the
[UC100 driver](https://github.com/liorknafo/UC100-driver) (pinned in `Cargo.toml`).

1. In the Machine library, set a machine's controller to **UC100**, choose its
   **UC100 profile file** (the driver's machine TOML: pins, steps per mm,
   speeds, limits, homing) and its **Connection**: **USB** for the real
   controller (macOS) or **Simulated**.
2. Open **Control**, pick the machine and press **Connect**. The machine
   starts in E-stop; **Reset** enables it.
3. While connected, a red **E-STOP** is in the top bar on every page, and
   **Esc** sends E-stop from anywhere, even while typing. Leaving E-stop
   always needs Reset.

**Simulated** runs the driver's real controller, planner and checks against a
simulated UC100: no hardware is opened and nothing moves. It keeps its own
saved offsets and position, separate from the real machine's. Jogging,
homing, work zero and running setups come in the next parts.
```

In "## Code map", add after the `src/post_check.rs` line:

```markdown
- `src/control.rs`: the UC100 connection: a machine's driver profile and Connection, the driver's controller for one machine, its saved state files and the snapshot the Control page draws.
```

- [ ] **Step 2: VALIDATION**

Add after the "## LinuxCNC post" section:

```markdown
## UC100 machine control (part 1)

- Profiles: a UC100 machine needs a profile file; the driver loads and checks it, and its messages (missing file, invalid TOML) are shown (`control::tests`, `a_uc100_machine_needs_its_profile_file`, `the_wizard_shows_the_uc100_fields_and_takes_a_picked_profile`).
- Simulated connection: Connect → E-stop → Reset → Idle → E-stop → Disconnect; the identity is SIMULATED; the simulator's state file is written and the real machine's is not (`connects_to_the_simulated_uc100_resets_and_e_stops`).
- Refusals: no UC100 machine, a missing profile; E-stop without a connection does nothing (`connect_reports_a_missing_profile_and_needs_a_uc100_machine`).
- Closing the app while connected disconnects (`closing_the_app_disconnects_the_machine`).
- The Control page renders disconnected, connecting, in E-stop and Idle, and the wizard locks the connected machine's UC100 fields (`the_control_page_renders_in_every_state`).
- Not yet validated: connecting to the real UC100 over USB from AXIS CAM (macOS, attended, no motion).
```

- [ ] **Step 3: Commit**

```bash
git add README.md docs/VALIDATION.md
git commit -m "README and VALIDATION: UC100 machine control" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 4: Final check and push**

Run: `cargo test` (all pass), then `git push -u origin t3/uc100-controller-integration`. Report the commits and test counts.
