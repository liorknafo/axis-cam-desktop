# AI Planner M2 — Part Analysis and Plan Checker Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Without any AI, the app can describe an imported part for a planner, turn a plan file into a job, and check that job with the real generators, the stock simulation, the export checks, a coarse gouge check and a coverage report.

**Architecture:** `analysis.rs` builds a deterministic part report for six axis-aligned orientations. `proposal.rs` defines the plan format (strict JSON schema), parses it and builds a job from it. `plan_check.rs` runs the stages (structure, generation, export, gouge) and measures coverage on the stock grid. Three command-line tools expose them: `--part-report`, `--proposal-schema`, `--check-plan`.

**Tech Stack:** Rust 2024, existing crates only.

**Spec:** `docs/superpowers/specs/2026-10-03-ai-planner-design.md` (section "M2")

## Global Constraints

- Source folder: `/Users/liorknafo/Documents/Codex/2026-09-21/i-x20/outputs/axis-cam-desktop`, branch `ai-planner`. Before every `cargo` command: `export CARGO_TARGET_DIR='/Users/liorknafo/Documents/Codex/2026-09-21/i-x20/work/native-target'`. Use `cargo test --locked`. No new crates.
- Never read, write or delete anything under `~/Library/Application Support/AXIS CAM Studio` or `~/.codex`. Never run the ignored Keychain test. Do not launch the app or contact OpenAI.
- Orientations and their setup rotations (rx, ry, rz): `+Z` (0,0,0), `-Z` (180,0,0), `+X` (0,−90,0), `-X` (0,90,0), `+Y` (90,0,0), `-Y` (−90,0,0). The orientation matching setup 1's up axis uses setup 1's own rotation. The first planned setup is always setup 1's orientation.
- Report limits: at most 40 planes and 100 holes per orientation, planes under 1 mm² dropped, lengths rounded to 0.01 mm.
- Proposal schema: every field required, no extra properties. Strategies `Face`, `Pocket`, `Contour`, `Drill`, `Parallel`, `Waterline`. Work zeros `Stock top center`, `Stock top front-left`, `Model origin`.
- Gouge: three or more stock-grid cells well inside the part (all six neighbours inside) removed by the simulation. Coverage numbers are approximate and labelled so.
- Model free text is only stored as names and notes (names at most 60 characters, notes at most 500).
- Commit messages end with a blank line and `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>`. Stage only the files each task names.

## Reference implementation

`docs/superpowers/plans/2026-10-03-ai-planner-m2/` holds `analysis.rs`, `proposal.rs` and `plan_check.rs`. They were prototyped against this codebase in a scratch copy, where their 18 tests pass. Tasks 1–3 move them into `src/` test-first. They are the exact code to use. Task 4 deletes the folder.

Test counts below are relative: run the suite before Task 1 and call the passing total N.

---

### Task 1: Part analysis

**Files:**
- Modify: `src/cam.rs` (visibility of geometry helpers), `src/main.rs` (module list)
- Create: `src/analysis.rs`

**Interfaces:**
- Produces: `analysis::{ORIENTATIONS, PartReport, MachineReport, ToolReport, OrientationReport, PlaneReport, HoleReport, up_axis(&Setup) -> Option<&'static str>, rotation_for(&Setup, &str) -> Option<[f32; 3]>, oriented(&Setup, &str) -> Option<Setup>, Precondition { label, ok, hint }, preconditions(&Document, Option<&Mesh>) -> Vec<Precondition>, report(&Document, &Mesh) -> Result<PartReport, String>}`; crate-visible `analysis::{Index, normal, centroid}`; crate-visible `cam::{Triangle, Geometry, Polygon, Region, region, plane_height}`.

- [ ] **Step 1: Open up the geometry helpers in `src/cam.rs`**

Make these crate-visible, changing nothing else:

```rust
pub(crate) type Polygon = Vec<(f64, f64)>;
pub(crate) type Region = Paths<Milli>;
pub(crate) fn region(mut polygons: Vec<Polygon>) -> Result<Region, String> {
#[derive(Clone)]
pub(crate) struct Triangle {
    pub p: [[f32; 3]; 3],
    pub feature: Option<usize>,
}
pub(crate) struct Geometry {
    pub triangles: Vec<Triangle>,
    pub bounds: Bounds,
}
    pub(crate) fn new(setup: &Setup, mesh: &Mesh) -> Self {      // in impl Geometry
pub(crate) fn plane_height(p: &[[f32; 3]; 3], xy: [f32; 2]) -> Option<f32> {
```

