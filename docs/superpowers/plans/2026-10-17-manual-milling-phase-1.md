# Manual milling, phase 1 — Implementation Plan

> **For agentic workers:** implement task by task (subagent-driven: one implementer and one reviewer per task, then a whole-branch review by astra). Steps use checkbox (`- [ ]`) syntax.

**Goal:** The user's big manual mill is a machine in the project with its own tools and vise. A setup on it plans only one-axis-at-a-time cuts (the new **Box** strategy, and Drill). They are simulated, so the CNC setups after them start from the stock they leave. The plan check counts manual, CNC and moving minutes. MACHINE shows a manual setup as a **visual step guide** with DRO numbers instead of Run. The user marks the manual work. No suggestions and no AI yet (phases 2 and 3).

**Architecture:**
- A new `src/manual.rs` holds the manual machine's defaults, its gear and hand-feed rules, the tool and holder fit, and the minutes rules.
- `Strategy::Box` is generated in `src/cam.rs` through the existing `Builder`. On a manual machine the Builder splits every multi-axis move into one-axis legs and uses the machine's hand rapid.
- A new `src/guide.rs` turns a manual setup's simulated plans into steps (Hold, Zero, Tool, Comp, Pass, Holes, Check, Hand over) with DRO targets. It is pure data and tested without the UI.
- The Steps page in `src/ui.rs` draws them: an iced canvas for the top and front views, and `viewport::stock_picture` at a larger size for the 3D views. It plugs into `control()` before the controller check, so it works with no controller connected.

**Tech Stack:** Rust 2024, Iced 0.14 (canvas), serde. No new crates.

**Spec:** `docs/superpowers/specs/2026-10-17-manual-milling-design.md` (phase 1 parts: §1, §2, §3 without "Pages to take to the machine", and §4 "Marked by the user").

## Global Constraints

- Worktree `/home/liorknafo/.t3/worktrees/axis-cam-desktop/t3-68de786c`, branch `t3/manual-milling-planning`.
  - `source ~/.bashrc` before cargo. Before the first cargo command, run `~/tools/seed-target.sh`.
  - At most two worktrees build at once; the throttler pauses extra cargo (state T). Never kill a paused cargo.
  - Start every full `cargo test --locked -- --test-threads=6` detached, in a Bash call of its own (`setsid nohup bash -c '… > /tmp/manual-test.log 2>&1; echo "EXIT $?" >> /tmp/manual-test.log' &`). Poll it from the foreground in loops of at most 25 × 20 s, repeated, and never end a turn while cargo runs.
  - Run `cargo fmt` as its own short call.
  - Targeted `cargo test --locked <name>` runs may be foreground with a 600000 ms timeout.
- Existing documents load unchanged:
  - a machine without `kind`, or with `kind` other than `Manual`, is a CNC machine;
  - a tool or holder with an empty `machines` fits every CNC machine and no manual machine;
  - a new `Document` field is `#[serde(default)]`.
- The CNC's G-code, the post, the UC100 Run and their checks do not change for CNC setups. Manual setups are skipped by every post, export and Run path, and never produce `.ngc`.
- On a manual machine, every move of every plan changes exactly one of X, Y and Z (a Dwell changes none). Tests assert this on every manual plan they build.
- Box passes are conventional milling with M3: the uncut material is on the right of the feed direction.
- Follow the codebase's style: `Fields` maps with `get`/`num`, compact functions, few comments, no `#[allow]`.
- Every new field key needs a `help::field_help` arm. Every new `Message` needs a `help::command` arm, and an entry in `while_locked` when it must work while the CNC runs (all Steps page messages must).
- UI text is short labels. Never claim checks that are not made: the guide does not read the DRO, and the vise is checked as boxes, as today.
- `cargo test --locked` passes and `cargo clippy --all-targets` adds no warnings after every task.
  - Test counts are relative: call the passing total before Task 1 N.
  - Commits end with a blank line and `Co-Authored-By:` naming the model that wrote them.

## Review Focus

