# Validation — 23 September 2026

This evidence covers the native Rust/Iced prototype's imports, bounded CAM previews, cumulative stock, adaptive stock surfaces and application workflows. It does not establish production machining, collision verification, production G-code beyond the checked LinuxCNC 3-axis subset or machine kinematics. Windows and Linux have not been verified.

## Automated checks

- `cargo fmt --check`: passed.
- `cargo test --locked`: **153 passed, 0 failed** (20.87 seconds in the final full run).
- Release build and renderer smoke: passed on Apple Silicon macOS 26.5.2, Rust 1.97.1.
- Renderer smoke completed **77 static screens and 24 camera-motion samples** in an isolated workspace. The normal saved project was not used by this workflow.

## LinuxCNC post

- Golden program for a small Facing job; post settings (tool table/touch-off, retract, G61/G64, dwell, Air) change the output as specified.
- All six implemented strategies post as one program and replay exactly: every position within 0.0006 mm, feeds within 0.05 mm/min, and tool, H offset, spindle, coolant and work offset active on every move.
- Corrupted programs are rejected: a moved coordinate, a missing F, the wrong tool, an arc and a dropped line.
- Flipped setups produce separate G54/G55 programs.
- Refusals: other controllers, 4/5-axis, travel, duplicate tool numbers, selected-point origin, non-LinuxCNC offsets, Air without an output, upstream failures, a setup without enabled operations and the example project.
- Rapid-safety rule tested for accepted retracts and peck re-entry, and for refused unproven descents, sideways rapids, holder depth, feed, rpm and travel (including inch profiles).

## Continuous stock and adaptive meshing

- Flat and ball cutter implicit fields are compared against an independent swept-solid membership calculation for horizontal strokes, ascending/descending ramps and vertical entries. Tests include finite flute extent and rotated/translated setup frames.
- Adaptive octree / manifold dual contouring produces triangle surfaces directly from continuous cutter history; the old occupancy grid no longer supplies rendered voxel faces. Tests verify finite vertices and two incident triangles per undirected mesh edge for the cylindrical cavity, ball-bottom cavity and sloped-channel fixtures.
- A 3 mm cylindrical wall fixture verifies radius error below 0.01 mm at Standard detail, improved accuracy over Draft, and flat floor positions. Its surface vertex count remains below 1% of the corresponding 256³ virtual domain. This is fixture evidence, not a general machining tolerance guarantee.
- A ball-bottom fixture checks spherical radius within 0.03 mm at Draft detail. A ramp fixture checks the analytic inclined cut surface within 0.02 mm and verifies a closed mesh.
- Cutter history is shared between snapshots. Consecutive collinear playback fragments merge into one continuous sweep; adding a new cut does not mutate an inherited snapshot.
- Background result tests reject surfaces from before a rewind, operation edit or detail change. Orbit/pan/zoom reuse the current mesh. While a newer snapshot is being meshed, the UI labels the retained surface Updating.
- The completed Fine pocket smoke used about 24,000 stored sweeps and 148,672 triangles at a 0.248 mm finest cell, taking about 2 seconds to mesh. The larger cumulative job reached about 48,000 sweeps; Standard updates took about 1.7–1.9 seconds. These are local measurements, not latency guarantees.
- Pocket completion, close-up Fine and Draft, rotated camera, Facing, Parallel, Waterline, inside/outside Contour, Drilling, flipped setup and restart screens were visually inspected. Flat faces, smooth pocket walls and tool-radius corners are visible. Fine sampling can still miss thinner features, and meshes are not certified collision/gouge results.
- Removed-volume readouts intentionally remain approximate: a separate bounded coarse occupancy grid supplies them. No dense fine-resolution volume is allocated for rendering.

## Cumulative operations and setup changes

- Pocket's starting stock exactly matches completed Facing stock. Restart returns to that inherited baseline; reverse scrub and incremental/direct playback agree.
- The next setup retains the complete prior cut history and empty occupancy cells after a 180° flip and translation. Its nominal stock offsets do not refill cavities. Frame roundtrips and finite-flute checks pass.
- Editing an earlier operation rebuilds downstream stock. Suppression skips the operation; an upstream generation failure blocks dependent previews with an explanation. Saved JSON regenerates the same baseline. Temporary stock bounds are not persisted.
- Setup reorder/delete relinks inherited stock sources. The first setup defines the initial blank; subsequent setups carry the same material.
- A renderer job with seven operations in the first setup and underside Facing in a flipped second setup verifies inherited start, partial removal, completion and restart. The whole-job report checks every enabled operation. The timeline remains per operation. New cap-coverage planning skips fully cleared top levels; general leftover-region rest machining is still unavailable.

## Automatic cutting start after earlier operations

- Facing → Pocket produces fewer depth levels and shorter estimated time, with the same completed stock occupancy as the original full-height path. Original XY clearance is retained; vertical approaches stop above the remaining top. The generated Pocket smoke shows Cutting start Z 43.43 mm and six levels from the faced stock.
- Tests verify partial facing keeps high regions, local coverage can lower a smaller region, finite flute lengths cannot erase an untouched cap, and a thin uncut strip cannot be ignored. Deliberately incorrect occupancy grids do not influence planning.
- Face bottom allowance, upstream edits, suppression and operation reorder rebuild the start. A flipped setup does not mistake the previous top for its new top. Temporary coverage state is not saved in project JSON.
- Outside Contour, drilling pecks and Waterline additionally check fewer levels, unchanged final targets, finite continuous motion and direct/incremental stock consistency after known cap removal.
- The optimization uses constant-height flat-cutter sweeps. Ramps, ball tools, partial regions and uncertain transformed boundaries conservatively retain passes when coverage cannot be proved.

