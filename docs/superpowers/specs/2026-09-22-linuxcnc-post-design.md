# Validated LinuxCNC postprocessor — design

Date: 22 September 2026. Approved in conversation before this spec was written.

## Goal

Replace the comments-only `.nc` export with executable 3-axis G-code for LinuxCNC. The G-code must encode exactly the motion the simulation plays for the six implemented strategies (Facing, Pocket, Contour, Drilling, Parallel, Waterline). Every export is checked and round-trip verified; if anything fails, nothing is written.

## Non-goals

- Arcs (G2/G3), canned cycles (G81/G83/G73), cutter compensation (G41/G42).
- Inch output, controllers other than LinuxCNC, 4/5-axis and TCP output.
- Collision checks against fixtures, clamps or the machine body. Gouge checks beyond what each generator already guarantees.
- Enabling any strategy that is currently marked unavailable.

## Approach

The postprocessor consumes the `Plan.moves` produced by `simulation::sequence`, which the viewport already plays. That includes rest-lowered starts, inherited stock and flipped setups. The postprocessor does not re-derive any toolpath. Because a single source of truth defines the motion, the simulation proves what gets cut.

Rejected alternatives: an intent-based emitter with arcs and canned cycles, because it would be a second source of truth, and a multi-controller template framework, which nothing needs yet.

## 1. Output files

- One `.ngc` program per setup (in touch-off mode, one per run of the same tool; see §4), written to a folder the user picks. The file name is `<job> - <n> <setup name> (<offset>).ngc` (ASCII hyphen), with characters that are illegal in file names replaced. Files are written atomically (temporary file + rename). Existing files with the same name are replaced.
- All or nothing: if any setup fails a check or verification, no file is written. The UI lists the reasons.
- Enabled operations are emitted in browser order. A disabled operation is skipped and noted in a comment.
- All output uses mm (`G21`), absolute coordinates (`G90`), the XY plane (`G17`) and feed per minute (`G94`). Numbers have 3 decimal places, and feeds have 1.
- Motion is `G0`/`G1` only. Drilling pecks and dwells are the same expanded `G0`/`G1`/`G4` moves the simulation shows. LinuxCNC `G0` is a straight coordinated line, which matches the simulated rapids.

## 2. Coordinate mapping

Plan coordinates are millimetres in the setup's placed frame, whose axes are aligned with the machine axes. For a setup:

`work = plan − datum`

The datum is determined by the setup's `origin` field:

| Origin | Datum (plan frame, mm) |
| --- | --- |
| Stock top center | XY center of the setup's stock box, Z = stock top |
| Stock top front-left | (stock min X, stock min Y, stock top); front is −Y |
| Model origin | The source file's (0,0,0) passed through the setup's placement |
| Selected point (planned) | Refuses export |

- "The setup's stock box" is `Plan.stock` for that setup. For a later setup this is the original stock box moved into the setup's frame; it does not shrink as material is removed. A stock-based origin is therefore refused when earlier setups' cuts cover every sampled point of a face the datum needs (the top face; −X/−Y for front-left; all four sides for center), with a message suggesting Model origin. Every plan in one setup must report the same stock box; if one differs, the post refuses.
- Model origin requires `Mesh` to remember the display-space location of the source (0,0,0). `mesh::normalize` currently discards it by centering, so the mesh gains an `origin: [f32; 3]` field computed there. The sample mesh sets it to its own display origin.

## 3. Program layout

Header comments:
- program, job, setup, offset register, datum description and touch-off instruction
- stock size, orientation, fixture
- tool list (number, name, diameter, stickout)
- program X/Y/Z extents in work coordinates
- disabled operations
- a warning that fixtures, clamps and the machine body are not checked, and that the program must be previewed in LinuxCNC and air-cut first

Comment text is sanitized so user text cannot close a comment or start a new line.

Body:

```
G17 G21 G40 G49 G80 G90 G94
G64 P0.010            (or G61 when blending tolerance is 0)
G54                   (setup offset register)
<safe retract>        (omitted before the first tool change in tool-table + clearance-plane mode)
(Operation 1/2: Facing - T1)
T1 M6
G43 H1                (tool-table mode; touch-off programs have no T/M6/G43, only a load-and-touch-off comment)
<safe retract>        (again, so the new tool's tip is at a safe height before any XY move)
S8000 M3              (M4 when the plan is counter-clockwise)
M8                    (coolant, per operation)
G4 P2.000             (spindle dwell, omitted when 0)
G0 X… Y…              (XY at the retracted height)
G0 Z…                 (down to the plan's first point)
… plan moves …
M9
M5
<safe retract>
M2
```

