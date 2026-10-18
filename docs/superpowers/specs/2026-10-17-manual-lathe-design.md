# Manual lathe — design

Date: 17 October 2026. Branch: `t3/lathe-planning` (from `main` 114f4c9). Linear: LIO-175.

## Goal

1. The user's **manual lathe** is a machine in the project, with its own tools and chucks.
2. A lathe setup turns **round bar** (or a part coming from the mills): face, turn outside diameters, bore, chamfer, groove, thread, part off, and drill on the centre line from the tailstock. The cuts are simulated, so later setups on the mills start from the turned part, and a lathe setup after the mills starts from the milled part.
3. A lathe setup has no G-code. MACHINE shows it as a **visual step guide**, like the manual mill's, but the lathe has **no DRO**: every number is shown as a **dial picture** (turns plus the dial reading) or as a **measurement** to take.

## Decisions made by the user

- The lathe is **manual, with no DRO** (17 October 2026).
- It does **all of**: facing, outside turning, boring, drilling on the centre, grooving, parting off, chamfers, threads.
- Parts go **both ways**: lathe → mill and mill → lathe.
- Holding: a **3-jaw** and a **4-jaw chuck**. Changing chucks is slow, so the **3-jaw is preferred** whenever it can hold the part.
- The dial collars can be zeroed; the **carriage has power feed** (the cross slide does not); the carriage handwheel has a dial.
- Whether the cross-slide dial reads diameter or radius is **not known**: the app asks once per machine with a **dial check** screen.
- The screens in the mockup of 17 October (`lathe-step-screens.html`) are the direction: one section per operation, dial pictures, a measure step before the last pass, Face as one screen.

## Non-goals (phase 1)

A CNC lathe and G-code for it, a DRO on the lathe, taper turning with the top slide set over (other than threads and chamfers), knurling, between-centres work and the steady or follow rest, finding the turned profile from the part automatically (phase 2), the AI planner (phase 3), and V-blocks or soft jaws for round parts in the mill vise (phase 2: phase 1 holds a round part in the vise on its bounding box and says so).

## 1. The lathe machine

`kind` gains `Lathe`. A lathe keeps `name`, `power` and these fields:

| Field | Meaning | Default |
| --- | --- | --- |
| `speeds` | Spindle speeds, as for the manual mill | `70,110,180,280,450,710,1120,1800` |
| `swing` | Largest diameter over the bed, mm | `250` |
| `centres` | Distance between centres, mm | `550` |
| `cross_reads` | `Diameter`, `Radius` or empty (unknown, until the dial check) | empty |
| `cross_turn` | What the cross-slide dial's numbers go through in one turn (the number at the index after one full turn) | `4` |
| `cross_div` | Divisions on the cross-slide dial | `100` |
| `top_turn`, `top_div` | Top slide, the same | `2`, `100` |
| `carriage_turn`, `carriage_div` | Carriage handwheel, the same | `20`, `100` |
| `quill_turn`, `quill_div` | Tailstock quill, the same | `2.5`, `50` |
| `feeds` | Carriage power feeds, mm per revolution | `0.05,0.1,0.15,0.2` |
| `pitches` | Threads the gearbox cuts, mm | `0.5,0.75,1,1.25,1.5,1.75,2,2.5,3` |
| `pass_time`, `tool_time`, `move_time` | As for the manual mill | `20`, `3`, `10` |

Every `*_turn` is in the dial's own numbers, so the guide never has to know the screw pitch. For the cross slide, `cross_reads` says what those numbers mean: with `Diameter`, a reading of `d` takes `d` off the diameter; with `Radius`, it takes `2d` off. The guide converts every cross-slide target with this rule and shows the reading the dial will show.

`manual::is_manual` stays "the manual mill". A new `manual::is_hand(&Machine)` is true for the manual mill and the lathe; every place that skips the post, the export, the UC100 Run and the G-code checks for the manual mill uses `is_hand`. `lathe::is_lathe` marks the lathe.

## 2. Round stock

A lathe setup's frame puts the **spindle axis along the setup's Z**, with **Z 0 at the free end of the bar** (the end the tool faces) and +Z out of the chuck toward the tailstock. X and Y are radial. The work zero is the existing "Stock top center" spot, so Z values are 0 at the end and negative toward the chuck, and the axis is X = Y = 0.

