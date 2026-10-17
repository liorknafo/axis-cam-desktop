# Manual milling before the CNC — design

Date: 17 October 2026. Branch: `t3/manual-milling-planning` (from `main` f85235d).

## Goal

1. The user's big **manual mill** is a machine in the project, with its own tools, and a setup can run on it.
2. On the manual mill the app plans only **cuts made by moving one axis at a time**: facing, squaring sides, steps and shoulders, slots, axis-aligned pockets and drilling. They are simulated like any other operation, so the CNC setups after them start from the stock they leave.
3. A manual setup has no G-code. MACHINE shows it as a **visual step guide**: one screen per step, with pictures of the stock, the tool and the cut, and the numbers the DRO should read. Text is limited to short labels.
4. The user marks what goes to the big mill, or the app or the AI planner chooses, with the **shortest total time** as the goal (manual time, CNC time and moving the stock between machines).

## Decisions made by the user

- Facing needs **no sub-steps**: only the tool and the depth to cut to (17 October 2026).
- The guide is split into **one section per cutting operation** (17 October 2026).
- The screens in the mockup of 17 October are accepted: squaring a side is one screen like facing, and the Hold and Check screens stay. Z 0 stays on the top picked up in the Zero step for the whole setup, so every Z in the guide is measured from it.
- The big mill is manual with a DRO. The DRO has **tool-radius compensation**.
- **No power feed** on any axis: every cut is fed by hand.
- The big mill does **everything that needs only one axis moving at a time**. Anything more complicated goes to the CNC.
- The instructions **must be visual**, not prose, because text is hard to follow and easy to misread at the machine.
- The user marks manual work themselves, or the planner may choose it. Choices aim for the fastest job.

## Non-goals

Power feed, a DRO connection (the app does not read the DRO), arcs or two axes moving together on the manual mill, a rotary table or dividing head, quill-only drilling depth stops, boring heads, tapping, printing, and changing the CNC's G-code output.

## 1. The manual machine

`Machine.p` gains `kind`: `CNC` (default, so existing documents read unchanged) or `Manual`. A manual machine keeps name, travel (`x`, `y`, `z`), `power`, and has its own fields:

| Field | Meaning | Default |
| --- | --- | --- |
| `speeds` | Spindle speeds the gearbox has, comma-separated rpm (or a range `min-max` for a variable drive) | `90,150,250,400,640,1000,1600` |
| `hand_feed` | The fastest feed the user can turn steadily by hand, mm/min | `150` |
| `hand_rapid` | How fast the user moves an axis off the cut, mm/min | `1500` |
| `turn_x`, `turn_y`, `turn_z` | mm per handwheel turn on each axis | `5`, `5`, `2.5` |
| `dro_comp` | `Yes` when the DRO has tool-radius compensation | `Yes` |
| `edge_finder` | Diameter of the edge finder the Zero step uses, mm; its radius is taken off each face touched | `10` |
| `pass_time` | Seconds to reset, set and lock the axes for one pass | `20` |
| `tool_time` | Minutes to change a tool and pick up the zero again | `3` |
| `move_time` | Minutes to move the stock between this machine and another | `10` |

On save, `rpm` is set to the highest gear and `feed` to `hand_feed`, so the existing rpm, feed and material checks apply to a manual machine unchanged. The post fields, probe fields, controller and Run do not apply to a manual machine and are hidden on its page. The machine editor shows "Manual mill — no G-code; MACHINE shows the steps."

**Tools.** `Tool.p` gains `machines`: the ids of the machines that hold the tool, comma-separated. Empty means every CNC machine (existing tools keep working on the CNC and do not show on the manual mill). The tool editor shows a check box per machine. An operation whose tool does not fit its setup's machine has an operation issue (shown in the editor; the simulation and the plan check stop on it like any other operation issue).

**Workholding.** Library entries gain the same optional `machines` field, with the same rule. The manual mill's vise is usually a different, bigger vise.

## 2. Manual cuts

### One strategy: Box

`Strategy` gains `Box`: remove an **axis-aligned box** of material with one tool. Facing, squaring, steps, slots and pockets are all boxes with different open sides:

| Cut | Box |
| --- | --- |
| Face the top | The whole stock top, from Z 0 down to the depth |
| Square a side | A thin slab along one side, full height |
| Step / shoulder | Open on one side and the top |
| Slot | Open on two opposite ends and the top, at least the tool's diameter wide |
| Pocket | Open on the top only; its inside corners keep the tool's radius |

Box fields (in the setup's frame, relative to the work zero): `x_min`, `x_max`, `y_min`, `y_max`, `z_top`, `z_bottom`, `allowance` (default 0.5 mm, which is shrunk off every side that touches the part, so the CNC finishes it; 0 makes the manual cut final), `stepover` (default 70 % of the diameter), `stepdown` (empty means automatic: the depth the machine's power can take at that width, rpm and feed, from a new power function next to the forces model, at most the flute length and the diameter), rpm (snapped to the nearest gear in `speeds`) and feed (at most `hand_feed`).

