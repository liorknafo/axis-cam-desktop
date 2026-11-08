# The AI planner plans the manual mill and the lathe — design

Date: 7 November 2026. Linear: LIO-353. Branch: `t3/318843f4` (from `main` 494c91a).

## Goal

The software plans the whole job. The manual mill and the manual lathe make no G-code: the user follows their Steps pages. The AI planner plans setups on them as it plans CNC setups, and the user can change those setups in the planner's chat like any other.

- On the **manual mill** the planner proposes **Box** and **Drill** (Drilling, Chip breaking, Deep drilling, Counterboring, Tapping, Reaming, Boring).
- On the **lathe** it proposes **Turn** (Face, Outside, Bore, Chamfer, Groove, Part off, Thread) and **Drill** on the centre line.
- Hand setups go **before** the CNC (facing and squaring a sawn block, turning a bar) or **after** it (tapping, turning a milled part).
- The aim is the **shortest total job**: the lathe's, the manual mill's and the CNC's minutes, plus the move time each time the stock changes machine (`plan_check` already adds them up).

## Decisions

- The user's own setup stays the anchor (decided 7 November 2026, from the manual milling design §4 phase 3). **The user's setup** is the first setup of the project that is not on a hand machine. When every setup is on a hand machine, it is the first setup. It keeps its id, name, work offset, work zero and holder, and its own stock fields define the fresh stock. Hand setups the planner puts before it take that stock, and the user's setup then starts from what they leave (`stock_source` of the setup before it). Its stock fields stay as they were, so planning again from scratch sees the user's block again.
- The planner chooses the manual work itself, for the shortest total job (manual milling design, "Decisions made by the user"). The user can also ask for it in the chat.
- Two phases, one PR each: (1) the manual mill; (2) the lathe. Phase 2 builds on phase 1's machine list and per-setup machine.
- Only the user's setup's CNC machine is offered: other CNC machines in the library are not (as today).
- The user's setup stays in every plan, with at least one operation. A part made wholly on the lathe still has its CNC setup as the anchor; planning a job with no CNC setup is not part of this work (the precondition still asks for a CNC setup).
- `machines` is numbered over the whole project (the library's hand machines, then those of the current setups by id), in the report and in the builder alike, so a hand machine that only a setup in front of the user's has keeps its number.

## Phase 1 — the manual mill

### The user's setup

`analysis::base(doc) -> usize` is the index of the user's setup.

- `analysis::report`, `preconditions`, `own_entry` and the report's `workholding` work on `doc.setups[base]` instead of `doc.setups[0]`. Its stock is resolved from its own stock fields, as for a first setup.
- The precondition keeps its label, "Setup 1: 3-axis LinuxCNC or UC100 machine and stock", checked on the user's setup.

### The part report

- `machines`: the machines a setup may use, numbered from 1.
  - Number 1 is the user's setup's machine.
  - Then come the library's manual mills (and in phase 2 its lathes), then any hand machine of a current setup that is not in the library, matched by id.
  - Each entry has `number`, `kind` (`CNC`, `Manual mill`, `Lathe`) and `travel`.
  - A CNC entry has `max_rpm` and `max_feed`.
  - A manual mill entry has:
    - `gears` (rpm list, or `[min, max]` with `variable` true);
    - `hand_feed` and `hand_rapid` (mm/min);
    - `power_kw`;
    - `dro_compensation`;
    - `pass_time_s`, `tool_time_min` and `move_time_min`.
  - No machine names are reported. Like the holders' names, they stay out of the report.
- `machine` stays as it is: setup 1's limits, now the user's setup's.
- `tools[].machines` lists the numbers of the machines the tool fits (`manual::fits`).
- `workholding_library[].machines` does the same for each holder.

### The proposal

- Every setup gains `machine` (an integer, required): a `number` from `machines`.
- Strategies gain `Box`, but only when `machines` has a manual mill. The schema and the instructions without hand machines stay exactly as they are, as with the rotary axis.
- Operations gain `box` (null for other strategies): `x_min`, `x_max`, `y_min`, `y_max`, `z_top`, `z_bottom`, in the setup's **work coordinates**, plus `allowance` (null: 0.5). `stepdown` and `stepover_percent` are the Box's (null: automatic stepdown, 70 %).
- The check report's `stocks` gain `work_zero`: where each setup's work zero is, from the model's -X/-Y/bottom corner in that setup's axes. Its `box_from_part_corner` and the plane bboxes can then be turned into work coordinates. The part report gains the same `work_zero` for setup 1's orientation, worked out from the user's stock and zero.

### Building

Let `u` be the first proposed setup on machine 1.

- It is the user's setup, built from `doc.setups[base]`: the setup-1 rules of today apply to it. Those rules cover its orientation, its `holder` (null or its own entry), and its `from` (in an edit, the user's setup's number).
- A plan without one is refused: "Keep your setup: give one setup `machine` 1."
- Setups before `u` must be on a hand machine.
- The first proposed setup, when it is not `u`, takes the fresh stock:
  - On a manual mill it must use setup 1's orientation, and it takes the user's setup's stock fields, rotation, translation, work zero, fixture and clearance.
  - It takes the user's holding when that holder fits the manual mill; otherwise it takes the `holder` it names, which must fit.
  - This is `Document::add_manual_setup`'s hand-over, done by the builder.
- Each setup is made on its machine:
  - machine 1 is the user's setup's machine snapshot;
  - any other machine is the library entry, cloned.
  - Hand setups take no work offset of their own (the first, handed over, carries the user's, which no check reads). Only setups on machine 1 count against the six offsets.
  - A plan has at most 10 setups.
- An orientation may be used once **on the CNC**. A hand machine may use one again: the manual mill may face the top that the CNC then machines from the same side, and the stock may come back to the manual mill after the CNC on that side (review: astra round 1).
- In each setup:
  - `Box` is refused off the manual mill.
  - On the manual mill, only `Box` and `Drill` are allowed. The Drill cycles `Helical milling` and `Thread milling` are refused there.
  - A tool or holder that does not fit the setup's machine is refused, with the machine's number.
- On a hand machine the rpm is set to the nearest gear (`manual::snap`) and the feed is capped at the hand feed. The cutting-data check then judges the result, as for any operation.
- The `through` rule stays "only in the first setup" of the job, which is where the stock's bottom is still whole.

### Changing a plan (edit)

- `describe` shows every setup with its `machine`, including the hand setups before the user's setup, and Box operations with their `box`.
- A setup keeps its `from` only on the same machine.
- The user's setup keeps the user's setup's `from`. The text "Setup 1 stays first" becomes "Your setup stays, with its from".

### Instructions

- The opening says that the planner plans CNC jobs and, when `machines` lists them, the manual mill's and the lathe's setups too.
- A **MACHINES** section, only when `machines` has a hand machine. It covers:
  - `machine` on every setup, and which setup is the user's;
  - hand setups before and after it, and how the stock passes between them;
  - which tools and holders fit each machine;
  - the gears and the hand feed;
  - the move time;
  - the goal of the shortest total, read from the check report's minutes.
- A **MANUAL MILL** section, only when `machines` has a manual mill. It covers:
  - Box, which moves one axis at a time and covers facing, squaring a side, steps, slots and pockets;
  - work coordinates and `work_zero`;
  - the allowance that is left for the CNC to finish (0 makes the cut final);
  - the conventional cut;
  - a closed box plunging, so it needs a centre-cutting end mill;
  - Drill and its cycles on the manual mill: Tapping, Reaming and Boring there, no Helical milling;
  - what to use it for: bulk removal from a sawn block when it saves more than the move time, tapping, and holes drilled by hand.

## Phase 2 — the lathe

Decided while building phase 1 (7 November 2026):

- A Turn's feed is not the model's: the lathe takes its power feed nearest 0.1 mm/rev for Outside, Bore and Thread passes, and hand feeds for the rest (`turn::POWER_FEED`, `HAND_FEED`). The proposal's `feed` is ignored for Turn. The lathe's speeds and feeds are not checked against the material (as today).
- The library's chucks join `workholding_library` only when `machines` has a lathe, so the numbering of a job without a lathe does not change.
- A lathe setup's part is placed in the bar as the chuck on A places it (`proposal::in_chuck`): on the axis, its top `free_end` below the bar's free end.

- `machines` lists the library's lathes. Each has `gears`, `feeds_mm_per_rev`, `pitches`, `cross_reads` (and a warning when it is empty), `travel` (the swing and the distance between centres), `pass_time_s`, `tool_time_min` and `move_time_min`.
- Lathe tools are reported: the turning tool's hand, nose radius and lead angle; the boring bar's nose radius, smallest bore and reach; the width and reach of grooving tools and parting blades; the threading tool's thread angle and pitches; the die's thread. The library's chucks are reported too: jaws, smallest and largest grip, jaw length and body.
- `turning` in the part report, only with a lathe: for each orientation, the part as turned about its vertical axis.
  - `round`: whether the part's section is centred and round over most of its length.
  - `profile`: the steps from the free end (the top), each with `from` and `to` (mm down from the part's top), `outer_diameter`, `inner_diameter` and `round` (`turn::profile` with a roundness test on the triangles that face out).
  - The threads on the axis (`thread::in_setup`).
- A setup on a lathe gains `bar` (null for the others): `diameter`, `length`, and `free_end` (stock beyond the part at the free end; null: 2 mm).
  - A lathe setup that takes the fresh stock (the first setup) must give it.
  - A lathe setup after another setup starts from what that setup left.
  - Its `holder` is a `Chuck` from the library that fits the lathe; null takes the 3-jaw.
  - The grip is the chuck's jaw length. The stick-out is the rest of the bar, as `add_lathe_setup` sets it.
  - Its orientation is the turning axis, and the work zero is `Stock top center` (Z 0 at the free end).
- Operations gain `turn` (null for other strategies): `kind` and the kind's fields as `lathe::turn_defaults` names them (`diameter`, `z_end`, `depth`, `finish`, `allowance`, `face_off`, `to_diameter`, `edge`, `size`, `z`, `width`, `thread_side`, `pitch`, `passes`). They are in lathe work coordinates: Z 0 at the free end and negative toward the chuck, with diameters as diameters. `feed` is mm per revolution, one of the lathe's `feeds`.
  - `Turn` is refused off the lathe. The lathe takes only `Turn` and `Drill`, and the Drill only on the centre line.
- A **LATHE** section in the instructions. It covers:
  - reading `turning`;
  - choosing the bar (the part's largest round diameter plus a few mm, its length plus the free end, the grip and a part-off width);
  - the order of the Turn kinds;
  - the 3-jaw for round stock;
  - handing the part to the mill (a V-block or soft jaws from the library hold it round) and from the mill (the lathe setup's orientation is the turning axis);
  - threads by die or threading tool;
  - what the lathe cannot do: no taper other than a chamfer, no knurling, nothing off the axis.

## Testing

- **Unchanged without hand machines.** With no hand machine in the library, the schema and the instructions are identical to today's, and every existing planner test passes unchanged.
- **The user's setup.** `base` is the first CNC setup. A report of a project whose setup 1 is a manual setup in front is the report of the project without it. The preconditions accept a user's setup on a hand machine.
- **Building:**
  - A manual setup first takes the stock, orientation, zero and holding (only when the holding fits).
  - The user's setup then starts from it, keeps its id, offset, holder and stock fields, and has `stock_source` of the manual setup.
  - These are refused: a Box on the CNC; a Pocket on the manual mill; Helical milling on the manual mill; a tool or vise not on the machine; a CNC setup before the user's setup; no setup on machine 1; a manual setup first in another orientation.
  - An orientation can be used once on the CNC; hand machines may repeat one.
  - The rpm on a hand machine is snapped to a gear.
- **Describing a plan.** `describe` of a planned job with a manual setup in front shows it with `machine` and `box`. Building what `describe` shows keeps every setup and operation identical.
- **Check.** A plan with a manual facing Box first passes every check. Its `manual_minutes` and `move_minutes` are in the check report, and `stocks[].work_zero` is right.
- **Model loop.** A scripted model (`fake_openai`) plans a manual Box and the CNC. The applied job has the manual setup first, and its Steps page shows the Box.
- **Phase 2.** `turning` for a stepped shaft gives its diameters from the end, and a square bar is not round. A lathe setup first with a bar and Outside, Face and Part off passes. The mill after it starts from the turned part. A Turn on the mill is refused, and a Turn whose tool is not a lathe tool is refused.