1. Every manual plan (Box and Drill) moves one axis at a time, including the approach and the retract. (Tasks 1, 4)
2. Box passes are conventional, never cut into the allowance, and never touch the part. (Task 4)
3. A manual setup never reaches the post, the export, the Run page or the UC100 checks, and CNC setups behave exactly as before. (Task 5)
4. DRO edge readouts equal the centre ± the tool radius on the operation's fixed compensation side, and every target matches the plan's moves. (Task 7)
5. Inserting a manual setup first keeps the stock, orientation, zero and holding the user had, and the later CNC setups' leftover stock follows the manual cuts. (Tasks 2, 5)

---

### Task 1: The manual machine, tool and holder fit

**Files:**
- Create `src/manual.rs`; add `mod manual;` to `src/main.rs` (alphabetical).
- Modify `src/model.rs`: `Machine::issues` (~265), `Tool::issues` (331); `src/holding.rs`: `Holder::issues`.

**Produces:**
- `manual::{is_manual(&Machine) -> bool, defaults() -> Machine, gears(&Machine) -> Vec<f32>, gear(&Machine, rpm: f32) -> f32, turns(&Machine, axis: usize, feed: f32) -> f32, settle(&mut Machine), fits(machines: &str, &Machine) -> bool}`.
- Keys on `Machine.p`: `kind`, `speeds`, `hand_feed`, `hand_rapid`, `turn_x`, `turn_y`, `turn_z`, `dro_comp`, `pass_time`, `tool_time`, `move_time`, with the spec's §1 defaults.
- Key `machines` on `Tool.p` and `Holder.p`.

- [ ] Write tests in `manual.rs`:
  - a `Machine::new(3)` and a machine whose `kind` is "CNC" or missing are not manual;
  - `defaults()` is manual with the spec's defaults, and its `issues()` is empty;
  - `gears`:
    - parses "90, 150 ,250";
    - a range `"100-2000"` gives a variable drive: `gear` clamps to the range and rounds to 10;
    - for fixed gears, `gear` picks the nearest: 600 → 640 with the default speeds;
  - `settle` sets `rpm` to the highest gear and `feed` to `hand_feed`;
  - `turns(m, 0, 150.)` is 30 per minute with `turn_x` 5;
  - `fits`:
    - `""` fits a CNC machine and not a manual one;
    - `"a,b"` fits the machines with ids a and b only, whatever their kind;
  - `Machine::issues` on a manual machine:
    - requires `speeds` (at least one positive gear), `hand_feed`, `hand_rapid`, and the three `turn_*` positive;
    - requires `pass_time`, `tool_time`, `move_time` ≥ 0;
    - does not require the post or probe fields.
- [ ] Implement:
  - `Machine::issues` branches on `manual::is_manual` for the manual rules;
  - the existing travel and `power` rules stay for both kinds.
- [ ] Add `field_help` arms for every new key.
- [ ] Run the targeted tests, then the full suite and clippy. Commit "Add the manual machine and tool fit".

### Task 2: Editing manual machines, tools, holders and setups

**Files:**
- `src/ui.rs`:
  - the machine wizard, 3945-4165;
  - `tools(app)`, 5624;
  - `workholding(app)`, 5840;
  - `holder_picker`, 3443;
  - the setup list and its buttons;
  - the strategy ribbon, 1005-1075.
- `src/main.rs`:
  - `Message::Edit` for `Target::Machine`, 2696;
  - `NewMachine`, 3066;
  - `SaveMachine`, 3361;
  - `AssignMachine`, 3055;
  - `AddSetup`, 2995;
  - a new `Message::AddManualSetup`.
- `src/model.rs`: `Document::setup_sheet`, 1662.
- `src/help.rs`.

**Produces:**
- Wizard:
  - step 0 gets a `kind` choice, CNC or Manual;
  - choosing Manual fills the manual defaults (`manual::defaults()`), keeping the name, travel and power entered so far;
  - for a manual machine, step 1 shows travel, power, `speeds`, `hand_feed`, `hand_rapid` and `turn_*`;
  - for a manual machine, steps 2 and 3 are replaced by one step with `dro_comp`, `pass_time`, `tool_time` and `move_time`, under the note "Manual mill — no G-code; MACHINE shows the steps.";
  - `SaveMachine` calls `manual::settle` before `issues()`.
