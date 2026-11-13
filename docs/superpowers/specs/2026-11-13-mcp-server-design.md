# MCP server in place of the AI planner — design

Date: 13 November 2026. Decisions taken with the user in conversation before this spec was written. Revised after the first review (gpt-6.1-sol, REJECT, 10 findings), see *Review history* at the end.

## Goal

An AI agent (Claude Code, Codex or any MCP client) can do everything a user can do in AXIS CAM Studio when planning a job. That covers importing a part, adding and editing setups and operations, picking surfaces, using the libraries, generating, checking, reviewing the NC output, exporting and saving. It can also look at the machine and stop it. The agent drives the app through an MCP server that the app binary provides. The in-app AI planner is removed: ChatGPT sign-in, the Planner page, the chat, the preview and Apply.

## Decisions already made by the user

- **Machine access is read-only, except for stopping.** The agent can read the machine's state: connection, phase, position, spindle, run progress and alarms. It can Hold, Stop and E-stop. Everything that moves the machine, turns the spindle or coolant on, connects, pairs, or changes what the controller holds stays a human action in the app.
- **The AI planner is removed** once the MCP server covers what its tools did. This covers the OpenAI client, ChatGPT sign-in, the session store, the Planner page and the planner CLI flags.
- **Live window first, headless otherwise** (the SolidCraft model). When the app's window is open, commands act on that window and the user watches them happen. When no app is running, the server runs its own windowless session.

## Non-goals

- Commands that move the machine, energise anything, change the controller or record shop-floor progress (see *The machine boundary*).
- An MCP server on the shop tablet build (`SHOP`).
- Remote access. The bridge listens on loopback only.
- An "auto-plan" button. The agent plans with the user's own commands.
- General undo in the app. The MCP offers checkpoints.

## Shape

```
MCP client ──stdio──▶ axis-cam --mcp ──loopback TCP + token──▶ running AXIS CAM window
                            │                                   (Message on the UI thread)
                            └── no app running: headless App on its own thread
```

- `axis-cam --mcp` speaks MCP over stdio: JSON-RPC 2.0, one message per line, protocol version `2025-06-18`. It supports `initialize`, `notifications/initialized`, `ping`, `tools/list` and `tools/call`.
  - It is written by hand on `serde_json`, because only these few methods are needed.
  - The stdio reader runs on its own thread, so a `cancel` or `estop` is read while another call is still working.
- Every call is carried out by one **engine**, `mcp::Engine`, against an `App`. It is the same code for the live window's `App` and a headless one.
- Requests are serialized per `App`. Only one mutating command runs at a time. Read-only tools and the stop commands go ahead of a waiting mutation.

### Instance discovery

- **Instance files.** Each desktop app with agent access on writes an instance file, `AXIS CAM Studio/mcp/<pid>.json`, beside `workspace.json`. It holds the port, the token, the pid, the start time, the project title and file, and a `state` of `on` or `off`.
  - The token is 256 random bits, from two v4 UUIDs.
  - The file is written with `storage::write_atomic`.
- **Access to the file.** On macOS and Linux the file is created with mode `0600`. On Windows it lives in `%LOCALAPPDATA%`, whose inherited ACL already limits it to the user. A test checks that the file is not readable by other users on Unix.
- **When agent access is off.** With the Agents toggle off, the app still writes the file, with `state: "off"` and no port, so a client learns the window exists but refuses agents.
- **Cleanup.** The app deletes only its own file on close. Files whose pid is not alive are deleted by the next reader.

**Choosing the backend at the first call:**

| Found | Backend |
| --- | --- |
| No live instance file | Headless (new or resumed session, below) |
| One, `on` | Live, pinned to that pid |
| One, `off` | Error: "Agent access is off in the open AXIS CAM window. Turn it on in Agents, or close the app to work without a window." Never headless. |
| Several | Error listing them (pid, project, started), until the agent calls `select_window {pid}` |
| `on`, but the token is refused or the port does not answer | Error saying so. Never headless. |

- The choice is pinned for the MCP session.
- If the pinned window closes, calls fail with "The AXIS CAM window closed". They do not move to a headless session.
- A headless session that has made no change yet moves to a window that appears later, as SolidCraft does. Otherwise it stays headless, and every result names the backend.

### Headless sessions

