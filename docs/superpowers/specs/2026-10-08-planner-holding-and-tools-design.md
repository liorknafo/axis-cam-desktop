# Leftover stock, holdable setups and planner tools — design

Date: 8 October 2026. Branch: `planner-holding` (from `main` b2e35f3).

## Goal

1. A later setup starts from **the stock earlier setups left**, not setup 1's original block: a faced top makes the flipped stock shorter, cut sides make it narrower, and the vise, clamps, checks and view follow.
2. The AI planner **cannot hand over a setup that cannot be held**: every later setup is checked on that leftover stock with the holder it uses, and a failure names the earlier operation that removed the material the holder needed.
3. The planner **chooses each later setup's vise or clamps from the user's library** and plans the holding of every setup before its operations.
4. The planner gets **tools** to investigate and test before it answers: `check_plan`, `stock_after`, `check_holding`, `cutting_data`, `surface_info` (next to the existing `view_model`), so it can go back and edit earlier setups within one request.

## Decisions made by the user

- The next setup must match the current stock state (faced top → lower bottom after the flip; cut sides → the vise opens less).
- The agent must plan holding in advance and must not leave stock the available workholding cannot hold; it needs a way to go back and edit its plan.
- Workholding per setup: any vise or clamp set from the user's library.
- Tools: all of `check_plan`, `stock_after` + `check_holding`, `cutting_data`, `surface_info`.

## Non-goals

Soft jaws, custom fixtures, tabs or bridges, gripping finished faces with protection, changing setup 1's holder (setup 1 is the user's), a holder the library does not have, physical clamping forces.

## 1. Stock left by earlier setups

### The leftover box

For a later setup, the stock box is the original block (in that setup's frame) **trimmed on each of its six sides by every layer earlier cuts provably removed whole**. It is computed by a new `RestStock::left(&self) -> Bounds` in `src/rest.rs`, generalising today's `RestStock::top` (which proves a removed cap from above) to all six directions:

- for each side, the candidate depths are the faces of the flat constant-height capsules (`RestStock`'s evidence: facing, pockets, contours at a level), and the slab from the side to a candidate depth is tested with the existing conservative `covered`;
- the deepest proven slab trims that side; with no proof, or an exhausted budget, the side stays where it was;
- the trimmed box is never smaller than the part's own bounds (a sanity floor), and with no earlier cuts it equals the original box.

Ball cutters, ramps and drilled holes never trim (they are not used as evidence, as today), so the box is never smaller than the material: a skin of 0.1 mm keeps its layer.

### Where it is used

- `simulation::sequence_with` sets a later setup's `preview_stock` to the leftover box (today: `v.transformed_bounds(frame)`, `src/simulation.rs:648`). Everything that reads the setup's stock through `plan.stock` follows: the vise and clamp layout (`holding::Request::suggest`), the opening and position checks, the export checks (`post_check`), the plan check, the jaws and clamps drawn, the work-zero touch-off rules, the facing area and the clearance heights.
- The simulated solid (`Volume`) is unchanged: it keeps the original block's cells and cuts; only the box the setup reasons with shrinks.
- **Setup editor before a preview.** `Document::setup_stock` keeps returning setup 1's box carried into the frame (it cannot simulate). The app keeps a cache `App.left_stock: Vec<Option<Bounds>>` filled by a background job after an edit (debounced like the NC program checks), which runs the earlier setups' sequence and stores the leftover box of each later setup, keyed by a fingerprint of the document's setups, operations, tools and mesh. `holding_issues`, `holding_boxes` and the stock label use the cached box when it is current; otherwise the nominal box and the note "Original stock size until the earlier setups are simulated."

## 2. Holding checks on the leftover stock

New checks in `src/holding.rs`, run wherever the workholding of a later setup is checked today (export checks, plan check, setup editor), on the leftover box and the earlier cuts (`rest::Removed::at`, the exact point test):

- **Vise.**
  - The vise opens to at least the leftover width across the jaws (today's `opening_issue`, now on the leftover box).
  - The stock is taller than the grip by at least 1 mm (something stands above the jaws).
  - **Material at each jaw:** points sampled on each jaw's contact face (across the jaw width × from the stock's bottom up to the grip, 12 × 4 points, inside the leftover box) must have material (not removed by an earlier cut) at ≥ 60 % of the points.
  - **Material to rest on:** points sampled on the stock's bottom face (8 × 8) have material at ≥ 50 % of the points.
- **Amended in the final review (9 October 2026, binding): the vise closes on the material in its grip band.** Across the jaws the stock is as wide as the band from its bottom up to the grip: the leftover box's ±X and ±Y sides moved in further past every layer of that band the earlier cuts provably removed (`RestStock::band`, the proof of `left` with slabs only as tall as the band; the sides the jaws close on proven over the jaws' reach along them). The opening check, the jaws' positions, the jaw samples (on the band's faces), the suggested layout and the material to rest on (under the band) use the band; the height rule and clamps use the box. Walls finished through the band can be gripped below stock left wider above them.
- **Clamps.** Points sampled under each clamp's bite on the stock's top (6 × 3) have material at ≥ 60 % of the points.
- **Cause.** Each cut carries the setup and operation it came from (`stock_surface::Cut` gains `source: (u16, u16)`; cuts of different operations are no longer merged). A failed sample names the operations whose cuts removed most of the failed points, in words, for example: "Setup 2 cannot be held in the vise: setup 1, operation 3 (Contour \"Outside\") removed the material the jaws grip on −Y from 0 to 8 mm above the stock's bottom. Leave at least 8 mm there (for example with `bottom_offset`) and finish it in a later setup, or choose another holder."
- Each failure is a problem with stage `workholding` on the setup (operation `None`), so a plan with an unholdable setup cannot pass, and the export refuses it as it refuses other workholding problems.

