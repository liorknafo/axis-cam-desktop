# AXIS CAM Studio — Rust / Iced

A working native desktop **CAM workflow prototype**, built with Rust and Iced 0.14. It carries the web prototype's Manufacture-style layout into a native application, with local persistence, editable operation parameters and a 3D model viewport. There is no embedded browser.

**G-code output is LinuxCNC 3-axis only.** Six imported-part operations have bounded motion previews with approximate stock removal: Facing, Pocket, inside/outside Contour, Drilling, Parallel and Waterline. Their exact simulated moves can be exported as one .ngc program per setup after checks and a round-trip replay (see below). Adaptive, Scallop, Indexed, Rotary and 5-axis remain explicitly unavailable for imported machining. Fixture, clamp and machine-body collisions are not checked, and production CAM remains pending. The example project's other paths and its planner are demonstrations. OpenAI is not connected, as requested for the prototype phase.

![Native workspace](docs/workspace.png)

## Run

The accompanying `AXIS CAM Studio.app` is an Apple Silicon build for macOS 26 or later (the bundled CAD libraries require macOS 26). It includes its STEP reader and runtime libraries; end users do not need Homebrew or Python. Double-click it to open. It is ad-hoc signed, not notarized for public distribution.

To build from source with a current stable Rust toolchain:

```sh
brew install opencascade
cargo run --release --locked
```

The build was tested with Rust 1.97.1 on Apple Silicon. The STEP helper uses a C++17 compiler and Open CASCADE 7.9. Set `OCCT_PREFIX` to a custom installation prefix if needed. `AXIS_SKIP_STEP=1` explicitly builds without STEP support; the macOS packaging script requires it. Windows and Linux builds have not been verified and may need platform-specific helper build/link changes.

Create a macOS bundle:

```sh
./scripts/package-macos.sh
```

The script writes `dist/AXIS CAM Studio.app`. An optional first argument overrides that output path. Set `AXIS_PACKAGE_ZIP` to an absolute ZIP output path to archive the verified bundle before copying it into a synced folder. It needs Xcode command-line tools and Python 3. `CARGO_TARGET_DIR` can keep build artifacts outside this directory.

## What works

| Area | Native implementation |
| --- | --- |
| Start / New project | Blank first launch, explicit example project, resume/open actions, New project shortcut and recoverable project history |
| Workspace | Grouped operation ribbon, custom SVG operation icons, setup/operation browser, parameter inspector, camera views, simulation strip |
| Machine library | Saved profiles; five-step wizard; 3-, 4-, 5-axis configurations; indexed/continuous modes; travel, spindle, feed, rotary limits, pivots, directions, controller, tool change, TCP preference |
| Multiple setups | Add, duplicate, reorder, delete; independent machine snapshots; G54–G59; automatic remaining stock from earlier setups; origin, fixture, rotation, translation and reclamping instructions |
| Stock sizing | Six independent offsets from placed part bounds; live X/Y/Z dimensions; fixed-size mode; shaded transparent stock with visible/hidden edges and shared part scale |
| Tool library | Editable tool types, diameter, flutes, flute length, stickout and holder diameter; in-use tool deletion protection |
| Operations | Six implemented imported-mesh preview strategies; explicit status for all eleven strategies; unavailable operations cannot be added to imported jobs |
| Geometry | Imported planar surface selection for Facing, Pocket and Drilling; whole-model Contour/Parallel/Waterline; separate selectable example housing; depth-tested rendering; orbit, pan, zoom and orthographic views |
| STL | Real binary and ASCII mesh preview |
| OBJ | Real mesh preview; polygon fan triangulation and negative face indices |
| STEP / STP | Open CASCADE CAD import and complete tessellated preview, assembly placements, source units converted to mm |
| 3MF | ZIP/XML mesh reader, declared units, build items, nested components, mirrored/placed instances and Production extension external model references |
| Hover help | Descriptions for every app button/input; choice help follows the selected value and related settings; matching native concept animations |
| Planner | Local deterministic sample plans for top, underside and side setups; review, apply and undo |
| Simulation | Six imported strategies with cumulative stock across operations/setups, adaptive octree surface meshing, background detail updates, pause/replay/scrub; separate example motion |
| Operation audit | Generate all checks every enabled operation across every setup, reports generated paths and actionable failures, skips suppressed operations |
| Saving | Automatic local workspace saving; explicit JSON project save/open; complete machine, setup, tool and operation data |
| Export | LinuxCNC 3-axis `.ngc` per setup (checked and replayed) and a Markdown setup sheet |

### Operation strategies

Facing, adaptive clearing, pocket, contour, drilling, parallel finishing, waterline finishing, scallop finishing, indexed face, rotary finishing and 5-axis surface.

