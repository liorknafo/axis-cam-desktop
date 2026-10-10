# UC100 machine control, part 1: foundation — design

Date: 11 October 2026. The scope and the split between the two repositories were approved in conversation; the details in §2–§6 are first written down here for review.

Linear: project "AXIS CAM — UC100 machine control", part 1 is LIO-30 (sub-issues LIO-36…40, LIO-48).

## Goal

AXIS CAM becomes the one program for CAM and for running the CNC: import, CAM, simulation, homing and zeroing, cutting, with no G-code export step. The machine is the JY5800 / CNC4030 on a CNCdrive UC100 USB motion controller, driven by the Rust library in `github.com/liorknafo/UC100-driver`.

Part 1 lays the foundation. When it is done, AXIS CAM can connect to a UC100, either the real one (on the Mac) or a simulated one (on any computer), show the machine's live state, and offer Reset and E-stop. Nothing moves yet.

## Decisions already made

- AXIS CAM covers everything, including machine setup (later parts). The driver's egui app was a proof that the driver moves the real machine and is retired for daily use once AXIS CAM covers it.
- The driver stays its own repository. AXIS CAM depends on it as a crate pinned to a git commit.
- Real machining happens on macOS. On this Windows PC everything runs against a simulated UC100 inside the driver, so the driver's real controller, planner and job code run everywhere; only the USB bytes are simulated.
- Driver issues found in the earlier review are fixed later (part 5), not here.
- Tool changes are manual (Z touch-off between tool runs); this matters from part 3 on.
- The machine will be drawn in AXIS CAM's existing 3D view (part 2), not with a game engine.

## Non-goals (later parts)

- Jogging, homing from the UI, work zero, spindle and coolant controls, MDI, live tool drawing (part 2).
- Running setups, the structured job command, admission checks, the tool-change flow (part 3).
- Editing the UC100 profile inside AXIS CAM, diagnostics, I/O test (part 4).
- Driver hardening and Windows USB (part 5). Camera (part 6).
- Agreement between the AXIS CAM machine's travel and the UC100 profile's soft limits (part 3, where it gates Cycle Start).

## Architecture

```
AXIS CAM (Iced UI thread)                       uc100 crate (driver)
  Control page, top-bar E-stop, Esc   ──send(Command)──▶  Controller ── uc100-io thread: Worker
  Tick (33 ms while connected)        ◀──snapshot()────   planner, reset sequence, streaming
                                                           │
                                                           Uc100 device
                                                           ├─ USB (FTDI via nusb)   real UC100
                                                           └─ Simulated             any computer
```

AXIS CAM uses the driver's existing `Controller` API unchanged: `start`, `send`, `snapshot`, `shutdown`. The simulation sits below the controller, so the controller cannot tell the difference.

## 1. Driver changes (UC100-driver)

### 1.1 Optional GUI (LIO-48)

- New cargo feature `gui`, on by default. It gates `eframe` and `rfd` (optional dependencies), `pub mod gui`, and the `uc100-gui` binary (`required-features = ["gui"]`). Tests that need egui are gated the same way.
- AXIS CAM depends with `default-features = false` and so does not build egui. Everything else (protocol, device, planner, controller, config, diagnostics, `macos::prevent_app_nap`) stays available on every platform. A GUI-less copy of the driver was checked to compile on Windows with Rust 1.91.
- The `uc100` command-line program stays in the default build and keeps working as today.

### 1.2 Simulated UC100 (LIO-37)

**Selecting it.** A new configuration field `controller.simulated` (boolean, default `false`). When true, `Worker::connect` opens the simulated device instead of USB, and `controller.serial` and `controller.running_firmware` are ignored. No API signature changes; the CLI and the egui app can use the simulator with a profile that sets the field. AXIS CAM sets the field itself from its own Connection choice (§2.2), overriding whatever the TOML says.

**Where it lives.** `Uc100` keeps its methods; inside, it talks either to USB or to a `Simulated` device in a new module `src/sim.rs`. The controller, reset sequence and planner are unchanged.

**Identity.** The connect path reports bootloader state 1, firmware `1041`, hardware `SIMULATED`, bootloader `SIMULATED`, and behaves as firmware 1041 (100 records per packet). The controller's log therefore reads "Connected: UC100 firmware 1041 (hardware SIMULATED, bootloader SIMULATED)".

