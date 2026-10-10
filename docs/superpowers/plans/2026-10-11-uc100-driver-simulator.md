# UC100 driver: optional GUI and simulated UC100 — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let programs embed the UC100 driver without egui, and let the real controller run against a simulated UC100 on any computer.

**Architecture:** A `gui` cargo feature gates the egui app. A new private module `src/sim.rs` implements the controller-side behaviour of a UC100 (step counters, record draining at 1.28 ms, stop latch, jog, homing, soft limits). `Uc100` talks either to USB or to that simulator, chosen by a new `controller.simulated` config field, so the controller worker, planner and reset sequence run unchanged.

**Tech Stack:** Rust 2024 edition, stable Rust 1.99 on this PC; crate `uc100` (nusb, serde, toml, thiserror).

**Spec:** `C:\Users\liork\.t3\worktrees\axis-cam-desktop\t3-1a348eb5\docs\superpowers\specs\2026-10-11-uc100-foundation-design.md` (§1 is this plan; read it first).

**Repository and branch:** `C:\Users\liork\.t3\worktrees\UC100-driver\axis-cam-foundation`, branch `axis-cam-foundation` (tracks `origin/axis-cam-foundation`), based on `master` 90dfb4b.

## Global Constraints

- Never open, probe or move real hardware. No test, example or command in this plan may talk to a USB UC100. Do not run `uc100 info`, `uc100-gui`, or any `examples/` program.
- The USB path must behave exactly as before. Every existing test keeps passing.
- `controller.simulated` defaults to `false`, is not written when false, and makes `connect` ignore `controller.serial` and `controller.running_firmware`.
- Simulated identity: bootloader state 1, firmware `"1041"`, hardware `"SIMULATED"`, bootloader `"SIMULATED"`; behaves as `Firmware::V1041`.
- Records drain at one per 1.28 ms (`protocol::PERIOD_S`); the queue holds at most `protocol::DEVICE_BUFFER_RECORDS` (900).
- The commissioning gate is unchanged and applies in simulation.
- `cargo test` and `cargo test --no-default-features` must both pass at the end of every task.
- Commit after every task with a message in the repo's style (imperative sentence) ending with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus

- E-stop in the middle of a move, then Reset: the simulator must drop the rest of the move and Reset must reach Idle again (pinned in Task 3, `a_move_reaches_its_target_and_e_stop_halts_motion`).
- Records sent after a long idle period must start draining from their arrival, not play out instantly (pinned in Task 2, `records_drain_at_the_firmware_period_from_when_they_arrive`).
- The JY5800 profile sets `serial` and `running_firmware` (attach without reset); with `simulated = true` it must connect to the simulator, never USB (pinned in Task 3, `the_jy5800_profile_runs_on_the_simulator`).
- `jog_stop` for another axis must not stop the running jog; axis 6 stops everything including homing (pinned in Task 2, `jog_moves_until_stopped_and_axis_6_stops_every_move`).
- Axes without soft limits get ±2^30 steps from the controller and must never trip the simulated soft limit (pinned in Task 2, `soft_limits_stop_motion_only_while_enabled`).

---

### Task 1: Optional `gui` feature — DONE (commit cac67e2, pushed)

Already implemented by the orchestrator; listed so the branch history makes sense.

- `Cargo.toml`: `[features] default = ["gui"]`, `gui = ["dep:eframe", "dep:rfd"]`; `eframe` and `rfd` optional; `[[bin]] uc100-gui` and `[[example]] guitest` have `required-features = ["gui"]`; `eframe = "0.36.2"` added to `[dev-dependencies]` so `egui_kittest` builds under `--no-default-features`.
- `src/lib.rs`: `#[cfg(feature = "gui")] pub mod gui;`.
- Verified on Windows (Rust 1.99): `cargo tree --no-default-features -e normal` has no egui/eframe/rfd; `cargo build --no-default-features --lib --bins`, `cargo test` and `cargo test --no-default-features` pass (61 tests).

---

### Task 2: The simulated UC100 (`src/sim.rs`)

**Files:**
- Create: `src/sim.rs`
- Modify: `src/lib.rs` (add `mod sim;`)
- Modify: `src/protocol.rs:117` (`fn speed_mult` becomes `pub(crate) fn speed_mult`)