- Tool and holder editors get a row of check boxes, one per library machine, editing `machines`. Messages: `Message::ToolMachine(id, bool)` and `Message::HolderMachine(id, bool)`.
- `holder_picker` lists only holders that `fits` the setup's machine.
- `Message::AddManualSetup` (the button shows only when the library has a manual machine):
  - it inserts a setup on the first manual machine, before the first CNC setup, named "Big mill", with no operations;
  - at index 0, it copies the stock, orientation (`rx/ry/rz`, `tx/ty/tz`), origin, offset and holding keys of the old first setup;
  - it shifts `doc.active` and `touch()`es the document.
- Ribbon:
  - a "MANUAL" group holds Box (index 11);
  - on a manual setup, only Box and Drill are enabled; the others are dimmed with the hover text "The big mill cuts one axis at a time.";
  - on a CNC setup, Box is dimmed with "Box runs on a manual mill.".
- Setup sheet, for a manual setup: `Machine: {name} (manual mill)` and "Manual mill: steps on the MACHINE page".

- [ ] Write tests with the `ui::probe` texts and clicks pattern:
  - the wizard shows the manual fields and hides post and probe fields for a manual draft;
  - saving settles `rpm` and `feed`;
  - the tool editor's check box toggles the tool's `machines`;
  - a holder that does not fit is missing from the picker.
- [ ] Write tests for `AddManualSetup` on `Document::sample()` with a manual machine added:
  - the new setup is first and its stock box equals the old first setup's;
  - the old setup is second, and `doc.active` follows it;
  - a second press inserts before the first CNC setup, not at 0.
- [ ] Write a test for the setup sheet lines.
- [ ] Add `help::command` arms. Run the full suite and clippy. Commit "Edit manual machines and add manual setups".

### Task 3: The Box strategy in the model

**Files:**
- `src/model.rs`:
  - `Strategy`, 23;
  - `ALL` and the `name`/`short`/`help` arrays, 103-160;
  - `availability`, 52;
  - `imported_preview`, 37;
  - `tabs`, 149;
  - `Operation::new`, 501-600;
  - `Operation::issues`, 616-788;
  - `OPERATION_FIELDS`, 1805.
- `src/help.rs` (2690 and the strategy help).
- `src/ui.rs`:
  - `issue_tab`, 472;
  - `operation_editor_with` and `pass_fields`, 2083 and 2574.
- `src/main.rs`: the icon list, 1086-1097, and `AddOp`'s tool pick, 2743. Add an `assets/op-11.svg` icon (a block with a stepped corner, in the style of the other icons).
- Wildcard matches to review: `cam.rs:318`, `proposal.rs:518/532/645` (Box is not offered to the planner in phase 1: keep it out of `proposal::STRATEGIES`), `ui.rs:2240/2365/6133/6163`, `viewport.rs:1778`.

**Produces:**
- `Strategy::Box`, named "Box" (short "Box").
  - Help text: "Remove a rectangular block of material, one axis at a time, on a manual mill."
  - `imported_preview` is true.
- Operation keys:
  - box: `x_min`, `x_max`, `y_min`, `y_max`, `z_top`, `z_bottom`;
  - `allowance` (Box default 0.5);
  - `stepover` (Box default 70, percent of the diameter);
  - `stepdown` (Box default "", meaning automatic);
  - Box defaults are rpm 640 and feed 120.
- `Operation::issues(machine, tools, earlier)`:
  - Box and Drill are the only strategies allowed on a manual machine: "The big mill cuts one axis at a time: use Box or Drill.";
  - Box is refused on a CNC machine: "Box runs on a manual mill.";
  - the tool must `fits` the machine: "{tool} is not on {machine}.";
  - on a manual machine the rpm must be within 1 % of a gear: "Spindle speed must be one of the machine's gears: {list}.";
  - the existing feed cap uses the settled `feed` (`hand_feed`);
  - box rules:
    - `x_min < x_max`, `y_min < y_max`, `z_bottom < z_top`;
    - the box is at least the tool's diameter wide across its passes;
    - `allowance` ≥ 0;
    - `stepover` is 10–100;
    - `stepdown` is empty or positive.
