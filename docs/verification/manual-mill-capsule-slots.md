# LIO-445: Manual-mill capsule counterbores and through slots

Implementation and simulated acceptance record for Linear LIO-445.
The orchestrating user subsequently confirmed the issue exists and authorized
push, PR, two independent review stages and squash merge.

## Problem and reproduction

In build 33814e9, Box picked on the adapter's lower-left capsule floor
produces approximately X -61.832..-43.868, Y -21.9..-8.9, Z 0..-10.
The intended 18 × 13 capsule has cutter centres X -55.35..-50.35,
Y -15.4 with a Ø13 centre-cutting slot drill. Box rejects the saved nominal
bounds at X -60.81, Y -20.68, Z -1.50. A rectangular Box is not an editable
round-ended slot workflow. Keep Box and LIO-443 outside corners unchanged.

## Required behavior

- A first-class manual Slot operation: editable geometry, part picking,
  persisted fields, planner support, actual cutter-centre moves and Steps.
- Four 18 × 13 counterbores at X ±52.85, Y -15.4 / 30.4, floor Z -10;
  then four 13 × 8 through slots there, 5 mm centre travel, through Z -20.
- Derive actual starting material from sequential stock. The seat ends at
  Y11.2: the lower row begins at Z -1.5, the upper shoulder at Z0.
- Use the same continuous cutter sweep for part clearance, stock removal,
  and modeled workholding checks; never credit a rectangular corner.
- Refuse unsupported cutter sizes/orientations honestly. Require explicit
  centre-cutting capability for cutting plunges, or prove the entry cleared
  through the full entry depth. Check flute length and stickout honestly.
- Preserve old projects and Box semantics; do not duplicate outside corners.

## Acceptance and protection

Use the real adapter STEP and a uniquely copied project, analytic conservative
fixtures, the exact prior Box failure, sizes/allowances/orientation/depth
boundaries, sequential eight-slot simulation, and headless rendered Steps.
Report measured simulated results separately from physical-machine safety.
Keep modeled vise (8 mm grip, Y axis); parallels remain unmodeled and must
clear all slots, drills and tap lead. No claim of verified physical holding.

Protected: running app PID81426 and /Applications/AXIS CAM Studio.app;
/Users/liorknafo/adapter-plate.axis.json and every workspace.json. No app UI
interaction or launch. Hash protected input files before/after. Build only in
this checkout's private target/ (clone-cargo-cache used); no CARGO_TARGET_DIR.

## Implemented scope

Slot appears in the manual toolbar and has editable/persisted/planner geometry,
X/Y part picking and its own operator Steps. Matching-width flat cutters only:
diameter = width minus twice wall allowance. Undersized/oversized cutters and
rotated slots are refused explicitly. Reorient the setup for a rotated feature.
No general undersize pocket clearing or arc interpolation is included.

A cutting plunge requires explicit tool Center cutting = Yes, or continuous
prior-stock proof of a full-diameter cleared/pre-drilled entry to final depth
(drill point depth alone is insufficient). Missing capability remains unknown;
T69 is confirmed only in the private acceptance fixture, not the saved original.
Flutes and stickout conservatively reach from original stock top to final depth,
including breakthrough. Stock removal and modeled holding use the actual Plan.
The existing Box failure remains truthful for its rectangular requested cut.

## Measured acceptance

Checkout: `/Users/liorknafo/.t3/worktrees/axis-cam-desktop/fix-manual-mill-capsule-bfa83d272071cd35`.
Unique private fixture/evidence: `/var/folders/fr/14gjzx5n02jdvr_25bl_6h8w0000gn/T/axis-capsule-dqdaiv05`.
Inputs are copied `adapter-plate.step` and `adapter-plate.axis.json`;
`capsule-acceptance.axis.json` contains the original operations plus eight Slots.

- Exact saved Box failure replayed: `The box cuts into the part at X-60.81,
  Y-20.68, Z-1.50.` (`acceptance.txt`).
- Four 18 × 13 counterbores then four 13 × 8 through slots simulate sequentially.
  Lower counterbores start at -1.5 and take 17 half-mm passes; upper shoulder
  counterbores start at 0 and take 20. All end at -10. Each through slot starts
  at -10 and takes 20 passes to -20. Total: 154 slot passes; 228 guide steps,
  14 enabled operations. No uniform -1.5 assumption on the upper row.
- Cutter centres: left X -55.35 → -50.35, right X 50.35 → 55.35, at Y -15.4
  or 30.4. Exactly 5 mm centre travel. Maximum XY float roundoff versus nominal
  hundredth-DRO geometry: 0.0000062918592 mm.
- All 20 real face picks pass: each counterbore floor and both ends, plus both
  ends of each through opening (`picking.txt`). Pairing requires both connecting
  straight walls; two same-row synthetic slots remain separate under triangle
  order permutations, including every end facet.
- Independent OCCT B-rep booleans against the real STEP: nominal hundredth-DRO
  sweeps have 0 mm³ intersection for all eight. Actual f32 plans, inset by the
  stated 0.00003 mm radial / 0.0001 mm axial envelope, also have 0 mm³ for all
  eight (acceptance threshold 1e-7 mm³). Raw f32 intersections are NOT literally
  zero: sum 0.00626656217707 mm³, maximum single 0.00112314115337 mm³.
  +0.001 mm end shifts detect 0.0801846–0.130506 mm³ interference;
  +0.001 mm cutter radius detects 0.351359–0.508438 mm³ (`brep.txt`).