**Interfaces:**
- Consumes (existing): `config::MachineConfig` (`controller.step_rate`, `inputs.{estop,probe,index}`, `axes.get(i)` → `AxisConfig { steps_per_unit, home_position, limit_pos, limit_neg, home, .. }`), `device::Identity`, `protocol::{self as p, HostFlags, NUM_AXES, Status, StatusBits, StepRate, Pin, input_bit, DEVICE_BUFFER_RECORDS, MotionEncoder, speed_word}`.
- Produces (crate-private, used by Task 3):
  - `pub(crate) fn identity() -> Identity`
  - `pub(crate) struct Simulated` with
    - `pub(crate) fn new(cfg: &MachineConfig, now: Instant) -> Simulated`
    - `pub(crate) fn status(&mut self, outputs: u16, flags: HostFlags, now: Instant) -> Status`
    - `pub(crate) fn send_motion(&mut self, records: &[[u8; 12]], now: Instant) -> bool` (false = rejected)
    - `pub(crate) fn stop(&mut self, now: Instant) -> u16`
    - `pub(crate) fn set_position(&mut self, steps: [i32; NUM_AXES], now: Instant)`
    - `pub(crate) fn jog_start(&mut self, axis: usize, negative: bool, speed: u16, now: Instant)`
    - `pub(crate) fn jog_stop(&mut self, axis: u8, now: Instant)`
    - `pub(crate) fn home(&mut self, axis: usize, negative: bool, speed: u16, now: Instant)`
    - `pub(crate) fn soft_limits(&mut self, high: [i32; NUM_AXES], low: [i32; NUM_AXES], now: Instant)`
    - `pub(crate) fn advance(&mut self, now: Instant)`

- [ ] **Step 1: Make `speed_mult` reachable and register the module**

In `src/protocol.rs`, change `    fn speed_mult(self) -> f64 {` to `    pub(crate) fn speed_mult(self) -> f64 {`.

In `src/lib.rs`, add `mod sim;` after `mod reset;` and change the first doc line to:

```rust
//! macOS/Linux driver and controller for the CNCdrive UC100 USB motion
//! controller, with a simulated UC100 for computers without one.
```

- [ ] **Step 2: Write `src/sim.rs` with its tests**