A headless session is kept on disk, so a client crash or restart does not lose the work.

- **Where it lives.** It has an id and its own folder, `AXIS CAM Studio/mcp-sessions/<id>/`. It starts from a copy of the user's `workspace.json`, so it has their libraries.
  - The app's own autosave (`persist`) writes that copy, never the user's workspace.
  - The checkpoints are in the same folder.
- **Reporting.** `inspect` and every result report the session id, its folder, whether the job is saved to a project file, and the paths of the last saves and exports.
- **Resuming.** On stdin EOF the session is left on disk. `resume_session {id}` (or `--mcp --session <id>`) continues it. `list_sessions` lists them with their title and last change. Sessions untouched for 30 days are deleted at the next start.
- **Loading.** Startup waits until the saved model is restored (`ModelRestored`) before the first command runs, so the part is loaded.

**The headless runner.** The `App` lives on one dedicated thread, which loops over a channel of events:
- requests from the engine;
- messages produced by tasks;
- a timer that sends `Tick` at the rate the window's subscription would (33 ms or 500 ms, `ticks_fast`).

Tasks are run with `iced_runtime::task::into_stream` on a multi-thread tokio runtime. This moves `iced_runtime` from dev-dependencies to dependencies; it is already in the tree through iced. Each task's output messages go back into the channel, and window and widget actions are dropped.

`generate_inline` stays off. Generation, layout, left stock and meshing run in the background exactly as in the window, so the loop keeps taking events: a `cancel` (`CancelGenerate`) or a status read is handled while a generation runs.

### Waiting for work: `App::pending()`

Both backends wait until `App::pending()` is empty, so the window and the headless session settle the same way. `App::pending()` lists the work still due for the job as it now is. It is new app code beside the busy flags, so the rules stay next to the work:

- a generation (`generating`) for the current revision;
- an import, reimport, model restore, save or export;
- the post check (`post_busy`), and the left stock and holding checks;
- the run check;
- the guide build;
- the part holes, the start and stock meshes, and the tooth load;
- a scrub catch-up or replay;
- **edits not yet settled:** `dirty` and the edit pause (`EDIT_PAUSE`) not yet over. The work that a later `Tick` starts counts as pending before it has started.

A command returns when nothing is pending, or after its `wait_s`, which defaults to 120 s and may be at most 600 s. If it runs out of time, its outcome is `still_running` with the pending list. `wait {}` waits again, and `cancel {}` cancels a generation.

In the window, a pending MCP request makes `Tick` run fast. The engine answers from the UI thread only once `pending()` is empty.

## Identity, revisions and concurrency

- **Revision.** `App` gains a `revision: u64`, which `touch()` increments; it counts changes to the job. Every result returns the revision and the stable ids of the setups and operations it touched. Setups and operations already have `id`s.
- **Addressing.** Commands address setups and operations by id (`"setup": "<id>"`); an index is accepted too and turned into the id when the request arrives. A command that acts on "the selected" operation first selects it by id in the same UI-thread turn. The window handles one message at a time, so a user's click cannot land inside a command.
- **Conflict check.** A mutating call may pass `if_revision`. If the job changed since then, the call is refused with the changes ("conflict").
- **The user's own changes.** The engine records the revision after each of its commands. When the next request finds the revision moved by someone else, its result says so: "the user changed the job since your last command", with what changed. The agent always learns about the user's work.
- **Checkpoints and the user's edits.** `restore_checkpoint` is refused when the user changed the job since the checkpoint, unless it is called with `discard_user_changes: true`. In the window, that also asks the user in a dialog, which they can decline.

## Tools

