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
- **One writing agent per window.** The first MCP session to send a mutating command holds the window's **write lease** until it disconnects; one headless session has only one client anyway.
  - Other sessions connected to the same window may read and stop the machine. Their mutations are refused: "Another agent is working in this window (<client name>, since <time>)."
  - The Agents page shows who holds the lease and can take it back.
  - When the lease changes hands, the checkpoints of the previous holder are dropped, so one agent can never restore over another's work.
- **One mutation at a time.** Mutations are never queued. A mutating command that arrives while an earlier one's work has not settled is refused at once with outcome `busy`: "Nothing was done; the previous command (<name>, <command_id>) is still running. Call wait, then retry." Read-only tools and the stop commands always go ahead.

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
- **What is saved.** `persist` saves only the `Document`, so the session also writes `session.json`, which holds everything else a resume needs:
  - the open library drafts;
  - the selection, the page and the tab;
  - the revision and the change log;
  - the revision last reported to the agent;
  - the project path;
  - the paths of the last saves and exports;
  - the checkpoint index.

  It is written atomically together with the `Document`, before the result of each command that changed anything is sent. Whatever a result reported is on disk.
- **One process at a time.** A session folder holds a lock file, `lock`, created exclusively, with the owner's pid and start time. A second `resume_session` of the same id is refused while the owner is alive: "Session <id> is open in another MCP server (pid …)". A lock whose pid is dead is taken over.
- **Reporting.** `inspect` and every result report the session id, its folder, whether the job is saved to a project file, and the paths of the last saves and exports.
- **Resuming.** On stdin EOF the session is left on disk. `resume_session {id}` (or `--mcp --session <id>`) continues it. `list_sessions` lists them with their title, last change, whether the job is saved to a project file, and its size.
- **Deleting.** Sessions are never deleted automatically, because a session may hold the only copy of a job. `delete_session {id}` deletes one.
  - It is refused for a session that is open (its lock is held).
  - It is refused for a session with work not saved to a project file, unless it is called with `discard_unsaved: true`.
  - The Agents page lists the sessions with a Delete button that asks the same.
  - Test: an unsaved job with an open tool draft survives a restart with the clock moved 31 days on.
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

**Every tool call answers within a bounded time**, reads and batches included. That time is its `wait_s`: 25 s by default, at most 50 s. Clients time out on their own, Codex at 60 s by default, so the server answers before them.

- **Long work.** When a call runs out of time, its outcome is `still_running`. Work that was dispatched gets a `command_id`. A costly read is the same: it keeps running and gets an id.
- **Waiting.** `wait {command_id, wait_s?}` waits again. When the work has finished, it returns the command's final result: `done`, `failed` or `cancelled`, with the same fields as `execute`. Results are kept until the session ends, so a result whose client timed out can still be read.
- **Batches.** A batch stops dispatching new steps once its time is up. Its result lists each step's outcome, steps after that point are "not run", and the step still working has its `command_id`.
- **`still_running` means started.** It is used only for work that was dispatched. A command that never started is `busy` or `refused`, and its reason says nothing was done.
- **`cancel {command_id}`** cancels a generation or a costly read, and its final result is `cancelled`.
- **`initialize`** answers at once, even while a resumed session is still restoring its model. The first command then simply waits for that work.

Tests use a deliberately slow generation, a slow `check_job` and a slow batch through the copied install commands with each client's default settings. Every call must answer before the client's timeout, and every `wait` must return the final result.

In the window, a pending MCP request makes `Tick` run fast. The engine answers from the UI thread only once `pending()` is empty.

## Identity, revisions and concurrency

### The work state and its snapshots

The **work state** is everything the user could lose or see change:
- the `Document`: job, libraries, the guide's saved place, measurements;
- every editable draft: `machine_draft`, `tool_draft`, `material_draft`, `holder_draft`, `clamps_draft` and `guide_typed`;
- the project file path;
- the loaded model's fingerprint.

