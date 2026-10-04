# AXIS CAM Studio — Rust / Iced

A working native desktop **CAM workflow prototype**, built with Rust and Iced 0.14. It carries the web prototype's Manufacture-style layout into a native application, with local persistence, editable operation parameters and a 3D model viewport. There is no embedded browser.

**G-code output is LinuxCNC 3-axis only.** Six imported-part operations have bounded motion previews with approximate stock removal: Facing, Pocket, inside/outside Contour, Drilling, Parallel and Waterline. Their exact simulated moves can be exported as one .ngc program per setup after checks and a round-trip replay (see below). Adaptive, Scallop, Indexed, Rotary and 5-axis remain explicitly unavailable for imported machining. Fixture, clamp and machine-body collisions are not checked, and production CAM remains pending. The example project's other paths are demonstrations. The AI planner signs in with a ChatGPT account and proposes a whole job for an imported part; the app generates, simulates and checks every proposal before it can be applied (see **Plan a job with AI**).

![Native workspace](docs/workspace.png)

## Run

The app bundle must be built from this source with `./scripts/package-macos.sh` (see below): a bundle built before the LinuxCNC G-code export and the AI planner has neither. Bundle status: not yet rebuilt from this source; this sentence is updated when it is. A bundle is an Apple Silicon build for macOS 26 or later (the bundled CAD libraries require macOS 26). It includes its STEP reader and runtime libraries; end users do not need Homebrew or Python. Double-click it to open. It is ad-hoc signed, not notarized for public distribution.

To build from source with a current stable Rust toolchain:

```sh
brew install opencascade
cargo run --release --locked
```

The build was tested with Rust 1.97.1 on Apple Silicon. The STEP helper uses a C++17 compiler and Open CASCADE 7.9. The HTTPS client builds aws-lc, which needs a C compiler and CMake. Set `OCCT_PREFIX` to a custom installation prefix if needed. `AXIS_SKIP_STEP=1` explicitly builds without STEP support; the macOS packaging script requires it. Windows and Linux builds have not been verified and may need platform-specific helper build/link changes.

Create a macOS bundle:

```sh
./scripts/package-macos.sh
```

The script builds the release binary, bundles the STEP helper with its libraries, the CAD resources, the icon and the license notices, signs the bundle ad hoc in a temporary staging folder and verifies the signature (deep, strict), then writes `dist/AXIS CAM Studio.app`. An optional first argument overrides that output path. Set `AXIS_PACKAGE_ZIP` to an absolute ZIP output path to archive the verified bundle before copying it into a synced folder. It needs Xcode command-line tools and Python 3. `CARGO_TARGET_DIR` can keep build artifacts outside this directory.

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
| Planner | One click asks an OpenAI model, on your ChatGPT plan, for setups, operations, tools and cutting parameters; the app checks every proposal with its generators, simulation and export checks, sends failures back for correction, and shows a passing plan for review, apply and undo |
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

