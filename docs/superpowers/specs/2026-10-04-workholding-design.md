# Workholding: vise and clamps — design

Date: 4 October 2026. To be built after the material library and planner progress (`docs/superpowers/specs/2026-10-04-materials-and-planner-progress-design.md`).

## Goal

Model the vise or clamps that hold the stock in each setup, so that toolpaths that would hit them are refused before export, and the AI planner's plans respect them.

## Decisions made by the user

- Model **machine vises** and **toe/strap clamps** first.
- **Refuse only**: toolpaths are not changed; a check refuses export (and planner plans) when any move hits a jaw or a clamp, and says where.
- **The app suggests a layout** for each setup from the stock size and the toolpaths; the user can move it.
- Dimensions come from a **workholding library** (like tools and materials).

## Non-goals

Toolpath avoidance or re-routing; parallels, bolts, the table and the machine body; custom fixture solids; clamping-force or vibration analysis; vises rotated at an angle to the setup axes.

## Library

`Document.workholding: Vec<Holder>` with `Holder { id, p: Fields }`, carried to new projects and merged by Explore example like tools.

- **Vise** (`kind = "Vise"`): `name`, `jaw_width` (along the jaws), `jaw_height` (above the parallels' top, informational), `jaw_thickness`, `max_opening`, `grip` (default height of stock held between the jaws, from the stock bottom).
- **Clamp** (`kind = "Clamp"`): `name`, `length` (along the clamp, outward from its tip), `width`, `height` (above the stock top), `bite` (how far the clamp reaches onto the stock from its edge).

Starters, marked "Measure yours": "100 mm machine vise" (jaw width 100, jaw height 30, jaw thickness 20, max opening 120, grip 8) and "Toe clamp 60 mm" (length 60, width 20, height 25, bite 12).

## Per setup

Keys on the setup: `holding` = "None" | "Vise" | "Clamps"; `holder` = library id; `holding_gap` (clearance, default 2 mm).
- Vise: `vise_axis` = "X" | "Y" (the jaws face the stock's ±X sides or ±Y sides), `vise_offset` (jaw centre shift along the jaws), `vise_grip`.
- Clamps: `clamps` = list of `side` (−X, +X, −Y, +Y) and `offset` along that side, at most 8.

Geometry in the setup's frame, from the setup's starting stock box S (bottom z0, top z1):
- **Vise jaws**: two boxes against the two chosen sides, each `jaw_thickness` deep outward, `jaw_width` long centred at `vise_offset`, from far below (z0 − 100 mm) up to z0 + `vise_grip`. The stock width across the jaws must not exceed `max_opening` (an issue on the setup).
- **Clamp**: a box `width` wide centred at its offset along its side, reaching `bite` inward from the stock edge and `length − bite` outward, from z1 to z1 + `height`.

Each box is grown by `holding_gap` for the check. The 3D view draws the boxes (without the gap), semi-transparent; the setup sheet and the G-code header describe them ("Workholding: 100 mm machine vise on ±Y, grip 8 mm"; "Workholding: 4 × Toe clamp 60 mm at −X 0, +X 0, −Y 0, +Y 0").

## Suggested layout

When the user picks Vise or Clamps (or presses **Suggest layout**), and for setups the AI planner adds, the app generates the setup's operations and proposes:
- **Vise**: the axis (X or Y sides) whose jaw boxes stay farthest from every move that reaches below z0 + grip + gap; offset 0; grip from the library, reduced in 1 mm steps down to 3 mm while a move would hit; if neither axis is clear, the clearer one is kept and the check reports the hits.
- **Clamps**: 4 clamps (2 when the stock is shorter than 3 × clamp width along both axes) chosen from candidate spots (each side's middle and quarter points) to maximise the smallest XY distance to moves below z1 + height + gap; ties prefer opposite sides.
The suggestion never moves anything the user has set by hand on that setup unless they press **Suggest layout** again. A status line says what was chosen and whether every operation is clear.

## Check

For every move (feeds, rapids, plunges) of every enabled operation, in the setup's frame:
- the cutter: a vertical cylinder of the tool's radius from the tip up to the tip + flute length;
- the holder: a vertical cylinder of the holder radius from the tip + stickout upward without limit;
- swept along the move (sampled every 1 mm, at least both ends).
A sample that intersects a grown jaw or clamp box is a problem: "Contour hits the vise jaw on +Y at X 12.0, Y 41.0, Z 3.0 (within the 2 mm gap)." One problem per operation and holder part.

The check runs:
- in the export checks (`post_check`), refusing the setup;
- in the plan check, as a stage after generation (problems go back to the model);
- in the operation preview, as a warning as soon as an operation is generated.

## Planner

- The part report includes setup 1's workholding boxes (in setup 1's frame, relative to the model like other report coordinates) and the clearance gap.
- Setups added by a plan copy setup 1's workholding kind and library entry and get a suggested layout after generation, before the check; the preview shows the layout.
- The instructions tell the model where the jaws and clamps are, that moves within the gap are refused, and to keep operations clear of them (for example, stop outside contours above the jaws' top or move them to a later setup).

## Texts

All "Not checked: … fixtures, clamps …" sentences change to say vise jaws and clamps are checked as boxes with a clearance gap when the setup has workholding; parallels, bolts, the table and the machine body are not. Setups with "None" keep the old warning.

## Testing

Library validation and starters; per-setup geometry (both vise axes, clamps on each side, offsets, gap); suggestion picks the clear axis and clear spots on synthetic paths; the check finds cutter and holder hits on feeds and rapids and passes clear jobs; export refusal and plan-check stage; the planner copies workholding to added setups and reports it; migration (old setups get "None"); renderer screens of a vise and of clamps in the 3D view.
