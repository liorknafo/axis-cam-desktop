# Manual lathe, phase 1 — Implementation Plan

> **For agentic workers:** implement task by task (one implementer per task, checked before the next; a whole-branch review by astra at the end). Steps use checkbox (`- [ ]`) syntax.

**Goal:** see the spec's Goal. Phase 1 = spec §1–§7.

**Spec:** `docs/superpowers/specs/2026-10-17-manual-lathe-design.md`. **Approved mockup:** `/home/liorknafo/Downloads/lathe-step-screens.html` (screenshot `lathe-step-screens.png`). **Precedent:** the manual mill (`docs/superpowers/specs/2026-10-17-manual-milling-design.md`, `src/manual.rs`, `src/guide.rs`, `src/steps.rs`, `Strategy::Box` in `src/cam.rs`) — follow its patterns.

## Global Constraints

- Worktree `/home/liorknafo/.t3/worktrees/axis-cam-desktop/t3-68de786c`, branch `t3/lathe-planning`.
  - In every Bash call that runs cargo: `export PATH=$HOME/.cargo/bin:$HOME/tools/dev/bin:$PATH` first. Never kill a cargo process (state T = paused by a throttler; wait).
  - Targeted `cargo test --locked <filter>` in the foreground (timeout 600000 ms). The full suite detached in its own Bash call (`setsid nohup bash -c '…cargo test --locked -- --test-threads=6 > /tmp/lathe-tN.log 2>&1; echo "EXIT $?" >> /tmp/lathe-tN.log' >/dev/null 2>&1 &`), polled in separate foreground calls of at most 25 × 20 s. Never finish while cargo runs; no Monitor or background waits.
  - `cargo fmt` as its own call; `cargo clippy --all-targets --locked` with no warnings and no `#[allow]` (dead code that a later task uses is allowed only if listed in the report).
  - Do not run the app window or `--smoke-test`.
- Old documents load unchanged. Mill behaviour (CNC and manual) does not change; CNC programs stay byte-equal.
- Lathe setups never reach the post, export or UC100 Run (`manual::is_hand`).
- Lathe work coordinates: Z 0 at the free end, negative toward the chuck; X as a diameter in everything the user sees.
- Every new field key has a `help::field_help` arm; every new `Message` a `help::command` arm (and `while_locked` for guide messages).
- UI text is short labels; the guide is pictures and dials, not prose (user requirement).
- Commits end with a blank line and `Co-Authored-By:` naming the model; titles end with `(LIO-175)`.

## Review Focus

1. Dial conversions (cross slide in both `cross_reads` modes, carriage, top slide, quill) — a wrong reading is a wrong cut. (Tasks 1, 7)
2. Revolve cuts remove exactly their ring, in the right frame, and mill setups see them. (Task 3)
3. Turn passes never cut inside the allowance or the part; the finish pass and the measure step. (Tasks 5, 7)
4. 3-jaw preferred; chuck checks. (Task 6)
5. No lathe setup reaches the post/export/Run. (Task 6)

---

### Task 1: The lathe machine and dial conversion

**Files:** new `src/lathe.rs` (+ `mod lathe;`), `src/manual.rs` (`is_hand`), `src/model.rs` (`Machine::issues`), `src/ui.rs` (wizard: kind choice gains Lathe; lathe steps Identity, Spindle & travel, Dials & feeds, Review; library card "Manual lathe · no G-code"), `src/main.rs` (wizard edits), `src/help.rs`.

**Produces:** `lathe::{is_lathe, defaults() -> Machine, Slide {Cross, Top, Carriage, Quill}, dial(&Machine, Slide, distance: f32) -> Result<(u32, f32), String>, feeds(&Machine) -> Vec<f32>, pitches(&Machine) -> Vec<f32>}`; `manual::is_hand`; the spec §1 keys and defaults.

- [ ] Tests: kinds (`is_manual` only for the mill, `is_lathe` only for the lathe, `is_hand` both); defaults valid; `Machine::issues` for the lathe (positive `*_turn`, whole positive `*_div`, parsable `speeds`/`feeds`/`pitches`, `cross_reads` one of `Diameter`/`Radius`/empty); dial: 6.00 off the diameter with `cross_turn` 4 → (1, 2.00) with `Diameter`, (0, 3.00) with `Radius`, `Err` when `cross_reads` is empty; carriage 65 with `carriage_turn` 20 → (3, 5.0); readings rounded to the dial's division; negative distances refused.
- [ ] Wizard tests (probe pattern), as Task 2 of the manual-mill plan did.
- [ ] Every existing `is_manual` call that means "no G-code / no Run / no post" uses `is_hand` (list them in the report); the ones that mean "the manual mill" (Box, gears snap, the mill guide) stay.
- [ ] Commit "Add the manual lathe machine and its dials (LIO-175)".