Milling operations use **Tool → Geometry → Heights → Passes → Linking**. Drilling replaces Passes with Cycle. Indexed, rotary and 5-axis strategies add Orientation.

Fields are specific to each strategy: optimal load/rest machining for adaptive clearing; compensation for contours; pecks/dwell for drilling; slopes and surface spacing for finishing; rotary ranges, lead/lean and tilt limits for multi-axis work.

## Operation coverage and checking a whole job

Open **Operation support** in the header for the complete support list. **Generate all / Check all enabled operations** runs validation and actual path generation for each enabled operation in every setup. Failures are collected individually, so one failure does not hide later operations. Reports clear when the document changes. Stock advances in document order across operations and setups. An upstream failure blocks dependent stock previews with an explanation; it never silently substitutes a new blank. Suppressed operations are skipped. The playback timeline remains per operation.

| Strategy | Imported-part status / scope |
| --- | --- |
| Facing | Highest planar face; rectangular raster; flat/face cutter; one-way or zigzag |
| Pocket | Selected recessed planar floor; offset loops preserve islands; flat cutter; plunge/profile ramp |
| Contour | Outside model silhouette or Inside walls/islands of a selected recessed floor; In computer compensation; depth layers and repeat passes |
| Drilling | Circular openings in selected horizontal rim(s), including counterbores; narrowest vertical bore check; drilling, chip breaking, deep peck and bottom-dwell cycles |
| Parallel | Whole-model vertical flat/ball cutter envelope; angled zigzag passes, surface spacing, slope range, vertical stock allowance |
| Waterline | Constant-Z contours around geometry projected above each level; flat cutter, radial allowance and Climb/Conventional |
| Adaptive | Unavailable: engagement-controlled clearing kernel pending |
| Scallop | Unavailable: constant surface-spacing kernel pending |
| Indexed | Unavailable: indexed tool frames / rotary kinematics pending |
| Rotary | Unavailable: continuous rotary paths and stock simulation pending |
| 5-axis | Unavailable: tool-axis planning / machine kinematics pending |

The five unavailable strategies retain their educational illustrations in the explicit example project. Those illustrations are not machining calculations. Existing imported projects containing those operations retain their settings and report a failure when checked.

### Contour, drilling and finishing flow

- **Contour:** choose a flat cutter with adequate reach → Geometry chooses Model silhouette for Outside, or Selected geometry for Inside and a recessed planar floor → Heights references model bottom for Outside or the selected floor for Inside → Passes selects side, Climb/Conventional, stepdown, allowance and repeat passes → Generate. Inside follows only walls/islands rather than clearing the pocket, and offers profile-ramp/plunge entry in Linking. Outside repeats at final depth; Inside repeats at each depth. Arbitrary edge-chain selection and controller compensation are unavailable.
- **Drilling:** choose a drill → select horizontal rim faces containing circular openings → review detected bore/opening diameters, centers and depths; optionally filter by bore diameter (0 includes all) → Heights controls bottom offset and explicit breakthrough → Cycle controls straight/peck/dwell and travel order → Generate. Smaller drills are allowed, so the preview can leave material for a later enlargement operation. Circular opening recognition is tessellation based; the narrowest projected corridor constrains drill diameter. Cycles show cylindrical removal, not a conical drill tip.
- **Parallel:** choose a flat or ball end mill → Geometry uses the whole model silhouette → Heights sets approach and a nonnegative bottom/allowance combination → Passes sets direction, surface spacing, slope range, sampling tolerance and vertical stock to leave → Generate. Cutter envelopes are evaluated against mesh triangle interiors and edges, then sampled on a bounded XY grid. The ball shape is used by stock removal and rendering. This is an approximate finishing preview, not a certified gouge check. Steep transitions and undercuts require additional strategies.
- **Waterline:** choose a flat cutter → whole Model silhouette → Heights sets model-bottom offset → Passes sets stepdown, radial allowance and milling direction → Generate. Projection of all geometry above a level keeps vertical access conservative, so overhung detail may remain unmachined. Ball-cutter waterline and slope filtering are not available.

Separate paths retract to clearance before moving in XY. These four strategies do not expose inactive lead-in or stay-down controls. Inside Contour also supports profile-ramp entry through the shared planar boundary kernel. Spindle RPM and coolant are saved metadata; feed affects estimated motion time. Every preview inherits earlier enabled cuts, including cuts in earlier setups. Fixture, holder and machine collision solvers remain absent. Depth planning now skips fully cleared top layers using earlier flat-cutter sweeps. General rest machining that isolates leftover islands or corners is still unavailable, and fixture or machine collisions are not checked; the moves can be exported as LinuxCNC G-code from the NC program page.