| Tool | Does |
| --- | --- |
| `list_commands {filter?, group?}` | Name, group, one-line help, and whether the command is allowed. Groups: project, part, setup, operation, picks, generate, libraries, view, review, export, machine. |
| `describe_command {name}` | Full help (from `help::command`, `machine_help` and `run_help`), its JSON schema, what must exist or be selected, and its expected effect. |
| `execute {command, args, wait_s?, if_revision?}` | Runs one command. |
| `batch {steps, stop_on_error: true}` | Runs commands in order, stopping at the first outcome that is not `done` or `no_change`. It is not all-or-nothing; take a checkpoint first. |
| `inspect {what?}` | The state as the user would read it. Project (title, file, material, saved, revision), setups and their operations (by id), selection, page and tab, status line, pending work, and the session. `what` narrows it to a setup, an operation, the libraries, the page, the NC review or the manual guide (read only). |
| `list_fields {target, setup?, operation?}` | Every editable field of a target (below). |
| `list_surfaces {setup?}` | The part's surfaces with id, normal, centre, area and smooth region, and the part report's planes from that setup's orientation. For the built-in sample, its features instead (`model::FEATURES`). |
| `part_report {}` | `analysis::report` for the part and setup 1. |
| `check_job {}` | The plan checks on the job **exactly as it is** (below). |
| `stock_after {setup}` / `check_holding {setup}` / `cutting_data {tool, op?}` / `surface_info {id}` | The planner tools, reading the live job. |
| `view_model {view, …}` | `look::draw`, as an MCP image (PNG, base64). |
| `screenshot {}` | Live: the window. Headless: refused, with a pointer to `view_model`. |
| `checkpoint {name}` / `restore_checkpoint {name, discard_user_changes?}` / `list_checkpoints {}` | See *Checkpoints*. |
| `machine_status {}` | See *Machine status*. |
| `wait {wait_s?}` / `cancel {}` | See *Waiting for work*. |
| `select_window {pid}` / `list_sessions {}` / `resume_session {id}` | Discovery. |
| `guide {topic}` | The machining rules the planner's instructions held: stepdowns, cutter choice, holding, chip load and minimum rpm. |

Read-only tools work on a snapshot of the `Document` and mesh. In the window, the costly ones (`check_job`, `check_holding`, `part_report`, `view_model`, `list_surfaces` on a large part) run off the UI thread, through a `Task`.

### Outcomes

Every `execute` result:

```
{ "outcome": "done" | "no_change" | "refused" | "failed" | "still_running" | "conflict",
  "reason": "…",                     // why, for anything but done
  "backend": "live" | "headless",
  "revision": 42,
  "status": "<the status line after the command>",
  "changed": [{"kind": "operation", "id": "…", "what": "added"}],
  "selection": {"setup": "<id>", "operation": "<id>", "page": "Work", "tab": "Geometry"},
  "pending": [],
  "user_changed": false,
  "notes": ["…"] }
```

The status line alone does not tell whether a command worked. Each registry entry declares these instead, and they are checked in code:

- **A precondition**, checked before any message is sent: the setup or operation exists, it is the right kind (a lathe setup for `add_turn`), a draft is open, the app is not locked by a run. If it fails, the outcome is `refused`, with a sentence that says what is missing.
- **An expected effect**, checked after the app has settled. If it holds, the outcome is `done`. If nothing changed and nothing was asked to change, `no_change`. Otherwise `failed`, and the reason is the status line if the command changed it, else a sentence for the command. Examples of expected effects:
  - `add_operation`: one more operation in the setup, now selected.
  - `save_tool`: the draft is closed and the library holds the tool, changed.
  - `save_project`: the file is written, and the project is not dirty.
  - `generate`: a simulation for the operation at this revision, or a `GenerationFailed` reason.
  - `select_operation`: the selection is that id.

Tests cover a failed save (path not writable), an invalid library draft, a selection that does not exist, a generation error, and a refused click (a sample-only command on an imported part).

### Checkpoints

A checkpoint holds the `Document`, the selection, the page and tab, the open library drafts, the project file path, whether the project was saved, and the revision. It is stored in the session folder (headless) or in memory (live, at most 20). Restoring sets these fields back directly through a new `App::restore_checkpoint`, not `replace_document`, which resets the drafts and the project file. Then the meshes and checks follow as after an edit.

## Commands

### The registry and coverage

`mcp::commands` holds one table: name → group, help key, argument schema, precondition, expected effect, and a builder. The builder maps the arguments and the `App` to `Message`s, and for an export it does the file dialog's work.

Coverage is enforced by the compiler, not by a list:
- `mcp::class(&Message) -> Class` is a `match` with no wildcard arm over every `Message` variant. Every nested user enum is classified the same way: `GuideAction`, `setup::Msg`, `setup::Edit`, `section::Edit`, `viewport::Event`, `control::Action`, `run::Action`, `ui::ControlTab` and `ui::JobView`.
- `Class` is one of:
  - `Command(name)`: reachable through that command;
  - `Machine`: refused (see the boundary);
  - `Internal`: results, ticks, window events, keys, hovers, frames;
  - `Planner`: until it is removed.