### Task 2: Lathe tools and chucks

**Files:** `src/model.rs` (`Tool::issues` per type), `src/ui.rs` (tool type list + per-type fields; holder editor for `Chuck`; holding choice gains Chuck), `src/holding.rs` (`KINDS`, `Holder::keys`, `issues`, starters, `Layout::Chuck { holder, grip, stickout }`, `layout`, `refusal`), `src/help.rs`, `src/main.rs` (AddOp tool pick later uses these).

- [ ] Tests: each lathe tool type's issues (no flutes asked; `nose_radius` ≥ 0; `width` > 0; `thread_angle` 55/60); lathe tools default to fitting lathes only; chuck starters valid; chuck issues (`jaws` 3/4, `min_grip < max_grip`, positive `jaw_length`, `body`); `Layout::Chuck` read from setup keys; a Vise/Clamp is refused on a lathe and a Chuck on a mill.
- [ ] Commit "Add lathe tools and chucks (LIO-175)".

### Task 3: Round bar and revolve cuts

**Files:** `src/stock.rs` (`Round bar` mode: `stock_diameter`, `stock_length`; relative: `stock_radial`, `stock_top`, `stock_bottom`; `sheet_description`), `src/volume.rs` (`Volume::new` clears outside the cylinder; `swept` for `Revolve`), `src/simulation.rs` (`CutterShape::Revolve`), `src/stock_surface.rs` (cylinder field; `Cut::field` and `extend` for Revolve), `src/rest.rs` (revolve cuts are not evidence), `src/viewport.rs` (block colouring on the cylinder), `src/ui.rs` (setup editor stock fields).