**Pick from the part.** In the operation editor, the user clicks a flat, axis-aligned face of the part (a floor or a wall). The app fills in the box from that face, up to the stock's edge or the next part face in each direction, inside the leftover stock (`RestStock::left`).

### Paths: one axis per move

The generator in `src/cam.rs` makes every move change exactly one of X, Y and Z:

- Passes run along the box's longer side. Between passes the tool steps across by `stepover`. Z steps down by `stepdown` until it reaches `z_bottom`.
- Each pass is **conventional milling**: with the spindle turning clockwise (M3), the uncut material stays on the left of the feed direction (on the right would be climb). A pass returns to the start off the cut rather than cutting back the other way. Backlash on a hand-fed mill can pull the cutter into a climb cut.
- Z goes down outside the stock when the pass starts at an open side. A box closed at both ends of its passes (a pocket or a closed slot) plunges straight down at a quarter of the feed once per level, at its first row, which needs a centre-cutting end mill; the Pass screen warns of it. Later rows on the level back off the cut wall 0.2 mm (never into the allowance), return along the cleared row and feed across into the next row, one axis at a time.
- **Drill** is reused: on a manual machine its moves are split the same way (Z up, then X, then Y, then Z down), so the holes also need one axis at a time.

Strategies other than Box and Drill are refused on a manual setup (problem at stage `structure`), and Box is refused on a CNC setup in phase 1.

### Simulation and checks

Box and Drill paths go through the existing simulation, so the solid, the leftover box, holding on the next setups and the CNC's rest material all follow. The checks on a manual setup:

- every move is single-axis;
- the box stays inside the machine's travel;
- the cut does not touch the part (the gouge check as today) and leaves `allowance` where it is above zero;
- the rpm is one of the machine's gears, the feed is not above `hand_feed`, and the force and power stay inside the forces model's limits for the machine;
- the vise or clamps hold the stock (§2 of the 8 October design, unchanged).

### Time

The minutes for a manual operation are: cut length ÷ feed + travel off the cut ÷ `hand_rapid` + passes × `pass_time` + tool changes × `tool_time`. The plan's total adds `move_time` each time the stock moves from one machine to another. The plan check shows manual, CNC and moving minutes apart and together, and the Steps page shows the setup's minutes. The setup sheet (which does not simulate) names the machine and says "Manual mill: steps on the MACHINE page".

## 3. The visual step guide

A manual setup in MACHINE shows a **Steps** page instead of Run. It works with no controller connected. The steps come from the setup's simulated operations, so the pictures always match the cut the app checked.

### Step kinds