- Tool change: `M9`, `M5` and the safe retract are emitted before each tool change, and the safe retract is repeated after `T# M6` + `G43 H#`. Otherwise a longer tool, or a tool left at the stock after touching off, would move sideways at a low height. In tool-table mode with the clearance-plane retract there is no retract before the first `T# M6` + `G43 H#`: at program start LinuxCNC's idea of the loaded tool (T0 after a restart) cannot be trusted, so the header tells the operator to start with the spindle raised clear. A tool change happens only when the operation's tool differs from the active tool. Spindle speed/direction and coolant are re-emitted only when they change between operations.
- After a tool change, the first plan point is reached by first rapiding in XY at the retracted height and then rapiding in Z. Consecutive operations with the same tool connect directly, because every plan starts and ends at its own clearance plane. The connecting move from one operation's last point to the next operation's first point is emitted as `G0`. If the two points are at different heights, the move is split so the tool climbs first and descends last. This connecting move is checked like any other rapid.
- Only words that change are emitted (modal X/Y/Z/F). A `G1` always has an active `F`.
- Safe retract: `G53 G0 Z0` (Machine Z home) or `G0 Z<clearance>` (Setup clearance plane: the highest tool-tip Z of the program's motion, in work coordinates).

## 4. Configuration

### Machine profile: new "Postprocessor" settings

Shown in the machine wizard's controller step when the controller is LinuxCNC. They are stored in `Machine.p`, and a `post_setting` fallback supplies defaults for older saved machines and setup snapshots, without rewriting anything else. The post reads the setup's machine snapshot, as simulation already does; after editing a profile, the user assigns it to the setup again. Invalid post settings block export only. They are not machine issues, so they never block simulation.

| Key | Choices / value | Default |
| --- | --- | --- |
| `post_tool_length` | `Tool table (G43 H#)`, `Touch off each tool (one program per tool)` | Tool table |
| `post_retract` | `Machine Z home (G53 G0 Z0)`, `Setup clearance plane` | Machine Z home |
| `post_blend` | tolerance in mm: 0 (→ `G61`) or 0.001–1 | `0.01` |
| `post_dwell` | spindle start dwell in seconds, 0–60 | `2` |
| `post_air` | `M7`, `M8`, `Not available` | `M7` |

Existing fields become enforced limits: travel `x`/`y`/`z`, maximum `rpm` and maximum `feed`. Inch profiles are converted with ×25.4 for these checks. Output stays in mm.

Touch-off mode writes one program per run of consecutive operations using the same tool, named `… - <i> of <k> T<t>.ngc`. Each header says which tool to load. Program 1 is touched off at the work zero with AXIS Touch Off, not Tool Touch Off, and the operator notes the Z of a fixed reference surface such as a vise jaw. Later programs are touched off on that reference, because earlier programs have cut the Z0 surface. There is no tool change, `G43` or `M0` inside, because LinuxCNC's AXIS cannot touch off while a program is paused.

Coolant mapping: Flood → `M8`, Mist → `M7`, Air → `post_air` (refuses if Not available), Off → none.

### Tool library: tool number

- `Tool.p["number"]` is a whole number from 1 to 9999, shown and editable in the tool library.
- New tools get the smallest unused number.
- On load, tools without a number get the smallest unused numbers in library order, persisted once. The user's current three tools become T1, T2 and T3.
- A missing, invalid or duplicate number is a tool issue, and blocks export when that tool is used.

## 5. Pre-export checks

Any failure blocks the whole export. Each message names the setup and operation.

1. The document has an imported part (the built-in example refuses), every setup has at least one enabled operation, and every enabled operation generates through `sequence`. Upstream failures block downstream setups, as they do in simulation.
2. The setup's machine: controller is LinuxCNC, 3-axis mode, valid post settings, no machine issues.
3. The setup's origin is supported and its offset register is G54–G59 (or G59.1–G59.3).
4. Each used tool has a valid, unique number. `Tool::issues` does not include the number, so an unnumbered tool never blocks simulation. `Document::tool_number_issues` reports it in the tool library and blocks export.
5. The operation's rpm is > 0 and ≤ the machine's maximum rpm. Every emitted feed is > 0 and ≤ the machine's maximum feed. Coolant has a mapping.
6. Travel: the program's X, Y and Z span in work coordinates (tool tip, including connecting and retract moves in work coordinates) does not exceed the machine travel. The header states that LinuxCNC's soft limits remain the check of absolute position.
7. **Rapids never enter material.** A rapid whose lowest point is at or above the setup's stock top passes. Otherwise it must be vertical (XY change ≤ 0.0005 mm), and the cutter column from its lowest point up to the stock top must be proven clear by one of:
   - (a) earlier operations: `RestStock::top` over the cutter's XY square at that point returns ≤ the lowest point + 0.0005 mm. This is the same conservative proof used for rest-lowered start heights, and uses the operation's inherited volume and frame.
   - (b) earlier motion with the same loaded tool: since the last tool change, the tool tip was at this XY (keyed to 0.001 mm) at a Z at or below the rapid's lowest point. This covers retracts out of a cut, drill pecks and re-entry. It is sound under the simulation's tool model (a cylinder of cutter diameter up to the stickout, then the holder). Material is only ever removed, and at that earlier moment the tool body occupied the whole column the rapid now sweeps; everything above the holder bottom is above the stock top by check 8.
8. **Holder clearance.** For every move, tip Z + tool stickout (holder bottom) ≥ the setup's stock top. This is conservative; the generators already limit depth by stickout.

Not checked, and stated in the header: fixtures and clamps (fixture clearance is a saved preference), the machine body, and gouging the part beyond each generator's own guarantees.

## 6. Round-trip verification

A separate strict reader, `ngc.rs`, parses each generated program. It accepts only the words the post emits: the listed G/M codes plus T, S, F, H, P, X, Y and Z, `(…)` comments and `(MSG, …)`. Any other word or malformed number is an error.

The reader replays modal state and produces a list of motions (rapid, feed with its F, dwell with its P), with tool, H offset, spindle and coolant state at each motion. Machine-coordinate (`G53`) moves are recorded separately.

The verifier maps the plans for each setup to work coordinates, adds the connecting and approach moves described in §3, and compares one to one:
- endpoints within 0.0006 mm
- motion type
- feed within 0.05 mm/min
- dwell within 0.001 s

Before every feed move it also checks that:
- tool-table mode: the tool is the operation's tool number and `G43 H` equals it
- touch-off mode: no tool change and no length offset are active
- the spindle is on in the plan's direction at the operation's rpm
- coolant matches the operation

The export writes files only after every setup passes. The same verification runs in the tests.

To make this possible, `simulation::Move` gains `feed: f32`: mm/min for Rapid and Feed moves, and 0 for dwells. It is set at all four construction sites (`simulation.rs` Facing closure, `cam.rs` `Builder::push` and drilling dwell, `pocket.rs` `push`). Timing is unchanged.

## 7. Modules

| Module | Responsibility |
| --- | --- |
| `src/post.rs` | `PostSettings` (parse and validate from a machine), datum mapping, per-setup program emission, file naming, `post_job(doc, mesh) -> JobPost` |
| `src/post_check.rs` | Pre-export checks §5 (limits, travel, rapid-in-material, holder) over the plans and simulations from `sequence` |
| `src/ngc.rs` | Strict reader/replayer, and `verify`, which compares a replay with the post's `Block` list §6 |
| `model.rs` | Tool number field, validation, migration on load; machine `post_setting` defaults; post-setting issues |
| `mesh.rs` | `Mesh.origin` |
| `main.rs` | Background post job with an epoch (stale results dropped after edits), folder picker, atomic writes, status |
| `ui.rs` | Program review page (checks per setup, preview, export button); controller step post settings; tool number field |
| `help.rs`, `choice_help.rs` | Tooltips for every new field and choice; replace "NC output pending" wording that is no longer true |
| `smoke.rs` | Program review page renders with checks passed and failed |

`JobPost` holds, per setup, either a program (file name, text, line count, extents, tools) or the list of reasons it failed. It also holds an overall `ready` flag.

## 8. UI

The Program review page runs the post in the background when the page opens and after any document edit while the page is shown. It shows:
- one card per setup: file name, pass/fail, reasons, and tool and operation list
- a preview of each passing program's first 120 lines, followed by the total line count
- **"Export LinuxCNC G-code…"**, disabled until every setup passes

Export reruns the post from the current document at click time, then writes. The setup-sheet export is unchanged.

A headless `--export-gcode <project.json> <folder>` command runs the same `post_job` and writes the same files. It is used to export saved jobs outside the UI and for release checks.

## 9. Testing

- Datum mapping for each supported origin, including rotated and flipped setups. Selected point refuses.
- A golden file for a small Facing job: exact text.
- Round-trip verification passes for all six strategies using the existing module fixtures. A deliberately corrupted program fails (moved coordinate, missing F, wrong tool, unknown word).
- A two-setup flip job produces two programs (G54, G55) with different datums and an inherited stock box.
- Both tool-length modes, both retract modes, blend 0 (G61) and >0, dwell 0 and >0, and the air mapping including Not available.
- Each refusal: non-LinuxCNC controller, 4-axis mode, rpm/feed over the limit, travel too small, duplicate tool number, failed upstream operation, a synthetic horizontal rapid below the stock top, a synthetic rapid into uncleared material, a holder below the stock top.
- Tool-number migration assigns 1..n once and keeps existing numbers.
- Fix the stale `choice_help_keeps_implemented_and_saved_preferences_distinct` assertions to the current accurate rest-machining wording.
- Full `cargo test`, `cargo fmt --check`, the release build and the smoke renderer. Then package, install, and export the user's saved `AXIS_FourSetup_Block` job to `outputs/` as a real-world check.
