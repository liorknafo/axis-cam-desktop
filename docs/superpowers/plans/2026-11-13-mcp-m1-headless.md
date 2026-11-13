# MCP M1 — headless server and the command core (LIO-470)

The spec is `docs/superpowers/specs/2026-11-13-mcp-server-design.md`, and it decides what this milestone does. This plan only orders the work. Each step is one commit on the branch and leaves `cargo test` green.

Module layout: `src/mcp/` holds:
- `mod.rs`;
- `protocol.rs`: JSON-RPC over stdio;
- `runner.rs`: the headless `App` thread;
- `engine.rs`: requests, outcomes, waiting, windows;
- `class.rs`: the exhaustive classification;
- `commands.rs`: the registry;
- `state.rs`: work-state snapshots, the change log and external events;
- `fields.rs`: the collector;
- `picks.rs`;
- `reads.rs`: the read-only tools;
- `checkpoint.rs`;
- `session.rs`.

## Step 1 — the server answers, and a headless App runs

- **`axis-cam --mcp [--session <id>]`.** `main()` dispatches it before iced starts.
  - A stdio reader thread parses JSON-RPC 2.0 lines.
  - It answers `initialize` at once (protocol `2025-06-18`, server instructions), plus `notifications/initialized`, `ping`, `tools/list` and `tools/call`.
  - Malformed lines get JSON-RPC errors. Responses are written by one writer.
- **`mcp::runner`.** It creates a session folder under `AXIS CAM Studio/mcp-sessions/<id>/`, copies `workspace.json` into it, and builds `App::from_workspace` on that copy, on a dedicated thread.
  - The thread runs an event loop over a channel: requests, task output messages, and a `Tick` timer at 33 ms or 500 ms (`ticks_fast`).
  - Tasks are pumped on a multi-thread tokio runtime with `iced_runtime::task::into_stream`, as `smoke.rs` `pump_task` does. Move `iced_runtime` to `[dependencies]`.
  - Window and widget actions are dropped. `generate_inline` stays off.
  - At start, the thread waits for `ModelRestored`.
- **`mcp::engine`, first version.** It provides `inspect` (project, setups and operations by id, selection, page, status, pending work and the session), `list_commands`, `describe_command`, and `execute` for a handful of commands: `select_setup`, `select_operation`, `add_operation`, `set_project_title`.
- **`App::pending()`** lists everything in the spec's *Waiting for work*. Commands wait for it within `wait_s`, which defaults to 25 s and may be at most 50 s.
- **Tests:**
  - the real binary over a pipe: `initialize`, `tools/list`, `tools/call inspect`, a malformed line;
  - a unit test that the runner settles after `add_operation`.

## Step 2 — classification, registry, machine boundary

- `mcp::class`: an exhaustive `match` with no wildcard over `Message` and every nested user enum. The classes are `Command(name)`, `Machine`, `Internal` and `Planner`.
- `mcp::commands`: the full command table from the spec's *The commands*, each entry with its schema, help (from `help::command`, `machine_help` and `run_help`), precondition, builder and expected effect.
- The machine boundary, exactly as the spec's allow-list.
- Path rules for the file commands.
- The export and save builders, which do the dialog's work: `open_project`, `save_project`, `import_model`, `export_gcode`, `export_setup_sheet`, `save_for_tablet`.
- **Tests:**
  - every `Command` message is built by its command;
  - the schemas are valid;
  - every command has help;
  - every `Machine` message is refused;
  - every `Edit` key literal in `ui.rs` and `main.rs` is reachable.

## Step 3 — outcomes and waiting

- The outcomes: `done`, `no_change`, `refused`, `busy`, `failed`, `still_running`, `cancelled` and `conflict`, from each command's precondition and expected effect.
- `command_id`, `wait {command_id}` and `cancel {command_id}`, with final results kept for the session.
- `batch`, with a deadline.
- Ownership windows, with `busy` for a mutation that arrives while one is open.
- The library normalisers move out of their handlers: `Document::saved_tool`, `saved_material`, `saved_holder` and `saved_machine`.
- **Tests:**
  - a failed save, an invalid draft, a missing selection, a generation error, a refused sample-only command, and `no_change`;
  - a slow generation answers `still_running` within `wait_s`, and `wait` returns the final result.

## Step 4 — revisions, change log, external events

- `mcp::state`: work-state snapshots, a streamed hash over the `WORK_STATE` list.
- The test over `App`'s fields: each is in `WORK_STATE` or in `NOT_WORK_STATE`.
- Revisions, the change log with the sources `agent`, `user` and `mixed`, and `if_revision`.
- `App::external(what)` at the sites that apply controller results.
- The shop-floor guard.
- The benchmark `mcp_snapshot_bench`, which is `#[ignore]`d.
- The spec's revision tests.

## Step 5 — fields

- A thread-local collector in the `ui.rs` field helpers.
- `operation_editor_with` gains a `tab` argument.
- `list_fields` and `edit_field`, with the key, choice and machine-key checks.
- **Tests:**
  - an unknown key is refused;
  - a value off the choice list is refused;
  - every `help::field_keys()` key and `Edit` literal is listed or covered by a command.

## Step 6 — picking

- `App::pick_kind`.
- `pick_surfaces` and `pick_features`, with toggle and fill semantics.
- The click transaction with its full undo.
- `pick_pin_holes`, `clear_pin_holes` and `place_pins`.
- The spec's picking tests.

## Step 7 — reads

- **Part and surfaces:** `list_surfaces`, `part_report`, `surface_info`, `cutting_data`.
- **`check_job`:** a new `plan_check` flag that fills in nothing, a revision and fingerprint on the result, and a stale refusal.
- **`check_holding`:** the saved placement, exactly as saved.
- **`stock_after`.**
- **`view_model`:** `look::draw` returned as an MCP image.
- **`inspect {what: "steps"}`:** with its own cursor.
- **`guide {topic}`:** the planner's machining rules.
- **Heavy and light reads:** heavy reads are refused while `locked()`, and their workers are counted in `planning_busy()` until they end.
- **`JobSnapshot`:** `Send`, checked at compile time.
- **Tests:** the misplaced vise and missing clamps fail; heavy reads are refused during a run, in both orderings.

## Step 8 — checkpoints

- `Document::project_id`.
- `App::job_replaced(doc, mesh)`, which `replace_document` also uses.
- `checkpoint`, `restore_checkpoint` and `list_checkpoints`, with the candidate mesh, the check in the turn that commits, saved worked out again by value, and the cross-project refusal.
- The spec's checkpoint tests.

## Step 9 — sessions

- `session.json`, written with the `Document` before a result is sent.
- The lock file.
- `list_sessions`, `resume_session` and `delete_session`.
- The client EOF leaves the session on disk.
- **Tests:** resume after a draft edit; two processes resuming the same session; the 31-day restart.

## Step 10 — end to end

- **STEP fixture:**
  - import the part, then add a setup with stock;
  - add Face, Pocket, Contour and Drill, with picks from `list_surfaces` and fields set;
  - `suggest_layout`, `generate_all`, `check_job`;
  - export G-code and compare it with `--export-gcode`;
  - save and re-open;
  - end the client, resume the session, and find the job intact.
- **Lathe and manual:** a manual setup and a lathe setup, each built through commands only.
- **Live check, headless:** Claude Code with `axis-cam --mcp` added plans the STEP fixture from a one-line request.
