# MCP server in place of the AI planner — design

Date: 13 November 2026. Decisions taken with the user in conversation before this spec was written.

## Goal

An AI agent (Claude Code, Codex or any MCP client) can do everything a user can do in AXIS CAM Studio when planning a job. It can import a part, add and edit setups and operations, pick surfaces, use the libraries, generate, check, review the NC output, export and save. It drives the app through an MCP server that the app binary provides. The in-app AI planner (ChatGPT sign-in, Planner page, chat, preview and Apply) is removed.

## Decisions already made by the user

- **Machine access is read-only, except for stopping.** The agent can read the machine's state: connection, phase, position, spindle, run progress and alarms. It can Hold, Stop and E-stop. Everything that moves the machine, turns the spindle or coolant on, or connects, pairs or configures the controller stays a human action in the app.
- **The AI planner is removed** once the MCP server covers what its tools did. This includes the OpenAI client, ChatGPT sign-in, the session store, the Planner page and the planner CLI flags.
- **Live window first, headless otherwise** (the SolidCraft model). When the app's window is open, commands act on that window and the user watches them happen. When it is not, the server runs its own windowless session.

## Non-goals

- Commands that move the machine, energise anything or change the controller. This covers Connect, Disconnect, jog, home, go-to, probe, calibrate, MDI, spindle and coolant on, cycle start and resume, limit override, output tests, Pi pairing and taking control, UC100 profile save, and the `uc100_profile` / `uc100_connection` machine fields.
- An MCP server on the shop tablet build (`SHOP`).
- Remote access. The bridge listens on loopback only.
- A new planning strategy or "auto-plan" button. The agent plans, using the same commands as the user.
- General undo in the app. The MCP offers checkpoints, described below.

## Shape

```
MCP client ──stdio──▶ axis-cam --mcp ──loopback TCP + token──▶ running AXIS CAM window
                            │                                   (Message on the UI thread)
                            └── no window: in-process headless App
```

- `axis-cam --mcp` speaks MCP over stdio: JSON-RPC 2.0, one message per line, protocol version `2025-06-18`. It handles `initialize`, `notifications/initialized`, `ping`, `tools/list` and `tools/call`. It is written by hand on `serde_json`, with no new crate, because only these few methods are needed.
- Every tool call is carried out by one **engine**, `mcp::Engine`, on an `App`. It is the same code whether the `App` is the live window's or a headless one.

### Backends

**Live.** The desktop app opens a loopback listener (`127.0.0.1:0`) when it starts. It writes `mcp.json` (port, a 256-bit random token from two v4 UUIDs, and its pid) beside `workspace.json`, with owner-only permissions, and deletes the file on close. An iced subscription accepts connections. Each request becomes `Message::Mcp(id, Request)`, is handled on the UI thread by the engine, and is answered through a oneshot channel. A connection that does not send the token first is closed.

**Headless.** With no reachable live app, the server builds `App::from_workspace` on a private copy of the user's `workspace.json` in a new temp directory, so it has the user's libraries. Generation runs inline (`generate_inline`). Tasks are run to completion with `iced_runtime::task::into_stream` on a current-thread tokio runtime. Their output messages are fed back to `update`, and window and widget actions are dropped. The headless session never writes the user's `workspace.json`. Its work leaves the session only through `save_project` (and the export commands) to a path the agent names.

**Choosing.** The backend is chosen at the first tool call. If a live app appears later, the next call moves to it only while the headless session is still unchanged, as SolidCraft does. Otherwise the server stays headless and every reply names the backend (`"backend": "live"|"headless"`). If the live app goes away mid-session, calls fail with "The AXIS CAM window closed" and do not silently start a headless session.

### Waiting for work

A command returns when the app is **settled**: no generation (`generating`), import or save (`import_busy`, `save_busy`), G-code export, post check, layout, left-stock or guide job is pending for the job as it now is. The wait is at most 120 s by default, and a call may raise it to 600 s. Past the limit, the reply says `still_running` with what is pending, and `wait {}` waits again. The work is not cancelled; `cancel {}` sends `CancelGenerate`.

In live mode, the engine polls these flags on `Tick`, which runs fast while an MCP request is pending.

## Tools

The surface is small and generic, so it covers every action without about 200 hand-written tools.

