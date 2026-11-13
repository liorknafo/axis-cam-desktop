# LIO-455: a manual-mill Slot wider than its cutter

Implementation and simulated acceptance record for Linear LIO-455.

## Problem

The redesigned spindle adapter plate (138 × 127 × 20) needs four round-ended
counterbores 16.5 × 21.5 mm. The largest end mill at hand is Ø12. The Slot
operation (LIO-445) only accepted a cutter exactly as wide as the slot: it
swept one centre line.

## What a narrower cutter does now

At each depth level, from the top of the material left down to the floor:

1. **Sweep** the centre line, as before: down at 25 % feed at one end's
   centre, along to the other's.
2. **Walk round** the slot from the far end's tip: the two sides as straight
   one-axis moves, the round ends as stepped walks one axis at a time.
   `corner::inside_stair` is Round corners' `stair` turned inside a round.
   Each step's corner is the one toward the end's centre, and every reading
   is on the DRO's hundredths, rounded toward that centre, so no reading
   cuts past the round. Where the gap from the sweep to the outline is
   wider than 0.9 × the cutter's diameter, walks step out between them. The
   level's last walk leaves 0.2 mm on the walls.
3. After the last level, **finish** with one walk at size on the floor. The
   cutter's side takes the 0.2 mm off the whole wall, its steps leaving at
   most the operation's scallop (new field, default 0.05 mm).

The walks go clockwise seen from above with the spindle clockwise
(conventional inside a hollow), and the other way with it counter-clockwise.
A cutter as wide as the slot still makes one sweep a level, with unchanged
moves and Steps. A wider cutter, or one within 0.02 mm of the width, is
refused.

Proof against the part: every tool centre the plan feeds through is measured
against the centre line. The capsule of that reach plus the cutter's radius
must be no wider than the slot, and it goes through the existing nominal-arc
part check (`check_part`). For the 16.5 counterbore the reach is exactly
2.2500.

## Steps

Each pass has its own screen (`Plan::slot_passes`, `Step::Slot` with the pass
kind, level, offset, what it leaves, its scallop and readings):

- **Sweep:** where the tool goes down, and along the centre line.
- **Walk:** a close-up from above with the walk and the tool. Each feed has
  its arrow and says what it is: out to the end's tip, straight side, or
  step n of m. Where the machine's DRO has arc holes (`dro_arc`), each round
  end's centre, the radius to the tool's centre and the angles.
- **Check:** length, width and depth, then R8.25 with the most the steps
  leave, then the centre.

## Acceptance

- `slot::tests::a_twelve_mill_cuts_the_adapters_16_5_counterbore_to_size`:
  a 16.5 × 21.5 counterbore 8.9 deep, along X and along Y, Ø12, stepdown
  0.5.
  - **Passes:** 18 levels × (sweep + one walk 2.05 out), then the finish
    2.25 out at Z −8.9. Every move is on one axis, and every walk reading is
    on a hundredth.
  - **No gouge:** tool centres stay within 2.25 + 0.00001 of the centre
    line, so the Ø12's edge stays within R8.25.
  - **Floor and levels:** at the floor, every point within R8.20 (the scallop
    off the walls) is within the cutter's radius of a feed, on a 0.05 mm
    grid. Each level is likewise covered up to its 0.2 mm.
  - **Simulation:** the part is fully cut (uncut "0%"). The floor is removed
    at the ends and sides, and the wall 0.25 mm past R8.25 is not.
- `a_wide_slot_steps_out_no_more_than_0_9_of_the_cutter_and_a_ccw_spindle_walks_back`:
  a 30 × 40 slot with a Ø6 takes three walks a level, no more than 5.4
  apart. The floor is covered and nothing is left uncut. With the spindle
  counter-clockwise the walks go round the other way.
- `a_narrower_cutters_steps_give_each_pass_its_dro_numbers_and_the_check_the_round`:
  37 Slot screens. Every walk reading is a DRO number on its screen, and
  each walk is closed at the end's tip (4.55 out, 4.75 for the finish). The
  Check gives 21.5, 16.5, 8.9 and R8.25 with "0.05".
- `corner::tests::an_inside_stair_leaves_at_most_the_scallop_and_never_cuts_past_the_round`:
  R8.25 with a Ø12, at size and 0.2 in, all four quarters, about centres on
  and off the hundredths.
- Existing Slot tests are unchanged except one: a Ø9 cutter in a 10 slot is
  now walked rather than refused. Ø11, Ø10.002 and Ø9.99 are refused.

### The real plate

`slot::tests::the_redesigned_adapters_counterbores_cut_with_a_twelve` and
`steps::tests::render_redesigned_adapter_counterbore_steps` (ignored; set
`AXIS_LIO455_FIXTURE` to a directory holding a private copy of
`adapter-plate.step`). The original STEP is only copied, and its SHA-256 is
the same before and after.

- **Setup:** the plate face up on the manual mill, its zero on the stock
  top's centre, held by two toe clamps on −Y and +Y (as the user's project
  holds it), clearance 40.
- **Picked:** the four counterbores were picked from the model at X ±52.85,
  Y 4.10 / 49.90, 21.5 × 16.5 along X. **The STEP's counterbores are 7.0 deep**
  (floor 13 above the back face, as `params.json` CB_FLOOR says), not the
  8.9 in the issue; the 8.9 case is the synthetic test above.
- **Cut:** each with the Ø12 in 14 levels, 29 passes. The finish leaves at
  most 0.0443 mm between steps, and tool centres stay within 2.2500 of the
  centre line.
- **Checked:** the floor is covered, and the simulated walls past R8.25 are
  untouched.
- **Then the slots:** the four 8 × 13 through slots, with a Ø8 slot drill as
  before.
- **Uncut:** no region is left in or round any counterbore or slot. What is
  left is the seat and bore side of the plate, the M8 threads and two
  corners, none of them cut here.

Rendered Steps pages, at 1440 × 1000:

- `01-tool`
- `02-level-1-sweep`
- `03-level-1-outline`
- `04-level-1-outline-walking`
- `05-finish`
- `06-finish-walking`
- `07-check`

This is a simulated acceptance. The real cutter, chip clearing, the parallels
under the through slots and the clamps as actually placed are for the
operator to check.