```rust
//! Simulated UC100. It answers the calls `Uc100` makes on the USB device, so
//! the controller, planner and reset sequence run unchanged on a computer
//! without a controller. Motion records drain at the firmware's 1.28 ms
//! period; only the step counters move. No switch, E-stop input or
//! communication fault is simulated.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crate::config::MachineConfig;
use crate::device::Identity;
use crate::protocol::{self as p, HostFlags, NUM_AXES, Status, StatusBits, StepRate};

/// One motion record (`protocol::PERIOD_S`).
const PERIOD: Duration = Duration::from_micros(1280);
/// Status bit 3 is set in every reply of the real controller.
const ALWAYS_SET: u8 = 0x08;

/// What the simulated bootloader reports.
pub(crate) fn identity() -> Identity {
    Identity {
        bootloader_state: 1,
        firmware: Some("1041".into()),
        hardware_id: Some("SIMULATED".into()),
        bootloader: Some("SIMULATED".into()),
    }
}

/// A controller-run move of one axis: a jog, or homing toward its switch.
#[derive(Debug, Clone, Copy)]
struct Drive {
    axis: usize,
    /// Signed speed, steps per second.
    speed: f64,
    /// Homing stops at this counter value; a jog runs until stopped.
    switch: Option<i32>,
}

pub(crate) struct Simulated {
    rate: StepRate,
    /// Counters in the motion stream's fixed-point units (`rate.scale()` per step).
    position: [f64; NUM_AXES],
    queue: VecDeque<[u8; 12]>,
    /// Time up to which motion has been played.
    clock: Instant,
    drive: Option<Drive>,
    host_estop: bool,
    latched: bool,
    soft_limits_on: bool,
    soft_limit_hit: bool,
    /// (high, low) soft limits in steps, once sent.
    limits: Option<([i32; NUM_AXES], [i32; NUM_AXES])>,
    /// Where each axis's home switch is, in steps (the profile's `home_position`).
    switches: [i32; NUM_AXES],
    inputs: u8,
    outputs: u16,
    timer: u16,
}

impl Simulated {
    pub(crate) fn new(cfg: &MachineConfig, now: Instant) -> Self {
        // Unconnected inputs read HIGH; every configured input reads its
        // inactive level: HIGH when active-low, LOW otherwise.
        let mut inputs = 0x1f_u8;
        let mut inactive = |pin: Option<p::Pin>| {
            if let Some(pin) = pin
                && let Some(bit) = p::input_bit(pin.pin)
            {
                if pin.active_low {
                    inputs |= 1 << bit;
                } else {
                    inputs &= !(1 << bit);
                }
            }
        };
        inactive(cfg.inputs.estop);
        inactive(cfg.inputs.probe);
        inactive(cfg.inputs.index);
        let mut switches = [0; NUM_AXES];
        for (i, switch) in switches.iter_mut().enumerate() {
            if let Some(a) = cfg.axes.get(i) {
                for pin in [a.limit_pos, a.limit_neg, a.home] {
                    inactive(pin);
                }
                *switch = (a.home_position * a.steps_per_unit).round() as i32;
            }
        }
        Simulated {
            rate: cfg.controller.step_rate,
            position: [0.0; NUM_AXES],
            queue: VecDeque::new(),
            clock: now,
            drive: None,
            host_estop: false,
            latched: false,
            soft_limits_on: false,
            soft_limit_hit: false,
            limits: None,
            switches,
            inputs,
            outputs: 0,
            timer: 0,
        }
    }

    fn steps(&self) -> [i32; NUM_AXES] {
        let scale = self.rate.scale();
        self.position.map(|v| (v / scale).round() as i32)
    }

    /// Steps per second in a jog or homing speed word (inverse of `protocol::speed_word`).
    fn speed(&self, word: u16) -> f64 {
        f64::from(word) / 256.0 / self.rate.speed_mult() * 781.0
    }

    /// Plays queued records and controller-run moves up to `now`.
    pub(crate) fn advance(&mut self, now: Instant) {
        if self.latched {
            // The stop latch halts the controller.
            self.queue.clear();
            self.drive = None;
        }
        while !self.queue.is_empty() && self.clock + PERIOD <= now {
            let record = self.queue.pop_front().unwrap();
            self.clock += PERIOD;
            for a in 0..NUM_AXES {
                let word = u16::from_be_bytes([record[2 * a], record[2 * a + 1]]);
                let magnitude = f64::from(word & 0x7fff);
                self.position[a] += if word & 0x8000 != 0 { -magnitude } else { magnitude };
            }
            if self.hold_inside_limits() {
                self.queue.clear();
            }
        }
        if !self.queue.is_empty() {
            return;
        }
        if let Some(d) = self.drive {
            let scale = self.rate.scale();
            let seconds = now.saturating_duration_since(self.clock).as_secs_f64();
            let mut next = self.position[d.axis] + d.speed * seconds * scale;
            if let Some(switch) = d.switch {
                let at = f64::from(switch) * scale;
                if (d.speed < 0.0 && next <= at) || (d.speed >= 0.0 && next >= at) {
                    next = at;
                    self.drive = None;
                }
            }
            self.position[d.axis] = next;
            if self.hold_inside_limits() {
                self.drive = None;
            }
        }
        self.clock = self.clock.max(now);
    }

    /// With soft limits on, keeps every axis inside them; returns whether one was reached.
    fn hold_inside_limits(&mut self) -> bool {
        let (true, Some((high, low))) = (self.soft_limits_on, self.limits) else {
            return false;
        };
        let scale = self.rate.scale();
        let mut hit = false;
        for a in 0..NUM_AXES {
            let (min, max) = (f64::from(low[a]) * scale, f64::from(high[a]) * scale);
            if self.position[a] > max {
                self.position[a] = max;
                hit = true;
            } else if self.position[a] < min {
                self.position[a] = min;
                hit = true;
            }
        }
        self.soft_limit_hit |= hit;
        hit
    }

    pub(crate) fn status(&mut self, outputs: u16, flags: HostFlags, now: Instant) -> Status {
        self.host_estop = flags.estop;
        self.latched |= flags.estop;
        self.soft_limits_on = flags.soft_limits;
        self.outputs = outputs;
        self.advance(now);
        self.timer = self.timer.wrapping_add(1);
        let mut bits = ALWAYS_SET;
        let mut axis_flags = 0_u8;
        match self.drive {
            Some(Drive { axis, switch: None, .. }) => {
                bits |= 0x01;
                axis_flags = 1 << axis;
            }
            Some(Drive { switch: Some(_), .. }) => bits |= 0x10,
            None => {}
        }
        if self.latched {
            bits |= 0x02;
        }
        if !self.queue.is_empty() {
            bits |= 0x04;
        }
        if self.soft_limit_hit {
            bits |= 0x20;
        }
        Status {
            position: self.steps(),
            // The firmware reads 1 when its buffer is empty.
            buffered: self.queue.len() as u16 + 1,
            bits: StatusBits(bits),
            timer: self.timer,
            inputs: self.inputs,
            outputs: self.outputs,
            axis_flags,
            ..Default::default()
        }
    }

    /// Queues motion records; false (rejected) when they do not fit the buffer.
    pub(crate) fn send_motion(&mut self, records: &[[u8; 12]], now: Instant) -> bool {
        self.advance(now);
        if self.queue.len() + records.len() > usize::from(p::DEVICE_BUFFER_RECORDS) {
            return false;
        }
        self.queue.extend(records.iter().copied());
        true
    }

    /// Flushes queued motion and stops every move; returns the records dropped.
    pub(crate) fn stop(&mut self, now: Instant) -> u16 {
        self.advance(now);
        let dropped = self.queue.len() as u16;
        self.queue.clear();
        self.drive = None;
        self.soft_limit_hit = false;
        // Firmware 1041: a stop clears the latch only once the host flag is off.
        if !self.host_estop {
            self.latched = false;
        }
        dropped
    }

    pub(crate) fn set_position(&mut self, steps: [i32; NUM_AXES], now: Instant) {
        self.advance(now);
        let scale = self.rate.scale();
        self.position = steps.map(|s| f64::from(s) * scale);
    }

    pub(crate) fn jog_start(&mut self, axis: usize, negative: bool, speed: u16, now: Instant) {
        self.advance(now);
        let v = self.speed(speed);
        self.drive = Some(Drive { axis, speed: if negative { -v } else { v }, switch: None });
    }

    /// Axis 6 stops every controller-run move, homing included.
    pub(crate) fn jog_stop(&mut self, axis: u8, now: Instant) {
        self.advance(now);
        match self.drive {
            _ if axis == 6 => self.drive = None,
            Some(Drive { axis: a, switch: None, .. }) if a == usize::from(axis) => self.drive = None,
            _ => {}
        }
    }

    /// Runs toward the axis's switch; finishes at once when already at or past it.
    pub(crate) fn home(&mut self, axis: usize, negative: bool, speed: u16, now: Instant) {
        self.advance(now);
        let (at, switch) = (self.steps()[axis], self.switches[axis]);
        if (negative && at <= switch) || (!negative && at >= switch) {
            return;
        }
        let v = self.speed(speed);
        self.drive = Some(Drive { axis, speed: if negative { -v } else { v }, switch: Some(switch) });
    }

    pub(crate) fn soft_limits(&mut self, high: [i32; NUM_AXES], low: [i32; NUM_AXES], now: Instant) {
        self.advance(now);
        self.limits = Some((high, low));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TEMPLATE;
    use crate::protocol::{MotionEncoder, Pin};

    fn sim() -> (Simulated, StepRate, Instant) {
        let cfg = MachineConfig::from_toml(TEMPLATE).unwrap();
        let now = Instant::now();
        (Simulated::new(&cfg, now), cfg.controller.step_rate, now)
    }

    fn flags(estop: bool) -> HostFlags {
        HostFlags { estop, ..Default::default() }
    }

    fn ms(t: Instant, ms: u64) -> Instant {
        t + Duration::from_millis(ms)
    }

    /// Records moving X one step further each: to 1, 2, … `n`.
    fn ramp(rate: StepRate, n: usize) -> Vec<[u8; 12]> {
        let mut enc = MotionEncoder::new(rate, [0; NUM_AXES]);
        (1..=n)
            .map(|k| {
                let mut target = [0.0; NUM_AXES];
                target[0] = k as f64;
                enc.record(&target)
            })
            .collect()
    }

    #[test]
    fn records_move_the_counters_to_the_encoder_position() {
        let (mut s, rate, t0) = sim();
        let mut enc = MotionEncoder::new(rate, [0; NUM_AXES]);
        // X out to 1000 steps, back to 400 (a reversal), out to 700; Y to -250 and back to -100.
        let mut targets: Vec<[f64; NUM_AXES]> = vec![];
        for (x, y, n) in [(1000.0, -250.0, 120), (400.0, -250.0, 80), (700.0, -100.0, 60)] {
            let start = targets.last().copied().unwrap_or([0.0; NUM_AXES]);
            for k in 1..=n {
                let f = k as f64 / n as f64;
                let mut target = start;
                target[0] = start[0] + (x - start[0]) * f;
                target[1] = start[1] + (y - start[1]) * f;
                targets.push(target);
            }
        }
        let records: Vec<_> = targets.iter().map(|t| enc.record(t)).collect();
        let mut t = t0;
        for chunk in records.chunks(100) {
            assert!(s.send_motion(chunk, t));
            t += PERIOD * (chunk.len() as u32 + 1);
            s.advance(t);
        }
        let st = s.status(0, flags(false), t);
        let expected = enc.position_steps();
        for a in 0..NUM_AXES {
            assert!(
                (f64::from(st.position[a]) - expected[a]).abs() <= 1.0,
                "axis {a}: {} vs {}",
                st.position[a],
                expected[a]
            );
        }
        assert!(!st.bits.running() && st.buffered == 1 && st.at_rest());
    }

    #[test]
    fn records_drain_at_the_firmware_period_from_when_they_arrive() {
        let (mut s, rate, t0) = sim();
        // A long idle period must not let new records play out at once.
        let t = t0 + Duration::from_secs(5);
        let _ = s.status(0, flags(false), t);
        assert!(s.send_motion(&ramp(rate, 50), t));
        let st = s.status(0, flags(false), t + PERIOD * 10);
        assert_eq!(st.buffered, 40 + 1);
        assert!(st.bits.running());
        assert_eq!(st.position[0], 10);
    }

    #[test]
    fn host_estop_latches_until_a_stop_after_release() {
        let (mut s, _, t) = sim();
        assert!(s.status(0, flags(true), t).bits.stop_latched());
        // Releasing the host flag alone does not clear the latch (firmware 1041).
        assert!(s.status(0, flags(false), t).bits.stop_latched());
        s.stop(t);
        let st = s.status(0, flags(false), t);
        assert!(!st.bits.stop_latched() && st.at_rest());
        assert_eq!(st.bits.0 & ALWAYS_SET, ALWAYS_SET);
        // A stop while the host flag is still set leaves it latched.
        let _ = s.status(0, flags(true), t);
        s.stop(t);
        assert!(s.status(0, flags(true), t).bits.stop_latched());
    }

    #[test]
    fn the_latch_halts_queued_motion() {
        let (mut s, rate, t) = sim();
        assert!(s.send_motion(&ramp(rate, 100), t));
        let before = s.status(0, flags(false), t + PERIOD * 10).position[0];
        assert_eq!(before, 10);
        let latched = s.status(0, flags(true), t + PERIOD * 10);
        assert_eq!(latched.position[0], before);
        let later = s.status(0, flags(true), t + PERIOD * 100);
        assert_eq!(later.position[0], before);
        assert!(!later.bits.running() && later.buffered == 1);
    }

    #[test]
    fn a_full_buffer_rejects_more_motion() {
        let (mut s, _, t) = sim();
        let still = [[0_u8; 12]; 100];
        for _ in 0..9 {
            assert!(s.send_motion(&still, t));
        }
        assert!(!s.send_motion(&still[..1], t));
        assert_eq!(s.stop(t), 900);
    }

    #[test]
    fn configured_inputs_read_inactive_in_both_polarities() {
        for active_low in [true, false] {
            let mut cfg = MachineConfig::from_toml(TEMPLATE).unwrap();
            let estop = Pin { pin: 15, active_low };
            cfg.inputs.estop = Some(estop);
            cfg.axes.x.as_mut().unwrap().limit_neg = Some(Pin { pin: 10, active_low: !active_low });
            let t = Instant::now();
            let mut s = Simulated::new(&cfg, t);
            let st = s.status(0, flags(false), t);
            assert_eq!(st.input_active(estop), Some(false));
            assert!(!cfg.limit_input_active(&st));
            // Unconfigured inputs read HIGH, like unconnected pins.
            assert_eq!(st.input_level(12), Some(true));
        }
    }

    #[test]
    fn jog_moves_until_stopped_and_axis_6_stops_every_move() {
        let (mut s, rate, t) = sim();
        let word = p::speed_word(1000.0, rate);
        s.jog_start(0, false, word, t);
        let st = s.status(0, flags(false), ms(t, 500));
        assert!(st.bits.jogging() && st.axis_jogging(0));
        assert!((st.position[0] - 500).abs() <= 5, "{}", st.position[0]);
        // Stopping another axis leaves this jog running.
        s.jog_stop(1, ms(t, 500));
        assert!(s.status(0, flags(false), ms(t, 600)).bits.jogging());
        s.jog_stop(6, ms(t, 600));
        let st = s.status(0, flags(false), ms(t, 2000));
        assert!(!st.bits.jogging() && st.axis_flags == 0 && st.at_rest());
        assert!((st.position[0] - 600).abs() <= 6, "{}", st.position[0]);
        // A negative jog runs the other way.
        s.jog_start(0, true, word, ms(t, 2000));
        let st = s.status(0, flags(false), ms(t, 2100));
        assert!(st.position[0] < 600);
        // Axis 6 also stops homing.
        s.jog_stop(6, ms(t, 2100));
        s.home(0, true, word, ms(t, 2100));
        assert!(s.status(0, flags(false), ms(t, 2110)).bits.homing());
        s.jog_stop(6, ms(t, 2110));
        assert!(!s.status(0, flags(false), ms(t, 2120)).bits.homing());
    }

    #[test]
    fn homing_runs_to_the_home_position_and_finishes_at_once_when_past_it() {
        // Template X: home_position 0, homes toward negative.
        let (mut s, rate, t) = sim();
        s.set_position([400, 0, 0, 0, 0, 0], t);
        let word = p::speed_word(2000.0, rate);
        s.home(0, true, word, t);
        let st = s.status(0, flags(false), ms(t, 100));
        assert!(st.bits.homing() && st.position[0] < 400 && st.position[0] > 0, "{}", st.position[0]);
        let st = s.status(0, flags(false), ms(t, 1000));
        assert!(!st.bits.homing() && st.position[0] == 0 && st.at_rest());
        // Already at the switch: the homing bit never rises.
        s.home(0, true, word, ms(t, 1000));
        assert!(!s.status(0, flags(false), ms(t, 1000)).bits.homing());
    }

    #[test]
    fn soft_limits_stop_motion_only_while_enabled() {
        let (mut s, rate, t) = sim();
        // Axes without limits get ±2^30 steps from the controller.
        let mut high = [1 << 30; NUM_AXES];
        let mut low = [-(1 << 30); NUM_AXES];
        high[0] = 100;
        low[0] = -100;
        s.soft_limits(high, low, t);
        let soft = HostFlags { soft_limits: true, ..Default::default() };
        let word = p::speed_word(1000.0, rate);
        s.jog_start(0, false, word, t);
        let st = s.status(0, soft, ms(t, 500));
        assert_eq!(st.position[0], 100);
        assert!(st.bits.soft_limit() && !st.bits.jogging());
        s.stop(ms(t, 500));
        assert!(!s.status(0, soft, ms(t, 500)).bits.soft_limit());
        // An unlimited axis never trips.
        s.jog_start(1, false, word, ms(t, 500));
        let st = s.status(0, soft, ms(t, 1000));
        assert!(st.position[1] > 400 && !st.bits.soft_limit());
        s.jog_stop(6, ms(t, 1000));
        // With soft limits off the same jog runs past the limit.
        s.jog_start(0, false, word, ms(t, 1000));
        let st = s.status(0, flags(false), ms(t, 1500));
        assert!(st.position[0] > 100 && !st.bits.soft_limit());
    }
}
```

