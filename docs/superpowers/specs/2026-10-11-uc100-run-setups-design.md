# UC100 machine control, part 3: run setups without G-code — design

Date: 11 October 2026. The user asked for no review stops, so this spec was written and followed without one; the decisions below were made here.

Linear: project "AXIS CAM — UC100 machine control", part 3 is LIO-32 (LIO-49…53; LIO-54 is left for the Mac). Two part 5 issues are taken too, because they live in the same driver code: LIO-61 and LIO-62.

## Goal

AXIS CAM runs a setup on the UC100 directly: the checked post blocks of each tool run are lowered to the driver's actions and sent as one job, with no G-code text in between. The Control page gets Cycle Start, Hold, Resume and Stop, progress per operation and block, the manual tool change between tool runs, and the checks that must pass before Cycle Start. The Work page's 3D view shows the live tool over the simulation.

Parts 1 and 2 (`2026-10-11-uc100-foundation-design.md`, `2026-10-11-uc100-machine-control-design.md`) are the base.

## Decisions

1. **One job per tool run.** Touch-off mode, as the LinuxCNC post does with "Touch off each tool": a setup splits into one program per run of the same tool. The tool change happens between jobs, in Idle, on the Control page. The driver's program pause (M0/M6) is not used by AXIS CAM.
2. **Retract to the setup clearance plane.** UC100 runs use the post's "Setup clearance plane" retract: `post_check` already proves that plane is above the stock top and above every jaw and clamp with its gap, so it is the "checked clearance" of LIO-52. The tool's top of travel is not used: moving onto a soft limit risks the controller's slow-down zone and a soft-limit E-stop.
3. **Machine target = plan point − datum + live work offset** (X, Y, Z; A, B, C stay where they are). The work offset is the driver's `work_offset` (active WCS + G92 + tool length) read when Cycle Start is pressed, right before lowering. Feeds: mm/min ÷ 60 = mm/s; rapids have no feed.
4. **Each action's `line` is its block's index + 1**, so the driver's `current_line` names the block being cut: that gives progress per operation and block, and the live position in the simulation.
5. **The driver checks again.** AXIS CAM's admission checks give clear messages and grey out Cycle Start; the driver's job preflight (the same soft-limit check as Run) still refuses anything outside the limits.
6. **Nothing of a run is saved.** The run state lives on the connection: disconnecting forgets which tool runs were done and the reference surface.

## 1. Driver (UC100-driver, branch `axis-cam-run`)

### 1.1 Structured job (LIO-49)

```rust
/// Runs actions in machine coordinates (a CAM program lowered by its host,
/// such as AXIS CAM) through the same checks and execution as `Run`, without
/// G-code. `Located::line` is the caller's own tag, reported as `current_line`.
Job { name: String, actions: Arc<Vec<Located>> },
```

- `Program` holds either G-code lines or actions (`Source::Gcode` / `Source::Actions`). `fill_planner` interprets the next line, or takes the next action; from there the path is the same: the planner, sync points, dwell, spindle spin-up, coolant, Pause, End, Hold, Stop, E-stop, the outcome. `program_lines` is the number of actions.
- Preflight, like Run: the machine is idle; Override limits is off; every number is finite, a move's feed is positive, a dwell is not negative; no Probe (not supported, as in G-code); the soft-limit check (`soft_limit_violation`, "line N: X … is outside the soft limits"). A refusal finishes the program as not started: "`<name>`: not started, …".
- Log: "Running job `<name>` (n actions)". Finished: "Program `<name>` finished", like Run.

### 1.2 Pause timing and safe re-entry (LIO-61)

**Timers.** A dwell or spindle spin-up wait is a deadline today, so it keeps running through a feed hold and may be over when the operator resumes. `Wait` gains `Held(remaining)`: FeedHold freezes the remaining time, Resume from Hold restarts it.

**Re-entry after a program pause.** A Pause action records where the program stopped: the planner position, the work offset, the spindle (state, rpm), flood and mist. While paused the operator may jog, run MDI, set work zero, run the spindle or coolant. Resume then:

1. Refuses (stays Paused, logs why) if the spindle now turns the other way from the recorded direction: reversal needs a standstill the driver cannot see.
2. Moves what is left of the program with any change of work offset d = offset now − offset at the pause: pending actions and a job's remaining actions shift by d; G-code lines not yet read already use the new offsets. The interpreter's position becomes the pause point + d, so axes a line leaves out stay where the program had them.
3. Restores the recorded spindle (a start waits the spin-up, as M3 does) and coolant.
4. Returns to the pause point + d if the machine is elsewhere: Z up to the higher of the two heights, across, then down, all rapids.
5. Checks the return moves, the pending actions and the rest of the program against the soft limits (G-code on a copy of the interpreter); a violation refuses Resume ("Resume refused: …") and the program stays paused.

A pause where nothing changed resumes exactly as before.

### 1.3 Spindle command failures (LIO-62)

Since commit 90dfb4b a failed PWM spindle command enters E-stop, which ends a running program as failed ("Program aborted by E-stop: Spindle command failed: …"). This part adds what was missing: the simulator can be told to fail spindle commands (tests only), and tests show a job fails when its spindle start fails and when the spindle stop at its end fails.

### 1.4 Driver tests

- Through the real Controller and the simulator: a job reaches its targets, reports its tags as `current_line`, finishes ok; it is refused outside the soft limits, when not idle, with Override limits on, with a NaN or a probe; Hold and Resume, Stop.
- A job with a Pause: jog away during the pause, Resume returns to the pause point and finishes at the job's target; a work zero change during the pause moves the rest of the job; a Resume whose remaining moves would leave the soft limits is refused and stays paused; a spindle turning the other way refuses Resume.
- Feed hold during a dwell: the dwell's remaining time is kept.
- A failed spindle command fails the job.

## 2. AXIS CAM

### 2.1 Checked programs for a UC100 setup (LIO-50)

- `PostSettings::for_uc100(machine)`: the machine is a 3-axis UC100 machine without issues; touch-off mode (no tool table), clearance-plane retracts, no blending (the driver's planner uses the profile's junction deviation), the machine's spindle start dwell (`post_dwell`, default 2 s; the wizard shows it for UC100 too), and the Air output (`post_air`: M7 = mist, M8 = flood).
- `post.rs`'s export walk is shared: the same stock, work zero, workholding, `check_setup` and holding checks run for a UC100 setup, without emitting or replaying G-code. G-code export for a UC100 machine stays refused.
- `run::programs(doc, mesh, setup)` simulates the job up to that setup (in the background, as the NC page does) and returns its tool runs: blocks, operation states, the setup's operation indexes, datum, WCS, and which plan move each block comes from.

### 2.2 Lowering (`src/run.rs`)

`lower(program, offset, start) -> Vec<Located>`, block by block:

| Block | Actions |
| --- | --- |
| Stop | Coolant off, then Spindle Off, for what is on |
| Retract | Move (rapid) to the clearance plane at the current X/Y; the first one at the machine's X/Y when Cycle Start was pressed |
| Start | — |
| Run | Spindle Cw/Ccw at the operation's rpm; Coolant (flood, mist) |
| Wait | Dwell |
| Approach | Move (rapid) across at the clearance plane |
| Descend | Move (rapid) down to the first point |
| Move rapid / feed / dwell | Move (rapid) / Move at feed ÷ 60 / Dwell |
| End | End |

Zero-length moves are dropped. Every action's line is its block index + 1.

### 2.3 Admission checks (LIO-51)

Cycle Start is offered only when all of these pass; otherwise the Run panel lists what is wrong:

