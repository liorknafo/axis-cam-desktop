# AI Planner M3 — The Plan Button Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** One click on the AI planner page asks the signed-in ChatGPT plan's model for a complete job, checks every proposal with the app's own generators, simulation and checks, sends failures back for correction, and shows a passing plan for review, Apply and Undo.

**Architecture:** `planner.rs` holds the loop (instructions, requests through a `Backend` trait, repair and coverage rounds, the best passing plan) and is tested with a scripted backend. `main.rs` gains a `Planner` state (`Idle`, `Running`, `Preview`, `Failed`), runs the loop as an abortable Iced task that reports progress, and applies or discards the result. `ui.rs` replaces the demo planner page. The demo planner (`Document::sample_plan` and its controls) is removed.

**Tech Stack:** Rust 2024, Iced 0.14, tokio; existing crates only.

**Spec:** `docs/superpowers/specs/2026-10-03-ai-planner-design.md` (section "M3")

## Global Constraints

- Source folder: `/Users/liorknafo/Documents/Codex/2026-09-21/i-x20/outputs/axis-cam-desktop`, branch `ai-planner`. Before every `cargo` command: `export CARGO_TARGET_DIR='/Users/liorknafo/Documents/Codex/2026-09-21/i-x20/work/native-target'`. Use `cargo test --locked --offline`. No new crates, no `#[allow]`.
- Never read, write or delete anything under `~/Library/Application Support/AXIS CAM Studio` or `~/.codex`. Never run the ignored Keychain test. No test contacts OpenAI or opens a browser. Unit tests never run Iced tasks and never build an `App` on the real workspace path.
- Nothing from the model reaches the job unless `plan_check::check` passed. Model text is shown as plain text, cleaned by `proposal::clip`.
- At most 4 model requests per run (`planner::MAX_REQUESTS`). A preview or a running plan is dropped whenever the job changes.
- UI text never claims fixture, clamp or machine collision checking, or exact gouge detection.
- Commit messages end with a blank line and `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>`. Stage only the files each task names.

## Reference implementation

`docs/superpowers/plans/2026-10-03-ai-planner-m3/` holds the code, prototyped against this codebase in a scratch clone where the full suite passes:

- `planner.rs`: the complete new module (Task 1).
- `task1.patch`, `task2-tests.patch`, `task2.patch`, `task3.patch`: exact diffs against the commit this plan was written on. Apply each with `git apply --3way <file>` from the repository root. If a patch does not apply cleanly, stop and report; do not hand-edit around it.

Task 3 deletes the folder. Test counts are relative: run the suite before Task 1 and call the passing total N.

---

### Task 1: Planner loop

**Files:**
- Create: `src/planner.rs`
- Modify: `src/openai.rs` (`Role::Assistant`), `src/main.rs` (module list)

**Interfaces:**
- Consumes: `analysis::report`, `proposal::{parse, schema, Proposal, Problem}`, `plan_check::{check, CheckReport}`, `openai::{Client, Request, Role, ApiError}`.
- Produces: `planner::{MAX_REQUESTS, Backend, Progress, Outcome, Failure, instructions, plan, OpenAiBackend, final_view}`; test-only `planner::tests::Script`.

- [ ] **Step 1: Write the failing tests**

Create `src/planner.rs` containing only the attachment's `#[cfg(test)] pub(crate) mod tests` (with `use super::*;` it will not compile yet). Apply `task1.patch` (adds `mod planner;` to `src/main.rs` and `Role::Assistant` to `src/openai.rs`).

Run: `cargo test --locked --offline --quiet planner:: 2>&1 | tail -5`
Expected: compile errors (`plan`, `Backend`, `Progress` not found).

- [ ] **Step 2: Add the implementation**

Replace `src/planner.rs` with the full attachment, unchanged.

- [ ] **Step 3: Run the tests**

Run: `cargo test --locked --offline --quiet 2>&1 | tail -3`
Expected: N + 8 passed; 0 failed. Dead-code warnings for `planner` items are expected until Task 2.