- Editor:
  - a "Box" tab with the six box fields, in a 2 × 3 grid with X, Y and Z rows;
  - a passes tab with `allowance`, `stepover` and `stepdown`, the last with the placeholder "Auto".

- [ ] Write tests:
  - every `Strategy::ALL` array has 12 entries;
  - `cam.rs:1900`'s count becomes 7;
  - `Operation::new(Box)` has the defaults above, and `issues` is empty on a manual setup with a fitting end mill;
  - each refusal above, one test per message;
  - `issue_tab` maps every new key.
- [ ] Update the hard-counted tests (`cam.rs:1900-1918`, `ui.rs:7675/8014`, `help.rs:3785/3850`) only for the new variant.
- [ ] Run the full suite and clippy. Commit "Add the Box strategy".

### Task 4: Box and Drill paths on a manual machine

**Files:**
- `src/cam.rs`:
  - `Builder`, 171-315;
  - `generate`, 317;
  - the Drill peck rapids, 807/813;
  - a new `boxed(…)` generator.
- `src/forces.rs`: a new `power_stepdown`.
- Tests in `cam.rs`, using `cam::tests::fixture` with a manual machine.

**Produces:**
- `Builder` gains `rapid: f32` (the machine's `hand_rapid` on a manual machine, 6000 otherwise) and `single_axis: bool`. Every `6000.` in the Builder and Drill uses `self.rapid`.
- With `single_axis`, `push` splits a move that changes more than one axis into legs with the same motion and rate. The order is:
  1. Z first if it rises;
  2. then X;
  3. then Y;
  4. then Z last if it falls.
  CNC plans are unchanged: `single_axis` is false and `rapid` is 6000.
- `forces::power_stepdown(machine, material, diameter, width, feed) -> f32`:
  - `ap = 0.7 × power[W] × 60000 / (kc × width × feed)`, with kc from the existing `KC` table and `kc(material)`;
  - `power` defaults to 800 W, as in `pushes`.
- `cam::boxed(setup, op, tool, mesh) -> Result<Plan, String>`, reached from `generate` for `Strategy::Box`. Box coordinates are in the setup's work frame (the same frame Drill's hole positions use; reuse its conversion to stock coordinates):
  1. **Open and closed sides.** The box is clipped to the leftover stock (`plan.stock`). A side lying on or beyond the stock's face is open; every other side and the floor (unless it is at or below the stock's bottom) is closed. Closed sides and the floor move in by `allowance`.
  2. **Pass axis** = the box's longer side in X/Y; the cross axis steps.
  3. **Levels** from `z_top` down to the floor by `stepdown` (or `power_stepdown` at the full stepover width, capped at the flute length and the diameter), with equal steps, the last exactly on the floor.
  4. **Rows across.** The first row is on the open side when the cross axis has one (else on the − side); rows step toward the closed side by `stepover`, the last row exactly at the closed side − radius. Rows are not climb: the feed direction is chosen so the uncut material (the side the rows step toward) is on the right of the feed with M3.
  5. **Each row:**
     1. Rapid Z to retract;
     2. rapid along the pass axis to the start: off the stock by radius + 2 mm when that end is open, else at the closed end − radius;
     3. rapid across to the row;
     4. down to the level (Rapid when the start is off the stock, else Feed at a quarter of the feed);
     5. a Dwell of `pass_time` seconds;
     6. one Feed along the pass axis to the far end (off the stock, or the closed end − radius);
     7. Rapid Z to retract.
     Rapids go only to the retract height, never to clearance between rows.
  6. **Part check.** The generator refuses with "The box cuts into the part at X…, Y…, Z…." when the swept box (the cut region) overlaps a part triangle in the stock frame. Use a triangle–box overlap test (separating axis) on the mesh triangles inside the box's bounds.
  7. `plan.milling_direction = "Conventional"`, `spindle_clockwise = true`.
  8. `Plan.duration` comes from the hand feed, the hand rapid and the dwells.