- [ ] **Step 2: Write the failing tests**

Create `src/analysis.rs` containing only the attachment's `#[cfg(test)] mod tests`. Add `mod analysis;` to `src/main.rs` (alphabetical position).

Run: `cargo test --locked --quiet analysis:: 2>&1 | tail -5`
Expected: compile errors (`report`, `up_axis`, `preconditions` not found).

- [ ] **Step 3: Add the implementation**

Replace `src/analysis.rs` with the full attachment, unchanged.

- [ ] **Step 4: Run the tests**

Run: `cargo test --locked --quiet 2>&1 | tail -3`
Expected: N + 5 passed; 0 failed. Dead-code warnings for the new module are expected until Task 3.

- [ ] **Step 5: Commit**

```bash
cargo fmt && git add src/cam.rs src/main.rs src/analysis.rs && git commit -q -m "Part analysis: what each orientation can reach

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 2: Proposal format and job builder

**Files:**
- Create: `src/proposal.rs`
- Modify: `src/main.rs` (module list)

**Interfaces:**
- Consumes: `analysis::{PartReport, OrientationReport, PlaneReport, ORIENTATIONS, rotation_for}`.
- Produces: `proposal::{Proposal, ProposedSetup, ProposedOp, Unmachined, Problem { setup, operation, stage, message }, ORIGINS, schema() -> Value, parse(&str) -> Result<Proposal, String>, build(&Document, &Mesh, &PartReport, &Proposal) -> Result<Document, Vec<Problem>>}`; test-only `proposal::tests::{block, good, op}`.

- [ ] **Step 1: Write the failing tests**

Create `src/proposal.rs` containing only the attachment's `#[cfg(test)] pub(crate) mod tests`. Add `mod proposal;` to `src/main.rs`.

Run: `cargo test --locked --quiet proposal:: 2>&1 | tail -5`
Expected: compile errors (`Proposal`, `build`, `schema` not found).

- [ ] **Step 2: Add the implementation**

Replace `src/proposal.rs` with the full attachment, unchanged.

- [ ] **Step 3: Run the tests**

Run: `cargo test --locked --quiet 2>&1 | tail -3`
Expected: N + 10 passed; 0 failed.

- [ ] **Step 4: Commit**

```bash
cargo fmt && git add src/main.rs src/proposal.rs && git commit -q -m "Plan proposals: strict schema, parsing and job building

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 3: Plan check and command-line tools

**Files:**
- Modify: `src/simulation.rs` (`sequence` → `sequence_with`), `src/main.rs` (module list, `planning_project`, `part_report`, `check_plan`, command dispatch)
- Create: `src/plan_check.rs`

**Interfaces:**
- Consumes: Tasks 1–2, `post::post_job`, `volume::Volume`.
- Produces: `simulation::sequence_with(doc, mesh, stop, visit, after: impl FnMut(usize, usize, &Volume))`; `plan_check::{CheckReport { passed, problems, operations, coverage, total_minutes }, OpResult, Coverage, CoveredPlane, CoveredSide, Leftover, PartGrid::{new, gouged}, coverage(..), check(&Document, &Mesh, &PartReport, &Proposal) -> (CheckReport, Option<Document>)}`; `main.rs` functions `planning_project`, `part_report`, `check_plan`.

- [ ] **Step 1: Write the failing tests**

Create `src/plan_check.rs` containing only the attachment's `#[cfg(test)] mod tests`. Add `mod plan_check;` to `src/main.rs`.

Run: `cargo test --locked --quiet plan_check:: 2>&1 | tail -5`
Expected: compile errors (`check`, `PartGrid`, `crate::check_plan` not found).

- [ ] **Step 2: Report the stock after each operation** (`src/simulation.rs`)

Replace the head of `sequence` so it delegates, and call `after` where the finished stock is computed:

```rust
pub fn sequence(
    doc: &crate::model::Document,
    mesh: &Mesh,
    stop: Option<(usize, usize)>,
    visit: impl FnMut(usize, usize, Result<Simulation, String>),
) {
    sequence_with(doc, mesh, stop, visit, |_, _, _| {})
}
/// `sequence`, also reporting the stock left after each completed operation.
pub fn sequence_with(
    doc: &crate::model::Document,
    mesh: &Mesh,
    stop: Option<(usize, usize)>,
    mut visit: impl FnMut(usize, usize, Result<Simulation, String>),
    mut after: impl FnMut(usize, usize, &crate::volume::Volume),
) {
    use crate::volume::{Frame, Volume};
```

(the rest of the old `sequence` body follows unchanged), and inside it:

```rust
                    let mut finished = sim.clone();
                    finished.seek(1.);
                    if let Some(v) = &finished.volume {
                        after(si, oi, v);
                    }
                    volume = finished.volume;
```

- [ ] **Step 3: Add the checker**

Replace `src/plan_check.rs` with the full attachment, unchanged.

- [ ] **Step 4: Command-line tools** (`src/main.rs`)

Add above `fn export_gcode`:

```rust
/// A saved project and its imported part, ready for planning.
fn planning_project(project: &str) -> Result<(Document, mesh::Mesh), String> {
    let mut doc = storage::load(std::path::Path::new(project))?;
    doc.sync_stock_sources();
    let file = doc.file.clone().ok_or("The project has no imported part.")?;
    let mesh = mesh::load(std::path::Path::new(&file.path))?;
    if let Some(unmet) = analysis::preconditions(&doc, Some(&mesh))
        .into_iter()
        .find(|p| !p.ok)
    {
        return Err(format!("{}: {}", unmet.label, unmet.hint));
    }
    Ok((doc, mesh))
}
/// The part report the planner sends to the model, as JSON.
fn part_report(project: Option<&String>) -> Result<String, String> {
    let project = project.ok_or("Usage: axis-cam --part-report <project.json>")?;
    let (doc, mesh) = planning_project(project)?;
    serde_json::to_string_pretty(&analysis::report(&doc, &mesh)?).map_err(|e| e.to_string())
}
/// Checks a proposal file against a saved project with every planner check.
/// Prints the report as JSON; `Err` when the plan does not pass.
fn check_plan(project: Option<&String>, proposal: Option<&String>) -> Result<String, String> {
    let (Some(project), Some(proposal)) = (project, proposal) else {
        return Err("Usage: axis-cam --check-plan <project.json> <proposal.json>".into());
    };
    let (doc, mesh) = planning_project(project)?;
    let report = analysis::report(&doc, &mesh)?;
    let text = std::fs::read_to_string(proposal).map_err(|e| format!("{proposal}: {e}"))?;
    let proposal = proposal::parse(&text)?;
    let (check, _) = plan_check::check(&doc, &mesh, &report, &proposal);
    println!(
        "{}",
        serde_json::to_string_pretty(&check).map_err(|e| e.to_string())?
    );
    if check.passed {
        Ok(format!(
            "The plan passes every check: {} operations, {} min.",
            check.operations.len(),
            check.total_minutes
        ))
    } else {
        Err(format!(
            "The plan does not pass: {} problem(s).",
            check.problems.len()
        ))
    }
}
```

In `fn main`, replace the existing `--export-gcode` block (the `if args.get(1)… == Some("--export-gcode") { … }` statement) with:

```rust
    let command = match args.get(1).map(String::as_str) {
        Some("--export-gcode") => Some(export_gcode(args.get(2), args.get(3))),
        Some("--check-plan") => Some(check_plan(args.get(2), args.get(3))),
        Some("--part-report") => Some(part_report(args.get(2))),
        Some("--proposal-schema") => {
            Some(serde_json::to_string_pretty(&proposal::schema()).map_err(|e| e.to_string()))
        }
        _ => None,
    };
    if let Some(result) = command {
        match result {
            Ok(message) => {
                println!("{message}");
                std::process::exit(0)
            }
            Err(message) => {
                eprintln!("{message}");
                std::process::exit(1)
            }
        }
    }
```