Generation limits: 30,000 moves for the new kernels; up to 300 waterline layers; 1,000 contour levels or drill pecks per hole; 500,000 finishing samples and 60 million triangle/sample evaluations. Finishing grid spacing below 0.05 mm is rejected. Polygon offsets include an additional margin of max(0.02 mm, 1% of cutter radius). These limits bound a desktop preview; they do not establish production tolerances.

![Whole-job operation checks](docs/operation-audit.png)

## Contextual help for choices

Hover a choice field to see **the selected option in the title** and its specific behavior. For example, Profile ramp explains a descending contour entry; Plunge explains a straight vertical entry. Changing the choice updates the text and illustration. Milling-method help also reads the saved spindle rotation; Facing explains when Both ways overrides the one-way method preference. Legacy values use the same resolved defaults as the visible control.

Selected-state animations cover spindle rotation, Climb/Conventional, one-way/zigzag facing, keep-down/retract links, ramp/plunge entry, four drilling cycles and cutter containment. Setup, machine, tool-type, coolant, stock mode, AI example, playback-speed and display choices also have option-specific text. Saved preferences are distinguished from implemented behavior. These are explanatory animations, not previews of the current job.

The contextual approach references Autodesk's [Fusion Face parameter reference](https://help.autodesk.com/view/fusion360/ENU/?contextId=MFG-REF-2D-FACE-CMD). The native illustrations and descriptions are original and reflect this app's supported behavior. Stock-detail selection uses visible Draft / Standard / Fine buttons so its tooltip cannot obscure a drop-down menu.

Fixture clearance has its own animated side view: a labeled holder, cutter and fixture with a dimension from the nearest tool-assembly edge to the fixture. This setup value records a requested margin; it does not yet drive collision checking or toolpath offsets. It is separate from operation clearance height.

![Selected machining choices and their help](docs/choice-help.png)

## Cumulative stock and adaptive surface detail

The first setup defines the blank. Every later operation starts with all earlier enabled cuts, in browser order. Later setups reposition that same material: their old nominal stock dimensions do not refill cavities. Editing, suppressing, reordering or removing operations/setups invalidates previews, and generation rebuilds the applicable history. Reordering setups makes the new first setup responsible for the initial blank. An upstream failure prevents dependent previews from presenting misleading fresh stock.

Cutting start heights also inherit prior work. Facing, Pocket, Contour, drilling pecks and Waterline test whether the volume above their machining envelope is already removed. A completed facing pass can therefore lower the next pocket's start and eliminate redundant depth levels. Positive top offsets and remaining allowances are respected. XY transfers retain the original clearance plane; the vertical approach can descend to the remaining top plus the retract margin. The computed start Z appears in playback and Heights.

This is conservative cap coverage: complete boxes must lie inside earlier constant-height flat-cutter sweeps, using their continuous cylindrical geometry and finite flute length. Coverage subdivides adaptively; any unresolved region or exhausted budget keeps the higher start. The coarse volume grid and display detail cannot cause material to be skipped. Ball/ramp sweeps are not yet used as proof for this optimization; complex transformed or partly cleared regions can retain air passes. A final target-level pass and explicit finishing repeats remain. This does not replace general rest machining with smaller tools.

Stock stores continuous swept cylinders for flat cutters and spherical-bottom sweeps for ball cutters, including finite flute length, vertical entries and sloping feed moves. The geometry stays in the first setup's frame; setup rotations and translations transform cutter coordinates into that frame. Restart and reverse scrubbing restore the inherited geometry before replaying the selected operation. Project files save the job; stock is regenerated when opened.

![Pocket starts at the stock left by Facing](docs/remaining-stock-start.png)