Separate paths retract to clearance before moving in XY. These four strategies do not expose inactive lead-in or stay-down controls. Inside Contour also supports profile-ramp entry through the shared planar boundary kernel. Spindle RPM and coolant are written to the LinuxCNC program (S with M3 or M4; M8 for Flood, M7 for Mist, the machine's Air output for Air, then M9); the preview does not simulate coolant. Feed affects estimated motion time. Every preview inherits earlier enabled cuts, including cuts in earlier setups. Fixture and machine collision solvers remain absent; export only checks that the holder stays above the stock top. Depth planning now skips fully cleared top layers using earlier flat-cutter sweeps. General rest machining that isolates leftover islands or corners is still unavailable, and fixture or machine collisions are not checked; the moves can be exported as LinuxCNC G-code from the NC program page.

Generation limits: 30,000 moves for the new kernels; up to 300 waterline layers; 1,000 contour levels or drill pecks per hole; 500,000 finishing samples and 60 million triangle/sample evaluations. Finishing grid spacing below 0.05 mm is rejected. Polygon offsets include an additional margin of max(0.02 mm, 1% of cutter radius). These limits bound a desktop preview; they do not establish production tolerances.

![Whole-job operation checks](docs/operation-audit.png)

## Contextual help for choices

Hover a choice field to see **the selected option in the title** and its specific behavior. For example, Profile ramp explains a descending contour entry; Plunge explains a straight vertical entry. Changing the choice updates the text and illustration. Milling-method help also reads the saved spindle rotation; Facing explains when Both ways overrides the one-way method preference. Legacy values use the same resolved defaults as the visible control.

Selected-state animations cover spindle rotation, Climb/Conventional, one-way/zigzag facing, keep-down/retract links, ramp/plunge entry, four drilling cycles and cutter containment. Setup, machine, tool-type, coolant, stock mode, playback-speed and display choices also have option-specific text. Saved preferences are distinguished from implemented behavior. These are explanatory animations, not previews of the current job.

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

Six operations generate imported-part motion and approximate stock removal within the scopes below. Production toolpaths, collision checks and the five explicitly unavailable strategies remain future work. To try the illustrative paths, choose **Start → Explore example project** explicitly.

## Try the example workflow

Open **Start → Explore example project** first. This loads the housing, sample library entries and example operations. Your saved tools and machine profiles stay in the library without duplicates: a sample tool that matches one of yours apart from its number takes your number, your other tools keep theirs, a sample tool whose number is then taken gets the lowest free one, and the status bar names the tools and machines added from your library.

1. Open **Machine library → Set up a machine**, complete the wizard and save. Assign the profile to the current setup.
2. Click **Edit setup** to configure stock, work offset, origin, fixture and material.
3. Choose an operation from the ribbon. Set the cutter and feeds/speeds.
4. Open **Geometry**. Select sample features from the list or click them in the viewport. Drag still orbits the model.
5. Set Heights, Passes/Cycle, Orientation when applicable, and Linking.
6. Click **Generate sample preview**, then **Play**. Previews are invalidated when job parameters change.
7. Duplicate a setup, use **Flip X 180°** or enter any rotation/translation, select its stock source and add reclamping instructions. Repositioned setups are saved and visualized; their toolpath calculation remains pending.
8. The AI planner works on imported parts, not on this example (see **Plan a job with AI**).
9. Open **NC program** to review configuration issues and export the setup sheet. G-code export is refused for the example project; it is for imported parts (see **LinuxCNC G-code export**).
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
- **Tool → Spindle rotation** selects **Clockwise (CW)** or **Counterclockwise (CCW)**, viewed from the spindle toward the part along setup −Z. Match the cutter's cutting hand. **Passes → One way → Milling direction** selects **Climb** or **Conventional**, relative to the nominal uncut side of each row. Changing spindle rotation reverses one-way cutting strokes for the same method. Both ways is explicitly **mixed climb/conventional**; the spindle does not reverse between rows. Entry and full-width engagement may contain both modes even in one-way paths. Animated hover illustrations compare cutter feed with a fixed CW spindle. The simulation identifies its method and rotation; it does not animate actual RPM. Spindle commands are written only to the exported G-code.
- New operations default to CW and a Climb preference for One way. Missing rotation/method settings in older one-way jobs resolve to CW/Conventional to preserve their existing motion; their path does not silently reverse. Both settings are saved with the operation and invalidate its preview when edited.
- **Linking → Between facing passes → Keep tool down** feeds between neighboring rows at cutting depth. With Continue from previous end it feeds vertically down at the last endpoint and cuts the next layer in reverse. **Retract each pass** lifts to clearance between rows. One way always retracts for its returns and repeats each depth from the first row. The new presets default to Both ways, Continue from previous end and Keep tool down; older saved operations retain Both ways, Return to first start and Retract each pass until changed.
- Initial entry and retracted links traverse at clearance, approach the retract plane, then plunge. Cutting uses the specified feed, plunging uses 25% feed and rapids assume 6,000 mm/min. Timing omits acceleration, spindle ramp-up, tool changes and controller behavior. Lead-in curves and fixture avoidance are not implemented. Keep-down links follow straight lines inside the convex rectangular cutter-center boundary; no collision checking is performed.
- Continuous flat/ball cutter sweeps subtract material from the initial stock. An adaptive octree and dual contouring produce the visible surface. Amber marks original exterior faces; green marks cut surfaces. Rewinding reconstructs the earlier state; Restart restores the stock left by preceding operations and setups. The volume readout is a separate coarse estimate.
- With Stock enabled, the viewport shows the remaining blank, so unmachined pockets remain filled. Hide Stock to see the reference model. Toolpath displays the latest cutting pass and active rapid. The cutter uses saved diameter, flute length and stickout; the holder uses its saved diameter with an illustrative 8 mm collar length.
- Edits to geometry, tools, stock and operation parameters invalidate the preview. Unsupported operations show an unavailable state instead of inert playback controls. Executable LinuxCNC output is on the **NC program** page (see **LinuxCNC G-code export**), and the AI planner checks for gouges only along the tool's centre line and coarsely on the stock grid (see **Planner checks without AI**). A single combined-program timeline, full collision/gouge verification and simultaneous multi-axis simulation are not implemented. The top-plane guard is a limited geometric check, not a manufacturability check.

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
- The header names the work zero, the stock, the fixture and, when the setup has notes, a `Workholding:` comment with them. LinuxCNC runs a comment that begins with `MSG,`, `LOGOPEN,`, `PROBEOPEN,` and the like as a command, so every comment that carries your text or the planner's (the job title, setup and operation names, notes, the fixture, tool and machine names) begins with a fixed word and a colon, such as `Job:`, `Operation 2:` or `Before starting:`. Such a comment is split over lines of at most 200 characters and never cut off; each later line begins with the same word and `, continued:`, such as `Workholding, continued:` or `Before starting, continued:`.
- For Model origin, which is a point in the CAD file, the header gives the part's box in work coordinates and the stock top's Z, and says to touch off on a part wall or the stock top and enter its coordinate. When an earlier setup has cut the stock top, wholly or in part, the stock top is left out and the header says "The stock top was cut by an earlier setup; touch off on a part face." The plan preview locates a Model-origin zero from the part's −X −Y corner and the stock top, or from the part's top with the same sentence when the stock top was cut.
- Machine profile → Controller step → Postprocessor: tool table (`T# M6` + `G43 H#`) or touch off each tool (one program per tool run: run G49 in MDI before touching off, touch the first tool off at the work zero with AXIS Touch Off, not Tool Touch Off, note the Z of a fixed reference such as a vise jaw, and touch each later tool off on that reference; AXIS cannot touch off while a program is paused), safe retract (`G53 G0 Z0` or the setup clearance plane), path blending (`G64 P`, or `G61` at 0), spindle start dwell and the Air output (M7/M8). Setups keep a machine snapshot, so assign the profile again after editing it.
- Tools have a number (T#) in the tool library. Tools without one (older libraries) are numbered 1, 2, 3… in library order when the project is opened; Open, Recover previous project, the workspace at start-up and `--export-gcode` then say so and ask you to check each tool against your LinuxCNC tool table.
- Stock-based work zeros (Stock top center, Stock top front-left) are refused when earlier setups have cut the stock's faces, or when the stock was defined in an orientation that is not square to the setup (a rotation that is not a multiple of 90° about each axis), because the zero would not be on a stock face. Choose Model origin for that setup.
- A stock-based zero also needs the spots you touch off on to be intact, not only its faces, and the work-zero line of the header says where they are. Stock top center: "Touch off Z0 on the stock top at its centre and X0 Y0 from the four sides within N mm below the top." Stock top front-left: "Touch off Z0 on the stock top within 10 mm of the front-left corner and X0 Y0 from the -X and -Y sides within N mm below the top." N is 10 mm, or a quarter of the stock's height when that is less: the band just below the top where an edge finder touches the sides above the vise. The corner area is 10 mm square, or a quarter of the stock's shorter top side when that is less, and begins 1 mm in from both edges. Program 1 in touch-off mode says the same, and the Stock top center and Stock top front-left help cards use the same words.
- Export is refused when an earlier setup has cut one of those spots: the top at its centre or the middle 10% of a side in that band for Stock top center; the corner area, or 5% to 15% of the −X or −Y side from the corner in that band, for Stock top front-left. A side cut only lower down, where the vise holds it, does not count, so a two-sided job whose first setup contours only part of the height can use a stock-based zero on the flipped setup. An earlier Face inside the part outline, for example, leaves a rim of stock at full height but lowers the top's centre, and a narrow rim at the corner does not leave room for a Z-setter or gauge block either: a Z0 touched off there would send the program too deep. The message names the spot: "earlier setups have cut the stock's top face at its centre, …", "… top face near the front-left corner, …" or "… +X side near its centre, …".
- A rapid may descend wholly beside the stock box, where there never was stock; a rapid that descends over the stock must be proven clear by earlier cuts. Fixtures are not checked there either.
- Not checked: fixtures, clamps and the machine body. LinuxCNC's own soft limits check absolute position. Preview each program in LinuxCNC and air-cut first.
- Headless: `axis-cam --export-gcode <project.json> <folder>` runs the same checks and writes the same files.

## Sign in with ChatGPT

The AI planner page has a **ChatGPT account** panel. **Continue with ChatGPT** opens your browser; after you approve, the app can send planning requests on your ChatGPT plan. No API key is used.

- OpenAI shares your name, email and profile picture with the app. The app cannot read your ChatGPT conversations.
- The app registers itself with OpenAI on the first sign-in. It identifies this installation by a random ID in `settings.json`; the ID says nothing about you.
- Requests count against your ChatGPT plan, up to a weekly per-app cap you set at [chatgpt.com/settings/usage](https://chatgpt.com/settings/usage). The panel's **Manage usage** opens that page.
- The sign-in is stored in the macOS Keychain (item `AXIS CAM Studio`). It is never written to project files. Non-secret settings (the chosen model, this installation's ID) are in `settings.json` beside the workspace file. On other platforms the sign-in is an owner-only file in the same folder; those platforms are unverified.
- **Check connection** sends one tiny request to the selected model. **Sign out** deletes the saved sign-in and asks OpenAI to revoke it.
- After rebuilding the app from source, macOS may ask once to allow the new build to read its Keychain item.
- Copies of the app share the saved sign-in. Before refreshing it, a copy reads the saved sign-in again; if another copy has already refreshed it, that one is used (and refreshed only if it is itself about to expire or was rejected). A saved sign-in is used only when it is newer than the copy's own, and a copy never sends a refresh token it has already used, even when a failed save left that token in the Keychain.
- Closing the window while signing out, while a new sign-in is being saved, or while the sign-in is being refreshed (from the request to OpenAI until the renewed sign-in is saved), waits for that to finish, at most 10 seconds, and the status bar says so. A sign-out still pending after the 10 seconds keeps the app running past the wait: the window closes, the saved sign-in is deleted, and the app ends when that has finished; a refresh still running then cannot save the sign-in again, and OpenAI is not asked to revoke it. A sign-in still being saved then is finished the same way, and so is a refresh that already has OpenAI's answer and is saving the renewed sign-in. A refresh still waiting for OpenAI after the 10 seconds no longer holds the close: the window closes, or, while a planning stage, the NC program check or the stock mesher is running, the app ends at once; the refresh is dropped and you may need to sign in again. Only closing the window waits like this: quitting with Cmd+Q (Quit in the app menu) ends the app at once, without waiting for sign-in work, because the app's UI toolkit (Iced 0.14) does not let it intercept Quit.

If you sign in without allowing plan usage, the panel says so; sign out and sign in again to allow it.

If OpenAI's model list is empty or cannot be loaded, the app keeps the saved model, or uses `gpt-6.1-sol` when none is saved, and says so in the panel. **Check connection** shows whether that model answers, and **Load models** tries the list again.

## Plan a job with AI

**AI planner → Plan this job** asks the selected model for a complete job and checks it before you see it. It needs an imported part that is a closed solid; setup 1 complete as you have it (a name, a work offset, a 3-axis LinuxCNC machine, stock, and rotations that are multiples of 90°); a material; a tool library with unique tool numbers; and a ChatGPT sign-in with plan usage. The checklist on the page shows what is missing; the part's closed surface is checked when you click Plan.

What happens after the click:

1. The app builds the part report on your computer (see **Planner checks without AI**).
2. The model proposes setups, operations, tools and cutting parameters. It can use only your tools and the six available strategies, and it is told the limits the app's generators enforce, such as each operation's 30,000-move limit and the stepdowns that keep within it.
3. The app builds the job, generates and simulates every operation, and runs every check described in the next section.
4. Problems go back to the model for correction. If a plan passes but faces are still covered by stock, one more request asks the model to machine them or to list them as not machinable. A run uses at most four model requests and can be cancelled at any time.
5. A plan that passes is shown for review: setups and operations with estimated times, where a Model-origin work zero lies on the part, the values each operation will run with (including coolant), the checks that passed and what they leave out, what is still covered by stock, what the planner could not machine and why, its warnings, and a 3D view of the stock after the whole plan. The view has its own camera; turning it does not move the machining viewport. Text written by the model (its summary, reasons, workholding notes, warnings and the Not machined list) is labelled as the planner's, and so are the names it gives operations and later setups ("Setup 2 · named by the planner: “Underside”"); those names keep only letters and digits of any script, accents and other marks on a letter, and - _ . , / ( ) # + : & % ~ < > = ± ° ⌀ ≤ ≥ ("Slot <6 mm", "Chamfer 45°", "Bore Ø10 ±0.02"). Fullwidth letters, digits and punctuation count as their ASCII forms ("粗加工（左）" becomes "粗加工(左)"), dashes and minus signs become - ("Holes 1-4", "Z-0.5"), ～ and 〜 become ~ ("穴あけ 1~4"), the Arabic decimal and thousands separators and percent sign become . , and % ("مته ۳.۵"), fraction and division slashes become / ("1/4 in drill"), ∅ becomes ⌀ and × becomes x; * becomes x between two digits ("10x10", "10 x 10") and is removed elsewhere ("**Roughing** pass" becomes "Roughing pass"); and an apostrophe stays between a letter or digit and a letter ("Bob's", "Op 2's"). Quotation and check marks and the listed look-alikes (letters drawn like quotes or primes, such as ʺ ʻ ʼ ᐥ) are removed from names; other symbols become spaces, so words and numbers never run together ("フェイス・トップ" becomes "フェイス トップ"). Invisible format characters (the soft hyphen, zero-width characters such as the Persian zero-width non-joiner, direction marks, the word joiner and the byte order mark) are dropped without a space, and so is a middle dot inside Catalan l·l ("paral·lel" becomes "parallel"); elsewhere a middle dot separates words ("드릴·탭" becomes "드릴 탭", "Face · top" becomes "Face top").

While it runs, the page shows the stage and the model the run was started with: analyzing the part, asking for a plan (1 of 4), asking for a correction after a failed check or an unreadable reply, asking about faces still covered, checking the plan, and building the 3D view of a plan that passed. **Cancel** stops the requests at once; the part analysis, a check or the 3D view already under way cannot be interrupted and finish in the background. Until they have, **Plan** is refused: the status bar says the previous plan check is still finishing and asks you to try again in a moment.

Nothing changes until you click **Apply plan**. Applying first keeps the current job in History (**Browse project history** on the start screen), then replaces the operations of setup 1 and removes later setups; setup 1 keeps its name, machine, stock, work zero and fixture, and the planner's workholding note for it is added at the end of setup 1's notes, on the same line ("Soft jaws. · Planner: …"); setup 1 remembers that note, saved with the project, and applying a later plan replaces it where it still stands unchanged and whole (followed by the end of the notes or a line break, or, when the note ends a sentence with . ! ? or 。, by a space) instead of adding another; your own text, including anything you typed before or after the planner's note, is never removed, and a planner note you changed stays as you left it. **Undo applied plan** first keeps the applied job, with your edits since, in History, then puts the previous setups and operations back, and re-adds any tool they use that you deleted since (each keeps its own tool number when no tool in the library has it; one whose number a newer tool has taken gets the lowest number that no tool uses or another re-added tool keeps; the status bar names each); tools, machines, the project title and the material you changed after applying stay as they are. Importing another part, opening or creating a project, or reloading the part ends the Undo (the job stays in History). Editing the job or opening another project discards a preview and cancels a running plan, because both describe the job as it was when you clicked; after an edit the status bar says so, after the edit's own message. Signing out during a run stops it, and the status bar says so too.

What is sent to OpenAI: the part's dimensions and feature list, your tools, machine limits, material and stock size, and on correction rounds the app's check report. The CAD file, file paths, the project title, your setup names and your account details are not sent: the checks run on a copy of the job without the title and with every setup named by its number (Setup 1, Setup 2), and the plan you review and apply gets setup 1's own name back. Requests ask OpenAI not to store them (`store: false`).

Limits:

- A plan is only as good as the model's proposal. The app's checks refuse plans that cannot be generated, fail the export checks, send a tool's centre line into the part, go below the stock, or remove part material visibly on the stock grid. They do not check fixtures, clamps or the machine body, cutting loads, tool deflection, or whether the speeds and feeds suit the material.
- The model is asked to list features it cannot machine under **Not machined**; the app does not verify that list. Compare it with the **Still covered** lines and the 3D view before applying. The planner uses only the tools in your library.
- Review each operation and play the simulation before exporting. Export is on the **NC program** page; it runs the export checks and the replay again, but not the planner's tool-axis and gouge checks, which covered the plan as applied, not your later edits.
- If no proposal passes within four requests, the page shows the last problems and nothing can be applied.
- A planning request can take several minutes and counts against your ChatGPT plan.

## Planner checks without AI

Three command-line tools show what the AI planner will see and how its plans are judged. `--part-report` and `--check-plan` need a saved project with an imported part that is a closed solid (a closed surface with no open edges that encloses a volume; its original file must still be where the project expects it), a 3-axis LinuxCNC machine and stock on setup 1 with rotations that are multiples of 90°, a material, and a tool library with at least one tool and unique tool numbers.

- `axis-cam --part-report <project.json>` prints the part report: for each of six orientations (the part axis that points up), the horizontal faces visible from above with their surface IDs, pocket floors with depth and the largest flat cutter that fits, and holes with diameter, depth and whether they go through.
- `axis-cam --proposal-schema` prints the JSON schema of a plan: setups with an orientation and a work zero, and operations with a strategy, a tool number, surface IDs and cutting parameters.
- `axis-cam --check-plan <project.json> <plan.json>` builds the job a plan describes, checks it and prints the report as JSON, with a one-line summary on standard error. Exit code 0 means the plan passes, 1 that it does not, 2 that the check could not run (wrong arguments, an unreadable project or plan, or a requirement above that is not met). The other two commands print JSON alone and exit with 0, or with 2 when they cannot run.

A plan is checked in stages:

1. **References and limits.** Orientations must exist, be used once and start with setup 1's. Tools must exist and suit the strategy. Surface IDs must belong to that orientation and suit the strategy: Facing takes the top plane, Pocket and inside Contour one floor that the cutter fits, Drilling one plane with holes, and the other strategies none. Spindle speed is 100 rpm to the machine maximum and feed is 1 mm/min to the machine maximum. A plan has at most 6 setups, 12 operations per setup and 40 in total. A drill marked as going through must select only holes that go through the part (`hole_diameter` narrows the selection), and its breakthrough must be at most half the drill diameter plus 1 mm. Only setup 1 can drill through: in later setups the part rests on the face where the drill would exit, so holes that go through along X or Y are drilled there with `through` false and their exits listed as not machined. Every number must be finite, and `bottom_offset` and the stock-to-leave values (`radial_allowance`, `axial_allowance`) cannot be negative, so a plan never cuts below the model's bottom except by a through drill's breakthrough. The built job must be one the app can save and reopen.
2. **Generation.** Every operation must generate, with the stock left by the ones before it.
3. **Export.** All LinuxCNC export checks must pass.
4. **Tool axis.** Measured against the part's mesh rather than the stock grid. For every move, sampled about every millimetre of horizontal travel, the tool's centre line above the tip must not pass through more than 0.1 mm of the part, whatever the strategy, and no move may take the tip below the bottom of the stock. In setups after the first, nothing may go below the part's underside either: earlier setups may have removed the stock there, and the part rests on it. This catches a tool of any size that goes deeper than the surface under its centre, such as a drill sent past the bottom of a blind hole. It does not see a cutter's edge cutting a wall beside its centre.
5. **Gouging (coarse).** Cells well inside the part that the stock simulation removed, on the simulation's grid (up to 96 cells on the longest side). The check fails once three or more such cells have been removed in all, and names each operation that removes some. It finds cuts deeper than about one cell by cutters wide enough to cover a cell centre; walls and floors thinner than about three cells are not covered by it.

The part report refuses a mesh with open edges and says how many. Copies of a vertex that differ only by rounding are one vertex. The tolerance covers float rounding and coordinates written as text with 7 significant digits (such as ASCII STL written with `%.6e`): it is 8 f32 steps at the part's largest coordinate, or, if that is more, √3 times the last of 7 significant digits there (0.000017 mm when the largest coordinate is 10–99 mm, 0.00017 mm when it is 100–999 mm). Text with 6 significant digits is not covered, so a mesh with T-junctions written that way is refused; export it as binary STL, 3MF or STEP, or as text with at least 7 digits. Copies are welded only in groups that fit in a box whose diagonal is at most twice the tolerance; the points of a wider group stay distinct, so a row of nearby points cannot weld an opening shut. A triangle written twice with the same winding counts once; an edge used by an odd number of triangles is then open unless shorter open edges along it match it, as where a long edge meets several shorter ones (a T-junction). A part that encloses no volume, such as a sheet written with both windings, is refused too. A vertical line whose only solid is of zero length (a face facing down and one facing up within 0.0001 mm of one height, with nothing solid on either side, as through a double-sided sheet whose two sides are triangulated differently) crosses the surface inconsistently, and a part where more than 2% of the test lines do is refused. A part more than 1,000,000 mm from its file origin is refused with its own message. Where a vertical line still crosses the surface inconsistently even when moved 0.002 mm each way (flipped triangles, overlapping bodies, a zero-length span alone), stages 4 and 5 treat that column as solid from its lowest crossing if that faces down, otherwise from the model's bottom, up to its highest crossing if that faces up, otherwise up to the model top; crossings at one height facing both ways count as facing the way that makes more of the column solid.

Known limit: meshes with T-junctions written as text with fewer than 7 significant digits, or more than about 1 m from their file origin, can be refused as not closed; re-export them as binary STL, 3MF or STEP. Known limit: part analysis can take tens of seconds on meshes with tens of thousands of long sliver triangles in one plane (synthetic fan or T-junction slivers); realistic meshes up to 200,000 triangles take a few seconds.

A problem in stage 1 stops the later stages, and a generation failure skips stage 3. Stages 4 and 5 run on every operation that generated, whatever stage 3 says, so one report lists every known problem. Each problem names its setup and operation where it has them and carries its stage: structure, generation, export, or gouge (used by both the tool axis and the coarse gouge check).

For the operations that generated, the report then gives coverage: how much of the part's surface is exposed, which faces are still covered, and how much stock remains above the part, around it, inside its bounding box (in pockets and holes) and below it. It also gives the estimated machining time per operation and in total. Coverage does not see stock thinner than about 1.5 cells. Coverage and the coarse gouge check are approximate. The generators' own rules, the export checks and the tool axis check are more precise, but none of them is a full collision check. Fixtures, clamps and the machine body are not checked.

Setup 1 stays as you configured it (name, machine, stock, work zero, fixture); the plan's own name and work zero for its first setup are ignored. A plan replaces setup 1's operations and any later setups the project had, and adds its own later setups; they share setup 1's machine and fixture and get the next free work offset, and the planner's workholding advice for each is in its notes.

## Persistence

On macOS, automatic state lives at:

```text
~/Library/Application Support/AXIS CAM Studio/workspace.json
```

Other platforms use the OS local application data directory. Writes use a temporary file and atomic rename. Closing the window saves the workspace first; if it cannot be saved, the window stays open and the status bar says so. When a planning stage (the part analysis, a plan check or the 3D view), the NC program check or the stock mesher is still running, closing ends the app once the workspace is saved instead of waiting for that work. A G-code export is not cut short: the window closes and the app ends when it has finished. A refresh of the ChatGPT sign-in is waited for (see Sign in with ChatGPT). An unreadable workspace is preserved, and a separate recovery workspace is used. Starting a new project, opening another project or loading the example archives the complete previous job in a `history` subdirectory, including unsaved parameter edits. A failed archive leaves the current project in place. An explicit project save is separate from the automatic workspace. The start screen offers the previous job without opening its model automatically; choose **Continue previous project** to resume it.

Older project files without an explicit sample-model flag retain their sample housing when they contain setups and no imported file. Newly saved blank projects stay blank after reopening.

Projects contain the **source file path and metadata**, not an embedded copy of the imported mesh. Keep the original mesh file accessible when reopening a project. No API keys are stored and the app has no telemetry. The only network requests are those of the ChatGPT sign-in and the AI planner (see above).

The `examples` directory includes a sample AXIS project, an OBJ cube, matching four-setup STEP and 3MF parts, an inch-unit STEP block and a placed/rotated STEP assembly.

## Tests and renderer checks

```sh
cargo test --locked
cargo fmt --check
cargo run --locked -- --smoke-test ./smoke-output
# Fast, isolated rendering of field help only:
cargo run --release --locked -- --smoke-test ./help-output --help-only
```

The explicit smoke-test mode uses an isolated workspace, exercises eighty-five screens plus twenty-four camera-motion samples through Iced's renderer and closes itself. The motion samples check that the model remains present across orbit, pan and zoom changes, with stock both shown and hidden. It does not modify the normal workspace. Simulation tests verify continuous motion, parameter-derived spacing/depth/time, stock removal against known volume, rapid moves removing nothing, swept cutter footprint, rewind/incremental consistency, rotated setups, rejected configurations the Play/pause/replay/invalidation lifecycle, both-way continuation at odd/even row counts and varied angles, one-way returns, reduced return time, unchanged final stock, and saved/legacy direction preferences. Imported Facing checks cover real visible-surface picking after setup transforms, planar grouping, selection persistence, replacement and stale-mesh handling, and sample isolation. Unit tests also cover complete tile pixel/gutter preservation below the upload threshold, stable orbit scale, blank startup and project recovery, the first import/machine/setup/tool flow, removal of the last setup, independent stock offsets, rotated and translated stock, stock transparency and occlusion, legacy stock migration, configuration validation, snapshots and dependencies, planner start, cancel, apply and undo, preview invalidation, import parsers, persistence, picking and the LinuxCNC program page (blocked and ready).

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
- `src/rest.rs`: conservative continuous-sweep coverage for automatic cutting start heights, also used by export to prove a rapid's descent clear and to find the stock faces earlier setups cut.
- `src/post.rs`: LinuxCNC postprocessor: blocks from the simulated moves, the emitter and its header (work zero, part box for Model origin, fixture, workholding), file names, and `post_job`/`write_job` for the NC program page and `--export-gcode`.
- `src/post_check.rs`: export checks: spindle and feed limits, travel, rapids into material, holder clearance and touch-off rules.
- `src/ngc.rs`: strict reader for the G-code subset the post writes, and the replay that compares each program move by move with the simulation.
- `src/simulation.rs`: bounded Facing planning, timed motion, cumulative operation/setup sequencing and rewind.
- `src/pocket.rs`: selected-floor polygon union/offsets, islands, depth layers, profile ramps, direction and reach/access guards.
- `src/surface.rs`: connected planar mesh regions, model-bound selections and planar geometry checks.
- `src/cam.rs`: bounded inside/outside contour, circular-hole detection/drilling, flat/ball drop-cutter parallel and constant-Z waterline previews; geometric and playback regressions.
- `src/volume.rs`: persistent cutter history, setup-frame transforms and approximate removal-volume grid.
- `src/stock_surface.rs`: continuous cutter implicit fields, adaptive octree meshing and detail levels.
- `src/stock.rs`: part placement, six-sided stock offsets, resolved bounds and stock report text.
- `src/analysis.rs`: part report (planes, pocket floors and holes for each of six orientations), orientation helpers and the planning preconditions.
- `src/proposal.rs`: plan format and JSON schema, reference checks and building a job from a plan.
- `src/plan_check.rs`: plan stages (structure, generation, export, tool axis, coarse gouge) and coverage.
- `src/planner.rs`: the one-button planner: instructions for the model, requests, correction rounds, the best passing plan.
- `src/oauth.rs`: Sign in with ChatGPT: PKCE, loopback redirect, ID token validation, refresh, revoke.
- `src/session.rs`: session and its Keychain or file store.
- `src/settings.rs`: non-secret app settings.
- `src/openai.rs`: Responses API client on the ChatGPT plan.
- `src/fake_openai.rs`: test-only loopback stand-in for OpenAI.
- `src/smoke.rs`: renderer smoke workflow and application action tests.
- `assets/op-*.svg`: the eleven original operation icons from the web prototype.

## Next engineering stages

1. Keep exact CAD topology in the application, extend imported geometry selection beyond tessellated patches, and implement exact stock/fixture placement.
2. Extend the six bounded operation previews into a production 3-axis CAM engine with exact boundaries, robust linking, additional strategies, rest-aware path optimization and fixture collision checks.
3. Implement rotary and 5-axis kinematics, limit checking, singularity handling and machine simulation.
4. Add more postprocessors (GRBL, Fanuc…), arcs and canned cycles, each with the same replay verification, and validate them on known machines.
5. Extend the planner as more strategies become available for imported parts, and make it aware of fixtures.

The current parameter checks are UI configuration checks, not machine safety or manufacturability verification. Several stock, fixture, compensation and collision-preference fields currently record intent only. The native JSON schema is separate from the earlier web prototype schema.

## Third-party components

STEP support uses dynamically linked Open CASCADE 7.9.3. See [THIRD_PARTY.md](THIRD_PARTY.md) for licenses, source links and rebuilding details. Bundled library notices are also inside the app at `Contents/Resources/ThirdParty`.