| Step | The screen shows |
| --- | --- |
| Hold | 3D view of the stock in the vise: which face is up, which faces go to the jaws, parallels under it, the grip height |
| Zero | The corner to pick up with the edge finder (DRO compensation off), the face each of X and Y is touched on and the number to set there (the zero less or plus the finder's radius, e.g. X −5.00 with a Ø10 finder on the left face), and Z set on the top |
| Tool | The tool drawn to scale, its diameter, the gear to select (rpm), and on a DRO with compensation the diameter to enter in it |
| Face | **One screen for a whole facing cut** (a Box open on all four sides, over the whole top): the tool and its gear, how much comes off, and the Z to face down to. Top and front views show the area and the final top. The user plans the passes themselves, so there is no Pass step and no Check step for it |
| Square | **One screen for squaring a side** (a Box with exactly one closed X/Y side, the wall, and no closed floor): the tool and its gear, the side, the X or Y to cut the side to (an edge readout with compensation), how much comes off, and how deep to go. No Pass or Check steps |
| Pass | One pass (the mockup): top view and front view, the hatched band this pass removes, the tool where it starts, an arrow to where it stops, what earlier passes removed |
| Holes | Top view with every hole numbered, the current hole highlighted, its X and Y, and the depth on the quill or Z |
| Check | 3D view of the stock after the operation, next to the previous one, with the main sizes |
| Hand over | The stock as it leaves, and the next setup it goes to ("→ CNC setup 1"), with its 3D view |

### Sections

The guide is split into **one section per cutting operation**, between a **Set up** section (Hold, Zero) and a **Hand over** section:

- A section bar on top replaces a bar of single screens. It has one entry per section with its name and progress ("Front step · pass 2/4"), done sections ticked, and "→ CNC setup N" last. Tapping a section jumps to its first unfinished screen.
- Each section header shows the operation's name, tool, count (passes or holes) and minutes.
- A section starts with its Tool screen (or Comp screen) when the tool or the compensation side changes, and ends with its Check screen. Facing is one screen and nothing else (see Face above).
- The last screen's button is "Section done". The user can stop after any section and come back to the next one later; finished sections fold up to their header.

### The Pass screen

- **Numbers in the order they are moved.** Each target is one large DRO readout with a step number: 1 Z up off the stock, 2 X to the start, 3 Y to the pass, 4 Z down to the depth, 5 feed along the cut to the end. The current target is the bright one. The axes to lock during the cut show a lock.
- **Edge or centre.** With `dro_comp` on, the X and Y readouts show the **tool edge**, with the tool-centre number small underneath. The compensation side is fixed per operation and axis (toward the box's closed side on that axis, + when both or neither are closed), so one operation never asks for a change. The Tool step shows the sides to set as arrows, and a Comp step is added when the next operation needs different sides with the same tool. With `dro_comp` off, the readouts show the tool centre.
- **Feed by hand.** The feed is shown as handwheel turns: "1 turn every 2 s" from `hand_feed` and `turn_x`/`turn_y`, with a dot that blinks once per turn.
- **Passes.** Small boxes for every pass of the operation: done, now, next.
- **Never below.** The lowest Z and the closest X/Y the operation reaches are shown as red lines on the views. These are the numbers the user must not pass.
- Buttons: Back and Pass done. Pass done on the last pass goes to the Check step. The guide remembers its place per setup (saved with the project, without marking the plan changed) so a restart resumes there.
- The guide works while the CNC runs another job, so both machines can work at once.

### Pages to take to the machine (phase 2)

"Save steps" writes one PNG per step into a folder chosen by the user. It needs a headless renderer in the app (today only the tests have one), so it comes after the screen itself.

## 4. Choosing what goes to the big mill

### Marked by the user (phase 1)

"Add manual setup" on the setup list inserts a setup on the manual machine before the first CNC setup. When it becomes the first setup, it takes the stock, orientation, zero and holding of the old first setup, which then starts from the stock the manual setup leaves. The user adds Box and Drill operations to it. Manual setups can be moved, for example to face the bottom after a flip.

### Suggested by the app (phase 2)

**Bulk boxes.** A new `src/bulk.rs` finds the material the manual mill can take, in each of the six directions the part can be clamped with:

1. A height map of the part over a grid of the stock top (1 mm cells), inflated by `allowance`.
2. Greedily, the axis-aligned box that removes the most material: a rectangle in X and Y whose floor is the highest part height under it plus the allowance, from the current top. The same is done from each side for steps and squaring.
3. Remove it and repeat, until the next box is narrower than the smallest manual tool or holds less than 2 % of the removable material.

**Fastest.** Each box gets manual minutes (§2, with the biggest manual tool that fits) and CNC minutes for the same material (its volume ÷ the forces model's highest removal rate for the CNC's best roughing tool). The app keeps the boxes that save time, and only proposes the manual setup when the total saved is more than `move_time`. "Suggest manual steps" on the setup list shows the boxes in 3D and the minutes saved, and adds the setup when the user accepts.

### Chosen by the AI planner (phase 3)

- The part report gains `machines`: every machine by number with its kind and limits (manual: gears, `hand_feed`, power, travel, `dro_comp`), the tools each machine holds, and the bulk boxes from phase 2 with their minutes.
- The proposal schema: each setup gains `machine` (a number). A manual setup may only have Box and Drill operations. Box operations name a bulk box or give their own box.
- **The user's setup.** Today setup 1 is the user's (stock and holder fixed). This becomes: the user's first CNC setup keeps its holder and orientation. If the planner puts manual setups before it, the stock fields are carried into the first setup and the user's setup starts from the leftover stock.
- `check_plan` answers with manual, CNC and moving minutes. The instructions gain a **MANUAL MILL** section: the big mill does one-axis cuts only; use it for bulk material when it makes the job faster in total; leave the allowance where the CNC finishes; always plan the holding on both machines.
- `proposal.rs:825` stops copying setup 1's machine into every setup and uses each setup's `machine`.

## Phases

1. Manual machine, tool and holder fit, the Box strategy, simulation, checks and times, and the step guide. The user marks the manual work.
2. Bulk boxes, "Suggest manual steps" and the PNG pages.
3. AI planner support.

## Testing

- Old documents read with every machine CNC and every tool on every CNC machine.
- Box paths: every move changes exactly one axis; passes are conventional with M3; the cut stays inside the box shrunk by the allowance; Z goes down only outside the stock or after a starter hole; slots narrower than the tool are refused.
- A faced top on the manual mill makes the next CNC setup's leftover stock shorter by the facing depth, and the CNC's rest material excludes the boxes.
- Checks refuse: a CNC-only strategy on a manual setup, a tool not on that machine, an rpm not in `speeds`, a feed above `hand_feed`, a box outside the travel.
- Minutes: a known box gives the expected manual time; the total adds `move_time` once per machine change.
- Guide: the step list for a sample job (hold, zero, tool, passes, check, hand over); edge readouts with `dro_comp` on equal the centre ± the radius on the cutting side; the guide's place survives save and load; gallery renders of each step kind checked by eye.
- Phase 2: "Save steps" writes one PNG per step; bulk boxes on a stepped block find the steps; boxes are never inside the part inflated by the allowance; the time rule drops a box that saves less than it costs.
- Phase 3: the schema's `machine`, Box only on manual setups, the user's setup rule, and a real-route run by the user.

## Open points

- The **allowance**: 0.5 mm by default. Faces the manual mill finishes exactly (allowance 0) are left to the user in phase 1, and the planner always leaves the allowance.
- Default gears, `hand_feed` and the handwheel mm per turn are placeholders until the user enters the big mill's numbers.
