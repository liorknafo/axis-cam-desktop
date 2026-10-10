# UC100 controller inside AXIS CAM Studio — handoff

Written 10 October 2026 so the work can continue in a new thread on another computer. **No code has been written for this yet.** This is the analysis the user has read, corrected for what the driver can do today.

## The goal

The user's words: "I want to add this program a controller so we will not need to convert to gcode at all and just do all the things in one place."

AXIS CAM Studio should drive the user's UC100 motion controller directly, with no G-code export and no second app.

## Where things are

| Item | Where |
| --- | --- |
| CAM app (Rust + Iced) | `github.com/liorknafo/axis-cam-desktop`, branch `op-actions`. It builds on `holding-solid` (open PR #1); neither is merged into `main` yet. |
| UC100 driver (Rust, crate `uc100`) | `github.com/liorknafo/UC100-driver` (**private**: this computer needs GitHub access), branch `master`, commit `90dfb4b` |
| The machine, in full | driver repo `docs/JY5800-COMMISSIONING.md`. Read it before any hardware work. |
| Machine config | driver repo `configs/jy5800-investigation.toml`. On the MacBook the live copy is `~/.uc100/machine.toml` (only comments differ). |
| Saved offsets and position | `~/.uc100/state.toml`, **only on the MacBook**: G54 = X41 Y-34 Z-10 and the last machine position. See "No home or limit switches" below. |

If the driver repo is missing, clone it into a normal folder, not inside a T3 worktree. Things that exist only on the MacBook are not needed. A 7 October CAM handoff and the exported test programs are there. To make a fresh test program:
1. In the app, import a part (for example `examples/four-setup-block.step` or `.3mf`).
2. Add operations, pick their geometry, then save the project. `--export-gcode` refuses a project without an imported part, and operations need geometry picked in the app.
3. Export from the NC program page, or run `axis-cam --export-gcode <saved project> <out-dir>`.
4. Run `uc100 check <out-dir>/<file>.ngc`. It needs no machine config.

**Building the CAM** needs a C++17 compiler and Open CASCADE 7.9 for STEP import (`OCCT_PREFIX` if installed elsewhere). It also needs a C compiler and CMake, for aws-lc. `AXIS_SKIP_STEP=1` builds without STEP. Only macOS on Apple Silicon has been tested; Windows and Linux builds may need changes (README line 20).

## What exists today

**UC100 driver** has three parts:
- **A library** that drives the UC100 over raw USB (`nusb`). The protocol was reverse engineered from CNCdrive's Mach3 plugins.
  - **Motion:** a look-ahead planner sends the controller a position every 1.28 ms.
  - **G-code:** a Mach3-style interpreter.
  - **`Controller`:** runs motion on its own real-time thread. Commands go in (`src/controller.rs`, `enum Command`), and a status snapshot comes out. The main commands are:
    - connection and safety: `Connect`, `Reset`, `EStop`
    - jogging: `JogStart`, `JogStop`, `JogStep`
    - setup: `Home`, `SetWork`, and `SetMachine`, which declares machine zero on a machine without switches
    - running: `Run { name, lines }`, `Mdi`, `FeedHold`, `Resume`, `Stop`, `FeedOverride`
    - outputs: `Spindle`, `Coolant`
- **"UC100 Control"**, an egui desktop app (`src/gui.rs`).
- **`uc100`**, a command-line tool (`run`, `check`, `mdi`…). The hardware test suites are separate: `cargo run --example selftest|iotest|guitest|reviewtest`.

**AXIS CAM Studio** turns operations into a list of moves per setup: straight XYZ feeds and rapids, no arcs. It checks that list, writes LinuxCNC G-code from it (`src/post.rs`), and re-reads the text to confirm it matches. Nothing in the CAM talks to a device.

**Today the link between them is a file.** The driver's `uc100 check` accepts the CAM's export as it is. The Setup 1 test file was 7,971 lines and 7,947 moves; all six setups (G54–G59) of a saved project passed too.

## The key point: what "no G-code" has to mean

The work zero and the tool length are only known at the machine, and they change during a job:
- at a tool change you touch off Z again;
- at a flip you touch off the new setup.

The driver converts each line from work coordinates to machine coordinates only shortly before it runs. It reads ahead up to 400 moves, but never past a pause. That is why "tool change pause → jog → Zero Z → Cycle Start" works.

So **the CAM must not send final machine positions worked out in advance.** They would be wrong after the first touch-off.

What replaces the G-code text is **a typed list of steps in work coordinates**: rapid, feed, retract, tool change, spindle, coolant, wait, end. The controller converts them as late as it does now. The CAM already builds nearly this list (`post::Block` in `src/post.rs`). It is in the CAM's own setup frame, so the work zero must be subtracted first, which the G-code writer already does.

## Proposed build (not yet approved by the user)

1. **Driver changes:**
   - Split the motion core from the GUI behind a Cargo feature. Today `eframe`, `rfd` and `clap` are all unconditional dependencies and `gui` is a public module in `src/lib.rs`, so the CAM would pull in all of them.
   - Add a "run steps" command (today `Run` only takes G-code lines). It must go through the same converter and the same pause handling.
   - Tag each step with its block number, so the CAM knows which move and operation is running.
2. **CAM changes:**
   - The app owns one connection to the controller.
   - A Machine/Run page:
     - position readout
     - RESET and E-STOP (Esc must work even when a text field has focus)
     - jog, zero, feed override
     - spindle
   - The screen refreshes about 30 times a second while connected.
3. **Everything in one place:**
   - Setup 1 shows the clamping picture and the touch-off point, then Cycle Start.
   - The 3D view shows the real tool and the stock being cut, the current operation and the time left.
   - At a tool change it says which tool to load and where to touch off.
   - At a flip it shows the new orientation and the next offset.
   - Operations the machine can't run are refused before Cycle Start (see M4 below).
4. **Correctness test:** send the same setup through the old G-code route and the new steps route. Both must give identical machine moves.

A cheaper first step is also possible: build the G-code in memory and pass it to the existing `Run`. That needs no driver change. The recommendation was still to go straight to typed steps.

## No home or limit switches (safety-critical)

This machine has **no home switches and no limit switches** (`docs/JY5800-COMMISSIONING.md`, around line 164). The consequences:
- **No axis can home.** `Command::Home` refuses: "Axis X has no home switch configured" (`src/controller.rs`, around line 1267). `[homing] order = "X"` in the config is only a default order.
- **Machine zero is not a physical place.** It is wherever the position counter was declared with `SetMachine`, or what `~/.uc100/state.toml` saved.
- **The CAM's default safe retract, `G53 G0 Z0`, is not a known safe height** (`src/model.rs`, around lines 425 and 438). It assumes machine Z0 is the top of travel. On this machine that holds only if the user set it so.
- **There is no travel check.** No soft limits are configured, so `soft_limit_violation` does nothing (`controller.rs`, around line 775). Even with limits set, the check needs `referenced`, which only homing sets; `SetMachine` does not.
- **Nothing stops an axis at the end of its travel.**

Any "check against the real travel" feature has to start from this. The user decides how machine zero and travel limits are defined.

## Other design findings

**Tool changes and flips**
- After the first tool faces the top, the Z0 surface is gone. Later tools must touch off on a fixed reference, such as a vise jaw, and the user enters its height. The CAM's "touch off each tool" mode already writes this instruction.
- **Don't mix the two tool-length methods.** If the driver's tool table is filled and you also touch off at the pause, Z will be off by the difference between the two tool lengths. Keep the table empty and touch off.
- **Tool-table mode** (the default): each program starts with `T<first tool> M6` and `G43 H<tool>` (`src/post.rs`, around line 648). The program pauses once at the start, even if that tool is already loaded. M6 stops the spindle and coolant before pausing; jog, Zero and MDI work during the pause.
- **Touch-off mode:** no M6 is written, so the program never pauses. The CAM splits the job into one program per tool. LinuxCNC needs that split; the driver doesn't, because it allows touch-off during a pause.
- **A flip must switch to the next offset (G55…) before it pauses.** Otherwise "Zero" overwrites G54.
- The driver supports only G54–G59, while the CAM offers 9 offsets. The limit is in the driver code, not the hardware.

**Where the machine differs from what the CAM assumes**
- The driver ignores the CAM's `G64 P0.010` and uses its own corner tolerance (`junction_deviation`, 0.02 mm).
- Probing: G38.2/G38.3 are parsed, but the controller stops the program with "probing (G38) is not supported yet". Touch-off is by jogging and pressing Zero.
- **Rapids:** the CAM assumes 6000 mm/min, hard-coded in `src/cam.rs`, `src/pocket.rs` and `src/simulation.rs`. This machine is set to 1000 mm/min on X and Y and 500 mm/min on Z, with acceleration 50. Time estimates will be far too short until the CAM reads the machine's limits.
- **Counter-clockwise spindle:** the spindle direction output is not assigned. A program still starts, but reaching M4 puts the machine into software E-stop mid-program (`controller.rs`, around line 1004). The CAM writes M4 for counter-clockwise operations, so it must refuse them before Cycle Start.
- No coolant output is configured on this machine.

## The machine (from `configs/jy5800-investigation.toml` and the commissioning doc)

- **Machine:** Jingyan CNC4030-800W with a JY5800 control box. UC100 serial CD9090D3, firmware 1041, 25 kHz step rate.
- **Axes:** 320 steps/mm on X, Y and Z, measured by ruler on 10 October. X is reversed so +X moves right. Max 1000/1000/500 mm/min (not yet tested at full speed), acceleration 50. Verified: the pinout, the E-stop input (pin 10), and that the drives need no enable signal.
- **Spindle:** PWM on pin 16, active-low, 1 kHz. A 10% run started and stopped. **RPM is not calibrated**; `spindle_max_rpm = 24000` is a placeholder.
- **Still unverified:**
  - spindle direction and RPM
  - whether the mushroom E-stop cuts spindle and motor power
  - behaviour on power loss and on USB loss. The software E-stop on USB errors doesn't prove what the machine does when the cable drops.
- **New in `90dfb4b`, the latest driver commit:**
  - the commissioning gate: Reset and motion stay blocked until the config marks the checks verified
  - the guarded reset (`src/reset.rs`)
  - spindle changes:
    - pin 16 holds its inactive level
    - an optional `spindle_dir` output
    - reversing a running spindle is always refused; CCW is refused without a direction output
  - a failed spindle or reset command enters E-stop at once
  - the diagnostics (`src/diagnostics.rs`, `re/diagnostics/`)

## Safety

- **Never send motion or spindle commands unless the user is at the machine and has asked for it.** Ask which computer the UC100 is plugged into before any hardware step.
- If the UC100 moves to a different computer, that computer has no `state.toml`, so the saved G54 and last position are lost. On a machine without switches, re-establish machine zero before running anything.
- If the controller runs inside the CAM and the CAM crashes, the UC100 stops on its own after about 1 second. At 1000 mm/min that is about 17 mm of travel. Running the controller as a separate process would reduce that risk.
- **CPU timing:**
  - This machine's config queues only **40 ms** of motion (`buffer_ms = 40`; the driver default is 150 ms). The CAM's heavy 3D work could starve the motion thread.
  - Only macOS raises the motion thread's priority (`controller.rs`, around line 202). On Windows or Linux it runs at normal priority.
- The driver's USB layer (`nusb`) has only run on macOS. On Windows the FTDI chip may need the WinUSB driver. Check before promising anything there.

## Decisions waiting for the user

1. Should G-code export stay as an optional extra (backup, other machines), or be removed?
2. Should the CAM use the driver as a git dependency, or should the driver move into the CAM repo?
3. With no home or limit switches: how should machine zero, the safe retract height and the travel limits be set? Options:
   - the user declares them by jogging
   - add switches
   - keep `state.toml`
4. Should the controller run inside the CAM app, or as a separate process?

Start by asking these. Don't write integration code until the user has answered.

## How this user works

- Explain in plain language. Judge pictures and motion as a machinist would.
- "Do everything, don't wait for my approval" applies to the CAM work. Moving the real machine still needs the user present.
- An independent review must accept the work before it is reported done.
- To check a "looks wrong" complaint, render the views before and after the operation, then fix.
- The 3D viewport is mirrored left/right; `look.rs` pictures flip to compensate.
- Commit messages end with a `Co-Authored-By:` line naming the model that wrote them.
- Never let tests or agents write to the app's real data folder: `dirs::data_local_dir()/AXIS CAM Studio` (`src/storage.rs`). On macOS that is `~/Library/Application Support/AXIS CAM Studio`; on Windows, `%LOCALAPPDATA%\AXIS CAM Studio`. Tests use temporary folders.
