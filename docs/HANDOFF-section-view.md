# Handoff: section view for the user and the planner agent

Branch `t3/session-view-user-agent`, moved here from the MacBook thread
"Add Session View for Users and Agents". Delete this file before the branch merges.

## What the user asked for

"Let's give the option to do session view, let's give this option to both user and agent."
They chose:
- **Section view.** The scene is cut on a plane, and the cut faces are filled and hatched. This applies in the app's 3D view and in the planner agent's `view_model` pictures.
- **Controls.** "The user will be able to select axis or a face and it will create a face slice that the user can slide."

## What is built (all in this branch)

- `src/section.rs` (new): `Section { through: Axis(k) | Surface(n), at, positive }` and `Plane`.
  - `Plane` provides `clip` (Sutherland–Hodgman), `clip_segment`, `crossing` and `box_face`.
  - `meet()` takes its two ends in a fixed order, so neighbouring faces get bit-identical crossing points.
- `src/main.rs`:
  - App state: `section` (with the part's fingerprint), `section_on`, `picking_section`.
  - `Message::Section(Edit::{Toggle, Axis, PickFace, At, Flip})` and `edit_section`.
  - `view_section()` only shows a cut that belongs to the current part.
  - Picking a face sends `viewport::Event::SectionSurface(n)`.
- `src/viewport.rs`, the shared CPU rasterizer:
  - The part's faces are clipped by the plane, but shaded with their whole face's normal (sliver faces used to shade dark).
  - The cut face is drawn by `raster_cap`: an even-odd scanline fill at the plane's exact screen depth (`Projector::plane_depth`), hatched by `section_hatch`.
  - The part's cut is blue (`PART_CUT`) and the stock's is tan (`STOCK_CUT`). The part's outline is drawn over the stock's cut face.
  - The stock box and jaws are a `SolidBox`, clipped too.
  - The ghost, the simulation grid stock and the toolpaths are clipped. The tool itself is not cut, on purpose.
- `src/ui.rs`:
  - A "Section" checkbox next to Stock / Toolpath.
  - A control row: X / Y / Z, "Face…", a slider over the scene bounds, a text saying where the cut is, "Cut away: …" and Flip.
  - The panel width `PANELS` is unchanged, on purpose.
- `src/look.rs` (the agent): `view_model` takes `section: {axis, surface, at, cut_away}` or null.
  - The schema is strict: every field is required and nullable. The field is `#[serde(default)]`, so older calls still work.
  - The caption gets "· SECTION", and the description names the cut.
- `src/help.rs` and `src/planner.rs`: help text, and the prompt's mention of `section`.

## Verified on the MacBook

- `cargo test`: 1472 passed, 0 failed, 27 ignored. `cargo fmt --check` is clean, and clippy has no warnings in the changed files.
- New tests:
  - `section::tests::*`
  - `viewport::tests::a_planes_depth_on_the_screen_is_the_depth_of_its_points`
  - `viewport::tests::a_step_parts_cut_outline_closes_on_itself`
  - `look::tests::a_section_cuts_the_part_open_and_hatches_its_cut_face`
  - `look::tests::a_section_of_an_operation_hatches_the_stock_left_and_outlines_the_part_on_it`
  - `smoke::tests::the_section_controls_cut_the_3d_view_on_an_axis_or_a_face`
- Pictures were looked at (block part, box, stock, half-ball STEP on X/Y/Z, the toolbar row). They were written by these ignored tests; run them with `--ignored --nocapture`:
  - `look::tests::picture_sections` writes `/tmp/section-{part,box,stock}-{front,iso_front_left}.png`.
  - `viewport::tests::picture_a_step_parts_sections` writes `/tmp/section-step-{x,y,z}.png`. It needs env `AXIS_SECTION_STEP=<a .step file>`; `AXIS_SECTION_ZOOM` is optional.
  - `smoke::tests::render_the_section_controls` writes `/tmp/section-app/{1-across-y,2-picking-a-face}.png`.

## What is left

1. Build and test here; check the counts.
2. Regenerate the pictures above and look at them.
3. Get the `astra` agent (`~/.claude/agents/astra.md`) to ACCEPT. Give it:
   - the user's request and choices above;
   - the pictures and the tests;
   - the deliberate choices: the tool is not cut, the translucent box's cut face stays translucent, and `PANELS` is unchanged.

   The MacBook's review attempt timed out before giving a verdict. Fix whatever it rejects, then ask again.
4. Report to the user. Delete this file before the branch is merged.