**Clock.** The simulator advances by real elapsed time on every call. Motion records drain at one per 1.28 ms (the firmware's period), so motion takes as long as on the machine.

**Behaviour of each call.**

| Call | Simulated behaviour |
| --- | --- |
| `send_motion(records)` | Appends to a queue of at most `DEVICE_BUFFER_RECORDS` (900); more is rejected like the firmware (`MotionRejected`). |
| draining | Each record's six words are decoded (magnitude in the low 15 bits, bit 15 = negative) into fixed-point deltas at the profile's step rate; the step counters follow. |
| `status(outputs, flags)` | Returns counters, queued records (+1, as the firmware reads 1 when empty), bits, inputs and the outputs word last written. Bit 3 is always set, as on the real controller. `running` while records are queued; `jogging` and the axis flag while jogging; `homing` while homing. |
| stop latch (bit 1) | Set while the host E-stop flag is set. It stays set after the flag is cleared, until a `stop()` arrives with the flag clear: the firmware 1041 behaviour the reset sequence was written for. |
| `stop()` | Returns the number of queued records, clears the queue, ends jogging and homing, clears the latch as above and clears the soft-limit bit. |
| `jog_start` / `jog_stop` | Continuous motion of one axis at the speed decoded from the speed word (inverse of `speed_word`), without ramps; stops on `jog_stop` (axis 6 = all). |
| `home(axis, negative, …)` | Sets the homing bit and moves the axis in the given direction at the commanded speed until its counter reaches the switch, which the simulator places at the profile's `home_position`. If the axis is already at or past the switch in that direction, it finishes at once, as the real firmware does when sitting on the switch. |
| `soft_limits(hi, lo, slow)` | Stored. While the host soft-limit flag is set, motion that would cross a limit stops at the limit and sets the soft-limit bit. |
| `set_position(steps)` | Sets the counters. |
| `configure`, `spindle_speed`, `spindle_on`, `spindle_off` | Accepted and remembered; no physical effect. |
| inputs | All five input pins read HIGH (unconnected), except that every input the profile configures (E-stop, limits, home, probe) reads its inactive level, so Reset succeeds. |

Never reported by the simulator: hard limit, communication lost, MPG counts, THC. It never fails a USB call. Fault injection belongs to part 5.

**Acceptance.** After any program, the simulated counters equal the planner's final position in steps (within one step). If the encoder's direction-reversal correction makes plain accumulation drift, the simulator applies the firmware-equivalent correction; the test decides.

### 1.3 Driver tests

- Record decoding: a sequence of `MotionEncoder` records, including direction reversals, leaves the simulated counters at the encoder's position within one step.
- The reset sequence completes against the simulator with the JY5800 profile and with the template profile.
- End to end through the real `Controller`, with `controller.simulated = true`: start → E-stop "Press Reset" → `Reset` → Idle within 2 s → `Mdi("G0 X5")` → back to Idle with machine X = 5.000 → `EStop` during a long move stops the counters → homing an axis sets it referenced at its home position.
- `cargo test` with and without default features.

## 2. AXIS CAM changes (axis-cam-desktop)

### 2.1 Dependency (LIO-38)

```toml
uc100 = { git = "https://github.com/liorknafo/UC100-driver.git", rev = "<commit>", default-features = false }
```

The repository is private. A new `.cargo/config.toml` sets `[net] git-fetch-with-cli = true`, so cargo fetches with git's own login (on this PC: `gh auth setup-git`, which makes git ask the GitHub CLI; on the Mac, whatever already lets git fetch the repo). No token is written to any file. The pinned commit must be pushed to GitHub; moving to a newer driver means changing `rev`.

### 2.2 UC100 machine profiles (LIO-39)

- The wizard's controller list gains **UC100**. Choosing it shows two fields under a "UC100" heading instead of the LinuxCNC postprocessor fields:
  - `uc100_profile`: the UC100 profile file (the driver's TOML, e.g. `configs/jy5800-investigation.toml`), a text field with a Browse… button.
  - `uc100_connection`: `USB` or `Simulated`. Default: `Simulated` on Windows, `USB` on macOS.
- `Machine::issues()` (no file access) requires a non-empty profile path for a UC100 machine. The file itself is loaded and validated with `uc100::config::MachineConfig::load` when the path changes and again at Connect; the result is shown under the field with the driver's message ("invalid config: …").
- The choice help explains UC100: AXIS CAM drives the machine directly; G-code export is not offered for it. Export for a UC100 machine is refused by the existing "only the LinuxCNC postprocessor is implemented" rule; nothing changes there.
- Existing projects and LinuxCNC machines are unaffected; the new fields are plain entries in the machine's field map.

### 2.3 Connection (LIO-40)

A new module `src/control.rs` owns the connection:

- `Connection { controller, machine_id, simulated, snapshot, _nap }`.
- `connect(machine)`: loads and validates the profile, sets `controller.simulated` from `uc100_connection`, picks the state file, calls `Controller::start`, and takes `uc100::macos::prevent_app_nap("AXIS CAM machine control")` so macOS cannot throttle AXIS CAM in the background and starve the motion buffer (a no-op elsewhere).
- State files (the driver's saved work offsets and last position): `<local data>/AXIS CAM Studio/uc100/<machine id>.toml` for USB and `<machine id>-simulated.toml` for the simulator, so a simulated session never overwrites the real machine's offsets or last position.
- `disconnect()`: sends `Disconnect`, then drops the controller, which shuts its thread down. Closing the app does the same before the window closes.
- One connection at a time. The machine picker and the profile fields are locked while connected.

### 2.4 Control page (LIO-40)

- A **Control** button in the top bar opens `Page::Control`.
- Left: a picker of the project's UC100 machines, the machine's Connection (read-only, from its profile), the profile check result, **Connect** / **Disconnect**.
- Main area:
  - The state in large type (Disconnected, E-stop, Idle, Jog, Homing, Run, Hold, Stopping, Paused), a **SIMULATED** badge when simulated, and the E-stop reason.
  - **Reset** (enabled in E-stop) and **E-stop**.
  - The controller identity line.
  - A position table for every enabled axis: machine and work coordinates (mm, 3 decimals) and whether the axis is homed.
  - Spindle state and RPM, flood and mist, feed rate and override.
- A log pane with the driver's log, newest last.
- **E-stop is always reachable while connected**: a red E-STOP button appears in the top bar on every page, and Esc sends E-stop from any page, even while typing in a field (as in the driver's app). E-stop needs Reset to leave, so an accidental press is harmless.
- While connected, the existing 33 ms tick runs and each tick copies `controller.snapshot()` into the connection; the page draws from that copy.

## 3. Error handling and safety

- Invalid or missing profile: Connect is disabled and the reason is shown; the driver also refuses an invalid configuration at connect and logs why.
- Connect failure (no UC100, wrong driver, unsupported firmware): the driver's message appears in the log and the state stays Disconnected. On Windows a USB connection is expected to fail (part 5); the message is shown as-is.
- Connected machines start in E-stop ("Press Reset to enable the machine"), as the driver does today. Nothing in part 1 commands motion.
- The profile's commissioning gate applies in simulation exactly as on the machine, so a profile behaves the same in both. The JY5800 profile records all four checks, so Reset is allowed with it.
- The SIMULATED badge, the identity line and the separate state file keep a simulated session from being mistaken for, or overwriting, the real machine.
- USB loss is handled by the driver as today (E-stop, reconnect attempts); AXIS CAM only displays it.
- The camera ideas (part 6) are aids, never safety devices; the hardware E-stop remains the protection on the machine.

## 4. Testing

- Driver: §1.3.
- AXIS CAM, in the existing `smoke.rs` style:
  - A UC100 machine with an empty profile path reports the issue; a valid profile (the driver's `config::TEMPLATE` written to a temporary file) passes; a broken file shows the driver's message.
  - The Control page renders disconnected, connected in E-stop, and connected Idle.
  - Connect to the simulated UC100 → E-stop → Reset → Idle within 2 s → E-stop (button) → E-stop state → Disconnect → Disconnected; the simulated state file is written, the USB one is not.
  - Esc sends E-stop only while connected; the top-bar E-STOP appears only while connected.
  - Closing the app while connected shuts the controller down.
- On the Mac with the real UC100 and no motion: Connect (USB) shows the real identity and E-stop, Reset reaches Idle, the E-STOP button and Esc each return to E-stop, Disconnect works, and the app keeps the connection while in the background.

## 5. Delivery

- This Windows PC is upgraded to the current stable Rust (≥ 1.95), so the driver's own egui tests build here. AXIS CAM itself does not need it.
- Driver work happens on a branch of UC100-driver in its own worktree and T3 thread. AXIS CAM work happens on `t3/uc100-controller-integration` in its own worktree and thread, after the driver branch is pushed (the pinned commit must exist on GitHub).
- The README code map and `docs/VALIDATION.md` are updated, as in earlier features.
