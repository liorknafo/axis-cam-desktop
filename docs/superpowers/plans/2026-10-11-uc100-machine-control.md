# UC100 machine control, part 2 (the machine control page) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The Control page gets machine zero and soft limits, homing, jogging, work zero and G54–G59, spindle and coolant, feed and limit override, MDI and a live model of the CNC4030; the status line stops saying "Connecting…".

**Architecture:** `src/control.rs` grows an `Action` enum handled by `Connection::act`, plus per-tick work in `Connection::refresh` (one queued `SetMachine` per tick, the held jog's keepalive). `main.rs` routes `Message::Machine(Action)`, the jog keys (with a focus check through a custom widget operation) and global release/unfocus events. `ui.rs` lays the page out. A new `src/machine_view.rs` draws the machine on its own canvas.

**Tech Stack:** Rust 2024, Iced 0.14 (adds the `advanced` feature for one widget operation), `uc100` driver at `36885a1fd0ea124e494b9db7d464e0393401ce6d`.

**Spec:** `docs/superpowers/specs/2026-10-11-uc100-machine-control-design.md`.

**Repository and branch:** `C:\Users\liork\.t3\worktrees\axis-cam-desktop\t3-uc100-machine-control`, branch `t3/uc100-machine-control`.

## Global Constraints

- Every connected test uses the simulated UC100. Never real hardware.
- On this PC: `export AXIS_SKIP_STEP=1` before cargo. Exactly two STEP tests fail for that reason (`cam::tests::real_step_and_3mf…`, `pocket::tests::actual_step_and_3mf…`); everything else passes. Baseline before this plan: 769 passed, 2 failed.
- `cargo fmt --check` clean; no new warnings.
- Never send an axis index ≥ 6 or a disabled axis to the driver.
- Commit after each task: a plain sentence, ending with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Shared test fixtures (`src/smoke.rs`, `mod tests`)

Added in Task 1 and used by all later tasks:

```rust
/// Like the JY5800: X, Y, Z with soft limits, no switches, a PWM spindle
/// without a direction output, flood but no mist.
const LIMITED: &str = "...";   // X 0…390, Y 0…280, Z −55…0; outputs.spindle_pwm 16, flood 8
/// Home switches on every axis, a limit switch on X, a reversible spindle, mist.
const SWITCHED: &str = "...";  // home 11/12/13, limit_neg 15; spindle_cw 1, spindle_ccw 14, mist 9

/// `uc100_app` with the given profile text.
fn profile_app(dir: &Path, profile: &str) -> App;
/// Connects, resets and waits for Idle.
fn idle(a: &mut App);
/// The connection.
fn conn(a: &App) -> &control::Connection;
```

`uc100_app(dir)` becomes `profile_app(dir, uc100::config::TEMPLATE)`.

---

### Task 1: Driver pin, the kept profile and the status line (LIO-75)

**Files:** `Cargo.toml`, `Cargo.lock`, `src/control.rs`, `src/main.rs` (Tick), `src/smoke.rs`.

- [ ] Pin `uc100` to `36885a1fd0ea124e494b9db7d464e0393401ce6d`; `cargo build`.
- [ ] Test `the_status_line_shows_the_machine_state_once_connected`: connect (TEMPLATE), status starts "Connecting to JY5800"; tick until E-stop; status == "Connected to JY5800: E-STOP". Also `profile_app`, `idle`, `conn`, `LIMITED`, `SWITCHED`, and a check that both profiles load (`control::profile` is Ok).
- [ ] `Connection.config: MachineConfig` (clone before `Controller::start`).
- [ ] Tick: after `c.refresh()`, if `status` starts with "Connecting to " and the snapshot has a state, replace it: `Connected to {name}: {label}`, or for `Disconnected` "Could not connect to {name}; see the log on the Control page." The name comes from `status` itself (`strip_prefix("Connecting to ")`, `trim_end_matches('…')`).
- [ ] `cargo test`; commit "The status line shows the machine's state once it connects".

### Task 2: Machine zero, soft limits and homing (LIO-74, LIO-41)

**Files:** `src/control.rs`, `src/main.rs` (`Message::Machine`), `src/ui.rs` (page layout, soft-limit box, position table, machine-zero panel, `act` button helper), `src/help.rs` (tooltips), `src/smoke.rs`.

**Interfaces produced:**

```rust
pub enum Action { Home, MachineZero(Option<usize>), KeepPosition, /* later tasks add the rest */ }
impl Connection {
    pub fn act(&mut self, a: Action);
    pub fn manual(&self) -> bool;               // Idle or Paused
    pub fn idle(&self) -> bool;                 // Idle
    pub fn home_switches(&self) -> bool;        // every enabled axis has a home pin
    pub fn soft_limits_configured(&self) -> bool;
    pub fn unset_limited_axes(&self) -> Vec<usize>;
}
// main.rs
Message::Machine(control::Action)
```

- [ ] Tests:
  - `machine_zero_turns_the_soft_limits_on` (LIMITED): Idle; `unset_limited_axes() == [0, 1, 2]`; MDI-free move: `Machine(MachineZero(None))` is sent only when Idle, so first send `Command::Mdi("G0 X20 Y10 Z-5")` through `conn().send` (soft limits are off, so it runs), wait for Idle at (20, 10, −5); `MachineZero(None)`; tick until all three referenced and machine X, Y, Z = 0 (±0.001); `unset_limited_axes()` empty; then `Mdi("G0 X-5")` is refused: X stays 0 and the log has "MDI:". Also `MachineZero(Some(0))` after moving X to 7 sets only X.
  - `keep_the_last_position_declares_where_the_machine_was_left` (LIMITED): Idle, `Mdi("G0 X12 Y7")`, wait, Disconnect; Connect again (same workspace), Reset, Idle: machine (12, 7, 0), not referenced; `KeepPosition`; tick until referenced; position unchanged.
  - `home_needs_home_switches_on_every_axis`: LIMITED → `home_switches()` false, `Home` sends nothing (no "no home switch" log line); SWITCHED → true, `Home` → state goes Homing then Idle with all referenced.
- [ ] `control.rs`: `Action`, `act`, `declare` queue, `refresh` sends one `SetMachine` per tick while Idle and clears the queue otherwise; the helpers above.
- [ ] `main.rs`: `Message::Machine(a) => if let Some(c) = &mut self.control { c.act(a) }`.
- [ ] `ui.rs`: page restructure (log under the machine picker; main area scrollable), soft-limit box, position table ("Work (G54)", "Position set"), Machine zero panel and note, Home when `home_switches()`. `fn act(label, Option<Message>)` button: greyed out without a message.
- [ ] `help.rs`: tooltips for `Machine(Home | MachineZero | KeepPosition)`.
- [ ] Render the page in `the_control_page_renders_in_every_state` with LIMITED, before and after machine zero.
- [ ] `cargo test`, `cargo fmt --check`; commit "Machine zero turns the soft limits on; Home for machines with switches".

### Task 3: Jogging (LIO-42)

**Files:** `Cargo.toml` (iced `advanced`), `src/control.rs`, `src/main.rs` (messages, subscription, `App::jog_key`), `src/ui.rs` (Jog panel), `src/help.rs`, `src/smoke.rs`.

**Interfaces produced:**

```rust
Action::{JogPress(usize, bool), JogRelease, JogStep(Option<f64>), JogSpeed(f32)}
impl Connection { pub fn held(&self) -> Option<(usize, bool)>; }
pub fn typing() -> impl iced::advanced::widget::Operation<bool>;   // any focused text field
// main.rs
Message::{JogKey(usize, bool, bool), JogKeyChecked(usize, bool, bool), JogKeyUp}
App::jog_key: Option<(usize, bool)>
fn jog_key(key: &iced::keyboard::key::Named) -> Option<(usize, bool)>
```

- [ ] Tests:
  - `a_held_jog_keeps_moving_and_stops_on_release` (TEMPLATE): Idle; `JogPress(0, false)`; tick for 700 ms (past the driver's 400 ms timeout): state still Jog, X grew; `JogRelease`; tick until Idle; `held()` is None; X then stays put.
  - `a_jog_step_moves_one_step` (TEMPLATE): `JogStep(Some(1.0))`, `JogPress(1, true)`; tick until Idle with Y = −1.000; `held()` None.
  - `a_jog_into_a_soft_limit_ends_at_the_limit` (LIMITED): machine zero; `JogPress(0, true)` continuous; tick until `held()` is None (≤ 3 s); X ≥ −0.01 and state Idle; `JogStep(Some(10.0))`, `JogPress(0, true)` → log "Jog refused", X unchanged.
  - `jog_keys_only_on_the_control_page_and_not_while_typing` (TEMPLATE): on Home page `JogKey(0,false,false)` sets nothing; on Control: `JogKeyChecked(0,false,false)` without `JogKey` does nothing; `JogKey` then `JogKeyChecked(.., typing = true)` does nothing; `JogKey` then `JogKeyUp` then `JogKeyChecked(.., false)` does nothing; `JogKey` then `JogKeyChecked(.., false)` holds X+; `JogKeyUp` releases; a repeat `JogKey(.., true)` does nothing.
  - `jog_key` maps ←→↑↓ PgUp PgDn to X−, X+, Y+, Y−, Z+, Z−.
- [ ] `control.rs`: `held: Option<Held { axis, negative, sent: Instant, started: bool }>`; `JogPress` → Step: `JogStep { distance: ±step, feed: max_velocity × speed% }`; Continuous: `JogStart { speed: speed%/100 }` and hold; both only when `manual()` and the axis is enabled; `JogRelease` → `JogStop` if held. `refresh`: resend every 100 ms while the snapshot says Jog; end the hold if it left Jog after starting or never started within 400 ms.
- [ ] `main.rs`: subscription arms (left release → `Machine(JogRelease)`, `Unfocused` → `Machine(JogRelease)`, jog key released → `JogKeyUp`, jog key pressed and `Ignored` → `JogKey`); `JogKey` (Control page, connected, not a repeat) sets `jog_key` and returns `iced::advanced::widget::operate(control::typing()).map(...)`; `JogKeyChecked` jogs only if not typing and `jog_key` still matches; `JogKeyUp` clears `jog_key` and releases.
- [ ] `ui.rs`: Jog panel: mode buttons, six `mouse_area` buttons (`on_press`, `on_release`, `on_exit`), highlighted while held, greyed when not `manual()` and not jogging; speed slider with the X feed; note on the keys.
- [ ] `cargo test`, fmt; commit "The machine jogs from the Control page: buttons, keys and steps".

### Task 4: Work zero, spindle and coolant, overrides, MDI (LIO-43…46)

**Files:** `src/control.rs`, `src/ui.rs`, `src/help.rs`, `src/smoke.rs`.

**Interfaces produced:**

```rust
Action::{WorkZero(Option<usize>), Wcs(usize), Rpm(String), Spindle(Spindle), Flood(bool), Mist(bool),
         FeedOverride(f32), OverrideLimits(bool), MdiText(String), MdiSend}
impl Connection { pub fn reversible(&self) -> bool; pub fn limit_inputs(&self) -> bool; pub fn rpm_value(&self) -> Option<f64>; }
```

- [ ] Tests:
  - `work_zero_and_the_coordinate_systems` (TEMPLATE): MDI G0 X5 Y6; `WorkZero(Some(0))` → work X 0, Y 6; `WorkZero(None)` → work all 0; `Wcs(1)` → active_wcs 1, work = machine; `Wcs(0)` → back, work 0.
  - `spindle_and_coolant_follow_the_profile`: LIMITED: `Rpm("12000")`, `Spindle(Cw)` → CW at 12000; `Spindle(Ccw)` → nothing (still CW, not E-stop); `Spindle(Off)` → off; `Spindle(Ccw)` → nothing (no direction output); `Flood(true)` → flood; `Mist(true)` → nothing; `Rpm("abc")` then CW → nothing. SWITCHED: CCW at 8000 works.
  - `feed_and_limit_override`: TEMPLATE `FeedOverride(150.)` → snapshot 1.5, `FeedOverride(500.)` → 2.0; `OverrideLimits(true)` → nothing without limit inputs; SWITCHED → `limits_override` true within 0.5 s.
  - `an_mdi_line_runs_and_the_field_clears` (TEMPLATE): `MdiText("G0 X5")`, `MdiSend` → field empty, X = 5; `MdiSend` with an empty field sends nothing.
- [ ] `control.rs`: the actions with their guards (see the spec's table).
- [ ] `ui.rs`: Work zero, Spindle and coolant, Overrides, MDI panels.
- [ ] `cargo test`, fmt; commit "Work zero, G54–G59, spindle, coolant, overrides and MDI on the Control page".

### Task 5: The machine model (LIO-47)

**Files:** create `src/machine_view.rs`; `src/main.rs` (`mod machine_view;`); `src/ui.rs` (the canvas beside the position).

**Interfaces produced:**

```rust
pub struct MachineView { pub pos: [f32; 3], pub known: bool, pub envelope: Option<[[f32; 3]; 2]> }
impl canvas::Program<Message> for MachineView { type State = Orbit; ... }
fn faces(pos: [f32; 3]) -> Vec<Face>;
fn painted(faces: &[Face], yaw: f32, pitch: f32) -> Vec<(Vec<[f32; 2]>, Color)>;
```

- [ ] Tests (`machine_view::tests`): `the_model_follows_the_machine_position` (the tool's lowest point is the position; the beam moves with Y, not X; the table never moves); `the_painter_culls_back_faces_and_draws_far_to_near` (one box seen from the default view: three faces, in increasing depth).
- [ ] Implement: ASSUMED constants block; boxes and cylinders (16 sides); orthographic orbit view; flat shading; fit to a fixed box; travel envelope lines (soft limits or assumed travel); red tool tip; translucent moving parts, amber box and label when `!known`; drag to orbit.
- [ ] `ui.rs`: `canvas(MachineView { .. }).width(Fill).height(340)` beside the position and machine zero.
- [ ] `cargo test`, fmt; commit "The Control page draws the CNC4030 moving with the machine".

### Task 6: Look, documentation, delivery

- [ ] Throwaway patch (never committed) as in part 1: `AXIS_LOOK_DIR` workspace with a simulated machine on the driver's `jy5800-investigation.toml` (from the pinned commit), Control page, Connect, Reset. Screenshot; Set machine zero; jog with the keyboard (real key presses) and the mouse; screenshot again. Fix what looks wrong. Remove the patch.
- [ ] README "Machine control (UC100)": the new controls, the keys, machine zero and soft limits, the model and its assumed dimensions. Code map: `src/machine_view.rs`.
- [ ] `docs/VALIDATION.md`: "UC100 machine control (part 2)" with the test names, the look, and what is not validated (the real machine).
- [ ] Full `cargo test`, `cargo fmt --check`; commit "README and VALIDATION: the machine control page"; push `t3/uc100-machine-control`.
- [ ] Linear: LIO-74, LIO-41…47, LIO-75 to Done with a short comment (commits, tests); LIO-31 too if all are done.