`stock_type` gains `Round bar` with `stock_diameter` and `stock_length` (and, as for boxes, a relative mode: `stock_radial` added to the part's largest radius about the setup Z axis, and `stock_top`/`stock_bottom` along it). The bounds are the bar's box (`[-R, R]² × [0, L]` before placement), so every box consumer keeps working conservatively; round-aware consumers check `stock_type`.

- **Simulation.** `Volume::new` clears the voxels outside the cylinder when the stock is round. `stock_surface::Snapshot` uses a cylinder field instead of the box field. The 3D view colours a triangle "block" when it lies on the bar's cylinder or end faces.
- **Turning cuts.** `CutterShape` gains `Revolve`: a `Cut` whose `a`/`b` are reused as the (z0, r0) and (z1, r1) corners of a ring in the cut's frame — every point whose axial position is between z0 and z1 and whose distance from the frame's Z axis is between r0 and r1 is removed. A chamfer is a short run of thin rings (0.1 mm steps). `swept`, `Cut::field`, `Cut::extend` (no merging of revolve cuts) and `RestStock` (revolve cuts are not evidence) handle it. Everything that reads earlier cuts through `Frame::transfer` then sees lathe cuts in mill setups and mill cuts in lathe setups.
- **Leftover stock.** A later setup's leftover box comes from `RestStock::left` as today (conservative). A lathe setup after the mills keeps a box stock when the mills left a box.

## 3. Chucks

`holding::KINDS` gains `Chuck` with keys `jaws` (3 or 4), `min_grip` and `max_grip` (the diameters it closes on, mm), `jaw_length` (how deep the jaws reach along the bar), `body` (chuck diameter) and `machines`. Starters: a 160 mm 3-jaw (Ø3–Ø140) and a 160 mm 4-jaw (Ø6–Ø150).

A lathe setup's holding is `Chuck`, with setup keys `grip` (length in the jaws, default the chuck's `jaw_length`) and `stickout` (from the jaws to the free end).

- **Choosing the chuck.** "Suggest" and "Add lathe setup" pick the **3-jaw** when the gripped section is round (round bar, or a section an earlier lathe setup turned) and within its range; the **4-jaw** only for a square or off-centre section, with the chip "4-jaw: change the chuck". The setup editor can override.
- **Checks** (workholding problems): the gripped diameter is inside the chuck's range; the grip is at least a third of the diameter and at most `jaw_length`; the stickout is at least the furthest cut from the end plus 5 mm; a stickout longer than 4 × the diameter warns "Long and thin: support it or cut it in two setups."; nothing is cut inside the grip; the swing is not exceeded.
- **Collisions.** The chuck body is a box `[-body/2, body/2]² × [grip end − 40, grip end]` in the setup frame; tool moves are checked against it as against vise jaws.

## 4. Lathe tools

`Tool` types gain `Turning tool`, `Boring bar`, `Grooving tool`, `Parting blade` and `Threading tool`. Drills are reused (in the tailstock). Their keys:

| Type | Keys |
| --- | --- |
| Turning tool | `hand` (Right/Left/Neutral), `nose_radius`, `lead_angle` (approach, default 93°) |
| Boring bar | `nose_radius`, `min_bore` (smallest bore it enters), `reach` |
| Grooving tool, Parting blade | `width`, `reach` (deepest plunge) |
| Threading tool | `thread_angle` (60 or 55), `pitches` (min–max) |

`Tool::issues` checks the keys of each type and does not ask a lathe tool for flutes or a mill diameter. Lathe tools fit lathes only (`machines`, as for the manual mill).

## 5. The Turn strategy

`Strategy` gains one variant, **`Turn`**, with `turn_kind`: `Face`, `Outside`, `Bore`, `Chamfer`, `Groove`, `Part off`, `Thread`. Drill is allowed on a lathe for holes on the centre line only. On a lathe, only Turn and Drill are offered; Turn is refused on the mills.

Fields (lathe work coordinates, Z 0 at the end, negative toward the chuck, diameters as diameters):