| Tool | Does |
| --- | --- |
| `list_commands {filter?, group?}` | Name, group and one-line help of each command. Groups: project, part, setup, operation, picks, generate, libraries, steps, view, review, export, machine. |
| `describe_command {name}` | Full help (from `help::command`), its JSON schema, what it needs selected, and whether it waits for work. |
| `execute {command, args, wait_s?}` | Runs one command, waits until settled, and replies with the result (below). |
| `batch {steps: [{command, args}], stop_on_error: true}` | Runs commands in order and replies with each result. Unlike SolidCraft, it is not all-or-nothing; take a checkpoint first to go back. |
| `inspect {what?}` | The state as the user would read it. Project: title, file, material, dirty. Setups: machine, stock, rotation, holding, work zero. Operations: strategy, tool, picks, key fields, generated or not. Selection, page, status line, problems, busy flags. `what` narrows it to a setup, an operation, the libraries or the page. |
| `list_fields {target}` | Every field the user can edit for the selected operation, the active setup or a library draft. For each: key, label, tab, current value, unit, allowed values and the help summary. |
| `list_surfaces {setup?}` | The part's surfaces: id, normal, centre, area and smooth region. Includes the part report's planes (top or floor, depth, largest cutter that fits, holes), seen from the setup's orientation. |
| `part_report {}` | `analysis::report` for the current part and setup 1. |
| `check_job {}` | The plan checks on the live job (`plan_check::run`, not planned): passed, problems by setup and operation, minutes, coverage, and stock before each setup. |
| `stock_after {setup}` / `check_holding {setup}` / `cutting_data {tool, op?}` / `surface_info {id}` | The planner tools, rewritten to read the live job instead of a proposal. |
| `view_model {view, …}` | `look::draw`, returned as an MCP image. The same views as the planner offered: an orientation, a named camera, a section, or the stock after an operation. |
| `screenshot {}` | Live: the window, from iced's window screenshot. Headless: refused with a pointer to `view_model`. |
| `checkpoint {name}` / `restore_checkpoint {name}` / `list_checkpoints {}` | Snapshots of the `Document` and the selection, kept in the session. Restoring goes through `replace_document`, as opening a project does. |
| `machine_status {}` | Read-only machine state: chosen machine, connection kind, connected, phase, machine and work position, work offset, spindle, coolant, feed override, run setup, step and progress, alarms, and limit switch states. |
| `wait {wait_s?}` / `cancel {}` | See *Waiting for work*. |

### Result of `execute`

```
{ "ok": bool,
  "backend": "live"|"headless",
  "status": "<the status line after the command>",
  "problem": bool,                // status_problem
  "changed": ["project", "setup 2", "operation 2.3", "page"],
  "selection": {"setup": 2, "operation": 3, "page": "Work", "tab": "Geometry"},
  "pending": [],                  // empty when settled
  "notes": ["…"] }
```

`ok` is false when the engine refused the command (unknown name, bad arguments, nothing selected, a machine command, the app is locked by a run), or when the app refused it. The app's refusals appear as a status line that the message set and marked as a problem, or as a refused click (the selection or job unchanged with a refusal status). The reason is the app's own sentence.

## Commands

### Registry

`mcp::commands` holds one table: name → group, help key, argument schema, safety class, and a builder that maps the arguments and the `App`'s state to one or more `Message`s. The builder can also do the work of a file dialog (below). Help text is taken from `help::command` / `machine_help` / `run_help`, so the agent reads what a hovering user reads.

A unit test lists every `Message` variant (and the user-action variants of `GuideAction`, `setup::Msg`, `section::Edit`, `control::Action` and `run::Action`) and requires each to be one of these:
- reachable from a command;
- classified `machine` (refused);
- `internal` (results, ticks, window events, keys, hovers);
- `planner` (until it is removed).

So a new user action cannot be added without deciding its MCP form.

### The commands

The names below are the command names. Arguments are JSON. Values that were `&'static str` in a message are enums in the schema.

