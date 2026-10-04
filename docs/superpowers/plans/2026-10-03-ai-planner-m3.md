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
- Commit messages end with a blank line and `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Stage only the files each task names.

## How this plan was executed

The code was written test-first in a scratch clone of this repository (branch `m3-draft`) and reviewed there before it touched this branch, because the review rounds changed it substantially:

1. Draft from the prototype (planner loop, planner page, Apply and Undo, demo planner removed).
2. A five-angle review with one skeptic per serious finding (22 confirmed, 34 minor), then fix passes F1–F15, G1–G12, H1–H8, J1–J3, K1–K3 and L1–L4, each verified by independent reviewers. Rulings, findings and results are in `.superpowers/sdd/2026-10-03-ai-planner-m3/` (progress.md, draft-review.json and the *-result.json files).
3. The draft was squash-merged onto this branch after Milestone 2's last fix round, the suite and the renderer smoke were run on the merged result, and it lands as the two commits below.

### Task 1: Planner loop and the planner page (code and tests)

**Files:** `src/planner.rs` (new), `src/main.rs`, `src/ui.rs`, `src/help.rs`, `src/choice_help.rs`, `src/model.rs`, `src/openai.rs`, `src/post.rs`, `src/ngc.rs`, `src/post_check.rs`, `src/cam.rs`, `src/proposal.rs` (`clip` only), `src/smoke.rs`.

- [x] Squash-merge `m3-draft` onto this branch; resolve conflicts (only `docs/VALIDATION.md`'s test total).
- [x] `cargo fmt --check`, `cargo test --locked --offline` (0 failed), `cargo build --release --locked --offline`.
- [x] Commit the code and tests: "AI planner: the Plan button, checked preview, Apply and Undo".

### Task 2: Documentation and renderer check

**Files:** `README.md`, `docs/VALIDATION.md`, `docs/superpowers/specs/2026-10-03-ai-planner-design.md`.

- [x] Isolated renderer smoke run on the merged code (85 static screens and 24 camera samples); screens 81–85 inspected.
- [x] Commit the documentation: "Document the AI planner: Plan, preview, Apply and Undo".
- [ ] Final review of Milestones 2 and 3 together (whole-branch review against `main`).