- [ ] **Step 3: Run the new tests**

Run: `cargo test --no-default-features sim::`
Expected: all 9 `sim::tests` pass. If `records_move_the_counters_to_the_encoder_position` is off by more than one step, the drift comes from the encoder's direction-reversal correction (`MotionEncoder::record`, `protocol.rs` ~line 623): apply the same correction to the simulated counter at each direction change of an axis (subtract `magnitude / rate.reversal_div()` when turning positive, add when turning negative, mirroring the encoder) and make `reversal_div` `pub(crate)`. Do not loosen the tolerance.

- [ ] **Step 4: Run the whole suite both ways**

Run: `cargo test` then `cargo test --no-default-features`
Expected: both pass (61 existing + 9 new). `cargo build --no-default-features --lib --bins` prints no warnings for `src/sim.rs` (until Task 3 the module is unused outside tests, so add `#![allow(dead_code)]` at the top of `src/sim.rs` only if the compiler warns, and remove it in Task 3).

- [ ] **Step 5: Commit**

```bash
git add src/sim.rs src/lib.rs src/protocol.rs
git commit -m "Add a simulated UC100" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: Connect to the simulator (`controller.simulated`)

**Files:**
- Modify: `src/config.rs:38-49` (`ControllerConfig`)
- Modify: `src/device.rs` (the `Uc100` type and its methods)
- Modify: `src/controller.rs:392-414` (`Worker::connect`)
- Create: `tests/simulated.rs`

**Interfaces:**
- Consumes: everything Task 2 produces.
- Produces (public, used by AXIS CAM):
  - `config::ControllerConfig::simulated: bool` (serde default false, skipped when false).
  - `device::Uc100::simulated(cfg: &MachineConfig) -> (Uc100, Identity)`.
  - Behaviour: `Controller::start(cfg, state_file)` with `cfg.controller.simulated == true` connects to the simulator and logs "Connected: UC100 firmware 1041 (hardware SIMULATED, bootloader SIMULATED)".

- [ ] **Step 1: Write the failing integration tests**

Create `tests/simulated.rs`:

```rust
//! The real controller against the simulated UC100. No hardware is opened.