- A new variant does not compile until it is classified.

A test then builds each `Command(name)` message through its command, and checks that each command's schema is valid and its help non-empty.

Button-backed edits are covered too. A test scans `ui.rs` and `main.rs` for every `Message::Edit(Target::…, "<key>"` literal and requires each key to be reachable, through `list_fields` or a named command. Examples are setup `origin`, operation `pin_setup`, `pin_holes` and `pin_place`.

### The machine boundary (allow-list)

The engine allows a command only if its class is `Command`. Every `Machine` variant is refused, and so is any command whose effect reaches the controller:

- **Refused, motion or energising:** Connect, Disconnect, `SwitchMachine`, `ControlMachine`, `MachineReset`, the connection kind, every Pi action (address, code, pair, forget, take control), the jog keys, and every `control::Action` other than `JogRelease`. That includes Home, GoTo, JogPress, CalMove, the probes, the calibrations, Spindle, Flood, Mist, FeedOverride, OverrideLimits, MdiSend and TestOutput.
- **Refused, controller state:**
  - `WorkZero`, `WorkHalf`, `WorkSet` (they send `SetWork`), `Wcs` (it sends MDI), `MachineZero`, `KeepPosition`, `RecordPins`, `ReferencePins`, `PinZero`, `SaveCentreline`, `Remounted`, `TouchOff` and `ToSimZero`;
  - every `setup::Msg`, which is the UC100 profile editor, including Save and UseCalibration.
- **Refused, the run:** every `run::Action` except `Hold` and `Stop`. That includes `Setup` (and so `RunOnMachine`), `Check`, `Next`, `From`, `Skip`, `SkipSteps`, `StartStep`, `StopAfterStep`, `KeepZero`, `KeepToolZ`, `CycleStart`, `Resume`, the reference probes and `CoolantOff`.
- **Refused, shop-floor progress:** every `GuideAction`. The manual mill and lathe Steps record progress, measured diameters and the cross-slide dial reading, which is the machinist's record at the machine. `inspect {what: "steps"}` reads the guide.
- **Refused, library changes that reach the controller:**
  - `add_rotary_machine`, which writes a UC100 profile file;
  - `save_machine` and `delete_machine` for the machine the Control page is connected to (`save_machine` pushes probe, pin and rotary settings into the live connection);
  - the machine fields `uc100_profile`, `uc100_connection` and `pi_*` (refused in `edit_field` for any machine, with this reason);
  - writes by `save_project` or an export onto a file a library machine names as its UC100 profile.

  Library editing of machines that are not connected stays allowed (travel, spindle, kinematics, controller name, rotary geometry). It changes the project, not a controller.
- **Allowed machine commands:**
  - `hold` (`Run(Hold)`);
  - `stop` (`Run(Stop)`);
  - `estop` (`MachineEStop`);
  - `jog_release` (`Machine(JogRelease)`: a jog the user holds is ended).

The refusal reason reads: "This moves the machine or changes what its controller holds, so it is done in the app by the user." These rules are in place in M1, before the live bridge exists.

While the app is `locked()`, which means a run, jog, probe or homing is in progress, mutating commands are refused with "The machine is running; the job cannot change until it stops." Read-only tools, `machine_status` and the four stop commands still work.

### Machine status

`machine_status` returns:
- the machine and connection kind;
- the link state, `live`, `lost` or `none`, and the age of the newest snapshot in ms;
- the Pi's control owner (this computer, another, or none) and whether this computer is following an unfollowed run;
- the phase, machine and work position, work offset, spindle, coolant and feed override;
- the run's setup, step and progress;
- alarms and limit inputs.