## Operation coverage

Six imported-mesh strategies have bounded generators: Facing, Pocket, inside/outside Contour, circular-hole Drilling, Parallel and Waterline. Adaptive clearing, Scallop, Indexed, continuous Rotary and simultaneous 5-axis explicitly report unavailable. Legacy unsupported operations remain inspectable; unavailable new operations cannot be added to imported jobs.

- Facing: depth/spacing/feed-derived motion, continuous segments and times, clearance, known-volume removal, one-way/zigzag, alternating layer starts, keep-down/retract links and CW/CCW × Climb/Conventional.
- Pocket: actual selected-floor union, islands/disjoint regions, sampled wall/cutter clearance, outside/island preservation, floor depth, allowances, profile-ramp angle, direction reversal and bounded generation.
- Contour: actual projected outside silhouette with radius/allowance compensation and final repeats; Inside follows only selected walls/islands. Tests distinguish inside contour from pocket clearing and verify floor targets and loop winding.
- Drilling: detected circular openings, counterbore opening versus narrowest bore, diameter filtering/rejection, four cycles, pecks/dwell, breakthrough, depth and travel order.
- Parallel: analytic flat/ball drop-cutter checks on plane interiors and edges, direction, spacing, slopes, allowances, translated geometry and bounded sample counts. This remains approximate surface finishing.
- Waterline: constant-Z projected contours, radial allowance, direction, layer limits and flat-cutter vertical access. Ball waterline and slope filtering remain unavailable.
- All four newer strategies run on both packaged STEP and 3MF fixtures. Application tests cover creation, compatible tools, geometry, tabs, generation, playback, rewind, edit invalidation, saved parameters and whole-job failure reporting.

## Imports, rendering and existing workflows

- Native STEP reads the four-setup block: 4,220 triangles, 120 × 80 × 40 mm. Tests cover source-unit conversion, translated/rotated assembly components and malformed input.
- Native 3MF reads the matching block: 10,676 triangles, 120 × 80 × 40 mm. Tests cover package roots/units, multiple build instances, nested transforms, reflections, external Production references, bad indices, missing objects, cycles, singular/non-finite transforms and unsupported required extensions.
- Imported geometry selection stays attached to the real mesh and its fingerprint. Tests cover planar grouping, occlusion, placement, stale selection, replacement and isolation from sample geometry.
- Complete triangle rendering and 512-pixel image tiles with filtering gutters avoid the earlier async image-upload holes. Tests reconstruct the raster byte-for-byte and verify upload-size bounds. All 24 motion samples contain the model. Camera movement leaves fit scale/target stable.
- Stock offsets, transparency, fixed/relative mode, rotation/translation, invalid parameters and legacy migration are covered. Hover descriptions and animated concept cards use the shared native renderer.
- Blank startup, explicit example loading, import before setup, first machine/tool/setup, last setup removal, project archive/recovery, atomic persistence, independent profiles, tool deletion protection, AI example apply/undo and comments-only NC export remain covered.

## Selected-option help

- All shared choice controls pass the displayed value and current fields into their help. Legacy fallback values therefore match what the user sees. Tests verify method help/animation updates with CW/CCW, One way/Both ways, and selected entry, linking, cycle and containment options.
- Option-specific text covers operation, setup, machine, tool-type, planner example, playback-speed, visibility and stock-detail choices. Tests distinguish saved preferences from implemented functionality and check relative/fixed stock and detail-level explanations.
- Seven additional shared-renderer galleries cover Facing pattern/links, milling method with spindle context, ramp/plunge entry, all drilling cycles, three containment modes, stock modes/detail and spindle rotation. These exercise the same cards used by hover overlays.
- Native stock-detail verification exposed a tooltip obscuring the old drop-down. Detail choices were changed to three visible buttons; each button now explains its own resolution and cost.

Fixture-clearance follow-up: the setup field now uses a dedicated side-gap animation instead of the operation-height illustration. The shared field-help gallery was rendered and inspected, and the actual hover overlay was verified in the running app. `--help-only` renders this gallery in an isolated workspace.

## Packaging

The macOS bundle includes the STEP helper, 27 relocated non-system dylibs, CAD resources and third-party notices. Fidget's MPL-2.0 license/source notice accompanies the unmodified dependency; Clipper and CAD notices remain included. The app is locally ad-hoc signed. Packaging verifies its deep/strict signature in a clean staging directory and creates the ZIP before copying into the synced workspace.

Cargo still reports a future-compatibility notice in transitive `block 0.1.6` from native dialogs; builds and tests pass.

## Native project and final archive checks

- The rebuilt adaptive-stock core resumed the user's existing `AXIS_FourSetup_Block.3mf`. Pocket generated with stock after one earlier Facing operation. Live playback advanced, Pause worked, and forward scrubbing showed the machined pocket. The existing project, stock dimensions and machine/tool libraries were retained. In the follow-up, the user's current two-operation Facing/Pocket project is the workspace being preserved.
- Selected-option cards were inspected through the shared Iced renderer. After the Mac was unlocked, the corrected fixture-clearance tooltip was also verified as a live native overlay.
- The final app ZIP was extracted into a clean temporary directory. Deep/strict code-signature verification passed; its executable matches the installed bundle, and Fidget/Clipper license notices are present.
- Saved workspace JSON is compared with the pre-update backup. Source and application archives are regenerated for this update. The final remaining-stock build could not be reopened during the locked-Mac check; shared-renderer checks and package verification are completed independently.