- [ ] **Step 5: Run the tests and the build**

```bash
cargo test --locked --quiet 2>&1 | tail -3
cargo build --locked 2>&1 | grep -E "^warning: (unused|function|constant|field|variant|method|struct)|never (used|read|constructed)" || echo "no new warnings"
```

Expected: N + 18 passed; 0 failed. The build shows no warnings for `analysis`, `proposal` or `plan_check` items (warnings that existed before this plan may remain; list them in the report).

- [ ] **Step 6: Commit**

```bash
cargo fmt && git add src/main.rs src/simulation.rs src/plan_check.rs && git commit -q -m "Plan check: generation, export, gouge and coverage; command-line tools

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 4: Documentation and cleanup

**Files:**
- Modify: `README.md`, `docs/VALIDATION.md`
- Delete: `docs/superpowers/plans/2026-10-03-ai-planner-m2/`

- [ ] **Step 1: README**

Add after the `## Sign in with ChatGPT` section:

```markdown
## Planner checks without AI

Three command-line tools show what the AI planner will see and how its plans are judged. They need a saved project with an imported part, a 3-axis LinuxCNC machine and stock on setup 1, a material, and numbered tools.

- `axis-cam --part-report <project.json>` prints the part report: for each of six orientations (the part axis that points up), the horizontal faces visible from above with their surface IDs, pocket floors with depth and the largest flat cutter that fits, and holes with diameter and depth.
- `axis-cam --proposal-schema` prints the JSON schema of a plan: setups with an orientation and a work zero, and operations with a strategy, a tool number, surface IDs and cutting parameters.
- `axis-cam --check-plan <project.json> <plan.json>` builds the job a plan describes and checks it: references (tools, surfaces, orientations), generation of every operation with cumulative stock, all LinuxCNC export checks, and a coarse gouge check. It then reports coverage: how much of the part's surface is exposed, which faces are still covered, and how much stock remains above, around, inside and below the part.

The gouge check and coverage use the stock simulation's grid (up to 96 cells on the longest side). The gouge check finds cuts deeper than about one cell into the part. Coverage does not see stock thinner than about 1.5 cells. Both are approximate; the generators' own rules and the export checks are the precise ones.

Setup 1 stays as you configured it (machine, stock, work zero, fixture). A plan replaces its operations and adds later setups.
```

In `## Code map`, add entries in the existing style for `src/analysis.rs` (part report and planning preconditions), `src/proposal.rs` (plan format, schema, job building), `src/plan_check.rs` (plan stages, gouge check, coverage).

- [ ] **Step 2: VALIDATION**

Update the test count to the current total and add:

```markdown
## Planner analysis and checks (no AI)

- Part report on the example block: the top plane with five holes (four Ø6.6 through with Ø11 openings, one Ø6 blind), counterbore rings, the 72 × 40 mm pocket at 12 mm depth with its fitting cutter, and the features of the other five orientations. A face is listed only where it is visible from above.
- Proposals: the schema is strict at every level and matches the Rust types; unknown fields and missing fields are rejected.
- Job building: setup 1 keeps its machine, stock, work zero and fixture; later setups get the next work offset and the orientation's rotation; required option combinations are fixed by the app.
- Every reference problem (unknown tool, wrong tool type, surfaces from another orientation or plane, a cutter that does not fit a floor, limits) is reported with its setup and operation.
- A hand-written two-setup plan passes all stages. A generation failure, an export refusal and a synthetic cut into the part are each reported at the right stage.
- Coverage rises as operations are added, and leftover stock is split into above, around, inside and below the part.
- Gouge and coverage are coarse grid measurements, labelled approximate.
```

- [ ] **Step 3: Remove the reference attachments and verify**

```bash
git rm -r -q docs/superpowers/plans/2026-10-03-ai-planner-m2
cargo fmt --check && cargo test --locked --quiet 2>&1 | tail -2 && cargo build --release --locked 2>&1 | tail -1
```

Expected: no fmt output; N + 18 passed; `Finished`.

- [ ] **Step 4: Commit**

```bash
git add README.md docs/VALIDATION.md && git commit -q -m "Document the planner's part report, plan format and checks

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```
