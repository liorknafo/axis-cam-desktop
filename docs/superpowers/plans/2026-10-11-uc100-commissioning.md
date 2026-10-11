# UC100 machine control, part 4 (machine setup) — Implementation Plan

**Goal:** A Setup view on the Control page: a UC100 profile form that keeps the file's comments, the inputs with their edges, an output test, the driver's audit, and steps-per-mm calibration.

**Architecture:** New module `src/setup.rs`: `Draft` (the profile's `toml_edit` document, number fields being typed, validation through the driver) with `open`, `edit`, `save`, `revert`; the Setup view. `src/control.rs` gains the input log (`InputTracker` + recent edges) in `refresh`, the forced outputs and the calibration start, and the `TestOutput` / `Calibrate` actions. `main.rs` routes `Message::Setup` and `Message::ControlSetup`; Save reconnects a connected machine. `ui.rs` adds the switch and shares its Control page helpers with `setup.rs`.

**Spec:** `docs/superpowers/specs/2026-10-11-uc100-commissioning-design.md`.

**Branch:** `t3/uc100-commissioning` (from `t3/uc100-machine-control` at 4149373), worktree `C:\Users\liork\.t3\worktrees\axis-cam-desktop\t3-uc100-machine-control`.

## Constraints

- Connected tests use the simulated UC100 only. `export AXIS_SKIP_STEP=1`; the two STEP tests fail on this PC as before (baseline 785 passed, 2 failed).
- `cargo fmt --check` clean, no new warnings; commit after each task, ending with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- No driver change; no `ApplyConfig`.

## Tasks

1. **The profile document** (`src/setup.rs`, unit tests): open a profile into a `Draft`; edit numbers (integer stays integer), text, flags, pins (inline `{ pin, active_low }`), step rate; empty optional fields remove the key; missing tables are created; trailing comments and untouched lines survive; validation with `MachineConfig::from_toml`; save refuses an invalid draft, a field that is not a number, and a file changed on disk; save writes atomically.
2. **The Setup view and saving**: Control | Setup switch; the form (commissioning gate, axes, inputs, spindle and outputs, homing, controller); Save, Revert, unsaved and validity markers; Save while connected reconnects. Smoke tests: edit and save from the page; save while connected reconnects with the new profile and clears the machine positions; refused while running; renders connected and disconnected, valid and invalid.
3. **Inputs and the output test**: input levels, roles, changes and recent edges; E-stop report and latch bits; outputs with readback, role and High/Low/Auto. Tests: forcing a free output while Idle shows in the readback and Auto releases it; refused in E-stop and on a step pin; E-stop forgets forced pins; the input log records edges (fed statuses).
4. **Audit and calibration**: the audit panel; Move records the counter and step-jogs; the result from measured travel; Use in profile fills the form. Tests on the simulator: 100 mm commanded at 200 steps/mm, 99.5 mm measured gives 201.005; Use in profile then Save reconnects with it.
5. **Look in the app** (simulator, the throwaway `AXIS_LOOK_DIR` patch, never committed), README / VALIDATION, push, Linear.
