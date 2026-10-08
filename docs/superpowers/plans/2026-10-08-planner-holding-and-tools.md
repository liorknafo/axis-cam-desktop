# Leftover stock, holdable setups and planner tools — Implementation Plan

> **For agentic workers:** implement task by task (subagent-driven: one implementer and one reviewer per task, then a whole-branch review). Steps use checkbox (`- [ ]`) syntax.

**Goal:** Later setups start from the stock earlier setups left; a setup that its holder cannot hold fails the export and the AI planner's check with the cause; the planner picks each later setup's holder from the library and gets tools (`check_plan`, `stock_after`, `check_holding`, `cutting_data`, `surface_info`) to test and edit its plan before answering.

**Architecture:** Cuts carry the operation they came from. `rest::RestStock::left` proves which outer layers earlier cuts removed and trims the original block to the leftover box; the simulation sequence gives that box to later setups, so every consumer of `plan.stock` follows. New holdability checks sample the leftover material (`rest::Removed::at`) at the jaws, the bottom and the clamp bites, inside `post::held`/`post::prepare` (export and plan check). The planner's tool loop becomes a small tool registry; `check_plan` runs the full check and keeps per-setup stock state for the other tools.

**Tech Stack:** Rust 2024, Iced 0.14, serde/serde_json, tokio, the in-repo fake OpenAI server.

**Spec:** `docs/superpowers/specs/2026-10-08-planner-holding-and-tools-design.md`

## Global Constraints