use std::time::{Duration, Instant};

use uc100::config::{MachineConfig, TEMPLATE};
use uc100::controller::{Command, Controller, Snapshot, State};

fn start(profile: &str) -> Controller {
    let mut cfg = MachineConfig::from_toml(profile).unwrap();
    cfg.controller.simulated = true;
    Controller::start(cfg, None)
}

/// Polls the snapshot until `ok` holds; fails after `secs`.
fn wait(c: &Controller, secs: f64, what: &str, ok: impl Fn(&Snapshot) -> bool) -> Snapshot {
    let end = Instant::now() + Duration::from_secs_f64(secs);
    loop {
        let s = c.snapshot();
        if ok(&s) {
            return s;
        }
        assert!(Instant::now() < end, "timed out waiting for {what}: state {:?}, log {:?}", s.state, s.log);
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn reset(c: &Controller) {
    wait(c, 2.0, "E-stop after connecting", |s| s.state == Some(State::EStop));
    c.send(Command::Reset);
    wait(c, 2.0, "Idle after Reset", |s| s.state == Some(State::Idle));
}

#[test]
fn connects_as_a_simulated_controller_and_resets() {
    let c = start(TEMPLATE);
    let s = wait(&c, 2.0, "the identity", |s| s.identity.is_some());
    assert_eq!(s.identity.unwrap().hardware_id.as_deref(), Some("SIMULATED"));
    assert!(s.log.iter().any(|l| l.contains("hardware SIMULATED")), "{:?}", s.log);
    reset(&c);
}

#[test]
fn the_jy5800_profile_runs_on_the_simulator() {
    // This profile attaches to known firmware by serial; simulated mode ignores both.
    let c = start(include_str!("../configs/jy5800-investigation.toml"));
    reset(&c);
}

#[test]
fn a_move_reaches_its_target_and_e_stop_halts_motion() {
    let c = start(TEMPLATE);
    reset(&c);
    c.send(Command::Mdi("G0 X5".into()));
    wait(&c, 5.0, "X5", |s| s.state == Some(State::Idle) && (s.machine_pos[0] - 5.0).abs() < 1e-3);
    // A long move, stopped part way.
    c.send(Command::Mdi("G0 X200".into()));
    wait(&c, 3.0, "moving past X10", |s| s.machine_pos[0] > 10.0);
    c.send(Command::EStop);
    let stopped = wait(&c, 1.0, "E-stop", |s| s.state == Some(State::EStop)).machine_pos[0];
    std::thread::sleep(Duration::from_millis(300));
    let after = c.snapshot().machine_pos[0];
    assert!(after < 200.0 && (after - stopped).abs() < 0.5, "{stopped} -> {after}");
    // Reset works again after an E-stop in the middle of a move.
    c.send(Command::Reset);
    wait(&c, 2.0, "Idle after the second Reset", |s| s.state == Some(State::Idle));
}

#[test]
fn homing_references_the_axis_at_its_home_position() {
    let c = start(TEMPLATE);
    reset(&c);
    c.send(Command::Mdi("G0 X3".into()));
    wait(&c, 5.0, "X3", |s| s.state == Some(State::Idle) && (s.machine_pos[0] - 3.0).abs() < 1e-3);
    c.send(Command::Home(vec![0]));
    let s = wait(&c, 10.0, "X homed", |s| s.state == Some(State::Idle) && s.referenced[0]);
    assert!(s.machine_pos[0].abs() < 1e-3, "{}", s.machine_pos[0]);
}
```

- [ ] **Step 2: Run them to see them fail**

Run: `cargo test --no-default-features --test simulated`
Expected: compile error, `no field 'simulated' on type 'ControllerConfig'`.

- [ ] **Step 3: Add the config field**

In `src/config.rs`, inside `pub struct ControllerConfig`, after `running_firmware`:

```rust
    /// Talk to a simulated UC100 instead of USB; nothing is opened.
    /// `serial` and `running_firmware` are then ignored.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub simulated: bool,
```

- [ ] **Step 4: Let `Uc100` talk to USB or the simulator**

In `src/device.rs`:

1. Imports: `use std::time::{Duration, Instant};` and add `use crate::config::MachineConfig;` and `use crate::sim::{self, Simulated};`.
2. Replace the struct:

```rust
/// Where a `Uc100` sends its commands.
enum Link {
    Usb(Ftdi),
    Sim(Box<Simulated>),
}

pub struct Uc100 {
    link: Link,
    fw: Firmware,
}
```

3. In `open`, build `Ok(Uc100 { link: Link::Usb(usb), fw: Firmware::V1041 })`.
4. Add, after `connect`:

```rust
    /// A simulated controller for `cfg`. Nothing is opened; it behaves as
    /// firmware 1041 that has already been identified and started.
    pub fn simulated(cfg: &MachineConfig) -> (Self, Identity) {
        let dev = Uc100 { link: Link::Sim(Box::new(Simulated::new(cfg, Instant::now()))), fw: Firmware::V1041 };
        (dev, sim::identity())
    }

    fn usb(&mut self) -> &mut Ftdi {
        match &mut self.link {
            Link::Usb(usb) => usb,
            Link::Sim(_) => unreachable!("the simulated UC100 has no USB link"),
        }
    }

    fn sim(&mut self) -> Option<&mut Simulated> {
        match &mut self.link {
            Link::Sim(s) => Some(s),
            Link::Usb(_) => None,
        }
    }
```

5. Replace every `self.usb.` with `self.usb().` and `dev.usb.purge()` in `connect` with `dev.usb().purge()`.
6. At the top of each method below, add the simulator branch shown (the USB code after it is unchanged):

```rust
    pub fn identify(&mut self) -> Result<Identity> {
        if self.sim().is_some() {
            return Ok(sim::identity());
        }
        // … existing body …
    }

    pub fn start_firmware(&mut self) -> Result<()> {
        if self.sim().is_some() {
            return Ok(());
        }
        // … existing body …
    }

    pub fn status(&mut self, outputs: u16, flags: HostFlags) -> Result<Status> {
        if let Some(s) = self.sim() {
            return Ok(s.status(outputs, flags, Instant::now()));
        }
        // … existing body …
    }

    pub fn configure(&mut self, cfg: &ConfigPacket) -> Result<()> {
        if self.sim().is_some() {
            return Ok(());
        }
        // … existing body …
    }

    pub fn send_motion(&mut self, records: &[[u8; 12]]) -> Result<()> {
        if let Some(s) = self.sim() {
            return if s.send_motion(records, Instant::now()) { Ok(()) } else { Err(Error::MotionRejected(0)) };
        }
        // … existing body …
    }

    pub fn stop(&mut self) -> Result<u16> {
        if let Some(s) = self.sim() {
            return Ok(s.stop(Instant::now()));
        }
        // … existing body …
    }

    pub fn set_position(&mut self, steps: [i32; p::NUM_AXES]) -> Result<()> {
        if let Some(s) = self.sim() {
            s.set_position(steps, Instant::now());
            return Ok(());
        }
        self.write(&p::encode_set_position(steps))
    }

    pub fn jog_start(&mut self, axis: u8, negative: bool, accel: u16, speed: u16) -> Result<()> {
        if let Some(s) = self.sim() {
            s.jog_start(usize::from(axis), negative, speed, Instant::now());
            return Ok(());
        }
        self.write(&p::encode_jog_start(axis, negative, accel, speed))
    }

    pub fn jog_stop(&mut self, axis: u8) -> Result<()> {
        if let Some(s) = self.sim() {
            s.jog_stop(axis, Instant::now());
            return Ok(());
        }
        self.write(&p::encode_jog_stop(axis))
    }

    pub fn home(&mut self, axis: u8, negative: bool, accel: u16, speed: u16) -> Result<()> {
        if let Some(s) = self.sim() {
            s.home(usize::from(axis), negative, speed, Instant::now());
            return Ok(());
        }
        self.write(&p::encode_home(axis, negative, accel, speed))
    }

    pub fn soft_limits(&mut self, hi: [i32; p::NUM_AXES], lo: [i32; p::NUM_AXES], slow: [i32; p::NUM_AXES]) -> Result<()> {
        if let Some(s) = self.sim() {
            s.soft_limits(hi, lo, Instant::now());
            return Ok(());
        }
        self.write(&p::encode_soft_limits(hi, lo, slow))
    }
```

and for `spindle_speed`, `spindle_on`, `spindle_off` add `if self.sim().is_some() { return Ok(()); }` as their first line.

Remove `#![allow(dead_code)]` from `src/sim.rs` if Task 2 added it.

- [ ] **Step 5: Choose the simulator in `Worker::connect`**

In `src/controller.rs`, `fn connect`, replace

```rust
        let connection = if let Some(fw) = self.cfg.controller.running_firmware {
```

with

```rust
        let connection = if self.cfg.controller.simulated {
            Ok(Uc100::simulated(&self.cfg)).map(|(dev, id)| (dev, Some(id)))
        } else if let Some(fw) = self.cfg.controller.running_firmware {
```

(the rest of the `if`/`else` chain is unchanged).

- [ ] **Step 6: Run the integration tests**

Run: `cargo test --no-default-features --test simulated`
Expected: 4 passed. These run in real time (a few seconds).

- [ ] **Step 7: Run everything both ways**

Run: `cargo test` and `cargo test --no-default-features`; `cargo build --no-default-features --lib --bins` with no warnings.
Expected: all pass.

- [ ] **Step 8: Commit**

```bash
git add src/config.rs src/device.rs src/controller.rs src/sim.rs tests/simulated.rs
git commit -m "Connect to the simulated UC100 with controller.simulated" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: Document it and publish the branch

**Files:**
- Modify: `README.md` (new section after "## Install")

- [ ] **Step 1: Add the README section**

Insert after the "## Install" section's code block:

````markdown
## Simulated UC100

Set `simulated = true` under `[controller]` to run without a controller.
The app, the CLI and programs that embed the library then talk to a
simulated UC100 instead of USB; `serial` and `running_firmware` are
ignored. It reports firmware 1041 with hardware and bootloader `SIMULATED`,
drains motion at the real 1.28 ms record period, and moves only its step
counters. Every profile check still applies, including the commissioning
gate. Nothing physical is simulated: no limit or home switches, no E-stop
input, no communication faults.

Programs that embed the controller can leave out the egui app:

```toml
uc100 = { git = "https://github.com/liorknafo/UC100-driver.git", rev = "<commit>", default-features = false }
```
````

- [ ] **Step 2: Final check and commit**

Run: `cargo test` and `cargo test --no-default-features`
Expected: all pass.

```bash
git add README.md
git commit -m "Document the simulated UC100 and the gui feature" -m "Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

- [ ] **Step 3: Push and report**

Run: `git push` (the branch already tracks `origin/axis-cam-foundation`), then `git rev-parse HEAD`.
Report the full commit hash: AXIS CAM pins it.