- [ ] Write tests in `cam.rs` (all with a manual machine, a 120 × 80 × 40 block, a Ø25 end mill):
  - **single axis:** every move of a Box plan and of a Drill plan with three holes changes at most one axis; the same Drill on a CNC machine still makes its usual multi-axis rapids (no change);
  - **a face:** a box covering the whole top 2 mm deep, all sides open, gives one level and rows covering the full width, with every feed move inside the stock's Y range ± radius;
  - **a step on −Y:** −Y open, +Y closed at 15.5, allowance 0.5. The last row's tool edge is at 15.0. No feed move puts the tool edge past 15.0. Rows step toward +Y with the feed along −X (conventional, uncut material on the right).
  - **a closed pocket:** plunges as Feed at a quarter of the feed, and the floor is `z_bottom` + allowance;
  - **levels:** a 9.5 mm deep box with `stepdown` 2.5 gives levels at −2.5, −5, −7.5, −9.5 (minus allowance on a closed floor); an empty `stepdown` gives a positive automatic depth ≤ the flute length and ≤ the diameter;
  - **part check:** a box overlapping the part is refused with the message; a box 0.5 mm off a part face passes;
  - **duration:** a known single-row box: length / feed × 60 + rapids / `hand_rapid` × 60 + `pass_time`, within 0.1 s;
  - **simulation:** `simulation::sequence` with the manual setup first (a 2 mm face) and a CNC setup second gives the second setup a `preview_stock` 2 mm shorter.
- [ ] Run the full suite and clippy. Commit "Generate Box passes and one-axis drilling on a manual mill".

### Task 5: Manual setups in the checks, export, Run and minutes

**Files:**
- `src/post.rs`: `post_job`, `post_simulated`, 2159; `held`.
- `src/plan_check.rs`: `run`, 830-1053; `CheckReport`, 25.
- `src/run.rs`: `plan_problems`; the setup list at `ui.rs:5101`.
- `src/main.rs`: `Message::Export`, 3777.
- `src/manual.rs`: minutes.

**Produces:**
- Every post, export and UC100 path skips manual setups: no program, no `PostSettings::from_machine` error for them. `held` and the holding checks still run on them.
  - Export writes no `.ngc` for a manual setup and adds a line to its summary: "{setup}: manual mill, no G-code — follow the steps on the MACHINE page."
- `manual::tool_changes(setup) -> usize`: changes of tool between enabled operations of a manual setup, counting the first tool.
- `manual::move_minutes(doc) -> f32`: for each pair of consecutive setups on different machines, the `move_time` of the manual machine in the pair, or 0 if neither is manual.
- `CheckReport` gains `manual_minutes`, `cnc_minutes` and `move_minutes` (serde-skipped, like the other UI-only fields).
  - `total_minutes` = operations' minutes + manual tool changes × `tool_time` + `move_minutes`.
  - Its summary line reads "Manual 18 min · CNC 42 min · moving 10 min".

- [ ] Write tests:
  - the `post::fixture::job()` document with a manual setup inserted first: `post_job` makes the same programs for the CNC setups as without it (byte-equal), and none for the manual setup;
  - `plan_check::run` on it: no export problem for the manual setup; a gouge, holding or generation problem in the manual setup is still reported; the minutes split adds up;
  - `tool_changes` and `move_minutes` on small documents (manual, CNC, manual, CNC: three moves);
  - the Run page lists no manual setup.
- [ ] Run the full suite and clippy. Commit "Keep manual setups out of the post and count their minutes".

### Task 6: Fill a box from a part face

**Files:**
- `src/ui.rs`: the Box tab gets a button "From selected face".
- `src/main.rs`: `Message::BoxFromFace`.
- `src/manual.rs`: `box_from_face(setup, mesh, selection, stock) -> Result<[f32; 6], String>`.

**Produces:** from the operation's selected surfaces (`Operation.surfaces`), which must be one planar face whose normal is within 1° of an axis of the setup's frame:
- **A floor (+Z):**
  - X and Y: the face's extent, grown in each direction up to the leftover stock's edge or the first part material above the floor, sampled on a 1 mm grid of mesh heights;
  - `z_top` is the stock top and `z_bottom` the floor.