## 3. Workholding chosen per setup

- **The part report** gains `workholding_library`: every library entry by number (1, 2, …) and size only — vise: jaw width, jaw height, jaw thickness, maximum opening, grip; clamp: length, width, height, bite — never names or notes. The existing `workholding` (setup 1's boxes) stays.
- **The proposal schema:** each setup gains `holder` (integer or null). Setup 1: must be null or the number of setup 1's own entry (setup 1's holder is the user's). Later setups: a library number, or null for "the same as setup 1" (today's behaviour). An unknown number is a problem at stage `structure`.
- `proposal::build` sets a later setup's `holding` (Vise / Clamps from the entry's kind) and `holder` (the entry's id) from it; the layout is suggested from the toolpaths as today.

## 4. Planner tools

All tools are offered together with `view_model` when pictures are on, and alone (without `view_model`) when pictures are off or refused; a refusal of the tools drops them all (as today's tool fallback). Every call answers with JSON text (or a picture for `view_model`), one line of description goes to the timeline, and every call counts as a tool round (at most **16** rounds per planner request, up from 8).

| Tool | Arguments | Answers | Limits |
| --- | --- | --- | --- |
| `check_plan` | `proposal` (the full proposal, same schema) | The check report the program would send after an answer (passed, problems by setup and operation, coverage, minutes), and the stock before each setup (as `stock_after`). The checked draft becomes the latest draft for the other tools and for `view_model` operation views. | 3 per planner request, 8 per run; it runs the full check (seconds). A draft that cannot be built answers with its build problems. |
| `stock_after` | `setup` (number in the latest draft) | The stock before that setup: size, box relative to the part's corner in that setup's frame, and for each of the six sides how much earlier setups removed and by which operations. | Needs a checked draft or answer; otherwise a one-sentence error. |
| `check_holding` | `setup`, `holder` (library number) | Whether that entry can hold the stock before that setup: the layout the program suggests (jaw sides and grip, or clamp sides and positions) and the problems (§2), empty when it holds. | Same as `stock_after`. |
| `cutting_data` | `tool` (tool number) | The rpm range from the material's surface speed and the machine's maximum, a recommended rpm (middle of the range, rounded to 10), and at that rpm the feed range from the tool diameter's chip-load band and the flutes, with a recommended feed, plus the band used. | Pure computation from the report. |
| `surface_info` | `surface` (ID) | Every orientation whose planes list it (plane kind, depth below top, area, bbox, `max_cutter_diameter`, holes), the surface's area and mean normal in setup 1's frame. | Pure computation from the report and the mesh. |

The timeline shows each call as a line under its request, for example "Checked a draft: 3 problems", "Stock before setup 2: 120 × 80 × 38 mm; top cut 2 mm by setup 1", "Vise 2 for setup 2: holds (opens 120, needs 84)", "Cutting data for T3: 7 960 rpm, 1 270 mm/min", "Surface 12: floor 8 mm deep in +Z".

## 5. Instructions

`planner::instructions_with` gains a **PLANNING THE HOLDING** section: plan, before any operation, how every setup is held and what stock it starts from; a vise needs material at both jaws up to the grip and on the bottom, clamps need material under their bites; leave that material in earlier setups (for example with `bottom_offset`) and finish it in the setup that no longer needs it; choose each later setup's `holder` from `workholding_library`; prefer checking a draft with `check_plan` before answering, and use `stock_after` / `check_holding` to decide the order of setups. A **TOOLS** section lists the tools and their limits. Without tools the text says the same rules without the tool names.

## Testing

- `RestStock::left`: a faced top trims the top by the facing depth exactly; a full-depth contour on ±Y trims both sides; a contour stopping 6 mm above the bottom trims nothing on the sides; a ball-end or ramped cut trims nothing; no cuts gives the original box.
- The sequence: setup 2 after facing setup 1 has a `preview_stock` shorter by the facing depth (flipped: its bottom moves up), and its jaws start at that bottom.
- Holding checks: vise opening on the leftover width; stock not taller than the grip; jaws on air after a full-depth side contour (the message names the operation and the band); bottom removed; clamps over a removed top; a holdable flipped setup passes.
- Cut provenance survives sequence and merging rules.
- The report's `workholding_library` (sizes only, no names or notes); the schema's `holder`; build sets later setups' holder; unknown numbers refused; setup 1's holder fixed.
- Each tool's answers and errors (no draft yet, unknown setup/holder/tool/surface), `check_plan`'s limits, the 16-round limit, the timeline lines, tool fallback.
- The editor cache: an edit invalidates it; the note shows until the job finishes; a current cache drives the opening check.
- A real-route run by the user (tools and the larger schema on the ChatGPT plan route cannot be tested here).