The surface uses [Fidget 0.4.3](https://docs.rs/fidget-mesh/0.4.3/fidget_mesh/)'s adaptive octree and manifold dual contouring. Uniform regions can remain coarse, and boundary cells resolve curved walls, ramps and cutter-radius corners. The visible mesh comes directly from continuous cutter geometry, not smoothing occupied cubes. No dense high-resolution 3D array is allocated. Cutter history uses shared snapshots and combines collinear playback fragments; a bounded coarse occupancy grid remains solely for approximate volume reporting and regression checks.

Choose **Draft**, **Standard** or **Fine** beside the simulation timeline. These use maximum octree depths 7, 8 and 9. The virtual root cube is 104% of the longest stock dimension; a 140 mm blank therefore has finest cell sizes of about 1.14, 0.57 and 0.28 mm. This is a sampling scale, not a dimensional accuracy guarantee. Dual contouring can locate surfaces between sample positions. Features thinner than a cell may be missed, and meshing is not a certified gouge/collision check.

Meshing runs in a background worker. **Updating…** means the last completed stock surface is displayed while a newer snapshot is calculated. Orbit/pan/zoom reuse that mesh. Rewinding discards surfaces from the future; changing detail, selecting a different operation or editing the job rejects stale worker results. Long jobs and Fine detail can take seconds to update; cutter motion is independent of that update. Untouched outer faces stay amber and cut faces are green. Hide Toolpath for an unobstructed view of the stock.

![Adaptive stock with cutter-radius pocket walls](docs/adaptive-stock.png)

## Start a project from nothing

The app opens on a start screen. Choose **New blank project** to begin with no model, material, setups or operations. On a fresh installation the machine and tool libraries are empty too. **New project** in the header or **Command/Ctrl+N** starts fresh at any time while retaining saved machine and tool profiles. The previous job is archived before switching; use **Start → Browse project history** to reopen it, or **Recover previous project** to undo the latest switch during the session.

1. Click **Import your first model** and choose STEP/STP, 3MF, STL or OBJ. The part can be viewed before a setup exists.
2. Choose **Create first setup**. Select a saved machine, or complete **Set up a machine**, save the profile and choose **Create first setup** from the library.
3. Set the stock offsets (or fixed dimensions), material, work offset, origin and workholding in the setup editor. New setups contain no operations.
4. Save a cutter in **Tool library**, then choose an operation and configure its parameters.
5. Add further setups for each repositioning of the part. The last setup can be deleted, returning the project to a part with no setups.
6. Use **Save project** to create a portable `.axis.json` file, and **Open** to restore it later.

Six operations generate imported-part motion and approximate stock removal within the scopes below. Production toolpaths, collision checks and the five explicitly unavailable strategies remain future work. To try the illustrative paths and planner, choose **Start → Explore example project** explicitly.

## Try the example workflow

Open **Start → Explore example project** first. This loads the housing, sample library entries and example operations.

1. Open **Machine library → Set up a machine**, complete the wizard and save. Assign the profile to the current setup.
2. Click **Edit setup** to configure stock, work offset, origin, fixture and material.
3. Choose an operation from the ribbon. Set the cutter and feeds/speeds.
4. Open **Geometry**. Select sample features from the list or click them in the viewport. Drag still orbits the model.
5. Set Heights, Passes/Cycle, Orientation when applicable, and Linking.
6. Click **Generate sample preview**, then **Play**. Previews are invalidated when job parameters change.
7. Duplicate a setup, use **Flip X 180°** or enter any rotation/translation, select its stock source and add reclamping instructions. Repositioned setups are saved and visualized; their toolpath calculation remains pending.
8. Try **AI planner → Create example plan → Apply example plan**. Undo restores the previous job during the current session.
9. Open **NC program** to review configuration issues, export the setup sheet or export a non-executable program outline.
10. **Save project** writes a portable `.axis.json` file. **Open** restores it.

Viewport: left-drag orbits, right/middle-drag pans, scroll zooms; ISO/TOP/FRONT/RIGHT reset the camera. Command/Ctrl+N starts a blank project, Command/Ctrl+S saves a project and Command/Ctrl+O opens one.

The fit scale stays constant while orbiting and panning. Full-resolution viewport frames are uploaded as small tiles with filtering gutters, keeping them below Iced 0.14's asynchronous image-upload threshold and preventing missing model frames during camera movement.

Imported geometry supports **preview, stock sizing, planar surface selection and the six operation previews listed below**. The viewport always shows the current project’s part. Example geometry and illustrative paths are available only through the separate example project. STEP and 3MF dimensions are reported in mm; STL and OBJ have no reliable declared unit and are reported in source units. Stock sizing treats STL/OBJ coordinates as mm, with an explicit note beside the fields. The display centers the mesh and fits the part and stock together; both use the same scale. Exact CAD-to-machine origin placement remains pending. Stock and operation settings use mm; machine specifications can use mm or inches.

### Facing on an imported part

1. Add **Facing**, choose a saved tool and open **Geometry**.
2. Click a planar region on the actual part in the viewport. Amber shows the hovered region; blue shows selected regions. Click again to remove it. Drag to orbit, or use TOP to look down setup Z.
3. Click **Finish picking**. The selection remains highlighted and is saved per operation. Use the selected-surface list or **Clear surface selection** to remove selections.
4. Configure Heights, Passes and Linking, then **Generate operation preview** or **Generate & play** below the viewport. Checks flag surfaces that are not perpendicular to setup Z (0.5° tolerance) or lie on different planes (0.001 mm height tolerance). Reposition the part in another setup to face a side, or use separate operations for different heights.

Selections are connected coplanar triangle patches, not exact CAD faces. Curved surfaces remain faceted. Winding is not treated as proof of machining access; fixture/tool collision, reachability and production toolpath computation are still pending. Selecting a surface alone does not generate a preview; generate it or use Generate & play.

Saved selections contain a SHA-256 fingerprint of the imported tessellation and its triangle order. Reopening the same geometry restores them; changed tessellation is reported and must be reselected. Importing a replacement part clears prior geometry selections while keeping operation parameters. Legacy imported projects’ sample-only selections are cleared on restore. Whole-model strategies use the imported silhouette. Unavailable strategies display an explicit status and cannot be added to imported jobs.

![Facing selection on the imported 3MF block](docs/imported-facing.png)

### Imported Facing simulation

Select a saved Facing operation with valid geometry, stock and a flat end mill or face mill. Click **Generate & play** below the viewport to generate and start. **Generate operation preview** in the inspector prepares the preview without starting it. Use **Pause**, **Restart**, **Replay**, the timeline and **1× / 10× / 30× / 60× / 120×** speed. The default for a generated Facing preview is 30×. Reopening a project requires regenerating its preview.

- The preview computes alternating straight passes from the tool diameter, stepover, maximum stepdown, pass direction, selected plane, top/bottom offsets and radial allowance. It uses the **XY bounding rectangle** of the selected surfaces, model or stock, including holes and gaps within that rectangle. Tool containment adjusts cutter-center limits against that rectangle.
- Before considering earlier cuts, the starting plane is the higher of stock top or selected plane + top offset. Proven prior removal can lower the cutting start. Bottom offset sets the target relative to the selected plane. The preview refuses a target below the highest point of the placed model. It supports repositioned setups whose selected face is now the highest plane, while retaining material removed in earlier setups.
- **Passes → Pass pattern** selects **Both ways (zigzag)** or **One way**. Both ways reverses each adjacent cutting row. **Next depth level → Continue from previous end** reverses the complete row sequence at each depth so the first stroke begins at the preceding endpoint; **Return to first start** repeats the layer from its original XY.
- **Tool → Spindle rotation** selects **Clockwise (CW)** or **Counterclockwise (CCW)**, viewed from the spindle toward the part along setup −Z. Match the cutter's cutting hand. **Passes → One way → Milling direction** selects **Climb** or **Conventional**, relative to the nominal uncut side of each row. Changing spindle rotation reverses one-way cutting strokes for the same method. Both ways is explicitly **mixed climb/conventional**; the spindle does not reverse between rows. Entry and full-width engagement may contain both modes even in one-way paths. Animated hover illustrations compare cutter feed with a fixed CW spindle. The simulation identifies its method and rotation; it does not animate actual RPM or output spindle commands.
- New operations default to CW and a Climb preference for One way. Missing rotation/method settings in older one-way jobs resolve to CW/Conventional to preserve their existing motion; their path does not silently reverse. Both settings are saved with the operation and invalidate its preview when edited.
- **Linking → Between facing passes → Keep tool down** feeds between neighboring rows at cutting depth. With Continue from previous end it feeds vertically down at the last endpoint and cuts the next layer in reverse. **Retract each pass** lifts to clearance between rows. One way always retracts for its returns and repeats each depth from the first row. The new presets default to Both ways, Continue from previous end and Keep tool down; older saved operations retain Both ways, Return to first start and Retract each pass until changed.
- Initial entry and retracted links traverse at clearance, approach the retract plane, then plunge. Cutting uses the specified feed, plunging uses 25% feed and rapids assume 6,000 mm/min. Timing omits acceleration, spindle ramp-up, tool changes and controller behavior. Lead-in curves and fixture avoidance are not implemented. Keep-down links follow straight lines inside the convex rectangular cutter-center boundary; no collision checking is performed.
- Continuous flat/ball cutter sweeps subtract material from the initial stock. An adaptive octree and dual contouring produce the visible surface. Amber marks original exterior faces; green marks cut surfaces. Rewinding reconstructs the earlier state; Restart restores the stock left by preceding operations and setups. The volume readout is a separate coarse estimate.
- With Stock enabled, the viewport shows the remaining blank, so unmachined pockets remain filled. Hide Stock to see the reference model. Toolpath displays the latest cutting pass and active rapid. The cutter uses saved diameter, flute length and stickout; the holder uses its saved diameter with an illustrative 8 mm collar length.
- Edits to geometry, tools, stock and operation parameters invalidate the preview. Unsupported operations show an unavailable state instead of inert playback controls. A single combined-program timeline, collision/gouge verification, simultaneous multi-axis simulation and executable machine output are not implemented. The top-plane guard is a limited geometric check, not a manufacturability check.

Generation is bounded to 2,000 cutting strokes. Very small step sizes, unsupported tools, no stock above the target, stale selections, unsuitable planes and unsupported stock sources show a specific explanation.

![Partial Facing stock simulation](docs/facing-simulation.png)

![Facing keep-down links](docs/facing-linking.png)

![One-way milling direction](docs/facing-milling-direction.png)

![Milling direction hover illustrations](docs/milling-direction-help.png)

### Pocket on an imported part

1. Add **Pocket**, choose a saved flat end mill and set **Tool → Spindle rotation** (CW/CCW, viewed toward the part).
2. In **Geometry**, click the recessed planar floor and **Finish picking**. Multiple coplanar regions can be selected. Their triangle union supplies the actual XY boundary; holes remain islands. Pocket uses **Selected geometry / Tool inside boundary**, rather than a rectangular Facing extent.
3. In **Passes**, choose **Milling direction → Climb or Conventional**, stepover (up to 50%), maximum stepdown, radial/axial stock to leave and optional repeated finishing loops. The milling direction is always visible. With CW, climb travels CCW around the internal wall and CW around islands; reversing spindle rotation reverses these paths.
4. In **Linking**, choose **Profile ramp** (0.1°–15°) or **Plunge**. A profile ramp descends along the current contour at 25% feed, then completes it at depth. Plunge requires a plunge-capable cutter. All contour transfers retract to clearance; links do not cross islands at depth.
5. Click **Generate operation preview** or **Generate & play**. Pause, replay and scrub work with the same remaining-stock model as Facing. Pocket paths on the current depth layer remain visible. Direction/rotation/geometry/tool edits invalidate the preview.

The planner offsets the selected floor inward from the cutter radius and radial allowance, then traverses the contours from inside outward at each depth. Holes/islands and disjoint regions are retained by polygon union/offset operations using [Clipper2](https://docs.rs/clipper2/0.6.0/clipper2/). A conservative geometry margin of max(0.02 mm, 1% of cutter radius) covers integer rounding and arc approximation. Narrow areas/corners that cannot admit the cutter remain uncut. Finishing loops repeat the boundary at the selected allowance; they do not automatically remove that allowance.

Bottom offset plus axial allowance is measured from the selected floor and cannot place the target below it. Start is stock top plus a positive Top offset. Maximum stepdown is limited by flute length, and full depth must be less than stickout. The selected floor must be horizontal and recessed; projected geometry above it is checked for obstructed vertical access. Sloped/overhanging floors, incompatible cutters, stale selections and excessive plans are rejected with an explanation. Bounds: 256 offset rings and 30,000 motion segments.

**Pocket starts from the stock left by preceding enabled operations and setups.** The continuous 3D stock retains cavities when the part is flipped. Each operation has its own timeline; Restart restores its inherited starting stock. The vertical-access check is not a tool/holder/fixture collision solver. This is an imported-part preview, not production NC output.

![Pocket stock after clearing around an island](docs/pocket-finished.png)

![Pocket direction and partial simulation](docs/pocket-partial.png)

### Relative and fixed stock

**Relative size box** measures extra material from each side of the part's bounding box after the setup's rotation:

| Axis | Negative side | Positive side | Resulting dimension |
| --- | --- | --- | --- |
| X | Left | Right | Part width + left + right |
| Y | Front | Back | Part depth + front + back |
| Z | Bottom | Top | Part height + bottom + top |

Each offset is independent, in mm; zero is flush with the part. For a 120 × 80 × 40 mm part, top = 10, right = 2 and all other offsets = 0 gives **122 × 80 × 50 mm stock**. Sides follow setup axes, not the camera. Relative stock follows part translation; rotation recalculates the part bounds. The read-only result, viewport and setup sheet all use the same calculation. Bounds come from the preview mesh, including the STEP tessellation, rather than exact CAD surface extrema.

**Fixed size box** uses explicit X/Y/Z dimensions and stays centered on X/Y with its bottom at the display Z=0 plane. Switching modes retains each mode's inputs. Invalid offsets, missing geometry or invalid dimensions display an explanation and suppress the stock box instead of using fallback dimensions.

Stock is rendered as a lightly shaded transparent amber box. Solid visible edges and faint dashed hidden edges distinguish the stock from the blue-gray part. Each offset field includes a looping illustration of extra material on that side.

![Relative stock with top and right offsets](docs/relative-stock.png)

Older saved projects whose relative mode contained only overall dimensions reopen as **Fixed size box**, preserving those dimensions. Select Relative size box to use the new per-side offsets. Later setups start from the material left by earlier enabled operations and setups, including after flips.

**STEP:** Open CASCADE reads surfaces/solids and assemblies; the helper tessellates at 0.05 mm linear deflection and 0.3 rad angular deflection. Assembly placements and unit conversion are applied. The original file stays intact. Exact CAD topology, colors, PMI and automatic machining feature recognition are not exposed to the app yet. Facing and Pocket selection use connected planar regions of the tessellated mesh. Wire-only models, failed translations, or faces that cannot be tessellated are rejected with a readable error. Conversion runs outside the GUI and times out after two minutes.

**3MF:** Uses the package root relationship, not an assumed filename. Only printable root build items are displayed, with referenced components, nested transforms and winding correction for reflections. Geometry-only support excludes material/color/texture and printer-specific settings, boolean union of overlapping parts, and non-mesh extensions. Unknown required extensions and mixed units across external model parts are rejected rather than guessed. Production geometry references are supported; this is not a complete 3MF conformance implementation. Percent-encoded package URIs are currently unsupported.

Limits: 50 MB source file, 100 MB expanded 3MF XML, 200,000 displayed triangles, 4,096 archive entries, 128 model parts and 64 levels of component nesting. Large meshes are rejected rather than previewed with missing triangles. OBJ fan triangulation assumes convex polygon faces. Saved imports reopen in the background; a missing or invalid original file is reported explicitly.

### Tooltips and animated help

Hold the pointer over a button, field label/input, dropdown or toggle for about **0.6 seconds**. Command cards explain the action; field cards explain meaning, units and practical consequences. Relevant cards show a five-second looping native canvas illustration: facing, pocket clearing, drilling, finishing, rotary/indexed motion, tool tilt, clearance planes, pass spacing, layer depth or cutter dimensions. No GIF download or network connection is needed. Illustrations explain concepts; they are not computed toolpaths and do not depend on the current parameter value. Animation updates run at approximately 30 fps while an animated help target is hovered.

## LinuxCNC G-code export

Open **NC program**. Checks run in the background for every setup: generation, controller (LinuxCNC, 3-axis), work offset (G54–G59.3), tool numbers, spindle and feed limits, machine travel span, rapids into material and holder clearance. Each program is then read back by a strict G-code reader and compared move by move with the simulation. **Export LinuxCNC G-code…** writes `<job> - <n> <setup> (<offset>).ngc` for every setup (touch-off mode: `… - <i> of <k> T<t>.ngc` per tool run), or nothing if any setup fails.

- Output: G21, G90, G17, G94, G0/G1/G4 only. Drilling pecks are expanded, with no canned cycles or arcs. Coordinates are relative to the setup's work zero (Stock top center, Stock top front-left or Model origin).
- Machine profile → Controller step → Postprocessor: tool table (`T# M6` + `G43 H#`) or touch off each tool (one program per tool run: run G49 in MDI before touching off, touch the first tool off at the work zero with AXIS Touch Off, not Tool Touch Off, note the Z of a fixed reference such as a vise jaw, and touch each later tool off on that reference; AXIS cannot touch off while a program is paused), safe retract (`G53 G0 Z0` or the setup clearance plane), path blending (`G64 P`, or `G61` at 0), spindle start dwell and the Air output (M7/M8). Setups keep a machine snapshot, so assign the profile again after editing it.
- Tools have a number (T#) in the tool library. Older libraries are numbered 1, 2, 3… once when opened.
- Not checked: fixtures, clamps and the machine body. LinuxCNC's own soft limits check absolute position. Preview each program in LinuxCNC and air-cut first.
- Headless: `axis-cam --export-gcode <project.json> <folder>` runs the same checks and writes the same files.

## Persistence

On macOS, automatic state lives at:

```text
~/Library/Application Support/AXIS CAM Studio/workspace.json
```

Other platforms use the OS local application data directory. Writes use a temporary file and atomic rename. An unreadable workspace is preserved, and a separate recovery workspace is used. Starting a new project, opening another project or loading the example archives the complete previous job in a `history` subdirectory, including unsaved parameter edits. A failed archive leaves the current project in place. An explicit project save is separate from the automatic workspace. The start screen offers the previous job without opening its model automatically; choose **Continue previous project** to resume it.

Older project files without an explicit sample-model flag retain their sample housing when they contain setups and no imported file. Newly saved blank projects stay blank after reopening.

Projects contain the **source file path and metadata**, not an embedded copy of the imported mesh. Keep the original mesh file accessible when reopening a project. No API keys are stored, no API requests are made, and the app has no telemetry.

The `examples` directory includes a sample AXIS project, an OBJ cube, matching four-setup STEP and 3MF parts, an inch-unit STEP block and a placed/rotated STEP assembly.

## Tests and renderer checks

```sh
cargo test --locked
cargo fmt --check
cargo run --locked -- --smoke-test ./smoke-output
# Fast, isolated rendering of field help only:
cargo run --release --locked -- --smoke-test ./help-output --help-only
```

The explicit smoke-test mode uses an isolated workspace, exercises seventy-seven screens plus twenty-four camera-motion samples through Iced's renderer and closes itself. The motion samples check that the model remains present across orbit, pan and zoom changes, with stock both shown and hidden. It does not modify the normal workspace. Simulation tests verify continuous motion, parameter-derived spacing/depth/time, stock removal against known volume, rapid moves removing nothing, swept cutter footprint, rewind/incremental consistency, rotated setups, rejected configurations the Play/pause/replay/invalidation lifecycle, both-way continuation at odd/even row counts and varied angles, one-way returns, reduced return time, unchanged final stock, and saved/legacy direction preferences. Imported Facing checks cover real visible-surface picking after setup transforms, planar grouping, selection persistence, replacement and stale-mesh handling, and sample isolation. Unit tests also cover complete tile pixel/gutter preservation below the upload threshold, stable orbit scale, blank startup and project recovery, the first import/machine/setup/tool flow, removal of the last setup, independent stock offsets, rotated and translated stock, stock transparency and occlusion, legacy stock migration, configuration validation, snapshots and dependencies, AI apply/undo, preview invalidation, import parsers, persistence, picking and the LinuxCNC program page (blocked and ready).

## Code map

- `src/main.rs`: application state, messages, file dialogs and actions.
- `src/ui.rs`: native layout, forms, wizard, libraries, planner and NC review.
- `src/model.rs`: serialized domain model, strategy defaults, validation and exports.
- `src/viewport.rs`: projection, cached depth-buffer rendering, feature picking and sample motion.
- `src/mesh.rs`: sample model, STL/OBJ readers and preview normalization.
- `src/three_mf.rs`: bounded 3MF package and mesh/component reader.
- `src/step.rs`, `native/step_preview.cpp`, `build.rs`: isolated Open CASCADE STEP converter.
- `src/help.rs`: shared hover cards and operation illustrations.
- `src/choice_help.rs`, `src/choice_diagram.rs`: selected-value descriptions, related-setting context and matching native animations.
- `src/storage.rs`: project loading and atomic persistence.
- `src/rest.rs`: conservative continuous-sweep coverage for automatic cutting start heights.
- `src/simulation.rs`: bounded Facing planning, timed motion, cumulative operation/setup sequencing and rewind.
- `src/pocket.rs`: selected-floor polygon union/offsets, islands, depth layers, profile ramps, direction and reach/access guards.
- `src/surface.rs`: connected planar mesh regions, model-bound selections and planar geometry checks.
- `src/cam.rs`: bounded inside/outside contour, circular-hole detection/drilling, flat/ball drop-cutter parallel and constant-Z waterline previews; geometric and playback regressions.
- `src/volume.rs`: persistent cutter history, setup-frame transforms and approximate removal-volume grid.
- `src/stock_surface.rs`: continuous cutter implicit fields, adaptive octree meshing and detail levels.
- `src/stock.rs`: part placement, six-sided stock offsets, resolved bounds and stock report text.
- `src/smoke.rs`: renderer smoke workflow and application action tests.
- `assets/op-*.svg`: the eleven original operation icons from the web prototype.

## Next engineering stages

1. Keep exact CAD topology in the application, extend imported geometry selection beyond tessellated patches, and implement exact stock/fixture placement.
2. Extend the six bounded operation previews into a production 3-axis CAM engine with exact boundaries, robust linking, additional strategies, rest-aware path optimization and fixture collision checks.
3. Implement rotary and 5-axis kinematics, limit checking, singularity handling and machine simulation.
4. Add more postprocessors (GRBL, Fanuc…), arcs and canned cycles, each with the same replay verification, and validate them on known machines.
5. Connect OpenAI planning to a typed, validated planning interface. Retain the review/apply workflow and use OS credential storage or a backend for secrets.

The current parameter checks are UI configuration checks, not machine safety or manufacturability verification. Several stock, fixture, compensation and collision-preference fields currently record intent only. The example AI brief is a placeholder; structured coverage and priority controls drive the local plan. The native JSON schema is separate from the earlier web prototype schema.

## Third-party components

STEP support uses dynamically linked Open CASCADE 7.9.3. See [THIRD_PARTY.md](THIRD_PARTY.md) for licenses, source links and rebuilding details. Bundled library notices are also inside the app at `Contents/Resources/ThirdParty`.