- The planner's analytic arc support checks the entire sweep continuously;
  triangulated wall sagitta is not general clearance slack. Only actual chord
  circular segments are removed from horizontal mesh caps before strict tests.
  Numerical bounds: radial 0.00003 mm, axial 0.0001 mm; partition slivers below
  0.00001 mm² are ignored. Analytic regressions reject subfacet end shifts and
  oversize, test flat sides, allowances, blind/through and flute/entry limits.
- Ideal areas: CB 197.732289614 mm², through 90.265482457 mm². Ideal post-seat
  CB removal 7316.094715724 mm³; additional through removal 3610.619298297 mm³;
  total 10926.714014022 mm³. Voxel removals are approximate: CBs 1699.23047,
  1699.23047,1961.34766,1961.34766; throughs 949.03906,949.03516,864.67969,
  864.67969 mm³. These are not substituted for analytic/B-rep evidence.
- Sixteen actual Steps PNGs rendered headlessly, first/last pass for all eight
  (`steps-07-first.png` through `steps-14-last.png`, `rendered-steps.txt`).
  Visually assessed readable centre DRO, work zero, row-specific material start,
  final depths, T69/T70, 1600 rpm, feed140 and entry35 mm/min, compensation OFF,
  lift/position/feed/retract ordering and clearance warnings. No UI app launched.
- Setup3 modeled Vise, grip8, axisY is preserved; full-plan holding validation
  passes. Synthetic clamp probes prove rectangular uncut corners remain clear
  while a clamp across the capsule end is rejected.

## Reproduce

From this checkout, with the unique private fixture directory as `F`:

```sh
AXIS_CAPSULE_FIXTURE="$F" cargo test slot::tests -- --include-ignored --nocapture
AXIS_CAPSULE_FIXTURE="$F" cargo test render_manual_capsule_adapter_steps -- --ignored --nocapture
cargo test manual_slot_schema_and_edit_roundtrip_preserve_all_geometry_and_depths
cargo test manual::tests
cargo test corner::tests
cargo test model::manual_setups
cargo clippy --tests -- -D warnings
cargo fmt --check
git diff --check
```

Slot core/real-project acceptance passed (7 tests before the additional headless
App toolbar/picking test). Planner roundtrip passed; existing manual regressions
24 passed, outside corners 19 passed, manual model compatibility 45 passed.
The headless App test exercises toolbar presence, AddOp auto-picking, actual
Surface dispatch, sequential-stock top and editable Slot fields.

Independent B-rep checker (Homebrew OpenCASCADE):

```sh
c++ -std=c++17 -O2 scripts/verify_manual_capsule.cpp \
  -I/opt/homebrew/opt/opencascade/include/opencascade \
  -L/opt/homebrew/opt/opencascade/lib \
  -Wl,-rpath,/opt/homebrew/opt/opencascade/lib \
  -lTKDESTEP -lTKXSBase -lTKPrim -lTKBO -lTKBool \
  -lTKBRep -lTKTopAlgo -lTKG3d -lTKMath -lTKernel -o "$F/verify-capsule"
"$F/verify-capsule" "$F/adapter-plate.step" "$F/sweeps.tsv"
```

## Limits

All evidence is simulated/rendered, not hardware acceptance. Confirm the actual
tools' centre-cutting capability, dimensions, runout and reach before machining.
Parallels are explicitly unmodeled; verify that they, bolts, table and tap lead
clear all four through features and any breakthrough. This does not establish
physical workholding safety. No claim of literal zero raw-f32 gouge is made.


Final protection check: original STEP SHA256 is unchanged. The live project and
live workspace drifted during this session; their hashes are recorded below.
No protected file was written by this implementation or acceptance harness.
The cause of external changes is not established by hashes. Acceptance uses the
initial private snapshot, not the changed live document. Final read-only `ps`
finds PID81426 no longer present; this agent never quit or relaunched it.

| Protected input | Initial SHA256 | Final SHA256 |
| --- | --- | --- |
| `/Users/liorknafo/adapter-plate.axis.json` | `1f4af69f848dda3429ede6ee5a0007b2ed5842afaae8212adffe2cba7f584f60` | `b68978b5360122636136ab63d6e8a43b1130da26d677fef2fcfc0075d8c562a2` |
| `/Users/liorknafo/Documents/Codex/2026-11-07-spindle-adapter/adapter-plate.step` | `ac82dee1fb3994a8f3083092455ef79132893c069f1fca599af85e5e707d57b8` | `ac82dee1fb3994a8f3083092455ef79132893c069f1fca599af85e5e707d57b8` |
| `/Users/liorknafo/Library/Application Support/AXIS CAM Studio/workspace.json` | `cfd07e21ec423c7165efe5fee632d20bce322f64ff67ead503bb25cd6e3f862f` | `0b9d4c6bdf22d50b0f909625d57110771ad4f309beae8bdbecc735a4c76287ac` |

Headless toolbar/AddOp/Surface/edit test passed; clippy with `-D warnings` passed.