- **Project:** `new_project`, `open_sample`, `resume_project`, `recover_previous_project`, `open_project {path}`, `save_project {path?}` (a path the first time, then like ⌘S), `set_project_title {title}`, `set_project_material {id}`, `goto_page {page}`.
- **Part:** `import_model {path}` (sends `ImportPath` and then `ImportFromPath`), `reimport_model`, `dismiss_pick_note`.
- **Setup:** `select_setup {index}`, `add_setup {machine?}` (`AddSetupOn`, or `AddSetup` without a machine), `add_manual_setup`, `add_lathe_setup`, `duplicate_setup`, `remove_setup` (sends `RemoveSetup` and then `ConfirmDeleteSetup(true)`), `move_setup {delta}`, `flip_setup {axis: rx|ry}`, `turn_stock {face, table_turn}`, `square_to_stock`, `assign_machine {id}`, `set_setup_step {step}`, `set_clamps {text}`, `suggest_layout`, `show_stock_after_setup`, `open_setup_editor`.
- **Operation:** `select_operation {index}`, `add_operation {strategy}`, `add_turn {kind}`, `duplicate_operation`, `remove_operation` (no confirmation step), `move_operation {delta}`, `toggle_operation`, `set_operation_tool {id}`, `recommend_speeds`, `set_rest_source {op}`, `set_tab {tab}`, `add_threading`, `add_needed_tool`, `add_hole_stages`, `add_centring`, `add_pin_drilling`, `thread_from_part`, `box_from_face`, `pick_all_corners`.
- **Fields:** `edit_field {target, key, value}`, where `target` is operation, setup, machine, tool, material or holder. The key must be one that `list_fields` lists for that target as it now is. Other keys are refused with the list, so a wrong key never enters the project, which `Edit` alone would allow. `undo_last_edit` sends `PutBackEdit`.
- **Picks:** `pick_surfaces {ids: [..]}` turns picking on when it is off and sends `Surface(id)` for each id. `drop_hole {index}`, `clear_picks`, `pick_pin_holes {ids}`, `place_pins {x, y}`.
- **Generate:** `generate`, `generate_all`, `cancel_generation`.
- **Libraries:** `new_machine`, `add_rotary_machine`, `edit_machine {id}`, `machine_wizard_step {index}`, `save_machine`, `delete_machine {id}`, `new_tool`, `edit_tool {id}`, `save_tool`, `delete_tool {id}`, `tool_machine {id, checked}`, `new_material`, `edit_material {id}`, `save_material`, `delete_material {id}`, `new_holder {kind}`, `edit_holder {id}`, `save_holder`, `delete_holder {id}`, `holder_machine {id, checked}`. Library drafts are edited with `edit_field`.
- **Manual mill and lathe steps:** `steps_open {setup}`, `steps_next`, `steps_back`, `steps_go {index}`, `steps_section {index}`, `steps_reached {index}`, `steps_resume`, `steps_fix {op?, problem}`, `steps_measure {key, mm}`, `lathe_cross_reads {reads}`. They change a page, not a machine.
- **View:** `set_camera {view}`, `section {action, …}`, `toggle_stock`, `toggle_uncut`, `toggle_paths`, `play`, `scrub {at}`, `playback_speed {x}`, `stock_detail {level}`, `control_tab {tab}`, `job_view {view}`.
- **Review:** `open_audit_row {index}`, `open_issue {setup, operation, message}`, `run_on_machine {setup}`. The last only picks the setup in the Run panel on the MACHINE page; it never connects, because Connect is a human action.
- **Export:**
  - `export_gcode {folder}`: `post::write_job` of the job, as `ExportGcode` does after its dialog, then `GcodeExported`.
  - `export_setup_sheet {path}`.
  - `save_for_tablet {path}`: `bundle::write`, then `TabletSaved`.
  - Each refuses a path that exists unless `overwrite: true`.
- **Machine:** `hold` (`Run(Hold)` while a run is running), `stop` (`Run(Stop)`), `estop` (`MachineEStop`), `jog_release`.

### Refused commands

These are listed by `list_commands` with group `machine` and the reason, and refused by `execute`. That way an agent learns they exist and that they are the user's to do:
- Connect, Disconnect, machine reset, connection kind, the Pi address, code, pair, forget and take control;
- every `control::Action` that moves or energises;
- `Run` CycleStart, Resume, Setup and the probes;
- UC100 profile edits and Save.

The reply is: "This moves or configures the machine, so it is done in the app by the user."

### Fields

The inspector's field helpers in `ui.rs` (`field`, `checked_field`, `hinted_field`, `choice`, `choice_value` and the three pick lists) record a `FieldInfo` when a collector is on: target, key, label, tab, current value, choices and unit (from the label). `list_fields` turns the collector on and builds the selected target's editor once per tab, without drawing. A test checks that every key `help::field_keys()` knows is listed by some page in the sample and fixture projects. A field the user cannot see is not offered, because the agent can only do what the user can.

## Safety

- The token file and loopback listener keep other users and machines out. The token is compared in constant time.
- Machine commands are refused in the engine, not in the client.
- While the app is `locked()` (a run, jog or probe is in progress), the app already drops most messages. The engine reports "The machine is running; only hold, stop and E-stop are accepted" instead of sending them.
- In live mode, the status line shows each command the agent ran ("Agent: added Pocket 2"), so the user always sees what changed.
- A toggle on the Agents page turns the bridge off. It is saved in the settings, and when it is off the window opens no listener.
- Paths: `import_model`, `open_project`, `save_project` and the exports accept absolute paths only. They expand `~`, and refuse to write inside the app's own data folder.

## The Agents page (replaces the Planner page)