1. The machine is Idle and Override limits is off.
2. The setup's machine is the connected machine.
3. The profile has soft limits on X, Y and Z, and the AXIS CAM machine's travel (X, Y, Z, in mm) agrees with them within 0.01 mm.
4. X, Y and Z have a machine position (referenced), so the soft limits are on.
5. The setup's work offset is the machine's active work system (G54…G59; G59.1–G59.3 are refused).
6. The tool run fits inside the soft limits at the current work zero: the program's extents (moves, descents, the clearance plane) − datum + work offset.
7. The spindle: no operation above the profile's `spindle_max_rpm`; CCW only with a direction output.
8. Coolant: an operation's flood or mist needs that output in the profile.
9. Tool changes: once a tool run has started in this run, the X/Y work offset and the WCS must stay as they were; a tool run with a different tool than the last one needs a Z touch-off since that run ended (Zero Z, Zero all or Set Z from reference on the Control page, or any change of the Z work offset).
10. The checked programs are current: an edit to the project after the check needs a new check.

### 2.4 Running (LIO-53)

A **Run** panel on the Control page, under the soft-limits line:

- The setups whose machine is the connected one. Picking one checks it (background) and lists its tool runs: number, tool, operations; done ✓, running ▶, next →. Clicking a tool run makes it the next one (not while running).
- **Cycle Start** (admission passes): lowers the next tool run with the live work offset and sends `Job { name: "<setup> · tool run i of n", actions }`. **Hold** (Run), **Resume** (Hold or Paused), **Stop** (Run, Hold, Paused).
- Progress: tool run i of n, operation k of m and its name, block j of the run's blocks, a bar.
- The job's outcome when it ends; a finished run moves "next" on; a failed one stays next.

**Live tool in the 3D view.** While a job of the setup shown on the Work page runs, its viewport draws the live tool in red at the machine position in plan coordinates (machine − offset at the start + datum), with the running tool's diameter and stickout. When the running operation's simulation is the one shown and not playing, it is moved to the block being cut.

### 2.5 Manual tool change (LIO-52)

Between tool runs, when the next run has another tool, the Run panel shows:

1. The last run ended with spindle and coolant off at the clearance plane (its blocks: Stop, Retract, End).
2. Before removing the old tool, touch it on a surface that stays put (vise jaw, table, a block) and press **Record reference**: AXIS CAM keeps that point's work Z. It can be recorded any time a tool is zeroed, also before tool run 1.
3. Jog up and change the tool to the next run's T#.
4. Touch the new tool on the same surface and press **Set Z from reference** (sends SetWork Z = the recorded work Z), or Zero Z on the stock top when the earlier runs have not cut it.
5. Cycle Start: admission rechecks the next run with the new Z offset (§2.3: X/Y zero kept, Z touched off, inside the soft limits); the run starts with Retract, Approach and Descend, all checked by `post_check`.

## 3. Safety

- E-stop, the top-bar E-STOP and Esc are unchanged; Stop decelerates then aborts (spindle off).
- Nothing starts without the admission checks, and the driver repeats the soft-limit check.
- The first Retract moves Z alone at the current X/Y to the checked clearance plane: the operator must leave the tool where a vertical move is safe, as with the LinuxCNC programs ("jog the tool clear of the part before Cycle Start").
- The assumed JY5800 travel (X 0…390, Y 0…280, Z −55…0) is in the profile and must be measured: admission compares the AXIS CAM machine's travel with it, so both must be corrected together.

## 4. Testing

- `run.rs` unit tests: lowering (targets, feeds, tags, spindle/coolant/dwell/end, zero-length moves, first retract), admission (each refusal of §2.3).
- `post.rs`: a UC100 setup is checked like export and split per tool; export still refuses UC100.
- `smoke.rs`, against the simulated UC100 with a fast profile: check a two-tool setup, Cycle Start runs tool run 1 to the end; Cycle Start for run 2 is refused until Z is touched off; Record reference and Set Z from reference; run 2 runs; Hold and Resume; Stop; the live position follows the machine; the Run panel renders.
- In the real app against the simulator (Windows): a setup run end to end.
- On the Mac (LIO-54): the attended checklist in `docs/VALIDATION.md`.
