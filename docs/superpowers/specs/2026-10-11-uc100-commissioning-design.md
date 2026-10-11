# UC100 machine control, part 4: machine setup (commissioning) — design

Date: 11 October 2026. Part 3 (running setups) is being built in its own thread, so this part goes ahead of it on its own branch, `t3/uc100-commissioning`, cut from part 2's `t3/uc100-machine-control`. The user asked for no review stops, so this spec was not reviewed before the plan.

Linear: project "AXIS CAM — UC100 machine control", part 4 is LIO-33 (LIO-55, LIO-56, LIO-57; LIO-58 waits, see Non-goals).

## Goal

Do in AXIS CAM the setup work that needs the driver's egui app or a text editor today: edit the UC100 profile, watch the inputs, force outputs for a wiring test, read the setup audit, and calibrate steps per mm.

## Decisions

1. **Where.** The Control page gets a **Control | Setup** switch at the top of its left panel. Setup shares the machine picker, the connection and the log. The profile editor and the audit work while disconnected; the I/O test and calibration need the connection.
2. **Profile editor (LIO-55): a form, not a text box.** Pins and polarity, steps/mm, maximum velocity and acceleration per axis, limit and home switches, homing direction, speed and position, soft limits; E-stop, probe and index inputs; spindle CW/CCW, PWM, direction, PWM frequency, maximum RPM and spin-up; flood, mist, drive enable, charge pump; homing order and limit override; step rate; the commissioning gate. Axes are not added or removed here (the JY5800 is 3-axis; a 4th axis is a TOML edit), and motion tuning and the tool table are left as they are.
3. **Comments survive.** The JY5800 profile keeps its commissioning evidence (dates, ruler measurements) in about 40 comment lines. The form edits the file's `toml_edit` document in place: a changed value keeps its trailing comment, unchanged lines are untouched, a cleared optional field removes its key. `toml_edit` 0.25 was already in the build (through `proc-macro-crate`); it becomes a direct dependency, with no new crate.
4. **Checked with the driver.** After every edit the document is parsed with `MachineConfig::from_toml` (the driver's own validation). Save is offered only when the profile is valid, every number field holds a number, and something changed. The driver's error and the commissioning blocker are shown.
5. **Saving.** Atomic write (`storage::write_atomic`). Refused if the file changed on disk since it was opened (someone edited it by hand): Revert reloads it.
6. **Applying a saved profile reconnects** instead of sending `ApplyConfig`. At the pinned commit, `ApplyConfig` keeps the axes' "position set" flags, so after a steps-per-mm change the soft limits would apply to a position counted with the old calibration. A reconnect clears them: the machine position must be set again (or Keep the last position). Save is offered while disconnected, in E-stop or Idle. The driver-side fix belongs to LIO-59 (part 5, position trust).
7. **Inputs (LIO-56, LIO-57).** Each input pin's raw level, what the profile wires to it, its changes since connecting (the driver's `InputTracker`), the last edges with their time, the E-stop report (`estop_input_report`) and the controller's latch bits. Edges are seen at AXIS CAM's tick (about 33 ms): shorter pulses can be missed, which the page says.
8. **Output test (LIO-57).** Each output pin's readback level and role, with High / Low / Auto. High and Low only while Idle and only on pins the firmware does not drive (step, direction, charge pump, PWM, spindle direction); Auto (release) always. The driver refuses the same and clears forced pins on E-stop; AXIS CAM marks the pins it forced and forgets them on E-stop.
9. **Audit (LIO-56).** The driver's `setup_report` for the profile as edited (when valid).
10. **Calibration (LIO-56).** Pick an axis and a commanded distance; **Move** records the commanded step counter and sends a step jog (soft limits apply). Measure the real travel and enter it: steps per mm = commanded pulses ÷ measured travel (`measured_steps_per_unit`). **Use in profile** puts it in the form; Save applies it (and reconnects, see 6).

## Non-goals

- LIO-58 (retire the egui app for daily use) waits for part 3: AXIS CAM cannot run a job yet.
- No driver change. Adding or removing axes, motion tuning (junction deviation, buffer, default feed) and the tool table stay TOML edits.

## Driver facts relied on (commit 36885a1)

- `Snapshot.status` is the raw `Status`: `inputs` (pins 10, 11, 12, 13, 15 in bits 0–4), `outputs` (applied output word), `position` (commanded step counters), `bits` (stop latch, limits).
- `Command::TestOutput { pin, high }`: High/Low refused unless Idle, an output pin and not firmware-controlled; `None` releases; E-stop clears all.
- Connecting clears `referenced`; `ApplyConfig` does not.
- `MachineConfig::validate`: pin roles and duplicates, step rate versus maximum velocity, soft_min below soft_max, homing order, E-stop verification needs an E-stop input.
- The simulator echoes the output word, counts steps for jogs, and reads configured inputs inactive.