- **A wall (±X or ±Y):**
  - from the wall out to the stock's edge on its normal side;
  - along the wall, its extent;
  - from the stock top down to the wall's lowest point.
- Anything else answers "Select one flat face that faces up or sideways.".
- The result is written to the six keys in work coordinates.

- [ ] Write tests on the `four-setup-block` example:
  - a floor face gives a box whose floor is the face's height and that does not overlap the part (Task 4's part check passes);
  - a side wall gives a step box open on the wall's normal side;
  - a slanted or curved face is refused with the message.
- [ ] Run the full suite and clippy. Commit "Fill a box from a part face".

### Task 7: The step guide model

**Files:**
- Create `src/guide.rs`; add `mod guide;`.
- `src/model.rs`: `Document.guide: BTreeMap<String, usize>`, `#[serde(default)]` (setup id → step index).

**Produces:**
- `guide::Guide { setup: usize, steps: Vec<Step>, minutes: f32 }`.
- `guide::build(doc, mesh, si) -> Result<Guide, String>` runs `simulation::sequence_with` up to the end of setup `si`. It keeps the stock box before the setup, each operation's `Plan` and the volume before and after each operation (for the pictures).
- `Step` is an enum:
  - `Hold { stock, holding }`
  - `Zero { corner, approach: [i8; 2] }`
  - `Tool { op, tool, rpm, diameter, comp: [i8; 2] }`
  - `Comp { op, comp }`
  - `Pass { op, index, of, targets: Vec<Target>, cut: Bounds, earlier: Vec<Bounds>, never: Limits, turns_per_minute: f32 }`
  - `Holes { op, holes: Vec<[f32; 2]>, depth }`
  - `Check { op }`
  - `HandOver { next: Option<usize> }`

  Here `Target { order: u8, axis: usize, centre: f32, shown: f32, edge: bool, lock: [bool; 3] }`.
- **Order:**
  1. Hold, then Zero;
  2. then per operation: Tool when the tool or the operation changes (Comp instead when only the compensation sides change), then one Pass per Box row and level (or Holes for Drill), then Check;
  3. HandOver last.
- **Targets** come straight from the plan's one-axis moves before each cutting Feed: the rapids and the plunge, then the cut. `centre` is the move's end in work coordinates, and `shown` is the readout:
  - with `dro_comp` "Yes", X and Y are shown as the edge, `centre + comp × radius`;
  - otherwise the centre.
  - `comp` is per operation and axis: toward the box's closed side on that axis, + when both or neither are closed.
- **Locks:** during a cut along X, Y and Z are locked, and the same for the other axes.
- **`never`:** the lowest Z and the extreme X/Y the operation's tool edge reaches.
- **`turns_per_minute`:** `manual::turns` on the pass axis.
- **`minutes`:** the setup's operation minutes plus `tool_changes × tool_time`.
- `Document.guide[setup.id]` is the saved place. `guide::place(doc, si) -> usize` clamps it to the steps.