**Produces:** `stock::is_round(&Setup)`, `Volume::revolve(z0, z1, r0, r1, current: Frame, source)` (pushes a `Cut` with `shape: Revolve`, `a = [z0, r0, 0]`, `b = [z1, r1, 0]` in the `current` frame's axis).

- [ ] Tests: Ø50 × 100 round bar resolves to the box `[-25,25]²×[..]`; `Volume::new` on a round setup has no occupied voxel outside the cylinder and `remaining` ≈ π·25²·100 within the voxel error; the meshed surface's vertices lie on the cylinder or end caps (within one display cell); a revolve ring removes exactly the voxels in the ring and `swept` agrees on sample points inside/outside each bound; a ring cut in a lathe setup is seen by a later mill setup whose frame is rotated 90° (`Frame::transfer`), e.g. its `leftover_box` shrinks to the turned diameter where the turned length covers the whole bar; `RestStock` ignores revolve cuts; box stocks unchanged (existing tests).
- [ ] Commit "Add round bar stock and revolve cuts (LIO-175)".

### Task 4: The Turn strategy and lathe setups

**Files:** `src/model.rs` (`Strategy::Turn` at index 12, tables, `Operation::new` defaults per `turn_kind`, `Operation::issues` per kind and per machine kind: on a lathe only Turn and Drill, Drill only on the centre (`hole_x`/`hole_y` 0 — read how Drill targets holes and add a lathe "on the centre" mode with `depth`); Turn refused on mills; `OPERATION_FIELDS`), `src/ui.rs` (ribbon group "LATHE"; Turn editor with a `turn_kind` chooser and per-kind fields; dimming on lathes), `src/main.rs` (`AddOp` picks a fitting lathe tool by kind; `Message::AddLatheSetup`), `src/model.rs` (`Document::add_lathe_setup`: inserts a lathe setup with a round bar and the 3-jaw), `assets/op-12.svg`, `src/help.rs`.

- [ ] Tests: tables have 13 entries; defaults per kind; each refusal; `add_lathe_setup` first and later; the ribbon on a lathe setup enables only Turn and Drill.
- [ ] Commit "Add the Turn strategy and lathe setups (LIO-175)".

### Task 5: Turn plans

**Files:** new `src/turn.rs` (`generate(setup, op, tool, mesh) -> Result<Plan, String>` per kind; `rings(&Plan)`), `src/simulation.rs` (dispatch Turn and lathe Drill; `sequence_walk` applies a lathe plan's rings with `Volume::revolve` instead of capsules), `src/plan_check.rs` (gouge for lathe plans via the part check).

- [ ] Plans per spec §5 with moves in the setup frame (X radial = the tool's X, Z axial) for time and the guide, and the ring list for the simulation. Feeds: the machine's power feed nearest to 0.1 mm/rev for Outside/Bore, by hand (0.05) for Face/Groove/Part off; mm/min = mm/rev × rpm.
- [ ] Part check: sample the mesh's distance from the setup Z axis on a (z, angle) grid → the part's largest radius per z (outside) and smallest bore radius per z (inside); refuse a ring within `allowance` of the part with "The cut reaches the part at Z …, Ø …."
- [ ] Tests: Outside Ø50.2 → Ø40 (allowance 0), depth 2, finish 0.2: roughing in equal steps no larger than `depth` down to `diameter + allowance + finish`, then the finish pass — exactly Ø48.2, 46.2, 44.2, 42.2, 40.2, 40.0 (six passes, as in the mockup). Bore after a Ø10 drill to Ø20; Face 1 mm; Groove 3 wide to Ø35; Part off at −62; Thread M40×1.5 outside: 8 passes whose cumulative infeed ends at 0.61 × 1.5 within 0.01, each smaller than the one before; Drill on the centre to 30 deep; the part check refuses a turn below the part's diameter and passes one 0.5 mm above it; durations; a sequence lathe → mill where the mill setup's leftover box is the turned diameter.
- [ ] Commit "Generate turning passes (LIO-175)".

### Task 6: Chuck checks, no post for lathes, minutes

**Files:** `src/model.rs` (`holding_problems` for chucks), `src/holding.rs` (chuck checks and `boxes` for the chuck body; `suggest` picks the 3-jaw for round sections), `src/post.rs`/`src/run.rs`/`src/plan_check.rs`/`src/ui.rs` (every manual-mill skip already uses `is_hand` after Task 1 — verify lathe setups make no program and show "manual lathe, no G-code — follow the steps on the MACHINE page"), `src/manual.rs` (`tool_changes`/`move_minutes` for hand machines), `src/plan_check.rs` (minutes split names "Lathe … · Manual mill … · CNC …").

- [ ] Tests per spec §3 checks (range, grip ⅓ D, stickout, L/D > 4 warning, cut inside the grip, swing); 3-jaw chosen for round, 4-jaw for square with the chip; CNC programs byte-equal with a lathe setup first; minutes split.
- [ ] Commit "Check chucks and keep lathes out of the post (LIO-175)".

### Task 7: The lathe guide model

**Files:** `src/guide.rs` (lathe branch: `Step::{DialCheck, LatheHold, LatheZero, LatheFace, TurnPass, Measure, TailDrill, Plunge, ThreadPass}` or a `LatheStep` sub-enum — choose and explain), `src/model.rs` (`Document.measured: BTreeMap<String, f32>`, `#[serde(default)]`).

- [ ] Dial targets use `lathe::dial` for every number: cross slide from the touch on the measured diameter, carriage from the end, top slide for Face/Thread infeed, quill for drills. A `Measure` step before each Outside/Bore finish pass; when `measured` has a value for it, the finish pass's cross reading is recomputed from the measured diameter (remaining = measured − target, all on the finish pass).
- [ ] `DialCheck` first in the Set up section only while the machine's `cross_reads` is empty; until it is answered, cross-slide steps show "Check the dial first" instead of a number (never a guessed reading).
- [ ] Tests: the sample part's step list (spec §Testing); every dial reading equals `lathe::dial` of the plan's numbers; the measured diameter recomputes the finish pass; the dial check appears only when unknown; save/load of `measured` and the place; carriage stop "power feed off 1 mm before" target.
- [ ] Commit "Build the lathe step guide (LIO-175)".

### Task 8: The lathe Steps page

**Files:** `src/steps.rs` (lathe branch: side-view drawing; a `Dial` canvas widget drawing the collar with the index, the arc, the reading and "N turns +"; the Measure input (`text_input` → `Message::Guide(GuideAction::Measured(key, f32))`); the dial-check answer buttons (`GuideAction::CrossReads(Diameter|Radius)` saves to the library machine and every setup snapshot of it, via `touch()` since it changes the machine)), `src/main.rs`, `src/help.rs`, `src/smoke.rs` (next free smoke step, "lathe steps").

- [ ] Match the approved mockup. Render every lathe screen with the `#[ignore]`d headless render test into `/tmp/lathe-steps/` and LOOK at each PNG; fix overlaps and clipping at 1280 × 800.
- [ ] Tests (probe pattern): each screen kind's texts; Measure input recomputes the finish pass readout; the dial-check answer saves and the cross readouts appear; works with no controller and while locked.
- [ ] Commit "Show lathe setups as visual dial steps (LIO-175)".

### Task 9: Docs, review, PR

- [ ] README "Manual lathe" section; VALIDATION entries (what is and is not checked: the thread form is not simulated, the app does not read the dials).
- [ ] astra acceptance review with the Review Focus and the renders; fix until ACCEPT.
- [ ] Merge `origin/main`, full suite, clippy; PR to `main` titled "Manual lathe: turning and a visual dial step guide (LIO-175)"; merge it; move LIO-175 to Done.