The top bar's AI entry becomes **Agents**:
- whether the bridge is on, with the toggle;
- the command to add the server to Claude Code (`claude mcp add axis-cam -- "<path to binary>" --mcp`) and to Codex, each with a copy button;
- whether an agent is connected now;
- the last 200 commands agents ran in this window, with time, command, arguments (shortened) and result.

## Server instructions

`initialize` returns instructions in the SolidCraft style:
- the workflow: inspect, import, plan setups, add operations, pick, generate all, `check_job`, review, export;
- the units;
- how picks and fields work;
- the checkpoints;
- that machine motion belongs to the user.

The machining rules in the planner's instructions (stepdowns, cutter choice, holding, chip load, the minimum rpm) move into a `guide {topic}` tool, so they are kept and not sent with every session.

## Removing the AI planner

The removal follows the map in the investigation, in its own milestone, after the MCP covers the planner's tools.

- **Deleted:**
  - modules: `planner.rs`, `planner_tools.rs`, `openai.rs`, `oauth.rs`, `session.rs`, `fake_openai.rs`, `plan_changes.rs`;
  - the planner and account `Message` variants, the `App` fields, constants and helpers, the planner region of `ui.rs`, and the planner help entries;
  - the CLI flags `--check-plan`, `--part-report`, `--proposal-schema` and `--planner-instructions`;
  - the sign-in settings fields;
  - the planner smoke tests and screens;
  - the README sections.
- **Kept:**
  - `analysis.rs` (its ray index and `solid` are core CAM code, and `report` feeds `list_surfaces`, `part_report` and coverage);
  - `plan_check.rs` (`PartGrid`, and `run` made public as `check_job`);
  - `look.rs` (`view_model`). `planner::view_after` moves to `simulation.rs`, and `look::tool()` goes.
- **`proposal.rs`:** `clip` moves to `model.rs`. Proposal parsing and building are deleted. The job builders that about 25 other tests use become a test-only `fixtures.rs` with `build` and the fixtures those tests use.
- **Crates:** `reqwest`, `jsonwebtoken`, the direct `base64`, `getrandom`, the dev `aws-lc-rs`, and the tokio features only sign-in used go. `sha2`, `security-framework` and `futures-util` stay because other code uses them. Then update `THIRD_PARTY.md`, `licenses/`, `NOTICES.md` and the README's build note.
- **Settings:** old `settings.json` files load unchanged, because the fields are `serde(default)` and unknown ones are ignored. The Keychain item `chatgpt-session` is deleted once on first start, best effort, so no token is left behind.
- **Close flow:** the sign-in waits go. The G-code export wait stays.

## Delivery

Four milestones, each with its own Linear issue and PR.

1. **M1 — Headless MCP.** `axis-cam --mcp` with the headless backend, the command registry and its coverage test, the field collector, `inspect`, `list_surfaces`, `part_report`, `check_job`, `stock_after`, `check_holding`, `cutting_data`, `surface_info`, `view_model`, checkpoints, `guide`, waiting, export and save.
2. **M2 — Live window.** The loopback bridge and token file, `Message::Mcp`, the agent line in the status bar, `screenshot`, and the Agents page with its toggle and log.
3. **M3 — Machine status and stops.** `machine_status`, `hold`, `stop`, `estop`, `jog_release`, and the refusal of every other machine command, with tests on the simulated UC100.
4. **M4 — Remove the AI planner.**

## Testing

- **Protocol:** `initialize`, `tools/list`, `tools/call` and errors on malformed lines, run over a pipe against the real binary (`--mcp`).
- **Registry coverage:** every user action maps to a command or is classified. Every command's schema is valid JSON Schema, and every command has help.
- **Headless end to end, on the STEP fixture:**
  - import it;
  - add a setup on the library CNC with stock;
  - add Face, Pocket, Contour and Drill;
  - pick surfaces from `list_surfaces`;
  - set fields;
  - run `generate_all` and `check_job`;
  - export G-code to a temp folder and compare the result with `--export-gcode` on the saved project;
  - save and re-open.
- **Lathe and manual mill:** a manual setup and a lathe setup each built only through commands, and the steps page walked.
- **Fields:** an unknown key is refused. A choice value outside its list is refused. Every `help::field_keys()` key is reachable.
- **Live bridge:** an `App` with its listener in a temp directory, and a client with and without the token. A request is answered after settling. A closed window fails the call.
- **Safety:**
  - every machine command is refused on a connected simulated UC100;
  - hold, stop and E-stop act on a running simulated job;
  - while it runs, a planning command is refused with the machine message.
- **Removal:** `cargo tree` has no `reqwest` or `jsonwebtoken`. The app opens a workspace whose `settings.json` has the old fields.
- **Live check:** Claude Code with the server added builds a two-setup job for the STEP fixture in the open window from a one-line request, and its G-code passes the NC review page.