- [ ] Write tests (the Task 4 block, a manual setup with Face 2 mm, Step on −Y, and Drill × 4, followed by a CNC setup):
  - **step list:**
    - the kinds in order (Hold, Zero, Tool, Pass…, Check, Tool, Pass…, Check, Tool, Holes, Check, HandOver);
    - `HandOver.next` is the CNC setup;
  - **targets:**
    - each Pass's targets replay the plan's moves exactly (applying them in order from the previous position reaches the cut's end);
    - order numbers run 1, 2, 3…;
  - **edge readouts:**
    - with `dro_comp` on, X/Y readouts are `centre ± 12.5` on the comp side, and the Step's last-row Y readout shows 15.0 (the wall at 15.5 less the 0.5 mm allowance);
    - with it off, the readouts are the centres;
  - **Comp step:** two Box operations with the same tool and opposite closed sides give a Comp step between them;
  - **turns:** 120 mm/min on X with `turn_x` 5 is 24 per minute;
  - **place:** survives `storage::save`/`load`, and clamps when the setup has fewer steps after an edit.
- [ ] Run the full suite and clippy. Commit "Build the manual step guide".

### Task 8: The Steps page

**Files:**
- `src/ui.rs`:
  - `control()`, 4320, branching before `let Some(c) = &app.control`, ~4350;
  - the MANUFACTURE setup card;
  - a new `steps(app)` view and a `StepDrawing` canvas program.
- `src/main.rs`:
  - `App.guide: Option<(u64, Result<Guide, String>)>`, keyed by a fingerprint of the setups, operations, tools and mesh, built by a background task like the leftover-stock cache;
  - messages `Message::Guide(GuideAction)` with `Next`, `Back`, `Go(usize)` and `Built(..)`;
  - `while_locked`.
- `src/viewport.rs`: `stock_picture` takes a size (default 128 × 96 for today's callers).
- `src/help.rs`.
- `src/smoke.rs`: step 124, `"100-manual-steps"`.

**Produces:**
- **When it shows.** When the active setup is manual, MACHINE shows `steps(app)` whether or not a controller is connected.
  - Its left column lists the manual setups; selecting one sets `doc.active`.
  - A manual setup's card in MANUFACTURE has a "Steps" button that opens MACHINE on it.
- **Layout** (the mockup in the conversation of 17 October):
  - A step bar on top with one pill per step, done ✓, the current one highlighted, and "→ CNC setup N" last.
  - **Left:**
    - the top view and the front view, drawn by `StepDrawing` with `canvas::Cache`:
      - the leftover stock rectangle;
      - the part's triangles projected and filled in grey;
      - the vise jaws;
      - earlier cuts hatched light;
      - this pass's band hatched in the accent colour;
      - the tool circle at the start, with an arrow to the end;
      - the DRO zero marker and the axis arrows;
      - the `never` lines in red;
    - or, for Hold, Check and HandOver, the 3D `stock_picture` of the volume at 480 × 320;
    - a small legend.
  - **Right:**
    - chips for the tool, rpm (gear) and feed, with "1 turn every N s" and a dot that blinks once per turn, via `iced::time::every` while the page is visible;
    - the targets as large readouts in order, the current one bright, with a lock icon on locked axes, an "edge" tag and arrow, or "centre", and the centre number small under an edge readout;
    - pass boxes (done, now, next);
    - Back and "Pass done ▶" (or "Next ▶" on non-pass steps).
  - **The guide's state:**
    - while it builds: "Simulating the setup…";
    - when it fails: the error and "Fix the setup in MANUFACTURE.";
    - the header shows the setup's minutes.
- **Saving the place.** Next and Back save `Document.guide` with `dirty = true; last_edit = now` (as `SelectSetup` does, `main.rs:2582`), not `touch()`.

- [ ] Write tests with the `ui::probe` pattern on the Task 7 document:
  - the page shows "Pass 1 of N" and the readouts' numbers for the first pass;
  - Next and Back move the place and keep it after a save and load;
  - the page shows with no controller and while `app.locked()`;
  - a CNC setup still shows today's MACHINE page.
- [ ] Add smoke step 124, `"100-manual-steps"`, which opens the Steps page on a manual setup and checks its texts.
- [ ] Look at the page in a gallery or Xvfb render (see the Xvfb memory: `~/tools/xvfb`). Fix anything unreadable at 1280 × 800. Attach the PNG to the final report.
- [ ] Run the full suite and clippy. Commit "Show manual setups as visual steps".

### Task 9: Docs and the final review

**Files:** `README.md`, `docs/VALIDATION.md`, and `src/help.rs` (a help card for the Steps page).

- [ ] README:
  - a "Manual mill" section: what Box does, the one-axis rule, the allowance, the DRO numbers (edge with compensation), and that the app does not read the DRO;
  - in VALIDATION, the manual checks and what is not checked.
- [ ] A whole-branch review by **astra** (the acceptance reviewer), with the Review Focus list, the Task 8 PNG and a machinist's eye for:
  - the conventional direction;
  - the edge numbers;
  - the safety of the plunge on closed boxes.
  Fix until astra answers ACCEPT.
- [ ] Run the full suite and clippy. Commit "Document the manual mill". Open a PR to `main` and link it to the thread.
