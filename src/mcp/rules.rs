//! The machining rules an agent plans by (LIO-470), by topic, for the
//! `guide {topic}` tool. They are the rules the AI planner's instructions
//! held (`planner::instructions_full`: its strategies, WORKHOLDING,
//! PLANNING THE HOLDING, SPEEDS AND FEEDS and RULES), said for an agent that
//! works with the user's own commands and fields rather than a proposal.
//! The planner's own text goes with it in M4; these stay.

/// Each topic: its name, a line on what it holds, and its text.
pub const TOPICS: [(&str, &str, &str); 6] = [
    (
        "workflow",
        "The order of work, and how to check it.",
        WORKFLOW,
    ),
    (
        "stepdowns",
        "How deep each pass goes, and the tool's reach.",
        STEPDOWNS,
    ),
    (
        "cutters",
        "Which cutter each operation takes, and in what order.",
        CUTTERS,
    ),
    (
        "holding",
        "How each setup's stock is held, and what that asks of the plan.",
        HOLDING,
    ),
    (
        "chip_load",
        "Speeds and feeds against the material's cutting data.",
        CHIP_LOAD,
    ),
    (
        "minimum_rpm",
        "The least spindle speed and feed, and what the machine's limits excuse.",
        MINIMUM_RPM,
    ),
];

/// The topic's text, or None for a topic there is not.
pub fn text(topic: &str) -> Option<&'static str> {
    TOPICS.iter().find(|(name, ..)| *name == topic).map(|t| t.2)
}

const WORKFLOW: &str = "\
- Read the job first: `inspect` (setups and operations by id), `part_report` (the part's size, orientations, planes, holes, the machine's limits and the material's cutting data) and `list_surfaces` (the surfaces an operation picks, with the planes of each setup's orientation).
- Decide before any operation how every setup is held and what stock it starts from (topic `holding`). A later setup starts from the stock the earlier setups left: `stock_after {setup}` tells it.
- Then for each setup: add its operations in order (topic `cutters`), pick their surfaces (`pick_surfaces`; on the built-in sample, its features with `pick_features`), set their fields (`list_fields`, `edit_field`): tool, stepdown (topic `stepdowns`), speeds and feeds (`cutting_data {tool, op}`, topic `chip_load`).
- Place the holding: `suggest_layout` lays the vise or clamps out clear of the setup's toolpaths, or `set_clamps` and the setup's fields place them by hand. Nothing else fills a placement in: `check_holding {setup}` and `check_job` check the job exactly as it is.
- `generate_all`, then `check_job`: it runs every check (generation, gouges, cutting data, workholding, export) on the job as the user sees it, and says `passed` or the `problems` by setup and operation. Fix each problem where it says, and check again. A check whose job changed while it ran is refused as stale: check again.
- Look when unsure: `view_model` draws the part, the stock, the holding and, for an operation, its toolpaths and the stock it leaves.
- Then review the NC, save the project and export.";

const STEPDOWNS: &str = "\
- A stepdown is at most the tool's flute length; a finer one multiplies the moves and the cutting time, so choose it for the cut, not for safety. With no stepdown the default is 2 mm.
- Face: levels = (the stock above the face − bottom offset) ÷ stepdown, rounded up. Facing needs stock above the face: face only where the setup's stock stands above it.
- Pocket and inside Contour: from the stock top down to each floor, the highest first; the floor's depth plus the stock above the top must be less than the tool's stickout.
- Outside Contour: from the stock top down to the part's bottom plus bottom offset. The cut depth from the stock top must be less than the stickout; for a tall part stop higher with bottom offset and finish from the setup turned over.
- Helical milling: a helix of stepdown per turn, at most the flute length; the drilling depth rule applies with the flute length.
- Waterline: from the part's top down to its bottom plus bottom offset, at most the stickout, about height ÷ stepdown layers.
- On the manual mill a Box with no stepdown takes what the spindle's power can: give one for soft stock. Its closed sides and floor keep an allowance (0.5 mm by default) for the CNC to finish; 0 makes the manual cut final.
- No tool tip may go deeper below the stock top than the tool's stickout, and nothing may go below the bottom of the stock; in setups after the first, nothing below the part's underside.";