A snapshot kept after a link was lost (`control.rs`'s last-known state) is marked `"last_known": true` with its age. It is never shown as current.

A stop command reports:
- whether the command was **sent**, or refused, with the reason (for example, Hold during probing is not supported, so `stop` or `estop` is suggested);
- the phase it **observed** within 2 s afterwards.

Tests use the simulated UC100, a Pi link dropped mid-run (`uc100-remote`'s test server), and a watcher that does not own the run.

### Fields

The inspector's field helpers in `ui.rs` record a `FieldInfo` while a collector is on. The helpers are `field`, `checked_field`, `hinted_field`, `choice`, `choice_value`, the three pick lists, and the button rows that send `Edit`. A `FieldInfo` holds the target, key, label, tab, current value, choices, and the unit from the label.

`list_fields` turns the collector on and builds the target's editor once per tab with `ui::probe`'s layout, without drawing. This runs off the UI thread on a copy of the `App` view state.

`edit_field {target, key, value, setup?, operation?}` checks three things:
- the key is listed for that target as it now is;
- a choice value is one of its choices;
- the key is not a refused machine key.

So a wrong key never enters the project, which `Edit` alone would allow. The tests check that every key `help::field_keys()` knows, and every `Edit` key literal in the code, is listed in the sample and fixture projects, or covered by a named command.

### The commands

Arguments are JSON. Values that were `&'static str` in a message are enums in the schema. `setup` and `operation` take an id or an index.

- **Project:**
  - `new_project`, `open_sample`, `resume_project`, `recover_previous_project`;
  - `open_project {path}`, `save_project {path?}`;
  - `set_project_title {title}`, `set_project_material {id}`;
  - `goto_page {page}`. It cannot go to Control in the headless backend, where there is no machine.
- **Part:** `import_model {path}`, `reimport_model`, `dismiss_pick_note`.
- **Setup:**
  - `select_setup`, `add_setup {machine?}`, `add_manual_setup`, `add_lathe_setup`;
  - `duplicate_setup`, `remove_setup` (it confirms itself), `move_setup {delta}`;
  - `flip_setup {axis: rx|ry}`, `turn_stock {face, table_turn}`, `square_to_stock`;
  - `assign_machine {id}`, `set_origin {origin}`, `set_setup_step {step}`;
  - `set_clamps {text}`, `suggest_layout`;
  - `show_stock_after_setup`, `open_setup_editor`.
- **Operation:**
  - `select_operation`, `add_operation {strategy}`, `add_turn {kind}`;
  - `duplicate_operation`, `remove_operation`, `move_operation {delta}`, `toggle_operation`;
  - `set_operation_tool {id}`, `recommend_speeds`, `set_rest_source {op}`, `set_pin_setup {setup}`, `set_tab {tab}`;
  - `add_threading`, `add_needed_tool`, `add_hole_stages`, `add_centring`, `add_pin_drilling`;
  - `thread_from_part`, `box_from_face`, `pick_all_corners`.
- **Fields:** `edit_field`, and `undo_last_edit` (`PutBackEdit`).
- **Picks:**
  - `pick_surfaces {ids}` (it turns picking on when it is off), `pick_features {ids}` (the sample's features, `Feature`), `drop_hole {index}`, `clear_picks`;
  - `pick_pin_holes {ids}`, `clear_pin_holes`, `place_pins {x, y}`.
- **Generate:** `generate`, `generate_all`, `cancel_generation`.
- **Libraries:**
  - machines: `new_machine`, `edit_machine {id}`, `machine_wizard_step {index}`, `save_machine`, `delete_machine {id}`;
  - tools: `new_tool`, `edit_tool {id}`, `save_tool`, `delete_tool {id}`, `tool_machine {id, checked}`;
  - materials: `new_material`, `edit_material {id}`, `save_material`, `delete_material {id}`;
  - holders: `new_holder {kind}`, `edit_holder {id}`, `save_holder`, `delete_holder {id}`, `holder_machine {id, checked}`.
- **View:**
  - camera: `set_camera {view}`, `orbit_camera {yaw, pitch}`, `pan_camera {dx, dy}`, `zoom_camera {factor}`;
  - section: `section {action: toggle|axis|at|flip|pick_face, …}`. `pick_face` sends `Viewport::SectionSurface(id)`.
  - stock and paths: `toggle_stock`, `toggle_uncut`, `toggle_paths`, `stock_detail {level}`;
  - playback: `play`, `scrub {at}` (it sends `ScrubDone` after, so the stock catches up), `playback_speed {x}`;
  - panels: `toggle_sidebar {side}`, `sidebar_width {side, px}`, `toggle_sim_details`, `toggle_warning_details`, `toggle_about`;
  - Control page: `control_tab {tab}`, `job_view {view}`.
- **Review:** `open_audit_row {index}`, `open_issue {setup, operation, message}`.
- **Export:**
  - `export_gcode {folder}`: `post::write_job`, then `GcodeExported`;
  - `export_setup_sheet {path}`;
  - `save_for_tablet {path}`: `bundle::write`, then `TabletSaved`;
  - each refuses an existing file unless `overwrite: true`.
- **Machine:** `hold`, `stop`, `estop`, `jog_release`.

## Safety and the user's view

- **Access.** The token and the loopback-only listener keep other users and computers out. The token is compared in constant time.
- **Paths.** Every path argument must be absolute; `~` is expanded. Writes inside the app's data folder are refused, except a headless session's own folder. Writes onto a UC100 profile are refused.
- **Visibility.** In the window, each command the agent runs shows in the status line ("Agent: added Pocket 2 to Setup 1"). The Agents page lists every command.
- **The toggle.** It is saved in the settings. Turned off, it closes the listener at once, refuses requests in flight, and marks the instance file `off`.

## The Agents page (replaces the Planner page)

The top bar's AI entry becomes **Agents**. The page has:
- the toggle;
- whether an agent is connected now, and since when;
- the command to add the server to Claude Code (`claude mcp add axis-cam -- "<path to binary>" --mcp`) and to Codex, each with a copy button;
- the last 200 commands agents ran in this window: time, command, arguments (shortened), outcome and reason.

## Server instructions

`initialize` returns short instructions in the SolidCraft style:
- the workflow: inspect, import, setups, operations, picks, `generate_all`, `check_job`, review, save, export;
- ids and revisions;
- the units;
- fields and picks;
- outcomes;
- checkpoints;
- headless sessions and saving;
- that machine motion and controller state belong to the user.

`guide` holds the machining rules.

## check_job

`check_job` runs the plan checks (`plan_check::run`) on the job the user sees, with nothing filled in:
- A new flag tells `run` to leave a setup without clamp or vise placements as it is. It then reports the missing placement as a problem: "Setup 2 has no clamp positions: use suggest_layout or set_clamps". Today `run` fills such placements even with `planned = false`, and so the check passed on clamps that did not exist (found by the review's probe).
- Chuck grip and stickout are never filled in either.
- The result carries the revision it was checked at. If the job changes while it runs, it is refused as stale.

Suggestions are applied only by `suggest_layout`, which is the user's own command.

## Removing the AI planner

The removal follows the investigation's map, in its own milestone, after the MCP covers the planner's tools.

**Deleted:**
- modules: `planner.rs`, `planner_tools.rs`, `openai.rs`, `oauth.rs`, `session.rs`, `fake_openai.rs`, `plan_changes.rs`;
- in the app: the planner and account `Message` variants, `App` fields, constants and helpers, the planner region of `ui.rs`, and its help entries;
- the CLI flags `--check-plan`, `--part-report`, `--proposal-schema` and `--planner-instructions`;
- the sign-in settings fields;
- the planner smoke tests and screens;
- the planner sections of the README.

**Kept and moved:**
- `analysis.rs` (its ray index and `solid` are core CAM code, and `report` feeds `list_surfaces`, `part_report` and coverage).
- From `plan_check.rs`: `PartGrid`, and `run`, made public for `check_job`. `CheckReport`'s `proposal::Problem` moves into `plan_check.rs`.
- `look.rs`, for `view_model`. `planner::view_after` moves to `simulation.rs`, and `look::tool()` goes.
- From `proposal.rs`: `clip` moves to `model.rs`. Parsing and the planner's schema are deleted. `build`, its types, and the fixtures other tests use become a test-only `fixtures.rs`.
  - Those fixtures are `block`, `good`, `aluminium`, `hold`, `side_up`, `turned`, `with_lathe`, `with_mill`, `milled`, `drilled_by_hand`, `facing_box`, `hand_centre_drill` and `op`, plus `fake_openai::rotary::{report, good}`.
  - They are used by the tests of `post`, `viewport`, `steps`, `guide`, `gpu_mesh`, `chamfer`, `centre`, `rotary_check`, `sheet`, `hole_stages`, `holding` and `cam`.
  - Those tests are kept, so their regression coverage stays.

**Crates:**
- Removed: `reqwest`, `jsonwebtoken`, `getrandom`, the dev `aws-lc-rs`, and the tokio features only sign-in used.
- Kept: `base64` (MCP images), `sha2`, `security-framework` and `futures-util`.
- Then update `THIRD_PARTY.md`, `licenses/`, `NOTICES.md`, `about.toml` and the README's build note.

**Credentials.** On the first start after the removal, both stores of the old sign-in are deleted, best effort, so no token is left behind. These are the macOS Keychain item `chatgpt-session` and the `chatgpt-session.json` file that other platforms used. Old `settings.json` files load unchanged.

**Close flow.** The sign-in waits go; the G-code export wait stays.

**Check.** `cargo build --release`, the app opened, and the full test suite run. `cargo tree` shows no `reqwest` or `jsonwebtoken`.

## Delivery

Four milestones, each with its own Linear issue and PR.

1. **M1 — Headless MCP and the command core.**
   - Server and backend: `axis-cam --mcp`, the headless runner and sessions, discovery of the (later) instance files, `App::pending()`, the revision and ids.
   - Commands: the registry with its compiler-checked classes, outcomes, the machine boundary, the field collector.
   - Tools: `inspect`, `list_surfaces`, `part_report`, `check_job` exactly as it is, `stock_after`, `check_holding`, `cutting_data`, `surface_info`, `view_model`, checkpoints, `guide`, save and export.
2. **M2 — Live window.** Instance files and the loopback bridge, `Message::Mcp`, serialized requests, conflicts, the agent line in the status bar, `screenshot`, and the Agents page with its toggle and log.
3. **M3 — Machine status and stops.** `machine_status` with link freshness and ownership, and `hold`, `stop`, `estop` and `jog_release` with sent and observed results. Tests on the simulator and on a Pi link that is lost.
4. **M4 — Remove the AI planner.**

## Testing

- **Protocol:** `initialize`, `tools/list`, `tools/call`, malformed lines, and a `cancel` read while a generation runs, all over a pipe against the real binary.
- **Coverage:** the exhaustive `class` matches compile. Every command builds its messages, has a valid schema and has help. Every `Edit` key literal is reachable.
- **Outcomes:** a failed save, an invalid library draft, a missing selection, a generation error, a refused sample-only command, and a `no_change`.
- **Headless end to end, on the STEP fixture:**
  - import;
  - a setup on the library CNC with stock;
  - Face, Pocket, Contour and Drill;
  - picks from `list_surfaces`, and fields set;
  - `suggest_layout`, `generate_all` and `check_job`;
  - export G-code to a temp folder and compare it with `--export-gcode` on the saved project;
  - save and re-open;
  - end the client, resume the session, and find the job intact.
- **Lathe and manual mill:** a manual setup and a lathe setup each built through commands only.
- **`check_job`:** a setup with no clamp positions is a problem, not a pass. The probe from the review becomes a test.
- **Live bridge:**
  - an `App` with its listener in a temp directory;
  - a client without a token, and one with a wrong token;
  - with agent access off: an error, never headless;
  - two instances: an error until `select_window`;
  - a user edit between two commands is reported;
  - a checkpoint restore over a user edit is refused;
  - a closed window fails the call.
- **Safety:**
  - every `Machine` class message is refused, on a connected simulated UC100 too;
  - `save_machine` for the connected machine is refused;
  - `edit_field` refuses `uc100_profile`;
  - a save onto a profile path is refused;
  - hold, stop and E-stop act on a running simulated job and report what they observed;
  - while the job runs, a planning command is refused and `machine_status` answers.
- **Removal:** a release build, `cargo tree` without `reqwest` or `jsonwebtoken`, an old `settings.json` opens, and both credential stores are cleared.
- **Live check:** Claude Code with the server added builds a two-setup job for the STEP fixture in the open window from a one-line request, and its G-code passes the NC review page.

## Review history

1. **gpt-6.1-sol, REJECT.** All ten findings were taken:
   - the machine allow-list, now covering library saves, the rotary preset, profile paths, work zero, Wcs and the Run panel;
   - `check_job` no longer fills in placements;
   - coverage enforced by the compiler, button edits, sample features, the view controls and the section face;
   - a responsive headless runner and `App::pending()`;
   - structured outcomes;
   - revisions, ids and conflicts;
   - discovery states;
   - durable sessions and full checkpoints;
   - machine freshness and stop results;
   - the corrections to the removal map.