- Leftover box: the original block trimmed per side only by layers proven removed by flat constant-height cuts (`RestStock`'s evidence); never smaller than the part's bounds; equal to the original with no cuts.
- Holdability thresholds: jaws ≥ 60 % of 12 × 4 contact samples with material; bottom ≥ 50 % of 8 × 8 samples; each clamp bite ≥ 60 % of 6 × 3 samples; stock taller than the grip by ≥ 1 mm.
- Holding failures are problems with stage `workholding`, operation `None`, and name the earlier operations that removed the material, e.g. "Setup 2 cannot be held in the vise: setup 1, operation 3 (Contour \"Outside\") removed the material the jaws grip on −Y from 0 to 8 mm above the stock's bottom. Leave at least 8 mm there (for example with `bottom_offset`) and finish it in a later setup, or choose another holder."
- Report `workholding_library`: entries numbered from 1, sizes only (no names, no notes). Proposal setup field `holder`: integer or null; setup 1 null or its own number; later setups null = same as setup 1.
- Tools: `view_model` (pictures on), `check_plan`, `stock_after`, `check_holding`, `cutting_data`, `surface_info`; all tool calls share **16** tool rounds per planner request; `check_plan` 3 per planner request and 8 per run; a refusal of the tools drops them all.
- Editor note, verbatim: "Original stock size until the earlier setups are simulated."
- Privacy unchanged: no project title, setup names, holder names or notes reach the model.
- `cargo test --locked` passes and `cargo clippy --all-targets` adds no warnings after every task. Commits end with a Co-Authored-By line for the model that wrote them.

## Review Focus

1. A part whose earlier setup removed all side material (full-depth outside contour) followed by a vise setup: the check must fail with the cause, not pass with jaws in air. (Task 3)
2. A thin skin left by a ball end mill or a ramp must not trim the leftover box (conservative). (Task 2)
3. `check_plan` with a proposal that does not parse, does not build, or takes long: one-sentence errors, limits enforced, no hang. (Task 7)
4. Tools called before any draft exists (`stock_after`, `check_holding`): one-sentence error, not a panic. (Task 7)
5. The editor cache must never show a leftover box for an edited document it was not computed for. (Task 4)

---

### Task 1: Cuts remember their operation

**Files:** `src/stock_surface.rs` (`Cut`, `Cut::extend`), `src/volume.rs` (`Volume::cut`), `src/simulation.rs` (callers of `Volume::cut`), tests in those files.

**Produces:** `stock_surface::Cut { .., pub source: (u16, u16) }` — setup and operation index (from 0) of the operation whose move made the cut; `Volume::cut(.., current: Frame, source: (u16, u16))`; `Cut::extend` merges only cuts with the same `source`.

- [ ] Write tests: a sequence with two operations in setup 1 and one in setup 2 yields cuts whose `source` values are exactly those three pairs, in order; two collinear consecutive moves of the same operation still merge; consecutive moves of two operations never merge.
- [ ] Add the field and the parameter; pass `(si, oi)` from the simulation (the setup and operation being simulated — find where `Simulation` cuts its volume during `seek`; it needs the indices: store them on `Simulation` when the sequence creates it, e.g. `pub source: (u16, u16)`, default `(0, 0)` for previews outside a sequence).
- [ ] Run `cargo test --locked`, clippy; commit "Cuts remember the operation that made them".

### Task 2: The leftover box

**Files:** `src/rest.rs` (`RestStock::left`), `src/simulation.rs:640-650` (`sequence_with`), tests.

**Consumes:** `RestStock`, its `covered` test and capsule evidence. **Produces:** `pub fn left(&self, part: Bounds) -> Bounds` on `RestStock` (in the current frame, mm), and later setups' `preview_stock` set to it.

- [ ] Write tests in `rest.rs`:
  - no cuts → the original box (`extent`);
  - a facing pass covering the whole top at height `top − 2` → the box's `max[2]` is exactly `top − 2` (within 0.001);
  - full-depth outside contour passes on −Y and +Y (flat end mill, levels down past the stock bottom) → `min[1]`/`max[1]` move in by the removed width;
  - the same contour stopping 6 mm above the bottom → sides unchanged;
  - a ball end mill pass or a ramp over the whole top → top unchanged;
  - the result never inside `part`.
- [ ] Implement: for each of the six sides, candidate depths = the capsule bounds' faces on that axis lying strictly inside the extent (plus the extent face itself); binary-search the deepest candidate whose slab (full extent on the other two axes, from the side to the candidate) is `covered` (same budget pattern as `top`); clamp so the box still contains `part`. Sides are trimmed in a fixed order (top, bottom, −X, +X, −Y, +Y) with each slab test using the box trimmed so far.
- [ ] In `sequence_with`, for a setup with an inherited `volume`: `setup.preview_stock = Some(RestStock::new(v, frame).left(part_bounds))` where `part_bounds` is the part's bounds in that setup's frame in mm (`Placement::new(mesh, Some(&setup)).bounds.scaled(1. / mesh.display_scale())`). Keep `preview_remaining` as it is.
- [ ] Sequence test: setup 1 faces 2 mm off the top; setup 2 is flipped (rx 180); setup 2's first simulation's `plan.stock` height is the original height − 2, and its `min[2]` is 2 mm higher than the original block carried into setup 2. Its vise jaws (`holding::boxes` on that stock) start at the new bottom.
- [ ] Run the full suite: existing later-setup tests (`plan_check`, `post`, `holding`, `smoke`) may change numbers because later setups' stock shrank; update an expectation only where the new number is the leftover box's correct consequence (explain each in the report); stop and report if a failure is anything else.
- [ ] Commit "Later setups start from the stock earlier setups left".

### Task 3: Holding checks on the leftover stock

**Files:** `src/holding.rs` (new `holdable`), `src/post.rs` (`held`, `prepare`), `src/model.rs` (`holding_problems` for the opening/height parts), tests.

**Consumes:** `rest::Removed::new(volume, frame).at(p)`, `Cut.source`, `holding::boxes`/`Layout`. **Produces:**

```rust
/// Why `layout` with `holder` cannot hold `stock` (the setup's starting box,
/// mm) given the material earlier setups removed (`removed`, None for setup 1):
/// stock not taller than the grip by 1 mm, too little material at a jaw, under
/// the stock, or under a clamp's bite. Each message names the operations
/// (`names(setup, operation)` gives "setup 1, operation 3 (Contour \"Outside\")")
/// whose cuts removed most of the failed samples.
pub fn holdable(
    layout: &Layout,
    holder: &Holder,
    stock: Bounds,
    removed: Option<&crate::rest::Removed>,
    names: &dyn Fn(u16, u16) -> String,
) -> Vec<String>
```

`rest::Removed` gains `pub fn by(&self, p: [f32; 3]) -> Option<(u16, u16)>` — the `source` of the first cut that removed `p`.

- [ ] Tests (fixtures: build a two-setup job in code like `plan_check` tests do, or use `proposal::tests::{block, good}` and variants):
  - a vise setup after a full-depth outside contour on the jaw sides → one message, starting "Setup 2 cannot be held in the vise:", naming the contour operation and the band "from 0 to {grip} mm above the stock's bottom";
  - grip ≥ stock height − 1 → "The stock is {h} mm tall; the vise grips {g} mm of it, which leaves less than 1 mm above the jaws."
  - bottom removed (e.g. a pocket through the bottom over most of it) → message about the stock's bottom;
  - clamps whose bites lie over a faced-away rim → message per clamp;
  - the same job with a `bottom_offset` that leaves the band → no message;
  - setup 1 (no `removed`) only gets the height rule.
- [ ] Implement sampling exactly as the Global Constraints say (contact face just inside the stock side, 0.05 mm in; points spread evenly, ends inset by 1 % of the span). The cause: count `by()` over failed samples, name the operations with ≥ 25 % of them, most first.
- [ ] Wire into `post::held` and `post::prepare`: after `holding_boxes` succeeds, for setups whose first simulation has `initial_volume` and `frame`, build `Removed` and append `holdable` messages (prefixed with the setup name like other holding messages, operation `None`). Setup 1 gets the height rule too. Export and the plan check both see them.
- [ ] `Document::holding_problems` (editor): add the height rule (no volume there).
- [ ] Full suite, clippy; commit "Workholding checks that the stock left can be held".

### Task 4: The setup editor knows the leftover box

**Files:** `src/main.rs` (cache, background job, invalidation), `src/ui.rs` (stock label, note, `holding_boxes`), `src/model.rs` (`holding_issues` with a stock override), tests in `src/smoke.rs`.

**Produces:** `App.left_stock: LeftStock { key: u64, boxes: Vec<Option<Bounds>> }` and `fn known_stock(&self, si: usize) -> (Option<Bounds>, bool /*current*/)`; `Document::holding_issues_with(setup, stock: Option<Bounds>)`.

- [ ] Tests: after an edit to setup 1's operation the cache is stale and the editor shows the note; when the job finishes (drive it synchronously in the test) the box is the leftover box and the opening issue uses it; an edit made while the job ran discards its result (key mismatch).
- [ ] Implement: key = hash of the serialized setups, tools and the mesh fingerprint; job runs `simulation::sequence` on the blocking pool (`planner::blocking`-style, or the pattern `refresh_post` uses — follow it) after the same debounce as the NC program refresh, only when the document has a later setup; store the first simulation's `plan.stock` of each later setup.
- [ ] `holding_issues`, `ui::holding_boxes` and the stock label use `known_stock`; when not current, the nominal box and the note under the setup's workholding controls.
- [ ] Full suite, clippy; commit "The setup editor uses the stock earlier setups left".

### Task 5: The planner chooses holders; holding rules in its instructions

**Files:** `src/analysis.rs` (`PartReport.workholding_library`), `src/proposal.rs` (schema `holder`, `ProposedSetup.holder`, build, validation), `src/planner.rs` (instructions), tests.

**Produces:** `analysis::LibraryEntry { number: usize, kind: &'static str, sizes: BTreeMap<&'static str, f32> }` serialized as `{"number":1,"kind":"Vise","jaw_width":…}`; `ProposedSetup.holder: Option<usize>`.

- [ ] Tests: the report lists every library entry by number with sizes and no names/notes (assert the JSON has no entry name); a proposal whose setup 2 names a clamp entry builds setup 2 with `holding` "Clamps" and that entry's id; null keeps setup 1's; an unknown number → problem stage `structure` "Setup 2: workholding 9 is not in workholding_library."; setup 1 naming another entry → problem.
- [ ] Implement (the schema stays strict: `holder` required, `{"type": ["integer","null"]}`); `instructions_with` gains PLANNING THE HOLDING (spec §5) — the tool names appear only when tools are offered (Task 6/7 pass a flag).
- [ ] Full suite, clippy; commit "The planner picks each setup's holder from the library".

### Task 6: A tool registry; `cutting_data` and `surface_info`

**Files:** new `src/planner_tools.rs` (registry, `cutting_data`, `surface_info`), `src/planner.rs` (`ask` dispatch, `MAX_TOOL_ROUNDS` 16, offering tools without pictures, fallback drops all), `src/look.rs` (unchanged API), `src/ui.rs`/`main.rs` (timeline lines reuse `Event::Looked` with `png: None`), tests.

**Produces:** `planner_tools::offered(pictures: bool) -> Vec<openai::Tool>`; `planner_tools::answer(name: &str, arguments: &str, state: &mut ToolState) -> Answer` where `Answer { output: String, line: String, picture: Option<look::Picture> }`; `ToolState { scene: look::Scene, checked: Option<Arc<Checked>>, checks_here: usize, checks_run: usize }` (Checked arrives in Task 7; define it empty here).

- [ ] Tests: `cutting_data` for the block's T3 at the material's data gives rpm range clipped to the machine maximum, a recommended rpm rounded to 10, feed range at that rpm from the band and flutes, and the band; unknown tool → error sentence; `surface_info` for a top surface lists every orientation that sees it with depth/area/bbox/holes and the surface's area and normal; unknown surface → error; the tools are offered with pictures off (no `view_model`); a refused tool request drops all tools; 16 rounds.
- [ ] Implement; move `view_model` handling into the registry (pictures limits unchanged); timeline line per call (spec §4 examples).
- [ ] Full suite, clippy; commit "Planner tools: a registry, cutting data and surface info".

### Task 7: `check_plan`, `stock_after`, `check_holding`

**Files:** `src/plan_check.rs` (`evaluate_full` returning per-setup state), `src/planner_tools.rs`, `src/planner.rs` (planner's own checks update the state), instructions TOOLS section, tests.

**Produces:**

```rust
pub struct CheckedSetup { pub stock: Bounds, pub frame: Option<Frame>, pub before: Option<Arc<Volume>>, pub plans: Vec<Plan> }
pub struct Checked { pub job: Arc<Document>, pub setups: Vec<CheckedSetup> }
pub fn evaluate_full(doc, mesh, report, proposal) -> (CheckReport, Option<Document>, Option<Checked>)
```

and `CheckReport` gains `stocks: Vec<StockSummary>` (serialized; one per setup: `size`, `box_from_part_corner`, `removed: {top, bottom, -X, +X, -Y, +Y}` mm, `by: [..operation names..]`).

- [ ] Tests: `check_plan` with a good draft answers "passed" and sets the state; with a draft that does not parse/build answers the problems; limits 3 per request / 8 per run with their messages; `stock_after(2)` after a facing draft reports the top removed by 2 mm by setup 1, operation 1; `stock_after` before any draft → "Check a draft with check_plan first, or answer with a proposal."; `check_holding(2, n)` for a vise that opens too little → the opening problem; for a holdable entry → empty problems and the suggested layout; unknown setup/holder → error sentences; the planner's own check of an answer also updates the state; the timeline lines.
- [ ] Implement; instructions TOOLS section lists each tool with its limits.
- [ ] Full suite, clippy; commit "Planner tools: check a draft, the stock before a setup, a holder for it".

### Task 8: Docs and smoke

**Files:** `src/smoke.rs` (a flipped setup with its vise on the leftover stock; a timeline with tool lines), `README.md`, `docs/VALIDATION.md`.

- [ ] Add smoke screens 92 (a flipped second setup after facing, jaws on the shorter stock) and 93 (a planner timeline with `check_plan`, `stock_after`, `check_holding`, `cutting_data` and `surface_info` lines); run the smoke test; inspect; README sections (leftover stock, holding checks, holder choice, tools and limits, editor note), VALIDATION section naming each test; full suite, clippy; commit "Document leftover stock, holding checks and planner tools".