- [ ] **Step 4: Commit**

```bash
cargo fmt && git add src/planner.rs src/openai.rs src/main.rs && git commit -q -m "Planner loop: proposals, repair rounds and the best passing plan

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 2: The planner page

**Files:**
- Modify: `src/main.rs` (state, messages, handlers, model fallback), `src/ui.rs` (planner page), `src/help.rs`, `src/choice_help.rs`, `src/model.rs` (demo planner removed), `src/smoke.rs` (app tests and renderer steps)

**Interfaces:**
- Consumes: Task 1.
- Produces: `PlanPreview { outcome, view }`, `Planner { Idle, Running(Progress), Preview(Arc<PlanPreview>), Failed(Failure) }`, `App::plan_checklist()`, messages `Plan`, `PlanProgress(u64, Progress)`, `PlanFinished(u64, Result<Arc<PlanPreview>, Failure>)`, `PlanCancel`, `ApplyPlan`, `DiscardPlan`, `UndoPlan`; `DEFAULT_MODEL`.

- [ ] **Step 1: Write the failing tests**

Apply `task2-tests.patch` (the test module and renderer steps in `src/smoke.rs`).

Run: `cargo test --locked --offline --quiet 2>&1 | tail -5`
Expected: compile errors (`Planner`, `PlanPreview`, `plan_checklist`, the new messages not found).

- [ ] **Step 2: Add the implementation**

Apply `task2.patch`.

- [ ] **Step 3: Run the tests and the build**

```bash
cargo test --locked --offline --quiet 2>&1 | tail -3
cargo build --locked --offline 2>&1 | grep -E "^warning: (unused|function|constant|field|variant|method|struct)|never (used|read|constructed)" || echo "no warnings"
```

Expected: 0 failed (the total is in the task brief); "no warnings" (the `Role::System` warning from before this plan is gone, because the planner constructs it).

- [ ] **Step 4: Commit**

```bash
cargo fmt && git add src/main.rs src/ui.rs src/help.rs src/choice_help.rs src/model.rs src/smoke.rs && git commit -q -m "AI planner page: Plan, progress, checked preview, Apply and Undo

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```

---

### Task 3: Renderer check, documentation and cleanup

**Files:**
- Modify: `README.md`, `docs/VALIDATION.md`
- Delete: `docs/superpowers/plans/2026-10-03-ai-planner-m3/`

- [ ] **Step 1: Documentation**

Apply `task3.patch` (README: "Plan a job with AI", the model fallback, stale demo-planner text removed; VALIDATION: "AI planner" section).

- [ ] **Step 2: Renderer smoke run**

```bash
cargo build --release --locked --offline 2>&1 | tail -1
rm -rf .superpowers/smoke-m3 && "$CARGO_TARGET_DIR/release/axis-cam" --smoke-test "$PWD/.superpowers/smoke-m3" > .superpowers/smoke-m3.log 2>&1; echo "exit $?"
ls .superpowers/smoke-m3 | grep -c png
```

Expected: `exit 0`, 109 PNG files (85 static screens and 24 camera samples). The run opens a window for several minutes and closes it; it uses its own folder, not the user's workspace. The folder is untracked. Look at `81-planner-preview.png`, `82-planner-preview-stock.png`, `83-planner-failed.png`, `84-planner-running.png` and `85-planner-ready.png` and confirm each shows what its name says, with no clipped or overlapping text.

- [ ] **Step 3: Totals, cleanup, verify**

Update the test total and the screen count in `docs/VALIDATION.md` and `README.md` to the measured numbers.

```bash
git rm -r -q docs/superpowers/plans/2026-10-03-ai-planner-m3
cargo fmt --check && cargo test --locked --offline --quiet 2>&1 | tail -2
```

- [ ] **Step 4: Commit**

```bash
git add README.md docs/VALIDATION.md && git commit -q -m "Document the AI planner; renderer check of the planner page

Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>"
```