| Kind | Fields |
| --- | --- |
| Face | `face_off` (how much comes off the end), `to_diameter` (0 = to the centre) |
| Outside | `diameter`, `z_end` (the shoulder), `depth` (off the diameter per pass, default 2), `finish` (last pass, default 0.2), `allowance` |
| Bore | `diameter`, `z_end`, `depth`, `finish`, `allowance` (needs a hole first: a Drill or a smaller bore) |
| Chamfer | `edge` (`End outside`, `End inside`, `Shoulder`), `size` (45° chamfer leg) |
| Groove | `z` (left side of the groove), `width`, `diameter` (bottom) |
| Part off | `z` (the cut's side toward the part) |
| Thread | `outside` or `inside`, `diameter` (major), `pitch`, `z_end`, `passes` (default from the pitch) |

Each kind makes a `Plan` with moves in the setup frame (X radial, Z axial, Y 0) for timing and the guide, and **revolve cuts** for the simulation:

- **Outside / Bore:** passes from the current diameter to `diameter` + `allowance`, each `depth` off the diameter, the last equal to `finish`; each pass feeds toward the chuck from Z +1 to `z_end` at the power feed nearest to 0.1 mm/rev.
- **Face:** one ring removing `face_off` across the end (the user plans the passes, as for milling).
- **Groove / Part off:** one plunge to `diameter` (Part off to the centre, minus 0.5 mm, then the part falls).
- **Thread:** infeed per pass from the usual table (the first deepest, ending on the thread depth 0.61 × pitch outside, 0.54 × pitch inside); simulated as a ring to the pitch diameter (the thread form is not simulated).
- **Drill on the centre:** a ring from r 0 to the drill radius, to the depth plus the point.

**Part check.** A Turn operation is refused when a ring it removes comes within its allowance of the part ("The cut reaches the part at Z…, Ø…."), using the part's distance from the axis sampled on rings.

**Time.** Feed per revolution × rpm, plus `pass_time` per pass and `tool_time` per tool change; `move_time` when the part changes machines, as today.

## 6. The lathe step guide

The guide (`guide.rs`) and the Steps page (`steps.rs`) gain a lathe branch. The structure is the manual mill's: sections, one per operation, between **Set up** and **Hand over**; Back / Next; the place is saved; it works with no controller and while the CNC runs. The drawing is a **side view** of the bar (chuck on the left, you at the front), with the cut hatched, the part's profile, the jaws, the tool and the "do not pass" lines. The 3D pictures are as for the mill.

| Step | Shows |
| --- | --- |
| Dial check | Only when `cross_reads` is empty: turn a skin, go in 10 divisions, cut 30 mm, measure; the user taps "the Ø went down by 2 × the reading" or "by the reading"; the answer is saved to the machine (in the library and in every setup's machine snapshot) |
| Hold | The bar in the chuck: the chuck (3- or 4-jaw), the stickout as a big number, the grip |
| Zero | Touch the end → zero the carriage dial; measure the diameter (the user types it); touch the diameter → zero the cross dial |
| Face | One screen: the tool, the gear, the top slide dial for how much comes off, "lock the carriage" |
| Outside / Bore pass | Per pass: the cross-slide dial (turns + reading) for this pass, the carriage dial where to stop (turns + reading), "power feed off 1 mm before", the pass tiles with depths; before the last pass a **Measure** step |
| Measure | The user types the measured diameter; the last pass's cross-slide number is recomputed from it |
| Drill | The tailstock: drill and centre drill, the quill dial depth (turns + reading), peck every 1 × diameter |
| Groove / Part off | The carriage dial for the tool's left side, the cross-slide dial for the depth, "lock the carriage" |
| Thread | The gearbox pitch, the top slide at 29.5° (27.5° for 55°), per pass the top slide dial infeed, the half-nuts rule (stay closed, reverse back) |
| Check, Hand over | As for the mill; Hand over says how the next machine holds the part |

**Dial readings.** A dial target is shown as a picture of the collar: the index at the top, the arc from 0 to the reading, the reading in the middle, and "N turns +" above it. `lathe::dial(machine, slide, distance) -> (turns, reading)` converts with the slide's `*_turn` and `*_div` and, for the cross slide, `cross_reads`. Carriage distances are measured from the carriage zero (the end); cross-slide distances from the touch on the measured diameter.

**Measured values.** `Document.measured: BTreeMap<String, f32>` (`#[serde(default)]`) keeps what the user typed, keyed by setup id and step. The guide uses a measured diameter instead of the planned one for the rest of that section.

## 7. Lathe ↔ mill

- **Add lathe setup** on the setup list inserts a lathe setup. Placed first, it takes the stock (turning a box stock into a round bar of its largest section is not automatic; the user picks Round bar) and becomes setup 1.
- **Lathe → mill:** the mill setup starts from the turned part's leftover box. Its vise holds the round part on the bounding box; the setup editor and the Hold screen say "Round part: hold it on a V-block or in soft jaws." (phase 2 models them).
- **Mill → lathe:** the lathe setup's frame is chosen like any setup's (rx/ry/rz) so the part's turning axis is Z. The chuck choice follows §3.

## Phases

1. Lathe machine and dial check, round bar and revolve simulation, chucks, lathe tools, the Turn strategy and Drill on the centre, checks and minutes, the lathe guide with dial pictures and measure steps, Add lathe setup, hand-over both ways on bounding boxes.
2. Finding the turned profile from the imported part (axis, rings, steps, bores) and suggesting Turn operations; V-blocks and soft jaws in the mill vise.
3. The AI planner on lathes.

## Testing

- Old documents read unchanged (no lathe kind, box stock).
- Dial conversion: taking 6.00 off the diameter with `cross_turn` 4 is "1 turn + 2.00" with `Diameter` and "0 turns + 3.00" with `Radius`; a carriage stop 65 mm from the end with `carriage_turn` 20 is "3 turns + 5.0"; the quill likewise.
- Round stock: the voxel volume and the meshed surface of a Ø50 × 100 bar; revolve cuts remove exactly the ring; a mill setup after a lathe setup sees the turned diameters (through the same `Frame::transfer`).
- Turn plans: passes for Outside and Bore with equal depths and the finish pass, never inside the allowance or the part; Face, Groove, Part off, Thread infeed table, Drill on the centre; the part check refuses a ring into the part.
- Chucks: 3-jaw chosen for round, 4-jaw for square; each check message; the stickout and grip rules.
- Guide: the step list for a sample part (Hold, Zero, Face, Outside passes with a Measure, Drill, Groove, Thread passes, Part off, Hand over); the dial check only while `cross_reads` is empty; measured diameters recompute the last pass; the place and the measurements survive save and load; gallery renders of every lathe screen checked by eye.
- No program, export or Run for a lathe setup; CNC programs unchanged.
