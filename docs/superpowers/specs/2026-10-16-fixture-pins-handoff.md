# Handoff: fixture pins (2026-10-16)

Self-contained brief for an agent picking this up on another computer. Start from branch `t3/fixture-pins` (based on `main` at 1662ed8). Implement the plan below, open a PR against `main`, and have it reviewed before reporting done (the MacBook setup uses an acceptance reviewer, "astra": an independent check that the user's issue is really solved, with pictures and tests).

## The user's answers (do not re-ask)
- Request: "set location pins so zeroing will not be needed for every case".
- Pins are **physical dowel pins**. A fixture plate is machined, holes are drilled in it and pins pressed in. The part locates on the pins, so after a flip its position and orientation are known exactly.
- The part locates by **holes in the part** that drop over the pins (not edges against pins).
- Pin positions are **entered by hand** (the app does not make the plate job).
- **Z is still touched off** each setup (pins give X/Y only).
- Re-finding the fixture: "we will need to fix the zeroing process so we will be able to zero the machine". The machine (JY5800 + UC100) has **no home or limit switches**, so machine zero is only "jog to the ends". The plan below makes a reference pin act as the machine's home for X/Y.
- The UC100 driver is the private git dependency github.com/liorknafo/UC100-driver (pinned rev in Cargo.toml). If the simulator needs pins (piece 4), change the driver there, push it and bump the rev.
- Never move the real machine unless the user is at it; test on the simulated UC100.


## Changed after review (17 October 2026)
The user: "the pins holes should be already part of the original design, in the app we should need to select the holes in the part". So the typed `pin1_x`/`pin1_y` of piece 3 and the drill source "Pin holes for <setup>" of piece 4 were replaced: the part's design has the holes, the Work zero step lists the round holes the part opens on its bottom and the user chooses them (in the list or by clicking them in the 3D view, setup key `pin_holes`); pin 1 is the chosen hole from which the plate puts every pin into another; an ordinary Drill on the face they open on drills them. See README "Fixture pins".

## Plan: Context
Today every setup's X/Y work zero is found on the stock with Probe corner, edge or centre, or by hand. After a flip it has to be found again.

The JY5800 has no home switches, so machine zero is only as good as "jog to the ends". Saved G54–G59 offsets are only trusted while nothing moved the machine.

The user wants the classic dowel-pin workflow:
1. Machine a fixture plate, drill holes in it and press pins in (done by hand; pin positions typed in).
2. The part has holes that drop over the pins, so its position and orientation are known exactly, also after a flip.
3. Z is still touched off each setup (user's choice).
4. The machine-zeroing process is fixed so the pins' place on the table is trusted again after an E-stop or power cycle.

## Plan: Design (four pieces)

### 1. The pin fixture: Machine library, per machine profile
- New machine fields, stored like the probe settings in `Machine.p`, parsed by a `pins_settings(&Fields)` next to `control::probe_settings` (`src/control.rs:168`):
  - `pins`: the pin list as "X Y; X Y; …" in the plate's own coordinates (mm, as drilled). Pin 1 is the **reference pin**.
  - `pin_diameter` and `pin_height` (height above the plate).
  - `pin_ref_x` and `pin_ref_y`: pin 1's machine X/Y. These are recorded by the app (piece 2), never typed.
- UI: a "Fixture pins" block on the machine's UC100 step, beside the probe fields. Help texts go in `help.rs`, next to `probe_tip`.
- The plate is assumed square to the machine axes. Piece 2 measures this and reports how far off it is.

### 2. Fix machine zeroing: the reference pin is the machine's home (Control page)
- New probe routine **Pin centre**, an outside boss. It is a new `Feature::Boss` arm in `Routine::plan/zero/found` (`src/control.rs:~720-880`), patterned on `Feature::Bore`:
  1. Touch the pin top.
  2. Rise. Go out past the pin radius plus the tip radius plus a margin on −X, down to the top minus min(3, h/2), and probe +X. Repeat from +X, then from −Y and +Y at the middle found.
  3. Return over the centre.
  - It is also offered on its own under **Probe a feature** as "Boss centre", which gives a general boss/pin finder.
- In **Machine zero**:
  - **Record the pins' place** (first time, after a normal machine zero): probes pin 1, stores `pin_ref_x/y` in the profile, then flies at a safe height to pin 2's expected place and probes it. It reports the plate's angle as "pin 2 is 0.04 mm off the line in Y over 100 mm", and amber over 0.05 mm.
  - **Reference X Y from pin 1** (every session, or after an E-stop): probes pin 1 and declares machine X/Y so its centre is at `pin_ref_x/y`, using the existing `declare` queue and `SetMachine`. It is allowed before X/Y have a machine position (short probe moves from where the tip was put). Z still gets its machine zero at the top of travel, as now.
  - Every saved G54–G59 X/Y offset, the soft limits and the pins' place are exact again.
  - `Connection` gets `pin_referenced: bool`, cleared on E-stop and disconnect like the machine position.

### 3. A setup that sits on the pins (setup editor, Work zero step)
- New setup keys: `on_pins` ("Yes"/""), plus `pin1_x` and `pin1_y`, where pin 1 stands in this setup's **work coordinates** (typed by hand: the spot on the part where pin 1 goes).
- From these and the pin list:
  - Every pin's place in the setup's plan frame: `datum + pin1_work + (pin_i − pin_1)`.
  - The work offset: X/Y offset = `pin_ref` − `pin1_work`. This is exact and needs no probe.
- Checks go in `post::zero_check` (`src/post.rs:1547`) and `Setup::issues`, worded like today's refusals:
  - The machine has no pins, or pin 1 is not set.
  - A pin is not under material of the stock.
  - The pins are not vertical in this setup (a side setup).
  - **No earlier setup drills holes for the pins.**
    - Holes count when they come from a drilling op whose holes, carried into this setup by `volume::Frame::transfer` (`src/volume.rs`), cover every pin.
    - They need diameter ≥ pin Ø, must open on this setup's bottom, and go at least the pin height deep.
    - This is a warning, not a refusal: the holes may have been drilled off the machine.
- The pins become holding geometry:
  - `holding::boxes` (`src/holding.rs:514`) adds a pin-sized box per pin, from the stock bottom up to the pin height.
  - So the viewport draws them, and export, the plan check and the probe path refuse tool moves into them through the existing `hits`.
  - This is independent of the Vise/Clamps choice, which still holds the part down.
- `zero_check`'s probe becomes "the pins locate X/Y". The editor's note says so, and Probe corner/edge/centre stay available.

### 4. Drilling the pin holes, and running
- **Drill op hole source "Pin holes for <setup>"**, a new option in `cam::drill` (`src/cam.rs:660`).
  - Holes are the pins of the named on-pins setup, carried back into this setup's frame. They are not holes found in the mesh.
  - Depth is pin height + 1 mm from the face they open on, or Through.
  - This drills the registration holes in the waste or in the part from setup 1, which is zeroed by probing as now. The same check as piece 3 then sees them.
- **Run panel** "Work zero on the stock" (`ui::work_zero_steps`, `src/ui.rs:5374`), for an on-pins setup:
  1. "Reference X Y from pin 1" (done this session).
  2. Select G5x.
  3. **Set X Y from the pins**, a new `Action::PinZero` that writes the X/Y offset by SetWork at the current position.
  4. Touch Z off as now (Probe Z or Zero Z).
- `run.rs` `problems()` refuses Cycle Start for an on-pins setup until `pin_referenced` is true and the X/Y offset equals the pin-derived one. Its existing "X/Y work zero changed" rule stays.
- Simulated machine: the simulator's block gets pins at `pin_ref` so the whole flow is exercised. Pin centre finds them because the simulated probe trips inside the pin's cylinder (driver `SimBlock` support, if it is only boxes: model each pin as a small box, which is fine for a ±X/±Y centre).

## Plan: Files
- `src/control.rs`: pin settings, Boss routine, Record and Reference actions, `PinZero`, `pin_referenced`.
- `src/ui.rs`: the Machine library pin fields, the Machine zero buttons, the Work zero step "Sits on the pins" block, the Run steps.
- `src/post.rs`: `zero_check`, and the offset from the pins in `Checked`.
- `src/holding.rs`: pin boxes.
- `src/cam.rs`: the pin-holes drill source.
- `src/run.rs`: the start checks.
- `src/model.rs`: setup issues.
- `src/help.rs`, `README.md`, `docs/VALIDATION.md`.
- `src/smoke.rs`: the end-to-end simulated flow.
- Backward compatible: string keys only. Old projects and profiles lack them, so pins are off.

## Plan: Verification
- Unit tests (`cargo test`) cover:
  - Boss routine geometry, including the tip radius.
  - Reference math: machine X/Y re-declared from the probed pin.
  - Pin-derived offset for an unflipped setup and for a setup flipped about X and about Y (hole pattern carried by `Frame::transfer`).
  - Every refusal, and the drill pin-holes source and its depth.
  - Holding boxes hit a contour that runs over a pin.
- Smoke test on the simulated UC100:
  1. Record the pins.
  2. E-stop, rough machine zero, then Reference from pin 1: the X/Y error matches the injected offset.
  3. Setup 1 drills pin holes.
  4. Setup 2 is flipped on the pins: Set X Y from the pins, Probe Z, then a Cycle Start check passes and the program's coordinates land on the same part points as setup 1's.
- Render the viewport for setup 2 with the pins drawn and the Run panel steps, and check them as a machinist would (memory: verify-visual-complaints).
- astra must ACCEPT before reporting done.