`profile_draft` and `pi_code` are the controller's and are not tracked, because the agent cannot touch them.

A unit test lists the fields of `App`. Each must be named in `mcp::WORK_STATE` or in `mcp::NOT_WORK_STATE` with a reason, so a new draft cannot be missed.

A **snapshot** of the work state is a hash of its saved serialization, written straight into the hasher with no string built. Next to the hash it keeps a per-item hash for each setup, operation, library entry and draft, so the change log can say what changed.

Snapshots are taken only at the edges of agent work, never per message (see *Ownership windows*). The UI thread therefore pays for a few snapshots per agent command, and nothing while no agent is working. A difference between two snapshots catches every write, whatever path made it: `Tick` writing pin references or rotary measurements, `Guide(Built)`, a draft keystroke, `NewProject`. Who made it is decided separately, from recorded events.

**Budget, checked by a release-build benchmark (`cargo test --release -- --ignored mcp_snapshot_bench`, run before each MCP milestone's PR):**
- a full snapshot of a 1 MB project takes at most 15 ms;
- one at the 20 MB project limit takes at most 250 ms.

Every snapshot hashes everything, with no incremental shortcut: setups and operations have no version, and controller writes change a machine without changing its `version`. If the benchmark misses the budget, M1 does not merge until this design is revised.

### Ownership windows

- **An agent command owns a window.** It opens when the engine takes the "before" snapshot and dispatches the command's messages, in one UI-thread turn. It closes only when everything the command started has settled (`pending()` is empty), and then the "after" snapshot is taken.
- **The window outlives the result.** If the command returns `still_running`, its window stays open: through `wait`, through read-only tools, and through a client reconnect. A new mutating command that arrives while the window is open is refused at once with outcome `busy`; nothing of it is dispatched. Read-only tools never open or close a window.
- **Outside windows, no agent work runs.** So any difference found when a window opens, compared with the last window's close, is not the agent's. This also covers edits made while no agent was connected: the last snapshot is kept with the `App` across disconnects, and in `session.json` headless.

### External events

A change is attributed to the user from **events recorded where they happen**, not from the class of the message that carried them. An external event is any of these:
- **A message from the window's own event handlers** (not from the engine) that is not view-only. It is counted on arrival, whatever its class.
- **A controller result written into the work state.** The agent has no machine commands, so every such write is the user's work at the machine. The sites that apply queued controller results call `App::external(what)`. These are `Tick`'s `record_pins` and `keep_rotary` (`main.rs` ~9335), `SaveCentreline`, `UseCalibration`, and every other place that takes a result out of `self.control` into the `Document` or a library.

  A test runs each such action on the simulated UC100, queues its result before an agent request, applies it during the request's window, and checks it is reported as the user's and protected from restore.
- **The completion of async work started by an external event**, matched by the epoch recorded when it started.

### Revisions and sources

- **Revision.** `revision: u64` goes up by one each time a snapshot differs from the one before it. It never goes down; a checkpoint restore also goes up. Each step appends a change-log entry with the revision, the items that differ, and a source:
  - **`agent`:** the difference was found when an ownership window closed, and no external event happened during the window;
  - **`mixed`:** found when a window closed, with external events during it. The result names them, for example "user: edit_field operation feed; machine: pin reference recorded";
  - **`user`:** found when a window opened, so it is everything since the last window closed.
- **Addressing.** Commands address setups and operations by their stable `id`. An index is accepted and turned into the id when the request arrives. A command that acts on "the selected" operation selects it by id in the same UI-thread turn that acts.
- **Reporting.** Every result lists the `user` and `mixed` entries since the revision last reported to the agent. A read-only tool takes a snapshot only when no window is open, and reports a `user` entry if one appears. A wait cannot absorb the user's work.
- **Async results.** A completion for work an agent command started is applied only if its epoch is still current. That is the app's rule for generations today, and it is extended to the layout, import and export results. The expected effect is then checked on the state as it actually is.
- **Conflict check.** A mutating call may pass `if_revision`. A window-open snapshot is taken first, so a user change since the last request counts. If the revision moved, the call is refused with outcome `conflict` and the entries.
- **Checkpoints and the user's edits.** `restore_checkpoint` is refused when a `user` or `mixed` entry follows the checkpoint, unless it is called with `discard_user_changes: true`. In the window, that also asks the user in a dialog, which they can decline. The final check is made in the turn that commits (see *Checkpoints*).

### Shop-floor state is protected

The guide's saved place (`Document::guide`), the measurements (`Document::measured`) and the cross-slide dial readings belong to the machinist and to the project they were recorded in.

- **Commands that replace the project** load the incoming project's records, as they do for the user: `new_project`, `open_sample`, `open_project`, `resume_project` and `recover_previous_project`. `NewProject` archives the outgoing project first, as it does today.

  For the agent, each of these is refused while the current project has unsaved changes, unless it is called with `discard_unsaved: true`. In the window, that also asks the user, as the app's own close does. So the agent cannot drop records the machinist made and never saved.
- **Every other allowed command** is guarded on the setups that exist both before and after it. Their records must not change during its window. Removing a setup removes its records, as it does for the user.
  - If a record changed and the window had **no** external event, the agent's path changed it. That is a bug in the boundary. The changed entries are put back to their values before the window, and the outcome is `failed`: "This would have changed the machinist's progress". A test checks that no allowed command triggers this.
  - If the window **had** external events (the machinist pressed Next or measured a diameter during the agent's wait), nothing is put back. The change is reported as theirs.
- **Tests:**
  - `new_project` and `open_project` with populated records, clean and dirty;
  - a human `Next` and `Measured` during an agent's pending `generate_all`, kept and reported;
  - `remove_setup` and `duplicate_setup` with records on other setups.

### Tests

- Each of these is reported to the agent, with the source that made it:
  - a user tool-draft edit;
  - a half-typed clamp positions field;
  - `NewProject`;
  - opening a project whose setups keep the same ids;
  - a pin reference recorded through `Tick`, both outside a window and queued before an agent request;
  - an edit made while no agent was connected.
- A user edit during an agent's `generate_all` makes its entry `mixed` and names the edit.
- An agent's `suggest_layout` that returns `still_running`, then finishes while the agent calls `inspect` and `wait`, is one `agent` entry. Restoring the checkpoint before it needs no `discard_user_changes`.
- A stale generation result is not applied after a user edit.
- An `if_revision` older than a user change gets `conflict`.
- The `App` field test fails when a field is in neither list.
- The release benchmark meets its budget on a 20 MB project.

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
| `stock_after {setup}` / `cutting_data {tool, op?}` / `surface_info {id}` | The planner tools, reading the live job. |
| `check_holding {setup}` | The saved holder and placement of the setup, checked **as they are**. For a vise that is its offset, axis and grip; for clamps, their positions; for a lathe, its chuck, grip and stickout. Nothing is substituted. Today's planner tool puts a suggested placement in when the saved one is not usable: the review's probe had a vise saved 10 000 mm off pass as `holds: true` on a 0 mm suggestion. A missing or invalid placement is reported as such. Suggestions come only from `suggest_layout`, which changes the job the user can see. Tests: that misplaced vise fails, a setup with no clamps fails, and a good placement passes, on a mill setup and a lathe setup. |
| `view_model {view, …}` | `look::draw`, as an MCP image (PNG, base64). |
| `screenshot {}` | Live: the window. Headless: refused, with a pointer to `view_model`. |
| `checkpoint {name}` / `restore_checkpoint {name, discard_user_changes?}` / `list_checkpoints {}` | See *Checkpoints*. |
| `machine_status {}` | See *Machine status*. |
| `wait {command_id, wait_s?}` / `cancel {command_id}` | See *Waiting for work*. |
| `list_sessions {}` / `delete_session {id, discard_unsaved?}` | See *Headless sessions*. |
| `select_window {pid}` / `list_sessions {}` / `resume_session {id}` | Discovery. |
| `guide {topic}` | The machining rules the planner's instructions held: stepdowns, cutter choice, holding, chip load and minimum rpm. |

Read-only tools work on an owned **`JobSnapshot`**: a clone of the `Document`, the `Arc` of the mesh, the selection and the revision.
- It is `Send`, which a compile-time assertion checks. `App` itself is not `Send` on macOS, because the controller's `ActivityToken` is held in it, and it never leaves the thread that owns it.
- In the window, the costly reads (`check_job`, `check_holding`, `part_report`, `view_model`, and `list_surfaces` on a large part) take a snapshot on the UI thread and run on it in a `Task`.
- **Freshness is checked by fingerprint, not by revision.** A revision moves only at an ownership window's edges, so it can stay the same while an agent's work is still settling.
  - A read takes the work-state fingerprint when it takes its snapshot, and again when its result is ready, both on the `App`'s thread. This does not touch the change log or the windows.
  - If the two differ, the result says `"stale": true` and what changed. `check_job` is then refused as stale rather than returned.
  - Test: `generate_all` returns `still_running`; `check_job` starts; the user changes a clamp position; the check returns while the window is still open, and it is refused as stale.

### Outcomes

Every `execute` result:

```
{ "outcome": "done" | "no_change" | "refused" | "busy" | "failed" | "still_running" | "cancelled" | "conflict",
  "command_id": "…",                 // for still_running, and when wait returns the final result
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
  - `save_tool`, `save_material`, `save_holder` and `save_machine`. A library save changes the draft on its way into the library. Material and holder names are trimmed. A machine save settles a hand machine (`manual::settle`), adds one to `version` when the machine already exists, and drops the centre-line marks (`CENTRE_MARK`, `CENTRE_REF`) when `a_centre_y` or `a_centre_z` changed. So these normalisations move out of the four handlers into pure functions, `Document::saved_tool`, `saved_material`, `saved_holder` and `saved_machine` (`(&self, draft) -> Result<Entry, Vec<String>>`), which the handlers call from now on.
    - Before sending, the engine computes the expected entry with the same function. An `Err` is `refused`, with the issues.
    - If the library already holds an entry equal to it, ignoring `version`, the save is not sent, and the outcome is `no_change`. A machine's version is then not raised for nothing.
    - After sending, `done` means the library holds exactly the expected entry, and the editor stays open, as it does for the user.
    - The connected machine's save, which also writes the controller's travel, is refused before any of this (see *The machine boundary*).
    - Tests: a successful save of each kind, an unchanged save (`no_change`, version unchanged), an invalid draft (`refused`), and a machine save that drops the centre marks.
  - `save_project`: the file is written, and the project is not dirty.
  - `generate`: a simulation for the operation at this revision, or a `GenerationFailed` reason.
  - `select_operation`: the selection is that id.

Tests cover a failed save (path not writable), an invalid library draft, a selection that does not exist, a generation error, and a refused click (a sample-only command on an imported part).

### Checkpoints

A checkpoint belongs to one project: the project the app had when it was taken.
- Its **project identity** is a new `Document::project_id`, a UUID given to a project when it is created and kept in its file. Old files get one when they load.
- `restore_checkpoint` is refused if the change log has a project replacement after the checkpoint (`new_project`, `open_sample`, `open_project`, `resume_project`, `recover_previous_project`, the user's own New or Open), or if the current `project_id` is not the checkpoint's: "Another project was opened or started since this checkpoint, so it can no longer be restored. Take a new checkpoint in the project you are working on."
- So a restore always keeps the **current** project's step and measurement records, and those are the same project's.
- Test: save A with guide step 7 and a measurement, checkpoint, `new_project`, restore. The restore is refused, and the new project is unchanged. Open A again (a project replacement), and the restore is still refused. Take a new checkpoint to go back within it.

A checkpoint holds:
- the work state: the `Document`, every tracked draft, and the project file path;
- the selection, the page and the tab;
- the model file it was taken with (`ModelFile`, with its fingerprint);
- the revision it was taken at.

It is stored in the session folder (headless) or in memory (live, at most 20).

`App::restore_checkpoint` validates first and commits once. A refused restore leaves the current document, mesh, drafts and revision exactly as they were.

1. **Validate, changing nothing.**
   - If the checkpoint's model fingerprint is not the loaded mesh's, the model file is loaded into a **candidate** mesh off the UI thread (`mesh::load`, as `restore_model` does, but not into `imported_mesh`). If the file is missing or unreadable, or its fingerprint is not the checkpoint's, the restore is refused, for example: "The model file changed since the checkpoint; restoring it would put picks on a different part." The current mesh is never cleared first. Today's `restore_model` clears it before its load.
2. **Check again, then commit, in one UI-thread turn.** The candidate's load takes time, and the user may act meanwhile; a draft edit, for example, moves no epoch. So in the turn that commits, before anything changes:
   - A snapshot is taken. If it differs from the one taken when the restore began, and an external event happened since, the restore is refused unless it has `discard_user_changes: true`. The change is reported.
   - If `locked()`, because a run, jog or probe started, it is refused: "The machine is running; the job cannot change until it stops."

   Then it commits, through a new `App::job_replaced(doc, mesh)`, which `replace_document` also uses from now on, so the two cannot drift. In order:
   1. Every piece of derived state is cleared, and the epochs of outstanding work are moved on, so a late result for the old state is dropped:
      - the simulation, playback position and replay;
      - the audit report;
      - the post check (NC review) results;
      - the left stock and holding checks;
      - the run check and prepared run programs (marked stale, as an edit marks them);
      - the guide build, part holes, start and stock meshes, and tooth load;
      - the generation and layout.
   2. The `Document` is set, but with the **current** guide place, measurements and dial readings, whatever the checkpoint holds (see *Shop-floor state is protected*). The drafts, selection, page, tab and project path are set, and the candidate mesh if there is one. Unlike `replace_document` today, the drafts and the path are not reset.
   3. **Saved or not** is worked out after this, never restored. Both are compared as `serde_json::Value`: the restored `Document` as `storage::save` would serialize it, and the project file on disk loaded with `storage::load` and serialized the same way. If they are equal, the project is saved; otherwise it is dirty. With no project file, it is dirty.
   4. The revision goes up by one (source `agent`, "checkpoint restored"). An `if_revision` from before the restore is therefore stale.
3. The derived state is rebuilt as after opening a project, and the command waits for it (`pending()`).

**Tests:**
- Save A, checkpoint, edit and save B, restore. The result shows A dirty, because the disk holds B. It has a new revision, and an old `if_revision` gets `conflict`.
- Checkpoint A, edit B on the same model and ids, run `generate_all` and the NC review, restore. No B simulation, audit row, NC result or run program is shown; A's are generated afresh.
- A late generation result for B is dropped.
- After a reimport between checkpoint and restore, the picks and the rendered part are A's.
- A restore whose model file was deleted or changed is refused, with the document, mesh and revision unchanged.
- With a slow candidate load: a user tool-draft edit during the load refuses the restore and keeps the edit; a simulated run started during the load refuses it too.

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
- **Refused, the run:** every `run::Action` except `Hold`, `Stop`, and the two that only fold the Run panel's rows open or shut, `More` and `Steps` (`run_details`). That includes `Setup` (and so `RunOnMachine`), `Check`, `PickStep`, `Next`, `From`, `Skip`, `SkipSteps`, `StartStep`, `StopAfterStep`, `KeepZero`, `KeepToolZ`, `ClearSteps`, `CycleStart`, `Resume`, the reference probes and `CoolantOff`.
- **The manual mill and lathe Steps (`GuideAction`), sorted by what each does:**
  - **Allowed, navigation that records nothing.** `Fix(op, problem)` opens the operation or setup editor at the problem; it is the `steps_fix` command.
  - **Refused, because it moves the place.** `Open(setup)` enters the steps at the selected operation (`enter_steps` → `apply_steps_entry` → `guide_to`). So does going to the Control page with a manual setup active. For the agent, `goto_page` refuses `Control`; it reads the machine with `machine_status` and the steps with `inspect`.
  - **Allowed, browsing.** `inspect {what: "steps", setup, step?}` reads any step of a setup's guide: its text, readouts, holes and pictures. It has a cursor of its own: it builds the guide (`guide::build`) on a `JobSnapshot` off the UI thread and reads the step asked for. It never calls `guide_to` and never touches the `App`'s guide state. It works in both backends.
    - Tests read a step with a cached guide and with one not yet built, with an earlier operation selected. `Document::guide` is unchanged afterwards.
  - **Refused, shop-floor progress.** `Next`, `Back`, `Go`, `Section`, `Resume` and `Reached` move the saved place (`guide_to` writes `Document::guide`). `Measured`, `Typed` and `CrossReads` record calipers and the dial. They are the machinist's record of what was done at the machine, so they stay human.
  - **Internal.** `Built` and `Beat`.
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

While the app is `locked()`, which means a run, jog, probe or homing is in progress, mutating commands are refused with "The machine is running; the job cannot change until it stops." `machine_status`, the four stop commands, and the **light** reads still work.

**Light and heavy reads.** The app keeps generation and simulation away from the motion driver while it runs. `App::planning_busy()` refuses Cycle Start while such work is under way, and the work is not started while the machine moves (`main.rs` ~3332). Reads that compute toolpaths or stock follow the same rule.
- **Heavy reads:** `check_job`, `check_holding`, `stock_after`, `view_model`'s views of the stock after an operation, and `inspect {what: "steps"}` when the guide it reads has to be built. The manual mill and lathe guides simulate the operations to replay the stock (`guide.rs` `simulate`, ~1319 and ~1555). Reading a guide the app has already built for the current job (`guide_key`) is light. While the machine is `locked()`, they are refused with "The machine is running; this check simulates toolpaths, so it waits until the run stops."
- **Counting heavy work.** Every heavy read's worker is counted in `planning_busy()` from when it starts until its thread has actually finished. It is not released when the read is cancelled, timed out or its client disconnects, as the planner's `plan_busy` does today. M4 keeps this count when the planner goes.
- **Light reads:** everything else, which reads state or draws the part only. They run at any time.
- **Tests:**
  - a heavy read started before Cycle Start makes Cycle Start wait, and that holds after the read is cancelled while its worker still runs;
  - a heavy read asked for during a run is refused;
  - the same three cases for `inspect {what: "steps"}` on a manual mill and a lathe setup whose guide is not built; a guide already built is read during a run;
  - `inspect` and `machine_status` answer during a run.

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

`list_fields` turns the collector on, a thread-local, and builds the target's editor `Element` once per tab. The helpers record a field while the element tree is being constructed. No layout, renderer or `ui::probe` is needed: probe and its renderer crates stay test-only.

Building an editor takes `&App`, so it runs on the thread that owns the `App`: the UI thread in the window, the app thread when headless. It takes a few milliseconds per tab and draws nothing. The editor reads the tab from `App::tab` (`ui.rs` `operation_editor_with`). So `operation_editor_with` gains a `tab: &str` argument, and `ui::view` passes `&app.tab` to it. `list_fields` passes each tab in turn, and the user's open tab never changes.

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
  - `goto_page {page}`, any page but Control (it would move the steps' place) and the planner's.
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
  - `pick_surfaces {ids, mode: add|remove|replace}`, `pick_features {ids, mode}` (the sample's features, `Feature`), `drop_hole {index}`, `clear_picks`;
    - **Two kinds of picking.** A click on a surface does one of two things, depending on the operation. The handler's branches (`main.rs` `Message::Surface`, ~6598) decide which, and that choice moves into one function both use, `App::pick_kind(op) -> Toggle | Fill`:
      - **Fill:** one face fills the operation's fields and replaces its pick. These are the lathe cuts and lathe drills (`picks_turn`), Slot (`slot::from_face`) and Box (`fill_box`).
      - **Toggle:** everything else. A click adds or drops a whole **group**, as the strategy groups faces: a smooth region, a hole's walls (threaded holes and arcs included), a corner, or a sloped face's steps. Two ids from one corner sent one by one therefore pick it and then drop it (found by the review's probe).
    - **The engine does not copy the grouping.** It sends real clicks and checks what each one did, so it always agrees with the handler:
      - **Fill:** only `mode: replace` with exactly one id is accepted. Several ids, `add` and `remove` are refused before anything changes: "<Strategy> takes one face, which fills it; send one id with mode replace, or clear_picks." A face that fills nothing is `failed`, with the handler's reason, and the pick is unchanged. Removal is `clear_picks`.
      - **Toggle, `add`:** ids already in the picks are skipped. For each other id, one click is sent, and the picks after it are compared with the picks before it. If the click dropped anything that was picked, because the id belonged to a group already picked, that click is undone: the operation's picks and geometry are put back to before it. The id is reported as "already picked with group …". If it added nothing, its reason is reported.
      - **Toggle, `remove`:** ids not in the picks are skipped (`no_change`). For each other id, one click is sent. It must leave that id unpicked; otherwise it is undone and reported.
      - **Toggle, `replace`:** `clear_picks`, then `add`.
    - **The result** returns the operation's picks afterwards (ids, and the holes or corners the app lists for them) with the mesh fingerprint they belong to, and what each id did. Picking is turned on when it is off, and set back afterwards.
    - **Tests:**
      - Toggle: several ids from one corner, one hole and one region in one call; the same `add` sent twice (`no_change`); `remove` of an id that is not picked (`no_change`); `remove` of one face of a picked corner;
      - Fill: Slot, Box and a lathe Turn given two ids (refused, nothing changed), `add` (refused), one id with `replace` (filled), the same again (`no_change`), and `clear_picks`.
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
- **Review:** `open_audit_row {index}`, `open_issue {setup, operation, message}`, `steps_fix {setup, operation?, problem}`, and `run_details {more?, steps?}` (the Run panel's fold-outs).
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
- The result carries the revision and the fingerprint it was checked at. If the job changes while it runs, by anyone and whether or not a window is open, it is refused as stale (see the freshness rule under *Tools*).

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
- From `proposal.rs`: `clip` moves to `model.rs`. What remains splits in two.
  - **Becomes a test-only `fixtures.rs`** (`#[cfg(test)]`, not in the app), because about 25 tests in other modules write their jobs in it: the proposal types, `parse`, `build` with its validation, and the fixtures.
    - The fixtures are `block`, `good`, `aluminium`, `hold`, `side_up`, `turned`, `with_lathe`, `with_mill`, `milled`, `drilled_by_hand`, `facing_box`, `hand_centre_drill` and `op`, plus `fake_openai::rotary::{report, good}`.
    - They are used by the tests of `post`, `viewport`, `steps`, `guide`, `gpu_mesh`, `chamfer`, `centre`, `rotary_check`, `sheet`, `hole_stages`, `holding` and `cam`.
    - Those tests and their machining assertions are kept, including the build refusals that `chamfer.rs`'s `refused` checks. `sheet.rs:898` keeps `parse`.
  - **Deleted, because only the model ever read them:** `describe` (the current plan as the model saw it), the `schema*` functions, the planner's instructions, and `Hands`.
    - The two assertions on them go: `chamfer.rs:1541-1548` (`describe` round-trip) and `chamfer.rs:1610-1630` (the schema offers Chamfer).
    - The same test's build, check and export assertions stay.

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
- **Packaged builds, before M4 removes the planner.** On the packaged macOS app (`/Applications/AXIS CAM Studio.app/Contents/MacOS/axis-cam`) and the packaged Windows `.exe`, each with Claude Code and with Codex added through the commands the Agents page copies, the agent:
  - builds a two-setup job for the STEP fixture, live in the open window and headless, and its G-code passes the NC review page;
  - works on the built-in sample, picking features;
  - browses a manual mill setup's steps without moving the machinist's place.

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
2. **gpt-6.1-sol, REJECT.** All seven findings were taken:
   - revisions from a work-state fingerprint with sourced change-log entries, not `touch()`, so user changes made during a wait are reported;
   - checkpoint restore as a new revision that cancels stale work, reloads the model and works out the saved state again;
   - the Steps and Run actions sorted by what they do: navigation and fold-outs allowed, progress recorded by hand refused, callbacks internal, browsing through `inspect`;
   - `save_*` effects that match the editor staying open, and `no_change`;
   - `session.json` and a session lock;
   - a `Send` `JobSnapshot` and fields collected without layout on the `App`'s own thread;
   - the test-only `fixtures.rs` and the named assertions.
3. **gpt-6.1-sol, REJECT.** All six findings were taken:
   - `Open` and the Control page are refused, because they move the guide's place, and the steps are browsed with a cursor of their own;
   - the machinist's state is protected by a before-and-after guard;
   - work-state snapshots are taken only at agent boundaries, cover every draft (checked by a field test) and every write path, including `Tick` and disconnects, and have a release budget;
   - restore validates the model on a candidate before it commits, and clears all derived state through one `job_replaced`;
   - saved state is compared as serialized values;
   - library saves are checked against shared normalising functions.
4. **gpt-6.1-sol, REJECT.** All five findings were taken:
   - **Progress guard:** commands that replace the project are exempt, but refused over unsaved records unless confirmed. Other commands are guarded per surviving setup, and nothing is reverted when the machinist acted during the window.
   - **Restore:** it checks for user changes and `locked()` again in the turn that commits.
   - **Who made a change:**
     - external events are recorded where they happen, including queued controller results applied on `Tick`, not inferred from message classes;
     - ownership windows last until the agent's work has settled, through `still_running`, reads and reconnects.
   - **Snapshots:** always full; there is no incremental fallback.
5. **gpt-6.1-sol, REJECT.** Both findings were taken:
   - checkpoints belong to one project (`project_id`), and a restore across a project replacement is refused;
   - reads check freshness by fingerprint at their start and their end, independent of ownership windows.
6. **gpt-6.1-sol, ACCEPT.** Its one minor point was taken: the refusal of a cross-project restore now says the checkpoint can no longer be restored, instead of "open that project first".
7. **gpt-6-astra, REJECT.** All seven findings were taken, and its suggested cuts were made:
   - **Heavy reads** are refused during a run and counted in `planning_busy()` until their workers finish.
   - **`check_holding`** checks the saved placement, with nothing substituted.
   - **Sessions** are never deleted automatically.
   - **Bounded answers:** every call answers within at most 50 s, under Codex's 60 s default, with `command_id`s and a `wait` that returns final results.
   - **Picking** works on whole groups, with `add`, `remove` and `replace`.
   - **`busy`** replaces a queued mutation, and `still_running` means only dispatched work.
   - **One writing agent per window,** with checkpoints dropped when the lease changes.
   - **Packaged-build checks** on both clients and both platforms before M4.
8. **gpt-6-astra, REJECT.** Both findings were taken:
   - reading Steps whose guide must be built is a heavy read;
   - picking is split into toggle and fill. It is driven by real clicks checked against what they did, not by a copied grouping function, and fill operations take one face with `replace` only.