const CUTTERS: &str = "\
- Use only the library's tools: `part_report` lists them (`tools`, by number, with diameter, flutes, flute length and stickout); a tool must fit the setup's machine.
- Face: a flat end mill or face mill. Pocket: a flat end mill no wider than the floor's largest cutter (`max_cutter_diameter` in the planes); a floor it does not fit is skipped. The same Pocket again with a smaller cutter is a rest pass: it cleans the corners and narrow places in a fraction of the time (give it no finish passes).
- Through openings keep the cutter's radius in their corners: finish cut-outs that need tight corners with a rest pass of the smallest cutter that reaches.
- Outside Contour: one path round the whole outline; it removes one tool width, not all the stock round it. Inside Contour: a floor's walls, the Pocket's tool and depth rules.
- Drill: a drill no wider than the hole; deep holes peck. Helical milling cuts any hole a flat end mill fits, with a flat floor (a drill leaves its 118° point); not on a manual mill.
- On the manual mill a Box closed at both ends of its passes (a pocket, a closed slot) plunges at its first row: give it a centre-cutting end mill. A Slot's cutter is the slot's width less twice its radial allowance.
- Order the operations of a setup: Face, then Pocket (larger cutters before the rest passes of smaller ones), then Contour, then Drill (centre drills and drills before chamfer mills), then finishing.
- Coolant only from the machine's coolant outputs: one it has no output for is refused.";

const HOLDING: &str = "\
- A later setup starts from the stock the earlier setups left, not the whole block: a faced top makes the flipped stock shorter, and cut sides make it narrower. A holder that does not fit that stock, or has nothing to grip on it, fails the check.
- A vise closes on the material in its grip band, from the stock's bottom up to the grip. It needs material at both jaws through the band (at least 60 % of each jaw's face), material under the band to rest on (at least half), stock at least 1 mm above the grip, and a band no wider across the jaws than its maximum opening. Walls an earlier setup cut through the whole band can be gripped; sides cut through only part of it cannot.
- Clamps press on the stock's top at its edges and need material under each bite (at least 60 %) for the whole setup: a setup held by clamps must not cut under its own clamps (face only the part's outline, keep pockets and contours off the bites). A bite over the part's outline loses its stock once the part's top is cut: clamps need stock beside the part on two opposite sides, and that stock must stay joined to the part until the setup ends (leave a skin or tabs, for example with bottom offset, until the last setup).
- The jaws and clamps are keep-out boxes grown by the setup's clearance gap: any move, rapids included, within the gap of one is a workholding problem. Keep outside contours above the jaws' top (bottom offset above it plus the gap) and finish below from the setup turned over. Parallels, bolts and the table are not checked.
- Plan the earlier setups for the later ones: a setup turned over grips what was their top, so cut those sides through the whole band or leave them whole; a setup facing the same way grips what was their bottom, so stop an outside Contour above the grip.
- A lathe chuck grips `grip` mm of the bar and the rest stands out (`stickout`): cut only the stickout, from its free end to the gap plus the tool's radius short of the jaws.
- Each setup's placement is the user's to see: `suggest_layout` places the vise or clamps for its toolpaths (or the chuck for its stock), `set_clamps` and the setup's fields place them by hand. `check_holding {setup}` checks the placement exactly as saved, and a setup with clamps but no positions fails it.";

const CHIP_LOAD: &str = "\
- The material's cutting data (in `part_report`, for carbide tools) gives the surface speed range in m/min and the chip load per tooth in mm, in five bands by tool diameter; the last band holds every larger diameter.
- Each operation is checked: surface speed Vc = π × D × rpm ÷ 1000 (m/min) must lie in the material's range, and chip load fz = feed ÷ (rpm × flutes) (mm per tooth) in the band of the tool's diameter. Under 3 mm the band shrinks in proportion to the diameter (a Ø1 tool takes a third of it, a Ø0.5 a sixth).
- Choose Vc in range and set rpm = Vc × 1000 ÷ (π × D); choose fz in the band and set feed = fz × rpm × flutes. For example a 3-flute Ø10 mm tool at 8000 rpm and 1000 mm/min runs at Vc = 251 m/min with fz = 0.042 mm per tooth.
- Drills are checked the same way: a drill's feed ÷ (rpm × flutes) must fall in its diameter's band. A small drill needs a high rpm to reach the surface speed.
- `cutting_data {tool, op?}` works this out for a tool on the operation's machine: the rpm range, a recommended rpm, and at that rpm the feed range with a recommended feed.
- Plunge and ramp feeds are not checked. An operation outside the material's cutting data is a problem at the stage `cutting data`.
- On a hand machine the rpm is set to the nearest of the machine's gears and the feed is capped at its hand feed, then checked the same way: choose a gear and a feed inside it. The lathe's speeds are not checked against the material: choose them from it yourself, at the largest diameter cut.";

const MINIMUM_RPM: &str = "\
- Every spindle speed is at least 100 rpm and every feed at least 1 mm/min, both within the machine's maximums. A tool whose material surface speed range gives less than 100 rpm even at its fastest cannot be used in that material: choose a smaller tool.
- When even the machine's maximum rpm gives a tool less than the material's minimum surface speed, running at the maximum rpm counts as in range. When the least chip load needs more than the machine's maximum feed, running at the maximum feed counts as in range.
- A value above the material's maximum is never excused, and rounding gets 1 % of slack on each limit.";
